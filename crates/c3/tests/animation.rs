use photocraft_c3::{
    animation::{AnimatedModel, Motion},
    material::Timing,
    math::IDENTITY,
};
fn word(out: &mut Vec<u8>, value: u32) {
    out.extend(value.to_le_bytes());
}
fn floats(out: &mut Vec<u8>, values: &[f32]) {
    for value in values {
        out.extend(value.to_le_bytes());
    }
}
fn name(out: &mut Vec<u8>, value: &str) {
    word(out, value.len() as u32);
    out.extend(value.as_bytes());
}
fn chunk(out: &mut Vec<u8>, tag: &[u8; 4], data: &[u8]) {
    out.extend(tag);
    word(out, data.len() as u32);
    out.extend(data);
}
fn motion(kind: &[u8; 4]) -> Vec<u8> {
    let mut out = Vec::new();
    word(&mut out, 1);
    word(&mut out, 11);
    if kind != b"RAW " {
        out.extend(kind);
        word(&mut out, 2);
    }
    for frame in [0, 10] {
        if kind == b"KKEY" {
            word(&mut out, frame);
        } else if kind != b"RAW " {
            out.extend((frame as u16).to_le_bytes());
        }
        let mut matrix = IDENTITY;
        matrix[12] = frame as f32;
        match kind {
            b"ZKEY" => floats(&mut out, &[0., 0., 0., 1., frame as f32, 0., 0.]),
            b"XKEY" => {
                for row in matrix.as_chunks::<4>().0 {
                    floats(&mut out, &row[..3]);
                }
            }
            _ => floats(&mut out, &matrix),
        }
    }
    if kind == b"RAW " {
        out.clear();
        word(&mut out, 1);
        word(&mut out, 11);
        for frame in 0..11 {
            let mut matrix = IDENTITY;
            matrix[12] = frame as f32;
            floats(&mut out, &matrix);
        }
    }
    word(&mut out, 0);
    out
}
fn geometry() -> Vec<u8> {
    let mut out = Vec::new();
    name(&mut out, "blade");
    word(&mut out, 0);
    word(&mut out, 3);
    word(&mut out, 0);
    for position in [[0., 0., 0.], [1., 0., 0.], [0., 0., 1.]] {
        floats(&mut out, &position);
        floats(&mut out, &[0., 0.]);
        word(&mut out, 0);
        word(&mut out, 0);
        word(&mut out, 0);
        floats(&mut out, &[1., 0.]);
    }
    word(&mut out, 1);
    word(&mut out, 0);
    for index in [0u16, 1, 2] {
        out.extend(index.to_le_bytes());
    }
    name(&mut out, "test.dds");
    floats(&mut out, &[0.; 6]);
    floats(&mut out, &IDENTITY);
    word(&mut out, 2);
    for keys in [vec![[0, 0f32.to_bits(), 0, 0], [10, 1f32.to_bits(), 0, 0]], vec![[3, 123, 0, 999]], vec![[0, 0, 0, 1], [5, 0, 0, 2]]] {
        word(&mut out, keys.len() as u32);
        for key in keys {
            for value in key {
                word(&mut out, value);
            }
        }
    }
    out.extend(b"STEP");
    floats(&mut out, &[0.1, -0.1]);
    out
}
#[test]
fn all_motion_formats_interpolate_the_same_integer_frame() {
    for kind in [b"RAW ", b"KKEY", b"XKEY", b"ZKEY"] {
        let motion = Motion::parse(&motion(kind)).unwrap();
        assert_eq!(motion.matrix(0, 5)[12], 5.);
        assert_eq!(motion.matrix(0, 99)[12], 10.);
    }
}
#[test]
fn material_union_fields_match_client_and_last_atlas_key_does_not_latch() {
    let mut out = b"MAXFILE C3 00001".to_vec();
    chunk(&mut out, b"PHY4", &geometry());
    chunk(&mut out, b"MOTI", &motion(b"KKEY"));
    let animated = AnimatedModel::decode(&out).unwrap();
    let frame = |frame| animated.sample(frame, [[1., 0., 0.], [0., 1., 0.]]).remove(0);
    assert_eq!(frame(0).vertices[0].uv, [0.5, 0.]);
    assert_eq!(frame(5).vertices[0].uv, [0., 0.5]);
    assert_eq!(frame(3).alpha, 0.3);
    assert!(!frame(3).visible);
    assert!(frame(4).visible);
    assert_eq!(frame(10).alpha, 1.);
    assert_eq!(frame(10).vertices[0].position, [10., 0., 0.]);
    assert_eq!(frame(6).vertices[0].uv, [0.6, 0.39999998]);
}
fn particle(kind: &[u8; 4]) -> Vec<u8> {
    let mut out = Vec::new();
    name(&mut out, "emitter");
    name(&mut out, "atlas.dds");
    word(&mut out, 2);
    if kind == b"PTC3" {
        out.extend([0, 0]);
        floats(&mut out, &[0., 0., 1., 1., 1.]);
        word(&mut out, 0);
        word(&mut out, 2);
        floats(&mut out, &[1., 1., 1., 0., 1.]);
    }
    word(&mut out, 1);
    word(&mut out, 2);
    word(&mut out, 0);
    word(&mut out, 1);
    if kind != b"PTCL" {
        out.extend(7u16.to_le_bytes());
    }
    floats(&mut out, &[10., 20., 30.]);
    floats(&mut out, &[0.5]);
    floats(&mut out, &[3.]);
    floats(&mut out, &IDENTITY);
    out
}
#[test]
fn every_particle_variant_uses_normalized_atlas_then_size_and_allocates_empty_first_frame() {
    for kind in [b"PTCL", b"PTCX", b"PTC3"] {
        let mut bytes = b"MAXFILE C3 00001".to_vec();
        chunk(&mut bytes, kind, &particle(kind));
        let animated = AnimatedModel::decode(&bytes).unwrap();
        assert_eq!(animated.model.meshes[0].vertices.len(), 4);
        assert_eq!(animated.model.meshes[0].indices.len(), 6);
        assert!(animated.sample(0, [[1., 0., 0.], [0., 1., 0.]])[0].indices.is_empty());
        let sample = animated.sample(1, [[1., 0., 0.], [0., 1., 0.]]);
        let sample = &sample[0];
        assert_eq!(sample.vertices[0].position, [7., 17., 30.]);
        assert_eq!(sample.vertices[0].uv, [0., 1.]);
        for length in 0..bytes.len() {
            assert!(AnimatedModel::decode(&bytes[..length]).is_err(), "accepted {kind:?} truncation {length}");
        }
    }
}
#[test]
fn shape_motion_builds_five_interpolated_segments_per_client_frame() {
    let mut shape = Vec::new();
    name(&mut shape, "edge");
    word(&mut shape, 1);
    word(&mut shape, 2);
    floats(&mut shape, &[0., 0., 0., 1., 0., 0.]);
    name(&mut shape, "trail.dds");
    word(&mut shape, 2);
    let mut motion = Vec::new();
    word(&mut motion, 3);
    for frame in 0..3 {
        let mut matrix = IDENTITY;
        matrix[13] = frame as f32;
        floats(&mut motion, &matrix);
    }
    let mut bytes = b"MAXFILE C3 00001".to_vec();
    chunk(&mut bytes, b"SHAP", &shape);
    chunk(&mut bytes, b"SMOT", &motion);
    let animated = AnimatedModel::decode(&bytes).unwrap();
    assert_eq!(animated.model.meshes[0].vertices.len(), 22);
    let sample = animated.sample(2, [[1., 0., 0.], [0., 1., 0.]]);
    assert_eq!(sample[0].indices.len(), 60);
    assert_eq!(sample[0].vertices.last().unwrap().position, [1., 2., 0.]);
}
#[test]
fn finite_effects_delay_pause_gap_and_end_are_unambiguous() {
    let timing = Timing { delay_ms: 100, frame_ms: 20, loops: 2, loop_interval_ms: 50, ..Default::default() };
    assert_eq!(timing.frame(0.099, 3), None);
    assert_eq!(timing.frame(0.1, 3), Some(0));
    assert_eq!(timing.frame(0.15, 3), Some(2));
    assert_eq!(timing.frame(0.18, 3), None);
    assert_eq!(timing.frame(0.21, 3), Some(0));
    assert_eq!(timing.frame(0.32, 3), None);
    assert_eq!(timing.frame(f64::NAN, 3), None);
}

