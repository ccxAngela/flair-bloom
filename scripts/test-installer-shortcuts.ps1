param(
    [Parameter(Mandatory = $true)][string]$MakeNsisPath,
    [Parameter(Mandatory = $true)][string]$TauriUtilsPath
)

$ErrorActionPreference = 'Stop'
$repoRoot = Split-Path -Parent $PSScriptRoot
$fixtureRoot = Join-Path ([System.IO.Path]::GetTempPath()) ('flair-shortcuts-' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $fixtureRoot | Out-Null

# 唯一替换项是桌面根目录，生产钩子的控制流与 COM 目标检查原样执行。
$hook = Get-Content -LiteralPath (Join-Path $repoRoot 'apps/main/src-tauri/installer.nsh') -Encoding UTF8 -Raw
$hook = $hook.Replace('$DESKTOP', '${FLAIR_FIXTURE_ROOT}\desktop')
[System.IO.File]::WriteAllText((Join-Path $fixtureRoot 'installer-under-test.nsh'), $hook, [System.Text.UTF8Encoding]::new($false))
Copy-Item -LiteralPath $TauriUtilsPath -Destination (Join-Path $fixtureRoot 'utils.nsh')

& $MakeNsisPath /V2 /INPUTCHARSET UTF8 "/DFLAIR_FIXTURE_ROOT=$fixtureRoot" (Join-Path $PSScriptRoot 'fixtures/installer-shortcuts.nsi')
if ($LASTEXITCODE -ne 0) { throw "NSIS 编译失败：$LASTEXITCODE" }

$installRun = Start-Process -FilePath (Join-Path $fixtureRoot 'installer-fixture.exe') -WindowStyle Hidden -Wait -PassThru
if ($installRun.ExitCode -ne 0) { throw "安装阶段回归失败：$($installRun.ExitCode)，夹具：$fixtureRoot" }
$uninstallRun = Start-Process -FilePath (Join-Path $fixtureRoot 'app/uninstall-fixture.exe') -ArgumentList @('/S', "_?=$fixtureRoot\app") -WindowStyle Hidden -Wait -PassThru
if ($uninstallRun.ExitCode -ne 0) { throw "卸载阶段回归失败：$($uninstallRun.ExitCode)，夹具：$fixtureRoot" }

Write-Output "快捷方式回归通过：创建选择、完成页、升级去重、目标冲突、更新卸载保留、正式卸载清理。夹具：$fixtureRoot"
