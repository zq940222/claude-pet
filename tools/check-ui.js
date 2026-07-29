// 前端静态检查。`node tools/check-ui.js`
//
// 存在的理由是一个真实事故：`i18n.js` 顶层的 `function t()` 在经典 script 里
// 会创建**全局** `t`，而 `app.js` 里有 `const t = ...`。撞车是**解析期**
// SyntaxError —— 整个文件一行都不执行，连 `window.onerror` 都注册不上，
// 表现为「界面空白且没有任何报错」。
//
// `node --check` 是单文件检查，永远发现不了这种跨文件冲突。所以这里把每个
// 页面的脚本按 HTML 里的顺序放进**同一个** context 真实执行一遍。

const fs = require("fs");
const path = require("path");
const vm = require("vm");

const UI = path.join(__dirname, "..", "ui");

/// 共享脚本只允许导出挂在 window 上的这些东西，不许泄漏任何全局。
/// 页面入口脚本（app.js / settings.js）可以泄漏 —— 每个页面只加载一个。
const SHARED = { "i18n.js": "I18N", "update.js": "UPDATE" };

let failed = 0;
function fail(msg) {
  console.error(`FAIL  ${msg}`);
  failed++;
}
function pass(msg) {
  console.log(`ok    ${msg}`);
}

function fakeNode(id) {
  return {
    id,
    textContent: "",
    innerHTML: "",
    hidden: false,
    disabled: false,
    checked: false,
    title: "",
    className: "",
    style: {},
    min: 0,
    max: 0,
    value: "",
    type: "",
    classList: { add() {}, remove() {}, toggle() {}, contains: () => false },
    appendChild() {},
    addEventListener() {},
    setAttribute() {},
    getAttribute: () => null,
    querySelectorAll: () => [],
    scrollIntoView() {},
    getBoundingClientRect: () => ({ width: 200, height: 80 }),
  };
}

/// 从 HTML 里按出现顺序抽出本地 <script src>。顺序很重要 ——
/// 冲突只在真实加载顺序下才复现。
function scriptsOf(html) {
  const src = fs.readFileSync(path.join(UI, html), "utf8");
  return [...src.matchAll(/<script\s+src="([^"]+)"/g)].map((m) => m[1]);
}

function checkPage(html) {
  const files = scriptsOf(html);
  if (!files.length) return fail(`${html}: 没找到任何 <script src>`);

  const nodes = {};
  const document = {
    getElementById: (id) => nodes[id] || (nodes[id] = fakeNode(id)),
    querySelectorAll: () => [],
    createElement: (t) => fakeNode("created-" + t),
    body: fakeNode("body"),
  };
  const win = {
    document,
    addEventListener() {},
    requestAnimationFrame(f) {
      setTimeout(f, 0);
    },
    setTimeout,
    clearTimeout,
  };
  const ctx = vm.createContext({
    window: win,
    document,
    console: { log() {}, warn() {}, error() {} },
    setTimeout,
    clearTimeout,
    Object, Array, String, Number, Math, JSON, Boolean, RegExp, parseInt, Error, Promise,
    fetch: async () => ({ ok: false, status: 0 }),
  });
  ctx.globalThis = ctx;

  for (const f of files) {
    const before = new Set(Object.keys(ctx));
    try {
      vm.runInContext(fs.readFileSync(path.join(UI, f), "utf8"), ctx, { filename: f });
    } catch (e) {
      // 这正是那次事故的形态：SyntaxError 意味着整个文件没执行
      return fail(`${html} → ${f}: ${e.constructor.name}: ${e.message}`);
    }
    const leaked = Object.keys(ctx).filter((k) => !before.has(k));
    if (f in SHARED) {
      if (leaked.length) {
        fail(`${f} 泄漏了全局变量 [${leaked.join(", ")}]，共享脚本必须包在 IIFE 里`);
      } else if (typeof win[SHARED[f]] !== "object") {
        fail(`${f} 没有导出 window.${SHARED[f]}`);
      } else {
        pass(`${f} 零全局泄漏，导出 window.${SHARED[f]}`);
      }
    }
  }
  pass(`${html}: ${files.join(" + ")} 在同一作用域下共存`);
}

function checkI18nParity() {
  const ctx = vm.createContext({
    window: {},
    document: { querySelectorAll: () => [], body: { getAttribute: () => null } },
    console, Object, Array, String, Number, Math, RegExp, parseInt,
  });
  ctx.globalThis = ctx;
  vm.runInContext(fs.readFileSync(path.join(UI, "i18n.js"), "utf8"), ctx, { filename: "i18n.js" });
  const I = ctx.window.I18N;

  // 两种语言的 key 必须完全一致。缺 key 会静默回落到中文，
  // 英文界面里冒出一句中文很难被发现。
  const langs = I.langs;
  const keysOf = (code) => {
    I.setLang(code);
    return null; // DICT 不外露，改用差集探测：见下
  };
  void keysOf;

  // DICT 是闭包内的，外面拿不到。改为对已知 key 逐个比对两种语言是否都有翻译：
  // t() 缺 key 时会回落到中文，所以「英文下取到的值等于中文下取到的值」
  // 且该值不是纯 ASCII，就说明英文漏翻了。
  const probe = [];
  const src = fs.readFileSync(path.join(UI, "i18n.js"), "utf8");
  for (const m of src.matchAll(/^\s*"([a-z]+\.[A-Za-z0-9.\-]+)":/gm)) probe.push(m[1]);
  const uniq = [...new Set(probe)];

  const missing = [];
  for (const k of uniq) {
    I.setLang("zh");
    const zh = I.t(k);
    I.setLang("en");
    const en = I.t(k);
    // 中文值含 CJK 而英文取到同一个值 => 英文没有这条
    if (en === zh && /[一-鿿]/.test(zh)) missing.push(k);
  }
  if (missing.length) {
    fail(`英文缺 ${missing.length} 条翻译: ${missing.slice(0, 8).join(", ")}${missing.length > 8 ? " …" : ""}`);
  } else {
    pass(`${uniq.length} 条文案两种语言都有 (${langs.join(", ")})`);
  }
}

checkPage("index.html");
checkPage("settings.html");
checkI18nParity();

console.log(failed ? `\n${failed} 项失败` : "\n全部通过");
process.exit(failed ? 1 : 0);
