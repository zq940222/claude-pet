// 挂件前端。
//
// 模型：一个会话 = 一只宠物，一个项目 = 一个工作空间。
// Rust 侧吐完整的会话树（已按 first_seen 排好序，保证图标不乱跳），
// 这里只负责渲染、选中、折叠展开。

const KNOWN_STATES = [
  "working",
  "waiting-permission",
  "waiting-input",
  "idle",
  "done",
];

const t = (k, v) => window.I18N.t(k, v);
const stateText = (s) => t(`state.${s}`);

// 挂件没有可见的控制台，脚本一出错就是整块空白，看不出所以然。
// 把错误写进详情行，这样至少「哪里炸了」是可见的。
function showFatal(prefix, msg) {
  // 必须写进 strip：折叠态下 .detail 是 display:none，写那儿等于看不见，
  // 而脚本挂掉时卡片正是停在折叠态的。
  const strip = document.getElementById("strip");
  if (strip) {
    strip.textContent = `${prefix}: ${msg}`;
    strip.style.color = "#ff8a82";
    strip.style.fontSize = "10px";
  }
}
window.addEventListener("error", (e) => showFatal("JS", e.message));
window.addEventListener("unhandledrejection", (e) =>
  showFatal("Promise", e.reason && e.reason.message ? e.reason.message : e.reason)
);
const PAD = 10; // 与 body 的 padding 一致，留给自绘阴影

const el = {
  card: document.getElementById("card"),
  toggle: document.getElementById("toggle"),
  strip: document.getElementById("strip"),
  summary: document.getElementById("summary"),
  workspaces: document.getElementById("workspaces"),
  detail: document.getElementById("detail"),
  dHead: document.getElementById("dHead"),
  dProj: document.getElementById("dProj"),
  dDetail: document.getElementById("dDetail"),
  actions: document.getElementById("actions"),
  allow: document.getElementById("allow"),
  deny: document.getElementById("deny"),
};

const app = {
  view: { workspaces: [], total: 0, waiting: 0, focus: null, pending: [] },
  selected: null,
  expanded: false,
  /// null = 跟着自动规则；true = 用户按住开着；false = 用户按住关着
  pinned: null,
  /// 鼠标是不是停在挂件上。悬停是现在的主要展开方式，
  /// 所以它和「有人在等你」「用户按住开着」是三个并列的展开理由。
  hovering: false,
  /// 检查到的新版本号，没有就是 null。只影响概览行的后缀。
  newVersion: null,
  /// 上一次「等待集合」的签名，用来识别是不是来了新的待处理事项
  lastWaitingSig: "",
  invoke: null,
};

function safeState(s) {
  return KNOWN_STATES.includes(s) ? s : "idle";
}

function allSessions(view) {
  return view.workspaces.flatMap((w) =>
    w.sessions.map((s) => ({ ...s, project: w.project }))
  );
}

/// 等待中会话的签名。只要它变了，就说明有「新的」东西需要你，
/// 这时即使用户之前手动收起过，也要强制展开。
function waitingSignature(view) {
  return allSessions(view)
    .filter((s) => s.state.startsWith("waiting"))
    .map((s) => `${s.id}:${s.state}`)
    .sort()
    .join("|");
}

// ── 折叠 / 展开状态机 ────────────────────────────────────────
//
// 展开有**三个并列的理由**，任一成立就展开：
//
//   1. 有会话在等你      —— 挂件存在的意义，优先级最高
//   2. 用户按住开着       —— 点一下卡片把它钉住
//   3. 鼠标停在挂件上     —— 悬停窥视，现在的主要交互方式
//
// 收起只在三个都不成立时发生。这条「或」的关系是刻意的：光靠 `pinned` 的
// 三态（null/true/false）表达不了「悬停」这个临时状态 —— 悬停不该覆盖
// 用户的钉住，也不该在有人等你时把它藏回去。

