//! Read a single local client resource for an explicitly chosen QA destination.
#[cfg(not(target_arch = "wasm32"))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use std::{path::Path, sync::atomic::AtomicBool};
    let arguments = std::env::args().collect::<Vec<_>>();
    let root = arguments.get(1).ok_or("pass a client folder")?;
    let key = arguments.get(2).ok_or("pass a resource key")?;
    let output = arguments.get(3).ok_or("pass a QA output file")?;
    let assets = photocraft_c3::assets::ClientAssets::quick(Path::new(root), &AtomicBool::new(false))?;
    let bytes = assets.read(key, photocraft_c3::MAX_FILE_BYTES)?;
    if let Some(parent) = Path::new(output).parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(output, &bytes)?;
    println!("Read {} bytes from {key}", bytes.len());
    Ok(())
}
#[cfg(target_arch = "wasm32")]
fn main() {}
