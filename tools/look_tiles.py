"""The settings' look pickers' pictures: every widget look drawn by Kasumi itself
(--native-snapshot, from the shipped default config with one look picked and sample
data), its panel cut from the bar (as if on its own, corners rounded, see-through around
it), scaled down into public/looks/.

    python -I tools/look_tiles.py [widget ...]

Needs a release build (target/release/kasumi.exe). Run again after changing a look
(name widgets to draw only theirs).
"""
import base64
import json
import os
import re
import shutil
import struct
import subprocess
import tempfile
import time
import math
import sys
import zlib

repo = os.path.abspath(os.path.join(os.path.dirname(__file__), ".."))
exe = os.path.join(repo, "src-tauri", "target", "release", "kasumi.exe")
out_dir = os.path.join(repo, "public", "looks")
os.makedirs(out_dir, exist_ok=True)
work = tempfile.mkdtemp(prefix="kasumi-looks-")

# widget: (bar, [styles], picture of: "panel", "search", "open")
LOOKS = {
    "media": ("top", ["", "record", "glow", "cover"], "panel"),
    "clock": ("top", ["", "week"], "panel"),
    "weather": ("top", ["", "sky", "curve", "week"], "panel"),
    "menu": ("top", ["", "grid", "strip", "greeting"], "panel"),
    "quick": ("bottom", ["", "blocks", "rings", "list"], "panel"),
    "launcher": ("bottom", ["", "centre", "grid", "prompt"], "search"),
    "tasks": ("bottom", ["", "labels", "dots"], "open"),  # dots: resting (closed)
    "temps": ("top", ["", "dials", "graph", "table"], "panel"),
    "tray": ("top", ["", "list"], "panel"),
    "clipboard": ("top", ["", "tiles", "compact"], "panel"),
    "disk": ("top", ["drives", "rings"], "panel"),
    "claude": ("top", ["", "rings", "pace", "sessions"], "panel"),
}
if sys.argv[1:]:
    LOOKS = {w: v for w, v in LOOKS.items() if w in sys.argv[1:]}


def png(w, h):
    rows = b""
    for y in range(h):
        rows += b"\x00" + b"".join(bytes((int(255 * (0.9 - 0.5 * y / h)), int(255 * (0.35 + 0.4 * x / w)), int(255 * (0.2 + 0.7 * x * y / (w * h))))) for x in range(w))

    def chunk(k, d):
        return struct.pack(">I", len(d)) + k + d + struct.pack(">I", zlib.crc32(k + d) & 0xFFFFFFFF)

    return b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 2, 0, 0, 0)) + chunk(b"IDAT", zlib.compress(rows)) + chunk(b"IEND", b"")


now = time.time() * 1000
hours = [(13, 7, 3), (14, 8, 3), (15, 8, 61), (16, 7, 61), (17, 6, 3), (18, 5, 3), (19, 5, 2), (20, 4, 2), (21, 4, 1), (22, 3, 0), (23, 3, 0), (0, 2, 0), (1, 2, 0)]
days = [("2026-11-04", 3, 9, 61), ("2026-11-05", 2, 8, 3), ("2026-11-06", 1, 11, 0), ("2026-11-07", 4, 13, 1), ("2026-11-08", 5, 10, 61), ("2026-11-09", 3, 9, 3), ("2026-11-10", 2, 7, 71)]
apps = [("Code", True), ("Firefox", True), ("Spotify", False), ("Terminal", True)]
data = {
    "media": {"title": "Glass Rooms", "artist": "Nadia Vale", "playing": False, "positionMs": 102000, "durationMs": 238000, "updatedAt": now, "canSeek": True,
              "canPrevious": True, "canPlayPause": True, "canNext": True, "art": "data:image/png;base64," + base64.b64encode(png(96, 96)).decode()},
    "weather": {"temp": 7, "feels": 4, "code": 3, "isDay": True, "min": 3, "max": 9, "location": "Oslo",
                "hourly": [{"time": f"2026-11-04T{h:02d}:00", "temp": t, "code": c, "isDay": 7 <= h <= 16} for h, t, c in hours],
                "daily": [{"date": d, "min": lo, "max": hi, "code": c} for d, lo, hi, c in days]},
    "system": {"volume": {"level": 0.6, "muted": False}, "brightness": 80, "battery": {"percent": 82, "charging": False, "plugged": False, "secondsLeft": 11400},
               "network": {"connected": True, "wifi": True, "name": "Home", "internet": True}},
    "quick:switches": {"wifi": True, "bluetooth": True, "dark": False},
    "launcher:pinned": [{"id": f"app:{n}", "name": n} for n in ["Code", "Firefox", "Spotify", "Terminal", "Calculator", "Notepad", "Photos", "Settings"]],
    "tasks": {"apps": [{"key": f"app:{n}", "name": n, "pinned": p} for n, p in apps],
              "windows": [{"hwnd": 1, "title": "Kasumi - launcher.rs", "app": "app:Code", "active": True, "monitor": "", "z": 0},
                          {"hwnd": 2, "title": "GitHub", "app": "app:Firefox", "monitor": "", "z": 1},
                          {"hwnd": 3, "title": "Docs", "app": "app:Firefox", "monitor": "", "z": 2},
                          {"hwnd": 4, "title": "Spotify", "app": "app:Spotify", "monitor": "", "z": 3}]},
}