#[test]
fn ccfl_rectangular_atlas_uses_columns_and_uv_crop_and_checks_record_boundaries() {
    let mut bytes = b"MAXFILE C3 00001".to_vec();
    chunk(&mut bytes, b"PTC3", &particle(b"PTC3"));
    let mut extension = b"ccflag".to_vec();
    word(&mut extension, 19);
    word(&mut extension, 3);
    word(&mut extension, (2 << 16) | 8);
    word(&mut extension, 4);
    word(&mut extension, (99 << 16) | 8);
    word(&mut extension, 123); // Unknown extensions are framed, not guessed.
    word(&mut extension, (6 << 16) | 20);
    floats(&mut extension, &[0.1, 0.2, 0.8, 0.6]);
    chunk(&mut bytes, b"CCFL", &extension);
    let animated = AnimatedModel::decode(&bytes).unwrap();
    let frame = animated.sample(1, [[1., 0., 0.], [0., 1., 0.]]);
    assert_eq!(frame[0].vertices[0].uv, [0.1, 0.8]);
    assert_eq!(frame[0].vertices[1].uv, [0.3, 0.8]);
    for length in 0..extension.len() {
        let mut bad = b"MAXFILE C3 00001".to_vec();
        chunk(&mut bad, b"PTC3", &particle(b"PTC3"));
        chunk(&mut bad, b"CCFL", &extension[..length]);
        assert!(AnimatedModel::decode(&bad).is_err(), "accepted CCFL truncation {length}");
    }
}

