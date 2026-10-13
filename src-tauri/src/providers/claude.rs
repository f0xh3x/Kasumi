//! Claude Code usage, from its local session files (`~/.claude/projects/*/*.jsonl`):
//! every assistant message records its model and token usage. Published as
//! "claude": hourly totals per model for the last 8 days (the page works out
//! today / the week in local time), the current 5-hour window, and whether a
//! session is busy right now (a session file written in the last few seconds).
//! Plan limits aren't stored locally: with `[widgets.claude] account` on (default)
//! the plan's usage comes from the Claude account itself (account_usage, what
//! /usage shows); else the widget compares against limits the user sets.

use serde::Serialize;
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::{BufRead, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};
use tauri::AppHandle;

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

/// The plan's usage from the Claude account, as /usage shows it: the 5-hour
/// session and the week (and per-model weeks where the plan has them), each a
/// percentage and when it resets. Read with the sign-in Claude Code keeps on this
/// PC (`~/.claude/.credentials.json`), which is only read here, never refreshed
/// or written (Claude Code renews it whenever it runs): expired, this says so.
/// Why the account's usage couldn't be read: a lasting state the widget shows
/// (signed out, sign-in expired), or a passing one (offline, rate limited: the
/// usage endpoint answers 429 when asked often) to wait out, keeping what was read.
enum AccountMiss {
    Show(Value),
    RetryIn(Duration),
}

fn account_usage() -> Result<Value, AccountMiss> {
    let home = std::env::var_os("USERPROFILE").map(PathBuf::from).unwrap_or_default();
    let Ok(text) = std::fs::read_to_string(home.join(".claude").join(".credentials.json")) else {
        return Err(AccountMiss::Show(json!({ "error": "signed-out" })));
    };
    let creds: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
    let oauth = &creds["claudeAiOauth"];
    let Some(token) = oauth["accessToken"].as_str() else {
        return Err(AccountMiss::Show(json!({ "error": "signed-out" })));
    };
    let now_ms = chrono::Utc::now().timestamp_millis();
    if oauth["expiresAt"].as_i64().is_some_and(|t| t <= now_ms) {
        return Err(AccountMiss::Show(json!({ "error": "expired" })));
    }
    let resp = super::http_agent()
        .get("https://api.anthropic.com/api/oauth/usage")
        .set("Authorization", &format!("Bearer {token}"))
        .set("anthropic-beta", "oauth-2025-04-20")
        .set("User-Agent", "Kasumi")
        .call();
    let body = match resp {
        Ok(r) => r.into_string().map_err(|_| AccountMiss::RetryIn(Duration::from_secs(60)))?,
        Err(ureq::Error::Status(429, _)) => return Err(AccountMiss::RetryIn(Duration::from_secs(300))),
        Err(ureq::Error::Status(401, _)) => return Err(AccountMiss::Show(json!({ "error": "expired" }))),
        Err(_) => return Err(AccountMiss::RetryIn(Duration::from_secs(60))),
    };
    let v: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
    let limit = |key: &str| {
        let l = &v[key];
        l["utilization"].as_f64().map(|p| json!({ "percent": p, "resetsAt": l["resets_at"] }))
    };
    Ok(json!({
        "session": limit("five_hour"),
        "week": limit("seven_day"),
        "weekOpus": limit("seven_day_opus"),
        "weekSonnet": limit("seven_day_sonnet"),
    }))
}

pub fn start(app: &AppHandle) {
    let app = app.clone();
    std::thread::Builder::new()
        .name("kasumi-claude".into())
        .spawn(move || {
            let mut state: Option<State> = None;
            let mut last_scan = Instant::now() - Duration::from_secs(60);
            let mut last_busy = false;
            // The account's usage, asked for every 3 minutes (more often runs into the
            // endpoint's rate limit); after a 429 the wait doubles, up to 30 minutes.
            let mut account = Value::Null;
            let mut next_account = Instant::now();
            let mut limited = 0u32;
            loop {
                if !super::placed(&app, "claude") {
                    // Not on any bar: drop what was read (it's read again if placed later).
                    state = None;
                    std::thread::sleep(Duration::from_secs(2));
                    continue;
                }
                let files = session_files();
                let now = SystemTime::now();
                let busy = files.iter().any(|(_, m)| now.duration_since(*m).is_ok_and(|d| d < BUSY));
                let st = state.get_or_insert_with(State::default);
                let mut changed = false;
                let from_account = super::widget_opts(&app, "claude").get("account").and_then(|v| v.as_bool()) != Some(false);
                if !from_account && !account.is_null() {
                    account = Value::Null;
                    changed = true;
                } else if from_account && Instant::now() >= next_account {
                    let wait = match account_usage() {
                        Ok(a) => {
                            limited = 0;
                            changed = a != account;
                            account = a;
                            Duration::from_secs(180)
                        }
                        Err(AccountMiss::Show(a)) => {
                            changed = a != account;
                            account = a;
                            Duration::from_secs(60)
                        }
                        // The last numbers stay (the widget rolls a limit over once its reset passes).
                        Err(AccountMiss::RetryIn(d)) => {
                            if d >= Duration::from_secs(300) {
                                limited = (limited + 1).min(3);
                                eprintln!("[kasumi] claude usage: rate limited, next try in {} min", 5 << (limited - 1));
                                Duration::from_secs(300 << (limited - 1)).min(Duration::from_secs(1800))
                            } else {
                                d
                            }
                        }
                    };
                    next_account = Instant::now() + wait;
                }
                // Re-read every 10 s, or right away when busy changes / while busy (every 2 s).
                if last_scan.elapsed() >= Duration::from_secs(if busy { 2 } else { 10 }) || busy != last_busy {
                    st.update(&files);
                    last_scan = Instant::now();
                    changed = true;
                }
                if changed {
                    let mut v = st.summary(busy);
                    v["account"] = account.clone();
                    super::publish(&app, "claude", v);
                }
                last_busy = busy;
                std::thread::sleep(Duration::from_secs(1));
            }
        })
        .expect("spawn claude thread");
}
