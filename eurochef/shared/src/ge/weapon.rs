//! A gun of its own for one of the port's custom weapons (`mods/weapons`): a file with the gun's
//! parts as entities and copies of a base gun's scripts out of o_guns.edb (0109000F).
//!
//! A gun as the game has it is animscripts of o_guns: its model (the weapon row's +0x178), and
//! what it does when fired, reloaded and so on (the weapon definition's +0x1C list). Each is the
//! same handful of parts (entities of the file, `82nnnnnn` by their index in its list) put on
//! locators, with keyframes for the parts that move. A script is one block, from its header to
//! the end of its keyframes, with nothing but relative pointers inside, so a copy of the bytes
//! works anywhere; only the parts it names are numbered again, as the new file's own.
//!
//! The parts come from a glTF scene: a node named `Part_N` (or `Body`, part 0) is part N, with
//! everything below it. A part the scene doesn't have is left empty, so it isn't seen. The
//! parts are numbered as the base's scripts first name them, and two on the same locator are
//! one part (a firing mode's script with a body of its own); `ge weapon --parts` lists them.
//!
//! The scripts keep the base's hashes. The port gives them hashes of the weapon's own as it
//! loads the file (the order stays the same) and points the weapon at them.

use std::collections::HashMap;

use super::{
    geomap::{EDB_MAGIC, EDB_VERSION, GeScene},
    mesh::{write_mesh, GeVertex, MeshData, MeshPart, ENTITY_MESH, ENTITY_SPLIT},
    weapon_rig::{rest_places, Place},
    texture::{write_texture, GeTexture},
    writer::{read_rel, read_u16, read_u32, Writer},
};

/// A file of things, no map, as o_guns is
const FILE_FLAGS: u32 = 0x2000000A;
const SECTION_FLAGS: u32 = 0x9007;
const HC_SECTION: u32 = 0x08000000;
const HC_DEFAULT_TEXTURE: u32 = 0x06000000;
const HC_LOCAL_ENTITY: u32 = 0x82000000;
const HC_LOCAL_TEXTURE: u32 = 0x86000000;
const HC_LOCAL_MATERIAL: u32 = 0xA5000000;
const TEXTURE_FLAGS: u32 = 0x04000000;
const TEXTURE_BLOOM: u32 = 0x8000;
/// What the game's own gun parts have in their flags
const PART_FLAGS: u32 = 2;
/// A script's commands start here, each with its size in its second byte, 0 ends them
const SCRIPT_COMMANDS: usize = 0x44;
const SCRIPT_CONTROLLERS: usize = 0x34;
const SCRIPT_THREADS: usize = 0x38;
const COMMAND_ENTITY: u8 = 3;
const COMMAND_PARTICLE: u8 = 6;
/// Where a command's data starts, and the entity's (or particle's) word in it
const COMMAND_DATA: usize = 12;
const COMMAND_THING: usize = 8;
const LOCAL: u32 = 0x80000000;
const MOST_BLOCK: usize = 0x100000;

