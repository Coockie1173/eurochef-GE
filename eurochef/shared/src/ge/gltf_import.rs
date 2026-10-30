//! Reads a glTF scene (.gltf or .glb, with its textures) into triangles for a geometry file.
//!
//! glTF is right handed with counter clockwise fronts, the game's levels are the mirror image:
//! x is negated and every triangle turned around.
//!
//! Meshes or nodes whose name starts with `collision`, `col_` or `ucx_` aren't drawn: they are
//! what a body collides with, in place of the drawn triangles. A material with `nocollide` in
//! its name is left out of the collision that is made from the drawn triangles. Triangles are
//! drawn from their front only, whatever glTF's own double sided says (Blender sets it on every
//! material that isn't told otherwise): a material with `twosided` or `nocull` in its name is
//! drawn from behind as well. Meshes or nodes
//! whose name starts with `collision_add` or `col_add` aren't drawn either and are collided
//! with on top of whatever else is: a ramp over stairs whose steps are `nocollide`. Those named
//! `collision_add_steep` or `col_add_steep` can be stood on up to 60 degrees steep, not 37. A node whose
//! name starts with `spawn` or `player_start` is where the player starts. A node whose name
//! starts with `mp_spawn` is a spawn point of a multiplayer game, looking along the node's own
//! +z (Blender's -y); `mp_spawn_team0...` and `mp_spawn_team1...` are a team's. A node whose name
//! starts with `golden_gun`, `goldeneye` or `black_box` is where that gamemode's gun, one of its
//! consoles or its box is (see `project`: which gamemode gets what). A node whose name
//! starts with `reference` or `ref_` is left out with everything below it. A node whose name
//! starts with `sky` is the sky, with everything below it: drawn where it stands whichever room
//! the camera is in, never collided with and not shaded by the importer's light.
//!
//! A node whose name starts with `volume_` isn't drawn or collided with: it is a trigger volume
//! (see `volumes`), a box around its mesh turned as the node is, or for an empty the box Blender
//! draws for a cube empty.
//!
//! A node whose name starts with `bhop_` is no place: its words are the level's own bunnyhop
//! rules, which the port plays by in that level (see `volumes`): `bhop_speed6_gravity0.8`.
//!
//! A mesh or node whose name starts with `vault`, `vault_long` or `climb` isn't drawn or collided
//! with: it stands for the top of an obstacle (a quad or a box that ends where the obstacle's top
//! does) and the rim of its faces that look up become the edges the player vaults over (about
//! 1.25 or 1.6 past the edge) or climbs up onto, from outside. One whose name starts with
//! `ladder` isn't drawn or collided with either: a flat upright quad that looks at the player
//! on it, as wide and as tall as the ladder.
//!
//! Rooms and portals (see `rooms`): a node named `RoomXX` holds a room, with everything below
//! it (`Room01`, `Room_01`, `Room01_walls` and `Room01.001` are all room 01). A node named
//! `Portal_XX_YY` isn't drawn: its mesh, a flat quad in the opening, is the portal between
//! rooms XX and YY (`Portal_01_02.001` and `Portal_01_02_b` are more of them). Triangles that
//! are in no room's node go to the room they lie in.
//!
//! Baked light: a node whose name starts with `lightmap` is a copy of an object with the baked
//! image as its texture and the lightmap's coordinates as its texture coordinates. It isn't
//! drawn as it is: each of its triangles gives the level's triangle in the same place its
//! lightmap, which the game draws over it and darkens it by.
//!
//! Lamps: a material with emission is its own light. Its triangles are drawn as they are, with
//! no light from above and no lightmap, which could only darken them.

use std::{collections::HashMap, path::Path};

use anyhow::Context;
use image::RgbaImage;

use super::{
    volumes::{rotation_from_axes, volume_of_name, SceneVolume, VolumeKind},
    geomap::{ladder_of, rim_edges, GeScene, ModeItemKind, SceneModeItem, ScenePortal, SceneSpawn, SceneTriangle, EDGE_CLIMB, EDGE_LADDER_TOP, EDGE_LONG_VAULT, EDGE_VAULT},
    mesh::{GeVertex, COLOUR_ONE},
    texture::GeTexture,
};

#[derive(Clone, Debug)]
pub struct ImportOptions {
    /// Game units for a glTF unit (metres in both, so 1)
    pub scale: f32,
    /// Shade the vertex colours by a fixed light from above: the game draws levels with the
    /// light in their vertex colours, and a scene without any looks flat. A mesh whose vertex
    /// colours were painted has its light already and is left as it is
    pub bake_light: bool,
    /// Keep the vertex and material colours as the file has them. glTF's are linear and the game
    /// puts a colour's bytes on the screen as they are, so without this they are made sRGB
    /// (a linear 0.21 is the 0.5 grey it was painted as): kept linear a scene comes out dark
    pub linear_colours: bool,
    /// Draw from both sides what glTF's material says is double sided. Blender says so of every
    /// material whose backface culling isn't turned on, so without this only a material's name
    /// (`twosided`, `nocull`) makes it two sided
    pub gltf_double_sided: bool,
    /// Every vertex colour times this, the sky's too: 2 is twice as bright. A colour can't get
    /// brighter than the game's brightest, about twice the level's texture as it is
    pub brightness: f32,
    /// How many smaller copies a lightmap's texture gets, 0: none. The game draws what is far
    /// away with the smaller ones, in which a lightmap's parts run into each other and into the
    /// black between them
    pub lightmap_mips: u32,
    /// Keep a lightmap's texture as it is (RGBA8, eight times the size) in place of CMPR, whose
    /// 4 by 4 blocks of two colours show in soft light and where two faces meet in a block
    pub lightmap_uncompressed: bool,
    /// Lamps (materials with emission) glow: their textures get the game's bloom
    pub bloom: bool,
    /// A gun's parts in place of rooms: a node named `Part_N` (or `Body`, part 0) holds part N
    /// with everything below it, in the scene's `rooms` as its number. A skinned mesh's
    /// triangles are the part of the joint (bone `Part_N...`) they are weighted to, in its space
    pub parts: bool,
    /// With `parts`: what takes a point of the assembled gun into each part's own space (by the
    /// part's number, a glTF matrix). Given, a part's triangles go from where they are in the
    /// scene into that space, whatever the scene's bones are; empty, into the joint's space by
    /// the scene's inverse bind matrix (and an unweighted mesh as it is placed)
    pub part_spaces: Vec<[[f32; 4]; 4]>,
}

