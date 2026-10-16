//! Compare single-sided rendering with a depth-tested two-sided surface oracle.
use photocraft_ui_egui::{
    PhotocraftApp, Services,
    control::{self, ControlRequest, Outcome},
};
use serde_json::{Value, json};
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

fn invoke(app: &mut PhotocraftApp, ctx: &egui::Context, params: Value) -> Result<(), String> {
    let (request, _) = ControlRequest::new("ui.menu.invoke", json!({"id":"preview.c3.playback", "params":params}));
    match control::handle(app, ctx, &request) {
        Outcome::Done(value) if value["ok"] == true => Ok(()),
        Outcome::Done(value) => Err(value.to_string()),
        _ => Err("unexpected pending command".into()),
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = std::env::args().collect::<Vec<_>>();
    let texture = args.get(1).ok_or("pass a client texture")?.clone();
    let output = PathBuf::from(args.get(2).ok_or("pass an analysis output directory")?);
    std::fs::create_dir_all(&output)?;
    let mut harness = egui_kittest::Harness::builder().with_size(egui::vec2(1500., 1000.)).with_pixels_per_point(1.).wgpu().build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        let services = Services {
            import: Some(Box::new(|name, bytes, _| photocraft_io::import(name, bytes).map(|r| (r.document, r.warnings)).map_err(|e| e.to_string()))),
            ..Default::default()
        };
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
        app.ui.c3_preview.paused = true;
        app.ui.c3_preview.alpha = 2;
        if let Some(rs) = &cc.wgpu_render_state {
            app.set_wgpu(rs.clone());
        }
        if let Err(error) = app.open_path(&texture) {
            app.ui.status = error;
            app.ui.status_error = true;
        }
        app
    });
    let deadline = Instant::now() + Duration::from_secs(90);
    let scene = loop {
        harness.step();
        let scene = control::inspect(harness.state(), &harness.ctx)["c3Preview"].clone();
        if scene["path"].is_string() && scene["loading"] == false && scene["textureUpdating"] == false && scene["textureSize"].is_array() {
            break scene;
        }
        if Instant::now() > deadline {
            return Err(format!("scene did not settle: {scene}").into());
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    std::fs::write(output.join("scene.json"), serde_json::to_vec_pretty(&scene)?)?;
    let pieces = scene["pieces"].as_array().ok_or("missing pieces")?.len();
    let rect = scene["rect"].as_array().ok_or("missing viewport")?;
    let number = |i| rect.get(i).and_then(Value::as_f64).ok_or("invalid viewport");
    let x = number(0)? as u32;
    let y = number(1)? as u32;
    let width = (number(2)? as u32).saturating_sub(x);
    let height = (number(3)? as u32).saturating_sub(y);
    let ctx = harness.ctx.clone();
    invoke(harness.state_mut(), &ctx, json!({"effects":false, "time":0., "paused":true}))?;
    let mut results = Vec::new();
    for axis in 0..4 {
        for view in ["front", "back", "left", "right", "top", "bottom"] {
            harness.state_mut().ui.c3_preview.up_axis = axis;
            harness.state_mut().ui.c3_preview.camera.preset(view);
            harness.state_mut().ui.c3_preview.camera.distance = 2.0;
            let mut frames = Vec::new();
            for cull in [0, 1, 2, 3] {
                for piece in 0..pieces {
                    invoke(harness.state_mut(), &ctx, json!({"material":{"piece":piece,"cull":cull}}))?;
                }
                harness.run_steps(3);
                let frame = harness.render()?;
                let crop = image::imageops::crop_imm(&frame, x, y, width, height).to_image();
                crop.save(output.join(format!("axis-{axis}-{view}-cull-{cull}.png")))?;
                frames.push(crop);
            }
            let reference = frames.get(1).ok_or("missing two-sided oracle")?;
            let differences = frames.iter().map(|frame| frame.pixels().zip(reference.pixels()).filter(|(a, b)| a != b).count()).collect::<Vec<_>>();
            results.push(json!({"axis":axis,"view":view,"differentFromTwoSided":differences}));
        }
    }
    std::fs::write(output.join("results.json"), serde_json::to_vec_pretty(&results)?)?;
    if args.get(3).map(String::as_str) == Some("--expect-exterior") {
        let total = |mode| results.iter().filter_map(|row| row["differentFromTwoSided"].get(mode).and_then(Value::as_u64)).sum::<u64>();
        if total(3) < 1000 || total(0).saturating_mul(4) > total(3) {
            return Err(format!("default culling disagrees with the exterior oracle: default {}, reversed {}", total(0), total(3)).into());
        }
    }
    println!("{} surface/camera cases captured", results.len());
    Ok(())
}
