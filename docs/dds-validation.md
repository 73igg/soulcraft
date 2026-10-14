# PhotoCraft Custom DDS release verification

Release: **v0.8.2-dds.1**, Windows 10/11 x64 portable.

Completed: 2026-10-14 02:57 UTC.

Upgraded from the earlier 0.2.0 DDS work to SoulCraft main at `99e1138877e6aa7e856c6883b415e708d042fc59` (PhotoCraft 0.8.2).

Compiled code commit: `983faa455e19c045b19b1b9fecacfe65e48217c3`. Release-tag changes after this commit contain documentation and screenshots only.

Binary version: `photocraft-cli 0.8.2 (983faa455, 2026-10-14)`. Cargo compilation and test concurrency were capped at two jobs/threads.

## Automated checks

| Check | Result | Command |
|---|---|---|
| `dds-final` | PASS; 34 passed, 0 ignored | `cargo test -p photocraft-codecs -p photocraft-io --test dds --test dds_full -- --test-threads=2` |
| `editor` | PASS; 6,350 passed, 39 ignored | `cargo test -p photocraft-codecs -p photocraft-doc -p photocraft-format -p photocraft-io -p photocraft-engine -p photocraft-ui-egui -p photocraft-automation -p photocraft -p photocraft-cli -p photocraft-web --lib --tests -- --test-threads=2` |
| `clippy` | PASS | `cargo clippy -p photocraft-codecs -p photocraft-doc -p photocraft-format -p photocraft-io -p photocraft-engine -p photocraft-ui-egui -p photocraft-automation -p photocraft -p photocraft-cli -p photocraft-web --all-targets -- -D warnings` |
| `layers` | PASS | `cargo xtask layers` |
| `parity` | PASS | `cargo xtask parity` |
| `wasm` | PASS | `cargo xtask wasm` |
| `wasm-web` | PASS | `cargo check --target wasm32-unknown-unknown -p photocraft-web --features heif` |
| `panic-hunt` | PASS; 1 passed, 0 ignored | `cargo test -p photocraft-engine --test panic_hunt -- --ignored --test-threads=2` |
| `corpus` | PASS; 374 passed, 1 ignored | `cargo test --release -p photocraft-codecs -p photocraft-io --lib --test affinity_corpus --test annotations --test color --test comps_artboards --test corpus --test deep_exr --test heif --test malformed --test multichannel --test png_jpeg --test psd32_export --test psd_kerning --test psd_type_photoshop_bytes --test smart_corpus --test text_corpus --test tiff_corpus --test vector --features photocraft-codecs/corpus,photocraft-io/corpus,photocraft-codecs/heif,photocraft-io/heif -- --test-threads=2` |
| `perf` | PASS | `cargo xtask perf --quick --bench perf_scenarios` |
| `fmt` | PASS | `cargo fmt --all -- --check` |
| `scorecard` | PASS | `cargo xtask scorecard` |
| `release` | PASS | `cargo build --release -p photocraft -p photocraft-cli --features photocraft/heif` |

The editor gate runs library and integration tests for all ten changed packages (codecs, document, native format, I/O, engine, egui UI, automation, desktop, CLI and web). Wasm checks cover the shared crates, optional HEIF codec and the actual web application. Corpus checks use the repository's pinned, hash-verified fixtures and every codecs/I/O integration target enabled by the corpus feature, plus their library tests. The standard xtask wrapper also rebuilds unrelated synthetic tests already covered by the editor gate; the direct Cargo command above selects the actual corpus targets. Compilation uses the normal release profile. Ignored tests are the upstream opt-in or platform-dependent cases; the adversarial command suite is run explicitly.

To reproduce the full upstream corpus gate, run `cargo xtask test-corpus -p codecs -p io -- --test-threads=2`. This additionally reruns the synthetic tests covered by the editor gate.

