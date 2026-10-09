# Screenshots a strip along the bottom (or top) of every monitor, in physical
# pixels, as <out>\<tag>-<n>.png (n = monitor index). Take one before and one
# after a change, then: python -I tools/frames.py compare <out> <before> <after>
#   powershell -File tools/snap.ps1 -tag before [-edge top] [-height 200]
param(
  [Parameter(Mandatory = $true)][string]$tag,
  [string]$edge = "bottom",
  [int]$height = 200,
  [string]$out = ""
)
Add-Type -AssemblyName System.Drawing, System.Windows.Forms
Add-Type 'using System; using System.Runtime.InteropServices; public static class SnapDpi { [DllImport("user32.dll")] public static extern bool SetProcessDpiAwarenessContext(IntPtr v); }'
[SnapDpi]::SetProcessDpiAwarenessContext([IntPtr](-4)) | Out-Null
if (-not $out) { $out = if ($env:CLAUDE_JOB_DIR) { "$env:CLAUDE_JOB_DIR\tmp\snaps" } else { "$env:TEMP\statusnotch-snaps" } }
New-Item -ItemType Directory -Force $out | Out-Null
$i = 0
foreach ($s in [System.Windows.Forms.Screen]::AllScreens | Sort-Object { $_.Bounds.X }) {
  $b = $s.Bounds
  $y = if ($edge -eq "top") { $b.Y } else { $b.Bottom - $height }
  $bmp = New-Object System.Drawing.Bitmap $b.Width, $height
  $g = [System.Drawing.Graphics]::FromImage($bmp)
  $g.CopyFromScreen($b.X, $y, 0, 0, $bmp.Size)
  $bmp.Save("$out\$tag-$i.png"); $g.Dispose(); $bmp.Dispose()
  "$tag-$i  $($s.DeviceName)  $($b.Width)x$($b.Height) at ($($b.X),$($b.Y))"
  $i++
}
