# Linked C3 objects

Open a texture from a client containing `c3` and `ini` directories. For a copied texture, select the client using **Client folder** in the right preview pane. A texture under `decomp/c3` can use the parent client's `ini` directory automatically.

SoulCraft reads the registry paths and the matching INI section's numbered mesh/texture pairs. It assembles sibling C3 files, plays their animation and attaches effects declared by the client catalogs. The INI texture binding takes precedence over an obsolete texture name embedded by an exporter. If a texture belongs to several records, choose the intended record in the **C3 Preview** tab's INI object selector.

## Inspect the object

The **C3 Preview** tab sits beside **Layers**, **Channels** and **Paths**. Open it through **Window > C3 Preview** if a saved workspace has hidden it.

- Use a piece's checkbox to show or hide that C3 file. **Solo** isolates it; **Show all** restores the file checkboxes.
- Expand a piece to control its individual mesh parts. These choices remain independent of the file checkbox.
- Use **Edit [texture]** under a piece to open or select its texture document.
- Missing or malformed files remain in the list with an explanation. Other available pieces still render.

The right 3D pane retains free rotation, zoom, pan, lighting, transparency, up-axis and manual mesh replacement. Wheel and Zoom slider cover 0.1×–1000×. Hiding pieces preserves framing and camera; **Fit all** frames the currently visible pose, while **Fit model** centers the base mesh without hiding effects. **Load mesh** replaces the preview with the selected standalone C3; **Refresh** in the object tab restores automatic lookup and rereads the client catalogs. **Effects** hides all effects while **Base object** controls the underlying object. **Pause / Play** freezes or resumes effect animation. See [C3 effects](c3-effects.md) for timing, rendering and individual attachment controls.

## Edit several textures

At the top of **Layers**, expand **Object textures**. Click a texture name to edit it. **Show all textures** opens available textures as separate documents and tiles their canvases. Clicking a tile makes its document active, so Layers, tools and undo follow that texture. Tile titles and the texture list mark unsaved changes with an asterisk.

The texture checkboxes control the tiled editing area; C3 file checkboxes control the 3D scene. Hiding a texture tile never closes it or removes its material from the model. **Single texture** restores the normal canvas for the active document. Unrelated open documents are excluded from this object's tiled area.

One document is reused for every piece sharing a texture. Brushes, layers, adjustments, masks and undo/redo update the linked material in the preview, including unsaved edits. Switching related textures keeps the camera and each document's DDS surface selection. Closing one texture leaves the scene available while its other texture documents remain open.

## DDS and TGA textures

The same linked-object workspace supports DDS and TGA materials, including objects using both formats. TGA can use uncompressed or RLE storage. A 32-bit TGA opens with its alpha in **Channels > Alpha 1**; the model preview uses that channel while RGB painting remains fully visible in the texture canvas. Edit the channel to change the model's opacity, and select **Cutout** or **Alpha** in the preview transparency control to inspect it. TGA export writes the single non-spot alpha channel at 32 bits; 24-bit export omits alpha. Multiple non-spot alpha channels follow the normal Targa export warning and are not combined.

An embedded `.tga` name can be an old authoring path. The game INI binding determines the actual game texture, which may be DDS. A missing authoring file is reported as missing; an existing DDS file is not renamed or converted just because the C3 contains a TGA name.

## Save

Use **Save active**, Ctrl+S, or normal Save/Save As for each edited texture. DDS settings, mip chains and surface mappings use the existing DDS workflow. Save a `.pcraft` copy when you need to retain editor layers. Packed resources open as documents requiring Save As to a loose file; the editor does not rewrite game archives or INI files. A saved copy remains bound to the same preview material during the session.

Each texture is saved separately. The tile arrangement and assembled scene are session view state; a `.pcraft` file stores that texture's document, rather than a multipart game-object project. Reopen a client texture to reconstruct its INI object.

## Coverage and bounds

Known catalogs include armor, weapons, mounts, hair, heads, armets, mantles, miscellaneous objects, simple/flying objects and effects. Numbered `MeshN`/`TextureN` and `EffectIdN`/`TextureIdN` pairs resolve through the corresponding registries; shared texture IDs are deduplicated. Client loose files take precedence over packaged resources. Ambiguous basename matches can require choosing a different INI object.

Declared `Part` and `Amount` counts bound the active numbered slots. Missing mesh or texture fields remain inspectable when another part identifies the object. Explicit INI texture paths use direct loose/archive lookup, with missing references reported promptly. UTF-8 and GBK catalogs are supported, including large modern effect tables; malformed catalogs produce notes while other available catalogs remain usable.

The view supports PHY1-5 motion, baked particles, trails, sockets and common client materials. PartEffect and three- through six-field Action3DEffect catalogs supply attachment choices. Camera/world state, the client's shared particle randomness, linked-emitter history and some environment/custom shaders remain accuracy boundaries; see the effect guide. Direct multipart records keep their original geometry transforms and do not acquire a character skeleton automatically.

Scenes are bounded to 128 C3 files, 512 internal mesh parts, two million vertices, six million indices, 128 distinct texture references and 512 MiB of decoded animation data. Each INI object has at most 64 numbered slots. Texture previews are reduced to at most 2048 pixels per side with a 64 MiB total decoded budget; retained encoded originals have a 128 MiB total budget. Full texture documents retain the original DDS layout. Opening linked textures together is limited to 64 million editable pixels across their inspected surfaces; larger files can be opened individually through File > Open. Budget failures and missing resources produce notes while other pieces remain usable.

Control commands: `preview.c3.object {id}`, `preview.c3.piece {index, visible|solo}` / `{all}`, `preview.c3.textures {index, visible?}` / `{all:true}` / `{single:true}`, and `preview.c3.playback` for playback, visibility and per-piece materials. `ui.inspect.c3Preview` reports object choices, effect groups, piece ranges, material bindings, playback, open documents and tile visibility.
