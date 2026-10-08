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
    #[cfg(any(target_os = "macos", test))]
    capture_retry: Option<CaptureRetry>,
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
            Some((4 | 5, message)) => (Status::Error, message),
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

// A transient capture failure (stall, Blank, Suspended, Stopped, a stream or
// frame error) releases the capture and starts a new one while Sync remains
// requested. Physical output is off until the new capture's first Complete
// frame; virtual colors hold. Native state 5 marks failures only the user can
// fix (permission, a stop chosen in macOS), which end in Error instead.
#[cfg(any(target_os = "macos", test))]
struct CaptureRetry {
    failures: u32,
    failed_at: Instant,
    retry_at: Instant,
}

#[cfg(any(target_os = "macos", test))]
impl CaptureRetry {
    const FIRST: Duration = Duration::from_millis(500);
    const MAX: Duration = Duration::from_secs(5);
    /// Failures further apart than this restart the backoff from `FIRST`.
    const RESET: Duration = Duration::from_secs(30);

    fn after(previous: Option<&Self>, now: Instant) -> Self {
        let failures = match previous {
            Some(previous) if now.saturating_duration_since(previous.failed_at) < Self::RESET => {
                previous.failures.saturating_add(1)
            }
            _ => 1,
        };
        let delay = Self::FIRST
            .saturating_mul(1 << (failures - 1).min(4))
            .min(Self::MAX);
        Self {
            failures,
            failed_at: now,
            retry_at: now + delay,
        }
    }
}

#[cfg(any(target_os = "macos", test))]
impl Runtime {
    fn wants_display_frame(&self) -> bool {
        self.snapshot.source == Source::Display
            && matches!(self.snapshot.status, Status::Starting | Status::Running)
    }

    /// Whether to poll the capture this tick, creating it if needed. A capture
    /// released after a transient failure is recreated after its retry delay.
    fn wants_capture(&self, now: Instant) -> bool {
        self.wants_display_frame()
            && self
                .capture_retry
                .as_ref()
                .is_none_or(|retry| now >= retry.retry_at)
    }

    // Applies a poll that was decoded without the runtime lock. A Stop or
    // shutdown that took the lock meanwhile wins and discards the stale frame;
    // a source started meanwhile waits for its own first Complete frame.
    // Returns true when the capture handle must be released.
    fn apply_display_poll(&mut self, update: Option<DisplayUpdate>, now: Instant) -> bool {
        update
            .filter(|_| self.wants_display_frame())
            .is_some_and(|update| self.update_display(update, now))
    }

