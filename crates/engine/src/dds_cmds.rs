//! DDS export and subresource queries use the same engine as the desktop UI.

use crate::{EngineError, Result, Session, commands::CommandSpec};
use serde_json::{Value, json};

fn export(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "file.export.dds";
    let path = crate::file_cmds::str_param(p, "path", cmd)?;
    if !path.to_ascii_lowercase().ends_with(".dds") {
        return Err(EngineError::BadParams { cmd: cmd.into(), msg: "path must end in .dds".into() });
    }
    let mut options = photocraft_io::ExportOptions::default();
    for (key, target) in [
        ("format", &mut options.encode.dds.format),
        ("mipmaps", &mut options.encode.dds.mipmaps),
        ("texture", &mut options.encode.dds.structure),
        ("quality", &mut options.encode.dds.quality),
    ] {
        if let Some(value) = p.get(key) {
            *target = value.as_str().ok_or_else(|| EngineError::BadParams { cmd: cmd.into(), msg: format!("{key} must be a string") })?.to_string();
        }
    }
    if p.get("color").is_some_and(|value| !value.is_string()) {
        return Err(EngineError::BadParams { cmd: cmd.into(), msg: "color must be a string".into() });
    }
    options.encode.dds.srgb = match p.get("color").and_then(Value::as_str) {
        None | Some("auto") => None,
        Some("srgb") => Some(true),
        Some("linear") => Some(false),
        _ => return Err(EngineError::BadParams { cmd: cmd.into(), msg: "color must be auto, srgb or linear".into() }),
    };
    options.encode.dds.validate().map_err(|e| EngineError::Other(e.to_string()))?;
    let state = s.active().ok_or(EngineError::NoDocument)?;
    let result = photocraft_io::export(&state.doc, path, &options).map_err(|e| EngineError::Other(e.to_string()))?;
    crate::file_cmds::write_file(path, &result.bytes)?;
    Ok(json!({"path": path, "bytes": result.bytes.len(), "warnings": result.warnings}))
}

fn inspect(s: &mut Session, _: &Value) -> Result<Value> {
    let state = s.active().ok_or(EngineError::NoDocument)?;
    let Some(source) = &state.doc.metadata.dds else {
        return Ok(json!({"dds": false}));
    };
    let (header, format, surfaces) = photocraft_codecs::dds::inspect(&source.bytes).map_err(|e| EngineError::Other(e.to_string()))?;
    let surfaces: Vec<Value> = surfaces
        .iter()
        .enumerate()
        .map(|(i, surface)| {
            json!({
                "layer": source.layers.get(i).map(|id| id.0), "name": surface.name,
                "width": surface.width, "height": surface.height, "mip": surface.mip,
                "item": surface.item, "depth": surface.depth,
            })
        })
        .collect();
    Ok(json!({"dds": true, "format": format!("{format:?}"), "srgb": header.is_srgb(),
        "cube": header.is_cube_map(), "volume": header.is_volume(), "arraySize": header.array_size(),
        "mipmaps": header.mipmap_count().get(), "surfaces": surfaces}))
}

fn has_doc(s: &Session) -> std::result::Result<(), String> {
    s.active().map(|_| ()).ok_or_else(|| "no document open".into())
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "file.export.dds",
            label: "Export DDS…",
            menu: &["File", "Export"],
            shortcut: None,
            params: "{\"path\":file.dds,\"format\":\"auto|bc1|bc2|bc3|bc4|bc5|bc6h|bc6hs|bc7|rgba8|rgba16|rgba16f|rgba32f|...\",\"mipmaps\":\"preserve|generate|none\",\"texture\":\"preserve|image|array|cube|volume\",\"color\":\"auto|srgb|linear\",\"quality\":\"fast|normal|high\"}",
            enabled: crate::file_cmds::native_doc,
            run: export,
            journal: true,
        },
        CommandSpec {
            id: "file.dds.inspect",
            label: "DDS Texture Information",
            menu: &[],
            shortcut: None,
            params: "{}",
            enabled: has_doc,
            run: inspect,
            journal: false,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn dds_commands_validate_params_without_writing() {
        let mut session = Session::new();
        assert!(session.execute("file.export.dds", json!({"path":"test.dds"})).is_err());
        session.execute("file.new", json!({"width":4,"height":4})).unwrap();
        for p in [
            json!({}),
            json!({"path":"test.png"}),
            json!({"path":"test.dds","format":"invalid"}),
            json!({"path":"test.dds","texture":42}),
            json!({"path":"test.dds","mipmaps":"invalid"}),
        ] {
            assert!(session.execute("file.export.dds", p).is_err());
        }
        assert_eq!(session.execute("file.dds.inspect", json!({})).unwrap()["dds"], false);
    }
}