/// The game's guns by the name a custom weapon's base has (port/customweapons.cpp), their
/// inventory id and the scripts of o_guns they are drawn and moved with: the row's model, then
/// what the definition and its parents' +0x1C lists give for each firing mode
pub const BASE_GUNS: &[(&str, u32, &[u32])] = &[
    ("P99", 0x0200016E, &[0x04001DAC, 0x0400207D, 0x04001DAD, 0x04001DEC, 0x04001DED, 0x0400286E, 0x04002872, 0x04002736]),
    ("Torka T3", 0x0200092E, &[0x04002537, 0x04002541, 0x04002538, 0x04002C27, 0x04002C24, 0x04001DEC, 0x04001DED, 0x04002736]),
    ("Hawksman M5A", 0x0200031D, &[0x0400254B, 0x0400254C, 0x0400254D, 0x04002C28, 0x04002C26, 0x04001DEC, 0x04001DED, 0x04002736]),
    ("Kunara V", 0x0200092F, &[0x04002548, 0x04002549, 0x0400254A, 0x04002C29, 0x04002C2A, 0x04001DEC, 0x04001DED, 0x04002736]),
    ("Wolfe .44", 0x0200074E, &[0x0400213E, 0x0400213F, 0x04002141, 0x04001DEC, 0x04001DED, 0x0400286E, 0x04002872, 0x04002736]),
    ("Golden Gun", 0x02000827, &[0x0400256D, 0x0400256E, 0x0400256F, 0x04001DEC, 0x04001DED, 0x0400286E, 0x04002872, 0x04002736]),
    ("Sigmus", 0x02000930, &[0x0400256A, 0x04002F79, 0x0400256B, 0x04002F7A, 0x0400256C, 0x04002F7B, 0x04001DF1, 0x04002634, 0x04002F76]),
    ("Sigmus 9", 0x0200031E, &[0x04002186, 0x04002263, 0x04002187, 0x04001DF1, 0x04002634, 0x04002F76]),
    ("Stauger UA-1", 0x02000931, &[0x0400255A, 0x04002559, 0x04002558, 0x04001DF1, 0x04002634, 0x04002F76]),
    ("Vargen FH-7", 0x0200048A, &[0x040020EA, 0x04002B85, 0x040020E9, 0x04001DF1, 0x04002634, 0x04002F76]),
    ("Strata SV-400", 0x02000932, &[0x04002567, 0x04002568, 0x04002569, 0x04001DF1, 0x04002634, 0x04002F76]),
    ("SLY 2020", 0x02000933, &[0x0400255E, 0x0400255F, 0x04002560, 0x04002152, 0x04002F80, 0x04002150, 0x04002F7F, 0x04001DF1, 0x04002634, 0x04002F76]),
    ("SEGS 550", 0x0200074F, &[0x040020EB, 0x04002F7C, 0x0400212C, 0x04002F7D, 0x040021A1, 0x04002F7E, 0x04002152, 0x04002F80, 0x04002150, 0x04002F7F, 0x04001DF1, 0x04002634, 0x04002F76]),
    ("Drumhead Type-12", 0x02000934, &[0x04002453, 0x04002454, 0x04002455, 0x04002557, 0x04002456, 0x04001DF1, 0x04002634, 0x04002F76]),
    ("PT-9 Interdictus", 0x02000935, &[0x04002561, 0x04002562, 0x04002563, 0x04002565, 0x04002564, 0x04001DF1, 0x04002634, 0x04002F76]),
    ("Masterton M-557", 0x02000936, &[0x04002554, 0x04002555, 0x04002556, 0x04001DF1, 0x04002634, 0x04002F76]),
    ("AK-47", 0x02000319, &[0x04001DB3, 0x04002F74, 0x04002B83, 0x04002F75, 0x04001DF1, 0x04002F78, 0x04002634, 0x04002F76]),
    ("Kallos-TT9", 0x02000937, &[0x04002551, 0x04002FBA, 0x04002553, 0x04002FBB, 0x04002552, 0x04002FBD, 0x040026B4, 0x04002FBC, 0x040021BA, 0x040021B9]),
    ("Anova DP3", 0x02000489, &[0x0400207B, 0x040020CF, 0x0400207C, 0x04002218, 0x040021BA, 0x040021B9]),
    ("Terralite III", 0x0200031F, &[0x0400254F, 0x0400254E, 0x04002550, 0x040026F7]),
    ("Ivana Spec-R", 0x02000938, &[0x04002577, 0x04002578, 0x04002579, 0x04001DF1, 0x04002634, 0x04002F76]),
    ("Pavlov ASR", 0x02000750, &[0x040020E7, 0x04002B84, 0x040020E8, 0x04001DF1, 0x04002634, 0x04002F76]),
    ("Toros AV-400", 0x02000939, &[0x04002570, 0x04002571, 0x04002572, 0x04001DF1, 0x04002634, 0x04002F76]),
    ("Talon HL 450", 0x0200093A, &[0x0400255C, 0x0400255D, 0x04002566, 0x04001DF1, 0x04002634, 0x04002F76]),
    ("WA2000", 0x0200048B, &[0x04001DFD, 0x04002F82, 0x04002105, 0x04002F83, 0x04001DFE, 0x04002F84, 0x04001DF1, 0x04002634, 0x04002F76]),
    ("Gambit CP-208", 0x0200077C, &[0x040021B7, 0x0400255B, 0x040021B8, 0x04001DF1, 0x04002634, 0x04002F76]),
    ("MJR-409", 0x02000828, &[0x04002405, 0x04002642, 0x04002406, 0x04002CAB]),
];

/// A base gun by its name (any case, spaces and dashes left out) or its id in hex
pub fn base_gun(name: &str) -> Option<&'static (&'static str, u32, &'static [u32])> {
    let squash = |s: &str| s.chars().filter(|c| c.is_ascii_alphanumeric()).collect::<String>().to_lowercase();
    let id = u32::from_str_radix(name.trim_start_matches("0x"), 16).ok();
    BASE_GUNS.iter().find(|(n, i, _)| squash(n) == squash(name) || Some(*i) == id)
}

