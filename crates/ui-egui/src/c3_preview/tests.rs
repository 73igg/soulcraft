use super::*;

#[test]
fn targa_preview_preserves_alpha_through_import_edit_and_export() {
    for rle in [false, true] {
        for bits in [24u8, 32] {
            for top in [false, true] {
                let mut bytes = vec![0u8; 18];
                bytes[2] = if rle { 10 } else { 2 };
                bytes[12] = 2;
                bytes[14] = 1;
                bytes[16] = bits;
                bytes[17] = if bits == 32 { 8 } else { 0 } | if top { 32 } else { 0 };
                if rle {
                    bytes.push(1); // A raw packet containing two pixels.
                }
                for alpha in [0, 128] {
                    bytes.extend([0, 0, 255]);
                    if bits == 32 {
                        bytes.push(alpha);
                    }
                }
                let asset = TextureAsset { key: "skin.TGA".into(), path: None, bytes: Some(Arc::new(bytes.clone())), source: false, note: None };
                assert_eq!(workspace::texture_pixels(&asset), Some(2));
                let mut doc = photocraft_io::import("skin.tga", &bytes).unwrap().document;
                if !rle && bits == 32 && top {
                    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
                    app.session.add_document(doc.clone(), Some("skin.TGA".into()));
                    assert!(visible(&app));
                }
                let plain = texture_image(&doc, None, None, false).unwrap();
                let preview = texture_image(&doc, None, None, true).unwrap();
                assert_eq!(plain.pixels, [255, 0, 0, 255, 255, 0, 0, 255]);
                assert_eq!(preview.pixels, if bits == 32 { vec![255, 0, 0, 0, 255, 0, 0, 128] } else { plain.pixels });
                if bits == 32 {
                    doc.channels[0].surface.write_pixel(0, 0, &[1.]);
                    assert_eq!(texture_image(&doc, None, None, true).unwrap().pixels[3], 255);
                    // Previewing must leave the editable alpha channel intact.
                    assert_eq!(doc.channels.len(), 1);
                    assert_eq!(doc.channels[0].surface.sample_channel(1, 0, 0), 128. / 255.);
                    doc.layers[0].visible = false;
                    assert_eq!(texture_image(&doc, None, None, true).unwrap().pixels, [255, 255, 255, 255, 255, 255, 255, 128]);
                }
                let exported = photocraft_io::export(&doc, "skin.tga", &Default::default()).unwrap();
                let reopened = photocraft_io::import("skin.tga", &exported.bytes).unwrap().document;
                assert_eq!(texture_image(&doc, None, None, true).unwrap().pixels, texture_image(&reopened, None, None, true).unwrap().pixels);
            }
        }
    }
}

#[test]
fn texture_preview_follows_visible_composite_at_all_depths() {
    use photocraft_color::PixelFormat;
    use photocraft_doc::{ColorMode, Layer, SampleType};
    use photocraft_geom::Size;
    use photocraft_raster::Surface;
    for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
        let mut doc = Document::new("texture", Size::new(2, 2), ColorMode::Rgb, depth);
        let mut surface = Surface::new(PixelFormat::new(ColorMode::Rgb, depth, true));
        for y in 0..2 {
            for x in 0..2 {
                surface.write_pixel(x, y, &[1., 0., 0., 1.]);
            }
        }
        let mut layer = Layer::new("red", photocraft_doc::LayerContent::Raster(surface));
        layer.visible = true;
        doc.layers.push(layer);
        let image = texture_image(&doc, None, None, false).unwrap();
        assert_eq!(image.size, [2, 2]);
        assert_eq!(image.pixels, vec![255, 0, 0, 255, 255, 0, 0, 255, 255, 0, 0, 255, 255, 0, 0, 255]);
    }
}

#[test]
fn marker_socket_names_are_hidden_but_body_is_visible() {
    assert!(marker("v_r_weapon"));
    assert!(marker("V_HEAD"));
    assert!(!marker("v_body"));
    assert!(!marker("custom_mesh"));
}

