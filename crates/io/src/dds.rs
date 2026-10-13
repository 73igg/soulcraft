//! DDS document mapping. Surfaces are ordinary editable layers, with the first
//! surface visible initially. Source bytes and layer ids are retained in .pcraft.

use std::sync::Arc;

use photocraft_codecs::{
    Image, Limits, SampleType,
    dds::{self as codec, DdsOptions, Header, PixelFormat},
};
use photocraft_doc::{DdsSource, Document, LayerId};
use photocraft_geom::Size;

use crate::{ExportOptions, ExportResult, ImportResult, IoError, flat};

fn unsupported(message: impl Into<String>) -> IoError {
    IoError::Unsupported(message.into())
}

pub fn import(name: &str, bytes: &[u8]) -> Result<ImportResult, IoError> {
    let (header, format, surfaces) = match codec::inspect(bytes) {
        Ok(info) => info,
        Err(_) if codec::uses_legacy_masks(bytes) => return flat::import_flat(name, bytes),
        Err(error) => return Err(error.into()),
    };
    let limits = Limits::default();
    // Account for the entire expanded texture, rather than only its first face.
    let mut total = 0u64;
    let mut tiled_total = 0u64;
    let tile = photocraft_geom::TILE_SIZE as u64;
    for s in &surfaces {
        limits.check(s.width, s.height, photocraft_codecs::ChannelLayout::Rgba, SampleType::F32)?;
        total = total.checked_add(s.width as u64 * s.height as u64 * 16).ok_or_else(|| unsupported("DDS allocation overflow"))?;
        let tile_pixels = (s.width as u64).div_ceil(tile) * (s.height as u64).div_ceil(tile) * tile * tile;
        let tile_bytes =
            tile_pixels.checked_mul(4 * codec::decoded_sample(format).bytes() as u64).ok_or_else(|| unsupported("DDS tile allocation overflow"))?;
        tiled_total = tiled_total.checked_add(tile_bytes).ok_or_else(|| unsupported("DDS tile allocation overflow"))?;
        if total > limits.max_alloc || tiled_total > limits.max_alloc {
            return Err(unsupported("DDS expanded surfaces exceed the memory limit"));
        }
    }
    let first = surfaces.first().ok_or_else(|| unsupported("DDS contains no surfaces"))?;
    let first_image = codec::decode_surface(bytes, format, first, &limits)?;
    let mut result = flat::image_to_document(name, &first_image)?;
    let mut layers = Vec::new();
    let mut ids = Vec::new();
    for (index, info) in surfaces.iter().enumerate() {
        let image = if index == 0 { first_image.clone() } else { codec::decode_surface(bytes, format, info, &limits)? };
        let part = flat::image_to_document(&info.name, &image)?;
        let mut layer = part.document.layers.into_iter().next().ok_or_else(|| unsupported("DDS surface has no layer"))?;
        layer.name = info.name.clone();
        layer.visible = index == 0;
        layer.locks = Default::default();
        ids.push(layer.id);
        layers.push(layer);
    }
    // The engine selects the top layer. Keep the first surface at the top.
    layers.reverse();
    result.document.layers = layers;
    result.document.metadata.dds = Some(DdsSource { bytes: Arc::new(bytes.to_vec()), layers: ids });
    if header.is_srgb() {
        result.document.icc_profile = Some(photocraft_cms::Builtin::Srgb.profile().to_bytes());
    } else if first_image.sample_type().is_float() && result.document.mode == photocraft_doc::ColorMode::Rgb {
        result.document.icc_profile = Some(photocraft_cms::Builtin::LinearSrgb.profile().to_bytes());
    }
    if surfaces.len() > 1 {
        result.warnings.push(format!("DDS {format:?}: {} editable surfaces. Only the first is visible; select a DDS layer and toggle visibility to view another. Save As DDS retains the texture layout and mip levels.", surfaces.len()));
    }
    Ok(result)
}

fn layer_image(doc: &Document, id: LayerId, width: u32, height: u32, extra_layers: bool) -> Result<Image, IoError> {
    let layer = doc.layer(id).ok_or_else(|| {
        unsupported("A DDS surface layer was deleted. Choose DDS Texture = Image to export the visible composite, or restore the missing layer.")
    })?;
    let mut alone = doc.clone();
    alone.size = Size::new(width, height);
    alone.selection = None;
    alone.metadata.dds = None;
    if extra_layers {
        let source = doc.metadata.dds.as_ref().ok_or_else(|| unsupported("missing DDS metadata"))?;
        for l in &mut alone.layers {
            if source.layers.contains(&l.id) {
                l.visible = l.id == id;
            }
        }
    } else {
        let mut layer = layer.clone();
        layer.visible = true;
        alone.layers = vec![layer];
    }
    // Each decoded subresource may have a different depth from the document.
    if let Some(s) = layer.surface() {
        alone.depth = s.format().sample;
        alone.mode = s.format().mode;
    }
    flat::document_to_image(&alone, &mut Vec::new())
}

