# Changelog

本项目遵循 [Keep a Changelog](https://keepachangelog.com/zh-CN/1.1.0/) 和 [语义化版本](https://semver.org/lang/zh-CN/)。

版本号的唯一来源是 `src-tauri/Cargo.toml` 的 `version` 字段 —— `tauri.conf.json` 刻意不写 `version`，缺省时 Tauri 会读 Cargo.toml，避免两处不一致。

发布走 `tools/release.ps1`：它把下面的 `[Unreleased]` 提升成带日期的版本段、构建 release、打 tag、建 GitHub Release。所以**新改动都写在 `[Unreleased]` 下面**。

## [Unreleased]

### Added

- **默认摆到屏幕顶部居中 + 悬停窥视 + 拖到边缘入坞** —— 三条合起来是一套「停在顶上不挡路，要看碰一下」的交互。
  - **默认位置改成 `top-center`**。只影响全新安装：已有的 `prefs.json` 写着明确的值，换默认值不该把老用户挪走
  - **鼠标移上去展开、移开收起**。离开有 180ms 缓冲 —— 尺寸变化本身会让 WebView 抛出假的 leave/enter，不缓冲会在边界上抖；进入不缓冲，即时感是这个交互的全部价值
  - 展开变成**三个并列理由的「或」**：有人在等你 / 用户钉住 / 鼠标在上面。**有会话在等你时鼠标移开也不收起** —— 那会把挂件存在的意义藏起来。原来 `pinned` 的三态表达不了「悬停」这个临时状态
  - **拖到离屏幕边 60px 内松手就贴上去并收起**，四条边分别判断（拖到角落两边都贴），留 2px 缝而不是贴死（透明圆角贴死之后鼠标很难从屏幕最边缘滑进去），底边额外避让任务栏
  - **入坞会把摆放模式写成 `free` 并落盘**，设置窗口的下拉框跟着变。吸附模式下拖动本来不留痕，不改模式的话下一次 `resize_pet` 会把它弹回去、用户看到的是「拖了但没用」。之前拒绝「拖一下自动切模式」是因为隐式状态变化，而入坞是用户主动拖到边上的结果、意图明确，且这次切换是**看得见**的
  - **拖动结束靠 `Moved` 停跳 250ms 判断** —— Tauri 的拖动区域不产生 drag-end 事件。不用 `GetAsyncKeyState` 查鼠标键：要么轮询很密，要么在键盘/触摸板惯性移动时判断错

- **用量 / 成本面板**（#11）—— 设置窗口的「用量」组，每个启用的 agent 一张卡：token、成本（有的话）、真实配额条 + 重置倒计时。另有 `claude-pet.exe --usage` 把同样的数据以 JSON 打到 stdout（诊断走 stderr，所以重定向拿到的是干净 JSON）。
  - **全部读自本地文件，不联网、不碰凭据。** TermiPet 的做法是拿本机凭据请求官方端点，我们刻意不做 —— 一个常驻置顶的小挂件不该多出「凭据读取」和「网络出口」两个面
  - **不内置价目表。** Claude Code 只有 token，算美元就得内置价格；价格会变，过期的表是**默默显示错数字**，比没有金额更坑人。所以只显示 agent 自己算好的成本（OpenClaw 的 `estimatedCostUsd`），其余只显示 token 并在卡片上说明原因。同理 Codex 的 `rate_limits` 整块为 null 时**不渲染配额条**，而不是显示「已用 0%」
  - **按 `requestId` 去重** —— 一次 API 请求会写多条 assistant 行、每条都带同一份 `usage`。实测本机一个转录 1354 行对 613 个 requestId（虚高 161.7%），全部转录 6531 行对 2855 个。另排掉 `model` 为 `<synthetic>` 的行
  - **Codex 的累计值取最后一条不相加** —— `info.total_token_usage` 是会话累计而非增量，逐条加会把同一会话重复计入几十次。`rate_limits` 是账户级的，取最新的一份
  - **Hermes 暂不读**（数据在 SQLite，要加 `rusqlite`、二进制约 +1MB）。卡片上写明原因而不是让它从列表里消失

- **支持 Codex / Hermes / OpenClaw**（#13）—— 设置窗口里勾选要盯哪些 agent，默认只开 Claude Code。四个 agent 的处理方式**不一样**，因为实测能拿到的东西不一样：
  - **Codex 是一等公民**，和 Claude Code 完全同构：按 cwd 分工作空间、一个会话一只宠物。状态从 `~/.codex/sessions/**/rollout-*.jsonl` 的**尾部**读 `task_started` / `task_complete` 两个边界事件（语义等同 Claude Code 的 `UserPromptSubmit` / `Stop`，所以复用同一套状态名和视觉）。刻意不看 `agent_message` / `function_call` —— 那些一轮里出现几十次，会把「刚说完一句话」误当成「结束了」
  - **Hermes / OpenClaw 各一只 gateway 宠物**，放在以自己命名的工作空间里，只回答「在不在跑」。它们**不是**按项目的交互式 agent：Hermes 的 `sessions` 表虽有 `cwd` / `ended_at` 列，但本机 22 条里只有 2 条 `cwd` 非空、16 条 `ended_at` 永远是 null，硬做成会话宠物会得到一堆永远「在跑」、永远归不进项目的僵尸；OpenClaw 是单 agent 从 20 多个聊天入口进来，`workspaceDir` 恒为 gateway 自己的工作区，`status` 只有 `done`/`failed`/`timeout`（运行结果，不是此刻状态）
  - **没在跑就不产出宠物**，而不是加一个 `offline` 状态 —— 没在跑就没什么要盯的，一只常驻的灰色宠物只是在占地方
  - **只有 Claude Code 有 hook**，其余靠 5 秒轮询。Codex 的 `notify` 是 `config.toml` 里的**单个**槽位，本机那格已被 OpenAI 自己的 `codex-computer-use.exe` 占着，抢过来会弄坏用户已有的功能；gateway 则本来就没有「一次会话结束」这种适合推事件的时机。设置界面标了「轮询，有延迟」，免得被当成 bug
  - 轮询**只在指纹变化时** emit（指纹含 id + state + detail，刻意不含 `updated_ms`）。无条件推会让前端每 5 秒重渲染，而重渲染要重新测量卡片并调 `resize_pet` —— 等于每 5 秒动一次窗口
  - 对轮询的 agent 扫描结果**覆盖**已有状态，对 Claude Code 不覆盖。不区分的话 Codex 宠物会永远停在第一次扫到的状态，整个轮询白做
  - **判活方式按各自有的信号来**：OpenClaw 走 `gateway.port` 的裸 TCP 探测（**刻意不读**同一段里的 `auth.token`，建连不需要认证，读它是多余的暴露面）；Hermes 走 `gateway_state.json` 的 pid，并额外要求 `gateway_state` 字段自己也说 `running`（pid 会被回收，只看 pid 可能在 gateway 被 kill -9 后虚报）
  - **agent 走边框样式，不走颜色** —— 颜色整条通道已被状态占满（红 = 要你动手）。实线 = Claude Code、虚线 = Codex、双线 = gateway，且 `border-style` 不改变盒模型尺寸，不会牵动「量卡片得出窗口尺寸」那套逻辑。折叠态 11px 的点刻意不区分：那个尺寸上形状差异不可读，而折叠条只需要回答「有没有红的」
  - 缓存版本升到 3；gateway 宠物 `cwd` 为空串，双击跳编辑器会被拦住并提示而不是报一个看不懂的错
  - 尊重 `CODEX_HOME` / `HERMES_HOME` / `OPENCLAW_CONFIG_DIR`

- **Windows 安装包**（#17）—— `cargo tauri build --bundles nsis` 产出 1.21 MB 的 `claude-pet-x.y.z-x64-setup.exe`。`installMode: currentUser` → `RequestExecutionLevel user`，**不弹 UAC**，装到 `%LOCALAPPDATA%\Claude Pet\`，建开始菜单 + 桌面快捷方式，安装界面可选简体中文 / English。发布时同时产出安装包和原来的绿色 zip。
  - **`NSIS_HOOK_PREINSTALL` 清理 `install.ps1` 的旧副本** —— 两种安装方式装到不同目录（`ClaudePet` vs `Claude Pet`），但自启只有**一个**值名 `HKCU\...\Run\Claude Pet`。不清理的话会留下两份 exe，自启指向后装的那份，另一份成为永远不会被更新的孤儿。反方向由 `install.ps1` 检测卸载注册表键后**直接拒绝**安装
  - **偏好刻意不删** —— 卸载器的「删除应用数据」只清 `%LOCALAPPDATA%\com.opsmateai.claude-pet`（WebView2 缓存），我们的 `prefs.json` / `sessions.json` / `window-anchor.json` 在 `%APPDATA%` 下，卸载后保留、重装即恢复
  - 补上 `bundle.publisher` / `copyright` —— 之前 exe 的版本资源里 `CompanyName` 和 `LegalCopyright` 是空的，配上未签名的 SmartScreen 警告观感很差
  - 产物重命名去掉空格：tauri 用 `productName` 当文件名，`Claude Pet_...` 在下载 URL 里会变成 `%20`
  - **语言名是 `SimpChinese` 不是 `SimplifiedChinese`** —— 必须逐字对上 NSIS 的 `.nlf` 文件名。写错时 tauri 只警告一句「not translated」，真正的中止发生在 `makensis` 找不到 `.nlf`，两处分离容易看漏
  - **`tauri-bundler` 下载 NSIS 工具链没有重试** —— 网络抖一下就抛 `io: unexpected end of file`，看起来像仓库坏了。同一个文件 `curl --retry` 一次就成。`release.ps1` 在这步失败时直接打印手动布置命令，README 记了缓存布局和两个 SHA1
  - **仍然不上 `tauri-plugin-updater`** —— 安装包解掉了当初否掉它的产物形式问题，但私钥丢失会让所有已装版本永久失去更新能力，且发布流程要多产 `latest.json`。换来的只是少一次手动确认，维持「新版本提醒」

- **[ADR 0001](docs/adr/0001-no-tab-precise-jump-back-on-windows.md)**（#12）—— 调研「跳回会话所在的终端 tab」，结论是**不实现**。
  - `wt` 确实有 `focus-tab --target`（`TerminalApp.dll` 里可查到），此前判断「不支持」是无效测试所致（PATH 上的 `wt.exe` 是 0 字节的 WindowsApps 别名 stub）
  - 但 `wt` **没有任何查询类子命令**，操作系统侧也无法把 ConPTY 映射到 tab 索引 —— 能命令、不能发现目标索引，等于不可实现
  - 实测本机所有 Claude Code 会话**都不在终端里**：真实宿主是 Claude 桌面应用，对全部会话只暴露一个 HWND 和一个不含项目名的标题
  - 唯一未走到底的线索是 `claude://` 深链（协议已注册，二进制含 `deeplink`，但无 `--session` 类字符串）；刻意没探测，因为那会导航用户正在使用的应用
  - README 中相应说法从「另有 issue 调研」更新为结论 + ADR 链接

### Fixed

- **一悬停就会自我入坞并改掉摆放模式** —— `top-center` 把窗口停在离顶边 12px 处，**远在 60px 入坞阈值之内**，而 `resize_pet` 每次展开/收起都 `set_position`、那会发 `Moved`。不区分程序化移动和用户拖动的话，碰一下挂件就「入坞」并把模式改成 `free`。现在所有程序化 `set_position` 都走 `set_position_quietly`，它把随后 400ms 内的 `Moved` 标记成自己人。用时间窗而不是布尔量：`Moved` 是异步送达的，`set_position` 返回时事件还没到，布尔量一置一清必然漏；而时间窗对真实拖动安全，因为拖动是一串持续的 `Moved`。

- **用量面板少算三成，且界面上毫无迹象** —— 第一版抄了 `discover.rs` 的字节上限模式，把每个文件封在 8MB。本机当前会话的转录有 11.5MB，于是 `cache_read` 显示 767,420,613 而真实值是 1,119,001,795。`discover.rs` 封顶是对的（它只要头部的 `cwd`），用量要全部行，任何上限都等于默默少算 —— 正是这个面板拒绝内置价目表的那个理由。现已去掉上限，并与独立重算逐项比对到 drift 0.000%。

- **gateway 宠物的 detail 在英文界面下混中文**（#13）—— 那行文案是探测时现拼的，第一版写了中文字面量。现在走 `i18n`，测试里加了 CJK 断言守住。

## [0.2.0] - 2026-07-29
### Added

- **`LICENSE`：Apache-2.0**（#15）—— apache.org 的原文（11358 字节 / 202 行 / 9 条条款，已校验）。`Cargo.toml` 补上 `license` 和 `repository` 字段。本项目只参考了 Open Island（GPL v3）的功能设计、未移植其代码，因此不受 GPL 传染。

- **新版本提醒**（#9）—— 启动查一次 GitHub Releases，有新版本就在概览行末尾追加提示；设置窗口可手动检查并给出可复制的升级命令。自动检查可关（是一次对外网络请求）。
  - **没有采用 `tauri-plugin-updater`**：读它的 Windows 逻辑（`updater.rs:883-913`）发现它解压 zip 后只认 `.exe`（当 NSIS 安装包）或 `.msi`，而我们发的是便携 zip 里的应用本体，被静默安装参数执行不会安装任何东西。要用它就得改发 NSIS 安装包，代价是丢掉便携免管理员的发布形式、多一把丢了就让所有已装版本永久失去更新能力的私钥、以及安装路径和自启路径全变。换来的只是少一次手动确认，对个人项目不值
  - GitHub API 对 releases 返回 `Access-Control-Allow-Origin: *`（实测），所以检查完全在前端 `fetch`，**Rust 侧零新增依赖**
  - 提示挂在概览行末尾而非徽标/弹窗：「有新版本」的紧急程度远低于「有会话在等你」，不该抢视觉
  - 一次性命令 `get_lang` 合并为 `get_boot`（语言 + 版本 + 仓库 + 是否自动检查），避免一个个往上堆
- **`tools/check-ui.js`** —— 前端静态检查：按 HTML 里的顺序把每个页面的脚本放进**同一个** context 真实执行，验证共享脚本零全局泄漏、并比对两种语言的文案完整性（现 87 条）。这是把一次真实事故变成可重跑的守卫，`node --check` 的单文件检查永远发现不了那类跨文件冲突。

- **界面国际化**（#10）—— 简体中文 / English，跟随系统或手动指定，设置窗口里改。
  - 语言由 **Rust 侧解析**（`"auto"` → 系统 UI 语言，读 `GetUserDefaultUILanguage`），前端不自己猜 —— 两边各猜一次必然会有不一致的时候
  - Rust 生成的固定文案（`思考中` / `恢复的会话` 等）也按语言产出。**没有**为此在协议里加 `detail_key` 字段：那要改缓存格式和 6 处构造点，而 Rust 本来就读得到偏好。代价是切换语言后缓存里的旧文案要等下一个事件才刷新，会自愈
  - 挂件改语言就地重渲染（`pet://lang` 事件）；**托盘菜单要重启才跟着变**（Tauri 菜单项文本不能原地改），设置界面里注明了
  - 缺失的 key 直接显示 key 本身，漏翻在界面上一眼可见，而不是变成空白

### Fixed

- **前端整体失效（严重）**：`i18n.js` 顶层的 `function t()` 在经典 script 里会创建**全局** `t`，而 `app.js` / `settings.js` 都有 `const t = ...` —— 撞车导致**解析期** SyntaxError，整个文件一行不执行，连 `window.onerror` 都注册不上，表现为「界面空白且毫无报错」。`node --check` 单文件检查查不出这类跨文件全局冲突。`i18n.js` 现在包在 IIFE 里，只导出 `window.I18N`，并加了「两文件同作用域共存 + 零全局泄漏」的回归测试。
- **权限按钮在没有权限请求时也显示**：`.actions { display: flex }` 是作者样式，会盖过 UA 样式表的 `[hidden] { display: none }`，所以 `hidden` 属性对它无效。补了 `.actions[hidden] { display: none }`。
- 挂件加了全局 `error` / `unhandledrejection` 处理，把异常写进**折叠态也可见**的 strip（写进详情行没用 —— 折叠态下它是 `display: none`）。另加 `--devtools` 启动参数，仅 debug 构建。

- **在挂件上批准 / 拒绝权限**（#6，默认关闭）—— 挂件上直接出现「允许 / 拒绝」按钮，不用切回终端。
  - 拦截走**独立 URL 路径** `/permission`，普通事件走 `/`：「哪些工具要挂住」完全由那条 hook 的 matcher 决定，挂件里不再存一份 pref（两处各存必然不一致）
  - **只有 `/permission` 走独立线程**，普通事件仍在主循环顺序处理，避免同一会话的事件被线程调度打乱。实测权限请求挂住期间普通事件仍是 4–32ms
  - **超时 fail-open**：内部等 30 秒，超时回空 200 = 不给决定，交还 Claude Code。挂件崩了或人不在绝不会卡死 Claude Code。hook 侧 timeout 设 40 秒，必须长于内部等待
  - 这条 hook 刻意**不带 `async`** —— `async: true` 是发射后不管，Claude Code 不会等决定
  - 改 matcher 是**替换而非叠加**（先摘旧条目再装），否则两条都命中、同一次调用要点两次
  - 新增 `--install-permission-hook [matcher]` / `--uninstall-permission-hook`，默认只拦 `Bash`（`*` 会让每次工具调用都等人点）
  - 装卸只动自己那条，实测普通 hook 和其余配置（statusLine、24 plugin）逐字节不变

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