/// A part the base's scripts put on the gun: its entity in the base file and the locator
/// (15xxxxxx) it is put on, FFFFFFFF for one that moves on its own
#[derive(Clone, Debug)]
pub struct GunPart {
    pub entity: u32,
    pub entity_hash: u32,
    pub locator: u32,
    /// the mesh's points (its +0x40 block 5), as they are in the base file: empty for none
    pub points: Vec<u8>,
    /// the part has a second level of detail (its row's +0x10 list), what a guard or a gun on the
    /// floor draws: that entity's points. The body's is the whole gun in one mesh
    pub lod_points: Option<Vec<u8>>,
}

/// A part's levels of detail: its entity row's +0x10 word (the low byte 0x20, above it where the
/// list is from the word), rows of 0x18 bytes ended by a 0 word: where the entity is from the row,
/// a distance and three more words of 0, and a short of FFFF and flags (0x2000 on the first).
/// A part drawn with its node's +0x137 set (a guard's gun, `ge_gun_parts`; one on the floor)
/// draws the one +0x136 numbers, 1, out of the node's own copies at +0x114 (fn_80312F20,
/// fn_8033AAC0): a part without the list has only its own and isn't drawn there
const ROW_LODS: usize = 0x10;
const LODS_FLAGS: u32 = 0x20;
const LOD_SIZE: usize = 0x18;
const LOD_FIRST: u32 = 0xFFFF2000;
const LOD_OTHER: u32 = 0xFFFF0000;

/// The entity a part's row draws far off (or on a guard): its list's second, when it has one
fn lod_entity(d: &[u8], row: usize) -> Option<usize> {
    let word = read_u32(d, row + ROW_LODS);
    if word >> 8 == 0 {
        return None;
    }
    let list = (row + ROW_LODS) as i64 + ((word as i32) >> 8) as i64;
    let second = list + LOD_SIZE as i64;
    if second < 0 || second as usize + LOD_SIZE > d.len() || read_u32(d, list as usize) == 0 {
        return None;
    }
    let to = read_u32(d, second as usize) as i32;
    (to != 0).then(|| (second + to as i64) as usize)
}

/// A mesh's points: block 5 of the blocks its +0x40 word lists (the low byte which are there, the
/// rest where their table is). A row of the table is where its block is (from the row, above
/// the low byte) and the block's number. Block 5 is the count at +5 and, from +6, where the 0x30
/// byte points are (a hash, a scale, a position and a rotation each): what a guard holds the gun
/// by and shoots from (ge_gun_parts has it read as the parts are drawn on a guard, fn_80313EA0).
/// A split (0x603, the Sigmus's body) has the same word: the muzzle 100000B8 and the sight 10000147
/// the hands' flash (fn_80154740) and aiming (fn_800F6620) look for are in it
fn mesh_points(d: &[u8], mesh: usize) -> Vec<u8> {
    if mesh + 0xC0 > d.len() || !matches!(read_u32(d, mesh), ENTITY_MESH | ENTITY_SPLIT) {
        return vec![];
    }
    let word = read_u32(d, mesh + MESH_BLOCKS);
    let bits = word & 0xFF;
    if bits & 1 << POINTS_BLOCK == 0 {
        return vec![];
    }
    let table = (mesh + MESH_BLOCKS) as i64 + ((word as i32) >> 8) as i64;
    let row = table + 4 * (bits & ((1 << POINTS_BLOCK) - 1)).count_ones() as i64;
    if row < 0 || row as usize + 4 > d.len() {
        return vec![];
    }
    let row = row as usize;
    let block = row as i64 + ((read_u32(d, row) as i32) >> 8) as i64;
    if block < 0 || block as usize + 8 > d.len() {
        return vec![];
    }
    let block = block as usize;
    let count = d[block + 5] as usize;
    let points = (block + 6) as i64 + read_u16(d, block + 6) as i16 as i64;
    if points < (block + 8) as i64 {
        return vec![];
    }
    let end = points as usize + count * POINT_SIZE;
    if end > d.len() {
        return vec![];
    }
    d[block..end].to_vec()
}

/// What a locator is, by the parts list `ge_gun_parts` (800CCF70) shows and hides them by
pub fn locator_name(locator: u32) -> &'static str {
    match locator {
        0x15000168 => "body",
        0x150001CA => "always shown",
        0x150000CD => "silencer",
        0x150000D7 => "reflex sight",
        0x1500014F => "acog scope",
        0x15000150 => "laser pointer",
        0x15000157 => "thermal scope",
        0x150000DA => "grenade launcher",
        0x150000DE => "sniper scope",
        0xFFFFFFFF => "moves on its own",
        _ => "",
    }
}

