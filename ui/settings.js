// 设置窗口。
//
// 每一项改完立刻落盘并生效，没有「保存」按钮 —— 设置项都是独立开关，
// 攒着一起提交只会让人怀疑到底存没存。

const el = (id) => document.getElementById(id);

const ui = {
  autostart: el("autostart"),
  window: el("window"),
  windowHint: el("windowHint"),
  soundOn: el("soundOn"),
  sound: el("sound"),
  preview: el("preview"),
  hooksStatus: el("hooksStatus"),
  installHooks: el("installHooks"),
  uninstallHooks: el("uninstallHooks"),
  claudeSettings: el("claudeSettings"),
  version: el("version"),
  port: el("port"),
  configDir: el("configDir"),
  repo: el("repo"),
  toast: el("toast"),
};

let invoke = null;
/// 当前偏好。apply() 每次都发完整对象，所以这里必须始终是最新值。
let prefs = null;
let toastTimer = null;

function toast(msg, isError) {
  ui.toast.textContent = msg;
  ui.toast.classList.toggle("err", !!isError);
  ui.toast.hidden = false;
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => {
    ui.toast.hidden = true;
  }, isError ? 4000 : 1600);
}

/// 把当前 prefs 整体推给 Rust。Rust 侧会 sanitise，所以越界值不会落盘。
async function apply(what) {
  try {
    await invoke("apply_prefs", { incoming: prefs });
    toast(what ? `${what}已保存` : "已保存");
  } catch (e) {
    toast(`保存失败: ${e}`, true);
  }
}

function renderHooks(installed, total) {
  ui.hooksStatus.textContent = `${installed}/${total}`;
  ui.hooksStatus.classList.toggle("ok", total > 0 && installed === total);
  ui.hooksStatus.classList.toggle("warn", installed !== total);
  // 全装好了就没必要再点安装；一个都没装时卸载也没意义
  ui.installHooks.disabled = total > 0 && installed === total;
  ui.uninstallHooks.disabled = installed === 0;
}

async function refreshHooks() {
  try {
    const v = await invoke("get_settings");
    renderHooks(v.hooks_installed, v.hooks_total);
  } catch (e) {
    ui.hooksStatus.textContent = "读取失败";
  }
}

function fill(v) {
  prefs = v.prefs;

  ui.autostart.checked = v.autostart;

  ui.window.min = v.window_min;
  ui.window.max = v.window_max;
  ui.window.value = prefs.discover_window_minutes;
  ui.windowHint.textContent =
    `启动时只把这段时间内活动过的会话恢复出来（${v.window_min}–${v.window_max} 分钟）。` +
    `改完会立刻用新窗口重扫一遍。`;

  ui.soundOn.checked = !prefs.muted;
  ui.sound.textContent = "";
  for (const s of v.sounds) {
    const o = document.createElement("option");
    o.value = s;
    o.textContent = s;
    ui.sound.appendChild(o);
  }
  ui.sound.value = prefs.sound;
  ui.sound.disabled = prefs.muted;
  ui.preview.disabled = prefs.muted;

  renderHooks(v.hooks_installed, v.hooks_total);

  ui.claudeSettings.textContent = v.about.claude_settings;
  ui.version.textContent = `v${v.about.version}`;
  ui.port.textContent = `127.0.0.1:${v.about.port}`;
  ui.configDir.textContent = v.about.config_dir;
  ui.repo.textContent = v.about.repo;
}

function wire() {
  ui.autostart.addEventListener("change", async () => {
    try {
      await invoke("set_autostart", { enabled: ui.autostart.checked });
      toast(ui.autostart.checked ? "已开启开机自启" : "已关闭开机自启");
    } catch (e) {
      // 写注册表可能失败，此时把勾选状态改回去，别让界面骗人
      ui.autostart.checked = !ui.autostart.checked;
      toast(`设置失败: ${e}`, true);
    }
  });

  // 用 change 而不是 input：时间窗改动会触发重扫，
  // 按 input 的话每敲一个数字就扫一遍。
  ui.window.addEventListener("change", () => {
    const n = parseInt(ui.window.value, 10);
    if (!Number.isFinite(n)) {
      ui.window.value = prefs.discover_window_minutes;
      return;
    }
    prefs.discover_window_minutes = n;
    apply("时间窗").then(refreshAfterClamp);
  });

  ui.soundOn.addEventListener("change", () => {
    prefs.muted = !ui.soundOn.checked;
    ui.sound.disabled = prefs.muted;
    ui.preview.disabled = prefs.muted;
    apply("提示音");
    // 打开时试听一声，好知道听到的是什么
    if (!prefs.muted) invoke("preview_sound", { alias: prefs.sound }).catch(() => {});
  });

  ui.sound.addEventListener("change", () => {
    prefs.sound = ui.sound.value;
    apply("声音");
    invoke("preview_sound", { alias: prefs.sound }).catch(() => {});
  });

  ui.preview.addEventListener("click", () => {
    invoke("preview_sound", { alias: ui.sound.value }).catch(() => {});
  });

  ui.installHooks.addEventListener("click", () => setHooks(true));
  ui.uninstallHooks.addEventListener("click", () => setHooks(false));
}

/// Rust 会把越界的时间窗 clamp 掉，所以要回读一次让界面显示真实落盘的值 ——
/// 否则输入框里留着 9999 而实际存的是 1440，界面在撒谎。
async function refreshAfterClamp() {
  try {
    const v = await invoke("get_settings");
    prefs = v.prefs;
    ui.window.value = prefs.discover_window_minutes;
  } catch (e) {
    /* 保持现状 */
  }
}

async function setHooks(install) {
  ui.installHooks.disabled = true;
  ui.uninstallHooks.disabled = true;
  try {
    const msg = await invoke("set_hooks", { install });
    toast(msg);
  } catch (e) {
    toast(`${install ? "安装" : "卸载"}失败: ${e}`, true);
  }
  await refreshHooks();
}

async function init() {
  const tauri = window.__TAURI__;
  if (!tauri) {
    document.body.textContent = "__TAURI__ 未注入，设置窗口无法工作";
    return;
  }
  invoke = (tauri.core && tauri.core.invoke) || tauri.invoke;

  try {
    fill(await invoke("get_settings"));
  } catch (e) {
    document.body.textContent = `读取设置失败: ${e}`;
    return;
  }
  wire();
}

init();