/// 当前应该展开吗。**唯一**决定 `expanded` 的地方，
/// 三个入口（事件、悬停、点击）都走它，避免各自算一遍算出不同结果。
function shouldExpand(waiting) {
  if (waiting) return true;
  if (app.pinned === true) return true;
  return app.hovering;
}

function applyCollapseRules(view) {
  const sig = waitingSignature(view);
  const isNewEpisode = sig !== "" && sig !== app.lastWaitingSig;
  app.lastWaitingSig = sig;

  if (isNewEpisode) {
    // 有新的东西要你处理：强制展开、选中它、并清掉手动状态，
    // 这样等它处理完之后又能自动收起。
    app.pinned = null;
    if (view.focus) app.selected = view.focus;
  }
  app.expanded = shouldExpand(sig !== "");
}

function toggleExpanded() {
  // 点击是「钉住 / 取消钉住」。取消钉住之后如果鼠标还在挂件上，
  // 它仍然是展开的 —— 这时收起要靠把鼠标移开，和悬停语义一致。
  app.pinned = app.pinned === true ? null : true;
  app.expanded = shouldExpand(app.lastWaitingSig !== "");
  render();
}

/// 鼠标进出。
///
/// 离开要延迟一下：展开/收起会改窗口尺寸，而尺寸变化本身可能让 WebView
/// 瞬间抛出一对 leave/enter —— 不缓冲的话挂件会在边界上抖动。
/// 进入不延迟，因为「碰一下就展开」的即时感正是这个交互的全部价值。
const LEAVE_DELAY_MS = 180;
let leaveTimer = null;

function setHovering(on) {
  clearTimeout(leaveTimer);
  if (on) {
    if (app.hovering) return;
    app.hovering = true;
    app.expanded = shouldExpand(app.lastWaitingSig !== "");
    render();
    return;
  }
  leaveTimer = setTimeout(() => {
    if (!app.hovering) return;
    app.hovering = false;
    app.expanded = shouldExpand(app.lastWaitingSig !== "");
    render();
  }, LEAVE_DELAY_MS);
}

/// 全局快捷键：跳到下一个在等你的会话。
///
/// 在「等待中的会话」里循环，而不是在全部会话里 —— 快捷键的用途就是
/// 「谁在等我」，把干活中和空闲的也串进去只会让人多按好几次。
function focusNextWaiting() {
  const waiting = allSessions(app.view).filter((s) =>
    s.state.startsWith("waiting")
  );
  if (!waiting.length) {
    app.expanded = true;
    app.pinned = true;
    render();
    flash(t("pet.nothingWaiting"));
    return;
  }
  const at = waiting.findIndex((s) => s.id === app.selected);
  app.selected = waiting[(at + 1) % waiting.length].id;
  app.expanded = true;
  // 是用户主动唤出来的，别下一个事件就给收起去
  app.pinned = true;
  render();
}

// ── 渲染 ─────────────────────────────────────────────────────

/// agent 的显示名。Rust 侧发的是稳定键（`claude-code` / `codex` / ...），
/// 这里翻成人看的名字。
///
/// t() 查不到时返回**键名本身**（不是 undefined），所以不能用 `||` 兜 ——
/// 那样会显示成 `agent.some-new-agent`。Rust 侧加了新 agent 而前端还没跟上时，
/// 显示裸键 `some-new-agent` 比显示带前缀的内部键好看。
function agentLabel(key) {
  if (!key) return "";
  const full = `agent.${key}`;
  const s = t(full);
  return s === full ? key : s;
}

