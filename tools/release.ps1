# 发布一个新版本：改版本号 -> 构建 -> 打 tag -> 建 GitHub Release。
#
# 用法：
#   .\tools\release.ps1 -Bump patch          # 0.1.0 -> 0.1.1
#   .\tools\release.ps1 -Bump minor          # 0.1.0 -> 0.2.0
#   .\tools\release.ps1 -Bump major          # 0.1.0 -> 1.0.0
#   .\tools\release.ps1 -Version 1.2.3       # 直接指定
#   .\tools\release.ps1 -Bump patch -DryRun  # 只看会做什么，不落地
#
# 版本号唯一来源是 src-tauri/Cargo.toml。

[CmdletBinding(DefaultParameterSetName = 'Bump')]
param(
    [Parameter(ParameterSetName = 'Bump', Mandatory)]
    [ValidateSet('patch', 'minor', 'major')]
    [string]$Bump,

    [Parameter(ParameterSetName = 'Explicit', Mandatory)]
    [ValidatePattern('^\d+\.\d+\.\d+$')]
    [string]$Version,

    [switch]$DryRun
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$repoRoot  = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$cargoToml = Join-Path $repoRoot 'src-tauri\Cargo.toml'
$changelog = Join-Path $repoRoot 'CHANGELOG.md'

function Step($m) { Write-Host "==> $m" -ForegroundColor Cyan }
function Warn($m) { Write-Host "!!  $m" -ForegroundColor Yellow }
function Die($m)  { Write-Host "x   $m" -ForegroundColor Red; exit 1 }

# 编码必须显式，且读写要对称。
# `Get-Content -Raw` 在 Windows PowerShell 5.1 下按 ANSI(GBK) 解码 ——
# 读进来再用 UTF-8 写回去，文件里的中文会被双重损坏。
# .NET 的 ReadAllText 默认 UTF-8 并自动识别 BOM，这才是要的行为。
function Read-Utf8([string]$path) {
    return [System.IO.File]::ReadAllText($path)
}
function Write-Utf8([string]$path, [string]$text) {
    # 不写 BOM：这些是 .md / .toml，.gitattributes 已把它们锁成 LF
    [System.IO.File]::WriteAllText($path, $text, (New-Object System.Text.UTF8Encoding $false))
}

# ── 前置检查 ─────────────────────────────────────────────────

Step 'checking prerequisites'

foreach ($tool in 'cargo', 'git', 'gh') {
    if (-not (Get-Command $tool -ErrorAction SilentlyContinue)) {
        Die "$tool not found on PATH"
    }
}

Push-Location $repoRoot
try {
    # 工作区必须干净 —— 否则 tag 指向的内容和你以为的不一样
    $dirty = git status --porcelain
    if ($dirty -and -not $DryRun) {
        Die "working tree is dirty; commit or stash first:`n$dirty"
    }

    # ── 算新版本号 ───────────────────────────────────────────

    # 用 regex 而不是 Select-String 管道：StrictMode 下对 $null 取属性会直接抛，
    # 拿不到版本号时我们想给出人话错误而不是一坨栈。
    $m = [regex]::Match((Read-Utf8 $cargoToml), '(?m)^version\s*=\s*"([^"]+)"')
    if (-not $m.Success) { Die "could not read version from $cargoToml" }
    $current = $m.Groups[1].Value

    if ($PSCmdlet.ParameterSetName -eq 'Explicit') {
        $new = $Version
    } else {
        $p = $current.Split('.')
        $maj = [int]$p[0]; $min = [int]$p[1]; $pat = [int]$p[2]
        switch ($Bump) {
            'major' { $maj++; $min = 0; $pat = 0 }
            'minor' { $min++; $pat = 0 }
            'patch' { $pat++ }
        }
        $new = "$maj.$min.$pat"
    }

    $tag = "v$new"
    Step "$current -> $new  (tag $tag)"

    if (git tag --list $tag) { Die "tag $tag already exists" }

    if ($DryRun) {
        Warn 'dry run: stopping here. Would have:'
        Write-Host "     - set version = `"$new`" in src-tauri/Cargo.toml"
        Write-Host "     - moved CHANGELOG [Unreleased] into a [$new] section"
        Write-Host "     - cargo build --release"
        Write-Host "     - git commit + tag $tag + push"
        Write-Host "     - gh release create $tag with claude-pet-$new-windows-x64.zip"
        exit 0
    }

    # ── 改版本号 ─────────────────────────────────────────────

    Step 'updating Cargo.toml'
    # 只替换 [package] 里的第一个 version，别碰依赖的 version。
    # 用 Regex 实例的 Replace(input, replacement, count) —— 静态方法的第 4 个
    # 参数是 RegexOptions 而不是 count，传 1 会变成 IgnoreCase 并替换全部。
    $rx = [regex]'(?m)^version\s*=\s*"[^"]+"'
    Write-Utf8 $cargoToml ($rx.Replace((Read-Utf8 $cargoToml), "version = `"$new`"", 1))

    # 改完立刻回读校验 —— 编码或正则出问题时要当场炸，
    # 而不是把损坏的文件一路 commit、tag、push 出去。
    $check = [regex]::Match((Read-Utf8 $cargoToml), '(?m)^version\s*=\s*"([^"]+)"')
    if (-not $check.Success -or $check.Groups[1].Value -ne $new) {
        Die "Cargo.toml verification failed after edit (expected $new). File may be corrupted -- run: git checkout -- src-tauri/Cargo.toml"
    }

    Step 'updating CHANGELOG.md'
    $today = (Get-Date -Format 'yyyy-MM-dd')
    $cl = Read-Utf8 $changelog
    if ($cl -match '(?m)^## \[Unreleased\]\s*$') {
        # 用 LF：.gitattributes 把 *.md 锁成 eol=lf，混进 CRLF 会造成整片行尾变更
        $cl = $cl -replace '(?m)^## \[Unreleased\]\s*$', "## [Unreleased]`n`n## [$new] - $today"
        Write-Utf8 $changelog $cl
        if ((Read-Utf8 $changelog) -notmatch [regex]::Escape("## [$new] - $today")) {
            Die "CHANGELOG verification failed after edit -- run: git checkout -- CHANGELOG.md"
        }
    } else {
        Warn 'no [Unreleased] heading found; CHANGELOG left alone'
    }

    # ── 构建 ─────────────────────────────────────────────────

    Step 'cargo build --release  (this takes a few minutes)'
    Push-Location (Join-Path $repoRoot 'src-tauri')
    try {
        # 运行中的 exe 会锁住输出文件
        Get-Process claude-pet -ErrorAction SilentlyContinue | Stop-Process -Force
        Start-Sleep -Milliseconds 600
        cargo build --release
        if ($LASTEXITCODE -ne 0) { Die 'release build failed' }
    } finally { Pop-Location }

    $exe = Join-Path $repoRoot 'src-tauri\target\release\claude-pet.exe'
    if (-not (Test-Path $exe)) { Die "built exe not found at $exe" }
    $sizeMb = [math]::Round((Get-Item $exe).Length / 1MB, 2)
    Step "built claude-pet.exe ($sizeMb MB)"

    # ── 打包 ─────────────────────────────────────────────────

    $zipName = "claude-pet-$new-windows-x64.zip"
    $zipPath = Join-Path $repoRoot "src-tauri\target\release\$zipName"
    if (Test-Path $zipPath) { Remove-Item $zipPath -Force }
    Compress-Archive -Path $exe -DestinationPath $zipPath
    Step "packaged $zipName"

    # ── 提交 / tag / 推送 ────────────────────────────────────

    Step 'committing and tagging'
    git add src-tauri/Cargo.toml src-tauri/Cargo.lock CHANGELOG.md
    git commit -m "Release $tag"
    if ($LASTEXITCODE -ne 0) { Die 'commit failed' }
    git tag -a $tag -m "Release $tag"
    git push
    git push origin $tag
    if ($LASTEXITCODE -ne 0) { Die 'push failed' }

    # ── GitHub Release ───────────────────────────────────────

    Step "creating GitHub release $tag"
    # 从 CHANGELOG 抠出这个版本那一段当 release notes
    $notesFile = Join-Path $env:TEMP "claude-pet-notes-$new.md"
    $section = [regex]::Match(
        (Read-Utf8 $changelog),
        "(?ms)^## \[$([regex]::Escape($new))\][^\r\n]*\r?\n(.*?)(?=^## \[|\z)")
    $notes = if ($section.Success) { $section.Groups[1].Value.Trim() } else { "Release $tag" }
    # release notes 要给 GitHub 看，中文必须是正确的 UTF-8
    Write-Utf8 $notesFile $notes

    gh release create $tag $zipPath --title $tag --notes-file $notesFile
    if ($LASTEXITCODE -ne 0) { Die 'gh release create failed' }
    Remove-Item $notesFile -Force -ErrorAction SilentlyContinue

    Step "done: $tag published"
    Write-Host ""
    Write-Host "install it anywhere with:" -ForegroundColor Green
    Write-Host "  irm https://raw.githubusercontent.com/zq940222/claude-pet/main/tools/install.ps1 | iex"
}
finally { Pop-Location }
