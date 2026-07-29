// Claude Pet —— 常驻置顶的 Claude Code 状态挂件
//
// 数据流：Claude Code 的 hook（type: "http"）POST 事件 JSON 到 127.0.0.1:47800，
// 这里解析成会话树后 emit 给 WebView 渲染。
//
// 为什么用 HTTP 而不是状态文件：官方文档明确「连接失败或超时 = 非阻塞错误，执行继续」，
// 所以挂件没开的时候 hook 静默失败，完全不影响 Claude Code 干活。
//
// 模型：一个会话 = 一只宠物；一个项目（cwd 末段）= 一个工作空间，容纳该项目下的所有会话。

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod agent;
mod codex;
mod discover;
mod editor;
mod gateway;
mod hooks;
mod i18n;
mod persist;
mod sound;

use agent::Agent;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{
    AppHandle, Emitter, LogicalSize, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder,
};
use tauri_plugin_autostart::{ManagerExt as AutostartExt, MacosLauncher};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};

/// 挂件监听端口。改这里的话记得同步改 ~/.claude/settings.json 里的 hook url。
const PORT: u16 = 47800;

/// 权限请求最多挂住多久等你点。
///
/// 必须**短于** hook 配置里的 `timeout`（我们装的是 40 秒），否则 Claude Code
/// 先放弃、我们的回复到得太晚就白挂了。
///
/// 超时后回空 200 = 不给决定，交还给 Claude Code 自己的权限流程（fail-open）。
/// 所以最坏情况是每次卡 30 秒，而不是永久卡死 —— 这也是为什么不敢把它设得更长。
const PERMISSION_WAIT: Duration = Duration::from_secs(30);

/// 仓库地址。前端拿它推出 GitHub API 的 releases 端点做版本检查。
const REPO_URL: &str = "https://github.com/zq940222/claude-pet";

// ── 会话状态 ─────────────────────────────────────────────────

/// 派生 `Deserialize` 是为了跨启动持久化（见 `persist` 模块）——
/// 落盘的就是这个结构，改字段记得同步 `persist` 里的 `CACHE_VERSION`。
#[derive(Clone, Debug, Serialize, Deserialize)]
struct Session {
    /// 哪个 agent 的会话。
    ///
    /// `#[serde(default)]` 是给旧缓存留的余地，但**缓存本身是版本门禁的**
    /// （加这个字段时 `CACHE_VERSION` 已经升到 3），所以实际走不到默认值。
    /// 留着是因为手改缓存文件的人不该因为漏一个字段就把整份缓存作废。
    #[serde(default = "default_agent")]
    agent: Agent,
    /// 完整工作目录。跳回 IDE 要用它 —— `project` 只是末段，拿不回原路径。
    /// Gateway 类的 agent（Hermes / OpenClaw）这里是空串，它们没有项目概念。
    cwd: String,
    project: String,
    state: String,
    detail: String,
    /// 首次出现的时间。用来给宠物图标定一个稳定顺序 ——
    /// HashMap 的迭代顺序是随机的，不排序图标每次刷新都会乱跳。
    first_seen: u128,
    updated_ms: u128,
}

fn default_agent() -> Agent {
    Agent::ClaudeCode
}

#[derive(Clone, Debug, Serialize)]
struct SessionView {
    id: String,
    /// 工作空间内的 1-based 序号，给宠物当显示名
    index: usize,
    state: String,
    detail: String,
    /// 完整路径。前端拿它做 tooltip，也用来判断能不能跳回 IDE。
    cwd: String,
    /// agent 的稳定键（`claude-code` / `codex` / `hermes` / `openclaw`）。
    /// 前端用它选角标和配色。
    agent: &'static str,
    /// 宠物身上那一个字符的角标
    badge: &'static str,
}

#[derive(Clone, Debug, Serialize)]
struct WorkspaceView {
    project: String,
    sessions: Vec<SessionView>,
    /// 该工作空间里最紧急的状态，折叠态给工作空间标签着色用
    worst: String,
}

#[derive(Clone, Debug, Serialize)]
struct AppView {
    workspaces: Vec<WorkspaceView>,
    total: usize,
    waiting: usize,
    /// 最该被关注的会话 id。前端用它做自动选中和自动展开判断。
    focus: Option<String>,
    /// 等着你点允许/拒绝的请求。空数组表示没有。
    pending: Vec<PendingView>,
}

type Store = Arc<Mutex<HashMap<String, Session>>>;

/// 用户偏好。托盘改它、HTTP 线程读它，所以要共享。
type PrefsState = Arc<Mutex<persist::Prefs>>;

/// 一条挂起的权限请求。
///
/// `tx` 是那条被挂住的 HTTP 请求线程在等的通道；前端点了允许/拒绝之后
/// 往它发一个 bool，那边就能构造响应体返回给 Claude Code。
struct Pending {
    id: u64,
    session_id: String,
    project: String,
    tool: String,
    detail: String,
    tx: std::sync::mpsc::Sender<bool>,
}

type PendingState = Arc<Mutex<Vec<Pending>>>;

/// 挂起请求的 id。用原子计数器而不是随机数 —— 不需要不可预测性，
/// 只需要在进程生命周期内唯一。
static NEXT_PENDING_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// 前端看到的挂起请求（不含通道）。
#[derive(Clone, Debug, Serialize)]
struct PendingView {
    id: u64,
    session_id: String,
    project: String,
    tool: String,
    detail: String,
}

/// 窗口右下角锚点 (right, bottom)。折叠/展开会同时改宽高，存左上角的话
/// 挂件每次重启会按当时的折叠状态上下左右漂移。恢复时反算左上角。
type AnchorState = Arc<Mutex<Option<(i32, i32)>>>;

fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

/// 谁更紧急：等你操作的优先，其次在干活的，最后才是空闲。
fn priority(state: &str) -> u8 {
    match state {
        "waiting-permission" => 4,
        "waiting-input" => 3,
        "working" => 2,
        "done" => 1,
        _ => 0,
    }
}

/// 从 cwd 取最后一段当工作空间名。Windows 反斜杠和 POSIX 斜杠都要吃。
fn project_of(cwd: &str) -> String {
    let name = cwd
        .replace('\\', "/")
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or("")
        .to_string();
    if name.is_empty() {
        "unknown".into()
    } else {
        name
    }
}

/// 「工具名: 关键参数」。权限请求和 working 状态都要显示这个。
///
/// 工具名和参数本身是语言中立的（`Bash: npm test`），只有拿不到工具名时的
/// 兜底词需要翻译。
fn tool_summary(v: &Value, lang: i18n::Lang) -> String {
    let tool = v
        .get("tool_name")
        .and_then(Value::as_str)
        .unwrap_or_else(|| i18n::tool_fallback(lang));
    // Bash 看 command，Edit/Write 看 file_path，其它退回 description
    let extra = v
        .get("tool_input")
        .and_then(|i| {
            i.get("command")
                .or_else(|| i.get("file_path"))
                .or_else(|| i.get("pattern"))
                .or_else(|| i.get("description"))
        })
        .and_then(Value::as_str)
        .unwrap_or("");
    if extra.is_empty() {
        tool.to_string()
    } else {
        format!("{tool}: {extra}")
    }
}

/// 把 hook 事件翻译成 (状态, 详情)。返回 None 表示这个事件不改变状态。
fn classify(v: &Value, lang: i18n::Lang) -> Option<(String, String)> {
    let ev = v.get("hook_event_name").and_then(Value::as_str).unwrap_or("");

    match ev {
        "UserPromptSubmit" => Some(("working".into(), i18n::thinking(lang).into())),

        "PreToolUse" | "PostToolUse" => Some(("working".into(), tool_summary(v, lang))),

        "Notification" => {
            let t = v
                .get("notification_type")
                .and_then(Value::as_str)
                .unwrap_or("");
            let msg = v
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            match t {
                "permission_prompt" => Some(("waiting-permission".into(), msg)),
                "agent_needs_input" => Some(("waiting-input".into(), msg)),
                "idle_prompt" => Some(("idle".into(), msg)),
                "agent_completed" => Some(("done".into(), msg)),
                _ => None,
            }
        }

        "Stop" => Some(("idle".into(), i18n::awaiting_reply(lang).into())),
        "SessionStart" => Some(("idle".into(), i18n::session_started(lang).into())),

        _ => None,
    }
}

