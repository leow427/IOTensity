use iotensity_lib::{
    config::{decode_configuration, validate, Configuration, LightOutput},
    hardware::{
        control::{Connection, Control, Status, Wish},
        lan_address,
        protocol::{hex, Session},
    },
};
use std::{
    cell::{Cell, RefCell},
    net::{Ipv4Addr, SocketAddrV4},
};
const ID: &str = "esp32-aabbcca1b2c3";
struct FakeControl {
    status: RefCell<Status>,
    requests: RefCell<Vec<(SocketAddrV4, String)>>,
    failed: Cell<bool>,
    lost_response: Cell<bool>,
    stopped: Cell<usize>,
    next_session: Cell<u8>,
}
impl Default for FakeControl {
    fn default() -> Self {
        Self {
            status: RefCell::new(Status {
                device_id: ID.into(),
                model: "esp32-rgb".into(),
                protocol: 1,
                udp_port: 49600,
                max_fps: 30,
                session_id: None,
            }),
            requests: RefCell::new(vec![]),
            failed: Cell::new(false),
            lost_response: Cell::new(false),
            stopped: Cell::new(0),
            next_session: Cell::new(1),
        }
    }
}
impl Control for FakeControl {
    fn status(&self, _: SocketAddrV4) -> Result<Status, String> {
        if self.failed.get() {
            return Err("Unavailable".into());
        }
        Ok(self.status.borrow().clone())
    }
    fn start(
        &self,
        endpoint: SocketAddrV4,
        _: &str,
        _: &str,
        request: &str,
    ) -> Result<Status, String> {
        let retry = self.status.borrow().session_id.is_some()
            && self
                .requests
                .borrow()
                .last()
                .is_some_and(|(_, old)| old == request);
        self.requests.borrow_mut().push((endpoint, request.into()));
        if !retry {
            self.status.borrow_mut().session_id = Some(hex(&[self.next_session.get(); 16]));
            self.next_session.set(self.next_session.get() + 1);
        }
        if self.lost_response.replace(false) {
            return Err("Lost response".into());
        }
        Ok(self.status.borrow().clone())
    }
    fn stop(&self, _: SocketAddrV4, _: &str, _: Session) -> Result<(), String> {
        self.stopped.set(self.stopped.get() + 1);
        self.status.borrow_mut().session_id = None;
        Ok(())
    }
    fn identify(&self, _: SocketAddrV4, _: &str) -> Result<(), String> {
        Ok(())
    }
}
fn wish() -> Wish {
    Wish {
        epoch: 1,
        light_id: Some("logical-room-light".into()),
        running: true,
        endpoints: vec![SocketAddrV4::new(Ipv4Addr::new(192, 168, 1, 20), 80)],
    }
}
#[test]
fn reboot_dhcp_outage_and_stop_preserve_binding_and_renew_sessions() {
    let control = FakeControl::default();
    let mut connection = Connection::new(ID.into(), "computer".into());
    let mut wish = wish();
    let first = connection.step(0, &wish, &control).target.clone().unwrap();
    assert_eq!(first.light_id, "logical-room-light");
    // Static status must not re-start streams on every poll.
    connection.step(500, &wish, &control);
    assert_eq!(control.requests.borrow().len(), 1);
    control.status.borrow_mut().session_id = None; // ESP32 restart
    let reboot = connection
        .step(1000, &wish, &control)
        .target
        .clone()
        .unwrap();
    assert_ne!(reboot.session, first.session);
    wish.endpoints[0].set_ip(Ipv4Addr::new(192, 168, 1, 55)); // DHCP change by full ID
    let moved = connection
        .step(1001, &wish, &control)
        .target
        .clone()
        .unwrap();
    assert_eq!(moved.address.ip(), wish.endpoints[0].ip());
    assert_eq!(moved.light_id, first.light_id);
    assert_ne!(moved.session, reboot.session);
    control.failed.set(true);
    assert!(!connection.step(2000, &wish, &control).online);
    assert!(connection.state.target.is_none());
    control.failed.set(false);
    control.status.borrow_mut().session_id = None;
    assert!(connection.step(7000, &wish, &control).target.is_some());
    wish.running = false;
    wish.epoch += 1;
    assert!(connection.step(7001, &wish, &control).target.is_none());
    assert!(control.stopped.get() >= 2);
    control.status.borrow_mut().session_id = None;
    connection.step(10000, &wish, &control);
    assert!(connection.state.target.is_none()); // Do not resume after explicit stop.
}
#[test]
fn retries_lost_start_ack_with_same_request_and_rejects_ip_reuse() {
    let control = FakeControl::default();
    control.lost_response.set(true);
    let mut connection = Connection::new(ID.into(), "computer".into());
    assert!(connection.step(0, &wish(), &control).target.is_none());
    assert!(connection.step(1000, &wish(), &control).target.is_some());
    let requests = control.requests.borrow();
    assert_eq!(requests[0].1, requests[1].1);
    drop(requests);
    control.status.borrow_mut().device_id = "esp32-ffffffffffff".into();
    assert!(connection.step(2000, &wish(), &control).target.is_none());
    assert!(connection.identify(&control).is_err());
    assert_eq!(control.requests.borrow().len(), 2);
}
#[test]
fn saved_offline_device_and_virtual_output_never_request_streams() {
    let control = FakeControl::default();
    let mut connection = Connection::new(ID.into(), "computer".into());
    let mut wish = wish();
    wish.endpoints.clear();
    assert!(!connection.step(0, &wish, &control).online);
    wish = crate::wish();
    wish.light_id = None;
    assert!(connection.step(1, &wish, &control).online);
    assert!(control.requests.borrow().is_empty());
    assert!(!lan_address(Ipv4Addr::new(8, 8, 8, 8)));
    assert!(!lan_address(Ipv4Addr::LOCALHOST));
    assert!(lan_address(Ipv4Addr::new(192, 168, 1, 20)));
}
#[test]
fn strict_legacy_migration_and_binding_contract() {
    let legacy: serde_json::Value =
        serde_json::from_str(include_str!("../../tests/fixtures/configuration-v1.json")).unwrap();
    let migrated = decode_configuration(legacy.clone()).unwrap();
    assert_eq!(migrated.schema_version, 2);
    assert!(migrated.rooms[0]
        .lights
        .iter()
        .all(|l| l.output == LightOutput::Virtual));
    assert_eq!(
        serde_json::to_value(&migrated).unwrap()["rooms"][0]["lights"][0]["id"],
        legacy["rooms"][0]["lights"][0]["id"]
    );
    let mut bad = legacy.clone();
    bad["rooms"][0]["lights"][0]["ip"] = "192.168.1.5".into();
    assert!(decode_configuration(bad).is_err());
    let mut bad = legacy;
    bad["rooms"][0]["lights"][0]["output"] = serde_json::json!({"kind":"virtual"});
    assert!(decode_configuration(bad).is_err());
    let mut config: Configuration =
        serde_json::from_str(include_str!("../../tests/fixtures/configuration.json")).unwrap();
    config.rooms[0].lights[0].output = LightOutput::Esp32 {
        device_id: ID.into(),
    };
    assert!(validate(&config).is_err()); // fixture already has this full hardware ID
    config.rooms[0].lights[0].output = LightOutput::Esp32 {
        device_id: "IOT-A1B2C3".into(),
    };
    assert!(validate(&config).is_err()); // display suffix is never an identity
    let mut raw = serde_json::to_value(migrated).unwrap();
    raw["rooms"][0]["lights"][0]["output"] =
        serde_json::json!({"kind":"esp32", "deviceId":ID, "online":true});
    assert!(decode_configuration(raw).is_err());
}
#[test]
fn legacy_read_is_non_destructive_then_normal_atomic_save_migrates() {
    use iotensity_lib::config::ConfigStore;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("configuration.json");
    let bytes = include_bytes!("../../tests/fixtures/configuration-v1.json");
    std::fs::write(&path, bytes).unwrap();
    let store = ConfigStore::new(path.clone());
    let migrated = store.load().unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    let saved = store.save(migrated, 0).unwrap();
    assert_eq!(saved.revision, 1);
    assert_eq!(store.load().unwrap(), saved);
    assert!(std::fs::read_to_string(path)
        .unwrap()
        .contains("\"schemaVersion\": 2"));
}
