use photocraft_codecs::{ChannelLayout, Image, SampleType, dds};
use photocraft_doc::{ColorMode, Document, Layer, LayerContent, Size};
use photocraft_geom::Rect;
use photocraft_io::{self as io, ExportOptions};
use photocraft_raster::Surface;

fn texture(kind: &str, count: usize) -> Vec<u8> {
    let mut doc = Document::new("DDS test", Size::new(8, 8), ColorMode::Rgb, photocraft_doc::SampleType::U8);
    for i in 0..count {
        let mut surface = Surface::new(doc.pixel_format());
        surface.write_region(doc.bounds(), &[i as f32 / count as f32, 0.25, 0.5, 1.0].repeat(64));
        doc.layers.push(Layer::new(format!("Surface {i}"), LayerContent::Raster(surface)));
    }
    let mut options = ExportOptions::default();
    options.encode.dds.structure = kind.into();
    options.encode.dds.format = "bc3".into();
    options.encode.dds.mipmaps = "generate".into();
    io::export(&doc, "texture.dds", &options).unwrap().bytes
}

#[test]
fn arrays_cubes_cube_arrays_volumes_and_mips_round_trip_byte_exact() {
    for (kind, count) in [("array", 3), ("cube", 6), ("cube", 12), ("volume", 4)] {
        let bytes = texture(kind, count);
        let imported = io::import("texture.dds", &bytes).unwrap().document;
        assert!(imported.layers.len() > count);
        assert_eq!(imported.layers.iter().filter(|l| l.visible).count(), 1);
        assert_eq!(io::export(&imported, "out.dds", &ExportOptions::default()).unwrap().bytes, bytes, "{kind} {count}");
        let native = io::export(&imported, "project.pcraft", &ExportOptions::default()).unwrap().bytes;
        let reopened = io::import("project.pcraft", &native).unwrap().document;
        assert_eq!(io::export(&reopened, "out.dds", &ExportOptions::default()).unwrap().bytes, bytes, "native {kind} {count}");
    }
}

#[test]
fn missing_mip_or_last_subresource_is_rejected() {
    for (kind, count) in [("image", 1), ("array", 3), ("cube", 6), ("volume", 4)] {
        let bytes = texture(kind, count);
        assert!(io::import("truncated.dds", &bytes[..bytes.len() - 1]).is_err(), "{kind}");
        assert!(photocraft_codecs::decode(&bytes[..bytes.len() - 1]).is_err(), "flat {kind}");
    }
}

#[test]
fn signed_hdr_and_transparent_channel_data_survives_editor_and_projects() {
    for format in ["bc4s", "bc5s", "bc6hs", "rgba32f", "rgba8", "bc3"] {
        let alpha = if matches!(format, "rgba32f" | "rgba8" | "bc3") { 0.0 } else { 1.0 };
        let red = if format.starts_with("bc") && format != "bc3" { -0.75 } else { 0.75 };
        let image = Image::from_normalized(4, 4, ChannelLayout::Rgba, SampleType::F32, &[red, 0.5, 0.25, alpha].repeat(16)).unwrap();
        let mut options = photocraft_codecs::EncodeOptions::default();
        options.dds.format = format.into();
        let original = photocraft_codecs::encode(&image, photocraft_codecs::Format::Dds, &options).unwrap();
        let doc = io::import("channels.dds", &original).unwrap().document;
        assert_eq!(io::export(&doc, "channels.dds", &ExportOptions::default()).unwrap().bytes, original, "editor {format}");
        let project = io::export(&doc, "channels.pcraft", &ExportOptions::default()).unwrap().bytes;
        let doc = io::import("channels.pcraft", &project).unwrap().document;
        assert_eq!(io::export(&doc, "channels.dds", &ExportOptions::default()).unwrap().bytes, original, "project {format}");
    }
}

#[test]
fn tiny_arrays_cannot_bypass_the_tiled_document_allocation_budget() {
    let format = dds::pixel_format("rgba32f").unwrap();
    let header: dds::Header = dds::Header::new_image(1, 1, format).to_dx10().unwrap().with_array_size(8193).into();
    let mut bytes = dds::write_header(&header).unwrap();
    let pixel: Vec<u8> = [0.25f32, 0.5, 0.75, 1.0].into_iter().flat_map(f32::to_le_bytes).collect();
    bytes.extend(pixel.repeat(8193));
    let error = io::import("tiny-array.dds", &bytes).unwrap_err().to_string();
    assert!(error.contains("memory limit"), "{error}");
}

