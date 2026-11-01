# Windows PowerShell 5.1+. Read-only collection; writes only its report.
# No elevation is requested. Keep the affected window visible when collecting.
[CmdletBinding()]
param(
    # Optional. Normal users do not need this.
    # The script auto-detects the default FlairBloom install location first.
    # This can point to the app root, resources folder, or ddhid-driver folder.
    [string]$DriverDir = "",

    # Optional. Defaults to Desktop\flair-environment-<timestamp>.txt.
    [string]$OutFile = "",

    # Internal app mode. The app writes its structured diagnosis separately and
    # uses this script only for Windows-side evidence that is hard to collect in Rust.
    [switch]$EvidenceOnly
)

$ErrorActionPreference = "Continue"
$script:Lines = New-Object System.Collections.Generic.List[string]

function Add-Line {
    param([string]$Text = "")
    [void]$script:Lines.Add($Text)
}

function Add-Section {
    param([string]$Name)
    Add-Line ""
    Add-Line ("==== {0} ====" -f $Name)
}

function Add-CommandOutput {
    param(
        [string]$Title,
        [scriptblock]$Block
    )
    Add-Section $Title
    try {
        $text = (& $Block 2>&1 | Out-String -Width 260).TrimEnd()
        if ([string]::IsNullOrWhiteSpace($text)) {
            Add-Line "<no output>"
        } else {
            Add-Line $text
        }
    } catch {
        Add-Line ("ERROR: {0}" -f $_.Exception.Message)
    }
}

function Test-IsAdmin {
    try {
        $identity = [Security.Principal.WindowsIdentity]::GetCurrent()
        $principal = New-Object Security.Principal.WindowsPrincipal($identity)
        return $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
    } catch {
        return $false
    }
}

function Add-UniqueCandidate {
    param(
        [System.Collections.Generic.List[string]]$List,
        [string]$Path
    )
    if ([string]::IsNullOrWhiteSpace($Path)) {
        return
    }
    $expanded = [Environment]::ExpandEnvironmentVariables($Path.Trim('"'))
    if ([string]::IsNullOrWhiteSpace($expanded)) {
        return
    }
    if (-not $List.Contains($expanded)) {
        [void]$List.Add($expanded)
    }
}

function Add-DriverDirVariants {
    param(
        [System.Collections.Generic.List[string]]$List,
        [string]$Root
    )
    if ([string]::IsNullOrWhiteSpace($Root)) {
        return
    }
    $clean = [Environment]::ExpandEnvironmentVariables($Root.Trim().Trim('"'))
    if ([string]::IsNullOrWhiteSpace($clean)) {
        return
    }
    Add-UniqueCandidate $List $clean
    Add-UniqueCandidate $List (Join-Path $clean "ddhid-driver")
    Add-UniqueCandidate $List (Join-Path $clean "resources")
    Add-UniqueCandidate $List (Join-Path $clean "resources\ddhid-driver")
    Add-UniqueCandidate $List (Join-Path $clean "2.hid\drv")
}

function Get-PathFromCommandText {
    param([string]$Text)
    if ([string]::IsNullOrWhiteSpace($Text)) {
        return ""
    }
    $trimmed = [Environment]::ExpandEnvironmentVariables($Text.Trim())
    if ($trimmed.StartsWith('"')) {
        $end = $trimmed.IndexOf('"', 1)
        if ($end -gt 1) {
            return $trimmed.Substring(1, $end - 1)
        }
    }
    $exeIndex = $trimmed.ToLowerInvariant().IndexOf(".exe")
    if ($exeIndex -gt 0) {
        return $trimmed.Substring(0, $exeIndex + 4).Trim('"')
    }
    return $trimmed.Trim('"')
}

function Add-RegistryInstallCandidates {
    param([System.Collections.Generic.List[string]]$List)
    $roots = @(
        "HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall",
        "HKLM:\Software\Microsoft\Windows\CurrentVersion\Uninstall",
        "HKLM:\Software\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall"
    )
    foreach ($root in $roots) {
        if (-not (Test-Path -LiteralPath $root)) {
            continue
        }
        try {
            Get-ChildItem -LiteralPath $root -ErrorAction SilentlyContinue | ForEach-Object {
                $p = Get-ItemProperty -LiteralPath $_.PSPath -ErrorAction SilentlyContinue
                $text = @($p.DisplayName, $p.DisplayIcon, $p.InstallLocation, $p.UninstallString, $p.QuietUninstallString) -join " "
                if ($text -notmatch "FlairBloom|flair-bloom|fun\.xwink\.flairbloom") {
                    return
                }
                Add-DriverDirVariants $List $p.InstallLocation
                foreach ($cmd in @($p.DisplayIcon, $p.UninstallString, $p.QuietUninstallString)) {
                    $exe = Get-PathFromCommandText $cmd
                    if (-not [string]::IsNullOrWhiteSpace($exe)) {
                        Add-DriverDirVariants $List (Split-Path -Parent $exe)
                    }
                }
            }
        } catch {
            # Registry probing is best-effort.
        }
    }
}

