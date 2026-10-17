//! Read-only probe of active catalog attachments and compressed resources.
#[cfg(not(target_arch = "wasm32"))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use photocraft_c3::{animation::AnimatedModel, assets::ClientAssets};
    use std::{path::Path, sync::atomic::AtomicBool, time::Instant};
    let args = std::env::args().collect::<Vec<_>>();
    let root = args.get(1).ok_or("pass a client folder")?;
    let texture = args.get(2).ok_or("pass a texture path")?;
    let report = args.get(3).ok_or("pass a report path")?;
    let start = Instant::now();
    let assets = ClientAssets::quick(Path::new(root), &AtomicBool::new(false))?;
    if matches!(texture.as_str(), "--binding-audit" | "--armor-binding-audit") {
        let catalog = if texture == "--armor-binding-audit" { "armor.ini" } else { "weapon.ini" };
        let bytes = photocraft_c3::assets::read_bounded(&Path::new(root).join("ini").join(catalog), 32 * 1024 * 1024)?;
        let text = encoding_rs::GBK.decode(&bytes).0;
        let mut textures = std::collections::BTreeSet::new();
        let mut object_rows = Vec::new();
        for line in text.lines().map(str::trim) {
            if let Some(section) = line.strip_prefix('[').and_then(|line| line.strip_suffix(']'))
                && let Some(object) = assets.object(&format!("{catalog}:{section}"))
            {
                textures.extend(object.parts.iter().filter_map(|part| part.texture.clone()));
                let effects = assets.effect_bindings(object).into_iter().filter(|binding| binding.enabled).map(|binding| {
                        serde_json::json!({"effect":binding.effect,"source":binding.source,"definitionAvailable":assets.effect_object(&binding.effect).is_some()})
                    }).collect::<Vec<_>>();
                object_rows.push(serde_json::json!({"id":object.id,"enabled":effects}));
            }
        }
        let mut rows = Vec::new();
        for texture in textures {
            let objects = assets.objects_for_texture(&texture);
            let Some(selected) = objects.first() else { continue };
            let bindings = assets.effect_bindings(selected);
            let effects = bindings.iter().filter(|binding| binding.enabled).map(|binding| {
                serde_json::json!({"effect":binding.effect,"source":binding.source,"definitionAvailable":assets.effect_object(&binding.effect).is_some()})
            }).collect::<Vec<_>>();
            rows.push(serde_json::json!({"texture":texture,"selected":selected.id,"alternatives":objects.len(),"enabled":effects}));
        }
        let mut result =
            serde_json::json!({"root":assets.root,"catalog":catalog,"groups":rows.len(),"weapons":rows,"elapsedSeconds":start.elapsed().as_secs_f64()});
        result[if catalog == "armor.ini" { "armorObjects" } else { "weaponObjects" }] = serde_json::json!(object_rows);
        std::fs::write(report, serde_json::to_vec_pretty(&result)?)?;
        println!("{} {catalog} texture groups audited; {:.3}s", result["groups"], start.elapsed().as_secs_f64());
        return Ok(());
    }
    let all = texture == "--all-effects";
    let definitions = if all {
        let mut definitions = Vec::new();
        for catalog in ["3deffect.ini", "3deffect3.ini"] {
            let path = Path::new(root).join("ini").join(catalog);
            if !path.is_file() {
                continue;
            }
            let bytes = photocraft_c3::assets::read_bounded(&path, 16 * 1024 * 1024)?;
            let text = std::str::from_utf8(&bytes).map(str::to_string).unwrap_or_else(|_| encoding_rs::GBK.decode(&bytes).0.into_owned());
            for line in text.lines().map(str::trim) {
                if let Some(section) = line.strip_prefix('[').and_then(|line| line.strip_suffix(']'))
                    && let Some(object) = assets.object(&format!("{catalog}:{section}"))
                {
                    definitions.push(object.clone());
                }
            }
        }
        definitions
    } else {
        assets.objects_for_texture(texture)
    };
    let mut rows = Vec::new();
    for object in definitions.iter().take(if all { usize::MAX } else { 1 }) {
        let bindings = if all { Vec::new() } else { assets.effect_bindings(object) };
        let mut definitions = vec![(object.id.clone(), true, object.clone())];
        for binding in bindings {
            if let Some(definition) = assets.effect_object(&binding.effect) {
                definitions.push((binding.effect, binding.enabled, definition.clone()));
            }
        }
        for (binding, enabled, definition) in definitions {
            for part in &definition.parts {
                let result = (|| -> Result<serde_json::Value, Box<dyn std::error::Error>> {
                    let mesh = part.mesh.as_ref().ok_or("missing mesh")?;
                    let bytes = assets.read_for_source(mesh, Path::new(texture), photocraft_c3::MAX_FILE_BYTES)?;
                    let model = AnimatedModel::decode(&bytes)?;
                    let sockets = model
                        .model
                        .meshes
                        .iter()
                        .filter_map(|mesh| model.socket(&mesh.name, 0).map(|matrix| serde_json::json!({"name":mesh.name,"matrix":matrix})))
                        .collect::<Vec<_>>();
                    let material = assets.material(&definition.id, part.slot);
                    let mut textures = part.texture.iter().cloned().chain(material.additional_textures).collect::<Vec<_>>();
                    textures.sort();
                    textures.dedup();
                    let textures = textures
                        .into_iter()
                        .map(|key| match assets.read_for_source(&key, Path::new(texture), 32 * 1024 * 1024) {
                            Ok(bytes) => serde_json::json!({"key":key,"bytes":bytes.len(),"editable":assets.editable_path(&key).is_some()}),
                            Err(e) => serde_json::json!({"key":key,"error":e.to_string()}),
                        })
                        .collect::<Vec<_>>();
                    Ok(
                        serde_json::json!({"mesh":mesh,"bytes":bytes.len(),"parts":model.model.meshes.len(),"frames":model.frame_count,"decodedBytes":model.memory_bytes(),"bounds":[model.model.min,model.model.max],"materialScale":material.scale,"meshTextures":model.model.meshes.iter().map(|mesh|&mesh.texture).collect::<Vec<_>>(),"sockets":sockets,"warnings":model.model.warnings,"textures":textures}),
                    )
                })();
                rows.push(serde_json::json!({"binding":binding,"enabled":enabled,"slot":part.slot,"result":result.unwrap_or_else(|e|serde_json::json!({"error":e.to_string()}))}));
            }
        }
    }
    let result = serde_json::json!({"root":assets.root,"objects":definitions.iter().map(|o|&o.id).collect::<Vec<_>>(),"resources":rows,"elapsedSeconds":start.elapsed().as_secs_f64()});
    std::fs::write(report, serde_json::to_vec_pretty(&result)?)?;
    println!("{} objects; {} resource rows; {:.3}s", definitions.len(), rows.len(), start.elapsed().as_secs_f64());
    Ok(())
}
#[cfg(target_arch = "wasm32")]
fn main() {}
