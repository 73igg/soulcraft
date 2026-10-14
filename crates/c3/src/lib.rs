//! Bounded, read-only C3 mesh importer. Positions are posed at the first embedded
//! motion key; no client DLLs, exporter code, or game assets are embedded.
#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

#[cfg(not(target_arch = "wasm32"))]
mod archive;
#[cfg(not(target_arch = "wasm32"))]
pub mod assets;
pub mod math;
mod reader;

use reader::Reader;

pub const MAX_FILE_BYTES: usize = 128 * 1024 * 1024;
const MAX_MESHES: usize = 128;
const MAX_VERTICES: usize = 2_000_000;
const MAX_INDICES: usize = 6_000_000;

#[derive(Debug, thiserror::Error)]
#[error("C3: {0}")]
pub struct Error(pub String);
type Result<T> = std::result::Result<T, Error>;

#[derive(Clone, Debug)]
pub struct Vertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    /// Direct3D UVs: V=0 is the top of the image, as in the editor.
    pub uv: [f32; 2],
}

#[derive(Clone, Debug)]
pub struct Mesh {
    pub name: String,
    pub texture: String,
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u32>,
    pub variant: String,
}

#[derive(Clone, Debug)]
pub struct Model {
    pub meshes: Vec<Mesh>,
    pub warnings: Vec<String>,
    pub min: [f32; 3],
    pub max: [f32; 3],
}

struct Binding {
    init: [f32; 16],
    bones: Vec<([u32; 2], [f32; 2])>,
    blend_count: u32,
    invalid: Vec<bool>,
}

/// Parse complete chunk framing, including unknown chunks. Malformed chunks are
/// rejected rather than returning a plausible-looking partial model.
fn chunks(bytes: &[u8]) -> Result<Vec<([u8; 4], &[u8])>> {
    if bytes.len() > MAX_FILE_BYTES {
        return Err(Error("file exceeds the 128 MiB preview limit".into()));
    }
    let mut r = Reader::new(bytes);
    let header = r.take(16)?;
    if header != b"MAXFILE C3 00001" && header != b"ACMEMAX AE 00008" {
        return Err(Error("unrecognized header (expected MAXFILE C3 or ACMEMAX AE)".into()));
    }
    let mut out = Vec::new();
    while r.remaining() != 0 {
        if out.len() >= 4096 {
            return Err(Error("too many chunks".into()));
        }
        let tag = r.array::<4>()?;
        if !tag.iter().all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || *b == b' ') {
            return Err(Error("invalid chunk tag".into()));
        }
        let length = r.u32()? as usize;
        out.push((tag, r.take(length)?));
    }
    Ok(out)
}

fn geometry_counts(r: &mut Reader<'_>, tag: &[u8; 4]) -> Result<(String, usize, usize, u32)> {
    let name = r.name()?;
    let blend_count = r.u32()?;
    if blend_count > 2 {
        return Err(Error("unsupported vertex blend count".into()));
    }
    let vertices = (r.u32()? as usize).checked_add(r.u32()? as usize).ok_or_else(|| Error("vertex count overflow".into()))?;
    if vertices > 65_536 {
        return Err(Error("mesh has more vertices than its 16-bit indices can address".into()));
    }
    let stride = match tag {
        b"PHY " => 76,
        b"PHY2" => 88,
        b"PHY3" => 52,
        b"PHY4" => 40,
        b"PHY5" => 60,
        _ => return Err(Error("unsupported PHY variant".into())),
    };
    r.require(vertices.saturating_mul(stride))?;
    Ok((name, vertices, stride, blend_count))
}

fn triangles(r: &mut Reader<'_>, vertices: usize) -> Result<Vec<u32>> {
    let count = (r.u32()? as usize).checked_add(r.u32()? as usize).and_then(|v| v.checked_mul(3)).ok_or_else(|| Error("index count overflow".into()))?;
    if count > MAX_INDICES {
        return Err(Error("too many triangle indices".into()));
    }
    r.require(count.saturating_mul(2))?;
    let mut out = Vec::with_capacity(count);
    for _ in 0..count {
        let i = r.u16()? as u32;
        if i as usize >= vertices {
            return Err(Error("triangle references a missing vertex".into()));
        }
        out.push(i);
    }
    Ok(out)
}