function Resolve-DriverDir {
    param([string]$InputDir)
    $candidates = New-Object System.Collections.Generic.List[string]

    if (-not [string]::IsNullOrWhiteSpace($InputDir)) {
        Add-DriverDirVariants $candidates $InputDir
    }

    Add-RegistryInstallCandidates $candidates

    Add-DriverDirVariants $candidates (Join-Path $env:LOCALAPPDATA "Programs\FlairBloom")
    Add-DriverDirVariants $candidates (Join-Path $env:LOCALAPPDATA "FlairBloom")
    Add-DriverDirVariants $candidates (Join-Path $env:ProgramFiles "FlairBloom")
    if (${env:ProgramFiles(x86)}) {
        Add-DriverDirVariants $candidates (Join-Path ${env:ProgramFiles(x86)} "FlairBloom")
    }
    Add-DriverDirVariants $candidates "C:\Program Files\FlairBloom"
    Add-DriverDirVariants $candidates "C:\Program Files (x86)\FlairBloom"

    if ($PSScriptRoot) {
        Add-DriverDirVariants $candidates $PSScriptRoot
    }
    Add-DriverDirVariants $candidates (Get-Location).Path

    foreach ($candidate in $candidates) {
        if ((Test-Path -LiteralPath (Join-Path $candidate "ddc.exe")) -or
            (Test-Path -LiteralPath (Join-Path $candidate "ddhid63340.inf"))) {
            return (Resolve-Path -LiteralPath $candidate -ErrorAction SilentlyContinue).Path
        }
    }
    return ""
}

function Add-FileProbe {
    param(
        [string]$Path,
        [string]$ExpectedSha256 = ""
    )
    if ([string]::IsNullOrWhiteSpace($Path)) {
        return
    }

    Add-Line ("-- {0}" -f $Path)
    if (-not (Test-Path -LiteralPath $Path)) {
        Add-Line "exists: false"
        return
    }

    try {
        $item = Get-Item -LiteralPath $Path -Force
        Add-Line ("exists: true")
        Add-Line ("length: {0}" -f $item.Length)
        Add-Line ("last_write_time: {0:o}" -f $item.LastWriteTime)
        Add-Line ("attributes: {0}" -f $item.Attributes)
    } catch {
        Add-Line ("item_error: {0}" -f $_.Exception.Message)
    }

    try {
        $hash = Get-FileHash -LiteralPath $Path -Algorithm SHA256
        Add-Line ("sha256: {0}" -f $hash.Hash)
        if (-not [string]::IsNullOrWhiteSpace($ExpectedSha256)) {
            Add-Line ("sha256_expected: {0}" -f $ExpectedSha256)
            Add-Line ("sha256_match: {0}" -f ($hash.Hash.ToUpperInvariant() -eq $ExpectedSha256.ToUpperInvariant()))
        }
    } catch {
        Add-Line ("hash_error: {0}" -f $_.Exception.Message)
    }

    try {
        $sig = Get-AuthenticodeSignature -LiteralPath $Path
        Add-Line ("signature_status: {0}" -f $sig.Status)
        Add-Line ("signature_status_message: {0}" -f $sig.StatusMessage)
        if ($sig.SignerCertificate) {
            Add-Line ("signer_subject: {0}" -f $sig.SignerCertificate.Subject)
            Add-Line ("signer_thumbprint: {0}" -f $sig.SignerCertificate.Thumbprint)
            Add-Line ("signer_not_before: {0:o}" -f $sig.SignerCertificate.NotBefore)
            Add-Line ("signer_not_after: {0:o}" -f $sig.SignerCertificate.NotAfter)
        }
    } catch {
        Add-Line ("signature_error: {0}" -f $_.Exception.Message)
    }
}

function Add-RegistryKey {
    param([string]$Path)
    Add-Line ("-- {0}" -f $Path)
    if (-not (Test-Path -LiteralPath $Path)) {
        Add-Line "exists: false"
        return
    }
    Add-Line "exists: true"
    try {
        Get-ItemProperty -LiteralPath $Path | Format-List * | Out-String -Width 260 | ForEach-Object { Add-Line $_.TrimEnd() }
    } catch {
        Add-Line ("registry_error: {0}" -f $_.Exception.Message)
    }
}

