//! 用量 / 成本，全部从**本地文件**读，不联网、不碰任何凭据。
//!
//! # 为什么不像 TermiPet 那样查官方接口
//!
//! TermiPet（macOS）拿本机登录凭据去请求官方端点，所以能显示统一的剩余额度。
//! 我们刻意不这么做：那要求一个常驻置顶的小挂件去读你的登录凭据并定期对外发请求，
//! 多出「凭据读取」和「网络出口」两个面。本地文件够用到什么程度见下表。
//!
//! # 每个 agent 能给什么（本机实测）
//!
//! | agent | token | 成本 | 账户配额 |
//! | --- | --- | --- | --- |
//! | Claude Code | ✅ 每条 assistant 的 `usage.*` | ❌ transcript 里没有 | ❌ 没有 |
//! | Codex | ✅ `info.total_token_usage.*` 现成累计 | ❌ | ✅ **真实**：`rate_limits.primary` |
//! | OpenClaw | ✅ `inputTokens` / `outputTokens` | ✅ 现成 `estimatedCostUsd` | ❌ |
//! | Hermes | 在 SQLite 里，见下 | 在 SQLite 里 | ❌ |
//!
//! **不内置价目表。** Claude Code 只有 token，要算美元就得在仓库里写一张价目表；
//! 价格会变，而过期的表是**默默显示错数字** —— 一个错的金额比没有金额更坑人。
//! 所以有成本就显示（OpenClaw 自己算好的），没有就只显示 token 并说明原因。
//!
//! **Hermes 暂不读。** 它的数据其实是四个里最全的（`state.db` 的
//! `session_model_usage` 表连 `estimated_cost_usd` 和 `actual_cost_usd` 都分开存），
//! 但读 SQLite 要给项目加 `rusqlite` 依赖（bundled 要从源码编 SQLite，
//! 二进制大约 +1MB）。这是个明确的取舍，不是遗漏 —— 界面上会说清楚。
//!
//! # 一个实测出来的坑：按 requestId 去重
//!
//! Claude Code 的转录里，**一次 API 请求会写多条 `assistant` 行，每条都带
//! 同一份 `usage` 对象**（一条内容块一行）。本机一个转录里 1354 条带 usage 的
//! assistant 行只对应 **613 个** `requestId`，直接累加 `output_tokens` 会虚高
//! **161.7%**。所以必须按 `requestId` 去重。
//!
//! 另外 `model` 为 `<synthetic>` 的要排掉 —— 那是本地合成的消息，没有真实 API 调用。

use crate::agent::Agent;
use serde::Serialize;
use serde_json::Value;
use std::collections::HashSet;
use std::fs::File;
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

// 这里**刻意没有**「每个文件最多读 N 字节」的上限。
//
// 第一版抄了 `discover.rs` 的 `MAX_HEAD_BYTES` 模式，封在 8MB。那是错的，
// 而且错得很隐蔽：本机当前会话的转录有 11.5MB，于是面板显示的 token 只有真实值的
// 约 69%（实测 cache_read 767,420,613 对真实的 1,119,001,795），
// 界面上没有任何迹象说明它被截断了。
//
// `discover.rs` 封顶是对的 —— 它只需要**头部**的 `cwd`。用量需要**全部**行，
// 任何上限都等于默默少算。而「默默显示错数字」正是这个面板刻意不内置价目表
// 时给出的理由，自己在这儿犯一遍就说不过去了。
//
// 代价：一次面板刷新要完整读一遍时间窗内的转录。用 `line.contains` 预筛，
// 绝大多数行不会进 JSON 解析，所以这是顺序 IO 而不是 CPU 瓶颈；而且它只在
// 用户打开设置窗口或点刷新时跑，不在任何热路径上。

#[derive(Clone, Debug, Default, Serialize)]
pub struct Tokens {
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
    /// 总数。**不是** 上面几项相加 —— 各家对「cached 算不算在 input 里」
    /// 的口径不同（Codex 的 `cached_input_tokens` 是 `input_tokens` 的子集），
    /// 所以能拿到官方总数时用官方的，拿不到才自己加。
    pub total: u64,
}