fn mesh(payload: &[u8], tag: &[u8; 4]) -> Result<(Mesh, Binding)> {
    let mut r = Reader::new(payload);
    let (name, count, _stride, blend_count) = geometry_counts(&mut r, tag)?;
    let morphs = if matches!(tag, b"PHY " | b"PHY2") { 4 } else { 1 };
    let mut vertices = Vec::with_capacity(count);
    let mut bones = Vec::with_capacity(count);
    let mut invalid = Vec::with_capacity(count);
    for _ in 0..count {
        let position = r.raw_floats::<3>()?;
        let bad_position = position.iter().any(|v| !v.is_finite() || v.abs() > 1.0e12);
        r.take((morphs - 1) * 12)?;
        let uv = r.raw_floats::<2>()?;
        let bad_uv = uv.iter().any(|v| !v.is_finite() || v.abs() > 1.0e12);
        r.take(4)?; // D3DCOLOR; the preview shows the texture without baked tint.
        let indices = [r.u32()?, r.u32()?];
        let weights = r.raw_floats::<2>()?;
        if matches!(tag, b"PHY2" | b"PHY3") {
            r.take(12)?;
        }
        if tag == b"PHY5" {
            r.take(20)?;
        }
        vertices.push(Vertex { position: if bad_position { [0.; 3] } else { position }, normal: [0.0; 3], uv: if bad_uv { [0.; 2] } else { uv } });
        bones.push((indices, weights));
        invalid.push(bad_uv || bad_position);
    }
    let indices = triangles(&mut r, count)?;
    let texture = r.name()?;
    r.take(24)?; // Exporter bounds are unreliable; recompute after skinning.
    let init = r.raw_floats::<16>()?;
    let bad_init = init.iter().any(|v| !v.is_finite() || v.abs() > 1.0e12);
    if bad_init {
        invalid.fill(true);
    }
    let _tex_rows = r.u32()?;
    for _ in 0..3 {
        let keys = r.u32()? as usize;
        r.take(keys.checked_mul(16).ok_or_else(|| Error("key count overflow".into()))?)?;
    }
    Ok((
        Mesh { name, texture, vertices, indices, variant: String::from_utf8_lossy(tag).trim().into() },
        Binding { init: if bad_init { math::IDENTITY } else { init }, bones, blend_count, invalid },
    ))
}

fn first_pose(payload: &[u8]) -> Result<Vec<[f32; 16]>> {
    let mut r = Reader::new(payload);
    let bones = r.u32()? as usize;
    let frames = r.u32()? as usize;
    if bones > 4096 || frames > 1_000_000 || bones == 0 || frames == 0 {
        return Err(Error("invalid motion bone/frame counts".into()));
    }
    let tag = r.array::<4>()?;
    let mut out = Vec::with_capacity(bones);
    match &tag {
        b"KKEY" | b"XKEY" | b"ZKEY" => {
            let keys = r.u32()? as usize;
            if keys == 0 || keys > 1_000_000 {
                return Err(Error("invalid motion key count".into()));
            }
            let (pos_size, bone_size) = match &tag {
                b"KKEY" => (4, 64),
                b"XKEY" => (2, 48),
                _ => (2, 28),
            };
            let size = bones
                .checked_mul(bone_size)
                .and_then(|n| n.checked_add(pos_size))
                .and_then(|n| n.checked_mul(keys))
                .ok_or_else(|| Error("motion size overflow".into()))?;
            r.require(size)?;
            r.take(pos_size)?;
            for _ in 0..bones {
                let m = match &tag {
                    b"KKEY" => r.raw_floats::<16>()?,
                    b"XKEY" => {
                        let v = r.raw_floats::<12>()?;
                        let mut m = math::IDENTITY;
                        for (dst, src) in m.as_chunks_mut::<4>().0.iter_mut().zip(v.as_chunks::<3>().0.iter()) {
                            dst[..3].copy_from_slice(src);
                        }
                        m
                    }
                    _ => {
                        let q = r.raw_floats::<4>()?;
                        let t = r.raw_floats::<3>()?;
                        math::quaternion_matrix(q, t).unwrap_or([f32::NAN; 16])
                    }
                };
                out.push(m);
            }
            // Only the first pose is needed; framing and total byte size were checked above.
        }
        _ => {
            r = Reader::new(payload.get(8..).ok_or_else(|| Error("truncated motion".into()))?);
            r.require(bones.checked_mul(frames).and_then(|n| n.checked_mul(64)).ok_or_else(|| Error("motion size overflow".into()))?)?;
            for _ in 0..bones {
                out.push(r.raw_floats::<16>()?);
                r.take((frames - 1).saturating_mul(64))?;
            }
        }
    }
    Ok(out)
}

