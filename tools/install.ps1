# Claude Pet 一键安装。
#
#   irm https://raw.githubusercontent.com/zq940222/claude-pet/main/tools/install.ps1 | iex
#
# 想带参数（irm | iex 传不了参数，得用 scriptblock 形式）：
#
#   & ([scriptblock]::Create((irm https://raw.githubusercontent.com/zq940222/claude-pet/main/tools/install.ps1))) -Autostart
#
# 参数：
#   -Autostart     装完就开启开机自启
#   -Version x.y.z 装指定版本，默认最新
#   -NoLaunch      装完不启动
#   -Uninstall     卸载（停进程、关自启、删文件和快捷方式）

[CmdletBinding()]
param(
    [switch]$Autostart,
    [string]$Version,
    [switch]$NoLaunch,
    [switch]$Uninstall
)

$ErrorActionPreference = 'Stop'

$Repo      = 'zq940222/claude-pet'
$InstallDir = Join-Path $env:LOCALAPPDATA 'ClaudePet'
$ExePath    = Join-Path $InstallDir 'claude-pet.exe'
$StartMenu  = Join-Path $env:APPDATA 'Microsoft\Windows\Start Menu\Programs\Claude Pet.lnk'

function Step($m) { Write-Host "==> $m" -ForegroundColor Cyan }
function Ok($m)   { Write-Host "OK  $m" -ForegroundColor Green }
function Warn($m) { Write-Host "!!  $m" -ForegroundColor Yellow }
function Die($m)  { Write-Host "x   $m" -ForegroundColor Red; exit 1 }

function Stop-Pet {
    $p = Get-Process claude-pet -ErrorAction SilentlyContinue
    if ($p) {
        Step 'stopping running instance'
        $p | Stop-Process -Force
        Start-Sleep -Milliseconds 800
    }
}

# ── 卸载 ─────────────────────────────────────────────────────

if ($Uninstall) {
    if (Test-Path $ExePath) {
        Step 'disabling autostart'
        # stderr 丢掉：WebView 退出时会打一行无害的 unregister class 报错
        & $ExePath --disable-autostart 2>$null | Out-Null
    }
    Stop-Pet
    if (Test-Path $StartMenu)  { Remove-Item $StartMenu -Force;            Ok 'removed Start Menu shortcut' }
    if (Test-Path $InstallDir) { Remove-Item $InstallDir -Recurse -Force;  Ok "removed $InstallDir" }
    Warn "config kept at $env:APPDATA\com.opsmateai.claude-pet (delete manually if you want)"
    Warn "hooks kept in ~/.claude/settings.json (remove the 127.0.0.1:47800 entries yourself)"
    Ok 'uninstalled'
    return
}

# ── 找 release ───────────────────────────────────────────────

Step 'looking up release'

# GitHub API 要求带 UA，否则 403
$headers = @{ 'User-Agent' = 'claude-pet-installer'; 'Accept' = 'application/vnd.github+json' }
$apiUrl = if ($Version) {
    "https://api.github.com/repos/$Repo/releases/tags/v$Version"
} else {
    "https://api.github.com/repos/$Repo/releases/latest"
}

try {
    $release = Invoke-RestMethod -Uri $apiUrl -Headers $headers -UseBasicParsing
} catch {
    Die "could not fetch release info from $apiUrl`n    $($_.Exception.Message)"
}

$tag = $release.tag_name
$asset = $release.assets | Where-Object { $_.name -like '*windows-x64.zip' } | Select-Object -First 1
if (-not $asset) { Die "release $tag has no *windows-x64.zip asset" }

Step "installing $tag  ($($asset.name), $([math]::Round($asset.size/1MB,2)) MB)"

# ── 下载并解压 ───────────────────────────────────────────────

$tmpZip = Join-Path $env:TEMP $asset.name
$ProgressPreference = 'SilentlyContinue'
try {
    Invoke-WebRequest -Uri $asset.browser_download_url -OutFile $tmpZip -UseBasicParsing
} catch {
    Die "download failed: $($_.Exception.Message)"
}

Stop-Pet
New-Item -ItemType Directory -Force $InstallDir | Out-Null
Expand-Archive -Path $tmpZip -DestinationPath $InstallDir -Force
Remove-Item $tmpZip -Force -ErrorAction SilentlyContinue

if (-not (Test-Path $ExePath)) { Die "expected $ExePath after extraction" }
Ok "installed to $InstallDir"

# ── 开始菜单快捷方式 ─────────────────────────────────────────

$shell = New-Object -ComObject WScript.Shell
$lnk = $shell.CreateShortcut($StartMenu)
$lnk.TargetPath = $ExePath
$lnk.WorkingDirectory = $InstallDir
$lnk.Description = 'Claude Code status widget'
$lnk.Save()
Ok 'created Start Menu shortcut'

# ── 自启 ─────────────────────────────────────────────────────

if ($Autostart) {
    Step 'enabling autostart'
    # 必须用「装好的」exe 调用 —— 插件写进注册表的是当前 exe 的路径
    & $ExePath --enable-autostart 2>$null | Out-Null
    if ($LASTEXITCODE -eq 0) { Ok 'autostart enabled' } else { Warn 'autostart could not be enabled' }
} else {
    Warn 'autostart not enabled (pass -Autostart, or use the tray menu)'
}

# ── 启动 ─────────────────────────────────────────────────────

if (-not $NoLaunch) {
    Start-Process -FilePath $ExePath -WorkingDirectory $InstallDir
    Start-Sleep -Seconds 2
    if (Get-Process claude-pet -ErrorAction SilentlyContinue) {
        Ok 'running (look at the bottom-right of your screen)'
    } else {
        Warn 'launched but process not found -- check WebView2 Runtime is installed'
    }
}

# ── 提示接 hook ──────────────────────────────────────────────

Write-Host ''
Write-Host 'One more step: the widget only lights up once Claude Code posts events to it.' -ForegroundColor Yellow
Write-Host 'Add this to the "hooks" object in ~/.claude/settings.json:' -ForegroundColor Yellow
Write-Host ''
@'
  "UserPromptSubmit": [ { "hooks": [ { "type": "http", "url": "http://127.0.0.1:47800/", "async": true, "timeout": 5 } ] } ],
  "PreToolUse":       [ { "matcher": "*", "hooks": [ { "type": "http", "url": "http://127.0.0.1:47800/", "async": true, "timeout": 5 } ] } ],
  "Notification":     [ { "matcher": "permission_prompt|agent_needs_input|idle_prompt|agent_completed",
                          "hooks": [ { "type": "http", "url": "http://127.0.0.1:47800/", "async": true, "timeout": 5 } ] } ],
  "Stop":             [ { "hooks": [ { "type": "http", "url": "http://127.0.0.1:47800/", "async": true, "timeout": 5 } ] } ],
  "SessionStart":     [ { "hooks": [ { "type": "http", "url": "http://127.0.0.1:47800/", "async": true, "timeout": 5 } ] } ],
  "SessionEnd":       [ { "hooks": [ { "type": "http", "url": "http://127.0.0.1:47800/", "async": true, "timeout": 5 } ] } ]
'@
Write-Host ''
Write-Host 'Hooks load at session start, so open a NEW Claude Code session to see it work.' -ForegroundColor Yellow
Write-Host 'Uninstall:  & ([scriptblock]::Create((irm https://raw.githubusercontent.com/zq940222/claude-pet/main/tools/install.ps1))) -Uninstall'
