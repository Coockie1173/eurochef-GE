//! Rooms and portals: what the game draws a level by.
//!
//! A map is zones joined by portals. For a view the game looks the camera's position up in the
//! map's tree (planes, a leaf is a zone), draws that zone and goes through its portals: a
//! portal is a quad, and only when something of it is on the screen (and it is seen from its
//! own side) the zone behind it is drawn, cut to the portal's rectangle, and walked in turn.
//!
//! The scene names its rooms and portals (`RoomXX` and `Portal_XX_YY`, see `gltf_import`). Here
//! - every triangle gets a room: the one of its node, or the one it lies in
//! - every portal becomes a quad whose corners turn so that its front is room XX's side
//! - the tree is made: points all over the level's rooms are given the room they lie in, then
//!   planes are picked that keep the rooms' points apart (the portals' own planes first, then
//!   the walls', then planes along the axes)
//!
//! Which room a point lies in is found by looking around from it: the walls, floors and
//! ceilings seen in most directions are its room's. A portal is looked through by nobody.

use super::{
    geomap::GeScene,
    mesh::{Bounds, GeVertex},
};

/// The game keeps a zone's number in a byte
pub const MAX_ZONES: usize = 255;
/// The tree's nodes are numbered with shorts
const MAX_TREE_NODES: usize = 30000;
const MAX_TREE_DEPTH: usize = 64;
/// How many points on the rooms' surfaces and in the level's box the tree is made from, about
const SURFACE_SAMPLES: f32 = 40000.0;
const GRID_SAMPLES: f32 = 20000.0;
/// How far in front of a surface its points are
const SURFACE_OFFSETS: [f32; 2] = [0.12, 0.5];
/// How far a wall's plane is moved behind the wall, so what stands on or against it is in front
const PLANE_BEHIND: f32 = 0.02;
const WALL_PLANES: usize = 256;

type Vec3 = [f32; 3];

fn sub(a: Vec3, b: Vec3) -> Vec3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn add(a: Vec3, b: Vec3) -> Vec3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn mul(a: Vec3, k: f32) -> Vec3 {
    [a[0] * k, a[1] * k, a[2] * k]
}

