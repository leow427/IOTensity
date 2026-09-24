use super::protocol::{hex, parse_token, token, Session, VERSION};
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use std::{io::Read, net::SocketAddrV4, time::Duration};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub device_id: String,
    pub model: String,
    pub protocol: u8,
    pub udp_port: u16,
    pub max_fps: u8,
    pub session_id: Option<String>,
}
impl Status {
    fn verify(&self, id: &str) -> Result<(), String> {
        if self.device_id != id
            || self.model != "esp32-rgb"
            || self.protocol != VERSION
            || self.udp_port == 0
            || self.max_fps != 30
        {
            return Err("Device identity or capabilities do not match the advertisement.".into());
        }
        if self
            .session_id
            .as_ref()
            .is_some_and(|id| parse_token(id).is_none())
        {
            return Err("Device returned an invalid session.".into());
        }
        Ok(())
    }
}
pub trait Control {
    fn status(&self, endpoint: SocketAddrV4) -> Result<Status, String>;
    fn start(
        &self,
        endpoint: SocketAddrV4,
        id: &str,
        client: &str,
        request: &str,
    ) -> Result<Status, String>;
    fn stop(&self, endpoint: SocketAddrV4, id: &str, session: Session) -> Result<(), String>;
    fn identify(&self, endpoint: SocketAddrV4, id: &str) -> Result<(), String>;
}
pub struct HttpControl(Client);
impl HttpControl {
    pub fn new() -> Result<Self, String> {
        Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_millis(300))
            .timeout(Duration::from_millis(600))
            .build()
            .map(Self)
            .map_err(|e| e.to_string())
    }
    fn request(
        &self,
        endpoint: SocketAddrV4,
        path: &str,
        body: Option<serde_json::Value>,
    ) -> Result<Status, String> {
        let url = format!("http://{endpoint}/v1/{path}");
        let request = match body {
            Some(body) => self.0.post(url).json(&body),
            None => self.0.get(url),
        };
        let response = request
            .send()
            .map_err(|_| "Device is not responding on the local network.".to_string())?;
        if response.status().as_u16() == 409 {
            return Err("Busy: another computer is streaming to this light.".into());
        }
        if !response.status().is_success() {
            return Err(format!(
                "Device rejected control request ({}).",
                response.status()
            ));
        }
        let mut bytes = Vec::new();
        response
            .take(8193)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        if bytes.len() > 8192 {
            return Err("Device response is too large.".into());
        }
        serde_json::from_slice(&bytes).map_err(|_| "Invalid device response.".into())
    }
}
impl Control for HttpControl {
    fn status(&self, endpoint: SocketAddrV4) -> Result<Status, String> {
        self.request(endpoint, "status", None)
    }
    fn start(
        &self,
        endpoint: SocketAddrV4,
        id: &str,
        client: &str,
        request: &str,
    ) -> Result<Status, String> {
        self.request(endpoint, "stream/start", Some(serde_json::json!({"deviceId":id, "protocol": VERSION, "clientId":client, "requestId":request})))
    }
    fn stop(&self, endpoint: SocketAddrV4, id: &str, session: Session) -> Result<(), String> {
        self.request(
            endpoint,
            "stream/stop",
            Some(serde_json::json!({"deviceId":id, "sessionId":hex(&session)})),
        )?
        .verify(id)
    }
    fn identify(&self, endpoint: SocketAddrV4, id: &str) -> Result<(), String> {
        self.request(
            endpoint,
            "identify",
            Some(serde_json::json!({"deviceId":id})),
        )?
        .verify(id)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Wish {
    pub epoch: u64,
    pub light_id: Option<String>,
    pub running: bool,
    pub endpoints: Vec<SocketAddrV4>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Target {
    pub light_id: String,
    pub address: SocketAddrV4,
    pub session: Session,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ConnectionState {
    pub online: bool,
    pub target: Option<Target>,
    pub message: String,
}
pub struct Connection {
    id: String,
    client: String,
    wish: Option<Wish>,
    endpoint: Option<SocketAddrV4>,
    session: Option<Session>,
    request: Option<String>,
    cursor: usize,
    failures: u32,
    next_check: u64,
    pub state: ConnectionState,
}
impl Connection {
    pub fn new(id: String, client: String) -> Self {
        Self {
            id,
            client,
            wish: None,
            endpoint: None,
            session: None,
            request: None,
            cursor: 0,
            failures: 0,
            next_check: 0,
            state: ConnectionState::default(),
        }
    }
    pub fn close(&mut self, control: &impl Control) {
        if let (Some(endpoint), Some(session)) = (self.endpoint, self.session.take()) {
            let _ = control.stop(endpoint, &self.id, session);
        }
        self.state.target = None;
        self.request = None;
    }
    pub fn identify(&self, control: &impl Control) -> Result<(), String> {
        let endpoint = self.endpoint.ok_or("Light is offline.")?;
        control.status(endpoint)?.verify(&self.id)?;
        control.identify(endpoint, &self.id)
    }
    /// The clock and control transport are injected so reconnect tests need no sleeps.
    pub fn step(&mut self, now: u64, wish: &Wish, control: &impl Control) -> &ConnectionState {
        if self.wish.as_ref() != Some(wish) {
            self.close(control);
            self.wish = Some(wish.clone());
            self.endpoint = None;
            self.failures = 0;
            self.next_check = now;
        }
        if now < self.next_check {
            return &self.state;
        }
        self.next_check = now + 500;
        self.state.target = None;
        if wish.endpoints.is_empty() {
            self.state = ConnectionState {
                message: "Offline · waiting for discovery".into(),
                ..Default::default()
            };
            return &self.state;
        }
        let endpoint = self
            .endpoint
            .unwrap_or(wish.endpoints[self.cursor % wish.endpoints.len()]);
        let result = self.check(endpoint, wish, control);
        match result {
            Ok(target) => {
                self.failures = 0;
                self.endpoint = Some(endpoint);
                self.state = ConnectionState {
                    online: true,
                    message: if target.is_some() {
                        "Streaming"
                    } else {
                        "Online"
                    }
                    .into(),
                    target,
                };
            }
            Err(error) => {
                let busy = error.starts_with("Busy:");
                self.state = ConnectionState {
                    online: busy,
                    target: None,
                    message: error,
                };
                self.failures = self.failures.saturating_add(1);
                self.next_check = now + (500_u64 << self.failures.min(3));
                self.cursor = self.cursor.wrapping_add(1);
                self.endpoint = None;
                self.session = None;
                // Retain requestId through lost start responses: retry is idempotent.
            }
        }
        &self.state
    }
    fn check(
        &mut self,
        endpoint: SocketAddrV4,
        wish: &Wish,
        control: &impl Control,
    ) -> Result<Option<Target>, String> {
        let status = control.status(endpoint)?;
        status.verify(&self.id)?;
        let Some(light_id) = wish.light_id.as_ref().filter(|_| wish.running) else {
            return Ok(None);
        };
        if let Some(session) = self.session {
            if status.session_id.as_deref() == Some(&hex(&session)) {
                return Ok(Some(Target {
                    light_id: light_id.clone(),
                    address: SocketAddrV4::new(*endpoint.ip(), status.udp_port),
                    session,
                }));
            }
            // Restart / timeout invalidated the previous association. Never reuse it.
            self.session = None;
            self.request = None;
        }
        let request = self.request.get_or_insert(hex(&token()?));
        let started = control.start(endpoint, &self.id, &self.client, request)?;
        started.verify(&self.id)?;
        let session = started
            .session_id
            .as_deref()
            .and_then(parse_token)
            .ok_or("Device did not establish a stream.")?;
        self.session = Some(session);
        Ok(Some(Target {
            light_id: light_id.clone(),
            address: SocketAddrV4::new(*endpoint.ip(), started.udp_port),
            session,
        }))
    }
}
