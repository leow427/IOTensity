use iotensity_lib::config::{validate, ConfigStore, Configuration};
use std::{fs, sync::Arc};

fn fixture() -> Configuration {
    serde_json::from_str(include_str!("../../tests/fixtures/configuration.json")).unwrap()
}

#[test]
fn shared_frontend_contract_round_trips_every_icon() {
    let config = fixture();
    validate(&config).unwrap();
    let encoded = serde_json::to_value(&config).unwrap();
    // JSON numbers may be rendered as 2 or 2.0; JS has the same numeric value.
    assert_eq!(
        serde_json::from_value::<Configuration>(encoded).unwrap(),
        config
    );
}

#[test]
fn first_use_and_complete_restart_preserve_snapshot() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("data/configuration.json");
    let store = ConfigStore::new(path.clone());
    assert!(store.load().unwrap().rooms[0].lights.is_empty());
    let written = store.save(fixture(), 0).unwrap();
    assert_eq!(written.revision, 1);
    drop(store);
    assert_eq!(ConfigStore::new(path).load().unwrap(), written);
}

#[test]
fn failed_validation_and_stale_saves_preserve_previous_bytes() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("configuration.json");
    let store = ConfigStore::new(path.clone());
    let mut saved = store.save(fixture(), 0).unwrap();
    let previous = fs::read(&path).unwrap();
    saved.rooms[0].lights[0].position.x = 9.0;
    assert_eq!(store.save(saved, 1).unwrap_err().code, "invalid");
    assert_eq!(store.save(fixture(), 0).unwrap_err().code, "conflict");
    assert_eq!(fs::read(path).unwrap(), previous);
}

#[test]
fn malformed_unsupported_and_unreadable_files_are_not_first_use() {
    for content in ["{broken", "{\"schemaVersion\":99}", "{\"schemaVersion\":1}"] {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("configuration.json");
        fs::write(&path, content).unwrap();
        let store = ConfigStore::new(path.clone());
        assert!(store.load().is_err());
        assert!(store.save(fixture(), 0).is_err());
        assert_eq!(fs::read_to_string(path).unwrap(), content);
    }
    let directory = tempfile::tempdir().unwrap();
    assert_eq!(
        ConfigStore::new(directory.path().to_owned())
            .load()
            .unwrap_err()
            .code,
        "io"
    );
}

#[test]
fn rejects_invalid_ids_names_axes_and_preferences() {
    let mut cases = vec![];
    let mut config = fixture();
    config.rooms[0].lights[0].id = "../outside".into();
    cases.push(config);
    let mut config = fixture();
    config.rooms[0].lights[1].id = config.rooms[0].lights[0].id.clone();
    cases.push(config);
    let mut config = fixture();
    config.rooms[0].lights[0].name = "  ".into();
    cases.push(config);
    let mut config = fixture();
    config.rooms[0].lights[0].position.y = f64::NAN;
    cases.push(config);
    let mut config = fixture();
    config.rooms[0].lights[0].position.z = f64::INFINITY;
    cases.push(config);
    let mut config = fixture();
    config.preferences.brightness = 101;
    cases.push(config);
    for config in cases {
        assert_eq!(validate(&config).unwrap_err().code, "invalid");
    }
}

#[test]
fn serde_rejects_unknown_icons_fields_and_fractional_brightness() {
    let config = serde_json::to_value(fixture()).unwrap();
    let mut icon = config.clone();
    icon["rooms"][0]["lights"][0]["iconKind"] = "device".into();
    let mut brightness = config.clone();
    brightness["preferences"]["brightness"] = 1.5.into();
    let mut runtime = config;
    runtime["running"] = true.into();
    for value in [icon, brightness, runtime] {
        assert!(serde_json::from_value::<Configuration>(value).is_err());
    }
}

#[test]
fn concurrent_stale_writers_cannot_overwrite_a_commit() {
    let directory = tempfile::tempdir().unwrap();
    let store = Arc::new(ConfigStore::new(
        directory.path().join("configuration.json"),
    ));
    let handles: Vec<_> = (0..6)
        .map(|_| {
            let store = Arc::clone(&store);
            std::thread::spawn(move || store.save(fixture(), 0))
        })
        .collect();
    let outcomes: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
    assert_eq!(outcomes.iter().filter(|r| r.is_ok()).count(), 1);
    assert_eq!(store.load().unwrap().revision, 1);
}

#[test]
#[cfg(unix)]
fn disk_write_failure_preserves_previous_configuration() {
    use std::os::unix::fs::PermissionsExt;
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("configuration.json");
    let store = ConfigStore::new(path.clone());
    let mut saved = store.save(fixture(), 0).unwrap();
    let previous = fs::read(&path).unwrap();
    fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o500)).unwrap();
    saved.preferences.brightness = 20;
    let result = store.save(saved, 1);
    fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700)).unwrap();
    assert_eq!(result.unwrap_err().code, "io");
    assert_eq!(fs::read(path).unwrap(), previous);
}

#[test]
fn errors_have_a_small_serializable_boundary_shape() {
    let mut config = fixture();
    config.schema_version = 4;
    let error = serde_json::to_value(validate(&config).unwrap_err()).unwrap();
    assert_eq!(error["code"], "version");
    assert!(error["message"]
        .as_str()
        .unwrap()
        .contains("not been changed"));
}
