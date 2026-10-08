//! Builds a geometry file (the game's mg_*.edb) from triangles and textures.
//!
//! What the file holds, in the order it is written:
//! - the GEOM header (0xD8 bytes): hash, version 263, sizes, then a count and a relative pointer
//!   for each list of the file's things
//! - the lists: the map (16 bytes), the entities (20 bytes each), materials (16), the hashes of
//!   the textures that aren't local, textures (28), sections (16), reference pointers (16)
//! - the one section: a 16 byte header, the materials (12 bytes: no hash, the texture's index),
//!   the map with its one zone, the entities, the two entities the zone refers to and the textures
//!
//! The file's first entity is a group (0x603) of all the meshes that are drawn, the others are
//! the parts of a collision that isn't drawn. The map places each with a placement (0x3C
//! bytes). What a body may touch is looked up
//! in the zone's placement tree: a node with children has a count, flags and for each child a
//! pointer and a box (0x1C bytes), a leaf (count 0) lists placement indices with their flags.
//! Flag 8 is on everything a body is tested against.
//!
//! A scene with rooms (see `rooms`) gets a zone for each: the room's drawn triangles are one
//! placed group in its zone, the portals follow the map (0x44 bytes each: the two zones, flags,
//! the four corners at +0x14) and each zone has its links at +0x20 (8 bytes: this zone, the
//! other, the portal's index, how many more links to the same zone follow, and 1 when the
//! portal's front is the other zone's side). What a body collides with is always parts that
//! aren't drawn then, each listed by every zone it reaches into: a body is tested against what
//! its own zone lists, also when it stands in a doorway.

use std::collections::HashMap;
use super::{
    mesh::{
        halve, write_mesh_with_collision, write_split, FACE_NO_COLLISION, Bounds, GeVertex, MeshData, MeshPart,
        COLOUR_ONE, MAX_STRIP_TRIANGLES,
    },
    rooms::{make_zoning, positions, Zoning},
    texture::{write_texture, GeTexture},
    writer::Writer,
};

pub const EDB_MAGIC: u32 = 0x47454F4D;
pub const EDB_VERSION: u32 = 263;
/// Files with a map and entities
const FILE_FLAGS: u32 = 0x20000006;
const SECTION_FLAGS: u32 = 0x9007;
pub const HC_MAP: u32 = 0x0500000D;
/// The map of a file that is only triggers, as the game's own (mpt_*) have it
pub const HC_TRIGGER_MAP: u32 = 0x0500001E;
const HC_SECTION: u32 = 0x08000000;
const HC_DEFAULT_TEXTURE: u32 = 0x06000000;
const HC_LOCAL_ENTITY: u32 = 0x82000000;
const HC_LOCAL_TEXTURE: u32 = 0x86000000;
const HC_LOCAL_MATERIAL: u32 = 0xA5000000;
const TEXTURE_FLAGS: u32 = 0x04000000;
const PLACEMENT_COLLIDES: u16 = 9;
/// A placement's flags in a zone's tree when no body is tested against it
const PLACEMENT_LISTED: u8 = 1;

/// Triangles a drawn mesh and a collision mesh hold at most. The game goes over all triangles of
/// a mesh whose bounds a body touches, so collision meshes are kept small
const MAX_DRAWN_TRIANGLES: usize = 600;
const MAX_COLLISION_TRIANGLES: usize = 256;
/// How far in front of a triangle a body's middle and its ends are when it touches it
const BODY_REACHES: [f32; 5] = [0.05, 0.3, 0.6, 1.0, 1.7];
const BODY_STEP: f32 = 0.4;
/// A zone's list that is no longer than this stays one leaf
const PLACEMENT_LEAF: usize = 16;
const PLACEMENT_TREE_LEVELS: usize = 3;
/// A node's box is this much larger than what is in it (a floor's own box has no height)
const PLACEMENT_BOX_MARGIN: f32 = 0.5;
/// How far outside a zone's box a collision part is still listed by the zone
const ZONE_REACH: f32 = 1.0;

/// The zone's settings (fog, ambience, colours) as the game's test level has them
const ZONE_IDENTIFIER: [u32; 20] = [
    0, 0, 0x3F000000, 0x3F800000, 0, 0x3F800000, 0, 0, 0x00010000, 0, 0, 0x80808000, 0xFFFFFF00,
    0x8080FF00, 0xFFFFFFFF, 0, 0xFFFFFFFF, 0, 0, 0,
];
/// Which of the map's skies a zone is drawn with is the word at +0x38 of its settings, -1: none
const ZONE_SKY_INDEX: usize = 14;

#[derive(Clone)]
pub struct SceneTriangle {
    /// Index into the scene's textures, None for the grey default
    pub texture: Option<usize>,
    pub vertices: [GeVertex; 3],
    /// Left out of the collision that is made from the drawn triangles
    pub no_collision: bool,
    /// Index into the scene's rooms, None for a triangle of no room's node
    pub room: Option<usize>,
    /// Drawn from behind as well
    pub two_sided: bool,
    /// Baked light that is drawn over it: a texture of the scene and where each corner is on it
    pub lightmap: Option<(usize, [[f32; 2]; 3])>,
}

/// A portal as the scene has it: the triangles of its mesh (a flat quad) and its two rooms
#[derive(Clone)]
pub struct ScenePortal {
    pub name: String,
    pub rooms: [usize; 2],
    pub triangles: Vec<[[f32; 3]; 3]>,
}

/// A spawn point of a multiplayer game as the scene has it
#[derive(Clone, Debug)]
pub struct SceneSpawn {
    pub position: [f32; 3],
    /// Which way the player looks, around the y axis: 0 is along +z, a quarter turn along +x
    pub yaw: f32,
    /// The team it belongs to in a team game, None: anyone's
    pub team: Option<u32>,
}

/// What a gamemode has in a level besides spawn points
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModeItemKind {
    /// Where Golden Gun's gun lies
    GoldenGun,
    /// One of GoldenEye's consoles
    Console,
    /// Where Black Box's box lies
    BlackBox,
}

/// A gamemode's thing as the scene has it
#[derive(Clone, Debug)]
pub struct SceneModeItem {
    pub kind: ModeItemKind,
    /// The node's name: the consoles are numbered in the order of their names
    pub name: String,
    pub position: [f32; 3],
}

/// An edge the player gets over or onto with the action button: the rim of an obstacle's top.
/// It is taken from one side only, towards `(dz, 0, -dx)` of the way it runs
#[derive(Clone, Debug, PartialEq)]
pub struct SceneEdge {
    pub from: [f32; 3],
    pub to: [f32; 3],
    /// EDGE_...
    pub flags: u16,
}

/// The player vaults over it and comes down about 1.25 behind it
pub const EDGE_VAULT: u16 = 0x01;
/// The same, further: about 1.6 behind it
pub const EDGE_LONG_VAULT: u16 = 0x02;
/// The player climbs up and stands on the top behind it
pub const EDGE_CLIMB: u16 = 0x10;
/// A ladder's top: where the player gets on from above and off at the end of the climb
pub const EDGE_LADDER_TOP: u16 = 0x100;
/// A polygon that is a piece of a ladder
pub const POLYGON_RUNG: u16 = 0x1000;
/// How tall the game's own pieces of a ladder are
pub const RUNG_HEIGHT: f32 = 0.5;
/// How far above the floor a ladder's lowest piece ends, as the game's own do: the player who
/// comes down one that reaches the floor never gets off it
pub const LADDER_FOOT: f32 = 0.5;
/// How far in front of an edge and behind it the zones are looked up that get it
const EDGE_ZONE_REACH: f32 = 0.5;
/// How far above the player the game takes an edge
pub const EDGE_HEIGHTS: std::ops::RangeInclusive<f32> = 0.5..=1.5;