impl Default for ImportOptions {
    fn default() -> Self {
        Self {
            scale: 1.0,
            bake_light: true,
            linear_colours: false,
            gltf_double_sided: false,
            brightness: 1.0,
            lightmap_mips: 2,
            lightmap_uncompressed: false,
            bloom: true,
            parts: false,
            part_spaces: vec![],
        }
    }
}

const LIGHT_DIRECTION: [f32; 3] = [0.35, 0.85, 0.4];
const LIGHT_AMBIENT: f32 = 0.55;
const LIGHT_DIFFUSE: f32 = 0.45;
/// A vertex colour darker than this was painted
const PAINTED_BELOW: f32 = 0.98;

fn linear_to_srgb(c: f32) -> f32 {
    let c = c.clamp(0.0, 1.0);
    if c <= 0.0031308 {
        c * 12.92
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    }
}

fn is_collision_name(name: Option<&str>) -> bool {
    name.map(|n| {
        let n = n.to_lowercase();
        n.starts_with("collision") || n.starts_with("col_") || n.starts_with("ucx_")
    })
    .unwrap_or(false)
}

/// Collision that is added to the drawn triangles' (or to the `collision...` meshes')
fn is_added_collision_name(name: Option<&str>) -> bool {
    name.map(|n| {
        let n = n.to_lowercase();
        n.starts_with("collision_add") || n.starts_with("col_add")
    })
    .unwrap_or(false)
}

/// Added collision that is stood on up to 60 degrees steep
fn is_steep_collision_name(name: Option<&str>) -> bool {
    name.map(|n| {
        let n = n.to_lowercase();
        n.starts_with("collision_add_steep") || n.starts_with("col_add_steep")
    })
    .unwrap_or(false)
}

fn is_reference_name(name: Option<&str>) -> bool {
    name.map(|n| {
        let n = n.to_lowercase();
        n.starts_with("reference") || n.starts_with("ref_")
    })
    .unwrap_or(false)
}

/// The team of a node named `mp_spawn_teamN...`, None for any other `mp_spawn...`
fn team_of_name(name: &str) -> Option<u32> {
    let rest = name.strip_prefix("mp_spawn")?.trim_start_matches(['_', ' ', '-']).strip_prefix("team")?;
    let digits: String = rest.trim_start_matches(['_', ' ', '-']).chars().take_while(|c| c.is_ascii_digit()).collect();
    digits.parse().ok()
}

/// What a node named `golden_gun...`, `goldeneye...` or `black_box...` is: a gamemode's thing
fn mode_item_of_name(name: &str) -> Option<ModeItemKind> {
    if name.starts_with("golden_gun") || name.starts_with("goldengun") {
        Some(ModeItemKind::GoldenGun)
    } else if name.starts_with("goldeneye") {
        Some(ModeItemKind::Console)
    } else if name.starts_with("black_box") || name.starts_with("blackbox") {
        Some(ModeItemKind::BlackBox)
    } else {
        None
    }
}

/// What the edges of a mesh or node named `vault...`, `vault_long...`, `climb...` or `ladder...`
/// are
fn edge_of_name(name: Option<&str>) -> Option<u16> {
    let name = name?.to_lowercase();
    [
        ("vault_long", EDGE_LONG_VAULT),
        ("long_vault", EDGE_LONG_VAULT),
        ("vault", EDGE_VAULT),
        ("climb", EDGE_CLIMB),
        ("ladder", EDGE_LADDER_TOP),
    ]
        .into_iter()
        .find(|(prefix, _)| name.starts_with(prefix))
        .map(|(_, flags)| flags)
}

/// A copy of an object that is drawn with its baked light: the same triangles, the lightmap as
/// their one texture and where they are on it as their texture coordinates
fn is_lightmap_name(name: Option<&str>) -> bool {
    name.map(|n| n.to_lowercase().starts_with("lightmap")).unwrap_or(false)
}

/// A place to the millimetre: what two copies of a triangle are told to be the same by
type Place = [i32; 3];

fn place_of(p: [f32; 3]) -> Place {
    p.map(|v| (v * 1024.0).round() as i32)
}

fn places_of(corners: &[[f32; 3]; 3]) -> [Place; 3] {
    let mut places = corners.map(place_of);
    places.sort();
    places
}

/// A triangle of a lightmap copy: its texture, and each corner's place and where it is on the
/// texture
struct LightmapTriangle {
    texture: usize,
    corners: [(Place, [f32; 2]); 3],
    used: bool,
}

/// Gives every texel of a lightmap that no triangle uses the colour of the nearest one that is
/// used. The game reads a lightmap between its texels, in 4 by 4 blocks (CMPR) and in smaller
/// copies of itself: all three reach past a triangle's edge, into what was baked for nothing
/// (black) or for another face. `triangles` are the lightmap's own, in texture coordinates
fn spread_lightmap(image: &mut RgbaImage, triangles: &[[[f32; 2]; 3]]) {
    let (width, height) = (image.width() as i32, image.height() as i32);
    if width == 0 || height == 0 {
        return;
    }
    let mut used = vec![false; (width * height) as usize];
    for t in triangles {
        let p = t.map(|uv| [uv[0] * width as f32, uv[1] * height as f32]);
        let area = (p[1][0] - p[0][0]) * (p[2][1] - p[0][1]) - (p[2][0] - p[0][0]) * (p[1][1] - p[0][1]);
        if !area.is_finite() || area.abs() < 1e-6 {
            continue;
        }
        let low = |k: usize| p.iter().map(|c| c[k]).fold(f32::MAX, f32::min).floor() as i32;
        let high = |k: usize| p.iter().map(|c| c[k]).fold(f32::MIN, f32::max).ceil() as i32;
        for y in low(1)..=high(1) {
            for x in low(0)..=high(0) {
                // a texel is a triangle's when its middle is in it
                let (cx, cy) = (x as f32 + 0.5, y as f32 + 0.5);
                let side = |a: [f32; 2], b: [f32; 2]| (b[0] - a[0]) * (cy - a[1]) - (b[1] - a[1]) * (cx - a[0]);
                let w = [side(p[0], p[1]) / area, side(p[1], p[2]) / area, side(p[2], p[0]) / area];
                if w.iter().all(|v| *v >= -0.01) {
                    used[(y.rem_euclid(height) * width + x.rem_euclid(width)) as usize] = true;
                }
            }
        }
    }
    if !used.iter().any(|u| *u) {
        return;
    }
    // outwards from the used texels, a ring at a time
    let mut ring: Vec<(i32, i32)> = (0..height)
        .flat_map(|y| (0..width).map(move |x| (x, y)))
        .filter(|(x, y)| used[(y * width + x) as usize])
        .collect();
    while !ring.is_empty() {
        let mut next = vec![];
        for (x, y) in ring {
            let colour = *image.get_pixel(x as u32, y as u32);
            for (nx, ny) in [(x - 1, y), (x + 1, y), (x, y - 1), (x, y + 1)] {
                if nx < 0 || ny < 0 || nx >= width || ny >= height || used[(ny * width + nx) as usize] {
                    continue;
                }
                used[(ny * width + nx) as usize] = true;
                image.put_pixel(nx as u32, ny as u32, colour);
                next.push((nx, ny));
            }
        }
        ring = next;
    }
}

