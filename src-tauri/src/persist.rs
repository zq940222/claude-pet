//! 小块本地状态的落盘：会话缓存 + 窗口锚点。
//!
//! 都放在 `app_config_dir()`（Windows 上是 `%APPDATA%\com.opsmateai.claude-pet\`）。
//!
//! 一律「读不出来就当没有」：这些是缓存而不是真相来源，损坏、版本变更、
//! 手工乱改都必须静默回落到空，绝不能让挂件起不来。

use crate::Session;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

use tauri::{AppHandle, Manager};

/// 缓存格式版本。改了字段就加一 —— 旧文件会被整体丢弃，
/// 而不是被 serde 误解析成半对半错的状态。
///
/// v2: `Session` 增加了 `cwd`（跳回 IDE 需要完整路径）。v1 的缓存里没有这个
/// 字段，加载后会是空串、双击跳不动，所以宁可整份丢掉重新扫。
const CACHE_VERSION: u32 = 2;

const SESSIONS_FILE: &str = "sessions.json";
const ANCHOR_FILE: &str = "window-anchor.json";

pub fn config_path(app: &AppHandle, file: &str) -> Option<PathBuf> {
    app.path().app_config_dir().ok().map(|d| d.join(file))
}

fn write_atomic(path: PathBuf, contents: &str) {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    // 先写临时文件再 rename：直接覆写的话，进程正好在写一半时被杀
    // （挂件是靠托盘退出或被 kill 的，这不是罕见情况）会留下截断的 JSON。
    let tmp = path.with_extension("tmp");
    if std::fs::write(&tmp, contents).is_ok() {
        if std::fs::rename(&tmp, &path).is_err() {
            // Windows 上目标已存在时 rename 可能失败，退回直接覆写
            let _ = std::fs::write(&path, contents);
            let _ = std::fs::remove_file(&tmp);
        }
    }
}

// ── 会话缓存 ─────────────────────────────────────────────────

#[derive(Serialize, Deserialize)]
struct SessionCache {
    version: u32,
    sessions: HashMap<String, Session>,
}

/// 序列化成字符串。**不**直接落盘 —— 调用方拿返回值和上次写的比较，
/// 只在变化时才真写，和窗口锚点用的是同一套「变了才写」策略。
pub fn encode_sessions(sessions: &HashMap<String, Session>) -> Option<String> {
    serde_json::to_string(&SessionCache {
        version: CACHE_VERSION,
        // 这里必须克隆：SessionCache 拥有数据才好序列化，
        // 而会话数量是个位数，克隆成本可以忽略。
        sessions: sessions.clone(),
    })
    .ok()
}

pub fn write_sessions(app: &AppHandle, encoded: &str) {
    if let Some(path) = config_path(app, SESSIONS_FILE) {
        write_atomic(path, encoded);
    }
}

/// 读回会话缓存，并按时间窗过滤。
///
/// 过滤依据是每个会话自己的 `updated_ms` 而不是文件的保存时间：挂件可能
/// 关了很久，缓存里既有几分钟前还活跃的会话，也有几小时前就停了的，
/// 不能因为文件是「刚保存的」就把后者当成当前状态展示。
pub fn load_sessions(
    app: &AppHandle,
    window: Duration,
    now_ms: u128,
) -> HashMap<String, Session> {
    let empty = HashMap::new();

    let Some(path) = config_path(app, SESSIONS_FILE) else {
        return empty;
    };
    let Ok(raw) = std::fs::read_to_string(&path) else {
        return empty;
    };
    let Ok(cache) = serde_json::from_str::<SessionCache>(&raw) else {
        eprintln!("[claude-pet] session cache unreadable, starting empty");
        return empty;
    };
    if cache.version != CACHE_VERSION {
        eprintln!(
            "[claude-pet] session cache version {} != {CACHE_VERSION}, discarding",
            cache.version
        );
        return empty;
    }

    let window_ms = window.as_millis();
    cache
        .sessions
        .into_iter()
        // saturating_sub 兜住时钟回拨导致 updated_ms > now 的情况
        .filter(|(_, s)| now_ms.saturating_sub(s.updated_ms) <= window_ms)
        .collect()
}

// ── 用户偏好 ─────────────────────────────────────────────────

