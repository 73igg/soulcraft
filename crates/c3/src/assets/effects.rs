//! Resolve effect graphs from the client's own catalogs, never numeric proximity.
use super::{Ini, ObjectDefinition, id, relative};
use crate::material::{Material, Timing};
use std::collections::HashMap;

#[derive(Clone, Debug)]
pub struct EffectBinding {
    pub effect: String,
    pub socket: String,
    pub source: String,
    pub enabled: bool,
}

struct ActionBinding {
    key: String,
    effect: String,
    action: u32,
    ordinary_appearance: bool,
    look: Option<u32>,
    actor_context: String,
}
struct ItemIdentity {
    item: String,
    look: u32,
    part: String,
}

#[derive(Default)]
pub(super) struct Catalog {
    materials: HashMap<String, Vec<Material>>,
    timing: HashMap<String, Timing>,
    part_timing: HashMap<String, Vec<Timing>>,
    definitions: HashMap<String, String>,
    part_effects: HashMap<String, HashMap<String, String>>,
    actions: HashMap<String, Vec<ActionBinding>>,
    identities: HashMap<String, ItemIdentity>,
    role_effects: HashMap<String, String>,
    role_references: HashMap<String, String>,
}

fn uint(fields: &HashMap<String, String>, key: &str, default: u32) -> u32 {
    fields.get(key).and_then(|v| v.trim().parse().ok()).unwrap_or(default)
}
fn number(fields: &HashMap<String, String>, key: &str, default: f32) -> f32 {
    fields.get(key).and_then(|v| v.trim().parse::<f32>().ok()).filter(|v| v.is_finite() && v.abs() <= 1.0e6).unwrap_or(default)
}
impl Catalog {
    pub(super) fn read(catalogs: &[(String, Ini)], textures: &HashMap<String, String>) -> Self {
        let mut out = Self::default();
        for (name, ini) in catalogs {
            if name == "roleparteffect.ini" {
                for section in &ini.sections {
                    let target = match section.name.as_str() {
                        "parteffect" => &mut out.role_effects,
                        "partreference" => &mut out.role_references,
                        _ => continue,
                    };
                    target.extend(section.fields.clone());
                }
                continue;
            }
            if matches!(name.as_str(), "armor.ini" | "weapon.ini") {
                for section in &ini.sections {
                    let key = section.name.to_lowercase();
                    let numeric = key.strip_prefix(name.trim_end_matches(".ini")).unwrap_or(&key);
                    if let Ok(number) = numeric.parse::<u32>()
                        && number >= 1_000_000
                        && (name == "armor.ini" || matches!(number / 1_000_000, 1 | 2))
                    {
                        // Armor rows encode body * 1,000,000 + item. Dyed rows
                        // (e.g. 011191500) reference the undyed body mesh
                        // (001191500); retain the row's item/quality, not the
                        // shared mesh's item, when resolving Action3DEffect.
                        let look = if name == "weapon.ini" {
                            number / 1_000_000
                        } else {
                            section
                                .fields
                                .get("mesh0")
                                .or_else(|| section.fields.get("mesh"))
                                .and_then(|value| value.parse::<u32>().ok())
                                .map(|mesh| mesh / 1_000_000)
                                .filter(|look| *look != 0)
                                .unwrap_or(number / 1_000_000)
                        };
                        let item = number % 1_000_000;
                        if let Some(part) = look.checked_mul(1_000_000).and_then(|body| body.checked_add(item)) {
                            out.identities.insert(format!("{name}:{key}"), ItemIdentity { item: item.to_string(), look, part: part.to_string() });
                        }
                    }
                }
            }
            if name == "parteffect.ini" {
                for section in &ini.sections {
                    out.part_effects.insert(section.name.to_lowercase(), section.fields.iter().map(|(key, value)| (id(key), value.clone())).collect());
                }
                continue;
            }
            if matches!(name.as_str(), "action3deffect.ini" | "action3deffect_new.ini" | "roleweapon3deffect.ini") {
                // The last three fields are action.family.suffix. Earlier
                // fields describe attachment contexts and can themselves be999
                // wildcards; they must not be mistaken for the action.
                for (key, value) in ini.sections.iter().flat_map(|section| section.fields.iter()) {
                    let fields = key.split('.').collect::<Vec<_>>();
                    let action_index = match fields.len() {
                        3 => 0,
                        4 => 1,
                        5 => 2,
                        6 => 3,
                        _ => continue,
                    };
                    let family_index = fields.len().saturating_sub(2);
                    let Some((family, suffix)) = fields.get(family_index).zip(fields.last()) else { continue };
                    let suffix = suffix.split_once('_').map(|(suffix, _)| suffix).unwrap_or(suffix);
                    if let (Ok(family), Ok(suffix), Some(action)) =
                        (family.parse::<u32>(), suffix.parse::<u32>(), fields.get(action_index).and_then(|v| v.parse::<u32>().ok()))
                        && suffix < 1000
                        && let Some(item) = family.checked_mul(1000).and_then(|v| v.checked_add(suffix))
                    {
                        let actor_context = fields
                            .iter()
                            .take(family_index)
                            .enumerate()
                            .filter(|(i, _)| *i != action_index)
                            .map(|(_, field)| *field)
                            .collect::<Vec<_>>()
                            .join(".");
                        // The field immediately before the action is the actor's
                        // look (1/2 for ordinary male/female; 999 is a wildcard).
                        // Transformation looks such as 531 must stay selectable,
                        // but cannot supply a standalone weapon's default glow.
                        let ordinary_appearance = action_index == 0
                            || fields
                                .get(action_index - 1)
                                .and_then(|field| field.parse::<u32>().ok())
                                .is_some_and(|look| matches!(look, 0 | 1 | 2 | 999 | 9999));
                        out.actions.entry(item.to_string()).or_default().push(ActionBinding {
                            key: key.clone(),
                            effect: value.clone(),
                            action,
                            ordinary_appearance,
                            look: action_index.checked_sub(1).and_then(|index| fields.get(index)).and_then(|field| field.parse().ok()),
                            actor_context,
                        });
                    }
                }
                continue;
            }
            for section in &ini.sections {
                let fields = &section.fields;
                let effect = fields.contains_key("amount") && fields.contains_key("effectid0");
                let count = uint(fields, if effect { "amount" } else { "part" }, 0).min(64);
                if count == 0 {
                    continue;
                }
                let key = format!("{name}:{}", section.name);
                let materials = (0..count)
                    .map(|slot| {
                        let mut material = Material {
                            blend: [uint(fields, &format!("asb{slot}"), 5), uint(fields, &format!("adb{slot}"), 6)],
                            depth_write: uint(fields, &format!("zbuffer{slot}"), u32::from(!effect)) != 0,
                            depth_test: uint(fields, &format!("ztest{slot}"), 1) != 0,
                            scale: number(fields, &format!("scale{slot}"), 100.) / 100.,
                            ..Material::default()
                        };
                        if material.scale == 0. {
                            material.scale = 1.;
                        }
                        material.billboard = uint(fields, &format!("billboard{slot}"), uint(fields, "billboard", 0)).min(3) as u8;
                        material.mix_mode = uint(fields, &format!("mixopt{slot}"), 0);
                        let alternate = if effect { format!("textureid{slot}_1") } else { format!("mixtex{slot}") };
                        material.mix_texture = fields
                            .get(&alternate)
                            .or_else(|| fields.get(&format!("textureid{slot}_m")))
                            .filter(|v| id(v) != "0")
                            .and_then(|v| textures.get(&id(v)))
                            .and_then(|p| relative(p).ok());
                        let prefix = format!("textureid{slot}_");
                        let mut references = fields
                            .iter()
                            .filter_map(|(key, value)| {
                                let stage = if let Some(suffix) = key.strip_prefix(&prefix) {
                                    if suffix == "m" { Some(1) } else { suffix.parse::<u32>().ok().filter(|n| *n > 0 && *n <= 16) }
                                } else if key == &format!("mixtex{slot}") {
                                    Some(1)
                                } else if key == &format!("thirdtex{slot}") {
                                    Some(2)
                                } else if key == &format!("fourthtex{slot}") {
                                    Some(3)
                                } else {
                                    None
                                };
                                stage
                                    .filter(|_| id(value) != "0")
                                    .and_then(|stage| textures.get(&id(value)).and_then(|path| relative(path).ok()).map(|path| (stage, path)))
                            })
                            .collect::<Vec<_>>();
                        references.sort();
                        references.dedup();
                        material.additional_textures = references.into_iter().map(|(_, path)| path).collect();
                        for stage in 1..=3 {
                            let key = if effect {
                                format!("textureid{slot}_{stage}")
                            } else {
                                format!(
                                    "{}{slot}",
                                    match stage {
                                        1 => "mixtex",
                                        2 => "thirdtex",
                                        _ => "fourthtex",
                                    }
                                )
                            };
                            let value = fields.get(&key).or_else(|| (stage == 1).then(|| fields.get(&format!("textureid{slot}_m"))).flatten());
                            if let Some(target) = material.texture_stages.get_mut(stage - 1) {
                                *target = value.filter(|v| id(v) != "0").and_then(|v| textures.get(&id(v))).and_then(|p| relative(p).ok());
                            }
                        }
                        material.mix_texture = material.texture_stages.first().cloned().flatten();
                        if let Some(data) = fields.get(&format!("mixdata{slot}")) {
                            for (dst, value) in material.mix_data.iter_mut().zip(data.split_whitespace()) {
                                *dst = value.parse::<f32>().ok().filter(|v| v.is_finite() && v.abs() <= 1.0e6).unwrap_or(0.);
                            }
                        }
                        material
                    })
                    .collect();
                out.materials.insert(key.clone(), materials);
                if effect {
                    let timing = Timing {
                        delay_ms: uint(fields, "delay", 0),
                        frame_ms: uint(fields, "frameinterval", 33).max(1),
                        loops: uint(fields, "looptime", u32::MAX),
                        loop_interval_ms: uint(fields, "loopinterval", 0),
                        offset: [number(fields, "offsetx", 0.), number(fields, "offsety", 0.), number(fields, "offsetz", 0.)],
                        manual_uv_step: uint(fields, "manualuvstep", 0) != 0,
                        ..Timing::default()
                    };
                    let parts = (0..count)
                        .map(|slot| {
                            let mut part = timing.clone();
                            part.step_frame_ms = timing.frame_ms;
                            part.frame_ms = uint(fields, &format!("interval{slot}"), 0);
                            if part.frame_ms == 0 {
                                part.frame_ms = timing.frame_ms;
                            }
                            part.frame_offset = fields.get(&format!("frameoffset{slot}")).and_then(|v| v.trim().parse::<i32>().ok()).unwrap_or(0);
                            part.loop_once = uint(fields, &format!("looponce{slot}"), 0) != 0;
                            for (axis, prefix) in ["xself", "yself", "zself"].into_iter().enumerate() {
                                if let Some(value) = part.offset.get_mut(axis) {
                                    *value += number(fields, &format!("{prefix}{slot}"), 0.);
                                }
                            }
                            part
                        })
                        .collect();
                    out.part_timing.insert(key.clone(), parts);
                    out.timing.insert(key.clone(), timing);
                    // Keep the legacy catalog authoritative; the newer catalog supplies absent names.
                    out.definitions.entry(section.name.to_lowercase()).or_insert(key);
                }
            }
        }
        for rows in out.actions.values_mut() {
            rows.sort_by(|a, b| (&a.key, &a.effect).cmp(&(&b.key, &b.effect)));
        }
        out
    }
    pub(super) fn material(&self, object: &str, slot: usize) -> Material {
        self.materials.get(object).and_then(|v| v.get(slot)).cloned().unwrap_or_default()
    }
    pub(super) fn textures(&self, object: &str) -> Vec<&str> {
        self.materials.get(object).into_iter().flatten().flat_map(|material| material.additional_textures.iter().map(String::as_str)).collect()
    }
    pub(super) fn timing(&self, object: &str) -> Timing {
        self.timing.get(object).cloned().unwrap_or_default()
    }
    pub(super) fn part_timing(&self, object: &str, slot: usize) -> Timing {
        self.part_timing.get(object).and_then(|v| v.get(slot)).cloned().unwrap_or_else(|| self.timing(object))
    }
    pub(super) fn definition(&self, name: &str) -> Option<&str> {
        self.definitions.get(&name.to_lowercase()).map(String::as_str)
    }

