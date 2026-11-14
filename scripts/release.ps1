# 打发布包：守门 → 复制 exe/文档 → 压缩
#
# 用法：
#   powershell -NoProfile -ExecutionPolicy Bypass -File scripts\release.ps1
#   powershell -NoProfile -ExecutionPolicy Bypass -File scripts\release.ps1 -SkipBuild
#
# 产物在 release\ 下：
#   vram-check.exe
#   vram-check-<版本>-win64.zip

param([switch]$SkipBuild)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
Push-Location $root
try {
    $tomlPath = Join-Path $root 'Cargo.toml'
    $toml = [System.IO.File]::ReadAllText($tomlPath, [System.Text.Encoding]::UTF8)
    if ($toml -notmatch '(?m)^\s*version\s*=\s*"([^"]+)"') { throw 'Cargo.toml 里读不到 version。' }
    $version = $Matches[1]

    if (-not $SkipBuild) {
        & powershell -NoProfile -ExecutionPolicy Bypass -File (Join-Path $PSScriptRoot 'gate.ps1')
        if ($LASTEXITCODE -ne 0) { throw '守门未通过，停止打包。' }
    }

    $exe = Join-Path $root 'target\release\vram-check.exe'
    if (-not (Test-Path $exe)) { throw "找不到 $exe，先 cargo build --release。" }

    $out = Join-Path $root 'release'
    New-Item -ItemType Directory -Force $out | Out-Null
    Copy-Item $exe (Join-Path $out 'vram-check.exe') -Force
    foreach ($name in 'LICENSE', 'README.md', 'THIRD-PARTY-NOTICES.md') {
        Copy-Item (Join-Path $root $name) (Join-Path $out $name) -Force
    }

    $zip = Join-Path $out "vram-check-$version-win64.zip"
    Compress-Archive -Path @(
        (Join-Path $out 'vram-check.exe'),
        (Join-Path $out 'LICENSE'),
        (Join-Path $out 'README.md'),
        (Join-Path $out 'THIRD-PARTY-NOTICES.md')
    ) -DestinationPath $zip -Force

    $exeSize = [math]::Round((Get-Item (Join-Path $out 'vram-check.exe')).Length / 1KB)
    $zipSize = [math]::Round((Get-Item $zip).Length / 1KB)
    "版本: $version"
    "exe : release\vram-check.exe（$exeSize KB）"
    "zip : release\vram-check-$version-win64.zip（$zipSize KB）"
}
finally {
    Pop-Location
}
