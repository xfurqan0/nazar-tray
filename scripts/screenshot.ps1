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
#   powershell -File scripts/screenshot.ps1 -View settings -Scroll 6 -Out x.png
#   powershell -File scripts/screenshot.ps1 -View usage -UsageTab all -Out x.png
#
# The usage view draws a synthetic history of its own since 0.2.0 — five weeks, four model
# ids, both providers — so a picture of it carries nobody's real model use. See
# crates/nazar-tray/src/demo.rs.
#
# Requires the debug or release binary to have been built already.

[CmdletBinding()]
param(
    [double] $Scale = 0,
    [ValidateSet('nazar', 'graphite')] [string] $Theme = 'nazar',
    [ValidateSet('light', 'dark')] [string] $Mode = 'dark',
    [ValidateSet('on', 'off')] [string] $Hint = 'off',
    [ValidateSet('on', 'off')] [string] $Offer = 'off',
    [ValidateSet('quota', 'settings', 'usage')] [string] $View = 'quota',
    # Which tab `-View usage` lands on. 'day' is the Week tab with its newest day opened,
    # which is the one state of that view no tab name reaches.
    [ValidateSet('week', 'weeks', 'all', 'models', 'day')] [string] $UsageTab = 'week',
    [int] $Scroll = 0,
    # How long to let the panel settle before the shutter. The usage view asks Rust for its
    # numbers after the window is already up, so it needs longer than the quota view does.
    [int] $Settle = 0,
    [string] $Locale = 'en',
    [string] $Out = '',
    # Where the cursor is put before the panel opens: the panel appears just above it, the
    # way it does when the bead is clicked in the tray.
    [int] $CursorX = -1,
    [int] $CursorY = -1
)

$ErrorActionPreference = 'Stop'

