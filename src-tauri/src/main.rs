// Claude Pet —— 常驻置顶的 Claude Code 状态挂件
//
// 数据流：Claude Code 的 hook（type: "http"）POST 事件 JSON 到 127.0.0.1:47800，
// 这里解析成状态后 emit 给 WebView 渲染。
//
// 为什么用 HTTP 而不是状态文件：官方文档明确「连接失败或超时 = 非阻塞错误，执行继续」，
// 所以挂件没开的时候 hook 静默失败，完全不影响 Claude Code 干活。

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Emitter, Manager, WebviewWindow};
use tauri_plugin_autostart::{ManagerExt as AutostartExt, MacosLauncher};

/// 挂件监听端口。改这里的话记得同步改 ~/.claude/settings.json 里的 hook url。
const PORT: u16 = 47800;

// ── 会话状态 ─────────────────────────────────────────────────

#[derive(Clone, Debug, Serialize)]
struct Session {
    project: String,
    state: String,
    detail: String,
    updated_ms: u128,
}

/// 聚合后真正显示在挂件上的东西。
#[derive(Clone, Debug, Serialize)]
struct PetView {
    state: String,
    project: String,
    detail: String,
    sessions: usize,
    waiting: usize,
}

type Store = Arc<Mutex<HashMap<String, Session>>>;

/// 窗口最新位置。拖动时只更新内存，由后台线程节流落盘 ——
/// Moved 事件在一次拖动里会触发几百次，每次都写文件太糙。
type PosState = Arc<Mutex<Option<(i32, i32)>>>;

fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

/// 多会话时谁上镜：等你操作的优先，其次在干活的，最后才是空闲。
fn priority(state: &str) -> u8 {
    match state {
        "waiting-permission" => 4,
        "waiting-input" => 3,
        "working" => 2,
        "done" => 1,
        _ => 0,
    }
}

/// 从 cwd 取最后一段当项目名。Windows 反斜杠和 POSIX 斜杠都要吃。
fn project_of(cwd: &str) -> String {
    cwd.replace('\\', "/")
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or("")
        .to_string()
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

fn handle_event(store: &Store, v: &Value) {
    let sid = v
        .get("session_id")
        .and_then(Value::as_str)
        .unwrap_or("unknown")
        .to_string();

    // 会话结束就从表里摘掉，否则关掉的终端会一直挂在计数里
    if v.get("hook_event_name").and_then(Value::as_str) == Some("SessionEnd") {
        if let Ok(mut m) = store.lock() {
            m.remove(&sid);
        }
        return;
    }

    if let Some((state, detail)) = classify(v) {
        let project = project_of(v.get("cwd").and_then(Value::as_str).unwrap_or(""));
        if let Ok(mut m) = store.lock() {
            m.insert(
                sid,
                Session {
                    project,
                    state,
                    detail,
                    updated_ms: now_ms(),
                },
            );
        }
    }
}

fn aggregate(store: &Store) -> PetView {
    let map = match store.lock() {
        Ok(m) => m,
        Err(_) => {
            return PetView {
                state: "idle".into(),
                project: String::new(),
                detail: "状态读取失败".into(),
                sessions: 0,
                waiting: 0,
            }
        }
    };

    let waiting = map
        .values()
        .filter(|s| s.state.starts_with("waiting"))
        .count();

    // 优先级相同时取最近更新的那个
    let best = map.values().max_by(|a, b| {
        priority(&a.state)
            .cmp(&priority(&b.state))
            .then(a.updated_ms.cmp(&b.updated_ms))
    });

    match best {
        Some(s) => PetView {
            state: s.state.clone(),
            project: s.project.clone(),
            detail: s.detail.clone(),
            sessions: map.len(),
            waiting,
        },
        None => PetView {
            state: "idle".into(),
            project: String::new(),
            detail: "没有活动会话".into(),
            sessions: 0,
            waiting: 0,
        },
    }
}

#[tauri::command]
fn get_state(store: tauri::State<Store>) -> PetView {
    aggregate(&store)
}

// ── HTTP 监听 ────────────────────────────────────────────────

fn spawn_server(app: AppHandle, store: Store) {
    std::thread::spawn(move || {
        let server = match tiny_http::Server::http(("127.0.0.1", PORT)) {
            Ok(s) => s,
            Err(e) => {
                // 这里刻意用英文：Windows 控制台默认 GBK 代码页，
                // 中文会变乱码，而这恰好是最需要看清的一条错误。
                eprintln!("[claude-pet] failed to bind 127.0.0.1:{PORT}: {e}");
                eprintln!("[claude-pet] port in use? another pet instance may be running");
                return;
            }
        };
        eprintln!("[claude-pet] listening on http://127.0.0.1:{PORT}");

        for mut req in server.incoming_requests() {
            let mut body = String::new();
            let _ = req.as_reader().read_to_string(&mut body);

            if let Ok(v) = serde_json::from_str::<Value>(&body) {
                handle_event(&store, &v);
                let _ = app.emit("pet://state", aggregate(&store));
            }

            // 必须回 2xx 空 body —— 官方约定这等价于 exit 0 无输出，
            // 不会给 Claude Code 注入任何上下文。
            let _ = req.respond(tiny_http::Response::empty(200));
        }
    });
}

// ── 窗口位置持久化 ───────────────────────────────────────────

#[derive(Serialize, Deserialize)]
struct SavedPos {
    x: i32,
    y: i32,
}

fn pos_file(app: &AppHandle) -> Option<PathBuf> {
    app.path()
        .app_config_dir()
        .ok()
        .map(|d| d.join("window-position.json"))
}

fn write_pos(app: &AppHandle, x: i32, y: i32) {
    let Some(path) = pos_file(app) else { return };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(s) = serde_json::to_string(&SavedPos { x, y }) {
        let _ = std::fs::write(path, s);
    }
}

fn read_pos(app: &AppHandle) -> Option<SavedPos> {
    let path = pos_file(app)?;
    let s = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&s).ok()
}

