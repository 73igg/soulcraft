//! A linked scene uses ordinary documents: each texture keeps its file layout,
//! history and save prompts. Hiding a tile never closes or discards a document.
use super::*;
use crate::PhotocraftApp;
use std::collections::{HashMap, HashSet};

pub(super) fn scene_id(app: &PhotocraftApp) -> Option<DocId> {
    app.session.active().map(|state| app.c3_preview.owner(state.doc.id))
}
fn normal(path: &str) -> String {
    path.replace('\\', "/").to_lowercase()
}
fn logical(path: &str, root: Option<&str>) -> String {
    let path = normal(path).trim_start_matches("//?/").to_string();
    let root = root.map(|root| normal(root).trim_start_matches("//?/").trim_end_matches('/').to_string());
    let key = root.as_ref().and_then(|root| path.strip_prefix(&(root.clone() + "/"))).unwrap_or(&path);
    key.strip_prefix("decomp/").or_else(|| key.strip_prefix("c3.extracted/")).unwrap_or(key).to_string()
}

pub(super) fn sync_documents(app: &mut PhotocraftApp) {
    let documents = app
        .session
        .documents()
        .iter()
        .map(|state| (state.doc.id, state.path.clone(), state.doc.name.clone(), state.is_dirty(), state.revision))
        .collect::<Vec<_>>();
    let open = documents.iter().map(|(id, ..)| *id).collect::<HashSet<_>>();
    let images = app.c3_preview.docs.iter().filter_map(|(id, preview)| Some((*id, (preview.image.clone()?, preview.key?)))).collect::<HashMap<_, _>>();
    app.c3_preview.owners.clear();
    let scenes = app.c3_preview.docs.iter().filter_map(|(id, preview)| preview.loaded.is_some().then_some(*id)).collect::<HashSet<_>>();
    let mut aliases = Vec::new();
    let mut forgotten = Vec::new();
    for (owner, preview) in &mut app.c3_preview.docs {
        let Some(loaded) = &mut preview.loaded else { continue };
        let closed = preview.texture_docs.iter().filter(|(_, id)| !open.contains(id)).map(|(asset, id)| (*asset, *id)).collect::<Vec<_>>();
        for (asset, id) in closed {
            preview.texture_docs.remove(&asset);
            if app.c3_preview.active == Some(id) {
                loaded.selected.fill(false);
                app.c3_preview.edited_doc = None;
            }
            forgotten.push((*owner, asset));
        }
        for (index, asset) in loaded.assets.iter_mut().enumerate() {
            if !preview.texture_docs.contains_key(&index) {
                let existing = documents.iter().find(|(id, path, name, ..)| {
                    (asset.source
                        && (Some(*id) == preview.source_doc
                            || (*id == *owner && (path.as_deref() == Some(preview.source.as_str()) || name == &preview.source))))
                        || match path {
                            Some(path) => {
                                asset.path.as_ref().is_some_and(|expected| logical(path, None) == logical(expected, None))
                                    || logical(path, preview.root.as_deref()) == asset.key
                            }
                            None => name == &asset.key,
                        }
                });
                if let Some((id, ..)) = existing {
                    preview.texture_docs.insert(index, *id);
                    preview.opening_textures.remove(&index);
                }
            }
            if let Some(id) = preview.texture_docs.get(&index).copied() {
                if id != *owner && !scenes.contains(&id) {
                    aliases.push((id, *owner));
                }
                if let Some((_, path, ..)) = documents.iter().find(|(open, ..)| *open == id)
                    && path.is_some()
                {
                    asset.path = path.clone();
                }
                if let Some((_, _, _, dirty, revision)) = documents.iter().find(|(open, ..)| *open == id)
                    && !dirty
                    && preview.saved_textures.get(&index).map(|(id, revision, _)| (*id, *revision)) != Some((id, *revision))
                    && let Some((image, key)) = images.get(&id).filter(|(_, key)| key.revision == *revision && key.preview == 0 && key.layer.is_none())
                {
                    let _ = key;
                    for (slot, target) in loaded.texture_slots.iter().zip(&mut loaded.textures) {
                        if *slot == Some(index) {
                            *target = Some(image.clone());
                        }
                    }
                    for (slots, targets) in loaded.mix_slots.iter().zip(&mut loaded.mix_textures) {
                        for (slot, target) in slots.iter().zip(targets) {
                            if *slot == Some(index) {
                                *target = Some(image.clone());
                            }
                        }
                    }
                    preview.saved_textures.insert(index, (id, *revision, image.clone()));
                }
            }
            if preview.opening_textures.contains(&index) && !app.jobs.opens.iter().any(|job| job.name == asset.key || job.path.as_ref() == asset.path.as_ref())
            {
                // A failed/cancelled importer must be retryable; normal open notices explain it.
                preview.opening_textures.remove(&index);
            }
        }
    }
    aliases.sort_by_key(|(id, owner)| (id.0, owner.0));
    for (id, owner) in aliases {
        app.c3_preview.owners.entry(id).or_insert(owner);
    }
    for (owner, asset) in forgotten {
        if app.c3_preview.gpu_doc == Some(owner)
            && let Some(loaded) = app.c3_preview.docs.get(&owner).and_then(|preview| preview.loaded.as_ref())
            && let Some(gpu) = &mut app.c3_preview.gpu
        {
            gpu.forget_texture(asset, &loaded.textures, &loaded.mix_textures);
        }
    }
}

