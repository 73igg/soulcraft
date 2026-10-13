//! DDS textures, including every mip, cube face, array item and volume slice.
//! Header and block codecs use the image-rs DDS implementation; the optional
//! BC6H encoder is supplied by the MIT-licensed, pure Rust OxideAV codec.

use std::io::Cursor;

use ::dds::{ColorFormat, DataLayout, DataRegion, ImageView, ImageViewMut, Precision, Size};
pub use ::dds::{Format as PixelFormat, header::Header};

use crate::{ChannelLayout, CodecError, Format, Image, Limits, SampleType};

/// Settings shared by desktop, CLI and headless exports. `auto` retains an
/// imported texture's storage; for new images it selects lossless storage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DdsOptions {
    pub format: String,
    /// `preserve`, `generate` (full chain), or `none`.
    pub mipmaps: String,
    /// `preserve`, `image`, `array`, `cube`, or `volume`.
    pub structure: String,
    /// None preserves the source flag (new colour images use their profile).
    pub srgb: Option<bool>,
    /// `fast`, `normal`, or `high` (BC6H uses its own HDR quality search).
    pub quality: String,
}

impl Default for DdsOptions {
    fn default() -> Self {
        Self { format: "auto".into(), mipmaps: "preserve".into(), structure: "preserve".into(), srgb: None, quality: "normal".into() }
    }
}

impl DdsOptions {
    pub fn validate(&self) -> Result<(), CodecError> {
        if self.format != "auto" {
            pixel_format(&self.format)?;
        }
        if !["preserve", "generate", "none"].contains(&self.mipmaps.as_str()) {
            return Err(CodecError::encode(Format::Dds, "DDS mipmaps must be preserve, generate or none"));
        }
        if !["preserve", "image", "array", "cube", "volume"].contains(&self.structure.as_str()) {
            return Err(CodecError::encode(Format::Dds, "DDS structure must be preserve, image, array, cube or volume"));
        }
        if !["fast", "normal", "high"].contains(&self.quality.as_str()) {
            return Err(CodecError::encode(Format::Dds, "DDS quality must be fast, normal or high"));
        }
        Ok(())
    }
}

pub const EXPORT_FORMATS: &[(&str, &str)] = &[
    ("auto", "Automatic / original"),
    ("bc1", "BC1 / DXT1"),
    ("bc2", "BC2 / DXT3"),
    ("bc3", "BC3 / DXT5"),
    ("bc4", "BC4 / ATI1 (red)"),
    ("bc4s", "BC4 signed"),
    ("bc5", "BC5 / ATI2 (red + green)"),
    ("bc5s", "BC5 signed"),
    ("bc6h", "BC6H HDR (unsigned)"),
    ("bc6hs", "BC6H HDR (signed)"),
    ("bc7", "BC7 RGBA"),
    ("rgba8", "RGBA 8-bit"),
    ("bgra8", "BGRA 8-bit"),
    ("rgb8", "RGB 8-bit"),
    ("rgb565", "RGB 5:6:5"),
    ("rgba5551", "RGBA 5:5:5:1"),
    ("rgba4444", "RGBA 4:4:4:4"),
    ("r8", "R 8-bit"),
    ("rg8", "RG 8-bit"),
    ("a8", "Alpha 8-bit"),
    ("r16", "R 16-bit"),
    ("rg16", "RG 16-bit"),
    ("rgba16", "RGBA 16-bit"),
    ("rgba16f", "RGBA half float"),
    ("rgba32f", "RGBA float"),
    ("r16f", "R half float"),
    ("r32f", "R float"),
    ("rg16f", "RG half float"),
    ("rg32f", "RG float"),
    ("rgb32f", "RGB float"),
    ("rgb10a2", "RGB 10-bit / alpha 2-bit"),
    ("rgb11f", "RGB 11:11:10 float"),
    ("rgb9e5", "RGB shared exponent"),
    ("dxt2", "DXT2 premultiplied"),
    ("dxt4", "DXT4 premultiplied"),
    ("rxgb", "RXGB"),
    ("dxt5nm", "DXT5 normal map"),
];

