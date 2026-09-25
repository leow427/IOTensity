#[cfg(target_os = "macos")]
mod capture;
pub mod processing;

use crate::config::Configuration;
use processing::{generated_image, LightColor, Processor, OUTPUT_INTERVAL};
use serde::{Deserialize, Serialize};
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
                            if rt.snapshot.source == Source::Simulation {
                                rt.snapshot.status = Status::Running;
                                rt.snapshot.message = "Animated colors".into();
                            } else if rt.snapshot.source == Source::Test {
                                if rt.snapshot.status == Status::Starting {
                                    rt.processor.image = Some(generated_image());
                                    rt.snapshot.status = Status::Running;
                                    rt.snapshot.message = "Test image".into();
                                }
                            } else {
                                #[cfg(target_os = "macos")]
                                {
                                    let stream =
                                        capture.get_or_insert_with(capture::Capture::start);
                                    let (state, message) = stream.state();
                                    if state == 4 {
                                        rt.snapshot.status = Status::Error;
                                        rt.snapshot.message = message;
                                        capture = None;
                                    } else if let Some(frame) = stream.take_latest() {
                                        match frame {
                                            Ok(image) => {
                                                rt.snapshot.message = "Main display".into();
                                                rt.processor.image = Some(image);
                                                rt.snapshot.status = Status::Running;
                                            }
                                            Err(error) => {
                                                rt.snapshot.status = Status::Error;
                                                rt.snapshot.message = error;
                                                capture = None;
                                            }
                                        }
                                    }
                                }
                                #[cfg(not(target_os = "macos"))]
                                {
                                    rt.snapshot.status = Status::Error;
                                    rt.snapshot.message =
                                        "Display capture is available on macOS only.".into();
                                }
                            }
                            if rt.snapshot.status == Status::Running {
                                if let Some(config) = rt.config.clone() {
                                    rt.snapshot.colors = if rt.snapshot.source == Source::Simulation
                                    {
                                        let reduced_motion = rt.reduced_motion;
                                        rt.simulation
                                            .tick(dt as f64, &config, reduced_motion)
                                            .colors
                                            .into_iter()
                                            .map(|c| LightColor {
                                                id: c.id,
                                                rgb: c.rgb,
                                            })
                                            .collect()
                                    } else {
                                        rt.processor.frame(
                                            &config.rooms[0].lights,
                                            config.preferences.brightness,
                                        )
                                    };
                                }
                            }
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
                hardware.publish(&snapshot.colors, snapshot.status == Status::Running);
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
