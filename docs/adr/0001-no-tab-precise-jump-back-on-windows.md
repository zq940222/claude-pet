# ADR 0001：不实现「跳回会话所在的终端 tab」

- 状态：已接受
- 日期：2026-07-29
- 相关：[#12](https://github.com/zq940222/claude-pet/issues/12)、[#16](https://github.com/zq940222/claude-pet/issues/16)（Epic）、[#7](https://github.com/zq940222/claude-pet/issues/7)（已实现的 IDE 跳转）

## 背景

[Open Island](https://github.com/Octane0411/open-vibe-island)（macOS）能从挂件精确跳回会话所在的终端 session，支持 7 种终端。对齐它的功能时，这是唯一一条一开始就怀疑存在**硬上限**而非工作量问题的能力，所以先做调研再决定做不做。

Windows 侧的结构性疑虑：Windows Terminal 是单 HWND 承载多 Tab，Win32 层面没有「激活第 N 个 tab」的通用能力，也没有 TTY 概念可以把进程映射到某个 tab。

## 调查结果

### 1. `wt` 确实支持 focus-tab（此前的判断是错的）

`TerminalApp.dll`（Windows Terminal 1.24.11911.0）里存在 `focus-tab`、`move-focus`、`focus-pane`、`--target`、`new-tab`、`split-pane`。

此前一次测试得出「不支持」的结论是**无效的**：PATH 上的 `wt.exe` 是 **0 字节**的 WindowsApps 别名 stub，`--help` 经它重定向抓不到任何输出。那是「没测到」，不是「不支持」。

所以**动作侧是可用的**。

### 2. 但没有任何办法查出该切到第几个 tab —— 这是硬阻塞

`TerminalApp.dll` 里搜不到任何查询类子命令：`list-tabs`、`query`、`--list`、`get-tab`、`list-windows` 全部不存在。`wt` 只有动作动词，没有查询动词。

操作系统侧也接不上。假设会话跑在 Windows Terminal 里，链条是：

| 环节 | 可行性 |
| --- | --- |
| 会话 → 它的 shell 进程 | 需另建关联（hook 事件不带 pid） |
| shell → ConPTY（`OpenConsole.exe`，1 pane = 1 个） | ✅ 父子关系可查 |
| ConPTY → Windows Terminal 窗口 | ⚠️ 部分（同进程可承载多窗口） |
| **窗口 → tab 索引** | ❌ **无任何可观测途径** |

`focus-tab --target <n>` 要的正是那个查不到的 `n`。能命令、不能发现，等于不能用。

### 3. 对本项目实际的使用方式，这个问题是无意义的

实测本机所有正在运行的 Claude Code 会话：**没有一个跑在终端里**。所有 `claude` / `node` 进程的祖先链里都不含 `conhost.exe` / `OpenConsole.exe` / `WindowsTerminal.exe`。

真实宿主是 **Claude 桌面应用**：`C:\Program Files\WindowsApps\Claude_<ver>\app\Claude.exe`，父进程 `explorer.exe`，16 个进程（Electron 多进程）。

而它：

- 只暴露**一个**顶层窗口（class `Chrome_WidgetWin_1`，标题就是 `Claude`）
- 标题里**不含**任何项目名或会话标识（对照挂件已知的 6 个工作空间逐个匹配，全部无命中）
- 多个并发会话在这一个窗口内以应用内 tab 呈现，那是 DOM 而非 HWND —— **没有按会话的窗口句柄**

这和 Windows Terminal 的 tab 是同一类结构性问题，而且更糟：WT 至少有 `focus-tab --target`，桌面应用连 CLI 都没有。

顺带纠正一处对机器状态的误读：本机确实有 `OpenConsole.exe` 和 `powershell.exe` 进程，但它们的父进程是 `idea64.exe` —— 属于 IntelliJ 的内嵌终端，与 Claude Code 无关。

### 4. 一条未走到底的线索：`claude://` 深链

注册表里确实注册了两个协议：

```
claude      -> "…\app\Claude.exe" "%1"
claude-cli  -> "…\.local\bin\claude.exe" --handle-uri "%1"
```

`Claude.exe` 二进制里能搜到 `deeplink`，但搜不到 `--session` / `--project` / `--focus` / `--cwd`。

**刻意没有继续探测。** 要弄清 URI 语义就得往用户正在使用的应用里发各种 URI，那可能把他当前的界面导航走 —— 为了探索而造成可见的副作用，不划算。这条留在下面「什么情况下会重新考虑」里。

### 5. WezTerm 可行但本机未装

`wezterm cli list` 能给出 pane ↔ pid 映射，理论上可做到 pane 级精确跳转。本机没装 WezTerm，**未验证**。这是唯一一条在 Windows 上有望做到 tab/pane 精确的路径，代价是只对一种终端有效。

## 决定

**不实现「跳回会话所在的终端 / 宿主 tab」，也不为它开实施 issue。**

理由按权重排序：

1. **tab 精确跳转在 Windows Terminal 上不可实现**，不是难做。缺的是发现目标索引的能力，而那是 Windows Terminal 没有暴露的东西，不是我们能绕开的。
2. **对本项目的实际使用方式完全不适用** —— 会话根本不在终端里，而真实宿主对所有会话只暴露一个窗口和一个标题。
3. **更有用的跳转目标已经做了。** [#7](https://github.com/zq940222/claude-pet/issues/7) 的双击宠物打开对应项目的 IDE 已经覆盖了「我要去处理这个会话」的主要动作 —— 要看的通常是代码，而不是终端回显。
4. 剩下能廉价拿到的只有「把宿主窗口提到前台」（单 HWND，`SetForegroundWindow` 即可）。但所有会话都在那一个窗口里，所以它只能做到「到达应用」，做不到「到达那个会话」，价值有限，不值得为它增加一个交互入口。

Epic 里「终端 tab 级跳回」这一行据此定性为**不可行**，而非待办。

## 后果

- 挂件不提供跳回终端/宿主的入口。双击宠物的语义保持为「打开项目的 IDE」，不做二义。
- Epic（#16）中该行标注为已调研且不可行，附本 ADR 链接。
- README 里关于「终端 tab 级跳回不在此列」的说法从「另有 issue 调研」更新为「已调研，结论见 ADR 0001」。

## 什么情况下会重新考虑

任一条成立就值得重开：

- **Windows Terminal 增加查询能力**（`wt list-tabs` 之类，或任何能把 ConPTY 映射到 tab 索引的接口）。动作侧已经就绪，缺的只有这一环。
- **Claude 桌面应用的 `claude://` 深链支持定位到具体会话**。这是本 ADR 唯一刻意没查到底的线索；如果它支持，跳转会变得既精确又不需要任何 Win32 技巧。探测方法：向 `claude-cli://` 发构造的 URI 并观察行为 —— 需要在一个**不含真实工作**的环境里做，因为它会导航正在使用的应用。
- **主要在 WezTerm 里跑 Claude Code**。那条路径有 `wezterm cli list` 的 pane ↔ pid 映射，可以只为它做精确跳转。
