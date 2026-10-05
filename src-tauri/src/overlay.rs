use std::sync::{Mutex, PoisonError};

use tauri::{Emitter, Manager, WebviewUrl, WebviewWindowBuilder};

const OVERLAY_LABEL: &str = "overlay";

// Serialises open/close transitions so concurrent commands cannot race between
// the existence check and the window build or destroy dispatch.
static OVERLAY_TRANSITION: Mutex<()> = Mutex::new(());

fn show_overlay(window: &tauri::WebviewWindow) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        unsafe extern "C" {
            fn io_overlay_reshow() -> bool;
            fn io_overlay_show(window: *mut std::ffi::c_void);
        }
        // Resolve the NSWindow on the main thread, where it is used. A close
        // queued before this task destroys the window, so look it up again.
        let app = window.app_handle().clone();
        window
            .run_on_main_thread(move || {
                // Once hosted, the webview lives in the panel and the owning
                // window has no content view to resolve, so reuse the panel.
                if unsafe { io_overlay_reshow() } {
                    return;
                }
                let Some(window) = app.get_webview_window(OVERLAY_LABEL) else {
                    return;
                };
                if let Ok(pointer) = window.ns_window() {
                    unsafe { io_overlay_show(pointer) }
                }
            })
            .map_err(|error| error.to_string())
    }
    #[cfg(not(target_os = "macos"))]
    window.show().map_err(|error| error.to_string())
}

#[tauri::command]
pub fn overlay_is_open(app: tauri::AppHandle) -> bool {
    app.get_webview_window(OVERLAY_LABEL).is_some()
}

#[tauri::command]
pub async fn set_overlay(app: tauri::AppHandle, open: bool) -> Result<(), String> {
    transition_overlay(&app, open)
}

fn transition_overlay(app: &tauri::AppHandle, open: bool) -> Result<(), String> {
    let _transition = OVERLAY_TRANSITION
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    if !open {
        if let Some(window) = app.get_webview_window(OVERLAY_LABEL) {
            window.destroy().map_err(|error| error.to_string())?;
        }
        return Ok(());
    }
    if let Some(window) = app.get_webview_window(OVERLAY_LABEL) {
        return show_overlay(&window);
    }
    let mut builder = WebviewWindowBuilder::new(
        app,
        OVERLAY_LABEL,
        WebviewUrl::App("index.html?overlay=1".into()),
    )
    .title("IOTensity mini room")
    .inner_size(360.0, 310.0)
    .resizable(false)
    .decorations(false)
    .always_on_top(true)
    .visible_on_all_workspaces(true)
    .skip_taskbar(true)
    .focused(false)
    .visible(!cfg!(target_os = "macos"));
    if let Ok(Some(monitor)) = app.primary_monitor() {
        let scale = monitor.scale_factor();
        builder = builder.position(
            (monitor.position().x as f64 + monitor.size().width as f64) / scale - 378.0,
            monitor.position().y as f64 / scale + 50.0,
        );
    }
    let window = builder.build().map_err(|error| error.to_string())?;
    show_overlay(&window)?;
    let _ = app.emit("overlay-changed", true);
    Ok(())
}
