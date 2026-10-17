# HappyPets client support

HappyPets uses binary resource catalogs in `ini/c3.wdb`. SoulCraft reads these
locally, without extracting or changing the client. Weapon, armor, head, armet,
miscellaneous, mount, simple-object and named-effect records use the same linked
object workspace as the other clients. DDS/TGA editing, per-piece visibility,
effect playback, particles, trails, painting and undo retain their normal flows.

Open a client texture, including one under `c3.extracted/c3` or `decomp/c3`.
SoulCraft locates the parent client catalogs. Companion meshes and textures from
the same extraction take precedence for that document; ordinary client resources
retain loose/packed priority. This keeps older extracted UVs paired with their
original textures. Copies outside a client can use **Client folder**.

The WDB reader handles the encrypted directory and RSDB, MESH, EFFE, SIMO and
SIMX tables. Duplicate catalog keys retain their first record, matching the
legacy dictionary format. Existing loose INI records remain overrides, while
absent records and catalogs come from WDB. Paths, table extents, counts and part
allocations are bounded. Invalid databases produce a note and do not disable
other readable catalogs.

EFFE's opaque per-part word is not interpreted as the INI `ScaleN` setting.
Weapon overlays already use the base mesh's coordinate units. This avoids
enlarging the effects of `510034`, `510214` and `510274` separately from their
weapons. Explicit loose INI scale settings still apply normally.

When an effect's catalog texture reference is broken, the preview can recover
an exact adjacent DDS/TGA with the same filename stem as its C3 mesh. A preview
note identifies that recovery; valid catalog references retain priority. This
recovers `510304`'s three dangling texture IDs using its existing `3.dds`,
`4.dds` and `7.dds`, without changing the client database. Obsolete exporter
paths no longer trigger an unrelated full-client texture search.

HappyPets' `9999` wildcard and persistent-action context is retained alongside
the other clients' `999`. Action-specific entries remain selectable separately.
`RoleWeapon3DEffect.ini` and the whitespace-delimited `RolePartEffect.ini` supply
additional action and role attachments, including explicit part references.
The actor-specific `Adjust` table is not applied to a standalone attachment;
the same character/skeleton and environment accuracy boundaries as the existing
preview still apply.

Patched TPI indexes can end with a declared free-block directory. These unused
block descriptors are bounded and skipped. The selected resources still use
their validated TPD extents, decompressed lengths and compressed byte counts.
An unpacked copy is not required to load ordinary packed resources.

Some original catalogs contain dangling registry IDs or paths. Those parts
remain visible in the object list with an explanation, as in other clients.
The importer follows actual resource bindings and does not guess replacement
IDs or change client data.

The reader is an independent Rust implementation checked against local binary
data. Public format references include CptSky's [WDB header and entry
definitions](https://gitlab.com/conquer-online/tools/wdb/-/blob/master/src_tree/include/wdb.h)
and the [DBC EFFE and MESH record layouts](https://github.com/conquer-online/co2_core_dll/tree/master/src/IO/DBC).
No implementation code, client DLLs, game textures or models are included.
