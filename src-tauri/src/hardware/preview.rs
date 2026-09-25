use crate::config::{valid_device_id, Position, X_BOUNDS, Y_BOUNDS};
use serde::Deserialize;
use std::time::{Duration, Instant};

// Renewed by selection/position changes, never by a frontend transmission timer.
pub const PREVIEW_DURATION: Duration = Duration::from_secs(2);

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EditMode {
    Location,
    Height,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PreviewRequest {
    pub device_id: String,
    pub position: Position,
    pub mode: EditMode,
}

impl PreviewRequest {
    pub fn color(&self) -> Result<[u8; 3], String> {
        let p = &self.position;
        if !valid_device_id(&self.device_id)
            || !p.x.is_finite()
            || !p.y.is_finite()
            || !p.z.is_finite()
            || !(X_BOUNDS.0..=X_BOUNDS.1).contains(&p.x)
            || !(Y_BOUNDS.0..=Y_BOUNDS.1).contains(&p.y)
            || !(-0.7..=4.0).contains(&p.z)
        {
            return Err("Invalid light preview.".into());
        }
        // Match the editor's sRGB calibration, including at zero sync brightness.
        let (value, bounds, from, to) = match self.mode {
            EditMode::Location => (p.x, X_BOUNDS, [0.25, 0.91, 0.53], [1.0, 0.35, 0.12]),
            EditMode::Height => (p.y, Y_BOUNDS, [0.24, 0.78, 1.0], [0.78, 0.43, 1.0]),
        };
        let amount = (value - bounds.0) / (bounds.1 - bounds.0);
        Ok(std::array::from_fn(|i| {
            ((from[i] + (to[i] - from[i]) * amount) * 255.0).round() as u8
        }))
    }
}

pub(super) struct Preview {
    pub device_id: String,
    pub rgb: [u8; 3],
    pub expires: Instant,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_the_editors_shared_rgb8_fixture() {
        #[derive(Deserialize)]
        struct Case {
            mode: EditMode,
            position: Position,
            rgb: [u8; 3],
        }
        let cases: Vec<Case> = serde_json::from_str(include_str!(
            "../../../tests/fixtures/placement-colors.json"
        ))
        .unwrap();
        for case in cases {
            let request = PreviewRequest {
                device_id: "esp32-020000a1b2c3".into(),
                position: case.position,
                mode: case.mode,
            };
            assert_eq!(request.color().unwrap(), case.rgb);
        }
    }

    #[test]
    fn rejects_invalid_identity_coordinates_and_extra_fields() {
        let mut request = PreviewRequest {
            device_id: "esp32-020000a1b2c3".into(),
            position: Position {
                x: 0.0,
                y: 1.2,
                z: 1.0,
            },
            mode: EditMode::Location,
        };
        for x in [f64::NAN, f64::INFINITY, -3.01, 3.01] {
            request.position.x = x;
            assert!(request.color().is_err());
        }
        request.position.x = 0.0;
        request.device_id = "IOT-A1B2C3".into();
        assert!(request.color().is_err());
        assert!(serde_json::from_value::<PreviewRequest>(serde_json::json!({
            "deviceId": "esp32-020000a1b2c3",
            "position": { "x": 0, "y": 1.2, "z": 1 },
            "mode": "location",
            "ip": "192.168.1.20"
        }))
        .is_err());
    }
}