/// A script of the base file as a block of bytes, and where in it the words are that name a
/// part (by its index among the gun's parts once `read_base` has numbered them)
pub struct ScriptBlock {
    pub hash: u32,
    pub bytes: Vec<u8>,
    pub parts: Vec<(usize, usize)>,
}

struct List {
    count: usize,
    at: usize,
}

fn list(d: &[u8], index: usize) -> List {
    let head = 0x40 + index * 8;
    let count = read_u16(d, head) as i16;
    List {
        count: count.max(0) as usize,
        at: read_rel(d, head + 4).unwrap_or(0),
    }
}

/// Where the things of a file start, to know where a script's block ends
fn starts(d: &[u8]) -> Vec<usize> {
    // the lists whose rows begin with a hash, a section, a debug number and an address
    const ROWS: [(usize, usize); 10] = [(2, 20), (3, 32), (4, 28), (5, 16), (6, 16), (7, 20), (8, 20), (11, 20), (14, 16), (15, 28)];
    let mut starts = vec![d.len()];
    for (index, size) in ROWS {
        let l = list(d, index);
        for i in 0..l.count {
            starts.push(read_u32(d, l.at + i * size + 8) as usize);
        }
    }
    let sections = list(d, 0);
    for i in 0..sections.count {
        starts.push(read_u32(d, sections.at + i * 16 + 4) as usize);
        starts.push(read_u32(d, sections.at + i * 16 + 8) as usize);
    }
    starts.sort_unstable();
    starts.dedup();
    starts
}

const MESH_BLOCKS: usize = 0x40;
const POINTS_BLOCK: u32 = 5;
const POINT_SIZE: usize = 0x30;

/// The base gun's scripts out of its file (o_guns.edb), and the parts they name, numbered in the
/// order they are first named
pub fn read_base(d: &[u8], hashes: &[u32]) -> anyhow::Result<(Vec<ScriptBlock>, Vec<GunPart>)> {
    anyhow::ensure!(d.len() > 0xD8 && read_u32(d, 0) == EDB_MAGIC, "not an .edb file");
    let starts = starts(d);
    let scripts = list(d, 5);
    let entities = list(d, 2);
    let mut parts: Vec<GunPart> = vec![];
    let mut blocks = vec![];
    for &hash in hashes {
        //the Kallos-TT9's fourth firing mode names 04002FBC, which o_guns doesn't have: the game
        //finds nothing there either
        let Some(row) = (0..scripts.count).map(|i| scripts.at + i * 16).find(|&r| read_u32(d, r) == hash) else {
            tracing::warn!("the base file has no script {hash:08X}, the gun is left without it as the game's is");
            continue;
        };
        let at = read_u32(d, row + 8) as usize;
        let end = *starts.iter().find(|&&s| s > at).unwrap_or(&d.len());
        anyhow::ensure!(end - at <= MOST_BLOCK, "script {hash:08X} runs over 1 MB");
        for pointer in [SCRIPT_CONTROLLERS, SCRIPT_THREADS] {
            if let Some(target) = read_rel(d, at + pointer) {
                anyhow::ensure!(target >= at && target < end, "script {hash:08X} points outside itself");
            }
        }
        let bytes = d[at..end].to_vec();
        let mut named = vec![];
        let mut p = SCRIPT_COMMANDS;
        while p + 4 <= bytes.len() {
            let (command, size) = (bytes[p], bytes[p + 1] as usize);
            if size == 0 {
                break;
            }
            let word = p + COMMAND_DATA + COMMAND_THING;
            if command == COMMAND_ENTITY && word + 4 <= p + size {
                let thing = read_u32(&bytes, word);
                if thing & LOCAL != 0 && thing >> 24 == HC_LOCAL_ENTITY >> 24 {
                    let entity = thing & 0xFFFFFF;
                    let locator = read_u32(&bytes, p + COMMAND_DATA);
                    //the same entity, or another on the same locator (a firing mode's own copy of
                    //the body): one part
                    let same = |part: &GunPart| part.entity == entity || (locator != 0xFFFFFFFF && part.locator == locator);
                    let index = match parts.iter().position(same) {
                        Some(index) => index,
                        None => {
                            let (entity_hash, points, lod_points) = if (entity as usize) < entities.count {
                                let row = entities.at + entity as usize * 20;
                                let lod = lod_entity(d, row).map(|lod| mesh_points(d, read_u32(d, lod + 8) as usize));
                                (read_u32(d, row), mesh_points(d, read_u32(d, row + 8) as usize), lod)
                            } else {
                                (0, vec![], None)
                            };
                            parts.push(GunPart { entity, entity_hash, locator, points, lod_points });
                            parts.len() - 1
                        }
                    };
                    named.push((word, index));
                }
            }
            if command == COMMAND_PARTICLE {
                tracing::warn!("script {hash:08X} plays a particle of its file, which a gun's own file doesn't have");
            }
            p += size;
        }
        blocks.push(ScriptBlock { hash, bytes, parts: named });
    }
    Ok((blocks, parts))
}