fn is_sky_name(name: Option<&str>) -> bool {
    name.map(|n| n.to_lowercase().starts_with("sky")).unwrap_or(false)
}

/// A name without what Blender puts behind a copy's (`.001`)
fn without_copy_number(name: &str) -> &str {
    match name.rsplit_once('.') {
        Some((head, tail)) if !tail.is_empty() && tail.chars().all(|c| c.is_ascii_digit()) => head,
        _ => name,
    }
}

/// How a room is told from another: its name's letters in lower case, a number without the
/// zeros in front (`01` and `1` are one room)
fn room_id(text: &str) -> String {
    let text = text.to_lowercase();
    if !text.is_empty() && text.chars().all(|c| c.is_ascii_digit()) {
        let number = text.trim_start_matches('0');
        return if number.is_empty() { "0".to_string() } else { number.to_string() };
    }
    text
}

/// The room a node named `RoomXX...` holds
fn room_of_name(name: Option<&str>) -> Option<String> {
    let name = without_copy_number(name?);
    if !name.get(..4)?.eq_ignore_ascii_case("room") {
        return None;
    }
    let id = name[4..].trim_start_matches(['_', ' ', '-']).split(['_', ' ']).next()?;
    (!id.is_empty()).then(|| room_id(id))
}

/// The gun part a node named `Part_N...` (`Part3`, `part_03`, `Part_3_slide`) or `Body...` holds:
/// its number
fn part_of_name(name: Option<&str>) -> Option<String> {
    let name = without_copy_number(name?);
    if name.get(..4)?.eq_ignore_ascii_case("body") {
        return Some("0".to_string());
    }
    if !name.get(..4)?.eq_ignore_ascii_case("part") {
        return None;
    }
    let id = name[4..].trim_start_matches(['_', ' ', '-']).split(['_', ' ', '-']).next()?;
    (!id.is_empty() && id.chars().all(|c| c.is_ascii_digit())).then(|| room_id(id))
}

/// How many times the game's size a gun's rig is drawn: a node named `Gun_xN`
fn gun_scale_of_name(name: Option<&str>) -> Option<f32> {
    let name = without_copy_number(name?);
    if !name.get(..5)?.eq_ignore_ascii_case("gun_x") {
        return None;
    }
    name[5..].parse::<f32>().ok().filter(|n| *n > 0.0)
}

/// A gun part's point an empty named `Point_HASH_Part_N` (`..._2` for the part's second point of
/// that hash) stands for: the hash, the part's number and which of its points of that hash
fn point_of_name(name: Option<&str>) -> Option<(u32, usize, usize)> {
    let name = without_copy_number(name?);
    if !name.get(..6)?.eq_ignore_ascii_case("point_") {
        return None;
    }
    let mut words = name[6..].split('_');
    let hash = u32::from_str_radix(words.next()?, 16).ok()?;
    if !words.next()?.eq_ignore_ascii_case("part") {
        return None;
    }
    let number = words.next()?.parse().ok()?;
    let nth = words.next().and_then(|w| w.parse::<usize>().ok()).map_or(0, |k| k.saturating_sub(1));
    Some((hash, number, nth))
}

/// The two rooms a node named `Portal_XX_YY...` joins
fn portal_of_name(name: Option<&str>) -> Option<[String; 2]> {
    let name = without_copy_number(name?);
    if !name.get(..6)?.eq_ignore_ascii_case("portal") {
        return None;
    }
    let mut ids = name[6..].split(['_', ' ', '-']).filter(|t| !t.is_empty());
    Some([room_id(ids.next()?), room_id(ids.next()?)])
}

fn to_rgba(image: &gltf::image::Data) -> anyhow::Result<RgbaImage> {
    use gltf::image::Format;
    let (w, h) = (image.width, image.height);
    let px = &image.pixels;
    let from = |channels: usize, f: &dyn Fn(&[u8]) -> [u8; 4]| -> anyhow::Result<RgbaImage> {
        anyhow::ensure!(px.len() >= (w * h) as usize * channels, "image data is cut short");
        Ok(RgbaImage::from_fn(w, h, |x, y| {
            let at = (y * w + x) as usize * channels;
            f(&px[at..at + channels]).into()
        }))
    };
    match image.format {
        Format::R8 => from(1, &|p| [p[0], p[0], p[0], 255]),
        Format::R8G8 => from(2, &|p| [p[0], p[0], p[0], p[1]]),
        Format::R8G8B8 => from(3, &|p| [p[0], p[1], p[2], 255]),
        Format::R8G8B8A8 => from(4, &|p| [p[0], p[1], p[2], p[3]]),
        Format::R16 => from(2, &|p| [p[1], p[1], p[1], 255]),
        Format::R16G16 => from(4, &|p| [p[1], p[1], p[1], p[3]]),
        Format::R16G16B16 => from(6, &|p| [p[1], p[3], p[5], 255]),
        Format::R16G16B16A16 => from(8, &|p| [p[1], p[3], p[5], p[7]]),
        other => anyhow::bail!("texture format {other:?} isn't supported"),
    }
}

fn transform_point(m: &[[f32; 4]; 4], p: [f32; 3]) -> [f32; 3] {
    [
        m[0][0] * p[0] + m[1][0] * p[1] + m[2][0] * p[2] + m[3][0],
        m[0][1] * p[0] + m[1][1] * p[1] + m[2][1] * p[2] + m[3][1],
        m[0][2] * p[0] + m[1][2] * p[1] + m[2][2] * p[2] + m[3][2],
    ]
}

