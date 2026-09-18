pub mod config;

use config::{ConfigError, ConfigStore, Configuration};
use tauri::Manager;

#[tauri::command]
fn load_config(store: tauri::State<'_, ConfigStore>) -> Result<Configuration, ConfigError> {
    store.load()
}

#[tauri::command]
fn save_config(
    store: tauri::State<'_, ConfigStore>,
    config: Configuration,
    expected_revision: u64,
) -> Result<Configuration, ConfigError> {
    store.save(config, expected_revision)
}

pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            let path = app.path().app_data_dir()?.join("configuration.json");
            app.manage(ConfigStore::new(path));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![load_config, save_config])
        .run(tauri::generate_context!())
        .expect("IOTensity could not start");
}
