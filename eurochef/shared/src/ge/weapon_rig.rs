//! A base gun as one rigged mesh in glTF, to make a custom weapon's gun from: an armature with a
//! bone for each part (`Part_N`, with the locator's role after it), placed where the gun's first
//! script puts the part on its first frame, and every part's triangles in the one mesh, weighted
//! all to its bone (a vertex group of the bone's name in Blender). The gun is seen put together;
//! a part is edited or swapped by its vertex group, and the build (`ge weapon`) takes each
//! triangle back to its bone's part, in the bone's own space, so the copied scripts place it again.
//!
//! The scripts' keyframes of the parts go along as animations (one per script with more than a
//! frame, named by its hash), to see how a reload moves them. They aren't read back.
//!
//! As everywhere in `ge`, the game is glTF's mirror image: x is negated, and with it a
//! rotation's y and z.

use std::{
    collections::HashMap,
    io::{Cursor, Seek},
};

use base64::Engine;
use eurochef_edb::{binrw::BinReaderExt, edb::EdbFile, entity::EXGeoEntity, versions::Platform};
use serde_json::{json, Value};

use super::{
    weapon::{locator_name, GunPart, ScriptBlock},
    writer::{read_rel, read_u16, read_u32},
};
use crate::{entities::read_entity, platform::texture::create_for_platform, textures::UXGeoTexture};

const SCRIPT_FRAME_RATE: usize = 0x0C;
const SCRIPT_LENGTH: usize = 0x04;
const SCRIPT_CONTROLLERS: usize = 0x34;
const SCRIPT_CONTROLLER_COUNT: usize = 0x3C;
const CHANNEL_POSITION: u32 = 0x4;
const CHANNEL_ROTATION: u32 = 0x8;
const CHANNEL_SCALE: u32 = 0x10;
/// A strip of a mesh drawn from both sides
const STRIP_TWO_SIDED: u16 = 0x40;

/// Where a script puts a part: position, rotation (x, y, z, w) and scale, as the game has them
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Place {
    pub position: [f32; 3],
    pub rotation: [f32; 4],
    pub scale: [f32; 3],
}

impl Default for Place {
    fn default() -> Self {
        Self { position: [0.0; 3], rotation: [0.0, 0.0, 0.0, 1.0], scale: [1.0; 3] }
    }
}

impl Place {
    /// The same place in glTF's mirrored space
    pub fn mirrored(&self) -> Place {
        let [x, y, z] = self.position;
        let [qx, qy, qz, qw] = self.rotation;
        let length = (qx * qx + qy * qy + qz * qz + qw * qw).sqrt().max(1e-12);
        Place {
            position: [-x, y, z],
            rotation: [qx / length, -qy / length, -qz / length, qw / length],
            scale: self.scale,
        }
    }

    /// The rotation as rows
    fn rows(&self) -> [[f32; 3]; 3] {
        let [x, y, z, w] = self.rotation;
        [
            [1.0 - 2.0 * (y * y + z * z), 2.0 * (x * y - z * w), 2.0 * (x * z + y * w)],
            [2.0 * (x * y + z * w), 1.0 - 2.0 * (x * x + z * z), 2.0 * (y * z - x * w)],
            [2.0 * (x * z - y * w), 2.0 * (y * z + x * w), 1.0 - 2.0 * (x * x + y * y)],
        ]
    }

    pub fn point(&self, p: [f32; 3]) -> [f32; 3] {
        let r = self.rows();
        let s = [p[0] * self.scale[0], p[1] * self.scale[1], p[2] * self.scale[2]];
        [0, 1, 2].map(|i| self.position[i] + r[i][0] * s[0] + r[i][1] * s[1] + r[i][2] * s[2])
    }

    pub fn direction(&self, n: [f32; 3]) -> [f32; 3] {
        let r = self.rows();
        let v = [0, 1, 2].map(|i| r[i][0] * n[0] + r[i][1] * n[1] + r[i][2] * n[2]);
        let length = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
        if length > 1e-9 { v.map(|c| c / length) } else { [0.0, 1.0, 0.0] }
    }

