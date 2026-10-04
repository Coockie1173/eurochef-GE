//! Reads a glTF scene (.gltf or .glb, with its textures) into triangles for a geometry file.
//!
//! glTF is right handed with counter clockwise fronts, the game's levels are the mirror image:
//! x is negated and every triangle turned around.
//!
//! Meshes or nodes whose name starts with `collision`, `col_` or `ucx_` aren't drawn: they are
//! what a body collides with, in place of the drawn triangles. A material with `nocollide` in
//! its name is left out of the collision that is made from the drawn triangles. A node whose
//! name starts with `spawn` or `player_start` is where the player starts. A node whose name
//! starts with `reference` or `ref_` is left out with everything below it.

use std::path::Path;

use anyhow::Context;
use image::RgbaImage;

use super::{
    geomap::{GeScene, SceneTriangle},
    mesh::{GeVertex, COLOUR_ONE},
    texture::GeTexture,
};

#[derive(Clone, Debug)]
pub struct ImportOptions {
    /// Game units for a glTF unit (metres in both, so 1)
    pub scale: f32,
    /// Shade the vertex colours by a fixed light from above: the game draws levels with the
    /// light in their vertex colours, and a scene without any looks flat
    pub bake_light: bool,
}

impl Default for ImportOptions {
    fn default() -> Self {
        Self {
            scale: 1.0,
            bake_light: true,
        }
    }
}

const LIGHT_DIRECTION: [f32; 3] = [0.35, 0.85, 0.4];
const LIGHT_AMBIENT: f32 = 0.55;
const LIGHT_DIFFUSE: f32 = 0.45;

fn is_collision_name(name: Option<&str>) -> bool {
    name.map(|n| {
        let n = n.to_lowercase();
        n.starts_with("collision") || n.starts_with("col_") || n.starts_with("ucx_")
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
    image_textures: Vec<Option<usize>>,
    images: &'a [gltf::image::Data],
    white: Option<usize>,
    skipped_primitives: usize,
}

impl Importer<'_> {
    fn texture_for(&mut self, material: &gltf::Material<'_>) -> anyhow::Result<usize> {
        let info = material.pbr_metallic_roughness().base_color_texture();
        if let Some(info) = info {
            let image = info.texture().source().index();
            if let Some(existing) = self.image_textures[image] {
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
            });
            let index = self.scene.textures.len() - 1;
            self.image_textures[image] = Some(index);
            return Ok(index);
        }

        // no texture: a white one, the material's colour goes into the vertices
        if self.white.is_none() {
            self.scene
                .textures
                .push(GeTexture::solid("white", [255, 255, 255, 255]));
            self.white = Some(self.scene.textures.len() - 1);
        }
        Ok(self.white.unwrap())
    }

    fn node(&mut self, node: &gltf::Node<'_>, parent: &[[f32; 4]; 4], collision: bool) -> anyhow::Result<()> {
        // something to model against (a figure the player's size), not a part of the level
        if is_reference_name(node.name()) {
            return Ok(());
        }
        let matrix = multiply(parent, &node.transform().matrix());
        let collision = collision || is_collision_name(node.name());

        let name = node.name().unwrap_or("").to_lowercase();
        if name.starts_with("spawn") || name.starts_with("player_start") || name.starts_with("playerstart") {
            let p = transform_point(&matrix, [0.0, 0.0, 0.0]);
            let scale = self.options.scale;
            self.scene.spawn = Some([-p[0] * scale, p[1] * scale, p[2] * scale]);
        }

        if let Some(mesh) = node.mesh() {
            let collision = collision || is_collision_name(mesh.name());
            for primitive in mesh.primitives() {
                if primitive.mode() != gltf::mesh::Mode::Triangles {
                    self.skipped_primitives += 1;
                    continue;
                }
                self.primitive(&primitive, &matrix, collision)?;
            }
        }

        for child in node.children() {
            self.node(&child, &matrix, collision)?;
        }
        Ok(())
    }

    fn primitive(
        &mut self,
        primitive: &gltf::Primitive<'_>,
        matrix: &[[f32; 4]; 4],
        collision: bool,
    ) -> anyhow::Result<()> {
        let buffers = self.buffers;
        let reader = primitive.reader(|b| buffers.get(b.index()).map(|d| d.0.as_slice()));
        let Some(positions) = reader.read_positions() else {
            self.skipped_primitives += 1;
            return Ok(());
        };
        let scale = self.options.scale;
        // mirrored: x is negated here, the triangles are turned around below
        let positions: Vec<[f32; 3]> = positions
            .map(|p| {
                let p = transform_point(matrix, p);
                [-p[0] * scale, p[1] * scale, p[2] * scale]
            })
            .collect();
        let indices: Vec<u32> = match reader.read_indices() {
            Some(indices) => indices.into_u32().collect(),
            None => (0..positions.len() as u32).collect(),
        };
        let triangles = indices
            .chunks_exact(3)
            .filter(|t| t.iter().all(|i| (*i as usize) < positions.len()))
            .map(|t| [t[0] as usize, t[2] as usize, t[1] as usize]);

        if collision {
            for t in triangles {
                self.scene.collision.push([positions[t[0]], positions[t[1]], positions[t[2]]]);
            }
            return Ok(());
        }

        let normals: Option<Vec<[f32; 3]>> = reader.read_normals().map(|n| {
            n.map(|n| {
                let n = transform_direction(matrix, n);
                [-n[0], n[1], n[2]]
            })
            .collect()
        });
        let uvs: Option<Vec<[f32; 2]>> = reader.read_tex_coords(0).map(|t| t.into_f32().collect());
        let colours: Option<Vec<[f32; 4]>> = reader.read_colors(0).map(|c| c.into_rgba_f32().collect());

        let material = primitive.material();
        let factor = material.pbr_metallic_roughness().base_color_factor();
        let texture = self.texture_for(&material)?;
        let no_collision = material
            .name()
            .map(|n| n.to_lowercase().contains("nocollide"))
            .unwrap_or(false);

        for t in triangles {
            let corners = [positions[t[0]], positions[t[1]], positions[t[2]]];
            let flat = face_normal(&corners);
            let vertices = [0, 1, 2].map(|k| {
                let i = t[k];
                let normal = normals
                    .as_ref()
                    .and_then(|n| n.get(i))
                    .copied()
                    .filter(|n| n.iter().all(|v| v.is_finite()))
                    .unwrap_or(flat);
                let mut colour = colours.as_ref().and_then(|c| c.get(i)).copied().unwrap_or([1.0; 4]);
                for k in 0..4 {
                    colour[k] *= factor[k];
                }
                if self.options.bake_light {
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
            self.scene.triangles.push(SceneTriangle {
                texture: Some(texture),
                vertices,
                no_collision,
            });
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
        image_textures: vec![None; images.len()],
        images: &images,
        white: None,
        skipped_primitives: 0,
    };

    let identity = [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0], [0.0, 0.0, 0.0, 1.0]];
    let scenes: Vec<gltf::Scene<'_>> = match document.default_scene() {
        Some(scene) => vec![scene],
        None => document.scenes().collect(),
    };
    for scene in scenes {
        for node in scene.nodes() {
            importer.node(&node, &identity, false)?;
        }
    }

    if importer.skipped_primitives > 0 {
        tracing::warn!(
            "{} primitive(s) of {} aren't triangles with positions, left out",
            importer.skipped_primitives,
            path.display()
        );
    }
    anyhow::ensure!(
        !importer.scene.triangles.is_empty() || !importer.scene.collision.is_empty(),
        "{} has no triangles",
        path.display()
    );
    Ok(importer.scene)
}
