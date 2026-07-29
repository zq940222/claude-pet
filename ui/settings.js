// 设置窗口。
//
// 每一项改完立刻落盘并生效，没有「保存」按钮 —— 设置项都是独立开关，
// 攒着一起提交只会让人怀疑到底存没存。
//
// 文案全部走 i18n.js 的 t()，HTML 里的静态文本靠 data-i18n 属性。

const el = (id) => document.getElementById(id);
const t = (k, v) => window.I18N.t(k, v);

const ui = {
  autostart: el("autostart"),
  agents: el("agents"),
  window: el("window"),
  windowHint: el("windowHint"),
  editor: el("editor"),
  editorHint: el("editorHint"),
  position: el("position"),
  lang: el("lang"),
  scToggle: el("scToggle"),
  scNext: el("scNext"),
  soundOn: el("soundOn"),
  sound: el("sound"),
  preview: el("preview"),
  hooksStatus: el("hooksStatus"),
  installHooks: el("installHooks"),
  uninstallHooks: el("uninstallHooks"),
  hooksHint: el("hooksHint"),
  permOn: el("permOn"),
  permMatcher: el("permMatcher"),
  permHint: el("permHint"),
  usageBody: el("usageBody"),
  usageHint: el("usageHint"),
  usageRefresh: el("usageRefresh"),
  version: el("version"),
  port: el("port"),
  configDir: el("configDir"),
  repo: el("repo"),
  checkUpdates: el("checkUpdates"),
  checkNow: el("checkNow"),
  updateResult: el("updateResult"),
  updateCmd: el("updateCmd"),
  toast: el("toast"),
};

/// 升级命令。和 README / install.ps1 里的写法保持一致 ——
/// 用 scriptblock 形式是因为 `irm | iex` 传不了参数。
const UPGRADE_CMD =
  "& ([scriptblock]::Create((irm https://raw.githubusercontent.com/zq940222/claude-pet/main/tools/install.ps1))) -Autostart -WireHooks";

let invoke = null;
/// 当前偏好。apply() 每次都发完整对象，所以这里必须始终是最新值。
let prefs = null;
/// AboutInfo，检查更新要用它的 repo 和 version。
let about = null;
let toastTimer = null;

function toast(msg, isError) {
  ui.toast.textContent = msg;
  ui.toast.classList.toggle("err", !!isError);
  ui.toast.hidden = false;
  clearTimeout(toastTimer);
  toastTimer = setTimeout(
    () => {
      ui.toast.hidden = true;
    },
    isError ? 4000 : 1600
  );
}

