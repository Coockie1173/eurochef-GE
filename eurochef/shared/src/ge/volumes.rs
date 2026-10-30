//! Trigger volumes: a place in a level that does something to whoever walks into it. They are
//! the game's own triggers with a shape:
//!
//! - type 0x13, the death barrier. Data 0 how far it reaches in tenths (the game looks at the
//!   shape only within that), data 2 who it kills (bit 0 the player, bit 1 the others)
//! - type 0x10, the damage zone. Data 0 as above, data 2 the damage, data 3 the seconds between
//!   hits (a float), data 4 who, data 5 0x40 flashes the screen. The damage goes by the body, not
//!   by a point
//! - type 0x0C, the level change. Data 0 as above, data 1 the level, data 2 its entrance, data 3
//!   how: 0x80 goes there, 0x90 shows the Mission Complete page on the way and saves the level
//!   as where Continue Game goes, so one that has nowhere to go names itself: a level of 01080004
//!   there breaks Continue Game, and entrance 1 of it is the credits.
//!   The page reads a level's row in the game's table, which a made level hasn't got: the port
//!   gives it one for the page, on a real Wii or Dolphin it would crash there
//! - type 3, the generic trigger, asleep (game flag 1) so the game never runs it: a teleport, only
//!   the port reads it. Data 0 as above, data 1 `TELE`, data 5 to 7 where it goes and data 8 the
//!   way to face there (floats). The type's maker changes data 0 and 4
//!
//! The first three hold 0x42000000 or 0x43000001 and 0x41080000 in the values nobody here knows, as
//! every one of the game's own does.

use anyhow::{bail, ensure};

pub use super::triggers::TRIGGER_LEVEL_CHANGE;
use super::triggers::{GeTrigger, TriggerShape, THIS_LEVEL};

pub const TRIGGER_GENERIC: u32 = 0x03;
pub const TRIGGER_DAMAGE_ZONE: u32 = 0x10;
pub const TRIGGER_DEATH_BARRIER: u32 = 0x13;


const WHO_PLAYER: u32 = 1;
const WHO_OTHERS: u32 = 2;
const DAMAGE_FLASH: u32 = 0x40;
const LEVEL_GO: u32 = 0x80;
const LEVEL_WON: u32 = 0x90;

const BHOP_MARK: u32 = 0x42484F50;
const TELEPORT_MARK: u32 = 0x54454C45;
const TELEPORT_FIRST_VALUE: usize = 5;
const BHOP_FIRST_VALUE: usize = 5;

const DEFAULT_DAMAGE: u32 = 10;
const DEFAULT_SECONDS: f32 = 1.0;

/// A volume's range for how far its shape reaches: data 0
pub fn volume_range(reach: f32) -> u32 {
    (reach * 10.0 + RANGE_SPARE).ceil() as u32
}

/// How much further than its shape a volume's range reaches, in tenths: a body's edge counts
const RANGE_SPARE: f32 = 10.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum VolumeKind {
    Death,
    Damage { amount: u32, seconds: f32 },
    /// Ends the mission with the Mission Complete page, on to `next` or this level again
    Finish { next: Option<u32> },
    /// Goes on to another level, no page
    Level { hash: u32, entrance: u32 },
    /// Puts whoever walks in at `to`, facing `yaw`
    Teleport { to: [f32; 3], yaw: f32 },
}

