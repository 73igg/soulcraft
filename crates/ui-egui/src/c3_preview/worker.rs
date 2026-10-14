use super::{Candidate, Found, FoundResult, Loaded, gpu::Image, visibility};
use photocraft_c3::assets::{ClientAssets, basename, infer_root, read_bounded};
use std::{
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, OnceLock, PoisonError,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
};

#[derive(Clone, Copy, PartialEq, PartialOrd)]
enum Level {
    Loose,
    Catalogs,
    Full,
}
type AssetCache = Vec<(PathBuf, Level, Arc<ClientAssets>)>;
static ASSETS: OnceLock<Mutex<AssetCache>> = OnceLock::new();
static CACHE_REVISION: AtomicU64 = AtomicU64::new(0);
pub fn clear_cache() {
    CACHE_REVISION.fetch_add(1, Ordering::Relaxed);
    if let Some(cache) = ASSETS.get() {
        cache.lock().unwrap_or_else(PoisonError::into_inner).clear();
    }
}
fn client_assets(root: &Path, cancel: &AtomicBool, level: Level) -> Result<Arc<ClientAssets>, photocraft_c3::Error> {
    let root = root.canonicalize().map_err(|e| photocraft_c3::Error(e.to_string()))?;
    let cache = ASSETS.get_or_init(|| Mutex::new(Vec::new()));
    if let Some(assets) = cache
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .iter()
        .find(|(key, cached_level, _)| *key == root && *cached_level >= level)
        .map(|(_, _, assets)| assets.clone())
    {
        return Ok(assets);
    }
    let revision = CACHE_REVISION.load(Ordering::Relaxed);
    let assets = Arc::new(match level {
        Level::Loose => ClientAssets::loose(&root, cancel)?,
        Level::Catalogs => ClientAssets::quick(&root, cancel)?,
        Level::Full => ClientAssets::build(&root, cancel)?,
    });
    let mut cache = cache.lock().unwrap_or_else(PoisonError::into_inner);
    if !cancel.load(Ordering::Relaxed) && revision == CACHE_REVISION.load(Ordering::Relaxed) {
        if let Some((_, _, existing)) = cache.iter().find(|(key, cached_level, _)| *key == root && *cached_level >= level) {
            return Ok(existing.clone());
        }
        cache.retain(|(key, _, _)| *key != root);
        if cache.len() >= 2 {
            cache.remove(0);
        }
        cache.push((root, level, assets.clone()));
    }
    Ok(assets)
}

pub fn discover(source: &str, root: Option<&str>, manual: Option<(String, Vec<u8>)>, cancel: &AtomicBool, mut progress: impl FnMut(Found)) -> FoundResult {
    let manual = manual
        .map(|(path, bytes)| {
            if bytes.is_empty() && Path::new(&path).is_absolute() {
                read_bounded(Path::new(&path), photocraft_c3::MAX_FILE_BYTES).map(|bytes| (path, bytes)).map_err(|e| e.to_string())
            } else {
                Ok((path, bytes))
            }
        })
        .transpose()?;
    let root_path = root
        .map(Path::new)
        .map(Path::to_path_buf)
        .or_else(|| manual.as_ref().and_then(|(p, _)| infer_root(Path::new(p))))
        .or_else(|| infer_root(Path::new(source)));
    let mut assets = match root_path.as_deref() {
        Some(root) => match client_assets(root, cancel, if manual.is_some() { Level::Loose } else { Level::Catalogs }) {
            Ok(a) => Some(a),
            Err(e) if manual.as_ref().is_some_and(|(_, b)| !b.is_empty()) => {
                let _ = e;
                None
            }
            Err(e) => return Err(e.to_string()),
        },
        None => None,
    };
    let mut warnings = assets.as_ref().map(|a| a.warnings.iter().filter(|w| !w.starts_with("souldata.dnp:")).cloned().collect::<Vec<_>>()).unwrap_or_default();
    let root = assets.as_ref().map(|a| a.root.to_string_lossy().into_owned());
    let mut candidates = Vec::new();
    if manual.is_none()
        && let Some(index) = &assets
    {
        candidates = index.candidates(source, cancel).map_err(|e| e.to_string())?.into_iter().map(|c| Candidate { path: c.path, reason: c.reason }).collect();
        if candidates.is_empty() {
            let full = client_assets(&index.root, cancel, Level::Full).map_err(|e| e.to_string())?;
            candidates =
                full.candidates(source, cancel).map_err(|e| e.to_string())?.into_iter().map(|c| Candidate { path: c.path, reason: c.reason }).collect();
            assets = Some(full);
        }
    }
    let mut ready = |loaded: &Loaded| {
        progress(Found {
            complete: false,
            candidates: candidates.clone(),
            root: root.clone(),
            warnings: warnings.clone(),
            loaded: Some(Loaded {
                model: loaded.model.clone(),
                textures: (0..loaded.textures.len()).map(|_| None).collect(),
                selected: loaded.selected.clone(),
                visible: loaded.visible.clone(),
                path: loaded.path.clone(),
                warnings: loaded.warnings.clone(),
            }),
        });
    };
    let mut attempt_errors = Vec::new();
    let loaded = if let Some((path, bytes)) = manual {
        let bytes = if bytes.is_empty() {
            assets
                .as_ref()
                .ok_or_else(|| "Choose a client folder before selecting a mapped mesh".to_string())?
                .read(&path, photocraft_c3::MAX_FILE_BYTES)
                .map_err(|e| e.to_string())?
        } else {
            bytes
        };
        Some(load(&path, &bytes, source, assets.as_deref(), cancel, &mut ready)?)
    } else if let Some(assets) = &assets {
        let mut loaded = None;
        for candidate in candidates.iter().take(32) {
            if cancel.load(Ordering::Relaxed) {
                return Err("cancelled".into());
            }
            let result = assets
                .read(&candidate.path, photocraft_c3::MAX_FILE_BYTES)
                .map_err(|e| e.to_string())
                .and_then(|bytes| load(&candidate.path, &bytes, source, Some(assets), cancel, &mut ready));
            match result {
                Ok(m) => {
                    loaded = Some(m);
                    break;
                }
                Err(e) => attempt_errors.push(format!("{}: {e}", candidate.path)),
            }
        }
        loaded
    } else {
        None
    };
    warnings.extend(attempt_errors);
    Ok(Found { complete: true, candidates, loaded, root, warnings })
}

