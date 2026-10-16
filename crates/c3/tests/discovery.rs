#![cfg(not(target_arch = "wasm32"))]
use photocraft_c3::assets::{ClientAssets, catalog_root, infer_root};
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
fn active_catalogs_override_decomp_snapshots_and_dotted_action_keys_attach_super_effects() {
    let f = Folder::new("active-effects");
    f.write("decomp/ini/weapon.ini", "[420230]\nPart=1\nMesh0=1\nTexture0=1\n");
    f.write("decomp/c3/texture/blade.dds", b"texture");
    f.write("ini/3dobj.ini", "1=c3/mesh/blade.c3\n");
    f.write("ini/3dtexture.ini", "1=c3/texture/blade.dds\n2=c3/texture/glow.dds\n3=c3/texture/mask.dds\n4=c3/texture/flow.dds\n");
    f.write("ini/3deffectobj.ini", "9=c3/mesh/glow.c3\n");
    f.write("ini/weapon.ini", "[420230]\nPart=1\nMesh0=1\nTexture0=1\n[420237]\nPart=1\nMesh0=1\nTexture0=1\n");
    f.write(
        "ini/3deffect3.ini",
        "[super+v_r_weapon]\nAmount=1\nEffectId0=9\nTextureId0=2\nTextureId0_1=3\nTextureId0_2=4\nASB0=5\nADB0=2\nFrameInterval=22\nScale0=0\nInterval0=44\nFrameOffset0=3\nLoopOnce0=1\nXSelf0=2.5\nManualUVStep=1\n",
    );
    f.write("ini/Action3DEffect.ini", "0.0.1.999.420.237=super+v_r_weapon\n0.0.2.999.420.237=other+v_r_weapon\n0.0.1.350.420.237=attack+v_r_weapon\n");
    let root = catalog_root(&f.0.join("decomp/c3/texture")).unwrap();
    assert_eq!(root, f.0.canonicalize().unwrap());
    let assets = ClientAssets::quick(&root, &AtomicBool::new(false)).unwrap();
    let definitions = assets.objects_for_texture(&f.0.join("decomp/c3/texture/blade.dds").to_string_lossy());
    assert_eq!(definitions[0].id, "weapon.ini:420237");
    let bindings = assets.effect_bindings(&definitions[0]);
    assert!(bindings.iter().any(|b| b.effect == "super+v_r_weapon" && b.enabled));
    assert!(bindings.iter().filter(|b| b.effect != "super+v_r_weapon").all(|b| !b.enabled));
    let effect = assets.effect_object("super+v_r_weapon").unwrap();
    let material = assets.material(&effect.id, 0);
    assert_eq!(material.blend, [5, 2]);
    assert_eq!(material.scale, 1.);
    assert_eq!(material.additional_textures.len(), 2);
    assert_eq!(assets.timing(&effect.id).frame_ms, 22);
    let timing = assets.part_timing(&effect.id, 0);
    assert_eq!(timing.frame_ms, 44);
    assert_eq!(timing.step_frame_ms, 22);
    assert_eq!(timing.frame_offset, 3);
    assert_eq!(timing.offset, [2.5, 0., 0.]);
    assert!(timing.loop_once && timing.manual_uv_step);
    assert!(!assets.objects_for_texture("c3/texture/flow.dds").is_empty());
}
#[test]
fn extracted_documents_use_matching_meshes_and_companions_without_changing_native_priority() {
    let f = Folder::new("extracted-generation");
    f.write("c3/mesh/001194600.c3", b"new mesh with new UVs");
    f.write("decomp/c3/mesh/001194600.c3", b"old mesh for the gold texture");
    f.write("c3/texture/001194600.dds", b"white texture");
    f.write("decomp/c3/texture/001194600.dds", b"gold texture");
    f.write("decomp/c3/texture/trim.dds", b"gold trim");
    f.write("c3/texture/trim.dds", b"white trim");
    f.write("c3/mesh/new-only.c3", b"fallback");
    let assets = ClientAssets::quick(&f.0, &AtomicBool::new(false)).unwrap();
    let extracted = f.0.join("decomp/c3/texture/001194600.dds");
    let native = f.0.join("c3/texture/001194600.dds");
    assert_eq!(assets.read("c3/mesh/001194600.c3", 100).unwrap(), b"new mesh with new UVs");
    assert_eq!(assets.read_for_source("c3/mesh/001194600.c3", &native, 100).unwrap(), b"new mesh with new UVs");
    assert_eq!(assets.read_for_source("c3/mesh/001194600.c3", &extracted, 100).unwrap(), b"old mesh for the gold texture");
    assert_eq!(assets.read_for_source("c3/texture/trim.dds", &extracted, 100).unwrap(), b"gold trim");
    assert_eq!(assets.read_for_source("c3/mesh/new-only.c3", &extracted, 100).unwrap(), b"fallback");
    assert!(assets.editable_path_for_source("c3/texture/trim.dds", &extracted).unwrap().ends_with("decomp/c3/texture/trim.dds"));
    assert!(assets.read_for_source("../escape", &extracted, 100).is_err());
    assert!(assets.read_for_source("c3/mesh/001194600.c3", &extracted, 4).is_err());
}

