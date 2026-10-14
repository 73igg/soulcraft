//! Read-only DawnPack lookup. The index is read once; only a requested payload
//! is read from the archive, with explicit allocation and file bounds.
use crate::{Error, MAX_FILE_BYTES, Result};
use std::{
    collections::HashMap,
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    sync::OnceLock,
};

/// Opening the client folder must not read package indexes the preview never uses.
pub(crate) struct LazyArchive {
    path: PathBuf,
    archive: OnceLock<Result<Archive>>,
}
impl LazyArchive {
    pub fn new(path: PathBuf) -> Self {
        Self { path, archive: OnceLock::new() }
    }
    fn get(&self) -> Result<&Archive> {
        self.archive.get_or_init(|| Archive::open(&self.path)).as_ref().map_err(|e| Error(e.to_string()))
    }
    pub fn contains(&self, path: &str) -> bool {
        self.get().is_ok_and(|a| a.contains(path))
    }
    pub fn read(&self, path: &str, limit: usize) -> Result<Option<Vec<u8>>> {
        self.get()?.read(path, limit)
    }
}

pub(crate) struct Archive {
    path: PathBuf,
    index: HashMap<u32, (u64, usize)>,
    length: u64,
}
impl Archive {
    pub fn open(path: &Path) -> Result<Self> {
        let file = File::open(path).map_err(|e| Error(e.to_string()))?;
        let length = file.metadata().map_err(|e| Error(e.to_string()))?.len();
        let mut file = std::io::BufReader::with_capacity(64 * 1024, file);
        let mut header = [0u8; 40];
        file.read_exact(&mut header).map_err(|e| Error(e.to_string()))?;
        if !header.starts_with(b"DawnPack.TqDigital") {
            return Err(Error("not a DawnPack archive".into()));
        }
        let version = u32::from_le_bytes(
            header.get(32..36).ok_or_else(|| Error("truncated package version".into()))?.try_into().map_err(|_| Error("truncated version".into()))?,
        );
        if version != 1001 {
            return Err(Error(format!("unsupported DawnPack version {version}")));
        }
        let count = u32::from_le_bytes(
            header.get(36..40).ok_or_else(|| Error("truncated index count".into()))?.try_into().map_err(|_| Error("truncated count".into()))?,
        ) as usize;
        let end = 40u64 + (count as u64) * 12;
        if count > 500_000 || end > length {
            return Err(Error("DawnPack index exceeds its bounds or entry budget".into()));
        }
        let mut index = HashMap::with_capacity(count);
        for _ in 0..count {
            let mut entry = [0u8; 12];
            file.read_exact(&mut entry).map_err(|e| Error(e.to_string()))?;
            let mut words = entry.as_chunks::<4>().0.iter().copied().map(u32::from_le_bytes);
            let id = words.next().ok_or_else(|| Error("missing archive hash".into()))? ^ 0x95279527;
            let size = (words.next().ok_or_else(|| Error("missing archive size".into()))? ^ 0x96120059) as usize;
            let offset = (words.next().ok_or_else(|| Error("missing archive offset".into()))? ^ 0x99589958) as u64;
            if size > 0 && size <= MAX_FILE_BYTES && offset >= end && offset + (size as u64) <= length {
                index.entry(id).or_insert((offset, size));
            }
        }
        Ok(Self { path: path.into(), index, length })
    }
    fn entry(&self, path: &str) -> Option<(u64, usize)> {
        let path = path.replace('/', "\\").to_ascii_lowercase();
        self.index.get(&hash(&path)).copied().or_else(|| {
            path.split_once('\\')
                .filter(|(head, _)| matches!(*head, "c3" | "data" | "map" | "ani" | "sound" | "music"))
                .and_then(|(_, tail)| self.index.get(&hash(tail)).copied())
        })
    }
    pub fn contains(&self, path: &str) -> bool {
        self.entry(path).is_some()
    }
    pub fn read(&self, path: &str, limit: usize) -> Result<Option<Vec<u8>>> {
        let Some((offset, size)) = self.entry(path) else {
            return Ok(None);
        };
        if size > limit {
            return Err(Error("archive asset exceeds the preview read limit".into()));
        }
        let mut file = File::open(&self.path).map_err(|e| Error(e.to_string()))?;
        if file.metadata().map_err(|e| Error(e.to_string()))?.len() != self.length {
            return Err(Error("archive changed during preview; find matching models again".into()));
        }
        file.seek(SeekFrom::Start(offset)).map_err(|e| Error(e.to_string()))?;
        let mut bytes = vec![0; size];
        file.read_exact(&mut bytes).map_err(|e| Error(e.to_string()))?;
        Ok(Some(bytes))
    }
}

/// The public DawnPack two-accumulator path hash, modulo 2^32. Wrapping is part
/// of the file format, not an unchecked length or allocation operation.
fn hash(name: &str) -> u32 {
    let mut bytes = name.to_ascii_lowercase().into_bytes();
    bytes.resize(bytes.len().div_ceil(4) * 4, 0);
    let words = bytes.as_chunks::<4>().0.iter().copied().map(u32::from_le_bytes).chain([0x9BE74448, 0x66F42C48]);
    let (mut seed, mut left, mut right) = (0xF4FA8928u32, 0x37A8470Eu32, 0x7758B42Bu32);
    for word in words {
        seed = seed.rotate_left(1);
        let salt = 0x267B0B11 ^ seed;
        left ^= word;
        right ^= word;
        let factor = (salt.wrapping_add(right) | 0x02040801) & 0xBFEF7FDF;
        let product = u64::from(factor) * u64::from(left);
        let high = (product >> 32) as u32;
        let low = (product as u32).wrapping_add(u32::from(high != 0));
        let sum = u64::from(low) + u64::from(high);
        let new_left = (sum as u32).wrapping_add((sum >> 32) as u32);
        let factor = (salt.wrapping_add(left) | 0x00804021) & 0x7DFEFBFF;
        let product = u64::from(right) * u64::from(factor);
        let twice = (product >> 32) * 2;
        let low = (product as u32).wrapping_add((twice >> 32) as u32);
        let sum = u64::from(low) + u64::from(twice as u32);
        right = (sum as u32).wrapping_add(((sum >> 32) as u32) * 2);
        left = new_left;
    }
    left ^ right
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn case_normalization_and_empty_hash_are_stable() {
        assert_eq!(hash("C3\\Mesh\\001000000.c3"), hash("c3\\mesh\\001000000.c3"));
        assert_ne!(hash("c3\\texture\\001000000.dds"), hash("c3\\mesh\\001000000.c3"));
    }
}
