# Changelog

本项目遵循 [Keep a Changelog](https://keepachangelog.com/zh-CN/1.1.0/) 和 [语义化版本](https://semver.org/lang/zh-CN/)。

版本号的唯一来源是 `src-tauri/Cargo.toml` 的 `version` 字段 —— `tauri.conf.json` 刻意不写 `version`，缺省时 Tauri 会读 Cargo.toml，避免两处不一致。

发布走 `tools/release.ps1`：它把下面的 `[Unreleased]` 提升成带日期的版本段、构建 release、打 tag、建 GitHub Release。所以**新改动都写在 `[Unreleased]` 下面**。

## [Unreleased]

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
