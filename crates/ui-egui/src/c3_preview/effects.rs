//! Scene playback and effect visibility are view state, independent of documents.
use super::*;
use photocraft_c3::{
    animation::{AnimatedModel, SampledMesh},
    material::{Material, Timing},
    math,
};

#[derive(Clone)]
pub(crate) struct SceneAnimation {
    pub asset: Arc<AnimatedModel>,
    pub range: std::ops::Range<usize>,
    pub timing: Timing,
    pub material: Material,
    pub effect: Option<usize>,
    pub parent: Option<(usize, String)>,
}
#[derive(Clone)]
pub(crate) struct EffectGroup {
    pub key: String,
    pub label: String,
    pub socket: String,
    pub source: String,
    pub visible: bool,
    pub note: Option<String>,
}

pub(super) fn synchronize_durations(animations: &mut [SceneAnimation]) {
    let mut durations = std::collections::HashMap::<usize, u64>::new();
    for animation in animations.iter() {
        if let Some(group) = animation.effect {
            let duration = animation.timing.part_duration(animation.asset.frame_count);
            let value = durations.entry(group).or_default();
            *value = (*value).max(duration);
        }
    }
    for animation in animations {
        animation.timing.duration_ms = animation.effect.and_then(|group| durations.get(&group)).copied().unwrap_or(0);
    }
}

pub(super) fn visibility(loaded: &Loaded, state: &State) -> Vec<bool> {
    let mut visible = workspace::effective_visibility(loaded);
    for animation in &loaded.animations {
        let group = animation.effect.and_then(|index| loaded.effects.get(index)).is_none_or(|group| group.visible);
        for index in animation.range.clone() {
            let variant = loaded.model.meshes.get(index).map(|mesh| mesh.variant.as_str()).unwrap_or_default();
            let effect = animation.effect.is_some() || matches!(variant, "Particles" | "Trail");
            let category = match variant {
                "Particles" => state.particles,
                "Trail" => state.trails,
                _ => animation.effect.is_none() || state.effect_meshes,
            };
            let enabled = if effect { state.effects_enabled && group } else { state.base_visible };
            if let Some(slot) = visible.get_mut(index) {
                *slot &= enabled && category;
            }
        }
    }
    visible
}

pub(super) fn sample(loaded: &Loaded, seconds: f64, view: photocraft_c3::animation::View, seed: u32, visible: &[bool]) -> Vec<(usize, SampledMesh)> {
    let mut out = Vec::new();
    for animation in &loaded.animations {
        let mask = animation
            .range
            .clone()
            .enumerate()
            .fold(0u128, |mask, (local, index)| if visible.get(index) == Some(&true) { mask | 1u128.checked_shl(local as u32).unwrap_or(0) } else { mask });
        if mask == 0 {
            continue;
        }
        let Some(frame) = animation.timing.frame(seconds, animation.asset.frame_count) else {
            out.extend(animation.range.clone().map(|index| {
                (
                    index,
                    SampledMesh {
                        vertices: Vec::new(),
                        indices: Vec::new(),
                        alpha: 0.,
                        visible: false,
                        colors: Vec::new(),
                        uv1: Vec::new(),
                        uv_steps: [[0.; 2]; 3],
                    },
                )
            }));
            continue;
        };
        let parent = |at: u32| {
            let socket = animation
                .parent
                .as_ref()
                .and_then(|(index, name)| {
                    loaded.animations.get(*index).and_then(|parent| {
                        let age = f64::from(frame.saturating_sub(at)) * f64::from(animation.timing.frame_ms) * 0.001;
                        let time = (seconds - age).max(0.);
                        let frame = parent.timing.frame(time, parent.asset.frame_count).unwrap_or(0);
                        let socket = parent.asset.socket(name, frame)?;
                        let mut world = math::IDENTITY;
                        for (index, value) in
                            [0, 5, 10].into_iter().map(|index| (index, parent.material.scale)).chain([12, 13, 14].into_iter().zip(parent.timing.offset))
                        {
                            if let Some(slot) = world.get_mut(index) {
                                *slot = value;
                            }
                        }
                        Some(math::multiply(&socket, &world))
                    })
                })
                .unwrap_or(math::IDENTITY);
            let scale = animation.material.scale;
            let mut local = math::IDENTITY;
            for index in [0, 5, 10] {
                if let Some(slot) = local.get_mut(index) {
                    *slot = scale;
                }
            }
            for (index, value) in [12, 13, 14].into_iter().zip(animation.timing.offset) {
                if let Some(slot) = local.get_mut(index) {
                    *slot = value;
                }
            }
            math::multiply(&local, &socket)
        };
        out.extend(animation.range.clone().zip(animation.asset.sample_in_view(
            frame,
            animation.timing.step(seconds),
            seconds * 1000.,
            photocraft_c3::animation::View {
                effect_billboard: animation.material.billboard,
                visible: mask,
                texture_size: loaded.textures.get(animation.range.start).and_then(Option::as_ref).map(|image| image.size).unwrap_or([0; 2]),
                ..view
            },
            &parent,
            seed,
        )));
    }
    out
}

