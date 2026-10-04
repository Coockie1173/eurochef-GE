//! The triggers of a map (the game's "entities": spawn points, enemies, doors, the triggers that
//! load other files...), read from a file and written back into it.
//!
//! The map's +0x58 points to the trigger header: a count and pointers to the triggers' headers
//! (a pointer to the trigger and a link, 8 bytes each), to the scripts' table, to the types
//! (type, subtype and 8 unused bytes each) and to the trigger collisions.
//! A trigger: the index of its type and a debug number (two shorts), the game's flags, a word
//! with a bit for each value that follows, position, rotation, scale, then the values: 16 of
//! data (bits 0 to 15), 8 links to other triggers by index (bits 16 to 23) and 8 for the engine
//! (bits 24 to 31: visual object, its file, script index, collision index, colour...).
//!
//! Changed triggers are written to the end of the file as a new block and the map is pointed at
//! it. Everything else in the file is addressed relative to itself and stays where it is.

use anyhow::{bail, ensure};

use super::writer::{read_f32, read_i32, read_rel, read_u16, read_u32, Writer};

pub const TRIGGER_LOAD_MAP: u32 = 62;
pub const TRIGGER_SPAWN: u32 = 0x1C;
/// A spawn point of a multiplayer game. The game wants floor within a metre below each one: it
/// stops for good on a level with one that has none ("SpawnPoint @ ... is not in the map")
pub const TRIGGER_MULTIPLAYER_SPAWN: u32 = 0x35;
/// A trigger's file, and the map in it (HT_Map_... 0)
pub const LOADED_MAP: u32 = 0x05000000;

const EDB_MAGIC: u32 = 0x47454F4D;
const HEADER_MAPS: usize = 0x70;
const HEADER_SECTIONS: usize = 0x40;
const MAP_TRIGGERS: usize = 0x58;

#[derive(Clone, Debug, PartialEq)]
pub struct GeTrigger {
    pub type_id: u32,
    pub subtype: u32,
    pub debug: u16,
    pub game_flags: u32,
    pub position: [f32; 3],
    pub rotation: [f32; 3],
    pub scale: [f32; 3],
    pub data: [Option<u32>; 16],
    /// Indices of other triggers
    pub links: [Option<i32>; 8],
    /// The engine's values: visual object, its file, script index, collision index, colour, ...
    pub engine: [Option<u32>; 8],
    pub link_ref: i32,
}

impl GeTrigger {
    pub fn new(type_id: u32, position: [f32; 3]) -> Self {
        Self {
            type_id,
            subtype: 0,
            debug: 0,
            game_flags: 0,
            position,
            rotation: [0.0, -0.0, -0.0],
            scale: [1.0, 1.0, 1.0],
            data: [None; 16],
            links: [None; 8],
            engine: [None; 8],
            link_ref: -1,
        }
    }

    /// A new trigger with the values the game's own levels give its type, for the types whose
    /// values are known
    pub fn with_defaults(type_id: u32, position: [f32; 3]) -> Self {
        let mut trigger = Self::new(type_id, position);
        match type_id {
            TRIGGER_SPAWN => {
                trigger.data[2] = Some(0x2D300000);
                trigger.data[3] = Some(0);
                trigger.data[4] = Some(0x43000001);
                trigger.data[5] = Some(0x2D300000);
                trigger.data[6] = Some(0x09000000);
                trigger.data[7] = Some(0x42000000);
            }
            TRIGGER_MULTIPLAYER_SPAWN => {
                trigger.data[0] = Some(0);
                trigger.data[1] = Some(0);
                trigger.data[3] = Some(1);
            }
            _ => {}
        }
        trigger
    }

    pub fn flags(&self) -> u32 {
        let mut flags = 0;
        for (i, v) in self.data.iter().enumerate() {
            flags |= (v.is_some() as u32) << i;
        }
        for (i, v) in self.links.iter().enumerate() {
            flags |= (v.is_some() as u32) << (16 + i);
        }
        for (i, v) in self.engine.iter().enumerate() {
            flags |= (v.is_some() as u32) << (24 + i);
        }
        flags
    }
}

