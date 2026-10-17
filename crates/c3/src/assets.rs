//! Read-only client asset discovery. INI associations take precedence over
//! filenames and exporter texture strings. Paths never escape the chosen root.
use crate::{Error, MAX_FILE_BYTES, Result};
mod effects;
mod role_parts;
mod wdb;
pub use effects::EffectBinding;
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

/// One INI object, retaining its sibling resources instead of treating them as
/// alternative models for the same texture.
#[derive(Clone, Debug)]
pub struct ObjectDefinition {
    pub id: String,
    pub label: String,
    pub parts: Vec<ObjectPart>,
}
#[derive(Clone, Debug)]
pub struct ObjectPart {
    pub slot: usize,
    pub binding: String,
    pub mesh: Option<String>,
    pub texture: Option<String>,
    pub note: Option<String>,
}

pub struct ClientAssets {
    pub root: PathBuf,
    files: HashMap<String, PathBuf>,
    associations: HashMap<String, Vec<Candidate>>,
    archives: Vec<crate::archive::LazyArchive>,
    packed: crate::packed::Packed,
    models: HashSet<String>,
    objects: Vec<ObjectDefinition>,
    object_textures: HashMap<String, Vec<usize>>,
    effects: effects::Catalog,
    pub warnings: Vec<String>,
}

pub fn norm(path: &str) -> String {
    path.replace('\\', "/").trim().to_lowercase()
}
pub fn basename(path: &str) -> String {
    norm(path).rsplit('/').next().unwrap_or_default().into()
}
/// Extraction directories retain the client's logical resource names.
pub fn resource_key(path: &str) -> String {
    let key = norm(path);
    key.strip_prefix("decomp/").or_else(|| key.strip_prefix("c3.extracted/")).unwrap_or(&key).to_string()
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
            | "3dsimpleobjex.ini"
            | "3dflyingobj.ini"
            | "hair.ini"
            | "head.ini"
            | "mount.ini"
            | "armet.ini"
            | "misc.ini"
            | "mantle.ini"
            | "parteffect.ini"
            | "roleparteffect.ini"
            | "roleweapon3deffect.ini"
            | "action3deffect.ini"
            | "action3deffect_new.ini"
            | "3deffect3.ini"
    )
}
fn sorted_candidates(mut result: Vec<Candidate>) -> Vec<Candidate> {
    result.sort_by(|a, b| (a.rank, &a.path).cmp(&(b.rank, &b.path)));
    let mut seen = HashSet::new();
    result.retain(|c| seen.insert(c.path.clone()));
    result.truncate(512);
    result
}