/// The parts of the scene: its triangles by the `Part_N` node they are under, each part's by
/// texture. Triangles under no part's node are part 0's
pub fn scene_parts(scene: &GeScene, count: usize) -> anyhow::Result<Vec<MeshData>> {
    let mut parts = vec![MeshData::default(); count];
    let mut numbers = vec![];
    for name in &scene.rooms {
        let n: usize = name.parse().unwrap_or(0);
        anyhow::ensure!(n < count, "Part_{n}: the base gun has parts 0 to {}", count - 1);
        numbers.push(n);
    }
    let mut by_texture: Vec<HashMap<usize, usize>> = vec![HashMap::new(); count];
    for t in &scene.triangles {
        let n = t.room.map(|r| numbers[r]).unwrap_or(0);
        let texture = t.texture.map(|i| i + 1).unwrap_or(0);
        let slot = *by_texture[n].entry(texture).or_insert_with(|| {
            parts[n].parts.push(MeshPart { texture: texture as u16, two_sided: t.two_sided, ..Default::default() });
            parts[n].parts.len() - 1
        });
        parts[n].parts[slot].triangles.push(t.vertices);
    }
    Ok(parts)
}

/// A part that isn't seen: one triangle without area
fn empty_part() -> MeshData {
    let v = GeVertex { pos: [0.0; 3], normal: [0.0, 1.0, 0.0], uv: [0.0; 2], color: [0, 0, 0, 0] };
    MeshData { parts: vec![MeshPart { texture: 0, triangles: vec![[v; 3]], ..Default::default() }] }
}

/// A part's far level of detail, an entity of its own after the parts
struct Lod {
    part: usize,
    mesh: MeshData,
    points: Vec<u8>,
}

const LOCATOR_BODY: u32 = 0x15000168;
const LOCATOR_SHOWN: u32 = 0x150001CA;
const LOCATOR_NONE: u32 = 0xFFFFFFFF;

/// The far ones of the parts that have them in the base, and the body's: the whole gun (the parts
/// the model script puts on it, attachments left out) in the body's space, as the game's is (the
/// P99's entity 0, as big as its body and where it is); another's is itself
fn lods(parts: &[Option<MeshData>], gun_parts: &[GunPart], blocks: &[ScriptBlock]) -> Vec<Lod> {
    let places = rest_places(blocks, gun_parts.len());
    let mut model: Vec<usize> = vec![];
    for &(_, part) in blocks.first().map(|b| b.parts.as_slice()).unwrap_or(&[]) {
        if !model.contains(&part) && matches!(gun_parts[part].locator, LOCATOR_BODY | LOCATOR_SHOWN | LOCATOR_NONE) {
            model.push(part);
        }
    }
    let mut lods = vec![];
    for (i, g) in gun_parts.iter().enumerate() {
        // the body always has one (the Golden Gun's has none in the game, its guards' gun is made
        // from the one in the hands), with its own points when the base's has none
        let points = match (&g.lod_points, g.locator) {
            (Some(points), _) => points,
            (None, LOCATOR_BODY) => &g.points,
            _ => continue,
        };
        let Some(own) = parts.get(i).and_then(|p| p.as_ref()) else { continue };
        let mut mesh = own.clone();
        if g.locator == LOCATOR_BODY {
            let body = places[i].inverse_matrix();
            let whole: Vec<MeshPart> = model
                .iter()
                .filter_map(|&p| parts.get(p).and_then(|m| m.as_ref()).map(|m| (p, m)))
                .flat_map(|(p, m)| {
                    let place = places[p];
                    m.parts.iter().map(move |part| MeshPart {
                        triangles: part
                            .triangles
                            .iter()
                            .map(|t| t.map(|v| GeVertex { pos: matrix_point(&body, place.point(v.pos)), normal: matrix_direction(&body, place.direction(v.normal)), ..v }))
                            .collect(),
                        flags: vec![],
                        ..part.clone()
                    })
                })
                .collect();
            // a mesh's vertices are numbered in 16 bits: a gun with more keeps its body alone
            if whole.iter().map(|p| p.triangles.len() * 3).sum::<usize>() <= 0xFFFF {
                mesh = MeshData { parts: whole };
            }
        }
        lods.push(Lod { part: i, mesh, points: points.clone() });
    }
    lods
}

