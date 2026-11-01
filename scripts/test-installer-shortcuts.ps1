param(
    [Parameter(Mandatory = $true)][string]$MakeNsisPath,
    [Parameter(Mandatory = $true)][string]$TauriUtilsPath
)

$ErrorActionPreference = 'Stop'
$repoRoot = Split-Path -Parent $PSScriptRoot
$windowsConfig = Get-Content -LiteralPath (Join-Path $repoRoot 'apps/main/src-tauri/tauri.windows.conf.json') -Encoding UTF8 -Raw | ConvertFrom-Json
if ($windowsConfig.bundle.resources -notcontains 'icons/icon.ico') { throw 'Windows 安装包缺少桌面快捷方式的独立图标资源。' }
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

$shortcutShell = New-Object -ComObject WScript.Shell
$migrated = $shortcutShell.CreateShortcut((Join-Path $fixtureRoot 'desktop/气质花 v0.8.2.lnk'))
$expectedIcon = Join-Path $fixtureRoot 'app/icons/icon.ico'
if ($migrated.IconLocation -ne ($expectedIcon + ',0') -or -not (Test-Path -LiteralPath $expectedIcon -PathType Leaf)) {
    throw "快捷方式未指向安装目录中的独立图标，夹具：$fixtureRoot"
}
if ($migrated.Arguments -ne '--fixture' -or $migrated.Description -ne '保留快捷方式属性' -or $migrated.WindowStyle -ne 7 -or $migrated.WorkingDirectory -ne (Join-Path $fixtureRoot 'app')) {
    throw "图标修复丢失快捷方式属性，夹具：$fixtureRoot"
}
$conflicting = $shortcutShell.CreateShortcut((Join-Path $fixtureRoot 'desktop/气质花 v0.8.3.lnk'))
if ($conflicting.IconLocation -notmatch '\\shell32\.dll,7$') { throw "错误修改其他安装的快捷方式图标，夹具：$fixtureRoot" }

$uninstallRun = Start-Process -FilePath (Join-Path $fixtureRoot 'app/uninstall-fixture.exe') -ArgumentList @('/S', "_?=$fixtureRoot\app") -WindowStyle Hidden -Wait -PassThru
if ($uninstallRun.ExitCode -ne 0) { throw "卸载阶段回归失败：$($uninstallRun.ExitCode)，夹具：$fixtureRoot" }

Write-Output "快捷方式回归通过：创建选择、完成页、升级去重、图标修复、属性保留、目标冲突、更新卸载保留、正式卸载清理。夹具：$fixtureRoot"