/// The edges of a mesh that stands for an obstacle's top: the rim of its faces that look up,
/// each run so that it is taken from outside, towards the faces
pub fn rim_edges(triangles: &[[[f32; 3]; 3]], flags: u16) -> Vec<SceneEdge> {
    // a millimetre apart is the same corner
    let key = |p: [f32; 3]| p.map(|v| (v * 1000.0).round() as i64);
    let top: Vec<&[[f32; 3]; 3]> = triangles
        .iter()
        .filter(|t| {
            let u = [0, 1, 2].map(|k| t[1][k] - t[0][k]);
            let v = [0, 1, 2].map(|k| t[2][k] - t[0][k]);
            let n = [u[1] * v[2] - u[2] * v[1], u[2] * v[0] - u[0] * v[2], u[0] * v[1] - u[1] * v[0]];
            let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
            len > 1e-9 && n[1] / len > 0.7
        })
        .collect();
    let mut uses: HashMap<([i64; 3], [i64; 3]), usize> = HashMap::new();
    let sides = |t: &[[f32; 3]; 3]| [(t[0], t[1], t[2]), (t[1], t[2], t[0]), (t[2], t[0], t[1])];
    for t in &top {
        for (a, b, _) in sides(t) {
            let (a, b) = (key(a), key(b));
            *uses.entry(if a <= b { (a, b) } else { (b, a) }).or_default() += 1;
        }
    }
    let mut edges = vec![];
    for t in &top {
        for (a, b, c) in sides(t) {
            let (ka, kb) = (key(a), key(b));
            if uses[&if ka <= kb { (ka, kb) } else { (kb, ka) }] != 1 {
                continue;
            }
            let (dx, dz) = (b[0] - a[0], b[2] - a[2]);
            if dx * dx + dz * dz < 0.05 * 0.05 {
                continue;
            }
            // the faces are on the side the player goes to: (dz, 0, -dx)
            let inside = dz * (c[0] - a[0]) - dx * (c[2] - a[2]) > 0.0;
            let (from, to) = if inside { (a, b) } else { (b, a) };
            edges.push(SceneEdge { from, to, flags });
        }
    }
    edges
}

/// A ladder: an upright strip the player climbs, facing it. `top` is the edge at its upper end
/// and runs as a vault's edge does (the player gets off towards `(dz, 0, -dx)` of it), `bottom`
/// are the two corners below its ends
#[derive(Clone, Debug, PartialEq)]
pub struct SceneLadder {
    pub top: [[f32; 3]; 2],
    pub bottom: [[f32; 3]; 2],
}

impl SceneLadder {
    pub fn height(&self) -> f32 {
        (self.top[0][1] + self.top[1][1] - self.bottom[0][1] - self.bottom[1][1]) * 0.5
    }

    pub fn width(&self) -> f32 {
        let d = [0, 1, 2].map(|k| self.top[1][k] - self.top[0][k]);
        (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt()
    }

    /// The way the top runs, level
    pub fn along(&self) -> [f32; 3] {
        let (dx, dz) = (self.top[1][0] - self.top[0][0], self.top[1][2] - self.top[0][2]);
        let len = (dx * dx + dz * dz).sqrt().max(1e-6);
        [dx / len, 0.0, dz / len]
    }

    /// From the ladder to the player on it, level
    pub fn front(&self) -> [f32; 3] {
        let along = self.along();
        [-along[2], 0.0, along[0]]
    }

    /// The pieces from the lowest up, as the game's own ladders have them: corners at the
    /// bottom and the top of the top's end, then of its start, which is counter clockwise
    /// seen from the player
    pub fn rungs(&self) -> Vec<[[f32; 3]; 4]> {
        let count = ((self.height() / RUNG_HEIGHT).round() as usize).max(1);
        let at = |side: usize, i: usize| {
            let f = i as f32 / count as f32;
            [0, 1, 2].map(|k| self.bottom[side][k] + (self.top[side][k] - self.bottom[side][k]) * f)
        };
        (0..count).map(|i| [at(1, i), at(1, i + 1), at(0, i + 1), at(0, i)]).collect()
    }
}

/// The ladder a mesh stands for: a flat upright face (a quad) that looks at the player on it,
/// as wide and as tall as the ladder. None for a mesh without a face that looks sideways
pub fn ladder_of(triangles: &[[[f32; 3]; 3]]) -> Option<SceneLadder> {
    let mut normal = [0.0f32; 3];
    for t in triangles {
        let u = [0, 1, 2].map(|k| t[1][k] - t[0][k]);
        let v = [0, 1, 2].map(|k| t[2][k] - t[0][k]);
        let n = [u[1] * v[2] - u[2] * v[1], u[2] * v[0] - u[0] * v[2], u[0] * v[1] - u[1] * v[0]];
        normal = [0, 1, 2].map(|k| normal[k] + n[k]);
    }
    let len = (normal[0] * normal[0] + normal[2] * normal[2]).sqrt();
    let whole = (normal[0] * normal[0] + normal[1] * normal[1] + normal[2] * normal[2]).sqrt();
    // lying down or closed (a box's faces cancel out)
    if len < 1e-6 || len < whole * 0.2 {
        return None;
    }
    let front = [normal[0] / len, 0.0, normal[2] / len];
    let along = [front[2], 0.0, -front[0]];
    let points: Vec<[f32; 3]> = triangles.iter().flatten().copied().collect();
    let side = |p: &[f32; 3]| p[0] * along[0] + p[2] * along[2];
    let out = |p: &[f32; 3]| p[0] * front[0] + p[2] * front[2];
    let range = |f: &dyn Fn(&[f32; 3]) -> f32| {
        points.iter().fold((f32::MAX, f32::MIN), |(lo, hi), p| (lo.min(f(p)), hi.max(f(p))))
    };
    let (s0, s1) = range(&side);
    let (y0, y1) = range(&|p| p[1]);
    if s1 - s0 < 0.05 || y1 - y0 < 0.05 {
        return None;
    }
    // a ladder that leans: how far out its lower and its upper half are
    let middle = (y0 + y1) * 0.5;
    let mean = |upper: bool| {
        let (sum, count) = points
            .iter()
            .filter(|p| (p[1] >= middle) == upper)
            .fold((0.0, 0), |(sum, count), p| (sum + out(p), count + 1));
        sum / count.max(1) as f32
    };
    let corner = |s: f32, y: f32, t: f32| [along[0] * s + front[0] * t, y, along[2] * s + front[2] * t];
    let (low, high) = (mean(false), mean(true));
    Some(SceneLadder {
        top: [corner(s0, y1, high), corner(s1, y1, high)],
        bottom: [corner(s0, y0, low), corner(s1, y0, low)],
    })
}

#[derive(Clone, Default)]
pub struct GeScene {
    pub textures: Vec<GeTexture>,
    pub triangles: Vec<SceneTriangle>,
    /// Triangles a body collides with. Empty: the drawn triangles are collided with
    pub collision: Vec<[[f32; 3]; 3]>,
    /// Triangles a body collides with as well, whichever the others are: a ramp over stairs
    pub added_collision: Vec<[[f32; 3]; 3]>,
    /// Where the scene says the player starts
    pub spawn: Option<[f32; 3]>,
    /// The rooms' names. Empty: the level is one zone, drawn as a whole
    pub rooms: Vec<String>,
    pub portals: Vec<ScenePortal>,
    /// The sky's triangles, where they stand in the level. Empty: no sky
    pub sky: Vec<SceneTriangle>,
    pub multiplayer_spawns: Vec<SceneSpawn>,
    /// The golden gun, the consoles and the black box: what only some gamemodes have
    pub mode_items: Vec<SceneModeItem>,
    /// What the player vaults over and climbs onto
    pub edges: Vec<SceneEdge>,
    pub ladders: Vec<SceneLadder>,
}

#[derive(Debug, Clone, Default)]
pub struct BuildStats {
    pub drawn_meshes: usize,
    pub drawn_triangles: usize,
    pub collision_meshes: usize,
    pub collision_triangles: usize,
    pub textures: usize,
    pub sky_triangles: usize,
    pub multiplayer_spawns: usize,
    pub edges: usize,
    pub ladders: usize,
    pub bounds: Option<Bounds>,
    /// 1 for a level without rooms
    pub zones: usize,
    pub portals: usize,
    /// What there is to say about the rooms: portals left out, rooms nothing leads to
    pub warnings: Vec<String>,
}

impl GeScene {
    /// The ladders as they are written: each one's foot put `LADDER_FOOT` above the floor in
    /// front of it, and what there is to say about the ones that can't be
    pub fn fitted_ladders(&self) -> (Vec<SceneLadder>, Vec<String>) {
        let mut warnings = vec![];
        let mut ladders = vec![];
        for ladder in &self.ladders {
            let front = ladder.front();
            let middle = |ends: &[[f32; 3]; 2]| [0, 1, 2].map(|k| (ends[0][k] + ends[1][k]) * 0.5);
            let (low, high) = (middle(&ladder.bottom), middle(&ladder.top));
            let place = format!("the ladder at {:.2} {:.2} {:.2}", high[0], high[1], high[2]);
            let floor = self.floor_at(low[0] + front[0] * 0.4, low[2] + front[2] * 0.4, Some((low[1] + high[1]) * 0.5));
            let Some(floor) = floor else {
                warnings.push(format!("{place} has no floor in front of its foot"));
                ladders.push(ladder.clone());
                continue;
            };
            // the wall it hangs on looks the other way when the quad was made the wrong way round.
            // All the way up it: the drawn ladder's own rungs look back at it too, here and there
            let collision = self.collision_triangles();
            let faces_a_wall = |at: [f32; 3]| {
                collision.iter().any(|t| {
                    let (a, b, c) = (t[0].pos, t[1].pos, t[2].pos);
                    let u = [0, 1, 2].map(|k| b[k] - a[k]);
                    let v = [0, 1, 2].map(|k| c[k] - a[k]);
                    let n = [u[1] * v[2] - u[2] * v[1], u[2] * v[0] - u[0] * v[2], u[0] * v[1] - u[1] * v[0]];
                    let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
                    if len < 1e-9 || (n[0] * front[0] + n[2] * front[2]) / len > -0.5 {
                        return false;
                    }
                    let away = (0..3).map(|k| n[k] / len * (at[k] - a[k])).sum::<f32>();
                    // within reach of the wall's plane, and in the triangle seen along its normal
                    let inside = |p: [f32; 3], q: [f32; 3]| {
                        let e = [0, 1, 2].map(|k| q[k] - p[k]);
                        let m = [0, 1, 2].map(|k| at[k] - p[k]);
                        let x = [e[1] * m[2] - e[2] * m[1], e[2] * m[0] - e[0] * m[2], e[0] * m[1] - e[1] * m[0]];
                        x[0] * n[0] + x[1] * n[1] + x[2] * n[2] >= 0.0
                    };
                    away.abs() < 0.3 && inside(a, b) && inside(b, c) && inside(c, a)
                })
            };
            let against_its_wall = [0.13, 0.31, 0.5, 0.69, 0.87]
                .iter()
                .all(|f| faces_a_wall([0, 1, 2].map(|k| low[k] + (high[k] - low[k]) * f)));
            if against_its_wall {
                warnings.push(format!(
                    "{place} looks into the wall behind it: its front is the side the player climbs, turn it around"
                ));
            }
            let foot = floor + LADDER_FOOT;
            if high[1] - foot < RUNG_HEIGHT {
                warnings.push(format!(
                    "{place} is {:.2} above the floor in front of it, too low for a ladder: left out",
                    high[1] - floor
                ));
                continue;
            }
            if low[1] - foot > 0.25 {
                warnings.push(format!(
                    "{place} starts {:.2} above the floor in front of it: too high to walk onto, it is only got onto from its top (the game's own start {LADDER_FOOT} above the floor)",
                    low[1] - floor
                ));
            }
            // along its own slope, for one that leans
            let f = (foot - low[1]) / (high[1] - low[1]);
            let mut fitted = ladder.clone();
            if f > 0.0 {
                for side in 0..2 {
                    fitted.bottom[side] = [0, 1, 2].map(|k| ladder.bottom[side][k] + (ladder.top[side][k] - ladder.bottom[side][k]) * f);
                }
            }
            ladders.push(fitted);
        }
        (ladders, warnings)
    }