/// A point through a glTF matrix (by columns)
fn matrix_point(m: &[[f32; 4]; 4], p: [f32; 3]) -> [f32; 3] {
    [0, 1, 2].map(|r| m[0][r] * p[0] + m[1][r] * p[1] + m[2][r] * p[2] + m[3][r])
}

fn matrix_direction(m: &[[f32; 4]; 4], n: [f32; 3]) -> [f32; 3] {
    let v = [0, 1, 2].map(|r| m[0][r] * n[0] + m[1][r] * n[1] + m[2][r] * n[2]);
    let length = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if length > 1e-9 { v.map(|c| c / length) } else { [0.0, 1.0, 0.0] }
}

fn list_head(w: &mut Writer, count: usize, hashes: i16) -> usize {
    w.i16(count as i16);
    w.i16(hashes);
    w.rel()
}

/// The gun's file: the parts (an empty one for each `None`), the scripts with the parts they
/// name numbered as the file's own, the scene's textures
pub fn build_gun_file(parts: &[Option<MeshData>], gun_parts: &[GunPart], blocks: &[ScriptBlock], textures: &[GeTexture], file_hash: u32, time: u32) -> Vec<u8> {
    let lods = lods(parts, gun_parts, blocks);
    let mut textures_all = vec![GeTexture::solid("default", [0x80, 0x80, 0x80, 0xFF])];
    textures_all.extend(textures.iter().cloned());
    let textures = textures_all;
    let mut blocks: Vec<&ScriptBlock> = blocks.iter().collect();
    blocks.sort_by_key(|b| b.hash); //the game finds a script by a binary search

    let mut w = Writer::new();
    w.u32(EDB_MAGIC);
    w.u32(file_hash);
    w.u32(EDB_VERSION);
    w.u32(FILE_FLAGS);
    w.u32(time);
    w.zeros(0x40 - 0x14);

    let p_sections = list_head(&mut w, 1, 1);
    let p_refptrs = list_head(&mut w, 0, 0);
    let p_entities = list_head(&mut w, parts.len() + lods.len(), 0);
    let p_anims = list_head(&mut w, 0, 0);
    let p_skins = list_head(&mut w, 0, 0);
    let p_scripts = list_head(&mut w, blocks.len(), blocks.len() as i16);
    let mut p_empty = vec![p_refptrs, p_anims, p_skins];
    for _ in 0..8 {
        p_empty.push(list_head(&mut w, 0, 0)); // map, anim modes and sets, particles, swooshes, spreadsheets, fonts, force feedback
    }
    let p_materials = list_head(&mut w, textures.len(), 1);
    let p_textures = list_head(&mut w, textures.len(), -1);
    w.u32(0);
    let p_rel0 = w.rel();
    w.u32(0);
    let p_rel1 = w.rel();
    w.u32(0);
    let p_rel2 = w.rel();
    debug_assert_eq!(w.pos(), 0xD8);

    let mut debug_index = 1u32;
    let mut next_debug = || {
        debug_index += 1;
        debug_index
    };

    for p in p_empty {
        w.point_here(p);
    }
    w.point_here(p_entities);
    let mut a_entities = vec![];
    let mut entity_rows = vec![];
    for i in 0..parts.len() + lods.len() {
        entity_rows.push(w.pos());
        w.u32(HC_LOCAL_ENTITY + i as u32);
        w.u32(next_debug());
        a_entities.push(w.pos());
        w.u32(0);
        w.u32(0);
        w.u32(0);
    }
    w.point_here(p_scripts);
    let mut a_scripts = vec![];
    for b in &blocks {
        w.u32(b.hash);
        w.u32(next_debug());
        a_scripts.push(w.pos());
        w.u32(0);
        w.u32(0);
    }

    w.point_here(p_materials);
    let mut a_materials = vec![];
    for i in 0..textures.len() {
        w.u32(if i == 0 { HC_DEFAULT_TEXTURE } else { HC_LOCAL_MATERIAL + i as u32 });
        w.u32(next_debug());
        a_materials.push(w.pos());
        w.u32(0);
        w.u32(0);
    }
    // the hashes of the textures that aren't local, then the textures
    w.u32(HC_DEFAULT_TEXTURE);
    w.point_here(p_textures);
    let mut a_textures = vec![];
    for (i, texture) in textures.iter().enumerate() {
        let (width, height) = texture.stored_size();
        w.u32(if i == 0 { HC_DEFAULT_TEXTURE } else { HC_LOCAL_TEXTURE + i as u32 });
        w.u32(next_debug());
        a_textures.push(w.pos());
        w.u32(0);
        w.u32(0);
        w.u16(width as u16);
        w.u16(height as u16);
        w.u32(0);
        w.u32(if texture.bloom { TEXTURE_FLAGS | TEXTURE_BLOOM } else { TEXTURE_FLAGS });
    }

    // the parts' levels of detail: themselves, then their own far one
    for (n, lod) in lods.iter().enumerate() {
        w.align(4);
        let row = entity_rows[lod.part];
        let list = w.pos();
        w.set_u32(row + ROW_LODS, ((list - (row + ROW_LODS)) as u32) << 8 | LODS_FLAGS);
        for (to, last) in [(row, LOD_FIRST), (entity_rows[parts.len() + n], LOD_OTHER)] {
            let at = w.pos();
            w.u32((to as i64 - at as i64) as i32 as u32);
            w.zeros(16);
            w.u32(last);
        }
        w.u32(0);
    }

    w.point_here(p_sections);
    w.point_here(p_rel0);
    w.u32(HC_SECTION);
    let a_section = w.pos();
    w.u32(0);
    w.u32(0);
    w.u32(0);

    w.align(32);
    let section_start = w.pos();
    w.point(p_rel1, section_start);
    w.point(p_rel2, section_start);
    w.set_u32(a_section, section_start as u32);
    w.u32(0); // the section's end again
    w.u32(SECTION_FLAGS);
    w.u32(0);
    w.u32(0);

    for (i, at) in a_materials.iter().enumerate() {
        w.set_u32(*at, w.pos() as u32);
        w.u32(0xFFFFFFFF);
        w.u32(i as u32);
        w.u32(0);
    }

    let empty = empty_part();
    let meshes = parts
        .iter()
        .zip(gun_parts)
        .map(|(part, g)| (part.as_ref().unwrap_or(&empty), g.points.as_slice()))
        .chain(lods.iter().map(|lod| (&lod.mesh, lod.points.as_slice())));
    for ((mesh, points), at) in meshes.zip(&a_entities) {
        w.align(32);
        let start = w.pos();
        w.set_u32(*at, start as u32);
        write_mesh(&mut w, mesh, PART_FLAGS);
        // the base part's points, so a guard holds and fires the gun as it does the base
        if !points.is_empty() {
            w.align(4);
            let table = w.pos();
            w.u32(4 << 8 | POINTS_BLOCK);
            w.bytes(points);
            w.set_u32(start + MESH_BLOCKS, ((table - (start + MESH_BLOCKS)) as u32) << 8 | 1 << POINTS_BLOCK);
        }
    }

    for (b, at) in blocks.iter().zip(&a_scripts) {
        w.align(32);
        let start = w.pos();
        w.set_u32(*at, start as u32);
        w.bytes(&b.bytes);
        for &(word, part) in &b.parts {
            w.set_u32(start + word, HC_LOCAL_ENTITY + part as u32);
        }
    }

    for (texture, at) in textures.iter().zip(&a_textures) {
        w.align(32);
        w.set_u32(*at, w.pos() as u32);
        write_texture(&mut w, texture);
    }

    w.align(32);
    let size = w.pos() as u32;
    w.set_u32(0x14, size);
    w.set_u32(0x18, size);
    w.set_u32(0x20, size);
    // what the file needs in memory: its size and something for each of its things
    w.set_u32(0x24, size + 216 + 36 * textures.len() as u32 + 16 * (parts.len() + lods.len()) as u32 + 64 * blocks.len() as u32);
    w.set_u32(a_section + 4, size);
    w.set_u32(section_start, size);
    w.buf
}

