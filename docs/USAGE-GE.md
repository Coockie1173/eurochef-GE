# Eurochef for GoldenEye 007 (Wii)
This fork of [Eurochef](https://github.com/eurotools/eurochef) can make new levels for GoldenEye 007 on the Wii (the 2010 one, EDB version 263). Model something in Blender, export it as glTF, and eurochef will poop out an `mt_NAME.edb` which the game loads as a level.
It consists out of three parts:
- The importer
- The CLI
- The viewer

Along with a smaller fourth entry, the example scene.
This document is the "how do I use it" one. If you want to know what the bytes mean, that's [goldeneye_wii.md](goldeneye_wii.md).

# Quick overview
## The importer
Reads a glTF scene (`.gltf` or `.glb`) and turns it into a level: triangles, textures, collision, rooms and portals, a sky, spawn points, things to vault over and ladders. You don't talk to it directly, both the CLI and the viewer use it.
It has no settings file and no custom properties. Everything is decided by **what you name things**. This sounds dumb but it means it works from any modelling program that can name an object.

## The CLI
`eurochef-cli ge ...`. Makes levels, lists and edits triggers, lists edges and ladders.

## The viewer
The Eurochef window. Opens the level you just made, shows the triggers, lets you move them around and save the file again. It also draws the vault edges and ladders now, so you can see whether they ended up where you thought they would.

## The example scene
[usage-ge/example.gltf](usage-ge/example.gltf). One small file that uses every feature in this document: two rooms with a portal, a sky, stairs with a ramp over them, a wall to vault, a fence to vault further, a crate to climb, a ladder, spawn points and a reference figure. Every screenshot below is that scene. Open it in Blender next to this document, it'll make a lot more sense.

![Screenshot of the example level in the viewer](./usage-ge/overview.png)

# Making a level
Build the thing first (`cargo build --release`, you get `target/release/eurochef-cli` and `target/release/eurochef`). Then:
```
eurochef-cli ge new-map docs/usage-ge/example.gltf --name example --id 9 -o mods/
```
and it tells you what it made:
```
docs/usage-ge/example.gltf: 298 drawn triangles in 2 meshes, 242 collision triangles in 2 meshes, 4 textures
2 rooms, 1 portals
12 edges to vault over or climb
1 ladders
4 multiplayer spawn points
a sky of 10 triangles
bounds -20.00 0.00 -10.00 to 10.00 5.00 10.00, the player starts at -0.00 0.00 0.00
no level for conflict: no node named mp_spawn...
no level for golden_gun: no node named mp_spawn...
no level for black_box: no node named black_box...
no level for goldeneye: no node named goldeneye...
no level for license_to_kill: no node named mp_spawn...
mods/mt_example.edb (level 01810209, 1760 bytes)
mods/mg_example.edb (geometry 018B0109, 62368 bytes)
mods/team_conflict/mt_example.edb (team_conflict 01811049, 2048 bytes)
mods/heroes/mt_example.edb (heroes 0181104D, 2048 bytes)
mods/team_license_to_kill/mt_example.edb (team_license_to_kill 0181104F, 2048 bytes)
```
That's it. `-o` is the game's `mods` folder: the level shows up under Extras > Mods*, and online in every gamemode it got a file for (see Gamemodes).

The level is made the way the game's own multiplayer maps are: everything you modelled is in `mg_example.edb`, once, and each `mt_example.edb` is a tiny file of triggers (spawn points and such) that loads it. `--one-file` gives you the old single `mt_example.edb` with everything in it and no gamemodes.
Read that output, by the way. If it says `0 ladders` and you made a ladder, you named something wrong, and it's a lot faster to notice here than after walking into a wall for a minute.

<sub>*In the recompiled PC port, which is what loads loose `.edb` files. The level's number (`--id`) decides its hashes: 1 is `01810201`, 9 is `01810209`, its geometry `018B0100 + id` and its gamemodes' levels `01811000 + id * 8` and the seven after. Two levels with the same number? The second one gets the next free ones.</sub>

| Option | What it does |
|--------|--------------|
| `--name` | The files become `mt_NAME.edb` and `mg_NAME.edb` |
| `--id` | The level's number, its hash is `0181 0200 + id` |
| `-o` | The folder it goes in |
| `--spawn x,y,z` | Where the player starts, in the game's coordinates. Wins over the scene's `spawn` node |
| `--scale` | Game units for one unit of the scene. The game is in metres, so is glTF, so leave it |
| `--no-light` | Don't shade the vertex colours by a light from above |
| `--brightness N` | Every vertex colour times N. 1.5 is half again as bright, the game stops at about 2 for a white vertex |
| `--linear-colours` | Keep the colours as glTF has them (darker) |
| `--gltf-double-sided` | Believe glTF's "double sided" flag (see Colours and sides) |
| `--one-file` | One `mt_NAME.edb` with the geometry in it, no gamemodes |
| `--sky PRESET` | Make a sky around the level (see The sky) |

## The names
This is the whole interface. A mesh or a node (an object in Blender) whose name **starts with** one of these is that thing. Case doesn't matter, and Blender's `.001` at the end is fine.

| Name starts with | What it is | Drawn | Collided with |
|------------------|------------|-------|---------------|
| anything else | The level | yes | yes* |
| `collision`, `col_`, `ucx_` | Collision, in place of the drawn triangles | no | yes |
| `collision_add`, `col_add` | Collision on top of everything else | no | yes |
| `spawn`, `player_start` | Where the player starts | — | — |
| `mp_spawn` | A multiplayer spawn point | — | — |
| `mp_spawn_team0`, `mp_spawn_team1` | A team's spawn point | — | — |
| `golden_gun` | Where Golden Gun's gun lies | — | — |
| `goldeneye` | One of GoldenEye's consoles | — | — |
| `black_box` | Where Black Box's box lies | — | — |
| `RoomXX` | A room, with everything below it | yes | yes |
| `Portal_XX_YY` | The opening between rooms XX and YY | no | no |
| `sky` | The sky, with everything below it | yes | no |
| `vault` | An obstacle's top the player vaults over | no | no |
| `vault_long` | The same, but further | no | no |
| `climb` | An obstacle's top the player climbs onto | no | no |
| `ladder` | A ladder | no | no |
| `reference`, `ref_` | Something to model against, left out completely | no | no |

And two for materials, these go **anywhere in** the material's name:
| Material name has | What it does |
|-------------------|--------------|
| `nocollide` | Its triangles are drawn but walked through |
| `twosided`, `nocull` | Its triangles are drawn from behind as well |

<sub>*Unless the scene has a `collision...` mesh, then only that is collided with.</sub>

Notice how "starts with" is doing a lot of work in that table. I named the wooden rungs of the example's ladder `ladder_rungs` at first, the importer took them for a second ladder and told me off for it. They're called `rungs` now.

## Units and mirroring
Metres. The player is 0.5 wide and 1.7 tall, the eye is at 1.5 standing and 0.9 crouched. [player_size_reference.gltf](../player_size_reference.gltf) is a figure that size, put it in a node named `reference_player` and it stays out of the level (the example has one next to the spawn point, you'll only ever see it in Blender).
The game's levels are the mirror image of glTF: x gets negated on import. You don't have to care about this while modelling, the level looks the way you made it. You do have to care when you type coordinates (`--spawn`, moving triggers): **what is at x 5 in Blender is at x -5 in the game**. Everything the CLI prints is in the game's coordinates.

## Colours and sides
The game draws levels unlit, the light is in the vertex colours. So the importer shades them by a fixed light from above (floors bright, walls a bit darker), which is why the example doesn't look like a grey blob. A mesh whose vertex colours you painted yourself (anything that isn't all white) has its light already and is left alone, shading it again would only make it darker. `--no-light` turns the importer's light off for the rest too.
Colours get made sRGB, so a grey you painted as 0.5 comes out as 0.5. A white, unshaded vertex shows the texture exactly as it is. Blender lights your scene on top of that and the game doesn't, so when it still comes out darker than what you were looking at, `--brightness 1.5` (or the slider in the window) is the knob.
Triangles are drawn from their front only, where Blender's normal points. Blender marks every material without Backface Culling as "double sided" in glTF, which would make everything double sided, so that flag is ignored unless you pass `--gltf-double-sided`. Want one fence to be seen from both sides? Name its material `fence_twosided`.
Textures are resized to powers of two between 8 and 1024. Alpha is one bit: there or not there.

## Collision
By default what you see is what you walk on. That's fine for a box room and annoying for stairs: the body bumps up every single step.
The fix is a ramp nobody sees. In the example the steps have a material named `steps_nocollide` (drawn, walked through) and there's a quad over them named `collision_add_ramp` (not drawn, walked on):

![Screenshot of the stairs, the ramp over them isn't drawn](./usage-ge/stairs.png)

`collision_add...` comes **on top of** what's already collided with. `collision...` (without the add) **replaces** all of it: the moment one of those is in the scene, the drawn triangles aren't collided with at all and you're on the hook for the whole level's collision.

## Spawn points
- `spawn`: an empty where the player starts. Without one you start on the floor in the middle of the scene, which is usually inside something.
- `mp_spawn`, as many as you want (`mp_spawn.001`, `mp_spawn_roof`...). The player looks along the node's own +z, that's -y in Blender.
- `mp_spawn_team0...` and `mp_spawn_team1...` for team games.

## Gamemodes
One scene, every gamemode: put all of it in the same file and `ge new-map` sorts out which gamemode gets what. The geometry is shared, only the triggers differ.

| Gamemode (its folder) | Gets | Is made when the scene has |
|-----------------------|------|----------------------------|
| `conflict`, `license_to_kill` | `mp_spawn` | an `mp_spawn` |
| `golden_gun` | `mp_spawn`, `golden_gun` | an `mp_spawn` and a `golden_gun` |
| `team_conflict`, `heroes`, `team_license_to_kill` | `mp_spawn`, `mp_spawn_team0`, `mp_spawn_team1` | a spawn point for each team |
| `goldeneye` | the same, and `goldeneye` | a spawn point for each team and a `goldeneye` |
| `black_box` | the same, and `black_box` | a spawn point for each team and a `black_box` |

So the golden gun never ends up in Conflict, and a map without a `black_box` simply isn't in Black Box's list. The output says which gamemodes were left out and what they're missing.

- `golden_gun`, `goldeneye...` and `black_box` are empties, like the spawn points. They stay exactly where you put them (not dropped to the floor), the game draws its own gun, console and box there.
- The game's own maps have one gun, one box and five consoles. The consoles are numbered in the order of their names, so `goldeneye_1` to `goldeneye_5`.
- The team gamemodes get the plain `mp_spawn` points too, the game's own maps have both.
- A gamemode you had before and don't any more (you deleted the `golden_gun`) leaves its old file behind in the folder. You get a warning, delete it yourself.

`ge export-triggers` on one of the game's own (`mpt_dubai_gec.edb`...) gives you these same names, if you want to see where the game put its consoles.

## Who the players are
Online, the game dresses the two teams by the map: Facility is Red-one's lot against Pvt. Gurov's, with Bond and Ourumov as the heroes. A level of yours gets Archives' unless you say otherwise:
```
eurochef-cli ge new-map yard.glb --name yard --id 1 -o mods/ --team0 Jones --team1 Oddjob --hero0 Bond --hero1 Zukovsky
```
That writes `mods/mt_yard.txt` (a `team0 = Jones` line each, edit it by hand all you want), which the port reads for every gamemode of the map. A value is a name as the game shows it, or a hash (`team0 = 4D000031`). Split screen doesn't care about any of this, everyone picks their own there, and Classic Conflict always has its two sets of villains.

### Teams
A team is one of the game's sets of characters, handed out to the players in turn. Name any one member, or give the set's hash. These are the ones whose members have names:

| Set | Members | Where the game uses it online |
|-----|---------|-------------------------------|
| `4D000030` | Jones, Davis, Smyth, Adams | Railyard, Statue Park, Zukovsky's, Docks |
| `4D000031` | Red-one, Red-four, Red-five, Red-nine | Facility, Archives, Underground, Jungle, Annex |
| `4D00003F` | Agent Zero, Agent Two, Agent Four, Agent Six | Sevproto |
| `4D000032` | Two-Two, Two-Four, Two-Six, Two-Seven | Railyard, Statue Park (the other team) |
| `4D000033` | Five-One, Five-Two, Five-Three, Five-Four | Underground, Jungle (the other team) |
| `4D000035` | Pvt. Gurov, Pvt. Harkov, Pvt. Kozmin, Pvt. Shkadov | Facility, Archives (the other team) |
| `4D000040` | Sgt. Morozov, Sgt. Zotkin, Sgt. Udalov, Sgt. Shubkin | Sevproto, Annex (the other team) |
| `4D00002F` | Vincent, Slav, Boris, Alfonso | Zukovsky's, Docks (the other team) |
| `4D000041` | Oddjob, Jaws, Blofeld, Scaramanga | Classic Conflict |
| `4D000042` | Dr. No, Baron Samedi, Rosa Klebb, Red Grant | Classic Conflict |
| `4D000034` | Sgt. Glebov, Sgt. Baikov, Sgt. Chzov, Sgt. Drashev | not online |
| `4D000043` | Bond, Natalya | not as a team |
| `4D000044` | Trevelyan, Onatopp, Ourumov, Zukovsky, Mishkin | not as a team |
| `4D000045` | HazMat, Soldier, Pilot, Security, Sky Briggs | not as a team |

Only the first ten are what the game's own online maps use, the rest is untried as a team.

The game has 40 more sets, the campaign's people. Their members have placeholder names or none, so these go by hash only (how many are in each in brackets):

| Set | Members |
|-----|---------|
| `4D000005` | CAMO RUSSIAN1, CAMO RUSSIAN2 (twice), two without a name (5) |
| `4D000006` | CHAR #17, CHAR #18 |
| `4D000008` | CAMO RUSSIAN4, 5, 7, 8 |
| `4D000009` | CAMO RUSSIAN6, CAMO RUSSIAN9 |
| `4D00000A` | CHAR #20, #23, #25, #26 |
| `4D00000B` | SNOW RUSSIAN1, 2, 3 |
| `4D00000C` | CHAR #19 |
| `4D00002C` | CHAR #39, one without a name |
| `4D000039` | CHAR #32, two without a name |
| `4D00003A` | CAMO RUSSIAN9, CHAR #16 |
| `4D00003B` | CAMO RUSSIAN3, one without a name |

Without any names: `4D00000D` (4), `4D000011` (2), `4D000012` (3), `4D000013` (14), `4D000014` (4), `4D000015` (18), `4D000016` (11), `4D000017` (5), `4D000018` (8), `4D000019` (9), `4D00001A` (6), `4D00001B` (9), `4D00001C` (10), `4D00001D` (6), `4D00001E` (2), `4D00001F` (2), `4D000020` (2), `4D000021` (6), `4D000024` (3), `4D000025` (2), `4D000026` (1), `4D00002E` (4), `4D000036` (4), `4D000037` (2), `4D000038` (1), `4D00003C` (4), `4D00003D` (2), `4D00003E` (9).

A single character's hash works as a team too: then all four of that team are that one.

### Heroes
`hero0` and `hero1` are one character each, and only Heroes uses them. What the game's own maps have:

| Hero | Hash | Where |
|------|------|-------|
| Bond | `4D0001B9` | Facility, Archives, Underground, Annex |
| Bond | `4D0001C5` | Railyard, Statue Park |
| Bond | `4D0001CC` | Zukovsky's |
| Bond | `4D0001EA` | Docks, Jungle |
| Bond | `4D0001EC` | Sevproto |
| Trevelyan | `4D0001BA` | Railyard, Statue Park, Underground, Jungle (the other team) |
| Ourumov | `4D0002E7` | Facility, Archives, Sevproto, Annex (the other team) |
| Zukovsky | `4D0002E8` | Zukovsky's, Docks (the other team) |

There are six Bonds (the sixth is `4D000213`) and two Trevelyans (`4D0002EC`), a different outfit each; the name alone gives you the first one, `4D0001B9` and `4D0001BA`. For another, write the hash.

Any character can be written there. Every one the game has a name for:

| Name | Hash | | Name | Hash |
|------|------|-|------|------|
| Adams | `4D000200` | | Oddjob | `4D0001A6` |
| Agent Zero | `4D000279` | | Onatopp | `4D0002E6` |
| Agent Two | `4D00027A` | | Ourumov | `4D0002E7` |
| Agent Four | `4D00027B` | | Pilot | `4D000216` |
| Agent Six | `4D00027C` | | Pvt. Gurov | `4D0002AA` |
| Alfonso | `4D0002B1` | | Pvt. Harkov | `4D0002AB` |
| Baron Samedi | `4D00020D` | | Pvt. Kozmin | `4D0002AC` |
| Blofeld | `4D0001AA` | | Pvt. Shkadov | `4D0002AD` |
| Bond | `4D0001B9` and five more | | Red Grant | `4D000260` |
| Boris | `4D0002B0` | | Red-one | `4D0001F9` |
| Davis | `4D0001FE` | | Red-four | `4D0001FA` |
| Dr. No | `4D00020C` | | Red-five | `4D0001FB` |
| Five-One | `4D0002B2` | | Red-nine | `4D0001FC` |
| Five-Two | `4D0002B3` | | Rosa Klebb | `4D00020E` |
| Five-Three | `4D0002B4` | | Scaramanga | `4D0001AB` |
| Five-Four | `4D0002B5` | | Security | `4D0002E9` |
| HazMat | `4D0001D0` | | Sgt. Baikov | `4D0002A7` |
| Jaws | `4D0001A7` | | Sgt. Chzov | `4D0002A8` |
| Jones | `4D0001FD` | | Sgt. Drashev | `4D0002A9` |
| Mishkin | `4D0002EA` | | Sgt. Glebov | `4D0002A6` |
| Natalya | `4D0002EB` | | Sgt. Morozov | `4D0002A2` |
| Sky Briggs | `4D0002ED` | | Sgt. Shubkin | `4D0002A5` |
| Slav | `4D0002AF` | | Sgt. Udalov | `4D0002A4` |
| Smyth | `4D0001FF` | | Sgt. Zotkin | `4D0002A3` |
| Soldier | `4D0001D3` | | Trevelyan | `4D0001BA`, `4D0002EC` |
| Two-Two | `4D0001E0` | | Vincent | `4D0002AE` |
| Two-Four | `4D000280` | | Zukovsky | `4D0002E8` |
| Two-Six | `4D000281` | | Two-Seven | `4D000282` |

Only the eight in the first table are heroes in the game's own maps; anyone else as a hero is untried. The placeholder ones (`CAMO RUSSIAN1`, `CHAR #17`...) can be written by those names as well.

Multiplayer spawn points get put on the floor below them. One that hangs over nothing gets a warning, and you want to listen to that one: the game checks every one of them when the level loads, single player too, and stops in an endless loop **on purpose** when one has no floor within a metre below it.

## Rooms and portals
Without rooms the level is one zone and the game draws all of it that's in front of the camera, walls or not. With them it draws the room you're in, and through each portal on screen the room behind it.

- A node named `RoomXX` is a room, with everything below it. `Room01`, `Room_01`, `Room01_walls` and `Room01.001` are all room 01.
- A node named `Portal_XX_YY` is the opening between rooms XX and YY: a flat quad that fills the door. Which way it faces doesn't matter.
- Triangles in no room go to the room they lie in.

The example has `Room01` (the yard) and `Room02` (the hall) with `Portal_01_02` in the door between them. The viewer says `2 zones` in the top left:

![Screenshot of the yard, the hall behind the door is the second room](./usage-ge/portal.png)

A room is **only** ever seen through portals. A hole between two rooms without a portal shows nothing behind it, and a room without any portal is only drawn from inside. The importer warns about the second one. Make the portal as large as the opening or a bit larger, never smaller.
Rooms should be closed but for their portals, since which room a spot belongs to is found by looking at the walls around it. 255 rooms at most, which should do.

## The sky
A node named `sky` is the sky: a dome or a box around the whole level with its faces turned **inwards**. The game draws it where it stands (it doesn't follow the camera), so it has to be larger than the level. Nothing collides with it and the importer's light leaves it alone, give it its colours yourself. The example's is a box with vertex colours, light at the horizon and blue above.

### Or let eurochef make one
Modelling a sky dome by hand gets old fast, so `ge new-map` can make it for you: a sphere around the whole level with the sky painted into its vertex colours.
```
eurochef-cli ge new-map level.glb --name yard -o mods/ --sky dusk --sky-clouds 0.5
```
The sphere's middle and size come from the level, so there's nothing to place. If the scene has a `sky` node of its own, the made one takes its place (and it tells you).
Don't want to start the game to see what `dusk` looks like? There's a preview:
```
eurochef-cli ge sky --preview sky.png --sky dusk --sky-clouds 0.5
```
![The dusk preset with half the sky in clouds, as ge sky --preview draws it](./usage-ge/sky.png)

`ge sky --list` lists the presets: `day`, `overcast`, `dusk`, `night` and `space`. Everything else is optional and the same for `new-map` and `sky`:

| Option | What it does |
|--------|--------------|
| `--sky PRESET` | The colours and clouds to start from |
| `--sky-zenith`, `--sky-horizon`, `--sky-ground` | A colour of your own, `#RRGGBB`, the way it should look |
| `--sky-falloff` | How soon the horizon gives way to the zenith: below 1 soon, above 1 late (0.6) |
| `--sky-clouds` | How much of the sky the clouds take, 0 to 1 |
| `--sky-cloud-colour`, `--sky-cloud-scale`, `--sky-cloud-softness`, `--sky-cloud-opacity` | What they look like. A larger scale is smaller clouds |
| `--sky-seed` | Another number, other clouds |
| `--sky-texture pano.png` | A picture of the whole sky unrolled (2:1, `.png`, `.jpg` or `.tga`) in place of the painted one |
| `--sky-radius`, `--sky-centre x,y,z` | Where the sphere is, if you know better than the level's bounds |
| `--sky-segments` | Around the sphere (64). More is smoother clouds and more triangles |

`ge sky --preview` also takes `--look AROUND,UP` in degrees, to look somewhere else than slightly up.
The clouds are painted per vertex, so they're soft blobs and not crisp. For crisp, use `--sky-texture`.

## Vaults and climbs
What the player can vault over isn't the collision. It's a list of edges, and you make them by putting a quad on top of the obstacle:

| Name | What the player does | Ends up |
|------|----------------------|---------|
| `vault...` | Vaults over | About 1.25 m behind the edge |
| `vault_long...` | Vaults over, further | About 1.6 m behind the edge |
| `climb...` | Climbs up | Standing on top |

The quad (or a box, only the faces that look up count) ends where the obstacle's top ends. Its rim becomes the edges, each one taken from outside. Not drawn, not collided with, it's only a marker.

![Screenshot of the vault edges in the viewer](./usage-ge/edges.png)

Green is the wall to vault, teal the fence to vault further, yellow the crate to climb. Things to know:
- The top has to be **0.5 to 1.5 m above the floor** in front of it (the game's own are all 1.1). The importer warns about one that isn't.
- The way is always as long as the table says, whatever is there. A `vault` over a wall 0.4 thick lands behind it. Over one 1.3 thick it ends on top of it. Pick the one that fits the obstacle.
- In game it's the action button (b on the classic controller) with the stick forward, facing the edge.

## Ladders
The new one. A ladder is a mesh named `ladder...`: **one flat upright quad** that
- looks at the player on it (its front, where Blender's normal points, is the side you climb),
- is as wide as the ladder (the game's own are 0.4 to 0.5),
- goes from the floor up to where the player gets off, so the top of the platform.

That's the whole thing. It may lean. It isn't drawn and isn't collided with, so model the actual ladder as a normal mesh next to it (and don't name that one `ladder`, see above).

![Screenshot of the example's ladder, the pink lines are what the game climbs](./usage-ge/ladder.png)

The pink lines are what the importer made of it. The game wants a ladder as a column of pieces half a metre tall plus an edge at the top where you get off, so the quad gets cut up:
```
$ eurochef-cli ge edges mods/mt_example.edb
a zone's entity at 0xc920: 13 edge(s), 5 polygon(s)
  ...
  0100 ladder top 0.25 3.00 6.00 > -0.25 3.00 6.00
  1000 ladder     0.00 0.50 6.00 > 0.00 3.00 6.00, 5 piece(s)
```
Notice how the ladder starts at 0.50 while the quad started at the floor. That's on purpose. A ladder whose pieces go all the way down works fine going up, and going down the player reaches the bottom and **never gets off again**. Stuck on the ladder forever. The game's own ladders all end half a metre above the floor, so the importer does the same with yours and you can just draw the quad from the floor.

How it plays:
| You | The player |
|-----|------------|
| Walk into the foot of it | Gets on, climbs with the stick, gets off onto the platform at the top |
| Back off the platform's edge above it | Grabs it, climbs down, steps off at the bottom |

And what the importer says when it doesn't like one:
| Warning | Meaning |
|---------|---------|
| `has no face that looks sideways, it is no ladder` | It's lying flat, or it's a closed box (its faces cancel out). Make it a single quad |
| `looks into the wall behind it ... turn it around` | The quad faces the wall instead of the climber. Flip its normal |
| `has no floor in front of its foot` | Nothing to stand on in front of it |
| `starts ... above the floor ... only got onto from its top` | The foot is too high to walk onto. Fine if that's what you want |
| `too low for a ladder: left out` | Less than a metre tall. Use a `climb` |

<sub>Ladders in a level with rooms just work, the importer gives each one to the rooms the player is in while on it.</sub>

# Looking at what you made
## In the terminal
```
eurochef-cli ge edges mt_example.edb
eurochef-cli ge triggers mt_example.edb
```
`ge edges` lists every edge with its flags and every ladder with its foot, its top and how many pieces it has. It works on the game's own levels too, which is how I found out what a ladder should look like (Carrier has three).
`ge triggers` lists the triggers, more on those below.

## In the viewer
Open the `.edb` (drag it in, or `eurochef mt_example.edb`) and go to the Maps tab. **Show Edges** in the toolbar draws the edges and ladders over the level:

| Colour | What |
|--------|------|
| Green | `vault` |
| Teal | `vault_long` |
| Yellow | `climb` |
| Pink | A ladder: its pieces and the edge at its top |
| Grey | Edges of the game's own levels that aren't any of these |

An edge you expected isn't there? Either the name is off, or the quad is upside down: only faces that look **up** make edges, and the importer says so (`has no face that looks up`).

# Triggers
Triggers are the game's "entities": spawn points, enemies, doors, the things that load other files. A new level has one for the player's spawn and one for each multiplayer spawn point.

## From the terminal
```
$ eurochef-cli ge triggers mods/mt_example.edb
5 trigger(s) in mods/mt_example.edb
  0  type  28 (0x1c)  at    -0.000     0.000     0.000  rot  0.000 -0.000 -0.000  ...
  1  type  53 (0x35)  at     8.000     0.020     8.000  rot  0.000 -2.356 -0.000  d0=0x1  d1=0x0  d3=0x1
  2  type  53 (0x35)  at    -8.000     0.020    -8.000  rot  0.000  0.785 -0.000  d0=0x1  d1=0x0  d3=0x1
  ...
```
`0x1C` is the player's spawn point, `0x35` a multiplayer one. To change them:
```
eurochef-cli ge edit-triggers mt_example.edb -o out.edb --move 1:6,0.02,6 --add 0x35:0,0.02,-8 --remove 4
```
| Option | Format | What it does |
|--------|--------|--------------|
| `--move` | `INDEX:x,y,z` | Moves a trigger |
| `--rotate` | `INDEX:x,y,z` | Turns it (radians) |
| `--set` | `INDEX:SLOT=VALUE` | Sets one of its 16 data values. `0x` for hex, a decimal point for a float, `none` clears it |
| `--copy` | `INDEX:x,y,z` | Adds a copy of it somewhere else |
| `--add` | `TYPE:x,y,z` | Adds a new one. Spawn points get the values the game's own have |
| `--remove` | `INDEX` | Removes one |

All of them can be given more than once. The indices are the ones `ge triggers` listed **before** any change, so you don't have to do sums when you remove two.
This works on the game's own level files as well (all the `mt_` ones but `mt_archive*`).

## Back into Blender
```
eurochef-cli ge export-triggers mt_example.edb -o triggers.gltf
```
writes the triggers as a glTF scene of empties, named the way `ge new-map` reads them: `spawn`, `mp_spawn_NNN`, `mp_spawn_teamT_NNN`, and `trigger_NNN_TYPE` for everything else (which `ge new-map` leaves alone). Import that into your scene and the spawn points are where the level has them.

## In the viewer
In the Maps tab, click a trigger and the **Entities** window lets you move it, turn it, change its values, duplicate it or delete it. *Add at the camera* adds a new one where you're looking from. *Save EDB as...* writes the file, *Export triggers as glTF...* does what `ge export-triggers` does.

![Screenshot of the trigger editor with a multiplayer spawn point selected](./usage-ge/editor.png)

# Making a level without the terminal
*New GoldenEye 007 map* in the menu bar does what `ge new-map` does, gamemodes and all, and opens the geometry file right away. Scene, folder, name, level number, a sky if you want one, done.

![Screenshot of the new map dialog](./usage-ge/newmap.png)

Pick a preset under *Sky* and the **Sky settings** open up. This is the one place where the window beats the terminal:
- a picture of the sky as a player sees it, drag it to look around,
- the four colours (zenith, horizon, ground, clouds), each a colour picker,
- sliders for the horizon's falloff and the clouds' cover, size, softness and opacity,
- the seed, with an *Other clouds* button for when you don't care which number,
- *Detail* (the sphere's segments), a panorama to use instead of the painted sky, and the sphere's radius if the level's own size isn't what you want.

Picking another preset only swaps the colours and the cloud cover, the rest stays as you set it. Under the picture is the same sky as `ge new-map` options, with a *Copy* button, so once you like it you can put it in a script.

It still has less options than the CLI for everything that isn't the sky (no `--spawn`, no `--linear-colours`). For those, well, there's a terminal.

# Things that will bite you
In no particular order, all of these got me at least once:
- **Names are prefixes.** `ladder_rungs` is a ladder. `skylight` is a sky. `collision_test_dont_use` is very much used.
- **x is mirrored** in everything you type and everything the CLI prints.
- **A multiplayer spawn point over nothing hangs the game** while loading. Read the warnings.
- **The triggers aren't in the file with the level in it.** `mg_NAME.edb` has no triggers and the `mt_NAME.edb` files have nothing to look at, so in the viewer you edit a gamemode's triggers without the walls around them. Easier to move the empties in Blender and run `ge new-map` again.
- **A `collision...` mesh turns off all other collision.** You wanted `collision_add...`.
- **A room without a portal is invisible from outside.** That's not a bug, that's what a room is.
- **The ladder quad faces the climber**, not the wall. The pink lines get drawn either way, so read the importer's warnings: a ladder that looks into its own wall can't be climbed.
- **Vault heights.** Under 0.5 or over 1.5 above the floor and the game ignores the edge.
