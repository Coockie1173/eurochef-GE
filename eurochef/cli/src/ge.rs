//! Commands for GoldenEye 007 (Wii): new maps from glTF scenes, and the triggers of a file.


use anyhow::Context;
use clap::Subcommand;
use eurochef_shared::ge::{
    edges::{edge_name, read_edges},
    geomap::{build_geometry_file, POLYGON_RUNG},
    gltf_import::{import_gltf, ImportOptions},
    level::geometry_hash,
    project::{new_map_from_gltf, NewMapOptions},
    sky::{self, SkyOptions},
    triggers::{read_triggers, write_triggers, GeTrigger},
    volumes::{is_bhop_preset, volume_of_name, BhopPreset, SceneVolume, TRIGGER_DAMAGE_ZONE, TRIGGER_DEATH_BARRIER, TRIGGER_LEVEL_CHANGE},
    weapon::{base_gun, build_weapon, locator_name, read_base, BASE_GUNS},
};

/// A made sky: a sphere around the level with the sky painted on it
#[derive(clap::Args, Debug, Clone)]
pub struct SkyArgs {
    /// Make a sky around the level: day, overcast, dusk, night or space (`ge sky --list`)
    #[arg(long, value_name = "PRESET")]
    sky: Option<String>,

    /// A picture of the whole sky unrolled (.png, .jpg or .tga, 2:1) in place of the painted one
    #[arg(long, value_name = "PANORAMA")]
    sky_texture: Option<String>,

    /// The colour straight up, #RRGGBB
    #[arg(long)]
    sky_zenith: Option<String>,

    /// The colour at the horizon
    #[arg(long)]
    sky_horizon: Option<String>,

    /// The colour below the horizon
    #[arg(long)]
    sky_ground: Option<String>,

    /// How soon the horizon gives way to the zenith: below 1 soon, above 1 late (0.6)
    #[arg(long)]
    sky_falloff: Option<f32>,

    /// How much of the sky the clouds take, 0 to 1 (0: none)
    #[arg(long)]
    sky_clouds: Option<f32>,

    #[arg(long)]
    sky_cloud_colour: Option<String>,

    /// Larger: smaller clouds (3)
    #[arg(long)]
    sky_cloud_scale: Option<f32>,

    /// How soft the clouds' edges are (0.18)
    #[arg(long)]
    sky_cloud_softness: Option<f32>,

    /// How much of the sky the clouds hide (0.9)
    #[arg(long)]
    sky_cloud_opacity: Option<f32>,

    /// Another number, other clouds
    #[arg(long)]
    sky_seed: Option<u64>,

    /// Of the sphere. Without it: the level's size times 2.5, 50 at least
    #[arg(long)]
    sky_radius: Option<f32>,

    /// Of the sphere, "x,y,z" in the game's coordinates. Without it: the level's middle
    #[arg(long, allow_hyphen_values = true)]
    sky_centre: Option<String>,

    /// Around the sphere, half of it down (64)
    #[arg(long)]
    sky_segments: Option<usize>,
}