/// What `build_weapon` made
pub struct BuiltWeapon {
    pub file: Vec<u8>,
    pub base: &'static str,
    pub scripts: usize,
    pub parts: usize,
    /// The parts the scene gave, by their number
    pub given: Vec<usize>,
    /// The points the scene moved: the part, the point's hash and how far, in the part's space
    pub moved: Vec<(usize, u32, [f32; 3])>,
    pub textures: usize,
}

/// A gun's file from a glTF scene (`Part_N` nodes, or a mesh rigged to `Part_N` bones) and the
/// base gun's scripts out of o_guns.edb (`guns`, its bytes)
pub fn build_weapon(gltf: &str, base: &str, guns: &[u8], brightness: f32, file_hash: u32) -> anyhow::Result<BuiltWeapon> {
    use super::gltf_import::{import_gltf, ImportOptions};
    let Some((name, _, scripts)) = base_gun(base) else {
        let names: Vec<&str> = BASE_GUNS.iter().map(|g| g.0).collect();
        anyhow::bail!("no gun {base}, the base is one of: {}", names.join(", "));
    };
    let (blocks, mut gun_parts) = read_base(guns, scripts)?;
    let places = rest_places(&blocks, gun_parts.len());
    let part_spaces = places.iter().map(|p| p.mirrored().inverse_matrix()).collect();
    let options = ImportOptions { parts: true, brightness, part_spaces, ..Default::default() };
    let scene = import_gltf(gltf, &options)?;
    let moved = place_points(&scene, &places, &mut gun_parts);
    let meshes = scene_parts(&scene, gun_parts.len())?;
    let made: Vec<Option<MeshData>> = meshes.into_iter().map(|m| (m.triangle_count() > 0).then_some(m)).collect();
    let time = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as u32).unwrap_or(0);
    let file = build_gun_file(&made, &gun_parts, &blocks, &scene.textures, file_hash, time);
    Ok(BuiltWeapon {
        file,
        base: name,
        scripts: blocks.len(),
        parts: gun_parts.len(),
        given: made.iter().enumerate().filter(|(_, m)| m.is_some()).map(|(i, _)| i).collect(),
        moved,
        textures: scene.textures.len(),
    })
}

