"""Measuring screenshots and recordings instead of eyeballing them.

  python -I tools/frames.py compare <dir> <tagA> <tagB>
      tools/snap.ps1 pairs (<tag>-<n>.png): identical, or where they differ.
  python -I tools/frames.py extract <video> <dir>
      every frame of a recording as <dir>/f0001.png ... (ffmpeg on PATH).
  python -I tools/frames.py changes <dir> [glob]
      for consecutive frames: which x ranges changed (finds what moved, and when).
  python -I tools/frames.py track <dir> <x0> <y0> <x1> <y1> [glob]
      per frame: the brightness-weighted center of bright pixels in that box
      (text / icons on black); only frames where it moved >0.25 px are printed.
      Track two boxes (e.g. an icon and its neighbour) to tell "this moved"
      from "everything moved".

Sub-pixel jumps are invisible to the eye in a zoomed screenshot but obvious here.
"""
import glob
import os
import subprocess
import sys

from PIL import Image, ImageChops


def frames(d, pattern="*.png"):
    return sorted(glob.glob(os.path.join(d, pattern)))


def compare(d, a, b):
    for fa in sorted(glob.glob(os.path.join(d, f"{a}-*.png"))):
        n = fa.rsplit("-", 1)[1]
        fb = os.path.join(d, f"{b}-{n}")
        diff = ImageChops.difference(Image.open(fa).convert("RGB"), Image.open(fb).convert("RGB")).convert("L")
        box = diff.point(lambda v: 255 if v > 24 else 0).getbbox()
        print(n[:-4], "identical" if not box else f"differs in {box}")


def extract(video, d):
    os.makedirs(d, exist_ok=True)
    subprocess.run(["ffmpeg", "-v", "error", "-y", "-i", video, os.path.join(d, "f%04d.png")], check=True)
    print(len(frames(d)), "frames")


def changes(d, pattern="*.png"):
    fs = frames(d, pattern)
    prev = Image.open(fs[0]).convert("RGB")
    for f in fs[1:]:
        im = Image.open(f).convert("RGB")
        mask = ImageChops.difference(prev, im).convert("L").point(lambda v: 255 if v > 30 else 0)
        if mask.getbbox():
            w, h = mask.size
            px = mask.load()
            cols = [x for x in range(w) if any(px[x, y] for y in range(h))]
            spans = []
            for x in cols:
                if spans and x - spans[-1][1] <= 3:
                    spans[-1][1] = x
                else:
                    spans.append([x, x])
            print(os.path.basename(f), spans[:12])
        prev = im


def center(im, box, threshold=150):
    c = im.crop(box)
    w, h = c.size
    px = c.load()
    sx = sy = n = 0
    for y in range(h):
        for x in range(w):
            v = px[x, y]
            if v > threshold:
                sx += x * v
                sy += y * v
                n += v
    return (box[0] + sx / n, box[1] + sy / n) if n > 500 else None


def track(d, x0, y0, x1, y1, pattern="*.png"):
    box = tuple(int(v) for v in (x0, y0, x1, y1))
    prev = None
    for f in frames(d, pattern):
        c = center(Image.open(f).convert("L"), box)
        if c and (prev is None or abs(c[0] - prev[0]) > 0.25 or abs(c[1] - prev[1]) > 0.25):
            print(os.path.basename(f), "x %.2f  y %.2f" % c)
        prev = c or prev


if __name__ == "__main__":
    cmd, args = sys.argv[1], sys.argv[2:]
    {"compare": compare, "extract": extract, "changes": changes, "track": track}[cmd](*args)
