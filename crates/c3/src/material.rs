//! Client material and playback values, independent of any graphics API.

#[derive(Clone, Debug, PartialEq)]
pub struct Material {
    /// Direct3D D3DBLEND values, retained without mapping to a guessed preset.
    pub blend: [u32; 2],
    pub depth_write: bool,
    pub depth_test: bool,
    pub scale: f32,
    pub mix_texture: Option<String>,
    pub texture_stages: [Option<String>; 3],
    pub additional_textures: Vec<String>,
    pub mix_mode: u32,
    pub mix_data: [f32; 16],
    pub billboard: u8,
    /// 0: client flag, 1: two-sided, 2: clockwise cull, 3: counterclockwise cull.
    pub cull: u8,
}
impl Default for Material {
    fn default() -> Self {
        Self {
            blend: [5, 6],
            depth_write: true,
            depth_test: true,
            scale: 1.,
            mix_texture: None,
            texture_stages: [None, None, None],
            additional_textures: Vec::new(),
            mix_mode: 0,
            mix_data: [0.; 16],
            billboard: 0,
            cull: 0,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Timing {
    pub delay_ms: u32,
    pub frame_ms: u32,
    pub loops: u32,
    pub loop_interval_ms: u32,
    pub offset: [f32; 3],
    /// Per-piece start delay, measured in this piece's animation frames.
    pub frame_offset: i32,
    pub loop_once: bool,
    /// Shared effect cycle duration, resolved after all of its C3 parts load.
    pub duration_ms: u64,
    /// The effect's global frame interval drives STEP independently of IntervalN.
    pub step_frame_ms: u32,
    pub manual_uv_step: bool,
    pub manual_step: u32,
}
impl Default for Timing {
    fn default() -> Self {
        Self {
            delay_ms: 0,
            frame_ms: 33,
            loops: u32::MAX,
            loop_interval_ms: 0,
            offset: [0.; 3],
            frame_offset: 0,
            loop_once: false,
            duration_ms: 0,
            step_frame_ms: 0,
            manual_uv_step: false,
            manual_step: 0,
        }
    }
}
impl Timing {
    pub fn step(&self, seconds: f64) -> u32 {
        if self.manual_uv_step {
            return self.manual_step;
        }
        if !seconds.is_finite() {
            return 0;
        }
        ((seconds * 1000. - f64::from(self.delay_ms)).max(0.) / f64::from(if self.step_frame_ms == 0 { self.frame_ms.max(1) } else { self.step_frame_ms }))
            .min(f64::from(u32::MAX)) as u32
    }
    pub fn part_duration(&self, frames: u32) -> u64 {
        ((i64::from(frames) + i64::from(self.frame_offset)).max(1) as u64).saturating_mul(u64::from(self.frame_ms.max(1)))
    }
    fn cycle_duration(&self, frames: u32) -> u64 {
        if self.duration_ms == 0 { self.part_duration(frames) } else { self.duration_ms }
    }
    /// None represents a delay, a loop interval, or a completed finite effect.
    pub fn frame(&self, seconds: f64, frames: u32) -> Option<u32> {
        if !seconds.is_finite() || seconds < 0. || frames == 0 || self.loops == 0 {
            return None;
        }
        let time = seconds * 1000. - f64::from(self.delay_ms);
        if time < 0. {
            return None;
        }
        let step = u64::from(self.frame_ms.max(1));
        let duration = self.cycle_duration(frames);
        let cycle = duration.saturating_add(u64::from(self.loop_interval_ms));
        let elapsed = time.min(u64::MAX as f64) as u64;
        if elapsed / cycle >= u64::from(self.loops) {
            return None;
        }
        let local = elapsed % cycle;
        if local >= duration {
            return None;
        }
        let frame = i128::from(local / step) - i128::from(self.frame_offset);
        if frame < 0 || (self.loop_once && frame >= i128::from(frames)) {
            return None;
        }
        Some((frame % i128::from(frames)) as u32)
    }
}
