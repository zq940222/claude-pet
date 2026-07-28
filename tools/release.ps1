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
    $m = [regex]::Match((Get-Content $cargoToml -Raw), '(?m)^version\s*=\s*"([^"]+)"')
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
    # 只替换 [package] 里的第一个 version，别碰依赖的 version
    $toml = Get-Content $cargoToml -Raw
    $toml = [regex]::Replace($toml, '(?m)^version\s*=\s*"[^"]+"', "version = `"$new`"", 1)
    [System.IO.File]::WriteAllText($cargoToml, $toml, (New-Object System.Text.UTF8Encoding $false))

    Step 'updating CHANGELOG.md'
    $today = (Get-Date -Format 'yyyy-MM-dd')
    $cl = Get-Content $changelog -Raw
    if ($cl -match '(?m)^## \[Unreleased\]\s*$') {
        $cl = $cl -replace '(?m)^## \[Unreleased\]\s*$', "## [Unreleased]`r`n`r`n## [$new] - $today"
        [System.IO.File]::WriteAllText($changelog, $cl, (New-Object System.Text.UTF8Encoding $false))
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
        (Get-Content $changelog -Raw),
        "(?ms)^## \[$([regex]::Escape($new))\][^\r\n]*\r?\n(.*?)(?=^## \[|\z)")
    $notes = if ($section.Success) { $section.Groups[1].Value.Trim() } else { "Release $tag" }
    [System.IO.File]::WriteAllText($notesFile, $notes, (New-Object System.Text.UTF8Encoding $false))

    gh release create $tag $zipPath --title $tag --notes-file $notesFile
    if ($LASTEXITCODE -ne 0) { Die 'gh release create failed' }
    Remove-Item $notesFile -Force -ErrorAction SilentlyContinue

    Step "done: $tag published"
    Write-Host ""
    Write-Host "install it anywhere with:" -ForegroundColor Green
    Write-Host "  irm https://raw.githubusercontent.com/zq940222/claude-pet/main/tools/install.ps1 | iex"
}
finally { Pop-Location }
