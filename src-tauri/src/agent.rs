//! 挂件支持哪些 agent，以及每个 agent 实际能提供什么。
//!
//! # 三种形态，不是一种
//!
//! 加 Codex / Hermes / OpenClaw 时最先发现的事：**它们不是同一种东西**。
//! 硬把三个都做成「一个会话一只宠物」会产出看着能用、实际在骗人的界面。
//! 本机实测的可得性：
//!
//! | agent | 形态 | 「在等我」信号 | cwd / 项目 |
//! | --- | --- | --- | --- |
//! | Claude Code | 交互式、按项目 | ✅ hook 实时推送 | ✅ 转录里就有 |
//! | Codex | 交互式、按项目 | ✅ `task_started` / `task_complete` | ✅ `session_meta.cwd` |
//! | Hermes | gateway + 20 个聊天平台 | ❌ 无实时状态 | ❌ 22 条会话里只有 2 条有 cwd |
//! | OpenClaw | gateway + 聊天入口 + cron | ❌ `status` 只是运行结果 | ❌ 恒为 gateway 自己的工作区 |
//!
//! 所以分两类处理：
//!
//! - **Session 类**（Claude Code、Codex）—— 和现在完全同构：按 cwd 分工作空间，
//!   每个会话一只宠物，有真实的 working / waiting / idle 状态。
//! - **Gateway 类**（Hermes、OpenClaw）—— 每个 agent **一只**宠物，显示
//!   「在跑 / 没跑」和活跃 agent 数。它们没有项目概念，硬塞进某个项目的
//!   工作空间只会让人以为那个项目里有东西在跑。
//!
//! 这个不对称是**数据决定的，不是偷懒**。Hermes 的 `sessions` 表确实有 `cwd`
//! 和 `git_repo_root` 列，但本机 22 条里只有 2 条非空、16 条 `ended_at` 永远是
//! null（CLI 不可靠地关闭会话）—— 拿它当会话状态源会得到一堆永远「在跑」、
//! 永远归不进项目的僵尸宠物。
//!
//! # 为什么只有 Claude Code 用 hook
//!
//! - **Codex** 有 `notify`，但那是 `config.toml` 里的**单个**槽位（一个数组，
//!   不是列表）。本机那格已经被 OpenAI 自己的 `codex-computer-use.exe` 占了，
//!   抢过来会弄坏用户已有的功能。所以 Codex 只能**轮询** rollout 文件尾部。
//! - **Hermes / OpenClaw** 是常驻 gateway，本来就没有「一次会话结束」这种
//!   适合推事件的时机。

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Agent {
    ClaudeCode,
    Codex,
    Hermes,
    OpenClaw,
}

/// 稳定的字符串键。落盘和发给前端都用它，**不要**改动已有的值 ——
/// `prefs.json` 里存的是这些字符串。
impl Agent {
    pub const ALL: &'static [Agent] =
        &[Agent::ClaudeCode, Agent::Codex, Agent::Hermes, Agent::OpenClaw];

    pub fn key(self) -> &'static str {
        match self {
            Agent::ClaudeCode => "claude-code",
            Agent::Codex => "codex",
            Agent::Hermes => "hermes",
            Agent::OpenClaw => "openclaw",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|a| a.key() == s)
    }

    /// 界面上显示的名字。刻意用各家自己的写法（大小写照抄官方）。
    pub fn label(self) -> &'static str {
        match self {
            Agent::ClaudeCode => "Claude Code",
            Agent::Codex => "Codex",
            Agent::Hermes => "Hermes",
            Agent::OpenClaw => "OpenClaw",
        }
    }

    /// 宠物身上的角标。一个字符 —— 折叠态每只宠物只有几个像素宽。
    pub fn badge(self) -> &'static str {
        match self {
            Agent::ClaudeCode => "C",
            Agent::Codex => "X",
            Agent::Hermes => "H",
            Agent::OpenClaw => "O",
        }
    }

    /// 这个 agent 是「一个会话一只宠物」还是「整个 agent 一只宠物」。
    pub fn is_gateway(self) -> bool {
        matches!(self, Agent::Hermes | Agent::OpenClaw)
    }
}

/// 一次发现的结果。四个 agent 的扫描器都产出这个。
#[derive(Debug)]
pub struct Discovered {
    pub agent: Agent,
    pub session_id: String,
    /// 工作目录。Gateway 类是**空串** —— 它们没有项目概念，双击跳回编辑器
    /// 也就自然被禁用（前端靠 `cwd` 是否为空判断能不能跳）。
    pub cwd: String,
    /// 工作空间名。`None` 表示从 `cwd` 末段推导（Session 类都这样）。
    /// Gateway 类填 agent 自己的名字，否则会被塞进某个项目的工作空间里，
    /// 让人以为那个项目有东西在跑。
    pub project: Option<String>,
    pub mtime_ms: u128,
    /// 扫描时就能确定的状态。
    ///
    /// Claude Code 是 `None` —— 转录能证明会话存在，但**证明不了**它此刻在干活
    /// 还是在等你（最后一条是 assistant 消息可能意味着刚回完话，也可能意味着
    /// 正在写下一条）。留空让 `hook` 事件几秒内给出真相。
    ///
    /// Codex 是 `Some` —— rollout 尾部的 `task_started` / `task_complete`
    /// 是明确的边界事件，和 Claude Code 的 `UserPromptSubmit` / `Stop` 同义。
    pub state: Option<String>,
    /// 状态旁边那行说明。`None` 时由调用方填「已恢复」之类的兜底文案。
    pub detail: Option<String>,
}
