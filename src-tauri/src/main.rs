// Claude Pet —— 常驻置顶的 Claude Code 状态挂件
//
// 数据流：Claude Code 的 hook（type: "http"）POST 事件 JSON 到 127.0.0.1:47800，
// 这里解析成状态后 emit 给 WebView 渲染。
//
// 为什么用 HTTP 而不是状态文件：官方文档明确「连接失败或超时 = 非阻塞错误，执行继续」，
// 所以挂件没开的时候 hook 静默失败，完全不影响 Claude Code 干活。

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use serde::Serialize;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use tauri::menu::{Menu, MenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{Emitter, Manager};

/// 挂件监听端口。改这里的话记得同步改 ~/.claude/settings.json 里的 hook url。
const PORT: u16 = 47800;

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

fn spawn_server(app: tauri::AppHandle, store: Store) {
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

/// 初始摆到右下角。减掉 56px 粗略避开任务栏 —— Tauri 的 monitor API
/// 给的是整个屏幕尺寸，拿不到工作区，所以这里只能估。
fn position_bottom_right(win: &tauri::WebviewWindow) {
    if let (Ok(Some(monitor)), Ok(size)) = (win.current_monitor(), win.outer_size()) {
        let ms = monitor.size();
        let mp = monitor.position();
        let margin = 24i32;
        let x = mp.x + ms.width as i32 - size.width as i32 - margin;
        let y = mp.y + ms.height as i32 - size.height as i32 - margin - 56;
        let _ = win.set_position(tauri::PhysicalPosition { x, y });
    }
}

fn main() {
    let store: Store = Arc::new(Mutex::new(HashMap::new()));

    tauri::Builder::default()
        .manage(store.clone())
        .invoke_handler(tauri::generate_handler![get_state])
        .setup(move |app| {
            // 窗口没有标题栏也不在任务栏，托盘是唯一的退出入口 —— 不能省。
            let quit = MenuItem::with_id(app, "quit", "退出 Claude Pet", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&quit])?;

            let mut tray = TrayIconBuilder::new()
                .menu(&menu)
                .tooltip("Claude Pet")
                .on_menu_event(|app, event| {
                    if event.id() == "quit" {
                        app.exit(0);
                    }
                });
            if let Some(icon) = app.default_window_icon() {
                tray = tray.icon(icon.clone());
            }
            tray.build(app)?;

            if let Some(win) = app.get_webview_window("pet") {
                position_bottom_right(&win);
                let _ = win.set_always_on_top(true);

                // Windows 上独占全屏程序、UAC 安全桌面会抢走 topmost，
                // 定期重新抬升。set_always_on_top 是幂等的，不会抢焦点。
                let w = win.clone();
                std::thread::spawn(move || loop {
                    std::thread::sleep(Duration::from_secs(5));
                    let _ = w.set_always_on_top(true);
                });
            }

            spawn_server(app.handle().clone(), store.clone());
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("Claude Pet 启动失败");
}
