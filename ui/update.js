// 新版本检查。挂件和设置窗口共用。
//
// 为什么在前端做：GitHub API 对 releases 返回 `Access-Control-Allow-Origin: *`
// （实测过），webview 能直接读，所以 Rust 侧不需要引入任何 HTTP 客户端依赖。
//
// 为什么**不**用 tauri-plugin-updater：它在 Windows 上解压 zip 后只认里面的
// `.exe`（当成 NSIS 安装包）或 `.msi`，会拿静默安装参数去执行。我们发的是
// 便携 zip 里一个应用本体，被那样执行不会安装任何东西。改成发 NSIS 安装包
// 才能用那个插件，代价是丢掉「便携、免管理员」这套发布形式，还多一把
// 一旦丢失就让所有已装版本永久失去更新能力的私钥。详见 README。
//
// 整个文件包在 IIFE 里，只导出 window.UPDATE。**别去掉这层包裹** ——
// 经典 script 共用全局作用域，顶层声明会和 app.js / settings.js 里的同名
// const 撞成解析期 SyntaxError，整个文件一行都不执行（i18n.js 上踩过）。

(function () {
  /// `https://github.com/owner/name` → `https://api.github.com/repos/owner/name/releases/latest`
  function apiUrl(repoUrl) {
    const m = /github\.com\/([^/]+)\/([^/?#]+)/.exec(repoUrl || "");
    if (!m) return null;
    const name = m[2].replace(/\.git$/, "");
    return `https://api.github.com/repos/${m[1]}/${name}/releases/latest`;
  }

  /// 语义化版本比较。返回 a > b 时为正。
  ///
  /// 只比数字段，预发布后缀（-rc.1 之类）一律忽略 —— 我们的 release.ps1
  /// 只产出 x.y.z，为不会出现的格式写解析没有意义。
  function cmp(a, b) {
    const pa = String(a).replace(/^v/, "").split(".").map((n) => parseInt(n, 10) || 0);
    const pb = String(b).replace(/^v/, "").split(".").map((n) => parseInt(n, 10) || 0);
    for (let i = 0; i < Math.max(pa.length, pb.length); i++) {
      const d = (pa[i] || 0) - (pb[i] || 0);
      if (d !== 0) return d;
    }
    return 0;
  }

  /// 查最新 release。
  ///
  /// 返回 `{latest, newer, url}`，或失败时 `{error}`。刻意不抛异常 ——
  /// 检查更新失败是完全正常的情况（离线、GitHub 挂了、限流），
  /// 不该让调用方用 try/catch 包起来，更不该影响挂件本身。
  async function check(repoUrl, currentVersion) {
    const url = apiUrl(repoUrl);
    if (!url) return { error: "bad repo url" };
    try {
      // GitHub 对匿名请求按 IP 限流 60 次/小时，启动查一次远远够用
      const res = await fetch(url, {
        headers: { Accept: "application/vnd.github+json" },
      });
      if (!res.ok) return { error: `HTTP ${res.status}` };
      const j = await res.json();
      const latest = String(j.tag_name || "").replace(/^v/, "");
      if (!latest) return { error: "no tag_name" };
      return {
        latest,
        newer: cmp(latest, currentVersion) > 0,
        url: j.html_url || repoUrl,
      };
    } catch (e) {
      return { error: e && e.message ? e.message : String(e) };
    }
  }

  window.UPDATE = { check, cmp };
})();