pub fn pixel_format(name: &str) -> Result<PixelFormat, CodecError> {
    use PixelFormat as P;
    let p = match name.to_ascii_lowercase().as_str() {
        "bc1" | "dxt1" => P::BC1_UNORM,
        "bc2" | "dxt3" => P::BC2_UNORM,
        "bc3" | "dxt5" => P::BC3_UNORM,
        "bc4" | "ati1" => P::BC4_UNORM,
        "bc4s" => P::BC4_SNORM,
        "bc5" | "ati2" => P::BC5_UNORM,
        "bc5s" => P::BC5_SNORM,
        "bc6h" => P::BC6H_UF16,
        "bc6hs" => P::BC6H_SF16,
        "bc7" => P::BC7_UNORM,
        "rgba8" => P::R8G8B8A8_UNORM,
        "bgra8" => P::B8G8R8A8_UNORM,
        "rgb8" => P::R8G8B8_UNORM,
        "rgb565" => P::B5G6R5_UNORM,
        "rgba5551" => P::B5G5R5A1_UNORM,
        "rgba4444" => P::B4G4R4A4_UNORM,
        "r8" => P::R8_UNORM,
        "rg8" => P::R8G8_UNORM,
        "a8" => P::A8_UNORM,
        "r16" => P::R16_UNORM,
        "rg16" => P::R16G16_UNORM,
        "rgba16" => P::R16G16B16A16_UNORM,
        "rgba16f" => P::R16G16B16A16_FLOAT,
        "rgba32f" => P::R32G32B32A32_FLOAT,
        "r16f" => P::R16_FLOAT,
        "r32f" => P::R32_FLOAT,
        "rg16f" => P::R16G16_FLOAT,
        "rg32f" => P::R32G32_FLOAT,
        "rgb32f" => P::R32G32B32_FLOAT,
        "rgb10a2" => P::R10G10B10A2_UNORM,
        "rgb11f" => P::R11G11B10_FLOAT,
        "rgb9e5" => P::R9G9B9E5_SHAREDEXP,
        "dxt2" => P::BC2_UNORM_PREMULTIPLIED_ALPHA,
        "dxt4" => P::BC3_UNORM_PREMULTIPLIED_ALPHA,
        "rxgb" => P::BC3_UNORM_RXGB,
        "dxt5nm" => P::BC3_UNORM_NORMAL,
        _ => return Err(CodecError::encode(Format::Dds, format!("unknown DDS pixel format: {name}"))),
    };
    Ok(p)
}

pub fn automatic_format(image: &Image) -> PixelFormat {
    use PixelFormat as P;
    match (image.layout(), image.sample_type()) {
        (ChannelLayout::Gray, SampleType::U8) => P::R8_UNORM,
        (ChannelLayout::Gray, SampleType::U16) => P::R16_UNORM,
        (ChannelLayout::Gray, SampleType::F16) => P::R16_FLOAT,
        (ChannelLayout::Gray, SampleType::F32) => P::R32_FLOAT,
        (_, SampleType::U16) => P::R16G16B16A16_UNORM,
        (_, SampleType::F16) => P::R16G16B16A16_FLOAT,
        (ChannelLayout::Rgb, SampleType::F32) => P::R32G32B32_FLOAT,
        (_, SampleType::F32) => P::R32G32B32A32_FLOAT,
        (ChannelLayout::Rgb, _) => P::R8G8B8_UNORM,
        _ => P::R8G8B8A8_UNORM,
    }
}

/// On-disk placement, used to retain untouched compressed blocks byte for byte.
#[derive(Debug, Clone)]
pub struct SurfaceInfo {
    pub width: u32,
    pub height: u32,
    pub mip: u32,
    pub item: u32,
    pub depth: u32,
    pub offset: usize,
    pub len: usize,
    pub name: String,
}

