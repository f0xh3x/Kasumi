"""Creates the GitHub release for a tag and attaches the installer.
Token: from Git Credential Manager (never printed).
  python -I .scratch/release.py v0.4.0 <installer path> <notes file>"""
import json
import os
import subprocess
import sys
import urllib.request

REPO = "f0xh3x/Kasumi"
tag, installer, notes_file = sys.argv[1], sys.argv[2], sys.argv[3]

out = subprocess.run(["git", "credential", "fill"], input="protocol=https\nhost=github.com\n\n", capture_output=True, text=True, check=True).stdout
token = dict(l.split("=", 1) for l in out.strip().splitlines() if "=" in l)["password"]


def call(method, url, data=None, ctype="application/json"):
    req = urllib.request.Request(url, data=data, method=method)
    req.add_header("Authorization", f"Bearer {token}")
    req.add_header("Accept", "application/vnd.github+json")
    req.add_header("User-Agent", "Kasumi-release")
    if data is not None:
        req.add_header("Content-Type", ctype)
    with urllib.request.urlopen(req) as r:
        return json.loads(r.read() or b"{}")


notes = open(notes_file, encoding="utf-8").read()
rel = call("POST", f"https://api.github.com/repos/{REPO}/releases", json.dumps({
    "tag_name": tag,
    "name": f"Kasumi {tag.lstrip('v')}",
    "body": notes,
    "make_latest": "true",
}).encode())
print("release:", rel["html_url"])
name = os.path.basename(installer)
upload = rel["upload_url"].split("{")[0] + f"?name={name}"
asset = call("POST", upload, open(installer, "rb").read(), "application/octet-stream")
print("asset:", asset["name"], asset["size"], asset["browser_download_url"])
