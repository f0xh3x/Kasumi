# Lists StatusNotch's visible windows: position, size, and whether they have a frame
# (they must not). Bars are ~345 px tall and docks ~709 once their page has run.
# A 16x16 window "with a frame" is a hidden helper window: normal.
Add-Type @"
using System; using System.Collections.Generic; using System.Runtime.InteropServices; using System.Text;
public static class NC {
  delegate bool P(IntPtr h, IntPtr l);
  [DllImport("user32.dll")] static extern bool EnumWindows(P p, IntPtr l);
  [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
  [DllImport("user32.dll")] static extern int GetWindowLong(IntPtr h, int i);
  [DllImport("user32.dll")] static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll")] static extern bool GetWindowRect(IntPtr h, out R r);
  [DllImport("user32.dll")] static extern bool GetClientRect(IntPtr h, out R r);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern int GetClassName(IntPtr h, StringBuilder s, int n);
  public struct R { public int L, T, Rr, B; }
  public static List<string> Run(uint pid) {
    var o = new List<string>();
    EnumWindows((h, l) => {
      uint p; GetWindowThreadProcessId(h, out p);
      var c = new StringBuilder(64); GetClassName(h, c, 64);
      if (p != pid || !IsWindowVisible(h)) return true;
      R w, cl; GetWindowRect(h, out w); GetClientRect(h, out cl); int s = GetWindowLong(h, -16);
      o.Add(string.Format("window ({0},{1}) {2}x{3}  client {4}x{5}  popup={6} caption={7}{8}", w.L, w.T, w.Rr - w.L, w.B - w.T, cl.Rr, cl.B, (s & unchecked((int)0x80000000)) != 0, (s & 0x00C00000) == 0x00C00000, (w.Rr - w.L != cl.Rr || w.B - w.T != cl.B) ? "   <-- has a frame" : ""));
      return true;
    }, IntPtr.Zero);
    return o;
  }
}
"@
foreach ($p in Get-Process statusnotch) { [NC]::Run([uint32]$p.Id) }
