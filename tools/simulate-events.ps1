# 模拟 Claude Code 的 hook 事件，用来验证挂件的状态机。
# 用法：
#   .\tools\simulate-events.ps1            # 依次走一遍全部状态
#   .\tools\simulate-events.ps1 -Multi     # 再加一个会话，验证多会话角标
#
# 挂件必须已经在跑（cd src-tauri && cargo run）。

param(
    [int]$Port = 47800,
    [int]$DelayMs = 1800,
    [switch]$Multi
)

$ErrorActionPreference = 'Stop'
$url = "http://127.0.0.1:$Port/"

function Send-Event {
    param([hashtable]$Body, [string]$Label)

    $json = $Body | ConvertTo-Json -Compress -Depth 6
    try {
        $r = Invoke-WebRequest -Uri $url -Method Post -Body $json `
            -ContentType 'application/json' -UseBasicParsing -TimeoutSec 5
        "{0,-24} -> HTTP {1}" -f $Label, $r.StatusCode
    } catch {
        "{0,-24} -> FAILED: {1}" -f $Label, $_.Exception.Message
        "  (挂件没在跑？先 cd src-tauri; cargo run)"
        exit 1
    }
    Start-Sleep -Milliseconds $DelayMs
}

$sid = 'sim-session-1'
# 从脚本位置推导仓库根目录，别硬编码本机路径
$cwd = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
# 第二个会话用假路径，只为验证多会话角标
$cwd2 = 'C:\projects\other-project'

"POST -> $url`n"

Send-Event -Label 'SessionStart' -Body @{
    session_id = $sid; cwd = $cwd; hook_event_name = 'SessionStart'
}

Send-Event -Label 'UserPromptSubmit' -Body @{
    session_id = $sid; cwd = $cwd; hook_event_name = 'UserPromptSubmit'
    user_prompt = 'add a test'
}

Send-Event -Label 'PreToolUse (Bash)' -Body @{
    session_id = $sid; cwd = $cwd; hook_event_name = 'PreToolUse'
    tool_name = 'Bash'; tool_input = @{ command = 'npm test' }
}

Send-Event -Label 'PreToolUse (Edit)' -Body @{
    session_id = $sid; cwd = $cwd; hook_event_name = 'PreToolUse'
    tool_name = 'Edit'; tool_input = @{ file_path = 'src/main.rs' }
}

if ($Multi) {
    Send-Event -Label 'second session working' -Body @{
        session_id = 'sim-session-2'; cwd = $cwd2
        hook_event_name = 'PreToolUse'
        tool_name = 'Grep'; tool_input = @{ pattern = 'TODO' }
    }
}

Send-Event -Label 'Notification permission' -Body @{
    session_id = $sid; cwd = $cwd; hook_event_name = 'Notification'
    notification_type = 'permission_prompt'; message = 'Bash wants to run rm -rf'
}

Send-Event -Label 'Notification needs_input' -Body @{
    session_id = $sid; cwd = $cwd; hook_event_name = 'Notification'
    notification_type = 'agent_needs_input'; message = 'Which file did you mean?'
}

Send-Event -Label 'Notification completed' -Body @{
    session_id = $sid; cwd = $cwd; hook_event_name = 'Notification'
    notification_type = 'agent_completed'; message = 'All 42 tests passed'
}

Send-Event -Label 'Stop' -Body @{
    session_id = $sid; cwd = $cwd; hook_event_name = 'Stop'
    stop_reason = 'end_turn'; last_assistant_message = 'Done.'
}

Send-Event -Label 'SessionEnd' -Body @{
    session_id = $sid; cwd = $cwd; hook_event_name = 'SessionEnd'; reason = 'clear'
}

if ($Multi) {
    Send-Event -Label 'SessionEnd (second)' -Body @{
        session_id = 'sim-session-2'; cwd = $cwd2
        hook_event_name = 'SessionEnd'; reason = 'clear'
    }
}

"`n跑完了。挂件应该回到 idle / 没有活动会话。"
