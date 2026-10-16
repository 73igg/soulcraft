# C3 effects and camera verification — 2026-10-16

The Windows editor was built with Cargo's optimized release profile and HEIF support. Builds used one BelowNormal job, LTO disabled and 16 codegen units, with a free-memory check before starting. No other project's compiler was stopped or reconfigured.

## Current checks

- 58 C3 tests passed: animation layouts and keyframes, hostile/truncated input, resource archives, materials, registry associations, body-specific item identity, numbered attachment channels and alternate texture bindings.
- 22 focused C3 preview tests passed: camera bounds, optical close zoom, pan scaling, legacy camera deserialization, atomic invalid commands, effect/playback controls, linked texture ownership and preserved document state.
- C3/UI all-targets Clippy with warnings denied, formatting, dependency layering and WASM checks including HEIF passed.
- Eight offscreen scenes passed: `001194600`, `2455030`, `480220`, `430040`, `440230`, `450030`, `001191500`, `410240`. Each rendered 10× and 1000× zoom, retained zero paused pixel changes and unchanged document history. Fitting the base model with effects enabled, then hiding effects, matched fitting the isolated base exactly.
- The exact optimized editor passed nine authenticated desktop/control tests: the eight cases above plus `802010`. Automatic object/effect selection, fit, close zoom, maximum zoom, reset, pause/hide/resume and clean exit passed. Garment and weapon close-ups were visually inspected. Interactive testing also confirmed the expanded zoom range.
- The earlier class-effect audits covered 10,053 weapon definitions/1,592 texture groups and 6,244 armor definitions/4,799 texture groups. They checked 751 body-encoded weapons, all 203 multipart weapon action records and 600 armor action records. These are association audits, not visual certification of every object.

Earlier full DDS/editor regression totals are historical and are not added to these counts. The existing DDS CLI is retained byte for byte from the previous release; its own version is `0.8.2 (983faa455, 2026-10-14)`.

## Package and source

The ZIP contains the editor, DDS CLI, runtime, licenses, instructions, these verification notes and per-file SHA256 sums. `BUILD-INFO.json` records the publishing source commit, editor hash, retained CLI provenance and the validation summary. The release's separate `SHA256SUMS.txt` also covers the whole ZIP. Client data, extracted textures/meshes, decompiled clients and local automation credentials are excluded.

The GUI version text remains `0.8.2 (dev build)` because compilation and native verification preceded the publishing checkout. Committed runtime sources are checked against the verified build's recorded source hashes, allowing only Git line-ending normalization.

## Scope

See [C3 effects](c3-effects.md) for format and rendering coverage. The separate S3 XML engine is not simulated. Unavailable catalog resources and unsupported packages remain explicit. Catalog/decoder coverage does not establish pixel-identical output for all clients or all animation frames. `.pcraft` retains each texture document, not an entire client scene.
