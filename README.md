# Claude Pet

常驻置顶的桌面挂件，实时显示 Claude Code 在干什么、有没有在等你。

解决的问题：Claude Code 的通知弹窗会自动消失，一走神就错过，回头发现它已经停在那儿等了十分钟。

## 安装

### 安装包（推荐）

从 [Releases](https://github.com/zq940222/claude-pet/releases/latest) 下载 `claude-pet-x.y.z-x64-setup.exe`，双击。

装到 `%LOCALAPPDATA%\Claude Pet\`，建开始菜单和桌面快捷方式。**不需要管理员权限**（`installMode: currentUser`），安装界面可选简体中文或英文。升级就是再跑一次新版安装包。

安装包**没有签名**，所以 SmartScreen 会拦一下 —— 「更多信息」→「仍要运行」。签名需要花钱买证书，这个项目暂时没有。

装完还要配 hook，见下面那一节。

### 一行命令（不装安装包）

适合 CI、或者不想在「应用和功能」里多一条记录的人：

```powershell
irm https://raw.githubusercontent.com/zq940222/claude-pet/main/tools/install.ps1 | iex
```

推荐带参数装 —— 一次把自启和 hook 都配好（`irm | iex` 没法传参，得用 scriptblock 形式）：

```powershell
& ([scriptblock]::Create((irm https://raw.githubusercontent.com/zq940222/claude-pet/main/tools/install.ps1))) -Autostart -WireHooks
```

装到 `%LOCALAPPDATA%\ClaudePet\`（注意和安装包**不是同一个目录**），建开始菜单快捷方式，然后启动。其它参数：`-Version x.y.z` 装指定版本、`-NoLaunch` 装完不启动、`-Uninstall` 卸载（会同时关自启、摘掉 hook）。

### 两种方式只能选一个

自启只有一个注册表值名 `HKCU\...\Run\Claude Pet`，而两种方式装到不同目录。同时用会留下两份 exe，自启指向后装的那份，另一份变成永远不会被更新的孤儿。所以：

- 脚本装过之后跑安装包 —— 安装包的 `NSIS_HOOK_PREINSTALL` 会自动清掉 `%LOCALAPPDATA%\ClaudePet`
- 安装包装过之后跑脚本 —— 脚本会检测到并**直接拒绝**，让你先在「设置 → 应用」里卸载

两种方式都**不会**碰 `%APPDATA%\com.opsmateai.claude-pet\` 里的 `prefs.json` / `sessions.json` / `window-anchor.json`。换安装方式、甚至卸载重装，设置都还在。卸载器的「删除应用数据」只清 `%LOCALAPPDATA%\com.opsmateai.claude-pet`（WebView2 的缓存），刻意不动偏好 —— 悄悄把用户的设置清零比留下三个小 JSON 糟得多。

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

## 支持的 agent

设置窗口 →「通用」→「盯哪些 agent」。**默认只开 Claude Code** —— 装这个挂件的人不一定装了另外三个，默认全开会让没装的人白付扫描开销，装了的人则突然多出一堆自己没要求盯的宠物。

四个 agent 给的东西**不一样**，所以处理方式也不一样。这个不对称是实测数据决定的，不是偷懒：

| agent | 形态 | 「在等我」信号 | cwd / 项目 |
| --- | --- | --- | --- |
| **Claude Code** | 交互式、按项目 | hook 实时推送 | 转录里就有 |
| **Codex** | 交互式、按项目 | `task_started` / `task_complete`，**轮询** | `session_meta.cwd` |
| **Hermes** | gateway + 20 多个聊天平台 | 无实时状态 | 22 条会话里只有 **2 条**有 cwd |
| **OpenClaw** | gateway + 聊天入口 + cron | `status` 只是运行结果 | 恒为 gateway 自己的工作区 |

于是分两类：

- **Session 类**（Claude Code、Codex）—— 按 cwd 分工作空间，每个会话一只宠物，有真实的 working / waiting / idle 状态。
- **Gateway 类**（Hermes、OpenClaw）—— 每个 agent **一只**宠物，放在以它自己命名的工作空间里，只回答「在不在跑」。没在跑时**不产出宠物**，而不是加一个 `offline` 状态：没在跑就没什么要盯的，一只常驻的灰色宠物只是在占地方。

硬把 Hermes 做成一个会话一只宠物试过，撑不住：它的 `sessions` 表确实有 `cwd` / `git_repo_root` / `ended_at` 列，但本机 22 条里只有 2 条 `cwd` 非空、16 条 `ended_at` 永远是 null（CLI 不可靠地关会话），结果是一堆永远「在跑」、永远归不进项目的僵尸宠物。

### 只有 Claude Code 有 hook，其余靠轮询

- **Codex** 有 `notify` 配置，但那是 `config.toml` 里的**单个**槽位（一个数组，不是列表）。本机那格已经被 OpenAI 自己的 `codex-computer-use.exe` 占着，抢过来会弄坏用户已有的功能。
- **Hermes / OpenClaw** 是常驻 gateway，本来就没有「一次会话结束」这种适合推事件的时机。

所以有个 5 秒轮询线程。代价是这三个 agent 的状态有几秒延迟，设置界面里标了「轮询，有延迟」，免得被当成 bug。

轮询**只在指纹变化时**才 emit（指纹 = 各会话的 id + state + detail，刻意不含 `updated_ms`）。无条件推的话前端每 5 秒重渲染一次，而重渲染会重新测量卡片并调 `resize_pet` —— 也就是每 5 秒动一次窗口。

对轮询的 agent，扫描结果**会覆盖**已有状态；对 Claude Code 则不覆盖（hook 事件是权威）。这条区分是必须的：对 Codex 也「已存在就跳过」的话，宠物会永远停在第一次扫到的状态上，整个轮询白做。

### Codex 的状态从文件尾部读

`~/.codex/sessions/YYYY/MM/DD/rollout-<时间>-<uuid>.jsonl`，每行一条带 `type` 的记录。状态只看两个边界事件：

| `event_msg` 的 `payload.type` | 我们的状态 |
| --- | --- |
| `task_started` | `working` |
| `task_complete` | `idle`（轮到你） |

语义和 Claude Code 的 `UserPromptSubmit` / `Stop` 一致，所以复用同一套状态名和视觉。**刻意不看** `agent_message` / `function_call` 之类 —— 那些在一轮里出现几十次，拿它们判断状态等于把「刚说完一句话」误当成「结束了」。

从尾部回读 64KB。单条 `response_item` 可能很大（工具输出），只读几行的量会错过边界事件。回读窗口里找不到边界事件时返回「状态未知」并回落 `idle`，**不猜** —— 猜错方向会让「在等你」的宠物显示成「在干活」，那正好是这个挂件要解决的问题的反面。

尊重 `CODEX_HOME`。

### Gateway 的判活方式不一样，因为它们给的东西不一样

- **OpenClaw** 在 `openclaw.json` 里配了 `gateway.port`（`bind: loopback`），裸 TCP 连一下就是确定性判活。**刻意不读同一段里的 `gateway.auth.token`** —— 建立 TCP 连接不需要认证，把用户的 token 读进内存是完全多余的暴露面。
- **Hermes** 没有端口配置，但 `gateway_state.json` 有 `pid`、`gateway_state`、`active_agents` 和每个平台的连接状态，所以查那个 pid 还在不在。

pid 判活的已知弱点：pid 会被系统回收，理论上可能有别的进程占了同一个号，导致虚报「在跑」。所以额外要求 `gateway_state` 字段自己也说 `running`。代价是可能虚报而不是漏报 —— 对一只状态挂件，虚报「在跑」比虚报「没跑」轻。

尊重 `HERMES_HOME` / `OPENCLAW_CONFIG_DIR`。

### 视觉上怎么区分

**agent 走边框样式，不走颜色。** 颜色整条通道已经被状态占满了（红 = 要你动手），拿它再表达 agent 会让两种含义打架。实线 = Claude Code，虚线 = Codex，双线 = gateway。`border-style` 不改变盒模型尺寸，所以不会牵动那套「量卡片得出窗口尺寸」的逻辑。

折叠态的 mini 点**刻意不区分**：11px 上任何形状差异都不可读，而折叠条要回答的问题只有「有没有红的」。agent 身份在展开态、tooltip 和详情行里都有。

gateway 宠物没有项目目录（`cwd` 是空串），双击跳回编辑器会被拦住并提示，而不是去调用 `open_in_editor` 然后报一个看不懂的错。

## 用量 / 成本面板

设置窗口 →「用量」。每个启用的 agent 一张卡：token、成本（有的话）、账户配额条 + 重置倒计时。

**全部读自本地文件，不联网、不碰任何凭据。** [TermiPet](https://github.com/bleeeet/TermiPet)（macOS）的做法是拿本机登录凭据去请求官方端点，所以能给出统一的剩余额度。我们刻意不这么做 —— 那要求一个常驻置顶的小挂件读你的凭据并定期对外发请求，多出「凭据读取」和「网络出口」两个面。本地文件能到什么程度：

| agent | token | 成本 | 账户配额 |
| --- | --- | --- | --- |
| Claude Code | ✅ 每条 assistant 的 `usage.*` | ❌ 转录里没有 | ❌ 转录里没有 |
| Codex | ✅ `info.total_token_usage.*` | ❌ | ✅ **真实的** `rate_limits.primary` |
| OpenClaw | ✅ `inputTokens` / `outputTokens` | ✅ 现成的 `estimatedCostUsd` | ❌ |
| Hermes | 在 SQLite 里，暂不读 | 在 SQLite 里 | ❌ |

也有个命令行出口，方便核对和脚本化：

```bash
claude-pet.exe --usage
```

JSON 走 stdout、诊断走 stderr，所以 `claude-pet --usage > u.json` 拿到的是干净的 JSON。

### 不内置价目表

Claude Code 只有 token。要算出美元就得在仓库里写一张价目表，而**价格会变，过期的表是默默显示错数字** —— 一个错的金额比没有金额更坑人。所以：agent 自己算好的成本就显示（OpenClaw 的 `estimatedCostUsd`），没有的就只显示 token 并在卡片上说明原因。

同理，Codex 的 `rate_limits` 整块经常是 null（用 API key 或自建 provider 时没有套餐配额），那种情况**不渲染配额条**而不是显示「已用 0%」—— 后者是在撒谎。

### 一次请求会写多条 assistant 行，必须按 `requestId` 去重

Claude Code 的转录里，一次 API 请求会写**多条** `assistant` 行（一条内容块一行），**每条都带同一份 `usage` 对象**。本机实测：一个转录里 1354 条带 usage 的 assistant 行只对应 **613 个** `requestId`，直接累加 `output_tokens` 会虚高 **161.7%**；跨全部转录是 6531 行对 2855 个 requestId。

另外 `model` 为 `<synthetic>` 的要排掉 —— 那是本地合成的消息，没有真实 API 调用。

### 读文件**不能**设字节上限

第一版抄了 `discover.rs` 的 `MAX_HEAD_BYTES` 模式，封在 8MB。那是错的，而且错得很隐蔽：本机当前会话的转录有 11.5MB，于是面板里的 token 只有真实值的约 69%（`cache_read` 显示 767,420,613，真实是 1,119,001,795），**界面上没有任何迹象说明它被截断了**。

`discover.rs` 封顶是对的 —— 它只要头部的 `cwd`。用量要全部行，任何上限都等于默默少算。而「默默显示错数字」正是这个面板拒绝内置价目表的理由，自己在这儿犯一遍说不过去。

### Codex 的累计值要取最后一条，不能相加

每个 rollout 里的 `token_count` 事件带 `info.total_token_usage`，那是**该会话的累计值**而不是增量。所以每个文件只取最后一条再跨文件相加；逐条累加会把同一个会话重复计入几十次。

`rate_limits` 是**账户级**的、跨会话共享，所以取时间上最新的那一份而不是相加。

### Hermes 暂不读

它的数据其实是四个里最全的 —— `state.db` 的 `session_model_usage` 表连 `estimated_cost_usd` 和 `actual_cost_usd` 都分开存。但读 SQLite 要给项目加 `rusqlite` 依赖（bundled 要从源码编 SQLite，二进制约 +1MB）。这是个**明确的取舍，不是遗漏**，卡片上直接写明了原因。

## 一个会话 = 一只宠物，一个项目 = 一个工作空间

`session_id` 唯一确定一只宠物；`cwd` 的末段作为工作空间名把宠物归组。宠物顺序按 `first_seen` 排 —— HashMap 的迭代顺序是随机的，不排序图标每次刷新都会乱跳。

`SessionEnd` 会把宠物摘掉，否则关掉的终端会一直挂着一只。会话的 `cwd` 变了会重新归组。

**展开态**：每个项目一行，行内是该项目下的宠物（带序号，可点选）；下方是选中宠物的详情。超过 5 行转滚动，选中的宠物会被滚进视野。

**折叠态**：一条紧凑胶囊，只有宠物点阵。工作空间 ≤ 3 个时显示项目名，多了就只留点 —— 7 个项目名塞进折叠条只会全被截成 `my-vid...` 这种没信息量的碎片，名字在展开态和 tooltip 里都拿得到。

## 悬停窥视 / 拖到边缘入坞

默认摆在**屏幕顶部居中** —— 那一带通常空着，而右下角挤着托盘、通知和一堆角标。只影响全新安装：已有的 `prefs.json` 里写着明确的值，换默认值不该把老用户挪走。

**鼠标移上去展开，移开收起。** 离开有 180ms 缓冲：展开/收起会改窗口尺寸，而尺寸变化本身可能让 WebView 抛出一对假的 leave/enter，不缓冲的话挂件会在边界上抖。进入不缓冲 —— 「碰一下就展开」的即时感正是这个交互的价值。

展开有**三个并列的理由**，任一成立就展开，都不成立才收起：

| 理由 | 说明 |
| --- | --- |
| 有会话在等你 | 优先级最高。**鼠标移开也不会收起** —— 那会把挂件存在的意义藏起来 |
| 用户点了一下 | 点卡片 = 钉住，再点一下取消 |
| 鼠标停在上面 | 悬停窥视 |

这条「或」的关系是刻意的：光靠原来 `pinned` 的三态表达不了「悬停」这个临时状态 —— 悬停既不该覆盖用户的钉住，也不该在有人等你时把它藏回去。

**拖到离屏幕边 60px 以内松手，就贴上去并收起。** 四条边分别判断，所以拖到角落会两个方向都贴。留 2px 缝而不是贴死：窗口是透明圆角的，贴死之后鼠标很难从屏幕最边缘滑进去把它唤出来。底边额外避让任务栏。入坞之后悬停照样能把它唤出来，这就是「停在边上别挡路，要看再碰一下」。

### 入坞会把摆放模式改成「跟随上次拖动」，而且是故意让你看见的

`bottom-right` / `top-center` 是**吸附**模式，位置由屏幕算出来、拖动不留痕。所以在那两个模式下拖到边上再松手，下一次 `resize_pet` 会立刻把挂件弹回去 —— 用户看到的是「拖了但没用」。

之前刻意不做「拖一下自动切成 free」，理由是隐式状态变化会让人意外。入坞是用户**主动拖到边上**的结果，意图明确，那条理由不适用。而且模式真的写进了 `prefs.json`，设置窗口的下拉框会跟着变成「跟随上次拖动」——所以这是一次**看得见**的模式切换，不是背着人改状态。

### 拖动结束怎么判断

Tauri 的 `data-tauri-drag-region` **不产生 drag-end 事件**，只有连续的 `WindowEvent::Moved`。所以记下最后一次 `Moved` 的时刻，由一个 100ms 周期的线程发现它超过 250ms 没再动就当拖完了。

不用 `GetAsyncKeyState(VK_LBUTTON)` 查鼠标键：那要么轮询得很密，要么在用键盘或触摸板惯性移动窗口时判断错。

### 程序自己挪的窗口必须和用户拖的分开，否则一悬停就自我入坞

这是实现时最容易漏的一条。`top-center` 模式下窗口本来就停在离顶边 12px（`EDGE_MARGIN / 2`）的位置，**远在 60px 入坞阈值之内**；而 `resize_pet` 每次展开/收起都要 `set_position`，那会发出 `Moved`。不区分的话，挂件一被悬停就「入坞」，顺手把用户的摆放模式改成 `free`。

所有程序化的 `set_position` 都走 `set_position_quietly`，它会把「接下来 400ms 内的 `Moved` 当成我们自己挪的」记下来。用时间窗而不是一个布尔量：`Moved` 是异步送达的，`set_position` 返回时事件还没到，布尔量一置一清必然漏。时间窗对**真实**拖动是安全的 —— 拖动是一串持续的 `Moved`，窗口过期后面的照样会被记下。

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

**托盘菜单是主控制入口** —— 挂件窗口没有标题栏也不在任务栏。菜单里有当前版本、开机自启、提示音、设置…、退出。

## 新版本提醒

启动时查一次 GitHub Releases，有新版本就在挂件的概览行末尾追加 `· 有新版本 v0.3.0`，设置窗口的「关于」里也能手动查、并给出可复制的升级命令。

挂在概览行末尾而不是做徽标或弹窗：挂件就这么大，而「有新版本」的紧急程度**远低于**「有会话在等你」，不该抢视觉。

启动自动检查可以关（它是一次对外网络请求）；关掉后「立即检查」仍然可用。

### 为什么不用 `tauri-plugin-updater`

最初的理由是**产物形式不兼容**：读它的 Windows 安装逻辑（`updater.rs:883-913`），它接受 zip，但解压后只认里面的 `.exe`（当成 NSIS 安装包）或 `.msi`，而我们当时只发装着应用本体的 zip，被那样执行不会安装任何东西。

**这个理由现在已经不成立了** —— 我们同时发 NSIS 安装包，插件的前置条件满足了。但仍然不用它，剩下的理由是：

- 多一把 minisign 私钥，而**私钥一旦丢失，所有已安装的版本永久失去更新能力**（它们只信任内嵌的那个公钥）
- 发布流程要多产出 `latest.json` 和 `.nsis.zip`，`release.ps1` 跟着变复杂
- 换来的只是「少手动确认一下」

对个人项目不值。所以维持「提醒 + 一键复制升级命令」，升级就是再跑一次新版安装包。这是一个明确的选择而不是遗漏，以后要切过去，代价和现在一样，不会因为这个选择变高。

顺带一个好处：GitHub API 对 releases 返回 `Access-Control-Allow-Origin: *`（实测过），所以版本检查完全在前端 `fetch`，**Rust 侧没有引入任何 HTTP 客户端依赖**。

## 界面语言

简体中文 / English，设置窗口里选，默认跟随系统。

**语言由 Rust 侧解析**（`"auto"` 走 `GetUserDefaultUILanguage`），前端不自己猜 —— 两边各猜一次必然会有不一致的时候。

改语言后挂件就地重渲染；**托盘菜单要重启挂件才跟着变**（Tauri 的菜单项文本不能原地改，为一个菜单重建整个托盘不值得）。

Rust 侧生成的固定文案（`思考中`、`恢复的会话`）也按语言产出。这里刻意**没有**在协议里加 `detail_key` 字段让前端翻译——那要改缓存格式和六处构造点，而 Rust 本来就读得到偏好。代价是切换语言后，缓存里已有的旧文案要等下一个事件才刷新，这个瑕疵会自愈。

缺失的翻译 key 会直接显示 key 本身，这样漏翻在界面上一眼可见，而不是变成一片空白。

### 两个前端的坑（都踩过，别再踩）

**`i18n.js` 必须包在 IIFE 里。** 经典 script 共用一个全局作用域，它顶层的 `function t()` 会创建全局 `t`，而 `app.js` / `settings.js` 里都有 `const t = ...`。撞上就是**解析期** SyntaxError —— 整个文件一行都不执行，连 `window.onerror` 都注册不上，表现为「界面空白且没有任何报错」。`node --check` 是单文件检查，发现不了这种跨文件冲突；`ui/` 下的回归测试专门覆盖了它。

**给设了 `display` 的元素用 `hidden` 属性无效。** UA 样式表的 `[hidden] { display: none }` 优先级低于任何作者样式的 `display`，所以 `.actions { display: flex }` 会让 `hidden` 形同虚设。要额外写 `.actions[hidden] { display: none }`。

## 设置窗口

托盘 →「设置…」，或启动时带 `--settings`（可以拿它做个开始菜单快捷方式）。

五组：**通用**（开机自启、会话发现时间窗、双击用哪个编辑器、挂件摆在哪、界面语言）、**提示音**（开关、声音选择、试听）、**快捷键**（展开/收起、跳到下一个在等你的会话）、**Hooks**（安装状态 + 一键装/卸、权限拦截）、**关于**（版本、端口、配置目录、Claude 配置路径、检查更新）。

摆放模式的下拉选项由 Rust 侧的 `persist::POSITION_MODES` 生成，前端不另维护一份枚举 —— 两边各写一份必然会漂。

**没有「保存」按钮** —— 每一项改完立刻落盘并生效。这些设置都是彼此独立的开关，攒着一起提交只会让人怀疑到底存没存。

对时间窗来说，「立即生效」意味着**用新窗口重扫一遍**：光改数字对已经建好的会话表没有任何影响。

窗口是**按需创建、关闭即销毁**的。常驻一个隐藏的 WebView2 会白占几十 MB，而挂件的卖点之一就是常驻内存小（约 35MB）。

「开机自启」和「提示音」这两个布尔开关同时出现在托盘和设置窗口里 —— 托盘适合高频快切，设置窗口需要完整可发现。既然出现在两处，设置窗口改完会**回写托盘的勾选状态**，否则托盘会显示过期的值。

设置窗口有独立的 `settings.css`，**刻意不复用** `style.css`：后者是给透明无边框挂件写的，`body` 是 flex 容器、卡片靠 `flex-shrink:0` 顶出真实尺寸供窗口测量，那套约束搬到普通窗口上只会互相坑。

## 双击宠物跳回编辑器

看到「core 在等你」之后，下一个动作必然是切过去。双击宠物就用对应项目的 cwd 打开编辑器。

单击仍是选中 —— 浏览器会先派发 `click` 再派发 `dblclick`，所以双击的效果是「先选中它，再跳过去」，正是想要的。

探测顺序：Cursor → VS Code → Windsurf → JetBrains 各款。设置窗口里可以指定具体某个；**指定了就不回落** —— 你明确选了 VS Code，静默换成 Cursor 是在骗人。下拉框只列本机实际装了的，列出没装的等于埋一个「选了却不工作」的坑。

`code` / `cursor` 在 Windows 上是 `.cmd` shim，而 Rust 的 `Command` 直接调 `CreateProcessW`，执行 `.cmd` 会失败。所以自己实现了 `which`（走 `PATH` + `PATHEXT`）拿到完整路径，再按扩展名决定要不要经 `cmd` 启动。

启动时切断了子进程的 stdio 继承。不切的话编辑器会一直持有继承来的管道句柄，任何等待挂件输出的调用方都会挂到编辑器关闭为止 —— 实测挂死过。

诊断用：

```bash
claude-pet.exe --open D:\some\project
```

会打印探测到的编辑器列表和实际结果，把「编辑器探测/启动」这一环单独拎出来验，不用猜是 UI 没响应还是 spawn 失败。

**没有「跳回会话所在的终端 tab」这个功能，而且不会有。** 已调研并定性为不可行，结论见 [ADR 0001](docs/adr/0001-no-tab-precise-jump-back-on-windows.md)。

一句话版：`wt` 有 `focus-tab --target <n>`，但**没有任何查询类子命令**，操作系统侧也无法把 ConPTY 映射到 tab 索引 —— 能命令、不能发现那个 `n`，等于不能用。而且实测本机所有 Claude Code 会话根本不在终端里，真实宿主（Claude 桌面应用）对全部会话只暴露一个窗口和一个不含项目名的标题。

## 在挂件上批准权限（默认关闭）

开启后，Claude Code 请求工具权限时挂件上直接出现「允许 / 拒绝」，不用切回终端。

**默认不开**，需要显式装一条额外的 hook：

```bash
claude-pet.exe --install-permission-hook          # 默认只拦 Bash
claude-pet.exe --install-permission-hook 'Bash|Write'
claude-pet.exe --uninstall-permission-hook
```

设置窗口的 Hooks 分组里也有开关。

### 为什么用独立的 URL 路径

拦截走 `http://127.0.0.1:47800/permission`，普通事件走 `/`。这样「哪些工具要挂住等人点」**完全由那条 hook 的 matcher 决定**，挂件里不用再存一份 pref —— 两处各存一份必然会不一致。服务端只看 URL 路径就知道走阻塞还是非阻塞。

### 三个关键设计

**只有 `/permission` 走独立线程。** 普通事件仍在主循环里顺序处理，这样同一会话的事件不会被线程调度打乱（`Stop` 抢在它前面的 `PreToolUse` 之前）。实测：一条权限请求挂住期间，普通事件的响应仍是 4–32ms。

**超时 fail-open。** 内部最多等 30 秒，超时回空 200 = 不给决定，交还 Claude Code 自己的权限流程。所以挂件崩了或人不在时**绝不会把 Claude Code 卡死**，代价是每次多等 30 秒 —— 这也是不敢把它设更长的原因。hook 侧的 `timeout` 设成 40 秒，必须长于内部等待，否则 Claude Code 先放弃。

**这条 hook 不能带 `async`。** `async: true` 是发射后不管，Claude Code 不会等我们的决定。装的时候刻意不写这个字段。

### 注意

`permissions.defaultMode` 为 `bypassPermissions` 时 Claude Code 本来就不问权限，这个功能不会触发。要用得先改回 `default`。

matcher 填 `*` 会让**每一次**工具调用都等你点，通常不是你想要的，所以默认只拦 `Bash`。

## 全局快捷键

| 默认 | 作用 |
| --- | --- |
| `Ctrl+Alt+P` | 展开 / 收起挂件 |
| `Ctrl+Alt+N` | 跳到下一个**在等你**的会话（选中并展开） |

在等待中的会话里循环，而不是在全部会话里 —— 快捷键的用途就是「谁在等我」，把干活中和空闲的也串进去只会让人多按好几次。

设置窗口可改，**留空表示不注册**（空串是明确的「我不要这个键」，不是错误）。

被别的程序占用时给出可见错误（实测提示 `HotKey already registered`），而且**不影响另一个快捷键** —— 一个冲突不该连坐。非法写法同理，会带上解析器的原文错误。

Rust 侧只负责把动作名 emit 给前端，折叠和选中状态都住在前端的状态机里 —— 在两边各存一份必然会不一致。

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

claude-pet.exe --settings             # 启动并直接打开设置窗口（这个不退出）
```

上面除 `--settings` 之外都是一次性命令，执行完就退出。`--settings` 是入口而不是命令，挂件照常起。

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

一条龙：改 `Cargo.toml` 版本号 → 把 CHANGELOG 的 `[Unreleased]` 提升成带日期的版本段 → `cargo tauri build --bundles nsis` → 打包 zip + 重命名安装包 → commit + tag + push → `gh release create`（release notes 从 CHANGELOG 那一段抠出来）。

每次发两个产物：`claude-pet-x.y.z-x64-setup.exe`（安装包）和 `claude-pet-x.y.z-windows-x64.zip`（绿色版，`install.ps1` 认的是这个）。安装包要重命名是因为 tauri 用 `productName` 当文件名，`Claude Pet_...` 里的空格在下载 URL 里会变成 `%20`。

要求工作区干净，并且 `cargo` / `git` / `gh` / `cargo-tauri` 都在 PATH 上。新改动写在 CHANGELOG 的 `[Unreleased]` 下面。

### NSIS 工具链下载会失败，且报错完全看不出是网络

第一次打包时 `tauri-bundler` 会去 GitHub 下 `nsis-3.11.zip` 和 `nsis_tauri_utils.dll`。它的下载器**没有重试**，网络抖一下就抛 `io: unexpected end of file` —— 看起来像仓库坏了。实测同一个文件用 `curl --retry` 一次就成。

`release.ps1` 在这一步失败时会直接把手动布置工具链的命令打出来。要点：

- 缓存目录是 `%LOCALAPPDATA%\tauri\NSIS\`（`dirs::cache_dir()/tauri/NSIS`）
- zip 解出来的顶层目录 `nsis-3.11` 要**重命名**成 `NSIS`，bundler 就是这么做的
- 还要单独放 `NSIS\Plugins\x86-unicode\additional\nsis_tauri_utils.dll`
- bundler 会校验一份 13 个文件的清单，缺任何一个就把整个目录删掉重下 —— 所以少放一个文件比不放更糟

校验和（对得上就说明下载完整，来自 `tauri-bundler` 源码里的常量）：

| 文件 | SHA1 |
| --- | --- |
| `nsis-3.11.zip` | `EF7FF767E5CBD9EDD22ADD3A32C9B8F4500BB10D` |
| `nsis_tauri_utils.dll` (v0.5.3) | `75197FEE3C6A814FE035788D1C34EAD39349B860` |

### 语言名是 `SimpChinese`，不是 `SimplifiedChinese`

`nsis.languages` 里的名字必须逐字对上 NSIS 自己的 `Contrib\Language files\*.nlf`。那里的文件叫 `SimpChinese.nlf` / `TradChinese.nlf`。写成 `SimplifiedChinese` 的话，tauri 只会警告一句「not translated」，然后 `makensis` 才因为找不到 `.nlf` 而中止 —— 警告和真正的错误分在两处，容易看漏。

## 前端没有构建步骤

`frontendDist` 直接指向 `../ui`，纯静态 HTML/CSS/JS，不用 npm、不用 vite。

**但改了 `ui/` 下的文件必须重新 `cargo build`** —— `tauri-build` 会把前端资源在编译期嵌进二进制，不是运行时从磁盘读。（`tauri-build` 对 `ui/` 有 rerun-if-changed，所以只改前端也会触发一次重编译，几秒钟。）

代价是拿不到 `@tauri-apps/api` 的 npm 包，所以靠 `withGlobalTauri: true` 注入的 `window.__TAURI__`。

## 几个刻意的设计决定

**窗口尺寸贴合卡片本体。** Tauri 的 `setIgnoreCursorEvents` 是整窗开关、没有 per-region，想做「宠物身上能点、周围透明区域穿透」就得自己写命中测试。把窗口做到刚好包住卡片直接绕开了这个问题 —— 没有大片透明区域，就不需要穿透。

**锚定方向必须随摆放模式变，不能写死。** 前端量完卡片调 `resize_pet`，Rust 侧要决定「窗口朝哪边长」：

| 摆放模式 | 保持不动的边 | 写死右下角会怎样 |
| --- | --- | --- |
| `bottom-right`（默认） | 右 + 底，朝左上长 | — |
| `top-center` | 上 + 水平中心，朝下长 | 展开时朝上长出屏幕顶部 |
| `free` | 离窗口最近的那两条边 | 拖到左上角后朝左上长就出界 |

只锚底边的话，折叠态点阵横向变长时窗口会冲出屏幕右缘；只锚左上角的话，展开时会长出屏幕底部。算完之后还会**无条件**夹一次显示器边界 —— 光「锚对边」不够，`free` 模式下窗口可能被拖到任意位置。

前两个是**吸附**模式：位置由屏幕算出来，拖动不留痕。只有 `free` 记住拖动。刻意不做「拖一下就自动切成 free」，那是会让人意外的隐式状态变化。

吸附用的是**窗口当前所在**的那块屏（`current_monitor`）而不是主显示器，否则多显示器下顶部居中会跑到另一块屏上去。

`top-center` 是 Open Island 那个刘海 overlay 在 Windows 上的等价物 —— Windows 没有刘海。

**卡片必须 `flex-shrink: 0`，别删。** 窗口尺寸是量卡片得出的，而卡片作为 body 的 flex 子项默认会被窗口宽度挤压 —— 于是形成循环依赖：窗口小 → 卡片被压 → 量出来还是小 → 窗口永远长不大，点阵被压扁裁断。禁止收缩后，卡片先按内容溢出视口，量到真实尺寸，下一帧窗口就跟上了。

同理，`.strip` 上**不能**设 `min-width: 0` 或 `overflow: hidden`：两者都会让它不参与卡片的 max-content 计算，结果一样是内容被压扁 —— 而 `overflow: hidden` 更坏，它把这个 bug 藏起来，看着像「刚好放下」。

**每 5 秒重新 `set_always_on_top(true)`。** Windows 上独占全屏程序和 UAC 安全桌面会抢走 topmost。这个调用是幂等的，不抢焦点。

**整张卡片是 `data-tauri-drag-region`。** 拖动窗口不用写一行 Rust。

**位置持久化。** 拖动时只更新内存，由那个 5 秒线程在变化时落盘 —— `Moved` 事件在一次拖动里会触发几百次，每次写文件太糙。代价是最多丢 5 秒内的移动。

恢复前会校验坐标**仍落在某个当前可用的显示器内**（左上和右下两个角都查），否则回落右下角。笔记本插拔扩展屏后旧坐标会把挂件扔到看不见的地方，而这个窗口没有标题栏也不在任务栏，用户根本找不回来，只会以为程序坏了。

只有 `free` 模式会**读**这个锚点，但吸附时也一直在**写**（连程序化移动都记）。这样从吸附切到 `free` 时挂件停在原地，而不是跳回几天前的某个旧坐标。

## 已知限制 / 下一步

- **设置窗口关掉后只能从托盘再开** —— 窗口是关闭即销毁的，`--settings` 只在启动时生效。挂件卡片上加个齿轮按钮能解决，但会占掉本就不多的地方。
- **任务栏躲避是估的** —— Tauri 的 monitor API 只给整屏尺寸拿不到工作区（`SPI_GETWORKAREA`），右下角吸附硬减了 56px。任务栏移到侧边或顶部、或多显示器不同缩放下可能偏。顶部居中不受影响 —— 任务栏默认在底部。
- **多显示器下的吸附只在单屏机器上验证过** —— 代码取的是 `current_monitor()` 并把显示器原点 `(mx, my)` 算进了坐标，但单屏上 `mx = my = 0`，这条路径没有能区分对错的测试。
- **圆角外的小块透明区域仍会吃点击** —— 影响很小，真要修就得上命中测试。
- **`permission_prompt` 在 `bypassPermissions` 模式下几乎不触发** —— 这个模式下 Claude Code 不问权限。实际会响的是 `agent_needs_input` / `idle_prompt` / `agent_completed`。
- 端口硬编码 47800，改的话记得同步 `~/.claude/settings.json` 里的 hook url。

## 许可

[Apache License 2.0](LICENSE)。

功能设计上参考了 [Open Island](https://github.com/Octane0411/open-vibe-island)（macOS，GPL v3），但**没有移植其任何代码** —— 差异见上文「新版本提醒」和「双击宠物跳回编辑器」两节里关于 Windows 平台限制的说明。所以本项目不受 GPL 传染，可以用 Apache-2.0。

`LICENSE` 是 apache.org 的原文，附录里的 `[yyyy] [name of copyright owner]` 模板按惯例保持原样（Kubernetes 等项目也是这么做的）。想显式声明版权人的话，标准做法是加一个 `NOTICE` 文件或在源文件头部加声明。
