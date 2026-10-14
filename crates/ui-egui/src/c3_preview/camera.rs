use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Camera {
    pub rotation: [f32; 4],
    pub distance: f32,
    pub pan: [f32; 2],
}
impl Default for Camera {
    fn default() -> Self {
        Self { rotation: [0., 0., 0., 1.], distance: 3.4, pan: [0.; 2] }
    }
}
impl Camera {
    pub fn reset(&mut self) {
        *self = Self::default();
    }
    pub fn preset(&mut self, view: &str) -> bool {
        let s = std::f32::consts::FRAC_1_SQRT_2;
        self.rotation = match view {
            "front" => [0., 0., 0., 1.],
            "back" => [0., 1., 0., 0.],
            "left" => [0., -s, 0., s],
            "right" => [0., s, 0., s],
            "top" => [s, 0., 0., s],
            "bottom" => [-s, 0., 0., s],
            _ => return false,
        };
        true
    }
    pub fn orbit(&mut self, from: egui::Pos2, to: egui::Pos2, rect: egui::Rect) {
        if rect.width() < 1. || rect.height() < 1. {
            return;
        }
        let a = sphere(from, rect);
        let b = sphere(to, rect);
        let axis = photocraft_c3::math::cross(a, b);
        let dot = a.into_iter().zip(b).map(|(a, b)| a * b).sum::<f32>().clamp(-1., 1.);
        let q = if dot < -0.9999 {
            let n = photocraft_c3::math::normalize(photocraft_c3::math::cross(a, [0., 1., 0.]));
            [n[0], n[1], n[2], 0.]
        } else {
            normalize([axis[0], axis[1], axis[2], 1. + dot])
        };
        self.rotation = normalize(multiply(q, self.rotation));
    }
    pub fn zoom(&mut self, delta: f32) {
        if delta.is_finite() {
            self.distance = (self.distance * (-delta.clamp(-1000., 1000.) * 0.002).exp()).clamp(1.15, 30.);
        }
    }
    pub fn move_by(&mut self, delta: egui::Vec2, viewport: egui::Vec2) {
        if viewport.y > 1. && viewport.x > 1. {
            let scale = 2. * self.distance / (viewport.y * 2.4142137 * (viewport.x / viewport.y).min(1.));
            self.pan[0] = (self.pan[0] + delta.x * scale).clamp(-30., 30.);
            self.pan[1] = (self.pan[1] - delta.y * scale).clamp(-30., 30.);
        }
    }
}

fn sphere(p: egui::Pos2, r: egui::Rect) -> [f32; 3] {
    let scale = r.width().min(r.height()) * 0.5;
    let x = (p.x - r.center().x) / scale;
    let y = (r.center().y - p.y) / scale;
    let length = x * x + y * y;
    if length <= 1. {
        [x, y, (1. - length).sqrt()]
    } else {
        let l = length.sqrt();
        [x / l, y / l, 0.]
    }
}
fn normalize(q: [f32; 4]) -> [f32; 4] {
    let length = q.iter().map(|v| v * v).sum::<f32>().sqrt();
    if length.is_finite() && length > 1.0e-8 { q.map(|v| v / length) } else { [0., 0., 0., 1.] }
}
fn multiply([x, y, z, w]: [f32; 4], [a, b, c, d]: [f32; 4]) -> [f32; 4] {
    [w * a + x * d + y * c - z * b, w * b - x * c + y * d + z * a, w * c + x * b - y * a + z * d, w * d - x * a - y * b - z * c]
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn free_rotation_stays_normalized_after_crossing_poles() {
        let r = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(300., 300.));
        let mut c = Camera::default();
        for i in 0..1000 {
            let t = i as f32 * 0.13;
            c.orbit(egui::pos2(150. + 120. * t.cos(), 150. + 120. * t.sin()), egui::pos2(150. + 120. * (t + 0.13).cos(), 150. + 120. * (t + 0.13).sin()), r);
        }
        assert!((c.rotation.iter().map(|v| v * v).sum::<f32>() - 1.).abs() < 1.0e-5);
        assert_ne!(c.rotation, Camera::default().rotation);
    }
    #[test]
    fn zoom_and_pan_are_bounded_and_reset_recovers() {
        let mut c = Camera::default();
        c.zoom(f32::INFINITY);
        for _ in 0..10 {
            c.zoom(-100000.);
        }
        assert_eq!(c.distance, 30.);
        for _ in 0..10 {
            c.zoom(100000.);
        }
        assert!(c.distance >= 1.15);
        c.move_by(egui::vec2(100000., 100000.), egui::vec2(300., 200.));
        assert_eq!(c.pan, [30., -30.]);
        c.reset();
        assert_eq!(c, Camera::default());
    }
}
