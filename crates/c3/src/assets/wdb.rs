//! Bounded WDB/DBC catalog reader. Binary records become the same catalogs as
//! loose INIs; game code and game assets are never embedded in the application.
use super::{Ini, IniSection, basename, relative};
use crate::{Error, Result, reader::Reader};
use std::{
    collections::{HashMap, HashSet},
    sync::atomic::{AtomicBool, Ordering},
};

fn error(message: &str) -> Error {
    Error(format!("WDB: {message}"))
}
fn check(cancel: &AtomicBool) -> Result<()> {
    if cancel.load(Ordering::Relaxed) { Err(error("cancelled")) } else { Ok(()) }
}
fn text(bytes: &[u8]) -> String {
    let value = bytes.split(|byte| *byte == 0).next().unwrap_or_default();
    encoding_rs::GBK.decode(value).0.into_owned()
}

// The WDB directory uses the Storm word cipher with a fixed format key.
// Generate its substitution table rather than embedding a third-party table.
fn cipher_table() -> [u32; 256] {
    let mut table = [0; 256];
    let mut seed = 0x10_0001u32;
    for value in &mut table {
        for stage in 0..5 {
            seed = (seed * 125 + 3) % 0x2a_aaab;
            let high = (seed & 0xffff) << 16;
            seed = (seed * 125 + 3) % 0x2a_aaab;
            if stage == 4 {
                *value = high | (seed & 0xffff);
            }
        }
    }
    table
}
fn directory_record(bytes: &[u8], table: &[u32; 256]) -> Result<Vec<u8>> {
    let mut key = 0xeffe_aabbu32;
    let mut state = 0xeeee_eeeeu32;
    let mut output = Vec::with_capacity(bytes.len());
    for word in bytes.as_chunks::<4>().0 {
        state = state.wrapping_add(*table.get((key & 255) as usize).ok_or_else(|| error("invalid cipher index"))?);
        let plain = u32::from_le_bytes(*word) ^ state.wrapping_add(key);
        output.extend(plain.to_le_bytes());
        key = (key >> 11) | (!key).wrapping_shl(21).wrapping_add(0x1111_1111);
        state = state.wrapping_add(plain).wrapping_add(state.wrapping_shl(5)).wrapping_add(3);
    }
    Ok(output)
}

pub(super) fn read(bytes: &[u8], cancel: &AtomicBool) -> Result<Vec<(String, Ini)>> {
    let mut reader = Reader::new(bytes);
    if reader.take(4)? != b"BDMG" {
        return Err(error("unrecognized package header"));
    }
    let size = reader.u32()? as usize;
    let offset = reader.u32()? as usize;
    let count = reader.u32()? as usize;
    if size != bytes.len() || count > 512 || offset < 16 || offset.checked_add(count.saturating_mul(84)) != Some(size) {
        return Err(error("invalid package directory bounds"));
    }
    let directory = bytes.get(offset..).ok_or_else(|| error("truncated directory"))?;
    let table = cipher_table();
    let mut names = HashSet::new();
    let mut ranges = Vec::new();
    let mut output = Vec::new();
    let mut fields = 0usize;
    for encoded in directory.as_chunks::<84>().0 {
        check(cancel)?;
        let decoded = directory_record(encoded, &table)?;
        let mut record = Reader::new(&decoded);
        record.take(4)?; // Package hash; the actual bounded name is authoritative.
        let start = record.u32()? as usize;
        let length = record.u32()? as usize;
        record.take(8)?;
        let name = relative(&text(record.take(64)?))?;
        let end = start.checked_add(length).ok_or_else(|| error("table extent overflow"))?;
        if start < 16 || end > offset || !names.insert(name.clone()) {
            return Err(error("invalid or duplicate table extent/name"));
        }
        ranges.push((start, end));
        let name = basename(&name);
        let Some(stem) = name.strip_suffix(".dbc") else { continue };
        let kind = match stem {
            "3dobj" | "3deffectobj" | "3dtexture" => 0,
            "armor" | "armet" | "head" | "misc" | "mount" | "weapon" | "3dsimpleobjex" => 1,
            "3deffect" => 2,
            "3dsimpleobj" => 3,
            _ => continue,
        };
        let data = bytes.get(start..end).ok_or_else(|| error("truncated table"))?;
        output.push((format!("{stem}.ini"), decode(data, kind, cancel, &mut fields)?));
    }
    ranges.sort_unstable();
    if ranges.windows(2).any(|pair| pair.first().zip(pair.get(1)).is_some_and(|(a, b)| a.1 > b.0)) {
        return Err(error("overlapping tables"));
    }
    Ok(output)
}

