//! C3 playback in integer client frames. Matrix keys interpolate component-wise,
//! matching Motion_GetMatrix rather than substituting quaternion interpolation.
use crate::{Binding, Error, Mesh, Model, Result, Vertex, chunks, math, mesh, reader::Reader};

type Matrix = [f32; 16];
const MAX_MATRICES: usize = 1_000_000;
const MAX_ANIMATION_BYTES: usize = 256 * 1024 * 1024;

#[derive(Clone, Debug, Default)]
struct Extension {
    columns: u32,
    rect: Option<[f32; 4]>,
    relation: Option<(u32, u32, [f32; 2])>,
    stretch: f32,
    rotation: Option<(f32, u32)>,
    initial_rotation: Option<[f32; 2]>,
}
impl Extension {
    fn parse(payload: &[u8]) -> Result<Self> {
        let mut r = Reader::new(payload);
        if r.take(6)? != b"ccflag" {
            return Err(Error("invalid CCFL signature".into()));
        }
        let _version = r.u32()?;
        let count = r.u32()?;
        if count > 4096 {
            return Err(Error("too many CCFL records".into()));
        }
        let mut out = Self::default();
        for _ in 0..count {
            let header = r.u32()?;
            let size = (header & 0xffff) as usize;
            if size < 4 {
                return Err(Error("invalid CCFL record length".into()));
            }
            let mut record = Reader::new(r.take(size - 4)?);
            match header >> 16 {
                2 => out.columns = record.u32()?,
                3 => out.relation = Some((record.u32()?, record.u32()?, record.raw_floats()?)),
                4 => out.stretch = record.raw_floats::<1>()?[0],
                5 => out.rotation = Some((record.raw_floats::<1>()?[0], record.u32()?)),
                6 => out.rect = Some(record.raw_floats()?),
                7 => out.initial_rotation = Some(record.raw_floats()?),
                _ => {}
            }
        }
        if out.columns > 65_536
            || out
                .rect
                .iter()
                .flatten()
                .chain(std::iter::once(&out.stretch))
                .chain(out.rotation.iter().map(|v| &v.0))
                .chain(out.initial_rotation.iter().flatten())
                .chain(out.relation.iter().flat_map(|v| v.2.iter()))
                .any(|v| !v.is_finite() || v.abs() > 1.0e6)
        {
            return Err(Error("invalid CCFL animation values".into()));
        }
        Ok(out)
    }
}

#[derive(Clone, Debug)]
pub struct Motion {
    pub frames: u32,
    pub bones: usize,
    keys: Vec<u32>,
    matrices: Vec<Matrix>,
}
impl Motion {
    pub fn parse(payload: &[u8]) -> Result<Self> {
        let mut r = Reader::new(payload);
        let bones = r.u32()? as usize;
        let frames = r.u32()?;
        if bones == 0 || bones > 4096 || frames == 0 || frames > 1_000_000 {
            return Err(Error("invalid animation bone/frame counts".into()));
        }
        let tag = r.array::<4>()?;
        let keyed = matches!(&tag, b"KKEY" | b"XKEY" | b"ZKEY");
        let count = if keyed { r.u32()? as usize } else { frames as usize };
        if count == 0 || bones.checked_mul(count).is_none_or(|n| n > MAX_MATRICES) {
            return Err(Error("motion exceeds the bounded matrix budget".into()));
        }
        let mut keys = Vec::with_capacity(count);
        let mut matrices = Vec::with_capacity(bones * count);
        if keyed {
            let stride = match &tag {
                b"KKEY" => 64,
                b"XKEY" => 48,
                _ => 28,
            };
            r.require(count.saturating_mul(bones.saturating_mul(stride).saturating_add(if tag == *b"KKEY" { 4 } else { 2 })))?;
            for _ in 0..count {
                let frame = if tag == *b"KKEY" { r.u32()? } else { u32::from(r.u16()?) };
                if keys.last().is_some_and(|previous| *previous > frame) {
                    return Err(Error("motion keys are not ordered".into()));
                }
                keys.push(frame);
                for _ in 0..bones {
                    let m = match &tag {
                        b"KKEY" => r.raw_floats::<16>()?,
                        b"XKEY" => {
                            let values = r.raw_floats::<12>()?;
                            let mut matrix = math::IDENTITY;
                            for (dst, src) in matrix.as_chunks_mut::<4>().0.iter_mut().zip(values.as_chunks::<3>().0) {
                                dst[..3].copy_from_slice(src);
                            }
                            matrix
                        }
                        _ => {
                            let q = r.raw_floats::<4>()?;
                            let t = r.raw_floats::<3>()?;
                            math::quaternion_matrix(q, t).unwrap_or([f32::NAN; 16])
                        }
                    };
                    matrices.push(m);
                }
            }
        } else {
            r = Reader::new(payload.get(8..).ok_or_else(|| Error("truncated motion".into()))?);
            r.require(bones.saturating_mul(count).saturating_mul(64))?;
            matrices.resize(bones * count, math::IDENTITY);
            keys.extend(0..frames);
            // Legacy exports are bone-major, unlike the keyed frame-major table.
            for bone in 0..bones {
                for frame in 0..count {
                    let matrix = r.raw_floats::<16>()?;
                    if let Some(slot) = matrices.get_mut(frame * bones + bone) {
                        *slot = matrix;
                    }
                }
            }
        }
        Ok(Self { frames, bones, keys, matrices })
    }
    pub fn matrix(&self, bone: usize, frame: u32) -> Matrix {
        if bone >= self.bones {
            return math::IDENTITY;
        }
        let after = self.keys.partition_point(|key| *key <= frame);
        let left = after.saturating_sub(1).min(self.keys.len().saturating_sub(1));
        let right = after.min(self.keys.len().saturating_sub(1));
        let a = self.matrices.get(left * self.bones + bone).copied().unwrap_or(math::IDENTITY);
        let b = self.matrices.get(right * self.bones + bone).copied().unwrap_or(a);
        let start = self.keys.get(left).copied().unwrap_or(0);
        let end = self.keys.get(right).copied().unwrap_or(start);
        let t = if end > start { frame.saturating_sub(start) as f32 / (end - start) as f32 } else { 0. };
        std::array::from_fn(|i| a[i] + (b[i] - a[i]) * t)
    }
}