#[derive(Clone, Debug, Serialize)]
pub struct Quota {
    pub used_percent: f64,
    pub window_minutes: u64,
    /// 配额重置的时间点（epoch 毫秒）。前端算「还剩多久」。
    pub resets_at_ms: Option<u128>,
    pub plan: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct AgentUsage {
    pub agent: &'static str,
    pub label: &'static str,
    pub sessions: usize,
    pub tokens: Tokens,
    pub cost_usd: Option<f64>,
    pub quota: Option<Quota>,
    /// 为什么某些格子是空的。**i18n 的键**，不是文案 —— 翻译在前端。
    /// 界面直接把它显示出来，而不是留个空格让人猜是坏了还是没有。
    pub note: Option<&'static str>,
}

impl AgentUsage {
    fn empty(a: Agent, note: Option<&'static str>) -> Self {
        Self {
            agent: a.key(),
            label: a.label(),
            sessions: 0,
            tokens: Tokens::default(),
            cost_usd: None,
            quota: None,
            note,
        }
    }
}

fn recent(path: &Path, window: Duration, now: SystemTime) -> bool {
    let Ok(meta) = std::fs::metadata(path) else {
        return false;
    };
    let Ok(modified) = meta.modified() else {
        return false;
    };
    // 时钟回拨导致的负数年龄当成「新」处理，和 discover 那边一致
    now.duration_since(modified).map(|age| age <= window).unwrap_or(true)
}

// ── Claude Code ──────────────────────────────────────────────

/// 扫转录累加 token。
///
/// 时间窗和会话发现用同一个 —— 「这段时间内的用量」和「这段时间内的会话」
/// 是同一个心智模型，两个窗口只会让人困惑。
pub fn claude_code(projects_dir: &Path, window: Duration) -> AgentUsage {
    let now = SystemTime::now();
    let mut out = AgentUsage::empty(Agent::ClaudeCode, Some("usage.noCostNoQuota"));

    let Ok(projects) = std::fs::read_dir(projects_dir) else {
        return out;
    };

    for project in projects.flatten() {
        let Ok(files) = std::fs::read_dir(project.path()) else {
            continue;
        };
        for entry in files.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
                continue;
            }
            if !recent(&path, window, now) {
                continue;
            }
            if sum_claude_file(&path, &mut out.tokens) {
                out.sessions += 1;
            }
        }
    }

    out.tokens.total =
        out.tokens.input + out.tokens.output + out.tokens.cache_read + out.tokens.cache_write;
    out
}

/// 返回 true 表示这个文件里确实有可计的用量。
fn sum_claude_file(path: &Path, acc: &mut Tokens) -> bool {
    let Ok(file) = File::open(path) else {
        return false;
    };
    let reader = BufReader::new(file);

    // 同一个 requestId 只算一次。见模块文档：一次请求会写多条 assistant 行，
    // 每条都带同一份 usage，不去重会虚高 161.7%（本机实测）。
    let mut seen: HashSet<String> = HashSet::new();
    let mut any = false;

    for line in reader.lines().map_while(Result::ok) {
        // 便宜的预筛：绝大多数行没有 usage，不值得为它们跑一次 JSON 解析
        if !line.contains("\"usage\"") {
            continue;
        }
        let Ok(v) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if v.get("type").and_then(Value::as_str) != Some("assistant") {
            continue;
        }
        let Some(msg) = v.get("message") else { continue };

        // `<synthetic>` 是本地合成的消息，没有真实 API 调用，不该计费也不该计量
        if msg.get("model").and_then(Value::as_str) == Some("<synthetic>") {
            continue;
        }
        let Some(u) = msg.get("usage") else { continue };

        // 没有 requestId 的行按行本身去重不了，只能计入 —— 实测所有带 usage 的
        // assistant 行都有 requestId，所以这条分支基本走不到。
        if let Some(rid) = v.get("requestId").and_then(Value::as_str) {
            if !seen.insert(rid.to_string()) {
                continue;
            }
        }

        let n = |k: &str| u.get(k).and_then(Value::as_u64).unwrap_or(0);
        acc.input += n("input_tokens");
        acc.output += n("output_tokens");
        acc.cache_read += n("cache_read_input_tokens");
        acc.cache_write += n("cache_creation_input_tokens");
        any = true;
    }

    any
}

