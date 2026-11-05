<#
.SYNOPSIS
    Saves a PNG of a top-level window, found by its title.

.DESCRIPTION
    Finds the window titled exactly -Title: first through the process whose main window
    title matches, then through user32 FindWindow, because .NET can pick a helper window as
    a process's main window (winit's "Winit Thread Event Target" is one). The script
    declares itself DPI aware first, so every rectangle is in physical pixels.

    By default it restores the window if minimized, calls SetForegroundWindow, reads the
    rectangle with GetWindowRect and copies that screen region. Windows may refuse to move
    the foreground while someone is using another window. In that case, and always with
    -Background, it asks the window to paint itself instead (PrintWindow with
    PW_RENDERFULLCONTENT). That works while the window is covered or off-screen, and
    neither moves the focus nor changes what is on screen.

    Works under Windows PowerShell 5.1 and PowerShell 7.

.EXAMPLE
    .\scripts\screenshot-window.ps1 -Title Aria -Out C:\temp\aria.png

.EXAMPLE
    .\scripts\screenshot-window.ps1 -Title Aria -Out C:\temp\aria.png -Background
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$Title,
    [Parameter(Mandatory = $true)][string]$Out,
    # Capture without bringing the window forward.
    [switch]$Background,
    # Milliseconds to wait after bringing the window forward, so it can repaint.
    [int]$SettleMs = 400
)

$ErrorActionPreference = 'Stop'

# Prints `message` to stderr and exits with code 1.
function Fail([string]$message) {
    [Console]::Error.WriteLine("screenshot-window: $message")
    exit 1
}

Add-Type -AssemblyName System.Drawing
# A type cannot be added twice to one session, so a second run in it reuses the first.
if (-not ('AriaShotUser32' -as [type])) {
    Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;

public static class AriaShotUser32 {
    [StructLayout(LayoutKind.Sequential)]
    public struct RECT { public int Left; public int Top; public int Right; public int Bottom; }

    [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    public static extern IntPtr FindWindowW(string className, string windowName);
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr hWnd);
    [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr hWnd, int nCmdShow);
    [DllImport("user32.dll")] public static extern bool IsIconic(IntPtr hWnd);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr hWnd, out RECT rect);
    [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr hWnd, IntPtr hdc, uint flags);
}
'@
}

# Before any coordinates are read, so they are physical pixels rather than scaled ones.
[void][AriaShotUser32]::SetProcessDPIAware()

$process = Get-Process |
    Where-Object { $_.MainWindowHandle -ne [IntPtr]::Zero -and $_.MainWindowTitle -eq $Title } |
    Select-Object -First 1
if ($process) {
    $hwnd = $process.MainWindowHandle
} else {
    # `[NullString]::Value`: a plain $null would reach the API as "" and match nothing.
    $hwnd = [AriaShotUser32]::FindWindowW([NullString]::Value, $Title)
}
if ($hwnd -eq [IntPtr]::Zero) {
    Fail "No window titled '$Title' was found."
}

$useScreen = -not $Background
if ($useScreen) {
    if ([AriaShotUser32]::IsIconic($hwnd)) {
        # SW_RESTORE: a minimized window has no useful rectangle.
        [void][AriaShotUser32]::ShowWindow($hwnd, 9)
    }
    [void][AriaShotUser32]::SetForegroundWindow($hwnd)
    Start-Sleep -Milliseconds $SettleMs
    if ([AriaShotUser32]::GetForegroundWindow() -ne $hwnd) {
        [Console]::Error.WriteLine(
            "screenshot-window: Windows kept another window in front; asking '$Title' to paint itself instead.")
        $useScreen = $false
    }
} elseif ([AriaShotUser32]::IsIconic($hwnd)) {
    Fail "The window titled '$Title' is minimized; -Background does not restore it."
}

$rect = New-Object AriaShotUser32+RECT
if (-not [AriaShotUser32]::GetWindowRect($hwnd, [ref]$rect)) {
    Fail "Could not read the rectangle of the window titled '$Title'."
}
$width = $rect.Right - $rect.Left
$height = $rect.Bottom - $rect.Top
if ($width -le 0 -or $height -le 0) {
    Fail "The window titled '$Title' has an empty rectangle ($width x $height)."
}

# Relative to the PowerShell location, which .NET's current directory need not match.
$outPath = $ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($Out)
$outDir = [System.IO.Path]::GetDirectoryName($outPath)
if ($outDir -and -not (Test-Path -LiteralPath $outDir)) {
    New-Item -ItemType Directory -Path $outDir -Force | Out-Null
}

$bitmap = New-Object System.Drawing.Bitmap $width, $height
try {
    $graphics = [System.Drawing.Graphics]::FromImage($bitmap)
    try {
        if ($useScreen) {
            $graphics.CopyFromScreen($rect.Left, $rect.Top, 0, 0, $bitmap.Size)
            $how = 'screen'
        } else {
            $hdc = $graphics.GetHdc()
            try {
                # PW_RENDERFULLCONTENT (2): also captures GPU-drawn content such as OpenGL.
                $painted = [AriaShotUser32]::PrintWindow($hwnd, $hdc, 2)
            } finally {
                $graphics.ReleaseHdc($hdc)
            }
            if (-not $painted) {
                Fail "The window titled '$Title' did not paint itself (PrintWindow failed)."
            }
            $how = 'PrintWindow'
        }
    } finally {
        $graphics.Dispose()
    }
    $bitmap.Save($outPath, [System.Drawing.Imaging.ImageFormat]::Png)
} finally {
    $bitmap.Dispose()
}

Write-Output "Saved $width x $height from ($($rect.Left), $($rect.Top)) via $how to $outPath"