    /// The floor below (or the nearest above) a point: the highest upward facing triangle of the
    /// collision that a vertical line through it hits
    pub fn floor_at(&self, x: f32, z: f32, below: Option<f32>) -> Option<f32> {
        let mut best: Option<f32> = None;
        for t in self.collision_triangles() {
            if t[0].normal[1] < 0.3 {
                continue;
            }
            let (a, b, c) = (t[0].pos, t[1].pos, t[2].pos);
            let det = (b[2] - c[2]) * (a[0] - c[0]) + (c[0] - b[0]) * (a[2] - c[2]);
            if det.abs() < 1e-9 {
                continue;
            }
            let u = ((b[2] - c[2]) * (x - c[0]) + (c[0] - b[0]) * (z - c[2])) / det;
            let v = ((c[2] - a[2]) * (x - c[0]) + (a[0] - c[0]) * (z - c[2])) / det;
            if u < 0.0 || v < 0.0 || u + v > 1.0 {
                continue;
            }
            let y = u * a[1] + v * b[1] + (1.0 - u - v) * c[1];
            if below.map(|limit| y > limit).unwrap_or(false) {
                continue;
            }
            if best.map(|b| y > b).unwrap_or(true) {
                best = Some(y);
            }
        }
        best
    }

    /// Where the player starts when nothing else says: the scene's own spawn point, or on the
    /// floor in the middle of it
    pub fn default_spawn(&self) -> [f32; 3] {
        if let Some(spawn) = self.spawn {
            return spawn;
        }
        let bounds = self.bounds().or_zero();
        let centre = bounds.center();
        // a few places around the middle, the first with a floor
        for (dx, dz) in [(0.0, 0.0), (0.25, 0.0), (-0.25, 0.0), (0.0, 0.25), (0.0, -0.25), (0.25, 0.25), (-0.25, -0.25)] {
            let x = centre[0] + dx * (bounds.max[0] - bounds.min[0]);
            let z = centre[2] + dz * (bounds.max[2] - bounds.min[2]);
            if let Some(y) = self.floor_at(x, z, None) {
                return [x, y + 0.02, z];
            }
        }
        [centre[0], bounds.max[1] + 0.02, centre[2]]
    }

    pub fn bounds(&self) -> Bounds {
        Bounds::of(
            self.triangles
                .iter()
                .flat_map(|t| t.vertices.iter().map(|v| v.pos))
                .chain(self.collision.iter().flatten().copied())
                .chain(self.added_collision.iter().flatten().copied()),
        )
    }

