#[path = "c3_effects_smoke/materials.rs"]
mod materials;
// Offscreen native effect rendering: <client texture> <output directory>.
use photocraft_ui_egui::{
    PhotocraftApp, Services,
    control::{self, ControlRequest, Outcome},
};
use serde_json::{Value, json};
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

fn invoke(app: &mut PhotocraftApp, ctx: &egui::Context, command: &str, params: Value) -> Result<Value, String> {
    let (request, _) = ControlRequest::new("ui.menu.invoke", json!({"id":command,"params":params}));
    match control::handle(app, ctx, &request) {
        Outcome::Done(value) if value["ok"] == true => Ok(value),
        Outcome::Done(value) => Err(value.to_string()),
        _ => Err("unexpected pending command".into()),
    }
}
fn changes(a: &image::RgbaImage, b: &image::RgbaImage, rect: egui::Rect) -> usize {
    let width = a.width().max(1) as usize;
    a.pixels()
        .zip(b.pixels())
        .enumerate()
        .filter(|(index, (a, b))| a != b && rect.contains(egui::pos2((*index % width) as f32, (*index / width) as f32)))
        .count()
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = std::env::args().collect::<Vec<_>>();
    if args.get(1).map(String::as_str) == Some("--materials") {
        return materials::run(PathBuf::from(args.get(2).ok_or("pass an output directory")?));
    }
    let texture = args.get(1).ok_or("pass a client texture")?.clone();
    let output = PathBuf::from(args.get(2).ok_or("pass an output directory")?);
    let client_root = args.get(3).cloned();
    let up_axis = args.get(4).map(|value| value.parse::<u8>()).transpose()?.unwrap_or(0);
    if up_axis > 3 {
        return Err("up axis must be 0..3".into());
    }
    let static_effect = args.get(5).map(String::as_str) == Some("--static");
    std::fs::create_dir_all(&output)?;
    let mut harness = egui_kittest::Harness::builder().with_size(egui::vec2(1500., 1000.)).with_pixels_per_point(1.).wgpu().build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        let services = Services {
            import: Some(Box::new(|name, bytes, _| photocraft_io::import(name, bytes).map(|r| (r.document, r.warnings)).map_err(|e| e.to_string()))),
            ..Default::default()
        };
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
        app.ui.c3_preview.paused = true;
        app.ui.c3_preview.up_axis = up_axis;
        app.ui.c3_preview.client_root = client_root.clone();
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
    let inspected = loop {
        harness.step();
        let inspected = control::inspect(harness.state(), &harness.ctx);
        if inspected["c3Preview"]["path"].is_string()
            && inspected["c3Preview"]["loading"] == false
            && inspected["c3Preview"]["textureUpdating"] == false
            && inspected["c3Preview"]["textureSize"].is_array()
        {
            break inspected;
        }
        if Instant::now() > deadline {
            return Err(format!("scene did not settle: {}", inspected["c3Preview"]).into());
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    std::fs::write(output.join("scene.json"), serde_json::to_vec_pretty(&inspected["c3Preview"])?)?;
    if inspected["c3Preview"]["effects"].as_array().is_none_or(|v| v.is_empty()) {
        return Err("no attached effects were discovered".into());
    }
    let rect = inspected["c3Preview"]["rect"].as_array().ok_or("missing preview rectangle")?;
    let value = |index| rect.get(index).and_then(Value::as_f64).map(|v| v as f32).ok_or("bad rectangle");
    let rect = egui::Rect::from_min_max(egui::pos2(value(0)?, value(1)?), egui::pos2(value(2)?, value(3)?));
    let ctx = harness.ctx.clone();
    let revision = harness.state().session.active().ok_or("no document")?.revision;
    invoke(harness.state_mut(), &ctx, "preview.c3.playback", json!({"time":0.37,"paused":true}))?;
    harness.run_steps(3);
    let first = harness.render()?;
    first.save(output.join("effect-frame-a.png"))?;
    harness.run_steps(12);
    let paused = harness.render()?;
    let paused_changes = changes(&first, &paused, rect);
    if paused_changes != 0 {
        return Err(format!("paused viewport changed {paused_changes} pixels").into());
    }
    invoke(harness.state_mut(), &ctx, "preview.c3.playback", json!({"time":0.91}))?;
    harness.run_steps(3);
    let second = harness.render()?;
    second.save(output.join("effect-frame-b.png"))?;
    let animated_changes = changes(&first, &second, rect);
    if !static_effect && animated_changes < 20 {
        return Err(format!("animation changed only {animated_changes} pixels").into());
    }
    if static_effect && animated_changes != 0 {
        return Err(format!("static client glow changed {animated_changes} pixels").into());
    }
    invoke(harness.state_mut(), &ctx, "preview.c3.playback", json!({"effects":false}))?;
    harness.run_steps(3);
    let base = harness.render()?;
    base.save(output.join("weapon-only.png"))?;
    let hidden_changes = changes(&second, &base, rect);
    if hidden_changes < 20 {
        return Err(format!("hiding effects changed only {hidden_changes} pixels").into());
    }
    invoke(harness.state_mut(), &ctx, "preview.c3.reset", json!({}))?;
    harness.run_steps(4);
    let fitted = harness.render()?;
    fitted.save(output.join("weapon-only-fitted.png"))?;
    harness.run_steps(8);
    if changes(&fitted, &harness.render()?, rect) != 0 {
        return Err("paused fitted viewport changed pixels".into());
    }
    invoke(harness.state_mut(), &ctx, "preview.c3.settings", json!({"zoom":10}))?;
    harness.run_steps(4);
    let close = harness.render()?;
    close.save(output.join("zoom-10x.png"))?;
    let zoom_changes = changes(&fitted, &close, rect);
    if zoom_changes < 20 {
        return Err(format!("close zoom changed only {zoom_changes} pixels").into());
    }
    invoke(harness.state_mut(), &ctx, "preview.c3.settings", json!({"zoom":1000}))?;
    harness.run_steps(4);
    harness.render()?.save(output.join("zoom-1000x.png"))?;
    invoke(harness.state_mut(), &ctx, "preview.c3.playback", json!({"effects":true}))?;
    invoke(harness.state_mut(), &ctx, "preview.c3.reset", json!({"fit":"model"}))?;
    harness.run_steps(4);
    harness.render()?.save(output.join("fit-model-with-effects.png"))?;
    if !harness.state().ui.c3_preview.effects_enabled {
        return Err("Fit model hid effects".into());
    }
    invoke(harness.state_mut(), &ctx, "preview.c3.playback", json!({"effects":false}))?;
    harness.run_steps(4);
    let model_fit = harness.render()?;
    model_fit.save(output.join("fit-model-base-only.png"))?;
    let fit_changes = changes(&fitted, &model_fit, rect);
    if fit_changes != 0 {
        return Err(format!("base framing included attached effects: {fit_changes} changed pixels").into());
    }
    invoke(harness.state_mut(), &ctx, "preview.c3.playback", json!({"effects":true,"base":false}))?;
    harness.run_steps(3);
    harness.render()?.save(output.join("effects-only.png"))?;
    if harness.state().session.active().ok_or("no document")?.revision != revision {
        return Err("playback controls changed document history".into());
    }
    std::fs::write(
        output.join("results.json"),
        serde_json::to_vec_pretty(
            &json!({"animatedPixels":animated_changes,"expectedMotion":!static_effect,"hiddenEffectPixels":hidden_changes,"pausedPixels":paused_changes,"historyPreserved":true,"zoomPixels":zoom_changes,"fitModelMatchesBaseOnly":fit_changes==0,"maxZoom":1000}),
        )?,
    )?;
    println!("animation: {animated_changes} changed pixels; effects hidden: {hidden_changes}; pause: {paused_changes}; document history preserved");
    Ok(())
}