pub(super) fn clock(app: &mut PhotocraftApp, ctx: &egui::Context, owner: Option<DocId>) {
    let now = ctx.input(|i| i.time);
    for (id, preview) in &mut app.c3_preview.docs {
        let elapsed = preview.last_time.map(|previous| (now - previous).max(0.)).unwrap_or(0.);
        preview.last_time = Some(now);
        if !app.ui.c3_preview.paused {
            preview.seconds = (preview.seconds + elapsed * app.ui.c3_preview.speed).min(86_400.);
            if owner == Some(*id)
                && app.ui.c3_preview.enabled
                && preview.loaded.as_ref().is_some_and(|loaded| {
                    let visible = visibility(loaded, &app.ui.c3_preview);
                    loaded
                        .animations
                        .iter()
                        .any(|animation| animation.asset.is_animated() && animation.range.clone().any(|index| visible.get(index) == Some(&true)))
                })
            {
                ctx.request_repaint_after(std::time::Duration::from_millis(16));
            }
        }
    }
}

pub(super) fn controls(app: &mut PhotocraftApp, ui: &mut egui::Ui, owner: Option<DocId>) {
    ui.horizontal_wrapped(|ui| {
        ui.checkbox(&mut app.ui.c3_preview.effects_enabled, "Effects");
        ui.checkbox(&mut app.ui.c3_preview.base_visible, "Base object");
        let paused = app.ui.c3_preview.paused;
        if ui.button(if paused { "Play" } else { "Pause" }).clicked() {
            app.ui.c3_preview.paused = !paused;
            for preview in app.c3_preview.docs.values_mut() {
                preview.last_time = None;
            }
        }
        if ui.small_button("Restart").clicked()
            && let Some(preview) = owner.and_then(|id| app.c3_preview.docs.get_mut(&id))
        {
            preview.seconds = 0.;
        }
    });
    ui.collapsing("Animation and rendering", |ui| {
        ui.horizontal_wrapped(|ui| {
            ui.checkbox(&mut app.ui.c3_preview.effect_meshes, "Effect meshes");
            ui.checkbox(&mut app.ui.c3_preview.particles, "Particles");
            ui.checkbox(&mut app.ui.c3_preview.trails, "Trails");
        });
        ui.horizontal(|ui| {
            ui.label("Speed");
            ui.add(egui::DragValue::new(&mut app.ui.c3_preview.speed).range(0.05..=8.).speed(0.05).suffix("x"));
            if ui.small_button("Step").clicked()
                && let Some(preview) = owner.and_then(|id| app.c3_preview.docs.get_mut(&id))
            {
                app.ui.c3_preview.paused = true;
                preview.seconds += 0.033;
            }
        });
        if let Some(preview) = owner.and_then(|id| app.c3_preview.docs.get_mut(&id)) {
            ui.horizontal(|ui| {
                ui.label("Time");
                if ui.add(egui::DragValue::new(&mut preview.seconds).range(0.0..=86_400.).speed(0.01).suffix(" s")).changed() {
                    app.ui.c3_preview.paused = true;
                }
            });
        }
        ui.checkbox(&mut app.ui.c3_preview.client_materials, "Client effect materials");
        ui.horizontal(|ui| {
            ui.label("Background");
            ui.color_edit_button_rgb(&mut app.ui.c3_preview.background);
        });
        ui.horizontal(|ui| {
            ui.label("Particle seed");
            ui.add(egui::DragValue::new(&mut app.ui.c3_preview.seed));
        });
    });
}