/// 把当前 prefs 整体推给 Rust。Rust 侧会 sanitise，所以越界值不会落盘。
///
/// 返回的是警告列表而不是错误：一个快捷键被占用不该让别的设置也存不下去，
/// 所以那种情况走 warning，而 toast 要把它显示成红色 —— 否则用户会以为设好了。
async function apply(whatKey) {
  try {
    const warnings = await invoke("apply_prefs", { incoming: prefs });
    if (Array.isArray(warnings) && warnings.length) {
      toast(warnings.join(" / "), true);
    } else {
      toast(whatKey ? t("set.savedWhat", { what: t(whatKey) }) : t("set.saved"));
    }
  } catch (e) {
    toast(t("set.saveFailed", { err: e }), true);
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

function renderPerm(matcher) {
  const on = matcher !== null && matcher !== undefined;
  ui.permOn.checked = on;
  ui.permMatcher.value = on ? matcher : "Bash";
  ui.permMatcher.disabled = !on;
}

async function refreshHooks() {
  try {
    const v = await invoke("get_settings");
    renderHooks(v.hooks_installed, v.hooks_total);
    renderPerm(v.permission_matcher);
  } catch (e) {
    ui.hooksStatus.textContent = t("set.readFail");
  }
}

function fill(v) {
  prefs = v.prefs;
  about = v.about;

  ui.autostart.checked = v.autostart;

  // agent 名单由 Rust 侧给（含本机检测结果），前端不另维护一份枚举
  ui.agents.textContent = "";
  for (const a of v.agent_options) {
    const row = document.createElement("label");
    row.className = "agent-row" + (a.detected ? "" : " missing");

    const box = document.createElement("input");
    box.type = "checkbox";
    box.value = a.key;
    box.checked = prefs.agents.includes(a.key);
    // 没装的不能勾 —— 勾了也扫不到东西，只会让人以为挂件坏了。
    // 但仍然列出来，否则装完 Codex 找不到开关在哪。
    box.disabled = !a.detected;
    row.appendChild(box);

    const name = document.createElement("span");
    name.className = "agent-name";
    name.textContent = a.label;
    row.appendChild(name);

    // 把三件会让人困惑的事直接写在旁边，而不是留给用户猜：
    // 没装、只有一只宠物、状态有几秒延迟。
    const notes = [];
    if (!a.detected) notes.push(t("set.agentMissing"));
    if (a.gateway) notes.push(t("set.agentGateway"));
    if (a.polled) notes.push(t("set.agentPolled"));
    if (notes.length) {
      const note = document.createElement("span");
      note.className = "agent-note";
      note.textContent = notes.join(" · ");
      row.appendChild(note);
    }

    ui.agents.appendChild(row);
  }

  ui.window.min = v.window_min;
  ui.window.max = v.window_max;
  ui.window.value = prefs.discover_window_minutes;
  ui.windowHint.textContent = t("set.windowHint", {
    min: v.window_min,
    max: v.window_max,
  });

  // 只列本机实际装了的。列出没装的选项等于埋一个「选了却不工作」的坑。
  ui.editor.textContent = "";
  const auto = document.createElement("option");
  auto.value = "auto";
  auto.textContent = v.editors.length
    ? t("set.editorAuto", { first: v.editors[0].label })
    : t("set.editorAutoNone");
  ui.editor.appendChild(auto);
  for (const e of v.editors) {
    const o = document.createElement("option");
    o.value = e.key;
    o.textContent = e.label;
    o.title = e.path;
    ui.editor.appendChild(o);
  }
  // prefs 里存的编辑器可能已经卸载了，此时回落到 auto 而不是显示一个空选项
  const known = ["auto", ...v.editors.map((e) => e.key)];
  ui.editor.value = known.includes(prefs.editor) ? prefs.editor : "auto";
  ui.editor.disabled = v.editors.length === 0;
  ui.editorHint.textContent = v.editors.length
    ? t("set.editorFound", { list: v.editors.map((e) => e.label).join(" / ") })
    : t("set.editorNone");

  // 模式列表由 Rust 侧给，避免前后端各维护一份枚举
  ui.position.textContent = "";
  for (const m of v.position_modes) {
    const o = document.createElement("option");
    o.value = m;
    o.textContent = t(`pos.${m}`);
    ui.position.appendChild(o);
  }
  ui.position.value = prefs.position_mode;

  ui.lang.value = prefs.lang;

  ui.scToggle.value = prefs.shortcut_toggle;
  ui.scNext.value = prefs.shortcut_next;

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
  renderPerm(v.permission_matcher);
  ui.hooksHint.innerHTML = t("set.hooksHint", { path: v.about.claude_settings });
  ui.permHint.innerHTML = t("set.permHint", { secs: v.permission_wait_secs });

  ui.version.textContent = `v${v.about.version}`;
  ui.port.textContent = `127.0.0.1:${v.about.port}`;
  ui.configDir.textContent = v.about.config_dir;
  ui.repo.textContent = v.about.repo;
  ui.checkUpdates.checked = prefs.check_updates;
}

// ── 用量面板 ─────────────────────────────────────────────────

/// 千分位。token 数动辄七位，不分组根本读不出量级。
const nf = new Intl.NumberFormat();

/// 「还剩 3 小时 12 分」。配额的重置时间点单独看没意义 ——
/// 有用的是「还要等多久」。
function untilText(ms) {
  const left = ms - Date.now();
  if (left <= 0) return t("usage.resetsNow");
  const mins = Math.round(left / 60000);
  if (mins < 60) return t("usage.resetsInMin", { n: mins });
  const h = Math.floor(mins / 60);
  const m = mins % 60;
  return m ? t("usage.resetsInHM", { h, m }) : t("usage.resetsInH", { h });
}

function usageCard(u) {
  const card = document.createElement("div");
  card.className = "usage-card";

  const head = document.createElement("div");
  head.className = "usage-head";
  const name = document.createElement("span");
  name.className = "usage-agent";
  name.textContent = u.label;
  head.appendChild(name);

  const n = document.createElement("span");
  n.className = "usage-sessions";
  n.textContent = t("usage.sessions", { n: u.sessions });
  head.appendChild(n);

  // 只有 agent 自己算好成本时才显示。我们刻意不内置价目表 ——
  // 价格会变，过期的表是默默显示错数字，比没有数字更坑人。
  if (typeof u.cost_usd === "number") {
    const c = document.createElement("span");
    c.className = "usage-cost";
    c.textContent = `$${u.cost_usd.toFixed(4)}`;
    head.appendChild(c);
  }
  card.appendChild(head);

  const toks = document.createElement("div");
  toks.className = "usage-tokens";
  for (const [key, val] of [
    ["usage.in", u.tokens.input],
    ["usage.out", u.tokens.output],
    ["usage.cacheRead", u.tokens.cache_read],
    ["usage.cacheWrite", u.tokens.cache_write],
    ["usage.total", u.tokens.total],
  ]) {
    // 0 的项不显示：四个 agent 的字段口径不同，恒为 0 的格子只是噪音
    if (!val) continue;
    const s = document.createElement("span");
    const k = document.createElement("span");
    k.className = "k";
    k.textContent = t(key) + " ";
    s.appendChild(k);
    const b = document.createElement("b");
    b.textContent = nf.format(val);
    s.appendChild(b);
    toks.appendChild(s);
  }
  if (!toks.children.length) {
    const s = document.createElement("span");
    s.className = "k";
    s.textContent = t("usage.none");
    toks.appendChild(s);
  }
  card.appendChild(toks);

  if (u.quota) {
    const q = document.createElement("div");
    q.className = "usage-quota";

    const bar = document.createElement("div");
    const pct = Math.max(0, Math.min(100, u.quota.used_percent));
    // 快满了才变色 —— 任何比例都染红等于永远在报警
    bar.className =
      "usage-bar" + (pct >= 90 ? " danger" : pct >= 75 ? " warn" : "");
    const fill = document.createElement("i");
    fill.style.width = `${pct}%`;
    bar.appendChild(fill);
    q.appendChild(bar);

    const meta = document.createElement("div");
    meta.className = "usage-quota-meta";
    const left = document.createElement("span");
    left.textContent = t("usage.quotaUsed", {
      pct: pct.toFixed(1),
      plan: u.quota.plan || t("usage.planUnknown"),
    });
    meta.appendChild(left);
    if (u.quota.resets_at_ms) {
      const right = document.createElement("span");
      right.textContent = untilText(u.quota.resets_at_ms);
      meta.appendChild(right);
    }
    q.appendChild(meta);
    card.appendChild(q);
  }

  // note 是 i18n 的键，说明「为什么某些格子是空的」。
  // 留空让用户猜是坏了还是没有，比多一行字糟。
  if (u.note) {
    const note = document.createElement("p");
    note.className = "usage-note";
    note.textContent = t(u.note);
    card.appendChild(note);
  }

  return card;
}

async function refreshUsage() {
  ui.usageRefresh.disabled = true;
  ui.usageBody.textContent = "";
  ui.usageHint.textContent = t("usage.reading");
  try {
    const list = await invoke("get_usage");
    ui.usageBody.textContent = "";
    if (!list.length) {
      ui.usageHint.textContent = t("usage.noAgents");
      return;
    }
    for (const u of list) ui.usageBody.appendChild(usageCard(u));
    // 说清楚统计范围。不说的话「只有 3 个会话」会被当成 bug，
    // 而实际是时间窗把更早的排除了。
    ui.usageHint.textContent = t("usage.windowHint", {
      min: prefs.discover_window_minutes,
    });
  } catch (e) {
    ui.usageHint.textContent = t("usage.failed", { err: e });
  } finally {
    ui.usageRefresh.disabled = false;
  }
}

async function runUpdateCheck(about) {
  ui.checkNow.disabled = true;
  ui.updateResult.textContent = t("set.checking");
  ui.updateCmd.hidden = true;
  const r = await window.UPDATE.check(about.repo, about.version);
  ui.checkNow.disabled = false;

  if (r.error) {
    ui.updateResult.textContent = t("set.updateFailed", { err: r.error });
    return;
  }
  if (r.newer) {
    ui.updateResult.textContent = t("set.updateFound", {
      latest: r.latest,
      cur: about.version,
    });
    // 命令直接摊开给用户复制。没有 opener 插件，弹不出浏览器也起不了终端，
    // 而把命令藏起来只会让人去 README 里找。
    ui.updateCmd.textContent = `${t("set.updateHow")}\n${UPGRADE_CMD}`;
    ui.updateCmd.hidden = false;
  } else {
    ui.updateResult.textContent = t("set.upToDate", { cur: about.version });
  }
}

function wire() {
  ui.autostart.addEventListener("change", async () => {
    try {
      await invoke("set_autostart", { enabled: ui.autostart.checked });
      toast(ui.autostart.checked ? t("set.autostartOn") : t("set.autostartOff"));
    } catch (e) {
      // 写注册表可能失败，此时把勾选状态改回去，别让界面骗人
      ui.autostart.checked = !ui.autostart.checked;
      toast(t("set.setFailed", { err: e }), true);
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
    apply("what.window").then(refreshAfterClamp);
  });

  ui.editor.addEventListener("change", () => {
    prefs.editor = ui.editor.value;
    apply("what.editor");
  });

  // 事件委托挂在容器上，因为勾选框是 fill() 动态建的 ——
  // 逐个绑定的话，切语言重新 fill 之后旧监听器就跟着 DOM 一起没了
  ui.agents.addEventListener("change", (e) => {
    const box = e.target;
    if (!box || box.type !== "checkbox") return;
    // 顺序按 Rust 侧的名单来，不按点击顺序 —— sanitise() 也会重排，
    // 不统一的话 prefs.json 的 diff 会毫无必要地翻来翻去
    prefs.agents = Array.from(ui.agents.querySelectorAll("input:checked")).map(
      (b) => b.value
    );
    apply("what.agents").then(refreshUsage);
  });

  ui.usageRefresh.addEventListener("click", refreshUsage);

  ui.position.addEventListener("change", () => {
    prefs.position_mode = ui.position.value;
    apply("what.position");
  });

  ui.lang.addEventListener("change", async () => {
    prefs.lang = ui.lang.value;
    await apply("what.lang");
    // 整个窗口换语言：重新拉一次设置，因为动态文案里带插值
    try {
      const v = await invoke("get_settings");
      window.I18N.setLang(v.lang_code);
      window.I18N.applyI18n();
      fill(v);
    } catch (e) {
      /* 保持现状 */
    }
  });

  // 用 change 而不是 input：每敲一个字符就去抢注册全局热键毫无意义，
  // 而且中间态（"Ctrl+"）必然注册失败，会刷出一串假警告。
  for (const [input, key] of [
    [ui.scToggle, "shortcut_toggle"],
    [ui.scNext, "shortcut_next"],
  ]) {
    input.addEventListener("change", () => {
      prefs[key] = input.value.trim();
      apply("what.shortcut");
    });
  }

  ui.soundOn.addEventListener("change", () => {
    prefs.muted = !ui.soundOn.checked;
    ui.sound.disabled = prefs.muted;
    ui.preview.disabled = prefs.muted;
    apply("what.sound");
    // 打开时试听一声，好知道听到的是什么
    if (!prefs.muted) {
      invoke("preview_sound", { alias: prefs.sound }).catch(() => {});
    }
  });

  ui.sound.addEventListener("change", () => {
    prefs.sound = ui.sound.value;
    apply("what.soundPick");
    invoke("preview_sound", { alias: prefs.sound }).catch(() => {});
  });

  ui.preview.addEventListener("click", () => {
    invoke("preview_sound", { alias: ui.sound.value }).catch(() => {});
  });

  ui.installHooks.addEventListener("click", () => setHooks(true));
  ui.uninstallHooks.addEventListener("click", () => setHooks(false));

  ui.checkUpdates.addEventListener("change", () => {
    prefs.check_updates = ui.checkUpdates.checked;
    apply();
  });

  ui.checkNow.addEventListener("click", () => {
    // 手动检查不看 check_updates 开关：那个开关管的是「启动时自动查」，
    // 关掉它的人仍然可能想主动查一次。
    if (about) runUpdateCheck(about);
  });

  ui.permOn.addEventListener("change", () => setPermHook(ui.permOn.checked));
  // 改 matcher 等于重装那条 hook（Rust 侧是先摘旧的再装，不会叠加）
  ui.permMatcher.addEventListener("change", () => {
    if (ui.permOn.checked) setPermHook(true);
  });
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

async function setPermHook(install) {
  ui.permOn.disabled = true;
  try {
    const msg = await invoke("set_permission_hook", {
      install,
      matcher: ui.permMatcher.value.trim() || "Bash",
    });
    toast(msg);
  } catch (e) {
    toast(t("set.failed", { err: e }), true);
  }
  ui.permOn.disabled = false;
  await refreshHooks();
}

async function setHooks(install) {
  ui.installHooks.disabled = true;
  ui.uninstallHooks.disabled = true;
  try {
    const msg = await invoke("set_hooks", { install });
    toast(msg);
  } catch (e) {
    toast(t(install ? "set.installFailed" : "set.uninstallFailed", { err: e }), true);
  }
  await refreshHooks();
}

async function init() {
  const tauri = window.__TAURI__;
  if (!tauri) {
    document.body.textContent = t("set.noTauri");
    return;
  }
  invoke = (tauri.core && tauri.core.invoke) || tauri.invoke;

  try {
    const v = await invoke("get_settings");
    // 语言先定再填内容：动态文案带插值，顺序反了会先渲染出中文再闪成英文
    window.I18N.setLang(v.lang_code);
    window.I18N.applyI18n();
    fill(v);
  } catch (e) {
    document.body.textContent = t("set.loadFailed", { err: e });
    return;
  }
  wire();
  // 用量要扫本地文件（本机 651 个转录），所以不阻塞设置窗口出现 ——
  // 不 await，扫完自己填进去。
  refreshUsage();
}

init();
