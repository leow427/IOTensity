pub mod control;
mod discovery;
pub mod engine;
mod output;
pub mod preview;
pub mod protocol;

use crate::config::{valid_device_id, Configuration, LightOutput};
use control::{Connection, ConnectionState, HttpControl, Wish};
use engine::OutputSnapshot;
use mdns_sd::{DaemonEvent, ServiceDaemon, ServiceEvent};
use serde::Serialize;
use std::{
    borrow::Cow,
    collections::{BTreeMap, HashMap, HashSet},
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
/// Shared 250 ms stall bound. Hardware applies it to publish ticks; sync applies
/// it to capture progress. Each is measured where it is observed, so a source
/// gap that sync still accepts can never expire output between ticks.
pub const STALL_TIMEOUT: Duration = Duration::from_millis(250);
/// Pause before recreating an mDNS daemon that could not be created or browse.
const DISCOVERY_RETRY_DELAY: Duration = Duration::from_secs(2);
const DAEMON_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(1);
const IDENTIFY_TIMEOUT: Duration = Duration::from_secs(4);
/// Unbound lights that stay offline and unadvertised this long are forgotten.
const DEVICE_GRACE: Duration = Duration::from_secs(5 * 60);
/// Delay before replacing a device worker that could not be started.
const WORKER_RETRY: Duration = Duration::from_secs(5);
type IdentifyReply = mpsc::SyncSender<Result<(), String>>;
struct PendingIdentify {
    request: u64,
    // The caller stops waiting here; a later blink would contradict its error.
    deadline: Instant,
    reply: IdentifyReply,
}
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
    pub output_error: Option<String>,
}
#[derive(Default)]
struct Device {
    // Runtime only: neither endpoints nor stream state appear in Configuration.
    advertisements: HashMap<String, Vec<SocketAddrV4>>,
    connection: ConnectionState,
    /// Generation of the only worker allowed to control this entry.
    worker: Option<u64>,
    worker_retry: Option<Instant>,
    idle_since: Option<Instant>,
    identify: Option<PendingIdentify>,
}
#[derive(Default)]
struct State {
    config: Configuration,
    output: OutputSnapshot,
    epoch: u64,
    devices: BTreeMap<String, Device>,
    discovery_error: Option<String>,
    output_error: Option<String>,
    output_available: bool,
    last_publish: Option<Instant>,
    preview: Option<preview::Preview>,
    identify_requests: u64,
    workers: u64,
    /// Saved device ID → logical light ID, rebuilt only with `config`.
    bindings: HashMap<String, String>,
}
/// Reads only the fields it needs so `prune_targets` can update devices in place.
fn resolve_color<'a>(
    preview: Option<&preview::Preview>,
    output: &OutputSnapshot,
    bindings: &'a HashMap<String, String>,
    id: &str,
) -> Option<(Cow<'a, str>, [u8; 3])> {
    let binding = bindings.get(id).map(String::as_str);
    if let Some(preview) = preview.filter(|p| p.device_id == id) {
        return Some((
            binding.map_or_else(|| Cow::Owned(format!("preview-{id}")), Cow::Borrowed),
            preview.rgb,
        ));
    }
    if !output.running {
        return None;
    }
    let light_id = binding?;
    output
        .colors
        .iter()
        .find(|c| c.id == light_id)
        .map(|c| (Cow::Borrowed(light_id), c.rgb))
}
impl State {
    fn queue_identify(
        &mut self,
        id: &str,
        reply: IdentifyReply,
        timeout: Duration,
        now: Instant,
    ) -> Result<u64, String> {
        let device = self
            .devices
            .get_mut(id)
            .ok_or("Discover this light before identifying it.")?;
        if !device.connection.online {
            return Err("Light is offline.".into());
        }
        if device.identify.as_ref().is_some_and(|p| now < p.deadline) {
            return Err("Identify is already pending.".into());
        }
        self.identify_requests += 1;
        device.identify = Some(PendingIdentify {
            request: self.identify_requests,
            deadline: now + timeout,
            reply,
        });
        Ok(self.identify_requests)
    }
    /// Clears only the caller's own request, never a newer one.
    fn cancel_identify(&mut self, id: &str, request: u64) {
        if let Some(device) = self.devices.get_mut(id) {
            if device
                .identify
                .as_ref()
                .is_some_and(|p| p.request == request)
            {
                device.identify = None;
            }
        }
    }
    /// Requests whose caller already timed out are dropped without blinking.
    fn take_identify(&mut self, id: &str, now: Instant) -> Option<IdentifyReply> {
        let pending = self.devices.get_mut(id)?.identify.take()?;
        (now < pending.deadline).then_some(pending.reply)
    }
    fn set_config(&mut self, config: Configuration) {
        self.bindings.clear();
        for light in config.rooms.iter().flat_map(|r| &r.lights) {
            if let LightOutput::Esp32 { device_id } = &light.output {
                // The first binding wins, as with a scan of the saved rooms.
                self.bindings
                    .entry(device_id.clone())
                    .or_insert_with(|| light.id.clone());
            }
        }
        self.config = config;
    }
    fn color(&self, id: &str) -> Option<(Cow<'_, str>, [u8; 3])> {
        resolve_color(self.preview.as_ref(), &self.output, &self.bindings, id)
    }
    // Sync (any source, including virtual-only rooms) or a physical placement
    // preview needs steady native timing; stopped/expired output does not.
    fn needs_activity(&self) -> bool {
        self.output.running || self.preview.is_some()
    }
    fn prune_targets(&mut self) {
        let (preview, output, bindings) = (self.preview.as_ref(), &self.output, &self.bindings);
        for (id, device) in &mut self.devices {
            if device.connection.target.as_ref().is_some_and(|target| {
                resolve_color(preview, output, bindings, id)
                    .is_none_or(|(light_id, _)| light_id != target.light_id.as_str())
            }) {
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
                .last_publish
                .is_none_or(|last| now.duration_since(last) >= STALL_TIMEOUT)
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
    fn reset_discovery(&mut self) {
        for device in self.devices.values_mut() {
            // Interface notifications also include unrelated VPN/IPv6 changes.
            // Existing identity-verified control probes decide whether a stream
            // is still healthy; rebuilding discovery must not tear it down.
            if !device.connection.online {
                device.advertisements.clear();
                device.connection = ConnectionState::default();
            }
        }
    }
    /// Saved bindings are kept forever; other offline, unadvertised entries are
    /// forgotten after a grace period so they release their worker and slot.
    fn prune_devices(&mut self, now: Instant) {
        let bound: HashSet<&str> = self
            .config
            .rooms
            .iter()
            .flat_map(|r| &r.lights)
            .filter_map(|light| match &light.output {
                LightOutput::Esp32 { device_id } => Some(device_id.as_str()),
                _ => None,
            })
            .collect();
        self.devices.retain(|id, device| {
            if bound.contains(id.as_str())
                || device.connection.online
                || !device.advertisements.is_empty()
            {
                device.idle_since = None;
                return true;
            }
            now.duration_since(*device.idle_since.get_or_insert(now)) < DEVICE_GRACE
        });
    }
    /// Assign a fresh worker generation to each entry that needs a worker.
    fn claim_workers(&mut self, now: Instant) -> Vec<(String, u64)> {
        let mut claims = Vec::new();
        for (id, device) in &mut self.devices {
            if device.worker.is_none() && device.worker_retry.is_none_or(|at| now >= at) {
                self.workers += 1;
                device.worker = Some(self.workers);
                device.worker_retry = None;
                claims.push((id.clone(), self.workers));
            }
        }
        claims
    }
    fn worker_failed(&mut self, id: &str, generation: u64, error: String, now: Instant) {
        if let Some(device) = self
            .devices
            .get_mut(id)
            .filter(|d| d.worker == Some(generation))
        {
            device.worker = None;
            device.worker_retry = Some(now + WORKER_RETRY);
            device.connection.message = error;
        }
    }
    /// A removed or replaced entry ends its old worker instead of sharing control.
    fn worker_wish(&self, id: &str, generation: u64) -> Option<Wish> {
        self.devices
            .get(id)
            .filter(|d| d.worker == Some(generation))?;
        self.wish(id)
    }
    fn binding(&self, id: &str) -> Option<&str> {
        self.bindings.get(id).map(String::as_str)
    }
    fn wish(&self, id: &str) -> Option<Wish> {
        let device = self.devices.get(id)?;
        let mut endpoints: Vec<_> = device.advertisements.values().flatten().copied().collect();
        endpoints.sort();
        endpoints.dedup();
        let light_id = self
            .output_available
            .then(|| self.color(id))
            .flatten()
            .map(|(light_id, _)| light_id.into_owned());
        Some(Wish {
            epoch: self.epoch,
            light_id,
            endpoints,
        })
    }
    fn snapshot(&self) -> DevicesSnapshot {
        DevicesSnapshot {
            discovery_error: self.discovery_error.clone(),
            output_error: self.output_error.clone(),
            devices: self
                .devices
                .iter()
                .map(|(id, device)| {
                    let colored = self.color(id).is_some();
                    DeviceView {
                        device_id: id.clone(),
                        short_id: format!("IOT-{}", id[12..].to_uppercase()),
                        model: "esp32-rgb".into(),
                        online: device.connection.online,
                        streaming: self.output_available
                            && colored
                            && device.connection.target.is_some(),
                        message: if colored && !self.output_available {
                            self.output_error
                                .clone()
                                .unwrap_or_else(|| "Connecting UDP output…".into())
                        } else if self.preview.as_ref().is_some_and(|p| &p.device_id == id)
                            && device.connection.target.is_some()
                        {
                            "Position preview".into()
                        } else if device.connection.message.is_empty() {
                            "Offline · waiting for discovery".into()
                        } else {
                            device.connection.message.clone()
                        },
                        bound_light_id: self.binding(id).map(str::to_owned),
                    }
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
        state.set_config(config);
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
    fn set_running(&self, running: bool) -> OutputSnapshot {
        let mut state = self.0.state.lock().unwrap();
        let running = running && state.config.rooms.iter().any(|r| !r.lights.is_empty());
        if state.output.running != running {
            state.epoch += 1;
        }
        state.output.running = running;
        if !running {
            state.preview = None;
            for device in state.devices.values_mut() {
                device.connection.target = None;
            }
        }
        state.output.clone()
    }
    pub fn identify(&self, id: &str) -> Result<(), String> {
        self.identify_within(id, IDENTIFY_TIMEOUT)
    }
    fn identify_within(&self, id: &str, timeout: Duration) -> Result<(), String> {
        let (sender, receiver) = mpsc::sync_channel(1);
        let request =
            self.0
                .state
                .lock()
                .unwrap()
                .queue_identify(id, sender, timeout, Instant::now())?;
        receiver.recv_timeout(timeout).map_err(|_| {
            self.0.state.lock().unwrap().cancel_identify(id, request);
            "Identify timed out. Try again.".to_string()
        })?
    }
    pub fn shutdown(&self) {
        self.set_running(false);
        self.0.shutdown.store(true, Ordering::Release);
    }
    pub fn is_shutdown(&self) -> bool {
        self.0.shutdown.load(Ordering::Acquire)
    }
    /// Accept only final per-light RGB8 from the native source. No transport smoothing.
    pub fn publish(&self, colors: &[crate::sync::processing::LightColor], running: bool) {
        self.publish_at(colors, running, Instant::now());
    }
    /// `published_at` is the publisher's tick time, not capture age: source
    /// progress is judged by sync, and this watchdog bounds publisher stalls.
    pub fn publish_at(
        &self,
        colors: &[crate::sync::processing::LightColor],
        running: bool,
        published_at: Instant,
    ) {
        let mut state = self.0.state.lock().unwrap();
        if state.output.running != running {
            state.epoch += 1;
        }
        state.output.running = running;
        state.last_publish = Some(published_at);
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
        let clock = Instant::now();
        let mut output = output::SocketRecovery::default();
        let mut sequences: HashMap<String, (protocol::Session, u32)> = HashMap::new();
        // Dropped when this loop ends at shutdown.
        let mut activity = crate::activity::Activity::default();
        // Reused each frame; packets are encoded under the lock and sent after it.
        let mut packets: Vec<([u8; protocol::PACKET_LEN], SocketAddrV4)> = Vec::new();
        while !self.is_shutdown() {
            output.poll(clock.elapsed().as_millis() as u64, || {
                let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0))?;
                socket.set_nonblocking(true)?;
                Ok(socket)
            });
            packets.clear();
            let active = {
                let mut state = self.0.state.lock().unwrap();
                state.output_available = output.socket.is_some();
                state.output_error = output.error.clone();
                state.expire(Instant::now());
                // Keep the sequence through a transient control failure that may
                // recover the same session via an idempotent start retry.
                sequences.retain(|id, _| state.devices.contains_key(id));
                if output.socket.is_some() {
                    for (id, device) in &state.devices {
                        let Some(target) = &device.connection.target else {
                            continue;
                        };
                        let Some((light_id, rgb)) = state.color(id) else {
                            continue;
                        };
                        if target.light_id != light_id {
                            continue;
                        }
                        if !sequences.contains_key(id) {
                            sequences.insert(id.clone(), (target.session, 0));
                        }
                        let entry = sequences.get_mut(id).expect("sequence inserted above");
                        if entry.0 != target.session {
                            *entry = (target.session, 0);
                        }
                        let packet = protocol::Frame {
                            session: target.session,
                            sequence: entry.1,
                            rgb,
                        }
                        .encode();
                        packets.push((packet, target.address));
                        entry.1 = entry.1.wrapping_add(1);
                    }
                }
                state.needs_activity()
            };
            activity.set(active);
            if let Some(socket) = &output.socket {
                for (packet, address) in &packets {
                    // A dropped send is replaced by the next complete frame; no queue or retry.
                    let _ = socket.send_to(packet, address);
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
        let mut control: Option<HttpControl> = None;
        while !self.is_shutdown() {
            let daemon = match ServiceDaemon::new() {
                Ok(daemon) => daemon,
                Err(error) => {
                    self.discovery_failed(error.to_string(), &publish);
                    continue;
                }
            };
            let _ = daemon.set_ip_check_interval(2);
            let monitor = daemon.monitor().ok();
            let mut events = match daemon.browse(SERVICE) {
                Ok(events) => events,
                Err(error) => {
                    // A dead daemon fails every browse; back off instead of spinning.
                    stop_daemon(&daemon);
                    self.discovery_failed(error.to_string(), &publish);
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
                    self.0.state.lock().unwrap().reset_discovery();
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
                let now = Instant::now();
                let claims = {
                    let mut state = self.0.state.lock().unwrap();
                    state.prune_devices(now);
                    state.claim_workers(now)
                };
                // One bounded HTTP client (and runtime thread) serves every device worker.
                let mut unavailable = String::new();
                if !claims.is_empty() && control.is_none() {
                    match HttpControl::new() {
                        Ok(shared) => control = Some(shared),
                        Err(error) => unavailable = error,
                    }
                }
                for (id, generation) in claims {
                    let spawned = match &control {
                        Some(shared) => {
                            let worker = self.clone();
                            let (id, shared) = (id.clone(), shared.clone());
                            thread::Builder::new()
                                .name("iotensity-device".into())
                                .spawn(move || worker.device_loop(id, generation, shared))
                                .map(drop)
                                .map_err(|e| e.to_string())
                        }
                        None => Err(unavailable.clone()),
                    };
                    if let Err(error) = spawned {
                        let mut state = self.0.state.lock().unwrap();
                        state.worker_failed(&id, generation, error, now);
                    }
                }
                let state = self.0.state.lock().unwrap();
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
            stop_daemon(&daemon);
        }
    }
    fn discovery_failed(&self, error: String, publish: &impl Fn(DevicesSnapshot)) {
        self.0.state.lock().unwrap().discovery_error = Some(error);
        publish(self.devices());
        wait_unless_shutdown(&self.0.shutdown, DISCOVERY_RETRY_DELAY);
    }
    fn device_loop(&self, id: String, generation: u64, control: HttpControl) {
        let mut connection = Connection::new(id.clone(), self.0.client_id.clone());
        let start = Instant::now();
        while !self.is_shutdown() {
            let wish = self.0.state.lock().unwrap().worker_wish(&id, generation);
            let Some(wish) = wish else {
                break;
            };
            let next = connection
                .step(start.elapsed().as_millis() as u64, &wish, &control)
                .clone();
            let identify = {
                let mut state = self.0.state.lock().unwrap();
                // Late responses for an obsolete binding or run state are discarded;
                // endpoint-only discovery changes do not invalidate a verified result.
                // A removed or replaced entry ends this worker's control too.
                if !state
                    .worker_wish(&id, generation)
                    .is_some_and(|latest| latest.same_stream(&wish))
                {
                    continue;
                }
                let Some(device) = state.devices.get_mut(&id) else {
                    break;
                };
                device.connection = next;
                state.take_identify(&id, Instant::now())
            };
            if let Some(reply) = identify {
                let _ = reply.send(connection.identify(&control));
            }
            thread::sleep(Duration::from_millis(50));
        }
        connection.close(&control);
    }
}

#[cfg(test)]
impl HardwareService {
    /// A service without output, discovery or control threads.
    pub(crate) fn detached() -> Self {
        Self(Arc::new(Inner {
            state: Mutex::new(State::default()),
            shutdown: AtomicBool::new(false),
            retry_discovery: AtomicBool::new(false),
            client_id: "test".into(),
        }))
    }
    /// Runs the output thread's watchdog at `now`; returns `(running, epoch)`.
    pub(crate) fn expire_at(&self, now: Instant) -> (bool, u64) {
        let mut state = self.0.state.lock().unwrap();
        state.expire(now);
        (state.output.running, state.epoch)
    }
}

/// Bounded: a wedged daemon thread must not stall discovery or app shutdown.
fn stop_daemon(daemon: &ServiceDaemon) {
    if let Ok(stopped) = daemon.shutdown() {
        let _ = stopped.recv_timeout(DAEMON_SHUTDOWN_TIMEOUT);
    }
}

/// Sleep for `delay`, returning early once `shutdown` is set.
fn wait_unless_shutdown(shutdown: &AtomicBool, delay: Duration) {
    let deadline = Instant::now() + delay;
    while !shutdown.load(Ordering::Acquire) {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return;
        }
        thread::sleep(remaining.min(Duration::from_millis(100)));
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
            output_available: true,
            ..State::default()
        };
        state.set_config(config);
        state.output.running = true;
        state.output.colors.push(engine::Color {
            id: logical_id.clone(),
            rgb: [1, 2, 3],
        });
        let mut device = Device::default();
        device.connection.online = true;
        device.connection.target = Some(control::Target {
            light_id: logical_id.clone(),
            address: "192.168.1.39:49600".parse().unwrap(),
            session: [7; 16],
        });
        device.advertisements.insert(
            "light._iotensity._tcp.local.".into(),
            vec!["192.168.1.39:80".parse().unwrap()],
        );
        state.devices.insert(id.clone(), device);
        let before = state.devices[&id].connection.clone();
        let wish = state.wish(&id).unwrap();
        assert!(wish.light_id.is_some());
        state.reset_discovery();
        assert_eq!(state.devices[&id].connection, before);
        assert_eq!(state.wish(&id).unwrap(), wish);
        // Repeated network notifications use the same reset as manual recovery.
        state.reset_discovery();
        assert_eq!(state.devices[&id].connection, before);
        assert_eq!(state.wish(&id).unwrap(), wish);
        assert_eq!(state.binding(&id), Some(logical_id.as_str()));
        assert_eq!(state.devices.len(), 1);
        // An expired mDNS record changes only candidates, never the stream itself.
        state
            .devices
            .get_mut(&id)
            .unwrap()
            .advertisements
            .remove("light._iotensity._tcp.local.");
        assert!(state.wish(&id).unwrap().endpoints.is_empty());
        assert!(state.wish(&id).unwrap().same_stream(&wish));
        let mut unbound = state.config.clone();
        unbound.rooms[0].lights.pop();
        state.set_config(unbound);
        assert!(!state.wish(&id).unwrap().same_stream(&wish));
        state.devices.get_mut(&id).unwrap().connection.online = false;
        state.reset_discovery();
        assert!(state.wish(&id).unwrap().endpoints.is_empty());
        assert!(state.devices[&id].connection.target.is_none());
    }

    #[test]
    fn discovery_recovery_cannot_clear_udp_fault_or_claim_streaming() {
        let id = "esp32-020000a1b2c3";
        let mut state = State {
            output_available: true,
            ..State::default()
        };
        state.devices.insert(id.into(), online_device());
        state
            .set_preview(Some(preview_request(id, 0.0)), Instant::now())
            .unwrap();
        state.devices.get_mut(id).unwrap().connection.target = Some(control::Target {
            light_id: format!("preview-{id}"),
            address: "192.168.1.39:49600".parse().unwrap(),
            session: [7; 16],
        });
        state.output_available = false;
        assert!(state.wish(id).unwrap().light_id.is_none());
        state.output_error = Some("UDP output unavailable: address unavailable".into());
        state.discovery_error = Some("Discovery unavailable".into());
        state.reset_discovery();
        state.discovery_error = None; // A successful daemon restart.
        let snapshot = state.snapshot();
        assert!(snapshot.discovery_error.is_none());
        assert!(snapshot.output_error.is_some());
        assert!(!snapshot.devices[0].streaming);
        assert!(snapshot.devices[0]
            .message
            .contains("UDP output unavailable"));
        state.output_error = None;
        state.output_available = true;
        assert!(state.wish(id).unwrap().light_id.is_some());
        assert!(state.snapshot().devices[0].streaming);
        assert_eq!(state.snapshot().devices[0].message, "Position preview");
    }

    #[test]
    fn stalled_publisher_stops_output_and_discards_targets() {
        let service = HardwareService::detached();
        let config: Configuration =
            serde_json::from_str(include_str!("../../../tests/fixtures/configuration.json"))
                .unwrap();
        let light = config.rooms[0].lights.last().unwrap();
        let LightOutput::Esp32 { device_id } = &light.output else {
            panic!("physical fixture")
        };
        let id = device_id.clone();
        let logical_id = light.id.clone();
        service.apply_saved(config);
        let colors = [crate::sync::processing::LightColor {
            id: logical_id.clone(),
            rgb: [1, 2, 3],
        }];
        let observed = Instant::now();
        service.publish_at(&colors, true, observed);
        let mut state = service.0.state.lock().unwrap();
        let target = control::Target {
            light_id: logical_id,
            address: "192.168.1.39:49600".parse().unwrap(),
            session: [7; 16],
        };
        state.devices.get_mut(&id).unwrap().connection.target = Some(target.clone());
        assert_eq!(state.last_publish, Some(observed));
        let epoch = state.epoch;
        state.expire(observed + STALL_TIMEOUT - Duration::from_millis(1));
        assert!(state.output.running);
        assert!(state.devices[&id].connection.target.is_some());
        state.expire(observed + STALL_TIMEOUT);
        assert!(!state.output.running);
        assert_eq!(state.epoch, epoch + 1);
        assert!(state.devices[&id].connection.target.is_none());
        drop(state);
        service.publish_at(&colors, true, observed);
        service
            .0
            .state
            .lock()
            .unwrap()
            .devices
            .get_mut(&id)
            .unwrap()
            .connection
            .target = Some(target);
        service.publish_at(&colors, false, observed);
        assert!(service.0.state.lock().unwrap().devices[&id]
            .connection
            .target
            .is_none());
    }

    #[test]
    fn cached_bindings_follow_saved_bind_unbind_and_rebind_across_rooms() {
        // The per-call scan the cache replaces; results must stay identical.
        fn scanned(config: &Configuration, id: &str) -> Option<String> {
            config
                .rooms
                .iter()
                .flat_map(|r| &r.lights)
                .find_map(|light| match &light.output {
                    LightOutput::Esp32 { device_id } if device_id == id => Some(light.id.clone()),
                    _ => None,
                })
        }
        let service = HardwareService(Arc::new(Inner {
            state: Mutex::new(State::default()),
            shutdown: AtomicBool::new(false),
            retry_discovery: AtomicBool::new(false),
            client_id: "test".into(),
        }));
        let mut config: Configuration =
            serde_json::from_str(include_str!("../../../tests/fixtures/configuration.json"))
                .unwrap();
        let first = "esp32-aabbcca1b2c3";
        let second = "esp32-020000112233";
        let mut office = config.rooms[0].clone();
        office.id = "office".into();
        for light in &mut office.lights {
            light.id = format!("office-{}", light.id);
            light.output = LightOutput::Virtual;
        }
        office.lights[1].output = LightOutput::Esp32 {
            device_id: second.into(),
        };
        config.rooms.push(office);
        let apply_and_compare = |config: &Configuration| {
            service.apply_saved(config.clone());
            let mut state = service.0.state.lock().unwrap();
            state.output.running = true;
            state.output.colors = config
                .rooms
                .iter()
                .flat_map(|r| &r.lights)
                .enumerate()
                .map(|(i, light)| engine::Color {
                    id: light.id.clone(),
                    rgb: [i as u8, 1, 2],
                })
                .collect();
            for id in [first, second, "esp32-0200000000ff"] {
                let expected = scanned(config, id);
                assert_eq!(state.binding(id).map(str::to_owned), expected);
                let color = expected.and_then(|light_id| {
                    let rgb = state.output.colors.iter().find(|c| c.id == light_id)?.rgb;
                    Some((light_id, rgb))
                });
                assert_eq!(
                    state
                        .color(id)
                        .map(|(light_id, rgb)| (light_id.into_owned(), rgb)),
                    color
                );
            }
        };
        apply_and_compare(&config);
        assert_eq!(
            service.0.state.lock().unwrap().binding(second),
            Some("office-light-bar")
        );
        let target = |light_id: &str| {
            Some(control::Target {
                light_id: light_id.into(),
                address: "192.168.1.40:49600".parse().unwrap(),
                session: [9; 16],
            })
        };
        service
            .0
            .state
            .lock()
            .unwrap()
            .devices
            .get_mut(second)
            .unwrap()
            .connection
            .target = target("office-light-bar");
        // Unbind: the cache forgets the device and its obsolete target is discarded.
        config.revision += 1;
        config.rooms[1].lights[1].output = LightOutput::Virtual;
        apply_and_compare(&config);
        {
            let state = service.0.state.lock().unwrap();
            assert!(state.binding(second).is_none());
            assert!(state.devices[second].connection.target.is_none());
        }
        // Rebind the same device to a different logical light in the other room.
        config.revision += 1;
        config.rooms[1].lights[2].output = LightOutput::Esp32 {
            device_id: second.into(),
        };
        apply_and_compare(&config);
        let mut state = service.0.state.lock().unwrap();
        assert_eq!(state.binding(second), Some("office-light-strip"));
        state.devices.get_mut(second).unwrap().connection.target = target("office-light-bar");
        state.prune_targets();
        assert!(state.devices[second].connection.target.is_none());
        state.devices.get_mut(second).unwrap().connection.target = target("office-light-strip");
        state.prune_targets();
        assert!(state.devices[second].connection.target.is_some());
        drop(state);
        // An older revision cannot regress the cached bindings.
        let mut stale = config.clone();
        stale.revision -= 1;
        stale.rooms[1].lights[2].output = LightOutput::Virtual;
        service.apply_saved(stale);
        assert_eq!(
            service.0.state.lock().unwrap().binding(second),
            Some("office-light-strip")
        );
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
            output_available: true,
            ..State::default()
        };
        state.set_config(config);
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

    #[test]
    fn only_unbound_idle_devices_are_pruned_after_grace() {
        let config: Configuration =
            serde_json::from_str(include_str!("../../../tests/fixtures/configuration.json"))
                .unwrap();
        let LightOutput::Esp32 { device_id } = &config.rooms[0].lights.last().unwrap().output
        else {
            panic!("physical fixture")
        };
        let bound = device_id.clone();
        let (idle, advertised, online) = (
            "esp32-020000a1b2c3",
            "esp32-020000112233",
            "esp32-020000445566",
        );
        let mut state = State {
            config,
            ..State::default()
        };
        for id in [bound.as_str(), idle, advertised] {
            state.devices.insert(id.into(), Device::default());
        }
        state.devices.insert(online.into(), online_device());
        state
            .devices
            .get_mut(advertised)
            .unwrap()
            .advertisements
            .insert(
                "light._iotensity._tcp.local.".into(),
                vec!["192.168.1.40:80".parse().unwrap()],
            );
        let now = Instant::now();
        state.prune_devices(now);
        state.prune_devices(now + DEVICE_GRACE - Duration::from_millis(1));
        assert_eq!(state.devices.len(), 4);
        // Seeing a light again restarts its grace period.
        state.devices.get_mut(idle).unwrap().connection.online = true;
        state.prune_devices(now + DEVICE_GRACE);
        state.devices.get_mut(idle).unwrap().connection.online = false;
        state.prune_devices(now + DEVICE_GRACE);
        state.prune_devices(now + DEVICE_GRACE * 2 - Duration::from_millis(1));
        assert!(state.devices.contains_key(idle));
        state.prune_devices(now + DEVICE_GRACE * 2);
        assert!(!state.devices.contains_key(idle));
        assert!(state.devices.contains_key(advertised));
        assert!(state.devices.contains_key(online));
        // A saved binding is never forgotten, however long it stays offline.
        state.prune_devices(now + DEVICE_GRACE * 100);
        assert!(state.devices.contains_key(&bound));
        state.config.rooms.clear();
        state.prune_devices(now + DEVICE_GRACE * 100);
        state.prune_devices(now + DEVICE_GRACE * 101);
        assert!(!state.devices.contains_key(&bound));
        assert_eq!(state.devices.len(), 2);
    }

    #[test]
    fn replaced_device_entry_ends_old_worker_and_failed_start_backs_off() {
        let id = "esp32-020000a1b2c3";
        let mut state = State::default();
        state.devices.insert(id.into(), online_device());
        let now = Instant::now();
        let first = state.claim_workers(now);
        assert_eq!(first.len(), 1);
        let old = first[0].1;
        assert!(state.claim_workers(now).is_empty());
        assert!(state.worker_wish(id, old).is_some());
        // Evicted and rediscovered before the old worker noticed.
        state.devices.remove(id);
        assert!(state.worker_wish(id, old).is_none());
        state.devices.insert(id.into(), Device::default());
        let new = state.claim_workers(now)[0].1;
        assert_ne!(new, old);
        assert!(state.worker_wish(id, old).is_none());
        assert!(state.worker_wish(id, new).is_some());
        // A stale worker cannot release the current one.
        state.worker_failed(id, old, "stale".into(), now);
        assert_eq!(state.devices[id].worker, Some(new));
        state.worker_failed(id, new, "no client".into(), now);
        assert_eq!(state.snapshot().devices[0].message, "no client");
        assert!(state
            .claim_workers(now + WORKER_RETRY - Duration::from_millis(1))
            .is_empty());
        let retry = state.claim_workers(now + WORKER_RETRY);
        assert_eq!(retry.len(), 1);
        assert!(retry[0].1 > new);
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
    fn identify_timeout_clears_only_its_own_request() {
        let id = "esp32-020000a1b2c3";
        let mut state = State::default();
        state.devices.insert(id.into(), online_device());
        let now = Instant::now();
        let first = state
            .queue_identify(id, mpsc::sync_channel(1).0, IDENTIFY_TIMEOUT, now)
            .unwrap();
        assert_eq!(
            state.queue_identify(id, mpsc::sync_channel(1).0, IDENTIFY_TIMEOUT, now),
            Err("Identify is already pending.".into())
        );
        state.cancel_identify(id, first);
        let second = state
            .queue_identify(id, mpsc::sync_channel(1).0, IDENTIFY_TIMEOUT, now)
            .unwrap();
        // A late cancellation from the first caller must not drop the newer click.
        state.cancel_identify(id, first);
        assert!(state.devices[id].identify.is_some());
        state.cancel_identify(id, second);
        assert!(state.devices[id].identify.is_none());
    }

    #[test]
    fn worker_skips_identify_whose_caller_timed_out() {
        let id = "esp32-020000a1b2c3";
        let mut state = State::default();
        state.devices.insert(id.into(), online_device());
        let now = Instant::now();
        let (reply, _receiver) = mpsc::sync_channel(1);
        state
            .queue_identify(id, reply, IDENTIFY_TIMEOUT, now)
            .unwrap();
        assert!(state.take_identify(id, now + IDENTIFY_TIMEOUT).is_none());
        assert!(state.devices[id].identify.is_none());
        let (reply, _receiver) = mpsc::sync_channel(1);
        state
            .queue_identify(id, reply, IDENTIFY_TIMEOUT, now)
            .unwrap();
        assert!(state.take_identify(id, now).is_some());
        assert!(state.take_identify(id, now).is_none());
    }

    #[test]
    fn timed_out_identify_allows_a_new_request() {
        let id = "esp32-020000a1b2c3";
        let service = HardwareService(Arc::new(Inner {
            state: Mutex::new(State::default()),
            shutdown: AtomicBool::new(false),
            retry_discovery: AtomicBool::new(false),
            client_id: "0".repeat(32),
        }));
        service
            .0
            .state
            .lock()
            .unwrap()
            .devices
            .insert(id.into(), online_device());
        // No worker runs, so every request times out.
        for _ in 0..2 {
            assert_eq!(
                service.identify_within(id, Duration::from_millis(10)),
                Err("Identify timed out. Try again.".into())
            );
            assert!(service.0.state.lock().unwrap().devices[id]
                .identify
                .is_none());
        }
    }

    #[test]
    fn unsaved_preview_renews_without_reconnecting_and_expires_without_sync() {
        let id = "esp32-020000a1b2c3";
        let other = "esp32-020000112233";
        let mut state = State {
            output_available: true,
            ..State::default()
        };
        state.devices.insert(id.into(), online_device());
        state.devices.insert(other.into(), online_device());
        let saved = state.config.clone();
        let now = Instant::now();
        state
            .set_preview(Some(preview_request(id, -3.0)), now)
            .unwrap();
        let wish = state.wish(id).unwrap();
        assert!(wish.light_id.is_some());
        assert!(state.needs_activity()); // App Nap must not throttle the preview.
        assert!(state.wish(other).unwrap().light_id.is_none());
        assert_eq!(state.color(id).unwrap().1, [64, 232, 135]);
        state
            .set_preview(Some(preview_request(id, 3.0)), now + Duration::from_secs(1))
            .unwrap();
        assert_eq!(state.wish(id).unwrap(), wish); // No session change for every mouse movement.
        assert_eq!(state.color(id).unwrap().1, [255, 89, 31]);
        state.expire(now + preview::PREVIEW_DURATION);
        assert!(state.wish(id).unwrap().light_id.is_some()); // Last edit renewed the lease.
        state.expire(now + Duration::from_secs(3));
        assert!(state.wish(id).unwrap().light_id.is_none());
        assert_ne!(state.wish(id).unwrap(), wish); // An in-flight start must be discarded.
        assert!(state.color(id).is_none());
        assert_eq!(state.config, saved);
        assert!(!state.output.running);
        assert!(!state.needs_activity());
    }

    #[test]
    fn selection_switch_cancel_and_invalid_requests_release_previous_light() {
        let id = "esp32-020000a1b2c3";
        let other = "esp32-020000112233";
        let mut state = State {
            output_available: true,
            ..State::default()
        };
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
            last_publish: Some(now),
            ..Default::default()
        };
        state.set_config(config);
        state.devices.insert(id.clone(), online_device());
        state.output.running = true;
        state.output.colors.push(engine::Color {
            id: logical_id,
            rgb: [10, 20, 30],
        });
        let saved = state.config.clone();
        assert!(state.needs_activity());
        let before = state.wish(&id).unwrap();
        state
            .set_preview(Some(preview_request(&id, -3.0)), now)
            .unwrap();
        assert_eq!(state.wish(&id).unwrap(), before);
        assert_eq!(state.color(&id).unwrap().1, [64, 232, 135]);
        let end = now + preview::PREVIEW_DURATION;
        state.last_publish = Some(end);
        state.expire(end);
        assert_eq!(state.wish(&id).unwrap(), before);
        assert_eq!(state.color(&id).unwrap().1, [10, 20, 30]);
        assert_eq!(state.config, saved);
        // A stalled sync publisher still stops, including when a placement preview exists.
        state
            .set_preview(Some(preview_request(&id, 0.0)), end)
            .unwrap();
        state.expire(end + STALL_TIMEOUT);
        assert!(!state.output.running);
        assert!(state.needs_activity()); // The preview lease still holds it.
        state.expire(end + preview::PREVIEW_DURATION);
        assert!(state.color(&id).is_none());
        assert!(!state.needs_activity());
    }

    #[test]
    fn discovery_failure_publishes_error_and_waits_before_retrying() {
        let service = HardwareService::detached();
        let published = Mutex::new(Vec::new());
        let start = Instant::now();
        service.discovery_failed("browse failed".into(), &|snapshot| {
            published.lock().unwrap().push(snapshot)
        });
        assert!(start.elapsed() >= DISCOVERY_RETRY_DELAY);
        let published = published.into_inner().unwrap();
        assert_eq!(published.len(), 1);
        assert_eq!(
            published[0].discovery_error.as_deref(),
            Some("browse failed")
        );
    }

    #[test]
    fn discovery_retry_wait_ends_promptly_on_shutdown() {
        let service = HardwareService::detached();
        service.shutdown();
        let start = Instant::now();
        service.discovery_failed("daemon failed".into(), &|_| {});
        assert!(start.elapsed() < Duration::from_millis(500));
        let shutdown = Arc::new(AtomicBool::new(false));
        let waiter = {
            let shutdown = shutdown.clone();
            thread::spawn(move || {
                let start = Instant::now();
                wait_unless_shutdown(&shutdown, Duration::from_secs(30));
                start.elapsed()
            })
        };
        thread::sleep(Duration::from_millis(50));
        shutdown.store(true, Ordering::Release);
        assert!(waiter.join().unwrap() < Duration::from_secs(5));
    }
}