impl SkyArgs {
    /// The sky that was asked for. None when nothing of it was: the scene's own stays
    fn options(&self, always: bool) -> anyhow::Result<Option<SkyOptions>> {
        let given = self.sky.is_some()
            || self.sky_texture.is_some()
            || self.sky_zenith.is_some()
            || self.sky_horizon.is_some()
            || self.sky_ground.is_some()
            || self.sky_clouds.is_some()
            || self.sky_cloud_colour.is_some()
            || self.sky_seed.is_some();
        if !given && !always {
            return Ok(None);
        }
        let mut o = SkyOptions::preset(self.sky.as_deref().unwrap_or("day"))?;
        let colour = |text: &Option<String>, into: &mut sky::Colour| -> anyhow::Result<()> {
            if let Some(text) = text {
                *into = sky::colour_of(text)?;
            }
            Ok(())
        };
        colour(&self.sky_zenith, &mut o.zenith)?;
        colour(&self.sky_horizon, &mut o.horizon)?;
        colour(&self.sky_ground, &mut o.ground)?;
        colour(&self.sky_cloud_colour, &mut o.cloud_colour)?;
        o.clouds = self.sky_clouds.unwrap_or(o.clouds).clamp(0.0, 1.0);
        o.falloff = self.sky_falloff.unwrap_or(o.falloff);
        o.cloud_scale = self.sky_cloud_scale.unwrap_or(o.cloud_scale);
        o.cloud_softness = self.sky_cloud_softness.unwrap_or(o.cloud_softness);
        o.cloud_opacity = self.sky_cloud_opacity.unwrap_or(o.cloud_opacity);
        o.seed = self.sky_seed.unwrap_or(o.seed);
        o.segments = self.sky_segments.unwrap_or(o.segments);
        o.radius = self.sky_radius;
        o.centre = self.sky_centre.as_deref().map(parse_vec3).transpose()?;
        o.texture = self.sky_texture.as_ref().map(std::path::PathBuf::from);
        Ok(Some(o))
    }
}

