//! Claude Code usage, from its local session files (`~/.claude/projects/*/*.jsonl`):
//! every assistant message records its model and token usage. Published as
//! "claude": hourly totals per model for the last 8 days (the page works out
//! today / the week in local time), the current 5-hour window, and whether a
//! session is busy right now (a session file written in the last few seconds).
//! Plan limits aren't stored locally: the widget compares against limits the
//! user sets.

use serde::Serialize;
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::{BufRead, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};
use tauri::{AppHandle, Manager};

const KEEP: i64 = 8 * 24 * 3600;
const WINDOW: i64 = 5 * 3600;
/// A session file written this recently means Claude is working.
const BUSY: Duration = Duration::from_secs(15);

struct Entry {
    ts: i64,
    model: String,
    session: String,
    /// Input + output + cache writes (what the widget counts).
    tokens: u64,
    cache_read: u64,
}

#[derive(Default)]
struct State {
    /// Bytes read so far per file.
    offsets: HashMap<PathBuf, u64>,
    entries: Vec<Entry>,
    /// message id + request id: streamed messages are written more than once.
    seen: HashSet<String>,
}

fn projects_dir() -> PathBuf {
    let base = std::env::var_os("CLAUDE_CONFIG_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(std::env::var_os("USERPROFILE").unwrap_or_default()).join(".claude"));
    base.join("projects")
}

/// Session files changed within the last 8 days, with their modification times.
fn session_files() -> Vec<(PathBuf, SystemTime)> {
    let cutoff = SystemTime::now() - Duration::from_secs(KEEP as u64);
    let mut out = Vec::new();
    let Ok(dirs) = std::fs::read_dir(projects_dir()) else { return out };
    for d in dirs.flatten() {
        let Ok(files) = std::fs::read_dir(d.path()) else { continue };
        for f in files.flatten() {
            let p = f.path();
            if p.extension().and_then(|e| e.to_str()) != Some("jsonl") {
                continue;
            }
            if let Ok(m) = f.metadata().and_then(|m| m.modified()) {
                if m >= cutoff {
                    out.push((p, m));
                }
            }
        }
    }
    out
}

impl State {
    /// Reads what was appended to each file since last time (whole lines only).
    fn update(&mut self, files: &[(PathBuf, SystemTime)]) {
        for (path, _) in files {
            let start = *self.offsets.get(path).unwrap_or(&0);
            let Ok(mut f) = std::fs::File::open(path) else { continue };
            let len = f.metadata().map(|m| m.len()).unwrap_or(0);
            if len < start {
                // Rewritten: read it again.
                self.offsets.insert(path.clone(), 0);
                continue;
            }
            if len == start || f.seek(SeekFrom::Start(start)).is_err() {
                continue;
            }
            // Line by line (session files reach tens of MB): whole lines only, a
            // half-written last line is read next time.
            let mut reader = std::io::BufReader::with_capacity(256 * 1024, f.take(len - start));
            let mut line = Vec::new();
            let mut done = start;
            loop {
                line.clear();
                match reader.read_until(b'\n', &mut line) {
                    Ok(0) | Err(_) => break,
                    Ok(n) if line.last() == Some(&b'\n') => {
                        self.parse(&line, path);
                        done += n as u64;
                    }
                    Ok(_) => break,
                }
            }
            self.offsets.insert(path.clone(), done);
        }
        let cutoff = chrono::Utc::now().timestamp() - KEEP;
        self.entries.retain(|e| e.ts >= cutoff);
    }

    fn parse(&mut self, line: &[u8], path: &Path) {
        // Cheap filter before parsing JSON: only assistant messages with usage.
        if !line.windows(8).any(|w| w == b"\"usage\":") {
            return;
        }
        let Ok(v) = serde_json::from_slice::<Value>(line) else { return };
        if v.get("type").and_then(|t| t.as_str()) != Some("assistant") {
            return;
        }
        let msg = &v["message"];
        let usage = &msg["usage"];
        let id = format!("{}:{}", msg["id"].as_str().unwrap_or(""), v["requestId"].as_str().unwrap_or(""));
        if id != ":" && !self.seen.insert(id) {
            return;
        }
        let Some(ts) = v["timestamp"].as_str().and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok()) else { return };
        let n = |k: &str| usage[k].as_u64().unwrap_or(0);
        let model = msg["model"].as_str().unwrap_or("unknown").to_string();
        if model == "<synthetic>" {
            return;
        }
        let session = v["sessionId"]
            .as_str()
            .map(str::to_string)
            .unwrap_or_else(|| path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default());
        self.entries.push(Entry {
            ts: ts.timestamp(),
            model,
            session,
            tokens: n("input_tokens") + n("output_tokens") + n("cache_creation_input_tokens"),
            cache_read: n("cache_read_input_tokens"),
        });
    }

    fn summary(&self, busy: bool) -> Value {
        #[derive(Serialize, Default)]
        struct Bucket {
            tokens: u64,
            #[serde(rename = "cacheRead")]
            cache_read: u64,
            messages: u32,
            sessions: Vec<String>,
        }
        // Hourly buckets per model (unix hour start).
        let mut hours: BTreeMap<(i64, String), Bucket> = BTreeMap::new();
        for e in &self.entries {
            let b = hours.entry((e.ts - e.ts.rem_euclid(3600), e.model.clone())).or_default();
            b.tokens += e.tokens;
            b.cache_read += e.cache_read;
            b.messages += 1;
            if !b.sessions.contains(&e.session) {
                b.sessions.push(e.session.clone());
            }
        }
        let buckets: Vec<Value> = hours
            .into_iter()
            .map(|((h, model), b)| json!({ "h": h, "model": model, "tokens": b.tokens, "cacheRead": b.cache_read, "messages": b.messages, "sessions": b.sessions }))
            .collect();
        // The current 5-hour window: windows start at the hour of the first message
        // after the previous one ended.
        let mut sorted: Vec<&Entry> = self.entries.iter().collect();
        sorted.sort_by_key(|e| e.ts);
        let mut start: Option<i64> = None;
        for e in &sorted {
            if start.is_none_or(|s| e.ts >= s + WINDOW) {
                start = Some(e.ts - e.ts.rem_euclid(3600));
            }
        }
        let now = chrono::Utc::now().timestamp();
        let window = start.filter(|s| now < s + WINDOW).map(|s| {
            let inside = sorted.iter().filter(|e| e.ts >= s);
            let (tokens, messages) = inside.fold((0u64, 0u32), |(t, m), e| (t + e.tokens, m + 1));
            json!({ "start": s, "end": s + WINDOW, "tokens": tokens, "messages": messages })
        });
        json!({ "buckets": buckets, "window": window, "busy": busy, "now": now })
    }
}

