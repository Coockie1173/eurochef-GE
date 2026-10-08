//! A new level: the file a level's hash names (the game's mt_*.edb).
//!
//! Its triggers start everything: the player's spawn point, and in the game's own levels a
//! trigger of type 62 that loads the map of another file, the geometry. A level made here is one
//! file with its geometry in it, or as the game's multiplayer maps are: a geometry file and a
//! small level file of triggers for each gamemode that load it (mpg_dubai and mpt_dubai_fdm,
//! mpt_dubai_gdm, ...).

use anyhow::ensure;

use super::{
    geomap::{build_empty_file, SceneEdge, SceneLadder, HC_MAP},
    triggers::{
        read_triggers, write_triggers, GeTrigger, LOADED_MAP, TRIGGER_LOAD_MAP, TRIGGER_MULTIPLAYER_SPAWN, TRIGGER_SPAWN,
    },
    writer::Writer,
};

/// The hashes of level files (mt_) and geometry files (mg_) start with these
pub const HC_LEVEL_FILE: u32 = 0x01810000;
pub const HC_GEOMETRY_FILE: u32 = 0x018B0000;
/// New files are numbered from here: the game's own stop below
pub const FIRST_FREE_LEVEL: u32 = 0x0200;
pub const FIRST_FREE_GEOMETRY: u32 = 0x0100;

pub fn level_hash(id: u32) -> u32 {
    HC_LEVEL_FILE | (FIRST_FREE_LEVEL + id) & 0xFFFF
}

pub fn geometry_hash(id: u32) -> u32 {
    HC_GEOMETRY_FILE | (FIRST_FREE_GEOMETRY + id) & 0xFFFF
}

/// The online gamemodes, in the order of the game's Choose Game Mode
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GameMode {
    Conflict,
    TeamConflict,
    GoldenGun,
    BlackBox,
    GoldenEye,
    Heroes,
    LicenseToKill,
    TeamLicenseToKill,
}

impl GameMode {
    pub const ALL: [GameMode; 8] = [
        GameMode::Conflict,
        GameMode::TeamConflict,
        GameMode::GoldenGun,
        GameMode::BlackBox,
        GameMode::GoldenEye,
        GameMode::Heroes,
        GameMode::LicenseToKill,
        GameMode::TeamLicenseToKill,
    ];

    /// The gamemode's folder in the game's mods folder: the levels in it are its maps
    pub fn folder(self) -> &'static str {
        match self {
            GameMode::Conflict => "conflict",
            GameMode::TeamConflict => "team_conflict",
            GameMode::GoldenGun => "golden_gun",
            GameMode::BlackBox => "black_box",
            GameMode::GoldenEye => "goldeneye",
            GameMode::Heroes => "heroes",
            GameMode::LicenseToKill => "license_to_kill",
            GameMode::TeamLicenseToKill => "team_license_to_kill",
        }
    }

    /// Played in two teams: the level has the teams' spawn points as well as anyone's
    pub fn teams(self) -> bool {
        !matches!(self, GameMode::Conflict | GameMode::GoldenGun | GameMode::LicenseToKill)
    }
}

/// A gamemode's level file of a split map. They are numbered apart from the levels that are
/// one file, eight for each map
pub fn mode_level_hash(id: u32, mode: GameMode) -> u32 {
    HC_LEVEL_FILE | (0x1000 + id * 8 + mode as u32) & 0xFFFF
}

pub const TRIGGER_GOLDENEYE_CONSOLE: u32 = 0x4B;
pub const TRIGGER_GOLDEN_GUN: u32 = 0x4C;
pub const TRIGGER_BLACK_BOX: u32 = 0x50;
/// What the three are drawn as, and the files that is in: the values every one of the game's
/// maps that doesn't bring its own has (mpt_archives_gdm, mpt_facility_gec, mpt_zukovsky_bbh...)
const GOLDEN_GUN_OBJECT: (u32, u32) = (0x0400293D, 0x0109000F);
const CONSOLE_OBJECT: (u32, u32) = (0x04003566, 0x01090053);
const BLACK_BOX_OBJECT: (u32, u32) = (0x04003644, 0x01090053);

/// Where Golden Gun's gun lies
pub fn golden_gun_trigger(position: [f32; 3]) -> GeTrigger {
    let mut trigger = GeTrigger::new(TRIGGER_GOLDEN_GUN, position);
    trigger.data[0] = Some(0x14);
    trigger.engine[0] = Some(GOLDEN_GUN_OBJECT.0);
    trigger.engine[1] = Some(GOLDEN_GUN_OBJECT.1);
    trigger
}

/// One of GoldenEye's consoles. The game's maps have five, numbered from 1
pub fn console_trigger(position: [f32; 3], number: u32) -> GeTrigger {
    let mut trigger = GeTrigger::new(TRIGGER_GOLDENEYE_CONSOLE, position);
    trigger.game_flags = 0x10;
    trigger.data[0] = Some(number);
    trigger.data[1] = Some(0x1E);
    trigger.engine[0] = Some(CONSOLE_OBJECT.0);
    trigger.engine[1] = Some(CONSOLE_OBJECT.1);
    trigger
}