pub(super) fn upload_document(runtime: &mut Runtime, id: DocId) {
    let owner = runtime.owner(id);
    if runtime.gpu_doc != Some(owner) {
        return;
    }
    let Some((image, key)) = runtime.docs.get(&id).and_then(|preview| Some((preview.image.clone()?, preview.key?))) else { return };
    let Some(preview) = runtime.docs.get_mut(&owner) else { return };
    let Some(loaded) = &mut preview.loaded else { return };
    let asset = preview.texture_docs.iter().find(|(_, doc)| **doc == id).map(|(index, _)| *index);
    let changed = runtime.uploaded_keys.get(&id) != Some(&key);
    if let Some(gpu) = &mut runtime.gpu {
        if let Some(asset) = asset {
            if changed {
                let slots = loaded.texture_slots.iter().enumerate().filter_map(|(index, slot)| (*slot == Some(asset)).then_some(index)).collect::<Vec<_>>();
                let mix = loaded
                    .mix_slots
                    .iter()
                    .enumerate()
                    .flat_map(|(index, slots)| slots.iter().enumerate().filter_map(move |(stage, slot)| (*slot == Some(asset)).then_some((index, stage))))
                    .collect::<Vec<_>>();
                gpu.upload_slots(asset, &slots, &mix, &image);
            }
            if runtime.active == Some(id) {
                if runtime.edited_doc != Some(id) && (loaded.object_id.is_some() || !loaded.assets.get(asset).is_some_and(|asset| asset.source)) {
                    for (selected, slot) in loaded.selected.iter_mut().zip(&loaded.texture_slots) {
                        *selected = *slot == Some(asset);
                    }
                }
                gpu.activate_texture(asset);
                runtime.edited_doc = Some(id);
            }
        } else if runtime.active == Some(id) && (changed || runtime.edited_doc != Some(id)) {
            if loaded.object_id.is_some() {
                loaded.selected.fill(false);
            }
            gpu.upload(&image);
            runtime.edited_doc = Some(id);
        }
    }
    if asset.is_some() || loaded.assets.is_empty() {
        runtime.uploaded_keys.insert(id, key);
    }
}

fn activate(app: &mut PhotocraftApp, id: DocId) -> Result<Value, String> {
    let index = app.session.documents().iter().position(|state| state.doc.id == id).ok_or_else(|| "Texture document was closed".to_string())?;
    app.session.set_active(index);
    Ok(json!({"document":id.0}))
}

