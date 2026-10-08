//! Mesh entities (0x601) as GoldenEye 007 (Wii) draws and collides with them.
//!
//! A mesh has a 0xC0 byte header, then for each texture a "strip": a 0x20 byte header and a GX
//! display list of triangle strips (0x98) whose vertices are four 16 bit indices: position,
//! normal, colour, texture coordinate. The positions (three floats, then the normal as three
//! signed bytes with 6 fraction bits and a byte 0x77) are 16 bytes each, the texture coordinates
//! two shorts whose fraction bits the header gives, the colours four bytes. A short of flags for
//! each triangle and the list of textures follow.
//!
//! The game collides with entities that have bit 0x100 in their flags, and not with the mesh
//! itself when it has the "variant" that is asked for, another whole mesh: the low byte of the
//! word at +0x40 has a bit for each variant there is, its top 24 bits are the offset from +0x40
//! to a word for each, whose own top 24 bits are the offset to the variant and its low byte the
//! variant's number.

use std::collections::HashMap;

use super::writer::Writer;

pub const ENTITY_MESH: u32 = 0x601;
/// A triangle a body doesn't collide with (the game's own have this on the triangles without
/// area that join strips, with bit 0x8000)
pub const FACE_NO_COLLISION: u16 = 0x8200;
pub const FLAG_COLLIDES: u32 = 0x100;
/// A strip's flags: seen from its front only, as nearly all of the game's own levels' are, or
/// from both sides (the game's test level)
const STRIP_ONE_SIDED: u16 = 0x80;
const STRIP_TWO_SIDED: u16 = 0x40;
/// A strip's flags and blend as the game's own lightmaps have them. The byte at +6 is the
/// blend (fn_8029E0D0): 0 none or alpha, 1 added, 2 subtracted, 3 what is there times the texture
const STRIP_LIGHTMAP: u16 = 0x05;
const BLEND_LIGHTMAP: u16 = 0x0301;
const VERTEX_FORMAT: u32 = 0x203;
const NORMAL_SCALE: f32 = 63.0;
const NORMAL_PAD: u8 = 0x77;
/// What a vertex colour of 1.0 is stored as: the game's levels are lit to about half of the range
pub const COLOUR_ONE: f32 = 128.0;
pub const MAX_STRIP_TRIANGLES: usize = 0xFFFF;
/// The variants a collision mesh is given as. What asks for a collision names the variant it
/// wants (fn_802CD850): the levels' meshes have 0 and 1, a body wants 1. An entity without the
/// one asked for is collided with itself
const COLLISION_VARIANTS: [u32; 2] = [0, 1];

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GeVertex {
    pub pos: [f32; 3],
    pub normal: [f32; 3],
    pub uv: [f32; 2],
    pub color: [u8; 4],
}

/// The triangles of a mesh that share a texture
#[derive(Clone, Default)]
pub struct MeshPart {
    /// Index into the file's texture list
    pub texture: u16,
    pub triangles: Vec<[GeVertex; 3]>,
    /// Flags for each triangle (FACE_...), none: all 0
    pub flags: Vec<u16>,
    /// Drawn from behind as well
    pub two_sided: bool,
    /// Drawn over the mesh's other triangles, darkening them by its texture: baked light
    pub lightmap: bool,
}

#[derive(Clone, Default)]
pub struct MeshData {
    pub parts: Vec<MeshPart>,
}

impl MeshData {
    pub fn triangle_count(&self) -> usize {
        self.parts.iter().map(|p| p.triangles.len()).sum()
    }

    pub fn positions(&self) -> impl Iterator<Item = [f32; 3]> + '_ {
        self.parts
            .iter()
            .flat_map(|p| p.triangles.iter())
            .flat_map(|t| t.iter().map(|v| v.pos))
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Bounds {
    pub min: [f32; 3],
    pub max: [f32; 3],
}

impl Bounds {
    pub const EMPTY: Bounds = Bounds {
        min: [f32::MAX; 3],
        max: [f32::MIN; 3],
    };

    pub fn of(points: impl Iterator<Item = [f32; 3]>) -> Self {
        let mut b = Self::EMPTY;
        for p in points {
            b.add(p);
        }
        b
    }

    pub fn add(&mut self, p: [f32; 3]) {
        for k in 0..3 {
            self.min[k] = self.min[k].min(p[k]);
            self.max[k] = self.max[k].max(p[k]);
        }
    }