/// 返回 true 表示该响一声提示音：状态**变成**了某个等待态。
///
/// 判据是 `新状态是 waiting 且不等于旧状态`：
/// - 同一等待态的重复事件不响 —— 否则一串事件会连成噪音
/// - `waiting-permission` → `waiting-input` 会响，因为要你处理的事情换了
fn handle_event(store: &Store, v: &Value, lang: i18n::Lang) -> bool {
    let sid = v
        .get("session_id")
        .and_then(Value::as_str)
        .unwrap_or("unknown")
        .to_string();

    // 会话结束就从表里摘掉，否则关掉的终端会一直挂着一只宠物
    if v.get("hook_event_name").and_then(Value::as_str) == Some("SessionEnd") {
        if let Ok(mut m) = store.lock() {
            m.remove(&sid);
        }
        return false;
    }

    let Some((state, detail)) = classify(v, lang) else {
        return false;
    };
    let cwd = v
        .get("cwd")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let project = project_of(&cwd);
    let now = now_ms();

    let Ok(mut m) = store.lock() else { return false };

    // 先把要用的旧值取出来，再 insert —— 不然 get 的借用和 insert 的可变借用冲突
    let (first_seen, prev_state, prev_agent) = match m.get(&sid) {
        // first_seen 只在第一次出现时写，后续更新要保留 —— 否则图标顺序会变
        Some(s) => (s.first_seen, Some(s.state.clone()), s.agent),
        None => (now, None, Agent::ClaudeCode),
    };
    let notify = state.starts_with("waiting") && prev_state.as_deref() != Some(state.as_str());

    m.insert(
        sid,
        Session {
            // hook 事件只会来自 Claude Code（只有它有 hook 系统），但仍然沿用
            // 旧值而不是写死 —— 万一将来别的 agent 也走这条路，
            // 一个事件不该把宠物的归属悄悄改掉。
            agent: prev_agent,
            cwd,
            project,
            state,
            detail,
            first_seen,
            updated_ms: now,
        },
    );
    notify
}

fn empty_view() -> AppView {
    AppView {
        workspaces: Vec::new(),
        total: 0,
        waiting: 0,
        focus: None,
        pending: Vec::new(),
    }
}

fn pending_views(pending: &PendingState) -> Vec<PendingView> {
    pending
        .lock()
        .map(|list| {
            list.iter()
                .map(|p| PendingView {
                    id: p.id,
                    session_id: p.session_id.clone(),
                    project: p.project.clone(),
                    tool: p.tool.clone(),
                    detail: p.detail.clone(),
                })
                .collect()
        })
        .unwrap_or_default()
}

fn build_view(store: &Store, pending: &PendingState) -> AppView {
    let pending_list = pending_views(pending);
    let map = match store.lock() {
        Ok(m) => m,
        Err(_) => {
            let mut v = empty_view();
            v.pending = pending_list;
            return v;
        }
    };

    // 按 first_seen 排 —— 宠物图标的顺序必须稳定
    let mut all: Vec<(&String, &Session)> = map.iter().collect();
    all.sort_by(|a, b| {
        a.1.first_seen
            .cmp(&b.1.first_seen)
            .then_with(|| a.0.cmp(b.0))
    });

    let mut workspaces: Vec<WorkspaceView> = Vec::new();
    for (id, s) in &all {
        if !workspaces.iter().any(|w| w.project == s.project) {
            workspaces.push(WorkspaceView {
                project: s.project.clone(),
                sessions: Vec::new(),
                worst: "idle".into(),
            });
        }
        // 上面刚保证存在，unwrap 安全
        let ws = workspaces
            .iter_mut()
            .find(|w| w.project == s.project)
            .expect("workspace just inserted");

        ws.sessions.push(SessionView {
            id: (*id).clone(),
            index: ws.sessions.len() + 1,
            state: s.state.clone(),
            detail: s.detail.clone(),
            cwd: s.cwd.clone(),
            agent: s.agent.key(),
            badge: s.agent.badge(),
        });
        if priority(&s.state) > priority(&ws.worst) {
            ws.worst = s.state.clone();
        }
    }

    let waiting = all
        .iter()
        .filter(|(_, s)| s.state.starts_with("waiting"))
        .count();

    // 最紧急的那个；同优先级取最近更新的
    let focus = all
        .iter()
        .max_by_key(|(_, s)| (priority(&s.state), s.updated_ms))
        .map(|(id, _)| (*id).clone());

    AppView {
        workspaces,
        total: all.len(),
        waiting,
        focus,
        pending: pending_list,
    }
}

#[tauri::command]
fn get_view(store: tauri::State<Store>, pending: tauri::State<PendingState>) -> AppView {
    build_view(&store, &pending)
}

/// 挂件启动时一次性拿走它需要的固定信息。
///
/// 这些**不**塞进 `AppView`：那个每来一个 hook 事件就 emit 一次，为几乎不变的
/// 值搭车没道理。语言变化另发 `pet://lang`。
#[derive(Serialize)]
struct BootInfo {
    lang: String,
    version: String,
    repo: String,
    /// 是否允许启动时检查新版本。这是一次对外网络请求，必须可关。
    check_updates: bool,
}

#[tauri::command]
fn get_boot(app: AppHandle, prefs: tauri::State<PrefsState>) -> BootInfo {
    let (lang, check_updates) = prefs
        .lock()
        .map(|p| (p.resolved_lang(), p.check_updates))
        .unwrap_or((i18n::Lang::Zh, false));
    BootInfo {
        lang: lang.code().to_string(),
        version: app.package_info().version.to_string(),
        repo: REPO_URL.to_string(),
        check_updates,
    }
}

/// 前端点了允许/拒绝。把决定送给那条挂住的 HTTP 请求线程。
#[tauri::command]
fn resolve_permission(
    pending: tauri::State<PendingState>,
    id: u64,
    allow: bool,
) -> Result<(), String> {
    let tx = {
        let mut list = pending.lock().map_err(|_| "pending lock poisoned")?;
        // 取出并移除 —— 一条请求只能被决定一次
        let idx = list
            .iter()
            .position(|p| p.id == id)
            .ok_or("这条权限请求已经不在了（可能已超时）")?;
        list.remove(idx).tx
    };
    // 对端可能已经因为超时退出了，那种情况 send 会失败，不算错误
    let _ = tx.send(allow);
    Ok(())
}

// ── 会话自动发现 ─────────────────────────────────────────────

/// 把扫到的会话并进 store，返回新增数量。
///
/// # 谁能覆盖谁
///
/// | 情况 | 行为 | 为什么 |
/// | --- | --- | --- |
/// | Claude Code，已在表里 | **不动** | hook 事件带着准确状态，扫描只能确定「存在」 |
/// | Codex / gateway，已在表里 | **更新状态** | 它们没有 hook，轮询扫描**就是**唯一的状态源 |
///
/// 这条区分是必须的。对 Codex 也「已存在就跳过」的话，宠物会永远停在第一次
/// 扫到的状态上 —— 那正好让整个轮询白做。
fn merge_discovered(
    store: &Store,
    found: Vec<agent::Discovered>,
    lang: i18n::Lang,
) -> usize {
    let mut added = 0;
    let Ok(mut map) = store.lock() else { return 0 };

    for d in found {
        if let Some(existing) = map.get(&d.session_id) {
            // 有 hook 的 agent：事件是权威，扫描不插手
            if !polls_for_state(d.agent) {
                continue;
            }
            // 没有 hook 的 agent：扫描出的状态就是最新的。
            // first_seen 保留旧值，否则宠物顺序会跟着每次轮询乱跳。
            let first_seen = existing.first_seen;
            let state = d.state.clone().unwrap_or_else(|| "idle".to_string());
            let detail = d
                .detail
                .clone()
                .unwrap_or_else(|| i18n::restored(lang).to_string());
            map.insert(
                d.session_id,
                Session {
                    agent: d.agent,
                    project: d.project.unwrap_or_else(|| project_of(&d.cwd)),
                    cwd: d.cwd,
                    state,
                    detail,
                    first_seen,
                    updated_ms: d.mtime_ms,
                },
            );
            continue;
        }

        map.insert(
            d.session_id,
            Session {
                agent: d.agent,
                project: d.project.unwrap_or_else(|| project_of(&d.cwd)),
                cwd: d.cwd,
                // 扫描器给不出状态时只能是 idle。Claude Code 就是这种情况：
                // 转录能告诉我们会话存在，但告诉不了它此刻是在干活还是在等你，
                // 真实事件几秒内会纠正。Codex 反过来 —— rollout 尾部的
                // task_started / task_complete 是明确的，所以这里有值。
                state: d.state.unwrap_or_else(|| "idle".to_string()),
                detail: d.detail.unwrap_or_else(|| i18n::restored(lang).to_string()),
                // 用 mtime 而不是 now，这样宠物顺序反映会话的实际活跃先后
                first_seen: d.mtime_ms,
                updated_ms: d.mtime_ms,
            },
        );
        added += 1;
    }
    added
}

/// 这个 agent 的状态是靠轮询扫描来的，而不是 hook 推的。
///
/// 只有 Claude Code 有 hook。Codex 的 `notify` 是 `config.toml` 里的**单个**
/// 槽位，本机那格已被 OpenAI 自己的 `codex-computer-use.exe` 占着，抢过来会
/// 弄坏用户已有的功能；Hermes / OpenClaw 是常驻 gateway，没有「一次会话结束」
/// 这种适合推事件的时机。
fn polls_for_state(a: Agent) -> bool {
    a != Agent::ClaudeCode
}

