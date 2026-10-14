//! Fixed-size vector and Direct3D row-vector matrix operations.
#![allow(clippy::indexing_slicing)] // All math indices address fixed-size arrays or constant-size chunks.
use crate::{Error, Result};

pub const IDENTITY: [f32; 16] = [1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1.];
pub fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
pub fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}
pub fn normalize(v: [f32; 3]) -> [f32; 3] {
    let length = v.iter().map(|n| n * n).sum::<f32>().sqrt();
    if length.is_finite() && length > 1.0e-12 { v.map(|n| n / length) } else { [0., 0., 1.] }
}
pub fn transform(v: [f32; 3], m: &[f32; 16]) -> [f32; 3] {
    [v[0] * m[0] + v[1] * m[4] + v[2] * m[8] + m[12], v[0] * m[1] + v[1] * m[5] + v[2] * m[9] + m[13], v[0] * m[2] + v[1] * m[6] + v[2] * m[10] + m[14]]
}
pub fn quaternion_matrix(q: [f32; 4], t: [f32; 3]) -> Result<[f32; 16]> {
    let len = q.iter().map(|v| v * v).sum::<f32>().sqrt();
    if !len.is_finite() || len < 1.0e-8 {
        return Err(Error("invalid motion quaternion".into()));
    }
    let [x, y, z, w] = q.map(|v| v / len);
    Ok([
        1. - 2. * (y * y + z * z),
        2. * (x * y + z * w),
        2. * (x * z - y * w),
        0.,
        2. * (x * y - z * w),
        1. - 2. * (x * x + z * z),
        2. * (y * z + x * w),
        0.,
        2. * (x * z + y * w),
        2. * (y * z - x * w),
        1. - 2. * (x * x + y * y),
        0.,
        t[0],
        t[1],
        t[2],
        1.,
    ])
}