#[derive(Clone, Debug)]
struct Key {
    frame: u32,
    value: u32,
}
#[derive(Clone, Debug, Default)]
struct Keys {
    alpha: Vec<Key>,
    draw: Vec<Key>,
    texture: Vec<Key>,
    rows: u32,
    step: [f32; 2],
    secondary_step: [[f32; 2]; 2],
    extra: Extension,
    billboard: u8,
    two_sided: bool,
}
impl Keys {
    fn parse(payload: &[u8], tag: &[u8; 4]) -> Result<Self> {
        let mut r = Reader::new(payload);
        let (_, count, stride, _) = crate::geometry_counts(&mut r, tag)?;
        r.take(count.saturating_mul(stride))?;
        let indices = (r.u32()? as usize).checked_add(r.u32()? as usize).and_then(|n| n.checked_mul(6)).ok_or_else(|| Error("index size overflow".into()))?;
        r.take(indices)?;
        r.name()?;
        r.take(88)?;
        let rows = r.u32()?.max(1);
        let mut tracks = Vec::new();
        for kind in 0..3 {
            let count = r.u32()? as usize;
            if count > 1_000_000 {
                return Err(Error("too many material keys".into()));
            }
            r.require(count.saturating_mul(16))?;
            let mut track = Vec::with_capacity(count);
            for _ in 0..count {
                let frame = r.u32()?;
                let fields = [r.u32()?, r.u32()?, r.u32()?];
                let value = fields.get(kind).copied().unwrap_or(0);
                track.push(Key { frame, value });
            }
            if track.windows(2).any(|pair| pair.first().zip(pair.get(1)).is_some_and(|(a, b)| a.frame > b.frame)) {
                return Err(Error("material keys are not ordered".into()));
            }
            tracks.push(track);
        }
        let mut step = [0.; 2];
        let tail = r.take(r.remaining())?;
        let mut tail = tail;
        if tail.starts_with(b"STEP") && tail.len() >= 12 {
            let mut reader = Reader::new(tail);
            reader.take(4)?;
            step = reader.raw_floats()?;
            tail = tail.get(12..).unwrap_or_default();
        }
        let two_sided = tail.starts_with(b"2SID");
        if two_sided {
            tail = tail.get(4..).unwrap_or_default();
        }
        let billboard = match tail.get(..4) {
            Some(b"BILB") => 1,
            Some(b"BIB2") => 2,
            Some(b"BIB3") => 3,
            Some(b"BIB4") => 4,
            _ => 0,
        };
        if billboard != 0 {
            tail = tail.get(4..).unwrap_or_default();
        }
        let mut secondary_step = [[0.; 2]; 2];
        if tag == b"PHY5" && tail.starts_with(b"STEP1") {
            let mut r = Reader::new(tail);
            r.take(5)?;
            secondary_step[0] = r.raw_floats()?;
            if r.remaining() >= 5 && r.take(5)? == b"STEP2" {
                secondary_step[1] = r.raw_floats()?;
            }
        }
        if step.iter().chain(secondary_step.iter().flatten()).any(|v| !v.is_finite() || v.abs() > 1.0e6) {
            return Err(Error("invalid UV step".into()));
        }
        let mut tracks = tracks.into_iter();
        Ok(Self {
            alpha: tracks.next().unwrap_or_default(),
            draw: tracks.next().unwrap_or_default(),
            texture: tracks.next().unwrap_or_default(),
            rows,
            step,
            secondary_step,
            extra: Extension::default(),
            billboard,
            two_sided,
        })
    }
    fn value(track: &[Key], frame: u32) -> Option<&Key> {
        track.get(track.partition_point(|key| key.frame <= frame).saturating_sub(1))
    }
    fn alpha(&self, frame: u32) -> f32 {
        let Some(a) = Self::value(&self.alpha, frame) else {
            return 1.;
        };
        let alpha = f32::from_bits(a.value);
        let next = self.alpha.get(self.alpha.partition_point(|key| key.frame <= frame));
        let result = if let Some(b) = next.filter(|b| b.frame > a.frame) {
            alpha + (f32::from_bits(b.value) - alpha) * frame.saturating_sub(a.frame) as f32 / (b.frame - a.frame) as f32
        } else {
            alpha
        };
        if result.is_finite() { result.clamp(0., 1.) } else { 0. }
    }
    fn uv(&self, frame: u32, step: u32) -> [f32; 2] {
        if let Some(key) = Self::value(&self.texture, frame)
            .filter(|key| key.frame <= frame && (key.frame == frame || self.texture.last().is_some_and(|last| last.frame > frame)) && (key.value as i32) >= 0)
        {
            let columns = self.extra.columns.max(1);
            let columns = if self.extra.columns == 0 { self.rows } else { columns };
            [(key.value % columns) as f32 / columns as f32, (key.value / columns) as f32 / self.rows as f32]
        } else {
            self.step.map(|value| (value * step as f32).rem_euclid(1.))
        }
    }
}

#[derive(Debug)]
struct Skin {
    rest: Mesh,
    binding: Binding,
    motion: Option<Motion>,
    keys: Keys,
    uv1: Vec<[f32; 2]>,
    colors: Vec<[f32; 4]>,
}
#[derive(Clone, Debug)]
struct ParticleFrame {
    ids: Vec<u16>,
    positions: Vec<[f32; 3]>,
    sizes: Vec<f32>,
    cells: Vec<f32>,
    matrix: Matrix,
}
#[derive(Debug)]
struct ParticleStyle {
    mode: u8,
    world_space: bool,
    spin: [f32; 2],
    alpha: [f32; 3],
    alpha_frames: [u32; 2],
    life_alpha: [f32; 3],
    life_range: [f32; 2],
    stretch: f32,
}
#[derive(Debug)]
struct Particles {
    kind: [u8; 4],
    name: String,
    texture: String,
    rows: u32,
    frames: Vec<ParticleFrame>,
    maximum: usize,
    style: Option<ParticleStyle>,
    births: Vec<Vec<u32>>,
    previous: Vec<Vec<Option<[u32; 2]>>>,
    extra: Extension,
}
#[derive(Debug)]
struct Trail {
    name: String,
    texture: String,
    endpoints: [[f32; 3]; 2],
    matrices: Vec<Matrix>,
    segments: usize,
}
#[derive(Debug)]
enum Track {
    Skin(Box<Skin>),
    Particles(Particles),
    Trail(Trail),
}