#[derive(Clone, Debug)]
pub struct TriggerSet {
    pub triggers: Vec<GeTrigger>,
    map: usize,
    scripts: Option<usize>,
    collisions: Option<usize>,
}

impl TriggerSet {
    /// Takes a trigger out. Links to it are dropped, links to the ones behind it follow them
    pub fn remove(&mut self, index: usize) {
        if index >= self.triggers.len() {
            return;
        }
        self.triggers.remove(index);
        let fix = |link: i32| match link {
            l if l == index as i32 => None,
            l if l > index as i32 => Some(l - 1),
            l => Some(l),
        };
        for trigger in &mut self.triggers {
            for link in &mut trigger.links {
                *link = link.and_then(fix);
            }
            if trigger.link_ref >= 0 {
                trigger.link_ref = fix(trigger.link_ref).unwrap_or(-1);
            }
        }
    }

    /// Keeps the triggers `keep` says yes to, with `remove`'s care for the links
    pub fn retain(&mut self, keep: impl Fn(&GeTrigger) -> bool) {
        let mut index = 0;
        while index < self.triggers.len() {
            if keep(&self.triggers[index]) {
                index += 1;
            } else {
                self.remove(index);
            }
        }
    }
}

fn first_map(data: &[u8]) -> anyhow::Result<usize> {
    ensure!(data.len() >= 0xD8 && read_u32(data, 0) == EDB_MAGIC, "not a big endian EDB file");
    let count = read_u16(data, HEADER_MAPS) as i16;
    ensure!(count > 0, "the file has no map");
    let list = read_rel(data, HEADER_MAPS + 4).ok_or_else(|| anyhow::anyhow!("the map list is outside the file"))?;
    ensure!(list + 16 <= data.len(), "the map list is cut short");
    let map = read_u32(data, list + 8) as usize;
    ensure!(map + 0x88 <= data.len() && read_u32(data, map) == 0x500, "the file's map isn't in its first part");
    Ok(map)
}

pub fn read_triggers(data: &[u8]) -> anyhow::Result<TriggerSet> {
    let map = first_map(data)?;
    let Some(header) = read_rel(data, map + MAP_TRIGGERS) else {
        return Ok(TriggerSet {
            triggers: vec![],
            map,
            scripts: None,
            collisions: None,
        });
    };
    ensure!(header + 0x14 <= data.len(), "the trigger header is cut short");
    let count = read_u32(data, header) as usize;
    let list = read_rel(data, header + 4);
    let scripts = read_rel(data, header + 8);
    let types = read_rel(data, header + 12);
    let collisions = read_rel(data, header + 16);

    let mut triggers = vec![];
    if count > 0 {
        let list = list.ok_or_else(|| anyhow::anyhow!("no trigger list"))?;
        let types = types.ok_or_else(|| anyhow::anyhow!("no trigger types"))?;
        ensure!(count < 0x10000 && list + count * 8 <= data.len(), "the trigger list is cut short");
        for i in 0..count {
            let entry = list + i * 8;
            let at = read_rel(data, entry).ok_or_else(|| anyhow::anyhow!("trigger {i} is outside the file"))?;
            ensure!(at + 0x30 <= data.len(), "trigger {i} is cut short");
            let type_index = read_u16(data, at) as usize;
            let flags = read_u32(data, at + 8);
            ensure!(
                at + 0x30 + flags.count_ones() as usize * 4 <= data.len() && types + type_index * 16 + 8 <= data.len(),
                "trigger {i} is cut short"
            );
            let vec3 = |o: usize| [read_f32(data, o), read_f32(data, o + 4), read_f32(data, o + 8)];
            let mut trigger = GeTrigger {
                type_id: read_u32(data, types + type_index * 16),
                subtype: read_u32(data, types + type_index * 16 + 4),
                debug: read_u16(data, at + 2),
                game_flags: read_u32(data, at + 4),
                position: vec3(at + 0xC),
                rotation: vec3(at + 0x18),
                scale: vec3(at + 0x24),
                data: [None; 16],
                links: [None; 8],
                engine: [None; 8],
                link_ref: read_i32(data, entry + 4),
            };
            let mut value = at + 0x30;
            for bit in 0..32 {
                if flags & (1 << bit) == 0 {
                    continue;
                }
                let v = read_u32(data, value);
                value += 4;
                match bit {
                    0..=15 => trigger.data[bit] = Some(v),
                    16..=23 => trigger.links[bit - 16] = Some(v as i32),
                    _ => trigger.engine[bit - 24] = Some(v),
                }
            }
            triggers.push(trigger);
        }
    }

    Ok(TriggerSet {
        triggers,
        map,
        scripts,
        collisions,
    })
}