pub(super) fn command(app: &mut PhotocraftApp, params: &Value) -> Result<Value, String> {
    let object = params.as_object().ok_or_else(|| "Playback settings must be an object".to_string())?;
    if let Some(key) = object.keys().find(|key| {
        !matches!(
            key.as_str(),
            "paused"
                | "speed"
                | "time"
                | "step"
                | "restart"
                | "effects"
                | "base"
                | "meshes"
                | "particles"
                | "trails"
                | "clientMaterials"
                | "seed"
                | "group"
                | "visible"
                | "background"
                | "material"
        )
    }) {
        return Err(format!("Unknown playback setting: {key}"));
    }
    if object.contains_key("material") {
        if object.len() != 1 {
            return Err("Apply piece material settings separately from playback settings".into());
        }
        return material_command(app, params.get("material").unwrap_or(&Value::Null));
    }
    let mut state = app.ui.c3_preview.clone();
    for (key, target) in [
        ("paused", &mut state.paused),
        ("effects", &mut state.effects_enabled),
        ("base", &mut state.base_visible),
        ("meshes", &mut state.effect_meshes),
        ("particles", &mut state.particles),
        ("trails", &mut state.trails),
        ("clientMaterials", &mut state.client_materials),
    ] {
        if let Some(value) = boolean(params, key)? {
            *target = value;
        }
    }
    if let Some(speed) = params.get("speed") {
        state.speed = speed.as_f64().filter(|v| v.is_finite() && (0.05..=8.).contains(v)).ok_or("Speed must be from 0.05 to 8")?;
    }
    if let Some(seed) = params.get("seed") {
        state.seed = seed.as_u64().and_then(|n| u32::try_from(n).ok()).ok_or("Seed must be a 32-bit unsigned integer")?;
    }
    if let Some(value) = params.get("background") {
        state.background = serde_json::from_value(value.clone()).map_err(|_| "Background must be three values from 0 to 1")?;
        if state.background.iter().any(|v| !v.is_finite() || !(0.0..=1.0).contains(v)) {
            return Err("Background must be three values from 0 to 1".into());
        }
    }
    let time = params
        .get("time")
        .map(|v| v.as_f64().filter(|n| n.is_finite() && (0.0..=86_400.).contains(n)).ok_or("Time must be from 0 to 86400 seconds"))
        .transpose()?;
    let step = params.get("step").map(|v| v.as_f64().filter(|n| n.is_finite() && n.abs() <= 60.).ok_or("Step must be within 60 seconds")).transpose()?;
    let restart = boolean(params, "restart")?;
    let visible = boolean(params, "visible")?;
    let group = params.get("group").map(|v| v.as_u64().and_then(|n| usize::try_from(n).ok()).ok_or("Group must be an effect index")).transpose()?;
    if visible.is_some() && group.is_none() {
        return Err("Specify an effect group index".into());
    }
    let owner = workspace::scene_id(app);
    let preview = owner.and_then(|id| app.c3_preview.docs.get_mut(&id));
    if group.is_some() || time.is_some() || step.is_some() || restart == Some(true) {
        let preview = preview.ok_or("Load a C3 scene first")?;
        if let Some(group) = group {
            let target = preview.loaded.as_mut().and_then(|loaded| loaded.effects.get_mut(group)).ok_or("Effect group index is out of range")?;
            target.visible = visible.ok_or("Specify effect group visibility")?;
        }
        if restart == Some(true) {
            preview.seconds = 0.;
        }
        if let Some(time) = time {
            preview.seconds = time;
            state.paused = boolean(params, "paused")?.unwrap_or(true);
        }
        if let Some(step) = step {
            preview.seconds = (preview.seconds + step).clamp(0., 86_400.);
            state.paused = boolean(params, "paused")?.unwrap_or(true);
        }
    } else if visible.is_some() {
        return Err("Specify an effect group index".into());
    }
    app.ui.c3_preview = state;
    for preview in app.c3_preview.docs.values_mut() {
        preview.last_time = None;
    }
    Ok(json!({"state":app.ui.c3_preview}))
}

