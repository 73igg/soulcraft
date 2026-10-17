//! Assemble sibling INI resources and retain their texture/document bindings.
use super::{
    Candidate, Found, FoundResult, Loaded, Piece, TextureAsset,
    gpu::Image,
    visibility,
    worker::{Level, client_assets},
};
use photocraft_c3::{
    Model,
    assets::{ClientAssets, ObjectDefinition, basename, catalog_root, infer_root, norm, read_bounded, resource_key},
};
use std::{
    collections::HashMap,
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

static SCENE_LOADER: Mutex<()> = Mutex::new(());

pub(super) fn discover(
    source: &str,
    root: Option<&str>,
    manual: Option<(String, Vec<u8>)>,
    object_id: Option<&str>,
    cancel: &AtomicBool,
    mut progress: impl FnMut(Found),
) -> FoundResult {
    // Keep rapid document changes from inflating multiple client indexes at
    // once. Waiting work observes cancellation before touching client assets.
    let _loader = SCENE_LOADER.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    check_cancel(cancel)?;
    let inferred = Path::new(source).parent().and_then(catalog_root);
    let manual_root = manual.as_ref().and_then(|(path, _)| Path::new(path).parent()).and_then(catalog_root);
    let root_path = manual_root
        .or(if manual.is_none() { inferred } else { None })
        .or_else(|| root.map(Path::new).map(|root| catalog_root(root).unwrap_or_else(|| root.to_path_buf())))
        .or_else(|| manual.as_ref().and_then(|(path, _)| infer_root(Path::new(path))))
        .or_else(|| infer_root(Path::new(source)));
    let mut assets = root_path
        .as_deref()
        .map(|root| client_assets(root, cancel, if manual.is_some() { Level::Loose } else { Level::Catalogs }))
        .transpose()
        .map_err(|error| error.to_string())?;
    let mut definitions = assets.as_ref().map(|assets| assets.objects_for_texture(source)).unwrap_or_default();
    let objects = definitions.iter().map(|object| (object.id.clone(), object.label.clone())).collect::<Vec<_>>();
    if let Some(id) = object_id {
        let definition = definitions
            .iter()
            .find(|definition| definition.id == id)
            .cloned()
            .or_else(|| assets.as_ref().and_then(|assets| assets.object(id)).cloned())
            .ok_or_else(|| "The INI object is no longer available; refresh the client lookup".to_string())?;
        definitions = vec![definition];
    }
    let mut candidates = Vec::new();
    if manual.is_none()
        && definitions.is_empty()
        && let Some(index) = &assets
    {
        candidates = index.candidates(source, cancel).map_err(|error| error.to_string())?;
        if candidates.is_empty() {
            let full = client_assets(&index.root, cancel, Level::Full).map_err(|error| error.to_string())?;
            candidates = full.candidates(source, cancel).map_err(|error| error.to_string())?;
            assets = Some(full);
        }
    }
    let candidates = candidates.into_iter().map(|candidate| Candidate { path: candidate.path, reason: candidate.reason }).collect::<Vec<_>>();
    let root = assets.as_ref().map(|assets| assets.root.to_string_lossy().into_owned());
    let mut warnings = assets.as_ref().map(|a| a.warnings.clone()).unwrap_or_default();
    let mut ready = |loaded: &Loaded| {
        progress(Found {
            complete: false,
            objects: objects.clone(),
            candidates: candidates.clone(),
            root: root.clone(),
            warnings: Vec::new(),
            loaded: Some(Loaded {
                model: loaded.model.clone(),
                textures: vec![None; loaded.model.meshes.len()],
                selected: loaded.selected.clone(),
                visible: loaded.visible.clone(),
                path: loaded.path.clone(),
                warnings: loaded.warnings.clone(),
                pieces: loaded.pieces.clone(),
                assets: loaded.assets.clone(),
                texture_slots: loaded.texture_slots.clone(),
                object_id: loaded.object_id.clone(),
                title: loaded.title.clone(),
                animations: loaded.animations.clone(),
                effects: loaded.effects.clone(),
                materials: loaded.materials.clone(),
                mix_slots: loaded.mix_slots.clone(),
                mix_textures: vec![[None, None, None]; loaded.model.meshes.len()],
            }),
        })
    };
    let loaded = if let Some((path, bytes)) = manual {
        Some(load(source, assets.as_deref(), None, Some((path, bytes)), cancel, &mut ready)?)
    } else if !definitions.is_empty() {
        let mut loaded = None;
        for definition in definitions.iter().take(32) {
            match load(source, assets.as_deref(), Some(definition), None, cancel, &mut ready) {
                Ok(scene) => {
                    loaded = Some(scene);
                    break;
                }
                Err(error) => warnings.push(format!("{}: {error}", definition.label)),
            }
        }
        loaded
    } else {
        let mut loaded = None;
        for candidate in candidates.iter().take(32) {
            match load(source, assets.as_deref(), None, Some((candidate.path.clone(), Vec::new())), cancel, &mut ready) {
                Ok(scene) => {
                    loaded = Some(scene);
                    break;
                }
                Err(error) => warnings.push(format!("{}: {error}", candidate.path)),
            }
        }
        loaded
    };
    Ok(Found { complete: true, objects, candidates, root, warnings, loaded })
}

fn check_cancel(cancel: &AtomicBool) -> Result<(), String> {
    if cancel.load(Ordering::Relaxed) { Err("cancelled".into()) } else { Ok(()) }
}

fn source_key(source: &str, assets: Option<&ClientAssets>) -> String {
    let canonical = Path::new(source).canonicalize().ok();
    let relative =
        canonical.as_deref().and_then(|source| assets.and_then(|assets| source.strip_prefix(&assets.root).ok())).map(|path| norm(&path.to_string_lossy()));
    relative.map(|key| resource_key(&key)).unwrap_or_else(|| norm(source))
}

fn asset_slot(
    assets: &mut Vec<TextureAsset>,
    map: &mut HashMap<String, usize>,
    texture: &str,
    source: &str,
    origin: &str,
    known_source: bool,
    client: Option<&ClientAssets>,
) -> Option<usize> {
    let source_available = assets.first().is_some_and(|asset| asset.key == source || asset.key == texture);
    let is_source = texture == source || (!known_source && source_available && basename(texture) == basename(source));
    if is_source {
        if let Some(asset) = assets.first_mut() {
            asset.key = texture.into();
        }
        map.insert(texture.into(), 0);
        return Some(0);
    }
    if let Some(index) = map.get(texture) {
        return Some(*index);
    }
    if texture.is_empty() || assets.len() >= 128 {
        return None;
    }
    let index = assets.len();
    map.insert(texture.into(), index);
    assets.push(TextureAsset {
        path: client.and_then(|client| client.editable_path_for_source(texture, Path::new(origin))).map(|path| path.to_string_lossy().into_owned()),
        key: texture.into(),
        bytes: None,
        source: false,
        note: None,
    });
    Some(index)
}

fn adjacent_texture(mesh: &str, source: &str, client: &ClientAssets) -> Option<String> {
    let (stem, extension) = mesh.rsplit_once('.')?;
    if !extension.eq_ignore_ascii_case("c3") {
        return None;
    }
    ["dds", "tga"]
        .into_iter()
        .map(|extension| format!("{stem}.{extension}"))
        .find(|texture| client.read_for_source(texture, Path::new(source), 32 * 1024 * 1024).is_ok())
}

fn load(
    source: &str,
    client: Option<&ClientAssets>,
    object: Option<&ObjectDefinition>,
    manual: Option<(String, Vec<u8>)>,
    cancel: &AtomicBool,
    ready: &mut impl FnMut(&Loaded),
) -> Result<Loaded, String> {
    let source_key = source_key(source, client);
    let known_source = Path::new(source).canonicalize().ok().is_some_and(|path| client.is_some_and(|client| path.starts_with(&client.root)));
    let mut assets =
        vec![TextureAsset { key: source_key.clone(), path: Path::new(source).is_file().then(|| source.to_string()), bytes: None, source: true, note: None }];
    let mut texture_map = HashMap::from([(source_key.clone(), 0usize)]);
    let mut pieces = Vec::new();
    let mut animations = Vec::new();
    let mut effect_groups = Vec::new();
    let mut materials = Vec::new();
    let mut mix_slots = Vec::new();
    let mut meshes = Vec::new();
    let mut slots = Vec::new();
    let mut visible = Vec::new();
    let mut animation_bytes = 0usize;
    let mut vertex_count = 0usize;
    let mut index_count = 0usize;
    let mut warnings = Vec::new();
    let mut requests = Vec::new();
    if let Some(object) = object {
        let direct_effect = object.id.starts_with("3deffect");
        if direct_effect {
            effect_groups.push(super::effects::EffectGroup {
                key: object.id.clone(),
                label: object.label.clone(),
                socket: String::new(),
                source: object.id.clone(),
                visible: true,
                note: None,
            });
        }
        for part in &object.parts {
            let material = client.map(|client| client.material(&object.id, part.slot)).unwrap_or_default();
            let timing = client.map(|client| client.part_timing(&object.id, part.slot)).unwrap_or_default();
            requests.push((
                format!("{}:{}", object.id, part.binding),
                part.mesh.clone().unwrap_or_default(),
                Vec::new(),
                part.texture.clone(),
                part.note.clone(),
                material,
                timing,
                direct_effect.then_some(0),
                String::new(),
            ));
        }
        if let Some(client) = client {
            for binding in client.effect_bindings(object) {
                let group = effect_groups.len();
                let definition = client.effect_object(&binding.effect);
                effect_groups.push(super::effects::EffectGroup {
                    key: binding.effect.clone(),
                    label: binding.effect.clone(),
                    socket: binding.socket.clone(),
                    source: binding.source,
                    visible: binding.enabled,
                    note: definition.is_none().then(|| "Effect definition is unavailable in the selected client".into()),
                });
                if let Some(definition) = definition {
                    for part in &definition.parts {
                        requests.push((
                            format!("{}:{}", binding.effect, part.binding),
                            part.mesh.clone().unwrap_or_default(),
                            Vec::new(),
                            part.texture.clone(),
                            part.note.clone(),
                            client.material(&definition.id, part.slot),
                            client.part_timing(&definition.id, part.slot),
                            Some(group),
                            binding.socket.clone(),
                        ));
                    }
                }
            }
        }
    } else if let Some((path, bytes)) = manual {
        requests.push((path.clone(), path, bytes, None, None, Default::default(), Default::default(), None, String::new()));
    }
    requests.sort_by_key(|request| request.7.map(|index| if effect_groups.get(index).is_some_and(|group| group.visible) { 1 } else { 2 }).unwrap_or(0));
    if requests.len() > 128 {
        warnings.push(format!("{} C3 files exceed the 128-file scene budget; select a smaller attachment set", requests.len()));
    }
    for (key, path, supplied, registry_texture, note, material, timing, effect, socket) in requests.into_iter().take(128) {
        check_cancel(cancel)?;
        let registry_slot =
            registry_texture.as_ref().and_then(|texture| asset_slot(&mut assets, &mut texture_map, texture, &source_key, source, known_source, client));
        let mut additional_slots = material
            .additional_textures
            .iter()
            .filter_map(|texture| asset_slot(&mut assets, &mut texture_map, texture, &source_key, source, known_source, client))
            .collect::<Vec<_>>();
        let mix_slot = std::array::from_fn(|stage| material.texture_stages.get(stage).and_then(Option::as_ref).and_then(|path| texture_map.get(path).copied()));
        if !matches!(
            material.mix_mode,
            0 | 2 | 4 | 7 | 8 | 9 | 10 | 11 | 13 | 15 | 16 | 19 | 23 | 28 | 30 | 31 | 32 | 33 | 34 | 35 | 36 | 37 | 38 | 39 | 40 | 45
        ) {
            warnings.push(format!(
                "{key}: MixOpt {} needs client lighting, reflection, or a shader not reproduced by this preview; displaying its primary material",
                material.mix_mode
            ));
        }
        let start = meshes.len();
        let result = (|| {
            if path.is_empty() {
                return Err(note.clone().unwrap_or_else(|| "INI mesh reference is missing".into()));
            }
            let bytes = if !supplied.is_empty() {
                supplied
            } else if Path::new(&path).is_absolute() {
                read_bounded(Path::new(&path), photocraft_c3::MAX_FILE_BYTES).map_err(|error| error.to_string())?
            } else {
                client
                    .ok_or_else(|| "Choose a client folder for mapped resources".to_string())?
                    .read_for_source(&path, Path::new(source), photocraft_c3::MAX_FILE_BYTES)
                    .map_err(|error| error.to_string())?
            };
            let animated = Arc::new(photocraft_c3::animation::AnimatedModel::decode(&bytes).map_err(|error| error.to_string())?);
            let next_animation_bytes = animation_bytes.saturating_add(animated.memory_bytes());
            if next_animation_bytes > 512 * 1024 * 1024 {
                return Err("Scene animations exceed the 512 MiB decoded-data budget".into());
            }
            let mut model = animated.model.clone();
            let vertices = model.meshes.iter().map(|mesh| mesh.vertices.len()).sum::<usize>();
            let indices = model.meshes.iter().map(|mesh| mesh.indices.len()).sum::<usize>();
            if vertex_count.saturating_add(vertices) > 2_000_000
                || index_count.saturating_add(indices) > 6_000_000
                || meshes.len().saturating_add(model.meshes.len()) > 512
            {
                return Err("Object exceeds the bounded geometry preview budget".into());
            }
            animation_bytes = next_animation_bytes;
            vertex_count += vertices;
            index_count += indices;
            warnings.append(&mut model.warnings);
            // A few original effect rows have dangling TextureId values and
            // obsolete exporter paths. Recover only the exact adjacent filename,
            // in the same extraction/package as the mesh; never a nearby ID or
            // a same-basename image elsewhere in the client.
            let registry_texture = registry_texture.clone().or_else(|| {
                if effect.is_some() && note.is_some() {
                    client.and_then(|client| adjacent_texture(&path, source, client)).inspect(|texture| {
                        warnings.push(format!("{key}: catalog texture is unavailable; using adjacent mesh texture {texture}"));
                    })
                } else {
                    None
                }
            });
            for mesh in &mut model.meshes {
                let texture =
                    registry_texture.clone().or_else(|| client.and_then(|client| client.texture_path(&mesh.texture))).unwrap_or_else(|| mesh.texture.clone());
                let slot = asset_slot(&mut assets, &mut texture_map, &texture, &source_key, source, known_source, client);
                if slot.is_none() && !texture.is_empty() {
                    warnings.push(format!("Texture budget reached: {texture}"));
                }
                if registry_texture.is_some() {
                    mesh.texture = texture;
                }
                slots.push(slot);
                mix_slots.push(mix_slot);
            }
            let mut defaults = visibility(&model);
            for (index, mesh) in model.meshes.iter().enumerate() {
                if matches!(mesh.variant.as_str(), "Particles" | "Trail")
                    && let Some(slot) = defaults.get_mut(index)
                {
                    *slot = true;
                }
            }
            visible.extend(defaults);
            materials.extend(std::iter::repeat_n(material.clone(), model.meshes.len()));
            let parent = if socket.is_empty() {
                None
            } else {
                animations.iter().enumerate().find_map(|(index, animation): (usize, &super::effects::SceneAnimation)| {
                    animation.effect.is_none().then(|| animation.asset.resolve_socket(&socket).map(|socket| (index, socket.to_owned()))).flatten()
                })
            };
            // v_r_weapon/v_l_weapon identify the standalone weapon's root. Other
            // attachment names must resolve to an actual socket, or stay explicit.
            if !socket.is_empty() && parent.is_none() && !matches!(socket.as_str(), "v_r_weapon" | "v_l_weapon" | "v_body" | "v_mount" | "v_mantle" | "v_head")
            {
                warnings.push(format!("{key}: attachment socket {socket} is absent from the loaded base geometry"));
            }
            animations.push(super::effects::SceneAnimation {
                asset: animated,
                range: start..start + model.meshes.len(),
                material: material.clone(),
                timing: timing.clone(),
                effect,
                parent,
            });
            meshes.extend(model.meshes);
            Ok(())
        })();
        let failure = result.err().or(note);
        if let Some(note) = &failure {
            warnings.push(format!("{path}: {note}"));
        }
        let mut textures = slots.get(start..meshes.len()).unwrap_or_default().iter().copied().flatten().chain(registry_slot).collect::<Vec<_>>();
        textures.append(&mut additional_slots);
        textures.sort_unstable();
        textures.dedup();
        pieces.push(Piece { key, path, range: start..meshes.len(), visible: true, note: failure, textures });
    }
    super::effects::synchronize_durations(&mut animations);
    if meshes.is_empty() && object.is_none() {
        return Err(warnings.first().cloned().unwrap_or_else(|| "Object has no drawable C3 geometry".into()));
    }
    let mut min = [f32::INFINITY; 3];
    let mut max = [f32::NEG_INFINITY; 3];
    for animation in &animations {
        if animation.effect.is_some_and(|index| effect_groups.get(index).is_some_and(|group| !group.visible)) {
            continue;
        }
        let socket = animation
            .parent
            .as_ref()
            .and_then(|(index, name)| animations.get(*index).and_then(|parent| parent.asset.socket(name, 0)))
            .unwrap_or(photocraft_c3::math::IDENTITY);
        for x in [animation.asset.model.min[0], animation.asset.model.max[0]] {
            for y in [animation.asset.model.min[1], animation.asset.model.max[1]] {
                for z in [animation.asset.model.min[2], animation.asset.model.max[2]] {
                    let point = [x, y, z].map(|v| v * animation.material.scale);
                    let point = std::array::from_fn(|i| point[i] + animation.timing.offset[i]);
                    let point = photocraft_c3::math::transform(point, &socket);
                    for ((lo, hi), value) in min.iter_mut().zip(&mut max).zip(point) {
                        *lo = lo.min(value);
                        *hi = hi.max(value);
                    }
                }
            }
        }
    }
    if min.iter().any(|value| !value.is_finite()) {
        min = [0.; 3];
        max = [0.; 3];
    }
    let model = Arc::new(Model { meshes, warnings: Vec::new(), min, max });
    let mut selected = slots.iter().zip(&visible).map(|(slot, visible)| *slot == Some(0) && *visible).collect::<Vec<_>>();
    if object.is_none() && !selected.iter().any(|selected| *selected) {
        let primary =
            model.meshes.iter().enumerate().filter(|(i, _)| visible.get(*i) == Some(&true)).max_by_key(|(_, mesh)| mesh.indices.len()).map(|(i, _)| i);
        if let Some(index) = primary
            && let Some(selected) = selected.get_mut(index)
        {
            *selected = true;
        }
    }
    let mut loaded = Loaded {
        textures: vec![None; model.meshes.len()],
        path: pieces.iter().find(|part| !part.range.is_empty()).or_else(|| pieces.first()).map(|part| part.path.clone()).unwrap_or_default(),
        title: object.map(|object| object.label.clone()).unwrap_or_else(|| "Selected C3".into()),
        animations,
        effects: effect_groups,
        materials,
        mix_slots,
        mix_textures: vec![[None, None, None]; model.meshes.len()],
        object_id: object.map(|object| object.id.clone()),
        model,
        selected,
        visible,
        pieces,
        assets,
        texture_slots: slots,
        warnings,
    };
    ready(&loaded);
    let mut decoded = Vec::new();
    let mut encoded_total = 0usize;
    let mut pixel_total = 0usize;
    let mut full_index: Option<Result<Arc<ClientAssets>, String>> = None;
    for asset in &mut loaded.assets {
        check_cancel(cancel)?;
        let result: Result<Arc<Image>, String> = (|| {
            let bytes = if let Some(path) = &asset.path {
                read_bounded(Path::new(path), 32 * 1024 * 1024).map_err(|error| error.to_string())?
            } else if let Some(client) = client {
                // An INI path is authoritative. Reading it directly also checks
                // lazy package indexes; a missing path must not scan the client
                // or substitute an unrelated file with the same basename.
                let authoritative = asset.key.contains(['/', '\\'])
                    || object.is_some_and(|definition| definition.parts.iter().any(|part| part.texture.as_deref() == Some(asset.key.as_str())));
                let key = if authoritative {
                    asset.key.clone()
                } else if let Some(key) = client.texture_path(&asset.key) {
                    key
                } else {
                    let index = full_index.get_or_insert_with(|| client_assets(&client.root, cancel, Level::Full).map_err(|error| error.to_string()));
                    index.as_ref().map_err(Clone::clone)?.texture_path(&asset.key).ok_or_else(|| "Texture is unavailable".to_string())?
                };
                asset.path = client.editable_path_for_source(&key, Path::new(source)).map(|path| path.to_string_lossy().into_owned());
                client.read_for_source(&key, Path::new(source), 32 * 1024 * 1024).map_err(|error| error.to_string())?
            } else {
                return Err("Choose a client folder to locate this texture".into());
            };
            encoded_total = encoded_total.saturating_add(bytes.len());
            if encoded_total > 128 * 1024 * 1024 {
                return Err("Object textures exceed the 128 MiB encoded-data budget".into());
            }
            let options = photocraft_codecs::DecodeOptions {
                limits: photocraft_codecs::Limits { max_width: 8192, max_height: 8192, max_pixels: 16 * 1024 * 1024, max_alloc: 256 * 1024 * 1024 },
                ..Default::default()
            };
            // Keep encoded originals for editing full DDS layouts through the normal importer.
            asset.bytes = Some(Arc::new(bytes));
            let image =
                photocraft_codecs::decode_with(asset.bytes.as_deref().map(Vec::as_slice).unwrap_or_default(), &options).map_err(|error| error.to_string())?;
            let width = image.width();
            let height = image.height();
            let rgba = image::RgbaImage::from_raw(width, height, image.to_rgba8()).ok_or_else(|| "Invalid preview image layout".to_string())?;
            let scale = (2048. / f64::from(width.max(height))).min(1.);
            let size = [(f64::from(width) * scale).round().max(1.) as u32, (f64::from(height) * scale).round().max(1.) as u32];
            let pixels = if size == [width, height] {
                rgba.into_raw()
            } else {
                image::imageops::resize(&rgba, size[0], size[1], image::imageops::FilterType::Triangle).into_raw()
            };
            pixel_total = pixel_total.saturating_add(pixels.len());
            if pixel_total > 64 * 1024 * 1024 {
                return Err("Object textures exceed the 64 MiB decoded preview budget".into());
            }
            Ok(Arc::new(Image { size, pixels }))
        })();
        match result {
            Ok(image) => decoded.push(Some(image)),
            Err(error) => {
                asset.note = Some(error.clone());
                loaded.warnings.push(format!("{}: {error}", asset.key));
                decoded.push(None);
            }
        }
    }
    loaded.textures = loaded.texture_slots.iter().map(|slot| slot.and_then(|slot| decoded.get(slot).cloned()).flatten()).collect();
    loaded.mix_textures = loaded
        .mix_slots
        .iter()
        .map(|slots| std::array::from_fn(|stage| slots.get(stage).copied().flatten().and_then(|slot| decoded.get(slot).cloned()).flatten()))
        .collect();
    Ok(loaded)
}