    pub fn merge(&mut self, other: &Bounds) {
        if !other.is_empty() {
            self.add(other.min);
            self.add(other.max);
        }
    }

    pub fn is_empty(&self) -> bool {
        self.min[0] > self.max[0]
    }

    /// Zeros for no points at all
    pub fn or_zero(&self) -> Bounds {
        if self.is_empty() {
            Bounds {
                min: [0.0; 3],
                max: [0.0; 3],
            }
        } else {
            *self
        }
    }

    pub fn center(&self) -> [f32; 3] {
        let b = self.or_zero();
        [
            (b.min[0] + b.max[0]) * 0.5,
            (b.min[1] + b.max[1]) * 0.5,
            (b.min[2] + b.max[2]) * 0.5,
        ]
    }

    pub fn radius(&self) -> f32 {
        let b = self.or_zero();
        let d = [b.max[0] - b.min[0], b.max[1] - b.min[1], b.max[2] - b.min[2]];
        (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt() * 0.5
    }
}

fn triangle_area(t: &[GeVertex; 3]) -> f32 {
    let (a, b, c) = (t[0].pos, t[1].pos, t[2].pos);
    let u = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
    let v = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
    let n = [
        u[1] * v[2] - u[2] * v[1],
        u[2] * v[0] - u[0] * v[2],
        u[0] * v[1] - u[1] * v[0],
    ];
    (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt() * 0.5
}

fn pack_normal(n: [f32; 3]) -> [u8; 3] {
    let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
    let n = if len > 1e-6 {
        [n[0] / len, n[1] / len, n[2] / len]
    } else {
        [0.0, 1.0, 0.0]
    };
    [
        (n[0] * NORMAL_SCALE).round() as i8 as u8,
        (n[1] * NORMAL_SCALE).round() as i8 as u8,
        (n[2] * NORMAL_SCALE).round() as i8 as u8,
    ]
}

/// Texture coordinates repeat, so a triangle's can be moved to start between 0 and 1: keeps the
/// 16 bit coordinates precise on surfaces tiled many times
fn rebased_uvs(t: &[GeVertex; 3]) -> [[f32; 2]; 3] {
    let mut res = [t[0].uv, t[1].uv, t[2].uv];
    for k in 0..2 {
        let low = res.iter().map(|uv| uv[k]).fold(f32::MAX, f32::min);
        if low.is_finite() {
            let shift = low.floor();
            for uv in &mut res {
                uv[k] -= shift;
            }
        } else {
            for uv in &mut res {
                uv[k] = 0.0;
            }
        }
    }
    res
}

/// Writes a mesh entity at the writer's position (a multiple of 32) and returns its bounds
pub fn write_mesh(w: &mut Writer, mesh: &MeshData, flags: u32) -> Bounds {
    write_mesh_in(w, mesh, flags, None)
}

/// A mesh that is a part of a group: its first sphere is the whole group's
pub fn write_mesh_in(w: &mut Writer, mesh: &MeshData, flags: u32, group: Option<&Bounds>) -> Bounds {
    let start = w.pos();
    debug_assert_eq!(start % 32, 0);

    let bounds = Bounds::of(mesh.positions());
    let shown = bounds.or_zero();
    let area: f32 = mesh
        .parts
        .iter()
        .filter(|p| !p.lightmap)
        .flat_map(|p| p.triangles.iter())
        .map(triangle_area)
        .sum();

    // the texture coordinates' fraction bits: 16 - shift, enough room for the largest one
    let uvs: Vec<Vec<[[f32; 2]; 3]>> = mesh
        .parts
        .iter()
        .map(|p| p.triangles.iter().map(rebased_uvs).collect())
        .collect();
    let largest = uvs
        .iter()
        .flatten()
        .flatten()
        .flat_map(|uv| uv.iter())
        .fold(0.0f32, |m, v| m.max(v.abs()));
    let mut uv_shift = 2u32;
    while uv_shift < 9 && largest >= (1u32 << (uv_shift - 1)) as f32 - 0.01 {
        uv_shift += 1;
    }
    let uv_scale = (1u32 << (16 - uv_shift)) as f32;

    let mut positions: Vec<([f32; 3], [u8; 3])> = vec![];
    let mut position_index: HashMap<([u32; 3], [u8; 3]), u16> = HashMap::new();
    let mut coords: Vec<[i16; 2]> = vec![];
    let mut coord_index: HashMap<[i16; 2], u16> = HashMap::new();
    let mut colours: Vec<[u8; 4]> = vec![];
    let mut colour_index: HashMap<[u8; 4], u16> = HashMap::new();

    let mut lists: Vec<Vec<u8>> = vec![];
    for (part, part_uvs) in mesh.parts.iter().zip(&uvs) {
        assert!(part.triangles.len() <= MAX_STRIP_TRIANGLES);
        let mut list = Writer::new();
        for (triangle, tri_uvs) in part.triangles.iter().zip(part_uvs) {
            // a strip of one triangle, behind a no-op that keeps the indices on even addresses
            list.u8(0x00);
            list.u8(0x98);
            list.u16(3);
            for (v, uv) in triangle.iter().zip(tri_uvs) {
                let normal = pack_normal(v.normal);
                let key = (v.pos.map(f32::to_bits), normal);
                let pos = *position_index.entry(key).or_insert_with(|| {
                    positions.push((v.pos, normal));
                    (positions.len() - 1) as u16
                });
                let coord = [
                    (uv[0] * uv_scale).round().clamp(-32768.0, 32767.0) as i16,
                    (uv[1] * uv_scale).round().clamp(-32768.0, 32767.0) as i16,
                ];
                let coord = *coord_index.entry(coord).or_insert_with(|| {
                    coords.push(coord);
                    (coords.len() - 1) as u16
                });
                let colour = *colour_index.entry(v.color).or_insert_with(|| {
                    colours.push(v.color);
                    (colours.len() - 1) as u16
                });
                list.u16(pos);
                list.u16(pos);
                list.u16(colour);
                list.u16(coord);
            }
        }
        list.align(32);
        lists.push(list.buf);
    }
    assert!(positions.len() <= 0xFFFF && coords.len() <= 0xFFFF && colours.len() <= 0xFFFF);
    if colours.is_empty() {
        colours.push([0x80, 0x80, 0x80, 0xFF]);
    }

    w.u32(ENTITY_MESH);
    w.u32(flags);
    w.u16(0); // sort value
    w.u8(0); // render order
    w.u8(0);
    w.f32(area);
    w.f32s(&[shown.min[0], shown.min[1], shown.min[2], 0.0]);
    w.f32s(&[shown.max[0], shown.max[1], shown.max[2], 0.0]);
    let centre = bounds.center();
    let whole = group.unwrap_or(&bounds);
    let whole_centre = whole.center();
    w.f32s(&[whole_centre[0], whole_centre[1], whole_centre[2], whole.radius()]);
    debug_assert_eq!(w.pos() - start, 0x40);
    w.u32(0); // variants, see write_mesh_with_collision
    w.u32(0); // a tree over the triangles, the game does without it
    w.u32(0);
    w.u32(0);
    w.u32(0); // gdi count and index
    let p_textures = w.rel();
    let p_strips = w.rel();
    let p_positions = w.rel();
    let p_coords = w.rel();
    let p_colours = w.rel();
    let p_faces = w.rel();
    w.u32(0); // face info
    w.u32(0); // index data
    w.u32(0);
    w.u32(0);
    w.f32s(&[centre[0], centre[1], centre[2], bounds.radius()]);
    w.f32s(&shown.min);
    w.f32s(&shown.max);
    debug_assert_eq!(w.pos() - start, 0xA4);
    w.u32(mesh.parts.len() as u32);
    w.u32(positions.len() as u32);
    w.u32(0); // how many triangles have flags, filled in below
    w.u32(uv_shift << 28 | VERTEX_FORMAT);
    w.zeros(12);
    debug_assert_eq!(w.pos() - start, 0xC0);

    w.point_here(p_strips);
    for (part, list) in mesh.parts.iter().zip(&lists) {
        w.u16(part.triangles.len() as u16);
        w.u16(part.texture);
        let side = if part.two_sided { STRIP_TWO_SIDED } else { STRIP_ONE_SIDED };
        w.u16(if part.lightmap { side | STRIP_LIGHTMAP } else { side });
        w.u16(if part.lightmap { BLEND_LIGHTMAP } else { 0 }); // blend, fade layer
        w.u32(list.len() as u32);
        w.u32(0);
        w.zeros(16);
        w.bytes(list);
    }

    w.align(32);
    w.point_here(p_positions);
    for (pos, normal) in &positions {
        w.f32s(pos);
        w.bytes(normal);
        w.u8(NORMAL_PAD);
    }

    w.align(32);
    w.point_here(p_coords);
    for coord in &coords {
        w.i16(coord[0]);
        w.i16(coord[1]);
    }

    w.align(32);
    w.point_here(p_colours);
    for colour in &colours {
        w.bytes(colour);
    }

    // a short of flags for each triangle, 0: collided with
    w.point_here(p_faces);
    let mut unflagged = 0;
    for part in &mesh.parts {
        for i in 0..part.triangles.len() {
            let flags = part.flags.get(i).copied().unwrap_or(0);
            unflagged += (flags != 0) as u32;
            w.u16(flags);
        }
    }
    w.set_u32(start + 0xAC, unflagged);
    w.align(4);

    w.point_here(p_textures);
    w.u16(mesh.parts.len() as u16);
    for part in &mesh.parts {
        w.u16(part.texture);
    }
    w.align(4);

    bounds
}

/// Writes a mesh that is drawn as `shown` and collided with as `collision`: the mesh gets
/// `collision` as its variant 0. Returns the bounds of both.
pub fn write_mesh_with_collision(
    w: &mut Writer,
    shown: &MeshData,
    collision: &MeshData,
    group: Option<&Bounds>,
) -> Bounds {
    let start = w.pos();
    let mut bounds = write_mesh_in(w, shown, FLAG_COLLIDES, group);

    w.align(4);
    let table = w.pos();
    w.zeros(4 * COLLISION_VARIANTS.len());

    // a copy for each variant: the game makes an object of every variant when the file is
    // loaded, one mesh can't be two of them
    let variants = start + 0x40;
    let mut bits = 0;
    for (i, id) in COLLISION_VARIANTS.iter().enumerate() {
        w.align(32);
        let variant = w.pos();
        bounds.merge(&write_mesh_in(w, collision, FLAG_COLLIDES, group));
        let word = table + i * 4;
        w.set_u32(word, ((variant - word) as u32) << 8 | *id);
        bits |= 1 << id;
    }
    w.set_u32(variants, ((table - variants) as u32) << 8 | bits);

    // the entity's own bounds are what the game tests a body against first: both meshes
    let b = bounds.or_zero();
    let centre = bounds.center();
    let mut head = Writer::new();
    head.f32s(&[b.min[0], b.min[1], b.min[2], 0.0]);
    head.f32s(&[b.max[0], b.max[1], b.max[2], 0.0]);
    let whole = group.unwrap_or(&bounds);
    let whole_centre = whole.center();
    head.f32s(&[whole_centre[0], whole_centre[1], whole_centre[2], whole.radius()]);
    w.buf[start + 0x10..start + 0x40].copy_from_slice(&head.buf);
    let mut tail = Writer::new();
    tail.f32s(&[centre[0], centre[1], centre[2], bounds.radius()]);
    tail.f32s(&b.min);
    tail.f32s(&b.max);
    w.buf[start + 0x7C..start + 0xA4].copy_from_slice(&tail.buf);

    bounds
}

/// Halves things along the longest side of the box their centres are in, so that parts made
/// by doing this again and again stay small boxes: the game tests an entity's bounds before it
/// goes over its triangles
pub fn halve<T>(mut items: Vec<T>, centre: &impl Fn(&T) -> [f32; 3]) -> (Vec<T>, Vec<T>) {
    let bounds = Bounds::of(items.iter().map(centre));
    let size = [
        bounds.max[0] - bounds.min[0],
        bounds.max[1] - bounds.min[1],
        bounds.max[2] - bounds.min[2],
    ];
    let axis = if size[0] >= size[1] && size[0] >= size[2] {
        0
    } else if size[1] >= size[2] {
        1
    } else {
        2
    };

    items.sort_by(|a, b| centre(a)[axis].total_cmp(&centre(b)[axis]));
    let upper = items.split_off(items.len() / 2);
    (items, upper)
}

pub const ENTITY_SPLIT: u32 = 0x603;
/// Nodes of a group in a split's tree
const TREE_GROUP: usize = 4;

enum TreeNode {
    Leaf(usize, Bounds),
    Group(Vec<TreeNode>, Bounds),
}

impl TreeNode {
    fn bounds(&self) -> &Bounds {
        match self {
            TreeNode::Leaf(_, b) | TreeNode::Group(_, b) => b,
        }
    }
}

fn tree_group(items: Vec<(usize, Bounds)>) -> Vec<TreeNode> {
    if items.len() <= TREE_GROUP {
        return items.into_iter().map(|(i, b)| TreeNode::Leaf(i, b)).collect();
    }
    let centre = |item: &(usize, Bounds)| item.1.center();
    let (lower, upper) = halve(items, &centre);
    let mut quarters = vec![];
    for half in [lower, upper] {
        if half.len() > 1 {
            let (a, b) = halve(half, &centre);
            quarters.push(a);
            quarters.push(b);
        } else {
            quarters.push(half);
        }
    }
    quarters
        .into_iter()
        .filter(|q| !q.is_empty())
        .map(|q| {
            if q.len() == 1 {
                return TreeNode::Leaf(q[0].0, q[0].1);
            }
            let mut bounds = Bounds::EMPTY;
            for (_, b) in &q {
                bounds.merge(b);
            }
            TreeNode::Group(tree_group(q), bounds)
        })
        .collect()
}

/// Writes a group's nodes (0x30 bytes each) and then the groups below it. Returns where the
/// pointers to the meshes are, to be filled in when those are written: (the pointer, the mesh)
fn write_tree_group(w: &mut Writer, group: &[TreeNode], leaves: &mut Vec<(usize, usize)>) {
    let mut below = vec![];
    for node in group {
        let b = node.bounds().or_zero();
        let centre = b.center();
        w.f32s(&b.min);
        // how many nodes the group has: negative on a node that is a mesh
        match node {
            TreeNode::Leaf(..) => w.i32(-(group.len() as i32)),
            TreeNode::Group(..) => w.i32(group.len() as i32),
        }
        w.f32s(&b.max);
        let pointer = w.rel();
        w.f32s(&[centre[0], centre[1], centre[2], b.radius()]);
        match node {
            TreeNode::Leaf(mesh, _) => leaves.push((pointer, *mesh)),
            TreeNode::Group(nodes, _) => below.push((pointer, nodes)),
        }
    }
    for (pointer, nodes) in below {
        w.point_here(pointer);
        write_tree_group(w, nodes, leaves);
    }
}

/// Writes a split entity (0x603): a group of meshes that is placed as one thing, with a tree of
/// boxes over them. A body collides with the meshes' own triangles, the ones without
/// FACE_NO_COLLISION: a mesh's variants count for nothing in a group. A node of the tree is two corners, a count, a pointer and a sphere: the
/// count is how many nodes its group has, negative when the pointer is to a mesh, positive when
/// it is to the first node of the group below.
pub fn write_split(w: &mut Writer, meshes: &[MeshData]) -> Bounds {
    let start = w.pos();
    debug_assert_eq!(start % 32, 0);

    let each: Vec<Bounds> = meshes.iter().map(|m| Bounds::of(m.positions())).collect();
    let mut bounds = Bounds::EMPTY;
    for b in &each {
        bounds.merge(b);
    }
    let shown = bounds.or_zero();
    let centre = bounds.center();
    let area: f32 = meshes
        .iter()
        .flat_map(|m| m.parts.iter())
        .flat_map(|p| p.triangles.iter())
        .map(triangle_area)
        .sum();

    w.u32(ENTITY_SPLIT);
    w.u32(FLAG_COLLIDES);
    w.u32(0);
    w.f32(area);
    w.f32s(&[shown.min[0], shown.min[1], shown.min[2], 0.0]);
    w.f32s(&[shown.max[0], shown.max[1], shown.max[2], 0.0]);
    w.f32s(&[centre[0], centre[1], centre[2], bounds.radius()]);
    w.zeros(0x14);
    debug_assert_eq!(w.pos() - start, 0x54);
    w.u32(meshes.len() as u32);
    let p_tree = w.rel();
    let p_meshes: Vec<usize> = meshes.iter().map(|_| w.rel()).collect();
    w.align(16);

    w.point_here(p_tree);
    let mut leaves = vec![];
    let root = tree_group(each.iter().copied().enumerate().collect());
    write_tree_group(w, &root, &mut leaves);

    for (i, mesh) in meshes.iter().enumerate() {
        w.align(32);
        let at = w.pos();
        w.point(p_meshes[i], at);
        for (pointer, leaf) in &leaves {
            if *leaf == i {
                w.point(*pointer, at);
            }
        }
        write_mesh_in(w, mesh, FLAG_COLLIDES, Some(&bounds));
    }

    bounds
}
