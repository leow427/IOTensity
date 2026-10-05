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
                // Every room shares the same screen mapping, in the same light
                // order as the simulation and hardware bindings.
                self.processor.frame(
                    config.rooms.iter().flat_map(|r| &r.lights),
                    config.preferences.brightness,
                )
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
// after Idle. Capture progress is judged here, once per tick; hardware bounds
// only stalls of this publisher, so it cannot expire output that sync accepts.
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
    fn wants_display_frame(&self) -> bool {
        self.snapshot.source == Source::Display
            && matches!(self.snapshot.status, Status::Starting | Status::Running)
    }

    // Applies a poll that was decoded without the runtime lock. A Stop or
    // shutdown that took the lock meanwhile wins and discards the stale frame;
    // a source started meanwhile waits for its own first Complete frame.
    fn apply_display_poll(&mut self, update: Option<DisplayUpdate>) {
        if let Some(update) = update.filter(|_| self.wants_display_frame()) {
            self.update_display(update);
        }
    }

    fn update_display(&mut self, update: DisplayUpdate) {
        // A retained explicit Idle state authorizes repetition until a later
        // status/delegate event. It does not promise another callback soon.
        let idle = update.frame_status == 1;
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
                .is_some_and(|age| age >= crate::hardware::STALL_TIMEOUT)
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
                // Decoding a full-resolution frame touches every pixel, so it
                // runs without the runtime lock: Stop, start, snapshots, saves
                // and shutdown never wait behind it.
                #[cfg(target_os = "macos")]
                let update = {
                    let wanted = {
                        let rt = inner.lock().unwrap();
                        if rt.shutdown {
                            break;
                        }
                        rt.wants_display_frame()
                    };
                    wanted.then(|| capture.get_or_insert_with(capture::Capture::start).poll())
                };
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
                                    rt.apply_display_poll(update);
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
                hardware.publish_at(&snapshot.colors, snapshot.status == Status::Running, now);
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
        rt.snapshot.colors.retain(|color| {
            config
                .rooms
                .iter()
                .flat_map(|r| &r.lights)
                .any(|l| l.id == color.id)
        });
        if config.rooms.iter().all(|r| r.lights.is_empty())
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
            .is_none_or(|c| c.rooms.iter().all(|r| r.lights.is_empty()))
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
        let mut rt = service.inner.lock().unwrap();
        rt.update_display(display_update(0, 0, true));
        rt.render(0.033);
        let held = rt.snapshot.colors.clone();
        assert!(!held.is_empty());
        for age_ms in [33, 100, 249] {
            rt.update_display(display_update(0, age_ms, false));
            rt.render(0.033);
            assert_eq!(rt.snapshot.status, Status::Running);
            assert_eq!(rt.snapshot.colors, held);
        }
        rt.update_display(display_update(0, 250, false));
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
    fn capture_gaps_sync_accepts_never_expire_physical_output_between_ticks() {
        let service = configured(Source::Display);
        let hardware = crate::hardware::HardwareService::detached();
        let gap = crate::hardware::STALL_TIMEOUT - Duration::from_millis(1);
        let start = Instant::now();
        let mut rt = service.inner.lock().unwrap();
        let mut running_epoch = None;
        let mut last_capture = start;
        // Complete frames arrive every 249 ms for two seconds of ~30 Hz ticks.
        for tick in 0..60 {
            let now = start + OUTPUT_INTERVAL * tick;
            let fresh = now >= last_capture + gap || tick == 0;
            if fresh {
                last_capture = now;
            }
            let age = now.duration_since(last_capture).as_millis() as u64;
            rt.update_display(display_update(0, age, fresh));
            rt.render(OUTPUT_INTERVAL.as_secs_f32());
            assert_eq!(rt.snapshot.status, Status::Running);
            hardware.publish_at(&rt.snapshot.colors, true, now);
            // The output thread may run just before the next sync tick.
            let (running, epoch) = hardware.expire_at(now + OUTPUT_INTERVAL);
            assert!(running);
            assert_eq!(*running_epoch.get_or_insert(epoch), epoch); // No session restarts.
        }
        // A real capture stall stops physical output at the first tick at or
        // after 250 ms without progress.
        let deadline = last_capture + crate::hardware::STALL_TIMEOUT + OUTPUT_INTERVAL;
        let mut now = start + OUTPUT_INTERVAL * 60;
        while rt.snapshot.status == Status::Running {
            assert!(now <= deadline);
            let age = now.duration_since(last_capture).as_millis() as u64;
            rt.update_display(display_update(0, age, false));
            rt.render(OUTPUT_INTERVAL.as_secs_f32());
            hardware.publish_at(
                &rt.snapshot.colors,
                rt.snapshot.status == Status::Running,
                now,
            );
            now += OUTPUT_INTERVAL;
        }
        assert_eq!(rt.snapshot.status, Status::Error);
        let (running, epoch) = hardware.expire_at(now);
        assert!(!running);
        assert_eq!(epoch, running_epoch.unwrap() + 1);
    }

    #[test]
    fn fresh_idle_samples_and_static_test_source_repeat_without_pixel_changes() {
        let service = configured(Source::Display);
        let mut rt = service.inner.lock().unwrap();
        rt.update_display(display_update(0, 0, true));
        rt.render(0.033);
        let held = rt.snapshot.colors.clone();
        for _ in 0..3 {
            rt.update_display(display_update(1, 0, false));
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
            let mut rt = service.inner.lock().unwrap();
            rt.update_display(display_update(0, 0, true));
            rt.render(0.033);
            let held = rt.snapshot.colors.clone();
            rt.update_display(display_update(1, 60_000, false));
            assert_eq!(rt.snapshot.status, Status::Running);
            // Even a pending old Complete buffer cannot override these statuses.
            rt.update_display(display_update(frame_status, 0, true));
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
            rt.update_display(update);
            assert_eq!(rt.snapshot.status, Status::Error);
            assert!(rt.processor.image.is_none());
        }
    }

    #[test]
    fn startup_waits_for_first_image_without_treating_start_ack_as_a_frame() {
        let service = configured(Source::Display);
        let mut rt = service.inner.lock().unwrap();
        for (status, age) in [(-1, 0), (-1, 250), (4, 1_000), (1, 60_000)] {
            rt.update_display(display_update(status, age, false));
            rt.render(0.033);
            assert_eq!(rt.snapshot.status, Status::Starting);
            assert!(rt.snapshot.colors.is_empty());
        }
        // Start completion has no API deadline for the first Complete frame;
        // no physical output is requested before that frame arrives.
        rt.update_display(display_update(0, 0, true));
        rt.render(0.033);
        assert_eq!(rt.snapshot.status, Status::Running);
        assert!(!rt.snapshot.colors.is_empty());
    }

    #[test]
    fn explicit_idle_persists_without_a_callback_heartbeat_but_can_be_invalidated() {
        let service = configured(Source::Display);
        let mut rt = service.inner.lock().unwrap();
        rt.update_display(display_update(0, 0, true));
        rt.render(0.033);
        let held = rt.snapshot.colors.clone();
        for elapsed in [33, 250, 1_000, 60_000] {
            rt.update_display(display_update(1, elapsed, false));
            rt.render(0.033);
            assert_eq!(rt.snapshot.status, Status::Running);
            assert_eq!(rt.snapshot.colors, held);
        }
        // Once new content is declared, missing source progress is subject to
        // the 250 ms watchdog again. Idle is not a permanently latched exemption.
        rt.update_display(display_update(0, 0, true));
        rt.update_display(display_update(0, 250, false));
        assert_eq!(rt.snapshot.status, Status::Error);
    }

    #[test]
    fn frames_decoded_outside_the_lock_cannot_override_a_later_stop() {
        let service = configured(Source::Display);
        assert!(service.inner.lock().unwrap().wants_display_frame());
        // The output thread decodes here without the runtime lock, so Stop wins it.
        let decoded = display_update(0, 0, true);
        assert_eq!(service.stop().status, Status::Stopping);
        let mut rt = service.inner.lock().unwrap();
        assert!(!rt.wants_display_frame());
        rt.apply_display_poll(Some(decoded));
        assert_eq!(rt.snapshot.status, Status::Stopping);
        assert!(rt.processor.image.is_none());
        rt.snapshot.status = Status::Stopped;
        drop(rt);
        // A source started after the unlocked decision waits for its own poll.
        assert_eq!(
            service.start(Source::Display).unwrap().status,
            Status::Starting
        );
        let mut rt = service.inner.lock().unwrap();
        rt.apply_display_poll(None);
        assert_eq!(rt.snapshot.status, Status::Starting);
        assert!(rt.processor.image.is_none());
        rt.apply_display_poll(Some(display_update(0, 0, true)));
        assert_eq!(rt.snapshot.status, Status::Running);
        assert!(rt.processor.image.is_some());
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
            rt.update_display(display_update(0, 0, true));
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
    fn display_and_test_sources_drive_lights_in_every_room() {
        let single: Configuration =
            serde_json::from_str(include_str!("../../../tests/fixtures/configuration.json"))
                .unwrap();
        let mut split = single.clone();
        let moved = split.rooms[0].lights.split_off(2);
        split.rooms.push(crate::config::Room {
            id: "den".into(),
            name: "Den".into(),
            lights: moved,
        });
        let render = |config: &Configuration, source: Source| {
            let service = SyncService::default();
            service.apply_saved(config.clone());
            service.start(source).unwrap();
            let mut rt = service.inner.lock().unwrap();
            if source == Source::Display {
                rt.update_display(display_update(0, 0, true));
            } else {
                rt.update_local_source();
            }
            rt.render(0.033);
            assert_eq!(rt.snapshot.status, Status::Running);
            rt.snapshot.colors.clone()
        };
        let simulated: Vec<_> = crate::hardware::engine::Simulation::default()
            .tick(0.033, &split, false)
            .colors
            .into_iter()
            .map(|c| c.id)
            .collect();
        for source in [Source::Display, Source::Test] {
            let colors = render(&split, source);
            // Same lights, same order as the simulation; rooms share one mapping.
            assert_eq!(
                colors.iter().map(|c| c.id.clone()).collect::<Vec<_>>(),
                simulated
            );
            assert_eq!(colors, render(&single, source));
            assert!(colors
                .iter()
                .any(|c| c.id == split.rooms[1].lights[0].id && c.rgb != [0; 3]));
        }
        // A configuration whose only lights are outside the first room can start.
        let mut later_only = split.clone();
        later_only.rooms[0].lights.clear();
        let colors = render(&later_only, Source::Test);
        assert_eq!(colors.len(), later_only.rooms[1].lights.len());
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