#[test]
fn billboard_meshes_face_the_eye_and_bib3_keeps_world_space() {
    use photocraft_c3::animation::View;
    for (tag, expected) in [(b"BILB", [5., -1., 0.]), (b"BIB2", [5., -1., 0.]), (b"BIB3", [5., 0., 1.]), (b"BIB4", [5., -1., 0.])] {
        let mut geometry = geometry();
        geometry.extend(b"2SID");
        geometry.extend(tag);
        let mut bytes = b"MAXFILE C3 00001".to_vec();
        chunk(&mut bytes, b"PHY4", &geometry);
        chunk(&mut bytes, b"MOTI", &motion(b"KKEY"));
        let model = AnimatedModel::decode(&bytes).unwrap();
        assert!(model.mesh_style(0).unwrap().two_sided);
        let view = View {
            axes: [[1., 0., 0.], [0., 0., -1.]],
            eye: Some([5., -100., 0.]),
            world_up: [0., 0., -1.],
            effect_billboard: 0,
            visible: u128::MAX,
            texture_size: [0; 2],
        };
        let frame = model.sample_in_view(5, 5, 165., view, &|_| IDENTITY, 0);
        assert_eq!(frame[0].vertices[2].position, expected, "{tag:?}");
    }
}

#[test]
fn phy5_keeps_the_second_uv_channel_and_scrolls_secondary_layers_by_global_time() {
    let base = geometry();
    let start = 4 + "blade".len() + 12;
    let mut geometry = base[..start].to_vec();
    for vertex in base[start..start + 3 * 40].as_chunks::<40>().0 {
        geometry.extend(vertex);
        floats(&mut geometry, &[0., 0., 1., 0.25, 0.75]);
    }
    geometry.extend(&base[start + 3 * 40..]);
    geometry.extend(b"STEP1");
    floats(&mut geometry, &[0.033, -0.066]);
    geometry.extend(b"STEP2");
    floats(&mut geometry, &[0.1, 0.2]);
    let mut bytes = b"MAXFILE C3 00001".to_vec();
    chunk(&mut bytes, b"PHY5", &geometry);
    chunk(&mut bytes, b"MOTI", &motion(b"KKEY"));
    let model = AnimatedModel::decode(&bytes).unwrap();
    assert!(model.has_continuous_uv());
    let frames = model.sample_with_clock(5, 5, 165., [[1., 0., 0.], [0., 1., 0.]], &|_| IDENTITY, 0);
    let sample = &frames[0];
    assert_eq!(sample.uv1[0], [0.25, 0.75]);
    assert!((sample.uv_steps[1][0] - 0.165).abs() < 1e-6);
    assert!((sample.uv_steps[1][1] + 0.33).abs() < 1e-6);
    assert!((sample.uv_steps[2][0] - 0.5).abs() < 1e-6);
    let later = model.sample_with_clock(5, 5, 330., [[1., 0., 0.], [0., 1., 0.]], &|_| IDENTITY, 0);
    assert!((later[0].uv_steps[1][0] - 0.33).abs() < 1e-6);
}