#[derive(Subcommand, Debug, Clone)]
pub enum GeCommand {
    /// Make a new level from a glTF scene: mg_NAME.edb with its geometry, textures and
    /// collision, mt_NAME.edb that loads it with the player's spawn point, and one more
    /// mt_NAME.edb in the folder of each online gamemode the scene has the spawn points and
    /// things of (conflict/, golden_gun/, ...)
    NewMap {
        /// .gltf or .glb file
        gltf: String,

        /// Name for the file
        #[arg(short, long, default_value = "custom")]
        name: String,

        /// Number of the new level, its file's hash follows from it (1: 01810201)
        #[arg(short, long, default_value_t = 1)]
        id: u32,

        /// Folder for the file (the game's mods folder)
        #[arg(short, long, default_value = "./")]
        output_folder: String,

        /// Where the player starts, "x,y,z" in the game's coordinates. Without it: the scene's
        /// node named spawn..., or the floor in the middle of the scene
        #[arg(long, allow_hyphen_values = true)]
        spawn: Option<String>,

        /// Game units for one unit of the scene
        #[arg(long, default_value_t = 1.0)]
        scale: f32,

        /// Don't shade the vertex colours by a light from above
        #[arg(long)]
        no_light: bool,

        /// Keep the vertex colours linear as glTF has them, not made sRGB (darker)
        #[arg(long)]
        linear_colours: bool,

        /// Every vertex colour times this: 1.5 is half again as bright
        #[arg(long, default_value_t = 1.0)]
        brightness: f32,

        /// How many smaller copies (mip levels) a lightmap's texture gets, 0: none. The game's
        /// own have 2
        #[arg(long, default_value_t = 2)]
        lightmap_mips: u32,

        /// Don't compress the lightmaps' textures: no blocks in soft light, eight times the size
        #[arg(long)]
        lightmap_uncompressed: bool,

        /// No glow around lamps (materials with emission)
        #[arg(long)]
        no_bloom: bool,

        /// Draw from both sides what the glTF materials say is double sided (Blender: every
        /// material without Backface Culling). Without it only materials named ...twosided or
        /// ...nocull are
        #[arg(long)]
        gltf_double_sided: bool,

        /// Who the players of an online game are in the level: a character's name as the game
        /// shows it, for the four of that character's set ("Jones": Jones, Davis, Smyth, Adams).
        /// Written to mt_NAME.txt, which the port reads
        #[arg(long, value_name = "CHARACTER")]
        team0: Option<String>,

        /// The other team's
        #[arg(long, value_name = "CHARACTER")]
        team1: Option<String>,

        /// The first team's hero in Heroes, a character's name ("Bond")
        #[arg(long, value_name = "CHARACTER")]
        hero0: Option<String>,

        /// The other team's hero
        #[arg(long, value_name = "CHARACTER")]
        hero1: Option<String>,

        /// Make the level one file, as it was before: no gamemodes, every multiplayer spawn
        /// point in it
        #[arg(long)]
        one_file: bool,

        #[command(flatten)]
        sky: SkyArgs,
    },
    /// Make only a geometry file from a glTF scene
    Geometry {
        /// .gltf or .glb file
        gltf: String,

        /// The .edb file to write
        #[arg(short, long)]
        output: String,

        /// The file's hashcode (0x018B....), by default the one new level 1 loads
        #[arg(long, value_parser = clap_num::maybe_hex::<u32>)]
        hash: Option<u32>,

        #[arg(long, default_value_t = 1.0)]
        scale: f32,

        #[arg(long)]
        no_light: bool,

        #[arg(long)]
        linear_colours: bool,

        #[arg(long, default_value_t = 1.0)]
        brightness: f32,

        #[arg(long, default_value_t = 2)]
        lightmap_mips: u32,

        /// Don't compress the lightmaps' textures: no blocks in soft light, eight times the size
        #[arg(long)]
        lightmap_uncompressed: bool,

        /// No glow around lamps (materials with emission)
        #[arg(long)]
        no_bloom: bool,

        #[arg(long)]
        gltf_double_sided: bool,
    },
    /// Try a made sky out before a level gets it: a picture of what a player sees of it, with
    /// the same --sky... options `new-map` takes
    Sky {
        /// List the presets
        #[arg(long)]
        list: bool,

        /// The .png to write
        #[arg(long, value_name = "PICTURE")]
        preview: Option<String>,

        /// Where the picture looks, "AROUND,UP" in degrees
        #[arg(long, default_value = "0,20", allow_hyphen_values = true)]
        look: String,

        #[command(flatten)]
        sky: SkyArgs,
    },
    /// List what a level file says can be vaulted over, climbed onto and climbed: its edges
    /// and ladders
    Edges {
        /// .edb file to read
        filename: String,
    },
    /// List the triggers (entities) of a file's map
    Triggers {
        /// .edb file to read
        filename: String,
    },
    /// Make a gun for a custom weapon of the port's mods/weapons: its parts from a glTF scene
    /// (nodes named Part_N, or Body for part 0, or a mesh rigged to bones Part_N), its scripts copied from the base gun's in
    /// o_guns.edb, so it moves as the base does
    Weapon {
        /// .gltf or .glb file with the gun (left out with --parts)
        gltf: Option<String>,

        /// The game's weapon it is made from, as the weapon's file names it (AK-47, P99, ...)
        #[arg(short, long)]
        base: String,

        /// The game's o_guns.edb
        #[arg(short, long)]
        from: String,

        /// The .edb file to write (into mods/weapons, next to the weapon's .txt)
        #[arg(short, long)]
        output: Option<String>,

        /// The file's hash, one no other file has
        #[arg(long, default_value = "0x0109F000", value_parser = clap_num::maybe_hex::<u32>)]
        hash: u32,

        /// Every vertex colour times this. The game's guns are near white, its levels half that
        #[arg(long, default_value_t = 1.75)]
        brightness: f32,

        /// List the base gun's parts and leave it at that
        #[arg(long)]
        parts: bool,

        /// Write the base gun as a glTF scene to start a gun from: an object for each part
        /// (Part_02_Magazine...) where the model script puts it, its points as empties under it,
        /// all ten times the game's size. Then leave it at that
        #[arg(long, value_name = "GLTF")]
        export: Option<String>,

        /// With --export: the scripts' keyframes as animations too, to look at (in Blender they
        /// hold the parts to their keys, so a part moved snaps back)
        #[arg(long)]
        animations: bool,

        /// With --export: leave out the parts only another gun's scripts show (`..._from_AK47`),
        /// the build makes them empty
        #[arg(long)]
        no_borrowed: bool,
    },
    /// Write the triggers of a file's map as a glTF scene of named nodes (spawn, mp_spawn_...,
    /// trigger_NNN_TYPE), the names `new-map` reads
    ExportTriggers {
        /// .edb file to read
        filename: String,

        /// The .gltf file to write
        #[arg(short, long)]
        output: String,
    },
    /// Change the triggers (entities) of a file's map and write the file again
    EditTriggers {
        /// .edb file to read
        filename: String,

        /// The .edb file to write
        #[arg(short, long)]
        output: String,

        /// Move a trigger: "INDEX:x,y,z"
        #[arg(long = "move", allow_hyphen_values = true)]
        moves: Vec<String>,

        /// Turn a trigger: "INDEX:x,y,z" (radians)
        #[arg(long, allow_hyphen_values = true)]
        rotate: Vec<String>,

        /// Set one of a trigger's 16 data values: "INDEX:SLOT=VALUE" (a number, 0x for hex, or
        /// a float with a decimal point; "none" clears it)
        #[arg(long)]
        set: Vec<String>,

        /// Add a copy of a trigger at another place: "INDEX:x,y,z"
        #[arg(long, allow_hyphen_values = true)]
        copy: Vec<String>,

        /// Add a new trigger: "TYPE:x,y,z". A spawn point (0x1C, or 0x35 for multiplayer) gets
        /// the values the game's own have, other types none
        #[arg(long, allow_hyphen_values = true)]
        add: Vec<String>,

        /// Add a trigger volume: "KIND:x,y,z:WIDTH,HEIGHT,DEPTH". KIND is what comes after
        /// volume_ in a scene: death, damage_25_0.5, finish, finish_0181013F, level_0181013F_2
        /// and sphere (death_sphere). WIDTH, HEIGHT and DEPTH are half the box's sides
        #[arg(long, allow_hyphen_values = true)]
        volume: Vec<String>,

        /// Add the level's own bunnyhop rules, what comes after bhop_ in a scene:
        /// speed6_gravity0.8_noprestrafe. Replaces a preset the file has
        #[arg(long)]
        bhop: Option<String>,

        /// Remove a trigger by index (as listed before any change), can be given several times
        #[arg(long)]
        remove: Vec<usize>,
    },
}