function makePet(session, project, big) {
  const node = document.createElement(big ? "button" : "span");
  const state = safeState(session.state);

  node.className = `pet ${big ? "big" : "mini"} s-${state}`;
  if (state.startsWith("waiting")) node.classList.add("waiting");
  if (big && session.id === app.selected) node.classList.add("selected");

  // agent 走**边框样式**，不走颜色。颜色整条通道已经被状态占满了
  // （红色 = 要你动手），再拿它区分 agent 就会两种含义打架。
  // 边框样式是正交的，而且不影响布局 —— 卡片宽度是量出来的，
  // 任何改变盒模型尺寸的区分手段都会牵动窗口大小。
  if (session.agent) node.dataset.agent = session.agent;

  node.title =
    `${project} #${session.index} — ${stateText(state)}` +
    (session.agent ? ` (${agentLabel(session.agent)})` : "") +
    (session.detail ? `\n${session.detail}` : "") +
    (session.cwd ? `\n${session.cwd}` : "") +
    (big && session.cwd ? `\n\n${t("pet.dblclickHint")}` : "");

  if (big) {
    node.type = "button";
    node.textContent = String(session.index);
    node.addEventListener("click", (e) => {
      e.stopPropagation();
      app.selected = session.id;
      render();
    });
    // 双击跳回 IDE。单击仍是选中 —— 浏览器会先派发 click 再派发 dblclick，
    // 所以两者不冲突：双击的结果是「先选中它，然后跳过去」，正是想要的。
    node.addEventListener("dblclick", async (e) => {
      e.stopPropagation();
      // gateway 类的宠物（Hermes / OpenClaw）没有项目目录，cwd 是空串。
      // 不拦住的话双击会去调 open_in_editor 然后报一个看不懂的错。
      if (!session.cwd) {
        flash(t("pet.noProjectDir", { agent: agentLabel(session.agent) }));
        return;
      }
      if (!app.invoke) return;
      try {
        const used = await app.invoke("open_in_editor", { sessionId: session.id });
        flash(t("pet.openedIn", { editor: used }));
      } catch (err) {
        flash(String(err), true);
      }
    });
  }
  return node;
}

/// 在详情行上短暂顶掉原文显示一条结果。挂件太小放不下 toast，
/// 而完全没有反馈的话，跳转失败时用户只会以为双击没生效。
let flashTimer = null;
function flash(msg, isError) {
  el.dDetail.textContent = msg;
  el.dDetail.classList.toggle("flash-err", !!isError);
  clearTimeout(flashTimer);
  flashTimer = setTimeout(() => {
    el.dDetail.classList.remove("flash-err");
    render();
  }, isError ? 3200 : 1600);
}

function renderStrip(view) {
  el.strip.textContent = "";

  if (view.total === 0) {
    const s = document.createElement("span");
    s.className = "strip-empty";
    s.textContent = t("pet.noSessions");
    el.strip.appendChild(s);
    return;
  }

  // 项目多了就不显示名字，只留点阵。7 个项目名根本塞不进折叠条，
  // 硬塞的结果是全被截成 "my-vid..." 这种没信息量的碎片。
  // 名字在展开态和每个点的 tooltip 里都拿得到。
  const showNames = view.workspaces.length <= 3;
  el.strip.classList.toggle("dots-only", !showNames);

  for (const ws of view.workspaces) {
    const group = document.createElement("span");
    group.className = "strip-group";

    if (showNames) {
      const name = document.createElement("span");
      name.className = "strip-name";
      name.textContent = ws.project;
      group.appendChild(name);
    }

    for (const s of ws.sessions) group.appendChild(makePet(s, ws.project, false));
    el.strip.appendChild(group);
  }
}

function renderSummary(view) {
  let s;
  if (view.total === 0) {
    s = t("pet.noSessions");
  } else if (view.waiting > 0) {
    s = t("pet.sessionsWaiting", { n: view.total, w: view.waiting });
  } else {
    s = t("pet.sessions", { n: view.total });
  }
  // 新版本提示挂在概览行末尾。不另做徽标/弹窗：挂件就这么大，
  // 而「有新版本」的紧急程度远低于「有会话在等你」，不该抢视觉。
  if (app.newVersion) {
    s += ` · ${t("pet.updateAvailable", { latest: app.newVersion })}`;
  }
  el.summary.textContent = s;
}

/// 渲染时记下选中的那个宠物节点，好在工作空间列表滚动时把它带进视野
let selectedNode = null;