#[test]
fn phy5_preserves_large_native_secondary_offsets_and_rejects_nonfinite_uvs() {
    let base = geometry();
    let start = 4 + "blade".len() + 12;
    // Native Face2270001-64: -1165084.375, retained bit-for-bit.
    for vertical in [f32::from_bits(0xc98e_38e3), f32::NAN] {
        let mut geometry = base[..start].to_vec();
        for vertex in base[start..start + 3 * 40].as_chunks::<40>().0 {
            geometry.extend(vertex);
            floats(&mut geometry, &[0., 0., 1., 0.25, vertical]);
        }
        geometry.extend(&base[start + 3 * 40..]);
        let mut bytes = b"MAXFILE C3 00001".to_vec();
        chunk(&mut bytes, b"PHY5", &geometry);
        if vertical.is_finite() {
            let model = AnimatedModel::decode(&bytes).unwrap();
            assert_eq!(model.sample(0, [[1., 0., 0.], [0., 1., 0.]])[0].uv1[0], [0.25, vertical]);
        } else {
            assert!(AnimatedModel::decode(&bytes).is_err());
        }
    }
}

#[test]
fn exporter_color_marker_enables_argb_vertex_colors_only_when_explicit() {
    let mut geometry = geometry();
    let first = 4 + "blade".len() + 12;
    for index in 0..3 {
        geometry[first + index * 40 + 20..first + index * 40 + 24].copy_from_slice(&0xff4080c0u32.to_le_bytes());
    }
    let mut plain = b"MAXFILE C3 00001".to_vec();
    chunk(&mut plain, b"PHY4", &geometry);
    assert!(AnimatedModel::decode(&plain).unwrap().sample(0, [[1., 0., 0.], [0., 1., 0.]])[0].colors.is_empty());
    let start = geometry.windows(8).position(|bytes| bytes == b"test.dds").unwrap();
    geometry[start - 4..start].copy_from_slice(&11u32.to_le_bytes());
    geometry.splice(start..start + 8, b"C3EXP_COLOR".iter().copied());
    let mut bytes = b"MAXFILE C3 00001".to_vec();
    chunk(&mut bytes, b"PHY4", &geometry);
    let model = AnimatedModel::decode(&bytes).unwrap();
    let frame = model.sample(0, [[1., 0., 0.], [0., 1., 0.]]);
    assert_eq!(frame[0].colors, vec![[64. / 255., 128. / 255., 192. / 255., 1.]; 3]);
    assert!(model.model.meshes[0].texture.is_empty());
}

#[test]
fn catalog_socket_aliases_use_existing_virtual_tracks_and_exact_names_win() {
    let mut bytes = b"MAXFILE C3 00001".to_vec();
    for label in ["v_l_leg", "v_r_leg", "r_leg"] {
        let mut mesh = Vec::new();
        name(&mut mesh, label);
        mesh.extend(&geometry()[9..]);
        chunk(&mut bytes, b"PHY4", &mesh);
        chunk(&mut bytes, b"MOTI", &motion(b"KKEY"));
    }
    let model = AnimatedModel::decode(&bytes).unwrap();
    assert_eq!(model.resolve_socket("l_leg"), Some("v_l_leg"));
    assert_eq!(model.resolve_socket("V_L_LEG"), Some("v_l_leg"));
    assert_eq!(model.resolve_socket("r_leg"), Some("r_leg"));
    assert_eq!(model.resolve_socket("missing_leg"), None);
    let resolved = model.resolve_socket("l_leg").unwrap();
    assert_eq!(model.socket(resolved, 5).unwrap()[12], 5.);
    assert_ne!(model.socket(resolved, 5), model.socket(resolved, 0));
}

