//! 往 `~/.claude/settings.json` 里装 / 卸挂件需要的 hook。
//!
//! 这是新用户上手路径上唯一还要手工编辑 JSON 的一步，也是最容易出错的一步 ——
//! 漏个逗号整份配置就失效，而 Claude Code 不会明确告诉你为什么。
//!
//! 三条硬约束：
//!
//! 1. **合并而不是覆盖。** 用户的 `statusLine`、`enabledPlugins`、别人装的 hook
//!    都不能动。往返用 serde_json 的 `preserve_order` + 2 空格 pretty 打印，
//!    实测对真实 settings.json 是逐字节一致的。
//! 2. **幂等。** 重复装不产生重复条目，判重看 hook 的 url。
//! 3. **只卸自己的。** 卸载只摘掉指向本挂件端口的 http hook。特别注意
//!    `Notification` 事件下通常还有一条**别的** hook（toast 脚本，`command` 类型、
//!    matcher 更窄），绝不能连它一起删。

use serde_json::{json, Map, Value};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use tauri::{AppHandle, Manager};

/// 判重和卸载都以这个 url 为标识。改端口的话这里和 `PORT` 要一起改。
const HOOK_URL: &str = "http://127.0.0.1:47800/";

/// 要装的事件和它们的 matcher。`None` 表示该事件不需要 matcher。
///
/// `Notification` 的 matcher 覆盖四种通知类型 —— 挂件要靠 `idle_prompt` 和
/// `agent_completed` 才能显示空闲/完成状态，砍窄了会丢状态。
const OUR_HOOKS: &[(&str, Option<&str>)] = &[
    ("UserPromptSubmit", None),
    ("PreToolUse", Some("*")),
    (
        "Notification",
        Some("permission_prompt|agent_needs_input|idle_prompt|agent_completed"),
    ),
    ("Stop", None),
    ("SessionStart", None),
    ("SessionEnd", None),
];

#[derive(Debug, Default)]
pub struct Report {
    pub changed: bool,
    /// 装上的事件数 / 摘掉的 hook 数
    pub touched: usize,
    pub backup: Option<PathBuf>,
}

pub fn settings_path(app: &AppHandle) -> Option<PathBuf> {
    if let Ok(dir) = std::env::var("CLAUDE_CONFIG_DIR") {
        let t = dir.trim();
        if !t.is_empty() {
            return Some(PathBuf::from(t).join("settings.json"));
        }
    }
    app.path()
        .home_dir()
        .ok()
        .map(|h| h.join(".claude").join("settings.json"))
}

fn our_hook() -> Value {
    json!({
        "type": "http",
        "url": HOOK_URL,
        "async": true,
        "timeout": 5
    })
}

fn is_ours(hook: &Value) -> bool {
    hook.get("type").and_then(Value::as_str) == Some("http")
        && hook.get("url").and_then(Value::as_str) == Some(HOOK_URL)
}

/// 序列化成与用户原文件同款的格式：2 空格 + 末尾换行。
fn encode(root: &Value) -> Option<String> {
    let mut s = serde_json::to_string_pretty(root).ok()?;
    s.push('\n');
    Some(s)
}

fn read_root(path: &PathBuf) -> Result<(Value, String), String> {
    match std::fs::read_to_string(path) {
        Ok(raw) => {
            let root: Value = serde_json::from_str(&raw)
                .map_err(|e| format!("{} is not valid JSON: {e}", path.display()))?;
            if !root.is_object() {
                return Err(format!("{} top level is not an object", path.display()));
            }
            Ok((root, raw))
        }
        // 文件不存在是正常情况（全新机器），当成空配置
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            Ok((Value::Object(Map::new()), String::new()))
        }
        Err(e) => Err(format!("cannot read {}: {e}", path.display())),
    }
}

/// 只在内容真的变了才写，顺带在写之前备份。
///
/// 「没变就不写」不只是省一次 IO：重复安装因此也不会刷出一堆备份文件。
fn commit(path: &PathBuf, original: &str, root: &Value) -> Result<(bool, Option<PathBuf>), String> {
    let Some(encoded) = encode(root) else {
        return Err("failed to serialise settings".into());
    };
    if encoded == original {
        return Ok((false, None));
    }

    // 备份原文件。用 epoch 毫秒而不是可读时间：不想为一个备份文件名引入 chrono，
    // 而毫秒数同样可排序、不会撞名。
    let mut backup = None;
    if !original.is_empty() {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        let bak = path.with_file_name(format!(
            "{}.bak-{stamp}",
            path.file_name().and_then(|n| n.to_str()).unwrap_or("settings.json")
        ));
        std::fs::write(&bak, original).map_err(|e| format!("backup failed: {e}"))?;
        backup = Some(bak);
    }

    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    // 临时文件 + rename：写一半被中断不该毁掉用户的配置
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, &encoded).map_err(|e| format!("write failed: {e}"))?;
    if std::fs::rename(&tmp, path).is_err() {
        std::fs::write(path, &encoded).map_err(|e| format!("write failed: {e}"))?;
        let _ = std::fs::remove_file(&tmp);
    }
    Ok((true, backup))
}