#[test]
fn body_prefixed_and_dyed_garments_resolve_their_item_and_matching_actor() {
    let f = Folder::new("garment-body");
    f.write("ini/3dobj.ini", "001191500=c3/mesh/male.c3\n002191520=c3/mesh/female.c3\n");
    f.write("ini/3dtexture.ini", "1=c3/texture/garment.dds\n");
    f.write("ini/armor.ini", "[001191500]\nMesh0=001191500\nTexture0=1\n[011191500]\nMesh0=001191500\nTexture0=1\n[002191520]\nMesh0=002191520\nTexture0=1\n");
    f.write("ini/Action3DEffect.ini", "0.0.1.999.191.500=male+v_mantle\n0.0.2.999.191.500=wrong-body\n0.0.531.999.191.500=transform\n0.0.1.350.191.500=attack\n0.0.2.999.191.520=female+v_mantle\n");
    let assets = ClientAssets::quick(&f.0, &AtomicBool::new(false)).unwrap();
    for (section, expected) in [("001191500", "male+v_mantle"), ("011191500", "male+v_mantle"), ("002191520", "female+v_mantle")] {
        let bindings = assets.effect_bindings(assets.object(&format!("armor.ini:{section}")).unwrap());
        assert_eq!(bindings.iter().filter(|binding| binding.enabled).map(|binding| binding.effect.as_str()).collect::<Vec<_>>(), [expected]);
    }
}

#[test]
fn all_numbered_outfit_sockets_play_together_in_one_actor_context() {
    let f = Folder::new("garment-sockets");
    f.write("ini/3dobj.ini", "001194600=c3/mesh/outfit.c3\n");
    f.write("ini/3dtexture.ini", "1=c3/texture/garment.dds\n");
    f.write("ini/armor.ini", "[001194600]\nMesh0=001194600\nTexture0=1\n");
    f.write("ini/Action3DEffect.ini", "0.0.1.999.194.600_0=legs+l_leg\n0.0.1.999.194.600_1=legs+r_leg\n0.0.1.999.194.600_2=head+v_head\n0.0.1.999.194.600_3=cloak+v_mantle\n1.0.1.999.194.600_0=other-profession\n0.0.1.350.194.600_0=attack\n0.0.2.999.194.600_0=other-body\n");
    let assets = ClientAssets::quick(&f.0, &AtomicBool::new(false)).unwrap();
    let bindings = assets.effect_bindings(assets.object("armor.ini:001194600").unwrap());
    let enabled = bindings.iter().filter(|binding| binding.enabled).map(|binding| binding.effect.as_str()).collect::<Vec<_>>();
    assert_eq!(enabled, ["legs+l_leg", "legs+r_leg", "head+v_head", "cloak+v_mantle"]);
    assert!(bindings.iter().filter(|binding| matches!(binding.effect.as_str(), "attack" | "other-body" | "other-profession")).all(|binding| !binding.enabled));
}

