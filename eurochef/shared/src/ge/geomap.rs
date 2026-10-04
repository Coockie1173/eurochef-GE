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

use super::{
    mesh::{
        halve, write_mesh_with_collision, write_split, FACE_NO_COLLISION, Bounds, GeVertex, MeshData, MeshPart,
        COLOUR_ONE, MAX_STRIP_TRIANGLES,
    },
    texture::{write_texture, GeTexture},
    writer::Writer,
};

pub const EDB_MAGIC: u32 = 0x47454F4D;
pub const EDB_VERSION: u32 = 263;
/// Files with a map and entities
const FILE_FLAGS: u32 = 0x20000006;
const SECTION_FLAGS: u32 = 0x9007;
pub const HC_MAP: u32 = 0x0500000D;
const HC_SECTION: u32 = 0x08000000;
const HC_DEFAULT_TEXTURE: u32 = 0x06000000;
const HC_LOCAL_ENTITY: u32 = 0x82000000;
const HC_LOCAL_TEXTURE: u32 = 0x86000000;
const HC_LOCAL_MATERIAL: u32 = 0xA5000000;
const TEXTURE_FLAGS: u32 = 0x04000000;
const PLACEMENT_COLLIDES: u16 = 9;

/// Triangles a drawn mesh and a collision mesh hold at most. The game goes over all triangles of
/// a mesh whose bounds a body touches, so collision meshes are kept small
const MAX_DRAWN_TRIANGLES: usize = 600;
const MAX_COLLISION_TRIANGLES: usize = 256;

/// The zone's settings (fog, ambience, colours) as the game's test level has them
const ZONE_IDENTIFIER: [u32; 20] = [
    0, 0, 0x3F000000, 0x3F800000, 0, 0x3F800000, 0, 0, 0x00010000, 0, 0, 0x80808000, 0xFFFFFF00,
    0x8080FF00, 0xFFFFFFFF, 0, 0xFFFFFFFF, 0, 0, 0,
];

#[derive(Clone)]
pub struct SceneTriangle {
    /// Index into the scene's textures, None for the grey default
    pub texture: Option<usize>,
    pub vertices: [GeVertex; 3],
    /// Left out of the collision that is made from the drawn triangles
    pub no_collision: bool,
}

#[derive(Clone, Default)]
pub struct GeScene {
    pub textures: Vec<GeTexture>,
    pub triangles: Vec<SceneTriangle>,
    /// Triangles a body collides with. Empty: the drawn triangles are collided with
    pub collision: Vec<[[f32; 3]; 3]>,
    /// Where the scene says the player starts
    pub spawn: Option<[f32; 3]>,
}

#[derive(Debug, Clone, Default)]
pub struct BuildStats {
    pub drawn_meshes: usize,
    pub drawn_triangles: usize,
    pub collision_meshes: usize,
    pub collision_triangles: usize,
    pub textures: usize,
    pub bounds: Option<Bounds>,
}