#[test]
fn standalone_head_and_weapon_meshes_remain_visible() {
    use photocraft_c3::{Mesh, Model, Vertex};
    let part = |name: &str, count: usize| Mesh {
        name: name.into(),
        texture: String::new(),
        vertices: (0..count).map(|_| Vertex { position: [0.; 3], normal: [0., 0., 1.], uv: [0.; 2] }).collect(),
        indices: vec![0, 1, 2],
        variant: "PHY4".into(),
    };
    let model = Model { meshes: vec![part("v_head", 1000)], warnings: Vec::new(), min: [0.; 3], max: [1.; 3] };
    assert_eq!(visibility(&model), [true]);
    let model = Model { meshes: vec![part("v_r_weapon", 12)], ..model };
    assert_eq!(visibility(&model), [true]);
    let model = Model { meshes: vec![part("v_head", 12), part("v_body", 1000)], ..model };
    assert_eq!(visibility(&model), [false, true]);
}

#[test]
fn view_commands_are_validated_without_changing_document() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let ctx = egui::Context::default();
    let default = app.ui.c3_preview.camera.clone();
    assert!(menu(&mut app, &ctx, "preview.c3.view", &json!({"view":"wrong"})).unwrap().is_err());
    assert_eq!(app.ui.c3_preview.camera, default);
    assert!(menu(&mut app, &ctx, "preview.c3.view", &json!({"view":"back"})).unwrap().is_ok());
    assert_ne!(app.ui.c3_preview.camera, default);
    assert!(menu(&mut app, &ctx, "preview.c3.load", &json!({"path":123})).unwrap().is_err());
    assert!(app.session.documents().is_empty());
}

#[test]
fn invalid_settings_are_atomic_and_invalid_camera_values_do_not_escape() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let before = app.ui.c3_preview.clone();
    for params in [
        json!({"lit":true,"alpha":99}),
        json!({"lit":true,"rotation":[0,0,0,0]}),
        json!({"distance":0}),
        json!({"zoom":0.01}),
        json!({"zoom":1001}),
        json!({"zoom":"close"}),
        json!({"zoom":10,"distance":2}),
        json!({"flipV":"yes"}),
        json!({"pan":[1e30,0]}),
        json!({"root":"wrong"}),
    ] {
        assert!(settings(&mut app, &params).is_err());
        assert_eq!(app.ui.c3_preview, before);
    }
}

#[test]
fn close_zoom_and_model_fit_commands_do_not_require_or_edit_a_document() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let ctx = egui::Context::default();
    settings(&mut app, &json!({"zoom":1000})).unwrap();
    assert!((app.ui.c3_preview.camera.zoom_factor() - 1000.).abs() < 0.001);
    assert_eq!(app.ui.c3_preview.camera.distance, 1.15);
    let before = app.ui.c3_preview.camera.clone();
    for params in [json!({"fit":false}), json!({"fit":"wrong"})] {
        assert!(menu(&mut app, &ctx, "preview.c3.reset", &params).unwrap().is_err());
        assert_eq!(app.ui.c3_preview.camera, before);
    }
    assert!(menu(&mut app, &ctx, "preview.c3.reset", &json!({"fit":"model"})).unwrap().is_ok());
    assert_eq!(app.ui.c3_preview.camera, Camera::default());
    assert!(app.session.active().is_none());
}

