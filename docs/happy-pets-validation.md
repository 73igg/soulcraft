# HappyPets compatibility verification ? 2026-10-17

The combined optimized Windows editor was built with HEIF support, LTO disabled
and 16 codegen units. The packaged executable's SHA256 is recorded in
`BUILD-INFO.json` and `SHA256SUMS.txt`. The existing DDS CLI and runtime
are retained in the same portable package.

## Automated checks

- 62 C3 tests passed, including binary catalogs, duplicate-key handling, malformed
  input, field budgets, archive free blocks, extracted asset provenance, fully
  packed client inference and role/persistent-action bindings.
- 2,708 UI tests passed; 12 pre-existing tests remain ignored.
- C3/UI all-targets Clippy with warnings denied and dependency layering passed.
- The complete WebAssembly checks, including HEIF, passed. The shared workspace
  normalizer uses no native-only filesystem module.

## Supplied client survey

15,013 linked records and 1,653 enabled attachment bindings were identified.
Every enabled binding resolved to an effect definition. The survey decoded and
sampled 28,044 unique meshes at initial, middle and final animation frames.
It also checked 24,934 unique texture paths through native loose/packed lookup.

| Catalog | Objects |
|---|---:|
| 3deffect.ini | 7,957 |
| 3dsimpleobj.ini | 213 |
| 3dsimpleobjex.ini | 1 |
| armet.ini | 446 |
| armor.ini | 3,469 |
| head.ini | 38 |
| misc.ini | 81 |
| mount.ini | 368 |
| weapon.ini | 2,440 |

The original data contains 65 dangling registry parts, 18 absent mesh paths,
18 absent texture paths, five motion-only files and one malformed C3
(`c3/effect/role/wing_2005047/6.c3`). These are reported as unavailable parts;
other pieces remain usable. No unsupported renderable mesh layout was found
among the remaining surveyed files. The importer does not invent replacement
resource IDs or modify the client.

## Render and editing checks

The initial seven effect scenes passed offscreen: three HappyPets weapons, a HappyPets
garment, two Eudemons weapons and a Conquer/WDF weapon. Animation, static-glow
behavior, pause, effect visibility, camera fit and 10x/1000x zoom were checked.
The HappyPets live-edit case changed 3,046 viewport pixels during an uncommitted
brush stroke; undo restored the exact model preview and orbit did not edit the
document. Screenshots were visually inspected.

Fifteen cases passed against the actual optimized executable with isolated settings,
authenticated local control, screenshots and clean exit:

| Opened texture | Selected object | Expected effect |
|---|---|---|
| 510034 | weapon.ini:510034 | weapon510034 |
| 510214 | weapon.ini:510214 | Knife |
| 510274 | weapon.ini:510274 | weapon510274 |
| 510304 | weapon.ini:510304 | weapon510304 |
| 410180 | weapon.ini:410184 | weapon410184a |
| 420100 | weapon.ini:420104 | weapon420104a |
| 209980 | weapon.ini:209980 | weapon209980 |
| 511274 | weapon.ini:511274 | weapon511274 |
| 517094 | weapon.ini:517094 | weapon517094 |
| 209660 | weapon.ini:209660 | Base model |
| 1191000 | armor.ini:1191000 | mancoat+v_misc |
| 999118008 | misc.ini:999118008 | mancoat+v_misc |
| 801001 | mount.ini:1001 | Base model |
| 2455030 | weapon.ini:2455030 | 2455030+v_r_weapon |
| 410240 | weapon.ini:410244 | 410109+v_r_weapon |

The `999118008` accessory has a transparent one-pixel marker texture; its
visible geometry comes from the role effect. Its desktop zoom check retains
those effects, rather than trying to zoom an empty base-only view. The wing
render at 10x was inspected. As with other clients, preview notes identify
attachment sockets absent from a standalone base; a character skeleton and
actor-specific environmental state are not fabricated.

## Weapon proportions and adjacent texture recovery

The initial checks established catalog loading and visible animation, but did
not catch the proportions of three weapon effects. The WDB reader had mapped
an opaque per-part EFFE word to the INI `ScaleN` setting. Values of 2.96
(`510034`), 2.79 (`510214`) and 4.12 (`510274`) enlarged their effects separately
from the underlying weapons. That mapping is removed. Explicit loose INI scales
still apply. The `510034` base mesh and its primary overlay have matching bounds
before that erroneous transform. All 13 distinct meshes referenced by the `510`
family passed the revised scale audit.

`510304` contains three dangling catalog texture references: 8145, 8146 and
8147. Its existing adjacent `3.dds`, `4.dds` and `7.dds` are now recovered by
exact path/stem, respecting the opened extraction. Valid catalog bindings keep
priority, and preview notes identify the recovery. All ten scene textures
decoded successfully; the inspected render shows textured gold effects.

Six revised offscreen cases passed: `510034`, `510214`, `510274`, `510304`,
`410180` and Eudemons `410240`. Their animation changed 3,457, 6,768, 1,273,
4,353, 2,553 and 5,859 viewport pixels respectively. All retained exact paused
frames, effect hiding, document history, camera fit and close/max zoom behavior.
The four reported weapons' images were visually inspected. The scale and
adjacent-texture regression tests are included in the updated totals above.
The optimized executable also passed the fifteen desktop cases in the table,
including all four reported weapons with fully decoded textures. Side views
with and without effects were captured for those four weapons and inspected.

## Package verification

The portable ZIP is checked for CRC errors and every payload hash. Installed
EXE/ZIP hashes are compared with staging. The previous installed application
and ZIP are backed up under `happypets/previous-installed` in the analysis
folder. The CLI and runtime are checked against their original hashes. The
portable marker and launch instructions now name `soulcraft.exe`; the renamed
executable is rebuilt with matching Windows version metadata and checked again
against the reported weapons before publication. Current implementation source is in `.soulcraftanalysis/source`; the
older `.soulcraft/source/photocraft-0.2.0` folder remains an earlier snapshot.