pub(super) fn texture_pixels(asset: &TextureAsset) -> Option<u64> {
    let bytes = asset.bytes.as_ref()?;
    if let Ok((_, _, surfaces)) = photocraft_codecs::dds::inspect(bytes) {
        return surfaces.iter().try_fold(0u64, |sum, surface| sum.checked_add(u64::from(surface.width) * u64::from(surface.height)));
    }
    let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes.as_slice())).with_guessed_format().ok()?;
    // Targa has no magic signature, so image's format guesser cannot recognize
    // it. Use the bounded codec header probe before checking the editing budget.
    if reader.format().is_none() && photocraft_codecs::detect(bytes) == Some(photocraft_codecs::Format::Tga) {
        reader.set_format(image::ImageFormat::Tga);
    }
    let (width, height) = reader.into_dimensions().ok()?;
    Some(u64::from(width) * u64::from(height))
}

fn open_texture(app: &mut PhotocraftApp, owner: DocId, index: usize) -> Result<Value, String> {
    sync_documents(app);
    if let Some(id) = app.c3_preview.docs.get(&owner).and_then(|preview| preview.texture_docs.get(&index)).copied() {
        return activate(app, id);
    }
    let preview = app.c3_preview.docs.get(&owner).ok_or_else(|| "Object workspace was closed".to_string())?;
    if preview.opening_textures.contains(&index) {
        return Ok(json!({"opening":true}));
    }
    let asset = preview.loaded.as_ref().and_then(|loaded| loaded.assets.get(index)).cloned().ok_or_else(|| "Texture index is out of range".to_string())?;
    let bytes = asset.bytes.as_ref().ok_or_else(|| asset.note.clone().unwrap_or_else(|| "Texture data is still loading or unavailable".into()))?;
    // Include pending opens, so Show All cannot queue dozens of individually
    // legal DDS files whose combined editable surfaces exhaust memory.
    let pixels = texture_pixels(&asset).ok_or_else(|| "Texture dimensions could not be inspected".to_string())?;
    let used = preview
        .loaded
        .as_ref()
        .map(|loaded| {
            loaded
                .assets
                .iter()
                .enumerate()
                .filter(|(index, _)| preview.texture_docs.contains_key(index) || preview.opening_textures.contains(index))
                .filter_map(|(_, asset)| texture_pixels(asset))
                .fold(0u64, u64::saturating_add)
        })
        .unwrap_or(0);
    if used.saturating_add(pixels) > 64 * 1024 * 1024 {
        return Err("Linked textures exceed the 64-million-pixel editing budget; open larger textures individually with File > Open".into());
    }
    if let Some(path) = &asset.path {
        app.open_path(path)?;
    } else {
        app.open_bytes_from(&asset.key, bytes, None)?;
    }
    if let Some(preview) = app.c3_preview.docs.get_mut(&owner) {
        preview.opening_textures.insert(index);
    }
    sync_documents(app);
    Ok(json!({"opening":true}))
}

