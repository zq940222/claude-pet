# Changelog

本项目遵循 [Keep a Changelog](https://keepachangelog.com/zh-CN/1.1.0/) 和 [语义化版本](https://semver.org/lang/zh-CN/)。

版本号的唯一来源是 `src-tauri/Cargo.toml` 的 `version` 字段 —— `tauri.conf.json` 刻意不写 `version`，缺省时 Tauri 会读 Cargo.toml，避免两处不一致。

发布走 `tools/release.ps1`：它把下面的 `[Unreleased]` 提升成带日期的版本段、构建 release、打 tag、建 GitHub Release。所以**新改动都写在 `[Unreleased]` 下面**。

## [Unreleased]

### Added

- **全局快捷键**（#8）—— 默认 `Ctrl+Alt+P` 展开/收起、`Ctrl+Alt+N` 跳到下一个在等你的会话，设置窗口可改。
  - 只在**等待中**的会话里循环：快捷键的用途就是「谁在等我」，串上干活中和空闲的只会让人多按几次
  - `on_shortcut` 的回调按下和松开都会触发，**只处理按下** —— 否则每次触发两遍
  - 注册失败只当警告返回，不阻止其它设置保存；实测一个键被占用（`HotKey already registered`）或写法非法时，**另一个键仍然注册成功**
  - 留空 = 不注册（明确的「我不要这个键」，不是错误）
  - Rust 侧只 emit 动作名，折叠/选中状态仍只住在前端状态机里 —— 两边各存一份必然不一致
  - release exe 3.37 → 3.57 MB

- **双击宠物跳回编辑器**（#7）—— 用会话的 cwd 打开 Cursor / VS Code / Windsurf / JetBrains 各款；设置窗口可指定，只列本机装了的。
  - `Session` 增加 `cwd` 字段（`project` 只是末段，拿不回原路径），缓存版本 1 → 2
  - 自己实现 `which`（`PATH` + `PATHEXT`）：`code`/`cursor` 是 `.cmd` shim，Rust 的 `Command` 直接调 `CreateProcessW` 执行不了，需按扩展名决定是否经 `cmd`
  - **指定了具体编辑器就不回落**：用户明确选了 VS Code，静默换成 Cursor 是在骗人
  - 切断子进程 stdio 继承 —— 不切的话编辑器会持有继承来的管道，等待挂件输出的调用方会挂到编辑器关闭为止（实测挂死 7 分钟）
  - 新增 `--open <dir>` 诊断参数

### Fixed

- **控制台中文乱码**：启动时把控制台输出代码页设为 UTF-8（Windows 默认 GBK，而 Rust 写的是 UTF-8）。此前的应对是「日志一律写英文」，属于绕开而非解决；现在诊断信息可以正常用中文了，`spawn_server` 的绑定失败提示已改回中文。

### Added

- **设置窗口**（#5）—— 托盘 →「设置…」，或启动带 `--settings`。四组：通用（自启、会话发现时间窗）、提示音（开关、声音选择、试听）、Hooks（安装状态 + 一键装/卸）、关于（版本、端口、配置目录、Claude 配置路径）。
  - **没有「保存」按钮**：每项改完立刻落盘生效。设置项彼此独立，攒着提交只会让人怀疑存没存
  - 时间窗的「立即生效」= **用新窗口重扫一遍**，光改数字对已建好的会话表没有影响
  - 窗口**按需创建、关闭即销毁**：常驻一个隐藏 WebView2 会白占几十 MB，而常驻内存小是挂件的卖点
  - 自启和提示音同时存在于托盘和设置窗口（托盘适合快切，窗口需要可发现），所以设置窗口改完会**回写托盘勾选状态**
  - 前端值一律经 Rust 侧 `sanitise()` 收敛后才落盘；时间窗被 clamp 后界面会回读真实值，不留「显示 9999 实际存 1440」的假象
  - 独立的 `settings.css`，刻意不复用挂件那套（`body` flex + `flex-shrink:0` 尺寸测量的约束搬过来会互相坑）
  - 新增 `--settings` 启动参数；与其它 CLI 开关不同，它**不退出**
  - 时间窗范围 1–1440 分钟，兑现了 #1 里 `discover::DEFAULT_WINDOW` 的 `TODO(#5)`（该常量已删除）

- **提示音与静音开关**（#4）—— 进入等待态时响一声，托盘可开关。
  - **只在状态「变成」等待态时响**：判据是「新状态是 waiting 且不等于旧状态」。同一等待态的重复事件不响（否则连成噪音），但 `waiting-permission` ↔ `waiting-input` 会响，因为要你处理的事情换了。实测 8 个事件的转换序列响 3 次
  - 直接 FFI 调 winmm 的 `PlaySoundW` 播放 Windows 声音方案里已有的系统音：**不引入音频 crate、零字节体积增量**，且自动尊重系统里配的声音和音量。release exe 从 3.37 MB 到 3.45 MB，增量全部来自本轮几个新模块而非音频
  - 带 `SND_NODEFAULT`：用户把某事件设成「无声音」是明确意图，不退回蜂鸣
  - 用完即弃的线程里同步播放，而不是 `SND_ASYNC` —— 异步模式下字符串指针的生命周期要求无法从文档确证，而「大概没问题」不足以拿来写 unsafe
  - 静音状态存 `prefs.json`，跨重启保留；打开时试听一声
  - `sound` 字段手改写错时启动警告并回落 —— `SND_NODEFAULT` 下打错字是静默无声的，不校验用户只会以为功能坏了
  - 新增 `prefs.json` **刻意不用** `version` 门禁（与会话缓存相反）：偏好是用户意图，加字段时必须让旧文件继续可读，靠 `#[serde(default)]` 实现。已验证缺字段的旧格式能被正确补齐
  - README 记录了与 toast 脚本的声音重叠及三个处理选择

- **hook 一键安装 / 卸载**（#3）—— 新增 `--install-hooks` / `--uninstall-hooks` / `--hooks-status`，消掉了上手路径上唯一需要手工编辑 JSON 的一步。`install.ps1` 加了 `-WireHooks` 开关，`-Uninstall` 现在也会顺带摘掉 hook。
  - **合并而不是覆盖**：serde_json 开 `preserve_order` + 2 空格 pretty 打印，实测对真实 settings.json（4542 字节、含 `statusLine` 和 24 个 plugin）是逐字节一致的往返；不开这个 feature 的话默认 Map 会按字母序重排用户所有的键
  - **幂等**：判重看 hook url；且「没变就不写」，所以重装不会刷出多余备份
  - **只卸自己的**：只摘指向 `127.0.0.1:47800` 的 http hook。`Notification` 下并存的 toast（`command` 类型、matcher 更窄）不受影响
  - 改动前备份成 `settings.json.bak-<epoch 毫秒>`，写入走临时文件 + rename
  - 配置文件不存在（全新机器）时会创建；卸载会连 `hooks` 键一起移除，是安装的真正逆操作
  - 尊重 `CLAUDE_CONFIG_DIR`

- **会话状态跨启动持久化**（#2）—— 会话表落盘到 `%APPDATA%\com.opsmateai.claude-pet\sessions.json`，重启后状态和 detail 完整保留。自动发现（#1）只能还原「有哪些会话」，状态一律是 idle；两者合起来才是完整的「重启后接着用」。
  - 复用现有那个 5 秒线程落盘，「变了才写」（比较序列化后的字符串），最多丢 5 秒内的状态变化
  - 写入走临时文件 + rename：直接覆写的话，进程正好写一半时被杀会留下截断的 JSON
  - 带 `version` 字段；版本不符时整体丢弃而不是让 serde 误解析成半对半错的状态
  - 缓存损坏 / 版本不符 / 根本不是 JSON 都静默回落到空，绝不阻止挂件启动
  - 按每个会话自己的 `updated_ms` 过时间窗，而不是按文件保存时间 —— 挂件可能关了很久，不能因为文件是刚保存的就把几小时前停掉的会话当成当前状态
  - **缓存优先于扫描**：缓存在 `spawn_discovery` 之前加载，`merge_discovered` 跳过已存在的 session_id，所以缓存自动胜出。反过来的话重启后所有宠物会被扫描刷成灰色
- 窗口锚点的读写移进新的 `persist` 模块，与会话缓存共用配置目录逻辑和原子写入。

- **会话自动发现**（#1）—— 启动时扫 `~/.claude/projects/<编码 cwd>/<session_id>.jsonl`，把最近 30 分钟内活动过的会话恢复出来。此前挂件只知道「启动之后推过 hook 事件」的会话，所以开机自启后或挂件重启后已有会话是隐形的。
  - 尊重 `CLAUDE_CONFIG_DIR` 环境变量
  - cwd 取自 jsonl **内容**里的 `cwd` 字段，不从目录名反解 —— 目录名把 `:` 和 `\` 都换成了 `-`，而项目名本身也可能含 `-`（如 `Claude-Code-Short-Drama-Studios`），反解无法唯一还原
  - 只取项目目录下**直接**的 jsonl；subagent 转录在 `<project>/<session_id>/subagents/` 更深一层，天然排除，另有 `isSidechain` 作为第二道保险
  - 先按 mtime 过滤再决定开不开文件：本机 651 个转录里通常只有几个落在窗口内，实测 2–3ms
  - 恢复的会话状态一律为 `idle`（转录能确定会话存在，确定不了它此刻在干活还是在等你），真实事件会在几秒内纠正
  - 真实 hook 事件优先：已在表里的会话不被扫描结果覆盖

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