pub fn inspect(bytes: &[u8]) -> Result<(Header, PixelFormat, Vec<SurfaceInfo>), CodecError> {
    let decoder = ::dds::Decoder::new(Cursor::new(bytes)).map_err(|e| CodecError::malformed(Format::Dds, e))?;
    let header = decoder.header().clone();
    let format = alpha_format(&header, decoder.format());
    let layout = decoder.layout();
    let start = 4usize.checked_add(header.byte_len()).ok_or_else(|| CodecError::malformed(Format::Dds, "header overflow"))?;
    let stored = usize::try_from(layout.data_len()).map_err(|_| CodecError::malformed(Format::Dds, "texture too large"))?;
    if bytes.len().saturating_sub(start) < stored {
        return Err(CodecError::malformed(Format::Dds, "truncated DDS texture data"));
    }
    let mut surfaces = Vec::new();
    let mut add = |desc: ::dds::SurfaceDescriptor, mip: u32, item: u32, depth: u32, name: String| -> Result<(), CodecError> {
        if surfaces.len() >= 65536 {
            return Err(CodecError::LimitExceeded("DDS has more than 65536 surfaces".into()));
        }
        let offset =
            usize::try_from(desc.data_offset()).ok().and_then(|o| start.checked_add(o)).ok_or_else(|| CodecError::malformed(Format::Dds, "offset overflow"))?;
        let len = usize::try_from(desc.data_len()).map_err(|_| CodecError::malformed(Format::Dds, "surface too large"))?;
        surfaces.push(SurfaceInfo { width: desc.width(), height: desc.height(), mip, item, depth, offset, len, name });
        Ok(())
    };
    match layout {
        DataLayout::Texture(t) => {
            for (mip, desc) in t.iter_mips().enumerate() {
                add(desc, mip as u32, 0, 0, format!("DDS - Mip {mip}"))?;
            }
        }
        DataLayout::TextureArray(a) => {
            let face_names: Vec<&str> = if let Some(h) = header.dx9() {
                let faces = h.cube_map_faces().unwrap_or_else(::dds::CubeMapFaces::empty);
                [
                    (::dds::CubeMapFaces::POSITIVE_X, "+X"),
                    (::dds::CubeMapFaces::NEGATIVE_X, "-X"),
                    (::dds::CubeMapFaces::POSITIVE_Y, "+Y"),
                    (::dds::CubeMapFaces::NEGATIVE_Y, "-Y"),
                    (::dds::CubeMapFaces::POSITIVE_Z, "+Z"),
                    (::dds::CubeMapFaces::NEGATIVE_Z, "-Z"),
                ]
                .into_iter()
                .filter(|(f, _)| faces.contains(*f))
                .map(|(_, n)| n)
                .collect()
            } else {
                vec!["+X", "-X", "+Y", "-Y", "+Z", "-Z"]
            };
            for (item, t) in a.iter().enumerate() {
                for (mip, desc) in t.iter_mips().enumerate() {
                    let name = if header.is_cube_map() {
                        let count = face_names.len().max(1);
                        format!("DDS - Cube {} Face {} - Mip {mip}", item / count, face_names.get(item % count).copied().unwrap_or("?"))
                    } else {
                        format!("DDS - Array {item} - Mip {mip}")
                    };
                    add(desc, mip as u32, item as u32, 0, name)?;
                }
            }
        }
        DataLayout::Volume(v) => {
            for (mip, volume) in v.iter_mips().enumerate() {
                for (depth, desc) in volume.iter_depth_slices().enumerate() {
                    add(desc, mip as u32, 0, depth as u32, format!("DDS - Depth {depth} - Mip {mip}"))?;
                }
            }
        }
    }
    Ok((header, format, surfaces))
}

/// Only unknown legacy channel masks use the compatibility reader. A known
/// format with invalid or truncated mip data must retain its decoding error.
pub fn uses_legacy_masks(bytes: &[u8]) -> bool {
    Header::read(&mut Cursor::new(bytes), &::dds::header::ParseOptions::default())
        .ok()
        .is_some_and(|header| matches!(PixelFormat::from_header(&header), Err(::dds::FormatError::UnsupportedPixelFormat)))
}