fn transform_direction(m: &[[f32; 4]; 4], p: [f32; 3]) -> [f32; 3] {
    let v = [
        m[0][0] * p[0] + m[1][0] * p[1] + m[2][0] * p[2],
        m[0][1] * p[0] + m[1][1] * p[1] + m[2][1] * p[2],
        m[0][2] * p[0] + m[1][2] * p[1] + m[2][2] * p[2],
    ];
    let len = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if len > 1e-9 {
        [v[0] / len, v[1] / len, v[2] / len]
    } else {
        [0.0, 1.0, 0.0]
    }
}

fn multiply(a: &[[f32; 4]; 4], b: &[[f32; 4]; 4]) -> [[f32; 4]; 4] {
    let mut res = [[0.0; 4]; 4];
    for (c, column) in res.iter_mut().enumerate() {
        for (r, cell) in column.iter_mut().enumerate() {
            *cell = (0..4).map(|k| a[k][r] * b[c][k]).sum();
        }
    }
    res
}

fn face_normal(t: &[[f32; 3]; 3]) -> [f32; 3] {
    let u = [t[1][0] - t[0][0], t[1][1] - t[0][1], t[1][2] - t[0][2]];
    let v = [t[2][0] - t[0][0], t[2][1] - t[0][1], t[2][2] - t[0][2]];
    transform_direction(
        &[[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0], [0.0, 0.0, 0.0, 1.0]],
        [
            u[1] * v[2] - u[2] * v[1],
            u[2] * v[0] - u[0] * v[2],
            u[0] * v[1] - u[1] * v[0],
        ],
    )
}

struct Importer<'a> {
    buffers: &'a [gltf::buffer::Data],
    options: &'a ImportOptions,
    scene: GeScene,
    /// The scene's texture for each glTF image that is used
    /// The scene's texture of each image of the file: plain, and the one that glows
    image_textures: Vec<[Option<usize>; 2]>,
    images: &'a [gltf::image::Data],
    white: [Option<usize>; 2],
    skipped_primitives: usize,
    /// The rooms as their names tell them apart, in the scene's order
    room_ids: Vec<String>,
    /// The portals, with the rooms they name
    portals: Vec<(String, [String; 2], Vec<[[f32; 3]; 3]>)>,
    /// The meshes that stand for an obstacle's top or a ladder: their name, the edges' flags,
    /// their triangles
    tops: Vec<(String, u16, Vec<[[f32; 3]; 3]>)>,
    /// The lightmap copies' triangles by their corners' places
    lightmaps: HashMap<[Place; 3], Vec<LightmapTriangle>>,
    lightmapped: usize,
    /// Triangles of lamps that had a lightmap and are drawn without it
    lamps_unmapped: usize,
}

/// A gun's rig: for each joint, what takes a vertex into its space and the part it is
#[derive(Default)]
struct SkinParts {
    inverse: Vec<[[f32; 4]; 4]>,
    rooms: Vec<Option<usize>>,
}

/// What a node is a part of, handed down to the nodes below it
#[derive(Clone, Copy, Default)]
struct Inherited {
    collision: bool,
    /// Its collision is added to the rest
    added: bool,
    /// Its collision is stood on up to 60 degrees steep
    steep: bool,
    room: Option<usize>,
    /// The portal its triangles are
    portal: Option<usize>,
    /// Its triangles are the sky's
    sky: bool,
    /// The obstacle's top its triangles are
    top: Option<usize>,
    /// A gun's rig drawn N times the game's size (`Gun_xN`): N (0 for none), what a skinned mesh,
    /// which its node doesn't place, is made smaller by
    shrink: f32,
}