#[derive(Clone, Debug)]
pub struct SampledMesh {
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u32>,
    pub alpha: f32,
    pub visible: bool,
    /// Empty means white; particle systems can assign a separate alpha per vertex.
    pub colors: Vec<[f32; 4]>,
    pub uv1: Vec<[f32; 2]>,
    pub uv_steps: [[f32; 2]; 3],
}
impl SampledMesh {
    pub fn hidden() -> Self {
        Self { vertices: Vec::new(), indices: Vec::new(), alpha: 0., visible: false, colors: Vec::new(), uv1: Vec::new(), uv_steps: [[0.; 2]; 3] }
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MeshStyle {
    pub two_sided: bool,
    /// 0: ordinary geometry; 1..4: BILB/BIB2/BIB3/BIB4 client variants.
    pub billboard: u8,
    pub skinned: bool,
    pub draw_order: u8,
}
#[derive(Clone, Copy, Debug)]
pub struct View {
    pub axes: [[f32; 3]; 2],
    pub eye: Option<[f32; 3]>,
    pub world_up: [f32; 3],
    pub effect_billboard: u8,
    pub visible: u128,
    pub texture_size: [u32; 2],
}
impl View {
    pub fn new(axes: [[f32; 3]; 2]) -> Self {
        Self { axes, eye: None, world_up: [0., 0., -1.], effect_billboard: 0, visible: u128::MAX, texture_size: [0; 2] }
    }
}
fn billboard_matrix(center: [f32; 3], kind: u8, view: View) -> Matrix {
    let fallback = math::normalize(math::cross(view.axes[1], view.axes[0]));
    let mut toward = view.eye.map(|eye| math::normalize(math::sub(eye, center))).filter(|v| v.iter().any(|v| v.abs() > 1.0e-6)).unwrap_or(fallback);
    let mut right = math::normalize(math::cross(toward, view.world_up));
    if right.iter().all(|v| v.abs() < 1.0e-6) {
        right = view.axes[0];
    }
    let up = if kind == 2 {
        toward = math::normalize(math::cross(view.world_up, right));
        view.world_up
    } else {
        math::normalize(math::cross(right, toward))
    };
    let mut matrix = math::IDENTITY;
    for (row, axis) in matrix.as_chunks_mut::<4>().0.iter_mut().zip([right, up, toward, center]) {
        row.get_mut(..3).unwrap_or_default().copy_from_slice(&axis);
    }
    matrix
}
#[derive(Debug)]
pub struct AnimatedModel {
    pub model: Model,
    pub frame_count: u32,
    tracks: Vec<Track>,
}

fn finite(point: [f32; 3]) -> bool {
    point.iter().all(|v| v.is_finite() && v.abs() < 1.0e12)
}
fn triangle_normals(vertices: &mut [Vertex], indices: &[u32], init: &Matrix) {
    for vertex in vertices.iter_mut() {
        vertex.normal = [0.; 3];
    }
    for t in indices.as_chunks::<3>().0 {
        let mut points = t.iter().filter_map(|i| vertices.get(*i as usize).map(|v| math::transform(v.position, init)));
        if let (Some(a), Some(b), Some(c)) = (points.next(), points.next(), points.next()) {
            let normal = math::normalize(math::cross(math::sub(c, a), math::sub(b, a)));
            for index in t {
                if let Some(vertex) = vertices.get_mut(*index as usize) {
                    for (v, n) in vertex.normal.iter_mut().zip(normal) {
                        *v += n;
                    }
                }
            }
        }
    }
    for vertex in vertices.iter_mut() {
        vertex.normal = math::normalize(vertex.normal);
    }
    // The client welds coincident export vertices after averaging unit face
    // normals, including vertices split by UV seams. Its tolerance is 0.01.
    let mut cells = std::collections::HashMap::<[i64; 3], Vec<usize>>::new();
    let mut groups = Vec::<([f32; 3], [f32; 3], u32)>::new();
    let mut assignments = Vec::with_capacity(vertices.len());
    for vertex in vertices.iter() {
        let position = math::transform(vertex.position, init);
        if !finite(position) {
            assignments.push(usize::MAX);
            continue;
        }
        let cell = position.map(|value| (f64::from(value) / 0.01).floor() as i64);
        let mut found = None;
        'neighbors: for dx in -1i64..=1 {
            for dy in -1i64..=1 {
                for dz in -1i64..=1 {
                    let key = [cell[0].saturating_add(dx), cell[1].saturating_add(dy), cell[2].saturating_add(dz)];
                    for index in cells.get(&key).into_iter().flatten() {
                        if let Some((point, _, _)) = groups.get(*index)
                            && point.iter().zip(position).all(|(a, b)| (*a - b).abs() < 0.01)
                        {
                            found = Some(*index);
                            break 'neighbors;
                        }
                    }
                }
            }
        }
        let index = found.unwrap_or_else(|| {
            let index = groups.len();
            groups.push((position, [0.; 3], 0));
            cells.entry(cell).or_default().push(index);
            index
        });
        if let Some((_, normal, count)) = groups.get_mut(index) {
            *normal = math::normalize(std::array::from_fn(|i| normal[i] * (*count as f32) + vertex.normal[i]));
            *count = count.saturating_add(1);
        }
        assignments.push(index);
    }
    for (vertex, index) in vertices.iter_mut().zip(assignments) {
        if let Some((_, normal, _)) = groups.get(index) {
            vertex.normal = *normal;
        }
    }
}

fn mesh_normals(rest: &mut Mesh, init: &Matrix, payload: &[u8], tag: &[u8; 4]) -> Result<()> {
    triangle_normals(&mut rest.vertices, &rest.indices, init);
    if matches!(tag, b"PHY2" | b"PHY3" | b"PHY5") {
        let mut reader = Reader::new(payload);
        let (_, count, stride, _) = crate::geometry_counts(&mut reader, tag)?;
        for index in 0..count {
            let bytes = reader.take(stride)?;
            let mut normal = Reader::new(bytes);
            normal.take(if tag == b"PHY2" { 76 } else { 40 })?;
            let value = normal.raw_floats::<3>()?;
            if finite(value)
                && let Some(vertex) = rest.vertices.get_mut(index)
            {
                vertex.normal = math::normalize(value);
            }
        }
    }
    Ok(())
}

impl Skin {
    fn sample(&self, frame: u32, step: u32, tick_ms: f64, override_matrix: Option<Matrix>) -> SampledMesh {
        let mut vertices = self.rest.vertices.clone();
        let local_frame = self.motion.as_ref().map(|m| frame % m.frames).unwrap_or(frame);
        let matrices = self.motion.as_ref().map(|motion| (0..motion.bones).map(|bone| motion.matrix(bone, local_frame)).collect::<Vec<_>>());
        let mut valid = self.binding.invalid.clone();
        let offset = self.keys.uv(local_frame, step);
        for (i, vertex) in vertices.iter_mut().enumerate() {
            let initial = math::transform(vertex.position, &self.binding.init);
            let position = if let Some(matrix) = &override_matrix {
                math::transform(initial, matrix)
            } else if let Some(matrices) = &matrices {
                if matrices.len() == 1 || self.binding.blend_count == 0 {
                    matrices.first().map(|m| math::transform(initial, m)).unwrap_or(initial)
                } else if let Some((bones, weights)) = self.binding.bones.get(i) {
                    let mut position = [0.; 3];
                    let mut total = 0.;
                    for (bone, weight) in bones.iter().zip(weights).take(self.binding.blend_count as usize) {
                        if *weight <= 0. || !weight.is_finite() {
                            continue;
                        }
                        if let Some(matrix) = matrices.get(*bone as usize) {
                            for (dst, src) in position.iter_mut().zip(math::transform(initial, matrix)) {
                                *dst += src * weight;
                            }
                            total += weight;
                        }
                    }
                    if total > 0. { position.map(|v| v / total) } else { initial }
                } else {
                    initial
                }
            } else {
                initial
            };
            let normal = self.rest.vertices.get(i).map(|vertex| vertex.normal).unwrap_or([0.; 3]);
            let normal = if let Some(matrix) = &override_matrix {
                math::direction(normal, matrix)
            } else if let Some(matrices) = &matrices {
                if matrices.len() == 1 || self.binding.blend_count == 0 {
                    matrices.first().map(|matrix| math::direction(normal, matrix)).unwrap_or(normal)
                } else if let Some((bones, weights)) = self.binding.bones.get(i) {
                    let mut transformed = [0.; 3];
                    let mut total = 0.;
                    for (bone, weight) in bones.iter().zip(weights).take(self.binding.blend_count as usize) {
                        if *weight > 0.
                            && weight.is_finite()
                            && let Some(matrix) = matrices.get(*bone as usize)
                        {
                            for (dst, value) in transformed.iter_mut().zip(math::direction(normal, matrix)) {
                                *dst += value * weight;
                            }
                            total += weight;
                        }
                    }
                    if total > 0. { transformed } else { normal }
                } else {
                    normal
                }
            } else {
                normal
            };
            vertex.normal = math::normalize(normal);
            if finite(position) {
                vertex.position = position;
            } else if let Some(valid) = valid.get_mut(i) {
                *valid = true;
            }
            for (uv, delta) in vertex.uv.iter_mut().zip(offset) {
                *uv += delta;
            }
            if let Some([x, y, w, h]) = self.keys.extra.rect {
                vertex.uv = [x + vertex.uv[0] * w, y + vertex.uv[1] * h];
            }
        }
        let indices = self
            .rest
            .indices
            .as_chunks::<3>()
            .0
            .iter()
            .filter(|t| t.iter().all(|i| valid.get(*i as usize) == Some(&false)))
            .flatten()
            .copied()
            .collect::<Vec<_>>();
        SampledMesh {
            vertices,
            indices,
            alpha: self.keys.alpha(local_frame),
            visible: self.keys.draw.iter().find(|key| key.frame == local_frame).is_none_or(|key| key.value != 0),
            colors: self.colors.clone(),
            uv1: self.uv1.clone(),
            uv_steps: [
                offset,
                self.keys.secondary_step[0].map(|v| (f64::from(v) * tick_ms / 33.).fract() as f32),
                self.keys.secondary_step[1].map(|v| (f64::from(v) * tick_ms / 33.).fract() as f32),
            ],
        }
    }
}

fn particles(payload: &[u8], tag: &[u8; 4]) -> Result<Particles> {
    let mut r = Reader::new(payload);
    let name = r.name()?;
    let texture = r.name()?;
    let rows = r.u32()?.max(1);
    let style = if tag == b"PTC3" {
        let mode = r.array::<1>()?.first().copied().unwrap_or(0);
        let mode = if mode >= 100 { mode - 100 } else { mode };
        let world_space = r.array::<1>()?.first().copied().unwrap_or(0) != 0;
        let spin = r.raw_floats::<2>()?;
        let alpha = r.raw_floats::<3>()?;
        let alpha_frames = [r.u32()?, r.u32()?];
        let life_alpha = r.raw_floats::<3>()?;
        let life_range = r.raw_floats::<2>()?;
        if spin.iter().chain(&alpha).chain(&life_alpha).chain(&life_range).any(|v| !v.is_finite() || v.abs() > 1.0e6) {
            return Err(Error("invalid particle animation parameters".into()));
        }
        let stretch = name
            .split("_C3EXP_STRETCH=")
            .nth(1)
            .and_then(|v| v.split(|c: char| !c.is_ascii_digit() && c != '.' && c != '-').next())
            .and_then(|v| v.parse::<f32>().ok())
            .filter(|v| v.is_finite())
            .unwrap_or(1.);
        Some(ParticleStyle { mode, world_space, spin, alpha, alpha_frames, life_alpha, life_range, stretch })
    } else {
        None
    };
    let capacity = r.u32()? as usize;
    let count = r.u32()? as usize;
    if capacity > 65_536 || count == 0 || count > 100_000 {
        return Err(Error("invalid particle capacity/frame count".into()));
    }
    let mut frames = Vec::with_capacity(count.min(4096));
    let mut total = 0usize;
    let mut maximum = 0;
    for _ in 0..count {
        let n = r.u32()? as usize;
        total = total.saturating_add(n);
        if n > capacity || total > 2_000_000 {
            return Err(Error("particle animation exceeds the sample budget".into()));
        }
        maximum = maximum.max(n);
        r.require(n.saturating_mul(if tag == b"PTCL" { 20 } else { 22 }).saturating_add(if n > 0 { 64 } else { 0 }))?;
        let mut ids = Vec::with_capacity(n);
        for index in 0..n {
            ids.push(if tag != b"PTCL" { r.u16()? } else { index as u16 });
        }
        let mut positions = Vec::with_capacity(n);
        let mut sizes = Vec::with_capacity(n);
        let mut cells = Vec::with_capacity(n);
        for _ in 0..n {
            positions.push(r.raw_floats::<3>()?);
        }
        for _ in 0..n {
            cells.push(r.raw_floats::<1>()?.first().copied().unwrap_or(0.));
        }
        for _ in 0..n {
            sizes.push(r.raw_floats::<1>()?.first().copied().unwrap_or(0.));
        }
        let matrix = if n > 0 { r.raw_floats::<16>()? } else { math::IDENTITY };
        frames.push(ParticleFrame { ids, positions, sizes, cells, matrix });
    }
    let mut last = std::collections::HashMap::<u16, (u32, u32, u32)>::new();
    let mut births = Vec::with_capacity(frames.len());
    let mut previous = Vec::with_capacity(frames.len());
    for (frame, pf) in frames.iter().enumerate() {
        let mut born = Vec::with_capacity(pf.ids.len());
        let mut prior = Vec::with_capacity(pf.ids.len());
        for (index, id) in pf.ids.iter().enumerate() {
            let old = last.get(id).filter(|(seen, _, _)| seen.saturating_add(1) == frame as u32).copied();
            let birth = old.map(|(_, _, birth)| birth).unwrap_or(frame as u32);
            born.push(birth);
            prior.push(old.map(|(frame, index, _)| [frame, index]));
            last.insert(*id, (frame as u32, index as u32, birth));
        }
        births.push(born);
        previous.push(prior);
    }
    Ok(Particles { kind: *tag, name, texture, rows, frames, maximum, style, births, previous, extra: Extension::default() })
}

fn trail(payload: &[u8], motion: Option<&[u8]>) -> Result<Trail> {
    let mut r = Reader::new(payload);
    let name = r.name()?;
    let count = r.u32()? as usize;
    if count == 0 || count > 4096 {
        return Err(Error("invalid shape spline count".into()));
    }
    let mut endpoints = [[0.; 3]; 2];
    for spline in 0..count {
        let points = r.u32()? as usize;
        if points > 65_536 {
            return Err(Error("too many shape points".into()));
        }
        for point in 0..points {
            let value = r.raw_floats::<3>()?;
            if spline == 0
                && let Some(dst) = endpoints.get_mut(point)
            {
                *dst = value;
            }
        }
    }
    let texture = r.name()?;
    let segments = (r.u32()? as usize).max(1).saturating_mul(5).min(800);
    let mut matrices = Vec::new();
    if let Some(bytes) = motion {
        let mut r = Reader::new(bytes);
        let frames = r.u32()? as usize;
        if frames == 0 || frames > MAX_MATRICES {
            return Err(Error("invalid shape motion frames".into()));
        }
        r.require(frames.saturating_mul(64))?;
        for _ in 0..frames {
            matrices.push(r.raw_floats::<16>()?);
        }
    }
    if matrices.is_empty() {
        matrices.push(math::IDENTITY);
    }
    Ok(Trail { name, texture, endpoints, matrices, segments })
}

impl AnimatedModel {
    pub fn mesh_style(&self, index: usize) -> Option<MeshStyle> {
        self.tracks.get(index).map(|track| match track {
            Track::Skin(s) => MeshStyle {
                two_sided: s.keys.two_sided,
                billboard: s.keys.billboard,
                skinned: s.binding.blend_count > 0 && s.motion.as_ref().is_some_and(|m| m.bones > 1),
                draw_order: 4,
            },
            Track::Particles(p) => MeshStyle {
                two_sided: true,
                billboard: 0,
                skinned: false,
                draw_order: match &p.kind {
                    b"PTCL" => 1,
                    b"PTCX" => 2,
                    _ => 3,
                },
            },
            Track::Trail(_) => MeshStyle { two_sided: true, billboard: 0, skinned: false, draw_order: 0 },
        })
    }
    pub fn memory_bytes(&self) -> usize {
        self.tracks
            .iter()
            .map(|track| match track {
                Track::Skin(s) => {
                    s.rest.vertices.len() * std::mem::size_of::<Vertex>()
                        + s.uv1.len() * 8
                        + s.colors.len() * 16
                        + s.rest.indices.len() * 4
                        + s.binding.bones.len() * 16
                        + s.motion.as_ref().map(|m| m.matrices.len() * 64 + m.keys.len() * 4).unwrap_or(0)
                }
                Track::Particles(p) => p.frames.len() * 256 + p.frames.iter().map(|f| f.positions.len() * 48).sum::<usize>(),
                Track::Trail(t) => t.matrices.len() * 64,
            })
            .sum()
    }
    pub fn has_continuous_uv(&self) -> bool {
        self.tracks.iter().any(|track| matches!(track,Track::Skin(s) if s.keys.secondary_step.iter().flatten().any(|v|*v!=0.)))
    }
    pub fn is_animated(&self) -> bool {
        self.has_continuous_uv() || self.frame_count > 1 || self.tracks.iter().any(|track| matches!(track,Track::Skin(s) if s.keys.step.iter().any(|v|*v!=0.)))
    }
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let chunks = chunks(bytes)?;
        let motions = chunks.iter().filter(|(tag, _)| tag == b"MOTI").map(|(_, payload)| Motion::parse(payload)).collect::<Result<Vec<_>>>()?;
        let smotions = chunks.iter().filter(|(tag, _)| tag == b"SMOT").map(|(_, p)| *p).collect::<Vec<_>>();
        let mut motions = motions.into_iter();
        let mut smotions = smotions.into_iter();
        let mut tracks = Vec::new();
        let mut warnings = Vec::new();
        let mut extension_target = None;
        for (tag, payload) in chunks {
            if tag == *b"CCFL" {
                if let Some(index) = extension_target {
                    let extra = Extension::parse(payload)?;
                    if extra.relation.is_some_and(|(kind, _, _)| kind != 0) {
                        warnings.push("CCFL particle relation geometry requires a linked-emitter render path".into());
                    }
                    match tracks.get_mut(index) {
                        Some(Track::Skin(s)) => s.keys.extra = extra,
                        Some(Track::Particles(p)) => p.extra = extra,
                        _ => {}
                    }
                }
                continue;
            }
            let track = match &tag {
                b"PHY " | b"PHY2" | b"PHY3" | b"PHY4" | b"PHY5" => {
                    let (mut rest, binding) = mesh(payload, &tag)?;
                    mesh_normals(&mut rest, &binding.init, payload, &tag)?;
                    let motion = motions.next();
                    if let Some(motion) = &motion
                        && motion.bones > 1
                        && binding.blend_count > 0
                    {
                        for (bones, weights) in &binding.bones {
                            for (bone, weight) in bones.iter().zip(weights).take(binding.blend_count as usize) {
                                if !weight.is_finite() || !(0.0..=1.001).contains(weight) {
                                    return Err(Error("invalid animated skin weight".into()));
                                }
                                if *weight > 0. && *bone as usize >= motion.bones {
                                    return Err(Error("animated skin references a missing bone".into()));
                                }
                            }
                        }
                    }
                    let mut uv1 = Vec::new();
                    if tag == *b"PHY5" {
                        let mut reader = Reader::new(payload);
                        let (_, count, _, _) = crate::geometry_counts(&mut reader, &tag)?;
                        for _ in 0..count {
                            reader.take(52)?;
                            let uv = reader.raw_floats::<2>()?;
                            // Some native face effects use million-tile offsets.
                            // Preserve finite coordinates just like primary UVs.
                            if uv.iter().any(|v| !v.is_finite() || v.abs() > 1.0e12) {
                                return Err(Error("invalid secondary UV channel".into()));
                            }
                            uv1.push(uv);
                        }
                    }
                    let mut colors = Vec::new();
                    if rest.texture == "C3EXP_COLOR" {
                        let mut reader = Reader::new(payload);
                        let (_, count, stride, _) = crate::geometry_counts(&mut reader, &tag)?;
                        let offset = if matches!(&tag, b"PHY " | b"PHY2") { 56 } else { 20 };
                        for _ in 0..count {
                            let vertex = reader.take(stride)?;
                            let mut color = Reader::new(vertex);
                            color.take(offset)?;
                            let rgba = color.u32()?;
                            colors.push([
                                ((rgba >> 16) & 255) as f32 / 255.,
                                ((rgba >> 8) & 255) as f32 / 255.,
                                (rgba & 255) as f32 / 255.,
                                (rgba >> 24) as f32 / 255.,
                            ]);
                        }
                    }
                    Track::Skin(Box::new(Skin { rest, binding, motion, keys: Keys::parse(payload, &tag)?, uv1, colors }))
                }
                b"PTCL" | b"PTCX" | b"PTC3" => Track::Particles(particles(payload, &tag)?),
                b"SHAP" => Track::Trail(trail(payload, smotions.next())?),
                // The client effect loader skips this exporter marker.
                b"MNEW" => continue,
                _ => continue,
            };
            if tracks.len() >= 128 {
                return Err(Error("too many animated mesh parts".into()));
            }
            if matches!(&tag, b"PHY3" | b"PHY4" | b"PHY5" | b"PTC3") {
                extension_target = Some(tracks.len());
            }
            tracks.push(track);
        }
        if tracks.is_empty() {
            return Err(Error("file has no renderable C3 mesh, particle, or shape geometry".into()));
        }
        let vertices = tracks
            .iter()
            .map(|track| match track {
                Track::Skin(s) => s.rest.vertices.len(),
                Track::Particles(p) => p.maximum.saturating_mul(4),
                Track::Trail(t) => t.segments.saturating_add(1).saturating_mul(2),
            })
            .sum::<usize>();
        if vertices > crate::MAX_VERTICES {
            return Err(Error("animated geometry exceeds the vertex budget".into()));
        }
        let frame_count = tracks
            .iter()
            .map(|track| match track {
                Track::Skin(skin) => skin.motion.as_ref().map(|m| m.frames).unwrap_or(1),
                Track::Particles(p) => p.frames.len() as u32,
                Track::Trail(t) => t.matrices.len() as u32,
            })
            .max()
            .unwrap_or(1);
        let mut out = Self { model: Model { meshes: Vec::new(), warnings, min: [0.; 3], max: [0.; 3] }, frame_count, tracks };
        if out.memory_bytes() > MAX_ANIMATION_BYTES {
            return Err(Error("animation exceeds the 256 MiB decoded-data budget".into()));
        }
        let initial = out.sample(0, [[1., 0., 0.], [0., 0., -1.]]);
        for (track, mut sample) in out.tracks.iter().zip(initial) {
            let (name, texture, variant) = match track {
                Track::Skin(s) => (&s.rest.name, &s.rest.texture, s.rest.variant.clone()),
                Track::Particles(p) => (&p.name, &p.texture, "Particles".into()),
                Track::Trail(t) => (&t.name, &t.texture, "Trail".into()),
            };
            // GPU allocation follows the maximum topology, including emitters
            // whose first frame is empty. Playback supplies the active indices.
            match track {
                Track::Particles(p) => {
                    sample.indices = (0..p.maximum as u32)
                        .flat_map(|index| {
                            let b = index * 4;
                            [b, b + 1, b + 2, b + 2, b + 1, b + 3]
                        })
                        .collect();
                }
                Track::Trail(t) => {
                    sample.indices = (1..=t.segments as u32)
                        .flat_map(|index| {
                            let b = index * 2;
                            [b - 2, b - 1, b, b, b - 1, b + 1]
                        })
                        .collect();
                }
                _ => {}
            }
            out.model.meshes.push(Mesh {
                name: name.clone(),
                texture: if texture == "C3EXP_COLOR" { String::new() } else { texture.clone() },
                variant,
                vertices: sample.vertices,
                indices: sample.indices,
            });
        }
        let (mut min, mut max) = ([f32::INFINITY; 3], [f32::NEG_INFINITY; 3]);
        for sample_frame in [0, frame_count / 4, frame_count / 2, frame_count.saturating_sub(1)] {
            for sample in out.sample(sample_frame, [[1., 0., 0.], [0., 0., -1.]]) {
                for vertex in sample.vertices {
                    if finite(vertex.position) {
                        for ((lo, hi), p) in min.iter_mut().zip(&mut max).zip(vertex.position) {
                            *lo = lo.min(p);
                            *hi = hi.max(p);
                        }
                    }
                }
            }
        }
        // Atlas emitters can be empty at all four framing samples. Bound all
        // baked centers and a camera-independent quad radius instead.
        for track in &out.tracks {
            if let Track::Particles(p) = track {
                let stretch = p.style.as_ref().map(|s| s.stretch.abs().max(1.)).unwrap_or(1.);
                for frame in &p.frames {
                    for (index, point) in frame.positions.iter().enumerate() {
                        let point = math::transform(*point, &frame.matrix);
                        let radius = frame.sizes.get(index).copied().unwrap_or(0.).abs() * stretch * 2f32.sqrt();
                        if finite(point) && radius.is_finite() && radius <= 1.0e9 {
                            for ((lo, hi), p) in min.iter_mut().zip(&mut max).zip(point) {
                                *lo = lo.min(p - radius);
                                *hi = hi.max(p + radius);
                            }
                        }
                    }
                }
            }
        }
        out.model.min = min.map(|v| if v.is_finite() { v } else { 0. });
        out.model.max = max.map(|v| if v.is_finite() { v } else { 0. });
        Ok(out)
    }
    /// Resolve a catalog attachment to an existing track. Some catalogs use
    /// `l_leg` while the corresponding C3 virtual track is `v_l_leg`. An exact
    /// name always wins; the fallback never invents a missing attachment.
    pub fn resolve_socket(&self, name: &str) -> Option<&str> {
        let find = |virtual_name: bool| {
            self.tracks.iter().find_map(|track| match track {
                Track::Skin(s)
                    if if virtual_name {
                        s.rest.name.get(..2).is_some_and(|prefix| prefix.eq_ignore_ascii_case("v_"))
                            && s.rest.name.get(2..).is_some_and(|suffix| suffix.eq_ignore_ascii_case(name))
                    } else {
                        s.rest.name.eq_ignore_ascii_case(name)
                    } =>
                {
                    Some(s.rest.name.as_str())
                }
                _ => None,
            })
        };
        find(false).or_else(|| find(true))
    }