fn dot(a: Vec3, b: Vec3) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: Vec3, b: Vec3) -> Vec3 {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn normalised(a: Vec3) -> Option<Vec3> {
    let len = dot(a, a).sqrt();
    (len.is_finite() && len > 1e-7).then(|| mul(a, 1.0 / len))
}

fn centre(t: &[Vec3; 3]) -> Vec3 {
    mul(add(add(t[0], t[1]), t[2]), 1.0 / 3.0)
}

/// A triangle's normal times twice its area
fn area_normal(t: &[Vec3; 3]) -> Vec3 {
    cross(sub(t[1], t[0]), sub(t[2], t[0]))
}

pub fn positions(t: &[GeVertex; 3]) -> [Vec3; 3] {
    [t[0].pos, t[1].pos, t[2].pos]
}

#[derive(Clone, Debug)]
pub struct Zone {
    pub name: String,
    pub bounds: Bounds,
}

/// A quad between two zones. Its corners turn so that `(c1 - c0) x (c2 - c0)` points from
/// `zones[0]` into `zones[1]`: the game sees through a portal from one side only and tells the
/// side by the way the corners turn on the screen
#[derive(Clone, Debug)]
pub struct ZonePortal {
    pub name: String,
    pub zones: [u16; 2],
    pub corners: [Vec3; 4],
}

/// A node of the tree a position's zone is looked up in. In front of the plane (or on it) the
/// first child is next, behind it the second. A child above 0 is a node, others a zone, negated
#[derive(Clone, Copy, Debug)]
pub struct ZoneTreeNode {
    pub plane: [f32; 4],
    pub children: [i16; 2],
}

#[derive(Clone, Debug)]
pub struct Zoning {
    pub zones: Vec<Zone>,
    /// The zone of each of the scene's drawn triangles
    pub triangle_zones: Vec<u16>,
    pub portals: Vec<ZonePortal>,
    pub tree: Vec<ZoneTreeNode>,
    /// What there is to say about the scene's rooms: portals left out, rooms nothing leads to
    pub warnings: Vec<String>,
}

impl Zoning {
    /// The zone a position lies in, the way the game looks it up
    pub fn zone_at(&self, p: Vec3) -> u16 {
        let mut node = 0usize;
        loop {
            let Some(n) = self.tree.get(node) else {
                return 0;
            };
            let side = dot([n.plane[0], n.plane[1], n.plane[2]], p) + n.plane[3] < 0.0;
            let child = n.children[side as usize];
            if child <= 0 {
                return (-child) as u16;
            }
            node = child as usize;
        }
    }
}

/// What a look from a point may end on
#[derive(Clone, Copy, PartialEq)]
enum Seen {
    Room(u16),
    Portal,
}

struct Blocker {
    corners: [Vec3; 3],
    seen: Seen,
}

/// The rooms' triangles and the portals in a tree of boxes, to look around in
struct Surroundings {
    blockers: Vec<Blocker>,
    /// A box, then either the two nodes below it or the blockers in it
    nodes: Vec<(Bounds, BoxNode)>,
}

enum BoxNode {
    Split(usize, usize),
    Leaf(usize, usize),
}

/// The directions looked in: along the axes and the diagonals, a little off so that a look
/// doesn't run along a wall or through an edge
const LOOKS: [Vec3; 14] = [
    [1.0, 0.031, 0.017],
    [-1.0, 0.023, -0.041],
    [0.019, 1.0, 0.037],
    [-0.029, -1.0, 0.013],
    [0.043, -0.011, 1.0],
    [-0.013, 0.047, -1.0],
    [1.0, 0.93, 1.07],
    [-1.0, 1.05, 0.91],
    [1.0, -0.95, 1.03],
    [-1.0, -1.09, 0.97],
    [1.0, 1.01, -0.89],
    [-1.0, 0.97, -1.06],
    [1.0, -1.04, -0.94],
    [-1.0, -0.92, -1.02],
];

impl Surroundings {
    fn new(blockers: Vec<Blocker>) -> Self {
        let mut res = Self {
            blockers,
            nodes: vec![],
        };
        let count = res.blockers.len();
        if count > 0 {
            res.build(0, count);
        }
        res
    }

    fn build(&mut self, from: usize, to: usize) -> usize {
        let bounds = Bounds::of(self.blockers[from..to].iter().flat_map(|b| b.corners));
        let index = self.nodes.len();
        self.nodes.push((bounds, BoxNode::Leaf(from, to)));
        if to - from > 4 {
            let size = sub(bounds.max, bounds.min);
            let axis = (0..3).max_by(|a, b| size[*a].total_cmp(&size[*b])).unwrap();
            let middle = (from + to) / 2;
            self.blockers[from..to].select_nth_unstable_by(middle - from, |a, b| {
                centre(&a.corners)[axis].total_cmp(&centre(&b.corners)[axis])
            });
            let lower = self.build(from, middle);
            let upper = self.build(middle, to);
            self.nodes[index].1 = BoxNode::Split(lower, upper);
        }
        index
    }

    /// What a look from a point in a direction ends on, the nearest thing
    fn look(&self, from: Vec3, direction: Vec3) -> Option<Seen> {
        if self.nodes.is_empty() {
            return None;
        }
        let inverse = direction.map(|d| if d.abs() > 1e-12 { 1.0 / d } else { 1e12 });
        let mut nearest = f32::INFINITY;
        let mut seen = None;
        let mut stack = vec![0usize];
        while let Some(index) = stack.pop() {
            let (bounds, node) = &self.nodes[index];
            let (mut enter, mut leave) = (0.0f32, nearest);
            for axis in 0..3 {
                let a = (bounds.min[axis] - from[axis]) * inverse[axis];
                let b = (bounds.max[axis] - from[axis]) * inverse[axis];
                enter = enter.max(a.min(b));
                leave = leave.min(a.max(b));
            }
            if enter > leave {
                continue;
            }
            match node {
                BoxNode::Split(lower, upper) => {
                    stack.push(*lower);
                    stack.push(*upper);
                }
                BoxNode::Leaf(first, last) => {
                    for blocker in &self.blockers[*first..*last] {
                        if let Some(distance) = hit(from, direction, &blocker.corners) {
                            if distance < nearest {
                                nearest = distance;
                                seen = Some(blocker.seen);
                            }
                        }
                    }
                }
            }
        }
        seen
    }

    /// The room a point lies in: the one most looks end on, with how many did and how many
    /// ended on any room at all
    fn room_at(&self, p: Vec3, rooms: usize) -> Option<(u16, usize, usize)> {
        let mut votes = vec![0usize; rooms];
        let mut total = 0;
        for direction in LOOKS {
            if let Some(Seen::Room(room)) = self.look(p, direction) {
                votes[room as usize] += 1;
                total += 1;
            }
        }
        let best = (0..rooms).max_by_key(|r| votes[*r])?;
        (votes[best] > 0).then_some((best as u16, votes[best], total))
    }

    /// The same, but only when the looks agree: a point in a wall or outside gets nothing
    fn sure_room_at(&self, p: Vec3, rooms: usize) -> Option<u16> {
        let (room, votes, total) = self.room_at(p, rooms)?;
        (total >= 5 && votes * 10 >= total * 7).then_some(room)
    }
}

/// How far along a direction a triangle is hit, from either side
fn hit(from: Vec3, direction: Vec3, t: &[Vec3; 3]) -> Option<f32> {
    let (e1, e2) = (sub(t[1], t[0]), sub(t[2], t[0]));
    let p = cross(direction, e2);
    let det = dot(e1, p);
    if det.abs() < 1e-12 {
        return None;
    }
    let inverse = 1.0 / det;
    let s = sub(from, t[0]);
    let u = dot(s, p) * inverse;
    if !(0.0..=1.0).contains(&u) {
        return None;
    }
    let q = cross(s, e1);
    let v = dot(direction, q) * inverse;
    if v < 0.0 || u + v > 1.0 {
        return None;
    }
    let distance = dot(e2, q) * inverse;
    (distance > 1e-5).then_some(distance)
}

/// The quad of a portal's triangles: their four corners when that is what they have, else the
/// smallest rectangle in their plane that holds them. Turned around the normal
fn quad_of(triangles: &[[Vec3; 3]]) -> Option<([Vec3; 4], Vec3)> {
    let sum = triangles.iter().fold([0.0; 3], |n, t| add(n, area_normal(t)));
    let normal = normalised(sum)?;

    let mut points: Vec<Vec3> = vec![];
    for p in triangles.iter().flatten() {
        if !points.iter().any(|q| dot(sub(*p, *q), sub(*p, *q)) < 1e-8) {
            points.push(*p);
        }
    }
    let middle = mul(points.iter().fold([0.0; 3], |s, p| add(s, *p)), 1.0 / points.len() as f32);
    // two directions in the plane
    let up = if normal[1].abs() < 0.9 { [0.0, 1.0, 0.0] } else { [1.0, 0.0, 0.0] };
    let u = normalised(sub(up, mul(normal, dot(up, normal))))?;
    let v = cross(normal, u);
    let flat = |p: Vec3| [dot(sub(p, middle), u), dot(sub(p, middle), v)];
    let lift = |x: f32, y: f32| add(middle, add(mul(u, x), mul(v, y)));

    if points.len() == 4 {
        let mut corners: Vec<(f32, Vec3)> = points
            .iter()
            .map(|p| {
                let f = flat(*p);
                (f[1].atan2(f[0]), lift(f[0], f[1]))
            })
            .collect();
        corners.sort_by(|a, b| a.0.total_cmp(&b.0));
        return Some(([corners[0].1, corners[1].1, corners[2].1, corners[3].1], normal));
    }

    // the smallest rectangle along one of the triangles' sides
    let mut best: Option<(f32, [Vec3; 4])> = None;
    for t in triangles {
        for k in 0..3 {
            let side = sub(flat(t[(k + 1) % 3]).into_3(), flat(t[k]).into_3());
            let Some(along) = normalised(side) else {
                continue;
            };
            let across = [-along[1], along[0], 0.0];
            let (mut low, mut high) = ([f32::INFINITY; 2], [f32::NEG_INFINITY; 2]);
            for p in &points {
                let f = flat(*p).into_3();
                let at = [dot(f, along), dot(f, across)];
                for i in 0..2 {
                    low[i] = low[i].min(at[i]);
                    high[i] = high[i].max(at[i]);
                }
            }
            let area = (high[0] - low[0]) * (high[1] - low[1]);
            if best.as_ref().map(|b| area < b.0).unwrap_or(true) {
                let corner = |x: f32, y: f32| {
                    let f = add(mul(along, x), mul(across, y));
                    lift(f[0], f[1])
                };
                best = Some((
                    area,
                    [
                        corner(low[0], low[1]),
                        corner(high[0], low[1]),
                        corner(high[0], high[1]),
                        corner(low[0], high[1]),
                    ],
                ));
            }
        }
    }
    best.map(|b| (b.1, normal))
}

trait Into3 {
    fn into_3(self) -> Vec3;
}

impl Into3 for [f32; 2] {
    fn into_3(self) -> Vec3 {
        [self[0], self[1], 0.0]
    }
}

/// A point of the level and the room it lies in
struct Sample {
    pos: Vec3,
    room: u16,
}

/// A plane the tree may cut along. Lower kinds are taken first when they cut as well
#[derive(Clone, Copy)]
struct Cut {
    plane: [f32; 4],
    kind: u8,
}

const KIND_PORTAL: u8 = 0;
const KIND_WALL: u8 = 1;
const KIND_AXIS: u8 = 2;

fn side_of(plane: &[f32; 4], p: Vec3) -> bool {
    plane[0] * p[0] + plane[1] * p[1] + plane[2] * p[2] + plane[3] < 0.0
}

/// How mixed the rooms on a plane's two sides are: 0 when each side has one room only
struct Mix {
    counts: Vec<u32>,
    total: u32,
    squares: f64,
}

impl Mix {
    fn new(rooms: usize) -> Self {
        Self {
            counts: vec![0; rooms],
            total: 0,
            squares: 0.0,
        }
    }

    fn add(&mut self, room: u16) {
        let c = &mut self.counts[room as usize];
        self.squares += (2 * *c + 1) as f64;
        *c += 1;
        self.total += 1;
    }

    fn remove(&mut self, room: u16) {
        let c = &mut self.counts[room as usize];
        self.squares -= (2 * *c - 1) as f64;
        *c -= 1;
        self.total -= 1;
    }

    fn mixed(&self) -> f64 {
        if self.total == 0 {
            0.0
        } else {
            self.total as f64 - self.squares / self.total as f64
        }
    }
}

struct TreeBuilder<'a> {
    samples: &'a [Sample],
    rooms: usize,
    /// The portals' and the walls' planes
    planes: Vec<Cut>,
    nodes: Vec<ZoneTreeNode>,
}

