#[cfg(target_os = "macos")]
mod capture;
pub mod processing;

use crate::config::Configuration;
use processing::{generated_image, LightColor, Processor, OUTPUT_INTERVAL};
use serde::{Deserialize, Serialize};
#[cfg(any(target_os = "macos", test))]
use std::time::Duration;
use std::{
    sync::{Arc, Mutex},
    thread,
    time::Instant,
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
            loop {
                let now = Instant::now();
                let dt = now.duration_since(last).as_secs_f32();
                last = now;
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
                                    rt.update_display(stream.poll());
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
                            #[cfg(not(target_os = "macos"))]
                            let stopped = true;
                            #[cfg(target_os = "macos")]
                            let mut stopped = true;
                            #[cfg(target_os = "macos")]
                            if let Some(stream) = &capture {
                                stream.stop();
                                let (state, message) = stream.state();
                                stopped = state == 3;
                                if state == 4 {
                                    rt.snapshot.status = Status::Error;
                                    rt.snapshot.message = message;
                                    capture = None;
                                } else if stopped {
                                    capture = None;
                                }
                            }
                            if stopped {
                                rt.snapshot.status = Status::Stopped;
                                rt.snapshot.message = "".into();
                                rt.processor.image = None;
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