/// 扫一遍所有启用的 agent。返回 (扫到数, 新增数)。
fn scan_agents(
    app: &AppHandle,
    store: &Store,
    window: Duration,
    lang: i18n::Lang,
    enabled: &[Agent],
) -> (usize, usize) {
    let home = app.path().home_dir().ok();
    let local = app.path().local_data_dir().ok();
    let mut found: Vec<agent::Discovered> = Vec::new();

    for a in enabled {
        match a {
            Agent::ClaudeCode => {
                if let Some(dir) = discover::projects_dir(home.clone()) {
                    if dir.is_dir() {
                        found.extend(discover::scan(&dir, window));
                    } else {
                        eprintln!("[claude-pet] {} missing, skipping", dir.display());
                    }
                }
            }
            Agent::Codex => {
                if let Some(dir) = codex::sessions_dir(home.clone()) {
                    if dir.is_dir() {
                        found.extend(codex::scan(&dir, window));
                    }
                }
            }
            // Gateway 类不受时间窗影响：它要么现在在跑，要么不在。
            // 「30 分钟内活动过」对一个常驻进程不是有意义的问题。
            Agent::Hermes => {
                found.extend(gateway::hermes_probe(gateway::hermes_home(local.clone()), lang));
            }
            Agent::OpenClaw => {
                found.extend(gateway::openclaw_probe(gateway::openclaw_home(home.clone()), lang));
            }
        }
    }

    // gateway 探测返回 None 表示「没在跑」，此时要把上一轮留下的宠物撤掉，
    // 否则关掉 gateway 之后那只宠物会永远挂在那儿。
    let alive: Vec<String> = found.iter().map(|d| d.session_id.clone()).collect();
    if let Ok(mut map) = store.lock() {
        map.retain(|id, s| !s.agent.is_gateway() || alive.iter().any(|a| a == id));
    }

    let scanned = found.len();
    let added = merge_discovered(store, found, lang);
    (scanned, added)
}

/// 后台线程里跑发现，扫完再 emit。不能放在 setup 的主路径上 ——
/// 本机 54 个项目目录、651 个转录，扫描不该拖慢窗口出现。
///
/// 改了时间窗设置或启用的 agent 后会再调一次：对这两个设置来说，
/// 「立即生效」只能是重扫一遍，光改配置对已经建好的会话表没有任何影响。
fn spawn_discovery(
    app: AppHandle,
    store: Store,
    pending: PendingState,
    window: Duration,
    lang: i18n::Lang,
    enabled: Vec<Agent>,
) {
    std::thread::spawn(move || {
        let started = std::time::Instant::now();
        let (scanned, added) = scan_agents(&app, &store, window, lang, &enabled);

        eprintln!(
            "[claude-pet] discovery: {scanned} session(s) across {} agent(s), {added} new, took {}ms",
            enabled.len(),
            started.elapsed().as_millis()
        );

        // 不能只在 added > 0 时 emit —— 轮询的 agent 可能只是**状态**变了
        // （Codex 从 working 变成 task_complete），会话数一个没多。
        // 那种情况不推的话宠物就永远停在旧状态上。
        let _ = app.emit("pet://view", build_view(&store, &pending));
    });
}

/// 没有 hook 的 agent 靠这个循环维持状态。
///
/// 5 秒一轮，和落盘线程同一个节奏。为什么是 5 秒：Codex 要读 rollout 文件
/// 尾部的 64KB，本机 151 个文件里落在时间窗内的通常只有几个，所以一轮的成本
/// 是几毫秒；再快没有意义，因为「有没有在等我」这个问题不需要亚秒级精度。
const POLL_INTERVAL: Duration = Duration::from_secs(5);

fn spawn_poller(app: AppHandle, store: Store, pending: PendingState, prefs: PrefsState) {
    std::thread::spawn(move || loop {
        std::thread::sleep(POLL_INTERVAL);

        // 每轮重读偏好：设置窗口里改了启用的 agent 要能立刻生效，
        // 而不是等重启。
        let (window, lang, enabled) = match prefs.lock() {
            Ok(p) => (p.discover_window(), p.resolved_lang(), p.enabled_agents()),
            Err(_) => continue,
        };
        let polled: Vec<Agent> = enabled.into_iter().filter(|a| polls_for_state(*a)).collect();
        if polled.is_empty() {
            continue;
        }

        let before = store.lock().ok().map(|m| snapshot(&m));
        scan_agents(&app, &store, window, lang, &polled);
        let after = store.lock().ok().map(|m| snapshot(&m));

        // 只在真的变了才 emit。轮询是每 5 秒一次的，无条件推会让前端
        // 每 5 秒重渲染一遍 —— 而重渲染会重新测量卡片、调 resize_pet，
        // 也就是每 5 秒动一次窗口。
        if before != after {
            let _ = app.emit("pet://view", build_view(&store, &pending));
        }
    });
}

/// 用来判断「有没有变化」的指纹。只含影响显示的字段 ——
/// `updated_ms` 每轮都在动，把它算进来等于无条件 emit。
fn snapshot(map: &HashMap<String, Session>) -> Vec<(String, String, String)> {
    let mut v: Vec<(String, String, String)> = map
        .iter()
        .map(|(id, s)| (id.clone(), s.state.clone(), s.detail.clone()))
        .collect();
    v.sort();
    v
}

/// 装/卸「权限拦截」那条 hook。与普通的 6 条分开，必须显式开启 ——
/// 它是**阻塞**的，装上之后匹配到的工具调用都要等你点，这个代价必须是自愿的。
#[tauri::command]
fn set_permission_hook(app: AppHandle, install: bool, matcher: String) -> Result<String, String> {
    let report = if install {
        hooks::install_permission(&app, matcher.trim())?
    } else {
        hooks::uninstall_permission(&app)?
    };
    Ok(if report.changed {
        match report.backup {
            Some(b) => format!("已更新，备份于 {}", b.display()),
            None => "已更新".into(),
        }
    } else {
        "无需改动".into()
    })
}

/// 前端量完内容后调这里改窗口大小。
///
/// **锚定方向必须随摆放模式变**，否则窗口会朝错的方向长出屏幕：
///
/// | 模式 | 保持不动的边 | 不这么做的后果 |
/// | --- | --- | --- |
/// | `bottom-right` | 右 + 底（朝左上长） | 变高长出屏幕底部、变宽冲出右缘 |
/// | `top-center` | 上 + 水平中心（朝下长） | 按右下锚定会朝上长出屏幕顶部 |
/// | `free` | 离窗口最近的那两条边 | 拖到左上角后朝左上长就出界 |
///
/// 最后无条件夹一次边界，因为「锚对边」还不足以保证不出界。
#[tauri::command]
fn resize_pet(
    app: AppHandle,
    prefs: tauri::State<PrefsState>,
    width: f64,
    height: f64,
) -> Result<(), String> {
    let win = app
        .get_webview_window("pet")
        .ok_or_else(|| "pet window missing".to_string())?;

    let mode = prefs
        .lock()
        .map(|p| PositionMode::parse(&p.position_mode))
        .unwrap_or(PositionMode::BottomRight);

    let old_pos = win.outer_position().map_err(|e| e.to_string())?;
    let old_size = win.outer_size().map_err(|e| e.to_string())?;
    let (old_w, old_h) = (old_size.width as i32, old_size.height as i32);

    win.set_size(LogicalSize::new(width, height))
        .map_err(|e| e.to_string())?;

    // 改完再读实际尺寸 —— set_size 收的是逻辑像素，实际物理尺寸要回读
    let new_size = win.outer_size().map_err(|e| e.to_string())?;
    let (new_w, new_h) = (new_size.width as i32, new_size.height as i32);

    let (x, y) = match mode {
        // 吸附模式直接重算目标位置，不依赖旧坐标
        PositionMode::BottomRight | PositionMode::TopCenter => {
            snap_position(&win, mode, new_w, new_h)
                .unwrap_or((old_pos.x, old_pos.y))
        }
        PositionMode::Free => {
            // 按窗口中心落在显示器的哪个象限，决定保持哪两条边 ——
            // 也就是「朝离得最远的方向长」，这样不管拖到哪都不会立刻撞边。
            let (mx, my, mw, mh) = monitor_rect(&win).unwrap_or((0, 0, i32::MAX, i32::MAX));
            let keep_right = old_pos.x + old_w / 2 > mx + mw / 2;
            let keep_bottom = old_pos.y + old_h / 2 > my + mh / 2;
            (
                if keep_right { old_pos.x + old_w - new_w } else { old_pos.x },
                if keep_bottom { old_pos.y + old_h - new_h } else { old_pos.y },
            )
        }
    };

    let (x, y) = clamp_into_monitor(&win, x, y, new_w, new_h);
    win.set_position(tauri::PhysicalPosition { x, y })
        .map_err(|e| e.to_string())?;

    Ok(())
}