/// 用户偏好。
///
/// 这里**刻意不用**会话缓存那套 `version` 门禁：偏好是用户的意图，加字段时
/// 必须让旧文件继续可读，不能整份丢掉（丢掉等于把用户的设置悄悄重置）。
/// 所以每个字段都带 `#[serde(default)]`，新增字段对旧文件就是取默认值。
///
/// 会话缓存反过来 —— 它是可丢弃的派生数据，宁可整份丢掉也不要半对半错。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Prefs {
    /// 静音。等待态提示音的总开关。
    #[serde(default)]
    pub muted: bool,
    /// 提示音用的 Windows 声音方案事件名，见 `sound::AVAILABLE`。
    #[serde(default = "default_sound")]
    pub sound: String,
    /// 会话自动发现的时间窗（分钟）。只认这段时间内动过的转录 ——
    /// 本机有 651 个历史转录，窗口太大会一次冒出几十个早已关掉的会话。
    #[serde(default = "default_window_minutes")]
    pub discover_window_minutes: u64,
    /// 双击宠物时用哪个编辑器。`"auto"` 表示按 `editor::EDITORS` 的顺序
    /// 取第一个装了的；指定具体某个时**不做回落** —— 用户明确选了
    /// VS Code，静默换成 Cursor 是在骗人。
    #[serde(default = "default_editor")]
    pub editor: String,
}

fn default_editor() -> String {
    "auto".to_string()
}

fn default_sound() -> String {
    crate::sound::DEFAULT_ALIAS.to_string()
}

fn default_window_minutes() -> u64 {
    30
}

/// 时间窗的合法范围。上限不是随便定的：本机 651 个转录里，窗口开到
/// 一天以上就会把大量早已关掉的会话拉回来，挂件反而没法看。
pub const WINDOW_MIN: u64 = 1;
pub const WINDOW_MAX: u64 = 1440;

impl Default for Prefs {
    fn default() -> Self {
        Self {
            muted: false,
            sound: default_sound(),
            discover_window_minutes: default_window_minutes(),
            editor: default_editor(),
        }
    }
}

impl Prefs {
    pub fn discover_window(&self) -> Duration {
        Duration::from_secs(self.discover_window_minutes.clamp(WINDOW_MIN, WINDOW_MAX) * 60)
    }

    /// 收敛非法值。前端和手改的文件都可能给出越界的东西。
    pub fn sanitise(&mut self) {
        if !crate::sound::AVAILABLE.contains(&self.sound.as_str()) {
            self.sound = default_sound();
        }
        self.discover_window_minutes = self
            .discover_window_minutes
            .clamp(WINDOW_MIN, WINDOW_MAX);
        if self.editor != "auto"
            && !crate::editor::EDITORS.iter().any(|e| e.key == self.editor)
        {
            self.editor = default_editor();
        }
    }
}

const PREFS_FILE: &str = "prefs.json";

pub fn load_prefs(app: &AppHandle) -> Prefs {
    let Some(path) = config_path(app, PREFS_FILE) else {
        return Prefs::default();
    };
    let Ok(raw) = std::fs::read_to_string(path) else {
        return Prefs::default();
    };
    let mut prefs = match serde_json::from_str::<Prefs>(&raw) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("[claude-pet] prefs unreadable ({e}), using defaults");
            return Prefs::default();
        }
    };

    // 校验声音别名。prefs.json 是给人手改的，打错字的话 PlaySoundW 会
    // 静默不响（我们刻意带了 SND_NODEFAULT），用户只会以为提示音坏了。
    if !crate::sound::AVAILABLE.contains(&prefs.sound.as_str()) {
        eprintln!(
            "[claude-pet] unknown sound \"{}\", falling back to {} (valid: {})",
            prefs.sound,
            crate::sound::DEFAULT_ALIAS,
            crate::sound::AVAILABLE.join(", ")
        );
        prefs.sound = default_sound();
    }

    let before = prefs.discover_window_minutes;
    prefs.sanitise();
    if before != prefs.discover_window_minutes {
        eprintln!(
            "[claude-pet] discover window {before} out of range, clamped to {}",
            prefs.discover_window_minutes
        );
    }

    prefs
}

pub fn save_prefs(app: &AppHandle, prefs: &Prefs) {
    let Some(path) = config_path(app, PREFS_FILE) else {
        return;
    };
    if let Ok(s) = serde_json::to_string_pretty(prefs) {
        write_atomic(path, &s);
    }
}

// ── 窗口锚点 ─────────────────────────────────────────────────

/// 存右下角而不是左上角 —— 和 `resize_pet` 的锚定方向保持一致。
#[derive(Serialize, Deserialize)]
pub struct SavedAnchor {
    pub right: i32,
    pub bottom: i32,
}

pub fn write_anchor(app: &AppHandle, right: i32, bottom: i32) {
    let Some(path) = config_path(app, ANCHOR_FILE) else {
        return;
    };
    if let Ok(s) = serde_json::to_string(&SavedAnchor { right, bottom }) {
        write_atomic(path, &s);
    }
}

pub fn read_anchor(app: &AppHandle) -> Option<SavedAnchor> {
    let path = config_path(app, ANCHOR_FILE)?;
    let raw = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&raw).ok()
}