#[test]
fn dyed_part_effect_fallback_preserves_exact_overrides_and_item_quality() {
    let f = Folder::new("armor-identity");
    f.write("ini/3dobj.ini", "001130000=c3/mesh/armor.c3\n");
    f.write("ini/3dtexture.ini", "1=c3/texture/armor.dds\n");
    f.write("ini/armor.ini", "[001130000]\nMesh0=001130000\nTexture0=1\n[001130004]\nMesh0=001130000\nTexture0=1\n[011130004]\nMesh0=001130000\nTexture0=1\n[011130005]\nMesh0=001130000\nTexture0=1\n");
    f.write("ini/PartEffect.ini", "[body]\n001130004=body+v_mantle\n001130005=wrong-fallback\n011130005=dye-specific\n");
    f.write("ini/Action3DEffect.ini", "0.0.1.999.130.004_0=super+v_l_arm\n0.0.1.999.130.004_1=super+v_r_arm\n");
    let assets = ClientAssets::quick(&f.0, &AtomicBool::new(false)).unwrap();
    for section in ["001130004", "011130004"] {
        let bindings = assets.effect_bindings(assets.object(&format!("armor.ini:{section}")).unwrap());
        assert_eq!(bindings.iter().filter(|binding| binding.enabled).count(), 3);
        assert!(bindings.iter().any(|binding| binding.effect == "body+v_mantle" && binding.enabled));
    }
    let dyed = assets.effect_bindings(assets.object("armor.ini:011130005").unwrap());
    assert_eq!(dyed.len(), 1);
    assert_eq!(dyed[0].effect, "dye-specific");
    assert_ne!(assets.objects_for_texture("c3/texture/armor.dds")[0].id, "armor.ini:001130000");
}

