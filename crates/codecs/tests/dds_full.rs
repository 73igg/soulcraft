use photocraft_codecs::{self as codecs, ChannelLayout as L, EncodeOptions, Format, Image, Limits, SampleType as S, dds};

fn image(w: u32, h: u32, sample: S) -> Image {
    let values: Vec<f32> = (0..w * h).flat_map(|i| [i as f32 / (w * h) as f32, 0.25, 0.75, 1.0]).collect();
    Image::from_normalized(w, h, L::Rgba, sample, &values).unwrap()
}

#[test]
fn lossless_dds_preserves_all_depths_and_pixel_values() {
    for sample in S::ALL {
        for layout in [L::Gray, L::GrayA, L::Rgb, L::Rgba] {
            let original = image(7, 5, sample).convert(layout, sample);
            let bytes = codecs::encode(&original, Format::Dds, &EncodeOptions::default()).unwrap();
            let decoded = codecs::decode(&bytes).unwrap();
            assert_eq!(decoded.dimensions(), original.dimensions());
            assert_eq!(decoded.sample_type(), sample);
            assert_eq!(decoded.data(), original.convert(decoded.layout(), sample).data(), "{layout:?} {sample:?}");
        }
    }
    assert_eq!(codecs::from_extension("TEXTURE.DDS"), Some(Format::Dds));
}

#[test]
fn all_export_formats_open_and_standard_formats_have_an_independent_decoder() {
    for &(format, _) in dds::EXPORT_FORMATS {
        let mut options = EncodeOptions::default();
        options.dds.format = format.into();
        options.dds.quality = "fast".into();
        let bytes = codecs::encode(&image(7, 5, S::F32), Format::Dds, &options).unwrap_or_else(|e| panic!("{format}: {e}"));
        assert_eq!(codecs::decode(&bytes).unwrap().dimensions(), (7, 5), "{format}");
        // OxideAV accepts legacy BGR24 but has no RGB24 mask layout.
        if matches!(format, "rgb8" | "rgb32f") {
            continue;
        }
        let mut oracle_bytes = bytes.clone();
        if format == "rxgb" {
            oracle_bytes[84..88].copy_from_slice(b"DXT5");
        }
        let independent = oxideav_dds::decode(&oracle_bytes).unwrap_or_else(|e| panic!("oracle {format}: {e}"));
        assert_eq!((independent.width(), independent.height()), (7, 5));
    }
}

#[test]
fn microsoft_srgb_premultiplied_fixture_decodes_and_encodes_in_linear_light() {
    let bytes = include_bytes!("fixtures/dds-premultiplied-srgb.dds");
    let (header, format, surfaces) = dds::inspect(bytes).unwrap();
    let image = codecs::decode(bytes).unwrap();
    for (actual, expected) in image.data()[..4].iter().zip([64i16, 128, 192, 128]) {
        assert!((i16::from(*actual) - expected).abs() <= 1);
    }
    let exact = Image::from_u8(4, 4, L::Rgba, [64, 128, 192, 128].repeat(16)).unwrap();
    let encoded = dds::encode_surface_with_header(&exact, &header, format, &Default::default()).unwrap();
    assert_eq!(&encoded[..4], &bytes[surfaces[0].offset..surfaces[0].offset + 4]);
    let changed = dds::set_header_format(header, format, false).unwrap();
    assert_eq!(changed.alpha_mode(), ::dds::header::AlphaMode::Premultiplied);
}

#[test]
fn srgb_dx10_premultiplied_blocks_decode_to_straight_alpha() {
    let original = Image::from_normalized(4, 4, L::Rgba, S::F32, &[0.25, 0.5, 0.75, 0.5].repeat(16)).unwrap();
    for format in ["dxt2", "dxt4"] {
        let mut options = EncodeOptions::default();
        options.dds.format = format.into();
        options.dds.srgb = Some(true);
        let bytes = codecs::encode(&original, Format::Dds, &options).unwrap();
        let (header, _, _) = dds::inspect(&bytes).unwrap();
        assert!(header.is_srgb());
        assert_eq!(header.alpha_mode(), ::dds::header::AlphaMode::Premultiplied);
        let decoded = codecs::decode(&bytes).unwrap().to_normalized();
        for (a, b) in decoded[..4].iter().zip([0.25, 0.5, 0.75, 0.5]) {
            assert!((*a - b).abs() < 0.06, "{format}: {decoded:?}");
        }
    }
}

