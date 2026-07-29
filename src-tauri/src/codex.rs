//! 从 Codex CLI 的 rollout 文件发现会话，并读出状态。
//!
//! 数据源（本机实测，Codex CLI 里的 `.codex`）：
//!
//! ```text
//! ~/.codex/sessions/YYYY/MM/DD/rollout-<ISO时间>-<uuid>.jsonl
//! ```
//!
//! 每行一条 `{"type":..,"payload":..,"timestamp":..}`，`type` 实测有：
//! `session_meta`（每文件一条，带 `session_id` / `cwd` / `cli_version`）、
//! `turn_context`（每轮一条，带 `cwd` / `model`）、`response_item`、
//! `event_msg`、`world_state`。
//!
//! # 状态从尾部读，不从头部
//!
//! `event_msg` 的 `payload.type` 里有两个明确的边界事件：
//!
//! | payload.type | 含义 | 对应我们的状态 |
//! | --- | --- | --- |
//! | `task_started` | 开始一轮 | `working` |
//! | `task_complete` | 一轮结束 | `idle`（轮到你） |
//!
//! 语义和 Claude Code 的 `UserPromptSubmit` / `Stop` 一致，所以复用同一套
//! 状态名和同一套视觉，不另造一种语言。
//!
//! **只看这两个事件**，不看 `agent_message` / `function_call` 之类：那些在
//! 一轮里出现几十次，用它们判断状态等于把「刚说完一句话」误当成「结束了」。
//!
//! # 为什么是轮询而不是 hook
//!
//! Codex 有 `notify` 配置，但那是 `config.toml` 里的**单个**槽位。本机那格
//! 已经被 OpenAI 自己的 `codex-computer-use.exe` 占着，抢过来会弄坏用户已有的
//! 功能。所以只能定期重扫 —— 代价是状态有几秒延迟，比弄坏别人的配置好。

use crate::agent::{Agent, Discovered};
use serde_json::Value;
use std::fs::File;
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// 找 `session_meta` 时最多读这么多。它实测是**第一行**，但第一行也可能很大
/// （`base_instructions.text` 本机就有 21KB），所以按字节封顶而不是只按行数。
const MAX_HEAD_BYTES: u64 = 256 * 1024;
const MAX_HEAD_LINES: usize = 20;

/// 从尾部回读这么多字节找边界事件。
///
/// 单条 `response_item` 可能很大（工具输出），所以不能只读几行的量。64KB 在
/// 本机样本里足够覆盖到最后一个 `task_started`/`task_complete` ——
/// 真的没覆盖到时我们返回 `None`（状态未知）而不是猜一个。
const MAX_TAIL_BYTES: u64 = 64 * 1024;

/// `~/.codex/sessions`，尊重 `CODEX_HOME` 覆盖。
///
/// 这个环境变量是 Codex 自己的配置根覆盖，和我们对 `CLAUDE_CONFIG_DIR` 的
/// 处理对称 —— 忽略它会在改过位置的机器上扫错目录。
pub fn sessions_dir(home: Option<PathBuf>) -> Option<PathBuf> {
    if let Ok(dir) = std::env::var("CODEX_HOME") {
        let trimmed = dir.trim();
        if !trimmed.is_empty() {
            return Some(PathBuf::from(trimmed).join("sessions"));
        }
    }
    home.map(|h| h.join(".codex").join("sessions"))
}

fn to_ms(t: SystemTime) -> u128 {
    t.duration_since(UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0)
}

/// 递归扫描 `sessions_dir`，返回时间窗内的会话。
///
/// 目录是 `YYYY/MM/DD` 三层，所以必须递归 —— 但**先按 mtime 过滤再开文件**，
/// 和 Claude Code 那边同一个策略：本机 151 个 rollout 文件，窗口内通常只有
/// 几个，这一步把 IO 降到最低。
pub fn scan(sessions_dir: &Path, window: Duration) -> Vec<Discovered> {
    let now = SystemTime::now();
    let mut found = Vec::new();
    walk(sessions_dir, window, now, &mut found, 0);
    found
}

/// 深度上限兜住意外的深层目录树（符号链接环之类）。
/// `sessions/YYYY/MM/DD/file` 只需要 3 层目录。
const MAX_DEPTH: usize = 4;