function Add-SetupApiMatches {
    param([string]$Path)
    Add-Line ("-- {0}" -f $Path)
    if (-not (Test-Path -LiteralPath $Path)) {
        Add-Line "exists: false"
        return
    }

    try {
        $matches = @(Select-String -LiteralPath $Path -Pattern "ddhid63340", "ddxoft", "ddhid63340\\HID_DEVICE" -Context 10, 18 -ErrorAction SilentlyContinue)
        if ($matches.Count -eq 0) {
            Add-Line "No direct DDHID matches found."
            Add-Line "Last 80 setupapi error-ish lines:"
            $errors = @(Select-String -LiteralPath $Path -Pattern "!!!", "#E", "0xe000", "failed", "error" -SimpleMatch -ErrorAction SilentlyContinue | Select-Object -Last 80)
            if ($errors.Count -eq 0) {
                Add-Line "<none>"
            } else {
                $errors | ForEach-Object { Add-Line ("{0}:{1}: {2}" -f $_.Path, $_.LineNumber, $_.Line.TrimEnd()) }
            }
            return
        }

        $matches | Select-Object -Last 12 | ForEach-Object {
            Add-Line ""
            Add-Line ("match_line: {0}" -f $_.LineNumber)
            if ($_.Context.PreContext) {
                $_.Context.PreContext | ForEach-Object { Add-Line ("  {0}" -f $_) }
            }
            Add-Line ("> {0}" -f $_.Line)
            if ($_.Context.PostContext) {
                $_.Context.PostContext | ForEach-Object { Add-Line ("  {0}" -f $_) }
            }
        }
    } catch {
        Add-Line ("setupapi_error: {0}" -f $_.Exception.Message)
    }
}

function Add-EventMatches {
    param(
        [string]$LogName,
        [string]$Pattern,
        [int]$MaxEvents = 300
    )
    Add-Line ("-- {0}" -f $LogName)
    try {
        $events = @(Get-WinEvent -LogName $LogName -MaxEvents $MaxEvents -ErrorAction Stop |
            Where-Object { $_.Message -match $Pattern } |
            Select-Object -First 30)
        if ($events.Count -eq 0) {
            Add-Line "<no matching events>"
            return
        }
        foreach ($event in $events) {
            Add-Line ("time: {0:o}" -f $event.TimeCreated)
            Add-Line ("provider: {0}" -f $event.ProviderName)
            Add-Line ("id: {0}" -f $event.Id)
            Add-Line ("level: {0}" -f $event.LevelDisplayName)
            Add-Line ($event.Message -replace "`r?`n", " ")
            Add-Line ""
        }
    } catch {
        Add-Line ("event_log_error: {0}" -f $_.Exception.Message)
    }
}

function Get-PendingRenameEntries {
    try {
        $value = (Get-ItemProperty -LiteralPath "HKLM:\SYSTEM\CurrentControlSet\Control\Session Manager" -Name "PendingFileRenameOperations" -ErrorAction SilentlyContinue).PendingFileRenameOperations
        if ($null -eq $value) { return @() }
        return @($value)
    } catch {
        return @()
    }
}

