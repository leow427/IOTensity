use crate::config::{Configuration, Intensity};
use crate::sync::processing::{linear_to_srgb, srgb_to_linear};
use std::collections::{HashMap, HashSet};

#[derive(Clone, Debug, PartialEq)]
pub struct Color {
    pub id: String,
    pub rgb: [u8; 3],
}
#[derive(Clone, Debug, Default)]
pub struct OutputSnapshot {
    pub running: bool,
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
    /// The smoothed sRGB color is scaled once in linear light, like Display/Test,
    /// and hardware consumes the final RGB8 output without any further processing.
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
        let brightness = f32::from(config.preferences.brightness) / 100.0;
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
                    linear_to_srgb(srgb_to_linear((c * 255.0).round() as u8) * brightness)
                }),
            });
        }
        let current: HashSet<&str> = colors.iter().map(|c| c.id.as_str()).collect();
        self.colors.retain(|id, _| current.contains(id.as_str()));
        OutputSnapshot {
            running: true,
            colors,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn deterministic_source_applies_brightness_once_in_linear_light() {
        use crate::sync::processing::{AnalysisImage, Processor};
        let mut config: Configuration =
            serde_json::from_str(include_str!("../../../tests/fixtures/configuration.json"))
                .unwrap();
        let lights: Vec<_> = config.rooms.iter().flat_map(|r| r.lights.clone()).collect();
        config.preferences.brightness = 100;
        let full = Simulation::default().tick(0.033, &config, false);
        // Full brightness is the unscaled smoothed sRGB color.
        let mut reference = Simulation::default();
        reference.tick(0.033, &config, false);
        for (light, output) in lights.iter().zip(full.colors.iter()) {
            assert_eq!(
                output.rgb,
                reference.colors[&light.id].map(|c| (c * 255.0).round() as u8)
            );
        }
        for brightness in [50, 25, 0] {
            config.preferences.brightness = brightness;
            let scaled = Simulation::default().tick(0.033, &config, false);
            for (light, (base, output)) in lights.iter().zip(full.colors.iter().zip(&scaled.colors))
            {
                // A uniform media frame of the same base color takes the Display/Test path.
                let processor = Processor {
                    image: Some(
                        AnalysisImage::from_bgra(
                            &[base.rgb[2], base.rgb[1], base.rgb[0], 255].repeat(4),
                            2,
                            2,
                            8,
                        )
                        .unwrap(),
                    ),
                };
                let media = processor.frame(std::slice::from_ref(light), brightness);
                assert_eq!(output.rgb, media[0].rgb);
            }
        }
        // 50% brightness is half the linear light, not half the encoded value.
        config.preferences.brightness = 50;
        let half = Simulation::default().tick(0.033, &config, false);
        for (a, b) in full.colors.iter().zip(half.colors.iter()) {
            for i in 0..3 {
                let expected = linear_to_srgb(srgb_to_linear(a.rgb[i]) * 0.5);
                assert_eq!(b.rgb[i], expected);
                assert!(a.rgb[i] < 64 || b.rgb[i] > a.rgb[i] / 2 + 1);
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
