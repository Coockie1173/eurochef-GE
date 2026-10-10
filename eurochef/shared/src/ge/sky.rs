//! A made sky for a level: a sphere around it, seen from inside, with the sky painted into its
//! vertex colours (zenith, horizon, ground, clouds) or with a panorama on it.
//!
//! The game draws the sky where it stands and it hides what is behind it, so the sphere is put
//! around the whole level: its middle and its size come from the level's. Colours are given the
//! way they look (sRGB), which is what the game puts on the screen.

use std::path::PathBuf;

use anyhow::Context;
use image::{Rgba, RgbaImage};

use super::{
    geomap::{GeScene, SceneTriangle},
    mesh::{Bounds, GeVertex, COLOUR_ONE},
    texture::GeTexture,
};

pub type Colour = [f32; 3];

/// Zenith, horizon, ground, clouds, how much of the sky they cover
pub const PRESETS: [(&str, [&str; 4], f32); 5] = [
    ("day", ["#2f6fd0", "#a9cdf2", "#6f7f8f", "#ffffff"], 0.35),
    ("overcast", ["#7d8791", "#a7afb5", "#5f6468", "#c9ced2"], 0.8),
    ("dusk", ["#1c2550", "#f08a4b", "#2a2030", "#f6b08a"], 0.4),
    ("night", ["#03050c", "#101a33", "#05070c", "#232c44"], 0.3),
    ("space", ["#000000", "#020308", "#000000", "#000000"], 0.0),
];

/// A colour from #RRGGBB
pub fn colour_of(text: &str) -> anyhow::Result<Colour> {
    let hex = text.trim().trim_start_matches('#');
    anyhow::ensure!(hex.len() == 6 && hex.is_ascii(), "\"{text}\" isn't a colour: #RRGGBB");
    let part = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).map(|v| v as f32 / 255.0);
    Ok([part(0)?, part(2)?, part(4)?])
}

#[derive(Clone, Debug)]
pub struct SkyOptions {
    pub zenith: Colour,
    pub horizon: Colour,
    pub ground: Colour,
    pub cloud_colour: Colour,
    /// How much of the sky the clouds take, 0 to 1
    pub clouds: f32,
    /// How soon the horizon gives way to the zenith: below 1 soon, above 1 late
    pub falloff: f32,
    /// Larger: smaller clouds
    pub cloud_scale: f32,
    pub cloud_softness: f32,
    pub cloud_opacity: f32,
    /// Another number, other clouds
    pub seed: u64,
    /// A picture of the whole sky unrolled (2:1) in place of the painted one
    pub texture: Option<PathBuf>,
    /// Around the sphere, half of it down
    pub segments: usize,
    /// None: the level's size times `margin`, 50 at least
    pub radius: Option<f32>,
    /// In the game's coordinates. None: the level's middle
    pub centre: Option<[f32; 3]>,
    pub margin: f32,
}

impl SkyOptions {
    /// One of `PRESETS` by its name
    pub fn preset(name: &str) -> anyhow::Result<Self> {
        let (_, colours, clouds) = PRESETS
            .iter()
            .find(|p| p.0.eq_ignore_ascii_case(name))
            .with_context(|| {
                let names: Vec<&str> = PRESETS.iter().map(|p| p.0).collect();
                format!("no sky is called \"{name}\": {}", names.join(", "))
            })?;
        Ok(Self {
            zenith: colour_of(colours[0])?,
            horizon: colour_of(colours[1])?,
            ground: colour_of(colours[2])?,
            cloud_colour: colour_of(colours[3])?,
            clouds: *clouds,
            falloff: 0.6,
            cloud_scale: 3.0,
            cloud_softness: 0.18,
            cloud_opacity: 0.9,
            seed: 1,
            texture: None,
            segments: 64,
            radius: None,
            centre: None,
            margin: 2.5,
        })
    }

    fn across(&self) -> usize {
        self.segments.clamp(8, 250)
    }

    fn down(&self) -> usize {
        (self.across() / 2).max(4)
    }
}

fn mix(a: Colour, b: Colour, t: f32) -> Colour {
    [0, 1, 2].map(|i| a[i] + (b[i] - a[i]) * t)
}