// ── Codex ────────────────────────────────────────────────────

/// 扫 rollout 累加 token，并取出**真实账户配额**。
///
/// 每个 rollout 里的 `event_msg/token_count` 带 `info.total_token_usage`，
/// 那是**该会话的累计值**（不是增量），所以每个文件只取**最后一条**再跨文件相加。
/// 逐条累加会把同一个会话数十次地重复计入。
///
/// `rate_limits` 是**账户级**的，跨会话共享，所以取时间上最新的那一份而不是相加。
pub fn codex(sessions_dir: &Path, window: Duration) -> AgentUsage {
    let now = SystemTime::now();
    let mut out = AgentUsage::empty(Agent::Codex, Some("usage.noCost"));
    let mut newest_quota: Option<(u128, Quota)> = None;

    let mut files: Vec<PathBuf> = Vec::new();
    collect_rollouts(sessions_dir, window, now, &mut files, 0);

    for path in files {
        let Some((tokens, quota, mtime)) = last_token_count(&path) else {
            continue;
        };
        out.sessions += 1;
        out.tokens.input += tokens.input;
        out.tokens.output += tokens.output;
        out.tokens.cache_read += tokens.cache_read;
        out.tokens.total += tokens.total;

        if let Some(q) = quota {
            if newest_quota.as_ref().map(|(t, _)| mtime > *t).unwrap_or(true) {
                newest_quota = Some((mtime, q));
            }
        }
    }

    out.quota = newest_quota.map(|(_, q)| q);
    out
}

const MAX_DEPTH: usize = 4;

fn collect_rollouts(
    dir: &Path,
    window: Duration,
    now: SystemTime,
    out: &mut Vec<PathBuf>,
    depth: usize,
) {
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
            collect_rollouts(&path, window, now, out, depth + 1);
            continue;
        }
        let is_rollout = path
            .file_name()
            .and_then(|n| n.to_str())
            .map(|n| n.starts_with("rollout-") && n.ends_with(".jsonl"))
            .unwrap_or(false);
        if is_rollout && recent(&path, window, now) {
            out.push(path);
        }
    }
}

/// 从尾部往前找最后一条 `token_count`。
///
/// 回读而不是顺读整个文件：rollout 本机有 400KB+，而我们只要最后一条累计值。
/// 回读窗口开到 512KB —— 比 codex.rs 判状态用的 64KB 大得多，因为
/// `token_count` 事件之后可能还跟着很长的 `response_item`。
const TAIL_BYTES: u64 = 512 * 1024;

fn last_token_count(path: &Path) -> Option<(Tokens, Option<Quota>, u128)> {
    let meta = std::fs::metadata(path).ok()?;
    let len = meta.len();
    let mtime = meta
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_millis();

    let mut file = File::open(path).ok()?;
    let take = TAIL_BYTES.min(len);
    let from = len.saturating_sub(take);
    file.seek(SeekFrom::Start(from)).ok()?;
    let mut buf = Vec::with_capacity(take as usize);
    file.take(take).read_to_end(&mut buf).ok()?;

    let text = String::from_utf8_lossy(&buf);
    let mut lines: Vec<&str> = text.lines().collect();
    // 从中间开始读必然切断第一行。从 0 开始时不能丢 —— 那会丢掉小文件唯一的一行。
    if from > 0 && !lines.is_empty() {
        lines.remove(0);
    }

    for line in lines.iter().rev() {
        if !line.contains("token_count") {
            continue;
        }
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let p = v.get("payload")?;
        if p.get("type").and_then(Value::as_str) != Some("token_count") {
            continue;
        }

        let info = p.get("info");
        let tu = info.and_then(|i| i.get("total_token_usage"));
        let n = |k: &str| {
            tu.and_then(|t| t.get(k)).and_then(Value::as_u64).unwrap_or(0)
        };
        let tokens = Tokens {
            input: n("input_tokens"),
            output: n("output_tokens"),
            // Codex 的 cached_input_tokens 是 input_tokens 的**子集**，
            // 所以只填进 cache_read 用于展示，不再加进 total
            cache_read: n("cached_input_tokens"),
            cache_write: n("cache_write_input_tokens"),
            // 用官方给的 total，别自己加 —— 口径由它定
            total: n("total_tokens"),
        };

        return Some((tokens, parse_quota(p), mtime));
    }
    None
}

