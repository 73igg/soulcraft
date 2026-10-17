//! HappyPets' whitespace-delimited role attachments, with explicit references.
use super::{Ini, IniSection, id};
use crate::{Error, Result};
use std::collections::HashMap;

pub(super) fn parse(bytes: &[u8]) -> Result<Ini> {
    let text = encoding_rs::GBK.decode(bytes).0;
    let mut sections = HashMap::<String, HashMap<String, String>>::new();
    let mut section = String::new();
    let mut count = 0usize;
    for line in text.lines() {
        if line.len() > 16_384 {
            return Err(Error("RolePartEffect line exceeds 16 KiB".into()));
        }
        let line = line.trim();
        if line.starts_with('[') {
            section = line.trim_matches(['[', ']']).to_lowercase();
            continue;
        }
        if !matches!(section.as_str(), "partreference" | "parteffect") || line.starts_with([';', '#', '/']) {
            continue;
        }
        let words = line.split_whitespace().collect::<Vec<_>>();
        let wanted = if section == "partreference" { 4 } else { 3 };
        if words.len() != wanted {
            continue;
        }
        count += 1;
        if count > 100_000 {
            return Err(Error("RolePartEffect exceeds the record budget".into()));
        }
        let Some((part, item)) = words.first().zip(words.get(1)) else { continue };
        if !part.bytes().all(|byte| byte.is_ascii_digit()) || !item.bytes().all(|byte| byte.is_ascii_digit()) {
            continue;
        }
        let value = if wanted == 4 {
            words.get(2).zip(words.get(3)).map(|(part, item)| format!("{}.{}", id(part), id(item))).unwrap_or_default()
        } else {
            words.get(2).copied().unwrap_or_default().into()
        };
        sections.entry(section.clone()).or_default().entry(format!("{}.{}", id(part), id(item))).or_insert(value);
    }
    Ok(Ini { flat: HashMap::new(), sections: sections.into_iter().map(|(name, fields)| IniSection { name, fields }).collect() })
}