// ── HTTP 监听 ────────────────────────────────────────────────

/// 处理一条挂起的权限请求。**跑在自己的线程里**，会阻塞到用户点了或超时。
fn handle_permission(
    app: AppHandle,
    store: Store,
    pending: PendingState,
    req: tiny_http::Request,
    body: String,
    lang: i18n::Lang,
) {
    // 解析不了就当没这回事：回空 200 = 不给决定，交还 Claude Code 自己判断
    let Ok(v) = serde_json::from_str::<Value>(&body) else {
        let _ = req.respond(tiny_http::Response::empty(200));
        return;
    };

    let session_id = v
        .get("session_id")
        .and_then(Value::as_str)
        .unwrap_or("unknown")
        .to_string();
    let cwd = v.get("cwd").and_then(Value::as_str).unwrap_or("").to_string();
    let project = project_of(&cwd);
    let tool = v
        .get("tool_name")
        .and_then(Value::as_str)
        .unwrap_or_else(|| i18n::tool_fallback(lang))
        .to_string();
    let detail = tool_summary(&v, lang);

    // 让宠物变红。复用现有的 waiting-permission 状态，不另造一套视觉语言 ——
    // 用户已经认识「红色 + ! + 呼吸」就是要他动手。
    if let Ok(mut m) = store.lock() {
        let now = now_ms();
        // 一次取完旧值再 insert —— 分两次 get 会和 insert 的可变借用打起来
        let (first_seen, prev_agent) = match m.get(&session_id) {
            Some(s) => (s.first_seen, s.agent),
            // 权限拦截 hook 只有 Claude Code 有
            None => (now, Agent::ClaudeCode),
        };
        m.insert(
            session_id.clone(),
            Session {
                agent: prev_agent,
                cwd,
                project: project.clone(),
                state: "waiting-permission".into(),
                detail: detail.clone(),
                first_seen,
                updated_ms: now,
            },
        );
    }

    let (tx, rx) = std::sync::mpsc::channel();
    let id = NEXT_PENDING_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    if let Ok(mut list) = pending.lock() {
        list.push(Pending {
            id,
            session_id: session_id.clone(),
            project,
            tool,
            detail: detail.clone(),
            tx,
        });
    }
    let _ = app.emit("pet://view", build_view(&store, &pending));

    let decision = rx.recv_timeout(PERMISSION_WAIT);

    // 无论怎么结束都要摘掉。超时那条路径 resolve_permission 没机会摘，
    // 不清理的话挂件上会永远留着一个点不掉的按钮。
    if let Ok(mut list) = pending.lock() {
        list.retain(|p| p.id != id);
    }

    let response = match decision {
        Ok(allow) => {
            let (verdict, reason) = if allow {
                ("allow", "用户在挂件上批准")
            } else {
                ("deny", "用户在挂件上拒绝")
            };
            eprintln!("[claude-pet] 权限请求 #{id} {verdict}: {detail}");
            let payload = serde_json::json!({
                "hookSpecificOutput": {
                    "hookEventName": "PreToolUse",
                    "permissionDecision": verdict,
                    "permissionDecisionReason": reason
                }
            })
            .to_string();
            let mut r = tiny_http::Response::from_string(payload);
            if let Ok(h) = "Content-Type: application/json".parse::<tiny_http::Header>() {
                r = r.with_header(h);
            }
            r.boxed()
        }
        Err(_) => {
            // 超时 = 不给决定，交还给 Claude Code 自己的权限流程（fail-open）。
            // 这是刻意的方向：挂件挂了或人不在，绝不能把 Claude Code 卡死。
            eprintln!("[claude-pet] 权限请求 #{id} 超时，交回 Claude Code 处理");
            tiny_http::Response::empty(200).boxed()
        }
    };

    let _ = req.respond(response);
    let _ = app.emit("pet://view", build_view(&store, &pending));
}

fn spawn_server(app: AppHandle, store: Store, prefs: PrefsState, pending: PendingState) {
    std::thread::spawn(move || {
        let server = match tiny_http::Server::http(("127.0.0.1", PORT)) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("[claude-pet] 无法监听 127.0.0.1:{PORT}: {e}");
                eprintln!("[claude-pet] 端口被占用？可能已经有一个挂件在跑");
                return;
            }
        };
        eprintln!("[claude-pet] listening on http://127.0.0.1:{PORT}");

        for mut req in server.incoming_requests() {
            let mut body = String::new();
            let _ = req.as_reader().read_to_string(&mut body);

            // /permission 是**阻塞**路径：要挂住等用户点。交给独立线程，
            // 主循环继续处理别的事件 —— 不这么做的话一条权限请求会把所有
            // 其它会话的状态更新堵住 30 秒。
            //
            // 反过来，普通事件刻意留在主循环里顺序处理：这样同一会话的
            // 事件不会被线程调度打乱顺序（Stop 抢在它前面的 PreToolUse 之前）。
            // 每条请求都重读语言：设置窗口随时可能改它，缓存一份就会过期
            let lang = prefs
                .lock()
                .map(|p| p.resolved_lang())
                .unwrap_or(i18n::Lang::Zh);

            if req.url().starts_with("/permission") {
                let app = app.clone();
                let store = store.clone();
                let pending = pending.clone();
                std::thread::spawn(move || {
                    handle_permission(app, store, pending, req, body, lang)
                });
                continue;
            }

            if let Ok(v) = serde_json::from_str::<Value>(&body) {
                let notify = handle_event(&store, &v, lang);
                let _ = app.emit("pet://view", build_view(&store, &pending));

                if notify {
                    // 读不到偏好时按「静音」处理：宁可漏一声，也不要在用户
                    // 已经设了静音的情况下因为读取失败而响。
                    let want = prefs
                        .lock()
                        .ok()
                        .filter(|p| !p.muted)
                        .map(|p| p.sound.clone());
                    // 这两行日志是「为什么没响」唯一可查的线索 —— 声音本身
                    // 不留痕迹，出问题时没有别的地方能看。
                    match want {
                        Some(alias) => {
                            eprintln!("[claude-pet] sound: {alias}");
                            sound::play(&alias);
                        }
                        None => eprintln!("[claude-pet] sound suppressed (muted)"),
                    }
                }
            }

            // 必须回 2xx 空 body —— 官方约定这等价于 exit 0 无输出，
            // 不会给 Claude Code 注入任何上下文。
            let _ = req.respond(tiny_http::Response::empty(200));
        }
    });
}

// ── 窗口位置 ─────────────────────────────────────────────────
//
// 锚点的读写在 `persist` 模块，和会话缓存放一起。

/// 挂件摆在哪。
///
/// `TopCenter` 是刘海 overlay 在 Windows 上的等价物（Windows 没有刘海）。
///
/// 前两个是**吸附**模式：位置由屏幕算出来，拖动不留痕。只有 `Free` 记住拖动。
/// 不做「拖一下就自动切成 Free」那种聪明劲 —— 那是会让人意外的隐式状态变化。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum PositionMode {
    BottomRight,
    TopCenter,
    Free,
}

impl PositionMode {
    fn parse(s: &str) -> Self {
        match s {
            "top-center" => PositionMode::TopCenter,
            "free" => PositionMode::Free,
            _ => PositionMode::BottomRight,
        }
    }
}

/// 离屏幕边缘留的空隙。
const EDGE_MARGIN: i32 = 24;

/// 任务栏高度的估值。
///
/// Tauri 的 monitor API 只给整屏尺寸，拿不到工作区（`SPI_GETWORKAREA`），
/// 所以底部吸附只能减一个估值。顶部吸附不需要它 —— 任务栏默认在底部。
const TASKBAR_GUESS: i32 = 56;

/// 窗口当前所在显示器的矩形（物理像素）：(x, y, w, h)。
///
/// 用 `current_monitor` 而不是主显示器，这样多显示器下吸附会落在
/// 挂件**当前所在**的那块屏上。
fn monitor_rect(win: &WebviewWindow) -> Option<(i32, i32, i32, i32)> {
    let m = win.current_monitor().ok().flatten()?;
    let p = m.position();
    let s = m.size();
    Some((p.x, p.y, s.width as i32, s.height as i32))
}

/// 把 (x, y) 夹进显示器边界，保证窗口任何一边都不越出。
///
/// 这是**无条件**的兜底：不管上面按什么模式算出的坐标，最后都过这一道。
/// 光靠「锚对边」不够 —— Free 模式下窗口可能被拖到任意位置，
/// 朝哪个方向长都可能出界。
fn clamp_into_monitor(win: &WebviewWindow, x: i32, y: i32, w: i32, h: i32) -> (i32, i32) {
    let Some((mx, my, mw, mh)) = monitor_rect(win) else {
        return (x, y);
    };
    // 窗口比屏幕还大时 max 会小于 min，此时贴左上角而不是产生负数区间
    let max_x = (mx + mw - w).max(mx);
    let max_y = (my + mh - h).max(my);
    (x.clamp(mx, max_x), y.clamp(my, max_y))
}

