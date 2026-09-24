use serde::{Deserialize, Serialize};
use std::{collections::HashSet, fs, io::Write, path::PathBuf, sync::Mutex};

pub const X_BOUNDS: (f64, f64) = (-3.0, 3.0);
pub const Y_BOUNDS: (f64, f64) = (0.15, 3.0);
const MAX_SAFE_REVISION: u64 = 9_007_199_254_740_991;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Configuration {
    pub schema_version: u32,
    pub revision: u64,
    pub rooms: Vec<Room>,
    pub preferences: Preferences,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Room {
    pub id: String,
    pub name: String,
    pub lights: Vec<Light>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Light {
    pub id: String,
    pub name: String,
    pub position: Position,
    pub icon_kind: IconKind,
    pub output: LightOutput,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum IconKind {
    Bulb,
    Bar,
    Strip,
    Lamp,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Position {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Preferences {
    pub brightness: u8,
    pub intensity: Intensity,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Intensity {
    Subtle,
    Balanced,
    Vivid,
    Punch,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ConfigError {
    pub code: &'static str,
    pub message: String,
}

impl ConfigError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
    fn io(error: std::io::Error) -> Self {
        Self::new("io", format!("Cannot access the configuration: {error}. Check the application data folder permissions, then retry. Your previous file has been preserved."))
    }
}

impl Default for Configuration {
    fn default() -> Self {
        Self {
            schema_version: 2,
            revision: 0,
            rooms: vec![Room {
                id: "studio".into(),
                name: "Studio".into(),
                lights: vec![],
            }],
            preferences: Preferences {
                brightness: 75,
                intensity: Intensity::Balanced,
            },
        }
    }
}

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
}
fn valid_name(name: &str) -> bool {
    !name.trim().is_empty()
        && name.len() <= 64
        && !name.chars().any(|c| c <= '\u{1f}' || c == '\u{7f}')
}

pub fn validate(config: &Configuration) -> Result<(), ConfigError> {
    let invalid = |message| Err(ConfigError::new("invalid", message));
    if config.schema_version != 2 {
        return Err(ConfigError::new("version", "Unsupported configuration version. The file has not been changed. Use a compatible version of IOTensity."));
    }
    if config.revision > MAX_SAFE_REVISION {
        return invalid("Invalid configuration revision.");
    }
    if config.rooms.is_empty() || config.rooms.len() > 16 {
        return invalid("Expected 1–16 rooms.");
    }
    if config.preferences.brightness > 100 {
        return invalid("Brightness must be between 0 and 100.");
    }
    let mut rooms = HashSet::new();
    let mut lights = HashSet::new();
    let mut devices = HashSet::new();
    for room in &config.rooms {
        if !valid_id(&room.id) || !rooms.insert(&room.id) {
            return invalid("Invalid or duplicate room ID.");
        }
        if !valid_name(&room.name) {
            return invalid("Room names must contain 1–64 bytes of text.");
        }
        if room.lights.len() > 64 {
            return invalid("A room supports at most 64 lights.");
        }
        for light in &room.lights {
            if !valid_id(&light.id) || !lights.insert(&light.id) {
                return invalid("Invalid or duplicate light ID.");
            }
            if !valid_name(&light.name) {
                return invalid("Light names must contain 1–64 bytes of text.");
            }
            if let LightOutput::Esp32 { device_id } = &light.output {
                if !valid_device_id(device_id) || !devices.insert(device_id) || devices.len() > 64 {
                    return invalid("Invalid or already bound hardware ID.");
                }
            }
            let p = &light.position;
            if !p.x.is_finite()
                || !p.y.is_finite()
                || !p.z.is_finite()
                || !(X_BOUNDS.0..=X_BOUNDS.1).contains(&p.x)
                || !(Y_BOUNDS.0..=Y_BOUNDS.1).contains(&p.y)
                || !(-0.7..=4.0).contains(&p.z)
            {
                return invalid("Light position is outside the room bounds.");
            }
        }
    }
    Ok(())
}

// No native live editor state. The mutex serializes file transactions only.
pub struct ConfigStore {
    path: PathBuf,
    gate: Mutex<()>,
}

impl ConfigStore {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            gate: Mutex::new(()),
        }
    }

    fn read(&self) -> Result<Configuration, ConfigError> {
        let metadata = match fs::metadata(&self.path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Configuration::default())
            }
            Err(error) => return Err(ConfigError::io(error)),
        };
        if metadata.len() > 1_048_576 {
            return Err(ConfigError::new(
                "invalid",
                "Configuration exceeds 1 MB. The file has not been changed.",
            ));
        }
        let bytes = fs::read(&self.path).map_err(ConfigError::io)?;
        let value: serde_json::Value = serde_json::from_slice(&bytes).map_err(|error| ConfigError::new("invalid", format!("Configuration is malformed ({error}). Restore a valid backup in the application data folder, then retry. The file has not been changed.")))?;
        decode_configuration(value)
    }

    pub fn load(&self) -> Result<Configuration, ConfigError> {
        let _guard = self
            .gate
            .lock()
            .map_err(|_| ConfigError::new("io", "Configuration lock failed. Restart IOTensity."))?;
        self.read()
    }

    pub fn save(
        &self,
        mut config: Configuration,
        expected_revision: u64,
    ) -> Result<Configuration, ConfigError> {
        let _guard = self
            .gate
            .lock()
            .map_err(|_| ConfigError::new("io", "Configuration lock failed. Restart IOTensity."))?;
        validate(&config)?;
        // Reading first also prevents a broken/unsupported file being silently overwritten.
        let current = self.read()?;
        if current.revision != expected_revision || config.revision != expected_revision {
            return Err(ConfigError::new("conflict", "Configuration changed since it was loaded. Your draft is intact. Restart IOTensity to load the saved version before trying again."));
        }
        if expected_revision >= MAX_SAFE_REVISION {
            return Err(ConfigError::new(
                "invalid",
                "Configuration revision limit reached. The existing file has been preserved.",
            ));
        }
        config.revision = expected_revision + 1;
        let parent = self
            .path
            .parent()
            .ok_or_else(|| ConfigError::new("io", "Invalid configuration directory."))?;
        fs::create_dir_all(parent).map_err(ConfigError::io)?;
        let bytes = serde_json::to_vec_pretty(&config)
            .map_err(|error| ConfigError::new("invalid", error.to_string()))?;
        let mut temporary = tempfile::NamedTempFile::new_in(parent).map_err(ConfigError::io)?;
        temporary.write_all(&bytes).map_err(ConfigError::io)?;
        temporary.write_all(b"\n").map_err(ConfigError::io)?;
        temporary.as_file().sync_all().map_err(ConfigError::io)?;
        // Same-directory atomic replacement on both macOS and Windows. Failed writes
        // leave the old file untouched; temporary files clean themselves up.
        temporary
            .persist(&self.path)
            .map_err(|error| ConfigError::io(error.error))?;
        Ok(config)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "lowercase", deny_unknown_fields)]
pub enum LightOutput {
    Virtual,
    Esp32 {
        #[serde(rename = "deviceId")]
        device_id: String,
    },
}

pub fn valid_device_id(id: &str) -> bool {
    id.len() == 18
        && id.starts_with("esp32-")
        && id[6..]
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

pub fn decode_configuration(mut value: serde_json::Value) -> Result<Configuration, ConfigError> {
    let invalid = || {
        ConfigError::new(
            "invalid",
            "Invalid legacy configuration. The file has not been changed.",
        )
    };
    match value.get("schemaVersion").and_then(|v| v.as_u64()) {
        Some(1) => {
            let rooms = value
                .get_mut("rooms")
                .and_then(|v| v.as_array_mut())
                .ok_or_else(invalid)?;
            for room in rooms {
                let lights = room
                    .get_mut("lights")
                    .and_then(|v| v.as_array_mut())
                    .ok_or_else(invalid)?;
                for light in lights {
                    let light = light.as_object_mut().ok_or_else(invalid)?;
                    if light.contains_key("output") {
                        return Err(invalid());
                    }
                    light.insert("output".into(), serde_json::json!({"kind":"virtual"}));
                }
            }
            value["schemaVersion"] = 2.into();
        }
        Some(2) => {}
        _ => {
            return Err(ConfigError::new(
                "version",
                "Unsupported configuration version. The file has not been changed.",
            ))
        }
    }
    let config: Configuration = serde_json::from_value(value).map_err(|error| {
        ConfigError::new(
            "invalid",
            format!("Invalid configuration: {error}. The file has not been changed."),
        )
    })?;
    validate(&config)?;
    Ok(config)
}