function renderWorkspaces(view) {
  el.workspaces.textContent = "";
  selectedNode = null;

  for (const ws of view.workspaces) {
    const row = document.createElement("div");
    row.className = "ws";
    // 行本身可拖动窗口，行内的宠物按钮不受影响
    row.setAttribute("data-tauri-drag-region", "");

    const name = document.createElement("span");
    name.className = "ws-name";
    name.textContent = ws.project;
    name.title = ws.project;
    name.setAttribute("data-tauri-drag-region", "");
    row.appendChild(name);

    const dots = document.createElement("span");
    dots.className = "ws-dots";
    for (const s of ws.sessions) {
      const pet = makePet(s, ws.project, true);
      if (s.id === app.selected) selectedNode = pet;
      dots.appendChild(pet);
    }
    row.appendChild(dots);

    el.workspaces.appendChild(row);
  }
}

/// 把 app.selected 收敛到一个真实存在的会话上。
/// 必须在渲染之前跑：否则工作空间行画选中环时用的是旧值、详情面板用的是新值，
/// 两边会差一帧对不上。
function resolveSelection(view) {
  const sessions = allSessions(view);
  if (sessions.some((s) => s.id === app.selected)) return;

  // 选中的会话结束了 —— 退回最该被关注的那个
  if (view.focus && sessions.some((s) => s.id === view.focus)) {
    app.selected = view.focus;
  } else {
    app.selected = sessions.length ? sessions[0].id : null;
  }
}

function renderDetail(view) {
  const current = allSessions(view).find((s) => s.id === app.selected);

  el.detail.classList.remove(...KNOWN_STATES.map((s) => `state-${s}`));

  // 这个会话有挂起的权限请求吗？有就给出允许/拒绝按钮。
  const pending = (view.pending || []).find(
    (p) => current && p.session_id === current.id
  );
  el.actions.hidden = !pending;
  el.allow.disabled = false;
  el.deny.disabled = false;
  app.pendingId = pending ? pending.id : null;

  if (!current) {
    el.detail.classList.add("state-idle");
    el.dHead.textContent = stateText("idle");
    el.dProj.textContent = "";
    el.dDetail.textContent = t("pet.noSessions");
    return;
  }

  const state = safeState(current.state);
  el.detail.classList.add(`state-${state}`);
  el.dHead.textContent = stateText(state);
  el.dProj.textContent = `${current.project} #${current.index}`;
  el.dDetail.textContent = current.detail || "";
}

function render() {
  const view = app.view;

  resolveSelection(view);

  el.card.classList.toggle("collapsed", !app.expanded);
  el.card.classList.toggle("has-waiting", view.waiting > 0);

  renderStrip(view);
  renderSummary(view);
  renderWorkspaces(view);
  renderDetail(view);

  // 工作空间超过 5 行会转滚动，把选中的宠物带进视野。
  // block:'nearest' 只动最近的可滚动祖先，不会连带滚 body。
  if (selectedNode) {
    selectedNode.scrollIntoView({ block: "nearest", inline: "nearest" });
  }

  syncSize();
}

// ── 窗口尺寸 ─────────────────────────────────────────────────

let lastW = 0;
let lastH = 0;

function syncSize() {
  // 等布局落定再量。折叠切换只改 display，一帧就够。
  requestAnimationFrame(() => {
    const r = el.card.getBoundingClientRect();
    const w = Math.ceil(r.width) + PAD * 2;
    const h = Math.ceil(r.height) + PAD * 2;
    if (w === lastW && h === lastH) return;
    lastW = w;
    lastH = h;
    if (app.invoke) {
      app.invoke("resize_pet", { width: w, height: h }).catch(() => {
        // 改不了大小不该让渲染挂掉，下一次 render 会再试
        lastW = 0;
        lastH = 0;
      });
    }
  });
}

// ── 启动 ─────────────────────────────────────────────────────

function onView(view) {
  if (!view) return;
  app.view = view;
  applyCollapseRules(view);
  render();
}

