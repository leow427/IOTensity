use crate::config::{Configuration, Intensity};
use serde::Serialize;
use std::collections::HashMap;

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct Color {
    pub id: String,
    pub rgb: [u8; 3],
}
#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OutputSnapshot {
    pub sequence: u64,
    pub running: bool,
    pub elapsed: f64,
    pub colors: Vec<Color>,
}
#[derive(Default)]
pub struct Simulation {
    elapsed: f64,
    colors: HashMap<String, [f64; 3]>,
}
pub fn target(seconds: f64, id: &str) -> [f64; 3] {
    let hash = id.encode_utf16().fold(0_u32, |hash, c| {
        hash.wrapping_mul(31).wrapping_add(c as u32)
    });
    let phase = (hash % 1024) as f64 / 1024.0 * std::f64::consts::TAU;
    [0.0, 2.1, 4.2].map(|offset| 0.5 + 0.48 * (seconds * 0.65 + phase + offset).sin())
}
impl Simulation {
    /// Injected elapsed time; smoothing belongs only to this synthetic source.
    /// Hardware consumes the final RGB8 output without any further processing.
    pub fn tick(
        &mut self,
        dt: f64,
        config: &Configuration,
        reduced_motion: bool,
    ) -> OutputSnapshot {
        let dt = dt.clamp(0.0, 0.1);
        self.elapsed += dt;
        let response: f64 = match config.preferences.intensity {
            Intensity::Subtle => 1.8,
            Intensity::Balanced => 0.8,
            Intensity::Vivid => 0.3,
            Intensity::Punch => 0.08,
        };
        let alpha = 1.0 - (-dt / response.max(if reduced_motion { 2.5 } else { 0.0 })).exp();
        let mut colors = Vec::new();
        for light in config.rooms.iter().flat_map(|r| &r.lights) {
            let color = self
                .colors
                .entry(light.id.clone())
                .or_insert([0.7, 0.73, 0.7]);
            let target = target(
                self.elapsed * if reduced_motion { 0.15 } else { 1.0 },
                &light.id,
            );
            for i in 0..3 {
                color[i] += (target[i] - color[i]) * alpha;
            }
            colors.push(Color {
                id: light.id.clone(),
                rgb: color.map(|c| {
                    (c * f64::from(config.preferences.brightness) / 100.0 * 255.0).round() as u8
                }),
            });
        }
        self.colors
            .retain(|id, _| colors.iter().any(|c| c.id == *id));
        OutputSnapshot {
            sequence: 0,
            running: true,
            elapsed: self.elapsed,
            colors,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn deterministic_source_applies_brightness_once() {
        let mut config: Configuration =
            serde_json::from_str(include_str!("../../../tests/fixtures/configuration.json"))
                .unwrap();
        config.preferences.brightness = 100;
        let full = Simulation::default().tick(0.033, &config, false);
        config.preferences.brightness = 50;
        let half = Simulation::default().tick(0.033, &config, false);
        for (a, b) in full.colors.iter().zip(half.colors.iter()) {
            for i in 0..3 {
                assert!((f64::from(a.rgb[i]) * 0.5 - f64::from(b.rgb[i])).abs() <= 1.0);
            }
        }
        config.preferences.brightness = 0;
        assert!(Simulation::default()
            .tick(0.1, &config, true)
            .colors
            .iter()
            .all(|c| c.rgb == [0; 3]));
        assert_eq!(target(2.0, "a"), target(2.0, "a"));
    }
}
