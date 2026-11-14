# 发布守门：体积 + 冷启动 + 依赖检查
#
# 用法：
#   powershell -NoProfile -File scripts\gate.ps1
#   powershell -NoProfile -File scripts\gate.ps1 -SkipBuild
#
# 红线（docs/CONVENTIONS.md §9）：
#   exe ≤ 2 MB；冷启动 ≤ 100 ms。

param([switch]$SkipBuild)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
Push-Location $root
try {
    $cargo = Get-Command cargo -ErrorAction SilentlyContinue
    if (-not $cargo) {
        $local = Join-Path $env:USERPROFILE '.cargo\bin\cargo.exe'
        if (Test-Path $local) { $cargo = Get-Item $local } else { throw '找不到 cargo，请先装 Rust 工具链。' }
    }

    if (-not $SkipBuild) {
        & $cargo build --release
        if ($LASTEXITCODE -ne 0) { throw 'cargo build --release 失败。' }
    }

    $exe = Join-Path $root 'target\release\vram-check.exe'
    if (-not (Test-Path $exe)) { throw "产物不存在：$exe" }

    $size = (Get-Item $exe).Length
    $sizeKb = [math]::Round($size / 1KB)
    "体积: $sizeKb KB（预算 2048 KB）"
    if ($size -gt 2MB) { throw "体积超标：$sizeKb KB > 2048 KB" }

    # 预热一次（磁盘/杀软），再测 3 次取中位。
    function Measure-StartOnce {
        $psi = New-Object System.Diagnostics.ProcessStartInfo
        $psi.FileName = $exe
        $psi.UseShellExecute = $false
        $sw = [System.Diagnostics.Stopwatch]::StartNew()
        $proc = [System.Diagnostics.Process]::Start($psi)
        $null = $proc.WaitForInputIdle(3000)
        $sw.Stop()
        $ms = $sw.Elapsed.TotalMilliseconds
        $proc.CloseMainWindow() | Out-Null
        $proc.WaitForExit(2000) | Out-Null
        return $ms
    }

    $null = Measure-StartOnce
    $runs = @()
    1..5 | ForEach-Object {
        $runs += Measure-StartOnce
        Start-Sleep -Milliseconds 250
    }
    $sorted = $runs | Sort-Object
    $median = [math]::Round($sorted[2], 1)
    $runText = ($runs | ForEach-Object { [math]::Round($_, 1) }) -join ', '
    "冷启动: $runText ms，中位 $median ms（预算 100 ms）"
    if ($median -gt 100) { throw "冷启动超标：中位 $median ms > 100 ms" }

    $tree = & $cargo tree --depth 1 --offline 2>$null
    $deps = ($tree | Select-String -Pattern '^[a-zA-Z].* v' | ForEach-Object { $_.Line.Trim() }) -join '; '
    "依赖: $deps"
    '守门通过。'
}
finally {
    Pop-Location
}
