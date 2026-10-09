# PRESSES KEYS ON THE USER'S PC: ask first. Sends key combos (simulated input,
# seen by every keyboard hook like real keys) and after each one reports the
# window in front and whether Windows' Start menu is open.
#   powershell -File tools/keys.ps1 -steps "LWin LWin LWin+LMenu+Space Escape"   (space-separated)
# Keys: names from System.Windows.Forms.Keys (LWin, LMenu = left Alt, Space,
# Escape, A..Z, F1..). Avoid Win+D / Alt+F4 style combos (they act on the user's windows).
param([string]$steps, [int]$waitMs = 800, [string]$front = "", [switch]$physical)
# -front zen: bring that program to the front first. -physical: StatusNotch's
# keyboard helper treats these keys as real key presses (it ignores simulated
# Windows key presses otherwise: keyhook.rs AS_PHYSICAL).
Add-Type -AssemblyName System.Windows.Forms
Add-Type @"
using System; using System.Runtime.InteropServices; using System.Text;
public static class KeySim {
  [StructLayout(LayoutKind.Sequential)] public struct KEYBDINPUT { public ushort wVk; public ushort wScan; public uint dwFlags; public uint time; public IntPtr dwExtraInfo; }
  [StructLayout(LayoutKind.Explicit, Size = 40)] public struct INPUT { [FieldOffset(0)] public uint type; [FieldOffset(8)] public KEYBDINPUT ki; }
  [DllImport("user32.dll")] static extern uint SendInput(uint n, INPUT[] inputs, int size);
  [DllImport("user32.dll")] static extern IntPtr GetForegroundWindow();
  [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
  [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern int GetClassName(IntPtr h, StringBuilder s, int n);
  [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern int GetWindowText(IntPtr h, StringBuilder s, int n);
  public static IntPtr Mark = IntPtr.Zero;
  public static void Combo(ushort[] vks) {
    var list = new System.Collections.Generic.List<INPUT>();
    foreach (var vk in vks) list.Add(Key(vk, false));
    for (int i = vks.Length - 1; i >= 0; i--) list.Add(Key(vks[i], true));
    SendInput((uint)list.Count, list.ToArray(), Marshal.SizeOf(typeof(INPUT)));
  }
  static INPUT Key(ushort vk, bool up) {
    var i = new INPUT { type = 1 };
    i.ki.wVk = vk;
    // Win, arrows etc. are extended keys.
    bool ext = vk == 0x5B || vk == 0x5C || (vk >= 0x21 && vk <= 0x2E);
    i.ki.dwFlags = (up ? 2u : 0u) | (ext ? 1u : 0u);
    i.ki.dwExtraInfo = Mark;
    return i;
  }
  public static string Front() {
    var h = GetForegroundWindow(); uint pid; GetWindowThreadProcessId(h, out pid);
    var c = new StringBuilder(256); GetClassName(h, c, 256);
    var t = new StringBuilder(256); GetWindowText(h, t, 256);
    string name = "?"; try { name = System.Diagnostics.Process.GetProcessById((int)pid).ProcessName; } catch {}
    return name + " | " + c + " | " + t;
  }
}
"@
if ($front) {
  $p = Get-Process $front -ErrorAction SilentlyContinue | Where-Object { $_.MainWindowHandle -ne 0 } | Select-Object -First 1
  if ($p) { (New-Object -ComObject WScript.Shell).AppActivate($p.Id) | Out-Null; Start-Sleep -Milliseconds 400 }
}
if ($physical) { [KeySim]::Mark = [IntPtr]0x534E0200 }
"before: $([KeySim]::Front())"
foreach ($s in $steps.Split(" ", [System.StringSplitOptions]::RemoveEmptyEntries)) {
  $vks = [uint16[]]($s.Split('+') | ForEach-Object { [uint16][System.Windows.Forms.Keys]$_ })
  [KeySim]::Combo($vks)
  Start-Sleep -Milliseconds $waitMs
  $front = [KeySim]::Front()
  $startOpen = $front -match "StartMenuExperienceHost|SearchHost"
  "{0,-20} -> {1}{2}" -f $s, $front, $(if ($startOpen) { "   <-- START MENU OPEN" } else { "" })
}