impl VolumeKind {
    /// A level change, which a gamemode has no use for
    pub fn single_player_only(self) -> bool {
        matches!(self, VolumeKind::Finish { .. } | VolumeKind::Level { .. })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct SceneVolume {
    pub name: String,
    pub kind: VolumeKind,
    pub sphere: bool,
    pub position: [f32; 3],
    /// Half its sides. A sphere's radius is the largest of them
    pub half: [f32; 3],
    /// The trigger's angles, as `rotation_from_axes` makes them
    pub rotation: [f32; 3],
}

impl SceneVolume {
    pub fn shape(&self) -> TriggerShape {
        if self.sphere {
            TriggerShape::sphere(self.half[0].max(self.half[1]).max(self.half[2]))
        } else {
            TriggerShape::cuboid(self.half)
        }
    }

    /// The volume's trigger. In a game with others in it each console looks after its own
    /// player only
    pub fn trigger(&self, multiplayer: bool) -> GeTrigger {
        let shape = self.shape();
        let mut trigger = GeTrigger::new(TRIGGER_DEATH_BARRIER, self.position);
        trigger.rotation = self.rotation;
        trigger.shape = Some(shape);
        trigger.data[0] = Some(volume_range(shape.reach()));
        trigger.data[15] = Some(0x41080000);
        match self.kind {
            VolumeKind::Death => {
                trigger.data[1] = Some(0x42000000);
                trigger.data[2] = Some(if multiplayer { WHO_PLAYER } else { WHO_PLAYER | WHO_OTHERS });
                trigger.data[3] = Some(1);
            }
            VolumeKind::Damage { amount, seconds } => {
                trigger.type_id = TRIGGER_DAMAGE_ZONE;
                trigger.data[1] = Some(0x42000000);
                trigger.data[2] = Some(amount);
                trigger.data[3] = Some(seconds.to_bits());
                trigger.data[4] = Some(WHO_PLAYER);
                trigger.data[5] = Some(DAMAGE_FLASH);
            }
            VolumeKind::Finish { next } => {
                trigger.type_id = TRIGGER_LEVEL_CHANGE;
                trigger.data[1] = Some(next.unwrap_or(THIS_LEVEL));
                trigger.data[2] = Some(1);
                trigger.data[3] = Some(LEVEL_WON);
                trigger.data[4] = Some(0x43000001);
            }
            VolumeKind::Level { hash, entrance } => {
                trigger.type_id = TRIGGER_LEVEL_CHANGE;
                trigger.data[1] = Some(hash);
                trigger.data[2] = Some(entrance);
                trigger.data[3] = Some(LEVEL_GO);
                trigger.data[4] = Some(0x43000001);
            }
            VolumeKind::Teleport { to, yaw } => {
                trigger.type_id = TRIGGER_GENERIC;
                trigger.game_flags = 1;
                trigger.data[1] = Some(TELEPORT_MARK);
                trigger.data[15] = None;
                for (slot, value) in to.into_iter().chain([yaw]).enumerate() {
                    trigger.data[TELEPORT_FIRST_VALUE + slot] = Some(value.to_bits());
                }
            }
        }
        trigger
    }
}

fn level_of(word: &str) -> anyhow::Result<u32> {
    let hash = u32::from_str_radix(word.trim_start_matches("0x"), 16)
        .map_err(|_| anyhow::anyhow!("\"{word}\" isn't a level's hash"))?;
    ensure!(hash >> 24 == 1 && hash & 0xFFFF != 0, "{hash:08X} isn't a level's hash, those look like 0181013F");
    Ok(hash)
}

/// The number a word starts with: `0.5` of `0.5.001`
fn number_of(word: &str) -> Option<f32> {
    let end = word.match_indices('.').nth(1).map(|(at, _)| at).unwrap_or(word.len());
    word[..end].parse().ok()
}

/// What a volume's name says, with what comes after `volume_` split at its underscores:
/// `death`, `damage_25_0.5` (25 every half a second), `finish`, `finish_0181013F` (on to that
/// level), `level_0181013F_2` (to that level's entrance 2). `sphere` anywhere makes it a sphere, words that
/// mean nothing here are the name's own (`death_pit`). Returns None for a name that isn't a
/// volume's
pub fn volume_of_name(name: &str) -> anyhow::Result<Option<(VolumeKind, bool)>> {
    let name = name.to_lowercase();
    let Some(rest) = name.strip_prefix("volume_") else {
        return Ok(None);
    };
    let mut words: Vec<&str> = rest.split('_').filter(|w| !w.is_empty()).collect();
    // Blender's copies end in .001: a number stops at its second dot, and the words below only
    // go by what is before their dot
    let mut plain: Vec<&str> = words.iter().map(|w| w.split('.').next().unwrap_or(w)).collect();
    ensure!(!words.is_empty(), "{name}: a volume is volume_death, volume_damage, volume_finish or volume_level");
    let sphere = plain.contains(&"sphere");
    words.retain(|w| !w.starts_with("sphere"));
    plain.retain(|w| *w != "sphere");
    ensure!(!words.is_empty(), "{name}: a volume is volume_death, volume_damage, volume_finish or volume_level");
    let kind = match plain[0] {
        "death" => VolumeKind::Death,
        "damage" => {
            let mut numbers = words[1..].iter().filter_map(|w| number_of(w));
            let amount = numbers.next().unwrap_or(DEFAULT_DAMAGE as f32);
            let seconds = numbers.next().unwrap_or(DEFAULT_SECONDS);
            ensure!(amount >= 1.0, "{name}: the damage is 1 or more");
            ensure!(seconds > 0.0, "{name}: the seconds between hits are above 0");
            VolumeKind::Damage { amount: amount as u32, seconds }
        }
        "finish" => VolumeKind::Finish {
            next: plain.get(1).filter(|w| w.len() == 8).map(|w| level_of(w)).transpose()?,
        },
        "level" => {
            let Some(hash) = plain.get(1) else {
                bail!("{name}: which level? volume_level_0181013F");
            };
            VolumeKind::Level {
                hash: level_of(hash)?,
                entrance: plain.get(2).and_then(|w| w.parse().ok()).unwrap_or(1),
            }
        }
        other => bail!("{name}: no volume \"{other}\", a volume is volume_death, volume_damage, volume_finish or volume_level"),
    };
    Ok(Some((kind, sphere)))
}

/// What pairs a `teleport_<name>` with its `destination_<name>`: the name after that prefix,
/// without `sphere` or Blender's .001, so copies of a volume share a destination
fn pair_of(name: &str, prefix: &str) -> Option<String> {
    let rest = name.to_lowercase().strip_prefix(prefix)?.to_string();
    let rest = rest.split('.').next().unwrap_or("").to_string();
    Some(rest.split('_').filter(|w| !w.is_empty() && *w != "sphere").collect::<Vec<_>>().join("_"))
}

/// A `teleport_<name>` volume's name, and whether it is a sphere. None for any other name
pub fn teleport_of_name(name: &str) -> anyhow::Result<Option<(String, bool)>> {
    let Some(pair) = pair_of(name, "teleport_") else {
        return Ok(None);
    };
    ensure!(!pair.is_empty(), "{name}: a teleport is teleport_<name>, going to destination_<name>");
    let sphere = name.to_lowercase().split('.').next().unwrap_or("").split('_').any(|w| w == "sphere");
    Ok(Some((pair, sphere)))
}

/// A `destination_<name>` node's name. None for any other name
pub fn destination_of_name(name: &str) -> anyhow::Result<Option<String>> {
    let Some(pair) = pair_of(name, "destination_") else {
        return Ok(None);
    };
    ensure!(!pair.is_empty(), "{name}: a destination is destination_<name>, for teleport_<name>");
    Ok(Some(pair))
}

pub fn is_teleport(trigger: &GeTrigger) -> bool {
    trigger.type_id == TRIGGER_GENERIC && trigger.data[1] == Some(TELEPORT_MARK)
}

/// The bunnyhop tunables of a map's preset, in the order the port has them and as they are
/// called in a node's name
pub const BHOP_VALUES: [&str; 11] =
    ["speed", "height", "gravity", "aircap", "airaccel", "wish", "max", "fall", "drop", "groundaccel", "friction"];

/// A map's own bunnyhop rules: only the port reads them. A trigger of the generic type 3, asleep
/// (game flag 1) so the game never runs it, with `BHOP` in data 1, data 2 bit 0 for bunnyhopping
/// on, bit 1 for pre-strafing, 2 Half-Life Crouch, 3 Hovering and 4 Ledge Hops, and data 5 to 15 the values (floats, the type's maker changes
/// data 0 and 4). A value left out is the port's default
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BhopPreset {
    pub on: bool,
    pub prestrafe: bool,
    pub crouch: bool,
    pub hover: bool,
    pub ledge: bool,
    pub values: [Option<f32>; 11],
}

impl Default for BhopPreset {
    fn default() -> Self {
        Self {
            on: true,
            prestrafe: true,
            crouch: false,
            hover: false,
            ledge: false,
            values: [None; 11],
        }
    }
}

pub fn is_bhop_preset(trigger: &GeTrigger) -> bool {
    trigger.type_id == TRIGGER_GENERIC && trigger.data[1] == Some(BHOP_MARK)
}

impl BhopPreset {
    /// Takes in the words of a name: `speed6_gravity0.8_noprestrafe_crouch`, `off`. What a word doesn't
    /// mention stays as it was, so a preset can be spread over several nodes
    pub fn read_words(&mut self, name: &str, words: &str) -> anyhow::Result<()> {
        for word in words.to_lowercase().split('_').filter(|w| !w.is_empty()) {
            let plain = word.split('.').next().unwrap_or(word);
            match plain {
                "on" => self.on = true,
                "off" => self.on = false,
                "prestrafe" => self.prestrafe = true,
                "noprestrafe" => self.prestrafe = false,
                "crouch" => self.crouch = true,
                "nocrouch" => self.crouch = false,
                "hover" => self.hover = true,
                "nohover" => self.hover = false,
                "ledge" => self.ledge = true,
                "noledge" => self.ledge = false,
                _ => {
                    let digits = word.find(|c: char| c.is_ascii_digit() || c == '.' || c == '-').unwrap_or(word.len());
                    let Some(slot) = BHOP_VALUES.iter().position(|v| *v == &word[..digits]) else {
                        // Blender's copy number
                        if digits == 0 && word.chars().all(|c| c.is_ascii_digit() || c == '.') {
                            continue;
                        }
                        bail!("{name}: no bhop value \"{word}\", they are {} (speed6), on, off, noprestrafe, crouch, hover and ledge", BHOP_VALUES.join(", "));
                    };
                    let Some(value) = number_of(&word[digits..]) else {
                        bail!("{name}: \"{word}\" has no number");
                    };
                    self.values[slot] = Some(value);
                }
            }
        }
        Ok(())
    }

    pub fn trigger(&self) -> GeTrigger {
        let mut trigger = GeTrigger::new(TRIGGER_GENERIC, [0.0; 3]);
        trigger.game_flags = 1;
        trigger.data[1] = Some(BHOP_MARK);
        trigger.data[2] = Some(self.on as u32 | (self.prestrafe as u32) << 1 | (self.crouch as u32) << 2 | (self.hover as u32) << 3 | (self.ledge as u32) << 4);
        for (slot, value) in self.values.iter().enumerate() {
            trigger.data[BHOP_FIRST_VALUE + slot] = value.map(f32::to_bits);
        }
        trigger
    }
}

/// A trigger's angles for a rotation, given as where its own x, y and z axes end up (unit and
/// at right angles). The game's matrix has these as its rows: `sy sx sz + cy cz, cx sz, cy sx sz - sy cz`,
/// `sy sx cz - cy sz, cx cz, cy sx cz + sy sz` and `sy cx, -sx, cy cx`
pub fn rotation_from_axes(axes: [[f32; 3]; 3]) -> [f32; 3] {
    let x = (-axes[2][1]).clamp(-1.0, 1.0).asin();
    if x.cos() > 1e-4 {
        [x, axes[2][0].atan2(axes[2][2]), axes[0][1].atan2(axes[1][1])]
    } else {
        // looking straight up or down: y and z turn around one axis, z stays still
        [x, (-axes[0][2]).atan2(axes[0][0]), 0.0]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn axes_of(angles: [f32; 3]) -> [[f32; 3]; 3] {
        let [x, y, z] = angles;
        let (sx, cx, sy, cy, sz, cz) = (x.sin(), x.cos(), y.sin(), y.cos(), z.sin(), z.cos());
        [
            [sy * sx * sz + cy * cz, cx * sz, cy * sx * sz - sy * cz],
            [sy * sx * cz - cy * sz, cx * cz, cy * sx * cz + sy * sz],
            [sy * cx, -sx, cy * cx],
        ]
    }

    #[test]
    fn angles_come_back() {
        for angles in [[0.0, 0.0, 0.0], [0.0, 1.2, 0.0], [0.3, -2.0, 0.4], [-0.7, 0.1, -1.1]] {
            let got = rotation_from_axes(axes_of(angles));
            for k in 0..3 {
                assert!((got[k] - angles[k]).abs() < 1e-4, "{angles:?} came back as {got:?}");
            }
        }
    }

    #[test]
    fn straight_up_keeps_the_axes() {
        let axes = axes_of([std::f32::consts::FRAC_PI_2, 0.8, 0.0]);
        let again = axes_of(rotation_from_axes(axes));
        for (a, b) in axes.iter().flatten().zip(again.iter().flatten()) {
            assert!((a - b).abs() < 1e-3);
        }
    }

    #[test]
    fn names() {
        let kind = |n: &str| volume_of_name(n).unwrap().unwrap();
        assert_eq!(volume_of_name("volumes").unwrap(), None);
        assert_eq!(kind("volume_death"), (VolumeKind::Death, false));
        assert_eq!(kind("Volume_Death_pit_sphere"), (VolumeKind::Death, true));
        assert_eq!(kind("volume_damage"), (VolumeKind::Damage { amount: 10, seconds: 1.0 }, false));
        assert_eq!(kind("volume_damage_25_0.5"), (VolumeKind::Damage { amount: 25, seconds: 0.5 }, false));
        assert_eq!(kind("volume_damage_25_0.5.001"), (VolumeKind::Damage { amount: 25, seconds: 0.5 }, false));
        assert_eq!(kind("volume_death.003"), (VolumeKind::Death, false));
        assert_eq!(kind("volume_death_sphere.003"), (VolumeKind::Death, true));
        assert_eq!(kind("volume_finish"), (VolumeKind::Finish { next: None }, false));
        assert_eq!(kind("volume_finish_end"), (VolumeKind::Finish { next: None }, false));
        assert_eq!(kind("volume_finish_0181013F.001"), (VolumeKind::Finish { next: Some(0x0181013F) }, false));
        assert_eq!(kind("volume_level_0181013F_2"), (VolumeKind::Level { hash: 0x0181013F, entrance: 2 }, false));
        assert!(volume_of_name("volume_lava").is_err());
        assert!(volume_of_name("volume_level").is_err());
        assert!(volume_of_name("volume_level_12345678").is_err());
        assert!(volume_of_name("volume_finish_12345678").is_err());
    }

    #[test]
    fn teleports() {
        assert_eq!(teleport_of_name("teleports").unwrap(), None);
        assert_eq!(teleport_of_name("Teleport_Roof").unwrap(), Some(("roof".to_string(), false)));
        assert_eq!(teleport_of_name("teleport_top_floor_sphere.002").unwrap(), Some(("top_floor".to_string(), true)));
        assert!(teleport_of_name("teleport_").is_err());
        assert_eq!(destination_of_name("destination_top_floor.001").unwrap(), Some("top_floor".to_string()));
        assert_eq!(destination_of_name("spawn").unwrap(), None);
        let volume = SceneVolume {
            name: String::new(),
            kind: VolumeKind::Teleport { to: [1.0, 2.0, 3.0], yaw: 0.5 },
            sphere: false,
            position: [0.0; 3],
            half: [1.0; 3],
            rotation: [0.0; 3],
        };
        let trigger = volume.trigger(true);
        assert!(is_teleport(&trigger) && trigger.shape.is_some());
        assert_eq!(trigger.game_flags, 1);
        assert_eq!(trigger.data[1], Some(0x54454C45));
        assert_eq!(trigger.data[5], Some(1f32.to_bits()));
        assert_eq!(trigger.data[8], Some(0.5f32.to_bits()));
        assert_eq!(trigger.data[15], None);
    }

    #[test]
    fn bhop_words() {
        let mut preset = BhopPreset::default();
        preset.read_words("n", "speed6_gravity0.8.001_noprestrafe").unwrap();
        preset.read_words("n", "groundaccel12.5").unwrap();
        preset.read_words("n", "friction4.001").unwrap();
        assert_eq!(preset.values[0], Some(6.0));
        assert_eq!(preset.values[2], Some(0.8));
        assert_eq!(preset.values[9], Some(12.5));
        assert_eq!(preset.values[10], Some(4.001));
        assert!(preset.on && !preset.prestrafe);
        let trigger = preset.trigger();
        assert_eq!(trigger.data[1], Some(0x42484F50));
        assert_eq!(trigger.data[2], Some(1));
        assert_eq!(trigger.data[5], Some(6f32.to_bits()));
        assert_eq!(trigger.data[0], None);
        assert_eq!(trigger.data[4], None);
        assert_eq!(trigger.data[15], Some(4.001f32.to_bits()));
        preset.read_words("n", "crouch_hover.001_ledge").unwrap();
        assert_eq!(preset.trigger().data[2], Some(0b11101));
        preset.read_words("n", "nocrouch_nohover_noledge").unwrap();
        assert_eq!(preset.trigger().data[2], Some(1));
        assert!(preset.read_words("n", "jump3").is_err());
        assert!(preset.read_words("n", "speed").is_err());
    }

    #[test]
    fn the_range_takes_in_the_box() {
        let volume = SceneVolume {
            name: String::new(),
            kind: VolumeKind::Death,
            sphere: false,
            position: [0.0; 3],
            half: [3.0, 4.0, 0.0],
            rotation: [0.0; 3],
        };
        assert_eq!(volume.trigger(false).data[0], Some(60));
        assert_eq!(volume.trigger(false).data[2], Some(3));
        assert_eq!(volume.trigger(true).data[2], Some(1));
    }
}
