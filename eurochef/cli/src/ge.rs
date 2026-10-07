//! Commands for GoldenEye 007 (Wii): new maps from glTF scenes, and the triggers of a file.


use anyhow::Context;
use clap::Subcommand;
use eurochef_shared::ge::{
    geomap::build_geometry_file,
    gltf_import::{import_gltf, ImportOptions},
    level::geometry_hash,
    project::{new_map_from_gltf, NewMapOptions},
    triggers::{read_triggers, write_triggers, GeTrigger},
};

#[derive(Subcommand, Debug, Clone)]
pub enum GeCommand {
    /// Make a new level from a glTF scene: mt_NAME.edb, with its geometry, textures, collision
    /// and the player's spawn point
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

        /// Draw from both sides what the glTF materials say is double sided (Blender: every
        /// material without Backface Culling). Without it only materials named ...twosided or
        /// ...nocull are
        #[arg(long)]
        gltf_double_sided: bool,
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

        #[arg(long)]
        gltf_double_sided: bool,
    },
    /// List the triggers (entities) of a file's map
    Triggers {
        /// .edb file to read
        filename: String,
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
            gltf_double_sided,
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
                },
            };
            let mut map = new_map_from_gltf(&gltf, &options)?;
            let wanted = map.level_hash;
            let path = map.save(&output_folder)?;
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
            if map.stats.multiplayer_spawns > 0 {
                println!("{} multiplayer spawn points", map.stats.multiplayer_spawns);
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
            println!("{} (level {:08X}, {} bytes)", path.display(), map.level_hash, map.data.len());
            Ok(())
        }
        GeCommand::Geometry {
            gltf,
            output,
            hash,
            scale,
            no_light,
            linear_colours,
            gltf_double_sided,
        } => {
            let scene = import_gltf(
                &gltf,
                &ImportOptions {
                    scale,
                    bake_light: !no_light,
                    linear_colours,
                    gltf_double_sided,
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
        GeCommand::Triggers { filename } => {
            let data = std::fs::read(&filename).with_context(|| format!("couldn't read {filename}"))?;
            let set = read_triggers(&data)?;
            println!("{} trigger(s) in {filename}", set.triggers.len());
            for (i, t) in set.triggers.iter().enumerate() {
                println!("{}", describe(i, t));
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
