// 挂件前端：只做两件事 —— 订阅 Rust 侧的状态、渲染。
// 所有状态判断都在 Rust 里做完了，这里不放业务逻辑。

const STATE_TEXT = {
  working: "干活中",
  "waiting-permission": "要你点允许",
  "waiting-input": "在等你回话",
  idle: "空闲",
  done: "完成了",
};

const KNOWN_STATES = Object.keys(STATE_TEXT);

const el = {
  pet: document.getElementById("pet"),
  headline: document.getElementById("headline"),
  project: document.getElementById("project"),
  detail: document.getElementById("detail"),
  badge: document.getElementById("badge"),
};

function render(view) {
  if (!view) return;

  const state = KNOWN_STATES.includes(view.state) ? view.state : "idle";

  el.pet.classList.remove(...KNOWN_STATES.map((s) => `state-${s}`));
  el.pet.classList.add(`state-${state}`);

  el.headline.textContent = STATE_TEXT[state];
  el.project.textContent = view.project || "";
  // detail 可能很长（比如整条 bash 命令），CSS 负责 ellipsis
  el.detail.textContent = view.detail || "";

  // 单会话时不显示角标，免得占地方；多会话且有人在等你时标红
  const sessions = view.sessions || 0;
  const waiting = view.waiting || 0;
  if (sessions > 1) {
    el.badge.hidden = false;
    el.badge.textContent = waiting > 0 ? `${waiting}/${sessions}` : String(sessions);
    el.badge.classList.toggle("alert", waiting > 0);
  } else {
    el.badge.hidden = true;
  }
}

async function init() {
  const tauri = window.__TAURI__;
  if (!tauri) {
    // withGlobalTauri 没生效时给个可见的提示，而不是静默死掉
    el.detail.textContent = "__TAURI__ 未注入";
    return;
  }

  // Tauri 2 把 invoke 挪到了 core 下，这里兼容一下老位置
  const invoke = (tauri.core && tauri.core.invoke) || tauri.invoke;

  try {
    await tauri.event.listen("pet://state", (e) => render(e.payload));
  } catch (err) {
    el.detail.textContent = `订阅失败: ${err}`;
    return;
  }

  // 挂件可能是在会话进行中才启动的，先主动拉一次当前状态
  try {
    render(await invoke("get_state"));
  } catch (err) {
    el.detail.textContent = `读取状态失败: ${err}`;
  }
}

init();
