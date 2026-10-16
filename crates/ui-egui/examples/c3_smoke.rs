//! Offscreen end-to-end check: c3_smoke <texture.dds> <output-directory> [replacement.c3].
//! Uses a local client/fixture; never changes the input file. Output is disposable.
use egui_kittest::kittest::Queryable;
use photocraft_ui_egui::{
    PhotocraftApp, Services,
    control::{self, ControlRequest, Outcome},
};
use serde_json::{Value, json};
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

fn invoke(app: &mut PhotocraftApp, ctx: &egui::Context, method: &str, params: Value) -> Result<Value, String> {
    let (request, _) = ControlRequest::new(method, params);
    match control::handle(app, ctx, &request) {
        Outcome::Done(value) if value.get("ok") == Some(&Value::Bool(true)) => Ok(value),
        Outcome::Done(value) => Err(value.to_string()),
        _ => Err("unexpected pending request".into()),
    }
}
macro_rules! settle {
    ($h:expr,$predicate:expr) => {{
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            $h.step();
            let inspected = control::inspect($h.state(), &$h.ctx);
            if ($predicate)(&inspected) {
                break inspected;
            }
            if Instant::now() > deadline {
                return Err(format!("C3 preview did not settle: {}", inspected["c3Preview"]).into());
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }};
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().collect();
    let texture = args.get(1).ok_or("pass a texture path")?.clone();
    let replacement = args.get(3).cloned();
    let dialog_path = replacement.clone();
    let output = PathBuf::from(args.get(2).ok_or("pass an output folder")?);
    std::fs::create_dir_all(&output)?;
    let mut harness = egui_kittest::Harness::builder().with_size(egui::vec2(1500., 960.)).with_pixels_per_point(1.).wgpu().build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        let services = Services {
            import: Some(Box::new(|name, bytes, _| photocraft_io::import(name, bytes).map(|r| (r.document, r.warnings)).map_err(|e| e.to_string()))),
            file_dialog: dialog_path.map(|path| {
                Box::new(move |_: photocraft_ui_egui::FileDialogRequest, _: Option<&eframe::Frame>, reply: photocraft_ui_egui::FileDialogReply| {
                    reply.send(Some(photocraft_ui_egui::FileDialogAnswer::Paths(vec![path.clone()])));
                }) as photocraft_ui_egui::FileDialogFn
            }),
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
    harness.run_steps(4);
    let inspected = settle!(harness, |v: &Value| !v["c3Preview"]["path"].is_null()
        && v["c3Preview"]["textureSize"].is_array()
        && v["c3Preview"]["loading"] == false
        && v["c3Preview"]["textureUpdating"] == false);
    let rect: &Vec<Value> = inspected["c3Preview"]["rect"].as_array().ok_or("no viewport rectangle")?;
    let coordinate = |index: usize| rect.get(index).and_then(Value::as_f64).map(|n| n as f32).ok_or("invalid viewport rectangle");
    let view = egui::Rect::from_min_max(egui::pos2(coordinate(0)?, coordinate(1)?), egui::pos2(coordinate(2)?, coordinate(3)?));
    let original = harness.render()?;
    original.save(output.join("original.png"))?;
    let baseline_revision = harness.state().session.active().ok_or("no document")?.revision;
    let ctx = harness.ctx.clone();
    // A held brush stroke must update the model before a history step commits.
    invoke(harness.state_mut(), &ctx, "ui.set", json!({"tool":"brush","brushSize":110}))?;
    harness.state_mut().run("tools.setColors", json!({"foreground":"#00ffff"}))?;
    invoke(harness.state_mut(), &ctx, "ui.pointer", json!({"events":[{"kind":"down","x":70,"y":100},{"kind":"move","x":190,"y":150}]}))?;
    let held = settle!(harness, |v: &Value| v["c3Preview"]["textureRevision"]["preview"].as_u64().is_some_and(|key| key > 0)
        && v["c3Preview"]["textureUpdating"] == false);
    if harness.state().session.active().ok_or("no document")?.revision != baseline_revision {
        return Err("held stroke committed too early".into());
    }
    let live = harness.render()?;
    live.save(output.join("live-stroke.png"))?;
    let in_view = |index: usize| {
        let width = original.width().max(1) as usize;
        let x = (index % width) as f32;
        let y = (index / width) as f32;
        view.contains(egui::pos2(x, y))
    };
    let live_changes = original.pixels().zip(live.pixels()).enumerate().filter(|(i, (a, b))| in_view(*i) && a != b).count();
    if live_changes < 100 {
        return Err(format!("held brush stroke changed only {live_changes} viewport pixels").into());
    }
    invoke(harness.state_mut(), &ctx, "ui.pointer", json!({"events":[{"kind":"up","x":190,"y":150}]}))?;
    let committed_revision = harness.state().session.active().ok_or("no document")?.revision;
    settle!(harness, |v: &Value| v["c3Preview"]["textureRevision"]["revision"].as_u64() == Some(committed_revision)
        && v["c3Preview"]["textureRevision"]["preview"] == 0
        && v["c3Preview"]["textureUpdating"] == false);
    harness.state_mut().run("edit.undo", json!({}))?;
    let undo_revision = harness.state().session.active().ok_or("no document")?.revision;
    settle!(harness, |v: &Value| v["c3Preview"]["textureRevision"]["revision"].as_u64() == Some(undo_revision) && v["c3Preview"]["textureUpdating"] == false);
    let undone = harness.render()?;
    undone.save(output.join("undo.png"))?;
    let undo_changes = original.pixels().zip(undone.pixels()).enumerate().filter(|(i, (a, b))| in_view(*i) && a != b).count();
    if undo_changes != 0 {
        return Err(format!("undo left {undo_changes} changed viewport pixels").into());
    }
    // Camera gestures must leave document content/history untouched.
    let start = view.center();
    let end = start + egui::vec2(75., -55.);
    harness.event(egui::Event::PointerMoved(start));
    harness.step();
    harness.event(egui::Event::PointerButton { pos: start, button: egui::PointerButton::Primary, pressed: true, modifiers: Default::default() });
    harness.step();
    harness.event(egui::Event::PointerMoved(end));
    harness.step();
    harness.event(egui::Event::PointerButton { pos: end, button: egui::PointerButton::Primary, pressed: false, modifiers: Default::default() });
    harness.run_steps(3);
    let camera = control::inspect(harness.state(), &ctx)["c3Preview"]["state"]["camera"].clone();
    if harness.state().ui.c3_preview.camera.rotation == [0., 0., 0., 1.] {
        return Err("viewport drag did not rotate the model".into());
    }
    if harness.state().session.active().ok_or("no document")?.revision != undo_revision {
        return Err("viewport gesture edited the texture".into());
    }
    harness.render()?.save(output.join("rotated.png"))?;
    harness.event(egui::Event::PointerButton { pos: end, button: egui::PointerButton::Secondary, pressed: true, modifiers: Default::default() });
    harness.step();
    harness.event(egui::Event::PointerButton { pos: end, button: egui::PointerButton::Secondary, pressed: false, modifiers: Default::default() });
    harness.run_steps(3);
    harness.render()?.save(output.join("context-menu.png"))?;
    if let Some(path) = replacement {
        harness.get_by_label("Load different C3 mesh…").click();
        settle!(harness, |v: &Value| v["c3Preview"]["path"].as_str() == Some(path.as_str()) && v["c3Preview"]["loading"] == false);
        if harness.state().session.active().ok_or("no document")?.revision != undo_revision {
            return Err("loading a different mesh edited the texture".into());
        }
        harness.render()?.save(output.join("replacement.png"))?;
    }
    let result = json!({"initial":inspected["c3Preview"],"held":held["c3Preview"],"liveChangedPixels":live_changes,"undoChangedPixels":undo_changes,"camera":camera,"final":control::inspect(harness.state(),&ctx)["c3Preview"]});
    std::fs::write(output.join("result.json"), serde_json::to_vec_pretty(&result)?)?;
    println!(
        "C3 smoke passed: {live_changes} viewport pixels updated during an uncommitted stroke; undo restored the model; drag rotated without editing the document."
    );
    Ok(())
}
