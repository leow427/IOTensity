use tauri::{Emitter, Manager, WebviewUrl, WebviewWindowBuilder};

fn show_overlay(window: &tauri::WebviewWindow) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        unsafe extern "C" {
            fn io_overlay_show(window: *mut std::ffi::c_void);
        }
        let pointer = window.ns_window().map_err(|error| error.to_string())? as usize;
        window
            .run_on_main_thread(move || unsafe { io_overlay_show(pointer as *mut _) })
            .map_err(|error| error.to_string())
    }
    #[cfg(not(target_os = "macos"))]
    window.show().map_err(|error| error.to_string())
}

#[tauri::command]
pub fn overlay_is_open(app: tauri::AppHandle) -> bool {
    app.get_webview_window("overlay").is_some()
}

#[tauri::command]
pub async fn set_overlay(app: tauri::AppHandle, open: bool) -> Result<(), String> {
    if !open {
        if let Some(window) = app.get_webview_window("overlay") {
            window.destroy().map_err(|error| error.to_string())?;
        }
        return Ok(());
    }
    if let Some(window) = app.get_webview_window("overlay") {
        return show_overlay(&window);
    }
    let mut builder = WebviewWindowBuilder::new(
        &app,
        "overlay",
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