#[test]
fn dx10_premultiplied_float_surfaces_preserve_and_reencode_correctly() {
    let format = dds::pixel_format("rgba32f").unwrap();
    let alpha_mode = dds::Header::new_image(4, 4, dds::pixel_format("dxt4").unwrap()).alpha_mode();
    let header: dds::Header = dds::Header::new_image(4, 4, format).to_dx10().unwrap().with_alpha_mode(alpha_mode).into();
    let image = Image::from_normalized(4, 4, ChannelLayout::Rgba, SampleType::F32, &[0.125, 0.25, 0.375, 0.5].repeat(16)).unwrap();
    let mut bytes = dds::write_header(&header).unwrap();
    bytes.extend(dds::encode_surface(&image, format, &Default::default()).unwrap());
    assert_eq!(&photocraft_codecs::decode(&bytes).unwrap().to_normalized()[..4], &[0.25, 0.5, 0.75, 0.5]);
    let mut doc = io::import("premultiplied.dds", &bytes).unwrap().document;
    assert_eq!(io::export(&doc, "out.dds", &ExportOptions::default()).unwrap().bytes, bytes);
    let layer = doc.metadata.dds.as_ref().unwrap().layers[0];
    doc.layer_mut(layer).unwrap().surface_mut().unwrap().write_region(Rect::new(0, 0, 4, 4), &[0.0, 1.0, 0.0, 0.5].repeat(16));
    let edited = io::export(&doc, "out.dds", &ExportOptions::default()).unwrap().bytes;
    assert_eq!(dds::inspect(&edited).unwrap().0.alpha_mode(), alpha_mode);
    assert_eq!(&photocraft_codecs::decode(&edited).unwrap().to_normalized()[..4], &[0.0, 1.0, 0.0, 0.5]);
}

#[test]
fn editing_hidden_cube_face_preserves_every_other_surface_and_header() {
    let bytes = texture("cube", 6);
    let mut doc = io::import("cube.dds", &bytes).unwrap().document;
    let source = doc.metadata.dds.as_ref().unwrap();
    let index = 4;
    let layer = source.layers[index];
    doc.layer_mut(layer).unwrap().surface_mut().unwrap().write_region(Rect::new(0, 0, 8, 8), &[0.0, 1.0, 0.0, 1.0].repeat(64));
    let result = io::export(&doc, "edited.dds", &ExportOptions::default()).unwrap().bytes;
    let (header, format, surfaces) = dds::inspect(&result).unwrap();
    assert!(header.is_cube_map());
    assert_eq!(&result[..128], &bytes[..128]);
    for (i, surface) in surfaces.iter().enumerate() {
        if i == index {
            let image = dds::decode_surface(&result, format, surface, &Default::default()).unwrap().to_rgba8();
            assert!(image[1] > 250 && image[0] < 4);
        } else {
            assert_eq!(&result[surface.offset..surface.offset + surface.len], &bytes[surface.offset..surface.offset + surface.len], "surface {i}");
        }
    }
}

#[test]
fn mip_regeneration_updates_all_levels_and_none_removes_them() {
    let bytes = texture("array", 2);
    let mut doc = io::import("array.dds", &bytes).unwrap().document;
    let layer = doc.metadata.dds.as_ref().unwrap().layers[0];
    let bounds = doc.bounds();
    doc.layer_mut(layer).unwrap().surface_mut().unwrap().write_region(bounds, &[0.0, 1.0, 0.0, 1.0].repeat(64));
    let mut options = ExportOptions::default();
    options.encode.dds.mipmaps = "generate".into();
    let result = io::export(&doc, "out.dds", &options).unwrap().bytes;
    let (_, format, surfaces) = dds::inspect(&result).unwrap();
    for surface in surfaces.iter().filter(|s| s.item == 0) {
        let image = dds::decode_surface(&result, format, surface, &Default::default()).unwrap().to_rgba8();
        assert!(image[1] > 250 && image[0] < 4, "mip {}", surface.mip);
    }
    options.encode.dds.mipmaps = "none".into();
    let result = io::export(&doc, "out.dds", &options).unwrap().bytes;
    assert_eq!(dds::inspect(&result).unwrap().2.len(), 2);
}

#[test]
fn plain_image_export_composites_editing_layers() {
    let original = Image::from_u8(4, 4, ChannelLayout::Rgba, vec![255, 0, 0, 255].repeat(16)).unwrap();
    let bytes = photocraft_codecs::encode(&original, photocraft_codecs::Format::Dds, &Default::default()).unwrap();
    let mut doc = io::import("test.dds", &bytes).unwrap().document;
    let mut surface = Surface::new(doc.pixel_format());
    surface.write_region(doc.bounds(), &[0.0, 0.0, 1.0, 1.0].repeat(16));
    doc.layers.push(Layer::new("Paint", LayerContent::Raster(surface)));
    let result = io::export(&doc, "out.dds", &ExportOptions::default()).unwrap();
    assert_eq!(photocraft_codecs::decode(&result.bytes).unwrap().to_rgba8()[..4], [0, 0, 255, 255]);
}

#[test]
fn missing_faces_and_invalid_cube_shapes_fail_with_useful_errors() {
    let bytes = texture("cube", 6);
    let mut doc = io::import("cube.dds", &bytes).unwrap().document;
    doc.layers.pop();
    assert!(io::export(&doc, "out.dds", &ExportOptions::default()).is_err());
    let mut options = ExportOptions::default();
    options.encode.dds.structure = "cube".into();
    assert!(io::export(&doc, "out.dds", &options).is_err());
}

#[test]
fn high_depth_and_hdr_dds_documents_keep_their_samples() {
    for sample in [SampleType::U16, SampleType::F16, SampleType::F32] {
        let original = Image::from_normalized(4, 4, ChannelLayout::Rgba, sample, &[0.25, 0.5, 1.0, 0.75].repeat(16)).unwrap();
        let bytes = photocraft_codecs::encode(&original, photocraft_codecs::Format::Dds, &Default::default()).unwrap();
        let doc = io::import("test.dds", &bytes).unwrap().document;
        assert_eq!(io::export(&doc, "out.dds", &ExportOptions::default()).unwrap().bytes, bytes);
    }
}