    pub fn socket(&self, name: &str, frame: u32) -> Option<Matrix> {
        self.tracks.iter().find_map(|track| match track {
            Track::Skin(s) if s.rest.name.eq_ignore_ascii_case(name) => {
                // Exporter init is baked into vertices. Native virtual sockets
                // take their animated matrix directly, without baking init twice.
                Some(s.motion.as_ref().map(|m| m.matrix(0, frame % m.frames)).unwrap_or(s.binding.init))
            }
            _ => None,
        })
    }
    pub fn sample(&self, frame: u32, billboard: [[f32; 3]; 2]) -> Vec<SampledMesh> {
        self.sample_with(frame, billboard, &|_| math::IDENTITY, 0)
    }
    pub fn sample_with(&self, frame: u32, billboard: [[f32; 3]; 2], parent: &dyn Fn(u32) -> Matrix, seed: u32) -> Vec<SampledMesh> {
        self.sample_with_step(frame, frame, billboard, parent, seed)
    }
    pub fn sample_with_step(&self, frame: u32, step: u32, billboard: [[f32; 3]; 2], parent: &dyn Fn(u32) -> Matrix, seed: u32) -> Vec<SampledMesh> {
        self.sample_with_clock(frame, step, f64::from(step) * 33., billboard, parent, seed)
    }
    pub fn sample_with_clock(
        &self,
        frame: u32,
        step: u32,
        tick_ms: f64,
        billboard: [[f32; 3]; 2],
        parent: &dyn Fn(u32) -> Matrix,
        seed: u32,
    ) -> Vec<SampledMesh> {
        self.sample_in_view(frame, step, tick_ms, View::new(billboard), parent, seed)
    }
    pub fn sample_in_view(&self, frame: u32, step: u32, tick_ms: f64, view: View, parent: &dyn Fn(u32) -> Matrix, seed: u32) -> Vec<SampledMesh> {
        let billboard = view.axes;
        let tick_ms = if tick_ms.is_finite() { tick_ms.clamp(0., 86_400_000.) } else { 0. };
        let effect_parent = |at| {
            let original = parent(at);
            if view.effect_billboard == 0 {
                return original;
            }
            let mut pivot = [0.; 3];
            let mut count = 0u32;
            for track in &self.tracks {
                if let Track::Skin(s) = track {
                    let matrix = s.motion.as_ref().map(|m| m.matrix(0, at % m.frames)).unwrap_or(math::IDENTITY);
                    for (sum, value) in pivot.iter_mut().zip(matrix.get(12..15).unwrap_or_default()) {
                        *sum += value;
                    }
                    count += 1;
                }
            }
            if count > 0 {
                pivot = pivot.map(|v| v / count as f32);
            }
            let center = math::transform([0.; 3], &original);
            let mut basis = billboard_matrix(center, view.effect_billboard, view);
            let scale = math::direction([1., 0., 0.], &original).iter().map(|v| v * v).sum::<f32>().sqrt();
            for row in basis.as_chunks_mut::<4>().0.iter_mut().take(3) {
                for v in row.iter_mut().take(3) {
                    *v *= scale;
                }
            }
            let translated = math::transform(pivot.map(|v| -v), &basis);
            for (dst, value) in basis.get_mut(12..15).unwrap_or_default().iter_mut().zip(translated.into_iter().zip(pivot).map(|(a, b)| a + b)) {
                *dst = value;
            }
            basis
        };
        self.tracks
            .iter()
            .enumerate()
            .map(|(index, track)| {
                if view.visible & 1u128.checked_shl(index as u32).unwrap_or(0) == 0 {
                    return SampledMesh::hidden();
                }
                match track {
                    Track::Skin(s) => {
                        let kind = s.keys.billboard;
                        let transform = if matches!(kind, 3 | 4) { parent(frame) } else { effect_parent(frame) };
                        let override_matrix = if matches!(kind, 1 | 2 | 4) {
                            let mut motion = s.motion.as_ref().map(|m| m.matrix(0, frame % m.frames)).unwrap_or(math::IDENTITY);
                            let center = math::transform(math::transform([0.; 3], &motion), &transform);
                            motion.get_mut(12..15).unwrap_or_default().fill(0.);
                            let scale = math::direction([1., 0., 0.], &transform).iter().map(|v| v * v).sum::<f32>().sqrt();
                            for row in motion.as_chunks_mut::<4>().0.iter_mut().take(3) {
                                for v in row.iter_mut().take(3) {
                                    *v *= scale;
                                }
                            }
                            Some(math::multiply(&motion, &billboard_matrix(center, kind, view)))
                        } else {
                            None
                        };
                        let mut sample = s.sample(frame, step, tick_ms, override_matrix);
                        if override_matrix.is_none() {
                            for vertex in &mut sample.vertices {
                                vertex.position = math::transform(vertex.position, &transform);
                                vertex.normal = math::normalize(math::direction(vertex.normal, &transform));
                            }
                        }
                        sample
                    }
                    Track::Particles(p) => {
                        let mut vertices = vec![Vertex { position: [0.; 3], normal: [0., 0., 1.], uv: [0.; 2] }; p.maximum.saturating_mul(4)];
                        let mut colors = vec![[1.; 4]; vertices.len()];
                        let mut indices = Vec::with_capacity(p.maximum.saturating_mul(6));
                        let local = frame as usize % p.frames.len();
                        if let Some(pf) = p.frames.get(local) {
                            let facing = p.style.as_ref().is_some_and(|style| matches!(style.mode, 1 | 4));
                            let future = if facing {
                                p.frames
                                    .get(local + 1)
                                    .map(|frame| frame.ids.iter().copied().zip(frame.positions.iter().copied()).collect::<std::collections::HashMap<_, _>>())
                                    .unwrap_or_default()
                            } else {
                                std::collections::HashMap::new()
                            };
                            for (index, position) in pf.positions.iter().enumerate() {
                                let birth = p.births.get(local).and_then(|v| v.get(index)).copied().unwrap_or(local as u32);
                                let parent_frame = if p.style.as_ref().is_some_and(|s| s.world_space) {
                                    frame.saturating_sub(local as u32).saturating_add(birth)
                                } else {
                                    frame
                                };
                                let transform =
                                    if p.style.as_ref().is_some_and(|s| matches!(s.mode, 3 | 4)) { parent(parent_frame) } else { effect_parent(parent_frame) };
                                let center = math::transform(math::transform(*position, &pf.matrix), &transform);
                                let scale = math::direction([1.; 3], &transform).iter().map(|v| v * v).sum::<f32>().sqrt() / 3f32.sqrt();
                                let size = pf.sizes.get(index).copied().unwrap_or(0.) * scale;
                                let life = pf.cells.get(index).copied().filter(|v| v.is_finite()).unwrap_or(0.);
                                // Exported particle texture positions are normalized over the complete atlas.
                                let columns = if p.extra.columns == 0 { p.rows } else { p.extra.columns };
                                let cell = (life.max(0.) * columns.saturating_mul(p.rows) as f32).trunc() as u32;
                                if !finite(center) || !size.is_finite() || size.abs() > 1.0e9 {
                                    continue;
                                }
                                let rect = p.extra.rect.unwrap_or([0., 0., 1., 1.]);
                                let span = [rect[2] / columns as f32, rect[3] / p.rows as f32];
                                let uv = [rect[0] + (cell % columns) as f32 * span[0], rect[1] + (cell / columns) as f32 * span[1]];
                                let mut axes = billboard;
                                let mut alpha = 1.;
                                let mut stretch = 1.;
                                let mut angle = 0.;
                                if let Some(style) = &p.style {
                                    alpha =
                                        envelope(style.alpha, style.alpha_frames[0] as f32, style.alpha_frames[1] as f32, p.frames.len() as f32, local as f32)
                                            * envelope(style.life_alpha, style.life_range[0], style.life_range[1], 1., life);
                                    let id = pf.ids.get(index).copied().unwrap_or(0);
                                    let fraction = particle_fraction(seed, id, birth);
                                    angle = spin_speed(style.spin, fraction) * local as f32 * std::f32::consts::PI / 180.;
                                    stretch = style.stretch;
                                    if matches!(style.mode, 1 | 4) {
                                        let prior = p.previous.get(local).and_then(|v| v.get(index)).copied().flatten();
                                        if let Some([prior_frame, prior_index]) = prior
                                            && let Some(prior) = p.frames.get(prior_frame as usize)
                                            && let Some(point) = prior.positions.get(prior_index as usize)
                                        {
                                            let past = math::transform(math::transform(*point, &prior.matrix), &parent(frame.saturating_sub(1)));
                                            let along = math::normalize(math::sub(center, past));
                                            let toward = view
                                                .eye
                                                .map(|eye| math::normalize(math::sub(eye, center)))
                                                .unwrap_or_else(|| math::cross(billboard[0], billboard[1]));
                                            axes = [math::normalize(math::cross(along, toward)), along];
                                        } else if let Some(point) = future.get(&id) {
                                            let next = math::transform(math::transform(*point, &pf.matrix), &transform);
                                            let along = math::normalize(math::sub(next, center));
                                            let toward = view
                                                .eye
                                                .map(|eye| math::normalize(math::sub(eye, center)))
                                                .unwrap_or_else(|| math::cross(billboard[0], billboard[1]));
                                            axes = [math::normalize(math::cross(along, toward)), along];
                                        } else if p.extra.relation.is_none_or(|(kind, _, _)| kind == 0) {
                                            continue;
                                        }
                                    }
                                }
                                let random = particle_fraction(seed, pf.ids.get(index).copied().unwrap_or(0), birth);
                                if let Some(range) = p.extra.initial_rotation {
                                    angle += (range[0] + random * (range[1] - range[0])) * std::f32::consts::PI / 180.;
                                }
                                if let Some((degrees, mode)) = p.extra.rotation {
                                    angle += degrees * if mode == 1 { 1. } else { local as f32 } * std::f32::consts::PI / 180.;
                                }
                                if (stretch - 1.).abs() > 1.0e-6 {
                                    stretch += p.extra.stretch * local as f32;
                                }
                                let mut aspect = [1., 1.];
                                if p.extra.columns != 0 && p.rows != 0 {
                                    let [width, height] = view.texture_size;
                                    if width > 0 && height > 0 {
                                        let cell_width = (width as f32 * rect[2] / columns as f32).max(1.);
                                        let cell_height = (height as f32 * rect[3] / p.rows as f32).max(1.);
                                        if p.rows < columns {
                                            aspect[0] = cell_width / cell_height;
                                        } else if columns < p.rows {
                                            aspect[1] = cell_height / cell_width;
                                        }
                                    } else if p.rows < columns {
                                        aspect[0] = p.rows as f32 / columns as f32;
                                    } else if columns < p.rows {
                                        aspect[1] = columns as f32 / p.rows as f32;
                                    }
                                }
                                let (sin, cos) = angle.sin_cos();
                                for (corner, [x, y]) in [[-1., -1.], [1., -1.], [-1., 1.], [1., 1.]].into_iter().enumerate() {
                                    let x1 = (x * cos - y * stretch * sin) * aspect[0];
                                    let y1 = (x * sin + y * stretch * cos) * aspect[1];
                                    if let Some(vertex) = vertices.get_mut(index * 4 + corner) {
                                        vertex.position = std::array::from_fn(|i| center[i] + (axes[0][i] * x1 + axes[1][i] * y1) * size);
                                        vertex.uv = [uv[0] + (x + 1.) * 0.5 * span[0], uv[1] + (1. - y) * 0.5 * span[1]];
                                    }
                                    if let Some(color) = colors.get_mut(index * 4 + corner) {
                                        *color = [1., 1., 1., (alpha.clamp(0., 1.) * 255.).trunc() / 255.];
                                    }
                                }
                                let base = index as u32 * 4;
                                indices.extend([base, base + 1, base + 2, base + 2, base + 1, base + 3]);
                            }
                        }
                        SampledMesh { vertices, indices, alpha: 1., visible: true, colors, uv1: Vec::new(), uv_steps: [[0.; 2]; 3] }
                    }
                    Track::Trail(t) => {
                        let mut vertices = Vec::with_capacity((t.segments + 1) * 2);
                        let mut indices = Vec::with_capacity(t.segments * 6);
                        let local = frame as usize % t.matrices.len();
                        let current = t.matrices.get(local).unwrap_or(&math::IDENTITY);
                        let endpoints = t.endpoints.map(|point| math::transform(math::transform(point, current), &parent(frame)));
                        let width = math::sub(endpoints[0], endpoints[1]).iter().map(|v| v * v).sum::<f32>().sqrt();
                        for segment in 0..=t.segments {
                            let past = (local as f32 - (t.segments - segment) as f32 / 5.).max(0.);
                            let before = past.floor() as usize;
                            let after = (before + 1).min(t.matrices.len() - 1);
                            let fraction = past.fract();
                            let a = t.matrices.get(before).unwrap_or(&math::IDENTITY);
                            let b = t.matrices.get(after).unwrap_or(a);
                            let mut edge = t.endpoints.map(|point| {
                                let pa = math::transform(math::transform(point, a), &parent(frame.saturating_sub(local as u32).saturating_add(before as u32)));
                                let pb = math::transform(math::transform(point, b), &parent(frame.saturating_sub(local as u32).saturating_add(after as u32)));
                                std::array::from_fn(|i| pa[i] + (pb[i] - pa[i]) * fraction)
                            });
                            // Native Shape_Draw preserves the current edge width while
                            // subdividing between poses, avoiding collapse during turns.
                            let along = math::sub(edge[0], edge[1]);
                            let length = along.iter().map(|v| v * v).sum::<f32>().sqrt();
                            if length > 1.0e-6 {
                                edge[0] = std::array::from_fn(|i| edge[1][i] + along[i] * width / length);
                            }
                            for (side, point) in edge.into_iter().enumerate() {
                                vertices.push(Vertex { position: point, normal: [0., 0., 1.], uv: [segment as f32 / t.segments as f32, side as f32] });
                            }
                            if segment > 0 && (t.segments - segment) < local.saturating_mul(5) {
                                let b = segment as u32 * 2;
                                indices.extend([b - 2, b - 1, b, b, b - 1, b + 1]);
                            }
                        }
                        if vertices.iter().any(|vertex| !finite(vertex.position)) {
                            SampledMesh::hidden()
                        } else {
                            SampledMesh { vertices, indices, alpha: 1., visible: true, colors: Vec::new(), uv1: Vec::new(), uv_steps: [[0.; 2]; 3] }
                        }
                    }
                }
            })
            .collect()
    }
}