impl Importer<'_> {
    /// `bloom`: for a lamp. A texture of its own then, a wall with the same image doesn't glow
    fn texture_for(&mut self, info: Option<gltf::texture::Info<'_>>, bloom: bool) -> anyhow::Result<usize> {
        let which = bloom as usize;
        if let Some(info) = info {
            let image = info.texture().source().index();
            if let Some(existing) = self.image_textures[image][which] {
                return Ok(existing);
            }
            let rgba = to_rgba(&self.images[image])
                .with_context(|| format!("texture {image} of the glTF"))?;
            let name = info
                .texture()
                .source()
                .name()
                .map(|n| n.to_string())
                .unwrap_or_else(|| format!("image{image}"));
            self.scene.textures.push(GeTexture {
                name,
                image: rgba,
                full_alpha: false,
                mip_levels: None,
                bloom,
            });
            let index = self.scene.textures.len() - 1;
            self.image_textures[image][which] = Some(index);
            return Ok(index);
        }

        // no texture: a white one, the material's colour goes into the vertices
        if self.white[which].is_none() {
            let mut texture = GeTexture::solid(if bloom { "white_bloom" } else { "white" }, [255, 255, 255, 255]);
            texture.bloom = bloom;
            self.scene.textures.push(texture);
            self.white[which] = Some(self.scene.textures.len() - 1);
        }
        Ok(self.white[which].unwrap())
    }

    /// The lightmap copies of the scene, read before anything else: a triangle is told whether
    /// it has baked light when it is read
    fn lightmap_nodes(&mut self, node: &gltf::Node<'_>, parent: &[[f32; 4]; 4], inside: bool) -> anyhow::Result<()> {
        if is_reference_name(node.name()) {
            return Ok(());
        }
        let matrix = multiply(parent, &node.transform().matrix());
        let inside = inside || is_lightmap_name(node.name());
        if let (true, Some(mesh)) = (inside, node.mesh()) {
            for primitive in mesh.primitives() {
                if primitive.mode() != gltf::mesh::Mode::Triangles {
                    continue;
                }
                let material = primitive.material();
                if material.pbr_metallic_roughness().base_color_texture().is_none() {
                    tracing::warn!(
                        "{} is a lightmap without a texture, left out",
                        node.name().unwrap_or("a node")
                    );
                    continue;
                }
                let texture = self.texture_for(material.pbr_metallic_roughness().base_color_texture(), false)?;
                self.scene.textures[texture].mip_levels = Some(self.options.lightmap_mips);
                self.scene.textures[texture].full_alpha = self.options.lightmap_uncompressed;
                let buffers = self.buffers;
                let reader = primitive.reader(|b| buffers.get(b.index()).map(|d| d.0.as_slice()));
                let (Some(positions), Some(uvs)) = (reader.read_positions(), reader.read_tex_coords(0)) else {
                    continue;
                };
                let scale = self.options.scale;
                let positions: Vec<[f32; 3]> = positions
                    .map(|p| {
                        let p = transform_point(&matrix, p);
                        [-p[0] * scale, p[1] * scale, p[2] * scale]
                    })
                    .collect();
                let uvs: Vec<[f32; 2]> = uvs.into_f32().collect();
                let indices: Vec<u32> = match reader.read_indices() {
                    Some(indices) => indices.into_u32().collect(),
                    None => (0..positions.len() as u32).collect(),
                };
                for t in indices.chunks_exact(3) {
                    let t = [t[0] as usize, t[1] as usize, t[2] as usize];
                    if t.iter().any(|i| *i >= positions.len() || *i >= uvs.len()) {
                        continue;
                    }
                    let corners = t.map(|i| positions[i]);
                    self.lightmaps.entry(places_of(&corners)).or_default().push(LightmapTriangle {
                        texture,
                        corners: t.map(|i| (place_of(positions[i]), uvs[i])),
                        used: false,
                    });
                }
            }
        }
        for child in node.children() {
            self.lightmap_nodes(&child, &matrix, inside)?;
        }
        Ok(())
    }

    /// The baked light of a triangle with these corners: a lightmap copy's triangle in the same
    /// place that no other has taken
    fn lightmap_of(&mut self, corners: &[[f32; 3]; 3]) -> Option<(usize, [[f32; 2]; 3])> {
        let copies = self.lightmaps.get_mut(&places_of(corners))?;
        let copy = copies.iter_mut().find(|c| !c.used)?;
        copy.used = true;
        self.lightmapped += 1;
        // corner by corner: the copy's triangle starts where it likes
        let mut taken = [false; 3];
        let uvs = corners.map(|p| {
            let place = place_of(p);
            let k = (0..3)
                .find(|k| !taken[*k] && copy.corners[*k].0 == place)
                .or_else(|| (0..3).find(|k| !taken[*k]))
                .unwrap_or(0);
            taken[k] = true;
            copy.corners[k].1
        });
        Some((copy.texture, uvs))
    }

    fn node(&mut self, node: &gltf::Node<'_>, parent: &[[f32; 4]; 4], inherited: Inherited) -> anyhow::Result<()> {
        // something to model against (a figure the player's size), not a part of the level
        if is_reference_name(node.name()) {
            return Ok(());
        }
        // a lightmap copy and what is below it: read already
        if is_lightmap_name(node.name()) {
            return Ok(());
        }
        let matrix = multiply(parent, &node.transform().matrix());
        if let Some(words) = node.name().and_then(|n| n.to_lowercase().strip_prefix("bhop_").map(str::to_string)) {
            self.scene.bhop.get_or_insert_with(Default::default).read_words(node.name().unwrap_or(""), &words)?;
            return Ok(());
        }
        if let Some((kind, sphere)) = volume_of_name(node.name().unwrap_or(""))? {
            let volume = self.volume(node, &matrix, kind, sphere)?;
            self.scene.volumes.push(volume);
            return Ok(());
        }
        if let Some((hash, number, nth)) = self.options.parts.then(|| point_of_name(node.name())).flatten() {
            let p = transform_point(&matrix, [0.0, 0.0, 0.0]);
            let scale = self.options.scale;
            self.scene.part_points.push((number, hash, nth, [-p[0] * scale, p[1] * scale, p[2] * scale]));
        }
        let mut part = inherited;
        part.steep |= is_steep_collision_name(node.name());
        part.added |= part.steep || is_added_collision_name(node.name());
        part.collision |= part.added || is_collision_name(node.name());
        let mesh_name = node.mesh().and_then(|m| m.name().map(|n| n.to_string()));
        part.sky |= is_sky_name(node.name()) || is_sky_name(mesh_name.as_deref());
        if !part.sky && part.portal.is_none() {
            if let Some(rooms) = portal_of_name(node.name()).or_else(|| portal_of_name(mesh_name.as_deref())) {
                let name = node.name().or(mesh_name.as_deref()).unwrap_or("portal").to_string();
                self.portals.push((name, rooms, vec![]));
                part.portal = Some(self.portals.len() - 1);
            } else if let Some(id) = if self.options.parts {
                part_of_name(node.name()).or_else(|| part_of_name(mesh_name.as_deref()))
            } else {
                room_of_name(node.name()).or_else(|| room_of_name(mesh_name.as_deref()))
            } {
                let shown = if self.options.parts {
                    id.clone()
                } else {
                    without_copy_number(node.name().or(mesh_name.as_deref()).unwrap_or("room")).to_string()
                };
                part.room = Some(self.room_for(id, shown));
            }
        }

        if part.top.is_none() {
            if let Some(flags) = edge_of_name(node.name()).or_else(|| edge_of_name(mesh_name.as_deref())) {
                let name = node.name().or(mesh_name.as_deref()).unwrap_or("vault").to_string();
                self.tops.push((name, flags, vec![]));
                part.top = Some(self.tops.len() - 1);
            }
        }

        let name = node.name().unwrap_or("").to_lowercase();
        if name.starts_with("mp_spawn") {
            let p = transform_point(&matrix, [0.0, 0.0, 0.0]);
            let ahead = transform_direction(&matrix, [0.0, 0.0, 1.0]);
            let scale = self.options.scale;
            self.scene.multiplayer_spawns.push(SceneSpawn {
                position: [-p[0] * scale, p[1] * scale, p[2] * scale],
                yaw: (-ahead[0]).atan2(ahead[2]),
                team: team_of_name(&name),
            });
        } else if name.starts_with("spawn") || name.starts_with("player_start") || name.starts_with("playerstart") {
            let p = transform_point(&matrix, [0.0, 0.0, 0.0]);
            let scale = self.options.scale;
            self.scene.spawn = Some([-p[0] * scale, p[1] * scale, p[2] * scale]);
        } else if let Some(kind) = mode_item_of_name(&name) {
            let p = transform_point(&matrix, [0.0, 0.0, 0.0]);
            let scale = self.options.scale;
            self.scene.mode_items.push(SceneModeItem {
                kind,
                name,
                position: [-p[0] * scale, p[1] * scale, p[2] * scale],
            });
        }

        if let Some(mesh) = node.mesh() {
            let mut part = part;
            part.steep |= is_steep_collision_name(mesh.name());
            part.added |= part.steep || is_added_collision_name(mesh.name());
            part.collision |= part.added || is_collision_name(mesh.name());
            let skin = if self.options.parts { node.skin().map(|s| self.skin_parts(&s)) } else { None };
            for primitive in mesh.primitives() {
                if primitive.mode() != gltf::mesh::Mode::Triangles {
                    self.skipped_primitives += 1;
                    continue;
                }
                self.primitive(&primitive, &matrix, part, skin.as_ref())?;
            }
        }

        for child in node.children() {
            self.node(&child, &matrix, part)?;
        }
        Ok(())
    }

    /// The box a `volume_...` node stands for, in the game's space
    fn volume(
        &self,
        node: &gltf::Node<'_>,
        matrix: &[[f32; 4]; 4],
        kind: VolumeKind,
        sphere: bool,
    ) -> anyhow::Result<SceneVolume> {
        let name = node.name().unwrap_or("volume").to_string();
        let (mut low, mut high) = ([f32::MAX; 3], [f32::MIN; 3]);
        if let Some(mesh) = node.mesh() {
            for primitive in mesh.primitives() {
                let reader = primitive.reader(|b| self.buffers.get(b.index()).map(|d| d.0.as_slice()));
                for p in reader.read_positions().into_iter().flatten() {
                    for k in 0..3 {
                        low[k] = low[k].min(p[k]);
                        high[k] = high[k].max(p[k]);
                    }
                }
            }
            anyhow::ensure!(low[0] <= high[0], "{name}: its mesh has no vertices");
        } else {
            (low, high) = ([-1.0; 3], [1.0; 3]);
        }
        let scale = self.options.scale;
        let centre = transform_point(matrix, [0, 1, 2].map(|k| (low[k] + high[k]) * 0.5));
        let mut half = [0.0; 3];
        let mut axes = [[0.0; 3]; 3];
        for k in 0..3 {
            let axis = [matrix[k][0], matrix[k][1], matrix[k][2]];
            let length = (axis[0] * axis[0] + axis[1] * axis[1] + axis[2] * axis[2]).sqrt();
            anyhow::ensure!(length > 1e-6, "{name}: its node is scaled to nothing");
            half[k] = (high[k] - low[k]) * 0.5 * length * scale;
            // mirrored as everything is, the box's own x with it
            let flip = if k == 0 { -1.0 } else { 1.0 };
            axes[k] = [-axis[0] * flip / length, axis[1] * flip / length, axis[2] * flip / length];
        }
        let cross = [
            axes[0][1] * axes[1][2] - axes[0][2] * axes[1][1],
            axes[0][2] * axes[1][0] - axes[0][0] * axes[1][2],
            axes[0][0] * axes[1][1] - axes[0][1] * axes[1][0],
        ];
        // scaled by a negative number: the box is the same turned the other way around
        if cross[0] * axes[2][0] + cross[1] * axes[2][1] + cross[2] * axes[2][2] < 0.0 {
            axes[2] = axes[2].map(|v| -v);
        }
        anyhow::ensure!(half.iter().all(|h| *h > 0.0) || sphere, "{name}: a volume can't be flat");
        Ok(SceneVolume {
            name,
            kind,
            sphere,
            position: [-centre[0] * scale, centre[1] * scale, centre[2] * scale],
            half,
            rotation: rotation_from_axes(axes),
        })
    }

    /// A room for the rooms' (or parts') id, the one it already has or a new one
    fn room_for(&mut self, id: String, shown: String) -> usize {
        match self.room_ids.iter().position(|r| *r == id) {
            Some(room) => room,
            None => {
                self.room_ids.push(id);
                self.scene.rooms.push(shown);
                self.room_ids.len() - 1
            }
        }
    }

    /// A gun's rig: each joint's inverse bind matrix (what takes a vertex into the bone's own
    /// space) and the part its name says it is
    fn skin_parts(&mut self, skin: &gltf::Skin<'_>) -> SkinParts {
        let buffers = self.buffers;
        let reader = skin.reader(|b| buffers.get(b.index()).map(|d| d.0.as_slice()));
        let inverse: Vec<[[f32; 4]; 4]> = reader.read_inverse_bind_matrices().map(|m| m.collect()).unwrap_or_default();
        let identity = [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0], [0.0, 0.0, 0.0, 1.0]];
        let mut parts = SkinParts::default();
        for (i, joint) in skin.joints().enumerate() {
            parts.inverse.push(inverse.get(i).copied().unwrap_or(identity));
            let room = part_of_name(joint.name()).map(|id| self.room_for(id.clone(), id));
            parts.rooms.push(room);
        }
        parts
    }

    fn primitive(
        &mut self,
        primitive: &gltf::Primitive<'_>,
        matrix: &[[f32; 4]; 4],
        part: Inherited,
        skin: Option<&SkinParts>,
    ) -> anyhow::Result<()> {
        let buffers = self.buffers;
        let reader = primitive.reader(|b| buffers.get(b.index()).map(|d| d.0.as_slice()));
        let Some(positions) = reader.read_positions() else {
            self.skipped_primitives += 1;
            return Ok(());
        };
        // a rigged gun: each vertex is its heaviest joint's, in that joint's own space (the
        // mesh node's place doesn't count for a skinned mesh)
        // one weighted to nothing (put into a rigged gun afterwards) is the part its node names
        let joints: Option<Vec<Option<usize>>> = skin.and_then(|skin| {
            let joints: Vec<[u16; 4]> = reader.read_joints(0)?.into_u16().collect();
            let weights: Vec<[f32; 4]> = reader.read_weights(0)?.into_f32().collect();
            Some(
                joints
                    .iter()
                    .zip(&weights)
                    .map(|(j, w)| {
                        let heaviest = (0..4).fold(0, |best, k| if w[k] > w[best] { k } else { best });
                        (w[heaviest] > 1e-6).then(|| (j[heaviest] as usize).min(skin.inverse.len().saturating_sub(1)))
                    })
                    .collect(),
            )
        });
        // with the base's part spaces: from where the triangles are on the assembled gun (a
        // weighted vertex as it is bound, another as its node places it) into the part's space,
        // the bones' own matrices left out (Blender turns bones as it likes). One under no
        // part's node is part 0's
        let space = |room: Option<usize>| -> Option<[[f32; 4]; 4]> {
            let number: usize = self.room_ids.get(room?)?.parse().ok()?;
            self.options.part_spaces.get(number).copied()
        };
        let in_part = match space(part.room.or_else(|| self.room_ids.iter().position(|id| id == "0"))) {
            Some(space) => multiply(&space, matrix),
            None => *matrix,
        };
        let shrink = if part.shrink > 0.0 { 1.0 / part.shrink } else { 1.0 };
        let smaller = [[shrink, 0.0, 0.0, 0.0], [0.0, shrink, 0.0, 0.0], [0.0, 0.0, shrink, 0.0], [0.0, 0.0, 0.0, 1.0]];
        let joint_spaces: Vec<[[f32; 4]; 4]> = skin
            .map(|skin| {
                skin.inverse.iter().zip(&skin.rooms).map(|(inverse, room)| multiply(&space(*room).unwrap_or(*inverse), &smaller)).collect()
            })
            .unwrap_or_default();
        let joint_of = |i: usize| joints.as_ref().and_then(|joints| joints.get(i).copied().flatten());
        let vertex_matrix = |i: usize| match joint_of(i) {
            Some(j) => joint_spaces.get(j).unwrap_or(&in_part),
            None => &in_part,
        };
        let room_of = |i: usize| match (joint_of(i), skin) {
            (Some(j), Some(skin)) => skin.rooms.get(j).copied().flatten(),
            _ => part.room,
        };
        let scale = self.options.scale;
        // mirrored: x is negated here, the triangles are turned around below
        let to_scene = |m: &[[f32; 4]; 4], p: [f32; 3]| {
            let p = transform_point(m, p);
            [-p[0] * scale, p[1] * scale, p[2] * scale]
        };
        let raw: Vec<[f32; 3]> = positions.collect();
        let positions: Vec<[f32; 3]> = raw.iter().enumerate().map(|(i, p)| to_scene(vertex_matrix(i), *p)).collect();
        // a triangle is the part most of its corners are (the second's when the last two agree,
        // else the first's), and all its corners go into that part's joint's space: a corner
        // weighted to another joint (where the slide meets the frame) would be off by the
        // difference between the two, a sliver out of the gun
        let chosen = |t: &[usize; 3]| -> Option<usize> {
            joints.as_ref()?;
            Some(if room_of(t[1]) == room_of(t[2]) { 1 } else { 0 })
        };
        let indices: Vec<u32> = match reader.read_indices() {
            Some(indices) => indices.into_u32().collect(),
            None => (0..positions.len() as u32).collect(),
        };
        let triangles = indices
            .chunks_exact(3)
            .filter(|t| t.iter().all(|i| (*i as usize) < positions.len()))
            .map(|t| [t[0] as usize, t[2] as usize, t[1] as usize]);

        if let Some(top) = part.top {
            for t in triangles {
                self.tops[top].2.push([positions[t[0]], positions[t[1]], positions[t[2]]]);
            }
            return Ok(());
        }
        let is_sky = part.sky;
        if let (Some(portal), false) = (part.portal, is_sky) {
            for t in triangles {
                self.portals[portal].2.push([positions[t[0]], positions[t[1]], positions[t[2]]]);
            }
            return Ok(());
        }
        if part.collision && !is_sky {
            let list = if part.steep {
                &mut self.scene.steep_collision
            } else if part.added {
                &mut self.scene.added_collision
            } else {
                &mut self.scene.collision
            };
            for t in triangles {
                list.push([positions[t[0]], positions[t[1]], positions[t[2]]]);
            }
            return Ok(());
        }

        let normals: Option<Vec<[f32; 3]>> = reader.read_normals().map(|n| n.collect());
        let uvs: Option<Vec<[f32; 2]>> = reader.read_tex_coords(0).map(|t| t.into_f32().collect());
        let colours: Option<Vec<[f32; 4]>> = reader.read_colors(0).map(|c| c.into_rgba_f32().collect());
        // vertex colours that aren't all white are the level's light as it was painted: the
        // importer's own on top of it would shade it twice
        let painted = colours
            .as_ref()
            .map(|c| c.iter().any(|c| c[..3].iter().any(|v| *v < PAINTED_BELOW)))
            .unwrap_or(false);

        let material = primitive.material();
        let mut factor = material.pbr_metallic_roughness().base_color_factor();
        let mut picture = material.pbr_metallic_roughness().base_color_texture();
        // a lamp: what glows is what is seen. Its own picture if the glow has one, the
        // material's if it has that, the glow's colour alone if neither
        let glow = material.emissive_factor();
        let emissive = glow.iter().any(|v| *v > 0.0);
        if emissive && (material.emissive_texture().is_some() || picture.is_none()) {
            picture = material.emissive_texture();
            for k in 0..3 {
                factor[k] = glow[k].min(1.0);
            }
        }
        let texture = self.texture_for(picture, emissive && self.options.bloom && !is_sky)?;
        let material_name = material.name().map(|n| n.to_lowercase()).unwrap_or_default();
        let no_collision = material_name.contains("nocollide");
        let two_sided = (self.options.gltf_double_sided && material.double_sided())
            || ["twosided", "two_sided", "doublesided", "double_sided", "nocull"]
                .iter()
                .any(|tag| material_name.contains(tag));

        for t in triangles {
            let corner_matrix = |k: usize| vertex_matrix(chosen(&t).map_or(t[k], |c| t[c]));
            let corners = [0, 1, 2].map(|k| to_scene(corner_matrix(k), raw[t[k]]));
            let flat = face_normal(&corners);
            // baked light is the triangle's light: the importer's own would shade it twice
            let mut lightmap = if is_sky { None } else { self.lightmap_of(&corners) };
            if emissive && lightmap.take().is_some() {
                self.lamps_unmapped += 1;
            }
            let vertices = [0, 1, 2].map(|k| {
                let i = t[k];
                let normal = normals
                    .as_ref()
                    .and_then(|n| n.get(i))
                    .map(|n| {
                        let n = transform_direction(corner_matrix(k), *n);
                        [-n[0], n[1], n[2]]
                    })
                    .filter(|n| n.iter().all(|v| v.is_finite()))
                    .unwrap_or(flat);
                let mut colour = colours.as_ref().and_then(|c| c.get(i)).copied().unwrap_or([1.0; 4]);
                for k in 0..4 {
                    colour[k] *= factor[k];
                }
                if !self.options.linear_colours {
                    for c in colour.iter_mut().take(3) {
                        *c = linear_to_srgb(*c);
                    }
                }
                if self.options.bake_light && !is_sky && !painted && !emissive && lightmap.is_none() {
                    let light = transform_direction(
                        &[[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0], [0.0; 4]],
                        LIGHT_DIRECTION,
                    );
                    let facing = (normal[0] * light[0] + normal[1] * light[1] + normal[2] * light[2]).max(0.0);
                    let shade = LIGHT_AMBIENT + LIGHT_DIFFUSE * facing;
                    for c in colour.iter_mut().take(3) {
                        *c *= shade;
                    }
                }
                for c in colour.iter_mut().take(3) {
                    *c *= self.options.brightness;
                }
                GeVertex {
                    pos: corners[k],
                    normal,
                    uv: uvs.as_ref().and_then(|u| u.get(i)).copied().unwrap_or([0.0, 0.0]),
                    color: [
                        (colour[0] * COLOUR_ONE).round().clamp(0.0, 255.0) as u8,
                        (colour[1] * COLOUR_ONE).round().clamp(0.0, 255.0) as u8,
                        (colour[2] * COLOUR_ONE).round().clamp(0.0, 255.0) as u8,
                        (colour[3] * 255.0).round().clamp(0.0, 255.0) as u8,
                    ],
                }
            });
            let triangle = SceneTriangle {
                texture: Some(texture),
                vertices,
                no_collision: no_collision || is_sky,
                // the part most of its corners are
                room: match chosen(&t) {
                    Some(c) => room_of(t[c]),
                    None => part.room,
                },
                two_sided,
                lightmap,
            };
            if is_sky {
                self.scene.sky.push(triangle);
            } else {
                self.scene.triangles.push(triangle);
            }
        }
        Ok(())
    }
}

