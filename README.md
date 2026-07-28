# Claude Pet

常驻置顶的桌面挂件，实时显示 Claude Code 在干什么、有没有在等你。

解决的问题：Claude Code 的通知弹窗会自动消失，一走神就错过，回头发现它已经停在那儿等了十分钟。

## 架构

```
Claude Code hooks (type: "http")
        │  POST 事件 JSON
        ▼
127.0.0.1:47800  ← tiny_http，跑在 Tauri 的 Rust 侧后台线程
        │  归类成状态 + 按 session_id 聚合
        ▼
  app.emit("pet://state")
        │
        ▼
   WebView（CSS/SVG 渲染）
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

多会话按 `session_id` 分别记账。上镜规则：**等你操作 > 干活中 > 完成 > 空闲**，同优先级取最近更新的。右上角角标在会话数 > 1 时出现，有人在等你时显示 `等待数/总数` 并标红。

`SessionEnd` 会把会话从表里摘掉，否则关掉的终端会一直挂在计数里。

## 跑起来

```bash
cd src-tauri && cargo run
```

打包（产出 NSIS 安装包）：

```bash
cd src-tauri && cargo tauri build
```

`cargo tauri build` 需要先装 CLI：`cargo install tauri-cli`。只是自己用的话 `cargo run --release` 就够，产物在 `src-tauri/target/release/claude-pet.exe`。

**退出走托盘右键。** 窗口没有标题栏也不在任务栏，托盘是唯一入口。

## 前端没有构建步骤

`frontendDist` 直接指向 `../ui`，纯静态 HTML/CSS/JS，不用 npm、不用 vite。改 `ui/` 下的文件保存后刷新窗口即可（`cargo run` 时按 F5，或用 devtools）。

代价是拿不到 `@tauri-apps/api` 的 npm 包，所以靠 `withGlobalTauri: true` 注入的 `window.__TAURI__`。

## 几个刻意的设计决定

**窗口尺寸贴合宠物本体（248×96）。** Tauri 的 `setIgnoreCursorEvents` 是整窗开关、没有 per-region，想做「宠物身上能点、周围透明区域穿透」就得自己写命中测试。把窗口做小直接绕开了这个问题 —— 没有大片透明区域，就不需要穿透。

**每 5 秒重新 `set_always_on_top(true)`。** Windows 上独占全屏程序和 UAC 安全桌面会抢走 topmost。这个调用是幂等的，不抢焦点。

**整张卡片是 `data-tauri-drag-region`。** 拖动窗口不用写一行 Rust。

## 已知限制 / 下一步

- **位置不持久化** —— 拖到别处后重启会回到右下角。要修就监听 `WindowEvent::Moved` 存到 app data。
- **任务栏躲避是估的** —— Tauri 的 monitor API 只给整屏尺寸拿不到工作区，右下角定位硬减了 56px。多显示器不同缩放下可能偏。
- **圆角外的小块透明区域仍会吃点击** —— 影响很小，真要修就得上命中测试。
- **`permission_prompt` 在 `bypassPermissions` 模式下几乎不触发** —— 这个模式下 Claude Code 不问权限。实际会响的是 `agent_needs_input` / `idle_prompt` / `agent_completed`。
- 端口硬编码 47800，改的话记得同步 `~/.claude/settings.json` 里的 hook url。