#[test]
fn dds_mip_preview_uses_that_surfaces_dimensions() {
    use photocraft_doc::{ColorMode, Document, Layer, LayerContent, SampleType};
    use photocraft_geom::Size;
    let mut doc = Document::new("mips", Size::new(16, 16), ColorMode::Rgb, SampleType::U8);
    let mut surface = photocraft_raster::Surface::new(photocraft_color::PixelFormat::RGBA8);
    for y in 0..16 {
        for x in 0..16 {
            surface.write_pixel(x, y, &[0., 1., 0., 1.]);
        }
    }
    doc.layers.push(Layer::new("green", LayerContent::Raster(surface)));
    let mut options = photocraft_io::ExportOptions::default();
    options.encode.dds.mipmaps = "generate".into();
    let bytes = photocraft_io::export(&doc, "mips.dds", &options).unwrap().bytes;
    let imported = photocraft_io::import("mips.dds", &bytes).unwrap().document;
    let id = imported.metadata.dds.as_ref().unwrap().layers[1].0;
    let image = texture_image(&imported, Some(id), None, false).unwrap();
    assert_eq!(image.size, [8, 8]);
    assert!(image.pixels.as_chunks::<4>().0.iter().all(|p| *p == [0, 255, 0, 255]));
}

#[test]
fn mesh_arrives_before_textures_and_completion_preserves_part_choices() {
    let id = DocId(42);
    let model = Arc::new(photocraft_c3::Model {
        meshes: vec![photocraft_c3::Mesh { name: "body".into(), texture: String::new(), vertices: Vec::new(), indices: Vec::new(), variant: "PHY4".into() }],
        warnings: Vec::new(),
        min: [0.; 3],
        max: [1.; 3],
    });
    let (tx, rx) = mpsc::channel();
    let mut runtime = Runtime::default();
    runtime.docs.insert(id, DocPreview { generation: 1, ..Default::default() });
    runtime.discovery = Some(Discovery { id, generation: 1, rx, cancel: Arc::new(AtomicBool::new(false)) });
    let found = |complete| {
        Ok(Found {
            complete,
            candidates: Vec::new(),
            root: None,
            warnings: Vec::new(),
            objects: Vec::new(),
            loaded: Some(Loaded {
                model: model.clone(),
                textures: vec![None],
                selected: vec![true],
                visible: vec![true],
                path: "body.c3".into(),
                warnings: Vec::new(),
                pieces: Vec::new(),
                assets: Vec::new(),
                texture_slots: vec![None],
                object_id: None,
                title: "Body".into(),
                animations: Vec::new(),
                effects: Vec::new(),
                materials: vec![Default::default()],
                mix_slots: Vec::new(),
                mix_textures: Vec::new(),
            }),
        })
    };
    assert!(tx.send(found(false)).is_ok());
    runtime.poll();
    assert!(runtime.discovery.is_some());
    assert!(runtime.docs.get(&id).unwrap().pending_textures);
    let loaded = runtime.docs.get_mut(&id).unwrap().loaded.as_mut().unwrap();
    loaded.selected[0] = false;
    loaded.visible[0] = false;
    loaded.materials[0].scale = 2.5;
    assert!(tx.send(found(true)).is_ok());
    runtime.poll();
    assert!(runtime.discovery.is_none());
    assert!(!runtime.docs.get(&id).unwrap().pending_textures);
    let loaded = runtime.docs.get(&id).unwrap().loaded.as_ref().unwrap();
    assert!(Arc::ptr_eq(&loaded.model, &model));
    assert_eq!(loaded.selected, [false]);
    assert_eq!(loaded.visible, [false]);
    assert_eq!(loaded.materials[0].scale, 2.5);
}

#[test]
fn stale_mesh_progress_cannot_replace_a_newer_selection() {
    let id = DocId(42);
    let (tx, rx) = mpsc::channel();
    let mut runtime = Runtime::default();
    runtime.docs.insert(id, DocPreview { generation: 2, status: "new selection".into(), ..Default::default() });
    runtime.discovery = Some(Discovery { id, generation: 1, rx, cancel: Arc::new(AtomicBool::new(false)) });
    assert!(tx.send(Ok(Found { complete: true, candidates: Vec::new(), root: None, warnings: Vec::new(), objects: Vec::new(), loaded: None })).is_ok());
    runtime.poll();
    assert_eq!(runtime.docs.get(&id).unwrap().status, "new selection");
}