/// Import all PHY variants and apply both skin weights with the first MOTI pose.
pub fn decode(bytes: &[u8]) -> Result<Model> {
    let mut meshes = Vec::new();
    let mut bindings = Vec::new();
    let mut poses = Vec::new();
    let mut warnings = Vec::new();
    let (mut vertex_count, mut index_count) = (0usize, 0usize);
    for (tag, payload) in chunks(bytes)? {
        match &tag {
            b"PHY " | b"PHY2" | b"PHY3" | b"PHY4" | b"PHY5" => {
                if meshes.len() >= MAX_MESHES {
                    return Err(Error("too many mesh parts".into()));
                }
                let (m, b) = mesh(payload, &tag)?;
                vertex_count = vertex_count.saturating_add(m.vertices.len());
                index_count = index_count.saturating_add(m.indices.len());
                if vertex_count > MAX_VERTICES || index_count > MAX_INDICES {
                    return Err(Error("model exceeds the preview geometry budget".into()));
                }
                meshes.push(m);
                bindings.push(b);
            }
            b"MOTI" => {
                if poses.len() >= MAX_MESHES {
                    return Err(Error("too many motion tracks".into()));
                }
                poses.push(first_pose(payload)?);
            }
            b"MNEW" => {
                if !warnings.iter().any(|w| w == "MNEW extension is not applied in the static mesh preview") {
                    warnings.push("MNEW extension is not applied in the static mesh preview".into());
                }
            }
            b"PTCL" | b"PTCX" | b"PTC3" | b"SHAP" | b"SMOT" => {
                let warning = format!("{} effects are not shown in the static mesh preview", String::from_utf8_lossy(&tag));
                if !warnings.contains(&warning) {
                    warnings.push(warning);
                }
            }
            _ => {}
        }
    }
    if !meshes.iter().any(|m| !m.indices.is_empty()) {
        return Err(Error("file has no renderable PHY geometry (it may contain animation or particles only)".into()));
    }
    if !poses.is_empty() && poses.len() != meshes.len() {
        warnings.push(format!("{} mesh parts but {} motion tracks; unmatched parts use their initial transform", meshes.len(), poses.len()));
    }
    let mut min = [f32::INFINITY; 3];
    let mut max = [f32::NEG_INFINITY; 3];
    for (i, (mesh, mut binding)) in meshes.iter_mut().zip(bindings).enumerate() {
        let pose = poses.get(i);
        for ((vertex, (bones, weights)), invalid) in mesh.vertices.iter_mut().zip(binding.bones).zip(&mut binding.invalid) {
            let initial = math::transform(vertex.position, &binding.init);
            let mut position = [0.0; 3];
            let mut total = 0.0;
            if let Some(pose) = pose {
                if pose.len() == 1 || binding.blend_count == 0 {
                    if let Some(matrix) = pose.first() {
                        position = math::transform(initial, matrix);
                        total = 1.;
                    }
                } else {
                    for (bone, weight) in bones.into_iter().zip(weights).take(binding.blend_count as usize) {
                        if !weight.is_finite() || weight < 0. || weight > 1.001 {
                            return Err(Error("invalid skin weights".into()));
                        }
                        if weight <= 0.0 {
                            continue;
                        }
                        let matrix = pose.get(bone as usize).ok_or_else(|| Error(format!("{} references missing bone {bone}", mesh.name)))?;
                        let point = math::transform(initial, matrix);
                        for (dst, src) in position.iter_mut().zip(point) {
                            *dst += src * weight;
                        }
                        total += weight;
                    }
                }
            }
            vertex.position = if total > 0.0 { position.map(|p| p / total) } else { initial };
            if vertex.position.iter().any(|p| !p.is_finite() || p.abs() > 1.0e12) {
                // Some production scenes contain unused/collapsed bones with
                // NaN translations. Only affected triangles are omitted, with
                // an explicit note; invalid values never reach GPU buffers.
                *invalid = true;
                vertex.position = initial;
            }
        }
        let before = mesh.indices.len();
        mesh.indices =
            mesh.indices.as_chunks::<3>().0.iter().filter(|t| t.iter().all(|i| binding.invalid.get(*i as usize) == Some(&false))).flatten().copied().collect();
        if before != mesh.indices.len() {
            warnings.push(format!("{}: omitted {} triangles with invalid vertex, UV, or pose values", mesh.name, (before - mesh.indices.len()) / 3));
        }
        // Hidden marker meshes still retain their index-aligned MOTI slot, but do
        // not enlarge framing bounds or become selectable render geometry.
        if mesh.indices.is_empty() {
            continue;
        }
        for index in &mesh.indices {
            if let Some(vertex) = mesh.vertices.get(*index as usize) {
                for ((lo, hi), p) in min.iter_mut().zip(&mut max).zip(vertex.position) {
                    *lo = lo.min(p);
                    *hi = hi.max(p);
                }
            }
        }
        for triangle in mesh.indices.as_chunks::<3>().0.iter() {
            let mut points = triangle.iter().filter_map(|i| mesh.vertices.get(*i as usize).map(|v| v.position));
            if let (Some(a), Some(b), Some(c)) = (points.next(), points.next(), points.next()) {
                let normal = math::cross(math::sub(b, a), math::sub(c, a));
                for i in triangle {
                    if let Some(v) = mesh.vertices.get_mut(*i as usize) {
                        for (dst, n) in v.normal.iter_mut().zip(normal) {
                            *dst += n;
                        }
                    }
                }
            }
        }
        for v in &mut mesh.vertices {
            v.normal = math::normalize(v.normal);
        }
    }
    if min.iter().chain(&max).any(|p| !p.is_finite()) {
        return Err(Error("model has no finite bounds".into()));
    }
    Ok(Model { meshes, warnings, min, max })
}

/// Read texture references without allocating vertex/index arrays. Intended for
/// background discovery in clients without usable mapping catalogs.
pub fn texture_references(bytes: &[u8]) -> Result<Vec<String>> {
    let mut out = Vec::new();
    for (tag, payload) in chunks(bytes)? {
        if !matches!(&tag, b"PHY " | b"PHY2" | b"PHY3" | b"PHY4" | b"PHY5") {
            continue;
        }
        if out.len() >= MAX_MESHES {
            return Err(Error("too many mesh parts".into()));
        }
        let mut r = Reader::new(payload);
        let (_, vertices, stride, _) = geometry_counts(&mut r, &tag)?;
        r.take(vertices.saturating_mul(stride))?;
        let indices = (r.u32()? as usize).checked_add(r.u32()? as usize).and_then(|n| n.checked_mul(6)).ok_or_else(|| Error("index size overflow".into()))?;
        r.take(indices)?;
        out.push(r.name()?);
    }
    Ok(out)
}
