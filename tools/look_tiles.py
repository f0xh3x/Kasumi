"""The settings' look pickers' pictures: every widget look drawn by Kasumi itself
(--native-snapshot, from the shipped default config with one look picked and sample
data), cropped to the widget and its panel, scaled down into public/looks/.

    python -I tools/look_tiles.py

Needs a release build (target/release/kasumi.exe). Run again after changing a look.
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
    "launcher": ("bottom", ["", "grid", "prompt"], "search"),
    "tasks": ("bottom", ["", "labels", "dots"], "open"),  # dots: resting (closed)
}


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


def crop_box(path, top):
    """Where the picture differs from the snapshot's ground, near the middle of the screen
    (a top bar: as wide as what hangs under the bar, the bar itself across that)."""
    w, h, bpp, rows = read_png(path)
    ground = rows[h // 2][:3]
    xs, ys = [], []
    for y in range(0, h, 2):
        r = rows[y]
        for x in range(max(w // 2 - 420, 0), min(w // 2 + 420, w), 2):
            px = r[x * bpp:x * bpp + 3]
            if bpp == 4 and r[x * bpp + 3] == 0:
                continue
            if sum(abs(px[k] - ground[k]) for k in range(3)) > 30:
                if not top or y > 44:
                    xs.append(x)
                ys.append(y)
    if not xs:
        return None
    return max(min(xs) - 14, 0), max(min(ys) - 2, 0), min(max(xs) + 14, w), min(max(ys) + 14, h)


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
            name = {"grid": f"-{bar}-launcher.png", "prompt": f"-{bar}-launcher-partial.png"}.get(style, name)
        src = os.path.join(snap, (prefix or "") + name)
        box = crop_box(src, bar == "top") if prefix and os.path.exists(src) else None
        target = os.path.join(out_dir, f"{widget}-{style or 'classic'}.png")
        if not box:
            print("no picture:", widget, style or "classic", src)
            continue
        x0, y0, x1, y1 = box
        if bar == "top":
            y0 = 0
        subprocess.run(["ffmpeg", "-loglevel", "error", "-y", "-i", src, "-vf", f"crop={x1 - x0}:{y1 - y0}:{x0}:{y0},scale='min(260,iw)':-2", target], check=True)
        print(target)
shutil.rmtree(work, ignore_errors=True)