/// The file with these triggers in place of its own
pub fn write_triggers(data: &[u8], set: &TriggerSet) -> anyhow::Result<Vec<u8>> {
    let map = first_map(data)?;
    ensure!(map == set.map, "these triggers were read from another file");
    let old_size = data.len();
    ensure!(
        read_u32(data, 0x14) as usize == old_size && read_u32(data, 0x18) as usize == old_size,
        "the file has parts that are loaded on their own (its header says {} of {} bytes stay loaded), triggers can't be added to it yet",
        read_u32(data, 0x18),
        read_u32(data, 0x14)
    );
    if set.triggers.len() > 0xFFFF {
        bail!("too many triggers");
    }

    let mut types: Vec<(u32, u32)> = vec![];
    let mut type_of = vec![];
    for trigger in &set.triggers {
        let key = (trigger.type_id, trigger.subtype);
        let index = match types.iter().position(|t| *t == key) {
            Some(i) => i,
            None => {
                types.push(key);
                types.len() - 1
            }
        };
        type_of.push(index as u16);
    }

    let mut w = Writer::new();
    w.bytes(data);
    w.align(16);
    let header = w.pos();
    w.u32(set.triggers.len() as u32);
    let p_list = w.rel();
    let p_scripts = w.rel();
    let p_types = w.rel();
    let p_collisions = w.rel();

    w.point_here(p_list);
    let entries: Vec<usize> = set
        .triggers
        .iter()
        .map(|t| {
            let at = w.rel();
            w.i32(t.link_ref);
            at
        })
        .collect();
    for ((trigger, entry), type_index) in set.triggers.iter().zip(entries).zip(type_of) {
        w.point_here(entry);
        w.u16(type_index);
        w.u16(trigger.debug);
        w.u32(trigger.game_flags);
        w.u32(trigger.flags());
        w.f32s(&trigger.position);
        w.f32s(&trigger.rotation);
        w.f32s(&trigger.scale);
        for v in trigger.data.iter().flatten() {
            w.u32(*v);
        }
        for v in trigger.links.iter().flatten() {
            w.i32(*v);
        }
        for v in trigger.engine.iter().flatten() {
            w.u32(*v);
        }
    }

    let types_at = w.pos();
    w.point(p_types, types_at);
    for (type_id, subtype) in &types {
        w.u32(*type_id);
        w.u32(*subtype);
        w.zeros(8);
    }
    // the scripts and the collisions stay where they are. a file without them: zeros
    let nothing = w.pos();
    w.zeros(16);
    w.point(p_scripts, set.scripts.unwrap_or(nothing));
    w.point(p_collisions, set.collisions.unwrap_or(nothing));
    w.align(32);

    w.point(map + MAP_TRIGGERS, header);

    let size = w.pos() as u32;
    let grown = size - old_size as u32;
    w.set_u32(0x14, size);
    w.set_u32(0x18, size);
    if read_u32(data, 0x20) as usize == old_size {
        w.set_u32(0x20, size);
    }
    w.set_u32(0x24, read_u32(data, 0x24) + grown);

    // the section the file ends with ends later now: in the section list, and in the section's
    // own first word
    let sections = read_u16(data, HEADER_SECTIONS) as usize;
    if let Some(list) = read_rel(data, HEADER_SECTIONS + 4) {
        for i in 0..sections {
            let entry = list + i * 16;
            if entry + 16 > old_size || read_u32(data, entry + 8) as usize != old_size {
                continue;
            }
            w.set_u32(entry + 8, size);
            let start = read_u32(data, entry + 4) as usize;
            if start + 4 <= old_size && read_u32(data, start) as usize == old_size {
                w.set_u32(start, size);
            }
        }
    }

    Ok(w.buf)
}
