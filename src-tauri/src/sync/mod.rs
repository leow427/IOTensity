#[cfg(target_os = "macos")]
mod capture;
pub mod processing;

use crate::config::Configuration;
use processing::{generated_image, LightColor, Processor, OUTPUT_INTERVAL};
use serde::{Deserialize, Serialize};
use std::{
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};
use tauri::Emitter;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    Simulation,
    Test,
    Display,
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Stopped,
    Starting,
    Running,
    Stopping,
    Error,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub sequence: u64,
    pub source: Source,
    pub status: Status,
    pub message: String,
    pub colors: Vec<LightColor>,
}
impl Default for Snapshot {
    fn default() -> Self {
        Self {
            sequence: 0,
            source: Source::Simulation,
            status: Status::Stopped,
            message: "".into(),
            colors: vec![],
        }
    }
}
#[derive(Default)]
struct Runtime {
    config: Option<Configuration>,
    snapshot: Snapshot,
    processor: Processor,
    shutdown: bool,
    reduced_motion: bool,
    simulation: crate::hardware::engine::Simulation,
}

impl Runtime {
    fn update_local_source(&mut self) {
        if self.snapshot.source == Source::Simulation {
            self.snapshot.status = Status::Running;
            self.snapshot.message = "Animated colors".into();
        } else if self.snapshot.source == Source::Test && self.snapshot.status == Status::Starting {
            self.processor.image = Some(generated_image());
            self.snapshot.status = Status::Running;
            self.snapshot.message = "Test image".into();
        }
    }

    fn render(&mut self, dt: f32) {
        if self.snapshot.status != Status::Running {
            return;
        }
        if let Some(config) = &self.config {
            self.snapshot.colors = if self.snapshot.source == Source::Simulation {
                self.simulation
                    .tick(dt as f64, config, self.reduced_motion)
                    .colors
                    .into_iter()
                    .map(|c| LightColor {
                        id: c.id,
                        rgb: c.rgb,
                    })
                    .collect()
            } else {
                self.processor
                    .frame(&config.rooms[0].lights, config.preferences.brightness)
            };
        }
    }
}

// ScreenCaptureKit start/stop completion handlers have no documented deadline;
// a stop normally confirms well under a second. Bound Stopping so a lost
// completion cannot block later starts. Physical output is already off while
// Stopping, and an abandoned native capture still stops itself if a late start
// or stop completion arrives (each handle owns independent native state).
const CAPTURE_STOP_TIMEOUT: Duration = Duration::from_secs(3);

impl Runtime {
    // `native` is the capture's (state, message), or None without a capture.
    // Returns true once Stopping has ended and the capture handle can be dropped.
    fn update_stopping(
        &mut self,
        native: Option<(i32, String)>,
        since: Instant,
        now: Instant,
    ) -> bool {
        let (status, message) = match native {
            None | Some((3, _)) => (Status::Stopped, String::new()),
            Some((4, message)) => (Status::Error, message),
            Some(_) if now.saturating_duration_since(since) >= CAPTURE_STOP_TIMEOUT => (
                Status::Error,
                "Display capture did not confirm that it stopped. Start again to retry, or restart IOTensity if the screen recording indicator remains.".into(),
            ),
            Some(_) => return false,
        };
        self.snapshot.status = status;
        self.snapshot.message = message;
        self.processor.image = None;
        true
    }
}

// ScreenCaptureKit is event-driven: Idle declares unchanged content, not a
// periodic heartbeat. These statuses mirror SCFrameStatus and remain portable
// for source injection; callback age alone cannot identify a silent OS hang
// after Idle. Native publisher stalls are independently bounded by hardware.
#[cfg(any(target_os = "macos", test))]
struct DisplayUpdate {
    state: i32,
    message: String,
    frame_status: i32,
    activity_age: Option<Duration>,
    frame: Option<Result<processing::AnalysisImage, String>>,
}

