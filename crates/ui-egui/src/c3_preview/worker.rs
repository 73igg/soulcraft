use photocraft_c3::assets::{ClientAssets, catalog_root};
use std::{
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, OnceLock, PoisonError,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
};

#[derive(Clone, Copy, PartialEq, PartialOrd)]
pub(super) enum Level {
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
pub(super) fn client_assets(root: &Path, cancel: &AtomicBool, level: Level) -> Result<Arc<ClientAssets>, photocraft_c3::Error> {
    let root = catalog_root(root).unwrap_or_else(|| root.to_path_buf()).canonicalize().map_err(|e| photocraft_c3::Error(e.to_string()))?;
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

pub(super) use super::scene_worker::discover;