fn parse_vec3(s: &str) -> anyhow::Result<[f32; 3]> {
    let parts: Vec<&str> = s.split(',').map(|p| p.trim()).collect();
    anyhow::ensure!(parts.len() == 3, "\"{s}\" isn't x,y,z");
    Ok([parts[0].parse()?, parts[1].parse()?, parts[2].parse()?])
}

fn parse_indexed_vec3(s: &str) -> anyhow::Result<(u32, [f32; 3])> {
    let (index, vec) = s
        .split_once(':')
        .ok_or_else(|| anyhow::anyhow!("\"{s}\" isn't NUMBER:x,y,z"))?;
    Ok((parse_int::parse::<u32>(index.trim())?, parse_vec3(vec)?))
}

fn parse_value(s: &str) -> anyhow::Result<Option<u32>> {
    let s = s.trim();
    if s.eq_ignore_ascii_case("none") {
        return Ok(None);
    }
    if s.contains('.') && !s.starts_with("0x") {
        return Ok(Some(s.parse::<f32>()?.to_bits()));
    }
    Ok(Some(parse_int::parse::<u32>(s)?))
}

fn volume_type_name(type_id: u32) -> Option<&'static str> {
    match type_id {
        TRIGGER_DEATH_BARRIER => Some("death"),
        TRIGGER_DAMAGE_ZONE => Some("damage"),
        TRIGGER_LEVEL_CHANGE => Some("level change"),
        _ => None,
    }
}

