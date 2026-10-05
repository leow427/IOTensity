mod activity;
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

// Commands that touch the disk or the sync mutex (held by the output loop while
// it processes a frame) run on the blocking pool. Synchronous Tauri commands
// run on the main thread and would stall the UI, overlay and event loop.
// The frontend serializes saves and sync commands; each body still runs in order.
async fn blocking<T: Send + 'static, E: Send + 'static>(
    task: impl FnOnce() -> Result<T, E> + Send + 'static,
    join_error: impl FnOnce(String) -> E,
) -> Result<T, E> {
    tauri::async_runtime::spawn_blocking(task)
        .await
        .unwrap_or_else(|error| Err(join_error(error.to_string())))
}

fn config_task_failed(error: String) -> ConfigError {
    ConfigError {
        code: "io",
        message: format!("Configuration task failed ({error}). Restart IOTensity."),
    }
}

#[tauri::command]
async fn load_config(app: tauri::AppHandle) -> Result<Configuration, ConfigError> {
    blocking(
        move || {
            let config = app.state::<ConfigStore>().load()?;
            app.state::<sync::SyncService>().apply_saved(config.clone());
            app.state::<hardware::HardwareService>()
                .apply_saved(config.clone());
            Ok(config)
        },
        config_task_failed,
    )
    .await
}

#[tauri::command]
async fn save_config(
    app: tauri::AppHandle,
    config: Configuration,
    expected_revision: u64,
) -> Result<Configuration, ConfigError> {
    blocking(
        move || {
            let saved = app.state::<ConfigStore>().save(config, expected_revision)?;
            app.state::<sync::SyncService>().apply_saved(saved.clone());
            app.state::<hardware::HardwareService>()
                .apply_saved(saved.clone());
            let _ = app.emit("configuration-saved", &saved);
            Ok(saved)
        },
        config_task_failed,
    )
    .await
}

#[tauri::command]
async fn sync_snapshot(
    sync: tauri::State<'_, sync::SyncService>,
) -> Result<sync::Snapshot, String> {
    let sync = sync.inner().clone();
    blocking(move || Ok(sync.snapshot()), |error| error).await
}
#[tauri::command]
async fn start_sync(
    sync: tauri::State<'_, sync::SyncService>,
    source: sync::Source,
    reduced_motion: Option<bool>,
) -> Result<sync::Snapshot, String> {
    let sync = sync.inner().clone();
    blocking(
        move || {
            sync.set_reduced_motion(reduced_motion.unwrap_or(false));
            sync.start(source)
        },
        |error| error,
    )
    .await
}
#[tauri::command]
async fn set_reduced_motion(
    sync: tauri::State<'_, sync::SyncService>,
    reduced_motion: bool,
) -> Result<(), String> {
    let sync = sync.inner().clone();
    blocking(
        move || {
            sync.set_reduced_motion(reduced_motion);
            Ok(())
        },
        |error| error,
    )
    .await
}
#[tauri::command]
async fn stop_sync(
    sync: tauri::State<'_, sync::SyncService>,
    hardware: tauri::State<'_, hardware::HardwareService>,
) -> Result<sync::Snapshot, String> {
    let sync = sync.inner().clone();
    let hardware = hardware.inner().clone();
    blocking(
        move || {
            let _ = hardware.preview(None);
            Ok(sync.stop())
        },
        |error| error,
    )
    .await
}

// Brief in-memory hardware state updates; preview stays on the main thread so
// requests apply in arrival order.
#[tauri::command]
fn set_light_preview(
    window: tauri::WebviewWindow,
    hardware: tauri::State<'_, hardware::HardwareService>,
    preview: Option<hardware::preview::PreviewRequest>,
) -> Result<(), String> {
    if window.label() != "main" {
        return Err("Light positioning is only available in the main window.".into());
    }
    hardware.preview(preview)
}

#[tauri::command]
fn hardware_snapshot(
    hardware: tauri::State<'_, hardware::HardwareService>,
) -> hardware::DevicesSnapshot {
    hardware.devices()
}

#[tauri::command]
fn retry_hardware_discovery(hardware: tauri::State<'_, hardware::HardwareService>) {
    hardware.retry_discovery();
}
#[tauri::command]
async fn identify_device(
    hardware: tauri::State<'_, hardware::HardwareService>,
    device_id: String,
) -> Result<(), String> {
    let hardware = hardware.inner().clone();
    blocking(move || hardware.identify(&device_id), |error| error).await
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
            retry_hardware_discovery,
            identify_device,
            set_light_preview,
            load_config,
            save_config,
            sync_snapshot,
            start_sync,
            set_reduced_motion,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocking_commands_return_task_results_and_config_error_shapes() {
        let ok = tauri::async_runtime::block_on(blocking(|| Ok::<_, String>(7), |e| e));
        assert_eq!(ok, Ok(7));
        let failed = tauri::async_runtime::block_on(blocking(
            || {
                Err::<(), _>(ConfigError {
                    code: "conflict",
                    message: "kept".into(),
                })
            },
            config_task_failed,
        ));
        assert_eq!(failed.unwrap_err().code, "conflict");
        // A panicking task becomes a serializable error instead of a dropped reply.
        let panicked = tauri::async_runtime::block_on(blocking(
            || -> Result<(), ConfigError> { panic!("task panicked") },
            config_task_failed,
        ));
        let error = panicked.unwrap_err();
        assert_eq!(error.code, "io");
        assert!(error.message.ends_with("Restart IOTensity."));
    }
}