    pub(super) fn bindings(&self, object: &ObjectDefinition) -> Vec<EffectBinding> {
        let Some((catalog, section)) = object.id.split_once(':') else {
            return Vec::new();
        };
        let normalized_section = section.to_lowercase();
        let identity = self.identities.get(&object.id.to_lowercase());
        let section = normalized_section.as_str();
        // Older catalogs spell sections [Weapon410009] / [Armor130103].
        let section = section
            .strip_prefix(catalog.trim_end_matches(".ini"))
            .filter(|value| !value.is_empty() && value.bytes().all(|b| b.is_ascii_digit()))
            .unwrap_or(section);
        let groups: &[&str] = match catalog {
            "weapon.ini" => &["r_weapon", "l_weapon", "r_weapon_extend1", "r_weapon_extend2", "l_weapon_extend1", "l_weapon_extend2"],
            "armor.ini" => &["body", "l_arm", "r_arm", "l_arm2", "r_arm2", "l_hand", "r_hand", "l_leg", "r_leg"],
            "mount.ini" => &[
                "mount",
                "mount_extend1",
                "mount_extend2",
                "mount_extend3",
                "mount_extend4",
                "mount_extend5",
                "mount_extend6",
                "mount_extend7",
                "mount_extend8",
                "mount_extend9",
                "mount_extend10",
                "mount_extend11",
                "mount_extend12",
                "mount_extend13",
                "mount_extend14",
            ],
            "mantle.ini" => &["mantle", "mantle_extend1", "mantle_extend2"],
            "armet.ini" => &["headwear"],
            "misc.ini" => &["misc"],
            _ => &[],
        };
        let mut out: Vec<EffectBinding> = Vec::new();
        let mut seen = HashMap::<String, usize>::new();
        let mut add = |value: &str, source: String, enabled: bool| {
            for effect in value.split('|').map(str::trim).filter(|v| !v.is_empty() && *v != "0").take(32) {
                if let Some(index) = seen.get(&effect.to_lowercase()).copied() {
                    if enabled && let Some(binding) = out.get_mut(index) {
                        binding.enabled = true;
                        binding.source = source.clone();
                    }
                    continue;
                }
                seen.insert(effect.to_lowercase(), out.len());
                let socket = effect.split_once('+').map(|(_, socket)| socket).unwrap_or_default().to_string();
                out.push(EffectBinding { effect: effect.into(), socket, source: source.clone(), enabled });
            }
        };
        // RolePartEffect uses equipment types (4 = body, 9 = miscellaneous)
        // and item IDs separately from body-prefixed MESH object IDs.
        if let Some(part) = match catalog {
            "armor.ini" => Some(4),
            "misc.ini" => Some(9),
            _ => None,
        } && let Ok(number) = section.parse::<u32>()
        {
            let item = identity.map(|identity| identity.item.clone()).unwrap_or_else(|| (number % 1_000_000).to_string());
            let key = format!("{part}.{item}");
            let target = self.role_references.get(&key).unwrap_or(&key);
            if let Some(value) = self.role_effects.get(target) {
                add(value, format!("RolePartEffect {key} -> {target}"), true);
            }
        }
        let variants = |table: &HashMap<String, String>| {
            let mut keys = table
                .keys()
                .filter(|key| key.len() > section.len() && key.len() <= section.len() + 4 && key.ends_with(section) && key.bytes().all(|b| b.is_ascii_digit()))
                .cloned()
                .collect::<Vec<_>>();
            keys.sort();
            keys
        };
        let has_right = catalog == "weapon.ini"
            && groups
                .iter()
                .filter(|group| group.starts_with("r_"))
                .any(|group| self.part_effects.get(*group).is_some_and(|table| table.contains_key(&id(section)) || !variants(table).is_empty()));
        for group in groups {
            if let Some(table) = self.part_effects.get(*group) {
                let hand_enabled = !(has_right && group.starts_with("l_"));
                let part_key = id(section);
                let (part_key, exact) = table
                    .get(&part_key)
                    .map(|value| (part_key.clone(), Some(value)))
                    .unwrap_or_else(|| identity.map(|item| (item.part.clone(), table.get(&item.part))).unwrap_or((part_key, None)));
                if let Some(value) = exact {
                    add(value, format!("PartEffect [{group}] {part_key}"), hand_enabled);
                }
                // Body/class prefixes identify alternative attachment contexts.
                if exact.is_none() && catalog == "weapon.ini" && section.len() == 6 {
                    let keys = variants(table);
                    for (index, key) in keys.iter().enumerate() {
                        if let Some(value) = table.get(key) {
                            add(value, format!("PartEffect [{group}] {key}"), index == 0 && hand_enabled);
                        }
                    }
                }
            }
        }
        if !groups.is_empty()
            && let Some(actions) = self.actions.get(&id(section)).or_else(|| identity.and_then(|item| self.actions.get(&item.item)))
        {
            // 999 is the client's persistent action binding. Action-specific entries
            // remain separately selectable, and do not all play over one another.
            let applicable = |binding: &&ActionBinding| {
                (catalog != "weapon.ini" || binding.ordinary_appearance)
                    && identity.is_none_or(|item| binding.look.is_none_or(|look| look == item.look || matches!(look, 999 | 9999)))
            };
            // The native loader stores _N as independent effect channels.
            // All channels for the selected actor context play together.
            let context = actions.iter().filter(applicable).filter(|binding| matches!(binding.action, 999 | 9999)).map(|binding| &binding.actor_context).min();
            let fallback = actions.iter().filter(applicable).find(|binding| matches!(binding.action, 100 | 101));
            for binding in actions {
                add(
                    &binding.effect,
                    format!("Action3DEffect {}", binding.key),
                    applicable(&binding)
                        && match context {
                            Some(context) => matches!(binding.action, 999 | 9999) && &binding.actor_context == context,
                            None => fallback.is_some_and(|fallback| binding.actor_context == fallback.actor_context && binding.effect == fallback.effect),
                        },
                );
            }
        }
        out.sort_by_key(|binding| !binding.enabled);
        out.truncate(64);
        out
    }
}
