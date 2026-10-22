#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod actions;
mod autostart;
mod config;
mod dropzone;
mod corners;
mod config_edit;
mod engine;
mod instance;
mod keyhook;
mod launcher;
mod legacy;
mod logfile;
mod native;
mod osd;
mod presets;
mod providers;
mod settings;
mod taskbar;
mod tray;
mod ui;
mod updater;
mod webmem;
mod win;

fn main() {
    // The keyboard hook's helper process (keyhook.rs): nothing else of the app.
    let args: Vec<String> = std::env::args().collect();
    if let Some(i) = args.iter().position(|a| a == "--keyhook") {
        keyhook::helper(args.get(i + 1).and_then(|p| p.parse().ok()).unwrap_or(0));
        return;
    }
    // Native bars drawn off screen into PNGs (native/snapshot.rs): nothing else of the app.
    if args.iter().any(|a| a == "--native-snapshot") {
        native::snapshot::run(&args);
        return;
    }
    legacy::migrate();
    logfile::init();
    legacy::report();
    instance::claim();
    // A panic must never leave the Windows taskbar hidden.
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        taskbar::restore();
        default_hook(info);
    }));
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![engine::get_layout, engine::set_regions, engine::fe_log, engine::set_content_height, engine::set_notch_widths, actions::run_action, providers::get_data, providers::media::media_control, providers::systray::tray_click, osd::switch_keyboard_layout, config_edit::config_read, config_edit::config_write, config_edit::config_reset, autostart::autostart_get, autostart::autostart_set, settings::open_settings, settings::open_setup_window, settings::open_notes_window, updater::release_notes, presets::presets_shipped, settings::app_version, updater::update_check, updater::update_install, settings::monitors_list, settings::fonts_list, settings::lhm_sensors, settings::system_time_format, config_edit::config_write_many, presets::presets_list, presets::preset_save, presets::preset_apply, presets::preset_rename, presets::preset_delete, providers::tasks::task_activate, providers::tasks::task_close, providers::tasks::task_launch, providers::tasks::task_reorder, providers::extras::disk_list, providers::clipboard::clipboard_paste, providers::clipboard::clipboard_remove, providers::clipboard::clipboard_clear, providers::notifications::notifications_seen, providers::notifications::notifications_dismiss, providers::komorebi::komorebi_focus, providers::notifications::notifications_windows_on, providers::system::volume_set, providers::audio::audio_outputs, providers::audio::audio_set_output, providers::audio::audio_sessions, providers::audio::audio_session_set, providers::system::volume_toggle_mute, providers::system::brightness_set, launcher::launcher_apps, launcher::launcher_icon, launcher::launcher_launch, launcher::launcher_user, engine::launcher_open, engine::launcher_close])
        .setup(|app| {
            // A first run: no config yet (the engine writes the defaults when it starts).
            let first_run = !config::path().exists();
            presets::install_shipped();
            taskbar::start();
            corners::start();
            launcher::start(app.handle());
            // [general] renderer: the bars drawn natively instead of as WebView2 pages.
            let native = std::env::args().any(|a| a == "--native") || config::load().is_ok_and(|c| c.general.renderer == "native");
            if native {
                native::start(app.handle());
            }
            engine::start(app.handle());
            osd::start(app.handle());
            keyhook::start(osd::volume_key);
            keyhook::on_win_digit(native::win_key);
            keyhook::on_alt_tab(native::alt_tab);
            autostart::refresh();
            updater::start(app.handle());
            settings::startup(app.handle(), first_run);
            tray::setup(app)?;
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("failed to build Kasumi")
        .run(|_app, event| {
            // Overlays come and go with monitors; only exit on an explicit app.exit().
            match event {
                tauri::RunEvent::ExitRequested { code: None, api, .. } => api.prevent_exit(),
                // Quitting: give the Windows taskbar back.
                tauri::RunEvent::Exit => taskbar::restore(),
                _ => {}
            }
        });
}