#[cfg(any(target_os = "macos", test))]
impl Runtime {
    fn update_display(&mut self, update: DisplayUpdate, now: Instant) -> Instant {
        let idle = update.frame_status == 1;
        let observed_at = if idle {
            // A retained explicit Idle state authorizes repetition until a later
            // status/delegate event. It does not promise another callback soon.
            now
        } else {
            update
                .activity_age
                .and_then(|age| now.checked_sub(age))
                .unwrap_or(now)
        };
        let failure = if update.state == 4 {
            Some(update.message)
        } else if matches!(update.state, 2 | 3) {
            Some("Display capture stopped. Restart capture to resume.".into())
        } else if matches!(update.frame_status, 2 | 3 | 5) {
            Some(
                match update.frame_status {
                    2 => "The display is blank. Restart capture when the display is available.",
                    3 => "Display capture is suspended. Restart capture to resume.",
                    _ => "Display capture stopped. Restart capture to resume.",
                }
                .into(),
            )
        } else if !idle
            && self.snapshot.status == Status::Running
            && update
                .activity_age
                .is_some_and(|age| age >= Duration::from_millis(250))
        {
            Some("Display capture stopped responding. Restart capture to resume.".into())
        } else {
            None
        };
        if let Some(message) = failure {
            self.snapshot.status = Status::Error;
            self.snapshot.message = message;
            self.processor.image = None;
        } else if let Some(frame) = update.frame {
            match frame {
                Ok(image) => {
                    self.processor.image = Some(image);
                    self.snapshot.status = Status::Running;
                    self.snapshot.message = "Main display".into();
                }
                Err(error) => {
                    self.snapshot.status = Status::Error;
                    self.snapshot.message = error;
                    self.processor.image = None;
                }
            }
        }
        observed_at
    }
}

