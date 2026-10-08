//! What a level's entities say can be vaulted over, climbed onto and climbed, read from a file.
//!
//! An entity's word at +0x40 has a bit for each "variant" in its low byte and the offset to a
//! word for each in the rest; variant 6 is a list of edges (chains of points, a short of flags
//! for each edge) and polygons (a ladder's pieces). `geomap` writes them, this reads them back.

use super::geomap::{EDGE_CLIMB, EDGE_LADDER_TOP, EDGE_LONG_VAULT, EDGE_VAULT, POLYGON_RUNG};
use super::writer::{read_f32, read_rel, read_u16, read_u32};

const EDB_MAGIC: u32 = 0x47454F4D;
const HEADER_REFPTRS: usize = 0x48;
const HEADER_ENTITIES: usize = 0x50;
const VARIANT_EDGES: u32 = 6;

#[derive(Clone, Debug, PartialEq)]
pub struct GeEdge {
    pub from: [f32; 3],
    pub to: [f32; 3],
    pub flags: u16,
}

#[derive(Clone, Debug, PartialEq)]
pub struct GePolygon {
    pub corners: Vec<[f32; 3]>,
    /// A bit for each side nothing joins
    pub open: u8,
    pub flags: u16,
}

/// The edges and polygons of one entity, in the entity's own space
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GeEdges {
    /// Where the entity is in the file
    pub entity: usize,
    /// A zone's own entity (0x608): its space is the level's. Any other is a mesh that is
    /// placed somewhere
    pub zone: bool,
    pub edges: Vec<GeEdge>,
    pub polygons: Vec<GePolygon>,
}

/// A ladder as its pieces make it up
#[derive(Clone, Debug, PartialEq)]
pub struct GeLadder {
    pub pieces: usize,
    /// The middle of its lowest and of its highest side
    pub foot: [f32; 3],
    pub top: [f32; 3],
}

impl GeEdges {
    /// The ladders: the pieces that touch one another are one
    pub fn ladders(&self) -> Vec<GeLadder> {
        let rungs: Vec<&GePolygon> = self.polygons.iter().filter(|r| r.flags & POLYGON_RUNG != 0 && !r.corners.is_empty()).collect();
        let touch = |a: &GePolygon, b: &GePolygon| {
            a.corners.iter().any(|p| b.corners.iter().any(|q| (0..3).all(|k| (p[k] - q[k]).abs() < 0.05)))
        };
        let mut group: Vec<usize> = (0..rungs.len()).collect();
        for i in 0..rungs.len() {
            for j in 0..i {
                if group[i] != group[j] && touch(rungs[i], rungs[j]) {
                    let (from, to) = (group[i], group[j]);
                    group.iter_mut().filter(|g| **g == from).for_each(|g| *g = to);
                }
            }
        }
        let mut ladders = vec![];
        for g in 0..rungs.len() {
            let corners: Vec<[f32; 3]> = (0..rungs.len()).filter(|i| group[*i] == g).flat_map(|i| rungs[i].corners.clone()).collect();
            if corners.is_empty() {
                continue;
            }
            let low = corners.iter().map(|c| c[1]).fold(f32::MAX, f32::min);
            let high = corners.iter().map(|c| c[1]).fold(f32::MIN, f32::max);
            let middle = |y: f32| {
                let at: Vec<&[f32; 3]> = corners.iter().filter(|c| (c[1] - y).abs() < 0.01).collect();
                [0, 1, 2].map(|k| at.iter().map(|c| c[k]).sum::<f32>() / at.len() as f32)
            };
            ladders.push(GeLadder {
                pieces: group.iter().filter(|x| **x == g).count(),
                foot: middle(low),
                top: middle(high),
            });
        }
        ladders
    }
}

/// What an edge's flags are called
pub fn edge_name(flags: u16) -> &'static str {
    match flags {
        f if f & EDGE_LADDER_TOP != 0 => "ladder top",
        f if f & EDGE_CLIMB != 0 => "climb",
        f if f & EDGE_LONG_VAULT != 0 => "long vault",
        f if f & EDGE_VAULT != 0 => "vault",
        f if f & POLYGON_RUNG != 0 => "ladder",
        _ => "edge",
    }
}

/// Every entity's edges. Empty for a file that isn't a geometry file of the game or has none
pub fn read_edges(data: &[u8]) -> Vec<GeEdges> {
    if data.len() < 0xD8 || read_u32(data, 0) != EDB_MAGIC {
        return vec![];
    }
    let mut entities: Vec<(usize, bool)> = vec![];
    for (head, size, zone) in [(HEADER_REFPTRS, 16, true), (HEADER_ENTITIES, 20, false)] {
        let count = read_u16(data, head) as i16;
        let Some(list) = read_rel(data, head + 4) else {
            continue;
        };
        for i in 0..count.max(0) as usize {
            let entry = list + i * size;
            if entry + size > data.len() {
                break;
            }
            let address = read_u32(data, entry + 8) as usize;
            if address != 0 && !entities.iter().any(|(a, _)| *a == address) {
                entities.push((address, zone));
            }
        }
    }
    entities.into_iter().filter_map(|(entity, zone)| read_entity(data, entity, zone)).collect()
}

fn point(data: &[u8], at: usize) -> [f32; 3] {
    [read_f32(data, at), read_f32(data, at + 4), read_f32(data, at + 8)]
}

