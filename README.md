# Claude Pet

常驻置顶的桌面挂件，实时显示 Claude Code 在干什么、有没有在等你。

解决的问题：Claude Code 的通知弹窗会自动消失，一走神就错过，回头发现它已经停在那儿等了十分钟。

## 安装

```powershell
irm https://raw.githubusercontent.com/zq940222/claude-pet/main/tools/install.ps1 | iex
```

推荐带参数装 —— 一次把自启和 hook 都配好（`irm | iex` 没法传参，得用 scriptblock 形式）：

```powershell
& ([scriptblock]::Create((irm https://raw.githubusercontent.com/zq940222/claude-pet/main/tools/install.ps1))) -Autostart -WireHooks
```

装到 `%LOCALAPPDATA%\ClaudePet\`，建开始菜单快捷方式，然后启动。其它参数：`-Version x.y.z` 装指定版本、`-NoLaunch` 装完不启动、`-Uninstall` 卸载（会同时关自启、摘掉 hook）。

**没带 `-WireHooks` 的话还有一步**：挂件只有在 Claude Code 把事件 POST 给它之后才会亮，需要在 `~/.claude/settings.json` 里配 hook。可以随时补上：

```bash
claude-pet.exe --install-hooks
```

hook 在 session 启动时加载，所以配完要**开一个新的 Claude Code 会话**才生效。

需要 [WebView2 运行时](https://developer.microsoft.com/microsoft-edge/webview2/)（Win11 和更新过的 Win10 自带）。

## 架构

```
Claude Code hooks (type: "http")
        │  POST 事件 JSON
        ▼
127.0.0.1:47800  ← tiny_http，跑在 Tauri 的 Rust 侧后台线程
        │  归类成状态，按 session_id 建会话，按项目分组成工作空间
        ▼
  app.emit("pet://view")
        │
        ▼
   WebView（CSS/SVG 渲染 + 折叠状态机）
