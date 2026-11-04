"""Screenshots of the settings window without the app: the built page (dist/) with a
stubbed Tauri backend (the user's config, read only; nothing is written), served
locally and drawn by headless Edge.

    python -I tools/settings_shot.py <out dir> <page>[#words] ...

A page is a settings page id (?page=), e.g. `layout`, `widget-settings`; add
`#search words` to type into the sidebar's search instead. Build first
(`npm run build`); tools/install.sh does.
"""
import functools
import http.server
import json
import os
import shutil
import subprocess
import sys
import threading
import tomllib

repo = os.path.abspath(os.path.join(os.path.dirname(__file__), ".."))
out, shots = os.path.abspath(sys.argv[1]), sys.argv[2:]
site = os.path.join(out, "site")
shutil.rmtree(site, ignore_errors=True)
shutil.copytree(os.path.join(repo, "dist"), site)

cfg = tomllib.load(open(os.path.join(os.environ["APPDATA"], "Kasumi", "config.toml"), "rb"))
stub = """<script>
const CFG = %s;
const q = new URLSearchParams(location.search);
const label = q.get("view") || "settings";
window.__TAURI_INTERNALS__ = {
  metadata: { currentWindow: { label }, currentWebview: { label, windowLabel: label } },
  transformCallback: (cb) => { const id = Math.floor(Math.random() * 1e9); window["_" + id] = cb; return id; },
  convertFileSrc: (s) => s,
  invoke: async (cmd) => {
    switch (cmd) {
      case "config_read": return CFG;
      case "app_version": return "0.7.2";
      case "autostart_get": return true;
      case "monitors_list": return [];
      case "fonts_list": return [];
      case "plugin:window|is_maximized": return false;
      // A made-up File menu, as Kasumi would have seen it.
      case "context_menu_info": return { default: [{ rest: true }], seen: { file: [
        { text: "Open", verb: "open", glyph: "E8E5" }, { text: "Open with", verb: "openas", sub: true, glyph: "E7AC" },
        { text: "Cut", verb: "cut", glyph: "E8C6" }, { text: "Copy", verb: "copy", glyph: "E8C8" },
        { text: "Rename", verb: "rename", glyph: "E8AC" }, { text: "Delete", verb: "delete", glyph: "E74D" },
        { text: "Properties", verb: "properties", glyph: "E946" } ] } };
      default: return null;
    }
  },
};
// #words: typed into the sidebar's search once the page is up.
window.addEventListener("load", () => setTimeout(() => {
  const words = decodeURIComponent(location.hash.slice(1));
  const s = words && document.querySelector(".nav-search");
  if (s) { s.value = words; s.dispatchEvent(new Event("input")); }
}, 300));
</script>""" % json.dumps(cfg, default=str)
html_path = os.path.join(site, "settings.html")
html = open(html_path, encoding="utf-8").read().replace("<head>", "<head>" + stub, 1)
open(html_path, "w", encoding="utf-8").write(html)

class Quiet(http.server.SimpleHTTPRequestHandler):
    def log_message(self, *args):
        pass


handler = functools.partial(Quiet, directory=site)
server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), handler)
threading.Thread(target=server.serve_forever, daemon=True).start()
port = server.server_address[1]

edge = r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe"
for shot in shots:
    page, _, words = shot.partition("#")
    name = (page + ("-" + words.replace(" ", "_") if words else "")).replace("/", "_")
    url = f"http://127.0.0.1:{port}/settings.html?page={page}" + (f"#{words}" if words else "")
    png = os.path.join(out, name + ".png")
    subprocess.run([edge, "--headless=new", "--disable-gpu", "--hide-scrollbars", f"--user-data-dir={os.path.join(out, 'edge')}",
                    "--window-size=1100,1500", "--virtual-time-budget=4000", f"--screenshot={png}", url], capture_output=True, timeout=60)
    print(png if os.path.exists(png) else f"failed: {shot}")
server.shutdown()
