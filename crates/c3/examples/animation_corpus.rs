//! Read-only validation of complete animation frames from a local path list.
#[cfg(not(target_arch = "wasm32"))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use photocraft_c3::{animation::AnimatedModel, assets::read_bounded};
    use std::{collections::BTreeMap, fs, path::Path};
    let args = std::env::args().collect::<Vec<_>>();
    let list = args.get(1).ok_or("pass a path-list file")?;
    let report = args.get(2).ok_or("pass a report file")?;
    let mut passed = 0usize;
    let mut skipped = 0usize;
    let mut errors = Vec::new();
    let mut variants = BTreeMap::<String, usize>::new();
    let mut warnings = BTreeMap::<String, usize>::new();
    for (index, path) in fs::read_to_string(list)?.lines().filter(|path| !path.is_empty()).enumerate() {
        let result = (|| -> Result<(), String> {
            let bytes = read_bounded(Path::new(path), photocraft_c3::MAX_FILE_BYTES).map_err(|e| e.to_string())?;
            let animated = AnimatedModel::decode(&bytes).map_err(|e| e.to_string())?;
            for mesh in &animated.model.meshes {
                *variants.entry(mesh.variant.clone()).or_default() += 1;
            }
            for warning in &animated.model.warnings {
                *warnings.entry(warning.clone()).or_default() += 1;
            }
            for frame in
                [0, 1, animated.frame_count / 4, animated.frame_count / 2, animated.frame_count.saturating_sub(1), animated.frame_count.saturating_add(1)]
            {
                for sample in animated.sample(frame, [[1., 0., 0.], [0., 0., -1.]]) {
                    if sample.indices.iter().any(|index| *index as usize >= sample.vertices.len()) {
                        return Err("sampled index is out of range".into());
                    }
                    for vertex in sample.indices.iter().filter_map(|i| sample.vertices.get(*i as usize)) {
                        if vertex.position.iter().chain(&vertex.normal).chain(&vertex.uv).any(|v| !v.is_finite()) {
                            return Err("nonfinite sampled vertex".into());
                        }
                    }
                }
            }
            Ok(())
        })();
        match result {
            Ok(()) => passed += 1,
            Err(error) if error.contains("no renderable") => skipped += 1,
            Err(error) => errors.push(serde_json::json!({"path":path,"error":error})),
        }
        if (index + 1) % 1000 == 0 {
            println!("{} checked; {passed} passed; {skipped} motion-only; {} errors", index + 1, errors.len());
        }
    }
    fs::write(
        report,
        serde_json::to_vec_pretty(&serde_json::json!({"passed":passed,"motionOnly":skipped,"errors":errors,"variants":variants,"warnings":warnings}))?,
    )?;
    println!("{passed} passed; {skipped} motion-only; {} errors", errors.len());
    Ok(())
}
#[cfg(target_arch = "wasm32")]
fn main() {}