pub(super) fn command(app: &mut PhotocraftApp, ctx: &egui::Context, id: &str, params: &Value) -> Option<Result<Value, String>> {
    if !matches!(id, "preview.c3.object" | "preview.c3.piece" | "preview.c3.textures") {
        return None;
    }
    Some((|| {
        let owner = scene_id(app).ok_or_else(|| "Open an object texture first".to_string())?;
        let object = params.as_object().ok_or_else(|| "Object preview parameters must be an object".to_string())?;
        let allowed: &[&str] = match id {
            "preview.c3.object" => &["id"],
            "preview.c3.piece" => &["index", "visible", "solo", "all"],
            _ => &["index", "visible", "all", "single"],
        };
        if let Some(key) = object.keys().find(|key| !allowed.contains(&key.as_str())) {
            return Err(format!("Unknown object-preview parameter: {key}"));
        }
        if id == "preview.c3.object" {
            let object =
                params.get("id").and_then(Value::as_str).filter(|id| id.len() <= 4096).ok_or_else(|| "id must be an INI object identifier".to_string())?;
            let preview = app.c3_preview.docs.get(&owner).ok_or_else(|| "Load an object first".to_string())?;
            let root = preview.root.clone().or_else(|| app.ui.c3_preview.client_root.clone());
            let (source_doc, source) = app
                .session
                .active()
                .map(|state| (state.doc.id, state.path.clone().unwrap_or_else(|| state.doc.name.clone())))
                .ok_or_else(|| "Open a texture first".to_string())?;
            app.c3_preview.discover_choice(owner, source, root, None, Some(object.into()), ctx);
            if let Some(preview) = app.c3_preview.docs.get_mut(&owner) {
                preview.source_doc = Some(source_doc);
            }
            return Ok(json!({"loading":true}));
        }
        let index = match params.get("index") {
            None => None,
            Some(value) => Some(value.as_u64().and_then(|value| usize::try_from(value).ok()).ok_or_else(|| "index must be a nonnegative index".to_string())?),
        };
        let visible = boolean(params, "visible")?;
        let all = boolean(params, "all")?;
        let solo = boolean(params, "solo")?;
        let single = boolean(params, "single")?;
        let preview = app.c3_preview.docs.get_mut(&owner).ok_or_else(|| "Load an object first".to_string())?;
        let loaded = preview.loaded.as_mut().ok_or_else(|| "Load an object first".to_string())?;
        if id == "preview.c3.piece" {
            if let Some(index) = index.filter(|index| *index >= loaded.pieces.len()) {
                return Err(format!("C3 piece {index} is out of range"));
            }
            if solo == Some(true) && index.is_none() {
                return Err("Solo requires a piece index".into());
            }
            if let Some(all) = all {
                for piece in &mut loaded.pieces {
                    piece.visible = all;
                }
            }
            if solo == Some(true) {
                for (piece_index, piece) in loaded.pieces.iter_mut().enumerate() {
                    piece.visible = Some(piece_index) == index;
                }
            }
            if let Some(index) = index
                && let Some(visible) = visible
                && let Some(piece) = loaded.pieces.get_mut(index)
            {
                piece.visible = visible;
            }
            return Ok(Value::Null);
        }
        if index.is_some_and(|index| index >= loaded.assets.len()) {
            return Err("Texture index is out of range".into());
        }
        if single == Some(true) {
            preview.tiled = false;
            return Ok(Value::Null);
        }
        if all == Some(true) {
            preview.hidden_textures.clear();
            preview.tiled = true;
            let indices = (0..loaded.assets.len()).collect::<Vec<_>>();
            let mut failures = Vec::new();
            for index in indices {
                if let Err(error) = open_texture(app, owner, index) {
                    failures.push(error);
                }
            }
            if !failures.is_empty()
                && let Some(preview) = app.c3_preview.docs.get_mut(&owner)
            {
                preview.warnings.extend(failures);
            }
            return Ok(json!({"tiled":true}));
        }
        let index = index.ok_or_else(|| "Choose a texture index or Show all".to_string())?;
        if let Some(visible) = visible {
            preview.tiled = true;
            if visible {
                preview.hidden_textures.remove(&index);
            } else {
                preview.hidden_textures.insert(index);
                return Ok(Value::Null);
            }
        } else {
            preview.hidden_textures.remove(&index);
        }
        open_texture(app, owner, index)
    })())
}

fn invoke(app: &mut PhotocraftApp, ui: &egui::Ui, id: &str, params: Value) {
    let result = crate::menus::invoke(app, ui.ctx(), id, params);
    super::report(app, result);
}