fn image_color(format: PixelFormat) -> (ChannelLayout, SampleType, ColorFormat) {
    let mut native = format.color();
    if signed_channels(format).is_some() {
        native.precision = Precision::F32;
    }
    let layout = match native.channels {
        ::dds::Channels::Grayscale => ChannelLayout::Gray,
        ::dds::Channels::Rgb => ChannelLayout::Rgb,
        _ => ChannelLayout::Rgba,
    };
    let sample = match native.precision {
        Precision::U8 => SampleType::U8,
        Precision::U16 => SampleType::U16,
        Precision::F32 => SampleType::F32,
    };
    let channels = match layout {
        ChannelLayout::Gray => ::dds::Channels::Grayscale,
        ChannelLayout::Rgb => ::dds::Channels::Rgb,
        _ => ::dds::Channels::Rgba,
    };
    (layout, sample, ColorFormat::new(channels, native.precision))
}

/// Whether the header needs an alpha transform beyond the format-specific codec.
pub fn header_premultiplies(header: &Header, format: PixelFormat) -> bool {
    header.alpha_mode() == ::dds::header::AlphaMode::Premultiplied
        && format.color().channels == ::dds::Channels::Rgba
        && (header.is_srgb() || !matches!(format, PixelFormat::BC2_UNORM_PREMULTIPLIED_ALPHA | PixelFormat::BC3_UNORM_PREMULTIPLIED_ALPHA))
}

fn alpha_format(header: &Header, format: PixelFormat) -> PixelFormat {
    if header.alpha_mode() != ::dds::header::AlphaMode::Premultiplied {
        return format;
    }
    match format {
        PixelFormat::BC2_UNORM => PixelFormat::BC2_UNORM_PREMULTIPLIED_ALPHA,
        PixelFormat::BC3_UNORM => PixelFormat::BC3_UNORM_PREMULTIPLIED_ALPHA,
        other => other,
    }
}

fn straight_format(format: PixelFormat) -> PixelFormat {
    match format {
        PixelFormat::BC2_UNORM_PREMULTIPLIED_ALPHA => PixelFormat::BC2_UNORM,
        PixelFormat::BC3_UNORM_PREMULTIPLIED_ALPHA => PixelFormat::BC3_UNORM,
        other => other,
    }
}

pub fn decoded_sample(format: PixelFormat) -> SampleType {
    if matches!(format, PixelFormat::R16_FLOAT | PixelFormat::R16G16_FLOAT | PixelFormat::R16G16B16A16_FLOAT) { SampleType::F16 } else { image_color(format).1 }
}