The ten-package editor run validated code commit `3555acbf7293dfa176620ce2b161dc7516fcedd9`. The final source additionally corrects automatic sRGB detection for DXT2/DXT4. That small I/O change is checked by the dedicated DDS suite, the release I/O real-file corpus suite, Clippy, Wasm and the final native/release checks. Its regression exercises image, array, cube and volume layouts with partially transparent pixels.

The web-app Wasm check retains the upstream unused `PAR_PIXELS` constant warning in `display_color.rs`; the native strict Clippy gate is clean.

Fixture extraction used Git's GNU tar 1.35 through a local verification helper because Windows' built-in tar rejected an upstream Japanese filename. Every installed fixture was checked against the committed SHA-256 manifests; the helper and test corpora are not part of the distributed application.

## Built application and independent decoding

- All 37 selectable DDS storage settings exported through the final release CLI and reimported successfully.
- Microsoft DirectXTex `texconv` v2026.5.8.1 independently opened the standard supported exports. Legacy conventions unsupported by that tool are identified below and covered by Rust regression decoders.
- Arrays, cube maps, cube arrays and volume textures retained their full mip/subresource layouts. Unedited DDS saves and DDS → `.pcraft` → DDS saves were byte identical.
- BC3 endpoint/alpha pixels matched Microsoft decoding exactly; sRGB premultiplied-alpha storage matched an independently generated Microsoft fixture.
- The running native GPU application opened an 11-mip DDS, displayed its editable surface layers and DDS export controls, painted a red stroke, saved DDS, reopened it and verified the painted pixels and retained layout. Its `.pcraft` save/reopen retained byte-exact DDS data.
- README screenshots were captured from that running application using public-domain Hokusai artwork.
- Malformed/truncated mip data and excessive tiled texture-array allocation were rejected.

DirectXTex executable SHA-256: `dcfdec10244e02cf5037fba089c55fb7e1326b1c8181742d77d15fa5cb5eef06` (verified against the official release asset digest).

Microsoft decoder limitations in this run: `auto`, `rgb8`.

## 24 megapixel release measurement

Input: 6000 × 4000 public-domain image; 13 mip levels; two Rayon threads. Machine: Windows-10-10.0.26100-SP0, AMD Ryzen 7 3700X 8-Core Processor             , 32 GiB RAM; CPU load at start 32%. Other work was running on this machine, so timings are observations rather than performance guarantees.

| Operation | Seconds |
|---|---:|
| Prepare 24 MP public-domain image | 2.795 |
| BC7 fast export with full sRGB mip chain | 14.492 |
| Import and inspect 24 MP DDS | 0.401 |
| Byte-exact DDS save | 1.410 |

Output: 32,003,188 bytes. The preserved output was byte identical. Upstream has no DDS exporter, so there is no meaningful upstream DDS-export timing baseline.

## Distribution checks

The portable package contains the native GUI and CLI, `portable.txt`, the required Microsoft VC runtime, DDS documentation, screenshots and licence notices. Packaging verifies PE x64 architecture and GUI/console subsystems, ZIP integrity, an extracted CLI DDS round trip, DDS opening in the extracted native GUI and its local portable preferences directory. `SHA256SUMS.txt` accompanies the archive; uploaded asset hashes are checked against GitHub's SHA-256 digests.

The package is an unsigned independent custom build. Platform installers and signed builds are outside this Windows portable release.

## Supported scope

See [the DDS guide](dds.md) for formats and editing semantics. DDS colour textures are supported across BC1–BC7, HDR/signed channels, uncompressed integer/float storage and full mip/array/cube/volume layouts. GPU depth/stencil, arbitrary resource buffers, raw integer resource views and platform-specific tiled payloads are not editable colour images. Unknown arbitrary channel masks use the legacy first-image compatibility importer.

The source regression tests contain the exact precision, alpha, mipmap, structure, round-trip and allocation cases. No claim is made that every possible malformed or platform-specific DDS file has been tested.