fn smooth(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Value noise in space, a few octaves: the same for the same seed
struct Noise {
    table: Vec<f32>,
    offset: [f32; 3],
}

impl Noise {
    fn new(seed: u64) -> Self {
        // splitmix64
        let mut state = seed.wrapping_mul(0x9E3779B97F4A7C15).wrapping_add(0x1234_5678_9ABC_DEF1);
        let mut next = move || {
            state = state.wrapping_add(0x9E3779B97F4A7C15);
            let mut z = state;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
            ((z ^ (z >> 31)) >> 40) as f32 / (1u64 << 24) as f32
        };
        let table = (0..4096).map(|_| next()).collect();
        let offset = [next() * 100.0, next() * 100.0, next() * 100.0];
        Self { table, offset }
    }

    fn cell(&self, x: i64, y: i64, z: i64) -> f32 {
        let hash = x.wrapping_mul(73856093) ^ y.wrapping_mul(19349663) ^ z.wrapping_mul(83492791);
        self.table[(hash & 4095) as usize]
    }

    fn at(&self, p: [f32; 3]) -> f32 {
        let base = p.map(|v| v.floor());
        let f = [0, 1, 2].map(|i| smooth(p[i] - base[i]));
        let [x, y, z] = base.map(|v| v as i64);
        let mut value = 0.0;
        for dx in 0..2 {
            for dy in 0..2 {
                for dz in 0..2 {
                    let weight = (if dx == 1 { f[0] } else { 1.0 - f[0] })
                        * (if dy == 1 { f[1] } else { 1.0 - f[1] })
                        * (if dz == 1 { f[2] } else { 1.0 - f[2] });
                    value += weight * self.cell(x + dx, y + dy, z + dz);
                }
            }
        }
        value
    }

    fn clouds(&self, direction: [f32; 3], scale: f32) -> f32 {
        // looked at from below a flat layer, so they get small and dense towards the horizon
        let height = direction[1].max(0.05);
        let p = [
            direction[0] / height * scale * 0.35 + self.offset[0],
            self.offset[1],
            direction[2] / height * scale * 0.35 + self.offset[2],
        ];
        let (mut value, mut size, mut total) = (0.0, 1.0, 0.0);
        for _ in 0..4 {
            value += self.at(p.map(|v| v / size)) * size;
            total += size;
            size *= 0.5;
        }
        value / total
    }
}

fn sky_colour(direction: [f32; 3], options: &SkyOptions, noise: &Noise) -> Colour {
    let up = direction[1];
    let mut colour = if up >= 0.0 {
        mix(options.horizon, options.zenith, up.powf(options.falloff))
    } else {
        mix(options.horizon, options.ground, smooth(-up * 6.0))
    };
    if options.clouds > 0.0 && up > 0.0 {
        let cover = noise.clouds(direction, options.cloud_scale);
        // from the wanted share of the sky to how high the noise has to be
        let edge = 1.0 - options.clouds.clamp(0.0, 1.0);
        let mut amount = smooth((cover - edge * 0.75) / options.cloud_softness.max(0.01));
        amount *= smooth(up * 8.0); // none in the haze right at the horizon
        colour = mix(colour, options.cloud_colour, amount * options.cloud_opacity);
    }
    colour.map(|c| c.clamp(0.0, 1.0))
}

/// Where the vertex in a row (0: the top) and a column of the sphere is from its middle, in the
/// scene's space (the game's is its mirror image)
fn sphere_direction(row: usize, column: usize, across: usize, down: usize) -> [f32; 3] {
    let latitude = std::f32::consts::FRAC_PI_2 - std::f32::consts::PI * row as f32 / down as f32;
    let longitude = std::f32::consts::TAU * column as f32 / across as f32;
    [latitude.cos() * longitude.cos(), latitude.sin(), latitude.cos() * longitude.sin()]
}

fn load_panorama(path: &PathBuf) -> anyhow::Result<RgbaImage> {
    Ok(image::open(path)
        .with_context(|| format!("couldn't read the sky's picture {}", path.display()))?
        .to_rgba8())
}

/// The sky's triangles around a level of these bounds, in the game's space, and the panorama
/// when it has one: the triangles' texture is then to be set to where it is put
pub fn make_sky(options: &SkyOptions, level: &Bounds) -> anyhow::Result<(Vec<SceneTriangle>, Option<GeTexture>)> {
    let (centre, size) = if level.is_empty() {
        ([0.0; 3], 0.0)
    } else {
        let diagonal: f32 = (0..3).map(|k| (level.max[k] - level.min[k]).powi(2)).sum();
        ([0, 1, 2].map(|k| (level.min[k] + level.max[k]) * 0.5), diagonal.sqrt() * 0.5)
    };
    let centre = options.centre.unwrap_or(centre);
    let radius = options.radius.unwrap_or_else(|| if size > 0.0 { (size * options.margin).max(50.0) } else { 300.0 });

    let texture = match &options.texture {
        Some(path) => Some(GeTexture { name: "sky_panorama".to_string(), image: load_panorama(path)?, full_alpha: false, mip_levels: None, bloom: false }),
        None => None,
    };
    let noise = Noise::new(options.seed);
    let (across, down) = (options.across(), options.down());
    let mut vertices = Vec::with_capacity((across + 1) * (down + 1));
    for row in 0..=down {
        for column in 0..=across {
            let d = sphere_direction(row, column, across, down);
            let colour = if texture.is_some() { [1.0; 3] } else { sky_colour(d, options, &noise) };
            // mirrored, as everything of a scene is
            vertices.push(GeVertex {
                pos: [centre[0] - d[0] * radius, centre[1] + d[1] * radius, centre[2] + d[2] * radius],
                normal: [d[0], -d[1], -d[2]],
                uv: [column as f32 / across as f32, row as f32 / down as f32],
                color: [
                    (colour[0] * COLOUR_ONE).round().clamp(0.0, 255.0) as u8,
                    (colour[1] * COLOUR_ONE).round().clamp(0.0, 255.0) as u8,
                    (colour[2] * COLOUR_ONE).round().clamp(0.0, 255.0) as u8,
                    255,
                ],
            });
        }
    }
    let mut triangles = vec![];
    let mut push = |corners: [usize; 3]| {
        triangles.push(SceneTriangle {
            texture: None,
            vertices: corners.map(|i| vertices[i]),
            no_collision: true,
            room: None,
            two_sided: false,
            lightmap: None,
        })
    };
    for row in 0..down {
        for column in 0..across {
            let a = row * (across + 1) + column;
            let (b, c, d) = (a + 1, a + across + 1, a + across + 2);
            // seen from inside: the game draws a triangle from its front only
            if row > 0 {
                push([a, b, c]);
            }
            if row < down - 1 {
                push([b, d, c]);
            }
        }
    }
    Ok((triangles, texture))
}

/// Puts a made sky around a scene, in place of the one it has
pub fn put_sky(scene: &mut GeScene, options: &SkyOptions) -> anyhow::Result<()> {
    let (mut triangles, texture) = make_sky(options, &scene.bounds())?;
    let index = match texture {
        Some(texture) => {
            scene.textures.push(texture);
            scene.textures.len() - 1
        }
        None => match scene.textures.iter().position(|t| t.name == "sky_white") {
            Some(index) => index,
            None => {
                scene.textures.push(GeTexture::solid("sky_white", [255, 255, 255, 255]));
                scene.textures.len() - 1
            }
        },
    };
    for triangle in &mut triangles {
        triangle.texture = Some(index);
    }
    if !scene.sky.is_empty() {
        tracing::warn!("the scene has a sky of its own ({} triangles): the made one takes its place", scene.sky.len());
    }
    scene.sky = triangles;
    Ok(())
}

/// What a player sees of the sky looking `yaw` around and `pitch` up (degrees): the game's
/// picture, with the colours blended between the sphere's vertices as the game does
pub fn preview(options: &SkyOptions, size: (u32, u32), yaw: f32, pitch: f32) -> anyhow::Result<RgbaImage> {
    let (across, down) = (options.across(), options.down());
    // the whole sky unrolled: a pixel for each vertex, or the panorama
    let panorama: RgbaImage = match &options.texture {
        Some(path) => load_panorama(path)?,
        None => {
            let noise = Noise::new(options.seed);
            RgbaImage::from_fn(across as u32 + 1, down as u32 + 1, |column, row| {
                let c = sky_colour(sphere_direction(row as usize, column as usize, across, down), options, &noise);
                Rgba([(c[0] * 255.0) as u8, (c[1] * 255.0) as u8, (c[2] * 255.0) as u8, 255])
            })
        }
    };
    let (width, height) = (panorama.width() as f32 - 1.0, panorama.height() as f32 - 1.0);
    let sample = |u: f32, v: f32| {
        let (x, y) = (u.rem_euclid(1.0) * width, v.clamp(0.0, 1.0) * height);
        let (x0, y0) = (x.floor() as u32, y.floor() as u32);
        let (x1, y1) = ((x0 + 1).min(width as u32), (y0 + 1).min(height as u32));
        let (fx, fy) = (x - x0 as f32, y - y0 as f32);
        let mut out = [0u8; 4];
        for (k, o) in out.iter_mut().enumerate() {
            let p = |x: u32, y: u32| panorama.get_pixel(x, y)[k] as f32;
            let top = p(x0, y0) + (p(x1, y0) - p(x0, y0)) * fx;
            let bottom = p(x0, y1) + (p(x1, y1) - p(x0, y1)) * fx;
            *o = (top + (bottom - top) * fy).round() as u8;
        }
        Rgba(out)
    };
    let reach = (75.0f32.to_radians() / 2.0).tan();
    let (sin_p, cos_p) = pitch.to_radians().sin_cos();
    let (sin_y, cos_y) = yaw.to_radians().sin_cos();
    Ok(RgbaImage::from_fn(size.0, size.1, |i, j| {
        let x = (i as f32 / size.0 as f32 * 2.0 - 1.0) * reach;
        let y = (1.0 - j as f32 / size.1 as f32 * 2.0) * reach * size.1 as f32 / size.0 as f32;
        let (y, z) = (y * cos_p + sin_p, cos_p - y * sin_p);
        let (x, z) = (x * cos_y + z * sin_y, z * cos_y - x * sin_y);
        let length = (x * x + y * y + z * z).sqrt();
        // the game's x is the scene's turned around
        let longitude = z.atan2(-x).rem_euclid(std::f32::consts::TAU);
        sample(longitude / std::f32::consts::TAU, (std::f32::consts::FRAC_PI_2 - (y / length).asin()) / std::f32::consts::PI)
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The sphere is around the level and every triangle looks at its middle
    #[test]
    fn sky_looks_inwards() {
        let level = Bounds { min: [-10.0, 0.0, -20.0], max: [30.0, 8.0, 20.0] };
        let options = SkyOptions::preset("dusk").unwrap();
        let (triangles, texture) = make_sky(&options, &level).unwrap();
        assert!(texture.is_none());
        assert_eq!(triangles.len(), 64 * 32 * 2 - 64 * 2);
        let centre = [10.0, 4.0, 0.0];
        for t in &triangles {
            let p = [0, 1, 2].map(|i| t.vertices[i].pos);
            let u = [0, 1, 2].map(|k| p[1][k] - p[0][k]);
            let v = [0, 1, 2].map(|k| p[2][k] - p[0][k]);
            let n = [u[1] * v[2] - u[2] * v[1], u[2] * v[0] - u[0] * v[2], u[0] * v[1] - u[1] * v[0]];
            let inwards: f32 = (0..3).map(|k| n[k] * (centre[k] - p[0][k])).sum();
            assert!(inwards > 0.0, "{p:?} looks out");
            let far = (0..3).map(|k| (p[0][k] - centre[k]).powi(2)).sum::<f32>().sqrt();
            assert!((far - 71.0).abs() < 1.0, "{far}");
        }
        // straight up is the zenith's colour, clouds or not at the horizon's rim
        let top = triangles[0].vertices.iter().map(|v| v.pos[1]).fold(f32::MIN, f32::max);
        assert!(top > 70.0);
        assert!(SkyOptions::preset("noon").is_err());
        assert!(colour_of("#12345").is_err());
    }

    #[test]
    fn same_seed_same_clouds() {
        let a = preview(&SkyOptions::preset("day").unwrap(), (64, 36), 0.0, 20.0).unwrap();
        let b = preview(&SkyOptions::preset("day").unwrap(), (64, 36), 0.0, 20.0).unwrap();
        let mut other = SkyOptions::preset("day").unwrap();
        other.seed = 7;
        assert!(a == b && a != preview(&other, (64, 36), 0.0, 20.0).unwrap());
    }
}