impl TreeBuilder<'_> {
    fn most(&self, indices: &[u32]) -> u16 {
        let mut counts = vec![0u32; self.rooms];
        for i in indices {
            counts[self.samples[*i as usize].room as usize] += 1;
        }
        (0..self.rooms).max_by_key(|r| counts[*r]).unwrap_or(0) as u16
    }

    /// The best plane to cut a node's points along, if any keeps rooms apart
    fn pick(&self, indices: &[u32]) -> Option<Cut> {
        let mut whole = Mix::new(self.rooms);
        for i in indices {
            whole.add(self.samples[*i as usize].room);
        }
        // the best of each kind
        let mut best: [Option<(f64, Cut)>; 3] = [None; 3];
        let mut offer = |mixed: f64, cut: Cut| {
            let slot = &mut best[cut.kind as usize];
            if slot.map(|b| mixed < b.0).unwrap_or(true) {
                *slot = Some((mixed, cut));
            }
        };

        for cut in &self.planes {
            let (mut front, mut behind) = (Mix::new(self.rooms), Mix::new(self.rooms));
            for i in indices {
                let s = &self.samples[*i as usize];
                if side_of(&cut.plane, s.pos) {
                    behind.add(s.room);
                } else {
                    front.add(s.room);
                }
            }
            if front.total > 0 && behind.total > 0 {
                offer(front.mixed() + behind.mixed(), *cut);
            }
        }

        let mut sorted = indices.to_vec();
        for axis in 0..3 {
            sorted.sort_unstable_by(|a, b| {
                self.samples[*a as usize].pos[axis].total_cmp(&self.samples[*b as usize].pos[axis])
            });
            let mut lower = Mix::new(self.rooms);
            let mut upper = Mix::new(self.rooms);
            for i in &sorted {
                upper.add(self.samples[*i as usize].room);
            }
            for pair in sorted.windows(2) {
                let (a, b) = (&self.samples[pair[0] as usize], &self.samples[pair[1] as usize]);
                lower.add(a.room);
                upper.remove(a.room);
                if b.pos[axis] - a.pos[axis] < 1e-4 {
                    continue;
                }
                let mut plane = [0.0; 4];
                plane[axis] = 1.0;
                plane[3] = -(a.pos[axis] + b.pos[axis]) * 0.5;
                offer(
                    lower.mixed() + upper.mixed(),
                    Cut {
                        plane,
                        kind: KIND_AXIS,
                    },
                );
            }
        }

        let least = best.iter().flatten().map(|b| b.0).fold(f64::INFINITY, f64::min);
        if !least.is_finite() || least >= whole.mixed() - 1e-9 {
            // nothing keeps anything apart: any cut along an axis still gets the points fewer
            return best[KIND_AXIS as usize].map(|b| b.1);
        }
        // a portal's or a wall's own plane is where the rooms really end: taken when it cuts
        // about as well as the best
        best.iter()
            .flatten()
            .find(|b| b.0 <= least * 1.02 + 0.5)
            .map(|b| b.1)
    }

    /// Makes the node for some points and returns what its parent points to
    fn build(&mut self, indices: Vec<u32>, depth: usize) -> i16 {
        let first = self.samples[indices[0] as usize].room;
        let one_room = indices.iter().all(|i| self.samples[*i as usize].room == first);
        if one_room {
            return -(first as i16);
        }
        if depth >= MAX_TREE_DEPTH || self.nodes.len() + 2 >= MAX_TREE_NODES {
            return -(self.most(&indices) as i16);
        }
        let Some(cut) = self.pick(&indices) else {
            return -(self.most(&indices) as i16);
        };
        let (behind, front): (Vec<u32>, Vec<u32>) = indices
            .iter()
            .partition(|i| side_of(&cut.plane, self.samples[**i as usize].pos));
        if behind.is_empty() || front.is_empty() {
            return -(self.most(&indices) as i16);
        }

        let index = self.nodes.len();
        self.nodes.push(ZoneTreeNode {
            plane: cut.plane,
            children: [0, 0],
        });
        let a = self.build(front, depth + 1);
        let b = self.build(behind, depth + 1);
        self.nodes[index].children = [a, b];
        index as i16
    }
}

