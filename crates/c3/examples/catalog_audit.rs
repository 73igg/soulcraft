//! Read-only validation of a client's binary/INI records and referenced assets.
#[cfg(not(target_arch = "wasm32"))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use photocraft_c3::{animation::AnimatedModel, assets::ClientAssets};
    use std::{
        collections::{BTreeMap, HashSet},
        path::Path,
        sync::atomic::AtomicBool,
    };
    let args = std::env::args().collect::<Vec<_>>();
    let root = args.get(1).ok_or("pass a client root")?;
    let report = args.get(2).ok_or("pass a report path")?;
    let assets = ClientAssets::quick(Path::new(root), &AtomicBool::new(false))?;
    let mut catalogs = BTreeMap::<String, usize>::new();
    let mut variants = BTreeMap::<String, usize>::new();
    let mut texture_paths = HashSet::new();
    let mut mesh_paths = HashSet::new();
    let mut bound = 0;
    let mut missing_definitions = Vec::new();
    let mut errors = Vec::new();
    let mut sample_rows = Vec::new();
    for object in assets.object_definitions() {
        *catalogs.entry(object.id.split(':').next().unwrap_or_default().into()).or_default() += 1;
        for binding in assets.effect_bindings(object).into_iter().filter(|binding| binding.enabled) {
            bound += 1;
            if assets.effect_object(&binding.effect).is_none() {
                missing_definitions.push(serde_json::json!({"object":object.id,"effect":binding.effect}));
            }
        }
        for part in &object.parts {
            if let Some(mesh) = &part.mesh {
                mesh_paths.insert(mesh.clone());
            }
            if let Some(texture) = &part.texture {
                texture_paths.insert(texture.clone());
            }
            if let Some(note) = &part.note {
                errors.push(serde_json::json!({"object":object.id,"error":note}));
            }
        }
        if object.id.starts_with("weapon.ini:") && sample_rows.len() < 10 {
            sample_rows.push(serde_json::json!({"id":object.id,"parts":object.parts.len(),"effects":assets.effect_bindings(object).into_iter().filter(|binding|binding.enabled).map(|binding|binding.effect).collect::<Vec<_>>()}));
        }
    }
    let source = Path::new(args.get(3).unwrap_or(root));
    let mut meshes = mesh_paths.into_iter().collect::<Vec<_>>();
    meshes.sort();
    let mut decoded = 0;
    let mut no_geometry = 0;
    for (index, path) in meshes.iter().enumerate() {
        let result = assets.read_for_source(path, source, photocraft_c3::MAX_FILE_BYTES).and_then(|bytes| AnimatedModel::decode(&bytes));
        match result {
            Ok(model) => {
                decoded += 1;
                for mesh in &model.model.meshes {
                    *variants.entry(mesh.variant.clone()).or_default() += 1;
                }
                for frame in [0, model.frame_count / 2, model.frame_count.saturating_sub(1)] {
                    if model
                        .sample(frame, [[1., 0., 0.], [0., 0., -1.]])
                        .iter()
                        .any(|mesh| mesh.vertices.iter().any(|vertex| vertex.position.iter().chain(&vertex.uv).any(|value| !value.is_finite())))
                    {
                        errors.push(serde_json::json!({"path":path,"error":"nonfinite sampled geometry"}));
                        break;
                    }
                }
            }
            Err(error) if error.0.contains("no renderable") => no_geometry += 1,
            Err(error) => errors.push(serde_json::json!({"path":path,"error":error.to_string()})),
        }
        if (index + 1) % 1000 == 0 {
            println!("{} meshes checked; {} errors", index + 1, errors.len());
        }
    }
    let mut unavailable_textures = Vec::new();
    for path in &texture_paths {
        if let Err(error) = assets.read_for_source(path, source, 32 * 1024 * 1024) {
            unavailable_textures.push(serde_json::json!({"path":path,"error":error.to_string()}));
        }
    }
    let result = serde_json::json!({"catalogs":catalogs,"boundEffects":bound,"missingEffectDefinitions":missing_definitions,"uniqueMeshes":meshes.len(),"decodedMeshes":decoded,"motionOnly":no_geometry,"uniqueTextures":texture_paths.len(),"unavailableTextures":unavailable_textures,"errors":errors,"variants":variants,"warnings":assets.warnings,"samples":sample_rows});
    std::fs::write(report, serde_json::to_vec_pretty(&result)?)?;
    println!("{decoded} decoded meshes; {bound} enabled effect bindings; {} errors; {} unavailable textures", errors.len(), unavailable_textures.len());
    Ok(())
}
#[cfg(target_arch = "wasm32")]
fn main() {}