fn is_srgb(doc: &Document) -> bool {
    doc.icc_profile
        .as_ref()
        .and_then(|bytes| photocraft_cms::Profile::parse(bytes).ok())
        .is_some_and(|profile| profile.to_bytes().as_slice() == photocraft_cms::Builtin::Srgb.profile().to_bytes().as_slice())
}

pub fn export(doc: &Document, opts: &ExportOptions) -> Result<ExportResult, IoError> {
    let settings = &opts.encode.dds;
    settings.validate()?;
    if settings.structure == "preserve" && doc.metadata.dds.is_some() {
        return export_original(doc, settings);
    }
    if matches!(settings.structure.as_str(), "preserve" | "image") {
        let mut options = opts.clone();
        // New DDS files default to lossless, with no mipmaps. sRGB only where
        // storage has an sRGB DXGI variant; HDR and channel data stay linear.
        if options.encode.dds.srgb.is_none() {
            let image = flat::document_to_image(doc, &mut Vec::new())?;
            let format = if settings.format == "auto" { codec::automatic_format(&image) } else { codec::pixel_format(&settings.format)? };
            options.encode.dds.srgb = Some(is_srgb(doc) && supports_srgb(format));
        }
        return flat::export_flat(doc, photocraft_codecs::Format::Dds, &options);
    }
    let mut images = Vec::new();
    // Top-to-bottom layer order is the order of cube faces / array items / slices.
    for layer in doc.layers.iter().rev() {
        images.push(layer_image(doc, layer.id, doc.size.width, doc.size.height, false)?);
    }
    if images.is_empty() {
        return Err(unsupported("DDS texture has no layers"));
    }
    let first = images.first().ok_or_else(|| unsupported("DDS texture has no layers"))?;
    let format = if settings.format == "auto" { codec::automatic_format(first) } else { codec::pixel_format(&settings.format)? };
    let srgb = settings.srgb.unwrap_or(is_srgb(doc) && supports_srgb(format));
    let count = u32::try_from(images.len()).map_err(|_| unsupported("too many DDS layers"))?;
    let mut header = match settings.structure.as_str() {
        "array" => Header::new_image(doc.size.width, doc.size.height, format)
            .to_dx10()
            .ok_or_else(|| unsupported("this storage format cannot be used for a texture array"))?
            .with_array_size(count)
            .into(),
        "cube" => {
            if count % 6 != 0 || doc.size.width != doc.size.height {
                return Err(unsupported("Cube maps need square images and six layers per cube, ordered +X, -X, +Y, -Y, +Z, -Z from top to bottom."));
            }
            let h = Header::new_cube_map(doc.size.width, doc.size.height, format);
            if count == 6 {
                h
            } else {
                h.to_dx10().ok_or_else(|| unsupported("cube arrays need a DX10-compatible storage format"))?.with_array_size(count / 6).into()
            }
        }
        "volume" => Header::new_volume(doc.size.width, doc.size.height, count, format),
        _ => return Err(unsupported("unknown DDS structure")),
    };
    if settings.mipmaps == "generate" {
        header = header.with_mipmaps();
    }
    if srgb {
        header = codec::set_header_format(header, format, true)?;
    }
    write_generated(header, format, images, settings, srgb)
}

fn supports_srgb(format: PixelFormat) -> bool {
    matches!(
        format,
        PixelFormat::BC1_UNORM
            | PixelFormat::BC2_UNORM
            | PixelFormat::BC3_UNORM
            | PixelFormat::BC2_UNORM_PREMULTIPLIED_ALPHA
            | PixelFormat::BC3_UNORM_PREMULTIPLIED_ALPHA
            | PixelFormat::BC7_UNORM
            | PixelFormat::R8G8B8A8_UNORM
            | PixelFormat::B8G8R8A8_UNORM
            | PixelFormat::B8G8R8X8_UNORM
    )
}

