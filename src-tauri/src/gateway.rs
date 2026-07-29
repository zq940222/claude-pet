//! Hermes 和 OpenClaw：常驻 gateway，每个 agent **一只**宠物。
//!
//! # 为什么不做成一个会话一只宠物
//!
//! 试过，数据撑不住（本机实测）：
//!
//! - **Hermes** 的 `sessions` 表确实有 `cwd` / `git_repo_root` / `ended_at` 列，
//!   但 22 条会话里只有 **2 条** `cwd` 非空、**16 条** `ended_at` 永远是 null
//!   （CLI 不可靠地关会话）。拿它当会话源会得到一堆永远「在跑」、永远归不进
//!   项目的僵尸宠物。
//! - **OpenClaw** 是单个 agent 从 20 多个聊天入口进来，`sessions.json` 的键是
//!   `agent:main:telegram:direct:...` / `agent:main:cron:...`，`workspaceDir`
//!   恒为 `~/.openclaw/workspace`（gateway 自己的工作区）。没有项目概念。
//!   `status` 只有 `done` / `failed` / `timeout` —— 那是**运行结果**，不是
//!   「此刻在干什么」。
//!
//! # 状态词汇不新造
//!
//! gateway 没在跑时**不产出宠物**，而不是加一个 `offline` 状态。理由：没在跑
//! 就没什么要盯的，一只常驻的灰色宠物只是在占地方。在跑时复用现有词汇：
//!
//! | 情况 | 状态 |
//! | --- | --- |
//! | 在跑，有 agent 正在干活 | `working` |
//! | 在跑，空闲 | `idle` |
//!
//! # 判活方式两个 agent 不一样，因为它们给的东西不一样
//!
//! - **OpenClaw** 在 `openclaw.json` 里配了 `gateway.port`（本机 18789，
//!   `bind: loopback`），所以裸 TCP 连一下就是确定性的判活。
//!   **刻意不读同一段里的 `gateway.auth.token`** —— 建立 TCP 连接不需要认证，
//!   而把用户的 token 读进内存是完全多余的暴露面。
//! - **Hermes** 没有端口配置，但 `gateway_state.json` 有 `pid` 和
//!   `gateway_state`，所以查那个 pid 还在不在。
//!
//! pid 判活有个已知弱点：pid 会被系统回收，理论上可能有另一个进程占了同一个
//! 号，导致宠物显示「在跑」而其实没在跑。所以额外要求 `gateway_state` 字段
//! 自己也说 `running`。代价是可能虚报，不是漏报 —— 对一只状态挂件，
//! 虚报「在跑」比虚报「没跑」轻。

use crate::agent::{Agent, Discovered};
use crate::i18n::Lang;
use serde_json::Value;
use std::net::{Ipv4Addr, SocketAddr, TcpStream};
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// TCP 判活的超时。探的是 loopback，正常情况下是微秒级；
/// 给 300ms 是为了不在系统卡顿时误判成「没在跑」。
const PROBE_TIMEOUT: Duration = Duration::from_millis(300);

fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

fn read_json(path: &PathBuf) -> Option<Value> {
    let raw = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&raw).ok()
}

// ── Hermes ───────────────────────────────────────────────────

/// `%LOCALAPPDATA%\hermes`，尊重 `HERMES_HOME` 覆盖。
///
/// 那个环境变量是真在用的 —— `gateway.pid` 里就记着 `hermes_home` 字段。
pub fn hermes_home(local_app_data: Option<PathBuf>) -> Option<PathBuf> {
    if let Ok(dir) = std::env::var("HERMES_HOME") {
        let trimmed = dir.trim();
        if !trimmed.is_empty() {
            return Some(PathBuf::from(trimmed));
        }
    }
    local_app_data.map(|d| d.join("hermes"))
}