    // Returns true when the capture handle must be released.
    fn update_display(&mut self, update: DisplayUpdate, now: Instant) -> bool {
        if update.state == 5 {
            self.capture_retry = None;
            self.snapshot.status = Status::Error;
            self.snapshot.message = update.message;
            self.processor.image = None;
            return true;
        }
        // A retained explicit Idle state authorizes repetition until a later
        // status/delegate event. It does not promise another callback soon.
        let idle = update.frame_status == 1;
        let reason = if update.state == 4 {
            update.message
        } else if matches!(update.state, 2 | 3) {
            "Display capture stopped.".into()
        } else if matches!(update.frame_status, 2 | 3 | 5) {
            match update.frame_status {
                2 => "The display is blank.",
                3 => "Display capture is suspended.",
                _ => "Display capture stopped.",
            }
            .into()
        } else if !idle
            && self.snapshot.status == Status::Running
            && update
                .activity_age
                .is_some_and(|age| age >= crate::hardware::STALL_TIMEOUT)
        {
            "Display capture stopped responding.".into()
        } else {
            match update.frame {
                Some(Ok(image)) => {
                    self.processor.image = Some(image);
                    self.snapshot.status = Status::Running;
                    self.snapshot.message = "Main display".into();
                    return false;
                }
                Some(Err(error)) => error,
                None => return false,
            }
        };
        self.capture_retry = Some(CaptureRetry::after(self.capture_retry.as_ref(), now));
        self.snapshot.status = Status::Starting;
        self.snapshot.message = if reason.is_empty() {
            "Reconnecting display capture…".into()
        } else {
            format!("{reason} Reconnecting…")
        };
        self.processor.image = None;
        true
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
                // Decoding touches every captured pixel, so it runs without the
                // runtime lock: Stop, start, snapshots, saves and shutdown never
                // wait behind it.
                #[cfg(target_os = "macos")]
                let update = {
                    let wanted = {
                        let rt = inner.lock().unwrap();
                        if rt.shutdown {
                            break;
                        }
                        rt.wants_capture(now)
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
                                    // Dropping stops the native stream. After a
                                    // transient failure, wants_capture starts a
                                    // new one once its retry delay has passed.
                                    if rt.apply_display_poll(update, now) {
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
                // Stamped as it is sent, after any decode: the hardware watchdog
                // bounds the gap between publishes, which is one tick long.
                hardware.publish(&snapshot.colors, snapshot.status == Status::Running);
                if snapshot.sequence != last_emitted {
                    last_emitted = snapshot.sequence;
                    let _ = app.emit("sync-output", snapshot);
                }
                // Ticks start OUTPUT_INTERVAL apart. A slow tick is followed at
                // once by the next one, never by a burst of catch-up ticks.
                thread::sleep(OUTPUT_INTERVAL.saturating_sub(now.elapsed()));
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
        #[cfg(any(target_os = "macos", test))]
        {
            rt.capture_retry = None;
        }
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
        assert!(!rt.update_display(display_update(0, 0, true), now));
        rt.render(0.033);
        let held = rt.snapshot.colors.clone();
        assert!(!held.is_empty());
        for age_ms in [33, 100, 249] {
            assert!(!rt.update_display(display_update(0, age_ms, false), now));
            rt.render(0.033);
            assert_eq!(rt.snapshot.status, Status::Running);
            assert_eq!(rt.snapshot.colors, held);
        }
        // The stalled capture is released for a restart while Sync stays requested.
        assert!(rt.update_display(display_update(0, 250, false), now));
        rt.render(0.033);
        assert_eq!(rt.snapshot.status, Status::Starting);
        assert_eq!(
            rt.snapshot.message,
            "Display capture stopped responding. Reconnecting…"
        );
        assert!(rt.processor.image.is_none());
        assert_eq!(rt.snapshot.colors, held); // Virtual colors hold; physical output stops.
        drop(rt);
        // Stop still ends a recovering source, after which another can start.
        assert!(service.start(Source::Test).is_err());
        assert_eq!(service.stop().status, Status::Stopping);
        assert!(service
            .inner
            .lock()
            .unwrap()
            .update_stopping(None, now, now));
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
            rt.update_display(display_update(0, age, fresh), now);
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
            rt.update_display(display_update(0, age, false), now);
            rt.render(OUTPUT_INTERVAL.as_secs_f32());
            hardware.publish_at(
                &rt.snapshot.colors,
                rt.snapshot.status == Status::Running,
                now,
            );
            now += OUTPUT_INTERVAL;
        }
        assert_eq!(rt.snapshot.status, Status::Starting); // Recovering, output off.
        let (running, epoch) = hardware.expire_at(now);
        assert!(!running);
        assert_eq!(epoch, running_epoch.unwrap() + 1);
    }

    #[test]
    fn fresh_idle_samples_and_static_test_source_repeat_without_pixel_changes() {
        let service = configured(Source::Display);
        let now = Instant::now();
        let mut rt = service.inner.lock().unwrap();
        rt.update_display(display_update(0, 0, true), now);
        rt.render(0.033);
        let held = rt.snapshot.colors.clone();
        for _ in 0..3 {
            assert!(!rt.update_display(display_update(1, 0, false), now));
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
    fn blank_suspended_stopped_and_failed_samples_stop_output_and_restart_capture() {
        let now = Instant::now();
        for (frame_status, reason) in [
            (2, "The display is blank."),
            (3, "Display capture is suspended."),
            (5, "Display capture stopped."),
        ] {
            let service = configured(Source::Display);
            let mut rt = service.inner.lock().unwrap();
            rt.update_display(display_update(0, 0, true), now);
            rt.render(0.033);
            let held = rt.snapshot.colors.clone();
            rt.update_display(display_update(1, 60_000, false), now);
            assert_eq!(rt.snapshot.status, Status::Running);
            // Even a pending old Complete buffer cannot override these statuses.
            assert!(rt.update_display(display_update(frame_status, 0, true), now));
            rt.render(0.033);
            assert_eq!(rt.snapshot.status, Status::Starting);
            assert_eq!(rt.snapshot.message, format!("{reason} Reconnecting…"));
            assert!(rt.processor.image.is_none());
            assert_eq!(rt.snapshot.colors, held);
        }
        for (state, native, decode, message) in [
            (4, "Capture failed.", None, "Capture failed. Reconnecting…"),
            (4, "", None, "Reconnecting display capture…"),
            (
                1,
                "",
                Some("Invalid frame."),
                "Invalid frame. Reconnecting…",
            ),
        ] {
            let service = configured(Source::Display);
            let mut update = display_update(0, 0, true);
            update.state = state;
            update.message = native.into();
            if let Some(error) = decode {
                update.frame = Some(Err(error.into()));
            }
            let mut rt = service.inner.lock().unwrap();
            assert!(rt.update_display(update, now));
            assert_eq!(rt.snapshot.status, Status::Starting);
            assert_eq!(rt.snapshot.message, message);
            assert!(rt.processor.image.is_none());
        }
    }

    #[test]
    fn transient_capture_failures_retry_with_bounded_backoff_then_resume() {
        let service = configured(Source::Display);
        let start = Instant::now();
        let mut rt = service.inner.lock().unwrap();
        assert!(rt.wants_capture(start)); // A new source captures at once.
        rt.update_display(display_update(0, 0, true), start);
        rt.render(0.033);
        assert_eq!(rt.snapshot.status, Status::Running);
        let mut now = start;
        for delay_ms in [500, 1_000, 2_000, 4_000, 5_000, 5_000] {
            // Each failure releases the capture; the next waits out its delay.
            assert!(rt.update_display(display_update(2, 0, false), now));
            let delay = Duration::from_millis(delay_ms);
            assert!(!rt.wants_capture(now + delay - Duration::from_millis(1)));
            assert!(rt.wants_capture(now + delay));
            now += delay;
            // A restarted capture with no frame yet keeps output off.
            assert!(!rt.update_display(display_update(-1, 0, false), now));
            rt.render(0.033);
            assert_eq!(rt.snapshot.status, Status::Starting);
        }
        // The first Complete frame from the new capture resumes output.
        assert!(!rt.update_display(display_update(0, 0, true), now));
        rt.render(0.033);
        assert_eq!(rt.snapshot.status, Status::Running);
        assert_eq!(rt.snapshot.message, "Main display");
        assert!(rt.processor.image.is_some());
        // A failure long after the previous one starts the backoff again.
        now += CaptureRetry::RESET;
        assert!(rt.update_display(display_update(0, 250, false), now));
        assert!(!rt.wants_capture(now + CaptureRetry::FIRST - Duration::from_millis(1)));
        assert!(rt.wants_capture(now + CaptureRetry::FIRST));
        drop(rt);
        // Stop during a retry delay ends Sync; a new start captures at once.
        assert_eq!(service.stop().status, Status::Stopping);
        assert!(!service.inner.lock().unwrap().wants_capture(now));
        assert!(service
            .inner
            .lock()
            .unwrap()
            .update_stopping(None, now, now));
        service.start(Source::Display).unwrap();
        assert!(service.inner.lock().unwrap().wants_capture(now));
    }

    #[test]
    fn permanent_capture_failures_end_in_error_without_a_restart() {
        let service = configured(Source::Display);
        let now = Instant::now();
        let mut rt = service.inner.lock().unwrap();
        rt.update_display(display_update(0, 0, true), now);
        rt.render(0.033);
        let held = rt.snapshot.colors.clone();
        // A transient failure first, then one only the user can fix.
        assert!(rt.update_display(display_update(0, 250, false), now));
        let mut update = display_update(-1, 0, false);
        update.state = 5;
        update.message =
            "Screen recording was stopped in macOS. Start Sync again to resume.".into();
        assert!(rt.update_display(update, now));
        rt.render(0.033);
        assert_eq!(rt.snapshot.status, Status::Error);
        assert_eq!(
            rt.snapshot.message,
            "Screen recording was stopped in macOS. Start Sync again to resume."
        );
        assert!(rt.processor.image.is_none());
        assert_eq!(rt.snapshot.colors, held);
        assert!(!rt.wants_capture(now + Duration::from_secs(60)));
        drop(rt);
        // Only a new start captures again, at once.
        service.start(Source::Display).unwrap();
        assert!(service.inner.lock().unwrap().wants_capture(now));
    }

    #[test]
    fn slow_ticks_below_the_stall_bound_keep_physical_output_streaming() {
        // Models the output loop: a tick starts OUTPUT_INTERVAL after the previous
        // start (at once after a slow tick), works, then publishes stamped when
        // sent. Only a single tick of STALL_TIMEOUT or more can stop output.
        let colors = vec![LightColor {
            id: "light".into(),
            rgb: [1, 2, 3],
        }];
        let start = Instant::now();
        for work_ms in [5, 120, 200, 249] {
            let hardware = crate::hardware::HardwareService::detached();
            let work = Duration::from_millis(work_ms);
            let mut tick = start;
            hardware.publish_at(&colors, true, tick + work);
            let (_, epoch) = hardware.expire_at(tick + work);
            for _ in 0..20 {
                let next_tick = tick + work.max(OUTPUT_INTERVAL);
                let next_publish = next_tick + work;
                // The output thread may check at any moment before the next publish.
                let mut at = tick + work;
                while at < next_publish {
                    assert_eq!(hardware.expire_at(at), (true, epoch), "{work_ms} ms ticks");
                    at += Duration::from_millis(5);
                }
                hardware.publish_at(&colors, true, next_publish);
                tick = next_tick;
            }
        }
        let hardware = crate::hardware::HardwareService::detached();
        hardware.publish_at(&colors, true, start);
        let (_, epoch) = hardware.expire_at(start);
        assert_eq!(
            hardware.expire_at(start + crate::hardware::STALL_TIMEOUT),
            (false, epoch + 1)
        );
    }

    #[test]
    fn startup_waits_for_first_image_without_treating_start_ack_as_a_frame() {
        let service = configured(Source::Display);
        let now = Instant::now();
        let mut rt = service.inner.lock().unwrap();
        for (status, age) in [(-1, 0), (-1, 250), (4, 1_000), (1, 60_000)] {
            assert!(!rt.update_display(display_update(status, age, false), now));
            rt.render(0.033);
            assert_eq!(rt.snapshot.status, Status::Starting);
            assert!(rt.snapshot.colors.is_empty());
        }
        // Start completion has no API deadline for the first Complete frame;
        // no physical output is requested before that frame arrives.
        rt.update_display(display_update(0, 0, true), now);
        rt.render(0.033);
        assert_eq!(rt.snapshot.status, Status::Running);
        assert!(!rt.snapshot.colors.is_empty());
    }

    #[test]
    fn explicit_idle_persists_without_a_callback_heartbeat_but_can_be_invalidated() {
        let service = configured(Source::Display);
        let now = Instant::now();
        let mut rt = service.inner.lock().unwrap();
        rt.update_display(display_update(0, 0, true), now);
        rt.render(0.033);
        let held = rt.snapshot.colors.clone();
        for elapsed in [33, 250, 1_000, 60_000] {
            assert!(!rt.update_display(display_update(1, elapsed, false), now));
            rt.render(0.033);
            assert_eq!(rt.snapshot.status, Status::Running);
            assert_eq!(rt.snapshot.colors, held);
        }
        // Once new content is declared, missing source progress is subject to
        // the 250 ms watchdog again. Idle is not a permanently latched exemption.
        rt.update_display(display_update(0, 0, true), now);
        assert!(rt.update_display(display_update(0, 250, false), now));
        assert_eq!(rt.snapshot.status, Status::Starting);
    }

    #[test]
    fn frames_decoded_outside_the_lock_cannot_override_a_later_stop() {
        let service = configured(Source::Display);
        assert!(service.inner.lock().unwrap().wants_display_frame());
        // The output thread decodes here without the runtime lock, so Stop wins it.
        let decoded = display_update(0, 0, true);
        assert_eq!(service.stop().status, Status::Stopping);
        let now = Instant::now();
        let mut rt = service.inner.lock().unwrap();
        assert!(!rt.wants_display_frame());
        assert!(!rt.apply_display_poll(Some(decoded), now));
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
        assert!(!rt.apply_display_poll(None, now));
        assert_eq!(rt.snapshot.status, Status::Starting);
        assert!(rt.processor.image.is_none());
        assert!(!rt.apply_display_poll(Some(display_update(0, 0, true)), now));
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
            (
                Some((5, "Permission revoked".into())),
                Status::Error,
                "Permission revoked",
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
                rt.update_display(display_update(0, 0, true), Instant::now());
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