/// The planes most of the rooms' surface lies in, a little behind it
fn wall_planes(triangles: &[[Vec3; 3]]) -> Vec<Cut> {
    use std::collections::HashMap;
    let mut areas: HashMap<[i32; 4], (f32, [f32; 4])> = HashMap::new();
    for t in triangles {
        let twice = area_normal(t);
        let Some(n) = normalised(twice) else {
            continue;
        };
        let d = -dot(n, t[0]);
        let key = [
            (n[0] * 200.0).round() as i32,
            (n[1] * 200.0).round() as i32,
            (n[2] * 200.0).round() as i32,
            (d * 50.0).round() as i32,
        ];
        let entry = areas.entry(key).or_insert((0.0, [n[0], n[1], n[2], d + PLANE_BEHIND]));
        entry.0 += dot(twice, twice).sqrt();
    }
    let mut planes: Vec<(f32, [f32; 4])> = areas.into_values().collect();
    planes.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1[3].total_cmp(&b.1[3])));
    planes
        .into_iter()
        .take(WALL_PLANES)
        .map(|p| Cut {
            plane: p.1,
            kind: KIND_WALL,
        })
        .collect()
}

/// Points on a triangle, evenly spread
fn points_on(t: &[Vec3; 3], count: usize) -> impl Iterator<Item = Vec3> + '_ {
    (0..count).map(move |i| {
        // the golden ratio's fractions, folded into the triangle
        let mut a = (0.5 + i as f32 * 0.618_034).fract();
        let mut b = (0.5 + i as f32 * 0.754_878).fract();
        if count == 1 {
            (a, b) = (1.0 / 3.0, 1.0 / 3.0);
        } else if a + b > 1.0 {
            (a, b) = (1.0 - a, 1.0 - b);
        }
        add(t[0], add(mul(sub(t[1], t[0]), a), mul(sub(t[2], t[0]), b)))
    })
}

