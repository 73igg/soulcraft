//! Validate named models through both loose files and DawnPack indexes.
#[cfg(not(target_arch = "wasm32"))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use serde_json::json;
    use std::{collections::BTreeMap, path::Path, sync::atomic::AtomicBool};
    let args: Vec<_> = std::env::args().collect();
    let root = args.get(1).ok_or("pass a client root")?;
    let report = args.get(2).ok_or("pass a report path")?;
    let assets = photocraft_c3::assets::ClientAssets::build(Path::new(root), &AtomicBool::new(false))?;
    let mut headers = BTreeMap::<String, usize>::new();
    let mut variants = BTreeMap::<String, usize>::new();
    let mut accepted = 0;
    let mut missing = 0;
    let mut motion_only = 0;
    let mut errors = Vec::new();
    for path in assets.model_paths() {
        let bytes = match assets.read(&path, photocraft_c3::MAX_FILE_BYTES) {
            Ok(bytes) => bytes,
            Err(_) => {
                missing += 1;
                continue;
            }
        };
        let header = String::from_utf8_lossy(bytes.get(..16).unwrap_or_default()).into_owned();
        *headers.entry(header).or_default() += 1;
        if photocraft_c3::texture_references(&bytes).is_ok_and(|references| references.is_empty()) {
            motion_only += 1;
            continue;
        }
        match photocraft_c3::decode(&bytes) {
            Ok(m) => {
                accepted += 1;
                for mesh in m.meshes {
                    *variants.entry(mesh.variant).or_default() += 1;
                }
            }
            Err(e) => errors.push(json!({"path":path,"error":e.to_string()})),
        }
    }
    let result = json!({"root":root,"accepted":accepted,"missing":missing,"motionOnly":motion_only,"errors":errors,"headers":headers,"variants":variants,"assetWarnings":assets.warnings});
    std::fs::write(report, serde_json::to_vec_pretty(&result)?)?;
    println!("{accepted} accepted, {missing} unavailable, {} errors", result["errors"].as_array().map_or(0, Vec::len));
    Ok(())
}
#[cfg(target_arch = "wasm32")]
fn main() {}
