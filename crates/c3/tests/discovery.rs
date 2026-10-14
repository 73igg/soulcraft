#![cfg(not(target_arch = "wasm32"))]
use photocraft_c3::assets::ClientAssets;
use std::{fs, path::PathBuf, sync::atomic::AtomicBool};
struct Folder(PathBuf);
impl Folder {
    fn new(name: &str) -> Self {
        let root = std::env::var_os("C3_TEST_ARTIFACT_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir)
            .join(format!("photocraft-c3-{name}-{}", std::process::id()));
        fs::create_dir_all(root.join("ini")).unwrap();
        fs::create_dir_all(root.join("c3/mesh")).unwrap();
        fs::create_dir_all(root.join("c3/texture")).unwrap();
        Self(root)
    }
    fn write(&self, path: &str, data: impl AsRef<[u8]>) {
        let path = self.0.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, data).unwrap();
    }
}
impl Drop for Folder {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
#[test]
fn aliases_and_different_mesh_ids_resolve_through_client_catalogs() {
    let f = Folder::new("aliases");
    f.write("ini/3dobj.ini", "000123=c3/mesh/body.c3\n456=c3/mesh/other.c3\n");
    f.write("ini/3dtexture.ini", "00999=c3/texture/skin.dds\n123=c3/texture/wrong.dds\n");
    f.write("ini/armor.ini", "[outfit]\nMesh0=000123\nTexture0=999\n[variant]\nMesh0=456\nTexture0=999\n");
    f.write("c3/mesh/body.c3", b"data");
    f.write("c3/mesh/other.c3", b"data");
    let cancel = AtomicBool::new(false);
    let assets = ClientAssets::build(&f.0, &cancel).unwrap();
    let matches = assets.candidates(&f.0.join("c3/texture/skin.dds").to_string_lossy(), &cancel).unwrap();
    assert_eq!(matches.len(), 2);
    assert_eq!(matches[0].path, "c3/mesh/body.c3");
    assert!(matches.iter().all(|c| c.rank == 0));
    assert!(assets.read("../outside.c3", 1000).is_err());
    assert!(assets.read("C:/outside.c3", 1000).is_err());
}
#[test]
fn effect_ids_use_the_effect_registry_and_do_not_cross_pair_sections() {
    let f = Folder::new("effects");
    f.write("ini/3deffectobj.ini", "7=c3/effect/a.c3\n8=c3/effect/b.c3\n");
    f.write("ini/3dtexture.ini", "99=c3/texture/glow.dds\n100=c3/texture/other.dds\n");
    f.write("ini/3deffect.ini", "[a]\nEffectId0=7\nTextureId0=99\n[b]\nEffectId0=8\nTextureId0=100\n");
    let cancel = AtomicBool::new(false);
    let assets = ClientAssets::build(&f.0, &cancel).unwrap();
    let matches = assets.candidates("glow.dds", &cancel).unwrap();
    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0].path, "c3/effect/a.c3");
}
#[test]
fn cancellation_and_hostile_catalogs_fail_cleanly() {
    let f = Folder::new("cancel");
    assert!(ClientAssets::build(&f.0, &AtomicBool::new(true)).is_err());
    f.write("ini/3dtexture.ini", "1=../outside.dds\n");
    f.write("ini/3dobj.ini", "1=../outside.c3\n");
    let assets = ClientAssets::build(&f.0, &AtomicBool::new(false)).unwrap();
    assert!(assets.candidates("outside.dds", &AtomicBool::new(false)).unwrap().is_empty());
}

#[test]
fn exact_texture_path_beats_an_identical_basename_in_another_effect() {
    let f = Folder::new("basename");
    f.write("ini/3dobj.ini", "1=c3/mesh/one.c3\n2=c3/mesh/two.c3\n");
    f.write("ini/3dtexture.ini", "1=c3/effect/one/1.dds\n2=c3/effect/two/1.dds\n");
    f.write("c3/effect/two/1.dds", b"texture");
    let cancel = AtomicBool::new(false);
    let assets = ClientAssets::build(&f.0, &cancel).unwrap();
    let matches = assets.candidates(&f.0.join("c3/effect/two/1.dds").to_string_lossy(), &cancel).unwrap();
    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0].path, "c3/mesh/two.c3");
}