fn describe(index: usize, t: &GeTrigger) -> String {
    let mut line = format!(
        "{index:3}  type {:3} (0x{:02x}){}  at {:9.3} {:9.3} {:9.3}  rot {:6.3} {:6.3} {:6.3}",
        t.type_id,
        t.type_id,
        if t.subtype != 0 {
            format!(" sub {}", t.subtype)
        } else {
            String::new()
        },
        t.position[0],
        t.position[1],
        t.position[2],
        t.rotation[0],
        t.rotation[1],
        t.rotation[2],
    );
    if is_bhop_preset(t) {
        line += "  bhop preset";
    } else if let Some(name) = volume_type_name(t.type_id) {
        line += &format!("  {name}");
    }
    if t.game_flags != 0 {
        line += &format!("  flags 0x{:x}", t.game_flags);
    }
    for (slot, v) in t.data.iter().enumerate() {
        if let Some(v) = v {
            line += &format!("  d{slot}=0x{v:x}");
        }
    }
    for (slot, v) in t.links.iter().enumerate() {
        if let Some(v) = v {
            line += &format!("  link{slot}={v}");
        }
    }
    for (slot, v) in t.engine.iter().enumerate() {
        if let Some(v) = v {
            line += &format!("  e{slot}=0x{v:x}");
        }
    }
    line
}