    /// What takes a point of the place back into the part's own space, as a glTF matrix (by columns)
    pub fn inverse_matrix(&self) -> [[f32; 4]; 4] {
        let r = self.rows();
        // the inverse's linear part is S^-1 R^T, its translation that times -position
        let linear = |i: usize, j: usize| r[j][i] / self.scale[i];
        let mut m = [[0.0; 4]; 4];
        for col in 0..3 {
            for row in 0..3 {
                m[col][row] = linear(row, col);
            }
        }
        for row in 0..3 {
            m[3][row] = -(0..3).map(|k| linear(row, k) * self.position[k]).sum::<f32>();
        }
        m[3][3] = 1.0;
        m
    }
}

/// A controller's keyframes: its frame and value
#[derive(Clone, Debug, Default)]
pub struct Keyframes {
    pub position: Vec<(f32, [f32; 3])>,
    pub rotation: Vec<(f32, [f32; 4])>,
    pub scale: Vec<(f32, [f32; 3])>,
}

impl Keyframes {
    /// Where it puts its part on the first frame
    pub fn first(&self) -> Place {
        let mut place = Place::default();
        if let Some((_, v)) = self.position.first() {
            place.position = *v;
        }
        if let Some((_, v)) = self.rotation.first() {
            place.rotation = *v;
        }
        if let Some((_, v)) = self.scale.first() {
            place.scale = *v;
        }
        place
    }
}

fn floats<const N: usize>(d: &[u8], at: usize) -> (f32, [f32; N]) {
    let f = |k: usize| f32::from_bits(read_u32(d, at + k * 4));
    (f(0), std::array::from_fn(|k| f(k + 1)))
}

/// A script's controller by its index (a command's byte 10), empty when the script has none of it
pub fn controller(script: &[u8], index: usize) -> Keyframes {
    let mut keys = Keyframes::default();
    if script.len() < 0x44 || index >= read_u16(script, SCRIPT_CONTROLLER_COUNT) as usize {
        return keys;
    }
    let Some(list) = read_rel(script, SCRIPT_CONTROLLERS) else {
        return keys;
    };
    let Some(at) = read_rel(script, list + index * 4) else {
        return keys;
    };
    if at + 12 > script.len() || read_u16(script, at) == 0 {
        return keys;
    }
    let mask = read_u32(script, at + 4);
    let mut channel = at + 12;
    for bit in 0..32 {
        if mask & (1 << bit) == 0 {
            continue;
        }
        if channel + 8 > script.len() {
            break;
        }
        let count = (read_u16(script, channel) as i16).unsigned_abs() as usize;
        let data = read_rel(script, channel + 4);
        channel += 8;
        let Some(data) = data else { continue };
        let read = |size: usize| (0..count).map(move |k| data + k * size).take_while(move |&p| p + size <= script.len());
        match 1u32 << bit {
            CHANNEL_POSITION => keys.position = read(16).map(|p| floats::<3>(script, p)).collect(),
            CHANNEL_ROTATION => keys.rotation = read(20).map(|p| floats::<4>(script, p)).collect(),
            CHANNEL_SCALE => keys.scale = read(16).map(|p| floats::<3>(script, p)).collect(),
            _ => {}
        }
    }
    keys
}

/// The first controller each part is moved with in a script, by the part's number
fn script_controllers(block: &ScriptBlock) -> HashMap<usize, Keyframes> {
    let mut by_part = HashMap::new();
    for &(word, part) in &block.parts {
        //the entity's word is 8 into the command's data, the controller byte 10 into the command
        let command = word - 8 - 12;
        by_part.entry(part).or_insert_with(|| controller(&block.bytes, block.bytes[command + 10] as usize));
    }
    by_part
}

/// Where each part rests: where the first script that names it puts it on its first frame
/// (the gun's model script first, as `read_base` reads them)
pub fn rest_places(blocks: &[ScriptBlock], count: usize) -> Vec<Place> {
    let mut places: Vec<Option<Place>> = vec![None; count];
    for block in blocks {
        for (part, keys) in script_controllers(block) {
            if part < count && places[part].is_none() {
                places[part] = Some(keys.first());
            }
        }
    }
    places.into_iter().map(|p| p.unwrap_or_default()).collect()
}

/// A bone's name: `Part_N`, and the locator's role when it has one
pub fn bone_name(index: usize, part: &GunPart) -> String {
    let role: String = locator_name(part.locator).chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect();
    if role.is_empty() { format!("Part_{index}") } else { format!("Part_{index}_{role}") }
}