#[test]
fn quick_catalogs_and_selected_meshes_do_not_walk_the_client_tree() {
    let f = Folder::new("quick");
    f.write("ini/3dobj.ini", "7=c3/mesh/body.c3\n");
    f.write("ini/3dtexture.ini", "99=c3/texture/skin.dds\n");
    f.write("ini/armor.ini", "[body]\nMesh0=7\nTexture0=99\n");
    f.write("c3/mesh/body.c3", b"selected mesh");
    f.write("c3/texture/skin.dds", b"texture");
    f.write("unrelated.dnp", b"a corrupt package must not delay a loose mesh");
    let deep = format!("c3/{}/unlisted.c3", ["deep"; 26].join("/"));
    f.write(&deep, b"not indexed");
    let cancel = AtomicBool::new(false);
    let quick = ClientAssets::quick(&f.0, &cancel).unwrap();
    let matched = quick.candidates(&f.0.join("c3/texture/skin.dds").to_string_lossy(), &cancel).unwrap();
    assert_eq!(matched.len(), 1);
    assert_eq!(matched[0].rank, 0);
    assert_eq!(quick.read(&matched[0].path, 100).unwrap(), b"selected mesh");
    assert!(!quick.model_paths().contains(&deep));
    assert!(quick.warnings.is_empty());
    let loose = ClientAssets::loose(&f.0, &cancel).unwrap();
    assert!(loose.model_paths().is_empty());
    assert_eq!(loose.read("c3/mesh/body.c3", 100).unwrap(), b"selected mesh");
    assert_eq!(loose.texture_path("C:/exporter/skin.dds").unwrap(), "c3/texture/skin.dds");
    assert!(loose.read("../outside.c3", 100).is_err());
    assert!(ClientAssets::quick(&f.0, &AtomicBool::new(true)).is_err());
}

#[test]
fn quick_lookup_preserves_loose_overrides_and_reads_archives_only_on_demand() {
    let f = Folder::new("lazy-archive");
    f.write("ini/3dobj.ini", "1=c3/mesh/001000000.c3\n");
    f.write("ini/3dtexture.ini", "1=c3/texture/001000000.dds\n");
    let mut archive = vec![0; 40];
    archive[..18].copy_from_slice(b"DawnPack.TqDigital");
    archive[32..36].copy_from_slice(&1001u32.to_le_bytes());
    archive[36..40].copy_from_slice(&1u32.to_le_bytes());
    // Fixed DawnPack path hash for c3\\mesh\\001000000.c3.
    let payload = b"packed mesh";
    archive.extend((0x13fb8830u32 ^ 0x95279527).to_le_bytes());
    archive.extend(((payload.len() as u32) ^ 0x96120059).to_le_bytes());
    archive.extend((52u32 ^ 0x99589958).to_le_bytes());
    archive.extend(payload);
    f.write("empty.dnp", archive);
    f.write("c3/mesh/001000000.c3", b"loose override");
    let quick = ClientAssets::quick(&f.0, &AtomicBool::new(false)).unwrap();
    assert_eq!(quick.read("c3/mesh/001000000.c3", 100).unwrap(), b"loose override");
    fs::remove_file(f.0.join("c3/mesh/001000000.c3")).unwrap();
    assert_eq!(quick.read("c3/mesh/001000000.c3", 100).unwrap(), payload);
    f.write("c3/texture/001000000.c3", b"same basename, different resource");
    assert_eq!(quick.texture_path("c3/mesh/001000000.c3").unwrap(), "c3/mesh/001000000.c3");
    assert!(quick.read("c3/mesh/missing.c3", 100).is_err());
}