```

用 HTTP 而不是状态文件中转，是因为官方文档明确：**hook 的 HTTP 连接失败或超时属于非阻塞错误，执行继续**。所以挂件没启动时 hook 静默失败，完全不影响 Claude Code 干活 —— 不需要任何降级逻辑。

## 状态机

| 状态 | 触发事件 | 表现 |
| --- | --- | --- |
| `working` | `UserPromptSubmit`、`PreToolUse`、`PostToolUse` | 蓝色，转圈，眼珠左右瞟，第二行显示当前工具（如 `Bash: npm test`） |
| `waiting-permission` | `Notification` / `permission_prompt` | 红色，呼吸发光，睁大眼 + `!` 角标 |
| `waiting-input` | `Notification` / `agent_needs_input` | 同上 |
| `idle` | `Notification` / `idle_prompt`、`Stop`、`SessionStart` | 灰色，闭眼 |
| `done` | `Notification` / `agent_completed` | 绿色，笑眼 |

## 会话自动发现

挂件启动时会扫本地转录，把已经在跑的会话恢复出来 —— 否则开机自启后挂件是空的，要等下一个 hook 事件才冒出东西。

```
~/.claude/projects/<编码后的 cwd>/<session_id>.jsonl
```

文件名就是 `session_id`，mtime 是最后活动时间，**cwd 取自文件内容里的 `cwd` 字段**。不从目录名反解 cwd：目录名把 `:` 和 `\` 都替换成了 `-`，而项目名本身也可能含 `-`（`Claude-Code-Short-Drama-Studios`），反解无法唯一还原。

只认最近 30 分钟内动过的转录 —— 本机有 651 个历史转录，不设窗口会一次冒出几十个早已关掉的会话。先按 mtime 过滤再决定开不开文件，所以扫描很便宜（实测 2–3ms）。

只取项目目录下**直接**的 jsonl。subagent 的转录在 `<project>/<session_id>/subagents/` 更深一层，这条规则天然把它们排除，否则一个会话会显示成好几只宠物。

恢复出来的会话一律是 `idle` 状态：转录能告诉我们会话存在，但告诉不了它此刻是在干活还是在等你。真实 hook 事件会在几秒内纠正过来，且**永远优先于**扫描结果。

尊重 `CLAUDE_CONFIG_DIR` 环境变量。

## 状态跨启动保留

扫描只能还原「有哪些会话」，还原不了状态。所以会话表还会落盘到 `%APPDATA%\com.opsmateai.claude-pet\sessions.json`，重启后红色的还是红色、detail 原文还在。

**缓存优先于扫描。** 缓存在扫描之前加载，而合并逻辑会跳过已存在的 `session_id` —— 冲突解决就靠这个顺序，不需要额外判断。方向不能反：缓存带着真实状态，扫描只能填 `idle`，反过来会把重启后的宠物全刷成灰色。

写入走临时文件 + rename。挂件是靠托盘退出或被 kill 的，直接覆写时进程正好写一半就会留下截断的 JSON。

缓存文件带 `version` 字段。损坏、版本不符、根本不是 JSON —— 三种情况都静默回落到空状态，绝不阻止挂件启动。它是缓存，不是真相来源。

过滤用的是每个会话自己的 `updated_ms` 而不是文件保存时间：挂件可能关了很久，缓存里既有几分钟前还活跃的会话，也有几小时前就停了的，不能因为文件「刚保存」就把后者当成当前状态。

## 一个会话 = 一只宠物，一个项目 = 一个工作空间

`session_id` 唯一确定一只宠物；`cwd` 的末段作为工作空间名把宠物归组。宠物顺序按 `first_seen` 排 —— HashMap 的迭代顺序是随机的，不排序图标每次刷新都会乱跳。

`SessionEnd` 会把宠物摘掉，否则关掉的终端会一直挂着一只。会话的 `cwd` 变了会重新归组。

**展开态**：每个项目一行，行内是该项目下的宠物（带序号，可点选）；下方是选中宠物的详情。超过 5 行转滚动，选中的宠物会被滚进视野。

**折叠态**：一条紧凑胶囊，只有宠物点阵。工作空间 ≤ 3 个时显示项目名，多了就只留点 —— 7 个项目名塞进折叠条只会全被截成 `my-vid...` 这种没信息量的碎片，名字在展开态和 tooltip 里都拿得到。

## 什么时候自动展开

| 情况 | 行为 |
| --- | --- |
| 任一会话进入 `waiting-permission` / `waiting-input` | **强制展开**并自动选中它 |
| 所有等待都处理完 | 自动收起 |
| 手动点开合按钮 | 记住你的选择，压制上面的自动规则 |
| 手动收起后又来了**新的**等待事项 | 重新强制展开 |

最后一条靠比对「等待中会话集合的签名」实现：只有签名变化才算新事项。所以手动收起能真正生效（同一批等待不会反复弹回来），但新的待处理事项一定叫得到你。

## 从源码跑

```bash
cd src-tauri && cargo run
```

Release 构建（产物在 `src-tauri/target/release/claude-pet.exe`）：

```bash
cd src-tauri && cargo build --release
```

**托盘菜单是唯一的控制入口** —— 窗口没有标题栏也不在任务栏。菜单里有当前版本、开机自启开关、提示音开关、退出。

## 提示音

挂件是视觉的，人离开屏幕就失效，所以进入等待态时会响一声。

**只在状态「变成」等待态时响一次。** 判据是「新状态是 waiting 且不等于旧状态」：

| 转换 | 响？ | 为什么 |
| --- | --- | --- |
| `working` → `waiting-input` | ✅ | 有新东西要你处理 |
| `waiting-input` → `waiting-input` | ❌ | 同一件事的重复事件，连着响会变噪音 |
| `waiting-input` → `waiting-permission` | ✅ | 要你处理的事情换了 |
| `waiting-*` → `working` / `idle` / `done` | ❌ | 不需要你了 |

播放的是 Windows 声音方案里**已有**的系统音（默认 `Notification.Default`），直接 FFI 调 winmm 的 `PlaySoundW`。所以不引入任何音频 crate、**零字节**体积增量，而且自动尊重你在系统设置里配的声音和音量 —— 自带一个 wav 反而会绕过这些。

带 `SND_NODEFAULT`：你把某个事件设成「无声音」是明确意图，不该被我们退回蜂鸣绕过。

静音开关在托盘菜单（正向表述：勾上 = 会响），状态存在 `%APPDATA%\com.opsmateai.claude-pet\prefs.json`。打开时会立刻试听一声，好知道自己听到的是什么。

`prefs.json` 里的 `sound` 字段可以手改，合法值见 `sound::AVAILABLE`。写错的话启动时会警告并回落到默认 —— `SND_NODEFAULT` 下打错字是**静默无声**的，不校验的话你只会以为提示音坏了。

### 和 toast 脚本的关系（会重叠）

如果你还配着 `claude-alert.ps1` 那个持久化 toast（matcher `permission_prompt|agent_needs_input`），它和挂件的提示音**会在同样的时机同时响**，而且 toast 用的是**循环闹铃**。三个选择：

1. **去掉 toast 的 hook** —— 挂件现在既有视觉也有声音，toast 的独特价值只剩「不点不消失」
2. **让 toast 静音** —— 把 `claude-alert.ps1` 里的 `<audio>` 换成 `<audio silent="true"/>`，保留持久化视觉，声音交给挂件
3. **关掉挂件的提示音** —— 托盘里取消勾选，声音继续由 toast 负责

推荐 2：toast 的价值在于「离开屏幕回来还能看到」，声音这件事挂件做得更精确（只在状态真的变化时响一次，而不是循环）。

### 无头 CLI

给脚本用的，不需要点托盘：

```bash
claude-pet.exe --enable-autostart     # 退出码 0 = 成功
claude-pet.exe --disable-autostart
claude-pet.exe --autostart-status     # 0 = 已开启, 2 = 已关闭, 1 = 读不到