/// Where Black Box's box lies
pub fn black_box_trigger(position: [f32; 3]) -> GeTrigger {
    let mut trigger = GeTrigger::new(TRIGGER_BLACK_BOX, position);
    trigger.game_flags = 0x20;
    trigger.engine[0] = Some(BLACK_BOX_OBJECT.0);
    trigger.engine[1] = Some(BLACK_BOX_OBJECT.1);
    trigger
}

/// A level that is only triggers: one that loads the map of the geometry file, the player's
/// spawn point and `more`. The geometry is there before the player is (the game's multiplayer
/// maps are made this way). What the player vaults over and climbs is the level's own
pub fn make_trigger_level(
    level_hash: u32,
    geometry_hash: u32,
    time: u32,
    spawn: [f32; 3],
    more: &[GeTrigger],
    edges: &[SceneEdge],
    ladders: &[SceneLadder],
) -> anyhow::Result<Vec<u8>> {
    let empty = build_empty_file(level_hash, time, edges, ladders);
    let mut set = read_triggers(&empty)?;
    let mut load = GeTrigger::new(TRIGGER_LOAD_MAP, [-0.5, 0.5, 0.5]);
    load.data[0] = Some(geometry_hash);
    load.data[1] = Some(HC_MAP);
    set.triggers.push(load);
    set.triggers.push(spawn_trigger(spawn));
    set.triggers.extend(more.iter().cloned());
    write_triggers(&empty, &set)
}

/// The player's spawn point with the values the game's test level gives it
pub fn spawn_trigger(position: [f32; 3]) -> GeTrigger {
    GeTrigger::with_defaults(TRIGGER_SPAWN, position)
}

/// A spawn point of a multiplayer game. The game's own have data 0 at 1 and the team in data 1
/// for a team's (Zukovsky's team deathmatch: the two ends of the club), both 0 for anyone's
pub fn multiplayer_spawn_trigger(position: [f32; 3], yaw: f32, team: Option<u32>) -> GeTrigger {
    let mut trigger = GeTrigger::with_defaults(TRIGGER_MULTIPLAYER_SPAWN, position);
    trigger.rotation[1] = yaw;
    if let Some(team) = team {
        trigger.data[0] = Some(1);
        trigger.data[1] = Some(team);
    }
    trigger
}

/// Makes a level of a geometry file: it gets the level's hash, a spawn point and `more` triggers. The game starts
/// a level when its file is loaded, so the player never stands in a level that isn't there yet
/// (a geometry file loaded by a trigger of type 62 comes in while the level already runs)
pub fn make_level_of_geometry(
    geometry: &[u8],
    level_hash: u32,
    spawn: [f32; 3],
    more: &[GeTrigger],
) -> anyhow::Result<Vec<u8>> {
    let mut set = read_triggers(geometry)?;
    ensure!(
        !set.triggers.iter().any(|t| t.type_id == TRIGGER_SPAWN),
        "the file has a spawn point already"
    );
    set.triggers.push(spawn_trigger(spawn));
    set.triggers.extend(more.iter().cloned());
    let mut w = Writer {
        buf: write_triggers(geometry, &set)?,
    };
    w.set_u32(4, level_hash);
    Ok(w.buf)
}

/// A level file that loads its geometry from another file, made from the game's own test level
#[derive(Clone, Debug)]
pub struct LevelOptions {
    pub level_hash: u32,
    pub geometry_hash: u32,
    /// Where the player starts, the template's own place when None
    pub spawn: Option<[f32; 3]>,
    /// Keep the template's other triggers (its doors, enemies and test zones)
    pub keep_template_triggers: bool,
    pub time: u32,
}

/// `template` is the game's mt_test.edb
pub fn build_level_file(template: &[u8], options: &LevelOptions) -> anyhow::Result<Vec<u8>> {
    let mut set = read_triggers(template)?;
    ensure!(
        set.triggers.iter().any(|t| t.type_id == TRIGGER_SPAWN),
        "the template has no spawn point (trigger type 0x1C)"
    );

    if !options.keep_template_triggers {
        set.retain(|t| t.type_id == TRIGGER_SPAWN || t.type_id == TRIGGER_LOAD_MAP);
    }

    let mut loads = false;
    for trigger in &mut set.triggers {
        if trigger.type_id == TRIGGER_LOAD_MAP {
            trigger.data[0] = Some(options.geometry_hash);
            trigger.data[1] = Some(LOADED_MAP);
            loads = true;
        }
        if trigger.type_id == TRIGGER_SPAWN {
            if let Some(spawn) = options.spawn {
                trigger.position = spawn;
            }
        }
    }
    if !loads {
        let mut trigger = GeTrigger::new(TRIGGER_LOAD_MAP, options.spawn.unwrap_or([0.0; 3]));
        trigger.data[0] = Some(options.geometry_hash);
        trigger.data[1] = Some(LOADED_MAP);
        set.triggers.push(trigger);
    }

    let mut w = Writer {
        buf: write_triggers(template, &set)?,
    };
    w.set_u32(4, options.level_hash);
    w.set_u32(0x10, options.time);
    Ok(w.buf)
}
