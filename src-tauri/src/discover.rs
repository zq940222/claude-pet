//! 从本地转录文件发现已经在跑的会话。
//!
//! 解决的问题：挂件只知道「启动之后推过 hook 事件」的会话，所以开机自启后
//! 或挂件重启后，已有会话是隐形的，要等下一个事件才冒出来。
//!
//! 数据源（本机实测）：
//!
//! ```text
//! ~/.claude/projects/<编码后的 cwd>/<session_id>.jsonl
//! ```
//!
//! - 文件名 = session_id，条目里的 `sessionId` 与之一致
//! - 条目里的 `cwd` 字段给出准确的工作目录
//! - 文件 mtime = 最后活动时间
//!
//! **不从目录名反解 cwd**：目录名把 `:` 和 `\` 都替换成了 `-`，而项目名本身
//! 也可能含 `-`（如 `Claude-Code-Short-Drama-Studios`），反解无法唯一还原。
//!
//! 也**没有**用项目目录下的 `sessions-index.json`：它确实直接带
//! `sessionId` / `projectPath` / `fileMtime`，但本机只有 21/54 个项目目录有它，
//! 覆盖率不够，回退路径无论如何都得写。而我们本来就先按 mtime 过滤、只读那几个
//! 近期文件（读几行很便宜），索引省下的开销不抵多一条代码路径的维护成本。

use serde_json::Value;
use std::fs::File;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

// 时间窗由调用方传入，默认值和范围在 `persist::Prefs`（设置窗口可改）。
// 必须设窗口：本机有 651 个历史转录，不限制会一次冒出几十个早已关掉的会话。

/// 每个文件最多读这么多字节找 `cwd`。实测 `cwd` 稳定出现在第 3–5 行，
/// 但单行可能很大（大的工具输出），所以按字节封顶而不是只按行数封顶 ——
/// 最大的转录有 30MB，不设上限会把它整个读进来。
const MAX_HEAD_BYTES: u64 = 256 * 1024;
const MAX_HEAD_LINES: usize = 40;

#[derive(Debug)]
pub struct Discovered {
    pub session_id: String,
    pub cwd: String,
    pub mtime_ms: u128,
}

/// `~/.claude/projects`，尊重 `CLAUDE_CONFIG_DIR` 覆盖。
///
/// 这个环境变量是真会被用的 —— 用户 settings.json 里的 statusLine 命令就写着
/// `${CLAUDE_CONFIG_DIR:-$HOME/.claude}`，忽略它会在那些人机器上扫错目录。
pub fn projects_dir(home: Option<PathBuf>) -> Option<PathBuf> {
    if let Ok(dir) = std::env::var("CLAUDE_CONFIG_DIR") {
        let trimmed = dir.trim();
        if !trimmed.is_empty() {
            return Some(PathBuf::from(trimmed).join("projects"));
        }
    }
    home.map(|h| h.join(".claude").join("projects"))
}

fn to_ms(t: SystemTime) -> u128 {
    t.duration_since(UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0)
}

/// 扫描 `projects_dir`，返回时间窗内的会话。
///
/// 只取项目目录下**直接**的 `*.jsonl`。subagent 的转录在
/// `<project>/<session_id>/subagents/agent-*.jsonl`，深度更深，因此这条规则
/// 天然把它们排除掉 —— 否则一个会话会显示成好几只宠物。
pub fn scan(projects_dir: &Path, window: Duration) -> Vec<Discovered> {
    let now = SystemTime::now();
    let mut found = Vec::new();

    let Ok(projects) = std::fs::read_dir(projects_dir) else {
        return found;
    };

    for project in projects.flatten() {
        if !project.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            continue;
        }
        let Ok(files) = std::fs::read_dir(project.path()) else {
            continue;
        };

        for entry in files.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
                continue;
            }

            let Ok(meta) = entry.metadata() else { continue };
            if !meta.is_file() {
                continue;
            }

            // 先按 mtime 过滤，再决定要不要打开文件读内容 ——
            // 651 个转录里通常只有几个落在窗口内，这一步把 IO 降到最低。
            let Ok(modified) = meta.modified() else { continue };
            let too_old = now
                .duration_since(modified)
                .map(|age| age > window)
                .unwrap_or(false); // 时钟回拨导致的负数年龄，当成「新」处理
            if too_old {
                continue;
            }

            let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };

            if let Some(d) = read_head(&path, stem, to_ms(modified)) {
                found.push(d);
            }
        }
    }

    found
}

fn read_head(path: &Path, stem: &str, mtime_ms: u128) -> Option<Discovered> {
    let file = File::open(path).ok()?;
    let reader = BufReader::new(file.take(MAX_HEAD_BYTES));

    let mut cwd: Option<String> = None;
    let mut session_id: Option<String> = None;

    for line in reader.lines().take(MAX_HEAD_LINES).map_while(Result::ok) {
        // 头几行常是 queue-operation 之类不带 cwd 的条目，解析失败或缺字段都直接跳过。
        // 被 MAX_HEAD_BYTES 截断的最后一行也会在这里解析失败，正常忽略。
        let Ok(v) = serde_json::from_str::<Value>(&line) else {
            continue;
        };

        // 侧链条目属于 subagent，不该单独算一只宠物。
        // 正常情况下按目录深度已经排除了，这里是第二道保险。
        if v.get("isSidechain").and_then(Value::as_bool) == Some(true) {
            return None;
        }

        if session_id.is_none() {
            session_id = v.get("sessionId").and_then(Value::as_str).map(str::to_string);
        }
        if cwd.is_none() {
            cwd = v.get("cwd").and_then(Value::as_str).map(str::to_string);
        }
        if cwd.is_some() && session_id.is_some() {
            break;
        }
    }

    // 找不到 cwd 就放弃这个文件。宁可不显示，也不要把宠物放进错误的工作空间 ——
    // 猜一个 cwd 比没有更糟。
    let cwd = cwd?;

    // 内容里的 sessionId 是权威值，缺失时退回文件名。
    Some(Discovered {
        session_id: session_id.unwrap_or_else(|| stem.to_string()),
        cwd,
        mtime_ms,
    })
}