/// `rate_limits.primary` → 我们的 `Quota`。
///
/// 本机实测这些字段经常整块是 null（用 API key / 自建 provider 时没有套餐配额），
/// 所以拿不到就返回 None 而不是填零 —— 显示「0% 已用」是在撒谎。
fn parse_quota(payload: &Value) -> Option<Quota> {
    let rl = payload.get("rate_limits")?;
    let primary = rl.get("primary")?;
    let used = primary.get("used_percent").and_then(Value::as_f64)?;
    Some(Quota {
        used_percent: used,
        window_minutes: primary
            .get("window_minutes")
            .and_then(Value::as_u64)
            .unwrap_or(0),
        // resets_at 是**秒**级 epoch，我们统一用毫秒
        resets_at_ms: primary
            .get("resets_at")
            .and_then(Value::as_u64)
            .map(|s| s as u128 * 1000),
        plan: rl
            .get("plan_type")
            .and_then(Value::as_str)
            .map(str::to_string),
    })
}

// ── OpenClaw ─────────────────────────────────────────────────

/// `~/.openclaw/agents/<agent>/sessions/sessions.json`。
///
/// 单个 JSON 对象，键是 `agent:main:<入口>:<id>`。每条自带
/// `inputTokens` / `outputTokens` / `totalTokens` / `estimatedCostUsd` ——
/// 是四个 agent 里唯一现成给出成本的，所以直接用它的数，不自己算。
///
/// **不按时间窗过滤**：这个文件是所有会话共一份，没法按会话过滤 mtime。
/// 它的条目自带 `lastInteractionAt`，用那个过滤。
pub fn openclaw(home: &Path, window: Duration) -> AgentUsage {
    let mut out = AgentUsage::empty(Agent::OpenClaw, Some("usage.noQuota"));

    let now_ms = SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let window_ms = window.as_millis();

    let mut cost = 0.0f64;
    let mut any_cost = false;

    // agents/*/sessions/sessions.json —— agent 目录名不固定（本机是 `main`）
    let agents = home.join("agents");
    let Ok(dirs) = std::fs::read_dir(&agents) else {
        return out;
    };

    for d in dirs.flatten() {
        let path = d.path().join("sessions").join("sessions.json");
        let Ok(raw) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Ok(v) = serde_json::from_str::<Value>(&raw) else {
            continue;
        };
        let Some(map) = v.as_object() else { continue };

        for entry in map.values() {
            // 时间窗按每条自己的 lastInteractionAt。缺这个字段的条目
            // （本机有几条）无法定位在时间上，跳过而不是当成「刚刚」。
            let Some(last) = entry.get("lastInteractionAt").and_then(Value::as_u64) else {
                continue;
            };
            if now_ms.saturating_sub(last as u128) > window_ms {
                continue;
            }

            out.sessions += 1;
            let n = |k: &str| entry.get(k).and_then(Value::as_u64).unwrap_or(0);
            out.tokens.input += n("inputTokens");
            out.tokens.output += n("outputTokens");
            out.tokens.total += n("totalTokens");

            if let Some(c) = entry.get("estimatedCostUsd").and_then(Value::as_f64) {
                cost += c;
                any_cost = true;
            }
        }
    }

    out.cost_usd = if any_cost { Some(cost) } else { None };
    out
}

// ── Hermes ───────────────────────────────────────────────────

/// Hermes 的用量在 SQLite 里，暂不读 —— 见模块文档的取舍说明。
///
/// 返回一条带说明的空记录而不是干脆不出现在列表里：用户点名要看 Hermes 的用量，
/// 界面上什么都不显示会让人以为功能坏了。
pub fn hermes() -> AgentUsage {
    AgentUsage::empty(Agent::Hermes, Some("usage.needsSqlite"))
}
