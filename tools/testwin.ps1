# Opens a plain test window (to see how notches react to windows) and closes it
# after a while: at given bounds, or maximized on a monitor. Doesn't touch your
# own windows. Optionally parks the cursor on that monitor first (pop-ups appear
# on the monitor with the cursor).
#   powershell -File tools/testwin.ps1 -x 200 -y 31 -w 1600 -h 800 [-seconds 8] [-cursor]
#   powershell -File tools/testwin.ps1 -maximize [-monitor 0] [-seconds 8] [-cursor]
# -monitor: index into [Screen]::AllScreens (default: the primary one).
param([int]$x = 200, [int]$y = 31, [int]$w = 1200, [int]$h = 700, [switch]$maximize, [int]$monitor = -1, [int]$seconds = 8, [switch]$cursor)
Add-Type -AssemblyName System.Windows.Forms
$screens = [System.Windows.Forms.Screen]::AllScreens
$screen = if ($monitor -ge 0) { $screens[$monitor] } else { [System.Windows.Forms.Screen]::PrimaryScreen }
$form = New-Object System.Windows.Forms.Form
$form.Text = "Kasumi test window"
$form.StartPosition = "Manual"
if ($maximize) {
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
$timer = New-Object System.Windows.Forms.Timer
$timer.Interval = $seconds * 1000
$timer.Add_Tick({ $form.Close() })
$timer.Start()
"$($screen.DeviceName) $(if ($maximize) { 'maximized' } else { "at $x,$y ${w}x$h" }) for ${seconds}s"
[void]$form.ShowDialog()