/// Reads the default scene of a glTF file (all scenes when it names none)
pub fn import_gltf<P: AsRef<Path>>(path: P, options: &ImportOptions) -> anyhow::Result<GeScene> {
    let path = path.as_ref();
    let (document, buffers, images) =
        gltf::import(path).with_context(|| format!("couldn't read {}", path.display()))?;

    let mut importer = Importer {
        buffers: &buffers,
        options,
        scene: GeScene::default(),
        image_textures: vec![[None; 2]; images.len()],
        images: &images,
        white: [None; 2],
        skipped_primitives: 0,
        room_ids: vec![],
        portals: vec![],
        tops: vec![],
        lightmaps: HashMap::new(),
        lightmapped: 0,
        lamps_unmapped: 0,
    };

    let identity = [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0], [0.0, 0.0, 0.0, 1.0]];
    let scenes: Vec<gltf::Scene<'_>> = match document.default_scene() {
        Some(scene) => vec![scene],
        None => document.scenes().collect(),
    };
    for scene in &scenes {
        for node in scene.nodes() {
            importer.lightmap_nodes(&node, &identity, false)?;
        }
    }
    // a gun's rig is drawn bigger than the game's (`Gun_x10`): the whole scene is that much
    // smaller in the game, a model put in beside it too
    let shrink = if options.parts { document.nodes().find_map(|n| gun_scale_of_name(n.name())) } else { None };
    let mut top = identity;
    if let Some(times) = shrink {
        for k in 0..3 {
            top[k][k] = 1.0 / times;
        }
    }
    for scene in &scenes {
        for node in scene.nodes() {
            importer.node(&node, &top, Inherited { shrink: shrink.unwrap_or(0.0), ..Default::default() })?;
        }
    }
    let mut by_texture: HashMap<usize, Vec<[[f32; 2]; 3]>> = HashMap::new();
    for t in importer.lightmaps.values().flatten() {
        by_texture.entry(t.texture).or_default().push(t.corners.map(|c| c.1));
    }
    for (texture, triangles) in &by_texture {
        spread_lightmap(&mut importer.scene.textures[*texture].image, triangles);
    }
    let unused = importer.lightmaps.values().flatten().filter(|t| !t.used).count();
    if importer.lightmapped > 0 || unused > 0 {
        tracing::info!("{} triangle(s) with baked light", importer.lightmapped - importer.lamps_unmapped);
    }
    if importer.lamps_unmapped > 0 {
        tracing::info!(
            "{} triangle(s) of lamps (materials with emission) had baked light, drawn without it",
            importer.lamps_unmapped
        );
    }
    if unused > 0 {
        tracing::warn!(
            "{unused} triangle(s) of lightmap copies have no triangle of the level in their place, left out: \
             a lightmap copy has to be where its object is"
        );
    }

    if importer.skipped_primitives > 0 {
        tracing::warn!(
            "{} primitive(s) of {} aren't triangles with positions, left out",
            importer.skipped_primitives,
            path.display()
        );
    }
    for (name, flags, triangles) in std::mem::take(&mut importer.tops) {
        if flags == EDGE_LADDER_TOP {
            match ladder_of(&triangles) {
                Some(ladder) => importer.scene.ladders.push(ladder),
                None => tracing::warn!("{name} has no face that looks sideways, it is no ladder: make it an upright quad that looks at the player on it"),
            }
            continue;
        }
        let edges = rim_edges(&triangles, flags);
        if edges.is_empty() {
            tracing::warn!("{name} has no face that looks up, nothing of it can be vaulted or climbed");
        }
        importer.scene.edges.extend(edges);
    }
    for (name, ids, triangles) in std::mem::take(&mut importer.portals) {
        let room = |id: &String| {
            importer.room_ids.iter().position(|r| r == id).with_context(|| {
                format!("portal {name} is between rooms {} and {}, but no node is named Room{id}", ids[0], ids[1])
            })
        };
        importer.scene.portals.push(ScenePortal {
            rooms: [room(&ids[0])?, room(&ids[1])?],
            name,
            triangles,
        });
    }
    anyhow::ensure!(
        importer.scene.portals.is_empty() || !importer.scene.rooms.is_empty(),
        "{} has portals but no rooms",
        path.display()
    );

    anyhow::ensure!(
        !importer.scene.triangles.is_empty() || !importer.scene.collision.is_empty(),
        "{} has no triangles",
        path.display()
    );
    Ok(importer.scene)
}
