// 界面文案。挂件和设置窗口共用这一份。
//
// 用法：
//   HTML 里给元素加 data-i18n="key"（纯文本）或 data-i18n-html="key"（含标签），
//   加载时调 applyI18n() 一次性替换；JS 里动态拼的文案用 t("key", {...})。
//
// 语言从 Rust 侧的 get_settings().lang_code 拿（那边处理了 "auto" → 系统语言），
// 前端不自己猜 —— 两边各猜一次必然会有不一致的时候。
//
// 整个文件包在 IIFE 里，只往外暴露 window.I18N。**别去掉这层包裹**：
// 经典 script 共用一个全局作用域，这里顶层的 `function t()` 会创建全局 `t`，
// 而 app.js / settings.js 里都有 `const t = ...` —— 撞上就是解析期
// SyntaxError，整个文件一行都不执行，连 window.onerror 都注册不上，
// 表现为「界面空白但没有任何报错」。单文件 node --check 查不出这种冲突。

(function () {
  const DICT = {
  zh: {
    // ── 挂件 ──
    "pet.booted": "挂件已启动",
    "pet.waitingEvents": "等 Claude Code 事件",
    "pet.toggleAria": "展开或收起",
    "pet.noSessions": "没有活动会话",
    "pet.sessions": "{n} 个会话",
    "pet.sessionsWaiting": "{n} 个会话 · {w} 个在等你",
    "pet.dblclickHint": "双击在编辑器中打开",
    "pet.noProjectDir": "{agent} 是常驻服务，没有项目目录",
    "agent.claude-code": "Claude Code",
    "agent.codex": "Codex",
    "agent.hermes": "Hermes",
    "agent.openclaw": "OpenClaw",
    "pet.openedIn": "已在 {editor} 中打开",
    "pet.nothingWaiting": "没有会话在等你",
    "pet.allow": "允许",
    "pet.deny": "拒绝",
    "pet.allowTitle": "允许这次工具调用",
    "pet.denyTitle": "拒绝这次工具调用",
    "pet.noTauri": "__TAURI__ 未注入",
    "pet.subscribeFailed": "订阅失败: {err}",
    "pet.readFailed": "读取状态失败: {err}",

    // 状态名
    "state.working": "干活中",
    "state.waiting-permission": "要你点允许",
    "state.waiting-input": "在等你回话",
    "state.idle": "空闲",
    "state.done": "完成了",

    // ── 设置窗口 ──
    "set.title": "Claude Pet 设置",
    "set.general": "通用",
    "set.agents": "盯哪些 agent",
    "set.agentsHint":
      "默认只开 Claude Code。<strong>只有 Claude Code 有 hook</strong>，状态是实时推送的；其余靠轮询本地文件，有几秒延迟。Hermes 和 OpenClaw 是常驻服务、没有项目概念，所以各自只有一只宠物、显示「在不在跑」。没装的灰掉但仍列出，装上之后开关就在这儿。",
    "set.agentMissing": "本机没找到",
    "set.agentGateway": "只有一只宠物",
    "set.agentPolled": "轮询，有延迟",
    "set.autostart": "开机自启",
    "set.autostartHint":
      "注册到 <code>HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Run</code>，记录的是当前 exe 的路径。",
    "set.window": "会话发现时间窗",
    "set.minutes": "分钟",
    "set.windowHint":
      "启动时只把这段时间内活动过的会话恢复出来（{min}–{max} 分钟）。改完会立刻用新窗口重扫一遍。",
    "set.editor": "双击宠物时打开",
    "set.editorAuto": "自动（{first}）",
    "set.editorAutoNone": "自动（未找到编辑器）",
    "set.editorFound": "双击宠物在对应项目里打开。已找到：{list}。",
    "set.editorNone":
      "PATH 上没找到 Cursor / VS Code / JetBrains 的命令行工具，双击不会有反应。",
    "set.lang": "界面语言",
    "set.langAuto": "跟随系统",
    "set.langHint": "改完立即生效。托盘菜单要重启挂件才会跟着变。",
    "set.position": "挂件位置",
    "pos.bottom-right": "右下角",
    "pos.top-center": "顶部居中",
    "pos.free": "跟随上次拖动",
    "set.positionHint":
      "「右下角」和「顶部居中」是<strong>吸附</strong>模式，位置由屏幕算出来，拖动不留痕；想拖到哪算哪就选「跟随上次拖动」。多显示器下吸附到挂件当前所在的那块屏。",

    "set.sound": "提示音",
    "set.soundOn": "进入等待态时提示",
    "set.soundOnHint": "只在状态「变成」等待态时响一次，同一等待态的重复事件不响。",
    "set.soundPick": "声音",
    "set.preview": "试听",
    "set.soundHint":
      "用的是 Windows 声音方案里已有的系统音，所以跟随你在系统设置里配的声音和音量。",

    "set.shortcuts": "全局快捷键",
    "set.scToggle": "展开 / 收起挂件",
    "set.scNext": "跳到下一个在等你的会话",
    "set.scHint":
      "写法如 <code>Ctrl+Alt+P</code>、<code>Ctrl+Shift+F9</code>。留空表示不注册。被别的程序占用时会在下方提示，且不影响另一个快捷键。",

    "set.hooks": "Hooks",
    "set.hooksStatus": "安装状态",
    "set.install": "安装",
    "set.uninstall": "卸载",
    "set.reading": "读取中…",
    "set.readFail": "读取失败",
    "set.hooksHint":
      "写入 <code>{path}</code>。改动前会备份，只增删指向本挂件端口的条目，其它配置不动。<strong>hook 在会话启动时加载，所以要开一个新的 Claude Code 会话才生效。</strong>",
    "set.permOn": "在挂件上批准权限",
    "set.permMatcher": "拦截哪些工具",
    "set.permHint":
      "匹配到的工具调用会<strong>挂住等你在挂件上点允许/拒绝</strong>，最多 {secs} 秒后交回 Claude Code 自己的权限流程。所以挂件没开或人不在时不会把 Claude Code 卡死，代价是每次多等 {secs} 秒。<br>默认只拦 <code>Bash</code>。填 <code>*</code> 会让每一次工具调用都等你点，通常不是你想要的。<br>另外：<code>permissions.defaultMode</code> 为 <code>bypassPermissions</code> 时 Claude Code 本来就不问权限，此功能不触发。",

    "set.about": "关于",
    "set.version": "版本",
    "set.port": "监听端口",
    "set.configDir": "配置目录",
    "set.repo": "仓库",
    "set.checkUpdates": "启动时检查新版本",
    "set.checkNow": "立即检查",
    "set.checking": "检查中…",
    "set.upToDate": "已是最新（v{cur}）",
    "set.updateFound": "有新版本 v{latest}（当前 v{cur}）",
    "set.updateHow": "升级命令（复制到 PowerShell 执行）：",
    "set.updateFailed": "检查失败：{err}",
    "set.checkUpdatesHint":
      "这是一次对 GitHub 的网络请求，不想联网可以关掉。关掉后仍可点「立即检查」。",

    // 挂件上的新版本提示
    "pet.updateAvailable": "有新版本 v{latest}",

    "set.saved": "已保存",
    "set.savedWhat": "{what}已保存",
    "set.saveFailed": "保存失败: {err}",
    "set.autostartOn": "已开启开机自启",
    "set.autostartOff": "已关闭开机自启",
    "set.setFailed": "设置失败: {err}",
    "set.failed": "失败: {err}",
    "set.installFailed": "安装失败: {err}",
    "set.uninstallFailed": "卸载失败: {err}",
    "set.noTauri": "__TAURI__ 未注入，设置窗口无法工作",
    "set.loadFailed": "读取设置失败: {err}",
    "what.window": "时间窗",
    "what.sound": "提示音",
    "what.soundPick": "声音",
    "what.editor": "编辑器",
    "what.shortcut": "快捷键",
    "what.lang": "语言",
    "what.position": "位置",
    "what.agents": "agent 名单",
  },

  en: {
    "pet.booted": "Widget started",
    "pet.waitingEvents": "Waiting for Claude Code events",
    "pet.toggleAria": "Expand or collapse",
    "pet.noSessions": "No active sessions",
    "pet.sessions": "{n} sessions",
    "pet.sessionsWaiting": "{n} sessions · {w} waiting on you",
    "pet.dblclickHint": "Double-click to open in your editor",
    "pet.noProjectDir": "{agent} is a long-running service, it has no project directory",
    "agent.claude-code": "Claude Code",
    "agent.codex": "Codex",
    "agent.hermes": "Hermes",
    "agent.openclaw": "OpenClaw",
    "pet.openedIn": "Opened in {editor}",
    "pet.nothingWaiting": "Nothing is waiting on you",
    "pet.allow": "Allow",
    "pet.deny": "Deny",
    "pet.allowTitle": "Allow this tool call",
    "pet.denyTitle": "Deny this tool call",
    "pet.noTauri": "__TAURI__ was not injected",
    "pet.subscribeFailed": "Subscribe failed: {err}",
    "pet.readFailed": "Could not read state: {err}",

    "state.working": "Working",
    "state.waiting-permission": "Needs your approval",
    "state.waiting-input": "Waiting for your reply",
    "state.idle": "Idle",
    "state.done": "Done",

    "set.title": "Claude Pet Settings",
    "set.general": "General",
    "set.agents": "Agents to watch",
    "set.agentsHint":
      "Only Claude Code by default. <strong>Only Claude Code has hooks</strong>, so its state is pushed live; the others are polled from local files and lag a few seconds. Hermes and OpenClaw are long-running services with no notion of a project, so each gets a single pet showing whether it is up. Ones that aren't installed stay listed but greyed out, so the switch is here once you do install them.",
    "set.agentMissing": "not found",
    "set.agentGateway": "one pet only",
    "set.agentPolled": "polled, lags",
    "set.autostart": "Start with Windows",
    "set.autostartHint":
      "Registered under <code>HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Run</code>, recording the path of the current exe.",
    "set.window": "Session discovery window",
    "set.minutes": "minutes",
    "set.windowHint":
      "On startup, only restore sessions active within this window ({min}–{max} minutes). Changing it re-scans immediately.",
    "set.editor": "Double-click opens",
    "set.editorAuto": "Automatic ({first})",
    "set.editorAutoNone": "Automatic (no editor found)",
    "set.editorFound":
      "Double-click a pet to open its project. Found: {list}.",
    "set.editorNone":
      "No Cursor / VS Code / JetBrains CLI on PATH, so double-click will do nothing.",
    "set.lang": "Language",
    "set.langAuto": "Follow system",
    "set.langHint":
      "Applies immediately. The tray menu follows after a restart.",
    "set.position": "Widget position",
    "pos.bottom-right": "Bottom right",
    "pos.top-center": "Top centre",
    "pos.free": "Wherever you last dragged it",
    "set.positionHint":
      "Bottom right and top centre are <strong>snap</strong> modes: the position is computed from the screen and dragging does not stick. Pick the last option to place it yourself. On multiple monitors it snaps to whichever monitor the widget is currently on.",

    "set.sound": "Sound",
    "set.soundOn": "Play a sound when a session starts waiting",
    "set.soundOnHint":
      "Sounds once when the state *becomes* a waiting one; repeats of the same waiting state stay silent.",
    "set.soundPick": "Sound",
    "set.preview": "Preview",
    "set.soundHint":
      "Uses a sound already in your Windows sound scheme, so it follows the sound and volume you configured there.",

    "set.shortcuts": "Global shortcuts",
    "set.scToggle": "Expand / collapse the widget",
    "set.scNext": "Jump to the next session waiting on you",
    "set.scHint":
      "For example <code>Ctrl+Alt+P</code> or <code>Ctrl+Shift+F9</code>. Leave blank to skip registering. If another program holds it you'll see a warning below, and the other shortcut is unaffected.",

    "set.hooks": "Hooks",
    "set.hooksStatus": "Installed",
    "set.install": "Install",
    "set.uninstall": "Remove",
    "set.reading": "Reading…",
    "set.readFail": "Read failed",
    "set.hooksHint":
      "Written to <code>{path}</code>. Backed up before any change; only entries pointing at this widget's port are added or removed, everything else is left alone. <strong>Hooks are read when a session starts, so open a new Claude Code session for this to take effect.</strong>",
    "set.permOn": "Approve permissions from the widget",
    "set.permMatcher": "Intercept which tools",
    "set.permHint":
      "Matching tool calls are <strong>held until you click Allow/Deny on the widget</strong>, for at most {secs} seconds, after which Claude Code's own permission flow takes over. So a closed widget or an absent human never wedges Claude Code; the cost is an extra {secs}s wait.<br>Defaults to <code>Bash</code> only. <code>*</code> would hold every single tool call, which is usually not what you want.<br>Note: with <code>permissions.defaultMode</code> set to <code>bypassPermissions</code>, Claude Code never asks for permission and this never triggers.",

    "set.about": "About",
    "set.version": "Version",
    "set.port": "Listening on",
    "set.configDir": "Config directory",
    "set.repo": "Repository",
    "set.checkUpdates": "Check for updates on startup",
    "set.checkNow": "Check now",
    "set.checking": "Checking…",
    "set.upToDate": "Up to date (v{cur})",
    "set.updateFound": "v{latest} is available (you have v{cur})",
    "set.updateHow": "Upgrade command (paste into PowerShell):",
    "set.updateFailed": "Check failed: {err}",
    "set.checkUpdatesHint":
      "This makes one network request to GitHub. Turn it off to stay offline; \"Check now\" still works.",

    "pet.updateAvailable": "v{latest} is available",

    "set.saved": "Saved",
    "set.savedWhat": "{what} saved",
    "set.saveFailed": "Save failed: {err}",
    "set.autostartOn": "Start with Windows enabled",
    "set.autostartOff": "Start with Windows disabled",
    "set.setFailed": "Could not apply: {err}",
    "set.failed": "Failed: {err}",
    "set.installFailed": "Install failed: {err}",
    "set.uninstallFailed": "Remove failed: {err}",
    "set.noTauri": "__TAURI__ was not injected; settings cannot work",
    "set.loadFailed": "Could not load settings: {err}",
    "what.window": "Discovery window",
    "what.sound": "Sound",
    "what.soundPick": "Sound",
    "what.editor": "Editor",
    "what.shortcut": "Shortcut",
    "what.lang": "Language",
    "what.position": "Position",
    "what.agents": "agent list",
  },
};

  let LANG = "zh";

  function setLang(code) {
    LANG = DICT[code] ? code : "zh";
  }

  /// 取文案并做 {name} 替换。缺 key 时返回 key 本身 ——
  /// 这样漏翻的地方在界面上一眼就能看出来，而不是变成空白。
  function t(key, vars) {
    const table = DICT[LANG] || DICT.zh;
    let s = table[key];
    if (s === undefined) s = DICT.zh[key] !== undefined ? DICT.zh[key] : key;
    if (vars) {
      for (const [k, v] of Object.entries(vars)) {
        s = s.split(`{${k}}`).join(String(v));
      }
    }
    return s;
  }

  /// 把带 data-i18n / data-i18n-html / data-i18n-title / data-i18n-aria 的元素刷一遍。
  function applyI18n(root) {
    const scope = root || document;
    for (const el of scope.querySelectorAll("[data-i18n]")) {
      el.textContent = t(el.getAttribute("data-i18n"));
    }
    // 含 <code>/<strong> 的文案必须走 innerHTML，用 textContent 会把标签变成字面量
    for (const el of scope.querySelectorAll("[data-i18n-html]")) {
      el.innerHTML = t(el.getAttribute("data-i18n-html"));
    }
    for (const el of scope.querySelectorAll("[data-i18n-title]")) {
      el.title = t(el.getAttribute("data-i18n-title"));
    }
    for (const el of scope.querySelectorAll("[data-i18n-aria]")) {
      el.setAttribute("aria-label", t(el.getAttribute("data-i18n-aria")));
    }
    const titleKey = document.body.getAttribute("data-i18n-doctitle");
    if (titleKey) document.title = t(titleKey);
  }

  window.I18N = { setLang, t, applyI18n, langs: Object.keys(DICT) };
})();