function Get-DisplayEnvironment {
$ErrorActionPreference = 'Stop'
$diagnosticErrors = New-Object 'System.Collections.Generic.List[string]'
$report = [ordered]@{
    reportVersion = 2
    capturedAt = (Get-Date).ToString('o')
    powershell = $PSVersionTable.PSVersion.ToString()
    process64Bit = [Environment]::Is64BitProcess
    actualWebviewZoom = 'Unavailable from external read-only collection; saved scale is not actual zoom.'
    notes = @('No app settings, registry, drivers, window placement or system scaling are changed.',
        'Only this report is written. Hidden/minimized window sizes may not describe the visible problem.')
}

function Hide-UserPath([string]$Text) {
    if ($env:USERPROFILE) { return $Text.Replace($env:USERPROFILE, '%USERPROFILE%') }
    return $Text
}

try {
    $os = Get-ItemProperty 'HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion'
    $osDetails = Get-CimInstance Win32_OperatingSystem
    $report.windows = [ordered]@{
        caption = $osDetails.Caption
        productNameRegistry = $os.ProductName
        displayVersion = $os.DisplayVersion
        build = ('{0}.{1}' -f $os.CurrentBuildNumber, $os.UBR)
        architecture = $osDetails.OSArchitecture
    }
    try {
        $textScale = Get-ItemProperty 'HKCU:\Software\Microsoft\Accessibility' -ErrorAction Stop
    } catch [System.Management.Automation.ItemNotFoundException] {
        $textScale = $null
    }
    $report.accessibilityTextScalePercent = if ($null -eq $textScale.TextScaleFactor) { 100 } else { [int]$textScale.TextScaleFactor }
} catch { $diagnosticErrors.Add('Windows info: ' + $_.Exception.Message) }

try {
    if (-not ('FlairDisplayReadOnlyV1' -as [type])) {
        Add-Type -TypeDefinition @'
using System;
using System.Text;
using System.Collections.Generic;
using System.Runtime.InteropServices;
public static class FlairDisplayReadOnlyV1 {
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int left, top, right, bottom; }
    [StructLayout(LayoutKind.Sequential, CharSet=CharSet.Unicode)] public struct MI {
        public int size; public RECT monitor, work; public uint flags;
        [MarshalAs(UnmanagedType.ByValTStr, SizeConst=32)] public string device;
    }
    delegate bool WindowProc(IntPtr h, IntPtr p);
    delegate bool MonitorProc(IntPtr h, IntPtr dc, ref RECT r, IntPtr p);
    [DllImport("user32.dll")] static extern IntPtr SetThreadDpiAwarenessContext(IntPtr c);
    [DllImport("user32.dll")] static extern bool EnumWindows(WindowProc f, IntPtr p);
    [DllImport("user32.dll")] static extern bool EnumChildWindows(IntPtr h, WindowProc f, IntPtr p);
    [DllImport("user32.dll")] static extern bool EnumDisplayMonitors(IntPtr dc, IntPtr clip, MonitorProc f, IntPtr p);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern bool GetMonitorInfo(IntPtr h, ref MI i);
    [DllImport("shcore.dll")] static extern int GetScaleFactorForMonitor(IntPtr h, out int scale);
    [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern int GetClassName(IntPtr h, StringBuilder s, int n);
    [DllImport("user32.dll")] static extern bool GetClientRect(IntPtr h, out RECT r);
    [DllImport("user32.dll")] static extern bool GetWindowRect(IntPtr h, out RECT r);
    [DllImport("user32.dll")] static extern uint GetDpiForWindow(IntPtr h);
    [DllImport("user32.dll")] static extern IntPtr GetWindowDpiAwarenessContext(IntPtr h);
    [DllImport("user32.dll")] static extern int GetAwarenessFromDpiAwarenessContext(IntPtr c);
    [DllImport("user32.dll")] static extern bool IsWindowVisible(IntPtr h);
    [DllImport("user32.dll")] static extern bool IsIconic(IntPtr h);
    [DllImport("user32.dll")] static extern IntPtr MonitorFromWindow(IntPtr h, uint flags);
    static MI MonitorInfo(IntPtr h) {
        MI info = new MI(); info.size = Marshal.SizeOf(typeof(MI));
        GetMonitorInfo(h, ref info); return info;
    }
    static object Rect(RECT r) { return new { x=r.left, y=r.top, width=r.right-r.left, height=r.bottom-r.top }; }
    static object Window(IntPtr h, IntPtr root) {
        uint pid; GetWindowThreadProcessId(h, out pid);
        var cls = new StringBuilder(256); GetClassName(h, cls, cls.Capacity);
        RECT outer, client; bool outerOk=GetWindowRect(h, out outer), clientOk=GetClientRect(h, out client);
        uint dpi = GetDpiForWindow(h);
        return new { hwnd=h.ToInt64().ToString("X"), rootHwnd=root.ToInt64().ToString("X"),
            processId=pid, isRoot=h==root, windowClass=cls.ToString(), visible=IsWindowVisible(h),
            minimized=IsIconic(h), monitor=MonitorInfo(MonitorFromWindow(h, 2)).device,
            dpi=dpi, dpiAwareness=GetAwarenessFromDpiAwarenessContext(GetWindowDpiAwarenessContext(h)),
            outerReadOk=outerOk, clientReadOk=clientOk, outerPhysical=Rect(outer), clientPhysical=Rect(client),
            clientLogicalWidth=dpi==0 ? 0 : Math.Round((client.right-client.left)*96.0/dpi, 3),
            clientLogicalHeight=dpi==0 ? 0 : Math.Round((client.bottom-client.top)*96.0/dpi, 3) };
    }
    public static object Capture(int[] processIds) {
        var monitors=new List<object>(); var windows=new List<object>();
        var wanted=new HashSet<int>(processIds);
        // Affects only this collector thread, and is restored before returning.
        IntPtr previous=SetThreadDpiAwarenessContext(new IntPtr(-4));
        try {
            EnumDisplayMonitors(IntPtr.Zero, IntPtr.Zero, delegate(IntPtr h, IntPtr dc, ref RECT r, IntPtr p) {
                var info=MonitorInfo(h); int scale; int status=GetScaleFactorForMonitor(h, out scale);
                monitors.Add(new { device=info.device, primary=(info.flags&1)!=0,
                    boundsPhysical=Rect(info.monitor), workAreaPhysical=Rect(info.work),
                    scalePercent=scale, scaleQueryStatus=status }); return true;
            }, IntPtr.Zero);
            EnumWindows(delegate(IntPtr h, IntPtr p) {
                uint pid; GetWindowThreadProcessId(h, out pid);
                if (!wanted.Contains((int)pid)) return true;
                windows.Add(Window(h,h));
                EnumChildWindows(h, delegate(IntPtr child, IntPtr unused) {
                    windows.Add(Window(child,h)); return true;
                }, IntPtr.Zero); return true;
            }, IntPtr.Zero);
            return new { physicalMeasurementContextApplied=previous!=IntPtr.Zero,
                dpiAwarenessLegend="0=unaware, 1=system aware, 2=per-monitor aware, -1=unknown",
                monitors=monitors.ToArray(), windows=windows.ToArray() };
        } finally { if (previous!=IntPtr.Zero) SetThreadDpiAwarenessContext(previous); }
    }
}
'@
    }
    $appProcesses = @(Get-Process -Name 'flair-bloom' -ErrorAction SilentlyContinue)
    $report.appProcesses = @($appProcesses | ForEach-Object {
        $version = $null
        try { if ($_.Path) { $version = [Diagnostics.FileVersionInfo]::GetVersionInfo($_.Path).FileVersion } } catch {}
        [ordered]@{ processId=$_.Id; fileVersion=$version }
    })
    $report.native = [FlairDisplayReadOnlyV1]::Capture([int[]]@($appProcesses | ForEach-Object { $_.Id }))
    if ($appProcesses.Count -eq 0) { $diagnosticErrors.Add('FlairBloom is not running; window measurements are unavailable.') }
} catch { $diagnosticErrors.Add('Window/display info: ' + $_.Exception.Message) }

try {
    $settingsPath = Join-Path $env:APPDATA 'fun.xwink.flairbloom\settings.json'
    if (Test-Path -LiteralPath $settingsPath) {
        $settings = Get-Content -LiteralPath $settingsPath -Raw -Encoding UTF8 | ConvertFrom-Json
        $report.savedInputMode = $settings.input_mode
        # Deliberately omit profile names, keys, licenses and all other settings.
        $report.savedAppAppearance = [ordered]@{
            layout=$settings.layout; panelScale=$settings.panelScale; panelMaximized=$settings.panelMaximized
        }
    } else { $diagnosticErrors.Add('App settings.json was not found in the current Windows account.') }
} catch { $diagnosticErrors.Add('Saved appearance: ' + $_.Exception.Message) }

try {
    $runtimeList = @()
    foreach ($registryRoot in @('HKLM:\SOFTWARE\Microsoft\EdgeUpdate\Clients',
        'HKLM:\SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients',
        'HKCU:\SOFTWARE\Microsoft\EdgeUpdate\Clients',
        'HKCU:\SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients')) {
        foreach ($key in @(Get-ChildItem $registryRoot -ErrorAction SilentlyContinue)) {
            $entry = Get-ItemProperty -LiteralPath $key.PSPath
            if ($entry.name -match 'WebView2') { $runtimeList += [ordered]@{ name=$entry.name; version=$entry.pv; registryRoot=$registryRoot } }
        }
    }
    $report.installedWebview2 = $runtimeList
    $processes = @(Get-CimInstance Win32_Process -Filter "Name='msedgewebview2.exe' OR Name='flair-bloom.exe'")
    $appIds = @($appProcesses | ForEach-Object { [int]$_.Id })
    $relatedIds = New-Object 'System.Collections.Generic.HashSet[int]'
    foreach ($idValue in $appIds) { [void]$relatedIds.Add($idValue) }
    do {
        $added = $false
        foreach ($proc in $processes) {
            if ($relatedIds.Contains([int]$proc.ParentProcessId) -and $relatedIds.Add([int]$proc.ProcessId)) { $added=$true }
        }
    } while ($added)
    $report.appWebviewProcesses = @($processes | Where-Object {
        $_.Name -eq 'msedgewebview2.exe' -and $relatedIds.Contains([int]$_.ProcessId)
    } | ForEach-Object {
        $version=$null
        try { if ($_.ExecutablePath) { $version=[Diagnostics.FileVersionInfo]::GetVersionInfo($_.ExecutablePath).FileVersion } } catch {}
        [ordered]@{ processId=$_.ProcessId; parentProcessId=$_.ParentProcessId; fileVersion=$version
            scaleSwitches=@([regex]::Matches([string]$_.CommandLine,
                '--(?:force-device-scale-factor|high-dpi-support|force-display-device-scale-factor)=[0-9.]+') | ForEach-Object { $_.Value }) }
    })
    $report.environmentScaleSwitches = @([regex]::Matches([string]$env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS,
        '--(?:force-device-scale-factor|high-dpi-support|force-display-device-scale-factor)=[0-9.]+') | ForEach-Object { $_.Value })
} catch { $diagnosticErrors.Add('WebView2 info: ' + $_.Exception.Message) }

try {
    $layers = @()
    foreach ($keyPath in @('HKCU:\Software\Microsoft\Windows NT\CurrentVersion\AppCompatFlags\Layers',
        'HKLM:\Software\Microsoft\Windows NT\CurrentVersion\AppCompatFlags\Layers')) {
        $values = Get-ItemProperty $keyPath -ErrorAction SilentlyContinue
        if ($values) {
            foreach ($property in $values.PSObject.Properties) {
                if ($property.Name -match '(?i)(^|[\\/])flair-bloom\.exe$') {
                    $layers += [ordered]@{ source=$keyPath; executable=(Hide-UserPath $property.Name); flags=$property.Value }
                }
            }
        }
    }
    $report.appCompatibilityFlags = $layers
} catch { $diagnosticErrors.Add('Compatibility flags: ' + $_.Exception.Message) }

$report.errors = @($diagnosticErrors.ToArray())

return $report
}

