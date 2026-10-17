//! On-demand readers for the client's compressed packages. No extraction or
//! writes to the client are needed. The manifest matching version.dat selects
//! C3Pack resources; ambiguous package versions are never chosen by filename.
use crate::{
    Error, MAX_FILE_BYTES, Result,
    assets::{norm, read_bounded, relative},
    reader::Reader,
};
use std::{
    collections::HashMap,
    fs::{self, File},
    io::{BufReader, Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
};

#[derive(Clone, Copy)]
struct Block {
    offset: u64,
    raw: usize,
    stored: usize,
}
struct Entry {
    raw: usize,
    stored: usize,
    blocks: Vec<Block>,
}
struct Package {
    path: PathBuf,
    length: u64,
    entries: HashMap<String, Entry>,
}
fn word(reader: &mut impl Read) -> Result<u32> {
    let mut b = [0; 4];
    reader.read_exact(&mut b).map_err(|e| Error(e.to_string()))?;
    Ok(u32::from_le_bytes(b))
}
fn name(bytes: &[u8]) -> Result<String> {
    let text = std::str::from_utf8(bytes).map(str::to_string).unwrap_or_else(|_| encoding_rs::GBK.decode(bytes).0.into_owned());
    relative(&text)
}
fn inflate(file: &Path, entry: &Entry, limit: usize) -> Result<Vec<u8>> {
    if entry.raw > limit || entry.stored > MAX_FILE_BYTES {
        return Err(Error("compressed resource exceeds the read budget".into()));
    }
    let mut f = File::open(file).map_err(|e| Error(e.to_string()))?;
    let length = f.metadata().map_err(|e| Error(e.to_string()))?.len();
    let mut bytes = Vec::with_capacity(entry.raw.min(1024 * 1024));
    for block in &entry.blocks {
        if block.offset.checked_add(block.stored as u64).is_none_or(|end| end > length) {
            return Err(Error("compressed resource lies outside its package".into()));
        }
        f.seek(SeekFrom::Start(block.offset)).map_err(|e| Error(e.to_string()))?;
        let mut decoder = flate2::read::ZlibDecoder::new((&mut f).take(block.stored as u64));
        let before = bytes.len();
        (&mut decoder).take(block.raw as u64 + 1).read_to_end(&mut bytes).map_err(|e| Error(format!("invalid compressed resource: {e}")))?;
        if bytes.len().saturating_sub(before) != block.raw || decoder.total_in() != block.stored as u64 {
            return Err(Error("compressed block size does not match its index".into()));
        }
    }
    if bytes.len() != entry.raw {
        return Err(Error("compressed resource size does not match its index".into()));
    }
    Ok(bytes)
}
impl Package {
    fn pack(path: &Path) -> Result<Self> {
        let file = File::open(path).map_err(|e| Error(e.to_string()))?;
        let length = file.metadata().map_err(|e| Error(e.to_string()))?.len();
        let mut file = BufReader::new(file);
        let mut header = [0; 16];
        file.read_exact(&mut header).map_err(|e| Error(e.to_string()))?;
        if header != [0; 16] || word(&mut file)? != 1 || word(&mut file)? != 0 {
            return Err(Error("unsupported C3Pack header".into()));
        }
        let payload = word(&mut file)? as u64;
        let count = word(&mut file)?;
        if count > 1_000_000 {
            return Err(Error("C3Pack exceeds the index budget".into()));
        }
        let mut entries = HashMap::new();
        for _ in 0..count {
            let mut size = [0];
            file.read_exact(&mut size).map_err(|e| Error(e.to_string()))?;
            let mut bytes = vec![0; usize::from(size[0])];
            file.read_exact(&mut bytes).map_err(|e| Error(e.to_string()))?;
            let key = name(&bytes)?;
            let raw = word(&mut file)? as usize;
            let stored = word(&mut file)? as usize;
            let offset = u64::from(word(&mut file)?);
            if entries.insert(key, Entry { raw, stored, blocks: vec![Block { offset, raw, stored }] }).is_some() {
                return Err(Error("duplicate C3Pack entry".into()));
            }
        }
        let base = file.stream_position().map_err(|e| Error(e.to_string()))?;
        if base.checked_add(payload).is_none_or(|end| end != length) {
            return Err(Error("C3Pack payload length does not match its header".into()));
        }
        for entry in entries.values_mut() {
            for block in &mut entry.blocks {
                block.offset = block.offset.checked_add(base).ok_or_else(|| Error("package offset overflow".into()))?;
                if block.offset.checked_add(block.stored as u64).is_none_or(|end| end > length) {
                    return Err(Error("C3Pack entry exceeds its payload".into()));
                }
            }
        }
        Ok(Self { path: path.into(), length, entries })
    }
    fn tpi(path: &Path) -> Result<Self> {
        let bytes = read_bounded(path, 64 * 1024 * 1024)?;
        let mut r = Reader::new(&bytes);
        if r.take(16)? != b"NetDragonDatPkg\0" || u64::from_le_bytes(r.array()?) != 1000 {
            return Err(Error("unsupported TPI header".into()));
        }
        r.take(8)?;
        if r.u32()? != 48 {
            return Err(Error("unsupported TPI index header".into()));
        }
        let count = r.u32()?;
        let index_bytes = r.u32()? as usize;
        let free_blocks = r.u32()? as usize;
        let index_end = index_bytes.checked_add(48);
        if count > 1_000_000 || free_blocks > 1_000_000 || index_end.and_then(|end| end.checked_add(free_blocks.saturating_mul(8))) != Some(bytes.len()) {
            return Err(Error("TPI index exceeds its bounds".into()));
        }
        let data = path.with_extension("tpd").canonicalize().map_err(|e| Error(e.to_string()))?;
        let folder = path.parent().and_then(|p| p.canonicalize().ok()).ok_or_else(|| Error("TPI folder unavailable".into()))?;
        if data.parent() != Some(folder.as_path()) {
            return Err(Error("TPD resolves outside its index folder".into()));
        }
        let mut f = File::open(&data).map_err(|e| Error(e.to_string()))?;
        let length = f.metadata().map_err(|e| Error(e.to_string()))?.len();
        let mut header = [0; 24];
        f.read_exact(&mut header).map_err(|e| Error(e.to_string()))?;
        if header.get(..16) != Some(b"NetDragonDatPkg\0".as_slice()) || header.get(16..24) != Some(1000u64.to_le_bytes().as_slice()) {
            return Err(Error("unsupported TPD header".into()));
        }
        let mut entries = HashMap::new();
        for _ in 0..count {
            let len = usize::from(r.array::<1>()?[0]);
            let key = name(r.take(len)?)?;
            let count = usize::from(r.u16()?);
            let raw = r.u32()? as usize;
            let stored = r.u32()? as usize;
            if count == 0 || count > 4096 {
                return Err(Error("invalid TPI block count".into()));
            }
            r.require(count.saturating_mul(12))?;
            let mut blocks = Vec::with_capacity(count);
            let (mut raw_sum, mut stored_sum) = (0usize, 0usize);
            for _ in 0..count {
                let stored = r.u32()? as usize;
                let raw = r.u32()? as usize;
                let offset = u64::from(r.u32()?);
                if offset < 32 || offset.checked_add(stored as u64).is_none_or(|end| end > length) {
                    return Err(Error("invalid TPI block extent".into()));
                }
                raw_sum = raw_sum.checked_add(raw).ok_or_else(|| Error("TPI raw size overflow".into()))?;
                stored_sum = stored_sum.checked_add(stored).ok_or_else(|| Error("TPI stored size overflow".into()))?;
                blocks.push(Block { offset, raw, stored });
            }
            if raw != raw_sum || stored != stored_sum {
                return Err(Error("invalid TPI block size totals".into()));
            }
            if entries.insert(key, Entry { raw, stored, blocks }).is_some() {
                return Err(Error("duplicate TPI entry".into()));
            }
        }
        // Live-patched clients append offset/length descriptors for unused TPD
        // blocks. They are not resource entries and are never dereferenced.
        if r.remaining() != free_blocks.saturating_mul(8) {
            return Err(Error("unexpected data after TPI index".into()));
        }
        r.take(free_blocks.saturating_mul(8))?;
        Ok(Self { path: data, length, entries })
    }
    fn read(&self, key: &str, limit: usize) -> Result<Option<Vec<u8>>> {
        let Some(entry) = self.entries.get(key) else {
            return Ok(None);
        };
        if fs::metadata(&self.path).map_err(|e| Error(e.to_string()))?.len() != self.length {
            return Err(Error("package changed; refresh client resources".into()));
        }
        inflate(&self.path, entry, limit).map(Some)
    }
}

pub(crate) struct Packed {
    root: PathBuf,
    manifest: OnceLock<Result<HashMap<String, String>>>,
    tpi: OnceLock<Result<Vec<Package>>>,
    packages: Mutex<HashMap<String, Package>>,
}
impl Packed {
    pub fn new(root: PathBuf) -> Self {
        Self { root, manifest: OnceLock::new(), tpi: OnceLock::new(), packages: Mutex::new(HashMap::new()) }
    }
    fn contained(&self, key: &str) -> Result<PathBuf> {
        let path = self.root.join(relative(key)?).canonicalize().map_err(|e| Error(e.to_string()))?;
        if !path.starts_with(&self.root) {
            return Err(Error("package resolves outside the client".into()));
        }
        Ok(path)
    }
    fn load_manifest(&self) -> Result<HashMap<String, String>> {
        let folder = self.root.join("c3pack");
        if !folder.is_dir() {
            return Ok(HashMap::new());
        }
        let version = read_bounded(&self.root.join("version.dat"), 256)
            .ok()
            .and_then(|b| String::from_utf8(b).ok())
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty() && v.bytes().all(|b| b.is_ascii_digit()));
        let manifest = if let Some(version) = version {
            format!("c3pack/pack_info_{version}.pack")
        } else {
            let mut paths = fs::read_dir(&folder)
                .map_err(|e| Error(e.to_string()))?
                .take(4096)
                .filter_map(std::result::Result::ok)
                .map(|e| norm(&e.file_name().to_string_lossy()))
                .filter(|n| n.starts_with("pack_info_") && n.ends_with(".pack"));
            let Some(one) = paths.next() else {
                return Ok(HashMap::new());
            };
            if paths.next().is_some() {
                return Err(Error("multiple C3Pack manifests require the client's version.dat".into()));
            }
            format!("c3pack/{one}")
        };
        let package = Package::pack(&self.contained(&manifest)?)?;
        let data = package.read("c3pack/pack_info.xml", MAX_FILE_BYTES)?.ok_or_else(|| Error("C3Pack manifest is missing pack_info.xml".into()))?;
        let xml = std::str::from_utf8(&data).map(str::to_string).unwrap_or_else(|_| encoding_rs::GBK.decode(&data).0.into_owned());
        // The manifest is a flat C3Pack/Pack/File table. Reject declarations
        // rather than resolving external entities or accepting a general XML DOM.
        if xml.contains("<!") {
            return Err(Error("unsupported XML declaration in C3Pack manifest".into()));
        }
        let mut out = HashMap::new();
        let mut pack = None;
        let mut total = 0usize;
        for line in xml.lines() {
            if line.len() > 16_384 {
                return Err(Error("C3Pack manifest line exceeds the budget".into()));
            }
            let line = line.trim();
            let attribute = || {
                line.split_once("file=\"")
                    .and_then(|(_, tail)| tail.split_once('"'))
                    .map(|(key, _)| key.replace("&amp;", "&").replace("&quot;", "\"").replace("&apos;", "'").replace("&lt;", "<").replace("&gt;", ">"))
            };
            if line.starts_with("<Pack ") {
                pack = Some(relative(&attribute().ok_or_else(|| Error("missing manifest package path".into()))?)?);
            } else if line.starts_with("</Pack") {
                pack = None;
            } else if line.starts_with("<File ") {
                total += 1;
                if total > 1_000_000 {
                    return Err(Error("C3Pack manifest exceeds the file budget".into()));
                }
                let key = relative(&attribute().ok_or_else(|| Error("missing manifest asset path".into()))?)?;
                let package = pack.as_ref().ok_or_else(|| Error("manifest asset outside a package".into()))?;
                if key.starts_with("c3/") && out.insert(key, package.clone()).is_some() {
                    return Err(Error("ambiguous C3Pack manifest resource".into()));
                }
            }
        }
        Ok(out)
    }
    fn manifest(&self) -> Result<&HashMap<String, String>> {
        self.manifest.get_or_init(|| self.load_manifest()).as_ref().map_err(|e| Error(e.to_string()))
    }
    fn tpi(&self) -> Result<&Vec<Package>> {
        self.tpi
            .get_or_init(|| {
                let mut paths = fs::read_dir(&self.root)
                    .map_err(|e| Error(e.to_string()))?
                    .take(300_000)
                    .filter_map(std::result::Result::ok)
                    .map(|e| e.path())
                    .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("tpi")))
                    .collect::<Vec<_>>();
                paths.sort();
                if paths.len() > 16 {
                    return Err(Error("client exceeds the TPI package budget".into()));
                }
                let mut packages = Vec::new();
                let mut entries = 0usize;
                for path in paths {
                    let path = self.contained(&path.file_name().unwrap_or_default().to_string_lossy())?;
                    let package = Package::tpi(&path)?;
                    entries = entries.saturating_add(package.entries.len());
                    if entries > 1_000_000 {
                        return Err(Error("TPI packages exceed the combined index budget".into()));
                    }
                    packages.push(package);
                }
                Ok(packages)
            })
            .as_ref()
            .map_err(|e| Error(e.to_string()))
    }
    pub fn contains(&self, key: &str) -> bool {
        self.manifest().is_ok_and(|m| m.contains_key(key)) || self.tpi().is_ok_and(|p| p.iter().any(|p| p.entries.contains_key(key)))
    }
    pub fn read(&self, key: &str, limit: usize) -> Result<Option<Vec<u8>>> {
        if let Some(path) = self.manifest()?.get(key) {
            let mut packages = self.packages.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            if !packages.contains_key(path) {
                if packages.len() >= 128 {
                    packages.clear();
                }
                let package = Package::pack(&self.contained(path)?)?;
                if packages.values().map(|p| p.entries.len()).sum::<usize>().saturating_add(package.entries.len()) > 1_000_000 {
                    packages.clear();
                }
                packages.insert(path.clone(), package);
            }
            return packages
                .get(path)
                .ok_or_else(|| Error("C3Pack index unavailable".into()))?
                .read(key, limit)?
                .map(Some)
                .ok_or_else(|| Error("C3Pack manifest and package disagree".into()));
        }
        let matches = self.tpi()?.iter().filter(|p| p.entries.contains_key(key)).collect::<Vec<_>>();
        if matches.len() > 1 {
            return Err(Error(format!("multiple TPI packages contain {key}; choose an unpacked override")));
        }
        matches.first().map(|p| p.read(key, limit)).unwrap_or(Ok(None))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    fn compressed(bytes: &[u8]) -> Vec<u8> {
        let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        z.write_all(bytes).unwrap();
        z.finish().unwrap()
    }
    fn pack(key: &str, bytes: &[u8]) -> Vec<u8> {
        let z = compressed(bytes);
        let mut out = vec![0; 16];
        for v in [1, 0, z.len() as u32, 1] {
            out.extend(v.to_le_bytes());
        }
        out.push(key.len() as u8);
        out.extend(key.as_bytes());
        for v in [bytes.len() as u32, z.len() as u32, 0] {
            out.extend(v.to_le_bytes());
        }
        out.extend(z);
        out
    }
    #[test]
    fn packages_select_exact_client_manifest_and_bound_decompression() {
        let root = std::env::temp_dir().join(format!("c3-packed-{}", std::process::id()));
        fs::create_dir_all(root.join("c3pack")).unwrap();
        fs::write(root.join("version.dat"), "123\r\n").unwrap();
        fs::write(root.join("c3pack/a.pack"), pack("c3/effect/test.c3", b"mesh bytes")).unwrap();
        let xml = b"<?xml version=\"1.0\"?>\n<C3Pack>\n<Pack file=\"c3pack/a.pack\">\n<File file=\"c3/effect/test.c3\" uid=\"1\" />\n</Pack>\n</C3Pack>";
        fs::write(root.join("c3pack/pack_info_123.pack"), pack("c3pack/pack_info.xml", xml)).unwrap();
        fs::write(root.join("c3pack/pack_info_999.pack"), b"wrong version").unwrap();
        let p = Packed::new(root.canonicalize().unwrap());
        assert!(p.contains("c3/effect/test.c3"));
        assert_eq!(p.read("c3/effect/test.c3", 100).unwrap(), Some(b"mesh bytes".to_vec()));
        assert!(p.read("c3/effect/test.c3", 2).is_err());
        let mut hostile = pack("../outside.c3", b"bad");
        let file = root.join("hostile.pack");
        fs::write(&file, &hostile).unwrap();
        assert!(Package::pack(&file).is_err());
        hostile = pack("c3/good.c3", b"bad");
        hostile.truncate(hostile.len() - 1);
        fs::write(&file, &hostile).unwrap();
        assert!(Package::pack(&file).is_err());
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn tpi_joins_native_compressed_blocks_and_validates_payload_relative_index_size() {
        let root = std::env::temp_dir().join(format!("c3-tpi-blocks-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let path = root.join("c3.tpi");
        let key = b"c3/effect/test.c3";
        let parts = [b"first".as_slice(), b" second".as_slice()];
        let z = parts.map(compressed);
        let mut data = b"NetDragonDatPkg\0".to_vec();
        data.extend(1000u64.to_le_bytes());
        data.extend(1u32.to_le_bytes());
        data.extend(3u32.to_le_bytes());
        // Blocks must be assembled in index order, independently of disk order.
        data.extend(&z[1]);
        data.extend(&z[0]);
        fs::write(path.with_extension("tpd"), data).unwrap();
        let mut index = b"NetDragonDatPkg\0".to_vec();
        index.extend(1000u64.to_le_bytes());
        for v in [1, 3, 48, 1, (1 + key.len() + 10 + 24) as u32, 0] {
            index.extend(v.to_le_bytes());
        }
        index.push(key.len() as u8);
        index.extend(key);
        index.extend(2u16.to_le_bytes());
        index.extend(12u32.to_le_bytes());
        index.extend(((z[0].len() + z[1].len()) as u32).to_le_bytes());
        for (i, offset) in [32 + z[1].len(), 32].into_iter().enumerate() {
            for v in [z[i].len() as u32, parts[i].len() as u32, offset as u32] {
                index.extend(v.to_le_bytes());
            }
        }
        fs::write(&path, &index).unwrap();
        let package = Package::tpi(&path).unwrap();
        assert_eq!(package.read("c3/effect/test.c3", 12).unwrap(), Some(b"first second".to_vec()));
        assert!(package.read("c3/effect/test.c3", 11).is_err());
        let whole_file_size = index.len() as u32;
        index[40..44].copy_from_slice(&whole_file_size.to_le_bytes());
        fs::write(&path, &index).unwrap();
        assert!(Package::tpi(&path).is_err());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn tpi_reads_only_requested_zlib_payload_and_rejects_bad_sizes() {
        let root = std::env::temp_dir().join(format!("c3-tpi-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let path = root.join("c3.tpi");
        let key = b"c3/effect/test.c3";
        let payload = b"particle data";
        let z = compressed(payload);
        let mut data = b"NetDragonDatPkg\0".to_vec();
        data.extend(1000u64.to_le_bytes());
        data.extend(1u32.to_le_bytes());
        data.extend(3u32.to_le_bytes());
        data.extend(&z);
        fs::write(path.with_extension("tpd"), data).unwrap();
        let mut index = b"NetDragonDatPkg\0".to_vec();
        index.extend(1000u64.to_le_bytes());
        for v in [1, 3, 48, 1, (1 + key.len() + 22) as u32, 0] {
            index.extend(v.to_le_bytes());
        }
        index.push(key.len() as u8);
        index.extend(key);
        index.extend(1u16.to_le_bytes());
        for v in [payload.len() as u32, z.len() as u32, z.len() as u32, payload.len() as u32, 32] {
            index.extend(v.to_le_bytes());
        }
        fs::write(&path, &index).unwrap();
        let p = Package::tpi(&path).unwrap();
        assert_eq!(p.read("c3/effect/test.c3", 100).unwrap(), Some(payload.to_vec()));
        assert!(p.read("c3/effect/test.c3", 1).is_err());
        assert!(p.read("c3/absent.c3", 100).unwrap().is_none());
        let mut patched = index.clone();
        patched[44..48].copy_from_slice(&1u32.to_le_bytes());
        patched.extend(8u32.to_le_bytes());
        patched.extend(32u32.to_le_bytes());
        fs::write(&path, &patched).unwrap();
        let patched_package = Package::tpi(&path).unwrap();
        assert_eq!(patched_package.read("c3/effect/test.c3", 100).unwrap(), Some(payload.to_vec()));
        let extracted = root.join("c3.extracted/c3/effect/test.dds");
        fs::create_dir_all(extracted.parent().unwrap()).unwrap();
        fs::write(&extracted, b"old texture").unwrap();
        fs::write(extracted.with_extension("c3"), b"old mesh").unwrap();
        let assets = crate::assets::ClientAssets::loose(&root, &std::sync::atomic::AtomicBool::new(false)).unwrap();
        assert_eq!(assets.read("c3/effect/test.c3", 100).unwrap(), payload);
        assert_eq!(assets.read_for_source("c3/effect/test.c3", &extracted, 100).unwrap(), b"old mesh");
        assert!(assets.editable_path("c3/effect/test.c3").is_none());
        patched.pop();
        fs::write(&path, &patched).unwrap();
        assert!(Package::tpi(&path).is_err());
        let end = index.len();
        index[end - 8..end - 4].copy_from_slice(&999u32.to_le_bytes());
        fs::write(&path, index).unwrap();
        assert!(Package::tpi(&path).is_err());
        fs::remove_dir_all(root).unwrap();
    }
}
