//! A new map from a glTF scene, ready for the game: one level file with geometry, collision,
//! textures and the spawn points, or split as the game's own multiplayer maps are, the geometry
//! in a file of its own that a small level of triggers loads, one for each gamemode.

use std::path::{Path, PathBuf};

use anyhow::Context;

use super::{
    geomap::{build_geometry_file, BuildStats, GeScene, ModeItemKind, EDGE_HEIGHTS},
    gltf_import::{import_gltf, ImportOptions},
    level::{
        black_box_trigger, console_trigger, geometry_hash, golden_gun_trigger, level_hash, make_level_of_geometry,
        make_trigger_level, mode_level_hash, multiplayer_spawn_trigger, GameMode,
    },
    sky::{put_sky, SkyOptions},
    triggers::GeTrigger,
};

/// How far below a floor a multiplayer spawn point may be modelled and still be put on it
const MULTIPLAYER_SPAWN_REACH: f32 = 0.3;
/// How many consoles the game's own GoldenEye maps have
const CONSOLES: usize = 5;

#[derive(Clone, Debug)]
pub struct NewMapOptions {
    /// Goes into the file names: mt_NAME.edb
    pub name: String,
    /// Which of the new levels this is, its files' hashes follow from it
    pub id: u32,
    /// Where the player starts. None: the scene's own spawn point, or its floor in the middle
    pub spawn: Option<[f32; 3]>,
    pub import: ImportOptions,
    /// A made sky around the level, in place of the scene's own. None: the scene's, if it has one
    pub sky: Option<SkyOptions>,
    /// The geometry in a file of its own (mg_NAME.edb) and a level of triggers that loads it for
    /// the folder and for each gamemode the scene has the things of. Without it: one file
    pub split: bool,
}

impl Default for NewMapOptions {
    fn default() -> Self {
        Self {
            name: "custom".to_string(),
            id: 1,
            spawn: None,
            import: ImportOptions::default(),
            sky: None,
            split: false,
        }
    }
}

pub struct NewMap {
    /// The level in the folder itself, the one Extras > Mods and GE_LEVEL start
    pub level_hash: u32,
    pub spawn: [f32; 3],
    pub stats: BuildStats,
    /// The gamemodes a split map has a level for
    pub modes: Vec<GameMode>,
    /// The gamemodes it has none for, and what the scene would need for each
    pub left_out: Vec<(GameMode, String)>,
    name: String,
    id: u32,
    time: u32,
    split: bool,
    /// The scene's file
    geometry: Vec<u8>,
    /// One file: the triggers it gets besides the player's spawn point
    more: Vec<GeTrigger>,
    /// Split: each gamemode's triggers
    mode_triggers: Vec<(GameMode, Vec<GeTrigger>)>,
}

/// A file `NewMap::save` wrote
pub struct SavedFile {
    pub path: PathBuf,
    pub hash: u32,
    pub size: usize,
    /// The gamemode whose level it is
    pub mode: Option<GameMode>,
}

/// The hashes of the .edb files in a folder and in the folders in it, without `ours`
fn hashes_in(folder: &Path, ours: &[PathBuf]) -> Vec<u32> {
    let mut taken = vec![];
    let mut folders = vec![folder.to_path_buf()];
    if let Ok(entries) = std::fs::read_dir(folder) {
        folders.extend(entries.flatten().map(|e| e.path()).filter(|p| p.is_dir()));
    }
    for folder in folders {
        let Ok(entries) = std::fs::read_dir(&folder) else { continue };
        for entry in entries.flatten() {
            let other = entry.path();
            if ours.contains(&other) || other.extension().map(|e| e != "edb").unwrap_or(true) {
                continue;
            }
            let mut head = [0u8; 8];
            if std::fs::File::open(&other)
                .and_then(|mut f| std::io::Read::read_exact(&mut f, &mut head))
                .is_ok()
            {
                taken.push(u32::from_be_bytes(head[4..8].try_into().unwrap()));
            }
        }
    }
    taken
}

/// `wanted`, or the next hash that isn't taken. It is taken afterwards
fn free_hash(wanted: u32, taken: &mut Vec<u32>) -> u32 {
    let mut hash = wanted;
    while taken.contains(&hash) && hash & 0xFFFF != 0xFFFF {
        hash += 1;
    }
    taken.push(hash);
    hash
}