$stamp = Get-Date -Format "yyyyMMdd-HHmmss-fff"
if ([string]::IsNullOrWhiteSpace($OutFile)) {
    $desktop = [Environment]::GetFolderPath("Desktop")
    if ([string]::IsNullOrWhiteSpace($desktop)) {
        $desktop = $env:TEMP
    }
    $OutFile = Join-Path $desktop ("flair-environment-{0}.txt" -f $stamp)
}

$resolvedDriverDir = Resolve-DriverDir -InputDir $DriverDir
$systemDriver = Join-Path $env:SystemRoot "System32\drivers\ddhid63340.sys"
$infDir = Join-Path $env:SystemRoot "INF"
$driverStore = Join-Path $env:SystemRoot "System32\DriverStore\FileRepository"
$appLogDir = Join-Path $env:LOCALAPPDATA "fun.xwink.flairbloom\logs"

$expectedHashes = @{
    "ddc.exe" = "3C535B334F0897B8A0870BCB476C30EA79AFD09CFC18F8E00190BDC7C6C46785"
    "ddhid63340.inf" = "17FE3814F57E98DD2AF97F56B63502E474EA5E41CDA1A510FFE435EE6AD7A104"
    "ddhid63340.cat" = "6135C664711127A62E0988F6844521E345D78ACE9D3747A392400CE99BE96983"
    "ddhid63340.sys" = "FBE510402B3822C63E94752051B7D5895B67875F22EC48593DE19764A649F8B1"
    "ddhid.63340.dll" = "01E8DB6893CF79E9E7AA3AFBEE76BEA6C4220C4D1A2C63BC2E5B7C109FDB831E"
}

