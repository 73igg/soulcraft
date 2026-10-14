use photocraft_c3::{decode, math::IDENTITY, texture_references};
fn u32v(out: &mut Vec<u8>, v: u32) {
    out.extend(v.to_le_bytes());
}
fn floats(out: &mut Vec<u8>, v: &[f32]) {
    for f in v {
        out.extend(f.to_le_bytes());
    }
}
fn name(out: &mut Vec<u8>, v: &[u8]) {
    u32v(out, v.len() as u32);
    out.extend(v);
}
fn chunk(out: &mut Vec<u8>, tag: &[u8; 4], data: &[u8]) {
    out.extend(tag);
    u32v(out, data.len() as u32);
    out.extend(data);
}
fn mesh(tag: &[u8; 4], bone: u32) -> Vec<u8> {
    let mut b = Vec::new();
    name(&mut b, b"v_body");
    u32v(&mut b, 2);
    u32v(&mut b, 3);
    u32v(&mut b, 0);
    for position in [[0., 0., 0.], [1., 0., 0.], [0., 0., 1.]] {
        floats(&mut b, &position);
        if tag == b"PHY " || tag == b"PHY2" {
            floats(&mut b, &[0.; 9]);
        }
        floats(&mut b, &[0.25, 0.75]);
        u32v(&mut b, 0xffffffff);
        u32v(&mut b, bone);
        u32v(&mut b, 1);
        floats(&mut b, &[0.25, 0.75]);
        if tag == b"PHY2" || tag == b"PHY3" {
            floats(&mut b, &[0., 1., 0.]);
        }
        if tag == b"PHY5" {
            b.extend([0; 20]);
        }
    }
    u32v(&mut b, 1);
    u32v(&mut b, 0);
    for i in [0u16, 1, 2] {
        b.extend(i.to_le_bytes());
    }
    name(&mut b, b"c3/texture/skin.dds");
    floats(&mut b, &[0., 0., 0., 1., 0., 1.]);
    floats(&mut b, &IDENTITY);
    u32v(&mut b, 1);
    for _ in 0..3 {
        u32v(&mut b, 0);
    }
    b
}
fn model(tag: &[u8; 4], kind: &[u8; 4], bone: u32) -> Vec<u8> {
    let mut b = b"MAXFILE C3 00001".to_vec();
    chunk(&mut b, tag, &mesh(tag, bone));
    let mut motion = Vec::new();
    u32v(&mut motion, 2);
    u32v(&mut motion, 1);
    if kind != b"RAW " {
        motion.extend(kind);
        u32v(&mut motion, 1);
        if kind == b"KKEY" {
            u32v(&mut motion, 0);
        } else {
            motion.extend(0u16.to_le_bytes());
        }
    }
    for x in [4., 8.] {
        let mut m = IDENTITY;
        m[12] = x;
        match kind {
            b"ZKEY" => floats(&mut motion, &[0., 0., 0., 1., x, 0., 0.]),
            b"XKEY" => {
                for row in m.as_chunks::<4>().0.iter() {
                    floats(&mut motion, &row[..3]);
                }
            }
            _ => floats(&mut motion, &m),
        }
    }
    u32v(&mut motion, 0);
    chunk(&mut b, b"MOTI", &motion);
    b
}
#[test]
fn every_geometry_and_motion_layout_applies_both_skin_weights() {
    for tag in [b"PHY ", b"PHY2", b"PHY3", b"PHY4", b"PHY5"] {
        for kind in [b"RAW ", b"KKEY", b"XKEY", b"ZKEY"] {
            let b = model(tag, kind, 0);
            let m = decode(&b).unwrap();
            assert_eq!(m.meshes[0].vertices[0].position, [7., 0., 0.]);
            assert_eq!(m.meshes[0].vertices[0].uv, [0.25, 0.75]);
            assert_eq!(texture_references(&b).unwrap(), ["c3/texture/skin.dds"]);
            assert_eq!(m.min, [7., 0., 0.]);
            assert_eq!(m.max, [8., 0., 1.]);
        }
    }
}
#[test]
fn rejects_all_truncations_missing_bones_and_hostile_lengths() {
    let b = model(b"PHY4", b"KKEY", 0);
    let mesh_end = 24 + mesh(b"PHY4", 0).len();
    for n in 0..b.len() {
        if n != mesh_end {
            assert!(decode(&b[..n]).is_err(), "accepted truncation {n}");
        }
    }
    assert!(decode(&model(b"PHY4", b"KKEY", 99)).is_err());
    let mut b = b"MAXFILE C3 00001".to_vec();
    chunk(&mut b, b"PHY4", &u32::MAX.to_le_bytes());
    assert!(decode(&b).is_err());
    let mut b = model(b"PHY4", b"KKEY", 0);
    b[50..54].copy_from_slice(&f32::NAN.to_le_bytes());
    assert!(decode(&b).is_err());
}
#[test]
fn accepts_alternate_exporter_header_and_unknown_framed_chunks() {
    let mut b = model(b"PHY5", b"XKEY", 0);
    b[..16].copy_from_slice(b"ACMEMAX AE 00008");
    chunk(&mut b, b"TEST", &[0; 12]);
    assert_eq!(decode(&b).unwrap().meshes.len(), 1);
    chunk(&mut b, b"PTCL", &[]);
    assert_eq!(decode(&b).unwrap().warnings.len(), 1);
}
#[test]
fn animation_only_files_explain_why_they_cannot_be_shown() {
    assert!(decode(b"MAXFILE C3 00001").unwrap_err().to_string().contains("no renderable"));
}
#[test]
fn random_inputs_never_panic() {
    let mut state = 0x31415926u32;
    for length in 0..2048 {
        let mut b = Vec::new();
        for _ in 0..length {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            b.push(state as u8);
        }
        if length >= 16 {
            b[..16].copy_from_slice(b"MAXFILE C3 00001");
        }
        let _ = decode(&b);
        let _ = texture_references(&b);
    }
}

#[test]
fn damaged_unused_motion_bones_do_not_hide_valid_geometry() {
    let mut b = model(b"PHY4", b"KKEY", 0);
    // Give every vertex full weight on bone 0, then corrupt the unused bone 1.
    let mesh_start = 24;
    let vertex_start = mesh_start + 4 + 6 + 12;
    for vertex in 0..3 {
        floats_at(&mut b, vertex_start + vertex * 40 + 32, &[1., 0.]);
    }
    let motion_start = 24 + mesh(b"PHY4", 0).len() + 8;
    floats_at(&mut b, motion_start + 20 + 64, &[f32::NAN]);
    let decoded = decode(&b).unwrap();
    assert_eq!(decoded.meshes[0].indices.len(), 3);
    assert!(decoded.warnings.is_empty());
}
fn floats_at(bytes: &mut [u8], offset: usize, values: &[f32]) {
    for (i, value) in values.iter().enumerate() {
        bytes[offset + i * 4..offset + i * 4 + 4].copy_from_slice(&value.to_le_bytes());
    }
}