fn load(path: &str, bytes: &[u8], source: &str, assets: Option<&ClientAssets>, cancel: &AtomicBool, ready: &mut impl FnMut(&Loaded)) -> Result<Loaded, String> {
    let model = Arc::new(photocraft_c3::decode(bytes).map_err(|e| e.to_string())?);
    let source_base = basename(source);
    let visible = visibility(&model);
    let mut selected: Vec<_> = model.meshes.iter().zip(&visible).map(|(m, v)| basename(&m.texture) == source_base && *v).collect();
    if !selected.iter().any(|v| *v) {
        let body = model.meshes.iter().position(|m| m.name.eq_ignore_ascii_case("v_body"));
        let primary = body
            .or_else(|| model.meshes.iter().enumerate().filter(|(i, _)| visible.get(*i) == Some(&true)).max_by_key(|(_, m)| m.indices.len()).map(|(i, _)| i));
        if let Some(value) = primary.and_then(|i| selected.get_mut(i)) {
            *value = true;
        }
    }
    ready(&Loaded {
        model: model.clone(),
        textures: (0..model.meshes.len()).map(|_| None).collect(),
        selected: selected.clone(),
        visible: visible.clone(),
        path: path.into(),
        warnings: model.warnings.clone(),
    });
    let mut textures = Vec::new();
    let mut warnings = model.warnings.clone();
    let mut texture_bytes = 0usize;
    let mut unresolved_index = None;
    for ((mesh, selected), visible) in model.meshes.iter().zip(&selected).zip(&visible) {
        if cancel.load(Ordering::Relaxed) {
            return Err("cancelled".into());
        }
        if mesh.indices.is_empty() || !*visible {
            textures.push(None);
            continue;
        }
        let image = (|| {
            let (tex, bytes) = if *selected && Path::new(source).is_file() {
                (source.to_string(), read_bounded(Path::new(source), 32 * 1024 * 1024).map_err(|e| e.to_string())?)
            } else if let Some(assets) = assets {
                let index = if assets.texture_path(&mesh.texture).is_none() {
                    if unresolved_index.is_none() {
                        unresolved_index = Some(client_assets(&assets.root, cancel, Level::Full).map_err(|e| e.to_string())?);
                    }
                    unresolved_index.as_deref().unwrap_or(assets)
                } else {
                    assets
                };
                let tex = index.texture_path(&mesh.texture).ok_or_else(|| format!("{}: other-part texture was not found: {}", mesh.name, mesh.texture))?;
                let bytes = index.read(&tex, 32 * 1024 * 1024).map_err(|e| e.to_string())?;
                (tex, bytes)
            } else {
                // Exporter paths may be absolute on another machine. Only look
                // beside the explicitly selected model, using a basename.
                let tex = Path::new(path).parent().unwrap_or_else(|| Path::new(".")).join(basename(&mesh.texture));
                let bytes = read_bounded(&tex, 32 * 1024 * 1024).map_err(|e| e.to_string())?;
                (tex.to_string_lossy().into_owned(), bytes)
            };
            let options = photocraft_codecs::DecodeOptions {
                limits: photocraft_codecs::Limits { max_width: 2048, max_height: 2048, max_pixels: 4 * 1024 * 1024, max_alloc: 64 * 1024 * 1024 },
                ..Default::default()
            };
            let image = photocraft_codecs::decode_with(&bytes, &options).map_err(|e| format!("{tex}: {e}"))?;
            let size = [image.width(), image.height()];
            let pixels = image.to_rgba8();
            texture_bytes = texture_bytes.saturating_add(pixels.len());
            if texture_bytes > 64 * 1024 * 1024 {
                return Err("Other-part textures exceed the 64 MiB preview budget".into());
            }
            Ok(Image { size, pixels })
        })();
        match image {
            Ok(i) => textures.push(Some(i)),
            Err(e) => {
                warnings.push(e);
                textures.push(Some(Image { size: [1, 1], pixels: vec![180, 180, 180, 255] }));
            }
        }
    }
    Ok(Loaded { model, textures, selected, visible, path: path.into(), warnings })
}