#[test]
fn sockets_use_motion_without_reapplying_exporter_init_and_hidden_tracks_allocate_no_pose() {
    let mut geometry = geometry();
    let texture = geometry.windows(8).position(|p| p == b"test.dds").unwrap();
    let init = texture + 8 + 24;
    geometry[init + 48..init + 52].copy_from_slice(&7f32.to_le_bytes());
    let mut bytes = b"MAXFILE C3 00001".to_vec();
    chunk(&mut bytes, b"PHY4", &geometry);
    chunk(&mut bytes, b"MOTI", &motion(b"KKEY"));
    let model = AnimatedModel::decode(&bytes).unwrap();
    assert_eq!(model.socket("blade", 5).unwrap()[12], 5.);
    assert_eq!(model.sample(5, [[1., 0., 0.], [0., 1., 0.]])[0].vertices[0].position, [12., 0., 0.]);
    let mut view = photocraft_c3::animation::View::new([[1., 0., 0.], [0., 1., 0.]]);
    view.visible = 0;
    let hidden = model.sample_in_view(5, 5, 165., view, &|_| IDENTITY, 0);
    assert!(!hidden[0].visible);
    assert!(hidden[0].vertices.is_empty());
    assert!(hidden[0].indices.is_empty());
}

#[test]
fn effect_parts_share_a_cycle_but_keep_their_own_start_rate_and_once_flag() {
    let slow = Timing { frame_ms: 40, step_frame_ms: 20, frame_offset: 2, duration_ms: 400, ..Default::default() };
    assert_eq!(slow.frame(0.079, 4), None);
    assert_eq!(slow.frame(0.080, 4), Some(0));
    assert_eq!(slow.frame(0.160, 4), Some(2));
    assert_eq!(slow.frame(0.240, 4), Some(0));
    assert_eq!(slow.frame(0.400, 4), None);
    assert_eq!(slow.step(0.240), 12);
    let once = Timing { loop_once: true, ..slow.clone() };
    assert_eq!(once.frame(0.240, 4), None);
    assert_eq!(once.frame(0.480, 4), Some(0));
    let manual = Timing { manual_uv_step: true, ..once };
    assert_eq!(manual.step(1.2), 0);
    let early = Timing { frame_offset: -2, ..slow };
    assert_eq!(early.frame(0., 4), Some(2));
}

#[test]
fn ccfl_initial_rotation_and_rectangular_cells_affect_particle_geometry() {
    use photocraft_c3::animation::View;
    let mut extension = b"ccflag".to_vec();
    word(&mut extension, 1);
    word(&mut extension, 3);
    word(&mut extension, (2 << 16) | 8);
    word(&mut extension, 4);
    word(&mut extension, (6 << 16) | 20);
    floats(&mut extension, &[0., 0., 0.5, 1.]);
    word(&mut extension, (7 << 16) | 12);
    floats(&mut extension, &[90., 90.]);
    let mut bytes = b"MAXFILE C3 00001".to_vec();
    chunk(&mut bytes, b"PTC3", &particle(b"PTC3"));
    chunk(&mut bytes, b"CCFL", &extension);
    let model = AnimatedModel::decode(&bytes).unwrap();
    let view = View { texture_size: [400, 200], ..View::new([[1., 0., 0.], [0., 1., 0.]]) };
    let frame = model.sample_in_view(1, 1, 33., view, &|_| IDENTITY, 0);
    let corner = &frame[0].vertices[0];
    assert!((corner.position[0] - 11.5).abs() < 1.0e-5);
    assert!((corner.position[1] - 17.).abs() < 1.0e-5);
    assert_eq!(corner.uv, [0., 1.]);
}