impl NewMap {
    /// Writes the map's files into a folder (the game's mods folder) and says which, the level
    /// in the folder itself first. One file: mt_NAME.edb. Split: that, mg_NAME.edb, and
    /// mt_NAME.edb in the folder of each gamemode the map has a level for. When another file
    /// there has a hash already (two maps made with the same number), the file gets the next
    /// hash that none has
    pub fn save<P: AsRef<Path>>(&mut self, folder: P) -> anyhow::Result<Vec<SavedFile>> {
        let folder = folder.as_ref();
        let level_name = format!("mt_{}.edb", self.name);
        let level_path = folder.join(&level_name);
        let geometry_path = folder.join(format!("mg_{}.edb", self.name));
        let mode_path = |mode: GameMode| folder.join(mode.folder()).join(&level_name);

        let mut ours = vec![level_path.clone()];
        if self.split {
            ours.push(geometry_path.clone());
            ours.extend(self.modes.iter().map(|m| mode_path(*m)));
        }
        let mut taken = hashes_in(folder, &ours);
        self.level_hash = free_hash(self.level_hash, &mut taken);

        let mut files = vec![];
        if self.split {
            let geometry = free_hash(geometry_hash(self.id), &mut taken);
            self.geometry[4..8].copy_from_slice(&geometry.to_be_bytes());
            let level = make_trigger_level(self.level_hash, geometry, self.time, self.spawn, &[])?;
            files.push((level_path, self.level_hash, level, None));
            files.push((geometry_path, geometry, self.geometry.clone(), None));
            for (mode, triggers) in &self.mode_triggers {
                let hash = free_hash(mode_level_hash(self.id, *mode), &mut taken);
                let level = make_trigger_level(hash, geometry, self.time, self.spawn, triggers)?;
                files.push((mode_path(*mode), hash, level, Some(*mode)));
            }
            for (mode, _) in &self.left_out {
                if mode_path(*mode).exists() {
                    self.stats.warnings.push(format!(
                        "{} is still there from before: the map has no level for that gamemode now, delete it",
                        mode_path(*mode).display()
                    ));
                }
            }
        } else {
            let level = make_level_of_geometry(&self.geometry, self.level_hash, self.spawn, &self.more)
                .context("couldn't add the spawn point")?;
            files.push((level_path, self.level_hash, level, None));
        }

        let mut saved = vec![];
        for (path, hash, data, mode) in files {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).with_context(|| format!("couldn't create {}", parent.display()))?;
            }
            std::fs::write(&path, &data).with_context(|| format!("couldn't write {}", path.display()))?;
            saved.push(SavedFile {
                path,
                hash,
                size: data.len(),
                mode,
            });
        }
        Ok(saved)
    }
}

/// What a gamemode's level gets of the scene, or what the scene lacks for it. Anyone's spawn
/// points are in every gamemode, a team's in the team gamemodes, and Golden Gun, GoldenEye and
/// Black Box want their gun, consoles and box
fn mode_triggers(
    mode: GameMode,
    anyones: &[GeTrigger],
    teams: &[Vec<GeTrigger>; 2],
    items: &[(ModeItemKind, GeTrigger)],
) -> Result<Vec<GeTrigger>, String> {
    let mut triggers = anyones.to_vec();
    if mode.teams() {
        if let Some(team) = teams.iter().position(|t| t.is_empty()) {
            return Err(format!("no node named mp_spawn_team{team}..."));
        }
        triggers.extend(teams.iter().flatten().cloned());
    } else if anyones.is_empty() {
        return Err("no node named mp_spawn...".to_string());
    }
    let wanted = match mode {
        GameMode::GoldenGun => Some((ModeItemKind::GoldenGun, "golden_gun")),
        GameMode::GoldenEye => Some((ModeItemKind::Console, "goldeneye")),
        GameMode::BlackBox => Some((ModeItemKind::BlackBox, "black_box")),
        _ => None,
    };
    if let Some((kind, name)) = wanted {
        let count = triggers.len();
        triggers.extend(items.iter().filter(|i| i.0 == kind).map(|i| i.1.clone()));
        if triggers.len() == count {
            return Err(format!("no node named {name}..."));
        }
    }
    Ok(triggers)
}

fn file_name_part(name: &str) -> String {
    let cleaned: String = name
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '_' { c } else { '_' })
        .collect();
    if cleaned.is_empty() {
        "custom".to_string()
    } else {
        cleaned
    }
}

