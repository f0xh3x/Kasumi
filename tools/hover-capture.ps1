# MOVES THE USER'S MOUSE: ask first (they're often working in other apps).
# Opens and closes a monitor's bottom dock by putting the cursor on its bottom
# edge and then away, `cycles` times, capturing a strip above the dock as fast as
# possible (~60 fps) the whole time. Frames: <out>\<index>_<ms>_<cycle>_<cursorY>.png
# (cursorY = the bottom edge while open). The cursor goes back where it was.
# Screen recorders change frame timing, so some glitches only show up this way.
#   powershell -File tools/hover-capture.ps1 -monitor 0 [-cycles 4]
#   python -I tools/frames.py changes <out> "*_0_*.png"      (cycle 0, open + close)
param(
  [int]$monitor = 0,          # index in left-to-right order, as tools/snap.ps1 numbers them
  [int]$width = 1000,
  [int]$height = 110,
  [int]$cycles = 4,
  [int]$openMs = 1600,
  [int]$closedMs = 1200,
  [string]$out = ""
)
Add-Type -AssemblyName System.Drawing, System.Windows.Forms
Add-Type 'using System; using System.Runtime.InteropServices; public static class HoverCap { [DllImport("user32.dll")] public static extern bool SetProcessDpiAwarenessContext(IntPtr v); [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y); [DllImport("user32.dll")] public static extern bool GetCursorPos(out POINT p); public struct POINT { public int X; public int Y; } }'
[HoverCap]::SetProcessDpiAwarenessContext([IntPtr](-4)) | Out-Null
if (-not $out) { $out = if ($env:CLAUDE_JOB_DIR) { "$env:CLAUDE_JOB_DIR\tmp\hover" } else { "$env:TEMP\kasumi-hover" } }
New-Item -ItemType Directory -Force $out | Out-Null
Get-ChildItem $out -Filter *.png -ErrorAction SilentlyContinue | Remove-Item

$b = ([System.Windows.Forms.Screen]::AllScreens | Sort-Object { $_.Bounds.X })[$monitor].Bounds
$cx = $b.X + [int]($b.Width / 2)
$x0 = $cx - [int]($width / 2); $y0 = $b.Bottom - $height
$was = New-Object HoverCap+POINT; [HoverCap]::GetCursorPos([ref]$was) | Out-Null
$frames = New-Object System.Collections.Generic.List[object]
$sw = [Diagnostics.Stopwatch]::StartNew()
for ($c = 0; $c -lt $cycles; $c++) {
  foreach ($phase in @(@(($b.Bottom - 1), $openMs), @(($b.Bottom - 450), $closedMs))) {
    [HoverCap]::SetCursorPos($cx, $phase[0]) | Out-Null
    $end = $sw.ElapsedMilliseconds + $phase[1]
    while ($sw.ElapsedMilliseconds -lt $end) {
      $bmp = New-Object System.Drawing.Bitmap $width, $height
      $g = [System.Drawing.Graphics]::FromImage($bmp); $g.CopyFromScreen($x0, $y0, 0, 0, $bmp.Size); $g.Dispose()
      $frames.Add(@($sw.ElapsedMilliseconds, $c, $phase[0], $bmp))
    }
  }
}
[HoverCap]::SetCursorPos($was.X, $was.Y) | Out-Null
$i = 0
foreach ($f in $frames) { $f[3].Save(("$out\{0:D4}_{1}_{2}_{3}.png" -f $i, $f[0], $f[1], $f[2])); $f[3].Dispose(); $i++ }
"$i frames in $($sw.ElapsedMilliseconds) ms -> $out"