#[test]
fn moving_particles_face_the_perspective_eye_and_mode_prefix_is_removed_only_once() {
    use photocraft_c3::animation::View;
    for mode in [1u8, 101, 201] {
        let mut particle = particle(b"PTC3");
        let mode_offset = 4 + "emitter".len() + 4 + "atlas.dds".len() + 4;
        particle[mode_offset] = mode;
        let count_offset = mode_offset + 2 + 8 + 12 + 8 + 12 + 8 + 4;
        particle[count_offset..count_offset + 4].copy_from_slice(&3u32.to_le_bytes());
        word(&mut particle, 1);
        particle.extend(7u16.to_le_bytes());
        floats(&mut particle, &[14., 20., 30., 0.75, 3.]);
        floats(&mut particle, &IDENTITY);
        let mut bytes = b"MAXFILE C3 00001".to_vec();
        chunk(&mut bytes, b"PTC3", &particle);
        let model = AnimatedModel::decode(&bytes).unwrap();
        let view = View { eye: Some([20., 40., 40.]), ..View::new([[1., 0., 0.], [0., 1., 0.]]) };
        let frame = model.sample_in_view(2, 2, 66., view, &|_| IDENTITY, 0);
        if mode < 200 {
            assert!((frame[0].vertices[0].position[2] - (30. - 6. / 5f32.sqrt())).abs() < 1e-5);
        } else {
            assert_eq!(frame[0].vertices[0].position[2], 30.);
        }
    }
}

#[test]
fn exported_phy_normals_survive_pose_sampling_and_parent_rotation() {
    let base = geometry();
    let start = 4 + "blade".len() + 12;
    for tag in [b"PHY3", b"PHY5"] {
        let mut geometry = base[..start].to_vec();
        for vertex in base[start..start + 3 * 40].as_chunks::<40>().0 {
            geometry.extend(vertex);
            floats(&mut geometry, &[1., 0., 0.]);
            if tag == b"PHY5" {
                floats(&mut geometry, &[0., 0.]);
            }
        }
        geometry.extend(&base[start + 3 * 40..]);
        let mut bytes = b"MAXFILE C3 00001".to_vec();
        chunk(&mut bytes, tag, &geometry);
        chunk(&mut bytes, b"MOTI", &motion(b"KKEY"));
        let model = AnimatedModel::decode(&bytes).unwrap();
        let rotation = [0., 1., 0., 0., -1., 0., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1.];
        let frame = model.sample_with(5, [[1., 0., 0.], [0., 1., 0.]], &|_| rotation, 0);
        assert_eq!(frame[0].vertices[0].normal, [0., 1., 0.]);
    }
}

#[test]
fn directional_particles_require_a_next_position_on_their_birth_frame() {
    let mut particle = particle(b"PTC3");
    let offset = 4 + "emitter".len() + 4 + "atlas.dds".len() + 4;
    particle[offset] = 1;
    let mut bytes = b"MAXFILE C3 00001".to_vec();
    chunk(&mut bytes, b"PTC3", &particle);
    let model = AnimatedModel::decode(&bytes).unwrap();
    assert!(model.sample(1, [[1., 0., 0.], [0., 1., 0.]])[0].indices.is_empty());
}

#[test]
fn particle_spin_uses_the_subsystems_wrapped_frame() {
    let mut particle = particle(b"PTC3");
    let spin = 4 + "emitter".len() + 4 + "atlas.dds".len() + 4 + 2;
    particle[spin..spin + 4].copy_from_slice(&30f32.to_le_bytes());
    let mut bytes = b"MAXFILE C3 00001".to_vec();
    chunk(&mut bytes, b"PTC3", &particle);
    let model = AnimatedModel::decode(&bytes).unwrap();
    let first = model.sample(1, [[1., 0., 0.], [0., 1., 0.]]);
    let repeated = model.sample(3, [[1., 0., 0.], [0., 1., 0.]]);
    assert_eq!(first[0].vertices[0].position, repeated[0].vertices[0].position);
}
