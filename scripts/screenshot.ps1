# Photograph the popup panel.
#
# Every picture in docs/screenshots is taken by this script, so that "how was this made"
# has an answer that is not a memory. It launches the tray with `--demo` (synthetic
# numbers, no advisory lock, nothing written), waits for the panel, captures exactly the
# window rectangle, and stops the process again.
#
# The DPI trick, because it is the interesting part: Windows will not change its display
# scaling for one application, so `--scale 1.5` passes
# `--force-device-scale-factor=1.5` to WebView2 through
# WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS and multiplies the window to match. The panel is
# then genuinely rendered at 150 %: the same CSS, the same layout code, one and a half
# times the pixels. It is not an upscaled screenshot.
#
#   powershell -File scripts/screenshot.ps1                      # every documented shot
#   powershell -File scripts/screenshot.ps1 -Scale 2 -Theme nazar -Mode dark -Out x.png
#
# Requires the debug or release binary to have been built already.

[CmdletBinding()]
param(
    [double] $Scale = 0,
    [ValidateSet('nazar', 'graphite')] [string] $Theme = 'nazar',
    [ValidateSet('light', 'dark')] [string] $Mode = 'dark',
    [ValidateSet('on', 'off')] [string] $Hint = 'off',
    [ValidateSet('on', 'off')] [string] $Offer = 'off',
    [ValidateSet('quota', 'settings')] [string] $View = 'quota',
    [string] $Locale = 'en',
    [string] $Out = '',
    # Where the cursor is put before the panel opens: the panel appears just above it, the
    # way it does when the bead is clicked in the tray.
    [int] $CursorX = -1,
    [int] $CursorY = -1
)

$ErrorActionPreference = 'Stop'
$repo = Split-Path -Parent $PSScriptRoot
$exe = Join-Path $repo 'target\debug\nazar-tray.exe'
if (-not (Test-Path $exe)) { $exe = Join-Path $repo 'target\release\nazar-tray.exe' }
if (-not (Test-Path $exe)) { throw 'nazar-tray.exe is not built: run `cargo build -p nazar-tray` first' }

Add-Type -AssemblyName System.Drawing
Add-Type -AssemblyName System.Windows.Forms
if (-not ('NazarShot' -as [type])) {
    Add-Type @'
using System;
using System.Runtime.InteropServices;
using System.Text;

public class NazarShot {
    public delegate bool EnumProc(IntPtr window, IntPtr extra);

    [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc callback, IntPtr extra);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr window);
    [DllImport("user32.dll")] public static extern int GetWindowTextLength(IntPtr window);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern int GetWindowText(IntPtr window, StringBuilder text, int count);
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr window, out uint processId);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr window, out RECT rect);
    [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
    [DllImport("dwmapi.dll")] public static extern int DwmGetWindowAttribute(IntPtr window, int attribute, out RECT value, int size);

    [StructLayout(LayoutKind.Sequential)]
    public struct RECT { public int Left, Top, Right, Bottom; }

    /// GetWindowRect includes the invisible frame Windows keeps around a window — eight
    /// pixels of desktop on three sides, which in a screenshot look like a panel that does
    /// not fit its own window. DWMWA_EXTENDED_FRAME_BOUNDS (9) is what is actually drawn.
    public static RECT VisibleBounds(IntPtr window) {
        RECT rect;
        if (DwmGetWindowAttribute(window, 9, out rect, Marshal.SizeOf(typeof(RECT))) == 0
            && rect.Right > rect.Left && rect.Bottom > rect.Top) {
            return rect;
        }
        GetWindowRect(window, out rect);
        return rect;
    }

    public static IntPtr FindVisibleWindow(uint processId) {
        IntPtr found = IntPtr.Zero;
        EnumWindows(delegate(IntPtr window, IntPtr extra) {
            uint owner;
            GetWindowThreadProcessId(window, out owner);
            if (owner != processId || !IsWindowVisible(window)) return true;
            RECT rect;
            if (!GetWindowRect(window, out rect)) return true;
            // The panel is the only window this process shows, but a webview keeps
            // invisible helper windows of its own; take the one with a real size.
            if (rect.Right - rect.Left < 100 || rect.Bottom - rect.Top < 100) return true;
            found = window;
            return false;
        }, IntPtr.Zero);
        return found;
    }
}
'@
}