pub fn decode_surface(bytes: &[u8], format: PixelFormat, info: &SurfaceInfo, limits: &Limits) -> Result<Image, CodecError> {
    let header = Header::read(&mut Cursor::new(bytes), &::dds::header::ParseOptions::default()).map_err(|e| CodecError::malformed(Format::Dds, e))?;
    let transform_alpha = header_premultiplies(&header, format);
    let format = if transform_alpha { straight_format(format) } else { format };
    let (layout, sample, color) = image_color(format);
    limits.check(info.width, info.height, layout, sample)?;
    let size = Size::new(info.width, info.height);
    let len = color.buffer_size(size).ok_or_else(|| CodecError::LimitExceeded("DDS surface too large".into()))?;
    let mut data = Vec::new();
    data.try_reserve_exact(len).map_err(|_| CodecError::LimitExceeded("cannot allocate DDS surface".into()))?;
    data.resize(len, 0);
    let end = info.offset.checked_add(info.len).ok_or_else(|| CodecError::malformed(Format::Dds, "offset overflow"))?;
    let encoded = bytes.get(info.offset..end).ok_or_else(|| CodecError::malformed(Format::Dds, "truncated surface"))?;
    if matches!(format, PixelFormat::BC3_UNORM | PixelFormat::BC3_UNORM_PREMULTIPLIED_ALPHA | PixelFormat::BC3_UNORM_RXGB) {
        // BC3 always has four colour entries, even when c0 <= c1. The image-rs
        // 0.2 decoder treats that case as BC1; use the independent BC3 decoder.
        let mut rgba = vec![0; info.width as usize * info.height as usize * 4];
        oxideav_dds::decode_bc3(encoded, info.width, info.height, &mut rgba).map_err(|e| CodecError::malformed(Format::Dds, e))?;
        // Match DirectXTex's normalized alpha interpolation (nearest integer),
        // rather than OxideAV's integer truncation at seventh-step values.
        for (index, block) in encoded.as_chunks::<16>().0.iter().enumerate() {
            let (a, b) = (u32::from(block[0]), u32::from(block[1]));
            let mut table = [a as u8, b as u8, 0, 0, 0, 0, 0, 255];
            for (i, value) in table.iter_mut().enumerate().skip(2) {
                if a > b {
                    *value = (((8 - i) as u32 * a + (i - 1) as u32 * b + 3) / 7) as u8;
                } else if i < 6 {
                    *value = (((6 - i) as u32 * a + (i - 1) as u32 * b + 2) / 5) as u8;
                }
            }
            let bits = block.get(2..8).unwrap_or_default().iter().enumerate().fold(0u64, |v, (i, byte)| v | (u64::from(*byte) << (i * 8)));
            let blocks_x = info.width.div_ceil(4) as usize;
            for t in 0..16 {
                let (x, y) = (index % blocks_x * 4 + t % 4, index / blocks_x * 4 + t / 4);
                if x < info.width as usize && y < info.height as usize {
                    let offset = (y * info.width as usize + x) * 4 + 3;
                    if let (Some(dst), Some(alpha)) = (rgba.get_mut(offset), table.get(((bits >> (t * 3)) & 7) as usize)) {
                        *dst = *alpha;
                    }
                }
            }
        }
        for pixel in rgba.as_chunks_mut::<4>().0 {
            if format == PixelFormat::BC3_UNORM_PREMULTIPLIED_ALPHA {
                let alpha = u32::from(pixel[3]);
                for value in pixel.iter_mut().take(3) {
                    *value = (u32::from(*value) * 255 + alpha / 2).checked_div(alpha).unwrap_or(0).min(255) as u8;
                }
            } else if format == PixelFormat::BC3_UNORM_RXGB {
                pixel[0] = pixel[3];
                pixel[3] = 255;
            }
        }
        data = Image::from_u8(info.width, info.height, ChannelLayout::Rgba, rgba)?.convert(layout, sample).data().to_vec();
    } else {
        let view = ImageViewMut::new(&mut data, size, color).ok_or_else(|| CodecError::malformed(Format::Dds, "invalid image buffer"))?;
        ::dds::decode(&mut Cursor::new(encoded), view, format, &::dds::DecodeOptions::default()).map_err(|e| CodecError::malformed(Format::Dds, e))?;
    }
    let mut image = Image::from_raw(info.width, info.height, layout, sample, data)?;
    if let Some(stored_channels) = signed_channels(format) {
        let mut values = image.to_normalized();
        for pixel in values.chunks_exact_mut(layout.channels()) {
            for value in pixel.iter_mut().take(stored_channels) {
                *value = *value * 2.0 - 1.0;
            }
        }
        image = Image::from_normalized(info.width, info.height, layout, SampleType::F32, &values)?;
    }
    if transform_alpha {
        limits.check(info.width, info.height, ChannelLayout::Rgba, SampleType::F32)?;
        image = transform_premultiplied(&image, header.is_srgb(), false)?;
    }
    // Legacy A8L8 uses the same bit storage as RG8, with G carrying alpha.
    // Preserve its grey + alpha meaning in the editor.
    if format == PixelFormat::R8G8_UNORM && bytes.get(80..84).is_some_and(|flags| flags == 0x20001u32.to_le_bytes()) {
        let values = image.to_normalized();
        let channels = image.layout().channels();
        let graya: Vec<f32> = values.chunks_exact(channels).flat_map(|p| [p[0], p[1]]).collect();
        image = Image::from_normalized(info.width, info.height, ChannelLayout::GrayA, sample, &graya)?;
    }
    if matches!(format, PixelFormat::R16_FLOAT | PixelFormat::R16G16_FLOAT | PixelFormat::R16G16B16A16_FLOAT) {
        Ok(image.convert(image.layout(), SampleType::F16))
    } else {
        Ok(image)
    }
}

