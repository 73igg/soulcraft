use super::*;
use photocraft_doc::{ColorMode, SampleType};
use photocraft_geom::Size;

fn workspace_app() -> (PhotocraftApp, DocId, DocId) {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.ui.c3_preview.automatic = false;
    for name in ["body.dds", "trim.dds", "unrelated.dds"] {
        app.session.add_document(Document::new(name, Size::new(2, 2), ColorMode::Rgb, SampleType::U8), None);
    }
    let owner = app.session.documents()[0].doc.id;
    let trim = app.session.documents()[1].doc.id;
    app.session.set_active(0);
    app.sync_views();
    let meshes = (0..3)
        .map(|i| photocraft_c3::Mesh { name: format!("mesh{i}"), texture: String::new(), vertices: Vec::new(), indices: Vec::new(), variant: "PHY4".into() })
        .collect();
    let loaded = Loaded {
        model: Arc::new(photocraft_c3::Model { meshes, warnings: Vec::new(), min: [0.; 3], max: [1.; 3] }),
        textures: vec![None; 3],
        selected: vec![true, true, false],
        visible: vec![true; 3],
        path: "body.c3".into(),
        warnings: Vec::new(),
        pieces: vec![
            Piece { key: "body".into(), path: "body.c3".into(), range: 0..2, visible: true, note: None, textures: vec![0] },
            Piece { key: "trim".into(), path: "trim.c3".into(), range: 2..3, visible: true, note: None, textures: vec![1] },
        ],
        assets: ["body.dds", "trim.dds"]
            .into_iter()
            .enumerate()
            .map(|(i, name)| TextureAsset { key: name.into(), path: None, bytes: None, source: i == 0, note: None })
            .collect(),
        texture_slots: vec![Some(0), Some(0), Some(1)],
        object_id: Some("armor.ini:outfit".into()),
        title: "Outfit".into(),
        animations: Vec::new(),
        effects: Vec::new(),
        materials: Vec::new(),
        mix_slots: Vec::new(),
        mix_textures: Vec::new(),
    };
    app.c3_preview.docs.insert(owner, DocPreview { source: "body.dds".into(), loaded: Some(loaded), ..Default::default() });
    workspace::sync_documents(&mut app);
    (app, owner, trim)
}

fn invoke(app: &mut PhotocraftApp, id: &str, params: Value) -> Result<Value, String> {
    menu(app, &egui::Context::default(), id, &params).unwrap()
}

fn with_effect(app: &mut PhotocraftApp, owner: DocId) {
    let mut payload = Vec::new();
    for name in ["emitter", "test.dds"] {
        payload.extend((name.len() as u32).to_le_bytes());
        payload.extend(name.as_bytes());
    }
    for value in [1u32, 1, 2, 0, 0] {
        payload.extend(value.to_le_bytes());
    }
    let mut bytes = b"MAXFILE C3 00001".to_vec();
    bytes.extend(b"PTCL");
    bytes.extend((payload.len() as u32).to_le_bytes());
    bytes.extend(payload);
    let asset = Arc::new(photocraft_c3::animation::AnimatedModel::decode(&bytes).unwrap());
    let loaded = app.c3_preview.docs.get_mut(&owner).unwrap().loaded.as_mut().unwrap();
    loaded.effects = vec![effects::EffectGroup {
        key: "super".into(),
        label: "Super weapon".into(),
        socket: "v_r_weapon".into(),
        source: "test catalog".into(),
        visible: true,
        note: None,
    }];
    loaded.animations = vec![
        effects::SceneAnimation { asset: asset.clone(), range: 0..2, material: Default::default(), timing: Default::default(), effect: None, parent: None },
        effects::SceneAnimation { asset, range: 2..3, material: Default::default(), timing: Default::default(), effect: Some(0), parent: None },
    ];
    loaded.materials = vec![Default::default(); 3];
}

#[test]
fn embedded_particles_follow_the_effect_switch_independently_of_the_base_mesh() {
    let (mut app, owner, _) = workspace_app();
    with_effect(&mut app, owner);
    let loaded = app.c3_preview.docs.get_mut(&owner).unwrap().loaded.as_mut().unwrap();
    loaded.animations[1].effect = None;
    Arc::make_mut(&mut loaded.model).meshes[2].variant = "Particles".into();
    app.ui.c3_preview.effects_enabled = false;
    assert_eq!(effects::visibility(loaded, &app.ui.c3_preview), [true, true, false]);
    app.ui.c3_preview.effects_enabled = true;
    app.ui.c3_preview.base_visible = false;
    assert_eq!(effects::visibility(loaded, &app.ui.c3_preview), [false, false, true]);
}

