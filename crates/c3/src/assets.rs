//! Read-only client asset discovery. INI associations take precedence over
//! filenames and exporter texture strings. Paths never escape the chosen root.
use crate::{Error, MAX_FILE_BYTES, Result};
use std::{
    collections::{HashMap, HashSet},
    fs,
    io::Read,
    path::{Component, Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
};

#[derive(Clone, Debug)]
pub struct Candidate {
    pub path: String,
    pub reason: String,
    pub rank: u8,
}

pub struct ClientAssets {
    pub root: PathBuf,
    files: HashMap<String, PathBuf>,
    associations: HashMap<String, Vec<Candidate>>,
    archives: Vec<crate::archive::LazyArchive>,
    models: HashSet<String>,
    pub warnings: Vec<String>,
}

pub fn norm(path: &str) -> String {
    path.replace('\\', "/").trim().to_lowercase()
}
pub fn basename(path: &str) -> String {
    norm(path).rsplit('/').next().unwrap_or_default().into()
}
fn id(value: &str) -> String {
    let value = value.trim();
    if value.bytes().all(|b| b.is_ascii_digit()) {
        let v = value.trim_start_matches('0');
        if v.is_empty() { "0".into() } else { v.into() }
    } else {
        value.to_lowercase()
    }
}
fn catalog_name(name: &str) -> bool {
    matches!(
        name,
        "3dobj.ini"
            | "3deffectobj.ini"
            | "3dtexture.ini"
            | "3deffect.ini"
            | "armor.ini"
            | "weapon.ini"
            | "3dsimpleobj.ini"
            | "3dflyingobj.ini"
            | "hair.ini"
            | "head.ini"
            | "mount.ini"
            | "armet.ini"
            | "misc.ini"
            | "mantle.ini"
    )
}
fn sorted_candidates(mut result: Vec<Candidate>) -> Vec<Candidate> {
    result.sort_by(|a, b| (a.rank, &a.path).cmp(&(b.rank, &b.path)));
    let mut seen = HashSet::new();
    result.retain(|c| seen.insert(c.path.clone()));
    result.truncate(512);
    result
}

fn relative(path: &str) -> Result<String> {
    let path = norm(path);
    if path.is_empty() || path.len() > 4096 || Path::new(&path).components().any(|c| !matches!(c, Component::Normal(_))) {
        return Err(Error("asset path must stay within the client folder".into()));
    }
    Ok(path)
}

pub fn read_bounded(path: &Path, limit: usize) -> Result<Vec<u8>> {
    let f = fs::File::open(path).map_err(|e| Error(format!("{}: {e}", path.display())))?;
    if f.metadata().map_err(|e| Error(e.to_string()))?.len() > limit as u64 {
        return Err(Error(format!("{} exceeds the read limit", path.display())));
    }
    let mut data = Vec::new();
    f.take(limit as u64 + 1).read_to_end(&mut data).map_err(|e| Error(e.to_string()))?;
    if data.len() > limit {
        return Err(Error("file grew beyond the read limit".into()));
    }
    Ok(data)
}

/// Infer a client folder only among a texture's ancestors, never the whole disk.
pub fn infer_root(texture: &Path) -> Option<PathBuf> {
    if !texture.is_absolute() {
        return texture.canonicalize().ok().and_then(|p| infer_root(&p));
    }
    for root in texture.parent()?.ancestors().take(12) {
        if root.join("ini").is_dir() && root.join("c3").is_dir() {
            return Some(root.into());
        }
        if root.file_name().is_some_and(|n| n.eq_ignore_ascii_case("c3")) {
            return root.parent().map(Path::to_path_buf);
        }
    }
    texture.parent().filter(|p| !p.as_os_str().is_empty()).map(Path::to_path_buf)
}

#[derive(Default)]
struct Ini {
    flat: HashMap<String, String>,
    sections: Vec<HashMap<String, String>>,
}
fn parse_ini(bytes: &[u8]) -> Result<Ini> {
    let text = match std::str::from_utf8(bytes) {
        Ok(t) => t.into(),
        Err(_) => encoding_rs::GBK.decode(bytes).0,
    };
    let mut out = Ini::default();
    let mut section = HashMap::new();
    let mut entries = 0;
    for line in text.lines() {
        if line.len() > 16_384 {
            return Err(Error("INI line exceeds 16 KiB".into()));
        }
        let line = line.trim().trim_start_matches('\u{feff}');
        if line.starts_with([';', '#']) || line.is_empty() {
            continue;
        }
        if line.starts_with('[') {
            if !section.is_empty() {
                out.sections.push(std::mem::take(&mut section));
            }
        } else if let Some((key, value)) = line.split_once('=') {
            entries += 1;
            if entries > 500_000 {
                return Err(Error("INI exceeds the entry budget".into()));
            }
            let key = id(key);
            let value: String = value.trim().trim_matches('"').into();
            section.insert(key.clone(), value.clone());
            // Registries use flat numerical keys. Section fields belong only to
            // their section; retaining them globally could cross-pair unrelated parts.
            if key.bytes().all(|b| b.is_ascii_digit()) {
                out.flat.insert(key, value);
            }
        }
    }
    if !section.is_empty() {
        out.sections.push(section);
    }
    Ok(out)
}

impl ClientAssets {
    /// Selected meshes need direct file/package lookup, not a client-wide scan.
    pub fn loose(root: &Path, cancel: &AtomicBool) -> Result<Self> {
        if cancel.load(Ordering::Relaxed) {
            return Err(Error("cancelled".into()));
        }
        let root = root.canonicalize().map_err(|e| Error(format!("client folder: {e}")))?;
        if !root.is_dir() {
            return Err(Error("choose a client folder".into()));
        }
        let mut out = Self {
            root: root.clone(),
            files: HashMap::new(),
            associations: HashMap::new(),
            archives: Vec::new(),
            models: HashSet::new(),
            warnings: Vec::new(),
        };
        let mut packages = Vec::new();
        for (count, entry) in fs::read_dir(&root).map_err(|e| Error(e.to_string()))?.enumerate() {
            if count >= 300_000 {
                return Err(Error("client root exceeds the entry budget".into()));
            }
            if cancel.load(Ordering::Relaxed) {
                return Err(Error("cancelled".into()));
            }
            let entry = entry.map_err(|e| Error(e.to_string()))?;
            if entry.path().extension().is_some_and(|s| s.eq_ignore_ascii_case("dnp")) {
                let ty = entry.file_type().map_err(|e| Error(e.to_string()))?;
                if ty.is_file() && !ty.is_symlink() {
                    packages.push(entry.path());
                }
            }
        }
        packages.sort();
        out.archives = packages.into_iter().take(16).map(crate::archive::LazyArchive::new).collect();
        Ok(out)
    }

    /// Catalog associations can be built without enumerating thousands of assets.
    pub fn quick(root: &Path, cancel: &AtomicBool) -> Result<Self> {
        let mut out = Self::loose(root, cancel)?;
        let mut ini_paths = Vec::new();
        for folder in [out.root.join("ini"), out.root.join("decomp/ini"), out.root.clone()] {
            if let Ok(resolved) = folder.canonicalize()
                && !resolved.starts_with(&out.root)
            {
                return Err(Error("catalog directory resolved outside the client folder".into()));
            }
            let entries = match fs::read_dir(&folder) {
                Ok(entries) => entries,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                Err(e) => return Err(Error(e.to_string())),
            };
            for (count, entry) in entries.enumerate() {
                if count >= 300_000 {
                    return Err(Error("catalog folder exceeds the entry budget".into()));
                }
                if cancel.load(Ordering::Relaxed) {
                    return Err(Error("cancelled".into()));
                }
                let entry = entry.map_err(|e| Error(e.to_string()))?;
                let name = norm(&entry.file_name().to_string_lossy());
                if catalog_name(&name) {
                    let ty = entry.file_type().map_err(|e| Error(e.to_string()))?;
                    if ty.is_file() && !ty.is_symlink() {
                        ini_paths.push((name, entry.path()));
                    }
                }
            }
        }
        out.read_catalogs(ini_paths, cancel)?;
        Ok(out)
    }

    pub fn build(root: &Path, cancel: &AtomicBool) -> Result<Self> {
        let root = root.canonicalize().map_err(|e| Error(format!("client folder: {e}")))?;
        if !root.is_dir() {
            return Err(Error("choose a client folder".into()));
        }
        let mut out = Self {
            root: root.clone(),
            files: HashMap::new(),
            associations: HashMap::new(),
            archives: Vec::new(),
            models: HashSet::new(),
            warnings: Vec::new(),
        };
        let mut stack = vec![(root.clone(), 0usize)];
        let mut seen = 0usize;
        let has_c3 = root.join("c3").is_dir();
        while let Some((dir, depth)) = stack.pop() {
            if cancel.load(Ordering::Relaxed) {
                return Err(Error("cancelled".into()));
            }
            if depth > 24 {
                out.warnings.push("folder depth limit reached".into());
                continue;
            }
            let entries = fs::read_dir(&dir).map_err(|e| Error(format!("{}: {e}", dir.display())))?;
            for entry in entries {
                if cancel.load(Ordering::Relaxed) {
                    return Err(Error("cancelled".into()));
                }
                let entry = entry.map_err(|e| Error(e.to_string()))?;
                let ty = entry.file_type().map_err(|e| Error(e.to_string()))?;
                if ty.is_symlink() {
                    continue;
                }
                seen += 1;
                if seen > 300_000 {
                    return Err(Error("client discovery exceeds 300,000 entries; choose a smaller asset folder".into()));
                }
                let path = entry.path();
                let rel = norm(&path.strip_prefix(&root).map_err(|e| Error(e.to_string()))?.to_string_lossy());
                if ty.is_dir() {
                    // At the root read only asset/catalog trees, unless this is a
                    // standalone folder without c3. Never crawl unrelated clients.
                    if depth != 0 || !has_c3 || matches!(rel.as_str(), "c3" | "ini" | "decomp") {
                        stack.push((path, depth + 1));
                    }
                } else if ty.is_file()
                    && path
                        .extension()
                        .and_then(|s| s.to_str())
                        .is_some_and(|s| matches!(s.to_ascii_lowercase().as_str(), "c3" | "dds" | "tga" | "png" | "jpg" | "bmp" | "ini" | "dnp"))
                {
                    out.files.insert(rel, path);
                }
            }
        }
        let ini_paths: Vec<_> =
            out.files.iter().filter(|(k, _)| k.ends_with(".ini") && (k.starts_with("ini/") || !k.contains('/'))).map(|(k, p)| (k.clone(), p.clone())).collect();
        let mut archives: Vec<_> = out.files.iter().filter(|(k, _)| k.ends_with(".dnp") && !k.contains('/')).map(|(k, p)| (k.clone(), p.clone())).collect();
        archives.sort();
        for (name, path) in archives.into_iter().take(16) {
            if cancel.load(Ordering::Relaxed) {
                return Err(Error("cancelled".into()));
            }
            let _ = name;
            out.archives.push(crate::archive::LazyArchive::new(path));
        }
        out.read_catalogs(ini_paths, cancel)?;
        Ok(out)
    }

    fn read_catalogs(&mut self, ini_paths: Vec<(String, PathBuf)>, cancel: &AtomicBool) -> Result<()> {
        let mut catalogs = Vec::new();
        let mut total = 0;
        for (key, path) in ini_paths {
            if cancel.load(Ordering::Relaxed) {
                return Err(Error("cancelled".into()));
            }
            // Avoid loading unrelated enormous gameplay tables.
            let name = basename(&key);
            if !catalog_name(&name) {
                continue;
            }
            let bytes = read_bounded(&path, 32 * 1024 * 1024)?;
            total += bytes.len();
            if total > 128 * 1024 * 1024 {
                return Err(Error("client INIs exceed the discovery budget".into()));
            }
            catalogs.push((name, parse_ini(&bytes)?));
        }
        let flat = |name: &str| catalogs.iter().find(|(k, _)| k == name).map(|(_, v)| v.flat.clone()).unwrap_or_default();
        let textures = flat("3dtexture.ini");
        let objects = flat("3dobj.ini");
        let effects = flat("3deffectobj.ini");
        self.models = objects.values().chain(effects.values()).filter_map(|p| relative(p).ok()).filter(|p| p.ends_with(".c3")).collect();
        for (mesh_id, mesh) in &objects {
            if let Some(tex) = textures.get(mesh_id) {
                self.add(tex, mesh, "shared client resource ID", 1);
            }
        }
        for (name, ini) in &catalogs {
            for fields in &ini.sections {
                for (key, texture_id) in fields {
                    let (mesh_key, registry) = if let Some(suffix) = key.strip_prefix("textureid") {
                        (format!("effectid{suffix}"), &effects)
                    } else if let Some(suffix) = key.strip_prefix("texture") {
                        (format!("mesh{suffix}"), &objects)
                    } else {
                        continue;
                    };
                    if let Some(mesh_id) = fields.get(&mesh_key)
                        && let (Some(mesh), Some(tex)) = (registry.get(&id(mesh_id)), textures.get(&id(texture_id)))
                    {
                        self.add(tex, mesh, &format!("{name}: {mesh_key}/{key}"), 0);
                    }
                }
            }
        }
        Ok(())
    }

    fn add(&mut self, tex: &str, mesh: &str, reason: &str, rank: u8) {
        let (Ok(tex), Ok(mesh)) = (relative(tex), relative(mesh)) else {
            return;
        };
        if !mesh.ends_with(".c3") {
            return;
        }
        let list = self.associations.entry(tex).or_default();
        if let Some(old) = list.iter_mut().find(|c| c.path == mesh) {
            if rank < old.rank {
                old.rank = rank;
                old.reason = reason.into();
            }
        } else if list.len() < 512 {
            list.push(Candidate { path: mesh, reason: reason.into(), rank });
        }
    }

    pub fn candidates(&self, texture: &str, cancel: &AtomicBool) -> Result<Vec<Candidate>> {
        let tex = norm(texture);
        let base = basename(texture);
        let texture_path = Path::new(texture);
        let canonical = texture_path.canonicalize().ok().or_else(|| Some(texture_path.parent()?.canonicalize().ok()?.join(texture_path.file_name()?)));
        let rel = canonical
            .as_deref()
            .unwrap_or_else(|| Path::new(texture))
            .strip_prefix(&self.root)
            .ok()
            .map(|p| norm(&p.to_string_lossy()).trim_start_matches("decomp/").to_string());
        let mut result = Vec::new();
        for (path, entries) in &self.associations {
            if Some(path) == rel.as_ref() || path == &tex {
                result.extend(entries.iter().cloned());
            }
        }
        if result.is_empty() {
            for (path, entries) in &self.associations {
                if basename(path) == base {
                    result.extend(entries.iter().cloned().map(|mut c| {
                        c.reason = format!("Texture filename match: {} ({path})", c.reason);
                        c.rank = c.rank.saturating_add(4);
                        c
                    }));
                }
            }
        }
        if !result.is_empty() {
            return Ok(sorted_candidates(result));
        }
        let stem = Path::new(&base).file_stem().and_then(|s| s.to_str()).unwrap_or_default();
        let mut model_paths: Vec<_> = self.files.keys().filter(|p| p.ends_with(".c3")).collect();
        model_paths.sort();
        for folder in ["mesh", "weapon", "hair", "head", "mantle", "npc", "monster"] {
            let path = format!("c3/{folder}/{stem}.c3");
            if self.loose_path(&path).is_some() || self.archives.iter().any(|a| a.contains(&path)) {
                result.push(Candidate { path, reason: "matching filename".into(), rank: 2 });
            }
        }
        for path in &model_paths {
            if Path::new(path).file_stem().and_then(|s| s.to_str()) == Some(stem) {
                result.push(Candidate { path: (*path).clone(), reason: "matching filename".into(), rank: 2 });
            }
        }
        if result.is_empty() {
            let mut scanned = 0usize;
            for path in model_paths {
                if cancel.load(Ordering::Relaxed) {
                    return Err(Error("cancelled".into()));
                }
                if scanned > 512 * 1024 * 1024 {
                    return Err(Error("embedded-reference search reached 512 MiB; select a mesh manually".into()));
                }
                let Some(file) = self.files.get(path) else {
                    continue;
                };
                // Animation-only files can be very large; do not scan them.
                let Ok(bytes) = read_bounded(file, 16 * 1024 * 1024) else {
                    continue;
                };
                scanned += bytes.len();
                if crate::texture_references(&bytes).is_ok_and(|refs| refs.iter().any(|t| basename(t) == base)) {
                    result.push(Candidate { path: path.clone(), reason: "embedded texture reference".into(), rank: 3 });
                }
            }
        }
        Ok(sorted_candidates(result))
    }

    fn loose_path(&self, key: &str) -> Option<PathBuf> {
        let key = relative(key).ok()?;
        if let Some(file) = self.files.get(&key).or_else(|| self.files.get(&format!("decomp/{key}"))) {
            return Some(file.clone());
        }
        for key in [&key, &format!("decomp/{key}")] {
            let path = self.root.join(key);
            if path.is_file() {
                return Some(path);
            }
            // Windows clients use case-insensitive resource names. Preserve that
            // behavior on other hosts without scanning the complete client.
            if !cfg!(windows) {
                let mut path = self.root.clone();
                let found = key.split('/').all(|part| {
                    let Ok(entries) = fs::read_dir(&path) else { return false };
                    let Some(entry) = entries.take(300_000).filter_map(std::result::Result::ok).find(|e| e.file_name().eq_ignore_ascii_case(part)) else {
                        return false;
                    };
                    if entry.file_type().is_ok_and(|t| t.is_symlink()) {
                        return false;
                    }
                    path = entry.path();
                    true
                });
                if found && path.is_file() {
                    return Some(path);
                }
            }
        }
        None
    }

    pub fn read(&self, relative_path: &str, limit: usize) -> Result<Vec<u8>> {
        let path = relative(relative_path)?;
        let Some(file) = self.loose_path(&path) else {
            let mut archive_error = None;
            for archive in &self.archives {
                match archive.read(&path, limit.min(MAX_FILE_BYTES)) {
                    Ok(Some(bytes)) => return Ok(bytes),
                    Ok(None) => {}
                    Err(e) => archive_error = Some(e.to_string()),
                }
            }
            return Err(Error(format!(
                "client asset is unavailable: {path}; unpack its archive or load another mesh{}",
                archive_error.map(|e| format!(" ({e})")).unwrap_or_default()
            )));
        };
        // Reject a link swapped in after discovery, and resolve containment again.
        let resolved = file.canonicalize().map_err(|e| Error(e.to_string()))?;
        if !resolved.starts_with(&self.root) {
            return Err(Error("asset resolved outside the client folder".into()));
        }
        read_bounded(&resolved, limit.min(MAX_FILE_BYTES))
    }

    pub fn texture_path(&self, embedded: &str) -> Option<String> {
        let key = norm(embedded);
        if self.loose_path(&key).is_some() {
            return Some(key);
        }
        if relative(&key).is_ok() && self.archives.iter().any(|a| a.contains(&key)) {
            return Some(key);
        }
        let base = basename(embedded);
        let conventional = format!("c3/texture/{base}");
        if self.loose_path(&conventional).is_some() {
            return Some(conventional);
        }
        if self.archives.iter().any(|a| a.contains(&conventional)) {
            return Some(conventional);
        }
        if self.loose_path(&base).is_some() {
            return Some(base);
        }
        let mut options = self.files.keys().filter(|p| basename(p) == base);
        let one = options.next()?;
        options.next().is_none().then(|| one.clone())
    }
    pub fn model_paths(&self) -> Vec<String> {
        let mut paths: HashSet<_> = self.files.keys().filter(|p| p.ends_with(".c3")).cloned().collect();
        paths.extend(self.models.iter().cloned());
        let mut paths: Vec<_> = paths.into_iter().collect();
        paths.sort();
        paths
    }
}