pub(crate) fn decode(bytes: &[u8], limits: &Limits) -> Result<Image, CodecError> {
    if let Some(dimensions) = bytes.get(12..20) {
        let parts = dimensions.as_chunks::<4>().0;
        if let (Some(height), Some(width)) = (parts.first(), parts.get(1)) {
            limits.check(u32::from_le_bytes(*width), u32::from_le_bytes(*height), ChannelLayout::Rgba, SampleType::U8)?;
        }
    }
    let full = (|| {
        let (_, format, surfaces) = inspect(bytes)?;
        decode_surface(bytes, format, surfaces.first().ok_or_else(|| CodecError::malformed(Format::Dds, "no surfaces"))?, limits)
    })();
    // The existing reader also handles arbitrary legacy channel masks. Keep that
    // compatibility path when a mask has no named DXGI/image-rs storage format.
    match full {
        Ok(image) => Ok(image),
        Err(error) if uses_legacy_masks(bytes) => crate::codecs::dds::decode(bytes, limits).or(Err(error)),
        Err(error) => Err(error),
    }
}

fn view_color(image: &Image) -> ColorFormat {
    let precision = match image.sample_type() {
        SampleType::U8 => Precision::U8,
        SampleType::U16 => Precision::U16,
        _ => Precision::F32,
    };
    let channels = match image.layout() {
        ChannelLayout::Gray => ::dds::Channels::Grayscale,
        ChannelLayout::Rgb => ::dds::Channels::Rgb,
        _ => ::dds::Channels::Rgba,
    };
    ColorFormat::new(channels, precision)
}

fn signed_channels(format: PixelFormat) -> Option<usize> {
    use PixelFormat as P;
    match format {
        P::BC4_SNORM | P::R8_SNORM | P::R16_SNORM => Some(1),
        P::BC5_SNORM | P::R8G8_SNORM | P::R16G16_SNORM => Some(2),
        P::R8G8B8A8_SNORM | P::R16G16B16A16_SNORM => Some(4),
        _ => None,
    }
}

pub fn encode_surface(image: &Image, format: PixelFormat, opts: &DdsOptions) -> Result<Vec<u8>, CodecError> {
    opts.validate()?;
    Limits::default().check(image.width(), image.height(), image.layout(), image.sample_type())?;
    if matches!(format, PixelFormat::BC6H_UF16 | PixelFormat::BC6H_SF16) {
        let rgb = image.convert(ChannelLayout::Rgb, SampleType::F32).to_normalized();
        let len = ::dds::PixelInfo::from(format)
            .surface_bytes(Size::new(image.width(), image.height()))
            .and_then(|n| usize::try_from(n).ok())
            .ok_or_else(|| CodecError::encode(Format::Dds, "texture too large"))?;
        let mut out = vec![0; len];
        // BC6H is expensive; independent block rows are encoded across CPU cores.
        #[cfg(not(target_arch = "wasm32"))]
        use rayon::prelude::*;
        let row_bytes = image.width().div_ceil(4) as usize * 16;
        let input_row = image.width() as usize * 3;
        #[cfg(not(target_arch = "wasm32"))]
        let rows = out.par_chunks_mut(row_bytes);
        #[cfg(target_arch = "wasm32")]
        let rows = out.chunks_mut(row_bytes);
        rows.enumerate().try_for_each(|(row, dst)| {
            let y = row * 4;
            let h = (image.height() as usize - y).min(4) as u32;
            let start = y * input_row;
            let end = start + h as usize * input_row;
            let src = rgb.get(start..end).ok_or_else(|| CodecError::encode(Format::Dds, "invalid HDR row"))?;
            let result = if format == PixelFormat::BC6H_SF16 {
                oxideav_dds::encode_bc6h_sf16_from_f32(src, image.width(), h, dst)
            } else {
                oxideav_dds::encode_bc6h_from_f32(src, image.width(), h, dst)
            };
            result.map_err(|e| CodecError::encode(Format::Dds, e))
        })?;
        return Ok(out);
    }
    let layout = if image.layout() == ChannelLayout::Gray {
        ChannelLayout::Gray
    } else if image.layout() == ChannelLayout::Rgb {
        ChannelLayout::Rgb
    } else {
        ChannelLayout::Rgba
    };
    let sample = if image.sample_type() == SampleType::F16 { SampleType::F32 } else { image.sample_type() };
    let mut image = if image.layout() == ChannelLayout::GrayA && format == PixelFormat::R8G8_UNORM {
        let values = image.to_normalized();
        let rg: Vec<f32> = values.as_chunks::<2>().0.iter().flat_map(|p| [p[0], p[1], 0.0]).collect();
        Image::from_normalized(image.width(), image.height(), ChannelLayout::Rgb, sample, &rg)?
    } else {
        image.convert(layout, sample)
    };
    if let Some(stored_channels) = signed_channels(format) {
        let mut values = image.to_normalized();
        for pixel in values.chunks_exact_mut(layout.channels()) {
            for value in pixel.iter_mut().take(stored_channels) {
                *value = *value * 0.5 + 0.5;
            }
        }
        image = Image::from_normalized(image.width(), image.height(), image.layout(), SampleType::F32, &values)?;
    }
    let view = ImageView::new(image.data(), Size::new(image.width(), image.height()), view_color(&image))
        .ok_or_else(|| CodecError::encode(Format::Dds, "invalid image buffer"))?;
    let mut encoding = ::dds::EncodeOptions::default();
    encoding.quality = match opts.quality.as_str() {
        "fast" => ::dds::CompressionQuality::Fast,
        "high" => ::dds::CompressionQuality::High,
        _ => ::dds::CompressionQuality::Normal,
    };
    let mut out = Vec::new();
    ::dds::encode(&mut out, view, format, None, &encoding).map_err(|e| CodecError::encode(Format::Dds, e))?;
    Ok(out)
}