pub(crate) fn panel(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let Some(owner) = scene_id(app) else {
        ui.label("Open a texture to inspect its C3 object.");
        return;
    };
    let Some(preview) = app.c3_preview.docs.get(&owner) else {
        ui.label("Load a C3 or select a client folder.");
        return;
    };
    let Some(loaded) = &preview.loaded else {
        ui.label(&preview.status);
        return;
    };
    let groups = loaded.effects.clone();
    let (title, pieces, model, objects, assets) =
        (loaded.title.clone(), loaded.pieces.clone(), loaded.model.clone(), preview.objects.clone(), loaded.assets.clone());
    ui.label(egui::RichText::new(title).strong());
    if !groups.is_empty() {
        ui.checkbox(&mut app.ui.c3_preview.effects_enabled, "Show effects");
        ui.collapsing(format!("{} effect groups", groups.len()), |ui| {
            for (index, group) in groups.iter().enumerate() {
                let mut on = group.visible;
                if ui.checkbox(&mut on, &group.label).on_hover_text(format!("{}\nSocket: {}", group.source, group.socket)).changed() {
                    invoke(app, ui, "preview.c3.playback", json!({"group":index,"visible":on}));
                }
                if let Some(note) = &group.note {
                    ui.label(egui::RichText::new(note).small());
                }
            }
        });
    }
    if objects.len() > 1 {
        let mut choose = None;
        egui::ComboBox::from_id_salt("c3-object-choice").selected_text(format!("{} INI objects", objects.len())).show_ui(ui, |ui| {
            for (id, label) in objects {
                if ui.selectable_label(false, label).clicked() {
                    choose = Some(id);
                }
            }
        });
        if let Some(id) = choose {
            invoke(app, ui, "preview.c3.object", json!({"id":id}));
        }
    }
    ui.horizontal_wrapped(|ui| {
        if ui.small_button("Show all").clicked() {
            invoke(app, ui, "preview.c3.piece", json!({"all":true}));
        }
        if ui.small_button("Hide all").clicked() {
            invoke(app, ui, "preview.c3.piece", json!({"all":false}));
        }
        if ui.small_button("Refresh").clicked() {
            invoke(app, ui, "preview.c3.rematch", json!({}));
        }
    });
    ui.label(egui::RichText::new("Visibility affects the preview; texture files stay open.").small());
    egui::ScrollArea::vertical().id_salt("c3-object-pieces").auto_shrink([false, false]).show(ui, |ui| {
        for (index, piece) in pieces.iter().enumerate() {
            ui.push_id(&piece.key, |ui| {
                let mut visible = piece.visible;
                ui.horizontal(|ui| {
                    if ui.add_enabled(!piece.range.is_empty(), egui::Checkbox::new(&mut visible, "")).on_hover_text("Show this C3 piece").changed() {
                        invoke(app, ui, "preview.c3.piece", json!({"index":index,"visible":visible}));
                    }
                    if ui.add_enabled(!piece.range.is_empty(), egui::Button::new("Solo").small()).clicked() {
                        invoke(app, ui, "preview.c3.piece", json!({"index":index,"solo":true}));
                    }
                    let label = if piece.path.is_empty() { "Missing C3 reference".into() } else { crate::file_open::display_name(&piece.path) };
                    crate::widgets::elided_label(ui, &label, ui.available_width()).on_hover_text(&piece.path);
                });
                if let Some(note) = &piece.note {
                    ui.label(egui::RichText::new(note).small());
                }
                ui.horizontal_wrapped(|ui| {
                    for index in &piece.textures {
                        if let Some(asset) = assets.get(*index)
                            && ui.small_button(format!("Edit {}", crate::file_open::display_name(&asset.key))).on_hover_text(&asset.key).clicked()
                        {
                            invoke(app, ui, "preview.c3.textures", json!({"index":index}));
                        }
                    }
                });
                super::effects::material_controls(app, ui, owner, index);
                ui.collapsing(format!("{} mesh parts", piece.range.len()), |ui| {
                    for part_index in piece.range.clone() {
                        if let Some(mesh) = model.meshes.get(part_index) {
                            let value = app
                                .c3_preview
                                .docs
                                .get(&owner)
                                .and_then(|p| p.loaded.as_ref())
                                .and_then(|l| l.visible.get(part_index))
                                .copied()
                                .unwrap_or(false);
                            let mut on = value;
                            if ui.checkbox(&mut on, &mesh.name).on_hover_text(&mesh.texture).changed() {
                                invoke(app, ui, "preview.c3.part", json!({"index":part_index,"visible":on}));
                            }
                        }
                    }
                });
                ui.separator();
            });
        }
    });
}