#[test]
fn monster_armor_uses_its_full_body_context_without_human_modulo_aliasing() {
    let f = Folder::new("monster-body");
    f.write("ini/3dobj.ini", "531000000=c3/mesh/monster.c3\n");
    f.write("ini/3dtexture.ini", "1=c3/texture/monster.dds\n");
    f.write("ini/armor.ini", "[531000000]\nMesh0=531000000\nTexture0=1\n");
    f.write("ini/Action3DEffect.ini", "0.0.531.999.000.000=monster\n0.0.1.999.000.000=human\n");
    let assets = ClientAssets::quick(&f.0, &AtomicBool::new(false)).unwrap();
    let bindings = assets.effect_bindings(assets.object("armor.ini:531000000").unwrap());
    assert!(bindings.iter().any(|binding| binding.effect == "monster" && binding.enabled));
    assert!(bindings.iter().filter(|binding| binding.effect == "human").all(|binding| !binding.enabled));
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
fn legacy_action_catalog_versions_attach_exact_items_and_preserve_contexts() {
    for (label, catalog, section, actions) in [
        ("era", "weapon.ini", "Weapon420009", "999.420.009=super\n401.420.009=attack\n999.420.019=unrelated\n"),
        ("old-eo", "weapon.ini", "410129", "1.999.410.129=super\n2.999.410.129=other\n1.812.410.129=attack\n"),
        ("conquer", "armor.ini", "Armor130103", "999.999.130.103=super\n999.100.130.103=stand\n999.401.130.103=attack\n"),
        ("five", "weapon.ini", "415050", "0.1.999.415.050_0=super\n0.1.999.415.050_1=other\n0.1.100.415.050_0=attack\n"),
    ] {
        let f = Folder::new(label);
        f.write("ini/3dobj.ini", "1=c3/mesh/blade.c3\n");
        f.write("ini/3dtexture.ini", "1=c3/texture/blade.dds\n");
        f.write(&format!("ini/{catalog}"), format!("[{section}]\nMesh=1\nTexture=1\n"));
        f.write("ini/Action3DEffect.ini", actions);
        let assets = ClientAssets::quick(&f.0, &AtomicBool::new(false)).unwrap();
        let object = assets.object(&format!("{catalog}:{section}")).unwrap();
        let bindings = assets.effect_bindings(object);
        assert!(bindings.iter().any(|binding| binding.effect == "super" && binding.enabled), "{label}");
        assert!(
            bindings.iter().filter(|binding| binding.effect != "super").all(|binding| binding.enabled == (label == "five" && binding.effect == "other")),
            "{label}"
        );
        assert!(!bindings.iter().any(|binding| binding.effect == "unrelated"));
    }
}

#[test]
fn wildcard_body_is_not_a_persistent_action_and_shared_conquer_skin_prefers_super() {
    let f = Folder::new("conquer-super");
    f.write("ini/3dobj.ini", "1=c3/mesh/blade.c3\n");
    f.write("ini/3deffectobj.ini", "9=c3/effect/glow.c3\n");
    f.write("ini/3dtexture.ini", "1=c3/texture/410008.dds\n2=c3/texture/glow.dds\n");
    f.write("ini/weapon.ini", "[410008]\nMesh0=1\nTexture0=1\n[410009]\nMesh0=1\nTexture0=1\n");
    f.write("ini/3deffect.ini", "[Flash4102]\nAmount=1\nEffectId0=9\nTextureId0=2\n[410009]\nAmount=1\nEffectId0=9\nTextureId0=2\n");
    f.write("ini/Action3DEffect.ini", "999.401.410.008=Flash4102\n999.401.410.009=Flash4102\n999.999.410.009=410009\n");
    let assets = ClientAssets::quick(&f.0, &AtomicBool::new(false)).unwrap();
    let objects = assets.objects_for_texture("c3/texture/410008.dds");
    assert_eq!(objects[0].id, "weapon.ini:410009");
    assert!(assets.effect_bindings(assets.object("weapon.ini:410008").unwrap()).iter().all(|binding| !binding.enabled));
    let bindings = assets.effect_bindings(&objects[0]);
    assert!(bindings.iter().any(|binding| binding.effect == "410009" && binding.enabled));
    assert!(bindings.iter().filter(|binding| binding.effect == "Flash4102").all(|binding| !binding.enabled));
}
#[test]
fn transformation_only_glow_does_not_override_the_shared_weapons_ordinary_super_variant() {
    let f = Folder::new("ordinary-super");
    f.write("ini/3dobj.ini", "410240=c3/mesh/blade.c3\n");
    f.write("ini/3dtexture.ini", "410240=c3/texture/410240.dds\n");
    f.write("ini/weapon.ini", "[410240]\nMesh0=410240\nTexture0=410240\n[410244]\nMesh0=410240\nTexture0=410240\n");
    // Actual modern Eudemons key shapes: 531 has a different weapon's glow.
    f.write("ini/Action3DEffect.ini", "0.0.531.999.410.240=410210+v_r_weapon\n0.0.1.350.410.240=a410109+v_r_weapon\n0.0.286.999.410.244=aj410109+v_r_weapon\n0.0.1.999.410.244=410109+v_r_weapon\n0.0.2.999.410.244=410109+v_r_weapon\n");
    let assets = ClientAssets::quick(&f.0, &AtomicBool::new(false)).unwrap();
    let objects = assets.objects_for_texture("c3/texture/410240.dds");
    assert_eq!(objects[0].id, "weapon.ini:410244");
    let ordinary = assets.effect_bindings(&objects[0]);
    assert!(ordinary.iter().any(|binding| binding.effect == "410109+v_r_weapon" && binding.enabled));
    assert!(ordinary.iter().filter(|binding| binding.effect != "410109+v_r_weapon").all(|binding| !binding.enabled));
    let transformed = assets.effect_bindings(assets.object("weapon.ini:410240").unwrap());
    assert!(transformed.iter().any(|binding| binding.effect == "410210+v_r_weapon"));
    assert!(transformed.iter().all(|binding| !binding.enabled));
}

#[test]
fn ordinary_look_numbers_accept_padding_and_nonweapon_looks_keep_their_context() {
    let f = Folder::new("appearance-context");
    f.write("ini/3dobj.ini", "1=c3/mesh/body.c3\n");
    f.write("ini/3dtexture.ini", "1=c3/texture/body.dds\n");
    f.write("ini/weapon.ini", "[410244]\nMesh0=1\nTexture0=1\n");
    f.write("ini/armor.ini", "[130103]\nMesh0=1\nTexture0=1\n");
    f.write("ini/Action3DEffect.ini", "0.0.001.999.410.244=ordinary\n0.0.531.999.410.244=transformed\n0.0.531.999.130.103=armor-look\n");
    let assets = ClientAssets::quick(&f.0, &AtomicBool::new(false)).unwrap();
    let bindings = assets.effect_bindings(assets.object("weapon.ini:410244").unwrap());
    assert!(bindings.iter().any(|binding| binding.effect == "ordinary" && binding.enabled));
    assert!(bindings.iter().filter(|binding| binding.effect == "transformed").all(|binding| !binding.enabled));
    assert!(assets.effect_bindings(assets.object("armor.ini:130103").unwrap()).iter().any(|binding| binding.effect == "armor-look" && binding.enabled));
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

#[test]
fn multipart_rows_remain_objects_and_shared_textures_are_not_alternative_meshes() {
    let f = Folder::new("multipart");
    f.write("ini/3dobj.ini", "1=c3/mesh/body.c3\n2=c3/mesh/trim.c3\n3=c3/mesh/other.c3\n");
    f.write("ini/3dtexture.ini", "10=c3/texture/body.dds\n20=c3/texture/trim.dds\n");
    f.write(
        "ini/armor.ini",
        "[outfit]\nPart=3\nMesh2=2\nTexture2=20\nMesh0=1\nTexture0=10\nMesh1=3\nTexture1=10\nMesh3=1\nTexture3=10\n[other]\nMesh0=3\nTexture0=10\n",
    );
    let assets = ClientAssets::quick(&f.0, &AtomicBool::new(false)).unwrap();
    let objects = assets.objects_for_texture("body.dds");
    assert_eq!(objects.len(), 2);
    let object = assets.object("armor.ini:outfit").unwrap();
    assert_eq!(object.parts.len(), 3);
    assert_eq!(object.parts.iter().map(|p| p.slot).collect::<Vec<_>>(), [0, 1, 2]);
    assert_eq!(object.parts[2].texture.as_deref(), Some("c3/texture/trim.dds"));
    assert_eq!(assets.objects_for_texture("trim.dds").len(), 1);
}

#[test]
fn wdf_packages_use_forward_slash_hashes_and_reject_bad_extents() {
    let f = Folder::new("wdf");
    let payload = b"DDS synthetic texture payload";
    let start = 12 + payload.len() as u32;
    let mut package = b"PFDW".to_vec();
    package.extend(1u32.to_le_bytes());
    package.extend(start.to_le_bytes());
    package.extend(payload);
    // Observed resource ID for c3/texture/410008.dds in Conquer5187 c3.wdf.
    package.extend(1_631_850_657u32.to_le_bytes());
    package.extend(12u32.to_le_bytes());
    package.extend((payload.len() as u32).to_le_bytes());
    package.extend(0u32.to_le_bytes());
    f.write("c3.wdf", &package);
    let assets = ClientAssets::quick(&f.0, &AtomicBool::new(false)).unwrap();
    assert_eq!(assets.read("C3/Texture/410008.dds", 100).unwrap(), payload);
    assert!(assets.read("c3/texture/410008.dds", 1).is_err());
    assert!(assets.read("c3/texture/missing.dds", 100).is_err());
    let offset = package.len() - 12;
    package[offset..offset + 4].copy_from_slice(&u32::MAX.to_le_bytes());
    f.write("c3.wdf", &package);
    let assets = ClientAssets::quick(&f.0, &AtomicBool::new(false)).unwrap();
    assert!(assets.read("c3/texture/410008.dds", 100).is_err());
    f.write("c3.wdf", b"PFDW\xff\xff\xff\xff\x0c\0\0\0");
    let assets = ClientAssets::quick(&f.0, &AtomicBool::new(false)).unwrap();
    assert!(assets.read("c3/texture/410008.dds", 100).is_err());
}

#[test]
fn dawnpack_1000_plain_indexes_load_and_encrypted_1002_is_reported() {
    for version in [1000u32, 1002] {
        let f = Folder::new(&format!("dnp-{version}"));
        let payload = b"MAXFILE C3 00001";
        let mut package = vec![0; 40];
        package[..18].copy_from_slice(b"DawnPack.TqDigital");
        package[32..36].copy_from_slice(&version.to_le_bytes());
        package[36..40].copy_from_slice(&1u32.to_le_bytes());
        let masks = if version == 1000 { [0; 3] } else { [0x95279527, 0x96120059, 0x99589958] };
        for (value, mask) in [0x13fb8830u32, payload.len() as u32, 52].into_iter().zip(masks) {
            package.extend((value ^ mask).to_le_bytes());
        }
        package.extend(payload);
        f.write("c3.dnp", package);
        let assets = ClientAssets::quick(&f.0, &AtomicBool::new(false)).unwrap();
        if version == 1000 {
            assert_eq!(assets.read("c3/mesh/001000000.c3", 100).unwrap(), payload);
        } else {
            assert!(assets.read("c3/mesh/001000000.c3", 100).unwrap_err().to_string().contains("encryption"));
        }
        assert!(assets.read("c3/mesh/001000000.c3", 1).is_err());
    }
}

#[test]
fn multipart_effects_and_missing_registry_parts_keep_their_identity() {
    let f = Folder::new("multipart-effects");
    f.write("ini/3deffectobj.ini", "1=c3/effect/glow.c3\n");
    f.write("ini/3dtexture.ini", "10=c3/texture/glow.dds\n");
    f.write("ini/3deffect.ini", "[glow]\nEffectId0=1\nTextureId0=10\nEffectId1=999\nTextureId1=10\nEffectId999999999=1\nTextureId999999999=10\n");
    let assets = ClientAssets::quick(&f.0, &AtomicBool::new(false)).unwrap();
    let objects = assets.objects_for_texture("glow.dds");
    assert_eq!(objects.len(), 1);
    assert_eq!(objects[0].parts.len(), 2);
    assert!(objects[0].parts[1].mesh.is_none());
    assert!(objects[0].parts[1].note.as_ref().unwrap().contains("999"));
}

#[test]
fn decompiled_textures_find_parent_registries_and_exact_object_paths_win() {
    let f = Folder::new("parent-ini");
    f.write("decomp/c3/effect/one/skin.dds", b"texture");
    f.write("decomp/c3/effect/two/skin.dds", b"texture");
    f.write("ini/3dobj.ini", "1=c3/mesh/body.c3\n2=c3/mesh/other.c3\n");
    f.write("ini/3dtexture.ini", "10=c3/effect/one/skin.dds\n20=c3/effect/two/skin.dds\n");
    f.write("ini/armor.ini", "[one]\nMesh0=1\nTexture0=10\n[two]\nMesh0=2\nTexture0=20\n");
    let texture = f.0.join("decomp/c3/effect/two/skin.dds");
    let expected = f.0.canonicalize().unwrap();
    assert_eq!(infer_root(&texture).unwrap(), expected);
    assert_eq!(catalog_root(&f.0.join("decomp")).unwrap(), expected);
    let assets = ClientAssets::quick(&f.0.join("decomp"), &AtomicBool::new(false)).unwrap();
    assert_eq!(assets.root, expected);
    let objects = assets.objects_for_texture(&texture.to_string_lossy());
    assert_eq!(objects.len(), 1);
    assert_eq!(objects[0].id, "armor.ini:two");
    assert!(assets.editable_path("../outside.dds").is_none());
}

#[test]
fn large_valid_effect_catalog_does_not_block_an_unrelated_armor_object() {
    let f = Folder::new("large-effects");
    f.write("ini/3dobj.ini", "1=c3/mesh/body.c3\n");
    f.write("ini/3dtexture.ini", "10=c3/texture/body.dds\n");
    f.write("ini/armor.ini", "[outfit]\nMesh0=1\nTexture0=10\n");
    f.write("ini/3deffect.ini", "Unused=0\n".repeat(576_988));
    let assets = ClientAssets::quick(&f.0, &AtomicBool::new(false)).unwrap();
    assert_eq!(assets.objects_for_texture("body.dds")[0].id, "armor.ini:outfit");
    assert!(assets.warnings.is_empty());
}

#[test]
fn malformed_optional_catalog_is_reported_without_disabling_other_objects() {
    let f = Folder::new("bad-effects");
    f.write("ini/3dobj.ini", "1=c3/mesh/body.c3\n");
    f.write("ini/3dtexture.ini", "10=c3/texture/body.dds\n");
    f.write("ini/armor.ini", "[outfit]\nMesh0=1\nTexture0=10\n");
    f.write("ini/3deffect.ini", "x".repeat(16_385));
    let assets = ClientAssets::quick(&f.0, &AtomicBool::new(false)).unwrap();
    assert_eq!(assets.objects_for_texture("body.dds")[0].id, "armor.ini:outfit");
    assert!(assets.warnings.iter().any(|warning| warning.contains("Skipped 3deffect.ini")));
}

#[test]
fn declared_missing_parts_and_meshes_without_texture_fields_remain_inspectable() {
    let f = Folder::new("missing-fields");
    f.write("ini/3dobj.ini", "1=c3/mesh/body.c3\n2=c3/mesh/trim.c3\n");
    f.write("ini/3dtexture.ini", "10=c3/texture/body.dds\n20=c3/texture/trim.dds\n");
    f.write("ini/armor.ini", "[outfit]\nPart=4\nMesh0=1\nTexture0=10\nMesh1=2\nTexture2=20\n");
    let assets = ClientAssets::quick(&f.0, &AtomicBool::new(false)).unwrap();
    let objects = assets.objects_for_texture("body.dds");
    let parts = &objects[0].parts;
    assert_eq!(parts.len(), 4);
    assert_eq!(parts[1].mesh.as_deref(), Some("c3/mesh/trim.c3"));
    assert!(parts[1].texture.is_none() && parts[1].note.is_some());
    assert!(parts[2].mesh.is_none());
    assert_eq!(parts[2].texture.as_deref(), Some("c3/texture/trim.dds"));
    assert!(parts[3].mesh.is_none() && parts[3].texture.is_none() && parts[3].note.is_some());
    assert_eq!(assets.objects_for_texture("trim.dds").len(), 1);
}

#[test]
fn class_weapon_body_prefixes_resolve_quality_and_matching_sex() {
    for family in [450u32, 480] {
        let f = Folder::new(&format!("class-{family}"));
        f.write("ini/3dobj.ini", "1=c3/mesh/male.c3\n2=c3/mesh/female.c3\n");
        f.write("ini/3dtexture.ini", "1=c3/texture/shared.dds\n");
        let male = 1_000_000 + family * 1000;
        let female = 2_000_000 + family * 1000;
        f.write("ini/weapon.ini", format!("[{male}]\nMesh0=1\nTexture0=1\n[{}]\nMesh0=1\nTexture0=1\n[{}]\nMesh0=2\nTexture0=1\n", male + 4, female + 4));
        f.write("ini/Action3DEffect.ini", format!("0.0.1.999.{family}.004=male\n0.0.2.999.{family}.004=female\n0.0.531.999.{family}.004=transformed\n"));
        let assets = ClientAssets::quick(&f.0, &AtomicBool::new(false)).unwrap();
        let objects = assets.objects_for_texture("shared.dds");
        assert_eq!(objects[0].id, format!("weapon.ini:{}", male + 4));
        for (section, effect) in [(male + 4, "male"), (female + 4, "female")] {
            let bindings = assets.effect_bindings(assets.object(&format!("weapon.ini:{section}")).unwrap());
            assert_eq!(bindings.iter().filter(|binding| binding.enabled).map(|binding| binding.effect.as_str()).collect::<Vec<_>>(), [effect]);
        }
    }
}

#[test]
fn prefixed_legacy_weapon_sections_keep_their_actor_and_all_channels() {
    let f = Folder::new("legacy-class-weapon");
    f.write("ini/3dobj.ini", "1=c3/mesh/claw.c3\n");
    f.write("ini/3dtexture.ini", "1=c3/texture/claw.dds\n");
    f.write("ini/weapon.ini", "[Weapon1455010]\nMesh=1\nTexture=1\n");
    f.write("ini/Action3DEffect.ini", "1.999.455.010_0=outer\n1.999.455.010_1=inner\n2.999.455.010_0=female\n");
    let assets = ClientAssets::quick(&f.0, &AtomicBool::new(false)).unwrap();
    let bindings = assets.effect_bindings(assets.object("weapon.ini:Weapon1455010").unwrap());
    assert_eq!(bindings.iter().filter(|binding| binding.enabled).map(|binding| binding.effect.as_str()).collect::<Vec<_>>(), ["outer", "inner"]);
}

#[test]
fn shared_weapon_texture_prefers_complete_super_over_basic_persistent_glow() {
    let f = Folder::new("super-over-basic");
    f.write("ini/3dobj.ini", "1=c3/mesh/staff.c3\n");
    f.write("ini/3dtexture.ini", "1=c3/texture/430040.dds\n");
    f.write("ini/weapon.ini", "[430040]\nMesh0=1\nTexture0=1\n[430044]\nMesh0=1\nTexture0=1\n");
    f.write(
        "ini/Action3DEffect.ini",
        "0.0.1.999.430.040=basic\n0.0.1.999.430.044_0=base\n0.0.1.999.430.044_1=color\n0.0.1.999.430.044_2=super\n0.0.2.999.430.044_0=other-sex\n",
    );
    let assets = ClientAssets::quick(&f.0, &AtomicBool::new(false)).unwrap();
    let objects = assets.objects_for_texture("430040.dds");
    assert_eq!(objects[0].id, "weapon.ini:430044");
    assert_eq!(
        assets.effect_bindings(&objects[0]).iter().filter(|binding| binding.enabled).map(|binding| binding.effect.as_str()).collect::<Vec<_>>(),
        ["base", "color", "super"]
    );
}

#[test]
fn unused_registered_texture_keeps_object_effects_and_overrides_only_its_mesh() {
    let f = Folder::new("registered-unused-texture");
    f.write("ini/3dobj.ini", "2455030=c3/weapon/female.c3\n9=c3/weapon/trim.c3\n");
    f.write("ini/3dtexture.ini", "2455030=c3/texture/2455030.dds\n455030=c3/texture/455030.dds\n9=c3/texture/trim.dds\n");
    f.write("ini/weapon.ini", "[2455030]\nPart=2\nMesh0=2455030\nTexture0=455030\nMesh1=9\nTexture1=9\n");
    f.write("ini/Action3DEffect.ini", "0.0.2.999.455.030=glow\n");
    let assets = ClientAssets::quick(&f.0, &AtomicBool::new(false)).unwrap();
    let objects = assets.objects_for_texture("2455030.dds");
    assert_eq!(objects.len(), 1);
    assert_eq!(objects[0].id, "weapon.ini:2455030");
    assert_eq!(objects[0].parts[0].texture.as_deref(), Some("c3/texture/2455030.dds"));
    assert_eq!(objects[0].parts[1].texture.as_deref(), Some("c3/texture/trim.dds"));
    assert!(assets.effect_bindings(&objects[0]).iter().any(|binding| binding.effect == "glow" && binding.enabled));
    assert_eq!(assets.object("weapon.ini:2455030").unwrap().parts[0].texture.as_deref(), Some("c3/texture/455030.dds"));
}

#[test]
fn registered_mesh_alias_never_overrides_a_direct_texture_binding_or_guesses_neighbors() {
    let f = Folder::new("registered-alias-priority");
    f.write("ini/3dobj.ini", "7=c3/mesh/one.c3\n8=c3/mesh/two.c3\n");
    f.write("ini/3dtexture.ini", "7=c3/texture/a.dds\n8=c3/texture/b.dds\n9=c3/texture/unpaired.dds\n");
    f.write("ini/weapon.ini", "[one]\nMesh0=7\nTexture0=8\n[two]\nMesh0=8\nTexture0=7\n");
    let assets = ClientAssets::quick(&f.0, &AtomicBool::new(false)).unwrap();
    assert_eq!(assets.objects_for_texture("a.dds")[0].id, "weapon.ini:two");
    assert!(assets.objects_for_texture("unpaired.dds").is_empty());
}
