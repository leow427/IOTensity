use crate::config::{Light, Position, X_BOUNDS, Y_BOUNDS};
use serde::Serialize;

pub const OUTPUT_INTERVAL: std::time::Duration = std::time::Duration::from_nanos(33_333_333);
const ANALYSIS_MAX_SIDE: usize = 256;
const SAMPLE_RADIUS: f64 = 0.1;
const CHROMA_BOOST: f32 = 3.0;

#[derive(Clone, Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LightColor {
    pub id: String,
    pub rgb: [u8; 3],
}

pub fn srgb_to_linear(value: u8) -> f32 {
    let c = value as f32 / 255.0;
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

pub fn linear_to_srgb(value: f32) -> u8 {
    let c = value.clamp(0.0, 1.0);
    let encoded = if c <= 0.0031308 {
        12.92 * c
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    };
    (encoded * 255.0).round() as u8
}

pub fn screen_uv(position: &Position) -> (f32, f32) {
    let u = (position.x - X_BOUNDS.0) / (X_BOUNDS.1 - X_BOUNDS.0);
    let v = 1.0 - (position.y - Y_BOUNDS.0) / (Y_BOUNDS.1 - Y_BOUNDS.0);
    (u.clamp(0.0, 1.0) as f32, v.clamp(0.0, 1.0) as f32)
}

// Keep both weighted moments through reduction: averaging colors first would
// erase small colorful details before the picker could give them any weight.
struct AnalysisPixel {
    weighted_rgb: [f32; 3],
    weight: f32,
}

pub struct AnalysisImage {
    pub width: usize,
    pub height: usize,
    pixels: Vec<AnalysisPixel>,
}

fn analysis_dimensions(width: usize, height: usize) -> (usize, usize) {
    let scale = (ANALYSIS_MAX_SIDE as f64 / width.max(height) as f64).min(1.0);
    (
        (width as f64 * scale).round().max(1.0) as usize,
        (height as f64 * scale).round().max(1.0) as usize,
    )
}

// Integrate a tent over each analysis pixel, including partial edge pixels.
// This avoids hard region-boundary jumps and works even on a one-pixel axis.
fn axis_weights(center: f32, size: usize) -> Vec<(usize, f32)> {
    let center = center as f64 * size as f64;
    let radius = SAMPLE_RADIUS * size as f64;
    let first = (center - radius).max(0.0).floor() as usize;
    let end = (center + radius).ceil().min(size as f64) as usize;
    let integral = |edge: f64| {
        let t = ((edge - center) / radius).clamp(-1.0, 1.0);
        t - 0.5 * t * t.abs()
    };
    (first..end)
        .map(|i| (i, (integral((i + 1) as f64) - integral(i as f64)) as f32))
        .collect()
}

impl AnalysisImage {
    // Area-weighted box reduction, including fractional edge coverage. Each source
    // component is decoded BEFORE contributing to any downscale or region average.
    pub fn from_bgra(
        bytes: &[u8],
        width: usize,
        height: usize,
        stride: usize,
    ) -> Result<Self, String> {
        if width == 0
            || height == 0
            || width > 32768
            || height > 32768
            || stride < width * 4
            || bytes.len() < stride.saturating_mul(height)
        {
            return Err("Invalid native image dimensions or buffer length.".into());
        }
        // Derive both axes from every incoming frame, never from a 16:9 preset.
        let (out_w, out_h) = analysis_dimensions(width, height);
        let lut: [f32; 256] = std::array::from_fn(|i| srgb_to_linear(i as u8));
        let mut pixels = Vec::with_capacity(out_w * out_h);
        for y in 0..out_h {
            let top = y as f64 * height as f64 / out_h as f64;
            let bottom = (y + 1) as f64 * height as f64 / out_h as f64;
            for x in 0..out_w {
                let left = x as f64 * width as f64 / out_w as f64;
                let right = (x + 1) as f64 * width as f64 / out_w as f64;
                let mut sum = [0.0; 3];
                let mut total_weight = 0.0;
                for sy in top.floor() as usize..(bottom.ceil() as usize).min(height) {
                    let wy = bottom.min((sy + 1) as f64) - top.max(sy as f64);
                    for sx in left.floor() as usize..(right.ceil() as usize).min(width) {
                        let coverage =
                            (wy * (right.min((sx + 1) as f64) - left.max(sx as f64))) as f32;
                        let offset = sy * stride + sx * 4;
                        let rgb: [f32; 3] =
                            std::array::from_fn(|c| lut[bytes[offset + 2 - c] as usize]);
                        let max = rgb[0].max(rgb[1]).max(rgb[2]);
                        let min = rgb[0].min(rgb[1]).min(rgb[2]);
                        // Bounded 1..4 preference for bright chromatic pixels.
                        // Unlike saturation alone, this does not amplify dark
                        // color noise; neutral pixels still contribute normally.
                        let weight = coverage * (1.0 + CHROMA_BOOST * (max - min));
                        for c in 0..3 {
                            sum[c] += rgb[c] * weight;
                        }
                        total_weight += weight;
                    }
                }
                let area = ((right - left) * (bottom - top)) as f32;
                pixels.push(AnalysisPixel {
                    weighted_rgb: sum.map(|v| v / area),
                    weight: total_weight / area,
                });
            }
        }
        Ok(Self {
            width: out_w,
            height: out_h,
            pixels,
        })
    }

    pub fn weighted_average(&self, position: &Position) -> [f32; 3] {
        let (u, v) = screen_uv(position);
        let columns = axis_weights(u, self.width);
        let rows = axis_weights(v, self.height);
        let mut sum = [0.0; 3];
        let mut total_weight = 0.0;
        for (y, wy) in rows {
            for &(x, wx) in &columns {
                let spatial_weight = wx * wy;
                let pixel = &self.pixels[y * self.width + x];
                for (c, value) in sum.iter_mut().enumerate() {
                    *value += pixel.weighted_rgb[c] * spatial_weight;
                }
                total_weight += pixel.weight * spatial_weight;
            }
        }
        sum.map(|c| (c / total_weight).clamp(0.0, 1.0))
    }
}

// A real, deterministic BGRA buffer: top-left red, top-right green,
// bottom-left blue, bottom-right white. It uses exactly the capture pipeline.
pub fn generated_image() -> AnalysisImage {
    let (width, height) = (640, 360);
    let mut bytes = vec![0; width * height * 4];
    for y in 0..height {
        for x in 0..width {
            let bgra = match (x < width / 2, y < height / 2) {
                (true, true) => [0, 0, 255, 255],
                (false, true) => [0, 255, 0, 255],
                (true, false) => [255, 0, 0, 255],
                (false, false) => [255, 255, 255, 255],
            };
            bytes[(y * width + x) * 4..(y * width + x + 1) * 4].copy_from_slice(&bgra);
        }
    }
    AnalysisImage::from_bgra(&bytes, width, height, width * 4).expect("valid generated image")
}

#[derive(Default)]
pub struct Processor {
    pub image: Option<AnalysisImage>,
}

impl Processor {
    /// Media output samples the latest image directly. Synthetic response presets
    /// do not delay screen changes; brightness is applied once in linear light.
    pub fn frame(&self, lights: &[Light], brightness: u8) -> Vec<LightColor> {
        lights
            .iter()
            .map(|light| {
                let rgb = self
                    .image
                    .as_ref()
                    .map(|image| image.weighted_average(&light.position))
                    .unwrap_or([0.0; 3]);
                LightColor {
                    id: light.id.clone(),
                    rgb: rgb.map(|c| linear_to_srgb(c * brightness as f32 / 100.0)),
                }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::IconKind;

    const CENTER: Position = Position {
        x: 0.0,
        y: 1.575,
        z: 0.0,
    };

    fn image(width: usize, height: usize, rgb: impl Fn(usize, usize) -> [u8; 3]) -> AnalysisImage {
        // Bright padding must never enter a sample.
        let stride = width * 4 + 16;
        let mut bytes = vec![255; stride * height];
        for y in 0..height {
            for x in 0..width {
                let [r, g, b] = rgb(x, y);
                let offset = y * stride + x * 4;
                bytes[offset..offset + 4].copy_from_slice(&[b, g, r, 255]);
            }
        }
        AnalysisImage::from_bgra(&bytes, width, height, stride).unwrap()
    }

    fn assert_rgb_close(actual: [f32; 3], expected: [f32; 3]) {
        for c in 0..3 {
            assert!(
                (actual[c] - expected[c]).abs() < 0.00001,
                "{actual:?} != {expected:?}"
            );
        }
    }

    fn light(id: &str, x: f64, y: f64) -> Light {
        Light {
            id: id.into(),
            name: id.into(),
            position: Position { x, y, z: 0.0 },
            icon_kind: IconKind::Bulb,
            output: crate::config::LightOutput::Virtual,
        }
    }
    #[test]
    fn quadrants_follow_saved_xy_and_ignore_depth() {
        let image = generated_image();
        assert_eq!((image.width, image.height), (256, 144));
        for (x, y, expected) in [
            (-3.0, 3.0, [1.0, 0.0, 0.0]),
            (3.0, 3.0, [0.0, 1.0, 0.0]),
            (-3.0, 0.15, [0.0, 0.0, 1.0]),
            (3.0, 0.15, [1.0; 3]),
        ] {
            for z in [-0.7, 4.0] {
                assert_rgb_close(image.weighted_average(&Position { x, y, z }), expected);
            }
        }
        assert_eq!(
            screen_uv(&Position {
                x: 0.0,
                y: 1.575,
                z: 0.0
            }),
            (0.5, 0.5)
        );
    }
    #[test]
    fn neutral_averaging_stays_linear_and_respects_stride() {
        let image = image(512, 1, |x, _| [if x % 2 == 0 { 0 } else { 255 }; 3]);
        assert_eq!((image.width, image.height), (256, 1));
        let mixed = image.weighted_average(&CENTER);
        assert_rgb_close(mixed, [0.5; 3]);
        assert_eq!(mixed.map(linear_to_srgb), [188; 3]);
    }

    #[test]
    fn sizing_follows_the_source_aspect_without_upscaling() {
        for (width, height, expected) in [
            (1920, 1080, (256, 144)),
            (2560, 1600, (256, 160)),
            (1600, 1200, (256, 192)),
            (3440, 1440, (256, 107)),
            (5120, 1440, (256, 72)),
            (1080, 1920, (144, 256)),
            (1440, 3440, (107, 256)),
            (2048, 2048, (256, 256)),
            (1512, 982, (256, 166)),
            (100, 200, (100, 200)),
            (1, 1, (1, 1)),
            (32768, 1, (256, 1)),
            (1, 32768, (1, 256)),
        ] {
            assert_eq!(analysis_dimensions(width, height), expected);
        }
    }

    #[test]
    fn actual_frames_keep_corner_mapping_across_aspects_and_size_changes() {
        let mut processor = Processor::default();
        let lights = [
            light("red", -3.0, 3.0),
            light("green", 3.0, 3.0),
            light("blue", -3.0, 0.15),
            light("white", 3.0, 0.15),
        ];
        for (width, height, expected) in [
            (640, 360, (256, 144)),
            (640, 400, (256, 160)),
            (640, 480, (256, 192)),
            (688, 288, (256, 107)),
            (1024, 288, (256, 72)),
            (360, 640, (144, 256)),
            (288, 688, (107, 256)),
            (512, 512, (256, 256)),
            (100, 200, (100, 200)),
        ] {
            let frame = image(width, height, |x, y| {
                match (x < width / 2, y < height / 2) {
                    (true, true) => [255, 0, 0],
                    (false, true) => [0, 255, 0],
                    (true, false) => [0, 0, 255],
                    (false, false) => [255; 3],
                }
            });
            assert_eq!((frame.width, frame.height), expected);
            processor.image = Some(frame);
            let colors = processor.frame(&lights, 100);
            assert_eq!(colors[0].rgb, [255, 0, 0]);
            assert_eq!(colors[1].rgb, [0, 255, 0]);
            assert_eq!(colors[2].rgb, [0, 0, 255]);
            assert_eq!(colors[3].rgb, [255; 3]);
        }
    }

    #[test]
    fn vivid_color_survives_neutral_background_and_reduction() {
        // The same repeating red/white signal at multiple capture resolutions.
        // Weight before reduction: equal areas become 4 red : 1 white, rather
        // than washing out to the unweighted [255, 188, 188].
        for scale in [1, 2, 3] {
            let image = image(512 * scale, 2 * scale, |x, _| {
                if (x / scale) % 2 == 0 {
                    [255, 0, 0]
                } else {
                    [255; 3]
                }
            });
            let color = image.weighted_average(&CENTER);
            assert_rgb_close(color, [1.0, 0.2, 0.2]);
            assert_eq!(color.map(linear_to_srgb), [255, 124, 124]);
        }
    }

    #[test]
    fn nearer_colors_have_more_influence_and_boundaries_are_continuous() {
        let patch = |start| {
            image(200, 100, |x, _| {
                if (start..start + 4).contains(&x) {
                    [255, 0, 0]
                } else {
                    [128; 3]
                }
            })
        };
        let center = patch(98).weighted_average(&CENTER);
        let edge = patch(80).weighted_average(&CENTER);
        assert!(center[0] - center[1] > 3.0 * (edge[0] - edge[1]));

        let stripes = image(
            200,
            100,
            |x, _| {
                if x % 2 == 0 {
                    [255, 0, 0]
                } else {
                    [0, 0, 255]
                }
            },
        );
        let before = stripes.weighted_average(&Position {
            x: -0.00001,
            ..CENTER
        });
        let after = stripes.weighted_average(&Position {
            x: 0.00001,
            ..CENTER
        });
        assert_rgb_close(before, after);
        assert_rgb_close(stripes.weighted_average(&CENTER), [0.5, 0.0, 0.5]);
    }

    #[test]
    fn uniform_black_gray_white_and_dim_colors_remain_faithful_at_edges() {
        for rgb in [[0; 3], [37; 3], [255; 3], [3, 0, 0], [55, 130, 210]] {
            for (width, height) in [(1, 1), (1, 300), (300, 1), (257, 129)] {
                let image = image(width, height, |_, _| rgb);
                for (x, y) in [
                    (-3.0, 3.0),
                    (3.0, 3.0),
                    (-3.0, 0.15),
                    (3.0, 0.15),
                    (0.0, 1.575),
                ] {
                    let color = image.weighted_average(&Position { x, y, z: 0.0 });
                    assert!(color.iter().all(|c| c.is_finite()));
                    assert_eq!(color.map(linear_to_srgb), rgb);
                }
            }
        }
        let isolated = image(100, 100, |x, y| {
            if x == 50 && y == 50 {
                [255, 0, 0]
            } else {
                [0; 3]
            }
        });
        // A bright speck gets a bounded boost, never full-screen brightness.
        let color = isolated.weighted_average(&CENTER);
        assert!(color[0] > 0.0 && color[0] < 0.05);
        assert_eq!([color[1], color[2]], [0.0; 2]);
    }

    #[test]
    fn fractional_reduction_keeps_weighted_moments_and_rejects_invalid_buffers() {
        let image = image(257, 1, |x, _| if x == 128 { [255, 0, 0] } else { [0; 3] });
        // One source pixel straddles the middle two analysis pixels equally.
        for x in [127, 128] {
            assert!((image.pixels[x].weighted_rgb[0] - 2.0 * 256.0 / 257.0).abs() < 0.00001);
            assert!((image.pixels[x].weight - (1.0 + 1.5 * 256.0 / 257.0)).abs() < 0.00001);
        }
        assert!(AnalysisImage::from_bgra(&[], 10, 10, 40).is_err());
        for (width, height, stride) in [
            (0, 1, 4),
            (1, 0, 4),
            (1, 1, 3),
            (32769, 1, 131076),
            (1, 2, usize::MAX),
        ] {
            assert!(AnalysisImage::from_bgra(&[0; 8], width, height, stride).is_err());
        }
    }
    #[test]
    fn media_changes_are_immediate_and_static_frames_are_repeatable() {
        let mut processor = Processor {
            image: Some(generated_image()),
        };
        let lights = [light("one", -3.0, 3.0)];
        assert_eq!(processor.frame(&lights, 100)[0].rgb, [255, 0, 0]);
        assert_eq!(processor.frame(&lights, 100), processor.frame(&lights, 100));
        processor.image = Some(image(100, 100, |_, _| [0, 255, 0]));
        assert_eq!(processor.frame(&lights, 100)[0].rgb, [0, 255, 0]);
    }
    #[test]
    fn brightness_is_once_and_position_updates_retarget_static_image() {
        let processor = Processor {
            image: Some(generated_image()),
        };
        let mut lights = [light("one", 3.0, 0.15)];
        assert_eq!(processor.frame(&lights, 50)[0].rgb, [188; 3]);
        assert_eq!(processor.frame(&lights, 0)[0].rgb, [0; 3]);
        assert_eq!(processor.frame(&lights, 100)[0].rgb, [255; 3]);
        lights[0].position = Position {
            x: -3.0,
            y: 3.0,
            z: 4.0,
        };
        assert_eq!(processor.frame(&lights, 100)[0].rgb, [255, 0, 0]);
        assert!(processor.frame(&[], 100).is_empty());
    }
}