fn decode(bytes: &[u8], kind: u8, cancel: &AtomicBool, field_budget: &mut usize) -> Result<Ini> {
    let mut reader = Reader::new(bytes);
    let tag = reader.take(4)?;
    let expected: &[u8] = match kind {
        0 => b"RSDB",
        1 if tag == b"SIMX" => b"SIMX",
        1 => b"MESH",
        2 => b"EFFE",
        _ => b"SIMO",
    };
    if tag != expected {
        return Err(error("unexpected DBC table header"));
    }
    let count = reader.u32()? as usize;
    if count > 100_000 {
        return Err(error("table exceeds the record budget"));
    }
    let mut out = Ini::default();
    if kind == 0 {
        let first_string = 8usize.saturating_add(count.saturating_mul(8));
        reader.require(count.saturating_mul(8))?;
        for _ in 0..count {
            check(cancel)?;
            *field_budget = field_budget.saturating_add(1);
            if *field_budget > 1_000_000 {
                return Err(error("catalog fields exceed the decoded-data budget"));
            }
            let id = reader.u32()?.to_string();
            let offset = reader.u32()? as usize;
            if offset < first_string {
                return Err(error("resource string overlaps its index"));
            }
            let tail = bytes.get(offset..).ok_or_else(|| error("invalid resource string offset"))?;
            let length = tail.iter().take(4097).position(|byte| *byte == 0).ok_or_else(|| error("unterminated resource path"))?;
            let path = relative(&text(tail.get(..length).ok_or_else(|| error("truncated resource path"))?))?;
            out.flat.entry(id).or_insert(path);
        }
        return Ok(out);
    }
    let mut names = HashSet::new();
    for _ in 0..count {
        check(cancel)?;
        let mut fields = HashMap::new();
        let name;
        let amount;
        if kind == 2 {
            name = text(reader.take(32)?).to_lowercase();
            amount = reader.u16()? as usize;
            for key in ["delay", "looptime", "frameinterval", "loopinterval", "offsetx", "offsety", "offsetz"] {
                fields.insert(key.into(), (reader.u32()? as i32).to_string());
            }
            reader.take(4)?; // Color/level flags, outside the common material pipeline.
            fields.insert("amount".into(), amount.to_string());
        } else {
            name = reader.u32()?.to_string();
            amount = reader.u32()? as usize;
            fields.insert("part".into(), amount.to_string());
        }
        if name.is_empty() || amount > 64 {
            return Err(error("invalid name or part count"));
        }
        for slot in 0..amount {
            let mesh = reader.u32()?;
            let texture = reader.u32()?;
            fields.insert(format!("{}{slot}", if kind == 2 { "effectid" } else { "mesh" }), mesh.to_string());
            fields.insert(format!("{}{slot}", if kind == 2 { "textureid" } else { "texture" }), texture.to_string());
            if kind == 2 {
                // EFFE's opaque per-part word is not an INI ScaleN field.
                // Applying it as a transform enlarges already aligned weapon
                // overlays (e.g. 510034/510214/510274) independently of the base.
                reader.take(4)?;
            } else if kind == 1 {
                fields.insert(format!("mixtex{slot}"), reader.u32()?.to_string());
            }
            if kind != 3 {
                let flags = reader.array::<4>()?;
                let (blend, mode) = if kind == 2 { (flags.get(..2), None) } else { (flags.get(1..3), flags.first()) };
                if let Some(blend) = blend {
                    if let Some(value) = blend.first() {
                        fields.insert(format!("asb{slot}"), value.to_string());
                    }
                    if let Some(value) = blend.get(1) {
                        fields.insert(format!("adb{slot}"), value.to_string());
                    }
                }
                if let Some(mode) = mode {
                    fields.insert(format!("mixopt{slot}"), mode.to_string());
                }
            }
        }
        *field_budget = field_budget.saturating_add(fields.len());
        if *field_budget > 1_000_000 {
            return Err(error("catalog fields exceed the decoded-data budget"));
        }
        // Production databases contain repeated keys; native dictionaries keep
        // the first record. Still consume and validate the complete later row.
        if names.insert(name.clone()) {
            out.sections.push(IniSection { name, fields });
        }
    }
    if reader.remaining() != 0 {
        return Err(error("unexpected trailing table data"));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assets::ClientAssets;
    use std::{fs, path::PathBuf};

    fn u32s(values: &[u32]) -> Vec<u8> {
        values.iter().flat_map(|value| value.to_le_bytes()).collect()
    }
    fn registry(rows: &[(u32, &str)]) -> Vec<u8> {
        let mut index = b"RSDB".to_vec();
        index.extend((rows.len() as u32).to_le_bytes());
        let mut strings = Vec::new();
        for (id, path) in rows {
            index.extend(u32s(&[*id, (8 + rows.len() * 8 + strings.len()) as u32]));
            strings.extend(path.as_bytes());
            strings.push(0);
        }
        index.extend(strings);
        index
    }
    fn package(tables: &[(&str, Vec<u8>)]) -> Vec<u8> {
        let mut data = b"BDMG".to_vec();
        data.resize(16, 0);
        let mut records = Vec::new();
        for (name, bytes) in tables {
            let mut row = u32s(&[0, data.len() as u32, bytes.len() as u32, 0, 0]);
            row.extend(name.as_bytes());
            row.resize(84, 0);
            records.push(row);
            data.extend(bytes);
        }
        let offset = data.len();
        let table = cipher_table();
        for row in records {
            let mut key = 0xeffe_aabbu32;
            let mut seed = 0xeeee_eeeeu32;
            for word in row.as_chunks::<4>().0 {
                seed = seed.wrapping_add(table[(key & 255) as usize]);
                let plain = u32::from_le_bytes(*word);
                data.extend((plain ^ seed.wrapping_add(key)).to_le_bytes());
                key = (key >> 11) | (!key).wrapping_shl(21).wrapping_add(0x1111_1111);
                seed = seed.wrapping_add(plain).wrapping_add(seed.wrapping_shl(5)).wrapping_add(3);
            }
        }
        let size = data.len() as u32;
        data[4..16].copy_from_slice(&u32s(&[size, offset as u32, tables.len() as u32]));
        data
    }
    fn fixture() -> Vec<u8> {
        let mut mesh = b"MESH".to_vec();
        mesh.extend(u32s(&[1, 209980, 1, 10, 20, 0]));
        mesh.extend([0, 5, 6, 0]);
        let mut effect = b"EFFE".to_vec();
        effect.extend(1u32.to_le_bytes());
        let mut name = b"weapon209980".to_vec();
        name.resize(32, 0);
        effect.extend(name);
        effect.extend(1u16.to_le_bytes());
        effect.extend(u32s(&[17, 100, 33, 5, 1, 2, 3]));
        effect.extend([0, 0, 1, 0]);
        effect.extend(u32s(&[11, 21]));
        effect.extend(1.5f32.to_le_bytes());
        effect.extend([5, 2, 0, 0]);
        package(&[
            ("ini/3DObj.dbc", registry(&[(10, "c3/weapon/blade.c3")])),
            ("ini/3DEffectObj.dbc", registry(&[(11, "c3/effect/glow.c3")])),
            ("ini/3DTexture.dbc", registry(&[(20, "c3/weapon/blade.dds"), (21, "c3/effect/glow.dds")])),
            ("ini/Weapon.dbc", mesh),
            ("ini/3DEffect.dbc", effect),
        ])
    }
    struct Folder(PathBuf);
    impl Folder {
        fn new() -> Self {
            static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let root = std::env::temp_dir().join(format!("soulcraft-wdb-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
            fs::create_dir_all(root.join("ini")).unwrap();
            fs::create_dir_all(root.join("c3/weapon")).unwrap();
            fs::write(root.join("ini/c3.wdb"), fixture()).unwrap();
            fs::write(root.join("ini/Action3DEffect.ini"), "9999.9999.209.980=weapon209980\n9999.0401.209.980=attack\n").unwrap();
            Self(root)
        }
    }
    impl Drop for Folder {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn binary_catalogs_resolve_materials_persistent_effects_and_extracted_edits() {
        let folder = Folder::new();
        let source = folder.0.join("c3.extracted/c3/weapon/blade.dds");
        fs::create_dir_all(source.parent().unwrap()).unwrap();
        fs::write(&source, b"editable").unwrap();
        fs::write(folder.0.join("c3/weapon/blade.c3"), b"live mesh").unwrap();
        fs::write(source.with_extension("c3"), b"extracted mesh").unwrap();
        let assets = ClientAssets::quick(&folder.0, &AtomicBool::new(false)).unwrap();
        assert!(assets.warnings.is_empty(), "{:?}", assets.warnings);
        let objects = assets.objects_for_texture(&source.to_string_lossy());
        assert_eq!(objects.len(), 1);
        assert_eq!(objects[0].id, "weapon.ini:209980");
        assert_eq!(objects[0].parts[0].mesh.as_deref(), Some("c3/weapon/blade.c3"));
        let bindings = assets.effect_bindings(&objects[0]);
        assert_eq!(bindings.iter().filter(|value| value.enabled).count(), 1);
        assert_eq!(bindings[0].effect, "weapon209980");
        let effect = assets.effect_object(&bindings[0].effect).unwrap();
        assert_eq!(effect.parts[0].texture.as_deref(), Some("c3/effect/glow.dds"));
        let material = assets.material(&effect.id, 0);
        assert_eq!(material.blend, [5, 2]);
        assert_eq!(material.scale, 1.);
        let timing = assets.part_timing(&effect.id, 0);
        assert_eq!(timing.delay_ms, 17);
        assert_eq!(timing.loops, 100);
        assert_eq!(timing.offset, [1., 2., 3.]);
        assert_eq!(assets.read_for_source("c3/weapon/blade.c3", &source, 1024).unwrap(), b"extracted mesh");
        assert_eq!(assets.read("c3/weapon/blade.c3", 1024).unwrap(), b"live mesh");
        assert_eq!(assets.editable_path_for_source("c3/weapon/blade.dds", &source).unwrap(), source.canonicalize().unwrap());
        fs::write(folder.0.join("ini/weapon.ini"), "[209980]\nPart=1\nMesh0=10\nTexture0=21\n").unwrap();
        let overridden = ClientAssets::quick(&folder.0, &AtomicBool::new(false)).unwrap();
        assert_eq!(overridden.object("weapon.ini:209980").unwrap().parts[0].texture.as_deref(), Some("c3/effect/glow.dds"));
        // Fully packed clients need no loose c3 directory to identify their
        // catalogs above an extraction. WDB alone identifies the client layout.
        fs::remove_file(folder.0.join("c3/weapon/blade.c3")).unwrap();
        fs::remove_dir(folder.0.join("c3/weapon")).unwrap();
        fs::remove_dir(folder.0.join("c3")).unwrap();
        assert_eq!(crate::assets::infer_root(&source).unwrap(), folder.0.canonicalize().unwrap());
        let packed = ClientAssets::quick(source.parent().unwrap(), &AtomicBool::new(false)).unwrap();
        assert!(packed.object("weapon.ini:209980").is_some());
    }

    #[test]
    fn opaque_effe_word_does_not_become_a_geometry_scale_and_loose_scale_is_retained() {
        let folder = Folder::new();
        fs::write(folder.0.join("ini/3DEffect.ini"), "[weapon209980]\nAmount=1\nEffectId0=11\nTextureId0=21\nScale0=150\n").unwrap();
        let assets = ClientAssets::quick(&folder.0, &AtomicBool::new(false)).unwrap();
        assert_eq!(assets.material("3deffect.ini:weapon209980", 0).scale, 1.5);
        let catalogs = read(&fixture(), &AtomicBool::new(false)).unwrap();
        let effect = catalogs.iter().find(|(name, _)| name == "3deffect.ini").unwrap();
        assert!(!effect.1.sections[0].fields.contains_key("scale0"));
    }

    #[test]
    fn rejects_truncation_counts_offsets_traversal_and_table_overlap() {
        let cancel = AtomicBool::new(false);
        assert_eq!(cipher_table()[0], 0x193a_a698);
        assert_eq!(cipher_table()[255], 0x7303_286c);
        let valid = fixture();
        assert_eq!(read(&valid, &cancel).unwrap().len(), 5);
        for size in [0, 4, 15, 16, valid.len() - 1] {
            assert!(read(&valid[..size], &cancel).is_err());
        }
        let mut bad = valid.clone();
        bad[12..16].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(read(&bad, &cancel).is_err());
        let bad = package(&[("../3dobj.dbc", registry(&[(1, "c3/blade.c3")]))]);
        assert!(read(&bad, &cancel).is_err());
        let bad = package(&[("ini/3dobj.dbc", registry(&[(1, "../blade.c3")]))]);
        assert!(read(&bad, &cancel).is_err());
        let mut bad = registry(&[(1, "c3/blade.c3")]);
        bad[12..16].copy_from_slice(&8u32.to_le_bytes());
        assert!(decode(&bad, 0, &cancel, &mut 0).is_err());
        let mut bad = b"MESH".to_vec();
        bad.extend(u32s(&[1, 1, u32::MAX]));
        assert!(decode(&bad, 1, &cancel, &mut 0).is_err());
        assert!(decode(&registry(&[(1, "c3/blade.c3")]), 0, &cancel, &mut 1_000_000).is_err());
        let cancelled = AtomicBool::new(true);
        assert!(read(&valid, &cancelled).is_err());
    }
}