    /// The triangles a body collides with: the given ones, or the drawn ones, and the added
    /// ones. Without the ones that have no area, with the face's own normal
    pub fn collision_triangles(&self) -> Vec<[GeVertex; 3]> {
        let mut source: Vec<[[f32; 3]; 3]> = if self.collision.is_empty() {
            self.triangles
                .iter()
                .filter(|t| !t.no_collision)
                .map(|t| [t.vertices[0].pos, t.vertices[1].pos, t.vertices[2].pos])
                .collect()
        } else {
            self.collision.clone()
        };
        source.extend(self.added_collision.iter().copied());

        source
            .into_iter()
            .filter_map(|t| {
                let u = [t[1][0] - t[0][0], t[1][1] - t[0][1], t[1][2] - t[0][2]];
                let v = [t[2][0] - t[0][0], t[2][1] - t[0][1], t[2][2] - t[0][2]];
                let n = [
                    u[1] * v[2] - u[2] * v[1],
                    u[2] * v[0] - u[0] * v[2],
                    u[0] * v[1] - u[1] * v[0],
                ];
                let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
                if !len.is_finite() || len < 1e-7 {
                    return None;
                }
                let normal = [n[0] / len, n[1] / len, n[2] / len];
                Some(t.map(|pos| GeVertex {
                    pos,
                    normal,
                    uv: [0.0, 0.0],
                    color: [COLOUR_ONE as u8, COLOUR_ONE as u8, COLOUR_ONE as u8, 0xFF],
                }))
            })
            .collect()
    }
}

fn centre_of(t: &[GeVertex; 3]) -> [f32; 3] {
    [
        (t[0].pos[0] + t[1].pos[0] + t[2].pos[0]) / 3.0,
        (t[0].pos[1] + t[1].pos[1] + t[2].pos[1]) / 3.0,
        (t[0].pos[2] + t[1].pos[2] + t[2].pos[2]) / 3.0,
    ]
}

/// One entity of the file
enum Entity {
    /// Everything that is drawn: a group of meshes that is placed as one thing, the way the
    /// game's own test level is made. The game draws what the camera sees of a group by its
    /// tree (meshes placed one by one get left out of the picture) and a body collides with the
    /// triangles that aren't flagged
    Drawn(Vec<MeshData>),
    /// A part of a collision that isn't drawn, placed on its own: a mesh nothing is seen of,
    /// with the triangles as its collision variants
    Hull(MeshData),
}

/// The sky's entity: a group like the drawn one, of triangles nothing collides with. It isn't
/// placed, the map lists it as its one sky and every zone's settings name it
fn sky_meshes(scene: &GeScene) -> Vec<MeshData> {
    let triangles: Vec<Drawn> = scene
        .sky
        .iter()
        .filter(|t| has_area(&t.vertices))
        .map(|t| (t.texture.map(|i| i as u16 + 1).unwrap_or(0), t.vertices, FACE_NO_COLLISION, t.two_sided, None))
        .collect();
    drawn_meshes(triangles)
}

/// The mesh a hull is drawn as: two triangles without area, in the corners of its box. Nothing
/// is seen of them, and the entity has the bounds of what it is collided with
fn unseen_mesh(hull: &MeshData) -> MeshData {
    let bounds = Bounds::of(hull.positions()).or_zero();
    let corner = |pos: [f32; 3]| {
        [GeVertex {
            pos,
            normal: [0.0, 1.0, 0.0],
            uv: [0.0, 0.0],
            color: [0, 0, 0, 0],
        }; 3]
    };
    MeshData {
        parts: vec![MeshPart {
            texture: 0,
            triangles: vec![corner(bounds.min), corner(bounds.max)],
            flags: vec![FACE_NO_COLLISION; 2],
            two_sided: false,
            lightmap: false,
        }],
    }
}

/// Halves the triangles along their longest side until every part has few enough
fn cut<T>(triangles: Vec<T>, max: usize, centre: &impl Fn(&T) -> [f32; 3], parts: &mut Vec<Vec<T>>) {
    if triangles.len() <= max.max(1) {
        if !triangles.is_empty() {
            parts.push(triangles);
        }
        return;
    }
    let (lower, upper) = halve(triangles, centre);
    cut(lower, max, centre, parts);
    cut(upper, max, centre, parts);
}

fn has_area(t: &[GeVertex; 3]) -> bool {
    let u = [t[1].pos[0] - t[0].pos[0], t[1].pos[1] - t[0].pos[1], t[1].pos[2] - t[0].pos[2]];
    let v = [t[2].pos[0] - t[0].pos[0], t[2].pos[1] - t[0].pos[1], t[2].pos[2] - t[0].pos[2]];
    let n = [
        u[1] * v[2] - u[2] * v[1],
        u[2] * v[0] - u[0] * v[2],
        u[0] * v[1] - u[1] * v[0],
    ];
    let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
    len.is_finite() && len >= 1e-7
}

/// An entity with the zone it is placed in
struct Placed {
    entity: Entity,
    zone: u16,
    bounds: Bounds,
}

/// A drawn triangle: its texture in the file, its corners, its flags, whether it is drawn from
/// both sides and its lightmap (a texture in the file, where the corners are on it)
type Drawn = (u16, [GeVertex; 3], u16, bool, Option<(u16, [[f32; 2]; 3])>);

fn part_for(parts: &mut Vec<MeshPart>, texture: u16, two_sided: bool, lightmap: bool) -> &mut MeshPart {
    let found = parts.iter().position(|p| {
        p.texture == texture
            && p.two_sided == two_sided
            && p.lightmap == lightmap
            && p.triangles.len() < MAX_STRIP_TRIANGLES
    });
    match found {
        Some(i) => &mut parts[i],
        None => {
            parts.push(MeshPart {
                texture,
                two_sided,
                lightmap,
                ..Default::default()
            });
            parts.last_mut().unwrap()
        }
    }
}

/// The meshes of a group: the triangles cut into parts that are small boxes, each part's
/// triangles sorted by texture. A triangle's lightmap stays in its mesh, as one more strip
/// behind the others: the same corners with the lightmap's texture, as the game's own
/// lightmapped meshes have them
fn drawn_meshes(triangles: Vec<Drawn>) -> Vec<MeshData> {
    let mut groups = vec![];
    cut(triangles, MAX_DRAWN_TRIANGLES, &|t| centre_of(&t.1), &mut groups);
    groups
        .into_iter()
        .map(|group| {
            let mut parts: Vec<MeshPart> = vec![];
            for (texture, triangle, flags, two_sided, lightmap) in group {
                let part = part_for(&mut parts, texture, two_sided, false);
                part.triangles.push(triangle);
                part.flags.push(flags);
                if let Some((texture, uvs)) = lightmap {
                    let mut lit = triangle;
                    for (v, uv) in lit.iter_mut().zip(uvs) {
                        v.uv = uv;
                        v.color = [COLOUR_ONE as u8, COLOUR_ONE as u8, COLOUR_ONE as u8, 0xFF];
                    }
                    let part = part_for(&mut parts, texture, two_sided, true);
                    part.triangles.push(lit);
                    part.flags.push(FACE_NO_COLLISION);
                }
            }
            parts.sort_by_key(|p| (p.lightmap, p.texture));
            MeshData { parts }
        })
        .collect()
}

/// The parts of a collision that isn't drawn
fn hulls(collision: Vec<[GeVertex; 3]>) -> Vec<MeshData> {
    let mut groups = vec![];
    cut(collision, MAX_COLLISION_TRIANGLES, &centre_of, &mut groups);
    groups
        .into_iter()
        .map(|group| MeshData {
            parts: vec![MeshPart {
                texture: 0,
                triangles: group,
                flags: vec![],
                two_sided: false,
                lightmap: false,
            }],
        })
        .collect()
}

fn make_entities(scene: &GeScene, zoning: Option<&Zoning>, stats: &mut BuildStats) -> Vec<Placed> {
    let mut entities = vec![];
    // with a collision of its own the scene's drawn triangles aren't collided with at all, and
    // with rooms the collision is always parts of its own
    // (added collision makes it one too: the drawn triangles and the added ones together)
    let own_collision = !scene.collision.is_empty() || !scene.added_collision.is_empty();
    let drawn_collides = !own_collision && zoning.is_none();

    // texture 0 of the file is the default one, the scene's follow. the flags: what a body
    // collides with of the drawn triangles is told triangle by triangle
    let drawn: Vec<Drawn> = scene
        .triangles
        .iter()
        .map(|t| {
            let collides = drawn_collides && !t.no_collision && has_area(&t.vertices);
            (
                t.texture.map(|i| i as u16 + 1).unwrap_or(0),
                t.vertices,
                if collides { 0 } else { FACE_NO_COLLISION },
                t.two_sided,
                t.lightmap.map(|(texture, uvs)| (texture as u16 + 1, uvs)),
            )
        })
        .collect();
    stats.drawn_triangles = drawn.len();
    let whole = Bounds::of(drawn.iter().flat_map(|t| positions(&t.1)));

    let Some(zoning) = zoning else {
        if drawn_collides {
            stats.collision_triangles = drawn.iter().filter(|t| t.2 == 0).count();
        }
        let meshes = drawn_meshes(drawn);
        stats.drawn_meshes = meshes.len();
        if drawn_collides {
            stats.collision_meshes = meshes
                .iter()
                .filter(|m| m.parts.iter().any(|p| p.flags.iter().any(|f| *f == 0)))
                .count();
        }
        if !meshes.is_empty() {
            entities.push(Placed {
                entity: Entity::Drawn(meshes),
                zone: 0,
                bounds: whole,
            });
        }

        if own_collision {
            let collision = scene.collision_triangles();
            stats.collision_triangles = collision.len();
            for hull in hulls(collision) {
                stats.collision_meshes += 1;
                entities.push(Placed {
                    bounds: Bounds::of(hull.positions()),
                    entity: Entity::Hull(hull),
                    zone: 0,
                });
            }
        }
        return entities;
    };

    // a group for each room
    let mut by_zone: Vec<Vec<Drawn>> = vec![vec![]; zoning.zones.len()];
    for (triangle, zone) in drawn.into_iter().zip(&zoning.triangle_zones) {
        by_zone[*zone as usize].push(triangle);
    }
    for (zone, triangles) in by_zone.into_iter().enumerate() {
        let bounds = Bounds::of(triangles.iter().flat_map(|t| positions(&t.1)));
        let meshes = drawn_meshes(triangles);
        stats.drawn_meshes += meshes.len();
        if !meshes.is_empty() {
            entities.push(Placed {
                entity: Entity::Drawn(meshes),
                zone: zone as u16,
                bounds,
            });
        }
    }

    // the collision's parts, each in the zone the game finds in front of its triangles
    let collision = scene.collision_triangles();
    stats.collision_triangles = collision.len();
    let mut by_zone: Vec<Vec<[GeVertex; 3]>> = vec![vec![]; zoning.zones.len()];
    for triangle in collision {
        let centre = centre_of(&triangle);
        let normal = triangle[0].normal;
        let front = [0, 1, 2].map(|k| centre[k] + normal[k] * 0.05);
        by_zone[zoning.zone_at(front) as usize].push(triangle);
    }
    for (zone, triangles) in by_zone.into_iter().enumerate() {
        for hull in hulls(triangles) {
            stats.collision_meshes += 1;
            entities.push(Placed {
                bounds: Bounds::of(hull.positions()),
                entity: Entity::Hull(hull),
                zone: zone as u16,
            });
        }
    }

    entities
}

/// Which placements a zone lists: its own, and the collision parts of other zones that reach
/// into its box (the wall and the floor of the next room, for a body in the doorway)
fn zone_lists(entities: &[Placed], zoning: Option<&Zoning>) -> Vec<Vec<usize>> {
    let Some(zoning) = zoning else {
        return vec![(0..entities.len()).collect()];
    };
    let mut boxes: Vec<Bounds> = zoning.zones.iter().map(|z| z.bounds).collect();
    for placed in entities {
        boxes[placed.zone as usize].merge(&placed.bounds);
    }
    let mut lists: Vec<Vec<usize>> = boxes
        .iter()
        .enumerate()
        .map(|(zone, zone_box)| {
            (0..entities.len())
                .filter(|i| {
                    let placed = &entities[*i];
                    if placed.zone as usize == zone {
                        return true;
                    }
                    matches!(placed.entity, Entity::Hull(_))
                        && !zone_box.is_empty()
                        && !placed.bounds.is_empty()
                        && (0..3).all(|k| {
                            placed.bounds.min[k] <= zone_box.max[k] + ZONE_REACH
                                && placed.bounds.max[k] >= zone_box.min[k] - ZONE_REACH
                        })
                })
                .collect()
        })
        .collect();

    // a body is tested against what the zone it is in lists, and that zone is the one the tree
    // gives its place: every part is listed as well by each zone the tree gives the places a
    // body is at when it stands on it or against it, whatever the boxes say
    let mut added = 0;
    for (i, placed) in entities.iter().enumerate() {
        let Entity::Hull(hull) = &placed.entity else {
            continue;
        };
        for t in hull.parts.iter().flat_map(|p| p.triangles.iter()) {
            let centre = centre_of(t);
            let normal = t[0].normal;
            // places all over it, no further apart than a body is wide
            let side = |a: usize, b: usize| {
                (0..3).map(|k| (t[a].pos[k] - t[b].pos[k]).powi(2)).sum::<f32>().sqrt()
            };
            let longest = side(0, 1).max(side(1, 2)).max(side(2, 0));
            let steps = ((longest / BODY_STEP).ceil() as usize).clamp(1, 64);
            let places = (0..=steps).flat_map(|a| {
                (0..=steps - a).map(move |b| {
                    // kept a little inside the triangle
                    let (u, v) = (a as f32 / steps as f32, b as f32 / steps as f32);
                    let on = [0, 1, 2].map(|k| {
                        t[0].pos[k] + (t[1].pos[k] - t[0].pos[k]) * u + (t[2].pos[k] - t[0].pos[k]) * v
                    });
                    [0, 1, 2].map(|k| on[k] + (centre[k] - on[k]) * 0.02)
                })
            });
            for place in places {
                for ahead in BODY_REACHES {
                    let zone = zoning.zone_at([0, 1, 2].map(|k| place[k] + normal[k] * ahead)) as usize;
                    if zone < lists.len() && !lists[zone].contains(&i) {
                        lists[zone].push(i);
                        added += 1;
                    }
                }
            }
        }
    }
    if added > 0 {
        tracing::info!("{added} collision part(s) listed by a zone they aren't in: its tree puts a body there");
    }
    lists
}

/// A zone's placement tree: what a body's surroundings are looked up in
enum PlacementNode {
    Leaf(Vec<usize>),
    Node(Vec<PlacementNode>),
}

/// The four parts (or fewer) a list of placements is cut into, each a box of its own
fn quarters(list: Vec<usize>, entities: &[Placed]) -> Vec<Vec<usize>> {
    let centre = |i: &usize| entities[*i].bounds.or_zero().center();
    let (lower, upper) = halve(list, &centre);
    let mut parts = vec![];
    for half in [lower, upper] {
        if half.len() > 1 {
            let (a, b) = halve(half, &centre);
            parts.push(a);
            parts.push(b);
        } else {
            parts.push(half);
        }
    }
    parts.retain(|p| !p.is_empty());
    parts
}

/// The tree of a zone's list. A long list is cut into small boxes the way the game's own levels
/// are, so a body is handed what is near it and not the whole zone: three levels of nodes with
/// up to four children each, then the leaves
fn placement_tree(list: Vec<usize>, entities: &[Placed]) -> PlacementNode {
    fn level(list: Vec<usize>, entities: &[Placed], levels: usize) -> PlacementNode {
        if levels == 0 {
            return PlacementNode::Leaf(list);
        }
        PlacementNode::Node(
            quarters(list, entities)
                .into_iter()
                .map(|part| level(part, entities, levels - 1))
                .collect(),
        )
    }
    if list.len() <= PLACEMENT_LEAF {
        return PlacementNode::Node(vec![PlacementNode::Leaf(list)]);
    }
    level(list, entities, PLACEMENT_TREE_LEVELS)
}

/// The flags of a placement in a zone's tree: bit 8 is on what a body is tested against. A
/// drawn group none of whose triangles are collided with (the collision is hulls) mustn't have
/// it: a body that is handed the group is tested against nothing that comes after it
fn placement_flags(placed: &Placed) -> u8 {
    let collides = match &placed.entity {
        Entity::Drawn(meshes) => meshes
            .iter()
            .flat_map(|m| m.parts.iter())
            .any(|p| p.flags.len() < p.triangles.len() || p.flags.iter().any(|f| *f != FACE_NO_COLLISION)),
        Entity::Hull(_) => true,
    };
    if collides {
        PLACEMENT_COLLIDES as u8
    } else {
        PLACEMENT_LISTED
    }
}

/// A node's flags: all that its entries have
fn node_flags(node: &PlacementNode, entities: &[Placed]) -> u8 {
    match node {
        PlacementNode::Leaf(list) => list.iter().fold(PLACEMENT_LISTED, |f, i| f | placement_flags(&entities[*i])),
        PlacementNode::Node(children) => children.iter().fold(PLACEMENT_LISTED, |f, c| f | node_flags(c, entities)),
    }
}

fn placement_box(node: &PlacementNode, entities: &[Placed]) -> Bounds {
    let mut bounds = Bounds::EMPTY;
    match node {
        PlacementNode::Leaf(list) => {
            for i in list {
                bounds.merge(&entities[*i].bounds);
            }
        }
        PlacementNode::Node(children) => {
            for child in children {
                bounds.merge(&placement_box(child, entities));
            }
        }
    }
    bounds
}

/// A node: its number of children, flags, then a pointer and a box (0x1C bytes) for each. A
/// leaf: 0, flags, the number of entries, then a placement's index and its flags for each.
/// `whole` is the box of a tree that is one leaf, as it was before lists were cut up
fn write_placement_node(w: &mut Writer, node: &PlacementNode, entities: &[Placed], whole: &Bounds) {
    let flags = node_flags(node, entities);
    match node {
        PlacementNode::Leaf(list) => {
            w.u8(0);
            w.u8(flags);
            w.u16(list.len() as u16);
            for i in list {
                w.u16(*i as u16);
                w.u8(placement_flags(&entities[*i]));
                w.u8(0);
            }
            w.u32(0);
            w.align(4);
        }
        PlacementNode::Node(children) => {
            w.u8(children.len() as u8);
            w.u8(flags);
            w.u16(0);
            let one_leaf = children.len() == 1 && matches!(children[0], PlacementNode::Leaf(_));
            let pointers: Vec<usize> = children
                .iter()
                .map(|child| {
                    let pointer = w.rel();
                    let bounds = if one_leaf {
                        *whole
                    } else {
                        let b = placement_box(child, entities).or_zero();
                        Bounds {
                            min: b.min.map(|v| v - PLACEMENT_BOX_MARGIN),
                            max: b.max.map(|v| v + PLACEMENT_BOX_MARGIN),
                        }
                    };
                    w.f32s(&bounds.min);
                    w.f32s(&bounds.max);
                    pointer
                })
                .collect();
            for (child, pointer) in children.iter().zip(pointers) {
                w.point_here(pointer);
                write_placement_node(w, child, entities, whole);
            }
        }
    }
}

/// The edges and ladders as an entity's variant 6: the variant word at the entity's +0x40 (a bit
/// for each variant, the offset to a word for each), that word (the offset to the variant, its
/// number), and the variant: the number of chains and a pointer to them, the same for polygons,
/// the number of 16 byte records in each. A chain is a record with the number of edges and the
/// first point, then a record for each edge: which of its ends nothing joins (1 the start, 2
/// the end), its flags, and the point it ends at. A polygon is a record with its number of
/// corners, a bit for each side nothing joins, its flags and the way a ladder's top runs, then a
/// record for each corner: the point and two shorts
fn write_edges(w: &mut Writer, entity: usize, edges: &[SceneEdge], ladders: &[SceneLadder]) {
    const VARIANT_EDGES: u32 = 6;
    let polygons: usize = ladders.iter().map(|l| l.rungs().len()).sum();
    let chains = edges.len() + ladders.len();
    let variants = entity + 0x40;
    let word = w.pos();
    w.set_u32(variants, ((word - variants) as u32) << 8 | 1 << VARIANT_EDGES);
    w.u32(4 << 8 | VARIANT_EDGES);
    w.u32(chains as u32);
    let p_chains = w.rel();
    w.u32(polygons as u32);
    let p_polygons = w.rel();
    w.u16(chains as u16 * 2);
    w.u16(polygons as u16 * 5);
    w.zeros(12);
    w.point_here(p_polygons);
    for ladder in ladders {
        let along = ladder.along();
        for (i, corners) in ladder.rungs().iter().enumerate() {
            w.u8(4);
            // the two sides are open, and the lowest piece's bottom
            w.u8(if i == 0 { 0x0D } else { 0x05 });
            w.u16(POLYGON_RUNG);
            w.f32s(&along);
            for (corner, (u, v)) in corners.iter().zip([(0, 0), (0x3F, 0), (0x3F, 0x3F), (0, 0x3F)]) {
                w.f32s(corner);
                w.u16(u);
                w.u16(v);
            }
        }
    }
    w.point_here(p_chains);
    let tops = ladders.iter().map(|l| SceneEdge { from: l.top[0], to: l.top[1], flags: EDGE_LADDER_TOP });
    for edge in edges.iter().cloned().chain(tops) {
        // the game's own chains have the first edge's values in the first record as well
        w.u8(1);
        w.u8(3);
        w.u16(edge.flags);
        w.f32s(&edge.from);
        w.u8(0);
        w.u8(3);
        w.u16(edge.flags);
        w.f32s(&edge.to);
    }
    w.align(4);
}

/// An array of nothing: no count, and a pointer that is set to the zeros at the map's end
fn empty_array(w: &mut Writer, p_end: &mut Vec<usize>) {
    w.u32(0);
    p_end.push(w.rel());
}

/// A count, the number of hashes that aren't local, and room for the pointer to the list
fn list_head(w: &mut Writer, count: usize, hashes: i16) -> usize {
    w.i16(count as i16);
    w.i16(hashes);
    w.rel()
}

/// A level in one file: the scene with its edges and ladders
pub fn build_geometry_file(scene: &GeScene, file_hash: u32, time: u32) -> (Vec<u8>, BuildStats) {
    let (ladders, warnings) = scene.fitted_ladders();
    let (file, mut stats) = build_map_file(scene, file_hash, HC_MAP, time, &scene.edges, &ladders);
    stats.warnings.extend(warnings);
    (file, stats)
}

/// The scene without its edges and ladders: the game doesn't ask a file that a level loads for
/// them, they go into the level's own (`build_empty_file`)
pub fn build_shared_geometry_file(scene: &GeScene, file_hash: u32, time: u32) -> (Vec<u8>, BuildStats) {
    build_map_file(scene, file_hash, HC_MAP, time, &[], &[])
}

/// A file with nothing in its map but the edges and ladders (`GeScene::fitted_ladders`): what a
/// level that loads its geometry from another file is before it gets its triggers
pub fn build_empty_file(file_hash: u32, time: u32, edges: &[SceneEdge], ladders: &[SceneLadder]) -> Vec<u8> {
    build_map_file(&GeScene::default(), file_hash, HC_TRIGGER_MAP, time, edges, ladders).0
}

fn build_map_file(
    scene: &GeScene,
    file_hash: u32,
    map_hash: u32,
    time: u32,
    edges: &[SceneEdge],
    ladders: &[SceneLadder],
) -> (Vec<u8>, BuildStats) {
    let mut stats = BuildStats::default();
    let zoning = make_zoning(scene);
    let zoning = zoning.as_ref();
    let entities = make_entities(scene, zoning, &mut stats);
    let lists = zone_lists(&entities, zoning);
    let zone_count = lists.len();
    stats.zones = zone_count;
    // the sky is the entity behind the placed ones
    let sky = sky_meshes(scene);
    stats.sky_triangles = sky.iter().flat_map(|m| m.parts.iter()).map(|p| p.triangles.len()).sum();
    let entity_count = entities.len() + !sky.is_empty() as usize;
    if let Some(zoning) = zoning {
        stats.portals = zoning.portals.len();
        stats.warnings = zoning.warnings.clone();
    }
    // each zone's links to its portals, by the zone behind: the other zone, the portal, and
    // whether the portal's front is the other zone's side
    let mut links: Vec<Vec<(u16, usize, bool)>> = vec![vec![]; zone_count];
    if let Some(zoning) = zoning {
        for (i, portal) in zoning.portals.iter().enumerate() {
            links[portal.zones[0] as usize].push((portal.zones[1], i, false));
            links[portal.zones[1] as usize].push((portal.zones[0], i, true));
        }
        for zone in &mut links {
            zone.sort_by_key(|l| (l.0, l.1));
        }
    }
    let mut textures = vec![GeTexture::solid("default", [0x80, 0x80, 0x80, 0xFF])];
    textures.extend(scene.textures.iter().cloned());
    stats.textures = scene.textures.len();
    let bounds = scene.bounds();
    stats.bounds = (!bounds.is_empty()).then_some(bounds);
    let bounds = bounds.or_zero();

    let mut w = Writer::new();
    w.u32(EDB_MAGIC);
    w.u32(file_hash);
    w.u32(EDB_VERSION);
    w.u32(FILE_FLAGS);
    w.u32(time);
    w.zeros(0x40 - 0x14); // the sizes, filled in at the end

    let p_sections = list_head(&mut w, 1, 1);
    let p_refptrs = list_head(&mut w, 2 * zone_count, 2 * zone_count as i16);
    let p_entities = list_head(&mut w, entity_count, 0);
    let mut p_empty = vec![];
    for _ in 0..3 {
        p_empty.push(list_head(&mut w, 0, 0)); // anims, skins, scripts
    }
    let p_maps = list_head(&mut w, 1, 1);
    for _ in 0..7 {
        p_empty.push(list_head(&mut w, 0, 0)); // anim modes and sets, particles, swooshes, spreadsheets, fonts, force feedback
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

    let mut debug_index = 0u32;
    let mut next_debug = || {
        debug_index += 1;
        debug_index + 1
    };

    w.point_here(p_maps);
    w.u32(map_hash);
    w.u32(0);
    let a_map = w.pos();
    w.u32(0);
    w.u32(0);

    for p in p_empty {
        w.point_here(p);
    }
    w.point_here(p_entities);
    let mut a_entities = vec![];
    for i in 0..entity_count {
        w.u32(HC_LOCAL_ENTITY + i as u32);
        w.u32(next_debug());
        a_entities.push(w.pos());
        w.u32(0);
        w.u32(0);
        w.u32(0);
    }

    w.point_here(p_materials);
    let mut a_materials = vec![];
    for i in 0..textures.len() {
        w.u32(if i == 0 {
            HC_DEFAULT_TEXTURE
        } else {
            HC_LOCAL_MATERIAL + i as u32
        });
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
        w.u32(if i == 0 {
            HC_DEFAULT_TEXTURE
        } else {
            HC_LOCAL_TEXTURE + i as u32
        });
        w.u32(next_debug());
        a_textures.push(w.pos());
        w.u32(0);
        w.u32(0);
        w.u16(width as u16);
        w.u16(height as u16);
        w.u32(0); // game flags
        w.u32(TEXTURE_FLAGS);
    }

    w.point_here(p_sections);
    w.point_here(p_rel0);
    w.u32(HC_SECTION);
    let a_section = w.pos();
    w.u32(0); // where it starts
    w.u32(0); // and ends
    w.u32(0);

    w.point_here(p_refptrs);
    let mut a_refptrs = vec![];
    for _ in 0..2 * zone_count {
        w.u32(0);
        w.u32(0);
        a_refptrs.push(w.pos());
        w.u32(0);
        w.u32(0);
    }

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
    w.align(16);

    // the map
    let map = w.pos();
    w.set_u32(a_map, map as u32);
    w.u32(0x500);
    let p_bsp = w.rel();
    let mut p_end = vec![];
    empty_array(&mut w, &mut p_end); // paths
    empty_array(&mut w, &mut p_end); // lights
    empty_array(&mut w, &mut p_end); // cameras
    empty_array(&mut w, &mut p_end); // sounds by index
    empty_array(&mut w, &mut p_end);
    empty_array(&mut w, &mut p_end); // sounds
    let portal_count = zoning.map(|z| z.portals.len()).unwrap_or(0);
    let p_portals = if portal_count == 0 {
        empty_array(&mut w, &mut p_end);
        None
    } else {
        w.u32(portal_count as u32);
        Some(w.rel())
    };
    let p_skies = if sky.is_empty() {
        empty_array(&mut w, &mut p_end);
        None
    } else {
        w.u32(1);
        Some(w.rel())
    };
    w.u32(entities.len() as u32);
    let p_placements = w.rel();
    empty_array(&mut w, &mut p_end); // placement groups
    let p_triggers = w.rel();
    w.u32(1);
    w.zeros(12);
    w.f32s(&bounds.min);
    w.f32s(&bounds.max);
    w.u32(zone_count as u32);
    debug_assert_eq!(w.pos() - map, 0x88);

    // the zones, 0x8C bytes each
    struct ZonePointers {
        identifier: usize,
        links: Option<usize>,
        placement_info: usize,
        at_2c: usize,
    }
    let mut zone_pointers = vec![];
    for zone in 0..zone_count {
        let start = w.pos();
        w.u32(zone as u32 * 2 + 1); // its entity: a reference pointer
        let identifier = w.rel();
        empty_array(&mut w, &mut p_end); // lights
        empty_array(&mut w, &mut p_end); // sounds
        empty_array(&mut w, &mut p_end);
        let links = if links[zone].is_empty() {
            empty_array(&mut w, &mut p_end);
            None
        } else {
            w.u32(links[zone].len() as u32);
            Some(w.rel())
        };
        let placement_info = w.rel();
        let at_2c = w.rel();
        w.u32(0xFFFFFFFF);
        w.u32(0); // section
        w.u32(1);
        // 0x48: a bit for each zone that is never seen from this one, none
        w.zeros(0x68 - 0x3C);
        match zoning {
            Some(zoning) => {
                let zone_box = zoning.zones[zone].bounds.or_zero();
                w.f32s(&zone_box.min);
                w.f32s(&zone_box.max);
            }
            None => w.zeros(0x18),
        }
        w.zeros(8);
        p_end.push(w.rel());
        debug_assert_eq!(w.pos() - start, 0x8C);
        zone_pointers.push(ZonePointers {
            identifier,
            links,
            placement_info,
            at_2c,
        });
    }
    w.align(16);

    // the tree that says which zone a point is in. without rooms: one node, everything is zone 0
    w.point_here(p_bsp);
    match zoning {
        Some(zoning) => {
            for node in &zoning.tree {
                w.f32s(&node.plane);
                w.i16(node.children[0]);
                w.i16(node.children[1]);
                w.zeros(12);
            }
        }
        None => {
            w.f32s(&[1.0, 0.0, 0.0, 0.0]);
            w.zeros(16);
        }
    }

    for pointers in &zone_pointers {
        w.point_here(pointers.identifier);
        for (i, v) in ZONE_IDENTIFIER.into_iter().enumerate() {
            w.u32(if i == ZONE_SKY_INDEX && !sky.is_empty() { 0 } else { v });
        }
    }

    // the skies: an entity's hash each (the game's own levels name scripts of entities too)
    if let Some(p_skies) = p_skies {
        w.point_here(p_skies);
        w.u32(HC_LOCAL_ENTITY + entities.len() as u32);
    }

    // no triggers: the four tables all point at the zeros behind
    w.point_here(p_triggers);
    w.u32(0);
    let trigger_tables: Vec<usize> = (0..4).map(|_| w.rel()).collect();
    for p in trigger_tables {
        w.point_here(p);
    }
    w.zeros(12);

    if let (Some(p_portals), Some(zoning)) = (p_portals, zoning) {
        w.point_here(p_portals);
        for portal in &zoning.portals {
            w.u16(portal.zones[0]);
            w.u16(portal.zones[1]);
            w.u32(0); // flags (1: not looked through)
            w.u32(0);
            w.f32(0.0); // from how far it is looked through, 0: any
            w.u32(0); // a face that is drawn in it
            for corner in &portal.corners {
                w.f32s(corner);
            }
        }
        for (zone, pointers) in zone_pointers.iter().enumerate() {
            let Some(p_links) = pointers.links else {
                continue;
            };
            w.point_here(p_links);
            for (i, (to, portal, flipped)) in links[zone].iter().enumerate() {
                let more = links[zone][i + 1..].iter().take_while(|l| l.0 == *to).count();
                w.u16(zone as u16);
                w.u16(*to);
                w.i16(*portal as i16);
                w.u8(more.min(255) as u8);
                w.u8(*flipped as u8);
            }
        }
        w.align(4);
    }

    // which placements each zone has, and the tree a body's surroundings are looked up in
    for (list, pointers) in lists.iter().zip(&zone_pointers) {
        w.point_here(pointers.placement_info);
        w.u32(list.len() as u32);
        let p_zone_placements = w.rel();
        let p_tree = w.rel();
        w.point_here(p_zone_placements);
        for i in list {
            w.u16(*i as u16);
        }
        w.zeros(4);
        w.align(16);
        w.point_here(p_tree);
        let tree = placement_tree(list.clone(), &entities);
        write_placement_node(&mut w, &tree, &entities, &bounds);
    }

    w.point_here(p_placements);
    for (i, placed) in entities.iter().enumerate() {
        w.u32(0xFFFFFFFF); // no hash
        w.f32s(&[0.0, 0.0, 0.0]);
        w.u32(0);
        w.f32s(&[0.0, -0.0, -0.0]);
        w.f32s(&[1.0, 1.0, 1.0]);
        w.u16(PLACEMENT_COLLIDES);
        w.u16(placed.zone); // the zone it is in
        w.u32(HC_LOCAL_ENTITY + i as u32);
        w.u16(0); // light set
        w.i16(-1); // group
        w.u32(0);
    }

    for pointers in &zone_pointers {
        w.point_here(pointers.at_2c);
        w.u32(0);
        let p_a = w.rel();
        let p_b = w.rel();
        w.point_here(p_a);
        w.point_here(p_b);
        w.zeros(0x1C);
    }

    w.align(32);
    for p in p_end {
        w.point_here(p);
    }

    for (placed, at) in entities.iter().zip(&a_entities) {
        w.align(32);
        w.set_u32(*at, w.pos() as u32);
        match &placed.entity {
            Entity::Drawn(meshes) => write_split(&mut w, meshes),
            Entity::Hull(hull) => write_mesh_with_collision(&mut w, &unseen_mesh(hull), hull, None),
        };
    }
    if !sky.is_empty() {
        w.align(32);
        w.set_u32(a_entities[entities.len()], w.pos() as u32);
        write_split(&mut w, &sky);
    }

    // an edge is its zone's: the one in front of it, where the player stands, and the one behind
    // it when that is another
    let mut edges_by_zone: Vec<Vec<SceneEdge>> = vec![vec![]; zone_count];
    for edge in edges {
        let Some(zoning) = zoning else {
            edges_by_zone[0].push(edge.clone());
            continue;
        };
        let (dx, dz) = (edge.to[0] - edge.from[0], edge.to[2] - edge.from[2]);
        let len = (dx * dx + dz * dz).sqrt().max(1e-6);
        let middle = [0, 1, 2].map(|k| (edge.from[k] + edge.to[k]) * 0.5);
        let beside = |way: f32| [middle[0] + dz / len * way, middle[1], middle[2] - dx / len * way];
        let front = zoning.zone_at(beside(-EDGE_ZONE_REACH)) as usize;
        let behind = zoning.zone_at(beside(EDGE_ZONE_REACH)) as usize;
        edges_by_zone[front.min(zone_count - 1)].push(edge.clone());
        if behind != front {
            edges_by_zone[behind.min(zone_count - 1)].push(edge.clone());
        }
    }
    // a ladder is the zones' the player is in on it: in front of its foot, its middle and its
    // top, and behind the top where the climb ends
    let mut ladders_by_zone: Vec<Vec<SceneLadder>> = vec![vec![]; zone_count];
    stats.ladders = ladders.len();
    for ladder in ladders {
        let Some(zoning) = zoning else {
            ladders_by_zone[0].push(ladder.clone());
            continue;
        };
        let front = ladder.front();
        let at = |f: f32, way: f32, up: f32| {
            let p = [0, 1, 2].map(|k| {
                let low = (ladder.bottom[0][k] + ladder.bottom[1][k]) * 0.5;
                let high = (ladder.top[0][k] + ladder.top[1][k]) * 0.5;
                low + (high - low) * f + front[k] * way
            });
            [p[0], p[1] + up, p[2]]
        };
        let mut zones: Vec<usize> = [at(0.0, EDGE_ZONE_REACH, 0.5), at(0.5, EDGE_ZONE_REACH, 0.0), at(1.0, EDGE_ZONE_REACH, 0.0), at(1.0, -EDGE_ZONE_REACH, 0.5)]
            .into_iter()
            .map(|p| (zoning.zone_at(p) as usize).min(zone_count - 1))
            .collect();
        zones.sort();
        zones.dedup();
        for zone in zones {
            ladders_by_zone[zone].push(ladder.clone());
        }
    }

    // each zone's own entity (0x608), which refers to an empty group (0x603) through the
    // reference pointer before its own: the level's triangles are all placed
    for zone in 0..zone_count {
        w.align(32);
        w.set_u32(a_refptrs[zone * 2 + 1], w.pos() as u32);
        let entity = w.pos();
        w.u32(0x608);
        w.zeros(0x50);
        w.u32(0x01000000);
        w.u32(zone as u32 * 2);
        w.zeros(0xC);
        w.f32(10000.0);
        // the game asks the entity of the zone the player is in for its edges
        if edges_by_zone[zone].is_empty() && ladders_by_zone[zone].is_empty() {
            w.zeros(0x14);
        } else {
            write_edges(&mut w, entity, &edges_by_zone[zone], &ladders_by_zone[zone]);
        }
        w.set_u32(a_refptrs[zone * 2], w.pos() as u32);
        w.u32(0x603);
        w.zeros(0x50);
        w.u32(0);
        w.u32(8);
        w.u32(0);
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
    w.set_u32(
        0x24,
        size + 216 + 36 * textures.len() as u32 + 16 * entity_count as u32 + 256 * (zone_count as u32 - 1),
    );
    w.set_u32(a_section + 4, size);
    w.set_u32(section_start, size);

    (w.buf, stats)
}

#[cfg(test)]
mod edge_tests {
    use super::*;

    /// A quad's rim: four edges, each with the quad on the side the player goes to
    #[test]
    fn rim_runs_around_the_top() {
        let (a, b, c, d) = ([0.0, 1.0, 0.0], [2.0, 1.0, 0.0], [2.0, 1.0, 1.0], [0.0, 1.0, 1.0]);
        // both ways round, the faces look up in one and down in the other
        let up = rim_edges(&[[a, c, b], [a, d, c]], EDGE_VAULT);
        assert_eq!(up.len(), 4);
        for edge in &up {
            let (dx, dz) = (edge.to[0] - edge.from[0], edge.to[2] - edge.from[2]);
            let middle = [(edge.from[0] + edge.to[0]) * 0.5, (edge.from[2] + edge.to[2]) * 0.5];
            let inwards = dz * (1.0 - middle[0]) - dx * (0.5 - middle[1]);
            assert!(inwards > 0.0, "{edge:?} is taken away from the quad");
        }
        assert!(rim_edges(&[[a, b, c], [a, c, d]], EDGE_VAULT).is_empty());
    }

    /// A quad on a wall at x 2 that looks along -x: the top runs so that the player gets off
    /// into the wall, the pieces are half a unit each and wound to look at the player
    #[test]
    fn ladder_faces_the_player() {
        let (a, b, c, d) = ([2.0, 0.0, 0.0], [2.0, 0.0, 0.4], [2.0, 3.0, 0.4], [2.0, 3.0, 0.0]);
        let ladder = ladder_of(&[[a, b, c], [a, c, d]]).unwrap();
        assert_eq!(ladder.front(), [-1.0, 0.0, 0.0]);
        let (dx, dz) = (ladder.top[1][0] - ladder.top[0][0], ladder.top[1][2] - ladder.top[0][2]);
        assert!(dz > 0.0 && dx.abs() < 1e-6, "the player gets off towards (dz, 0, -dx): +x");
        assert!((ladder.height() - 3.0).abs() < 1e-6 && (ladder.width() - 0.4).abs() < 1e-6);
        let rungs = ladder.rungs();
        assert_eq!(rungs.len(), 6);
        assert!((rungs[0][0][1], rungs[5][1][1]) == (0.0, 3.0));
        for r in &rungs {
            let u = [0, 1, 2].map(|k| r[1][k] - r[0][k]);
            let v = [0, 1, 2].map(|k| r[2][k] - r[0][k]);
            assert!(u[1] * v[2] - u[2] * v[1] < 0.0, "{r:?} looks away from the player");
        }
        // turned and leaning: the top still runs across the way the faces look, the foot is
        // further out than the top
        let (a, b, c, d) = ([1.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.2, 2.0, 1.2], [1.2, 2.0, 0.2]);
        let leaning = ladder_of(&[[a, b, c], [a, c, d]]).unwrap();
        let (front, along) = (leaning.front(), leaning.along());
        assert!(front[0] < -0.7 && front[2] < -0.7 && (front[0] * along[0] + front[2] * along[2]).abs() < 1e-6);
        let out = |p: [f32; 3]| p[0] * front[0] + p[2] * front[2];
        assert!(out(leaning.bottom[0]) > out(leaning.top[0]) + 0.2);
        for r in leaning.rungs() {
            let u = [0, 1, 2].map(|k| r[1][k] - r[0][k]);
            let v = [0, 1, 2].map(|k| r[2][k] - r[0][k]);
            let n = [u[1] * v[2] - u[2] * v[1], 0.0, u[0] * v[1] - u[1] * v[0]];
            assert!(n[0] * front[0] + n[2] * front[2] > 0.0, "{r:?} looks away from the player");
        }
        // lying flat: no ladder
        let (e, f, g) = ([0.0, 1.0, 0.0], [1.0, 1.0, 0.0], [1.0, 1.0, 1.0]);
        assert!(ladder_of(&[[e, g, f]]).is_none());
    }
}
