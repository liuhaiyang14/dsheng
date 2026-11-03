<# dsheng 独立 CLI 用户级安装模板（Windows 10 22H2 / Windows 11 x64，用例 37）。

行为约定（docs/specs/provider-config-delivery.md §6）：
  - 复制 dsheng.exe 到 %LOCALAPPDATA%\Programs\dsheng
  - 不提权（无需管理员）、不修改 PATH、不执行策略绕过
    （不调用 Set-ExecutionPolicy、Unblock-File 或关闭 SmartScreen）
  - 如本脚本被执行策略拦截：不要改系统策略，改用手动复制（README.txt）：
    Copy-Item <归档>\dsheng.exe "$env:LOCALAPPDATA\Programs\dsheng\"
    手动路径与脚本等效，本脚本只是省事的可选辅助。

用法（在解压后的 CLI 归档目录内，且当前策略允许运行脚本时）：
  powershell -File .\install\windows-user-install.ps1
  可选参数：-Source <dsheng.exe 路径>  -InstallDir <目标目录>  -NoVerify

注意：本文件为模板，须在 Windows x64 真机/CI 产物上验证后交付（见
docs/release/RELEASE-CHECKLIST.md）。
#>
[CmdletBinding()]
param(
    [string]$Source = "",
    [string]$InstallDir = (Join-Path $env:LOCALAPPDATA 'Programs\dsheng'),
    [switch]$NoVerify
)

$ErrorActionPreference = 'Stop'

if (-not $Source) {
    foreach ($cand in @((Join-Path $PSScriptRoot '..\dsheng.exe'), (Join-Path $PSScriptRoot '..\bin\dsheng.exe'))) {
        if (Test-Path $cand) { $Source = (Resolve-Path $cand).Path; break }
    }
}
if (-not $Source -or -not (Test-Path $Source)) {
    Write-Error "未找到 dsheng.exe（用 -Source 指定路径）。"
}

$sumsPath = Join-Path $PSScriptRoot '..\SHA256SUMS.txt'
if (Test-Path $sumsPath) {
    $rel = Split-Path -Leaf $Source
    $expected = $null
    foreach ($line in Get-Content $sumsPath) {
        if ($line -match '^([0-9a-f]{64})\s+\*?([^\s]+)$' -and (Split-Path -Leaf $Matches[2]) -eq $rel) {
            $expected = $Matches[1]; break
        }
    }
    if ($expected) {
        $actual = (Get-FileHash -Algorithm SHA256 $Source).Hash.ToLowerInvariant()
        if ($actual -ne $expected) { Write-Error "SHA-256 校验失败：$rel" }
        Write-Output "校验通过：$rel ($($actual.Substring(0,16))...)"
    }
}

New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null
$dest = Join-Path $InstallDir 'dsheng.exe'
Copy-Item -Path $Source -Destination $dest -Force
Write-Output "已安装：$dest"

if (-not $NoVerify) {
    try {
        $out = & $dest --version 2>$null | Out-String
        if ($LASTEXITCODE -eq 0 -and $out -match '"ok":\s*true') {
            Write-Output "冒烟通过：$dest --version"
        } else {
            Write-Warning "dsheng --version 未返回成功 JSON；若被 SmartScreen 拦截属正常安全流程，见 docs/release/UNSIGNED.md。"
        }
    } catch {
        Write-Warning "运行 $dest --version 失败：$_（SmartScreen/策略拦截见 docs/release/UNSIGNED.md；本脚本不绕过。）"
    }
}

Write-Output ''
Write-Output 'PATH 未修改。如需任意目录调用，请手动把下面目录加入用户 PATH（系统设置→环境变量）：'
Write-Output "  $InstallDir"
Write-Output "然后运行：$dest --help"
