//! End-to-end GPU checks against constant-color client shader equations.
use super::*;
fn word(b: &mut Vec<u8>, v: u32) {
    b.extend(v.to_le_bytes());
}
fn floats(b: &mut Vec<u8>, v: &[f32]) {
    for f in v {
        b.extend(f.to_le_bytes());
    }
}
fn name(b: &mut Vec<u8>, v: &str) {
    word(b, v.len() as u32);
    b.extend(v.as_bytes());
}
fn chunk(b: &mut Vec<u8>, tag: &[u8; 4], payload: &[u8]) {
    b.extend(tag);
    word(b, payload.len() as u32);
    b.extend(payload);
}
fn fixture(root: &std::path::Path) -> Result<String, Box<dyn std::error::Error>> {
    std::fs::create_dir_all(root.join("ini"))?;
    std::fs::create_dir_all(root.join("c3/texture"))?;
    for (name, color) in [("a", [120, 100, 80, 255]), ("b", [60, 40, 20, 255]), ("c", [30, 20, 10, 255]), ("d", [10, 10, 10, 255])] {
        image::RgbaImage::from_pixel(16, 16, image::Rgba(color)).save(root.join(format!("c3/texture/{name}.tga")))?;
    }
    let mut geometry = Vec::new();
    name(&mut geometry, "v_body");
    for v in [0, 4, 0] {
        word(&mut geometry, v);
    }
    for (point, uv) in [[-1., 0., -1.], [1., 0., -1.], [-1., 0., 1.], [1., 0., 1.]].into_iter().zip([[0., 0.], [1., 0.], [0., 1.], [1., 1.]]) {
        floats(&mut geometry, &point);
        floats(&mut geometry, &uv);
        for v in [0, 0, 0] {
            word(&mut geometry, v);
        }
        floats(&mut geometry, &[1., 0.]);
        floats(&mut geometry, &[0., -1., 0.]);
    }
    word(&mut geometry, 2);
    word(&mut geometry, 0);
    // C3 exports use clockwise exterior triangles; author the reference
    // plane with that winding rather than compensating for a renderer bug.
    for i in [0u16, 2, 1, 2, 3, 1] {
        geometry.extend(i.to_le_bytes());
    }
    name(&mut geometry, "c3/texture/a.tga");
    floats(&mut geometry, &[0.; 6]);
    floats(&mut geometry, &photocraft_c3::math::IDENTITY);
    word(&mut geometry, 1);
    for v in [2, 0, 0.5f32.to_bits(), 0, 0, 1, 0, 0, 0, 0, 0] {
        word(&mut geometry, v);
    }
    let mut motion = Vec::new();
    word(&mut motion, 1);
    word(&mut motion, 2);
    floats(&mut motion, &photocraft_c3::math::IDENTITY);
    floats(&mut motion, &photocraft_c3::math::IDENTITY);
    word(&mut motion, 0);
    let mut bytes = b"MAXFILE C3 00001".to_vec();
    chunk(&mut bytes, b"PHY3", &geometry);
    chunk(&mut bytes, b"MOTI", &motion);
    let mut particle = Vec::new();
    name(&mut particle, "reference-sprite");
    name(&mut particle, "c3/texture/a.tga");
    for value in [1, 1, 1, 1] {
        word(&mut particle, value);
    }
    floats(&mut particle, &[0., 0., 0., 0., 1.]);
    floats(&mut particle, &photocraft_c3::math::IDENTITY);
    chunk(&mut bytes, b"PTCL", &particle);
    std::fs::write(root.join("c3/test.c3"), bytes)?;
    std::fs::write(root.join("ini/3deffectobj.ini"), "1=c3/test.c3\n")?;
    std::fs::write(root.join("ini/3dtexture.ini"), "1=c3/texture/a.tga\n2=c3/texture/b.tga\n3=c3/texture/c.tga\n4=c3/texture/d.tga\n")?;
    std::fs::write(
        root.join("ini/3deffect3.ini"),
        "[material]\nAmount=1\nEffectId0=1\nTextureId0=1\nTextureId0_1=2\nTextureId0_2=3\nTextureId0_3=4\nMixOpt0=28\nASB0=5\nADB0=1\nZBuffer0=1\nZTest0=1\n",
    )?;
    Ok(root.join("c3/texture/a.tga").to_string_lossy().into_owned())
}
pub fn run(output: PathBuf) -> Result<(), Box<dyn std::error::Error>> {
    std::fs::create_dir_all(&output)?;
    let texture = fixture(&output.join("client"))?;
    let mut harness = egui_kittest::Harness::builder().with_size(egui::vec2(1500., 1000.)).with_pixels_per_point(1.).wgpu().build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        let services = Services {
            import: Some(Box::new(|name, bytes, _| photocraft_io::import(name, bytes).map(|r| (r.document, r.warnings)).map_err(|e| e.to_string()))),
            ..Default::default()
        };
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
        app.ui.c3_preview.paused = true;
        if let Some(rs) = &cc.wgpu_render_state {
            app.set_wgpu(rs.clone());
        }
        if let Err(e) = app.open_path(&texture) {
            app.ui.status = e;
            app.ui.status_error = true;
        }
        app
    });
    let deadline = Instant::now() + Duration::from_secs(90);
    let scene = loop {
        harness.step();
        let v = control::inspect(harness.state(), &harness.ctx)["c3Preview"].clone();
        if v["loading"] == false && v["textureUpdating"] == false && v["textureSize"].is_array() {
            break v;
        }
        if Instant::now() > deadline {
            return Err(format!("fixture did not settle: {v}").into());
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    std::fs::write(output.join("scene.json"), serde_json::to_vec_pretty(&scene)?)?;
    let rect = scene["rect"].as_array().ok_or("no viewport")?;
    let f = |i| rect.get(i).and_then(Value::as_f64).ok_or("bad viewport coordinate");
    let x = ((f(0)? + f(2)?) * 0.5) as u32;
    let y = ((f(1)? + f(3)?) * 0.5) as u32;
    let ctx = harness.ctx.clone();
    let mut results = Vec::new();
    let parameters = [0.5, 0.2, 0.8, 2., 1., 0., 0., 0., 0., 0., 0., 0., 0., 0., 0., 0.];
    // Exact constant inputs from the behavioral equations, independent of mesh UVs.
    for (mode, expected) in [
        (8, [90, 70, 50]),
        (39, [60, 45, 30]),
        (40, [75, 55, 35]),
        (28, [90, 70, 50]),
        (30, [220, 170, 120]),
        (31, [64, 53, 42]),
        (32, [96, 29, 23]),
        (33, [120, 100, 80]),
        (35, [120, 90, 60]),
        (37, [180, 140, 100]),
        (38, [180, 140, 100]),
        (36, [120, 100, 80]),
    ] {
        invoke(
            harness.state_mut(),
            &ctx,
            "preview.c3.playback",
            json!({"material":{"piece":0,"mixMode":mode,"mixData":if matches!(mode,39|40) {[0.;16]}else{parameters}}}),
        )?;
        harness.run_steps(3);
        let image = harness.render()?;
        image.save(output.join(format!("mode-{mode}.png")))?;
        let pixel = image.get_pixel(x, y).0;
        results.push(json!({"mode":mode,"expected":expected,"actual":pixel}));
        std::fs::write(output.join("results.json"), serde_json::to_vec_pretty(&results)?)?;
        if pixel[..3].iter().zip(expected).any(|(a, b)| (i32::from(*a) - b).abs() > 2) {
            return Err(format!("mode {mode}: pixel {pixel:?}, expected {expected:?}").into());
        }
    }
    invoke(harness.state_mut(), &ctx, "preview.c3.playback", json!({"material":{"piece":0,"mixMode":30}}))?;
    // Isolate the sprite through visibility, independently of mesh winding.
    invoke(harness.state_mut(), &ctx, "preview.c3.playback", json!({"meshes":false}))?;
    harness.run_steps(3);
    let sprite = harness.render()?;
    let pixel = sprite.get_pixel(x, y).0;
    if pixel[..3].iter().zip([120, 100, 80]).any(|(a, b)| (i32::from(*a) - b).abs() > 2) {
        return Err(format!("sprite inherited the mesh shader: {pixel:?}").into());
    }
    invoke(harness.state_mut(), &ctx, "preview.c3.playback", json!({"meshes":true,"particles":false}))?;
    invoke(harness.state_mut(), &ctx, "preview.c3.playback", json!({"material":{"piece":0,"cull":3}}))?;
    harness.run_steps(3);
    let hidden = harness.render()?;
    let pixel = hidden.get_pixel(x, y).0;
    let background = hidden.get_pixel((f(0)? + 5.) as u32, (f(1)? + 5.) as u32).0;
    if pixel != background {
        return Err(format!("reversed culling did not hide the native-winding plane: {pixel:?} vs {background:?}").into());
    }
    invoke(harness.state_mut(), &ctx, "preview.c3.playback", json!({"material":{"piece":0,"cull":1}}))?;
    harness.run_steps(3);
    let visible = harness.render()?;
    let pixel = visible.get_pixel(x, y).0;
    if pixel[..3].iter().zip([220, 170, 120]).any(|(a, b)| (i32::from(*a) - b).abs() > 2) {
        return Err("two-sided override did not restore the reference plane".into());
    }
    invoke(harness.state_mut(), &ctx, "preview.c3.textures", json!({"index":2}))?;
    harness.run_steps(5);
    harness.state_mut().run("edit.fill", json!({"color":"#460a1e"}))?;
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        harness.step();
        let v = control::inspect(harness.state(), &ctx);
        if v["c3Preview"]["textureUpdating"] == false
            && v["c3Preview"]["textureRevision"]["revision"].as_u64() == harness.state().session.active().map(|s| s.revision)
        {
            break;
        }
        if Instant::now() > deadline {
            return Err("third texture did not update".into());
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    harness.run_steps(3);
    let edited = harness.render()?;
    edited.save(output.join("third-texture-edited.png"))?;
    let pixel = edited.get_pixel(x, y).0;
    if pixel[..3].iter().zip([255, 160, 140]).any(|(a, b)| (i32::from(*a) - b).abs() > 2) {
        return Err(format!("third texture live edit: pixel {pixel:?}").into());
    }
    harness.state_mut().run("edit.undo", json!({}))?;
    for _ in 0..30 {
        harness.step();
        std::thread::sleep(Duration::from_millis(10));
    }
    let image = harness.render()?;
    image.save(output.join("third-texture-undone.png"))?;
    let pixel = image.get_pixel(x, y).0;
    if pixel[..3].iter().zip([220, 170, 120]).any(|(a, b)| (i32::from(*a) - b).abs() > 2) {
        return Err(format!("third texture undo: pixel {pixel:?}").into());
    }
    invoke(harness.state_mut(), &ctx, "preview.c3.playback", json!({"time":0.033,"particles":false}))?;
    harness.run_steps(3);
    let zero = harness.render()?;
    let pixel = zero.get_pixel(x, y).0;
    let background = zero.get_pixel((f(0)? + 5.) as u32, (f(1)? + 5.) as u32).0;
    if pixel != background {
        return Err(format!("zero-alpha ordinary shader still rendered: {pixel:?} vs {background:?}").into());
    }
    invoke(harness.state_mut(), &ctx, "preview.c3.playback", json!({"material":{"piece":0,"mixMode":8}}))?;
    harness.run_steps(3);
    let zero = harness.render()?;
    let pixel = zero.get_pixel(x, y).0;
    if pixel[..3].iter().zip([60, 40, 20]).any(|(a, b)| (i32::from(*a) - b).abs() > 2) {
        return Err(format!("zero-alpha compositing shader hid its secondary layer: {pixel:?}").into());
    }
    println!("12 shader modes match reference color equations; third texture edits and undo update the GPU");
    Ok(())
}
