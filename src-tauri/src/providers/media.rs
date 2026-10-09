//! Now-playing info and transport controls via Windows' System Media
//! Transport Controls (the same session the volume flyout shows).

use super::publish;
use serde_json::{json, Value};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Manager, State};
use windows::core::Interface;
use windows::Media::Control::{
    GlobalSystemMediaTransportControlsSessionManager as SessionManager,
    GlobalSystemMediaTransportControlsSessionMediaProperties as MediaProperties,
    GlobalSystemMediaTransportControlsSessionPlaybackStatus as PlaybackStatus,
};
use windows::Storage::Streams::{DataReader, IInputStream};

const POLL: Duration = Duration::from_millis(1000);
/// 100 ns ticks between 1601-01-01 (Windows) and 1970-01-01 (Unix).
const EPOCH_DIFF_TICKS: i64 = 116_444_736_000_000_000;

enum Command {
    PlayPause,
    Next,
    Previous,
    /// Position in milliseconds from the start of the track.
    Seek(i64),
}

pub struct MediaSender(Mutex<Sender<Command>>);

#[tauri::command]
pub async fn media_control(
    sender: State<'_, MediaSender>,
    action: String,
    position_ms: Option<f64>,
) -> Result<(), String> {
    let cmd = match action.as_str() {
        "play_pause" => Command::PlayPause,
        "next" => Command::Next,
        "previous" => Command::Previous,
        "seek" => Command::Seek(position_ms.ok_or("seek needs position_ms")?.max(0.0) as i64),
        other => return Err(format!("unknown media action '{other}'")),
    };
    sender.0.lock().unwrap().send(cmd).map_err(|e| e.to_string())
}

pub fn start(app: &AppHandle) {
    let (tx, rx) = mpsc::channel();
    app.manage(MediaSender(Mutex::new(tx)));
    let app = app.clone();
    std::thread::Builder::new()
        .name("statusnotch-media".into())
        .spawn(move || run(app, rx))
        .expect("spawn media thread");
}

/// Album art as a data URL, or None when the player provides none.
fn read_art(props: &MediaProperties) -> Option<String> {
    let reference = props.Thumbnail().ok()?;
    let stream = reference.OpenReadAsync().ok()?.get().ok()?;
    let size = stream.Size().ok()?;
    if size == 0 || size > 8 * 1024 * 1024 {
        return None;
    }
    let mime = stream.ContentType().map(|h| h.to_string()).unwrap_or_default();
    let reader = DataReader::CreateDataReader(&stream.cast::<IInputStream>().ok()?).ok()?;
    reader.LoadAsync(size as u32).ok()?.get().ok()?;
    let mut bytes = vec![0u8; size as usize];
    reader.ReadBytes(&mut bytes).ok()?;
    let mime = if mime.starts_with("image/") { mime } else { "image/png".into() };
    Some(super::data_url(&mime, &bytes))
}

fn unix_ms_now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64)
}

/// Art is read once per track, not on every poll.
#[derive(Default)]
struct ArtCache {
    track: String,
    art: Option<String>,
    /// Players may publish the art a moment after the track changes.
    tries: u8,
}

/// The current session, or `Value::Null` when nothing is playing.
fn snapshot(mgr: &SessionManager, cache: &mut ArtCache) -> windows::core::Result<Value> {
    let Ok(session) = mgr.GetCurrentSession() else { return Ok(Value::Null) };
    let props = session.TryGetMediaPropertiesAsync()?.get()?;
    let info = session.GetPlaybackInfo()?;
    let controls = info.Controls()?;
    let title = props.Title()?.to_string();
    if title.is_empty() {
        return Ok(Value::Null);
    }
    let artist = props.Artist()?.to_string();
    let app = session.SourceAppUserModelId()?.to_string();

    let track = format!("{app}\n{title}\n{artist}");
    if cache.track != track {
        cache.track = track;
        cache.art = None;
        cache.tries = 0;
    }
    if cache.art.is_none() && cache.tries < 5 {
        cache.tries += 1;
        cache.art = read_art(&props);
    }

    // Timeline: position as of LastUpdatedTime; the frontend extrapolates while playing.
    let timeline = session.GetTimelineProperties()?;
    let ticks_to_ms = |t: i64| t / 10_000;
    let start = timeline.StartTime()?.Duration;
    let end = timeline.EndTime()?.Duration;
    let position = timeline.Position()?.Duration;
    let updated = timeline.LastUpdatedTime()?.UniversalTime;
    let updated_at = if updated > EPOCH_DIFF_TICKS { ticks_to_ms(updated - EPOCH_DIFF_TICKS) } else { unix_ms_now() };

    Ok(json!({
        "title": title,
        "artist": artist,
        "playing": info.PlaybackStatus()? == PlaybackStatus::Playing,
        "canPrevious": controls.IsPreviousEnabled()?,
        "canNext": controls.IsNextEnabled()?,
        "canPlayPause": controls.IsPlayPauseToggleEnabled()?,
        "canSeek": controls.IsPlaybackPositionEnabled()? && end > start,
        "positionMs": ticks_to_ms(position - start),
        "durationMs": ticks_to_ms(end - start),
        "updatedAt": updated_at,
        "art": cache.art,
        "app": app,
    }))
}

fn execute(mgr: &SessionManager, cmd: Command) -> windows::core::Result<()> {
    let session = mgr.GetCurrentSession()?;
    let op = match cmd {
        Command::PlayPause => session.TryTogglePlayPauseAsync()?,
        Command::Next => session.TrySkipNextAsync()?,
        Command::Previous => session.TrySkipPreviousAsync()?,
        Command::Seek(ms) => {
            let start = session.GetTimelineProperties()?.StartTime()?.Duration;
            session.TryChangePlaybackPositionAsync(start + ms * 10_000)?
        }
    };
    op.get()?;
    Ok(())
}

fn run(app: AppHandle, rx: Receiver<Command>) {
    let mgr = loop {
        match SessionManager::RequestAsync().and_then(|op| op.get()) {
            Ok(m) => break m,
            Err(e) => {
                eprintln!("[statusnotch] media: session manager unavailable: {e}");
                std::thread::sleep(Duration::from_secs(10));
            }
        }
    };
    let mut cache = ArtCache::default();
    loop {
        match snapshot(&mgr, &mut cache) {
            Ok(v) => publish(&app, "media", v),
            Err(e) => eprintln!("[statusnotch] media: {e}"),
        }
        match rx.recv_timeout(POLL) {
            Ok(cmd) => {
                if let Err(e) = execute(&mgr, cmd) {
                    eprintln!("[statusnotch] media command failed: {e}");
                }
                // Let the player update its state before we read it back.
                std::thread::sleep(Duration::from_millis(150));
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }
    }
}