#[test]
fn effect_switches_playback_and_piece_materials_are_independent_view_state() {
    let (mut app, owner, _) = workspace_app();
    with_effect(&mut app, owner);
    let revision = app.session.active().unwrap().revision;
    invoke(&mut app, "preview.c3.playback", json!({"time":1.25,"paused":false,"speed":2.0})).unwrap();
    assert!(!app.ui.c3_preview.paused);
    assert_eq!(app.c3_preview.docs[&owner].seconds, 1.25);
    invoke(&mut app, "preview.c3.playback", json!({"paused":true,"effects":false})).unwrap();
    assert!(app.ui.c3_preview.paused);
    assert_eq!(effects::visibility(app.c3_preview.docs[&owner].loaded.as_ref().unwrap(), &app.ui.c3_preview), [true, true, false]);
    invoke(&mut app, "preview.c3.playback", json!({"effects":true,"base":false})).unwrap();
    assert_eq!(effects::visibility(app.c3_preview.docs[&owner].loaded.as_ref().unwrap(), &app.ui.c3_preview), [false, false, true]);
    invoke(&mut app, "preview.c3.playback", json!({"group":0,"visible":false})).unwrap();
    assert_eq!(effects::visibility(app.c3_preview.docs[&owner].loaded.as_ref().unwrap(), &app.ui.c3_preview), [false; 3]);
    invoke(&mut app, "preview.c3.playback", json!({"material":{"piece":1,"src":2,"dst":2,"depthWrite":false,"scale":2.5,"offset":[1.,2.,3.],"frameMs":40}}))
        .unwrap();
    let loaded = app.c3_preview.docs[&owner].loaded.as_ref().unwrap();
    assert_eq!(loaded.materials[2].blend, [2, 2]);
    assert_eq!(loaded.materials[0].blend, [5, 6]);
    assert_eq!(loaded.animations[1].timing.offset, [1., 2., 3.]);
    assert_eq!(loaded.animations[1].material.scale, 2.5);
    for params in [
        json!({"time":-1}),
        json!({"speed":0}),
        json!({"group":999,"visible":true}),
        json!({"time":3,"visible":true}),
        json!({"material":{"piece":1,"src":0,"scale":5.}}),
        json!({"material":{"piece":999,"src":1}}),
        json!({"material":{"piece":1,"offset":[1,2]}}),
        json!({"material":{"piece":1,"scale":8,"cull":4}}),
        json!({"material":{"piece":1,"uvStep":-1}}),
        json!({"material":{"piece":1,"frameOffset":"no"}}),
        json!({"material":{"piece":1,"loopOnce":"yes"}}),
        json!({"paused":"yes"}),
        json!({"seed":-1}),
    ] {
        assert!(invoke(&mut app, "preview.c3.playback", params).is_err());
    }
    assert_eq!(app.c3_preview.docs[&owner].loaded.as_ref().unwrap().animations[1].material.scale, 2.5);
    assert_eq!(app.session.active().unwrap().revision, revision);
}

#[test]
fn gallery_reuses_documents_and_hides_tiles_without_closing_or_editing_them() {
    let (mut app, owner, trim) = workspace_app();
    app.session.execute("layer.new.layer", json!({"name":"Unsaved work"})).unwrap();
    let revisions = app.session.documents().iter().map(|state| state.revision).collect::<Vec<_>>();
    let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800., 600.));
    invoke(&mut app, "preview.c3.textures", json!({"all":true})).unwrap();
    assert_eq!(app.session.documents().len(), 3);
    assert_eq!(app.session.active().unwrap().doc.id, trim);
    assert_eq!(app.c3_preview.owner(trim), owner);
    assert_eq!(workspace::arranged_cells(&app, rect).unwrap().iter().map(|(i, _)| *i).collect::<Vec<_>>(), [0, 1]);
    invoke(&mut app, "preview.c3.textures", json!({"index":0,"visible":false})).unwrap();
    assert_eq!(workspace::arranged_cells(&app, rect).unwrap().len(), 1);
    assert_eq!(app.session.documents().iter().map(|state| state.revision).collect::<Vec<_>>(), revisions);
    invoke(&mut app, "preview.c3.textures", json!({"single":true})).unwrap();
    assert!(workspace::arranged_cells(&app, rect).is_none());
    assert!(app.session.documents()[0].is_dirty());
}