/// The glTF file's one buffer, filled as the accessors are made
struct Gltf {
    buffer: Vec<u8>,
    views: Vec<Value>,
    accessors: Vec<Value>,
}

impl Gltf {
    fn accessor(&mut self, bytes: &[u8], component: u32, kind: &str, count: usize, extra: Value) -> usize {
        while self.buffer.len() % 4 != 0 {
            self.buffer.push(0);
        }
        self.views.push(json!({ "buffer": 0, "byteOffset": self.buffer.len(), "byteLength": bytes.len() }));
        self.buffer.extend_from_slice(bytes);
        let mut a = json!({ "bufferView": self.views.len() - 1, "componentType": component, "count": count, "type": kind });
        if let (Some(a), Some(extra)) = (a.as_object_mut(), extra.as_object()) {
            a.extend(extra.clone());
        }
        self.accessors.push(a);
        self.accessors.len() - 1
    }

    fn floats(&mut self, values: &[f32], width: usize, kind: &str, bounds: bool) -> usize {
        let bytes: Vec<u8> = values.iter().flat_map(|v| v.to_le_bytes()).collect();
        let extra = if bounds && !values.is_empty() {
            let column = |k: usize| values.iter().skip(k).step_by(width).copied();
            let min: Vec<f32> = (0..width).map(|k| column(k).fold(f32::INFINITY, f32::min)).collect();
            let max: Vec<f32> = (0..width).map(|k| column(k).fold(f32::NEG_INFINITY, f32::max)).collect();
            json!({ "min": min, "max": max })
        } else {
            json!({})
        };
        self.accessor(&bytes, 5126, kind, values.len() / width, extra)
    }
}

fn png_uri(rgba: &[u8], width: u32, height: u32) -> anyhow::Result<String> {
    let mut png = Cursor::new(vec![]);
    image::write_buffer_with_format(&mut png, rgba, width, height, image::ColorType::Rgba8, image::ImageOutputFormat::Png)?;
    let mut uri = "data:image/png;base64,".to_string();
    base64::engine::general_purpose::STANDARD.encode_string(png.into_inner(), &mut uri);
    Ok(uri)
}