pub fn set_header_format(header: Header, format: PixelFormat, srgb: bool) -> Result<Header, CodecError> {
    let base = Header::new_image(header.width(), header.height(), format);
    if !srgb
        && let Header::Dx9(shape) = &header
        && let Some(storage) = base.to_dx9()
    {
        return Ok(shape.clone().with_pixel_format(storage.pixel_format).into());
    }
    if let (Some(shape), Some(storage)) = (header.to_dx10(), base.to_dx10()) {
        let dxgi = if srgb { storage.dxgi_format.to_srgb() } else { storage.dxgi_format.to_linear() };
        if srgb && !dxgi.is_srgb() {
            return Err(CodecError::encode(Format::Dds, "this DDS pixel format cannot signal sRGB; choose linear"));
        }
        let alpha = if PixelFormat::from_header(&header).is_ok_and(|f| alpha_format(&header, f) == format) { header.alpha_mode() } else { storage.alpha_mode };
        return Ok(shape.with_dxgi_format(dxgi).with_alpha_mode(alpha).into());
    }
    if srgb {
        return Err(CodecError::encode(Format::Dds, "this legacy DDS format cannot signal sRGB; choose linear"));
    }
    if let (Header::Dx9(shape), Header::Dx9(storage)) = (header, base) {
        return Ok(shape.with_pixel_format(storage.pixel_format).into());
    }
    Err(CodecError::encode(Format::Dds, "cannot represent texture shape with this pixel format"))
}

pub fn write_header(header: &Header) -> Result<Vec<u8>, CodecError> {
    let mut bytes = Vec::new();
    header.write(&mut bytes).map_err(|e| CodecError::encode(Format::Dds, e))?;
    Ok(bytes)
}

fn transform_premultiplied(image: &Image, srgb: bool, multiply: bool) -> Result<Image, CodecError> {
    let rgba = image.convert(ChannelLayout::Rgba, image.sample_type());
    let mut values = rgba.to_normalized();
    for pixel in values.as_chunks_mut::<4>().0 {
        let alpha = pixel[3];
        for channel in pixel.iter_mut().take(3) {
            let value = if srgb { linear(*channel) } else { *channel };
            let value = if multiply {
                value * alpha
            } else if alpha.abs() > 0.0 {
                value / alpha
            } else {
                0.0
            };
            *channel = if srgb { gamma(value) } else { value };
        }
    }
    Image::from_normalized(rgba.width(), rgba.height(), rgba.layout(), rgba.sample_type(), &values)
}

