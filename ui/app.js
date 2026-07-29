// 挂件前端。
//
// 模型：一个会话 = 一只宠物，一个项目 = 一个工作空间。
// Rust 侧吐完整的会话树（已按 first_seen 排好序，保证图标不乱跳），
// 这里只负责渲染、选中、折叠展开。

const STATE_TEXT = {
  working: "干活中",
  "waiting-permission": "要你点允许",
  "waiting-input": "在等你回话",
  idle: "空闲",
  done: "完成了",
};

const KNOWN_STATES = Object.keys(STATE_TEXT);
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
};

const app = {
  view: { workspaces: [], total: 0, waiting: 0, focus: null },
  selected: null,
  expanded: false,
  /// null = 跟着自动规则；true = 用户按住开着；false = 用户按住关着
  pinned: null,
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

function applyCollapseRules(view) {
  const sig = waitingSignature(view);
  const isNewEpisode = sig !== "" && sig !== app.lastWaitingSig;
  app.lastWaitingSig = sig;

  if (isNewEpisode) {
    // 有新的东西要你处理：强制展开、选中它、并清掉手动状态，
    // 这样等它处理完之后又能自动收起。
    app.pinned = null;
    app.expanded = true;
    if (view.focus) app.selected = view.focus;
  } else if (app.pinned === null) {
    // 自动模式：有人在等你就展开，没有就收起
    app.expanded = sig !== "";
  }
  // app.pinned !== null 时保持用户的选择不动
}

function toggleExpanded() {
  app.expanded = !app.expanded;
  // 记下这是用户的意愿，直到下一批新的等待事项才让位
  app.pinned = app.expanded;
  render();
}

// ── 渲染 ─────────────────────────────────────────────────────

function makePet(session, project, big) {
  const node = document.createElement(big ? "button" : "span");
  const state = safeState(session.state);

  node.className = `pet ${big ? "big" : "mini"} s-${state}`;
  if (state.startsWith("waiting")) node.classList.add("waiting");
  if (big && session.id === app.selected) node.classList.add("selected");

  node.title =
    `${project} #${session.index} — ${STATE_TEXT[state]}` +
    (session.detail ? `\n${session.detail}` : "") +
    (session.cwd ? `\n${session.cwd}` : "") +
    (big ? "\n\n双击在编辑器中打开" : "");

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
      if (!app.invoke) return;
      try {
        const used = await app.invoke("open_in_editor", { sessionId: session.id });
        flash(`已在 ${used} 中打开`);
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
    s.textContent = "没有活动会话";
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
  if (view.total === 0) {
    el.summary.textContent = "没有活动会话";
  } else if (view.waiting > 0) {
    el.summary.textContent = `${view.total} 个会话 · ${view.waiting} 个在等你`;
  } else {
    el.summary.textContent = `${view.total} 个会话`;
  }
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

  if (!current) {
    el.detail.classList.add("state-idle");
    el.dHead.textContent = "空闲";
    el.dProj.textContent = "";
    el.dDetail.textContent = "没有活动会话";
    return;
  }

  const state = safeState(current.state);
  el.detail.classList.add(`state-${state}`);
  el.dHead.textContent = STATE_TEXT[state];
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
    el.dDetail.textContent = "__TAURI__ 未注入";
    return;
  }

  // Tauri 2 把 invoke 挪到了 core 下，这里兼容一下老位置
  app.invoke = (tauri.core && tauri.core.invoke) || tauri.invoke;

  el.toggle.addEventListener("click", (e) => {
    e.stopPropagation();
    toggleExpanded();
  });

  try {
    await tauri.event.listen("pet://view", (e) => onView(e.payload));
  } catch (err) {
    el.dDetail.textContent = `订阅失败: ${err}`;
    return;
  }

  // 挂件可能是在会话进行中才启动的，先主动拉一次当前状态
  try {
    onView(await app.invoke("get_view"));
  } catch (err) {
    el.dDetail.textContent = `读取状态失败: ${err}`;
    render();
  }
}

init();