fn read_entity(data: &[u8], entity: usize, zone: bool) -> Option<GeEdges> {
    let variants = entity + 0x40;
    if variants + 4 > data.len() || read_u32(data, entity) & 0xFFF0 != 0x600 {
        return None;
    }
    let word = read_u32(data, variants);
    if word & 1 << VARIANT_EDGES == 0 {
        return None;
    }
    // a word for each variant the entity has, in the order of their bits
    let before = (word & ((1 << VARIANT_EDGES) - 1)).count_ones() as usize;
    let entry = variants + (word >> 8) as usize + before * 4;
    if entry + 4 > data.len() {
        return None;
    }
    let header = entry + (read_u32(data, entry) >> 8) as usize;
    if header + 0x14 > data.len() {
        return None;
    }
    let mut out = GeEdges { entity, zone, ..Default::default() };
    let mut at = read_rel(data, header + 4).unwrap_or(data.len());
    for _ in 0..read_u32(data, header) {
        if at + 16 > data.len() {
            break;
        }
        let count = data[at] as usize;
        if at + (count + 1) * 16 > data.len() {
            break;
        }
        for k in 0..count {
            let record = at + (k + 1) * 16;
            out.edges.push(GeEdge {
                from: point(data, record - 12),
                to: point(data, record + 4),
                flags: read_u16(data, record + 2),
            });
        }
        at += (count + 1) * 16;
    }
    let mut at = read_rel(data, header + 12).unwrap_or(data.len());
    for _ in 0..read_u32(data, header + 8) {
        if at + 16 > data.len() {
            break;
        }
        let count = data[at] as usize;
        if at + (count + 1) * 16 > data.len() {
            break;
        }
        out.polygons.push(GePolygon {
            corners: (1..=count).map(|k| point(data, at + k * 16)).collect(),
            open: data[at + 1],
            flags: read_u16(data, at + 2),
        });
        at += (count + 1) * 16;
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ge::geomap::{build_geometry_file, GeScene, SceneEdge, SceneLadder, SceneTriangle};

    /// What `geomap` writes comes back: a vault's edge, and a ladder as its top and its pieces,
    /// the lowest half a unit above the floor
    #[test]
    fn written_edges_are_read_back() {
        let floor = [[-8.0, 0.0, -8.0], [-8.0, 0.0, 8.0], [8.0, 0.0, 8.0], [8.0, 0.0, -8.0]];
        let triangle = |a: usize, b: usize, c: usize| {
            [a, b, c].map(|i| crate::ge::mesh::GeVertex { pos: floor[i], normal: [0.0, 1.0, 0.0], uv: [0.0, 0.0], color: [0x80; 4] })
        };
        let mut scene = GeScene::default();
        for vertices in [triangle(0, 1, 2), triangle(0, 2, 3)] {
            scene.triangles.push(SceneTriangle { vertices, texture: None, no_collision: false, room: None, two_sided: false });
        }
        scene.edges.push(SceneEdge { from: [0.0, 1.1, 2.0], to: [1.0, 1.1, 2.0], flags: EDGE_VAULT });
        scene.ladders.push(SceneLadder {
            top: [[2.0, 3.0, 0.0], [2.0, 3.0, 0.4]],
            bottom: [[2.0, 0.0, 0.0], [2.0, 0.0, 0.4]],
        });
        let (file, stats) = build_geometry_file(&scene, 0x01810201, 0);
        assert_eq!(stats.ladders, 1);
        let read = read_edges(&file);
        assert_eq!(read.len(), 1);
        assert!(read[0].zone);
        assert_eq!(
            read[0].edges,
            vec![
                GeEdge { from: [0.0, 1.1, 2.0], to: [1.0, 1.1, 2.0], flags: EDGE_VAULT },
                GeEdge { from: [2.0, 3.0, 0.0], to: [2.0, 3.0, 0.4], flags: EDGE_LADDER_TOP },
            ]
        );
        let rungs = &read[0].polygons;
        assert_eq!(rungs.len(), 5);
        assert!(rungs.iter().all(|r| r.flags == POLYGON_RUNG && r.corners.len() == 4));
        assert_eq!((rungs[0].open, rungs[1].open), (0x0D, 0x05));
        assert_eq!((rungs[0].corners[0][1], rungs[4].corners[1][1]), (0.5, 3.0));
        assert_eq!(read[0].ladders(), vec![GeLadder { pieces: 5, foot: [2.0, 0.5, 0.2], top: [2.0, 3.0, 0.2] }]);
        assert!(stats.warnings.is_empty(), "{:?}", stats.warnings);

        // a wall behind it that looks the same way as the ladder is fine, one that looks the
        // other way means the quad was made the wrong way round
        let wall = |x: f32, flip: bool| {
            let corners = [[x, 0.0, -1.0], [x, 4.0, -1.0], [x, 4.0, 1.0], [x, 0.0, 1.0]];
            let order: [[usize; 3]; 2] = if flip { [[0, 2, 1], [0, 3, 2]] } else { [[0, 1, 2], [0, 2, 3]] };
            let normal = [if flip { -1.0 } else { 1.0 }, 0.0, 0.0];
            order.map(|o| SceneTriangle {
                vertices: o.map(|i| crate::ge::mesh::GeVertex { pos: corners[i], normal, uv: [0.0, 0.0], color: [0x80; 4] }),
                texture: None,
                no_collision: false,
                room: None,
                two_sided: false,
            })
        };
        let front = scene.ladders[0].front();
        let mut with_wall = scene.clone();
        with_wall.triangles.extend(wall(2.0, front[0] < 0.0));
        assert!(with_wall.fitted_ladders().1.is_empty());
        let mut wrong = scene.clone();
        wrong.triangles.extend(wall(2.0, front[0] > 0.0));
        assert!(wrong.fitted_ladders().1.iter().any(|w| w.contains("turn it around")));
    }
}
