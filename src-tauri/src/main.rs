#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod actions;
mod autostart;
mod config;
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
mod win;

fn main() {
    // The keyboard hook's helper process (keyhook.rs): nothing else of the app.
    let args: Vec<String> = std::env::args().collect();
    if let Some(i) = args.iter().position(|a| a == "--keyhook") {
        keyhook::helper(args.get(i + 1).and_then(|p| p.parse().ok()).unwrap_or(0));
        return;
    }
    // The context menu's helper process (native/ctxmenu): nothing else of the app.
    if let Some(i) = args.iter().position(|a| a == "--menu") {
        native::ctxmenu::helper(args.get(i + 1).and_then(|p| p.parse().ok()).unwrap_or(0));
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
        .invoke_handler(tauri::generate_handler![engine::fe_log, actions::run_action, config_edit::config_read, config_edit::config_write, config_edit::config_reset, autostart::autostart_get, autostart::autostart_set, settings::open_settings, settings::open_setup_window, settings::open_notes_window, updater::release_notes, presets::presets_shipped, settings::app_version, updater::update_check, updater::update_install, settings::monitors_list, settings::fonts_list, settings::lhm_sensors, config_edit::config_write_many, presets::presets_list, presets::preset_save, presets::preset_apply, presets::preset_rename, presets::preset_delete, providers::extras::disk_list, providers::notifications::notifications_windows_on, providers::notifications::taskbar_all_displays, native::ctxmenu::context_menu_info])
        .setup(|app| {
            // A first run: no config yet (the engine writes the defaults when it starts).
            let first_run = !config::path().exists();
            presets::install_shipped();
            taskbar::start();
            corners::start();
            launcher::start(app.handle());
            // The bars: drawn natively (Direct2D) on a thread of their own.
            native::start(app.handle());
            engine::start(app.handle());
            osd::start(app.handle());
            keyhook::start(osd::volume_key);
            keyhook::on_win_digit(native::win_key);
            keyhook::on_alt_tab(native::alt_tab);
            keyhook::on_lang(native::lang_switch);
            native::ctxmenu::watch(app.handle().clone());
            native::apptips::start(app.handle());
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
