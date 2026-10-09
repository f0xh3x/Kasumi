"""Copies the user's presets into src-tauri/presets for shipping, minus personal bits:
named [[monitors]] rules (the "*" rule stays), tasks pins, the weather city.
A release step: python -I tools/ship_presets.py"""
import os
import re
import tomllib

src = os.path.join(os.environ["APPDATA"], "StatusNotch", "presets")
dst = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "src-tauri", "presets")
os.makedirs(dst, exist_ok=True)

for name in ["Classic status bar", "Classic taskbar", "Modern"]:
    text = open(os.path.join(src, name + ".toml"), encoding="utf-8").read().replace("\r\n", "\n")
    out, i = [], 0
    lines = text.split("\n")
    while i < len(lines):
        line = lines[i]
        if line.strip() == "[[monitors]]":
            block = [line]
            i += 1
            while i < len(lines) and not lines[i].startswith("["):
                block.append(lines[i])
                i += 1
            if any(re.match(r'\s*match\s*=\s*"\*"', b) for b in block):
                out.extend(block)
            continue
        if re.match(r"\s*(pins|location|latitude|longitude)\s*=", line):
            i += 1
            continue
        if line.startswith("# Open-Meteo, no API key."):
            line = "# Open-Meteo, no API key. Pick your city in Settings > Widgets > Weather."
        out.append(line)
        i += 1
    result = "\n".join(out)
    cfg = tomllib.loads(result)  # still valid TOML
    assert all(m.get("match") == "*" for m in cfg.get("monitors", [])), name
    assert "pins" not in cfg.get("widgets", {}).get("tasks", {}), name
    open(os.path.join(dst, name + ".toml"), "w", encoding="utf-8", newline="\n").write(result)
    print(name, "->", len(result), "bytes,", len(cfg.get("monitors", [])), "monitor rule(s)")
