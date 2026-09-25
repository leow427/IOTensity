pub mod control;
mod discovery;
pub mod engine;
pub mod preview;
pub mod protocol;

use crate::config::{valid_device_id, Configuration, LightOutput};
use control::{Connection, ConnectionState, HttpControl, Wish};
use engine::OutputSnapshot;
use mdns_sd::{DaemonEvent, ServiceDaemon, ServiceEvent};
use serde::Serialize;
use std::{
    collections::{BTreeMap, HashMap},
    net::{Ipv4Addr, SocketAddrV4, UdpSocket},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};

pub const SERVICE: &str = "_iotensity._tcp.local.";
pub const FRAME_INTERVAL: Duration = Duration::from_nanos(1_000_000_000 / 30 + 1);
type IdentifyReply = mpsc::SyncSender<Result<(), String>>;
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceView {
    pub device_id: String,
    pub short_id: String,
    pub model: String,
    pub online: bool,
    pub streaming: bool,
    pub message: String,
    pub bound_light_id: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DevicesSnapshot {
    pub devices: Vec<DeviceView>,
    pub discovery_error: Option<String>,
}
#[derive(Default)]
struct Device {
    // Runtime only: neither endpoints nor stream state appear in Configuration.
    advertisements: HashMap<String, Vec<SocketAddrV4>>,
    connection: ConnectionState,
    worker: bool,
    identify: Option<IdentifyReply>,
}
#[derive(Default)]
struct State {
    config: Configuration,
    output: OutputSnapshot,
    epoch: u64,
    reduced_motion: bool,
    devices: BTreeMap<String, Device>,
    discovery_error: Option<String>,
    last_frame: Option<Instant>,
    preview: Option<preview::Preview>,
}
impl State {
    fn color(&self, id: &str) -> Option<(String, [u8; 3])> {
        if let Some(preview) = self.preview.as_ref().filter(|p| p.device_id == id) {
            return Some((
                self.binding(id).unwrap_or_else(|| format!("preview-{id}")),
                preview.rgb,
            ));
        }
        if !self.output.running {
            return None;
        }
        let light_id = self.binding(id)?;
        self.output
            .colors
            .iter()
            .find(|c| c.id == light_id)
            .map(|c| (light_id, c.rgb))
    }
    fn prune_targets(&mut self) {
        let desired: HashMap<_, _> = self
            .devices
            .keys()
            .map(|id| (id.clone(), self.color(id).map(|(light_id, _)| light_id)))
            .collect();
        for (id, device) in &mut self.devices {
            if device
                .connection
                .target
                .as_ref()
                .is_some_and(|target| desired[id].as_ref() != Some(&target.light_id))
            {
                device.connection.target = None;
            }
        }
    }
    fn expire(&mut self, now: Instant) {
        if self.preview.as_ref().is_some_and(|p| now >= p.expires) {
            self.preview = None;
        }
        if self.output.running
            && self
                .last_frame
                .is_none_or(|last| now.duration_since(last) > Duration::from_millis(250))
        {
            self.output.running = false;
            self.epoch += 1;
        }
        self.prune_targets();
    }
    fn set_preview(
        &mut self,
        request: Option<preview::PreviewRequest>,
        now: Instant,
    ) -> Result<(), String> {
        // Even a failed replacement must release the previous selected light.
        self.preview = None;
        let result = (|| {
            if let Some(request) = request {
                let rgb = request.color()?;
                if !self
                    .devices
                    .get(&request.device_id)
                    .is_some_and(|d| d.connection.online)
                {
                    return Err("Light is offline.".into());
                }
                self.preview = Some(preview::Preview {
                    device_id: request.device_id,
                    rgb,
                    expires: now + preview::PREVIEW_DURATION,
                });
            }
            Ok(())
        })();
        self.prune_targets();
        result
    }
    fn reset_discovery(&mut self, network_changed: bool) {
        for device in self.devices.values_mut() {
            if network_changed || !device.connection.online {
                device.advertisements.clear();
                device.connection = ConnectionState::default();
            }
        }
    }
    fn binding(&self, id: &str) -> Option<String> {
        self.config
            .rooms
            .iter()
            .flat_map(|r| &r.lights)
            .find_map(|light| match &light.output {
                LightOutput::Esp32 { device_id } if device_id == id => Some(light.id.clone()),
                _ => None,
            })
    }
    fn wish(&self, id: &str) -> Option<Wish> {
        let device = self.devices.get(id)?;
        let mut endpoints: Vec<_> = device.advertisements.values().flatten().copied().collect();
        endpoints.sort();
        endpoints.dedup();
        Some(Wish {
            epoch: self.epoch,
            light_id: self.color(id).map(|(light_id, _)| light_id),
            running: self.color(id).is_some(),
            endpoints,
        })
    }
    fn snapshot(&self) -> DevicesSnapshot {
        DevicesSnapshot {
            discovery_error: self.discovery_error.clone(),
            devices: self
                .devices
                .iter()
                .map(|(id, device)| DeviceView {
                    device_id: id.clone(),
                    short_id: format!("IOT-{}", id[12..].to_uppercase()),
                    model: "esp32-rgb".into(),
                    online: device.connection.online,
                    streaming: self.color(id).is_some() && device.connection.target.is_some(),
                    message: if self.preview.as_ref().is_some_and(|p| &p.device_id == id)
                        && device.connection.target.is_some()
                    {
                        "Position preview".into()
                    } else if device.connection.message.is_empty() {
                        "Offline · waiting for discovery".into()
                    } else {
                        device.connection.message.clone()
                    },
                    bound_light_id: self.binding(id),
                })
                .collect(),
        }
    }
}
struct Inner {
    state: Mutex<State>,
    shutdown: AtomicBool,
    retry_discovery: AtomicBool,
    client_id: String,
}
#[derive(Clone)]
pub struct HardwareService(Arc<Inner>);
impl HardwareService {
    pub fn spawn(devices: impl Fn(DevicesSnapshot) + Send + 'static) -> Result<Self, String> {
        let service = Self(Arc::new(Inner {
            state: Mutex::new(State::default()),
            shutdown: AtomicBool::new(false),
            retry_discovery: AtomicBool::new(false),
            client_id: protocol::hex(&protocol::token()?),
        }));
        let sender = service.clone();
        thread::Builder::new()
            .name("iotensity-output".into())
            .spawn(move || sender.output_loop())
            .map_err(|e| e.to_string())?;
        let discovery = service.clone();
        thread::Builder::new()
            .name("iotensity-discovery".into())
            .spawn(move || discovery.discovery_loop(devices))
            .map_err(|e| e.to_string())?;
        Ok(service)
    }
    pub fn apply_saved(&self, config: Configuration) {
        let mut state = self.0.state.lock().unwrap();
        if config.revision < state.config.revision {
            return;
        }
        state.config = config;
        let bound: Vec<_> = state
            .config
            .rooms
            .iter()
            .flat_map(|r| &r.lights)
            .filter_map(|l| match &l.output {
                LightOutput::Esp32 { device_id } => Some(device_id.clone()),
                _ => None,
            })
            .collect();
        for id in bound {
            if !state.devices.contains_key(&id) && state.devices.len() >= 128 {
                if let Some(unbound) = state
                    .devices
                    .keys()
                    .find(|id| state.binding(id).is_none())
                    .cloned()
                {
                    state.devices.remove(&unbound);
                }
            }
            state.devices.entry(id).or_default();
        }
        state.preview = None;
        if state.config.rooms.iter().all(|r| r.lights.is_empty()) {
            state.output.running = false;
            state.epoch += 1;
        }
        state.prune_targets();
    }
    pub fn snapshot(&self) -> OutputSnapshot {
        self.0.state.lock().unwrap().output.clone()
    }
    pub fn devices(&self) -> DevicesSnapshot {
        self.0.state.lock().unwrap().snapshot()
    }
    pub fn retry_discovery(&self) {
        self.0.retry_discovery.store(true, Ordering::Release);
    }
    pub fn preview(&self, request: Option<preview::PreviewRequest>) -> Result<(), String> {
        if self.is_shutdown() {
            return Err("App is closing.".into());
        }
        self.0
            .state
            .lock()
            .unwrap()
            .set_preview(request, Instant::now())
    }
    pub fn set_reduced_motion(&self, reduced_motion: bool) {
        self.0.state.lock().unwrap().reduced_motion = reduced_motion;
    }
    fn set_running(&self, running: bool, reduced_motion: bool) -> OutputSnapshot {
        let mut state = self.0.state.lock().unwrap();
        let running = running && state.config.rooms.iter().any(|r| !r.lights.is_empty());
        if state.output.running != running {
            state.epoch += 1;
        }
        state.reduced_motion = reduced_motion;
        state.output.running = running;
        state.output.sequence += 1;
        if !running {
            state.preview = None;
            for device in state.devices.values_mut() {
                device.connection.target = None;
            }
        }
        state.output.clone()
    }
    pub fn identify(&self, id: &str) -> Result<(), String> {
        let (sender, receiver) = mpsc::sync_channel(1);
        {
            let mut state = self.0.state.lock().unwrap();
            let device = state
                .devices
                .get_mut(id)
                .ok_or("Discover this light before identifying it.")?;
            if !device.connection.online {
                return Err("Light is offline.".into());
            }
            if device.identify.is_some() {
                return Err("Identify is already pending.".into());
            }
            device.identify = Some(sender);
        }
        receiver
            .recv_timeout(Duration::from_secs(4))
            .map_err(|_| "Identify timed out. Try again.".to_string())?
    }
    pub fn shutdown(&self) {
        self.set_running(false, false);
        self.0.shutdown.store(true, Ordering::Release);
    }
    pub fn is_shutdown(&self) -> bool {
        self.0.shutdown.load(Ordering::Acquire)
    }
    /// Accept only final per-light RGB8 from the native source. No transport smoothing.
    pub fn publish(&self, colors: &[crate::sync::processing::LightColor], running: bool) {
        let mut state = self.0.state.lock().unwrap();
        if state.output.running != running {
            state.epoch += 1;
        }
        state.output.running = running;
        state.last_frame = Some(Instant::now());
        state.output.colors = colors
            .iter()
            .map(|c| engine::Color {
                id: c.id.clone(),
                rgb: c.rgb,
            })
            .collect();
        state.prune_targets();
    }
    fn output_loop(&self) {
        let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).and_then(|socket| {
            socket.set_nonblocking(true)?;
            Ok(socket)
        });
        if let Err(error) = &socket {
            self.0.state.lock().unwrap().discovery_error =
                Some(format!("UDP output unavailable: {error}"));
        }
        let mut sequences: HashMap<String, (protocol::Session, u32)> = HashMap::new();
        while !self.is_shutdown() {
            let targets = {
                let mut state = self.0.state.lock().unwrap();
                state.expire(Instant::now());
                // Keep the sequence through a transient control failure that may
                // recover the same session via an idempotent start retry.
                sequences.retain(|id, _| state.devices.contains_key(id));
                state
                    .devices
                    .iter()
                    .filter_map(|(id, device)| {
                        let target = device.connection.target.clone()?;
                        let (light_id, rgb) = state.color(id)?;
                        (target.light_id == light_id).then(|| (id.clone(), target, rgb))
                    })
                    .collect::<Vec<_>>()
            };
            if let Ok(socket) = &socket {
                for (id, target, rgb) in targets {
                    let entry = sequences.entry(id).or_insert((target.session, 0));
                    if entry.0 != target.session {
                        *entry = (target.session, 0);
                    }
                    let packet = protocol::Frame {
                        session: target.session,
                        sequence: entry.1,
                        rgb,
                    }
                    .encode();
                    // A dropped send is replaced by the next complete frame; no queue or retry.
                    let _ = socket.send_to(&packet, target.address);
                    entry.1 = entry.1.wrapping_add(1);
                }
            }
            // No catch-up bursts after a slow frame or sleep/wake. Static colors repeat too.
            thread::sleep(FRAME_INTERVAL);
        }
    }
    fn discovery_loop(&self, publish: impl Fn(DevicesSnapshot)) {
        let mut previous = None;
        let clock = Instant::now();
        let mut recovery = discovery::Recovery::default();
        while !self.is_shutdown() {
            let daemon = match ServiceDaemon::new() {
                Ok(daemon) => daemon,
                Err(error) => {
                    self.0.state.lock().unwrap().discovery_error = Some(error.to_string());
                    publish(self.devices());
                    thread::sleep(Duration::from_secs(2));
                    continue;
                }
            };
            let _ = daemon.set_ip_check_interval(2);
            let monitor = daemon.monitor().ok();
            let mut events = match daemon.browse(SERVICE) {
                Ok(events) => events,
                Err(error) => {
                    self.0.state.lock().unwrap().discovery_error = Some(error.to_string());
                    let _ = daemon.shutdown();
                    continue;
                }
            };
            self.0.state.lock().unwrap().discovery_error = None;
            let mut verified = Instant::now();
            let mut searched = Instant::now();
            while !self.is_shutdown() {
                if let Ok(event) = events.recv_timeout(Duration::from_millis(200)) {
                    let mut state = self.0.state.lock().unwrap();
                    match event {
                        ServiceEvent::ServiceResolved(info) => {
                            let id = info.get_property_val_str("id").unwrap_or("");
                            if valid_device_id(id)
                                && info.get_property_val_str("model") == Some("esp32-rgb")
                                && info.get_property_val_str("pv") == Some("1")
                                && info.get_port() != 0
                                && (state.devices.len() < 128 || state.devices.contains_key(id))
                            {
                                state.discovery_error = None;
                                let endpoints = info
                                    .get_addresses_v4()
                                    .into_iter()
                                    .filter(|ip| lan_address(*ip))
                                    .map(|ip| SocketAddrV4::new(ip, info.get_port()))
                                    .collect();
                                state
                                    .devices
                                    .entry(id.into())
                                    .or_default()
                                    .advertisements
                                    .insert(info.get_fullname().into(), endpoints);
                            }
                        }
                        ServiceEvent::ServiceRemoved(_, name) => {
                            for device in state.devices.values_mut() {
                                device.advertisements.remove(&name);
                            }
                        }
                        _ => {}
                    }
                }
                let mut network_changed = false;
                if let Some(monitor) = &monitor {
                    while let Ok(event) = monitor.try_recv() {
                        match event {
                            DaemonEvent::Error(error) => {
                                self.0.state.lock().unwrap().discovery_error =
                                    Some(format!("Local network discovery: {error}"));
                            }
                            DaemonEvent::IpAdd(_) | DaemonEvent::IpDel(_) => network_changed = true,
                            _ => {}
                        }
                    }
                }
                let requested = self.0.retry_discovery.swap(false, Ordering::AcqRel);
                let online = self
                    .0
                    .state
                    .lock()
                    .unwrap()
                    .devices
                    .values()
                    .any(|d| d.connection.online);
                if recovery.restart(
                    clock.elapsed().as_millis() as u64,
                    network_changed || requested,
                    online,
                ) {
                    self.0
                        .state
                        .lock()
                        .unwrap()
                        .reset_discovery(network_changed);
                    // Keep identities and room bindings, but recreate the socket
                    // and multicast membership instead of only issuing another query.
                    break;
                }
                // Keep looking for additional lights while existing ones are healthy.
                if searched.elapsed() >= Duration::from_secs(10) {
                    let refreshed = daemon
                        .stop_browse(SERVICE)
                        .and_then(|_| daemon.browse(SERVICE));
                    match refreshed {
                        Ok(receiver) => events = receiver,
                        Err(error) => {
                            self.0.state.lock().unwrap().discovery_error = Some(error.to_string());
                            publish(self.devices());
                            break;
                        }
                    }
                    searched = Instant::now();
                }
                let mut state = self.0.state.lock().unwrap();
                for (id, device) in &mut state.devices {
                    if !device.worker {
                        let worker = self.clone();
                        let id = id.clone();
                        match thread::Builder::new()
                            .name("iotensity-device".into())
                            .spawn(move || worker.device_loop(id))
                        {
                            Ok(_) => device.worker = true,
                            Err(error) => device.connection.message = error.to_string(),
                        }
                    }
                }
                if verified.elapsed() >= Duration::from_secs(5) {
                    for device in state.devices.values().filter(|d| !d.connection.online) {
                        for name in device.advertisements.keys() {
                            let _ = daemon.verify(name.clone(), Duration::from_secs(2));
                        }
                    }
                    verified = Instant::now();
                }
                let snapshot = state.snapshot();
                drop(state);
                if previous.as_ref() != Some(&snapshot) {
                    publish(snapshot.clone());
                    previous = Some(snapshot);
                }
            }
            if let Ok(stopped) = daemon.shutdown() {
                let _ = stopped.recv_timeout(Duration::from_secs(1));
            }
        }
    }
    fn device_loop(&self, id: String) {
        let control = match HttpControl::new() {
            Ok(control) => control,
            Err(error) => {
                if let Some(device) = self.0.state.lock().unwrap().devices.get_mut(&id) {
                    device.connection.message = error;
                    device.worker = false;
                }
                return;
            }
        };
        let mut connection = Connection::new(id.clone(), self.0.client_id.clone());
        let start = Instant::now();
        while !self.is_shutdown() {
            let wish = self.0.state.lock().unwrap().wish(&id);
            let Some(wish) = wish else {
                break;
            };
            let next = connection
                .step(start.elapsed().as_millis() as u64, &wish, &control)
                .clone();
            let identify = {
                let mut state = self.0.state.lock().unwrap();
                if state.wish(&id).as_ref() != Some(&wish) {
                    continue;
                }
                let Some(device) = state.devices.get_mut(&id) else {
                    break;
                };
                device.connection = next;
                device.identify.take()
            };
            if let Some(reply) = identify {
                let _ = reply.send(connection.identify(&control));
            }
            thread::sleep(Duration::from_millis(50));
        }
        connection.close(&control);
    }
}

