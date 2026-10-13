# Opens a plain test window (to see how notches react to windows) and closes it
# after a while: at given bounds, or maximized on a monitor. Doesn't touch your
# own windows. Optionally parks the cursor on that monitor first (pop-ups appear
# on the monitor with the cursor).
#   powershell -File tools/testwin.ps1 -x 200 -y 31 -w 1600 -h 800 [-seconds 8] [-cursor]
#   powershell -File tools/testwin.ps1 -maximize [-monitor 0] [-seconds 8] [-cursor]
#   powershell -File tools/testwin.ps1 -fullscreen [-monitor 0] [-seconds 8] [-cursor]   (borderless, covers it)
# -monitor: index into [Screen]::AllScreens (default: the primary one).
param([int]$x = 200, [int]$y = 31, [int]$w = 1200, [int]$h = 700, [switch]$maximize, [switch]$fullscreen, [int]$monitor = -1, [int]$seconds = 8, [switch]$cursor)
Add-Type -AssemblyName System.Windows.Forms
$screens = [System.Windows.Forms.Screen]::AllScreens
$screen = if ($monitor -ge 0) { $screens[$monitor] } else { [System.Windows.Forms.Screen]::PrimaryScreen }
$form = New-Object System.Windows.Forms.Form
$form.Text = "Kasumi test window"
$form.StartPosition = "Manual"
if ($fullscreen) {
  # Borderless and covering the monitor, like a fullscreen video or game.
  $form.FormBorderStyle = "None"
  $form.Bounds = $screen.Bounds
  $form.BackColor = [System.Drawing.Color]::FromArgb(30, 30, 40)
} elseif ($maximize) {
  $form.Location = New-Object System.Drawing.Point(($screen.Bounds.X + 100), ($screen.Bounds.Y + 100))
  $form.Size = New-Object System.Drawing.Size(400, 300)
  $form.WindowState = "Maximized"
} else {
  $form.Location = New-Object System.Drawing.Point(($screen.Bounds.X + $x), ($screen.Bounds.Y + $y))
  $form.Size = New-Object System.Drawing.Size($w, $h)
}
if ($cursor) {
  [System.Windows.Forms.Cursor]::Position = New-Object System.Drawing.Point(($screen.Bounds.X + $screen.Bounds.Width / 2), ($screen.Bounds.Y + $screen.Bounds.Height / 2))
}
# In front, like the app the user is in (Windows only lets the app in front hand
# the foreground over: borrow its input queue for the switch).
Add-Type @"
using System; using System.Runtime.InteropServices;
public static class TwFront {
  [DllImport("user32.dll")] static extern IntPtr GetForegroundWindow();
  [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
  [DllImport("user32.dll")] static extern bool SetForegroundWindow(IntPtr h);
  [DllImport("user32.dll")] static extern bool AttachThreadInput(uint a, uint b, bool f);
  [DllImport("kernel32.dll")] static extern uint GetCurrentThreadId();
  public static void Activate(IntPtr w) {
    uint pid; uint front = GetWindowThreadProcessId(GetForegroundWindow(), out pid), me = GetCurrentThreadId();
    AttachThreadInput(me, front, true); SetForegroundWindow(w); AttachThreadInput(me, front, false);
  }
}
"@
$form.Add_Shown({ [TwFront]::Activate($form.Handle) })
$timer = New-Object System.Windows.Forms.Timer
$timer.Interval = $seconds * 1000
$timer.Add_Tick({ $form.Close() })
$timer.Start()
"$($screen.DeviceName) $(if ($fullscreen) { 'fullscreen' } elseif ($maximize) { 'maximized' } else { "at $x,$y ${w}x$h" }) for ${seconds}s"
[void]$form.ShowDialog()
