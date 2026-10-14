//! Read-only real-client validation: corpus <paths.txt> <report.json>.
#[cfg(not(target_arch = "wasm32"))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use serde_json::json;
    use std::{collections::BTreeMap, fs, path::Path, time::Instant};
    let args: Vec<_> = std::env::args().collect();
    let list = args.get(1).ok_or("pass a path-list file")?;
    let report = args.get(2).ok_or("pass a report file")?;
    let start = Instant::now();
    let mut passed = 0usize;
    let mut failures = Vec::new();
    let mut variants = BTreeMap::<String, usize>::new();
    let mut warning_counts = BTreeMap::<String, usize>::new();
    let mut examples = BTreeMap::<String, String>::new();
    let mut max_parts = 0;
    for (i, path) in fs::read_to_string(list)?.lines().filter(|p| !p.is_empty()).enumerate() {
        let result = photocraft_c3::assets::read_bounded(Path::new(path), photocraft_c3::MAX_FILE_BYTES).and_then(|bytes| photocraft_c3::decode(&bytes));
        match result {
            Ok(model) => {
                passed += 1;
                max_parts = max_parts.max(model.meshes.len());
                for m in model.meshes {
                    *variants.entry(m.variant.clone()).or_default() += 1;
                    if !m.indices.is_empty() {
                        examples.entry(m.variant).or_insert_with(|| path.into());
                    }
                }
                for w in model.warnings {
                    *warning_counts.entry(w).or_default() += 1;
                }
            }
            Err(e) => failures.push(json!({"path":path,"error":e.to_string()})),
        }
        if (i + 1) % 1000 == 0 {
            println!("{} checked, {passed} accepted, {} rejected", i + 1, failures.len());
        }
    }
    let result = json!({"passed":passed,"rejected":failures.len(),"variants":variants,"examples":examples,"max_parts":max_parts,"warnings":warning_counts,"failures":failures,"seconds":start.elapsed().as_secs_f64()});
    fs::write(report, serde_json::to_vec_pretty(&result)?)?;
    println!("{passed} accepted, {} rejected; report: {report}", result["rejected"]);
    Ok(())
}
#[cfg(target_arch = "wasm32")]
fn main() {}