#[test]
fn piece_visibility_and_solo_preserve_individual_mesh_choices_and_document_history() {
    let (mut app, owner, _) = workspace_app();
    let before = app.session.active().unwrap().revision;
    invoke(&mut app, "preview.c3.part", json!({"index":0,"visible":false})).unwrap();
    invoke(&mut app, "preview.c3.piece", json!({"index":1,"solo":true})).unwrap();
    let loaded = app.c3_preview.docs[&owner].loaded.as_ref().unwrap();
    assert_eq!(workspace::effective_visibility(loaded), [false, false, true]);
    invoke(&mut app, "preview.c3.piece", json!({"all":true})).unwrap();
    assert_eq!(workspace::effective_visibility(app.c3_preview.docs[&owner].loaded.as_ref().unwrap()), [false, true, true]);
    for params in [json!({"index":999,"all":false}), json!({"solo":true}), json!({"index":-1}), json!({"visible":"yes"}), json!({"bogus":true})] {
        assert!(invoke(&mut app, "preview.c3.piece", params).is_err());
    }
    assert!(app.c3_preview.docs[&owner].loaded.as_ref().unwrap().pieces.iter().all(|part| part.visible));
    assert_eq!(app.session.active().unwrap().revision, before);
}

#[test]
fn linked_texture_switches_share_camera_and_survive_closing_the_original_document() {
    let (mut app, owner, trim) = workspace_app();
    let ctx = egui::Context::default();
    upkeep(&mut app, &ctx);
    app.ui.c3_preview.camera.preset("back");
    let camera = app.ui.c3_preview.camera.clone();
    invoke(&mut app, "preview.c3.textures", json!({"index":1})).unwrap();
    upkeep(&mut app, &ctx);
    assert!(visible(&app));
    assert_eq!(app.ui.c3_preview.camera, camera);
    assert_eq!(app.c3_preview.owner(trim), owner);
    app.session.close(0).unwrap();
    upkeep(&mut app, &ctx);
    assert!(app.c3_preview.docs[&owner].loaded.is_some());
    assert_eq!(workspace::scene_id(&app), Some(owner));
    assert!(!app.c3_preview.docs[&owner].texture_docs.contains_key(&0));
    assert_eq!(app.c3_preview.docs[&owner].texture_docs[&1], trim);
}

#[test]
fn hiding_the_viewport_keeps_texture_linking_and_does_not_start_unrelated_lookups() {
    let (mut app, owner, trim) = workspace_app();
    app.ui.c3_preview.enabled = false;
    app.c3_preview.settings_loaded = true;
    app.c3_preview.owners.clear();
    app.c3_preview.docs.get_mut(&owner).unwrap().texture_docs.remove(&1);
    app.session.set_active(1);
    let ctx = egui::Context::default();
    ctx.run_ui(Default::default(), |ui| show(&mut app, ui)).textures_delta.clear();
    assert_eq!(app.c3_preview.owner(trim), owner);
    assert!(app.c3_preview.rect.is_none());
    app.ui.c3_preview.automatic = true;
    app.session.set_active(2);
    let unrelated = app.session.active().unwrap().doc.id;
    ctx.run_ui(Default::default(), |ui| show(&mut app, ui)).textures_delta.clear();
    assert!(!app.c3_preview.docs.contains_key(&unrelated));
    assert!(app.c3_preview.discovery.is_none());
}

#[test]
fn independent_scenes_never_form_an_owner_cycle_and_save_as_keeps_the_binding() {
    let (mut app, owner, trim) = workspace_app();
    app.session.set_active(1);
    app.session.active_mut().unwrap().path = Some("C:/new-skin/renamed.dds".into());
    workspace::sync_documents(&mut app);
    assert_eq!(app.c3_preview.docs[&owner].loaded.as_ref().unwrap().assets[1].path.as_deref(), Some("C:/new-skin/renamed.dds"));
    // A texture that independently owns a scene retains that scene. Other scenes
    // may still read its live texture without making the ownership graph cyclic.
    let loaded = app.c3_preview.docs[&owner].loaded.clone();
    app.c3_preview.docs.insert(trim, DocPreview { loaded, ..Default::default() });
    workspace::sync_documents(&mut app);
    assert_eq!(app.c3_preview.owner(trim), trim);
    assert_eq!(app.c3_preview.owner(app.c3_preview.owner(owner)), app.c3_preview.owner(owner));
}