fn envelope(values: [f32; 3], begin: f32, end: f32, duration: f32, time: f32) -> f32 {
    let value = if time < begin && begin > 0. {
        values[0] + (values[1] - values[0]) * time / begin
    } else if time >= end && duration > end {
        values[1] + (values[2] - values[1]) * (time - end) / (duration - end)
    } else {
        values[1]
    };
    if value.is_finite() { value.clamp(0., 1.) } else { 0. }
}

fn particle_fraction(seed: u32, id: u16, birth: u32) -> f32 {
    let random = seed.wrapping_add(u32::from(id)).wrapping_add(birth.wrapping_mul(65537)).wrapping_mul(214013).wrapping_add(2531011);
    ((random >> 16) & 0x7fff) as f32 / 32767.
}

fn spin_speed([mean, range]: [f32; 2], fraction: f32) -> f32 {
    if range < 0. && mean >= 0. {
        let jitter = range - 2. * range * fraction;
        jitter + if jitter < 0. { -mean } else { mean }
    } else {
        mean + (2. * fraction - 1.) * range
    }
}
#[cfg(test)]
mod spin_tests {
    #[test]
    fn negative_variance_selects_both_rotation_directions() {
        assert_eq!(super::spin_speed([10., -2.], 0.), -12.);
        assert_eq!(super::spin_speed([10., -2.], 0.25), -11.);
        assert_eq!(super::spin_speed([10., -2.], 0.75), 11.);
        assert_eq!(super::spin_speed([10., -2.], 1.), 12.);
    }
}