Add-Line "FlairBloom environment diagnostic report"
Add-Line ("generated_at: {0:o}" -f (Get-Date))
Add-Line ("script_path: {0}" -f $PSCommandPath)
Add-Line ("output_file: {0}" -f $OutFile)
Add-Line ("is_admin: {0}" -f (Test-IsAdmin))
Add-Line ("driver_dir_input: {0}" -f $DriverDir)
Add-Line ("driver_dir_resolved: {0}" -f $resolvedDriverDir)
Add-Line ("evidence_only: {0}" -f $EvidenceOnly.IsPresent)

Add-Line "collection_mode: read-only; no elevation, repair, driver load or injection"
Add-Line "Permission-denied and missing-data errors are diagnostic evidence."
Add-CommandOutput "Display, text scaling, windows and WebView2 (JSON)" {
    $displayJson = Get-DisplayEnvironment | ConvertTo-Json -Depth 10
    if ($env:USERPROFILE) { $displayJson = $displayJson.Replace($env:USERPROFILE, '%USERPROFILE%') }
    $displayJson
}

Add-CommandOutput "Interception 类设备编号（只读，不能单独判定驱动失效）" {
    foreach ($className in @('KeyboardClass', 'PointerClass')) {
        $classKey = [Microsoft.Win32.Registry]::LocalMachine.OpenSubKey(('HARDWARE\DEVICEMAP\' + $className))
        try {
            if ($classKey) {
                $indices = @($classKey.GetValueNames() | ForEach-Object {
                    if ($_ -match '(\d+)$') { [int]$Matches[1] }
                })
                "${className} 编号：[$($indices -join ', ')]"
            } else {
                "${className}：未找到类设备映射"
            }
        } finally {
            if ($classKey) { $classKey.Dispose() }
        }
    }
    '类设备编号和满槽只表示容量风险；实际可用性请在应用内诊断中查看可读槽号与驱动通道。'
}

Add-CommandOutput "OS and PowerShell" {
    $computer = Get-ComputerInfo -Property OsName, OsVersion, OsBuildNumber, OsArchitecture, WindowsProductName -ErrorAction SilentlyContinue
    [PSCustomObject]@{
        Is64BitOS = [Environment]::Is64BitOperatingSystem
        ProcessorArchitecture = $env:PROCESSOR_ARCHITECTURE
        PowerShellVersion = $PSVersionTable.PSVersion.ToString()
        OsName = $computer.OsName
        OsVersion = $computer.OsVersion
        OsBuildNumber = $computer.OsBuildNumber
        OsArchitecture = $computer.OsArchitecture
        WindowsProductName = $computer.WindowsProductName
    } | Format-List *
}

Add-CommandOutput "Security policy: HVCI / SAC / DeviceGuard" {
    "HVCI registry:"
    Get-ItemProperty -LiteralPath "HKLM:\SYSTEM\CurrentControlSet\Control\DeviceGuard\Scenarios\HypervisorEnforcedCodeIntegrity" -ErrorAction SilentlyContinue | Format-List *
    ""
    "Smart App Control registry:"
    Get-ItemProperty -LiteralPath "HKLM:\SYSTEM\CurrentControlSet\Control\CI\Policy" -ErrorAction SilentlyContinue | Format-List *
    ""
    "Win32_DeviceGuard:"
    Get-CimInstance -ClassName Win32_DeviceGuard -Namespace root\Microsoft\Windows\DeviceGuard -ErrorAction SilentlyContinue | Format-List *
}

Add-Section "Pending reboot markers"
Add-Line 'restart_note: Use Windows Start > Power > Restart. Shutdown/power-on may retain kernel and driver state when Fast Startup is enabled.'
Add-CommandOutput "Fast Startup configuration" {
    $powerKey = 'HKLM:\SYSTEM\CurrentControlSet\Control\Session Manager\Power'
    Get-ItemProperty -LiteralPath $powerKey -Name HiberbootEnabled -ErrorAction Stop |
        Select-Object HiberbootEnabled | Format-List
    'HiberbootEnabled is a configuration value, not proof that the last boot used Fast Startup.'
}
$pendingRename = @(Get-PendingRenameEntries)
Add-Line ("PendingFileRenameOperations_count: {0}" -f $pendingRename.Count)
if ($pendingRename.Count -gt 0) {
    Add-Line "PendingFileRenameOperations_first_80:"
    $pendingRename | Select-Object -First 80 | ForEach-Object { Add-Line ("  {0}" -f $_) }
}
Add-Line ("CBS_RebootPending: {0}" -f (Test-Path -LiteralPath "HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Component Based Servicing\RebootPending"))
Add-Line ("WindowsUpdate_RebootRequired: {0}" -f (Test-Path -LiteralPath "HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\WindowsUpdate\Auto Update\RebootRequired"))

if (-not $EvidenceOnly) {
    Add-Section "DDHID package files"
    if ([string]::IsNullOrWhiteSpace($resolvedDriverDir)) {
        Add-Line "No DriverDir was resolved. Re-run with -DriverDir pointing to the folder that contains ddc.exe."
    } else {
        Add-Line ("DriverDir: {0}" -f $resolvedDriverDir)
        Add-CommandOutput "DriverDir listing" {
            Get-ChildItem -LiteralPath $resolvedDriverDir -File -Force |
                Where-Object { $_.Name -in @("ddc.exe", "ddhid63340.inf", "ddhid63340.cat", "ddhid63340.sys", "ddhid.63340.dll") } |
                Sort-Object Name | Format-Table Mode, LastWriteTime, Length, Name -AutoSize
        }
        foreach ($name in @("ddc.exe", "ddhid63340.inf", "ddhid63340.cat", "ddhid63340.sys")) {
            Add-FileProbe -Path (Join-Path $resolvedDriverDir $name) -ExpectedSha256 $expectedHashes[$name]
        }

        $parentDll = Join-Path (Split-Path -Parent $resolvedDriverDir) "ddhid.63340.dll"
        $sameDirDll = Join-Path $resolvedDriverDir "ddhid.63340.dll"
        if (Test-Path -LiteralPath $parentDll) {
            Add-FileProbe -Path $parentDll -ExpectedSha256 $expectedHashes["ddhid.63340.dll"]
        } elseif (Test-Path -LiteralPath $sameDirDll) {
            Add-FileProbe -Path $sameDirDll -ExpectedSha256 $expectedHashes["ddhid.63340.dll"]
        } else {
            Add-Line "-- ddhid.63340.dll"
            Add-Line "not found next to driver folder or in driver folder"
        }
    }
}

Add-Section "Installed DDHID state"
Add-FileProbe -Path $systemDriver
Add-RegistryKey -Path "HKLM:\SYSTEM\CurrentControlSet\Services\ddhid63340"
Add-CommandOutput "sc.exe ddhid63340" {
    sc.exe query ddhid63340
    ""
    sc.exe qc ddhid63340
}
Add-CommandOutput "ControlSet service keys" {
    Get-ChildItem -LiteralPath "HKLM:\SYSTEM" -ErrorAction SilentlyContinue |
        Where-Object { $_.PSChildName -like "ControlSet*" } |
        ForEach-Object {
            $path = "Registry::{0}\Services\ddhid63340" -f $_.Name
            if (Test-Path -LiteralPath $path) {
                "FOUND: $path"
                Get-ItemProperty -LiteralPath $path | Format-List Type, Start, ErrorControl, ImagePath, DisplayName, DriverDelete, DeleteFlag
            }
        }
}

Add-CommandOutput "Windows INF entries containing ddhid63340" {
    $matches = @(Select-String -Path (Join-Path $infDir "oem*.inf") -Pattern "ddhid63340", "www.ddxoft.com" -SimpleMatch -ErrorAction SilentlyContinue)
    if ($matches.Count -eq 0) {
        "No matching OEM INF files."
    } else {
        $matches | Select-Object Path, LineNumber, Line | Format-Table -Wrap -AutoSize
        ""
        "Unique INF paths:"
        $matches | Select-Object -ExpandProperty Path -Unique
    }
}

Add-CommandOutput "DriverStore ddhid directories" {
    Get-ChildItem -LiteralPath $driverStore -Filter "ddhid*" -Force -ErrorAction SilentlyContinue | Format-Table Mode, LastWriteTime, Length, Name -AutoSize
}

Add-CommandOutput "PnP driver/device view" {
    "pnputil /enum-drivers /class HIDClass"
    pnputil.exe /enum-drivers /class HIDClass
    ""
    "pnputil /enum-devices /class HIDClass /drivers"
    pnputil.exe /enum-devices /class HIDClass /drivers
    ""
    "Get-PnpDevice DDHID filtered"
    Get-PnpDevice -Class HIDClass -ErrorAction SilentlyContinue |
        Where-Object {
            ($_.InstanceId -match "ddhid|63340") -or
            ($_.FriendlyName -match "ddhid|63340") -or
            ($_.Manufacturer -match "ddxoft|ddhid|63340") -or
            ($_.Service -match "ddhid|63340")
        } |
        Format-List *
}

Add-Section "SetupAPI log snippets"
Add-SetupApiMatches -Path (Join-Path $infDir "setupapi.dev.log")
Add-SetupApiMatches -Path (Join-Path $infDir "setupapi.app.log")

Add-Section "Relevant event logs"
Add-EventMatches -LogName "System" -Pattern "ddhid|63340|ddc\.exe" -MaxEvents 500
Add-EventMatches -LogName "Microsoft-Windows-CodeIntegrity/Operational" -Pattern "ddhid|63340|ddc\.exe|flair-bloom|flairbloom" -MaxEvents 500
Add-EventMatches -LogName "Microsoft-Windows-Windows Defender/Operational" -Pattern "ddhid|63340|ddc\.exe|flair-bloom|flairbloom" -MaxEvents 500

Add-Section "FlairBloom app logs"
Add-Line ("app_log_dir: {0}" -f $appLogDir)
if (-not (Test-Path -LiteralPath $appLogDir)) {
    Add-Line "app_log_dir_exists: false"
} else {
    Add-Line "app_log_dir_exists: true"
    Add-CommandOutput "Latest app log files" {
        Get-ChildItem -LiteralPath $appLogDir -Force |
            Sort-Object LastWriteTime -Descending |
            Select-Object -First 8 Mode, LastWriteTime, Length, Name |
            Format-Table -AutoSize
    }
    Add-CommandOutput "App log driver/display/error lines" {
        $files = @(Get-ChildItem -LiteralPath $appLogDir -File -ErrorAction SilentlyContinue | Sort-Object LastWriteTime -Descending | Select-Object -First 5)
        foreach ($file in $files) {
            "-- $($file.FullName)"
            Select-String -LiteralPath $file.FullName -Pattern "DD-HID", "ddhid", "ddc.exe", "Interception", "就绪", "driver", "驱动", "error", "failed", "失败", "WebView", "DPI", "缩放", "window", "ScaleFactor" -SimpleMatch -ErrorAction SilentlyContinue |
                Select-Object -Last 80 |
                ForEach-Object { "{0}:{1}: {2}" -f $_.Path, $_.LineNumber, $_.Line }
        }
    }
}

if (-not $EvidenceOnly) {
    Add-Section "Computed hints"
    $sysExists = Test-Path -LiteralPath $systemDriver
    $svcExists = Test-Path -LiteralPath "HKLM:\SYSTEM\CurrentControlSet\Services\ddhid63340"
    $infMatches = @(Select-String -Path (Join-Path $infDir "oem*.inf") -Pattern "ddhid63340", "www.ddxoft.com" -SimpleMatch -ErrorAction SilentlyContinue)
    $storeMatches = @(Get-ChildItem -LiteralPath $driverStore -Filter "ddhid*" -Force -ErrorAction SilentlyContinue)
    $pendingAny = ($pendingRename.Count -gt 0) -or
        (Test-Path -LiteralPath "HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Component Based Servicing\RebootPending") -or
        (Test-Path -LiteralPath "HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\WindowsUpdate\Auto Update\RebootRequired")

    Add-Line ("sys_exists: {0}" -f $sysExists)
    Add-Line ("service_exists: {0}" -f $svcExists)
    Add-Line ("matching_oem_inf_count: {0}" -f $infMatches.Count)
    Add-Line ("driverstore_ddhid_dir_count: {0}" -f $storeMatches.Count)
    Add-Line ("pending_reboot_marker_exists: {0}" -f $pendingAny)

    if ($sysExists -and $svcExists) {
        Add-Line "hint: DDHID looks installed at file+service level. If the app still fails, check DD_btn(0), admin privilege, app log, and CodeIntegrity events."
    } elseif (-not $sysExists -and $svcExists) {
        Add-Line "hint: Service key exists but ddhid63340.sys is missing. This is a likely half-uninstalled/pending-reboot state; reboot first, then repair residue if it remains."
    } elseif ($sysExists -and -not $svcExists) {
        Add-Line "hint: ddhid63340.sys exists but service key is missing. This is an inconsistent install state; reboot and clean/reinstall via PnP."
    } else {
        Add-Line "hint: DDHID is not installed at file+service level. If install fails, SetupAPI snippets and CodeIntegrity/Defender events are the key evidence."
    }
}

try {
    $outDir = Split-Path -Parent $OutFile
    if (-not [string]::IsNullOrWhiteSpace($outDir)) {
        New-Item -ItemType Directory -Path $outDir -Force | Out-Null
    }
    $script:Lines | Set-Content -LiteralPath $OutFile -Encoding UTF8
    Write-Host "FlairBloom environment diagnostic report written to:"
    Write-Host $OutFile
} catch {
    Write-Error ("Failed to write report: {0}" -f $_.Exception.Message)
    exit 1
}
