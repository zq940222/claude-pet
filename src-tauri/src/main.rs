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

mod discover;
mod editor;
mod hooks;
mod persist;
mod sound;

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

// ── 会话状态 ─────────────────────────────────────────────────

/// 派生 `Deserialize` 是为了跨启动持久化（见 `persist` 模块）——
/// 落盘的就是这个结构，改字段记得同步 `persist` 里的 `CACHE_VERSION`。
#[derive(Clone, Debug, Serialize, Deserialize)]
struct Session {
    /// 完整工作目录。跳回 IDE 要用它 —— `project` 只是末段，拿不回原路径。
    cwd: String,
    project: String,
    state: String,
    detail: String,
    /// 首次出现的时间。用来给宠物图标定一个稳定顺序 ——
    /// HashMap 的迭代顺序是随机的，不排序图标每次刷新都会乱跳。
    first_seen: u128,
    updated_ms: u128,
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
}

type Store = Arc<Mutex<HashMap<String, Session>>>;

/// 用户偏好。托盘改它、HTTP 线程读它，所以要共享。
type PrefsState = Arc<Mutex<persist::Prefs>>;

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

/// 把 hook 事件翻译成 (状态, 详情)。返回 None 表示这个事件不改变状态。
fn classify(v: &Value) -> Option<(String, String)> {
    let ev = v.get("hook_event_name").and_then(Value::as_str).unwrap_or("");

    match ev {
        "UserPromptSubmit" => Some(("working".into(), "思考中".into())),

        "PreToolUse" | "PostToolUse" => {
            let tool = v.get("tool_name").and_then(Value::as_str).unwrap_or("工具");
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
            let detail = if extra.is_empty() {
                tool.to_string()
            } else {
                format!("{tool}: {extra}")
            };
            Some(("working".into(), detail))
        }

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

        "Stop" => Some(("idle".into(), "等你下一句".into())),
        "SessionStart" => Some(("idle".into(), "会话开始".into())),

        _ => None,
    }
}