pub(crate) fn texture_bar(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let Some(owner) = scene_id(app) else { return };
    let Some(preview) = app.c3_preview.docs.get(&owner) else { return };
    let Some(loaded) = &preview.loaded else { return };
    if loaded.assets.is_empty() {
        return;
    }
    let loading = preview.pending_textures;
    let entries = loaded
        .assets
        .iter()
        .enumerate()
        .map(|(index, asset)| {
            let document = preview.texture_docs.get(&index).copied();
            let dirty = document.and_then(|id| app.session.documents().iter().find(|state| state.doc.id == id)).is_some_and(|state| state.is_dirty());
            let active = document == app.session.active().map(|state| state.doc.id);
            (
                index,
                asset.key.clone(),
                !preview.hidden_textures.contains(&index),
                asset.bytes.is_some() || document.is_some(),
                dirty,
                active,
                asset.note.clone(),
            )
        })
        .collect::<Vec<_>>();
    egui::CollapsingHeader::new("Object textures").default_open(true).show(ui, |ui| {
        ui.horizontal_wrapped(|ui| {
            if ui.add_enabled(!loading, egui::Button::new("Show all textures").small()).clicked() {
                invoke(app, ui, "preview.c3.textures", json!({"all":true}));
            }
            if ui.small_button("Single texture").clicked() {
                invoke(app, ui, "preview.c3.textures", json!({"single":true}));
            }
            if ui.small_button("Save active").clicked() {
                invoke(app, ui, "file.save", json!({}));
            }
        });
        if loading {
            ui.label(egui::RichText::new("Loading object textures...").small());
        }
        egui::ScrollArea::vertical().id_salt("c3-object-textures").max_height(130.).show(ui, |ui| {
            for (index, key, mut shown, ready, dirty, active, note) in entries {
                ui.horizontal(|ui| {
                    if ui.add_enabled(ready, egui::Checkbox::new(&mut shown, "")).on_hover_text("Show this texture in the tiled editing area").changed() {
                        invoke(app, ui, "preview.c3.textures", json!({"index":index,"visible":shown}));
                    }
                    let label = format!("{}{}", crate::file_open::display_name(&key), if dirty { " *" } else { "" });
                    if ui.add_enabled(ready, egui::Button::selectable(active, label).small()).on_hover_text(&key).clicked() {
                        invoke(app, ui, "preview.c3.textures", json!({"index":index}));
                    }
                });
                if let Some(note) = note {
                    ui.label(egui::RichText::new(note).small());
                }
            }
        });
    });
    ui.separator();
}

pub(crate) fn arranged_cells(app: &PhotocraftApp, rect: egui::Rect) -> Option<Vec<(usize, egui::Rect)>> {
    let owner = scene_id(app)?;
    let preview = app.c3_preview.docs.get(&owner)?;
    if !preview.tiled {
        return None;
    }
    let loaded = preview.loaded.as_ref()?;
    let indices = (0..loaded.assets.len())
        .filter(|index| !preview.hidden_textures.contains(index))
        .filter_map(|index| preview.texture_docs.get(&index))
        .filter_map(|id| app.session.documents().iter().position(|state| state.doc.id == *id))
        .collect::<Vec<_>>();
    let cells = crate::view_cmds::cells("tile", rect, indices.len()).unwrap_or_else(|| vec![rect]);
    Some(indices.into_iter().zip(cells).collect())
}

pub(crate) fn tiled(app: &PhotocraftApp) -> bool {
    scene_id(app).and_then(|id| app.c3_preview.docs.get(&id)).is_some_and(|preview| preview.tiled)
}

pub(super) fn effective_visibility(loaded: &Loaded) -> Vec<bool> {
    loaded
        .visible
        .iter()
        .enumerate()
        .map(|(index, visible)| *visible && loaded.pieces.iter().find(|piece| piece.range.contains(&index)).is_none_or(|piece| piece.visible))
        .collect()
}