/// 保存的坐标必须落在某个「当前可用」的显示器里才能用。
/// 笔记本插拔扩展屏后旧坐标会把挂件扔到看不见的地方 —— 那种情况下
/// 用户只会以为程序坏了，因为窗口没有标题栏也不在任务栏，找不回来。
fn point_is_visible(win: &WebviewWindow, x: i32, y: i32) -> bool {
    let Ok(monitors) = win.available_monitors() else {
        return false;
    };
    monitors.iter().any(|m| {
        let p = m.position();
        let s = m.size();
        x >= p.x - 8 && y >= p.y - 8 && x < p.x + s.width as i32 && y < p.y + s.height as i32
    })
}

/// 按摆放模式算出窗口左上角该在哪（物理像素）。
///
/// `Free` 返回 `None` —— 那个模式的位置来自保存的锚点而不是算出来的。
fn snap_position(win: &WebviewWindow, mode: PositionMode, w: i32, h: i32) -> Option<(i32, i32)> {
    let (mx, my, mw, mh) = monitor_rect(win)?;
    match mode {
        PositionMode::BottomRight => Some((
            mx + mw - w - EDGE_MARGIN,
            my + mh - h - EDGE_MARGIN - TASKBAR_GUESS,
        )),
        // 水平居中、贴近上边。顶部不用避让任务栏。
        PositionMode::TopCenter => Some((mx + (mw - w) / 2, my + EDGE_MARGIN / 2)),
        PositionMode::Free => None,
    }
}

/// 把挂件摆到位。启动时和切换摆放模式时都走这里。
fn place_window(win: &WebviewWindow, app: &AppHandle, mode: PositionMode) {
    let Ok(size) = win.outer_size() else { return };
    let (w, h) = (size.width as i32, size.height as i32);

    let target = match snap_position(win, mode, w, h) {
        Some(p) => Some(p),
        // Free：用保存的右下角锚点反算左上角
        None => persist::read_anchor(app).and_then(|saved| {
            let (x, y) = (saved.right - w, saved.bottom - h);
            // 左上角和右下角都要在屏幕内 —— 只查一个角的话，
            // 换了分辨率后窗口可能一半在屏幕外
            if point_is_visible(win, x, y) && point_is_visible(win, saved.right - 1, saved.bottom - 1)
            {
                Some((x, y))
            } else {
                eprintln!(
                    "[claude-pet] 保存的锚点 (right {}, bottom {}) 已在屏幕外，回落右下角",
                    saved.right, saved.bottom
                );
                None
            }
        }),
    };

    // Free 模式下锚点无效时回落右下角：宁可摆错位置，也不能让一个
    // 没有标题栏、不在任务栏的窗口停在看不见的地方。
    let (x, y) = target
        .or_else(|| snap_position(win, PositionMode::BottomRight, w, h))
        .unwrap_or((0, 0));
    let (x, y) = clamp_into_monitor(win, x, y, w, h);
    let _ = win.set_position(tauri::PhysicalPosition { x, y });
}

// ── 全局快捷键 ───────────────────────────────────────────────

/// 按偏好注册全局快捷键，返回给用户看的警告（注册失败不该让别的设置也保存不了）。
///
/// 快捷键只负责 emit 一个动作名给前端 —— 折叠状态和选中状态都住在前端的
/// 状态机里，在 Rust 侧另存一份必然会不一致。
fn register_shortcuts(app: &AppHandle, prefs: &persist::Prefs) -> Vec<String> {
    let gs = app.global_shortcut();
    // 先全撤。改绑定时不撤的话旧的还留着，会出现两个键都能用的鬼现象。
    let _ = gs.unregister_all();

    let mut warnings = Vec::new();
    for (accel, action) in [
        (prefs.shortcut_toggle.trim(), "toggle"),
        (prefs.shortcut_next.trim(), "next"),
    ] {
        // 空串是明确的「我不要这个快捷键」，不是错误
        if accel.is_empty() {
            continue;
        }
        let owned = action.to_string();
        let result = gs.on_shortcut(accel, move |app, _shortcut, event| {
            // 按下和松开都会回调，只处理按下 —— 否则每次触发两遍
            if event.state() == ShortcutState::Pressed {
                let _ = app.emit("pet://shortcut", owned.clone());
            }
        });
        match result {
            Ok(()) => eprintln!("[claude-pet] 快捷键 {accel} → {action}"),
            Err(e) => warnings.push(format!(
                "快捷键「{accel}」注册失败，可能已被别的程序占用：{e}"
            )),
        }
    }
    warnings
}

// ── 设置窗口 ─────────────────────────────────────────────────

/// 托盘勾选项的句柄。
///
/// 「开机自启」和「提示音」这两个布尔开关同时出现在托盘和设置窗口里 ——
/// 托盘适合高频快切，设置窗口需要完整可发现。既然出现在两处，就必须
/// 保证两处一致：设置窗口改完要回写这两个勾选状态，否则托盘会骗人。
struct TrayItems {
    autostart: CheckMenuItem<tauri::Wry>,
    sound: CheckMenuItem<tauri::Wry>,
}

type TrayState = Arc<Mutex<Option<TrayItems>>>;

#[derive(Serialize)]
struct AboutInfo {
    version: String,
    config_dir: String,
    claude_settings: String,
    repo: String,
    port: u16,
}

#[derive(Clone, Debug, Serialize)]
struct AgentOption {
    key: &'static str,
    label: &'static str,
    badge: &'static str,
    /// 一个会话一只宠物（false）还是整个 agent 一只（true）。
    /// 界面上要说明这件事，否则用户会奇怪为什么 Hermes 只有一只。
    gateway: bool,
    /// 本机找到配置目录了吗
    detected: bool,
    /// 状态是轮询来的还是 hook 推的。界面上标一下，
    /// 否则用户会以为 Codex 的几秒延迟是 bug。
    polled: bool,
}

/// 探一下本机装了哪些 agent。只看配置目录存在与否 —— 不去跑它们的 CLI，
/// 那会在设置窗口打开时莫名启动别人的进程。
fn agent_options(app: &AppHandle) -> Vec<AgentOption> {
    let home = app.path().home_dir().ok();
    let local = app.path().local_data_dir().ok();

    Agent::ALL
        .iter()
        .map(|a| {
            let detected = match a {
                Agent::ClaudeCode => discover::projects_dir(home.clone())
                    .map(|d| d.is_dir())
                    .unwrap_or(false),
                Agent::Codex => codex::sessions_dir(home.clone())
                    .map(|d| d.is_dir())
                    .unwrap_or(false),
                Agent::Hermes => gateway::hermes_home(local.clone())
                    .map(|d| d.is_dir())
                    .unwrap_or(false),
                Agent::OpenClaw => gateway::openclaw_home(home.clone())
                    .map(|d| d.is_dir())
                    .unwrap_or(false),
            };
            AgentOption {
                key: a.key(),
                label: a.label(),
                badge: a.badge(),
                gateway: a.is_gateway(),
                detected,
                polled: polls_for_state(*a),
            }
        })
        .collect()
}

/// 设置窗口一次性拿走它需要的全部东西，省得开好几个命令来回问。
#[derive(Serialize)]
struct SettingsView {
    prefs: persist::Prefs,
    sounds: Vec<String>,
    /// 本机实际装了的编辑器。没装的不该出现在下拉框里。
    editors: Vec<editor::Available>,
    window_min: u64,
    window_max: u64,
    autostart: bool,
    hooks_installed: usize,
    hooks_total: usize,
    /// 权限拦截 hook 装了没；装了就是它的 matcher。None = 没装。
    permission_matcher: Option<String>,
    permission_wait_secs: u64,
    /// 合法的摆放模式，设置窗口用它填下拉框
    position_modes: Vec<String>,
    /// 所有可选的 agent。设置窗口用它渲染勾选列表，前端不另维护一份名单。
    ///
    /// `detected` 表示本机找到了它的配置目录 —— 用来在界面上标注「没装」，
    /// 而不是把它藏起来：藏起来的话，用户装完 Codex 会找不到开关在哪。
    agent_options: Vec<AgentOption>,
    /// 解析后的语言（`"auto"` 已经变成 `zh` / `en`）。
    /// 前端不自己猜系统语言 —— 两边各猜一次必然会有不一致的时候。
    lang_code: String,
    about: AboutInfo,
}

