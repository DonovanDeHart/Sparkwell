<#
  Sparkwell Windows smoke test (clean install -> launch -> behave -> uninstall).

  Run on a Windows machine with an interactive desktop, after `npm run app:build`:
    pwsh -File scripts/windows-smoke.ps1

  Verifies, against the real packaged app:
    1. The NSIS installer installs silently (per-user, no admin).
    2. The app launches, creates and seeds the local library.
    3. The window docks to the right edge of the monitor work area (taskbar excluded).
    4. A second launch does not create a second instance.
    5. The global activation hotkey (Ctrl+Alt+Space) hides and re-shows the panel.
    6. The core loop by keyboard: type a goal, Enter, Ctrl+Enter -> the complete
       Spark is on the Windows clipboard and the unpinned panel collapses.
    7. The library persists across a restart (retrieved and copied again).
    8. Uninstalling never deletes the user's library.
  A screenshot of the docked panel is written to smoke-artifacts/.
#>
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$root = Split-Path -Parent $PSScriptRoot
$artifacts = Join-Path $root 'smoke-artifacts'
New-Item -ItemType Directory -Force -Path $artifacts | Out-Null

Add-Type -AssemblyName System.Drawing
Add-Type -AssemblyName System.Windows.Forms
Add-Type @"
using System;
using System.Runtime.InteropServices;
public static class Win {
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left, Top, Right, Bottom; }
  [StructLayout(LayoutKind.Sequential)] public struct MONITORINFO { public int cbSize; public RECT rcMonitor; public RECT rcWork; public int dwFlags; }
  [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern IntPtr FindWindow(string cls, string title);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern IntPtr MonitorFromWindow(IntPtr h, uint flags);
  [DllImport("user32.dll")] public static extern bool GetMonitorInfo(IntPtr m, ref MONITORINFO mi);
  [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
  [DllImport("user32.dll")] public static extern void keybd_event(byte vk, byte scan, uint flags, UIntPtr extra);
  [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
  public delegate bool EnumProc(IntPtr h, IntPtr l);
  [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc cb, IntPtr l);
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
  [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern int GetWindowText(IntPtr h, System.Text.StringBuilder s, int n);
  [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern int GetClassName(IntPtr h, System.Text.StringBuilder s, int n);
  // Top-level windows belonging to any of the given process ids.
  public static System.Collections.Generic.List<IntPtr> WindowsOf(uint[] pids) {
    var found = new System.Collections.Generic.List<IntPtr>();
    EnumWindows((h, l) => {
      uint pid; GetWindowThreadProcessId(h, out pid);
      if (Array.IndexOf(pids, pid) >= 0) found.Add(h);
      return true;
    }, IntPtr.Zero);
    return found;
  }
  public static string Title(IntPtr h) { var s = new System.Text.StringBuilder(256); GetWindowText(h, s, 256); return s.ToString(); }
  public static string Class(IntPtr h) { var s = new System.Text.StringBuilder(256); GetClassName(h, s, 256); return s.ToString(); }
}
"@
[Win]::SetProcessDPIAware() | Out-Null

$failures = New-Object System.Collections.Generic.List[string]
function Check([bool]$ok, [string]$what) {
  if ($ok) { Write-Host "PASS  $what" } else { Write-Host "FAIL  $what"; $failures.Add($what) }
}

function Sparkwell-Pids { [uint32[]]@(Get-Process -Name 'sparkwell' -ErrorAction SilentlyContinue | ForEach-Object { $_.Id }) }

# The sidebar: the process's top-level window titled "Sparkwell". Helper
# windows (tray, event loop) may share the title but are never visible, so
# prefer a visible one. Returns IntPtr.Zero when none exists.
function Find-Sparkwell {
  # @() + cast: a function returning an empty or single-item array unrolls it.
  $pids = [uint32[]]@(Sparkwell-Pids)
  if ($pids.Count -eq 0) { return [IntPtr]::Zero }
  $candidates = @([Win]::WindowsOf($pids) | Where-Object { [Win]::Title($_) -eq 'Sparkwell' })
  $visible = @($candidates | Where-Object { [Win]::IsWindowVisible($_) })
  if ($visible.Count -gt 0) { return $visible[0] }
  if ($candidates.Count -gt 0) { return $candidates[0] }
  return [IntPtr]::Zero
}

function Dump-Windows([string]$label) {
  Write-Host "--- top-level windows of Sparkwell ($label)"
  foreach ($h in [Win]::WindowsOf([uint32[]]@(Sparkwell-Pids))) {
    $r = New-Object Win+RECT
    [Win]::GetWindowRect($h, [ref]$r) | Out-Null
    Write-Host ("  0x{0:X} class='{1}' title='{2}' visible={3} rect=({4},{5})-({6},{7})" -f $h.ToInt64(), [Win]::Class($h), [Win]::Title($h), [Win]::IsWindowVisible($h), $r.Left, $r.Top, $r.Right, $r.Bottom)
  }
}

function Save-FullScreenshot([string]$name) {
  $b = [System.Windows.Forms.SystemInformation]::VirtualScreen
  $r = New-Object Win+RECT
  $r.Left = $b.Left; $r.Top = $b.Top; $r.Right = $b.Right; $r.Bottom = $b.Bottom
  Save-Screenshot $name $r
}

function Wait-Until([scriptblock]$cond, [int]$seconds = 20) {
  $deadline = (Get-Date).AddSeconds($seconds)
  while ((Get-Date) -lt $deadline) {
    if (& $cond) { return $true }
    Start-Sleep -Milliseconds 250
  }
  return $false
}

function Press-Hotkey {
  # Ctrl+Alt+Space via injected input; RegisterHotKey responds to it like a real keypress.
  $VK_CONTROL = 0x11; $VK_MENU = 0x12; $VK_SPACE = 0x20; $UP = 0x2
  [Win]::keybd_event($VK_CONTROL, 0, 0, [UIntPtr]::Zero)
  [Win]::keybd_event($VK_MENU, 0, 0, [UIntPtr]::Zero)
  [Win]::keybd_event($VK_SPACE, 0, 0, [UIntPtr]::Zero)
  Start-Sleep -Milliseconds 60
  [Win]::keybd_event($VK_SPACE, 0, $UP, [UIntPtr]::Zero)
  [Win]::keybd_event($VK_MENU, 0, $UP, [UIntPtr]::Zero)
  [Win]::keybd_event($VK_CONTROL, 0, $UP, [UIntPtr]::Zero)
}

# Types a goal into the focused panel, retrieves, and copies the Best Match.
# Returns the clipboard text afterwards.
function Copy-BestMatch([string]$goal) {
  Set-Clipboard -Value 'sparkwell-smoke-sentinel'
  [System.Windows.Forms.SendKeys]::SendWait($goal)
  [System.Windows.Forms.SendKeys]::SendWait('{ENTER}')
  Start-Sleep -Milliseconds 1200
  [System.Windows.Forms.SendKeys]::SendWait('^{ENTER}')
  Start-Sleep -Milliseconds 800
  return (Get-Clipboard -Raw)
}

function Save-Screenshot([string]$name, $rect) {
  $w = [Math]::Max(1, $rect.Right - $rect.Left); $h = [Math]::Max(1, $rect.Bottom - $rect.Top)
  $bmp = New-Object System.Drawing.Bitmap $w, $h
  $g = [System.Drawing.Graphics]::FromImage($bmp)
  $g.CopyFromScreen($rect.Left, $rect.Top, 0, 0, $bmp.Size)
  $path = Join-Path $artifacts $name
  $bmp.Save($path, [System.Drawing.Imaging.ImageFormat]::Png)
  $g.Dispose(); $bmp.Dispose()
  Write-Host "screenshot: $path"
}

# ------------------------------------------------------------------ install
$installer = Get-ChildItem (Join-Path $root 'src-tauri/target/release/bundle/nsis') -Filter '*-setup.exe' | Select-Object -First 1
if (-not $installer) { throw 'NSIS installer not found; run `npm run app:build` first.' }
Write-Host "installer: $($installer.FullName)"
Start-Process -FilePath $installer.FullName -ArgumentList '/S' -Wait
$installDir = Join-Path $env:LOCALAPPDATA 'Sparkwell'
$exe = Get-ChildItem $installDir -Filter 'sparkwell*.exe' | Where-Object { $_.Name -notlike 'uninstall*' } | Select-Object -First 1
Check ($null -ne $exe) "installer placed the app in $installDir"
if (-not $exe) { exit 1 }

$library = Join-Path $env:LOCALAPPDATA 'Sparkwell\Library\sparkwell.db'
$log = Join-Path $env:LOCALAPPDATA 'com.sparkwell.app\logs\Sparkwell.log'

# ------------------------------------------------------------------ launch
$proc = Start-Process -FilePath $exe.FullName -PassThru
$appeared = Wait-Until { $h = Find-Sparkwell; $h -ne [IntPtr]::Zero -and [Win]::IsWindowVisible($h) } 30
Check $appeared 'sidebar window appears on launch'
if (-not $appeared) { Dump-Windows 'after launch'; Save-FullScreenshot 'launch-fullscreen.png' }
Check (-not $proc.HasExited) 'process keeps running'
Check (Wait-Until { Test-Path $library } 10) "library created at $library"
if (Test-Path $library) { Check ((Get-Item $library).Length -gt 0) 'library file is non-empty' }

if ($appeared) {
  Start-Sleep -Milliseconds 800  # let the entrance animation settle
  $h = Find-Sparkwell
  $r = New-Object Win+RECT
  [Win]::GetWindowRect($h, [ref]$r) | Out-Null
  $mi = New-Object Win+MONITORINFO
  $mi.cbSize = [Runtime.InteropServices.Marshal]::SizeOf($mi)
  [Win]::GetMonitorInfo([Win]::MonitorFromWindow($h, 2), [ref]$mi) | Out-Null
  $work = $mi.rcWork
  Write-Host ("window  L{0} T{1} R{2} B{3}" -f $r.Left, $r.Top, $r.Right, $r.Bottom)
  Write-Host ("work    L{0} T{1} R{2} B{3}" -f $work.Left, $work.Top, $work.Right, $work.Bottom)
  Check ([Math]::Abs($r.Right - $work.Right) -le 1) 'docked to the right edge of the work area'
  Check ([Math]::Abs($r.Top - $work.Top) -le 1 -and [Math]::Abs($r.Bottom - $work.Bottom) -le 1) 'spans the work area height (taskbar respected)'
  $width = $r.Right - $r.Left
  Check ($width -ge 380 -and $width -le 1100) "compact width ($width px)"
  Save-Screenshot 'sparkwell-docked.png' $r
}

# ------------------------------------------------------------------ single instance
$second = Start-Process -FilePath $exe.FullName -PassThru
$exited = Wait-Until { $second.HasExited } 10
Check $exited 'second launch exits (single instance)'
$count = @(Get-Process | Where-Object { $_.Path -eq $exe.FullName }).Count
Check ($count -eq 1) "exactly one Sparkwell process ($count)"

# ------------------------------------------------------------------ global hotkey
if ($appeared) {
  # Visible+focused -> hide. If focus was elsewhere the first press focuses, the second hides.
  Press-Hotkey
  $hidden = Wait-Until { -not [Win]::IsWindowVisible((Find-Sparkwell)) } 3
  if (-not $hidden) { Press-Hotkey; $hidden = Wait-Until { -not [Win]::IsWindowVisible((Find-Sparkwell)) } 3 }
  Check $hidden 'activation hotkey hides the panel'
  Press-Hotkey
  Check (Wait-Until { [Win]::IsWindowVisible((Find-Sparkwell)) } 3) 'activation hotkey shows the panel again'
  $focused = Wait-Until { (Find-Sparkwell) -eq [Win]::GetForegroundWindow() } 3
  Check $focused 'shown panel takes keyboard focus'

  # ---------------------------------------------------------------- core loop
  if ($focused) {
    $clip = Copy-BestMatch 'I need AI to help me build an MCP server'
    Check ($clip -like '*Model Context Protocol*' -and $clip -like '*What the server should do:*') 'Copy Spark puts the complete Best Match Spark on the clipboard'
    Check (Wait-Until { -not [Win]::IsWindowVisible((Find-Sparkwell)) } 3) 'unpinned panel collapses after copying'
  }
}

# ------------------------------------------------------------------ restart persistence
Stop-Process -Id $proc.Id -Force
Start-Sleep -Seconds 1
$proc = Start-Process -FilePath $exe.FullName -PassThru
$relaunched = Wait-Until { $h = Find-Sparkwell; $h -ne [IntPtr]::Zero -and [Win]::IsWindowVisible($h) } 30
Check $relaunched 'relaunches cleanly'
if ($relaunched -and (Wait-Until { (Find-Sparkwell) -eq [Win]::GetForegroundWindow() } 5)) {
  Start-Sleep -Milliseconds 800
  $clip = Copy-BestMatch 'research a topic deeply with sources'
  Check ($clip -like '*meticulous research analyst*') 'library persists across restart (retrieved and copied again)'
} else {
  Check $false 'relaunched panel takes focus for the persistence check'
}
Stop-Process -Id $proc.Id -Force
Start-Sleep -Seconds 1

# ------------------------------------------------------------------ uninstall keeps the library
$uninstaller = Join-Path $installDir 'uninstall.exe'
if (Test-Path $uninstaller) {
  Start-Process -FilePath $uninstaller -ArgumentList '/S' -Wait
  Start-Sleep -Seconds 3
  Check (-not (Test-Path $exe.FullName)) 'uninstaller removes the app'
  Check (Test-Path $library) 'uninstall never deletes the Spark library'
} else {
  Check $false "uninstaller present at $uninstaller"
}

if (Test-Path $log) {
  Write-Host '--- app log ---'
  Get-Content $log -Tail 40
  Copy-Item $log (Join-Path $artifacts 'Sparkwell.log')
}

if ($failures.Count -gt 0) {
  Write-Host "`n$($failures.Count) smoke check(s) failed:"
  $failures | ForEach-Object { Write-Host " - $_" }
  exit 1
}
Write-Host "`nAll Windows smoke checks passed."