/// A point's position in a mesh's points block (block 5: the count at +5, then from where +6 says
/// the 0x30-byte points, a hash, three floats of 0.5, the position, the rotation)
const POINT_POSITION: usize = 0x14;

/// A part's points as `GunPart::points` has them: the hash, the position in the part's space and
/// where in the bytes the position is
pub fn point_list(points: &[u8]) -> Vec<(u32, [f32; 3], usize)> {
    if points.len() < 8 {
        return vec![];
    }
    let first = 6 + read_u16(points, 6) as i16 as isize;
    (0..points[5] as usize)
        .map(|i| first as usize + i * POINT_SIZE)
        .take_while(|at| at + POINT_POSITION + 12 <= points.len())
        .map(|at| {
            let p = at + POINT_POSITION;
            (read_u32(points, at), [0, 1, 2].map(|k| f32::from_bits(read_u32(points, p + k * 4))), p)
        })
        .collect()
}

/// The points the scene has (`Point_HASH_Part_N` empties) where it puts them, into each part's
/// space as the base's script places it; the far copy's point of the same hash goes as far (the
/// body's: the whole gun's grip and muzzle). Their turn stays the base's
fn place_points(scene: &GeScene, places: &[Place], gun_parts: &mut [GunPart]) -> Vec<(usize, u32, [f32; 3])> {
    let mut moved = vec![];
    for &(part, hash, nth, at) in &scene.part_points {
        let Some(g) = gun_parts.get_mut(part) else { continue };
        let near = point_list(&g.points).into_iter().filter(|p| p.0 == hash).nth(nth);
        let far_only = || g.lod_points.as_deref().and_then(|lod| point_list(lod).into_iter().filter(|p| p.0 == hash).nth(nth));
        let Some((_, old, offset)) = near.or_else(far_only) else { continue };
        let new = places[part].inverse_point(at);
        let by = [0, 1, 2].map(|k| new[k] - old[k]);
        if by.iter().all(|v| v.abs() < 1e-5) {
            continue;
        }
        let shift = |points: &mut Vec<u8>, at: usize| {
            for k in 0..3 {
                let v = f32::from_bits(read_u32(points, at + k * 4)) + by[k];
                points[at + k * 4..at + k * 4 + 4].copy_from_slice(&v.to_bits().to_be_bytes());
            }
        };
        if near.is_none() {
            // a point of the far copy alone: only that one moves
            if let Some(lod) = g.lod_points.as_mut() {
                shift(lod, offset);
            }
            moved.push((part, hash, by));
            continue;
        }
        shift(&mut g.points, offset);
        if let Some(lod) = g.lod_points.as_mut() {
            if let Some(&(_, _, at)) = point_list(lod).iter().filter(|p| p.0 == hash).nth(nth) {
                shift(lod, at);
            }
        }
        moved.push((part, hash, by));
    }
    moved.sort_by_key(|m| (m.0, m.1));
    moved
}
