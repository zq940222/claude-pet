# Claude Pet installer.
#
#   irm https://raw.githubusercontent.com/zq940222/claude-pet/main/tools/install.ps1 | iex
#
# With arguments (irm | iex cannot pass parameters, so use the scriptblock form):
#
#   & ([scriptblock]::Create((irm https://raw.githubusercontent.com/zq940222/claude-pet/main/tools/install.ps1))) -Autostart
#
# Parameters:
#   -Autostart      enable start-with-Windows after installing
#   -WireHooks      write the Claude Code hook config for you (backs up first)
#   -Version x.y.z  install a specific version (default: latest)
#   -NoLaunch       do not start the widget after installing
#   -Uninstall      stop it, disable autostart, remove hooks, files and shortcut
#
# NOTE TO MAINTAINERS: this file must stay pure ASCII with NO byte-order mark.
# It is fetched over HTTP and handed to iex / [scriptblock]::Create, and a
# leading U+FEFF from a BOM counts as a statement, which makes param() no
# longer the first statement and breaks parsing with
# "Unexpected attribute 'CmdletBinding'". Keep comments English so no BOM is
# needed for Windows PowerShell 5.1 to decode the file correctly.

[CmdletBinding()]
param(
    [switch]$Autostart,
    [switch]$WireHooks,
    [string]$Version,
    [switch]$NoLaunch,
    [switch]$Uninstall
)

$ErrorActionPreference = 'Stop'

$Repo       = 'zq940222/claude-pet'
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

# Run claude-pet.exe with a flag and return its exit code.
#
# Do NOT write `& $exe --flag 2>$null` here. In Windows PowerShell 5.1,
# redirecting a native command's stderr wraps every stderr line in a
# NativeCommandError ErrorRecord and sets $? to false even when the exe exits
# 0 -- and with $ErrorActionPreference = 'Stop' that aborts this script. The
# exe does print a harmless "Failed to unregister class Chrome_WidgetWin_0" on
# these paths, so the noise does need to go somewhere; Start-Process redirects
# it out of band to a file without any ErrorRecord wrapping.
function Invoke-PetExe([string]$Flag) {
    $errFile = Join-Path $env:TEMP 'claude-pet-install.err'
    $proc = Start-Process -FilePath $ExePath -ArgumentList $Flag `
        -RedirectStandardError $errFile -WindowStyle Hidden -PassThru -Wait
    Remove-Item $errFile -Force -ErrorAction SilentlyContinue
    return $proc.ExitCode
}

# -- Uninstall ------------------------------------------------

if ($Uninstall) {
    if (Test-Path $ExePath) {
        Step 'disabling autostart'
        $null = Invoke-PetExe '--disable-autostart'
        # Only our own http hooks are touched; anything else in settings.json,
        # including other tools' hooks, is left alone. settings.json is backed
        # up first. Leaving them would be harmless (an unreachable hook is a
        # non-blocking error) but it is untidy.
        Step 'removing hooks from settings.json'
        if ((Invoke-PetExe '--uninstall-hooks') -eq 0) {
            Ok 'hooks removed'
        } else {
            Warn 'could not remove hooks -- edit ~/.claude/settings.json yourself'
        }
    }
    Stop-Pet
    if (Test-Path $StartMenu)  { Remove-Item $StartMenu -Force;           Ok 'removed Start Menu shortcut' }
    if (Test-Path $InstallDir) { Remove-Item $InstallDir -Recurse -Force; Ok "removed $InstallDir" }
    Warn "config kept at $env:APPDATA\com.opsmateai.claude-pet (delete it yourself if you want)"
    Ok 'uninstalled'
    return
}

# -- Locate the release ---------------------------------------

Step 'looking up release'

# The GitHub API rejects requests without a User-Agent.
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

# -- Download and extract -------------------------------------

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

# -- Start Menu shortcut --------------------------------------

$shell = New-Object -ComObject WScript.Shell
$lnk = $shell.CreateShortcut($StartMenu)
$lnk.TargetPath = $ExePath
$lnk.WorkingDirectory = $InstallDir
$lnk.Description = 'Claude Code status widget'
$lnk.Save()
Ok 'created Start Menu shortcut'

# -- Autostart ------------------------------------------------

if ($Autostart) {
    Step 'enabling autostart'
    # Must be invoked from the INSTALLED exe: the plugin records the path of
    # whichever binary makes the call.
    if ((Invoke-PetExe '--enable-autostart') -eq 0) {
        Ok 'autostart enabled'
    } else {
        Warn 'autostart could not be enabled'
    }
} else {
    Warn 'autostart not enabled (pass -Autostart, or use the tray menu)'
}

# -- Hooks ----------------------------------------------------

$hooksWired = $false
if ($WireHooks) {
    Step 'writing hook config into settings.json'
    # Merges into the existing file and backs it up first; only entries pointing
    # at this widget's port are added, and re-running is a no-op.
    if ((Invoke-PetExe '--install-hooks') -eq 0) {
        Ok 'hooks installed'
        $hooksWired = $true
    } else {
        Warn 'could not write hooks -- falling back to the manual snippet below'
    }
}

# -- Launch ---------------------------------------------------

if (-not $NoLaunch) {
    Start-Process -FilePath $ExePath -WorkingDirectory $InstallDir
    Start-Sleep -Seconds 2
    if (Get-Process claude-pet -ErrorAction SilentlyContinue) {
        Ok 'running (look at the bottom-right of your screen)'
    } else {
        Warn 'launched but process not found -- check the WebView2 Runtime is installed'
    }
}

# -- Tell them about the hooks --------------------------------

Write-Host ''
if ($hooksWired) {
    Write-Host 'Hooks are wired up. Open a NEW Claude Code session to see the widget light up' -ForegroundColor Yellow
    Write-Host '(hooks are read when a session starts).' -ForegroundColor Yellow
    Write-Host ''
    Write-Host 'Uninstall:' -ForegroundColor Yellow
    Write-Host '  & ([scriptblock]::Create((irm https://raw.githubusercontent.com/zq940222/claude-pet/main/tools/install.ps1))) -Uninstall'
    return
}

Write-Host 'One more step: the widget only lights up once Claude Code posts events to it.' -ForegroundColor Yellow
Write-Host 'Either re-run this installer with -WireHooks, or run:' -ForegroundColor Yellow
Write-Host ''
Write-Host "  & '$ExePath' --install-hooks"
Write-Host ''
Write-Host 'Both merge into settings.json and back it up first. To do it by hand instead,' -ForegroundColor Yellow
Write-Host 'add this to the "hooks" object in ~/.claude/settings.json:' -ForegroundColor Yellow
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
Write-Host 'Uninstall:' -ForegroundColor Yellow
Write-Host '  & ([scriptblock]::Create((irm https://raw.githubusercontent.com/zq940222/claude-pet/main/tools/install.ps1))) -Uninstall'