#[test]
fn late_disk_textures_preserve_the_saved_material_after_its_document_closes() {
    let (mut app, owner, trim) = workspace_app();
    let saved = Arc::new(Image { size: [1, 1], pixels: vec![0, 255, 0, 255] });
    let revision = app.session.documents()[1].revision;
    app.c3_preview.docs.insert(
        trim,
        DocPreview {
            image: Some(saved.clone()),
            key: Some(TextureKey { id: trim, revision, preview: 0, display: 0, layer: None, targa: false }),
            ..Default::default()
        },
    );
    workspace::sync_documents(&mut app);
    assert!(app.c3_preview.docs[&owner].saved_textures.contains_key(&1));
    app.session.close(1).unwrap();
    workspace::sync_documents(&mut app);
    let mut late = app.c3_preview.docs[&owner].loaded.clone().unwrap();
    late.textures.fill(None);
    let (tx, rx) = mpsc::channel();
    app.c3_preview.discovery = Some(Discovery { id: owner, generation: 0, rx, cancel: Arc::new(AtomicBool::new(false)) });
    tx.send(Ok(Found { complete: true, candidates: Vec::new(), root: None, warnings: Vec::new(), loaded: Some(late), objects: Vec::new() })).unwrap();
    app.c3_preview.poll();
    assert!(Arc::ptr_eq(app.c3_preview.docs[&owner].loaded.as_ref().unwrap().textures[2].as_ref().unwrap(), &saved));
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn ini_assembly_shares_textures_and_preserves_missing_piece_rows() {
    use std::{fs, path::PathBuf};
    struct Folder(PathBuf);
    impl Drop for Folder {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    let root = std::env::var_os("C3_TEST_ARTIFACT_DIR").map(PathBuf::from).unwrap_or_else(std::env::temp_dir).join(format!("scene-{}", std::process::id()));
    let folder = Folder(root);
    for dir in ["ini", "c3/mesh", "c3/texture"] {
        fs::create_dir_all(folder.0.join(dir)).unwrap();
    }
    fs::write(folder.0.join("ini/3dobj.ini"), "1=c3/mesh/body.c3\n2=c3/mesh/trim.c3\n3=c3/mesh/missing.c3\n").unwrap();
    fs::write(folder.0.join("ini/3dtexture.ini"), "10=c3/texture/body.png\n20=c3/texture/trim.png\n").unwrap();
    fs::write(folder.0.join("ini/armor.ini"), "[outfit]\nMesh0=1\nTexture0=10\nMesh1=2\nTexture1=20\nMesh2=2\nTexture2=20\nMesh3=3\nTexture3=20\n").unwrap();
    for name in ["body", "trim"] {
        fs::write(folder.0.join(format!("c3/mesh/{name}.c3")), triangle()).unwrap();
        image::RgbaImage::from_pixel(2, 2, image::Rgba([255, 0, 0, 255])).save(folder.0.join(format!("c3/texture/{name}.png"))).unwrap();
    }
    let source = folder.0.join("c3/texture/body.png").to_string_lossy().into_owned();
    let mut geometry_first = false;
    let found = scene_worker::discover(&source, None, None, None, &AtomicBool::new(false), |found| {
        geometry_first = found.loaded.is_some_and(|loaded| loaded.model.meshes.len() == 3 && loaded.textures.iter().all(Option::is_none));
    })
    .unwrap();
    assert!(geometry_first);
    let scene = found.loaded.unwrap();
    assert_eq!(scene.pieces.len(), 4);
    assert_eq!(scene.assets.len(), 2);
    assert_eq!(scene.texture_slots, [Some(0), Some(1), Some(1)]);
    assert_eq!(scene.selected, [true, false, false]);
    assert!(Arc::ptr_eq(scene.textures[1].as_ref().unwrap(), scene.textures[2].as_ref().unwrap()));
    assert!(scene.pieces[3].range.is_empty());
    assert!(scene.pieces[3].note.is_some());
    assert_eq!(scene.pieces[3].textures, [1]);
    assert_eq!(scene.object_id.as_deref(), Some("armor.ini:outfit"));
}

#[cfg(not(target_arch = "wasm32"))]
fn triangle() -> Vec<u8> {
    triangle_with_texture(b"obsolete-exporter-texture.dds")
}

#[cfg(not(target_arch = "wasm32"))]
fn triangle_with_texture(texture: &[u8]) -> Vec<u8> {
    fn u(out: &mut Vec<u8>, value: u32) {
        out.extend(value.to_le_bytes());
    }
    fn f(out: &mut Vec<u8>, values: &[f32]) {
        for value in values {
            out.extend(value.to_le_bytes());
        }
    }
    fn name(out: &mut Vec<u8>, value: &[u8]) {
        u(out, value.len() as u32);
        out.extend(value);
    }
    let mut mesh = Vec::new();
    name(&mut mesh, b"v_body");
    for value in [2, 3, 0] {
        u(&mut mesh, value);
    }
    for position in [[0., 0., 0.], [1., 0., 0.], [0., 0., 1.]] {
        f(&mut mesh, &position);
        f(&mut mesh, &[0., 0.]);
        for value in [u32::MAX, 0, 1] {
            u(&mut mesh, value);
        }
        f(&mut mesh, &[1., 0.]);
    }
    u(&mut mesh, 1);
    u(&mut mesh, 0);
    for value in [0u16, 1, 2] {
        mesh.extend(value.to_le_bytes());
    }
    name(&mut mesh, texture);
    f(&mut mesh, &[0., 0., 0., 1., 0., 1.]);
    f(&mut mesh, &photocraft_c3::math::IDENTITY);
    for value in [1, 0, 0, 0] {
        u(&mut mesh, value);
    }
    let mut file = b"MAXFILE C3 00001".to_vec();
    file.extend(b"PHY4");
    u(&mut file, mesh.len() as u32);
    file.extend(mesh);
    file
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn missing_effect_texture_uses_exact_adjacent_image_without_overriding_a_registered_texture() {
    use std::fs;
    let root = std::env::temp_dir().join(format!("scene-sidecar-{}", std::process::id()));
    for dir in ["ini", "c3/mesh", "c3/texture"] {
        fs::create_dir_all(root.join(dir)).unwrap();
    }
    fs::write(root.join("ini/3dobj.ini"), "1=c3/mesh/body.c3\n").unwrap();
    fs::write(root.join("ini/3deffectobj.ini"), "2=c3/mesh/glow.c3\n3=c3/mesh/registered.c3\n").unwrap();
    fs::write(root.join("ini/3dtexture.ini"), "10=c3/texture/body.png\n20=c3/texture/registered.png\n").unwrap();
    fs::write(root.join("ini/weapon.ini"), "[510034]\nPart=1\nMesh0=1\nTexture0=10\n").unwrap();
    fs::write(root.join("ini/Action3DEffect.ini"), "9999.9999.510.034=glow\n").unwrap();
    fs::write(root.join("ini/3deffect.ini"), "[glow]\nAmount=2\nEffectId0=2\nTextureId0=999\nEffectId1=3\nTextureId1=20\n").unwrap();
    for name in ["body", "glow", "registered"] {
        fs::write(root.join(format!("c3/mesh/{name}.c3")), triangle_with_texture(b"")).unwrap();
    }
    fs::write(root.join("c3/mesh/glow.c3"), triangle()).unwrap();
    let red = image::RgbaImage::from_pixel(2, 2, image::Rgba([255, 0, 0, 255]));
    let green = image::RgbaImage::from_pixel(2, 2, image::Rgba([0, 255, 0, 255]));
    red.save(root.join("c3/texture/body.png")).unwrap();
    red.save(root.join("c3/texture/registered.png")).unwrap();
    green.save(root.join("c3/mesh/glow.tga")).unwrap();
    green.save(root.join("c3/mesh/registered.tga")).unwrap();
    let source = root.join("c3/texture/body.png").to_string_lossy().into_owned();
    let scene = scene_worker::discover(&source, None, None, None, &AtomicBool::new(false), |_| {}).unwrap().loaded.unwrap();
    assert_eq!(scene.texture_slots, [Some(0), Some(1), Some(2)]);
    assert_eq!(scene.assets[1].key, "c3/mesh/glow.tga");
    assert_eq!(scene.assets[2].key, "c3/texture/registered.png");
    assert_eq!(&scene.textures[1].as_ref().unwrap().pixels[..4], &[0, 255, 0, 255]);
    assert_eq!(&scene.textures[2].as_ref().unwrap().pixels[..4], &[255, 0, 0, 255]);
    assert!(scene.warnings.iter().any(|note| note.contains("using adjacent mesh texture c3/mesh/glow.tga")));
    fs::remove_dir_all(root).unwrap();
}
