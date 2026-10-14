//! Read-only stage timings: load_timing <client> <texture-path> <mesh-relative-path>.
#[cfg(not(target_arch = "wasm32"))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use photocraft_c3::assets::ClientAssets;
    use std::{path::Path, sync::atomic::AtomicBool, time::Instant};
    let args: Vec<_> = std::env::args().collect();
    let root = args.get(1).ok_or("pass a client folder")?;
    let texture = args.get(2).ok_or("pass a texture path")?;
    let mesh = args.get(3).ok_or("pass a relative mesh path")?;
    let cancel = AtomicBool::new(false);
    let started = Instant::now();
    let assets = match args.get(4).map(String::as_str) {
        Some("quick") => ClientAssets::quick(Path::new(root), &cancel)?,
        Some("loose") => ClientAssets::loose(Path::new(root), &cancel)?,
        _ => ClientAssets::build(Path::new(root), &cancel)?,
    };
    let index_ms = started.elapsed().as_secs_f64() * 1000.;
    let started = Instant::now();
    let candidates = assets.candidates(texture, &cancel)?;
    let match_ms = started.elapsed().as_secs_f64() * 1000.;
    let started = Instant::now();
    let bytes = assets.read(mesh, photocraft_c3::MAX_FILE_BYTES)?;
    let read_ms = started.elapsed().as_secs_f64() * 1000.;
    let started = Instant::now();
    let model = photocraft_c3::decode(&bytes)?;
    let decode_ms = started.elapsed().as_secs_f64() * 1000.;
    println!(
        "{}",
        serde_json::json!({"indexMs":index_ms,"matchMs":match_ms,"readMs":read_ms,"decodeMs":decode_ms,"candidates":candidates.len(),"parts":model.meshes.len()})
    );
    Ok(())
}
#[cfg(target_arch = "wasm32")]
fn main() {}