/// The scene's rooms and portals as the map's zones. None for a scene without rooms: one zone
pub fn make_zoning(scene: &GeScene) -> Option<Zoning> {
    if scene.rooms.is_empty() {
        return None;
    }
    let rooms = scene.rooms.len().min(MAX_ZONES);
    let mut warnings = vec![];
    if scene.rooms.len() > MAX_ZONES {
        warnings.push(format!(
            "{} rooms, the game has {} at most: the others' triangles go to the rooms they are nearest to",
            scene.rooms.len(),
            MAX_ZONES
        ));
    }
    let room_of = |room: Option<usize>| room.filter(|r| *r < rooms).map(|r| r as u16);

    // the portals' quads
    let mut quads: Vec<(usize, [Vec3; 4], Vec3)> = vec![];
    for (i, portal) in scene.portals.iter().enumerate() {
        if portal.rooms[0] >= rooms || portal.rooms[1] >= rooms || portal.rooms[0] == portal.rooms[1] {
            warnings.push(format!("portal {} doesn't join two rooms, left out", portal.name));
            continue;
        }
        match quad_of(&portal.triangles) {
            Some((corners, normal)) => quads.push((i, corners, normal)),
            None => warnings.push(format!("portal {} has no area, left out", portal.name)),
        }
    }

    // what is looked around in: the triangles that have a room, and the portals
    let mut blockers: Vec<Blocker> = scene
        .triangles
        .iter()
        .filter_map(|t| {
            Some(Blocker {
                corners: positions(&t.vertices),
                seen: Seen::Room(room_of(t.room)?),
            })
        })
        .collect();
    for (_, corners, _) in &quads {
        for corners in [[corners[0], corners[1], corners[2]], [corners[0], corners[2], corners[3]]] {
            blockers.push(Blocker {
                corners,
                seen: Seen::Portal,
            });
        }
    }
    let around = Surroundings::new(blockers);

    // every triangle's zone: its node's room, or the room in front of it
    let mut room_bounds = vec![Bounds::EMPTY; rooms];
    for t in &scene.triangles {
        if let Some(room) = room_of(t.room) {
            for v in &t.vertices {
                room_bounds[room as usize].add(v.pos);
            }
        }
    }
    let nearest_room = |p: Vec3| -> u16 {
        let distance = |b: &Bounds| -> f32 {
            if b.is_empty() {
                return f32::INFINITY;
            }
            (0..3)
                .map(|k| (b.min[k] - p[k]).max(p[k] - b.max[k]).max(0.0))
                .map(|d| d * d)
                .sum()
        };
        (0..rooms)
            .min_by(|a, b| distance(&room_bounds[*a]).total_cmp(&distance(&room_bounds[*b])))
            .unwrap_or(0) as u16
    };
    let mut strays = 0;
    let triangle_zones: Vec<u16> = scene
        .triangles
        .iter()
        .map(|t| {
            room_of(t.room).unwrap_or_else(|| {
                strays += 1;
                let corners = positions(&t.vertices);
                let middle = centre(&corners);
                let front = normalised(area_normal(&corners)).unwrap_or([0.0, 1.0, 0.0]);
                let p = add(middle, mul(front, 0.05));
                around
                    .room_at(p, rooms)
                    .map(|r| r.0)
                    .unwrap_or_else(|| nearest_room(middle))
            })
        })
        .collect();
    if strays > 0 {
        tracing::info!("{strays} triangles of no room went to the rooms they lie in");
    }

    let mut zones: Vec<Zone> = (0..rooms)
        .map(|r| Zone {
            name: scene.rooms[r].clone(),
            bounds: Bounds::EMPTY,
        })
        .collect();
    for (t, zone) in scene.triangles.iter().zip(&triangle_zones) {
        for v in &t.vertices {
            zones[*zone as usize].bounds.add(v.pos);
        }
    }

    // the portals, each turned so that its front is its first room's side
    let room_middles: Vec<Option<Vec3>> = zones
        .iter()
        .map(|z| (!z.bounds.is_empty()).then(|| z.bounds.center()))
        .collect();
    let mut portals = vec![];
    for (i, mut corners, mut normal) in quads {
        let portal = &scene.portals[i];
        let (a, b) = (portal.rooms[0] as u16, portal.rooms[1] as u16);
        let middle = mul(corners.iter().fold([0.0; 3], |s, p| add(s, *p)), 0.25);
        // which room is in front of the quad
        let mut towards_b = 0;
        for distance in [0.1, 0.3, 0.8] {
            for (sign, room_ahead) in [(1.0, b), (-1.0, a)] {
                match around.sure_room_at(add(middle, mul(normal, sign * distance)), rooms) {
                    Some(room) if room == room_ahead => towards_b += 1,
                    Some(room) if room == a || room == b => towards_b -= 1,
                    _ => {}
                }
            }
        }
        if towards_b == 0 {
            if let (Some(ma), Some(mb)) = (room_middles[a as usize], room_middles[b as usize]) {
                towards_b = if dot(sub(mb, ma), normal) >= 0.0 { 1 } else { -1 };
            }
        }
        if towards_b < 0 {
            corners.reverse();
            normal = mul(normal, -1.0);
        }
        portals.push((
            ZonePortal {
                name: portal.name.clone(),
                zones: [a, b],
                corners,
            },
            normal,
        ));
    }

    for (r, zone) in zones.iter().enumerate() {
        if zone.bounds.is_empty() {
            warnings.push(format!("room {} has no triangles", zone.name));
        } else if rooms > 1 && !portals.iter().any(|p| p.0.zones.contains(&(r as u16))) {
            warnings.push(format!(
                "room {} has no portal: it is seen from inside only and nothing is seen from it",
                zone.name
            ));
        }
    }

    // points all over the rooms with the room they lie in
    let mut samples: Vec<Sample> = vec![];
    let surfaces: Vec<[Vec3; 3]> = scene.triangles.iter().map(|t| positions(&t.vertices)).collect();
    let total_area: f32 = surfaces.iter().map(|t| dot(area_normal(t), area_normal(t)).sqrt() * 0.5).sum();
    let area_each = (total_area / SURFACE_SAMPLES).max(0.25);
    for t in &surfaces {
        let twice = area_normal(t);
        let Some(front) = normalised(twice) else {
            continue;
        };
        let count = ((dot(twice, twice).sqrt() * 0.5 / area_each).round() as usize).clamp(1, 64);
        for p in points_on(t, count) {
            for offset in SURFACE_OFFSETS {
                let pos = add(p, mul(front, offset));
                if let Some(room) = around.sure_room_at(pos, rooms) {
                    samples.push(Sample { pos, room });
                }
            }
        }
    }
    let bounds = Bounds::of(surfaces.iter().flatten().copied()).or_zero();
    let size = sub(bounds.max, bounds.min);
    let step = ((size[0] * size[1] * size[2]) / GRID_SAMPLES).cbrt().max(1.0);
    let steps = size.map(|s| (s / step).ceil() as usize);
    for x in 0..steps[0] {
        for y in 0..steps[1] {
            for z in 0..steps[2] {
                let pos = [
                    bounds.min[0] + (x as f32 + 0.5) * step,
                    bounds.min[1] + (y as f32 + 0.5) * step,
                    bounds.min[2] + (z as f32 + 0.5) * step,
                ];
                if let Some(room) = around.sure_room_at(pos, rooms) {
                    samples.push(Sample { pos, room });
                }
            }
        }
    }
    // and on both sides of every portal, where the rooms are known
    for (portal, normal) in &portals {
        let middle = mul(portal.corners.iter().fold([0.0; 3], |s, p| add(s, *p)), 0.25);
        let spots = portal
            .corners
            .iter()
            .map(|c| add(middle, mul(sub(*c, middle), 0.85)))
            .chain([middle]);
        for spot in spots {
            for distance in [0.05, 0.3] {
                samples.push(Sample {
                    pos: add(spot, mul(*normal, distance)),
                    room: portal.zones[1],
                });
                samples.push(Sample {
                    pos: sub(spot, mul(*normal, distance)),
                    room: portal.zones[0],
                });
            }
        }
    }

    let mut planes: Vec<Cut> = portals
        .iter()
        .map(|(portal, normal)| Cut {
            plane: [normal[0], normal[1], normal[2], -dot(*normal, portal.corners[0])],
            kind: KIND_PORTAL,
        })
        .collect();
    planes.extend(wall_planes(&surfaces));

    let mut builder = TreeBuilder {
        samples: &samples,
        rooms,
        planes,
        nodes: vec![],
    };
    let root = if samples.is_empty() {
        0
    } else {
        builder.build((0..samples.len() as u32).collect(), 0)
    };
    let mut tree = builder.nodes;
    if tree.is_empty() {
        // one room, or nothing known: everywhere is the same zone
        tree.push(ZoneTreeNode {
            plane: [1.0, 0.0, 0.0, 0.0],
            children: [root.min(0), root.min(0)],
        });
    }

    let zoning = Zoning {
        zones,
        triangle_zones,
        portals: portals.into_iter().map(|p| p.0).collect(),
        tree,
        warnings,
    };
    let wrong = samples.iter().filter(|s| zoning.zone_at(s.pos) != s.room).count();
    tracing::info!(
        "{} rooms, {} portals, a tree of {} planes from {} points ({} of them in another room by it)",
        zoning.zones.len(),
        zoning.portals.len(),
        zoning.tree.len(),
        samples.len(),
        wrong
    );
    Some(zoning)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ge::geomap::{ScenePortal, SceneTriangle};

    /// A rectangle from `p` along `u` and `v`, its front where `u x v` points
    fn rectangle(p: Vec3, u: Vec3, v: Vec3) -> [[Vec3; 3]; 2] {
        let (a, b, c, d) = (p, add(p, u), add(add(p, u), v), add(p, v));
        [[a, b, c], [a, c, d]]
    }

    /// The inside of a box, without the part of its wall at `x = max` and of its ceiling that
    /// `door` and `hatch` say (z from, z to, height / x from, x to)
    fn room(min: Vec3, max: Vec3, door: Option<[f32; 3]>, hatch: Option<[f32; 2]>) -> Vec<[Vec3; 3]> {
        let size = sub(max, min);
        let (x, y, z) = ([size[0], 0.0, 0.0], [0.0, size[1], 0.0], [0.0, 0.0, size[2]]);
        let mut quads = vec![
            rectangle(min, z, x),
            rectangle(min, y, z),
            rectangle(min, x, y),
            rectangle([min[0], min[1], max[2]], y, x),
        ];
        match hatch {
            Some([from, to]) => {
                quads.push(rectangle([min[0], max[1], min[2]], [from - min[0], 0.0, 0.0], z));
                quads.push(rectangle([to, max[1], min[2]], [max[0] - to, 0.0, 0.0], z));
            }
            None => quads.push(rectangle([min[0], max[1], min[2]], x, z)),
        }
        match door {
            Some([from, to, height]) => {
                quads.push(rectangle([max[0], min[1], min[2]], [0.0, 0.0, from - min[2]], y));
                quads.push(rectangle([max[0], min[1], to], [0.0, 0.0, max[2] - to], y));
                quads.push(rectangle(
                    [max[0], min[1] + height, from],
                    [0.0, 0.0, to - from],
                    [0.0, size[1] - height, 0.0],
                ));
            }
            None => quads.push(rectangle([max[0], min[1], min[2]], z, y)),
        }
        quads.into_iter().flatten().collect()
    }

    fn scene() -> GeScene {
        let mut scene = GeScene::default();
        // room 0, room 1 behind a wall without thickness at x = 6 and room 2 on top of room 0
        let rooms = [
            room([0.0, 0.0, 0.0], [6.0, 3.0, 6.0], Some([2.0, 3.2, 2.2]), Some([1.0, 2.0])),
            room([6.0, 0.0, 0.0], [10.0, 3.0, 6.0], None, None)
                .into_iter()
                // its wall at x = 6 is open where the door is: left out as a whole here, the
                // other room's wall stands there
                .filter(|t| t.iter().any(|p| p[0] > 6.01))
                .collect(),
            room([0.0, 3.0, 0.0], [6.0, 6.0, 6.0], None, None)
                .into_iter()
                .filter(|t| t.iter().any(|p| p[1] > 3.01))
                .collect(),
        ];
        for (index, triangles) in rooms.into_iter().enumerate() {
            scene.rooms.push(format!("Room{index}"));
            for t in triangles {
                scene.triangles.push(SceneTriangle {
                    texture: None,
                    vertices: t.map(|pos| GeVertex {
                        pos,
                        normal: [0.0, 1.0, 0.0],
                        uv: [0.0; 2],
                        color: [128; 4],
                    }),
                    no_collision: false,
                    two_sided: false,
                    lightmap: None,
                    room: Some(index),
                });
            }
        }
        scene.portals.push(ScenePortal {
            name: "Portal_1_0".to_string(),
            rooms: [1, 0],
            triangles: rectangle([6.0, 0.0, 2.0], [0.0, 0.0, 1.2], [0.0, 2.2, 0.0]).to_vec(),
        });
        scene.portals.push(ScenePortal {
            name: "Portal_0_2".to_string(),
            rooms: [0, 2],
            triangles: rectangle([1.0, 3.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 6.0]).to_vec(),
        });
        scene
    }

    #[test]
    fn rooms_are_found() {
        let zoning = make_zoning(&scene()).unwrap();
        assert_eq!(zoning.zones.len(), 3);
        assert_eq!(zoning.portals.len(), 2);
        assert!(zoning.warnings.is_empty(), "{:?}", zoning.warnings);
        let places = [
            ([3.0, 1.5, 3.0], 0),
            ([5.9, 1.5, 5.0], 0),
            ([6.1, 1.5, 5.0], 1),
            ([5.95, 1.0, 2.6], 0),
            ([6.05, 1.0, 2.6], 1),
            ([9.0, 0.0, 1.0], 1),
            ([3.0, 0.0, 3.0], 0),
            ([3.0, 2.9, 3.0], 0),
            ([3.0, 3.0, 3.0], 2),
            ([1.5, 2.95, 3.0], 0),
            ([1.5, 3.05, 3.0], 2),
            ([5.0, 5.5, 5.0], 2),
        ];
        for (place, zone) in places {
            assert_eq!(zoning.zone_at(place), zone, "at {place:?}");
        }
    }

    #[test]
    fn portals_face_their_second_room() {
        let zoning = make_zoning(&scene()).unwrap();
        for portal in &zoning.portals {
            let c = &portal.corners;
            let normal = cross(sub(c[1], c[0]), sub(c[2], c[0]));
            let middle = mul(c.iter().fold([0.0; 3], |s, p| add(s, *p)), 0.25);
            let ahead = add(middle, mul(normalised(normal).unwrap(), 0.3));
            assert_eq!(zoning.zone_at(ahead), portal.zones[1], "{}", portal.name);
        }
    }
}
