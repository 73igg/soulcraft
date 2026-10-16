# C3 effects

## Using the preview

Open a weapon or other mapped object texture. SoulCraft resolves the exact client INI object, then its PartEffect and Action3DEffect attachments. Select **Client folder** for a texture copied outside a client; choose the INI object if several definitions share the image. Persistent effects start enabled. Alternative actions, handedness and body contexts remain separate selectable groups.

Standalone weapons use persistent bindings for ordinary character appearances when choosing their default shared-texture super variant. Transformation-specific appearance bindings remain available as disabled alternatives; their presence does not substitute another model's glow. Effect resource numbers can differ from the weapon number when the client explicitly maps them that way.

Body-specific weapon rows, including Vampire and Shadow Knight weapons, resolve their body prefix separately from their item/quality number and use that body's action effects. Numbered action channels play together within the selected actor context for weapons as well as outfits. When an EO image is shared by basic and super rows with persistent effects, the preview prefers the registered first super-quality row (ending in 4); the object chooser retains the other qualities. Other client quality schemes keep their available registered glow variants.

A registered alternate texture can still load its matching weapon object and effects even when the client normally shares another texture for that mesh. The opened image stays bound for live editing and object reselection. This follows exact client resource associations, without choosing nearby numeric IDs.

Garments and armor resolve the body prefix separately from the item/quality number. For example, `001191500` uses the body-1 action entries for item `191500`. Dyed variants use the body mesh named by their armor row while retaining their own item quality. Numbered armor action entries (`_0`, `_1`, etc.) play together within one actor context, so head, arms, legs and mantle attachments can all appear. Exact PartEffect overrides take priority over an undyed body fallback.

Attachments prefer the exact C3 socket name. If the catalog uses an unprefixed name such as `l_leg`, the preview can resolve the existing `v_l_leg` virtual track and follow its animated pose. A missing track remains explicit.

Opening a texture inside a client's `decomp` folder uses the corresponding extracted mesh and companion textures when present. This keeps older extracted UV layouts paired with their own images; some updated client packs reuse an ID for a completely different outfit. Missing extracted companions fall back to normal client resources. Opening an ordinary client resource retains native package priority. The active client INIs still supply the associations in either case.

- **Effects** hides attached effects, embedded particles and trails. **Base object** hides the underlying object. Both switches preserve the scene and texture documents.
- **Wheel** and the **Zoom** slider cover 0.1× to 1000× relative to the fitted view. Close zoom uses optical magnification after reaching the safe camera distance, so it does not move inside the fitted mesh. Shift-drag or middle-drag pans at the current zoom.
- **Fit model** centers the base mesh independently of large attachments and keeps effects enabled. **Fit all** (or double-clicking the viewport) resets rotation, zoom and pan around currently visible geometry. A file containing only effects falls back to its visible scene. Visibility changes preserve your framing until you fit again.
- **Pause / Play**, **Restart**, **Step**, **Time** and **Speed** control playback. Pausing freezes motion, particles, shader scrolling and trails while camera navigation and texture editing continue.
- **Animation and rendering** has mesh/particle/trail switches, client-material rendering, background color and a repeatable particle seed.
- The **C3 Preview** object tab retains per-file and per-mesh visibility, solo controls and edit bindings. Effect groups can be toggled independently.
- Each file's **Material and timing** controls expose Direct3D blend factors, depth testing/writes, scale, offset, culling, billboards, frame interval/offset, delay, cycle count/gap, play-once behavior, manual UV stepping, MixOpt and all 16 shader parameters. **Refresh** reloads client values. These are preview settings and do not rewrite C3 or INI files.
- **Object textures > Show all textures** opens the scene's declared primary and additional textures, including shader stages and disabled attachment alternatives. Each remains an independent document. Painting, adjustments and undo update every bound stage live. Save textures explicitly; packaged images require Save As.

Keep **Effects** off while working on the weapon alone. Per-piece switches can also isolate static glow geometry stored as ordinary PHY parts in a base file.

Face culling preserves the C3 exterior winding under every up-axis setting. **Material and timing > Cull** can override a piece to two-sided, clockwise or counterclockwise rendering; **Client flag** restores its original 2SID/default behavior.

## Implemented formats and behavior

- PHY, PHY2, PHY3, PHY4 and PHY5; RAW, KKEY, XKEY and ZKEY motion; component-wise client matrix interpolation and vertex skinning.
- Alpha, draw and atlas-cell keys; STEP and PHY5 STEP1/STEP2; secondary UV coordinates; C3EXP_COLOR vertex colors; 2SID and BILB/BIB2/BIB3/BIB4 flags.
- PTCL, PTCX and PTC3 baked particles, particle IDs/births, local/world attachments, alpha envelopes, camera-facing and velocity-facing quads, spin/stretch, rectangular atlas columns/crops and CCFL initial/extra rotation.
- SHAP/SMOT trails with five interpolation subdivisions per client frame, width preservation and native UV span. Per-file native order is trail, PTCL, PTCX, PTC3, PHY.
- Socket poses follow motion matrices without applying the exporter's vertex-init transform to a virtual socket. Particle birth poses use the parent's actual animation time.
- Shared multipart effect cycles use the longest part; individual IntervalN, FrameOffsetN, LoopOnceN, X/Y/ZSelfN and global timing/ManualUVStep are retained.
- Four GPU texture stages. Shader behaviors include modulate/add, alpha composition, signed add, screen/decal/lightmap, camera-normal reflection, skinned fluid, two/four-texture layers, dissolve/colored dissolve/alternate dissolve, hair cutout, star-sky, masked fluid, Fresnel and two/three-layer alpha combinations.
- Loose client files, version-selected C3Pack resources, bounded TPI/TPD resources with split zlib blocks and DawnPack 1000/1001 and WDF lookup. Active catalogs/packages take precedence by default; an explicitly opened extracted document selects its available extracted companions as described above. No client DLL is executed or embedded.

## Accuracy boundaries

Newer clients also use a separate S3 XML effect engine through `3deffects3.ini`. Those effect graphs are not C3 animation files and are not simulated by this C3 mesh/particle/trail preview. Missing C3 definitions remain explicit; a numerically similar effect is never substituted.

This is an evidence-backed standalone preview, not a verified claim that every client frame is pixel-identical. The client shares a stateful random generator across systems; the preview uses a repeatable seed. PTC3 linked-emitter relation/blur history is retained and reported but not fully reproduced. Trail history is reconstructed from integer animation frames, so frame-rate-dependent client history can differ. Environment/cubemap, scene-lighting and custom/new shaders without complete evidence fall back to the primary material with a scene warning. Particle aspect uses the part's assigned texture dimensions; an unmapped file with several different embedded particle textures may need per-piece inspection. Action trails that require a character swing or skeleton motion need that external animation; a stationary weapon cannot supply it. Camera/projection and game-scene placement are adjustable and are not a reproduction of the entire running game scene. Preview textures are limited to 2048 pixels per side and one GPU mip level; client DDS mip/LOD sampling is not reproduced by this preview path. Encrypted DawnPack 1002 archives require unpacked resources; their payload encryption is not implemented.