pub(crate) fn relative(path: &str) -> Result<String> {
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

/// A decompiled c3 tree can have its registries in the parent client. Search
/// only a bounded ancestor chain, without enumerating unrelated client trees.
fn client_layout(root: &Path) -> bool {
    (root.join("ini").is_dir() || root.join("decomp/ini").is_dir())
        && (root.join("c3").is_dir()
            || root.join("decomp/c3").is_dir()
            || root.join("c3.tpi").is_file()
            || root.join("ini/c3.wdb").is_file()
            || root.join("decomp/ini/c3.wdb").is_file())
}
pub fn catalog_root(folder: &Path) -> Option<PathBuf> {
    let folder = folder.canonicalize().ok()?;
    let root = folder.ancestors().take(12).find(|root| client_layout(root)).map(Path::to_path_buf)?;
    // An unpacked snapshot can contain stale INIs as well as assets. The live
    // client directly above its literal decomp folder remains authoritative.
    if root.file_name().is_some_and(|name| name.eq_ignore_ascii_case("decomp"))
        && let Some(parent) = root.parent().filter(|parent| client_layout(parent))
    {
        return Some(parent.to_path_buf());
    }
    Some(root)
}

/// Infer a client folder only among a texture's ancestors, never the whole disk.
pub fn infer_root(texture: &Path) -> Option<PathBuf> {
    if !texture.is_absolute() {
        return texture.canonicalize().ok().and_then(|p| infer_root(&p));
    }
    if let Some(root) = catalog_root(texture.parent()?) {
        return Some(root);
    }
    for root in texture.parent()?.ancestors().take(12) {
        if root.file_name().is_some_and(|n| n.eq_ignore_ascii_case("c3")) {
            return root.parent().map(Path::to_path_buf);
        }
    }
    texture.parent().filter(|p| !p.as_os_str().is_empty()).map(Path::to_path_buf)
}

#[derive(Default)]
struct Ini {
    flat: HashMap<String, String>,
    sections: Vec<IniSection>,
}
struct IniSection {
    name: String,
    fields: HashMap<String, String>,
}
fn parse_ini(bytes: &[u8]) -> Result<Ini> {
    let text = match std::str::from_utf8(bytes) {
        Ok(t) => t.into(),
        Err(_) => encoding_rs::GBK.decode(bytes).0,
    };
    let mut out = Ini::default();
    let mut section = HashMap::new();
    let mut section_name = String::new();
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
                out.sections.push(IniSection { name: section_name.clone(), fields: std::mem::take(&mut section) });
            }
            section_name = line.trim_start_matches('[').split(']').next().unwrap_or_default().trim().into();
        } else if let Some((key, value)) = line.split_once('=') {
            entries += 1;
            if entries > 1_000_000 {
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
        out.sections.push(IniSection { name: section_name, fields: section });
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
            packed: crate::packed::Packed::new(root.clone()),
            models: HashSet::new(),
            objects: Vec::new(),
            object_textures: HashMap::new(),
            effects: effects::Catalog::default(),
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
            if entry.path().extension().is_some_and(|s| s.eq_ignore_ascii_case("dnp") || s.eq_ignore_ascii_case("wdf")) {
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
        let root = catalog_root(root).unwrap_or_else(|| root.to_path_buf());
        let mut out = Self::loose(&root, cancel)?;
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
        let inferred = catalog_root(root);
        let root = inferred.as_deref().unwrap_or(root);
        let root = root.canonicalize().map_err(|e| Error(format!("client folder: {e}")))?;
        if !root.is_dir() {
            return Err(Error("choose a client folder".into()));
        }
        let mut out = Self {
            root: root.clone(),
            files: HashMap::new(),
            associations: HashMap::new(),
            archives: Vec::new(),
            packed: crate::packed::Packed::new(root.clone()),
            models: HashSet::new(),
            objects: Vec::new(),
            object_textures: HashMap::new(),
            effects: effects::Catalog::default(),
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
                    if depth != 0 || !has_c3 || matches!(rel.as_str(), "c3" | "ini" | "decomp" | "c3.extracted") {
                        stack.push((path, depth + 1));
                    }
                } else if ty.is_file()
                    && path
                        .extension()
                        .and_then(|s| s.to_str())
                        .is_some_and(|s| matches!(s.to_ascii_lowercase().as_str(), "c3" | "dds" | "tga" | "png" | "jpg" | "bmp" | "ini" | "dnp" | "wdf"))
                {
                    out.files.insert(rel, path);
                }
            }
        }
        let ini_paths: Vec<_> = out
            .files
            .iter()
            .filter(|(k, _)| k.ends_with(".ini") && (k.starts_with("ini/") || k.starts_with("decomp/ini/") || !k.contains('/')))
            .map(|(k, p)| (k.clone(), p.clone()))
            .collect();
        let mut archives: Vec<_> =
            out.files.iter().filter(|(k, _)| (k.ends_with(".dnp") || k.ends_with(".wdf")) && !k.contains('/')).map(|(k, p)| (k.clone(), p.clone())).collect();
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

    fn read_catalogs(&mut self, mut ini_paths: Vec<(String, PathBuf)>, cancel: &AtomicBool) -> Result<()> {
        ini_paths.sort_by_key(|(_, path)| {
            path.strip_prefix(&self.root).ok().map(|p| {
                let key = norm(&p.to_string_lossy());
                (key.starts_with("decomp/"), !key.starts_with("ini/"), key)
            })
        });
        let mut catalogs = Vec::new();
        let mut seen_catalogs = HashSet::new();
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
            if !seen_catalogs.insert(name.clone()) {
                continue;
            }
            let bytes = match read_bounded(&path, 32 * 1024 * 1024) {
                Ok(bytes) => bytes,
                Err(error) => {
                    self.warnings.push(format!("Skipped {name}: {error}"));
                    continue;
                }
            };
            total += bytes.len();
            if total > 128 * 1024 * 1024 {
                self.warnings.push("Remaining client INIs exceed the discovery budget".into());
                break;
            }
            match if name == "roleparteffect.ini" { role_parts::parse(&bytes) } else { parse_ini(&bytes) } {
                Ok(ini) => catalogs.push((name, ini)),
                Err(error) => self.warnings.push(format!("Skipped {name}: {error}")),
            }
        }
        // WDB clients keep the same resource catalogs as binary DBC tables.
        // Explicit loose catalogs remain overrides; absent ones come from WDB.
        for folder in [self.root.join("ini"), self.root.join("decomp/ini")] {
            let path = folder.join("c3.wdb");
            if !path.is_file() {
                continue;
            }
            let tables = (|| {
                let path = path.canonicalize().map_err(|error| Error(error.to_string()))?;
                if !path.starts_with(&self.root) {
                    return Err(Error("WDB resolved outside the client folder".into()));
                }
                wdb::read(&read_bounded(&path, MAX_FILE_BYTES)?, cancel)
            })();
            match tables {
                Ok(tables) => {
                    for (name, ini) in tables {
                        if let Some((_, existing)) = catalogs.iter_mut().find(|(key, _)| *key == name) {
                            for (key, value) in ini.flat {
                                existing.flat.entry(key).or_insert(value);
                            }
                            let mut sections = existing.sections.iter().map(|section| section.name.to_lowercase()).collect::<HashSet<_>>();
                            existing.sections.extend(ini.sections.into_iter().filter(|section| sections.insert(section.name.to_lowercase())));
                        } else {
                            catalogs.push((name, ini));
                        }
                    }
                }
                Err(error) => self.warnings.push(format!("Skipped c3.wdb: {error}")),
            }
            break;
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
            for section in &ini.sections {
                let fields = &section.fields;
                let mut parts = Vec::new();
                for (key, texture_id) in fields {
                    let (mesh_key, registry) = if let Some(suffix) = key.strip_prefix("textureid") {
                        (format!("effectid{suffix}"), &effects)
                    } else if let Some(suffix) = key.strip_prefix("texture") {
                        (format!("mesh{suffix}"), &objects)
                    } else {
                        continue;
                    };
                    if let Some(mesh_id) = fields.get(&mesh_key) {
                        let suffix = key.trim_start_matches("textureid").trim_start_matches("texture");
                        let slot = if suffix.is_empty() { Some(0) } else { suffix.parse::<usize>().ok() };
                        let count_key = if key.starts_with("textureid") { "amount" } else { "part" };
                        let limit = fields.get(count_key).and_then(|value| value.parse::<usize>().ok()).unwrap_or(64).min(64);
                        if let Some(slot) = slot.filter(|slot| *slot < limit) {
                            let mesh = registry.get(&id(mesh_id)).and_then(|path| relative(path).ok()).filter(|p| p.ends_with(".c3"));
                            let texture = textures.get(&id(texture_id)).and_then(|path| relative(path).ok());
                            let note = if mesh.is_none() || texture.is_none() {
                                Some(format!("{name} [{}] part {slot}: unresolved mesh {mesh_id} or texture {texture_id}", section.name))
                            } else {
                                None
                            };
                            parts.push(ObjectPart { slot, binding: mesh_key.clone(), mesh, texture, note });
                        }
                    }
                    if let Some(mesh_id) = fields.get(&mesh_key)
                        && let (Some(mesh), Some(tex)) = (registry.get(&id(mesh_id)), textures.get(&id(texture_id)))
                    {
                        self.add(tex, mesh, &format!("{name}: {mesh_key}/{key}"), 0);
                    }
                }
                // A missing TextureN must not silently remove its C3 geometry.
                for (key, mesh_id) in fields {
                    let (suffix, registry, count_key) = if let Some(suffix) = key.strip_prefix("mesh") {
                        (suffix, &objects, "part")
                    } else if let Some(suffix) = key.strip_prefix("effectid") {
                        (suffix, &effects, "amount")
                    } else {
                        continue;
                    };
                    let slot = if suffix.is_empty() { Some(0) } else { suffix.parse::<usize>().ok() };
                    let limit = fields.get(count_key).and_then(|value| value.parse::<usize>().ok()).unwrap_or(64).min(64);
                    if let Some(slot) = slot.filter(|slot| *slot < limit)
                        && !parts.iter().any(|part| part.binding == *key)
                    {
                        let mesh = registry.get(&id(mesh_id)).and_then(|path| relative(path).ok()).filter(|path| path.ends_with(".c3"));
                        parts.push(ObjectPart {
                            slot,
                            binding: key.clone(),
                            mesh,
                            texture: None,
                            note: Some(format!("{name} [{}] part {slot}: missing texture reference for {key}={mesh_id}", section.name)),
                        });
                    }
                }
                for (prefix, texture_prefix, count_key) in [("mesh", "texture", "part"), ("effectid", "textureid", "amount")] {
                    if !parts.iter().any(|part| part.binding.starts_with(prefix)) {
                        continue;
                    }
                    let Some(count) = fields.get(count_key).and_then(|value| value.parse::<usize>().ok()) else { continue };
                    for slot in 0..count.min(64) {
                        if parts.iter().any(|part| part.slot == slot && part.binding.starts_with(prefix)) {
                            continue;
                        }
                        let binding = format!("{prefix}{slot}");
                        let texture =
                            fields.get(&format!("{texture_prefix}{slot}")).and_then(|value| textures.get(&id(value))).and_then(|path| relative(path).ok());
                        parts.push(ObjectPart {
                            slot,
                            binding: binding.clone(),
                            mesh: None,
                            texture,
                            note: Some(format!("{name} [{}] part {slot}: missing {binding} reference", section.name)),
                        });
                    }
                }
                parts.sort_by_key(|part| (part.slot, part.binding.clone()));
                parts.truncate(64);
                if !parts.is_empty() && self.objects.len() < 100_000 {
                    let index = self.objects.len();
                    let mut seen = HashSet::new();
                    for part in &parts {
                        if let Some(texture) = &part.texture
                            && seen.insert(texture.clone())
                        {
                            self.object_textures.entry(texture.clone()).or_default().push(index);
                        }
                    }
                    self.objects.push(ObjectDefinition { id: format!("{name}:{}", section.name), label: format!("{name} [{}]", section.name), parts });
                }
            }
        }
        self.effects = effects::Catalog::read(&catalogs, &textures);
        for (index, object) in self.objects.iter().enumerate() {
            for texture in self.effects.textures(&object.id) {
                let matches = self.object_textures.entry(texture.to_string()).or_default();
                if !matches.contains(&index) {
                    matches.push(index);
                }
            }
        }
        Ok(())
    }

    pub fn material(&self, object: &str, slot: usize) -> crate::material::Material {
        self.effects.material(object, slot)
    }
    pub fn timing(&self, object: &str) -> crate::material::Timing {
        self.effects.timing(object)
    }
    pub fn part_timing(&self, object: &str, slot: usize) -> crate::material::Timing {
        self.effects.part_timing(object, slot)
    }
    pub fn effect_bindings(&self, object: &ObjectDefinition) -> Vec<EffectBinding> {
        self.effects.bindings(object)
    }
    pub fn effect_object(&self, name: &str) -> Option<&ObjectDefinition> {
        self.effects.definition(name).and_then(|id| self.object(id))
    }

    pub fn object(&self, id: &str) -> Option<&ObjectDefinition> {
        self.objects.iter().find(|object| object.id == id)
    }
    /// All catalog records, including objects stored in a WDB database.
    pub fn object_definitions(&self) -> &[ObjectDefinition] {
        &self.objects
    }
    pub fn objects_for_texture(&self, texture: &str) -> Vec<ObjectDefinition> {
        let canonical = Path::new(texture).canonicalize().ok();
        let relative = canonical.as_deref().and_then(|path| path.strip_prefix(&self.root).ok()).map(|path| resource_key(&path.to_string_lossy()));
        let key = relative.unwrap_or_else(|| norm(texture));
        let mut indices = self.object_textures.get(&key).cloned().unwrap_or_default();
        if indices.is_empty() {
            let base = basename(texture);
            for (path, matches) in &self.object_textures {
                if basename(path) == base {
                    indices.extend(matches);
                }
            }
        }
        let mut alias = None;
        if indices.is_empty() {
            // A registered texture can be unused by TextureN while its paired
            // 3dobj resource is still a catalog mesh (e.g. female claws).
            // Only follow an explicit shared registry ID, never nearby numbers.
            let registered = if self.associations.contains_key(&key) {
                Some(key.clone())
            } else {
                let base = basename(texture);
                let mut keys = self.associations.keys().filter(|path| basename(path) == base);
                keys.next().cloned().filter(|_| keys.next().is_none())
            };
            if let Some(registered) = registered {
                let meshes: HashSet<_> = self
                    .associations
                    .get(&registered)
                    .into_iter()
                    .flatten()
                    .filter(|candidate| candidate.rank == 1)
                    .map(|candidate| candidate.path.clone())
                    .collect();
                if !meshes.is_empty() {
                    indices.extend(
                        self.objects
                            .iter()
                            .enumerate()
                            .filter(|(_, object)| {
                                !object.id.starts_with("3deffect")
                                    && object.parts.iter().any(|part| part.mesh.as_ref().is_some_and(|mesh| meshes.contains(mesh)))
                            })
                            .map(|(index, _)| index),
                    );
                    alias = Some((registered, meshes));
                }
            }
        }
        indices.sort_unstable();
        indices.dedup();
        let mut objects: Vec<_> = indices.into_iter().filter_map(|index| self.objects.get(index)).collect();
        objects.sort_by_cached_key(|object| {
            let enabled = self.effect_bindings(object).iter().any(|binding| binding.enabled);
            // EO's first super-quality row ends in 4. Prefer that registered
            // glow over the basic row when both share a texture. Other client
            // quality schemes retain their first enabled registered variant.
            let super_variant = enabled
                && object.id.strip_prefix("weapon.ini:").is_some_and(|section| {
                    let section = section.to_ascii_lowercase();
                    section.strip_prefix("weapon").unwrap_or(&section).parse::<u32>().is_ok_and(|item| item % 10 == 4)
                });
            (object.id.starts_with("3deffect"), !enabled, !super_variant, object.id.clone())
        });
        objects.truncate(512);
        objects
            .into_iter()
            .map(|object| {
                let mut object = object.clone();
                if let Some((texture, meshes)) = &alias {
                    for part in &mut object.parts {
                        if part.mesh.as_ref().is_some_and(|mesh| meshes.contains(mesh)) {
                            part.texture = Some(texture.clone());
                        }
                    }
                }
                object
            })
            .collect()
    }

    /// A loose editable texture keeps its actual file path. Packed textures are
    /// opened as documents without an overwrite target and use Save As.
    pub fn editable_path(&self, key: &str) -> Option<PathBuf> {
        let path = self.loose_path(key)?.canonicalize().ok()?;
        if path.strip_prefix(&self.root).ok().is_some_and(|p| p.starts_with("decomp") || p.starts_with("c3.extracted")) && self.packed.contains(&norm(key)) {
            return None;
        }
        path.starts_with(&self.root).then_some(path)
    }

    // Opening an explicitly extracted texture selects that extraction's
    // resource generation. A newer packed mesh can have entirely different UVs
    // under the same resource ID. Ordinary client reads retain native priority.
    fn extracted_path(&self, key: &str, source: &Path) -> Result<Option<PathBuf>> {
        let canonical = source.canonicalize().ok();
        let Some(extraction) = ["decomp", "c3.extracted"]
            .into_iter()
            .map(|name| self.root.join(name))
            .find(|folder| canonical.as_ref().is_some_and(|source| source.starts_with(folder)))
        else {
            return Ok(None);
        };
        let key = relative(key)?;
        let path = extraction.join(resource_key(&key));
        match path.canonicalize() {
            Ok(path) if !path.starts_with(&extraction) => Err(Error("extracted asset resolved outside its folder".into())),
            Ok(path) => Ok(path.is_file().then_some(path)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(Error(error.to_string())),
        }
    }

    /// Pair an extracted document with its extracted mesh/material assets when
    /// available; otherwise use the client's normal loose/packed resolution.
    pub fn read_for_source(&self, key: &str, source: &Path, limit: usize) -> Result<Vec<u8>> {
        if let Some(path) = self.extracted_path(key, source)? { read_bounded(&path, limit.min(MAX_FILE_BYTES)) } else { self.read(key, limit) }
    }

    /// Editable companion selected with the same provenance as read_for_source.
    pub fn editable_path_for_source(&self, key: &str, source: &Path) -> Option<PathBuf> {
        self.extracted_path(key, source).ok().flatten().or_else(|| self.editable_path(key))
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
        let rel = canonical.as_deref().unwrap_or_else(|| Path::new(texture)).strip_prefix(&self.root).ok().map(|p| resource_key(&p.to_string_lossy()));
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
            if self.loose_path(&path).is_some() || self.packed.contains(&path) || self.archives.iter().any(|a| a.contains(&path)) {
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
        if let Some(file) = self.files.get(&key).or_else(|| self.files.get(&format!("decomp/{key}"))).or_else(|| self.files.get(&format!("c3.extracted/{key}")))
        {
            return Some(file.clone());
        }
        for key in [&key, &format!("decomp/{key}"), &format!("c3.extracted/{key}")] {
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
        let loose = self.loose_path(&path);
        let native =
            loose.as_ref().is_some_and(|p| p.strip_prefix(&self.root).ok().is_some_and(|p| !p.starts_with("decomp") && !p.starts_with("c3.extracted")));
        if !native && let Some(bytes) = self.packed.read(&path, limit.min(MAX_FILE_BYTES))? {
            return Ok(bytes);
        }
        let Some(file) = loose else {
            let mut archive_error = None;
            for archive in &self.archives {
                match archive.read(&path, limit.min(MAX_FILE_BYTES)) {
                    Ok(Some(bytes)) => return Ok(bytes),
                    Ok(None) => {}
                    Err(e) => {
                        archive_error.get_or_insert_with(|| e.to_string());
                    }
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
        if relative(&key).is_ok() && (self.packed.contains(&key) || self.archives.iter().any(|a| a.contains(&key))) {
            return Some(key);
        }
        let base = basename(embedded);
        let conventional = format!("c3/texture/{base}");
        if self.loose_path(&conventional).is_some() {
            return Some(conventional);
        }
        if self.packed.contains(&conventional) || self.archives.iter().any(|a| a.contains(&conventional)) {
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