#[tauri::command]
fn get_settings(app: AppHandle, prefs: tauri::State<PrefsState>) -> SettingsView {
    let current = prefs.lock().map(|p| p.clone()).unwrap_or_default();
    let (hooks_installed, hooks_total) = hooks::status(&app).unwrap_or((0, 0));
    // 先算出来，下面 prefs 字段会把 current move 掉
    let lang_code = current.resolved_lang().code().to_string();

    SettingsView {
        prefs: current,
        sounds: sound::AVAILABLE.iter().map(|s| s.to_string()).collect(),
        editors: editor::available(),
        window_min: persist::WINDOW_MIN,
        window_max: persist::WINDOW_MAX,
        autostart: app.autolaunch().is_enabled().unwrap_or(false),
        hooks_installed,
        hooks_total,
        permission_matcher: hooks::permission_status(&app),
        permission_wait_secs: PERMISSION_WAIT.as_secs(),
        position_modes: persist::POSITION_MODES.iter().map(|s| s.to_string()).collect(),
        agent_options: agent_options(&app),
        lang_code,
        about: AboutInfo {
            version: app.package_info().version.to_string(),
            config_dir: persist::config_path(&app, "")
                .map(|p| p.display().to_string())
                .unwrap_or_default(),
            claude_settings: hooks::settings_path(&app)
                .map(|p| p.display().to_string())
                .unwrap_or_default(),
            repo: "https://github.com/zq940222/claude-pet".into(),
            port: PORT,
        },
    }
}

#[tauri::command]
fn apply_prefs(
    app: AppHandle,
    prefs_state: tauri::State<PrefsState>,
    tray: tauri::State<TrayState>,
    store: tauri::State<Store>,
    incoming: persist::Prefs,
) -> Result<Vec<String>, String> {
    let mut next = incoming;
    // 前端传来的值不能信 —— 手改 prefs.json 和前端 bug 都可能给出越界值
    next.sanitise();

    // 在锁里算出「哪些变了」，出了锁再去做重扫/重注册这些慢动作
    let (window_changed, shortcuts_changed, lang_changed, position_changed, agents_changed) = {
        let mut p = prefs_state.lock().map_err(|_| "prefs lock poisoned")?;
        let changed = (
            p.discover_window_minutes != next.discover_window_minutes,
            p.shortcut_toggle != next.shortcut_toggle || p.shortcut_next != next.shortcut_next,
            p.resolved_lang() != next.resolved_lang(),
            p.position_mode != next.position_mode,
            p.agents != next.agents,
        );
        *p = next.clone();
        persist::save_prefs(&app, &p);
        changed
    };

    // 摆放模式的「立即生效」= 马上挪过去。光存下来不动，用户会以为没生效。
    if position_changed {
        if let Some(win) = app.get_webview_window("pet") {
            place_window(&win, &app, PositionMode::parse(&next.position_mode));
        }
    }

    // 挂件要重渲染成新语言。托盘菜单不跟着变（Tauri 菜单项文本不能原地改，
    // 为此重建整个托盘不值得），所以设置界面里注明了要重启才生效。
    if lang_changed {
        let _ = app.emit("pet://lang", next.resolved_lang().code());
    }

    // 托盘的勾选状态要跟着变，否则两处显示不一致
    if let Ok(items) = tray.lock() {
        if let Some(i) = items.as_ref() {
            let _ = i.sound.set_checked(!next.muted);
        }
    }

    // 时间窗或启用的 agent 变了就重扫一遍，这才叫「立即生效」。
    // 光改配置对已经建好的会话表没有任何影响。
    if window_changed || agents_changed {
        // 取消勾选的 agent 留下的宠物要先撤掉 —— 不撤的话它们会一直挂着，
        // 而用户明确说了不想看它们。gateway 宠物由 scan_agents 自己 retain，
        // 这里处理的是 Codex 这种「不再扫但表里还有」的情况。
        if agents_changed {
            let keep = next.enabled_agents();
            if let Ok(mut m) = store.lock() {
                m.retain(|_, s| keep.contains(&s.agent));
            }
        }
        let pending: tauri::State<PendingState> = app.state();
        spawn_discovery(
            app.clone(),
            (*store).clone(),
            (*pending).clone(),
            next.discover_window(),
            next.resolved_lang(),
            next.enabled_agents(),
        );
    }

    // 快捷键注册失败只当警告返回：别因为一个键被占用就让其它设置也存不下去
    let warnings = if shortcuts_changed {
        register_shortcuts(&app, &next)
    } else {
        Vec::new()
    };

    Ok(warnings)
}

#[tauri::command]
fn set_autostart(
    app: AppHandle,
    tray: tauri::State<TrayState>,
    enabled: bool,
) -> Result<(), String> {
    let mgr = app.autolaunch();
    let r = if enabled { mgr.enable() } else { mgr.disable() };
    r.map_err(|e| e.to_string())?;

    if let Ok(items) = tray.lock() {
        if let Some(i) = items.as_ref() {
            let _ = i.autostart.set_checked(enabled);
        }
    }
    Ok(())
}

/// 双击宠物 → 在编辑器里打开该会话的 cwd。
#[tauri::command]
fn open_in_editor(
    store: tauri::State<Store>,
    prefs: tauri::State<PrefsState>,
    session_id: String,
) -> Result<String, String> {
    let cwd = store
        .lock()
        .map_err(|_| "store lock poisoned".to_string())?
        .get(&session_id)
        .map(|s| s.cwd.clone())
        .ok_or("会话已不存在")?;

    if cwd.is_empty() {
        // 从 v1 缓存恢复的会话没有 cwd。理论上 v2 的版本门禁已经把这种
        // 情况挡掉了，留着这条是因为「猜一个路径」比报错更糟。
        return Err("这个会话没有记录工作目录".into());
    }

    let preferred = prefs
        .lock()
        .map(|p| p.editor.clone())
        .unwrap_or_else(|_| "auto".to_string());

    editor::open(&cwd, &preferred)
}

#[tauri::command]
fn preview_sound(alias: String) {
    // 只放白名单里的，别让前端把任意字符串塞进 PlaySoundW
    if sound::AVAILABLE.contains(&alias.as_str()) {
        sound::play(&alias);
    }
}

#[tauri::command]
fn set_hooks(app: AppHandle, install: bool) -> Result<String, String> {
    let report = if install {
        hooks::install(&app)?
    } else {
        hooks::uninstall(&app)?
    };
    Ok(if report.changed {
        match report.backup {
            Some(b) => format!("已更新，备份于 {}", b.display()),
            None => "已更新".into(),
        }
    } else {
        "无需改动".into()
    })
}

/// 打开设置窗口。按需创建 —— 常驻一个隐藏的 WebView2 会白占几十 MB，
/// 而挂件的卖点之一就是常驻内存小。
fn open_settings(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("settings") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
        return;
    }
    let lang = app
        .state::<PrefsState>()
        .lock()
        .map(|p| p.resolved_lang())
        .unwrap_or(i18n::Lang::Zh);
    match WebviewWindowBuilder::new(app, "settings", WebviewUrl::App("settings.html".into()))
        .title(i18n::window_settings(lang))
        .inner_size(540.0, 620.0)
        .min_inner_size(460.0, 480.0)
        .resizable(true)
        .center()
        // 设置窗口是普通窗口：要标题栏、要进任务栏、不置顶。
        // 挂件那套透明/无边框/置顶的属性一个都不能带过来。
        .decorations(true)
        .always_on_top(false)
        .skip_taskbar(false)
        .build()
    {
        Ok(_) => {}
        Err(e) => eprintln!("[claude-pet] cannot open settings window: {e}"),
    }
}

// ── 托盘 ─────────────────────────────────────────────────────

