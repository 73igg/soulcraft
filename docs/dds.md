# DDS in PhotoCraft Custom

This custom build is based on SoulCraft's current PhotoCraft 0.8.2 source. It adds DDS export and expands DDS import from a single 2D BC1–BC3 image to complete colour-texture layouts.

## Open, edit and save

Open `.dds` through File → Open, drag and drop, or the command line. Every mip level, cube face, array entry and volume slice becomes an editable layer. The first surface is visible and selected initially; hide it and show another DDS layer to work on that surface. Layer names identify its face or array item, depth slice and mip level.

Use the ordinary brushes, selections, masks, filters and adjustments. **Save / Save As → DDS** retains the original header, texture shape, surface ordering and mip count. Untouched surfaces retain their original compressed bytes. Edits to additional painting or adjustment layers are composited into the first surface; other texture surfaces are exported from their corresponding DDS layers.

Keep the mapped DDS surface layers at the document's top level when preserving the original texture layout. Use **Texture → Image** to export an arbitrary grouped layer composition as one flattened image.

Save a working copy as `.pcraft` to keep the editing layers, the original DDS container and its surface mapping. Reopening the project restores the mapping even when layer identities are remapped.

After editing a base image, use **File → Export → Export DDS** and **Mipmaps → Generate full chain** to refresh its mip levels. **Keep existing** preserves edited or hand-authored mips. **No mipmaps** removes lower levels. After resizing an imported DDS, Generate or None rebuilds its layout; preserving incompatible sizes produces an error.

## Export controls

| Control | Choices and behaviour |
|---|---|
| Compression / storage | Automatic retains an imported DDS's storage. For a new image it chooses lossless storage matching the sample depth. Explicit formats include the table below. |
| Mipmaps | Keep existing, Generate full chain, No mipmaps. Generation filters sRGB colour in linear light and alpha with premultiplied colour. Volume mip levels halve width, height and depth. |
| Texture | Keep original layout; Image (visible composite); Texture array (layers); Cube map (six faces per cube); Volume (depth slices). |
| Colour space | Automatic/original, sRGB, Linear. This controls DDS header signalling. Texture channel values are retained; use Edit → Convert to Profile when a pixel colour-space conversion is wanted. Only DDS storage formats with an sRGB DXGI variant can carry that flag. |
| Compression quality | Fast, Normal, High for BC1-BC5 and BC7. BC6H uses a fixed HDR quality search, parallelized across block rows. |
| Scale | Export As's ordinary scale control. For original layouts, regenerate or remove mips when changing size. |

For a new array or volume, each top-level layer supplies one full-canvas texture or depth slice. Cube maps require square images and six layers per cube, ordered **+X, −X, +Y, −Y, +Z, −Z from top to bottom**. Twelve layers produce a two-cube array. Hidden layers are included in these explicit layer-based texture exports.

## Storage formats

| Family | Import and export |
|---|---|
| BC1–BC3 | BC1 / DXT1, BC2 / DXT3, BC3 / DXT5; DXT2 and DXT4 premultiplied alpha |
| Channel compression | BC4 / ATI1 and BC5 / ATI2, unsigned and signed. BC5 retains R and G rather than reconstructing the normal vector's third component. Signed samples remain signed floats in the document. |
| HDR compression | BC6H unsigned and signed, preserving HDR values and negative signed values |
| Modern RGBA compression | BC7 |
| Uncompressed colour | RGB8, RGBA8, BGRA8, RGB565, RGBA5551, RGBA4444, RGB10A2, 16-bit RGBA UNORM |
| Uncompressed channels | R8, RG8, A8, R16, RG16; legacy luminance and luminance/alpha import |
| Float storage | R/RG/RGBA half float; R/RG/RGB/RGBA 32-bit float; RGB 11:11:10 float and RGB shared exponent |
| Game conventions | RXGB import/export; DXT5 normal-map export. DXT5 normal maps have no distinct FourCC and open as their stored DXT5 channels unless the calling codec explicitly selects the normal-map interpretation. |
| Headers and shapes | Legacy FourCC/channel-mask headers and DX10; 2D images and mip chains, cube maps (including partial legacy cubes), cube arrays, texture arrays and volumes. Original 1D texture headers are preserved. |

The decoder also accepts the other normalized, packed colour and supported YUV layouts provided by image-rs DDS. Common typeless colour aliases use the decoder's corresponding colour view. DX10 premultiplied-alpha data is converted to straight pixels for editing and converted back on export; sRGB premultiplication works in linear light. Unknown formats, malformed headers, truncated subresources, unsupported sRGB combinations and missing required surface layers return errors. GPU depth/stencil, raw integer resource views, platform-specific tiled payloads and arbitrary buffers are not presented as editable colour images. DDS cannot retain PhotoCraft editing layers or ICC/EXIF/XMP metadata; `.pcraft` keeps the working document.

The low-level flat-image decoder returns the first surface. The document importer opens the complete texture, so editor and CLI workflows keep all surfaces. Arbitrary legacy masks without a named DXGI format retain the previous single-image compatibility decoder and export through a standard colour layout.

RGB8 is a legacy 24-bit DDS layout without a DXGI equivalent. Automatic may select it for a new 8-bit RGB image. PhotoCraft reads and writes it, but Microsoft's DirectXTex decoder rejects it. Choose RGBA8 or a BC format for DirectX tool compatibility and DX10 texture arrays.

## Input limits

The importer checks the complete texture and its tiled document allocation before creating layers. It permits at most 65,536 subresources. The codec's default allocation budget is 8 GiB on 64-bit targets and 1 GiB on 32-bit targets, with additional pixel and dimension limits. Excessive texture arrays, invalid headers and missing mip data fail with an error.

## CLI and engine

```powershell
photocraft-cli convert input.png output.dds --dds-format bc7 --dds-mipmaps generate --dds-color srgb
photocraft-cli convert original.dds preserved.dds
photocraft-cli run original.dds --cmd image.adjustments.invert --out edited.dds --dds-mipmaps generate
photocraft-cli run original.dds --cmd file.dds.inspect
```

`convert`, `run` and `batch` accept `--dds-format`, `--dds-mipmaps`, `--dds-texture`, `--dds-color` and `--dds-quality`. Engine command `file.export.dds` takes `path`, `format`, `mipmaps`, `texture`, `color` and `quality`; `file.dds.inspect` reports the original storage, shape and all surface-layer mappings. The normal automation file-authority policy applies to filesystem exports.

## Build and verification

```powershell
cargo build --release -p photocraft -p photocraft-cli --features photocraft/heif
cargo test -p photocraft-codecs --lib --tests
cargo test -p photocraft-io --test dds
```

The custom regression suite covers lossless 8/16/half/float data, all selectable storage formats, signed channels, BC6H HDR, sRGB/alpha mip filtering, non-multiple-of-four geometry, malformed files, full arrays/cubes/volumes, byte-exact saves, editing hidden faces, mip generation/removal and DDS preservation through `.pcraft`. The release is also checked through the native application and Microsoft's DirectXTex tools. Details and results are recorded in the [release verification report](dds-validation.md).

The Windows portable distribution has its own folder, executable identity and preferences. `portable.txt` stores settings in `PhotoCraftData` beside the executable; its platform fallback is `Photocraft Custom`, separate from the original installation.

DDS support uses [image-rs DDS 0.2.0](https://github.com/image-rs/image-dds) and the BC6H/BC3 implementations in [OxideAV DDS 0.0.6](https://github.com/OxideAV/oxideav-dds). Licence texts are included in `licenses/`.