async function init() {
  const tauri = window.__TAURI__;
  if (!tauri) {
    el.dDetail.textContent = t("pet.noTauri");
    return;
  }

  // Tauri 2 把 invoke 挪到了 core 下，这里兼容一下老位置
  app.invoke = (tauri.core && tauri.core.invoke) || tauri.invoke;

  // 语言要在第一次 render 之前定。Rust 侧已经把 "auto" 解析成了具体语言，
  // 前端不自己猜系统语言 —— 两边各猜一次必然会有不一致的时候。
  let boot = null;
  try {
    boot = await app.invoke("get_boot");
    window.I18N.setLang(boot.lang);
  } catch (err) {
    /* 拿不到就用默认，不该因此白屏 */
  }
  window.I18N.applyI18n();

  // 版本检查放在后台，不 await —— 网络慢或不通时不该拖着挂件不显示。
  if (boot && boot.check_updates) {
    window.UPDATE.check(boot.repo, boot.version).then((r) => {
      if (r && r.newer) {
        app.newVersion = r.latest;
        render();
      }
    });
  }

  el.toggle.addEventListener("click", (e) => {
    e.stopPropagation();
    toggleExpanded();
  });

  // 悬停窥视。挂在 document 而不是卡片上：窗口尺寸就是卡片尺寸，
  // 但圆角外那一点点透明区域仍属于窗口，绑在卡片上时鼠标划过圆角
  // 会多抛一对 leave/enter。
  //
  // 用 mouseenter/mouseleave 而不是 mouseover/mouseout —— 后者会随着
  // 鼠标在子元素之间移动不停冒泡，每动一下就重算一次展开状态。
  document.addEventListener("mouseenter", () => setHovering(true));
  document.addEventListener("mouseleave", () => setHovering(false));

  // 权限请求：点一次就把按钮禁掉，避免连点重复提交 ——
  // 那条 HTTP 请求只能被决定一次，第二次调用会拿到「已经不在了」的错误。
  for (const [node, allow] of [
    [el.allow, true],
    [el.deny, false],
  ]) {
    node.addEventListener("click", async (e) => {
      e.stopPropagation();
      const id = app.pendingId;
      if (id == null || !app.invoke) return;
      el.allow.disabled = true;
      el.deny.disabled = true;
      try {
        await app.invoke("resolve_permission", { id, allow });
      } catch (err) {
        flash(String(err), true);
      }
    });
  }

  try {
    await tauri.event.listen("pet://view", (e) => onView(e.payload));
    // 全局快捷键只送动作名过来；折叠和选中状态都住在这边的状态机里
    await tauri.event.listen("pet://shortcut", (e) => {
      if (e.payload === "toggle") toggleExpanded();
      else if (e.payload === "next") focusNextWaiting();
    });
    // 设置窗口改了语言就地重渲染，不用重启挂件
    await tauri.event.listen("pet://lang", (e) => {
      window.I18N.setLang(e.payload);
      window.I18N.applyI18n();
      render();
    });
    // 拖到屏幕边缘入坞之后收起。由 Rust 侧在检测到拖动结束时发过来 ——
    // 入坞的意思就是「停在边上别挡路」，展开着就没意义了。
    //
    // 同时清掉钉住状态：用户刚刚亲手把它拖到边上，那是比之前某次点击
    // 更新的意图。悬停仍然能把它唤出来。
    await tauri.event.listen("pet://docked", () => {
      app.pinned = null;
      app.hovering = false;
      clearTimeout(leaveTimer);
      app.expanded = shouldExpand(app.lastWaitingSig !== "");
      render();
    });
  } catch (err) {
    el.dDetail.textContent = t("pet.subscribeFailed", { err });
    return;
  }

  // 挂件可能是在会话进行中才启动的，先主动拉一次当前状态
  try {
    onView(await app.invoke("get_view"));
  } catch (err) {
    el.dDetail.textContent = t("pet.readFailed", { err });
    render();
  }
}

init();