def icon(w, rgb, shape):
    """A made-up tray icon (no real app's): a rounded square, circle or diamond."""
    px = b""
    for y in range(w):
        for x in range(w):
            dx, dy = x - w / 2 + 0.5, y - w / 2 + 0.5
            inside = {"circle": dx * dx + dy * dy < (w / 2 - 2) ** 2, "diamond": abs(dx) + abs(dy) < w / 2 - 1}.get(shape, max(abs(dx), abs(dy)) < w / 2 - 3)
            px += bytes(rgb + (255,)) if inside else b"\0\0\0\0"
    return base64.b64encode(px).decode()


sec = time.time()
tray_apps = [("Cloud sync", (60, 130, 230), "circle"), ("Chat", (110, 90, 230), "square"), ("Music", (40, 185, 90), "circle"), ("Games", (30, 40, 70), "square"),
             ("Security", (40, 120, 210), "diamond"), ("Graphics", (110, 180, 30), "square"), ("Notes", (240, 180, 40), "diamond"), ("Mail", (220, 70, 60), "circle")]
data["tray"] = [{"id": f"i{i}", "tip": n, "name": n, "exe": n.lower().replace(" ", "") + ".exe", "w": 32, "h": 32, "rgba": icon(32, c, sh)} for i, (n, c, sh) in enumerate(tray_apps)]
data["temps"] = {"ok": True, "cpu": 63, "gpu": 51, "cpuLoad": 24, "cpuPower": 48, "cpuClock": 4475, "gpuLoad": 18, "gpuPower": 41, "vramUsed": 3100, "vramTotal": 12288,
                 "ramUsed": 14.2, "ramAvailable": 17.6, "cpuName": "AMD Ryzen 7 7800X3D", "gpuName": "NVIDIA GeForce RTX 4070"}
data["temps~series"] = [dict(data["temps"], cpu=56 + 7 * math.sin(i / 30), gpu=47 + 4 * math.sin(i / 45 + 1)) for i in range(300)]
data["clipboard"] = [
    {"id": 1, "time": sec - 40, "kind": "text", "text": "shell:Downloads"},
    {"id": 2, "time": sec - 300, "kind": "image", "width": 1280, "height": 720, "thumb": {"w": 96, "h": 54, "rgba": base64.b64encode(b"".join(bytes((40 + x, 70 + y, 160, 255)) for y in range(54) for x in range(96))).decode()}},
    {"id": 3, "time": sec - 900, "kind": "text", "text": "Meeting moved to Thursday 15:00, same room."},
    {"id": 4, "time": sec - 3600, "kind": "text", "text": "https://example.com/notes/kasumi"},
    {"id": 5, "time": sec - 7200, "kind": "text", "text": "#eeff00"},
]
data["disks"] = [{"id": "C:", "label": "System", "total": 1000e9, "free": 312e9}, {"id": "D:", "label": "Games", "total": 2000e9, "free": 160e9}]
data["recycle"] = {"bytes": 2.4e9, "items": 37}
iso = lambda t: time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime(t))
day0 = sec - sec % 86400
data["claude"] = {
    "buckets": [{"h": day0 - d * 86400 + h * 3600, "model": "claude-opus", "tokens": t * (1 + d % 3 * 0.4), "cacheRead": 0, "messages": 30, "sessions": [f"s{d}"]}
                for d in range(7) for h, t in [(9, 1.2e6), (14, 2e6)] if day0 - d * 86400 + h * 3600 < sec],
    "window": {"start": sec - 2 * 3600, "end": sec + 3 * 3600, "tokens": 3.1e6, "messages": 58}, "busy": False, "now": sec,
    "account": {"session": {"percent": 42, "resetsAt": iso(sec + 3 * 3600)}, "week": {"percent": 61, "resetsAt": iso(sec + 2.5 * 86400)}},
    "sessions": [{"id": "a", "project": "website", "title": "", "state": "working", "quiet": 3}, {"id": "b", "project": "notes", "title": "", "state": "asks", "quiet": 140},
                 {"id": "c", "project": "scripts", "title": "", "state": "done", "quiet": 600}],
}
data_path = os.path.join(work, "data.json")
json.dump(data, open(data_path, "w"))