impl GeScene {
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
                .chain(self.collision.iter().flatten().copied()),
        )
    }

    /// The triangles a body collides with: the given ones, or the drawn ones. Without the ones
    /// that have no area, with the face's own normal
    pub fn collision_triangles(&self) -> Vec<[GeVertex; 3]> {
        let source: Vec<[[f32; 3]; 3]> = if self.collision.is_empty() {
            self.triangles
                .iter()
                .filter(|t| !t.no_collision)
                .map(|t| [t.vertices[0].pos, t.vertices[1].pos, t.vertices[2].pos])
                .collect()
        } else {
            self.collision.clone()
        };

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

fn make_entities(scene: &GeScene, stats: &mut BuildStats) -> Vec<Entity> {
    let mut entities = vec![];
    // with a collision of its own the scene's drawn triangles aren't collided with at all
    let own_collision = !scene.collision.is_empty();

    // texture 0 of the file is the default one, the scene's follow. the flags: what a body
    // collides with of the drawn triangles is told triangle by triangle
    let drawn: Vec<(u16, [GeVertex; 3], u16)> = scene
        .triangles
        .iter()
        .map(|t| {
            let collides = !own_collision && !t.no_collision && has_area(&t.vertices);
            (
                t.texture.map(|i| i as u16 + 1).unwrap_or(0),
                t.vertices,
                if collides { 0 } else { FACE_NO_COLLISION },
            )
        })
        .collect();
    stats.drawn_triangles = drawn.len();
    if !own_collision {
        stats.collision_triangles = drawn.iter().filter(|t| t.2 == 0).count();
    }
    let mut groups = vec![];
    cut(drawn, MAX_DRAWN_TRIANGLES, &|t| centre_of(&t.1), &mut groups);
    let meshes: Vec<MeshData> = groups
        .into_iter()
        .map(|group| {
            let mut parts: Vec<MeshPart> = vec![];
            for (texture, triangle, flags) in group {
                let part = match parts
                    .iter()
                    .position(|p| p.texture == texture && p.triangles.len() < MAX_STRIP_TRIANGLES)
                {
                    Some(i) => &mut parts[i],
                    None => {
                        parts.push(MeshPart {
                            texture,
                            ..Default::default()
                        });
                        parts.last_mut().unwrap()
                    }
                };
                part.triangles.push(triangle);
                part.flags.push(flags);
            }
            parts.sort_by_key(|p| p.texture);
            MeshData { parts }
        })
        .collect();
    stats.drawn_meshes = meshes.len();
    if !own_collision {
        stats.collision_meshes = meshes
            .iter()
            .filter(|m| m.parts.iter().any(|p| p.flags.iter().any(|f| *f == 0)))
            .count();
    }
    if !meshes.is_empty() {
        entities.push(Entity::Drawn(meshes));
    }

    if own_collision {
        let collision = scene.collision_triangles();
        stats.collision_triangles = collision.len();
        let mut groups = vec![];
        cut(collision, MAX_COLLISION_TRIANGLES, &centre_of, &mut groups);
        stats.collision_meshes = groups.len();
        for group in groups {
            entities.push(Entity::Hull(MeshData {
                parts: vec![MeshPart {
                    texture: 0,
                    triangles: group,
                    flags: vec![],
                }],
            }));
        }
    }

    entities
}

/// A count, the number of hashes that aren't local, and room for the pointer to the list
fn list_head(w: &mut Writer, count: usize, hashes: i16) -> usize {
    w.i16(count as i16);
    w.i16(hashes);
    w.rel()
}

pub fn build_geometry_file(scene: &GeScene, file_hash: u32, time: u32) -> (Vec<u8>, BuildStats) {
    let mut stats = BuildStats::default();
    let entities = make_entities(scene, &mut stats);
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
    let p_refptrs = list_head(&mut w, 2, 2);
    let p_entities = list_head(&mut w, entities.len(), 0);
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
    w.u32(HC_MAP);
    w.u32(0);
    let a_map = w.pos();
    w.u32(0);
    w.u32(0);

    for p in p_empty {
        w.point_here(p);
    }
    w.point_here(p_entities);
    let mut a_entities = vec![];
    for i in 0..entities.len() {
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
    for _ in 0..2 {
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
    let mut empty_array = |w: &mut Writer| {
        w.u32(0);
        p_end.push(w.rel());
    };
    empty_array(&mut w); // paths
    empty_array(&mut w); // lights
    empty_array(&mut w); // cameras
    empty_array(&mut w); // sounds by index
    empty_array(&mut w);
    empty_array(&mut w); // sounds
    empty_array(&mut w); // portals
    empty_array(&mut w); // skies
    w.u32(entities.len() as u32);
    let p_placements = w.rel();
    empty_array(&mut w); // placement groups
    let p_triggers = w.rel();
    w.u32(1);
    w.zeros(12);
    w.f32s(&bounds.min);
    w.f32s(&bounds.max);
    w.u32(1); // zones
    debug_assert_eq!(w.pos() - map, 0x88);

    // the zone, 0x8C bytes
    let zone = w.pos();
    w.u32(1); // its entity: the second reference pointer
    let p_identifier = w.rel();
    empty_array(&mut w); // lights
    empty_array(&mut w); // sounds
    empty_array(&mut w);
    empty_array(&mut w);
    let p_placement_info = w.rel();
    let p_zone_2c = w.rel();
    w.u32(0xFFFFFFFF);
    w.u32(0); // section
    w.u32(1);
    w.zeros(0x88 - 0x3C);
    p_end.push(w.rel());
    debug_assert_eq!(w.pos() - zone, 0x8C);
    w.align(16);

    // the tree that says which zone a point is in: one node, everything is zone 0
    w.point_here(p_bsp);
    w.f32s(&[1.0, 0.0, 0.0, 0.0]);
    w.zeros(16);

    w.point_here(p_identifier);
    for v in ZONE_IDENTIFIER {
        w.u32(v);
    }

    // no triggers: the four tables all point at the zeros behind
    w.point_here(p_triggers);
    w.u32(0);
    let trigger_tables: Vec<usize> = (0..4).map(|_| w.rel()).collect();
    for p in trigger_tables {
        w.point_here(p);
    }
    w.zeros(12);

    // which placements the zone has, and the tree a body's surroundings are looked up in
    w.point_here(p_placement_info);
    w.u32(entities.len() as u32);
    let p_zone_placements = w.rel();
    let p_tree = w.rel();
    w.point_here(p_zone_placements);
    for i in 0..entities.len() {
        w.u16(i as u16);
    }
    w.zeros(4);
    w.align(16);
    w.point_here(p_tree);
    let tree_flags = PLACEMENT_COLLIDES as u8;
    w.u8(1); // one child
    w.u8(tree_flags);
    w.u16(0);
    let p_leaf = w.rel();
    w.f32s(&bounds.min);
    w.f32s(&bounds.max);
    w.point_here(p_leaf);
    w.u8(0);
    w.u8(tree_flags);
    w.u16(entities.len() as u16);
    for i in 0..entities.len() {
        w.u16(i as u16);
        w.u8(PLACEMENT_COLLIDES as u8);
        w.u8(0);
    }
    w.u32(0);
    w.align(4);

    w.point_here(p_placements);
    for i in 0..entities.len() {
        w.u32(0xFFFFFFFF); // no hash
        w.f32s(&[0.0, 0.0, 0.0]);
        w.u32(0);
        w.f32s(&[0.0, -0.0, -0.0]);
        w.f32s(&[1.0, 1.0, 1.0]);
        w.u16(PLACEMENT_COLLIDES);
        w.u16(0); // the zone it is in
        w.u32(HC_LOCAL_ENTITY + i as u32);
        w.u16(0); // light set
        w.i16(-1); // group
        w.u32(0);
    }

    w.point_here(p_zone_2c);
    w.u32(0);
    let p_a = w.rel();
    let p_b = w.rel();
    w.point_here(p_a);
    w.point_here(p_b);
    w.zeros(0x1C);

    w.align(32);
    for p in p_end {
        w.point_here(p);
    }

    for (entity, at) in entities.iter().zip(&a_entities) {
        w.align(32);
        w.set_u32(*at, w.pos() as u32);
        match entity {
            Entity::Drawn(meshes) => write_split(&mut w, meshes),
            Entity::Hull(hull) => write_mesh_with_collision(&mut w, &unseen_mesh(hull), hull, None),
        };
    }

    // the zone's own entity (0x608), which refers to an empty group (0x603) through the first
    // reference pointer: the level's triangles are all placed
    w.align(32);
    w.set_u32(a_refptrs[1], w.pos() as u32);
    w.u32(0x608);
    w.zeros(0x50);
    w.u32(0x01000000);
    w.u32(0);
    w.zeros(0xC);
    w.f32(10000.0);
    w.zeros(0x14);
    w.set_u32(a_refptrs[0], w.pos() as u32);
    w.u32(0x603);
    w.zeros(0x50);
    w.u32(0);
    w.u32(8);
    w.u32(0);

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
        size + 216 + 36 * textures.len() as u32 + 16 * entities.len() as u32,
    );
    w.set_u32(a_section + 4, size);
    w.set_u32(section_start, size);

    (w.buf, stats)
}