/// 返回 true 表示该响一声提示音：状态**变成**了某个等待态。
///
/// 判据是 `新状态是 waiting 且不等于旧状态`：
/// - 同一等待态的重复事件不响 —— 否则一串事件会连成噪音
/// - `waiting-permission` → `waiting-input` 会响，因为要你处理的事情换了
fn handle_event(store: &Store, v: &Value) -> bool {
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

    let Some((state, detail)) = classify(v) else {
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
    let (first_seen, prev_state) = match m.get(&sid) {
        // first_seen 只在第一次出现时写，后续更新要保留 —— 否则图标顺序会变
        Some(s) => (s.first_seen, Some(s.state.clone())),
        None => (now, None),
    };
    let notify = state.starts_with("waiting") && prev_state.as_deref() != Some(state.as_str());

    m.insert(
        sid,
        Session {
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
    }
}

fn build_view(store: &Store) -> AppView {
    let map = match store.lock() {
        Ok(m) => m,
        Err(_) => return empty_view(),
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
    }
}

#[tauri::command]
fn get_view(store: tauri::State<Store>) -> AppView {
    build_view(&store)
}

// ── 会话自动发现 ─────────────────────────────────────────────

/// 把扫到的会话并进 store，返回新增数量。
///
/// 真实 hook 事件优先：已经在表里的会话不覆盖 —— 事件带着准确的状态和 detail，
/// 而扫描只能确定「这个会话存在」。
fn merge_discovered(store: &Store, found: Vec<discover::Discovered>) -> usize {
    let mut added = 0;
    let Ok(mut map) = store.lock() else { return 0 };

    for d in found {
        if map.contains_key(&d.session_id) {
            continue;
        }
        map.insert(
            d.session_id,
            Session {
                project: project_of(&d.cwd),
                cwd: d.cwd,
                // 状态只能是 idle：转录能告诉我们会话存在，但告诉不了它此刻
                // 是在干活还是在等你。真实事件几秒内就会把它纠正过来。
                state: "idle".into(),
                detail: "恢复的会话".into(),
                // 用 mtime 而不是 now，这样宠物顺序反映会话的实际活跃先后
                first_seen: d.mtime_ms,
                updated_ms: d.mtime_ms,
            },
        );
        added += 1;
    }
    added
}

/// 后台线程里跑发现，扫完再 emit。不能放在 setup 的主路径上 ——
/// 本机 54 个项目目录、651 个转录，扫描不该拖慢窗口出现。
///
/// 改了时间窗设置后会再调一次：对这个设置来说，「立即生效」只能是
/// 用新窗口重扫一遍，光改数字对已经建好的会话表没有任何影响。
fn spawn_discovery(app: AppHandle, store: Store, window: Duration) {
    std::thread::spawn(move || {
        let home = app.path().home_dir().ok();
        let Some(dir) = discover::projects_dir(home) else {
            eprintln!("[claude-pet] cannot locate ~/.claude/projects, skipping discovery");
            return;
        };
        if !dir.is_dir() {
            eprintln!("[claude-pet] {} does not exist, skipping discovery", dir.display());
            return;
        }

        let started = std::time::Instant::now();
        let found = discover::scan(&dir, window);
        let scanned = found.len();
        let added = merge_discovered(&store, found);

        eprintln!(
            "[claude-pet] discovery: {scanned} recent session(s), {added} restored, took {}ms",
            started.elapsed().as_millis()
        );

        if added > 0 {
            let _ = app.emit("pet://view", build_view(&store));
        }
    });
}

/// 前端量完内容后调这里改窗口大小。
///
/// 关键：**右边和底边都要锚定**，让窗口朝左上生长。挂件停在屏幕右下角，
/// 按左上角改尺寸的话，变高会长出屏幕底部、变宽会冲出屏幕右缘 ——
/// 折叠态的宠物点阵横向增长时尤其明显。
#[tauri::command]
fn resize_pet(app: AppHandle, width: f64, height: f64) -> Result<(), String> {
    let win = app
        .get_webview_window("pet")
        .ok_or_else(|| "pet window missing".to_string())?;

    // 先记下旧的右下角（物理像素）
    let old_pos = win.outer_position().map_err(|e| e.to_string())?;
    let old_size = win.outer_size().map_err(|e| e.to_string())?;
    let right = old_pos.x + old_size.width as i32;
    let bottom = old_pos.y + old_size.height as i32;

    win.set_size(LogicalSize::new(width, height))
        .map_err(|e| e.to_string())?;

    // 改完再读实际尺寸，反算左上角让右下角回到原处
    let new_size = win.outer_size().map_err(|e| e.to_string())?;
    win.set_position(tauri::PhysicalPosition {
        x: right - new_size.width as i32,
        y: bottom - new_size.height as i32,
    })
    .map_err(|e| e.to_string())?;

    Ok(())
}

// ── HTTP 监听 ────────────────────────────────────────────────

fn spawn_server(app: AppHandle, store: Store, prefs: PrefsState) {
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

            if let Ok(v) = serde_json::from_str::<Value>(&body) {
                let notify = handle_event(&store, &v);
                let _ = app.emit("pet://view", build_view(&store));

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

/// 初始摆到右下角。减掉 56px 粗略避开任务栏 —— Tauri 的 monitor API
/// 给的是整个屏幕尺寸，拿不到工作区，所以这里只能估。
fn position_bottom_right(win: &WebviewWindow) {
    if let (Ok(Some(monitor)), Ok(size)) = (win.current_monitor(), win.outer_size()) {
        let ms = monitor.size();
        let mp = monitor.position();
        let margin = 24i32;
        let x = mp.x + ms.width as i32 - size.width as i32 - margin;
        let y = mp.y + ms.height as i32 - size.height as i32 - margin - 56;
        let _ = win.set_position(tauri::PhysicalPosition { x, y });
    }
}

fn restore_position(win: &WebviewWindow, app: &AppHandle) {
    if let Some(saved) = persist::read_anchor(app) {
        if let Ok(size) = win.outer_size() {
            let x = saved.right - size.width as i32;
            let y = saved.bottom - size.height as i32;
            // 校验左上角和右下角都在屏幕内 —— 只查一个角的话，
            // 换了分辨率后窗口可能一半在屏幕外
            if point_is_visible(win, x, y) && point_is_visible(win, saved.right - 1, saved.bottom - 1)
            {
                let _ = win.set_position(tauri::PhysicalPosition { x, y });
                return;
            }
            eprintln!(
                "[claude-pet] saved anchor (right {}, bottom {}) is off-screen, using default",
                saved.right, saved.bottom
            );
        }
    }
    position_bottom_right(win);
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
    about: AboutInfo,
}

#[tauri::command]
fn get_settings(app: AppHandle, prefs: tauri::State<PrefsState>) -> SettingsView {
    let current = prefs.lock().map(|p| p.clone()).unwrap_or_default();
    let (hooks_installed, hooks_total) = hooks::status(&app).unwrap_or((0, 0));

    SettingsView {
        prefs: current,
        sounds: sound::AVAILABLE.iter().map(|s| s.to_string()).collect(),
        editors: editor::available(),
        window_min: persist::WINDOW_MIN,
        window_max: persist::WINDOW_MAX,
        autostart: app.autolaunch().is_enabled().unwrap_or(false),
        hooks_installed,
        hooks_total,
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
    let (window_changed, shortcuts_changed) = {
        let mut p = prefs_state.lock().map_err(|_| "prefs lock poisoned")?;
        let changed = (
            p.discover_window_minutes != next.discover_window_minutes,
            p.shortcut_toggle != next.shortcut_toggle || p.shortcut_next != next.shortcut_next,
        );
        *p = next.clone();
        persist::save_prefs(&app, &p);
        changed
    };

    // 托盘的勾选状态要跟着变，否则两处显示不一致
    if let Ok(items) = tray.lock() {
        if let Some(i) = items.as_ref() {
            let _ = i.sound.set_checked(!next.muted);
        }
    }

    // 时间窗变了就用新窗口重扫一遍，这才叫「立即生效」
    if window_changed {
        spawn_discovery(app.clone(), (*store).clone(), next.discover_window());
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
    match WebviewWindowBuilder::new(app, "settings", WebviewUrl::App("settings.html".into()))
        .title("Claude Pet 设置")
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
    let autostart_item =
        CheckMenuItem::with_id(app, "autostart", "开机自启", true, autostart_on, None::<&str>)?;

    // 正向表述：勾上 = 会响。比「静音」勾上=不响少一层反向理解。
    let sound_on = !prefs.lock().map(|p| p.muted).unwrap_or(false);
    let sound_item =
        CheckMenuItem::with_id(app, "sound", "提示音", true, sound_on, None::<&str>)?;

    let sep = PredefinedMenuItem::separator(app)?;
    let sep2 = PredefinedMenuItem::separator(app)?;
    let settings_item = MenuItem::with_id(app, "settings", "设置…", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "退出 Claude Pet", true, None::<&str>)?;

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
        .invoke_handler(tauri::generate_handler![
            get_view,
            resize_pet,
            get_settings,
            apply_prefs,
            set_autostart,
            preview_sound,
            set_hooks,
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
            if let Ok(mut p) = prefs.lock() {
                *p = persist::load_prefs(&handle);
                window = p.discover_window();
                eprintln!(
                    "[claude-pet] prefs: muted={} sound={} window={}min",
                    p.muted, p.sound, p.discover_window_minutes
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
                restore_position(&win, &handle);
                let _ = win.show();
                let _ = win.set_always_on_top(true);

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

            spawn_server(handle.clone(), store.clone(), prefs.clone());
            spawn_discovery(handle, store.clone(), window);
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("Claude Pet 启动失败");
}