fn build_tray(app: &tauri::App, prefs: PrefsState, tray_state: TrayState) -> tauri::Result<()> {
    let version = app.package_info().version.to_string();

    // is_enabled 会真的去读 HKCU\...\Run，所以这行日志能证明插件是通的
    let autostart_on = match app.autolaunch().is_enabled() {
        Ok(on) => {
            eprintln!("[claude-pet] v{version}, autostart={on}");
            on
        }
        Err(e) => {
            eprintln!("[claude-pet] v{version}, autostart state unreadable: {e}");
            false
        }
    };

    // 版本项 enabled=false，纯展示 —— 想知道自己跑的是哪个版本
    let version_item = MenuItem::with_id(
        app,
        "version",
        format!("Claude Pet v{version}"),
        false,
        None::<&str>,
    )?;
    // 托盘菜单在建的时候定语言。改语言后要重启挂件才会跟着变 ——
    // Tauri 的菜单项文本不能原地改，重建整个托盘的代价和收益不成比例。
    let lang = prefs
        .lock()
        .map(|p| p.resolved_lang())
        .unwrap_or(i18n::Lang::Zh);

    let autostart_item = CheckMenuItem::with_id(
        app,
        "autostart",
        i18n::tray_autostart(lang),
        true,
        autostart_on,
        None::<&str>,
    )?;

    // 正向表述：勾上 = 会响。比「静音」勾上=不响少一层反向理解。
    let sound_on = !prefs.lock().map(|p| p.muted).unwrap_or(false);
    let sound_item = CheckMenuItem::with_id(
        app,
        "sound",
        i18n::tray_sound(lang),
        true,
        sound_on,
        None::<&str>,
    )?;

    let sep = PredefinedMenuItem::separator(app)?;
    let sep2 = PredefinedMenuItem::separator(app)?;
    let settings_item =
        MenuItem::with_id(app, "settings", i18n::tray_settings(lang), true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", i18n::tray_quit(lang), true, None::<&str>)?;

    let menu = Menu::with_items(
        app,
        &[
            &version_item,
            &sep,
            &autostart_item,
            &sound_item,
            &sep2,
            &settings_item,
            &quit,
        ],
    )?;

    // 存下句柄，设置窗口改完偏好后要回写勾选状态
    if let Ok(mut slot) = tray_state.lock() {
        *slot = Some(TrayItems {
            autostart: autostart_item.clone(),
            sound: sound_item.clone(),
        });
    }

    let check_handle = autostart_item.clone();
    let sound_handle = sound_item.clone();
    let prefs_for_menu = prefs.clone();
    let mut tray = TrayIconBuilder::new()
        .menu(&menu)
        .tooltip(format!("Claude Pet v{version}"))
        .on_menu_event(move |app, event| match event.id().as_ref() {
            "quit" => app.exit(0),
            "settings" => open_settings(app),
            "autostart" => {
                let mgr = app.autolaunch();
                let currently_on = mgr.is_enabled().unwrap_or(false);
                let result = if currently_on {
                    mgr.disable()
                } else {
                    mgr.enable()
                };
                match result {
                    // 只在真的切换成功后才改勾选状态，否则菜单会骗人
                    Ok(()) => {
                        let _ = check_handle.set_checked(!currently_on);
                    }
                    Err(e) => eprintln!("[claude-pet] autostart toggle failed: {e}"),
                }
            }
            "sound" => {
                let mut now_on = false;
                if let Ok(mut p) = prefs_for_menu.lock() {
                    p.muted = !p.muted;
                    now_on = !p.muted;
                    persist::save_prefs(app, &p);
                }
                let _ = sound_handle.set_checked(now_on);
                // 打开时立刻试听一声，这样用户知道自己听到的是什么
                if now_on {
                    let alias = prefs_for_menu
                        .lock()
                        .map(|p| p.sound.clone())
                        .unwrap_or_else(|_| sound::DEFAULT_ALIAS.to_string());
                    sound::play(&alias);
                }
            }
            _ => {}
        });

    if let Some(icon) = app.default_window_icon() {
        tray = tray.icon(icon.clone());
    }
    tray.build(app)?;
    Ok(())
}

// ── 无头 CLI ─────────────────────────────────────────────────

/// 处理 `--open <dir>`：用配置的编辑器打开一个目录然后退出。
///
/// 存在的理由是可诊断性：双击宠物跳不动时，这条命令能把「编辑器探测/启动」
/// 这一环单独拎出来验，不必去猜是 UI 没响应还是 spawn 失败。
fn handle_open_cli(app: &tauri::App) -> Option<i32> {
    let args: Vec<String> = std::env::args().collect();
    let idx = args.iter().position(|a| a == "--open")?;
    let Some(dir) = args.get(idx + 1) else {
        eprintln!("[claude-pet] --open needs a directory");
        return Some(1);
    };

    let preferred = persist::load_prefs(app.handle()).editor;
    eprintln!("[claude-pet] editors found: {:?}",
        editor::available().iter().map(|e| e.key.as_str()).collect::<Vec<_>>());
    match editor::open(dir, &preferred) {
        Ok(label) => {
            eprintln!("[claude-pet] opened {dir} in {label} (preferred={preferred})");
            Some(0)
        }
        Err(e) => {
            eprintln!("[claude-pet] open failed: {e}");
            Some(1)
        }
    }
}

/// 处理 `--install-hooks` / `--uninstall-hooks` / `--hooks-status`。
/// 约定与 autostart 那组一致：用退出码传结果。
fn handle_hooks_cli(app: &tauri::App) -> Option<i32> {
    let args: Vec<String> = std::env::args().collect();
    // 权限拦截 hook 单独一组：它是阻塞的，不该跟普通那 6 条混在一个开关里
    if let Some(i) = args.iter().position(|a| a == "--install-permission-hook") {
        let matcher = args.get(i + 1).map(String::as_str).unwrap_or("Bash");
        return Some(match hooks::install_permission(app.handle(), matcher) {
            Ok(r) => {
                eprintln!(
                    "[claude-pet] 权限拦截 hook 已装（matcher = {matcher}）{}",
                    if r.changed { "" } else { "，无需改动" }
                );
                if let Some(b) = r.backup {
                    eprintln!("[claude-pet] 备份: {}", b.display());
                }
                eprintln!("[claude-pet] 注意：匹配到的工具调用会挂住等你在挂件上点，最多 {} 秒后交回 Claude Code", PERMISSION_WAIT.as_secs());
                0
            }
            Err(e) => {
                eprintln!("[claude-pet] 装权限拦截 hook 失败: {e}");
                1
            }
        });
    }
    if args.iter().any(|a| a == "--uninstall-permission-hook") {
        return Some(match hooks::uninstall_permission(app.handle()) {
            Ok(r) => {
                eprintln!(
                    "[claude-pet] 权限拦截 hook {}",
                    if r.changed { "已卸载" } else { "本来没装" }
                );
                0
            }
            Err(e) => {
                eprintln!("[claude-pet] 卸载权限拦截 hook 失败: {e}");
                1
            }
        });
    }

    let flag = args.iter().find(|a| {
        matches!(
            a.as_str(),
            "--install-hooks" | "--uninstall-hooks" | "--hooks-status"
        )
    })?;

    let handle = app.handle();

    let code = match flag.as_str() {
        "--install-hooks" => match hooks::install(handle) {
            Ok(r) => {
                if r.changed {
                    eprintln!("[claude-pet] installed {} hook event(s)", r.touched);
                    if let Some(b) = r.backup {
                        eprintln!("[claude-pet] backup: {}", b.display());
                    }
                } else {
                    eprintln!("[claude-pet] hooks already installed, nothing to do");
                }
                0
            }
            Err(e) => {
                eprintln!("[claude-pet] install-hooks failed: {e}");
                1
            }
        },
        "--uninstall-hooks" => match hooks::uninstall(handle) {
            Ok(r) => {
                if r.changed {
                    eprintln!("[claude-pet] removed {} hook(s)", r.touched);
                    if let Some(b) = r.backup {
                        eprintln!("[claude-pet] backup: {}", b.display());
                    }
                } else {
                    eprintln!("[claude-pet] no hooks of ours were present");
                }
                0
            }
            Err(e) => {
                eprintln!("[claude-pet] uninstall-hooks failed: {e}");
                1
            }
        },
        // status: 0 = 全装好，2 = 没装或只装了一部分，1 = 读不了配置
        _ => match hooks::status(handle) {
            Ok((installed, total)) => {
                eprintln!("[claude-pet] hooks installed: {installed}/{total}");
                if installed == total {
                    0
                } else {
                    2
                }
            }
            Err(e) => {
                eprintln!("[claude-pet] hooks-status failed: {e}");
                1
            }
        },
    };

    Some(code)
}

/// 处理 `--enable-autostart` / `--disable-autostart` / `--autostart-status`。
/// 返回 Some(exit_code) 表示这是一次 CLI 调用，调用方应当立刻退出；
/// None 表示正常启动挂件。
///
/// 用退出码而不是 stdout 传结果：release 构建是 windows_subsystem="windows"，
/// 没有控制台，println! 会掉进虚空。
fn handle_autostart_cli(app: &tauri::App) -> Option<i32> {
    let args: Vec<String> = std::env::args().collect();
    let mgr = app.autolaunch();

    let flag = args.iter().find(|a| {
        matches!(
            a.as_str(),
            "--enable-autostart" | "--disable-autostart" | "--autostart-status"
        )
    })?;

    let code = match flag.as_str() {
        "--enable-autostart" => match mgr.enable() {
            Ok(()) => {
                eprintln!("[claude-pet] autostart enabled");
                0
            }
            Err(e) => {
                eprintln!("[claude-pet] enable failed: {e}");
                1
            }
        },
        "--disable-autostart" => match mgr.disable() {
            Ok(()) => {
                eprintln!("[claude-pet] autostart disabled");
                0
            }
            Err(e) => {
                eprintln!("[claude-pet] disable failed: {e}");
                1
            }
        },
        // status: 0 = 已开启，2 = 已关闭，1 = 读不到。用退出码区分。
        _ => match mgr.is_enabled() {
            Ok(true) => {
                eprintln!("[claude-pet] autostart is ON");
                0
            }
            Ok(false) => {
                eprintln!("[claude-pet] autostart is OFF");
                2
            }
            Err(e) => {
                eprintln!("[claude-pet] status unreadable: {e}");
                1
            }
        },
    };

    Some(code)
}

// ── main ─────────────────────────────────────────────────────

/// 把控制台输出代码页设成 UTF-8。
///
/// Windows 控制台默认是 GBK（本机 936），而 Rust 的 `eprintln!` 写的是 UTF-8，
/// 于是所有中文诊断信息都会变成 `涓嶅湪 PATH 涓?` 这种乱码。之前的应对是
/// 「日志一律写英文」，那是绕开而不是解决 —— 用户看到的错误信息本该是中文的。
///
/// release 构建带 `windows_subsystem = "windows"`，没有控制台，这里是安全的空操作。
#[cfg(windows)]
fn use_utf8_console() {
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn SetConsoleOutputCP(cp: u32) -> i32;
    }
    const CP_UTF8: u32 = 65001;
    // SAFETY: 只是设置当前进程控制台的输出代码页，没有指针参与。
    unsafe {
        SetConsoleOutputCP(CP_UTF8);
    }
}