fn material_command(app: &mut PhotocraftApp, params: &Value) -> Result<Value, String> {
    let object = params.as_object().ok_or("Material settings must be an object")?;
    if let Some(key) = object.keys().find(|key| {
        !matches!(
            key.as_str(),
            "piece"
                | "src"
                | "dst"
                | "depthWrite"
                | "depthTest"
                | "scale"
                | "offset"
                | "frameMs"
                | "delayMs"
                | "loops"
                | "loopIntervalMs"
                | "mixMode"
                | "mixData"
                | "billboard"
                | "frameOffset"
                | "loopOnce"
                | "manualUVStep"
                | "cull"
                | "uvStep"
        )
    }) {
        return Err(format!("Unknown material setting: {key}"));
    }
    let piece = params.get("piece").and_then(Value::as_u64).and_then(|v| usize::try_from(v).ok()).ok_or("Specify a piece index")?;
    let owner = workspace::scene_id(app).ok_or("Load a C3 scene first")?;
    let loaded = app.c3_preview.docs.get(&owner).and_then(|p| p.loaded.as_ref()).ok_or("Load a C3 scene first")?;
    let piece = loaded.pieces.get(piece).ok_or("Piece index is out of range")?;
    let index = loaded.animations.iter().position(|animation| animation.range == piece.range).ok_or("Piece has no animation or material")?;
    let animation = loaded.animations.get(index).ok_or("Piece animation is unavailable")?;
    let mut material = animation.material.clone();
    let mut timing = animation.timing.clone();
    if let Some(value) = params.get("frameOffset") {
        timing.frame_offset = value.as_i64().and_then(|v| i32::try_from(v).ok()).ok_or("Frame offset must be a signed 32-bit integer")?;
    }
    if let Some(value) = boolean(params, "loopOnce")? {
        timing.loop_once = value;
    }
    if let Some(value) = boolean(params, "manualUVStep")? {
        timing.manual_uv_step = value;
    }
    for (key, target, min, max) in [
        ("uvStep", &mut timing.manual_step, 0, u32::MAX),
        ("src", &mut material.blend[0], 1, 15),
        ("mixMode", &mut material.mix_mode, 0, 255),
        ("frameMs", &mut timing.frame_ms, 1, 60_000),
        ("delayMs", &mut timing.delay_ms, 0, 86_400_000),
        ("loops", &mut timing.loops, 0, u32::MAX),
        ("loopIntervalMs", &mut timing.loop_interval_ms, 0, 86_400_000),
    ] {
        if let Some(value) = params.get(key) {
            *target = value.as_u64().and_then(|v| u32::try_from(v).ok()).filter(|v| (*v >= min) && (*v <= max)).ok_or_else(|| format!("Invalid {key}"))?;
        }
    }
    if let Some(value) = params.get("dst") {
        material.blend[1] = value.as_u64().and_then(|v| u32::try_from(v).ok()).filter(|v| (1..=15).contains(v)).ok_or("Invalid dst blend factor")?;
    }
    if let Some(value) = params.get("mixData") {
        material.mix_data = serde_json::from_value(value.clone()).map_err(|_| "MixData must contain sixteen finite numbers")?;
        if material.mix_data.iter().any(|v| !v.is_finite() || v.abs() > 1.0e6) {
            return Err("MixData values must be within 1000000".into());
        }
    }
    if let Some(value) = params.get("billboard") {
        material.billboard = value.as_u64().filter(|v| *v <= 3).ok_or("Billboard mode must be from 0 to 3")? as u8;
    }
    if let Some(value) = params.get("cull") {
        material.cull = value.as_u64().filter(|v| *v <= 3).ok_or("Cull mode must be from 0 to 3")? as u8;
    }
    for (key, target) in [("depthWrite", &mut material.depth_write), ("depthTest", &mut material.depth_test)] {
        if let Some(value) = boolean(params, key)? {
            *target = value;
        }
    }
    if let Some(value) = params.get("scale") {
        material.scale = value.as_f64().filter(|v| v.is_finite() && v.abs() <= 1000.).ok_or("Scale must be within 1000")? as f32;
    }
    if let Some(value) = params.get("offset") {
        timing.offset = serde_json::from_value(value.clone()).map_err(|_| "Offset must be three finite values")?;
        if timing.offset.iter().any(|v| !v.is_finite() || v.abs() > 1.0e6) {
            return Err("Offset must be within 1000000".into());
        }
    }
    let loaded = app.c3_preview.docs.get_mut(&owner).and_then(|p| p.loaded.as_mut()).ok_or("Scene is no longer loaded")?;
    let animation = loaded.animations.get_mut(index).ok_or("Piece animation is no longer loaded")?;
    animation.material = material.clone();
    animation.timing = timing;
    for index in animation.range.clone() {
        if let Some(slot) = loaded.materials.get_mut(index) {
            *slot = material.clone();
        }
    }
    synchronize_durations(&mut loaded.animations);
    if let Some(gpu) = &mut app.c3_preview.gpu {
        gpu.invalidate_animation();
    }
    Ok(json!({"ok":true}))
}