base = open(os.path.join(repo, "src-tauri", "default-config.toml"), encoding="utf-8").read()


def put(text, section, lines):
    """`lines` into [section], replacing keys it already has (or a new section)."""
    keys = [l.split("=")[0].strip() for l in lines.strip().splitlines()]
    out, inside, done = [], False, False
    for line in text.splitlines():
        if line.startswith("["):
            inside = line.strip() == f"[{section}]"
            out.append(line)
            if inside:
                out.extend(lines.strip().splitlines())
                done = True
            continue
        m = re.match(r"\s*(\w+)\s*=", line)
        if inside and m and m.group(1) in keys:
            continue
        out.append(line)
    if not done:
        out += ["", f"[{section}]"] + lines.strip().splitlines()
    return "\n".join(out) + "\n"


def read_png(path):
    raw_png = open(path, "rb").read()
    pos, idat = 8, b""
    while pos < len(raw_png):
        n = struct.unpack(">I", raw_png[pos:pos + 4])[0]
        kind, body = raw_png[pos + 4:pos + 8], raw_png[pos + 8:pos + 8 + n]
        if kind == b"IHDR":
            w, h, _, ctype = struct.unpack(">IIBB", body[:10])
        elif kind == b"IDAT":
            idat += body
        pos += 12 + n
    bpp = 4 if ctype == 6 else 3
    raw, stride, rows, prev, i = zlib.decompress(idat), w * bpp, [], bytearray(w * bpp), 0
    for _ in range(h):
        f = raw[i]
        i += 1
        line = bytearray(raw[i:i + stride])
        i += stride
        for x in range(stride):
            a = line[x - bpp] if x >= bpp else 0
            b, c = prev[x], (prev[x - bpp] if x >= bpp else 0)
            if f == 1:
                line[x] = (line[x] + a) & 255
            elif f == 2:
                line[x] = (line[x] + b) & 255
            elif f == 3:
                line[x] = (line[x] + (a + b) // 2) & 255
            elif f == 4:
                p = a + b - c
                pa, pb, pc = abs(p - a), abs(p - b), abs(p - c)
                line[x] = (line[x] + (a if pa <= pb and pa <= pc else b if pb <= pc else c)) & 255
        rows.append(bytes(line))
        prev = line
    return w, h, bpp, rows


def spans(path):
    """Each row's drawn span (first and last x that isn't the snapshot's ground), the
    picture's size and its ground colour."""
    w, h, bpp, rows = read_png(path)
    ground = rows[h // 2][:3]
    out = []
    for y in range(h):
        r = rows[y]
        xs = [x for x in range(0, w, 1) if sum(abs(r[x * bpp + k] - ground[k]) for k in range(3)) > 30]
        out.append((xs[0], xs[-1]) if xs else None)
    return w, h, out, ground


def detached_box(path, top):
    """The panel alone, cut where it meets its bar: x0, y0, x1, y1 (inclusive) and its
    corners' radius. The panel's body is the tallest run of rows of one width; past its
    free end (under a top bar, above a dock) its rounded corners narrow it."""
    w, h, sp, _ = spans(path)
    width = lambda s: s[1] - s[0] if s else 0
    best, run = (0, 0), None
    for y in range(h + 1):
        s = sp[y] if y < h else None
        if run and s and abs(width(s) - width(sp[run[0]])) <= 2 and abs(s[0] - sp[run[0]][0]) <= 2:
            run = (run[0], y)
            continue
        if run and run[1] - run[0] > best[1] - best[0]:
            best = run
        run = (y, y) if s else None
    y0, y1 = best
    x0, x1 = sp[y0]
    # The free end: rows narrowing (its corners) past the run.
    step = 1 if top else -1
    end, radius = (y1 if top else y0), 0
    y = end + step
    while 0 <= y < h and sp[y] and sp[y][0] > sp[y - step][0] - 1 and sp[y][0] - x0 < 40:
        radius += 1
        y += step
    if top:
        y1 = end + radius
    else:
        y0 = end - radius
    return x0, y0, x1, y1, max(radius, 8)


def crop_box(path):
    """Around everything drawn near the middle of the screen (the dock with its tasks)."""
    w, h, sp, _ = spans(path)
    ys = [y for y in range(h) if sp[y] and sp[y][1] - sp[y][0] < w - 40]
    xs = [x for y in ys for x in sp[y]]
    if not ys:
        return None
    return max(min(xs) - 14, 0), max(min(ys) - 2, 0), min(max(xs) + 14, w - 1), min(max(ys) + 14, h - 1)


def save(src, target, box=None, radius=0):
    """`src` cut to `box` (corners rounded off, see-through), at most 300 x 220."""
    vf = []
    if box:
        x0, y0, x1, y1 = box
        vf.append(f"crop={x1 - x0 + 1}:{y1 - y0 + 1}:{x0}:{y0}")
    vf.append("format=rgba")
    if radius:
        r = radius
        d = f"hypot(max(max({r}-X\\,X-(W-1-{r}))\\,0)\\,max(max({r}-Y\\,Y-(H-1-{r}))\\,0))"
        vf.append(f"geq=r='r(X,Y)':g='g(X,Y)':b='b(X,Y)':a='alpha(X,Y)*clip({r}-{d}+0.5\\,0\\,1)'")
    vf.append("scale='min(300,iw)':'min(220,ih)':force_original_aspect_ratio=decrease")
    subprocess.run(["ffmpeg", "-loglevel", "error", "-y", "-i", src, "-vf", ",".join(vf), target], check=True)
    print(target)


for widget, (bar, styles, shot) in LOOKS.items():
    for style in styles:
        lists = f'left = []\ncenter = ["{widget}"]\nright = []\nnotch = [{chr(34) + widget + chr(34) if bar == "top" else ""}]\n'
        cfg = put(base, f"surfaces.{bar}", lists)
        cfg = put(cfg, f"surfaces.{'bottom' if bar == 'top' else 'top'}", "enabled = false\n")
        cfg = put(cfg, f"widgets.{widget}", f'enabled = true\nstyle = "{style}"\n' + ("pinned = true\n" if widget == "launcher" else ""))
        snap = os.path.join(work, f"{widget}-{style or 'classic'}")
        os.makedirs(snap, exist_ok=True)
        cfg_path = snap + ".toml"
        open(cfg_path, "w", encoding="utf-8").write(cfg)
        subprocess.run([exe, "--native-snapshot", snap, "--config", cfg_path, "--data", data_path], capture_output=True, timeout=120)
        pics = sorted(f for f in os.listdir(snap) if f.endswith(".png") and f"-{bar}" in f)
        prefix = pics[0].split(f"-{bar}")[0] if pics else None
        name = {"panel": f"-{bar}-{widget}.png", "search": f"-{bar}-launcher-search.png", "open": f"-{bar}-open.png"}[shot]
        if widget == "tasks" and style == "dots":
            name = f"-{bar}-closed.png"
        if widget == "launcher":
            name = {"grid": f"-{bar}-launcher.png", "prompt": f"-{bar}-launcher-partial.png", "centre": f"-{bar}-launcher-centre.png"}.get(style, name)
        src = os.path.join(snap, (prefix or "") + name)
        target = os.path.join(out_dir, f"{widget}-{style or 'classic'}.png")
        if not (prefix and os.path.exists(src)):
            print("no picture:", widget, style or "classic", src)
            continue
        if style == "centre":
            # Already on its own: drawn without a bar, see-through around it.
            save(src, target)
        elif shot == "open" or widget == "tasks":
            # The tasks' look is the dock itself: as it sits.
            save(src, target, crop_box(src))
        else:
            x0, y0, x1, y1, r = detached_box(src, bar == "top")
            save(src, target, (x0, y0, x1, y1), r)
shutil.rmtree(work, ignore_errors=True)