claude-pet.exe --install-hooks        # 写 hook 配置进 settings.json
claude-pet.exe --uninstall-hooks      # 只摘掉指向本挂件的
claude-pet.exe --hooks-status         # 0 = 全装好, 2 = 未装或只装了一部分, 1 = 读不了配置
```

### 改 settings.json 的三条硬约束

`--install-hooks` 改的是用户正在用的配置文件，所以：

**合并而不是覆盖。** `statusLine`、`enabledPlugins`、别人装的 hook 都不能动。往返用 serde_json 的 `preserve_order` + 2 空格 pretty 打印 —— 实测对真实 settings.json（4542 字节、含 statusLine 和 24 个 plugin）是**逐字节一致**的往返。少了 `preserve_order` 的话默认 Map 会按字母序重排所有键。

**幂等。** 判重看 hook 的 url，重复装不产生重复条目。而且「没变就不写」，所以重装也不会刷出一堆备份文件。

**只卸自己的。** 卸载只摘掉指向 `127.0.0.1:47800` 的 http hook。特别注意 `Notification` 事件下通常还有**别的** hook（比如 toast 脚本，`command` 类型、matcher 更窄），绝不能连它一起删。

改动前会备份成 `settings.json.bak-<epoch 毫秒>`，写入走临时文件 + rename。尊重 `CLAUDE_CONFIG_DIR`。

自启走 `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`，注册的是**调用时那个 exe 的路径**。所以要从安装好的位置调用，别从 `target\debug\` 开自启。

用退出码而不是 stdout 传结果，是因为 release 构建带 `windows_subsystem = "windows"`，没有控制台，`println!` 会掉进虚空。这条路径退出时 WebView 会打一行 `Failed to unregister class Chrome_WidgetWin_0`，无害，安装脚本会吞掉。

## 发布

```powershell
.\tools\release.ps1 -Bump patch      # 0.1.0 -> 0.1.1
.\tools\release.ps1 -Bump minor -DryRun
```

一条龙：改 `Cargo.toml` 版本号 → 把 CHANGELOG 的 `[Unreleased]` 提升成带日期的版本段 → `cargo build --release` → 打包 zip → commit + tag + push → `gh release create`（release notes 从 CHANGELOG 那一段抠出来）。

要求工作区干净，并且 `cargo` / `git` / `gh` 都在 PATH 上。新改动写在 CHANGELOG 的 `[Unreleased]` 下面。

## 前端没有构建步骤

`frontendDist` 直接指向 `../ui`，纯静态 HTML/CSS/JS，不用 npm、不用 vite。

**但改了 `ui/` 下的文件必须重新 `cargo build`** —— `tauri-build` 会把前端资源在编译期嵌进二进制，不是运行时从磁盘读。（`tauri-build` 对 `ui/` 有 rerun-if-changed，所以只改前端也会触发一次重编译，几秒钟。）

代价是拿不到 `@tauri-apps/api` 的 npm 包，所以靠 `withGlobalTauri: true` 注入的 `window.__TAURI__`。

## 几个刻意的设计决定

**窗口尺寸贴合卡片本体。** Tauri 的 `setIgnoreCursorEvents` 是整窗开关、没有 per-region，想做「宠物身上能点、周围透明区域穿透」就得自己写命中测试。把窗口做到刚好包住卡片直接绕开了这个问题 —— 没有大片透明区域，就不需要穿透。

**尺寸同时锚定右边和底边。** 前端量完卡片调 `resize_pet`，Rust 侧保持右下角不动、朝左上生长。只锚底边的话，折叠态点阵横向变长时窗口会冲出屏幕右缘；只锚左上角的话，展开时会长出屏幕底部。位置持久化存的也是右下角，和这个方向保持一致。

**卡片必须 `flex-shrink: 0`，别删。** 窗口尺寸是量卡片得出的，而卡片作为 body 的 flex 子项默认会被窗口宽度挤压 —— 于是形成循环依赖：窗口小 → 卡片被压 → 量出来还是小 → 窗口永远长不大，点阵被压扁裁断。禁止收缩后，卡片先按内容溢出视口，量到真实尺寸，下一帧窗口就跟上了。

同理，`.strip` 上**不能**设 `min-width: 0` 或 `overflow: hidden`：两者都会让它不参与卡片的 max-content 计算，结果一样是内容被压扁 —— 而 `overflow: hidden` 更坏，它把这个 bug 藏起来，看着像「刚好放下」。

**每 5 秒重新 `set_always_on_top(true)`。** Windows 上独占全屏程序和 UAC 安全桌面会抢走 topmost。这个调用是幂等的，不抢焦点。

**整张卡片是 `data-tauri-drag-region`。** 拖动窗口不用写一行 Rust。

**位置持久化。** 拖动时只更新内存，由那个 5 秒线程在变化时落盘 —— `Moved` 事件在一次拖动里会触发几百次，每次写文件太糙。代价是最多丢 5 秒内的移动。

恢复前会校验坐标**仍落在某个当前可用的显示器内**，否则回落右下角。笔记本插拔扩展屏后旧坐标会把挂件扔到看不见的地方，而这个窗口没有标题栏也不在任务栏，用户根本找不回来，只会以为程序坏了。

## 已知限制 / 下一步

- **任务栏躲避是估的** —— Tauri 的 monitor API 只给整屏尺寸拿不到工作区，右下角定位硬减了 56px。多显示器不同缩放下可能偏。
- **圆角外的小块透明区域仍会吃点击** —— 影响很小，真要修就得上命中测试。
- **`permission_prompt` 在 `bypassPermissions` 模式下几乎不触发** —— 这个模式下 Claude Code 不问权限。实际会响的是 `agent_needs_input` / `idle_prompt` / `agent_completed`。
- 端口硬编码 47800，改的话记得同步 `~/.claude/settings.json` 里的 hook url。