pub fn execute_command(cmd: GeCommand) -> anyhow::Result<()> {
    match cmd {
        GeCommand::NewMap {
            gltf,
            name,
            id,
            output_folder,
            spawn,
            scale,
            no_light,
            linear_colours,
            brightness,
            lightmap_mips,
            lightmap_uncompressed,
            no_bloom,
            gltf_double_sided,
            one_file,
            team0,
            team1,
            hero0,
            hero1,
            sky,
        } => {
            let options = NewMapOptions {
                name,
                id,
                spawn: spawn.as_deref().map(parse_vec3).transpose()?,
                import: ImportOptions {
                    scale,
                    bake_light: !no_light,
                    linear_colours,
                    gltf_double_sided,
                    brightness,
                    lightmap_mips,
                    lightmap_uncompressed,
                    bloom: !no_bloom,
                    parts: false,
                    part_spaces: vec![],
                },
                sky: sky.options(false)?,
                split: !one_file,
            };
            let mut map = new_map_from_gltf(&gltf, &options)?;
            let wanted = map.level_hash;
            let saved = map.save(&output_folder)?;
            if map.level_hash != wanted {
                println!(
                    "level {:08X} is another file's in {}: this one is {:08X}",
                    wanted, output_folder, map.level_hash
                );
            }
            println!(
                "{}: {} drawn triangles in {} meshes, {} collision triangles in {} meshes, {} textures",
                gltf,
                map.stats.drawn_triangles,
                map.stats.drawn_meshes,
                map.stats.collision_triangles,
                map.stats.collision_meshes,
                map.stats.textures
            );
            if map.stats.zones > 1 {
                println!("{} rooms, {} portals", map.stats.zones, map.stats.portals);
            }
            if map.stats.edges > 0 {
                println!("{} edges to vault over or climb", map.stats.edges);
            }
            if map.stats.ladders > 0 {
                println!("{} ladders", map.stats.ladders);
            }
            if map.stats.multiplayer_spawns > 0 {
                println!("{} multiplayer spawn points", map.stats.multiplayer_spawns);
            }
            if map.stats.volumes > 0 {
                println!("{} trigger volumes", map.stats.volumes);
            }
            if map.stats.sky_triangles > 0 {
                println!("a sky of {} triangles", map.stats.sky_triangles);
            }
            for warning in &map.stats.warnings {
                println!("warning: {warning}");
            }
            if let Some(b) = map.stats.bounds {
                println!(
                    "bounds {:.2} {:.2} {:.2} to {:.2} {:.2} {:.2}, the player starts at {:.2} {:.2} {:.2}",
                    b.min[0], b.min[1], b.min[2], b.max[0], b.max[1], b.max[2], map.spawn[0], map.spawn[1], map.spawn[2]
                );
            }
            let cast: String = [("team0", team0), ("team1", team1), ("hero0", hero0), ("hero1", hero1)]
                .into_iter()
                .filter_map(|(key, name)| name.map(|name| format!("{key} = {name}\n")))
                .collect();
            if !cast.is_empty() {
                let path = saved[0].path.with_extension("txt");
                std::fs::write(&path, &cast).with_context(|| format!("couldn't write {}", path.display()))?;
                println!("{} (who the players are online)", path.display());
            }
            for (mode, why) in &map.left_out {
                println!("no level for {}: {why}", mode.folder());
            }
            for file in &saved {
                let what = match file.mode {
                    Some(mode) => mode.folder(),
                    None if file.hash == map.level_hash => "level",
                    None => "geometry",
                };
                println!("{} ({what} {:08X}, {} bytes)", file.path.display(), file.hash, file.size);
            }
            Ok(())
        }
        GeCommand::Geometry {
            gltf,
            output,
            hash,
            scale,
            no_light,
            linear_colours,
            brightness,
            lightmap_mips,
            lightmap_uncompressed,
            no_bloom,
            gltf_double_sided,
        } => {
            let scene = import_gltf(
                &gltf,
                &ImportOptions {
                    scale,
                    bake_light: !no_light,
                    linear_colours,
                    gltf_double_sided,
                    brightness,
                    lightmap_mips,
                    lightmap_uncompressed,
                    bloom: !no_bloom,
                    parts: false,
                    part_spaces: vec![],
                },
            )?;
            let time = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs() as u32)
                .unwrap_or(0);
            let (data, stats) = build_geometry_file(&scene, hash.unwrap_or(geometry_hash(1)), time);
            std::fs::write(&output, &data).with_context(|| format!("couldn't write {output}"))?;
            println!("{output}: {} bytes, {stats:?}", data.len());
            Ok(())
        }
        GeCommand::Sky { list, preview, look, sky: args } => {
            if list {
                for (name, [zenith, horizon, ground, cloud], share) in sky::PRESETS {
                    println!("{name:9} zenith {zenith} horizon {horizon} ground {ground} clouds {cloud} over {share}");
                }
                return Ok(());
            }
            let Some(picture) = preview else {
                anyhow::bail!("name a picture with --preview, or --list the presets. A level gets a sky with `ge new-map --sky PRESET`");
            };
            let options = args.options(true)?.unwrap();
            let (around, up) = look.split_once(',').with_context(|| format!("\"{look}\" isn't AROUND,UP"))?;
            let image = sky::preview(&options, (960, 540), around.trim().parse()?, up.trim().parse()?)?;
            image.save(&picture).with_context(|| format!("couldn't write {picture}"))?;
            println!("{picture}: the sky as a player sees it");
            Ok(())
        }
        GeCommand::Edges { filename } => {
            let data = std::fs::read(&filename).with_context(|| format!("couldn't read {filename}"))?;
            let sets = read_edges(&data);
            if sets.is_empty() {
                println!("no edges in {filename}");
            }
            for set in &sets {
                println!(
                    "{} at {:#x}: {} edge(s), {} polygon(s)",
                    if set.zone { "a zone's entity" } else { "an entity" },
                    set.entity,
                    set.edges.len(),
                    set.polygons.len()
                );
                let p = |p: &[f32; 3]| format!("{:.2} {:.2} {:.2}", p[0], p[1], p[2]);
                for edge in &set.edges {
                    println!("  {:04x} {:<10} {} > {}", edge.flags, edge_name(edge.flags), p(&edge.from), p(&edge.to));
                }
                for ladder in set.ladders() {
                    println!(
                        "  {:04x} {:<10} {} > {}, {} piece(s)",
                        POLYGON_RUNG,
                        edge_name(POLYGON_RUNG),
                        p(&ladder.foot),
                        p(&ladder.top),
                        ladder.pieces
                    );
                }
                for polygon in set.polygons.iter().filter(|r| r.flags & POLYGON_RUNG == 0) {
                    println!("  {:04x} polygon of {} corners at {}", polygon.flags, polygon.corners.len(), p(&polygon.corners[0]));
                }
            }
            Ok(())
        }
        GeCommand::Triggers { filename } => {
            let data = std::fs::read(&filename).with_context(|| format!("couldn't read {filename}"))?;
            let set = read_triggers(&data)?;
            println!("{} trigger(s) in {filename}", set.triggers.len());
            for (i, t) in set.triggers.iter().enumerate() {
                println!("{}", describe(i, t));
            }
            Ok(())
        }
        GeCommand::Weapon { gltf, base, from, output, hash, brightness, parts, export, animations, no_borrowed } => {
            let Some((name, id, scripts)) = base_gun(&base) else {
                let names: Vec<&str> = BASE_GUNS.iter().map(|g| g.0).collect();
                anyhow::bail!("no gun {base}, the base is one of: {}", names.join(", "));
            };
            let data = std::fs::read(&from).with_context(|| format!("couldn't read {from}"))?;
            let (blocks, gun_parts) = read_base(&data, scripts)?;
            if let Some(export) = export {
                let text = eurochef_shared::ge::weapon_rig::export_rig(*id, &data, &blocks, &gun_parts, animations, !no_borrowed)?;
                std::fs::write(&export, text).with_context(|| format!("couldn't write {export}"))?;
                println!("{export}: {name}'s {} part(s), ten times the game's size (Gun_x10)", gun_parts.len());
                return Ok(());
            }
            if parts || gltf.is_none() {
                println!("{name} ({id:08X}): {} part(s) in {} script(s)", gun_parts.len(), blocks.len());
                let labels = eurochef_shared::ge::weapon_rig::part_labels(*id, &blocks, &gun_parts);
                for (part, label) in gun_parts.iter().zip(&labels) {
                    println!("  {:<34} entity {:3} ({:08X}) on {:08X} {}", label.name, part.entity, part.entity_hash, part.locator, locator_name(part.locator));
                    let points: Vec<String> = eurochef_shared::ge::weapon::point_list(&part.points)
                        .iter()
                        .map(|(hash, at, _)| format!("{hash:08X} at {:.3} {:.3} {:.3}", at[0], at[1], at[2]))
                        .collect();
                    if !points.is_empty() {
                        println!("      points: {}", points.join(", "));
                    }
                    //the far copy's (the body's is the whole gun: what a guard holds and fires from)
                    let far: Vec<String> = part.lod_points.as_deref().map(eurochef_shared::ge::weapon::point_list).unwrap_or_default()
                        .iter()
                        .map(|(hash, at, _)| format!("{hash:08X} at {:.3} {:.3} {:.3}", at[0], at[1], at[2]))
                        .collect();
                    if !far.is_empty() {
                        println!("      far points: {}", far.join(", "));
                    }
                    match label.from {
                        Some(from) => println!("      the {from}'s, only in its {}", label.shown_in.join(", ")),
                        None => println!("      in {}", label.shown_in.join(", ")),
                    }
                }
                return Ok(());
            }
            let gltf = gltf.unwrap();
            let output = output.context("--output: the .edb to write")?;
            let made = build_weapon(&gltf, &base, &data, brightness, hash)?;
            std::fs::write(&output, &made.file).with_context(|| format!("couldn't write {output}"))?;
            let given: Vec<String> = made.given.iter().map(|i| format!("Part_{i}")).collect();
            println!("{output}: {name}'s {} script(s), {} of its {} part(s) from the scene ({}), {} texture(s)", made.scripts, given.len(), made.parts, given.join(" "), made.textures);
            for (part, hash, by) in &made.moved {
                println!("  Part_{part}'s point {hash:08X} moved {:.4} {:.4} {:.4}", by[0], by[1], by[2]);
            }
            Ok(())
        }
        GeCommand::ExportTriggers { filename, output } => {
            let data = std::fs::read(&filename).with_context(|| format!("couldn't read {filename}"))?;
            let set = read_triggers(&data)?;
            let text = eurochef_shared::ge::gltf_export::export_triggers_gltf(&set.triggers, &|_| None)?;
            std::fs::write(&output, text).with_context(|| format!("couldn't write {output}"))?;
            println!("{output}: {} trigger(s)", set.triggers.len());
            Ok(())
        }
        GeCommand::EditTriggers {
            filename,
            output,
            moves,
            rotate,
            set,
            copy,
            add,
            volume,
            bhop,
            mut remove,
        } => {
            let data = std::fs::read(&filename).with_context(|| format!("couldn't read {filename}"))?;
            let mut triggers = read_triggers(&data)?;
            let count = triggers.triggers.len();
            let check = |i: u32| -> anyhow::Result<usize> {
                anyhow::ensure!((i as usize) < count, "there is no trigger {i}, the file has {count}");
                Ok(i as usize)
            };

            for m in &moves {
                let (i, pos) = parse_indexed_vec3(m)?;
                triggers.triggers[check(i)?].position = pos;
            }
            for r in &rotate {
                let (i, rot) = parse_indexed_vec3(r)?;
                triggers.triggers[check(i)?].rotation = rot;
            }
            for s in &set {
                let parsed = s.split_once(':').and_then(|(i, rest)| {
                    let (slot, value) = rest.split_once('=')?;
                    Some((i, slot, value))
                });
                let (i, slot, value) = parsed.ok_or_else(|| anyhow::anyhow!("\"{s}\" isn't INDEX:SLOT=VALUE"))?;
                let slot: usize = slot.trim().parse()?;
                anyhow::ensure!(slot < 16, "a trigger has data values 0 to 15");
                triggers.triggers[check(parse_int::parse::<u32>(i.trim())?)?].data[slot] = parse_value(value)?;
            }
            for c in &copy {
                let (i, pos) = parse_indexed_vec3(c)?;
                let mut trigger = triggers.triggers[check(i)?].clone();
                trigger.position = pos;
                triggers.triggers.push(trigger);
            }
            for a in &add {
                let (type_id, pos) = parse_indexed_vec3(a)?;
                triggers.triggers.push(GeTrigger::with_defaults(type_id, pos));
            }
            for v in &volume {
                let parsed: Vec<&str> = v.split(':').collect();
                let [kind, position, half] = parsed[..] else {
                    anyhow::bail!("\"{v}\" isn't KIND:x,y,z:WIDTH,HEIGHT,DEPTH");
                };
                let name = format!("volume_{kind}");
                let (kind, sphere) = volume_of_name(&name)?.context("a volume's kind")?;
                let half = parse_vec3(half)?;
                anyhow::ensure!(half.iter().all(|h| *h > 0.0) || sphere, "{name}: a volume can't be flat");
                let scene = SceneVolume {
                    name,
                    kind,
                    sphere,
                    position: parse_vec3(position)?,
                    half,
                    rotation: [0.0, -0.0, -0.0],
                };
                triggers.triggers.push(scene.trigger(false));
            }
            if let Some(words) = bhop {
                let mut preset = BhopPreset::default();
                preset.read_words("--bhop", &words)?;
                triggers.retain(|t| !is_bhop_preset(t));
                triggers.triggers.push(preset.trigger());
            }
            remove.sort_unstable();
            remove.dedup();
            for i in remove.into_iter().rev() {
                check(i as u32)?;
                triggers.remove(i);
            }

            let out = write_triggers(&data, &triggers)?;
            std::fs::write(&output, &out).with_context(|| format!("couldn't write {output}"))?;
            println!("{output}: {} trigger(s), {} bytes", triggers.triggers.len(), out.len());
            Ok(())
        }
    }
}
