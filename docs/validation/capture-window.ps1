Add-Type -AssemblyName System.Drawing
Add-Type @"
using System;
using System.Runtime.InteropServices;
public class ApertureCapture {
 [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left, Top, Right, Bottom; }
 [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
 [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
 [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr h, IntPtr dc, uint flags);
}
"@
[ApertureCapture]::SetProcessDPIAware() | Out-Null
$appWindow = Get-Process aperture | Where-Object MainWindowTitle -eq 'Aperture' | Select-Object -First 1
if (!$appWindow) { throw 'Aperture window not found' }
$ws = New-Object -ComObject WScript.Shell
if ($ws.AppActivate($appWindow.Id)) { $ws.SendKeys('^{HOME}') }
Start-Sleep -Milliseconds 300
$r = New-Object ApertureCapture+RECT
[ApertureCapture]::GetWindowRect($appWindow.MainWindowHandle, [ref]$r) | Out-Null
$bitmap = New-Object System.Drawing.Bitmap(($r.Right-$r.Left),($r.Bottom-$r.Top))
$graphics = [System.Drawing.Graphics]::FromImage($bitmap)
$dc = $graphics.GetHdc()
try { if (![ApertureCapture]::PrintWindow($appWindow.MainWindowHandle,$dc,2)) { throw 'App capture failed' } }
finally { $graphics.ReleaseHdc($dc) }
$bitmap.Save((Join-Path (Get-Location) 'docs/validation/windows-observer.png'))
$graphics.Dispose(); $bitmap.Dispose()


