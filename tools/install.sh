#!/usr/bin/env bash
# Builds StatusNotch and swaps it into the installed copy, then checks it came up.
#   tools/install.sh          typecheck, frontend build, release build, install, check
#   tools/install.sh --test   the same, with the Rust tests first
# Never force-kills: the new instance asks the running one to quit (it hands the
# tray back to Explorer and restores the taskbar on the way out).
set -euo pipefail
export PATH="/c/Program Files/nodejs:$HOME/.cargo/bin:$PATH"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
APP="$LOCALAPPDATA/Programs/StatusNotch"
LOG="$APPDATA/StatusNotch/statusnotch.log"

cd "$ROOT"
npx tsc --noEmit
npm run build 2>&1 | grep -E "error|built in"
cd src-tauri
if [[ "${1:-}" == "--test" ]]; then
  cargo test --features tauri/custom-protocol 2>&1 | grep -E -A6 "^error|test result|FAILED|panicked"
fi
cargo build --release --features tauri/custom-protocol 2>&1 | grep -E "^(warning|error)|Finished"

rm -f "$APP/StatusNotch.old.exe"
mv "$APP/StatusNotch.exe" "$APP/StatusNotch.old.exe"
cp target/release/statusnotch.exe "$APP/StatusNotch.exe"
START=$(wc -l < "$LOG")
powershell -NoProfile -Command "Start-Process -FilePath '$APP\StatusNotch.exe' -WorkingDirectory '$APP'; Start-Sleep 10"

# Each overlay window: bars ~345 px tall, docks ~709 (room for panels). Only
# 78 / 93 px means the page's script never ran (nothing drawn): see the log.
powershell -NoProfile -ExecutionPolicy Bypass -File "$ROOT/tools/windows.ps1"
# Page errors since this start ("[statusnotch:<window>] ..." lines come from the pages).
tail -n +"$START" "$LOG" | grep "^\[statusnotch:" || echo "no page errors logged"