/// 这个事件下已经有指向我们的 hook 了吗？
fn event_has_ours(event_value: &Value) -> bool {
    event_value
        .as_array()
        .map(|entries| {
            entries.iter().any(|entry| {
                entry
                    .get("hooks")
                    .and_then(Value::as_array)
                    .map(|hs| hs.iter().any(is_ours))
                    .unwrap_or(false)
            })
        })
        .unwrap_or(false)
}

pub fn install(app: &AppHandle) -> Result<Report, String> {
    let path = settings_path(app).ok_or("cannot locate settings.json")?;
    let (mut root, original) = read_root(&path)?;

    let obj = root.as_object_mut().ok_or("settings root is not an object")?;
    let hooks = obj
        .entry("hooks".to_string())
        .or_insert_with(|| Value::Object(Map::new()));
    let hooks = hooks
        .as_object_mut()
        .ok_or("settings.hooks exists but is not an object")?;

    let mut touched = 0;
    for (event, matcher) in OUR_HOOKS {
        let entry_list = hooks
            .entry((*event).to_string())
            .or_insert_with(|| Value::Array(Vec::new()));

        // 幂等：这个事件下已经有我们的 hook 就跳过
        if event_has_ours(entry_list) {
            continue;
        }

        let arr = entry_list
            .as_array_mut()
            .ok_or_else(|| format!("settings.hooks.{event} is not an array"))?;

        // 我们的 hook 总是放在自己的条目里，不塞进用户已有的条目 ——
        // 这样卸载时不需要从共享数组里做外科手术。
        let mut entry = Map::new();
        if let Some(m) = matcher {
            entry.insert("matcher".into(), json!(m));
        }
        entry.insert("hooks".into(), json!([our_hook()]));
        arr.push(Value::Object(entry));
        touched += 1;
    }

    let (changed, backup) = commit(&path, &original, &root)?;
    Ok(Report { changed, touched, backup })
}

pub fn uninstall(app: &AppHandle) -> Result<Report, String> {
    let path = settings_path(app).ok_or("cannot locate settings.json")?;
    let (mut root, original) = read_root(&path)?;
    if original.is_empty() {
        return Ok(Report::default());
    }

    let mut touched = 0;
    let mut drop_hooks_key = false;

    if let Some(hooks) = root
        .as_object_mut()
        .and_then(|o| o.get_mut("hooks"))
        .and_then(Value::as_object_mut)
    {
        let mut empty_events = Vec::new();

        for (event, entry_list) in hooks.iter_mut() {
            let Some(entries) = entry_list.as_array_mut() else {
                continue;
            };

            for entry in entries.iter_mut() {
                if let Some(hs) = entry.get_mut("hooks").and_then(Value::as_array_mut) {
                    let before = hs.len();
                    // 只摘我们自己的。同一个条目里可能还有别人的 hook ——
                    // 用户手工把我们的 hook 合进了自己的条目时就是这种情况。
                    hs.retain(|h| !is_ours(h));
                    touched += before - hs.len();
                }
            }

            // hooks 数组被清空的条目整个丢掉，否则会留下 {"matcher": "..."} 这种空壳
            entries.retain(|e| {
                e.get("hooks")
                    .and_then(Value::as_array)
                    .map(|hs| !hs.is_empty())
                    .unwrap_or(true) // 结构不认识的条目一律保留，不是我们的东西
            });

            if entries.is_empty() {
                empty_events.push(event.clone());
            }
        }

        for e in empty_events {
            hooks.remove(&e);
        }
        // hooks 全空就把这个键也去掉，让卸载成为安装的真正逆操作
        drop_hooks_key = hooks.is_empty();
    }

    if drop_hooks_key {
        if let Some(o) = root.as_object_mut() {
            o.remove("hooks");
        }
    }

    let (changed, backup) = commit(&path, &original, &root)?;
    Ok(Report { changed, touched, backup })
}

/// 返回 (已装事件数, 应装事件数)。
pub fn status(app: &AppHandle) -> Result<(usize, usize), String> {
    let path = settings_path(app).ok_or("cannot locate settings.json")?;
    let (root, original) = read_root(&path)?;
    if original.is_empty() {
        return Ok((0, OUR_HOOKS.len()));
    }
    let installed = root
        .get("hooks")
        .and_then(Value::as_object)
        .map(|hooks| {
            OUR_HOOKS
                .iter()
                .filter(|(event, _)| hooks.get(*event).map(event_has_ours).unwrap_or(false))
                .count()
        })
        .unwrap_or(0);
    Ok((installed, OUR_HOOKS.len()))
}