fn placed(app: &AppHandle) -> bool {
    let cfg = app.state::<super::SharedConfig>();
    let cfg = cfg.read().unwrap();
    cfg.widget_placed("claude")
}

pub fn start(app: &AppHandle) {
    let app = app.clone();
    std::thread::Builder::new()
        .name("statusnotch-claude".into())
        .spawn(move || {
            let mut state: Option<State> = None;
            let mut last_scan = Instant::now() - Duration::from_secs(60);
            let mut last_busy = false;
            loop {
                if !placed(&app) {
                    // Not on any bar: drop what was read (it's read again if placed later).
                    state = None;
                    std::thread::sleep(Duration::from_secs(2));
                    continue;
                }
                let files = session_files();
                let now = SystemTime::now();
                let busy = files.iter().any(|(_, m)| now.duration_since(*m).is_ok_and(|d| d < BUSY));
                let st = state.get_or_insert_with(State::default);
                // Re-read every 10 s, or right away when busy changes / while busy (every 2 s).
                if last_scan.elapsed() >= Duration::from_secs(if busy { 2 } else { 10 }) || busy != last_busy {
                    st.update(&files);
                    last_scan = Instant::now();
                    super::publish(&app, "claude", st.summary(busy));
                }
                last_busy = busy;
                std::thread::sleep(Duration::from_secs(1));
            }
        })
        .expect("spawn claude thread");
}