pub fn hermes_probe(home: Option<PathBuf>, lang: Lang) -> Option<Discovered> {
    let home = home?;
    let v = read_json(&home.join("gateway_state.json"))?;

    // 两个条件都要满足才算在跑：字段自己说 running，且那个 pid 还活着。
    // 只看字段的话，gateway 被 kill -9 之后文件永远停在 "running"。
    if v.get("gateway_state").and_then(Value::as_str) != Some("running") {
        return None;
    }
    let pid = v.get("pid").and_then(Value::as_u64)?;
    if !pid_alive(pid as u32) {
        return None;
    }

    let active = v.get("active_agents").and_then(Value::as_u64).unwrap_or(0);

    // 平台连接状态。只列出真的连上的 —— 配了但没连上的平台对
    // 「现在能不能找到它」这个问题是噪音。
    let mut connected: Vec<&str> = Vec::new();
    if let Some(platforms) = v.get("platforms").and_then(Value::as_object) {
        for (name, p) in platforms {
            if p.get("state").and_then(Value::as_str) == Some("connected") {
                connected.push(name.as_str());
            }
        }
    }
    connected.sort_unstable();

    Some(Discovered {
        agent: Agent::Hermes,
        session_id: gateway_id(Agent::Hermes),
        cwd: String::new(),
        project: Some(Agent::Hermes.label().to_string()),
        mtime_ms: now_ms(),
        state: Some(if active > 0 { "working".into() } else { "idle".into() }),
        detail: Some(describe(lang, active, &connected)),
    })
}

// ── OpenClaw ─────────────────────────────────────────────────

/// `~/.openclaw`，尊重 `OPENCLAW_CONFIG_DIR` 覆盖。
pub fn openclaw_home(home: Option<PathBuf>) -> Option<PathBuf> {
    if let Ok(dir) = std::env::var("OPENCLAW_CONFIG_DIR") {
        let trimmed = dir.trim();
        if !trimmed.is_empty() {
            return Some(PathBuf::from(trimmed));
        }
    }
    home.map(|h| h.join(".openclaw"))
}

pub fn openclaw_probe(home: Option<PathBuf>, lang: Lang) -> Option<Discovered> {
    let home = home?;
    let v = read_json(&home.join("openclaw.json"))?;

    // 只取 port。同一个 gateway 段里还有 auth.token，刻意不碰。
    let port = v
        .get("gateway")
        .and_then(|g| g.get("port"))
        .and_then(Value::as_u64)
        .filter(|p| *p > 0 && *p < 65536)? as u16;

    if !port_open(port) {
        return None;
    }

    Some(Discovered {
        agent: Agent::OpenClaw,
        session_id: gateway_id(Agent::OpenClaw),
        cwd: String::new(),
        project: Some(Agent::OpenClaw.label().to_string()),
        mtime_ms: now_ms(),
        // 拿不到活跃 agent 数（那在 sqlite 里，而且实测本机的活动基本是 cron），
        // 所以只报「在跑」，不假装知道它忙不忙。
        state: Some("idle".into()),
        detail: Some(format!("{} · :{port}", crate::i18n::gateway_up(lang))),
    })
}

// ── 共用 ─────────────────────────────────────────────────────

/// gateway 宠物的 id。**必须稳定** —— `first_seen` 靠它保持宠物顺序不乱跳，
/// 每次探测换一个 id 会让宠物每几秒重新出现一次。
fn gateway_id(a: Agent) -> String {
    format!("gateway:{}", a.key())
}

fn describe(lang: Lang, active: u64, connected: &[&str]) -> String {
    let head = if active > 0 {
        crate::i18n::gateway_active(lang, active)
    } else {
        crate::i18n::gateway_up(lang).to_string()
    };
    if connected.is_empty() {
        head
    } else {
        format!("{head} · {}", connected.join(" / "))
    }
}

/// loopback 上某个端口是否有人在听。
fn port_open(port: u16) -> bool {
    let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    TcpStream::connect_timeout(&addr, PROBE_TIMEOUT).is_ok()
}

/// 那个 pid 还在不在。
///
/// 用 `PROCESS_QUERY_LIMITED_INFORMATION`（不是 `PROCESS_QUERY_INFORMATION`）:
/// 前者对不同完整性级别的进程也能打开，后者会在这里频繁失败并被我们误判成
/// 「没在跑」。
#[cfg(windows)]
fn pid_alive(pid: u32) -> bool {
    const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;
    const STILL_ACTIVE: u32 = 259;

    extern "system" {
        fn OpenProcess(access: u32, inherit: i32, pid: u32) -> isize;
        fn GetExitCodeProcess(handle: isize, code: *mut u32) -> i32;
        fn CloseHandle(handle: isize) -> i32;
    }

    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if h == 0 {
            return false;
        }
        let mut code: u32 = 0;
        let ok = GetExitCodeProcess(h, &mut code) != 0;
        CloseHandle(h);
        // 拿不到退出码时保守当成没在跑 —— 宁可漏报一只 gateway 宠物，
        // 也不要显示一只指向已死进程的
        ok && code == STILL_ACTIVE
    }
}

#[cfg(not(windows))]
fn pid_alive(_pid: u32) -> bool {
    false
}
