# GoldenEye 007 (Wii)

EDB version 263, big endian, GX meshes and textures. The game's folder is `bondx`; its files
come out of `Filelist.bin` with `eurochef-cli filelist extract`.

Everything below was checked in the running game (a static recompilation that loads loose
`.edb` files), not only against the reader.

## What the tools do

```
eurochef-cli ge new-map scene.glb --name yard --id 1 -o mods/
eurochef-cli ge triggers mt_test.edb
eurochef-cli ge edit-triggers mt_test.edb -o out.edb --move 15:-12,0.05,12 --copy 0:1,0,2 --remove 20
```

- `ge new-map` makes one level file (`mt_NAME.edb`, hash `0181 0200 + id`) from a glTF scene:
  triangles, textures (CMPR, with mip levels), collision and the player's spawn point.
- `ge triggers` lists the triggers of a file's map, `ge edit-triggers` moves, turns, copies,
  adds and removes them and writes the file again.
- The viewer: *New GoldenEye 007 map* in the menu bar does what `ge new-map` does and opens the
  result. In the Maps tab the *Entities* window moves the selected trigger, changes its values,
  duplicates and deletes it, adds new ones at the camera and saves the file.

### The glTF scene

- Units are metres. The game's levels are the mirror image of glTF: x is negated and the
  triangles are turned around on import.
- Vertex colours carry the light (the game draws levels unlit). Without `--no-light` they are
  shaded by a fixed light from above. A colour of 1.0 is stored as 0x80.
- A mesh or node named `collision...`, `col_...` or `ucx_...` isn't drawn, it is what a body
  collides with. With any of those in the scene the drawn triangles aren't collided with at
  all. Without: the drawn triangles are, except those of a material with `nocollide` in its
  name and those without area.
- A node named `spawn...` or `player_start...` is where the player starts. Without one: the
  floor in the middle of the scene.
- A node named `reference...` or `ref_...` is left out with everything below it: something to
  model against, like a figure the player's size (0.5 m wide, 1.7 m tall, the eye at 1.5 m
  standing and 0.9 m crouched): `player_size_reference.gltf` is one.
- Textures are resized to powers of two between 8 and 1024. Alpha is kept as CMPR's one bit.

## The file

| Offset | |
| --- | --- |
| 0x00 | `GEOM`, the file's hash, version 263, flags (0x20000000, 2: entities, 4: a map, 8: scripts) |
| 0x10 | build time, size, the size that stays loaded, 0, size again, memory needed |
| 0x40 | 16 lists: a count (short), the number of hashes that aren't local (short), a relative pointer. Sections, reference pointers, entities, anims, skins, scripts, maps, anim modes, anim sets, particles, swooshes, spreadsheets, fonts, force feedback, materials, textures |
| 0xC0 | three more arrays: a count (word) and a pointer |

A negative number of hashes means that many hashes (sorted, the entries that aren't local come
first) stand in front of the list. List entries are `hash, section (short), debug number
(short), address, 0`; entities have one more word, textures `width, height, game flags, flags
(0x04000000)`.

A section entry is `hash 0x0800000N, start, end, 0` and the section starts with `end,
0x9007, 0, 0`. Files whose size is larger than the size that stays loaded have sections that
are loaded on their own.

### Meshes (0x601)

A 0xC0 byte header: flags (+4, 0x100: collided with), area, a box and a sphere, then relative
pointers to the texture list (+0x54), strips (+0x58), positions (+0x5C), texture coordinates
(+0x60), colours (+0x64) and triangle flags (+0x68); a second sphere and box (+0x7C); the
number of strips (+0xA4), positions (+0xA8), flagged triangles (+0xAC) and at +0xB0 the
vertex format 0x203 with the texture coordinates' shift in the top 4 bits (they have `16 -
shift` fraction bits).

A strip is a 0x20 byte header (triangles, texture index, flags, transparency: shorts; the
size of what follows) and a GX display list padded to 32 bytes: a no-op byte, `0x98`, a
count, then for each vertex four 16 bit indices: position, normal, colour, texture
coordinate. Positions are three floats and the normal as three signed bytes (6 fraction
bits) and a byte 0x77. A short of flags for each triangle in strip order: 0x200 is not
collided with.

The word at +0x40 has a bit for each "variant" in its low byte and the offset from +0x40 to
a word for each in the top 24 bits; that word is the offset to the variant (a whole mesh)
and its number. A body asks for variant 1, a placed mesh that has it is collided with as the
variant and not as itself. The game makes an object of every variant when the file is
loaded, so two variants can't be the same mesh. In a group (0x603) the variants count for
nothing.

### Groups (0x603)

The base header, the number of meshes (+0x54), a pointer to the tree (+0x58), a pointer for
each mesh (+0x5C), then the tree's nodes, 0x30 bytes each: a corner, a count, the other
corner, a pointer, a sphere. The count is how many nodes the node's group has; negative: the
pointer is to a mesh, positive: to the first node of the group below.

A level's drawn triangles belong in one placed group. Meshes placed one by one are found by
the zone's lists (below).

### The map (0x500)

Pointers and arrays (paths, lights, sounds, portals, skies...), placements at +0x48 (0x3C
bytes each: hash, position, flags, rotation, scale, engine flags and zone as shorts, the
entity's hash, light set, group), the trigger header at +0x58, the bounds at +0x6C, the
number of zones at +0x84 and the zones from +0x88, **0x8C** bytes each.

A zone: the index of its entity in the reference pointers, a pointer to its settings (fog,
colours), four arrays, at +0x28 a pointer to its placements: a count, a pointer to their
indices (**shorts**) and a pointer to a tree. A node of that tree with children: a count
(byte), flags (byte), then for each child a pointer and a box (0x1C bytes). A leaf (count
0): flags, the number of entries (short), then for each a placement's index (short) and its
engine flags (byte). Flag 8 is on what a body is tested against.

### Triggers

`trigger header`: a count, pointers to the trigger headers (a pointer and a link, 8 bytes
each), to the script table, to the types (type, subtype, 8 unused bytes) and to the trigger
collisions. A trigger: the type's index and a debug number (shorts), game flags, a word with
a bit for each value that follows, position, rotation, scale, then 16 data values (bits 0 to
15), 8 links by trigger index (16 to 23) and 8 values for the engine (24 to 31).

Type 0x35 is a spawn point of a multiplayer game (`data 0` 0, `data 1` 0, `data 3` 1 in the
game's maps). The game traces from 0.3 above each one 1.3 down when a level loads, in single
player too, and stops in an endless loop on purpose when one hits nothing: they have to be
within a metre above the floor.

A level is its `mt_` file's hash. Type 0x1C is the player's spawn point, type 62 loads the
map of another file (`data 0`: the file's hash, `data 1`: 0x05000000), which comes in while
the level already runs: a large geometry file loaded that way isn't there yet when the
player spawns. Levels made here are one file.

`ge edit-triggers` writes the new triggers behind the end of the file and points the map at
them, so it only works on files that are loaded as a whole (all of the game's `mt_` level
files but `mt_archive*`).

### Textures

A 0x40 byte header (size, frames, the number of mip levels less one at +0x12, the format at
+0x13: 0 CMPR, 1 RGBA8, ...; the average colour; the size of the data) and behind it a TPL
file with one image: its pixels start 0x40 into the TPL, all mip levels down to one pixel on
the longer side.