fn export_original(doc: &Document, opts: &DdsOptions) -> Result<ExportResult, IoError> {
    let source = doc.metadata.dds.as_ref().ok_or_else(|| unsupported("missing DDS source"))?;
    let (original_header, original_format, surfaces) = codec::inspect(&source.bytes)?;
    if source.layers.len() != surfaces.len() {
        return Err(unsupported("DDS surface mapping is incomplete"));
    }
    let format = if opts.format == "auto" { original_format } else { codec::pixel_format(&opts.format)? };
    let srgb = opts.srgb.unwrap_or(original_header.is_srgb());
    let has_extra = doc.layers.iter().any(|layer| !source.layers.contains(&layer.id));
    let resized = doc.size.width != original_header.width() || doc.size.height != original_header.height();
    if resized && opts.mipmaps == "preserve" {
        return Err(unsupported("The DDS image size changed. Choose Mipmaps = Generate or None in Export As to rebuild its texture layout."));
    }
    let mut header = original_header.clone();
    if format != original_format || srgb != header.is_srgb() {
        header = codec::set_header_format(header, format, srgb)?;
    }
    if resized {
        let depth = header.depth();
        header = header.with_dimensions(doc.size.width, doc.size.height, depth);
    }
    if opts.mipmaps == "generate" || opts.mipmaps == "none" {
        header = if opts.mipmaps == "generate" { header.with_mipmaps() } else { header.with_mipmap_count(1) };
        let mut images = Vec::new();
        for (index, _) in surfaces.iter().enumerate().filter(|(_, info)| info.mip == 0) {
            let id = *source.layers.get(index).ok_or_else(|| unsupported("missing DDS surface mapping"))?;
            images.push(layer_image(doc, id, doc.size.width, doc.size.height, index == 0 && has_extra)?);
        }
        return write_generated(header, format, images, opts, srgb);
    }
    let same_header = header == original_header;
    let start = surfaces.first().map(|s| s.offset).ok_or_else(|| unsupported("DDS has no surfaces"))?;
    let mut bytes =
        if same_header { source.bytes.get(..start).ok_or_else(|| unsupported("DDS header is truncated"))?.to_vec() } else { codec::write_header(&header)? };
    let mut unchanged = same_header;
    for (index, info) in surfaces.iter().enumerate() {
        let id = *source.layers.get(index).ok_or_else(|| unsupported("missing DDS surface mapping"))?;
        let image = layer_image(doc, id, info.width, info.height, index == 0 && has_extra)?;
        let original = codec::decode_surface(&source.bytes, original_format, info, &Limits::default())?;
        let comparable = image.convert(original.layout(), original.sample_type());
        let same_alpha = header.alpha_mode() == original_header.alpha_mode()
            && (header.is_srgb() == original_header.is_srgb()
                || (!codec::header_premultiplies(&header, format) && !codec::header_premultiplies(&original_header, original_format)));
        let same = format == original_format && same_alpha && comparable.data() == original.data();
        unchanged &= same;
        if same {
            let end = info.offset.checked_add(info.len).ok_or_else(|| unsupported("DDS offset overflow"))?;
            bytes.extend_from_slice(source.bytes.get(info.offset..end).ok_or_else(|| unsupported("DDS surface is truncated"))?);
        } else {
            bytes.extend(encode_surface(&image, &header, format, opts)?);
        }
    }
    if unchanged {
        bytes = source.bytes.as_ref().clone();
    } else if same_header && let Some(last) = surfaces.last() {
        let end = last.offset.checked_add(last.len).ok_or_else(|| unsupported("DDS offset overflow"))?;
        if let Some(tail) = source.bytes.get(end..) {
            bytes.extend_from_slice(tail);
        }
    }
    // Verify the complete encoded texture, including every declared subresource.
    codec::inspect(&bytes)?;
    let mut warnings = Vec::new();
    if has_extra {
        warnings.push("Additional editing layers were composited into the first DDS surface.".into());
    }
    if !unchanged && header.mipmap_count().get() > 1 {
        warnings.push("Existing mip levels were preserved. Choose Mipmaps = Generate to refresh them from the edited base images.".into());
    }
    Ok(ExportResult { bytes, warnings })
}

fn encode_surface(image: &Image, header: &Header, format: PixelFormat, opts: &DdsOptions) -> Result<Vec<u8>, IoError> {
    codec::encode_surface_with_header(image, header, format, opts).map_err(IoError::from)
}

fn write_generated(header: Header, format: PixelFormat, mut images: Vec<Image>, opts: &DdsOptions, srgb: bool) -> Result<ExportResult, IoError> {
    let mut bytes = codec::write_header(&header)?;
    if header.is_volume() {
        for mip in 0..header.mipmap_count().get() {
            if mip > 0 {
                let new_depth = (images.len() / 2).max(1);
                let old_depth = images.len();
                images = (0..new_depth)
                    .map(|z| {
                        let start = z * old_depth / new_depth;
                        let end = (z + 1) * old_depth / new_depth;
                        codec::downsample(images.get(start..end).ok_or_else(|| unsupported("invalid volume slice"))?, srgb).map_err(IoError::from)
                    })
                    .collect::<Result<Vec<_>, _>>()?;
            }
            for image in &images {
                bytes.extend(encode_surface(image, &header, format, opts)?);
            }
        }
    } else {
        for mut image in images {
            for mip in 0..header.mipmap_count().get() {
                if mip > 0 {
                    image = codec::downsample(&[image], srgb)?;
                }
                bytes.extend(encode_surface(&image, &header, format, opts)?);
            }
        }
    }
    codec::inspect(&bytes)?;
    Ok(ExportResult { bytes, warnings: Vec::new() })
}