# A tray that is already running is the one thing that stops this script dead, and the symptom
# says nothing about the cause: `--scale` passes WebView2 a browser argument, two processes
# cannot share one WebView2 user-data folder with different arguments, so the panel is created
# and never shown and every scaled shot waits out its timeout. Quit the running tray from its
# own menu first — which also releases the advisory lock, where killing it would not.
if (Get-Process -Name nazar-tray -ErrorAction SilentlyContinue) {
    throw 'a nazar-tray is already running: quit it from its tray menu first, or the scaled shots will never open a panel'
}

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
    [DllImport("user32.dll")] public static extern void mouse_event(uint flags, int dx, int dy, int data, IntPtr extra);
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr window);

    /// One notch of the wheel, downwards, wherever the pointer is. The settings page is
    /// taller than the panel window it lives in, so a section near its end cannot be
    /// photographed without scrolling to it.
    public static void WheelDown(int notches) {
        for (int i = 0; i < notches; i++) {
            mouse_event(0x0800, 0, 0, -120, IntPtr.Zero);
            System.Threading.Thread.Sleep(60);
        }
    }
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
    param([double] $Scale, [string] $Theme, [string] $Mode, [string] $Hint, [string] $Offer, [string] $View, [string] $UsageTab, [string] $Locale, [string] $Out, [int] $CursorX, [int] $CursorY, [int] $Scroll = 0, [int] $Settle = 0)

    $arguments = @('--demo', '--theme', $Theme, '--mode', $Mode, '--hint', $Hint, '--offer', $Offer, '--locale', $Locale)
    if ($View -eq 'settings') { $arguments += @('--view', 'settings') }
    if ($View -eq 'usage') { $arguments += @('--view', 'usage', '--usage-tab', $UsageTab) }
    if ($Scale -gt 0) { $arguments += @('--scale', $Scale.ToString([System.Globalization.CultureInfo]::InvariantCulture)) }
    if ($Settle -le 0) { $Settle = if ($View -eq 'usage') { 2200 } else { 900 } }

    $screen = [System.Windows.Forms.Screen]::PrimaryScreen.Bounds
    if ($CursorX -lt 0) { $CursorX = $screen.Width - 160 }
    if ($CursorY -lt 0) { $CursorY = $screen.Height - 60 }
    [void][NazarShot]::SetCursorPos($CursorX, $CursorY)

    $process = Start-Process -FilePath $exe -ArgumentList $arguments -PassThru
    try {
        $window = [IntPtr]::Zero
        # The panel opens as soon as the webview has drawn; give it a moment to settle so
        # the countdown and the bars are painted rather than half-painted.
        #
        # Forty seconds rather than fifteen. `--scale` passes WebView2 a browser argument, and
        # a WebView2 environment it has not seen before is built from scratch — on a cold
        # machine, or one whose antivirus is reading a binary that was linked a minute ago,
        # that is slower than a run with no argument at all. Waiting longer costs a failed run
        # nothing; waiting too little costs the whole set.
        for ($try = 0; $try -lt 160 -and $window -eq [IntPtr]::Zero; $try++) {
            Start-Sleep -Milliseconds 250
            $window = [NazarShot]::FindVisibleWindow($process.Id)
        }
        if ($window -eq [IntPtr]::Zero) { throw "the panel never appeared ($Out)" }
        Start-Sleep -Milliseconds $Settle

        $rect = [NazarShot]::VisibleBounds($window)
        $width = $rect.Right - $rect.Left
        $height = $rect.Bottom - $rect.Top

        # The settings page is taller than the window, so a section near its end needs the
        # wheel. The pointer is parked over the middle of the panel for the duration and put
        # back afterwards, because where it is decides which element receives the scroll.
        if ($Scroll -gt 0) {
            [void][NazarShot]::SetCursorPos(($rect.Left + [int]($width / 2)), ($rect.Top + [int]($height / 2)))
            [void][NazarShot]::SetForegroundWindow($window)
            Start-Sleep -Milliseconds 200
            [NazarShot]::WheelDown($Scroll)
            Start-Sleep -Milliseconds 500
            [void][NazarShot]::SetCursorPos($CursorX, $CursorY)
            Start-Sleep -Milliseconds 200
        }

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
    Capture-Panel -Scale $Scale -Theme $Theme -Mode $Mode -Hint $Hint -Offer $Offer -View $View -UsageTab $UsageTab -Locale $Locale -Out $Out -CursorX $CursorX -CursorY $CursorY -Scroll $Scroll -Settle $Settle
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
    @{ Scale = 1.0; Theme = 'nazar'; Mode = 'dark'; Hint = 'off'; Offer = 'on'; Out = 'docs/screenshots/wp5-100-offer.png' },
    # WP7: the status-line section, which sits below the fold of the settings page.
    @{ Scale = 1.0; Theme = 'nazar'; Mode = 'dark'; Hint = 'off'; View = 'settings'; Scroll = 6; Out = 'docs/screenshots/wp7-100-statusline.png' },
    # 0.2.0: the usage view, one picture per tab, plus the day a click opens and the two
    # switches that decide what the numbers mean. Named for what they show rather than for a
    # work package, because the four tabs are one feature and `wp20`..`wp23` would spread it
    # over four prefixes nobody outside this repository could read.
    @{ Scale = 1.0; Theme = 'nazar'; Mode = 'dark'; Hint = 'off'; View = 'usage'; UsageTab = 'week'; Out = 'docs/screenshots/usage-100-week.png' },
    @{ Scale = 1.0; Theme = 'nazar'; Mode = 'dark'; Hint = 'off'; View = 'usage'; UsageTab = 'weeks'; Out = 'docs/screenshots/usage-100-weeks.png' },
    @{ Scale = 1.0; Theme = 'nazar'; Mode = 'dark'; Hint = 'off'; View = 'usage'; UsageTab = 'all'; Out = 'docs/screenshots/usage-100-all.png' },
    @{ Scale = 1.0; Theme = 'nazar'; Mode = 'dark'; Hint = 'off'; View = 'usage'; UsageTab = 'models'; Out = 'docs/screenshots/usage-100-models.png' },
    @{ Scale = 1.0; Theme = 'nazar'; Mode = 'dark'; Hint = 'off'; View = 'usage'; UsageTab = 'day'; Out = 'docs/screenshots/usage-100-day.png' },
    @{ Scale = 1.0; Theme = 'nazar'; Mode = 'dark'; Hint = 'off'; View = 'settings'; Scroll = 8; Out = 'docs/screenshots/usage-100-settings.png' }
)

# WP6: the same panel in all six languages. The point of the set is the *width* — the panel
# measures itself and asks Rust for a window that fits, so Russian and Spanish, which are the
# longest, are the proof that a translation cannot clip its own layout.
foreach ($language in @('en', 'tr', 'zh', 'ko', 'ru', 'es')) {
    $shots += @{
        Scale  = 1.0
        Theme  = 'nazar'
        Mode   = 'dark'
        Hint   = 'off'
        Locale = $language
        Out    = "docs/screenshots/wp6-100-$language.png"
    }
}

foreach ($shot in $shots) {
    $shotView = if ($shot.ContainsKey('View')) { $shot.View } else { 'quota' }
    $shotOffer = if ($shot.ContainsKey('Offer')) { $shot.Offer } else { 'off' }
    $shotLocale = if ($shot.ContainsKey('Locale')) { $shot.Locale } else { $Locale }
    $shotScroll = if ($shot.ContainsKey('Scroll')) { $shot.Scroll } else { 0 }
    $shotTab = if ($shot.ContainsKey('UsageTab')) { $shot.UsageTab } else { 'week' }
    Capture-Panel -Scale $shot.Scale -Theme $shot.Theme -Mode $shot.Mode -Hint $shot.Hint -Offer $shotOffer -View $shotView -UsageTab $shotTab -Locale $shotLocale -Out $shot.Out -CursorX $CursorX -CursorY $CursorY -Scroll $shotScroll
}