pub(super) fn material_controls(app: &mut PhotocraftApp, ui: &mut egui::Ui, owner: DocId, piece: usize) {
    let animation = app
        .c3_preview
        .docs
        .get(&owner)
        .and_then(|p| p.loaded.as_ref())
        .and_then(|loaded| loaded.pieces.get(piece).and_then(|piece| loaded.animations.iter().find(|animation| animation.range == piece.range)))
        .cloned();
    let Some(animation) = animation else {
        return;
    };
    ui.collapsing("Material and timing",|ui| {
        let mut material=animation.material;let mut timing=animation.timing;let mut changed=false;
        ui.horizontal_wrapped(|ui| {
            changed|=ui.checkbox(&mut material.depth_test,"Depth test").changed();changed|=ui.checkbox(&mut material.depth_write,"Mesh writes depth").changed();
        });
        ui.horizontal(|ui| {
            ui.label("Blend");
            for (id,value) in material.blend.iter_mut().enumerate() {
                egui::ComboBox::from_id_salt(("blend",id)).width(105.).selected_text(blend_label(*value)).show_ui(ui,|ui| {for candidate in 1..=15 {changed|=ui.selectable_value(value,candidate,blend_label(candidate)).changed();}});
            }
        });
        ui.horizontal(|ui| {ui.label("Scale");changed|=ui.add(egui::DragValue::new(&mut material.scale).range(-1000.0..=1000.0).speed(0.01)).changed();});
        ui.horizontal(|ui| {ui.label("Cull");egui::ComboBox::from_id_salt(("cull",piece)).selected_text(match material.cull {1=>"Two-sided",2=>"Clockwise",3=>"Counterclockwise",_=>"Client flag"}).show_ui(ui,|ui| {for (value,label) in [(0,"Client flag"),(1,"Two-sided"),(2,"Clockwise"),(3,"Counterclockwise")] {changed|=ui.selectable_value(&mut material.cull,value,label).changed();}});});
        ui.horizontal(|ui| {ui.label("Offset");for value in &mut timing.offset {changed|=ui.add(egui::DragValue::new(value).range(-1.0e6..=1.0e6).speed(0.1)).changed();}});
        ui.horizontal(|ui| {ui.label("Frame (ms)");changed|=ui.add(egui::DragValue::new(&mut timing.frame_ms).range(1..=60_000)).changed();});
        ui.horizontal(|ui| {ui.label("Start frame offset");changed|=ui.add(egui::DragValue::new(&mut timing.frame_offset)).changed();});
        ui.horizontal_wrapped(|ui| {changed|=ui.checkbox(&mut timing.loop_once,"Play piece once per cycle").changed();changed|=ui.checkbox(&mut timing.manual_uv_step,"Manual UV stepping").changed();});
        ui.horizontal(|ui| {ui.label("Delay (ms)");changed|=ui.add(egui::DragValue::new(&mut timing.delay_ms).range(0..=86_400_000)).changed();});
        ui.horizontal(|ui| {ui.label("Loop gap (ms)");changed|=ui.add(egui::DragValue::new(&mut timing.loop_interval_ms).range(0..=86_400_000)).changed();});
        ui.horizontal(|ui| {ui.label("Cycle count");changed|=ui.add(egui::DragValue::new(&mut timing.loops)).changed();});
        if timing.manual_uv_step {ui.horizontal(|ui| {ui.label("UV step");changed|=ui.add(egui::DragValue::new(&mut timing.manual_step)).changed();});}
        ui.horizontal(|ui| {ui.label("Mesh MixOpt");changed|=ui.add(egui::DragValue::new(&mut material.mix_mode).range(0..=255)).changed();});
        ui.horizontal(|ui| {ui.label("Billboard");egui::ComboBox::from_id_salt(("billboard",piece)).selected_text(match material.billboard {1=>"Spherical",2=>"World up",3=>"Client mode 3",_=>"None"}).show_ui(ui,|ui| {for (value,label) in [(0,"None"),(1,"Spherical"),(2,"World up"),(3,"Client mode 3")] {changed|=ui.selectable_value(&mut material.billboard,value,label).changed();}});});
        ui.collapsing("Shader parameters",|ui| {egui::Grid::new(("shader-parameters",piece)).show(ui,|ui| {for (index,value) in material.mix_data.iter_mut().enumerate() {if index%4==0 {ui.label(format!("Param {}",index/4));}changed|=ui.add(egui::DragValue::new(value).range(-1.0e6..=1.0e6).speed(0.01)).changed();if index%4==3 {ui.end_row();}}});});
        ui.label(egui::RichText::new(format!("Client MixOpt {} · {} animation frames",material.mix_mode,animation.asset.frame_count)).small());
        if changed {let result=crate::menus::invoke(app,ui.ctx(),"preview.c3.playback",json!({"material":{"piece":piece,"src":material.blend[0],"dst":material.blend[1],"depthTest":material.depth_test,"depthWrite":material.depth_write,"scale":material.scale,"offset":timing.offset,"frameMs":timing.frame_ms,"delayMs":timing.delay_ms,"loopIntervalMs":timing.loop_interval_ms,"mixMode":material.mix_mode,"mixData":material.mix_data,"billboard":material.billboard,"frameOffset":timing.frame_offset,"loopOnce":timing.loop_once,"manualUVStep":timing.manual_uv_step,"cull":material.cull,"uvStep":timing.manual_step,"loops":timing.loops}}));super::report(app,result);}
    });
}
fn blend_label(value: u32) -> &'static str {
    match value {
        1 => "Zero",
        2 => "One",
        3 => "Source color",
        4 => "Inverse source",
        5 => "Source alpha",
        6 => "Inverse alpha",
        7 => "Dest. alpha",
        8 => "Inv. dest. alpha",
        9 => "Dest. color",
        10 => "Inv. dest. color",
        11 => "Alpha saturate",
        12 => "Both source alpha",
        13 => "Both inverse alpha",
        14 => "Blend constant",
        15 => "Inverse constant",
        _ => "Unknown",
    }
}
