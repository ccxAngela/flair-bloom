# Windows PowerShell 5.1+. Reads display/app state; writes only the JSON report.
# Keep the affected app window visible while running. No elevation required.
[CmdletBinding()]
param([string]$OutFile = '')

$ErrorActionPreference = 'Stop'
$diagnosticErrors = New-Object 'System.Collections.Generic.List[string]'
$report = [ordered]@{
    reportVersion = 1
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
    $textScale = Get-ItemProperty 'HKCU:\Software\Microsoft\Accessibility' -ErrorAction SilentlyContinue
    $report.accessibilityTextScalePercent = $textScale.TextScaleFactor
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
$json = Hide-UserPath ($report | ConvertTo-Json -Depth 10)
if (-not $OutFile) {
    $OutFile = Join-Path ([Environment]::GetFolderPath('Desktop')) ('flair-display-' + (Get-Date -Format 'yyyyMMdd-HHmmss-fff') + '.json')
}
[IO.File]::WriteAllText($OutFile, $json, (New-Object Text.UTF8Encoding($false)))
Write-Output ('Report saved: ' + $OutFile)