/// 保存的坐标必须落在某个「当前可用」的显示器里才能用。
/// 笔记本插拔扩展屏后旧坐标会把挂件扔到看不见的地方 —— 那种情况下
/// 用户只会以为程序坏了，因为窗口没有标题栏也不在任务栏，找不回来。
fn position_is_visible(win: &WebviewWindow, x: i32, y: i32) -> bool {
    let Ok(monitors) = win.available_monitors() else {
        return false;
    };
    monitors.iter().any(|m| {
        let p = m.position();
        let s = m.size();
        // 留 8px 容差，避免贴边时的边界抖动
        x >= p.x - 8
            && y >= p.y - 8
            && x < p.x + s.width as i32
            && y < p.y + s.height as i32
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
    if let Some(saved) = read_pos(app) {
        if position_is_visible(win, saved.x, saved.y) {
            let _ = win.set_position(tauri::PhysicalPosition {
                x: saved.x,
                y: saved.y,
            });
            return;
        }
        eprintln!(
            "[claude-pet] saved position ({}, {}) is off-screen, using default",
            saved.x, saved.y
        );
    }
    position_bottom_right(win);
}

// ── 托盘 ─────────────────────────────────────────────────────

fn build_tray(app: &tauri::App) -> tauri::Result<()> {
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
    let version_item =
        MenuItem::with_id(app, "version", format!("Claude Pet v{version}"), false, None::<&str>)?;
    let autostart_item =
        CheckMenuItem::with_id(app, "autostart", "开机自启", true, autostart_on, None::<&str>)?;
    let sep = PredefinedMenuItem::separator(app)?;
    let quit = MenuItem::with_id(app, "quit", "退出 Claude Pet", true, None::<&str>)?;

    let menu = Menu::with_items(app, &[&version_item, &sep, &autostart_item, &quit])?;

    let check_handle = autostart_item.clone();
    let mut tray = TrayIconBuilder::new()
        .menu(&menu)
        .tooltip(format!("Claude Pet v{version}"))
        .on_menu_event(move |app, event| match event.id().as_ref() {
            "quit" => app.exit(0),
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
            _ => {}
        });

    if let Some(icon) = app.default_window_icon() {
        tray = tray.icon(icon.clone());
    }
    tray.build(app)?;
    Ok(())
}

// ── 无头 CLI ─────────────────────────────────────────────────

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

fn main() {
    let store: Store = Arc::new(Mutex::new(HashMap::new()));
    let pos: PosState = Arc::new(Mutex::new(None));

    tauri::Builder::default()
        .plugin(tauri_plugin_autostart::init(
            // Windows 上走 HKCU\...\Run；LaunchAgent 只影响 macOS
            MacosLauncher::LaunchAgent,
            None,
        ))
        .manage(store.clone())
        .invoke_handler(tauri::generate_handler![get_state])
        .setup(move |app| {
            // 无头开关自启：安装脚本要用，我也用它来验证 autostart 真的写了注册表。
            // 必须从「安装后的」exe 调用 —— 插件注册的是当前 exe 的路径。
            if let Some(code) = handle_autostart_cli(app) {
                std::process::exit(code);
            }

            // 窗口没有标题栏也不在任务栏，托盘是唯一的退出入口 —— 不能省。
            build_tray(app)?;

            let handle = app.handle().clone();

            if let Some(win) = app.get_webview_window("pet") {
                // 窗口在 config 里是 visible:false，先摆好位置再 show，
                // 否则会先在默认位置闪一下再跳到恢复的位置。
                restore_position(&win, &handle);
                let _ = win.show();
                let _ = win.set_always_on_top(true);

                // 拖动时只记内存，不碰磁盘
                let pos_for_move = pos.clone();
                win.on_window_event(move |event| {
                    if let tauri::WindowEvent::Moved(p) = event {
                        if let Ok(mut slot) = pos_for_move.lock() {
                            *slot = Some((p.x, p.y));
                        }
                    }
                });

                // 一个后台线程干两件事：
                //  1. 重新抬升置顶 —— Windows 上独占全屏程序和 UAC 安全桌面会抢走
                //     topmost。set_always_on_top 是幂等的，不抢焦点。
                //  2. 落盘位置 —— 只在变化时写，最多丢 5 秒内的移动。
                let w = win.clone();
                let pos_for_flush = pos.clone();
                let app_for_flush = handle.clone();
                std::thread::spawn(move || {
                    let mut last_written: Option<(i32, i32)> = None;
                    loop {
                        std::thread::sleep(Duration::from_secs(5));
                        let _ = w.set_always_on_top(true);

                        let current = pos_for_flush.lock().ok().and_then(|g| *g);
                        if let Some((x, y)) = current {
                            if last_written != Some((x, y)) {
                                write_pos(&app_for_flush, x, y);
                                last_written = Some((x, y));
                            }
                        }
                    }
                });
            }

            spawn_server(handle, store.clone());
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("Claude Pet 启动失败");
}