#[cfg(not(windows))]
fn use_utf8_console() {}

fn main() {
    use_utf8_console();

    let store: Store = Arc::new(Mutex::new(HashMap::new()));
    let anchor: AnchorState = Arc::new(Mutex::new(None));
    // 真正的值在 setup 里从磁盘读 —— 那时才拿得到 AppHandle 来解析配置目录
    let prefs: PrefsState = Arc::new(Mutex::new(persist::Prefs::default()));
    let tray_state: TrayState = Arc::new(Mutex::new(None));
    let pending: PendingState = Arc::new(Mutex::new(Vec::new()));

    tauri::Builder::default()
        .plugin(tauri_plugin_autostart::init(
            // Windows 上走 HKCU\...\Run；LaunchAgent 只影响 macOS
            MacosLauncher::LaunchAgent,
            None,
        ))
        // 不给全局 handler：每个快捷键在 register_shortcuts 里挂自己的闭包
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .manage(store.clone())
        .manage(prefs.clone())
        .manage(tray_state.clone())
        .manage(pending.clone())
        .invoke_handler(tauri::generate_handler![
            get_view,
            get_boot,
            resize_pet,
            get_settings,
            apply_prefs,
            set_autostart,
            preview_sound,
            set_hooks,
            set_permission_hook,
            resolve_permission,
            open_in_editor
        ])
        .setup(move |app| {
            // 无头 CLI：安装脚本要用，也方便手工排查。
            // 自启那组必须从「安装后的」exe 调用 —— 插件注册的是当前 exe 的路径。
            if let Some(code) = handle_autostart_cli(app) {
                std::process::exit(code);
            }
            if let Some(code) = handle_hooks_cli(app) {
                std::process::exit(code);
            }
            if let Some(code) = handle_open_cli(app) {
                std::process::exit(code);
            }

            let handle = app.handle().clone();

            // 偏好要在建托盘之前读 —— 托盘的「提示音」勾选状态来自它
            let mut window = persist::Prefs::default().discover_window();
            let mut startup_lang = i18n::Lang::Zh;
            let mut startup_position_mode = persist::Prefs::default().position_mode;
            let mut startup_agents = persist::Prefs::default().enabled_agents();
            if let Ok(mut p) = prefs.lock() {
                *p = persist::load_prefs(&handle);
                window = p.discover_window();
                startup_lang = p.resolved_lang();
                startup_position_mode = p.position_mode.clone();
                startup_agents = p.enabled_agents();
                eprintln!(
                    "[claude-pet] prefs: lang={} muted={} sound={} window={}min position={} agents={}",
                    startup_lang.code(),
                    p.muted,
                    p.sound,
                    p.discover_window_minutes,
                    p.position_mode,
                    p.agents.join(",")
                );
            }

            // 窗口没有标题栏也不在任务栏，托盘是唯一的退出入口 —— 不能省。
            build_tray(app, prefs.clone(), tray_state.clone())?;

            if let Ok(p) = prefs.lock() {
                for w in register_shortcuts(&handle, &p) {
                    eprintln!("[claude-pet] {w}");
                }
            }

            // 缓存必须在 spawn_discovery 之前加载。
            //
            // 冲突解决就靠这个顺序：merge_discovered 会跳过已存在的 session_id，
            // 所以先进来的缓存自动胜出。这是想要的方向 —— 缓存带着真实的状态和
            // detail，而扫描只能确定「这个会话存在」，状态一律填 idle。
            // 反过来的话，重启后所有宠物都会被扫描结果刷成灰色。
            let restored = persist::load_sessions(&handle, window, now_ms());
            let restored_count = restored.len();
            if restored_count > 0 {
                if let Ok(mut map) = store.lock() {
                    map.extend(restored);
                }
            }
            eprintln!("[claude-pet] session cache: {restored_count} restored");

            // `--settings` 启动即打开设置窗口。和上面那些 CLI 开关不同，
            // 这个**不退出**，挂件照常起 —— 它是个入口而不是一次性命令。
            if std::env::args().any(|a| a == "--settings") {
                open_settings(&handle);
            }

            if let Some(win) = app.get_webview_window("pet") {
                // 窗口在 config 里是 visible:false，先摆好位置再 show，
                // 否则会先在默认位置闪一下再跳到恢复的位置。
                place_window(&win, &handle, PositionMode::parse(&startup_position_mode));
                let _ = win.show();
                let _ = win.set_always_on_top(true);

                // 挂件没有可见的控制台，前端一出问题只能靠 devtools 定位。
                // 只在 debug 构建里开，release 不会带。
                #[cfg(debug_assertions)]
                if std::env::args().any(|a| a == "--devtools") {
                    win.open_devtools();
                }

                // 拖动时只记内存，不碰磁盘。存右下角，和 resize_pet 的锚定一致。
                let anchor_for_move = anchor.clone();
                let win_for_move = win.clone();
                win.on_window_event(move |event| {
                    if let tauri::WindowEvent::Moved(p) = event {
                        if let Ok(size) = win_for_move.outer_size() {
                            if let Ok(mut slot) = anchor_for_move.lock() {
                                *slot = Some((
                                    p.x + size.width as i32,
                                    p.y + size.height as i32,
                                ));
                            }
                        }
                    }
                });

                // 一个后台线程干三件事：
                //  1. 重新抬升置顶 —— Windows 上独占全屏程序和 UAC 安全桌面会抢走
                //     topmost。set_always_on_top 是幂等的，不抢焦点。
                //  2. 落盘锚点 —— 只在变化时写，最多丢 5 秒内的移动。
                //  3. 落盘会话缓存 —— 同样「变了才写」，比较序列化后的字符串。
                //     最多丢 5 秒内的状态变化，对「重启后接着看」够用了。
                let w = win.clone();
                let anchor_for_flush = anchor.clone();
                let store_for_flush = store.clone();
                let app_for_flush = handle.clone();
                std::thread::spawn(move || {
                    let mut last_anchor: Option<(i32, i32)> = None;
                    let mut last_sessions: Option<String> = None;
                    loop {
                        std::thread::sleep(Duration::from_secs(5));
                        let _ = w.set_always_on_top(true);

                        let current = anchor_for_flush.lock().ok().and_then(|g| *g);
                        if let Some((right, bottom)) = current {
                            if last_anchor != Some((right, bottom)) {
                                persist::write_anchor(&app_for_flush, right, bottom);
                                last_anchor = Some((right, bottom));
                            }
                        }

                        // 先序列化再比较，避免没变化时白写一次磁盘。
                        // 会话数是个位数，每 5 秒序列化一次的开销可以忽略。
                        let encoded = store_for_flush
                            .lock()
                            .ok()
                            .and_then(|m| persist::encode_sessions(&m));
                        if let Some(json) = encoded {
                            if last_sessions.as_deref() != Some(json.as_str()) {
                                persist::write_sessions(&app_for_flush, &json);
                                last_sessions = Some(json);
                            }
                        }
                    }
                });
            }

            spawn_server(handle.clone(), store.clone(), prefs.clone(), pending.clone());
            // 没有 hook 的 agent（Codex / Hermes / OpenClaw）靠这个循环维持状态。
            // 只要有任何一个被启用它就有事做；一个都没启用时它每轮空转一次就睡，
            // 成本可以忽略，所以不按配置决定要不要起 —— 那样改设置就得重启。
            spawn_poller(handle.clone(), store.clone(), pending.clone(), prefs.clone());
            spawn_discovery(
                handle,
                store.clone(),
                pending.clone(),
                window,
                startup_lang,
                startup_agents,
            );
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("Claude Pet 启动失败");
}