#[derive(Clone, Default)]
pub struct SyncService {
    inner: Arc<Mutex<Runtime>>,
}
impl SyncService {
    pub fn spawn(&self, app: tauri::AppHandle, hardware: crate::hardware::HardwareService) {
        let inner = self.inner.clone();
        thread::spawn(move || {
            let mut last = Instant::now();
            let mut last_emitted = u64::MAX;
            #[cfg(target_os = "macos")]
            let mut capture: Option<capture::Capture> = None;
            let mut stopping_since: Option<Instant> = None;
            loop {
                let now = Instant::now();
                let dt = now.duration_since(last).as_secs_f32();
                last = now;
                #[allow(unused_mut)]
                let mut observed_at = now;
                let snapshot = {
                    let mut rt = inner.lock().unwrap();
                    if rt.shutdown {
                        break;
                    }
                    match rt.snapshot.status {
                        Status::Starting | Status::Running => {
                            if rt.snapshot.source != Source::Display {
                                rt.update_local_source();
                            } else {
                                #[cfg(target_os = "macos")]
                                {
                                    let stream =
                                        capture.get_or_insert_with(capture::Capture::start);
                                    observed_at = rt.update_display(stream.poll(), now);
                                    if rt.snapshot.status == Status::Error {
                                        capture = None;
                                    }
                                }
                                #[cfg(not(target_os = "macos"))]
                                {
                                    rt.snapshot.status = Status::Error;
                                    rt.snapshot.message =
                                        "Display capture is available on macOS only.".into();
                                }
                            }
                            rt.render(dt);
                            rt.snapshot.sequence += 1;
                        }
                        Status::Stopping => {
                            #[cfg(target_os = "macos")]
                            let native = capture.as_ref().map(|stream| {
                                stream.stop();
                                stream.state()
                            });
                            #[cfg(not(target_os = "macos"))]
                            let native = None;
                            let since = *stopping_since.get_or_insert(now);
                            if rt.update_stopping(native, since, now) {
                                stopping_since = None;
                                #[cfg(target_os = "macos")]
                                {
                                    // Dropping releases only Rust's reference; pending
                                    // native completions finish on the abandoned object.
                                    capture = None;
                                }
                            }
                            rt.snapshot.sequence += 1;
                        }
                        _ => {}
                    }
                    rt.snapshot.clone()
                };
                hardware.publish_at(
                    &snapshot.colors,
                    snapshot.status == Status::Running,
                    observed_at,
                );
                if snapshot.sequence != last_emitted {
                    last_emitted = snapshot.sequence;
                    let _ = app.emit("sync-output", snapshot);
                }
                thread::sleep(OUTPUT_INTERVAL); // Never catch up with a burst after a slow frame.
            }
        });
    }
    // Called only with a native load/save acknowledgement, never a frontend draft.
    pub fn apply_saved(&self, config: Configuration) {
        let mut rt = self.inner.lock().unwrap();
        if rt
            .config
            .as_ref()
            .is_some_and(|old| old.revision > config.revision)
        {
            return;
        }
        rt.snapshot
            .colors
            .retain(|color| config.rooms[0].lights.iter().any(|l| l.id == color.id));
        if config.rooms[0].lights.is_empty()
            && matches!(rt.snapshot.status, Status::Starting | Status::Running)
        {
            rt.snapshot.status = Status::Stopping;
        }
        rt.config = Some(config);
        rt.snapshot.sequence += 1;
    }
    pub fn set_reduced_motion(&self, reduced_motion: bool) {
        self.inner.lock().unwrap().reduced_motion = reduced_motion;
    }
    pub fn start(&self, source: Source) -> Result<Snapshot, String> {
        let mut rt = self.inner.lock().unwrap();
        if matches!(
            rt.snapshot.status,
            Status::Starting | Status::Running | Status::Stopping
        ) {
            return Err("Sync is already active.".into());
        }
        if rt
            .config
            .as_ref()
            .is_none_or(|c| c.rooms[0].lights.is_empty())
        {
            return Err("Save at least one light before starting sync.".into());
        }
        rt.snapshot.source = source;
        rt.snapshot.status = Status::Starting;
        rt.processor.image = None;
        rt.snapshot.message = match source {
            Source::Simulation | Source::Test => "Starting…",
            Source::Display => "Starting capture… Allow Screen Recording if prompted.",
        }
        .into();
        rt.snapshot.sequence += 1;
        Ok(rt.snapshot.clone())
    }
    pub fn stop(&self) -> Snapshot {
        let mut rt = self.inner.lock().unwrap();
        if matches!(rt.snapshot.status, Status::Starting | Status::Running) {
            rt.snapshot.status = Status::Stopping;
            rt.snapshot.message = "Stopping capture…".into();
            rt.snapshot.sequence += 1;
        }
        rt.snapshot.clone()
    }
    pub fn snapshot(&self) -> Snapshot {
        self.inner.lock().unwrap().snapshot.clone()
    }
    pub fn is_shutdown(&self) -> bool {
        self.inner.lock().unwrap().shutdown
    }
    pub fn shutdown(&self) {
        self.inner.lock().unwrap().shutdown = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{ConfigStore, Position};

    fn configured(source: Source) -> SyncService {
        let service = SyncService::default();
        service.apply_saved(
            serde_json::from_str(include_str!("../../../tests/fixtures/configuration.json"))
                .unwrap(),
        );
        service.start(source).unwrap();
        service
    }

    fn display_update(frame_status: i32, age_ms: u64, with_frame: bool) -> DisplayUpdate {
        DisplayUpdate {
            state: 1,
            message: String::new(),
            frame_status,
            activity_age: Some(Duration::from_millis(age_ms)),
            frame: with_frame.then(|| Ok(generated_image())),
        }
    }

    #[test]
    fn cached_display_polls_do_not_renew_the_source_watchdog() {
        let service = configured(Source::Display);
        let now = Instant::now();
        let mut rt = service.inner.lock().unwrap();
        assert_eq!(rt.update_display(display_update(0, 0, true), now), now);
        rt.render(0.033);
        let held = rt.snapshot.colors.clone();
        assert!(!held.is_empty());
        for age_ms in [33, 100, 249] {
            let at = now + Duration::from_millis(age_ms);
            assert_eq!(rt.update_display(display_update(0, age_ms, false), at), now);
            rt.render(0.033);
            assert_eq!(rt.snapshot.status, Status::Running);
            assert_eq!(rt.snapshot.colors, held);
        }
        rt.update_display(
            display_update(0, 250, false),
            now + Duration::from_millis(250),
        );
        rt.render(0.033);
        assert_eq!(rt.snapshot.status, Status::Error);
        assert!(rt.processor.image.is_none());
        assert_eq!(rt.snapshot.colors, held); // Virtual colors hold; physical output stops.
        drop(rt);
        assert_eq!(
            service.start(Source::Test).unwrap().status,
            Status::Starting
        );
        let mut rt = service.inner.lock().unwrap();
        rt.update_local_source();
        rt.render(0.033);
        assert_eq!(rt.snapshot.status, Status::Running);
    }

    #[test]
    fn fresh_idle_samples_and_static_test_source_repeat_without_pixel_changes() {
        let service = configured(Source::Display);
        let now = Instant::now();
        let mut rt = service.inner.lock().unwrap();
        rt.update_display(display_update(0, 0, true), now);
        rt.render(0.033);
        let held = rt.snapshot.colors.clone();
        for elapsed in [100, 1_000, 60_000] {
            let at = now + Duration::from_millis(elapsed);
            assert_eq!(rt.update_display(display_update(1, 0, false), at), at);
            rt.render(0.033);
            assert_eq!(rt.snapshot.status, Status::Running);
            assert_eq!(rt.snapshot.colors, held);
        }
        drop(rt);
        let test = configured(Source::Test);
        let mut rt = test.inner.lock().unwrap();
        for dt in [0.033, 1.0, 60.0] {
            rt.update_local_source();
            rt.render(dt);
            assert_eq!(rt.snapshot.status, Status::Running);
            assert_eq!(rt.snapshot.colors, held);
        }
    }

    #[test]
    fn blank_suspended_stopped_and_failed_samples_stop_output_and_release_image() {
        for frame_status in [2, 3, 5] {
            let service = configured(Source::Display);
            let now = Instant::now();
            let mut rt = service.inner.lock().unwrap();
            rt.update_display(display_update(0, 0, true), now);
            rt.render(0.033);
            let held = rt.snapshot.colors.clone();
            rt.update_display(display_update(1, 60_000, false), now);
            assert_eq!(rt.snapshot.status, Status::Running);
            // Even a pending old Complete buffer cannot override these statuses.
            rt.update_display(display_update(frame_status, 0, true), now);
            rt.render(0.033);
            assert_eq!(rt.snapshot.status, Status::Error);
            assert!(rt.processor.image.is_none());
            assert_eq!(rt.snapshot.colors, held);
        }
        for failed_decode in [false, true] {
            let service = configured(Source::Display);
            let mut update = display_update(0, 0, true);
            if failed_decode {
                update.frame = Some(Err("Invalid frame".into()));
            } else {
                update.state = 4;
                update.message = "Capture failed".into();
            }
            let mut rt = service.inner.lock().unwrap();
            rt.update_display(update, Instant::now());
            assert_eq!(rt.snapshot.status, Status::Error);
            assert!(rt.processor.image.is_none());
        }
    }

    #[test]
    fn startup_waits_for_first_image_without_treating_start_ack_as_a_frame() {
        let service = configured(Source::Display);
        let mut rt = service.inner.lock().unwrap();
        for (status, age) in [(-1, 0), (-1, 250), (4, 1_000), (1, 60_000)] {
            rt.update_display(display_update(status, age, false), Instant::now());
            rt.render(0.033);
            assert_eq!(rt.snapshot.status, Status::Starting);
            assert!(rt.snapshot.colors.is_empty());
        }
        // Start completion has no API deadline for the first Complete frame;
        // no physical output is requested before that frame arrives.
        rt.update_display(display_update(0, 0, true), Instant::now());
        rt.render(0.033);
        assert_eq!(rt.snapshot.status, Status::Running);
        assert!(!rt.snapshot.colors.is_empty());
    }

    #[test]
    fn explicit_idle_persists_without_a_callback_heartbeat_but_can_be_invalidated() {
        let service = configured(Source::Display);
        let mut rt = service.inner.lock().unwrap();
        let now = Instant::now();
        rt.update_display(display_update(0, 0, true), now);
        rt.render(0.033);
        let held = rt.snapshot.colors.clone();
        for elapsed in [33, 250, 1_000, 60_000] {
            let at = now + Duration::from_millis(elapsed);
            assert_eq!(rt.update_display(display_update(1, elapsed, false), at), at);
            rt.render(0.033);
            assert_eq!(rt.snapshot.status, Status::Running);
            assert_eq!(rt.snapshot.colors, held);
        }
        // Once new content is declared, missing source progress is subject to
        // the 250 ms watchdog again. Idle is not a permanently latched exemption.
        let changed = now + Duration::from_secs(61);
        rt.update_display(display_update(0, 0, true), changed);
        let stalled = changed + Duration::from_millis(250);
        assert_eq!(
            rt.update_display(display_update(0, 250, false), stalled),
            changed
        );
        assert_eq!(rt.snapshot.status, Status::Error);
    }

    #[test]
    fn display_stop_ends_on_native_confirmation_failure_or_deadline() {
        let since = Instant::now();
        for (native, status, message) in [
            (None, Status::Stopped, ""),
            (Some((3, String::new())), Status::Stopped, ""),
            (
                Some((4, "Could not stop".into())),
                Status::Error,
                "Could not stop",
            ),
        ] {
            let service = configured(Source::Display);
            service.stop();
            let mut rt = service.inner.lock().unwrap();
            assert!(rt.update_stopping(native, since, since));
            assert_eq!(rt.snapshot.status, status);
            assert_eq!(rt.snapshot.message, message);
        }

        // A start or stop completion that never arrives leaves the native state
        // at stopping (2); the deadline releases the handle so Sync can restart.
        let service = configured(Source::Display);
        let now = Instant::now();
        {
            let mut rt = service.inner.lock().unwrap();
            rt.update_display(display_update(0, 0, true), now);
            rt.render(0.033);
        }
        let held = service.snapshot().colors;
        assert!(!held.is_empty());
        assert_eq!(service.stop().status, Status::Stopping);
        let mut rt = service.inner.lock().unwrap();
        for elapsed in [0, 33, 1_000, 2_999] {
            let at = now + Duration::from_millis(elapsed);
            assert!(!rt.update_stopping(Some((2, String::new())), now, at));
            assert_eq!(rt.snapshot.status, Status::Stopping);
        }
        drop(rt);
        assert!(service.start(Source::Display).is_err());
        let mut rt = service.inner.lock().unwrap();
        assert!(rt.update_stopping(Some((2, String::new())), now, now + CAPTURE_STOP_TIMEOUT));
        assert_eq!(rt.snapshot.status, Status::Error);
        assert!(rt
            .snapshot
            .message
            .contains("did not confirm that it stopped"));
        assert!(rt.processor.image.is_none());
        assert_eq!(rt.snapshot.colors, held); // Virtual colors hold; physical output is off.
        drop(rt);
        assert_eq!(
            service.start(Source::Display).unwrap().status,
            Status::Starting
        );
    }

    #[test]
    fn reduced_motion_updates_an_active_simulation_without_restarting_it() {
        let service = configured(Source::Simulation);
        {
            let mut rt = service.inner.lock().unwrap();
            rt.update_local_source();
            rt.render(0.033);
        }
        service.set_reduced_motion(true);
        let mut rt = service.inner.lock().unwrap();
        assert_eq!(rt.snapshot.status, Status::Running);
        assert!(rt.reduced_motion);
        rt.render(0.033);
        assert!(!rt.snapshot.colors.is_empty());
    }

    #[test]
    fn only_acknowledged_revisions_retarget_the_output() {
        let directory = tempfile::tempdir().unwrap();
        let disk = ConfigStore::new(directory.path().join("configuration.json"));
        let config: Configuration =
            serde_json::from_str(include_str!("../../../tests/fixtures/configuration.json"))
                .unwrap();
        let saved = disk.save(config, 0).unwrap();
        let service = SyncService::default();
        service.apply_saved(saved.clone());
        let mut draft = saved.clone();
        draft.rooms[0].lights[0].position = Position {
            x: 3.0,
            y: 3.0,
            z: 4.0,
        };
        assert_eq!(
            service.inner.lock().unwrap().config.as_ref().unwrap(),
            &saved
        );
        assert!(disk.save(draft.clone(), 0).is_err());
        assert_eq!(
            service.inner.lock().unwrap().config.as_ref().unwrap(),
            &saved
        );
        let acknowledged = disk.save(draft, saved.revision).unwrap();
        service.apply_saved(acknowledged.clone());
        service.apply_saved(saved); // A delayed load/save result cannot regress sampling.
        assert_eq!(
            service.inner.lock().unwrap().config.as_ref().unwrap(),
            &acknowledged
        );
        assert_eq!(service.snapshot().status, Status::Stopped);
    }

    #[test]
    fn starts_stopped_and_rejects_duplicate_or_empty_start() {
        let service = SyncService::default();
        assert_eq!(service.snapshot().status, Status::Stopped);
        assert!(service.start(Source::Test).is_err());
        let config: Configuration =
            serde_json::from_str(include_str!("../../../tests/fixtures/configuration.json"))
                .unwrap();
        service.apply_saved(config);
        assert_eq!(
            service.start(Source::Test).unwrap().status,
            Status::Starting
        );
        assert!(service.start(Source::Display).is_err());
        assert_eq!(service.stop().status, Status::Stopping);
        assert!(service.start(Source::Test).is_err());
    }
}