/// This milestone supports IPv4 private/link-local LANs. No URLs come from React.
pub fn lan_address(ip: Ipv4Addr) -> bool {
    (ip.is_private() || ip.is_link_local()) && !ip.is_broadcast() && !ip.is_unspecified()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn socket_recovery_retains_bindings_and_does_not_interrupt_healthy_outputs() {
        let config: Configuration =
            serde_json::from_str(include_str!("../../../tests/fixtures/configuration.json"))
                .unwrap();
        let light = config.rooms[0].lights.last().unwrap();
        let LightOutput::Esp32 { device_id } = &light.output else {
            panic!("physical fixture")
        };
        let id = device_id.clone();
        let logical_id = light.id.clone();
        let mut state = State {
            config,
            ..State::default()
        };
        let mut device = Device::default();
        device.connection.online = true;
        device.advertisements.insert(
            "light._iotensity._tcp.local.".into(),
            vec!["192.168.1.39:80".parse().unwrap()],
        );
        state.devices.insert(id.clone(), device);
        state.reset_discovery(false);
        assert!(state.devices[&id].connection.online);
        assert!(!state.wish(&id).unwrap().endpoints.is_empty());
        state.reset_discovery(true);
        assert!(!state.devices[&id].connection.online);
        assert!(state.wish(&id).unwrap().endpoints.is_empty());
        assert_eq!(state.binding(&id), Some(logical_id));
        assert_eq!(state.devices.len(), 1);
    }

    #[test]
    fn offline_binding_is_visible_even_without_output_frames() {
        let config: Configuration =
            serde_json::from_str(include_str!("../../../tests/fixtures/configuration.json"))
                .unwrap();
        let light = config.rooms[0].lights.last().unwrap();
        let LightOutput::Esp32 { device_id } = &light.output else {
            panic!("fixture must contain a physical binding")
        };
        let id = device_id.clone();
        let logical_id = light.id.clone();
        let mut state = State {
            config,
            ..State::default()
        };
        state.devices.insert(id.clone(), Device::default());
        let view = &state.snapshot().devices[0];
        assert!(!view.online);
        assert_eq!(view.bound_light_id.as_deref(), Some(logical_id.as_str()));
        // Saved assignment is independent of whether this source emits that light.
        assert!(state.wish(&id).unwrap().light_id.is_none());
        state.output.colors.push(engine::Color {
            id: logical_id.clone(),
            rgb: [1, 2, 3],
        });
        state.output.running = true;
        assert_eq!(state.wish(&id).unwrap().light_id, Some(logical_id));
    }

    fn preview_request(id: &str, x: f64) -> preview::PreviewRequest {
        preview::PreviewRequest {
            device_id: id.into(),
            position: crate::config::Position { x, y: 1.2, z: 1.0 },
            mode: preview::EditMode::Location,
        }
    }

    fn online_device() -> Device {
        Device {
            connection: ConnectionState {
                online: true,
                ..Default::default()
            },
            ..Default::default()
        }
    }

    #[test]
    fn unsaved_preview_renews_without_reconnecting_and_expires_without_sync() {
        let id = "esp32-020000a1b2c3";
        let other = "esp32-020000112233";
        let mut state = State::default();
        state.devices.insert(id.into(), online_device());
        state.devices.insert(other.into(), online_device());
        let saved = state.config.clone();
        let now = Instant::now();
        state
            .set_preview(Some(preview_request(id, -3.0)), now)
            .unwrap();
        let wish = state.wish(id).unwrap();
        assert!(wish.running);
        assert!(!state.wish(other).unwrap().running);
        assert_eq!(state.color(id).unwrap().1, [64, 232, 135]);
        state
            .set_preview(Some(preview_request(id, 3.0)), now + Duration::from_secs(1))
            .unwrap();
        assert_eq!(state.wish(id).unwrap(), wish); // No session change for every mouse movement.
        assert_eq!(state.color(id).unwrap().1, [255, 89, 31]);
        state.expire(now + preview::PREVIEW_DURATION);
        assert!(state.wish(id).unwrap().running); // Last edit renewed the lease.
        state.expire(now + Duration::from_secs(3));
        assert!(!state.wish(id).unwrap().running);
        assert_ne!(state.wish(id).unwrap(), wish); // An in-flight start must be discarded.
        assert!(state.color(id).is_none());
        assert_eq!(state.config, saved);
        assert!(!state.output.running);
    }

    #[test]
    fn selection_switch_cancel_and_invalid_requests_release_previous_light() {
        let id = "esp32-020000a1b2c3";
        let other = "esp32-020000112233";
        let mut state = State::default();
        state.devices.insert(id.into(), online_device());
        state.devices.insert(other.into(), online_device());
        let now = Instant::now();
        state
            .set_preview(Some(preview_request(id, 0.0)), now)
            .unwrap();
        let old_wish = state.wish(id).unwrap();
        state.devices.get_mut(id).unwrap().connection.target = Some(control::Target {
            light_id: old_wish.light_id.clone().unwrap(),
            address: "192.168.1.20:49600".parse().unwrap(),
            session: [1; 16],
        });
        state
            .set_preview(Some(preview_request(other, 1.0)), now)
            .unwrap();
        assert_ne!(state.wish(id).unwrap(), old_wish);
        assert!(state.devices[id].connection.target.is_none());
        assert!(state.color(id).is_none());
        assert!(state.color(other).is_some());
        state.set_preview(None, now).unwrap();
        assert!(state.color(other).is_none());
        state
            .set_preview(Some(preview_request(id, 0.0)), now)
            .unwrap();
        assert!(state
            .set_preview(Some(preview_request(other, 99.0)), now)
            .is_err());
        assert!(state.color(id).is_none());
        state.devices.get_mut(other).unwrap().connection.online = false;
        assert!(state
            .set_preview(Some(preview_request(other, 0.0)), now)
            .is_err());
    }

    #[test]
    fn preview_overrides_only_selected_device_then_resumes_saved_output() {
        let config: Configuration =
            serde_json::from_str(include_str!("../../../tests/fixtures/configuration.json"))
                .unwrap();
        let light = config.rooms[0].lights.last().unwrap();
        let LightOutput::Esp32 { device_id } = &light.output else {
            panic!("physical fixture")
        };
        let id = device_id.clone();
        let logical_id = light.id.clone();
        let now = Instant::now();
        let mut state = State {
            config,
            last_frame: Some(now),
            ..Default::default()
        };
        state.devices.insert(id.clone(), online_device());
        state.output.running = true;
        state.output.colors.push(engine::Color {
            id: logical_id,
            rgb: [10, 20, 30],
        });
        let saved = state.config.clone();
        let before = state.wish(&id).unwrap();
        state
            .set_preview(Some(preview_request(&id, -3.0)), now)
            .unwrap();
        assert_eq!(state.wish(&id).unwrap(), before);
        assert_eq!(state.color(&id).unwrap().1, [64, 232, 135]);
        let end = now + preview::PREVIEW_DURATION;
        state.last_frame = Some(end);
        state.expire(end);
        assert_eq!(state.wish(&id).unwrap(), before);
        assert_eq!(state.color(&id).unwrap().1, [10, 20, 30]);
        assert_eq!(state.config, saved);
        // A stale sync source still stops, including when a placement preview exists.
        state
            .set_preview(Some(preview_request(&id, 0.0)), end)
            .unwrap();
        state.expire(end + Duration::from_millis(251));
        assert!(!state.output.running);
        state.expire(end + preview::PREVIEW_DURATION);
        assert!(state.color(&id).is_none());
    }
}
