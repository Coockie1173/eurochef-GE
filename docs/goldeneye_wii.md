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
  triangles, textures (CMPR, with mip levels), collision, rooms and portals and the player's
  spawn point. When
  another file in the output folder has that hash already, the level gets the next free one.
- `ge triggers` lists the triggers of a file's map, `ge edit-triggers` moves, turns, copies,
  adds and removes them and writes the file again.
- The viewer: *New GoldenEye 007 map* in the menu bar does what `ge new-map` does and opens the
  result. In the Maps tab the *Entities* window moves the selected trigger, changes its values,
  duplicates and deletes it, adds new ones at the camera and saves the file.

### The glTF scene

- Units are metres. The game's levels are the mirror image of glTF: x is negated and the
  triangles are turned around on import.
- Vertex and material colours are linear in glTF and are made sRGB, which is what the game
  puts on the screen: a grey painted as 0.5 comes out as 0.5. `--linear-colours` keeps them as
  the file has them (darker).
- Triangles are drawn from their front only (the side they are counter clockwise from in
  glTF, where Blender's normal points). glTF's own "double sided" isn't gone by, Blender sets it
  on every material that hasn't Backface Culling ticked: a material with `twosided` or `nocull`
  in its name is drawn from both sides, `--gltf-double-sided` goes by glTF's.
- Vertex colours carry the light (the game draws levels unlit). Without `--no-light` they are
  shaded by a fixed light from above. A colour of 1.0 is stored as 0x80.
- A mesh or node named `collision...`, `col_...` or `ucx_...` isn't drawn, it is what a body
  collides with. With any of those in the scene the drawn triangles aren't collided with at
  all. Without: the drawn triangles are, except those of a material with `nocollide` in its
  name and those without area.
- A mesh or node named `collision_add...` or `col_add...` isn't drawn either, and is collided
  with on top of everything else that is: the drawn triangles keep their collision. A ramp over
  stairs is a `collision_add_ramp` and a material with `nocollide` in its name on the steps.
- A node named `spawn...` or `player_start...` is where the player starts. Without one: the
  floor in the middle of the scene.
- A node named `mp_spawn...` is a spawn point of a multiplayer game (`mp_spawn`,
  `mp_spawn.001`, `mp_spawn_roof`), as many as wanted. The player looks along the node's own +z
  (Blender's -y). It is put on the floor below it; one without a floor gets a warning, the game
  doesn't start a level with one. `mp_spawn_team0...` and `mp_spawn_team1...` are a team's.
- A node named `reference...` or `ref_...` is left out with everything below it: something to
  model against, like a figure the player's size (0.5 m wide, 1.7 m tall, the eye at 1.5 m
  standing and 0.9 m crouched): `player_size_reference.gltf` is one.
- A mesh or node named `sky...` is the sky, with everything below it: a dome or a box around the
  whole level, its faces turned inwards. The game draws it where it stands (it doesn't follow
  the camera) and it hides what is behind it like any wall, so it has to be larger than the
  level. Nothing collides with it, it is in no room and the light from above doesn't shade it.
- Textures are resized to powers of two between 8 and 1024. Alpha is kept as CMPR's one bit.

### Rooms and portals

Without them a level is one zone and the game draws all of it that is in front of the camera,
walls or not. With them it draws the room the camera is in and, through each portal that is on
the screen, the room behind it (and so on, cut to the portal's rectangle each time).

- A node named `RoomXX` is a room, with everything below it. `Room01`, `Room_01`,
  `Room01_walls` and Blender's `Room01.001` are all room 01, so a room can be several objects.
  XX is a number or a word.
- A node named `Portal_XX_YY` is the portal between rooms XX and YY: a flat quad that fills the
  opening (a door, a window, the end of a corridor). It isn't drawn. Which way it faces doesn't
  matter. More than one between the same rooms: `Portal_01_02.001`, `Portal_01_02_b`. A mesh
  that isn't four corners becomes the smallest rectangle around it.
- Triangles in no room's node (a doorway's frame, props) go to the room they lie in.
- A room is only ever seen through portals: one without any is drawn from inside only, and a
  hole between two rooms that has no portal shows nothing behind it. The importer warns about
  rooms without portals. A portal should be no smaller than the opening, larger does no harm.
- Rooms should be closed but for their portals. Which room a place belongs to is found by
  looking around from it (the walls seen in most directions are its room's), and the map's
  tree is made of planes that keep the rooms apart: the portals' own planes first, then the
  walls'. 255 rooms at most.
- The collision is always made of parts of its own then (the drawn triangles, or the
  `collision...` meshes), each listed by every zone it reaches into: a body is tested against
  what its own zone lists, also when it stands in a doorway. The file is larger for it.

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
size of what follows; flags 0x80: drawn from the front only, as nearly all strips of the game's
levels are, 0x40: from both sides, the test level's) and a GX display list padded to 32 bytes: a no-op byte, `0x98`, a
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

The tree at +4 says which zone a position is in: nodes of 0x20 bytes, a plane (four floats)
and two shorts. In front of the plane or on it (`n.p + d >= 0`) the first is next, behind it
the second; above 0 it is a node's index, otherwise a zone's number, negated.

The skies (+0x40: count and pointer) are hashes: an entity of the file (`82000000 + index`, a
0x601 mesh or a 0x603 group that isn't placed) or a script of entities (`84...`), as the
game's own levels have both. A zone's settings name one by its index at +0x38, -1: none; Dam
has seven. A new map has one, an entity behind the placed ones, and every zone names it.

A portal (+0x38: count and pointer) is 0x44 bytes: the two zones (shorts), flags (bit 0: not
looked through), 8 bytes more (a distance at +0xC from which it is looked through, 0: any; a
face at +0x10), and four corners from +0x14. A zone's links (+0x20) are 8 bytes each: the zone,
the zone behind (-1: another map's), the portal's index, the number of links to the same zone
that follow (a byte) and a byte that is 0 when `(c1 - c0) x (c2 - c0)` of the portal's corners
points into the zone behind, 1 when it points back: the game projects the corners and takes the
portal only when they turn the way that byte says, so it is seen through from one side by each
of its two links. The zone's 8 words at +0x48 have a bit for every zone that is never seen
from it (the walk doesn't go there whatever the portals say), +0x68 is its box.

A zone: the index of its entity in the reference pointers (each zone its own 0x608, which
names its 0x603 group's reference pointer at +0x58), a pointer to its settings (fog,
colours), four arrays, at +0x28 a pointer to its placements: a count, a pointer to their
indices (**shorts**) and a pointer to a tree. A node of that tree with children: a count
(byte), flags (byte), then for each child a pointer and a box (0x1C bytes). A leaf (count
0): flags, the number of entries (short), then for each a placement's index (short) and its
engine flags (byte). Flag 8 is on what a body is tested against. A level whose collision is hulls
mustn't have it on the drawn group: a body that is handed a large group nothing of which is
collided with is tested against nothing that comes after it in the tree (flag 1 there). The
game looks a body's zone up in the map's tree by the middle of its sphere (`fn_80315C10`), goes
down that zone's tree by the boxes (`fn_802EFA70`; at load the game makes each box larger by
its entities' own, `fn_802EFBE0`) and tests each entry's box again. The game's own trees are
three levels of nodes with up to four children, and so is a long list here.

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
