//! 双击宠物跳回对应项目的 IDE。
//!
//! 走的是 Open Island 同样的「workspace 激活」路线：拿会话的 cwd 调 IDE 的 CLI。
//! 终端的 tab 级跳回不在这里 —— 那条在 Windows 上有结构性障碍，另有 issue 调研。

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

pub struct Editor {
    /// 存进 prefs 的键
    pub key: &'static str,
    pub label: &'static str,
    /// PATH 里的可执行名（不带扩展名）
    pub exe: &'static str,
    /// 复用已有窗口的参数。没有就每次开新窗口。
    pub reuse: Option<&'static str>,
}

/// 探测顺序即数组顺序（`editor = "auto"` 时取第一个找得到的）。
///
/// Cursor 排在 VS Code 前面：装了 Cursor 的人基本是拿它当主编辑器，
/// 而 Cursor 是 VS Code 的 fork，两个 CLI 会同时存在。
pub const EDITORS: &[Editor] = &[
    Editor { key: "cursor", label: "Cursor", exe: "cursor", reuse: Some("-r") },
    Editor { key: "code", label: "VS Code", exe: "code", reuse: Some("-r") },
    Editor { key: "windsurf", label: "Windsurf", exe: "windsurf", reuse: Some("-r") },
    Editor { key: "idea", label: "IntelliJ IDEA", exe: "idea64", reuse: None },
    Editor { key: "webstorm", label: "WebStorm", exe: "webstorm64", reuse: None },
    Editor { key: "pycharm", label: "PyCharm", exe: "pycharm64", reuse: None },
    Editor { key: "goland", label: "GoLand", exe: "goland64", reuse: None },
    Editor { key: "rustrover", label: "RustRover", exe: "rustrover64", reuse: None },
];

/// 自己实现 which，而不是 spawn 一个 `where.exe`。
///
/// 拿到完整路径有两个好处：启动时不再依赖 PATH 解析，而且能按扩展名判断
/// 要不要经 shell —— `code` / `cursor` 在 Windows 上是 `.cmd` shim，
/// Rust 的 `Command` 直接调 CreateProcessW，执行 `.cmd` 会失败。
fn which(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    // PATHEXT 决定要试哪些扩展名；给个保守的兜底
    let pathext = std::env::var("PATHEXT").unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".to_string());

    for dir in std::env::split_paths(&path) {
        if dir.as_os_str().is_empty() {
            continue;
        }
        // 先试带扩展名的，再试原名（有人把 PATH 里放的就是全名）
        for ext in pathext.split(';').filter(|e| !e.is_empty()) {
            let candidate = dir.join(format!("{name}{ext}"));
            if candidate.is_file() {
                return Some(candidate);
            }
        }
        let bare = dir.join(name);
        if bare.is_file() {
            return Some(bare);
        }
    }
    None
}

fn needs_shell(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase())
            .as_deref(),
        Some("cmd") | Some("bat")
    )
}

#[derive(serde::Serialize)]
pub struct Available {
    pub key: String,
    pub label: String,
    pub path: String,
}

/// 本机装了哪些。设置窗口用它填下拉框。
pub fn available() -> Vec<Available> {
    EDITORS
        .iter()
        .filter_map(|e| {
            which(e.exe).map(|p| Available {
                key: e.key.to_string(),
                label: e.label.to_string(),
                path: p.display().to_string(),
            })
        })
        .collect()
}

/// 在编辑器里打开 `dir`。
///
/// `preferred` 为 `"auto"` 或找不到时，按 `EDITORS` 顺序取第一个装了的。
/// 返回实际用的编辑器 label。
pub fn open(dir: &str, preferred: &str) -> Result<String, String> {
    if !Path::new(dir).is_dir() {
        return Err(format!("目录不存在: {dir}"));
    }

    // 指定了就只试它 —— 用户明确选了 VS Code，静默回落到 Cursor 是在骗人
    let candidates: Vec<&Editor> = if preferred == "auto" {
        EDITORS.iter().collect()
    } else {
        EDITORS.iter().filter(|e| e.key == preferred).collect()
    };
    if candidates.is_empty() {
        return Err(format!("未知的编辑器: {preferred}"));
    }

    for ed in candidates {
        let Some(exe) = which(ed.exe) else { continue };

        let mut cmd = if needs_shell(&exe) {
            // .cmd / .bat 必须经 cmd.exe。/d 跳过 AutoRun 注册表项 ——
            // 有些机器在那儿挂了脚本，会污染我们的调用。
            let mut c = Command::new("cmd");
            c.arg("/d").arg("/c").arg(&exe);
            c
        } else {
            Command::new(&exe)
        };

        if let Some(flag) = ed.reuse {
            cmd.arg(flag);
        }
        cmd.arg(dir);

        // 必须切断 stdio 继承。否则编辑器会一直持有我们继承下去的管道句柄，
        // 任何等待挂件输出结束的调用方（比如 Start-Process -Wait 配合
        // stderr 重定向）都会一直等到编辑器关掉为止 —— 实测会挂死。
        cmd.stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());

        match cmd.spawn() {
            Ok(_) => return Ok(ed.label.to_string()),
            Err(e) => eprintln!("[claude-pet] {} spawn failed: {e}", ed.label),
        }
    }

    Err(if preferred == "auto" {
        "没找到任何支持的编辑器（Cursor / VS Code / JetBrains）".into()
    } else {
        format!("{preferred} 不在 PATH 上")
    })
}
