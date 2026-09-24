pub mod control;
pub mod engine;
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
}
impl State {
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
            light_id: self
                .binding(id)
                .filter(|light_id| self.output.colors.iter().any(|color| &color.id == light_id)),
            running: self.output.running,
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
                    streaming: self.output.running && device.connection.target.is_some(),
                    message: if device.connection.message.is_empty() {
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
    client_id: String,
}
#[derive(Clone)]
pub struct HardwareService(Arc<Inner>);
impl HardwareService {
    pub fn spawn(devices: impl Fn(DevicesSnapshot) + Send + 'static) -> Result<Self, String> {
        let service = Self(Arc::new(Inner {
            state: Mutex::new(State::default()),
            shutdown: AtomicBool::new(false),
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
        let running = state.output.running;
        let bindings: HashMap<_, _> = state
            .devices
            .keys()
            .map(|id| (id.clone(), state.binding(id)))
            .collect();
        for (id, device) in &mut state.devices {
            if !running
                || device
                    .connection
                    .target
                    .as_ref()
                    .is_some_and(|t| bindings[id].as_ref() != Some(&t.light_id))
            {
                device.connection.target = None;
            }
        }
        if state.config.rooms.iter().all(|r| r.lights.is_empty()) {
            state.output.running = false;
            state.epoch += 1;
        }
    }
    pub fn snapshot(&self) -> OutputSnapshot {
        self.0.state.lock().unwrap().output.clone()
    }
    pub fn devices(&self) -> DevicesSnapshot {
        self.0.state.lock().unwrap().snapshot()
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
        if !running {
            for device in state.devices.values_mut() {
                device.connection.target = None;
            }
        }
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
            let (frame, targets) = {
                let mut state = self.0.state.lock().unwrap();
                if state.output.running
                    && state
                        .last_frame
                        .is_none_or(|last| last.elapsed() > Duration::from_millis(250))
                {
                    state.output.running = false;
                    state.epoch += 1;
                    for device in state.devices.values_mut() {
                        device.connection.target = None;
                    }
                }
                let targets: Vec<_> = state
                    .devices
                    .values()
                    .filter_map(|d| d.connection.target.clone())
                    .filter(|_| state.output.running)
                    .collect();
                (state.output.clone(), targets)
            };
            sequences.retain(|id, _| frame.colors.iter().any(|c| c.id == *id));
            if let Ok(socket) = &socket {
                for target in targets {
                    if let Some(color) = frame.colors.iter().find(|c| c.id == target.light_id) {
                        let entry = sequences
                            .entry(target.light_id.clone())
                            .or_insert((target.session, 0));
                        if entry.0 != target.session {
                            *entry = (target.session, 0);
                        }
                        let packet = protocol::Frame {
                            session: target.session,
                            sequence: entry.1,
                            rgb: color.rgb,
                        }
                        .encode();
                        // A dropped send is replaced by the next complete frame; no queue or retry.
                        let _ = socket.send_to(&packet, target.address);
                        entry.1 = entry.1.wrapping_add(1);
                    }
                }
            }
            // No catch-up bursts after a slow frame or sleep/wake. Static colors repeat too.
            thread::sleep(FRAME_INTERVAL);
        }
    }
    fn discovery_loop(&self, publish: impl Fn(DevicesSnapshot)) {
        let mut previous = None;
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
                // mDNS's normal browse backoff reaches an hour. Bound recovery time
                // after a lost announcement, network switch, or sleep/wake instead.
                if network_changed || searched.elapsed() >= Duration::from_secs(10) {
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
            let _ = daemon.shutdown();
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
        assert_eq!(state.wish(&id).unwrap().light_id, Some(logical_id));
    }
}