/// The base gun as a rigged glTF file's text. `data` is o_guns.edb, `blocks` and `parts` what
/// `read_base` read of it. `animations`: the scripts' keyframes too
pub fn export_rig(data: &[u8], blocks: &[ScriptBlock], parts: &[GunPart], animations: bool) -> anyhow::Result<String> {
    let mut edb = EdbFile::new(Box::new(Cursor::new(data.to_vec())), Platform::Wii)?;
    let header = edb.header.clone();
    let decoder = create_for_platform(Platform::Wii);
    let places = rest_places(blocks, parts.len());

    let mut gltf = Gltf { buffer: vec![], views: vec![], accessors: vec![] };
    let mut primitives = vec![];
    let mut materials: Vec<Value> = vec![];
    let mut images: Vec<Value> = vec![];
    let mut material_of: HashMap<u32, usize> = HashMap::new();
    let mut left_out = vec![];

    for (index, part) in parts.iter().enumerate() {
        let Some(entity) = header.entity_list.iter().find(|e| e.common.hashcode == part.entity_hash) else {
            left_out.push(index);
            continue;
        };
        edb.seek(std::io::SeekFrom::Start(entity.common.address as u64))?;
        let entity = edb.read_type_args::<EXGeoEntity>(edb.endian, (header.version, Platform::Wii))?;
        let (mut vertices, mut indices, mut strips) = (vec![], vec![], vec![]);
        read_entity(&entity, &mut vertices, &mut indices, &mut strips, &mut edb, 4, false, true)?;
        if vertices.is_empty() || strips.is_empty() {
            left_out.push(index);
            continue;
        }

        // in its place on the gun, mirrored
        let place = places[index].mirrored();
        let mut positions = vec![];
        let mut normals = vec![];
        let mut uvs = vec![];
        let mut colours = vec![];
        for v in &vertices {
            positions.extend(place.point([-v.pos[0], v.pos[1], v.pos[2]]));
            normals.extend(place.direction([-v.norm[0], v.norm[1], v.norm[2]]));
            uvs.extend(v.uv);
            colours.extend(v.color);
        }
        let joints: Vec<u8> = vertices.iter().flat_map(|_| [index as u8, 0, 0, 0]).collect();
        let weights: Vec<f32> = vertices.iter().flat_map(|_| [1.0, 0.0, 0.0, 0.0]).collect();
        let a_position = gltf.floats(&positions, 3, "VEC3", true);
        let a_normal = gltf.floats(&normals, 3, "VEC3", false);
        let a_uv = gltf.floats(&uvs, 2, "VEC2", false);
        let a_colour = gltf.floats(&colours, 4, "VEC4", false);
        let a_joints = gltf.accessor(&joints, 5121, "VEC4", vertices.len(), json!({}));
        let a_weights = gltf.floats(&weights, 4, "VEC4", false);

        for strip in &strips {
            let texture = if strip.texture_index == u32::MAX { u32::MAX } else { header.texture_list[strip.texture_index as usize].common.hashcode };
            let material = match material_of.get(&texture) {
                Some(m) => *m,
                None => {
                    let mut material = json!({
                        "name": format!("{texture:08x}_{:08x}.png", header.hashcode),
                        "pbrMetallicRoughness": { "metallicFactor": 0.0, "roughnessFactor": 1.0 },
                        "doubleSided": strip.flags & STRIP_TWO_SIDED != 0,
                        "alphaMode": if strip.transparency == 0 { "OPAQUE" } else { "BLEND" },
                    });
                    let found = header.texture_list.iter().find(|t| t.common.hashcode == texture);
                    if let Some(t) = found {
                        if let Ok(decoded) = UXGeoTexture::read(t.common.address, &mut edb, decoder.as_ref(), t.flags) {
                            if let Some(frame) = decoded.frames.first() {
                                let cutout = (decoded.flags >> 0x18 >> 5) & 1 != 0;
                                let blend = (decoded.flags >> 0x18 >> 6) & 1 != 0;
                                if blend || cutout {
                                    material["alphaMode"] = json!(if blend { "BLEND" } else { "MASK" });
                                }
                                images.push(json!({ "uri": png_uri(frame, decoded.width as u32, decoded.height as u32)? }));
                                material["pbrMetallicRoughness"]["baseColorTexture"] = json!({ "index": images.len() - 1 });
                            }
                        }
                    }
                    materials.push(material);
                    material_of.insert(texture, materials.len() - 1);
                    materials.len() - 1
                }
            };
            let range = strip.start_index as usize..(strip.start_index + strip.index_count) as usize;
            let Some(strip_indices) = indices.get(range) else { continue };
            let bytes: Vec<u8> = strip_indices.iter().flat_map(|i| i.to_le_bytes()).collect();
            let a_indices = gltf.accessor(&bytes, 5125, "SCALAR", strip_indices.len(), json!({}));
            primitives.push(json!({
                "attributes": {
                    "POSITION": a_position, "NORMAL": a_normal, "TEXCOORD_0": a_uv, "COLOR_0": a_colour,
                    "JOINTS_0": a_joints, "WEIGHTS_0": a_weights,
                },
                "indices": a_indices,
                "material": material,
            }));
        }
    }
    anyhow::ensure!(!primitives.is_empty(), "none of the base gun's parts could be read");
    if !left_out.is_empty() {
        tracing::warn!("parts {left_out:?} have no triangles of their own, their bones are left empty");
    }

    // nodes: the gun's mesh, the armature, a bone for each part
    let first_bone = 2;
    let mut nodes = vec![
        json!({ "name": "Gun", "mesh": 0, "skin": 0 }),
        json!({ "name": "Armature", "children": (first_bone..first_bone + parts.len()).collect::<Vec<_>>() }),
    ];
    let mut inverse = vec![];
    for (index, part) in parts.iter().enumerate() {
        let place = places[index].mirrored();
        nodes.push(json!({
            "name": bone_name(index, part),
            "translation": place.position,
            "rotation": place.rotation,
            "scale": place.scale,
            "extras": { "entity": format!("{:08X}", part.entity_hash), "locator": format!("{:08X}", part.locator) },
        }));
        inverse.extend(place.inverse_matrix().iter().flatten().copied());
    }
    let a_inverse = gltf.floats(&inverse, 16, "MAT4", false);

    let mut out_animations = vec![];
    if animations {
        for block in blocks {
            let length = read_u32(&block.bytes, SCRIPT_LENGTH);
            let rate = f32::from_bits(read_u32(&block.bytes, SCRIPT_FRAME_RATE));
            if length <= 1 || !(rate > 0.0) {
                continue;
            }
            let mut samplers = vec![];
            let mut channels = vec![];
            let mut by_part: Vec<(usize, Keyframes)> = script_controllers(block).into_iter().collect();
            by_part.sort_by_key(|(part, _)| *part);
            for (part, keys) in by_part {
                let rest = Place::default();
                let mut add = |path: &str, times: Vec<f32>, values: Vec<f32>, width: usize, kind: &str| {
                    let input = gltf.floats(&times, 1, "SCALAR", true);
                    let output = gltf.floats(&values, width, kind, false);
                    samplers.push(json!({ "input": input, "output": output, "interpolation": "LINEAR" }));
                    channels.push(json!({ "sampler": samplers.len() - 1, "target": { "node": first_bone + part, "path": path } }));
                };
                //a channel the controller doesn't have stays as the game leaves it
                let mirror = |p: Place| p.mirrored();
                let position: Vec<(f32, [f32; 3])> = if keys.position.is_empty() { vec![(0.0, rest.position)] } else { keys.position.clone() };
                let rotation: Vec<(f32, [f32; 4])> = if keys.rotation.is_empty() { vec![(0.0, rest.rotation)] } else { keys.rotation.clone() };
                let scale: Vec<(f32, [f32; 3])> = if keys.scale.is_empty() { vec![(0.0, rest.scale)] } else { keys.scale.clone() };
                add(
                    "translation",
                    position.iter().map(|k| k.0 / rate).collect(),
                    position.iter().flat_map(|k| mirror(Place { position: k.1, ..rest }).position).collect(),
                    3,
                    "VEC3",
                );
                add(
                    "rotation",
                    rotation.iter().map(|k| k.0 / rate).collect(),
                    rotation.iter().flat_map(|k| mirror(Place { rotation: k.1, ..rest }).rotation).collect(),
                    4,
                    "VEC4",
                );
                add("scale", scale.iter().map(|k| k.0 / rate).collect(), scale.iter().flat_map(|k| k.1).collect(), 3, "VEC3");
            }
            if !channels.is_empty() {
                out_animations.push(json!({ "name": format!("{:08X}", block.hash), "samplers": samplers, "channels": channels }));
            }
        }
    }

    let mut uri = "data:application/octet-stream;base64,".to_string();
    base64::engine::general_purpose::STANDARD.encode_string(&gltf.buffer, &mut uri);
    let textures: Vec<Value> = (0..images.len()).map(|i| json!({ "source": i, "sampler": 0 })).collect();
    let mut root = json!({
        "asset": { "version": "2.0", "generator": "eurochef ge weapon" },
        "scene": 0,
        "scenes": [{ "nodes": [0, 1] }],
        "nodes": nodes,
        "meshes": [{ "name": "Gun", "primitives": primitives }],
        "skins": [{ "name": "Gun", "joints": (first_bone..first_bone + parts.len()).collect::<Vec<_>>(), "inverseBindMatrices": a_inverse, "skeleton": 1 }],
        "materials": materials,
        "images": images,
        "textures": textures,
        "samplers": [{ "magFilter": 9729, "minFilter": 9987, "wrapS": 10497, "wrapT": 10497 }],
        "accessors": gltf.accessors,
        "bufferViews": gltf.views,
        "buffers": [{ "byteLength": gltf.buffer.len(), "uri": uri }],
    });
    if !out_animations.is_empty() {
        root["animations"] = Value::Array(out_animations);
    }
    Ok(serde_json::to_string(&root)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inverse_takes_a_point_back() {
        let place = Place { position: [1.0, -2.0, 0.5], rotation: [0.2, 0.3, -0.1, 0.927], scale: [2.0, 1.0, 0.5] }.mirrored();
        let p = [0.3, -0.7, 1.1];
        let w = place.point(p);
        let m = place.inverse_matrix();
        let back = [0, 1, 2].map(|r| m[0][r] * w[0] + m[1][r] * w[1] + m[2][r] * w[2] + m[3][r]);
        for k in 0..3 {
            assert!((back[k] - p[k]).abs() < 1e-5, "{back:?} {p:?}");
        }
    }
}
