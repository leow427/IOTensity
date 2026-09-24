//! Run against scripts/esp32-emulator.py --test-api (never against an actual LED).
//! Uses production discovery/control/UDP and a temporary on-disk configuration.
use iotensity_lib::{
    config::{ConfigStore, Configuration, LightOutput},
    hardware::HardwareService,
    sync::processing::LightColor,
};
use std::{
    thread,
    time::{Duration, Instant},
};

const ID: &str = "esp32-020000a1b2c3";
fn online(service: &HardwareService) {
    let start = Instant::now();
    while !service
        .devices()
        .devices
        .iter()
        .any(|d| d.device_id == ID && d.online)
    {
        assert!(
            start.elapsed() < Duration::from_secs(20),
            "mDNS discovery/control timed out: {:?}",
            service.devices()
        );
        thread::sleep(Duration::from_millis(50));
    }
}
fn feed(service: &HardwareService, rgb: [u8; 3], seconds: f64) {
    let start = Instant::now();
    while start.elapsed().as_secs_f64() < seconds {
        service.publish(
            &[LightColor {
                id: "light-bulb".into(),
                rgb,
            }],
            true,
        );
        thread::sleep(Duration::from_millis(20));
    }
}
fn status(client: &reqwest::blocking::Client, endpoint: &str) -> serde_json::Value {
    client
        .get(format!("{endpoint}/v1/status"))
        .send()
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .unwrap()
}
fn main() {
    let endpoint = std::env::args()
        .nth(1)
        .expect("Usage: esp32_probe http://<emulator LAN IP>:8099");
    let client = reqwest::blocking::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(2))
        .build()
        .unwrap();
    let dir = tempfile::tempdir().unwrap();
    let disk = ConfigStore::new(dir.path().join("configuration.json"));
    let mut config: Configuration =
        serde_json::from_str(include_str!("../../tests/fixtures/configuration.json")).unwrap();
    for light in &mut config.rooms[0].lights {
        light.output = LightOutput::Virtual;
    }
    config.rooms[0].lights[0].output = LightOutput::Esp32 {
        device_id: ID.into(),
    };
    let saved = disk.save(config, 0).unwrap();
    let service = HardwareService::spawn(|_| {}).unwrap();
    service.apply_saved(saved);
    online(&service);
    service.identify(ID).unwrap();
    assert!(
        status(&client, &endpoint)["identifyCount"]
            .as_u64()
            .unwrap()
            > 0
    );
    feed(&service, [64, 128, 192], 1.2);
    let first = status(&client, &endpoint);
    assert_eq!(first["rgb"], serde_json::json!([64, 128, 192]));
    let before = first["acceptedFrames"].as_u64().unwrap();
    let measured = Instant::now();
    feed(&service, [64, 128, 192], 2.0);
    let last = status(&client, &endpoint);
    let count = last["acceptedFrames"].as_u64().unwrap() - before;
    let fps = count as f64 / measured.elapsed().as_secs_f64();
    assert!(
        (25.0..=30.5).contains(&fps),
        "Unexpected packet rate: {fps}"
    );
    assert_eq!(last["sessionId"], first["sessionId"]); // Static keepalives preserve session.
                                                       // Exercise the receiver reboot; this endpoint only exists in the test emulator.
    client
        .post(format!("{endpoint}/_test/reboot"))
        .json(&serde_json::json!({"deviceId": ID}))
        .send()
        .unwrap()
        .error_for_status()
        .unwrap();
    feed(&service, [5, 15, 25], 1.5);
    let rebooted = status(&client, &endpoint);
    assert_ne!(rebooted["sessionId"], first["sessionId"]);
    assert_eq!(rebooted["rgb"], serde_json::json!([5, 15, 25]));
    // A temporary outage must retain the assignment and resume only while requested.
    client
        .post(format!("{endpoint}/_test/reboot"))
        .json(&serde_json::json!({"deviceId": ID, "offlineSeconds": 2}))
        .send()
        .unwrap()
        .error_for_status()
        .unwrap();
    feed(&service, [9, 19, 29], 1.2);
    let devices = service.devices();
    let device = devices.devices.iter().find(|d| d.device_id == ID).unwrap();
    assert!(!device.online);
    assert_eq!(device.bound_light_id.as_deref(), Some("light-bulb"));
    feed(&service, [9, 19, 29], 5.0);
    assert_eq!(
        status(&client, &endpoint)["rgb"],
        serde_json::json!([9, 19, 29])
    );
    service.publish(&[], false);
    thread::sleep(Duration::from_millis(750));
    assert!(status(&client, &endpoint)["sessionId"].is_null());
    service.shutdown();
    let restarted = HardwareService::spawn(|_| {}).unwrap();
    restarted.apply_saved(disk.load().unwrap());
    online(&restarted);
    assert!(!restarted.devices().devices.iter().any(|d| d.streaming));
    feed(&restarted, [32, 64, 96], 1.5);
    assert_eq!(
        status(&client, &endpoint)["rgb"],
        serde_json::json!([32, 64, 96])
    );
    assert_eq!(
        restarted
            .devices()
            .devices
            .iter()
            .filter(|d| d.device_id == ID)
            .count(),
        1
    );
    // Stop feeding the source: native watchdog + firmware timeout must turn it off.
    thread::sleep(Duration::from_millis(1500));
    assert!(status(&client, &endpoint)["sessionId"].is_null());
    assert_eq!(
        status(&client, &endpoint)["rgb"],
        serde_json::json!([0, 0, 0])
    );
    restarted.shutdown();
    println!("PASS: mDNS, Identify, exact RGB, {fps:.1} FPS static keepalive, receiver reboot, offline retention/recovery, disk binding/app restart, explicit stop and source timeout");
}