fn walk(dir: &Path, window: Duration, now: SystemTime, out: &mut Vec<Discovered>, depth: usize) {
    if depth > MAX_DEPTH {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(meta) = entry.metadata() else { continue };

        if meta.is_dir() {
            walk(&path, window, now, out, depth + 1);
            continue;
        }
        if !meta.is_file() || path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
            continue;
        }
        // 只认 rollout-*：同目录下未来可能出现别的产物，按前缀收窄比
        // 「所有 jsonl 都试着解析」更不容易误吞。
        let is_rollout = path
            .file_name()
            .and_then(|n| n.to_str())
            .map(|n| n.starts_with("rollout-"))
            .unwrap_or(false);
        if !is_rollout {
            continue;
        }

        let Ok(modified) = meta.modified() else { continue };
        let too_old = now
            .duration_since(modified)
            .map(|age| age > window)
            .unwrap_or(false); // 时钟回拨导致的负数年龄当成「新」
        if too_old {
            continue;
        }

        if let Some(d) = read_session(&path, to_ms(modified), meta.len()) {
            out.push(d);
        }
    }
}

fn read_session(path: &Path, mtime_ms: u128, len: u64) -> Option<Discovered> {
    let (session_id, cwd) = read_head(path)?;
    let (state, detail) = read_tail_state(path, len);

    Some(Discovered {
        agent: Agent::Codex,
        session_id,
        cwd,
        project: None, // 和 Claude Code 一样按 cwd 末段分工作空间
        mtime_ms,
        state,
        detail,
    })
}

/// 从头部拿 `session_id` 和 `cwd`。
///
/// 两个都取 `session_meta`。`turn_context` 也带 `cwd`，但那是**每轮**一条、
/// 可能中途变化；`session_meta` 是会话的身份，用它更稳。
fn read_head(path: &Path) -> Option<(String, String)> {
    let file = File::open(path).ok()?;
    let reader = BufReader::new(file.take(MAX_HEAD_BYTES));

    let mut session_id: Option<String> = None;
    let mut cwd: Option<String> = None;

    for line in reader.lines().take(MAX_HEAD_LINES).map_while(Result::ok) {
        // 被 MAX_HEAD_BYTES 截断的最后一行会在这里解析失败，正常忽略
        let Ok(v) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        let p = v.get("payload");

        if v.get("type").and_then(Value::as_str) == Some("session_meta") {
            if let Some(p) = p {
                session_id = p
                    .get("session_id")
                    .or_else(|| p.get("id"))
                    .and_then(Value::as_str)
                    .map(str::to_string);
                cwd = p.get("cwd").and_then(Value::as_str).map(str::to_string);
            }
        }
        // turn_context 的 cwd 只作兜底：session_meta 缺 cwd 时总比没有好
        if cwd.is_none() && v.get("type").and_then(Value::as_str) == Some("turn_context") {
            cwd = p.and_then(|p| p.get("cwd")).and_then(Value::as_str).map(str::to_string);
        }

        if session_id.is_some() && cwd.is_some() {
            break;
        }
    }

    // 没有 cwd 就放弃这个文件 —— 猜一个工作目录比不显示更糟，
    // 会把宠物放进错误的工作空间。和 Claude Code 那边同一条规则。
    Some((session_id?, cwd?))
}

/// 回读尾部，找最后一个边界事件。
///
/// 返回 `None` 表示**状态未知**，由调用方回落到 `idle`。不猜 —— 猜错方向会让
/// 「在等你」的宠物显示成「在干活」，那正好是这个挂件要解决的问题的反面。
fn read_tail_state(path: &Path, len: u64) -> (Option<String>, Option<String>) {
    let Ok(mut file) = File::open(path) else {
        return (None, None);
    };

    let take = MAX_TAIL_BYTES.min(len);
    let from = len.saturating_sub(take);
    if file.seek(SeekFrom::Start(from)).is_err() {
        return (None, None);
    }

    let mut buf = Vec::with_capacity(take as usize);
    if file.take(take).read_to_end(&mut buf).is_err() {
        return (None, None);
    }

    // 从中间开始读必然切断第一行，丢掉它。从 0 开始时不能丢 ——
    // 那会把小文件唯一的一行也丢掉。
    let text = String::from_utf8_lossy(&buf);
    let mut lines: Vec<&str> = text.lines().collect();
    if from > 0 && !lines.is_empty() {
        lines.remove(0);
    }

    for line in lines.iter().rev() {
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if v.get("type").and_then(Value::as_str) != Some("event_msg") {
            continue;
        }
        let Some(kind) = v.get("payload").and_then(|p| p.get("type")).and_then(Value::as_str)
        else {
            continue;
        };
        match kind {
            "task_complete" => return (Some("idle".into()), None),
            "task_started" => return (Some("working".into()), None),
            _ => continue,
        }
    }

    (None, None)
}
