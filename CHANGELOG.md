# Changelog

本项目遵循 [Keep a Changelog](https://keepachangelog.com/zh-CN/1.1.0/) 和 [语义化版本](https://semver.org/lang/zh-CN/)。

版本号的唯一来源是 `src-tauri/Cargo.toml` 的 `version` 字段 —— `tauri.conf.json` 刻意不写 `version`，缺省时 Tauri 会读 Cargo.toml，避免两处不一致。

发布走 `tools/release.ps1`：它把下面的 `[Unreleased]` 提升成带日期的版本段、构建 release、打 tag、建 GitHub Release。所以**新改动都写在 `[Unreleased]` 下面**。

## [Unreleased]

### Changed

- **重构成「一个会话一只宠物」的工作空间 dock**，取代原来只显示聚合赢家的单张卡片：
  - 每个 `session_id` 一只宠物，按 `cwd` 末段分组成工作空间，一行一个项目
  - 点宠物切换查看该会话详情；宠物顺序按 `first_seen` 固定，不会因为 HashMap 迭代顺序乱跳
  - 工作空间超过 5 行转滚动，选中的宠物会被自动滚进视野
- **折叠 / 自动展开**：干活或空闲时收成一条只有宠物点阵的胶囊；任一会话进入等待态时强制展开并选中它，处理完自动收起。手动开合会被记住并压制自动规则，但**新的**等待事项（靠等待集合签名变化识别）仍能强制展开。
- 折叠态工作空间 ≤ 3 个时显示项目名，多了只留点阵 —— 7 个项目名塞进折叠条只会全被截成无信息量的碎片，名字在展开态和 tooltip 里都拿得到。
- 窗口尺寸改为随内容动态调整，**同时锚定右边和底边**朝左上生长。只锚底边的话折叠态点阵变长会冲出屏幕右缘，只锚左上角的话展开会长出屏幕底部。
- 位置持久化从左上角改存右下角，和上面的锚定方向一致（`window-position.json` → `window-anchor.json`，旧文件会被忽略并回落默认位置）。

### Fixed

- 事件名 `pet://state` → `pet://view`，命令 `get_state` → `get_view`（载荷从单个聚合状态改成完整会话树）。
- 修正 README 里「改 `ui/` 保存后刷新窗口即可」的错误说法：前端资源由 `tauri-build` 在**编译期嵌入**二进制，改了必须重新 `cargo build`。

## [0.1.0] - 2026-07-28
### Added

- **常驻置顶状态挂件** —— 透明无边框窗口，不进任务栏，整张卡片可拖动。
- **hook 驱动的状态机** —— Claude Code 的 hook 以 `type: "http"` POST 到 `127.0.0.1:47800`，映射为五个状态：
  - `working`（蓝，转圈，显示当前工具如 `Bash: npm test`）
  - `waiting-permission` / `waiting-input`（红，呼吸发光，`!` 角标）
  - `idle`（灰，闭眼）
  - `done`（绿，笑眼）
- **多会话聚合** —— 按 `session_id` 分别记账，上镜优先级为「等你操作 > 干活中 > 完成 > 空闲」，角标显示 `等待数/总数`。
- **窗口位置持久化** —— 存在 `%APPDATA%\com.opsmateai.claude-pet\window-position.json`；恢复前校验坐标仍落在可用显示器内，否则回落右下角（插拔扩展屏后不会把挂件丢到看不见的地方）。
- **开机自启** —— 托盘勾选项，另有 `--enable-autostart` / `--disable-autostart` / `--autostart-status` 供脚本无头调用。
- **托盘菜单** —— 显示当前版本、自启开关、退出。窗口没有标题栏，托盘是唯一退出入口。
- **命令行安装** —— `tools/install.ps1`，从 GitHub Release 拉取安装，支持 `-Autostart` 和 `-Uninstall`。
- **发布脚本** —— `tools/release.ps1`，改版本号、构建、打 tag、发 Release 一条龙。
- **事件模拟器** —— `tools/simulate-events.ps1`，不依赖真实 Claude Code 活动即可验证全部状态。

### Notes

- 每 5 秒重新 `set_always_on_top(true)`：Windows 上独占全屏程序和 UAC 安全桌面会抢走 topmost。
- 窗口尺寸刻意贴合宠物本体（248×96）。Tauri 的 `setIgnoreCursorEvents` 是整窗开关、没有 per-region，把窗口做小可以绕开整套命中测试。
- 前端无构建步骤：`frontendDist` 直接指向 `../ui`，靠 `withGlobalTauri` 注入 `window.__TAURI__`。
- 窗口 `visible: false` 起，摆好位置后才 `show()`，避免在默认位置闪一下再跳走。

[Unreleased]: https://github.com/zq940222/claude-pet/commits/main