#[test]
fn bc6h_signed_and_unsigned_keep_hdr_values() {
    for (format, red) in [("bc6h", 4.0), ("bc6hs", -2.0)] {
        let original = Image::from_normalized(4, 4, L::Rgb, S::F32, &[red, 3.0, 1.0].repeat(16)).unwrap();
        let mut options = EncodeOptions::default();
        options.dds.format = format.into();
        let bytes = codecs::encode(&original, Format::Dds, &options).unwrap();
        let decoded = codecs::decode(&bytes).unwrap().to_normalized();
        assert!((decoded[0] - red).abs() < 0.15, "{format}: {}", decoded[0]);
        assert!((decoded[1] - 3.0).abs() < 0.15);
    }
}

#[test]
fn signed_channel_textures_preserve_negative_samples() {
    let original = Image::from_normalized(4, 4, L::Rgb, S::F32, &[-0.75, 0.5, 0.0].repeat(16)).unwrap();
    for format in ["bc4s", "bc5s"] {
        let mut options = EncodeOptions::default();
        options.dds.format = format.into();
        let bytes = codecs::encode(&original, Format::Dds, &options).unwrap();
        let decoded = codecs::decode(&bytes).unwrap();
        assert_eq!(decoded.sample_type(), S::F32);
        let values = decoded.to_normalized();
        assert!((values[0] + 0.75).abs() < 0.02, "{format}: {}", values[0]);
        if format == "bc5s" {
            assert!((values[1] - 0.5).abs() < 0.02);
        }
    }
}

#[test]
fn channel_textures_use_red_green_and_alpha_without_luminance_conversion() {
    let original = Image::from_u8(4, 4, L::Rgba, [64, 192, 0, 127].repeat(16)).unwrap();
    for (format, expected) in [("r8", 64), ("a8", 127), ("bc4", 64), ("bc5", 64)] {
        let mut options = EncodeOptions::default();
        options.dds.format = format.into();
        let bytes = codecs::encode(&original, Format::Dds, &options).unwrap();
        let decoded = codecs::decode(&bytes).unwrap();
        let value = if format == "a8" { decoded.to_rgba8()[3] } else { decoded.data()[0] };
        assert!((i32::from(value) - expected).abs() <= 2, "{format}: {value}");
        if format == "bc5" {
            assert!((i32::from(decoded.to_rgba8()[1]) - 192).abs() <= 2);
        }
    }
}

#[test]
fn mipmaps_filter_srgb_in_linear_light_and_preserve_alpha_edges() {
    let original = Image::from_u8(2, 1, L::Rgba, vec![0, 0, 0, 255, 255, 255, 255, 255]).unwrap();
    let mip = dds::downsample(&[original], true).unwrap();
    assert!((i32::from(mip.data()[0]) - 188).abs() <= 1);
    let original = Image::from_u8(2, 1, L::Rgba, vec![255, 0, 0, 255, 0, 0, 255, 0]).unwrap();
    let mip = dds::downsample(&[original], false).unwrap();
    assert_eq!(mip.data(), &[255, 0, 0, 128]);
}

#[test]
fn generated_mips_and_srgb_header_are_complete() {
    let mut options = EncodeOptions::default();
    options.dds.format = "bc7".into();
    options.dds.mipmaps = "generate".into();
    options.dds.srgb = Some(true);
    let bytes = codecs::encode(&image(7, 5, S::U8), Format::Dds, &options).unwrap();
    let (header, _, surfaces) = dds::inspect(&bytes).unwrap();
    assert!(header.is_srgb());
    assert_eq!(surfaces.iter().map(|s| (s.width, s.height)).collect::<Vec<_>>(), [(7, 5), (3, 2), (1, 1)]);
    for surface in surfaces {
        dds::decode_surface(&bytes, dds::pixel_format("bc7").unwrap(), &surface, &Limits::default()).unwrap();
    }
}

#[test]
fn truncated_and_hostile_dds_returns_errors_without_panics() {
    let bytes = codecs::encode(&image(4, 4, S::U8), Format::Dds, &EncodeOptions::default()).unwrap();
    for n in 0..bytes.len() {
        assert!(codecs::decode_as(Format::Dds, &bytes[..n]).is_err(), "truncation {n}");
    }
    let mut hostile = bytes.clone();
    hostile[16..20].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(codecs::decode(&hostile).is_err());
    for n in 1..20 {
        let mut hostile = bytes.clone();
        hostile[28..32].copy_from_slice(&(1u32 << n).to_le_bytes());
        let _ = codecs::decode(&hostile);
    }
    let mut options = EncodeOptions::default();
    options.dds.format = "unknown".into();
    assert!(codecs::encode(&image(4, 4, S::U8), Format::Dds, &options).is_err());
}
