# SoulCraft - Windows x64 (DDS + C3)

One Windows application for DDS/TGA texture editing, linked C3 objects and animated effects. Download **soulcraft-windows-x64-portable.zip**, extract it into its own folder and run **photocraft.exe**. Keep the runtime DLL and license files beside the application. Portable preferences use `PhotoCraftData` beside the executable.

## Animated C3 effects and close-up camera controls — 2026-10-16

- Render C3 mesh animation, baked particles and trails with client catalog attachments, socket motion, texture stages and supported material/blend behavior.
- Show/hide all effects, the base mesh, effect categories, groups, files or individual parts. Pause/Play, Restart, Step, Time and Speed control animation. Open all linked textures for live editing and undo.
- Zoom from 0.1× to 1000× with the mouse wheel or Zoom slider. **Fit model** centers the mesh independently of large effects while keeping effects enabled; **Fit all** restores the full visible scene. Panning follows the current zoom.
- Correct exterior face culling, registered super-effect selection, garment/armor attachments and Vampire/Shadow Knight weapon mappings, including all matching numbered action channels.
- Pair explicitly opened extracted textures with their available extracted companion meshes and materials, avoiding mismatched UV layouts when newer packed files reuse the same resource ID.
- Preserve the existing DDS/TGA editor, linked-object and CLI workflows. The portable package retains the verified DDS CLI, runtime and licenses from the previous release.

The exact optimized Windows editor passed nine desktop cases: `2455030`, `480220`, `430040`, `440230`, `450030`, `001191500`, `001194600`, `410240` and `802010`. Also passed: 58 C3 tests, 22 focused preview tests, eight offscreen render scenes, C3/UI all-targets Clippy, formatting, layering and WASM including HEIF. See [verification](docs/c3-effects-validation.md) and [effects controls](docs/c3-effects.md).

This updates the combined **current** release; DDS and C3 remain one download. The editor retains upstream version text `0.8.2 (dev build)` because the verified executable was compiled before Git metadata was attached to the publishing checkout. Client assets are not included.

The separate S3 XML effect engine is not simulated. See the effects guide for supported formats, resource handling and remaining fidelity limits; this release does not claim universal pixel-identical client rendering.
