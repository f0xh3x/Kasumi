//! Volume: the keyboard helper (keyhook.rs) takes over the volume keys (so the
//! Windows flyout never appears) and an audio thread applies them through Core Audio.
//! A Core Audio volume callback shows the OSD for every master-volume change,
//! whatever caused it.

use crate::providers::SharedConfig;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Manager};
use windows::core::{implement, GUID};
use windows::Win32::Media::Audio::Endpoints::{
    IAudioEndpointVolume, IAudioEndpointVolumeCallback, IAudioEndpointVolumeCallback_Impl,
};
use windows::Win32::Media::Audio::{
    eConsole, eRender, IMMDeviceEnumerator, MMDeviceEnumerator, AUDIO_VOLUME_NOTIFICATION_DATA,
};
use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CLSCTX_ALL, COINIT_MULTITHREADED};

#[derive(Clone, Copy)]
enum Key {
    Up,
    Down,
    Mute,
}

static KEY_TX: OnceLock<Mutex<Sender<Key>>> = OnceLock::new();

pub fn start(app: &AppHandle) {
    let (tx, rx) = mpsc::channel();
    let _ = KEY_TX.set(Mutex::new(tx));
    let a = app.clone();
    std::thread::Builder::new().name("kasumi-audio".into()).spawn(move || run_audio(a, rx)).expect("spawn audio");
}

/// A volume key press from the keyboard helper (keyhook.rs), for the audio thread.
pub(super) fn key(key: crate::keyhook::VolumeKey) {
    use crate::keyhook::VolumeKey;
    let key = match key {
        VolumeKey::Up => Key::Up,
        VolumeKey::Down => Key::Down,
        VolumeKey::Mute => Key::Mute,
    };
    if let Some(tx) = KEY_TX.get() {
        let _ = tx.lock().unwrap().send(key);
    }
}

#[implement(IAudioEndpointVolumeCallback)]
struct VolumeCallback {
    app: AppHandle,
}

impl IAudioEndpointVolumeCallback_Impl for VolumeCallback_Impl {
    fn OnNotify(&self, data: *mut AUDIO_VOLUME_NOTIFICATION_DATA) -> windows::core::Result<()> {
        if let Some(d) = unsafe { data.as_ref() } {
            // Quick settings shows the volume too: it gets it from here.
            crate::providers::system::volume_changed(Some((d.fMasterVolume, d.bMuted.as_bool())));
            super::show(&self.app, "volume", d.fMasterVolume, d.bMuted.as_bool());
        }
        Ok(())
    }
}

struct Endpoint {
    id: String,
    volume: IAudioEndpointVolume,
    callback: IAudioEndpointVolumeCallback,
}

impl Drop for Endpoint {
    fn drop(&mut self) {
        unsafe {
            let _ = self.volume.UnregisterControlChangeNotify(&self.callback);
        }
    }
}

/// Binds to the current default output device, keeping the existing binding if unchanged.
fn bind(app: &AppHandle, enumerator: &IMMDeviceEnumerator, current: Option<Endpoint>) -> windows::core::Result<Endpoint> {
    unsafe {
        let device = enumerator.GetDefaultAudioEndpoint(eRender, eConsole)?;
        let id = device.GetId()?.to_string().unwrap_or_default();
        if let Some(ep) = current {
            if ep.id == id {
                return Ok(ep);
            }
        }
        let volume: IAudioEndpointVolume = device.Activate(CLSCTX_ALL, None)?;
        let callback: IAudioEndpointVolumeCallback = VolumeCallback { app: app.clone() }.into();
        volume.RegisterControlChangeNotify(&callback)?;
        // A new output device: quick settings starts from its current volume.
        crate::providers::system::volume_changed(Some((volume.GetMasterVolumeLevelScalar()?, volume.GetMute()?.as_bool())));
        Ok(Endpoint { id, volume, callback })
    }
}

fn apply(app: &AppHandle, vol: &IAudioEndpointVolume, key: Key, step_pct: f64) -> windows::core::Result<()> {
    unsafe {
        let muted = vol.GetMute()?.as_bool();
        let ctx: *const GUID = std::ptr::null();
        let mut level = vol.GetMasterVolumeLevelScalar()?;
        match key {
            Key::Mute => vol.SetMute(!muted, ctx)?,
            Key::Up | Key::Down => {
                // Like Windows: changing the volume unmutes.
                if muted {
                    vol.SetMute(false, ctx)?;
                }
                let step = (step_pct / 100.0).clamp(0.005, 0.5) as f32;
                let dir = if matches!(key, Key::Up) { 1.0 } else { -1.0 };
                level = (((level / step).round() + dir) * step).clamp(0.0, 1.0);
                vol.SetMasterVolumeLevelScalar(level, ctx)?;
            }
        }
        // At 0% or 100% nothing changes and no callback fires; show the OSD anyway.
        let muted_now = matches!(key, Key::Mute) && !muted;
        super::show(app, "volume", level, muted_now);
        Ok(())
    }
}

fn run_audio(app: AppHandle, rx: Receiver<Key>) {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
    }
    let enumerator: IMMDeviceEnumerator = match unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL) } {
        Ok(e) => e,
        Err(e) => {
            eprintln!("[kasumi] Core Audio unavailable: {e}");
            return;
        }
    };
    let mut endpoint: Option<Endpoint> = None;
    let mut last_bind = Instant::now() - Duration::from_secs(60);
    loop {
        let (replace, step) = {
            let cfg = app.state::<SharedConfig>();
            let cfg = cfg.read().unwrap();
            (cfg.osd.enabled && cfg.osd.replace_volume_keys, cfg.osd.volume_step)
        };
        crate::keyhook::set_replace_volume(replace && endpoint.is_some());

        // Follow default-device changes (headphones plugged in, etc.).
        if last_bind.elapsed() >= Duration::from_secs(2) {
            last_bind = Instant::now();
            endpoint = match bind(&app, &enumerator, endpoint.take()) {
                Ok(ep) => Some(ep),
                Err(e) => {
                    eprintln!("[kasumi] no default audio device: {e}");
                    crate::providers::system::volume_changed(None);
                    None
                }
            };
        }

        match rx.recv_timeout(Duration::from_millis(500)) {
            Ok(key) => {
                if let Some(ep) = &endpoint {
                    if let Err(e) = apply(&app, &ep.volume, key, step) {
                        eprintln!("[kasumi] volume change failed: {e}");
                    }
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }
    }
}