/// Encode a subresource using its container's alpha and colour-space semantics.
pub fn encode_surface_with_header(image: &Image, header: &Header, format: PixelFormat, opts: &DdsOptions) -> Result<Vec<u8>, CodecError> {
    opts.validate()?;
    Limits::default().check(image.width(), image.height(), image.layout(), image.sample_type())?;
    if header_premultiplies(header, format) {
        Limits::default().check(image.width(), image.height(), ChannelLayout::Rgba, SampleType::F32)?;
        let image = transform_premultiplied(image, header.is_srgb(), true)?;
        return encode_surface(&image, straight_format(format), opts);
    }
    encode_surface(image, format, opts)
}

/// Area-filter a mip level. sRGB colour samples are averaged in linear light;
/// alpha is filtered with premultiplied colour, then returned as straight alpha.
pub fn downsample(images: &[Image], srgb: bool) -> Result<Image, CodecError> {
    let first = images.first().ok_or_else(|| CodecError::encode(Format::Dds, "missing mip source"))?;
    let (w, h) = (first.width(), first.height());
    if images.iter().any(|i| i.dimensions() != (w, h)) {
        return Err(CodecError::encode(Format::Dds, "mip sources have different dimensions"));
    }
    let (nw, nh) = ((w / 2).max(1), (h / 2).max(1));
    let src: Vec<Vec<f32>> = images.iter().map(|i| i.convert(ChannelLayout::Rgba, SampleType::F32).to_normalized()).collect();
    let mut pixels = Vec::with_capacity(nw as usize * nh as usize * 4);
    for y in 0..nh {
        for x in 0..nw {
            let (x0, x1) = (x as u64 * w as u64 / nw as u64, (x + 1) as u64 * w as u64 / nw as u64);
            let (y0, y1) = (y as u64 * h as u64 / nh as u64, (y + 1) as u64 * h as u64 / nh as u64);
            let mut sum = [0.0f32; 4];
            let mut count = 0.0;
            for plane in &src {
                for sy in y0..y1 {
                    for sx in x0..x1 {
                        let offset = ((sy * w as u64 + sx) * 4) as usize;
                        let p = plane.get(offset..offset + 4).ok_or_else(|| CodecError::encode(Format::Dds, "mip source out of bounds"))?;
                        for c in 0..3 {
                            let v = if srgb { linear(p[c]) } else { p[c] };
                            sum[c] += v * p[3];
                        }
                        sum[3] += p[3];
                        count += 1.0;
                    }
                }
            }
            let alpha = sum[3];
            for value in sum.iter_mut().take(3) {
                *value = if alpha.abs() > 1e-12 { *value / alpha } else { 0.0 };
                if srgb {
                    *value = gamma(*value);
                }
            }
            sum[3] /= count;
            pixels.extend_from_slice(&sum);
        }
    }
    let rgba = Image::from_normalized(nw, nh, ChannelLayout::Rgba, SampleType::F32, &pixels)?;
    Ok(rgba.convert(first.layout(), first.sample_type()))
}

fn linear(v: f32) -> f32 {
    if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
}
fn gamma(v: f32) -> f32 {
    if v <= 0.0031308 { v * 12.92 } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 }
}

pub(crate) fn encode(image: &Image, opts: &DdsOptions) -> Result<Vec<u8>, CodecError> {
    opts.validate()?;
    let format = if opts.format == "auto" { automatic_format(image) } else { pixel_format(&opts.format)? };
    let srgb = opts.srgb.unwrap_or(false);
    let mut header = Header::new_image(image.width(), image.height(), format);
    if opts.mipmaps == "generate" {
        header = header.with_mipmaps();
    }
    // Retain legacy FourCC headers for older game engines when no DX10 signal is needed.
    if srgb {
        header = set_header_format(header, format, true)?;
    }
    let mut bytes = write_header(&header)?;
    let mut mip = image.clone();
    for level in 0..header.mipmap_count().get() {
        if level > 0 {
            mip = downsample(&[mip], srgb)?;
        }
        bytes.extend(encode_surface_with_header(&mip, &header, format, opts)?);
    }
    Ok(bytes)
}