pub fn new_map_from_scene(scene: &GeScene, options: &NewMapOptions) -> anyhow::Result<NewMap> {
    let time = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as u32)
        .unwrap_or(0);
    let level_hash = level_hash(options.id);
    let spawn = options.spawn.unwrap_or_else(|| scene.default_spawn());

    let (geometry, mut stats) = build_geometry_file(scene, level_hash, time);
    // the game stops for good on a multiplayer spawn point without floor within a metre below
    // it: each is put on the floor it stands over
    let mut anyones = vec![];
    let mut teams = [vec![], vec![]];
    for point in &scene.multiplayer_spawns {
        let [x, y, z] = point.position;
        let y = match scene.floor_at(x, z, Some(y + MULTIPLAYER_SPAWN_REACH)) {
            Some(floor) => floor + 0.02,
            None => {
                stats.warnings.push(format!(
                    "the multiplayer spawn point at {x:.2} {y:.2} {z:.2} has no floor below it, the game doesn't start a level with one"
                ));
                y
            }
        };
        let trigger = multiplayer_spawn_trigger([x, y, z], point.yaw, point.team);
        match point.team {
            Some(team) if team < 2 => teams[team as usize].push(trigger),
            Some(team) => stats.warnings.push(format!("the spawn point of team {team} is left out, the teams are 0 and 1")),
            None => anyones.push(trigger),
        }
    }
    stats.multiplayer_spawns = anyones.len() + teams[0].len() + teams[1].len();

    // a gamemode's things stay where they were modelled. the consoles are numbered by name
    let mut placed: Vec<_> = scene.mode_items.iter().collect();
    placed.sort_by(|a, b| a.name.cmp(&b.name));
    let mut items = vec![];
    let mut consoles = 0;
    for item in placed {
        items.push((
            item.kind,
            match item.kind {
                ModeItemKind::GoldenGun => golden_gun_trigger(item.position),
                ModeItemKind::BlackBox => black_box_trigger(item.position),
                ModeItemKind::Console => {
                    consoles += 1;
                    console_trigger(item.position, consoles)
                }
            },
        ));
    }
    let count = |kind| items.iter().filter(|i| i.0 == kind).count();
    if options.split {
        if consoles > 0 && consoles as usize != CONSOLES {
            stats.warnings.push(format!("{consoles} goldeneye... nodes: the game's own maps have {CONSOLES} consoles"));
        }
        for (kind, name) in [(ModeItemKind::GoldenGun, "golden_gun"), (ModeItemKind::BlackBox, "black_box")] {
            if count(kind) > 1 {
                stats.warnings.push(format!("{} {name}... nodes: the game's own maps have one", count(kind)));
            }
        }
    } else if !items.is_empty() {
        stats.warnings.push("a map that is one file has no gamemodes: its golden_gun, goldeneye and black_box nodes are left out".to_string());
    }

    let mut modes = vec![];
    let mut left_out = vec![];
    let mut per_mode = vec![];
    if options.split {
        for mode in GameMode::ALL {
            match mode_triggers(mode, &anyones, &teams, &items) {
                Ok(triggers) => {
                    modes.push(mode);
                    per_mode.push((mode, triggers));
                }
                Err(why) => left_out.push((mode, why)),
            }
        }
    }
    let more: Vec<GeTrigger> = anyones.into_iter().chain(teams.into_iter().flatten()).collect();
    // the game takes an edge that is 0.5 to 1.5 above the player: the floor in front of it
    stats.edges = scene.edges.len();
    for edge in &scene.edges {
        let (dx, dz) = (edge.to[0] - edge.from[0], edge.to[2] - edge.from[2]);
        let len = (dx * dx + dz * dz).sqrt().max(1e-6);
        let middle = [0, 1, 2].map(|k| (edge.from[k] + edge.to[k]) * 0.5);
        let (x, z) = (middle[0] - dz / len * 0.4, middle[2] + dx / len * 0.4);
        if let Some(floor) = scene.floor_at(x, z, Some(middle[1])) {
            let height = middle[1] - floor;
            if !EDGE_HEIGHTS.contains(&height) {
                stats.warnings.push(format!(
                    "the edge at {:.2} {:.2} {:.2} is {height:.2} above the floor in front of it, the game takes one from {} to {}",
                    middle[0], middle[1], middle[2], EDGE_HEIGHTS.start(), EDGE_HEIGHTS.end()
                ));
            }
        }
    }
    Ok(NewMap {
        level_hash,
        spawn,
        stats,
        modes,
        left_out,
        name: file_name_part(&options.name),
        id: options.id,
        time,
        split: options.split,
        geometry,
        more,
        mode_triggers: per_mode,
    })
}

pub fn new_map_from_gltf<P: AsRef<Path>>(gltf: P, options: &NewMapOptions) -> anyhow::Result<NewMap> {
    let mut scene = import_gltf(gltf, &options.import)?;
    if let Some(sky) = &options.sky {
        put_sky(&mut scene, sky)?;
    }
    new_map_from_scene(&scene, options)
}