function Capture-Panel {
    param([double] $Scale, [string] $Theme, [string] $Mode, [string] $Hint, [string] $Offer, [string] $View, [string] $Locale, [string] $Out, [int] $CursorX, [int] $CursorY)

    $arguments = @('--demo', '--theme', $Theme, '--mode', $Mode, '--hint', $Hint, '--offer', $Offer, '--locale', $Locale)
    if ($View -eq 'settings') { $arguments += @('--view', 'settings') }
    if ($Scale -gt 0) { $arguments += @('--scale', $Scale.ToString([System.Globalization.CultureInfo]::InvariantCulture)) }

    $screen = [System.Windows.Forms.Screen]::PrimaryScreen.Bounds
    if ($CursorX -lt 0) { $CursorX = $screen.Width - 160 }
    if ($CursorY -lt 0) { $CursorY = $screen.Height - 60 }
    [void][NazarShot]::SetCursorPos($CursorX, $CursorY)

    $process = Start-Process -FilePath $exe -ArgumentList $arguments -PassThru
    try {
        $window = [IntPtr]::Zero
        # The panel opens as soon as the webview has drawn; give it a moment to settle so
        # the countdown and the bars are painted rather than half-painted.
        for ($try = 0; $try -lt 60 -and $window -eq [IntPtr]::Zero; $try++) {
            Start-Sleep -Milliseconds 250
            $window = [NazarShot]::FindVisibleWindow($process.Id)
        }
        if ($window -eq [IntPtr]::Zero) { throw "the panel never appeared ($Out)" }
        Start-Sleep -Milliseconds 900

        $rect = [NazarShot]::VisibleBounds($window)
        $width = $rect.Right - $rect.Left
        $height = $rect.Bottom - $rect.Top

        $bitmap = New-Object System.Drawing.Bitmap $width, $height
        $graphics = [System.Drawing.Graphics]::FromImage($bitmap)
        $graphics.CopyFromScreen($rect.Left, $rect.Top, 0, 0, $bitmap.Size)
        $graphics.Dispose()

        $target = Join-Path $repo $Out
        New-Item -ItemType Directory -Force -Path (Split-Path -Parent $target) | Out-Null
        $bitmap.Save($target, [System.Drawing.Imaging.ImageFormat]::Png)
        $bitmap.Dispose()
        Write-Output ("{0}  {1}x{2}" -f $Out, $width, $height)
    }
    finally {
        # Stopped the blunt way, which is safe here for one reason: `--demo` never took the
        # advisory lock, so there is nothing to leave behind. A real tray is quit through
        # the tray menu, which releases it.
        Stop-Process -Id $process.Id -Force -ErrorAction SilentlyContinue
        Start-Sleep -Milliseconds 400
    }
}

if ($Out) {
    Capture-Panel -Scale $Scale -Theme $Theme -Mode $Mode -Hint $Hint -Offer $Offer -View $View -Locale $Locale -Out $Out -CursorX $CursorX -CursorY $CursorY
    return
}

# The documented set. Three scales of the default theme in the dark, plus the light mode
# and the graphite theme at 100 %, plus the first-run hint, which is a state a machine can
# only be in once.
$shots = @(
    @{ Scale = 1.0; Theme = 'nazar'; Mode = 'dark'; Hint = 'off'; Out = 'docs/screenshots/wp4-100-nazar-dark.png' },
    @{ Scale = 1.5; Theme = 'nazar'; Mode = 'dark'; Hint = 'off'; Out = 'docs/screenshots/wp4-150-nazar-dark.png' },
    @{ Scale = 2.0; Theme = 'nazar'; Mode = 'dark'; Hint = 'off'; Out = 'docs/screenshots/wp4-200-nazar-dark.png' },
    @{ Scale = 1.0; Theme = 'nazar'; Mode = 'light'; Hint = 'off'; Out = 'docs/screenshots/wp4-100-nazar-light.png' },
    @{ Scale = 1.0; Theme = 'graphite'; Mode = 'dark'; Hint = 'off'; Out = 'docs/screenshots/wp4-100-graphite-dark.png' },
    @{ Scale = 1.0; Theme = 'graphite'; Mode = 'light'; Hint = 'off'; Out = 'docs/screenshots/wp4-100-graphite-light.png' },
    @{ Scale = 1.0; Theme = 'nazar'; Mode = 'dark'; Hint = 'on'; Out = 'docs/screenshots/wp4-100-first-run.png' },
    @{ Scale = 1.0; Theme = 'nazar'; Mode = 'dark'; Hint = 'off'; View = 'settings'; Out = 'docs/screenshots/wp5-100-settings.png' },
    @{ Scale = 1.0; Theme = 'nazar'; Mode = 'dark'; Hint = 'off'; Offer = 'on'; Out = 'docs/screenshots/wp5-100-offer.png' }
)

foreach ($shot in $shots) {
    $shotView = if ($shot.ContainsKey('View')) { $shot.View } else { 'quota' }
    $shotOffer = if ($shot.ContainsKey('Offer')) { $shot.Offer } else { 'off' }
    Capture-Panel -Scale $shot.Scale -Theme $shot.Theme -Mode $shot.Mode -Hint $shot.Hint -Offer $shotOffer -View $shotView -Locale $Locale -Out $shot.Out -CursorX $CursorX -CursorY $CursorY
}
