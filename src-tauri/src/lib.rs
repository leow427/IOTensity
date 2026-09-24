pub mod config;
pub mod hardware;
mod overlay;
pub mod sync;

use config::{ConfigError, ConfigStore, Configuration};
use tauri::{Emitter, Manager};

fn request_main_close(app: &tauri::AppHandle) {
    if let Some(main) = app.get_webview_window("main") {
        let _ = main.show();
        let _ = main.set_focus();
        let _ = main.close();
    }
}

#[cfg(target_os = "macos")]
fn install_guarded_quit(app: &tauri::AppHandle) -> tauri::Result<()> {
    use tauri::menu::{Menu, MenuItem, SubmenuBuilder};
    // AppKit's predefined Quit calls terminate: directly, bypassing Tauri's
    // ExitRequested interception. Route the menu and Cmd+Q through window close.
    let menu = Menu::default(app)?;
    let application = SubmenuBuilder::new(app, "IOTensity")
        .about(None)
        .separator()
        .services()
        .separator()
        .hide()
        .hide_others()
        .show_all()
        .separator()
        .item(&MenuItem::with_id(
            app,
            "guarded-quit",
            "Quit IOTensity",
            true,
            Some("CmdOrCtrl+Q"),
        )?)
        .build()?;
    // Preserve Tauri's File, Edit, View, Window and Help menus unchanged.
    menu.remove_at(0)?;
    menu.insert(&application, 0)?;
    app.set_menu(menu)?;
    Ok(())
}

#[tauri::command]
fn load_config(
    store: tauri::State<'_, ConfigStore>,
    sync: tauri::State<'_, sync::SyncService>,
    hardware: tauri::State<'_, hardware::HardwareService>,
) -> Result<Configuration, ConfigError> {
    let config = store.load()?;
    sync.apply_saved(config.clone());
    hardware.apply_saved(config.clone());
    Ok(config)
}

#[tauri::command]
fn save_config(
    store: tauri::State<'_, ConfigStore>,
    app: tauri::AppHandle,
    config: Configuration,
    expected_revision: u64,
    sync: tauri::State<'_, sync::SyncService>,
    hardware: tauri::State<'_, hardware::HardwareService>,
) -> Result<Configuration, ConfigError> {
    let saved = store.save(config, expected_revision)?;
    sync.apply_saved(saved.clone());
    hardware.apply_saved(saved.clone());
    let _ = app.emit("configuration-saved", &saved);
    Ok(saved)
}

#[tauri::command]
fn sync_snapshot(sync: tauri::State<'_, sync::SyncService>) -> sync::Snapshot {
    sync.snapshot()
}
#[tauri::command]
fn start_sync(
    sync: tauri::State<'_, sync::SyncService>,
    source: sync::Source,
    reduced_motion: Option<bool>,
) -> Result<sync::Snapshot, String> {
    sync.set_reduced_motion(reduced_motion.unwrap_or(false));
    sync.start(source)
}
#[tauri::command]
fn stop_sync(sync: tauri::State<'_, sync::SyncService>) -> sync::Snapshot {
    sync.stop()
}

#[tauri::command]
fn hardware_snapshot(
    hardware: tauri::State<'_, hardware::HardwareService>,
) -> hardware::DevicesSnapshot {
    hardware.devices()
}
#[tauri::command]
async fn identify_device(
    hardware: tauri::State<'_, hardware::HardwareService>,
    device_id: String,
) -> Result<(), String> {
    let hardware = hardware.inner().clone();
    tauri::async_runtime::spawn_blocking(move || hardware.identify(&device_id))
        .await
        .map_err(|e| e.to_string())?
}

pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            #[cfg(target_os = "macos")]
            install_guarded_quit(app.handle())?;
            let path = app.path().app_data_dir()?.join("configuration.json");
            app.manage(ConfigStore::new(path));
            let devices_app = app.handle().clone();
            let hardware = hardware::HardwareService::spawn(move |devices| {
                let _ = devices_app.emit("hardware-devices", devices);
            })
            .map_err(std::io::Error::other)?;
            let sync = sync::SyncService::default();
            sync.spawn(app.handle().clone(), hardware.clone());
            app.manage(hardware);
            app.manage(sync);
            Ok(())
        })
        .on_menu_event(|app, event| {
            if event.id() == "guarded-quit" {
                request_main_close(app);
            }
        })
        .on_window_event(|window, event| {
            if matches!(event, tauri::WindowEvent::Destroyed) {
                if window.label() == "main" {
                    window.state::<sync::SyncService>().shutdown();
                    window.state::<hardware::HardwareService>().shutdown();
                    if let Some(overlay) = window.app_handle().get_webview_window("overlay") {
                        let _ = overlay.destroy();
                    }
                    window.app_handle().exit(0);
                } else if window.label() == "overlay" {
                    let _ = window.app_handle().emit("overlay-changed", false);
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            hardware_snapshot,
            identify_device,
            load_config,
            save_config,
            sync_snapshot,
            start_sync,
            stop_sync,
            overlay::set_overlay,
            overlay::overlay_is_open
        ])
        .build(tauri::generate_context!())
        .expect("IOTensity could not start")
        .run(|app, event| match event {
            tauri::RunEvent::ExitRequested { api, .. }
                if !app.state::<sync::SyncService>().is_shutdown() =>
            {
                if app.get_webview_window("main").is_some() {
                    api.prevent_exit();
                    request_main_close(app);
                }
            }
            tauri::RunEvent::Exit => {
                app.state::<sync::SyncService>().shutdown();
                app.state::<hardware::HardwareService>().shutdown();
            }
            _ => {}
        });
}
