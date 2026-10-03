//! Textures the way GoldenEye 007 (Wii) keeps them: an EXGeoTexture header and a TPL file with
//! one image and its mip levels behind it.

use image::{imageops::FilterType, RgbaImage};

use super::writer::Writer;

/// GX texture formats (the TPL's) and the format number the EDB uses for them
const GX_CMPR: u32 = 0xE;
const GX_RGBA8: u32 = 0x6;
const EX_CMPR: u8 = 0;
const EX_RGBA8: u8 = 1;

const TPL_MAGIC: u32 = 0x0020AF30;
const MAX_SIZE: u32 = 1024;
const MIN_SIZE: u32 = 8;

#[derive(Clone)]
pub struct GeTexture {
    pub name: String,
    pub image: RgbaImage,
    /// Keep every bit of the alpha channel (RGBA8, four times the size of CMPR)
    pub full_alpha: bool,
}

impl GeTexture {
    pub fn solid(name: &str, rgba: [u8; 4]) -> Self {
        Self {
            name: name.to_string(),
            image: RgbaImage::from_pixel(16, 16, rgba.into()),
            full_alpha: false,
        }
    }

    /// The size it is stored at: powers of two between 8 and 1024
    pub fn stored_size(&self) -> (u32, u32) {
        let fit = |v: u32| v.clamp(MIN_SIZE, MAX_SIZE).next_power_of_two().min(MAX_SIZE);
        (fit(self.image.width()), fit(self.image.height()))
    }

    pub fn has_alpha(&self) -> bool {
        self.image.pixels().any(|p| p[3] < 250)
    }
}

/// Writes the texture at the writer's position (which has to be a multiple of 32)
pub fn write_texture(w: &mut Writer, texture: &GeTexture) {
    let (width, height) = texture.stored_size();
    let base = if (width, height) == texture.image.dimensions() {
        texture.image.clone()
    } else {
        image::imageops::resize(&texture.image, width, height, FilterType::Triangle)
    };

    // down to 1 pixel on the longer side, as the game's own textures
    let max_lod = width.max(height).trailing_zeros();
    let mut levels = vec![base];
    for level in 1..=max_lod {
        let (lw, lh) = ((width >> level).max(1), (height >> level).max(1));
        levels.push(image::imageops::resize(&levels[0], lw, lh, FilterType::Triangle));
    }

    let (gx_format, ex_format) = if texture.full_alpha {
        (GX_RGBA8, EX_RGBA8)
    } else {
        (GX_CMPR, EX_CMPR)
    };
    let mut pixels = vec![];
    for level in &levels {
        if texture.full_alpha {
            encode_rgba8(level, &mut pixels);
        } else {
            encode_cmpr(level, &mut pixels);
        }
    }

    let (mut r, mut g, mut b) = (0u64, 0u64, 0u64);
    for p in levels[0].pixels() {
        r += p[0] as u64;
        g += p[1] as u64;
        b += p[2] as u64;
    }
    let count = (width * height) as u64;

    let start = w.pos();
    debug_assert_eq!(start % 32, 0);
    w.u16(width as u16);
    w.u16(height as u16);
    w.u16(1); // depth
    w.u16(0); // game flags
    w.i16(0); // scroll u, v
    w.i16(0);
    w.u8(1); // frames
    w.u8(1); // images
    w.u8(0); // frame rate
    w.u8(0);
    w.u8(0); // values used
    w.u8(0); // regions
    w.u8(max_lod as u8);
    w.u8(ex_format);
    w.u32(0);
    w.bytes(&[0xFF, (r / count) as u8, (g / count) as u8, (b / count) as u8]);
    w.u32(0xFFFFFFFF); // no external file
    // animseq, values, fur, regions: 16 bit offsets, the used ones to the end of the header
    w.i16(0);
    w.i16(0x40 - 0x22);
    w.i16(0);
    w.i16(0x40 - 0x26);
    w.u32(0x40 + pixels.len() as u32);
    w.u32(0x40 - 0x2C); // the one frame, right behind the header
    w.zeros(0x10);
    debug_assert_eq!(w.pos() - start, 0x40);

    // the TPL
    w.u32(TPL_MAGIC);
    w.u32(1); // images
    w.u32(0xC); // the image table
    w.u32(0x14); // the image's header
    w.u32(0); // no palette
    w.u16(height as u16);
    w.u16(width as u16);
    w.u32(gx_format);
    w.u32(0x40); // the pixels
    w.u32(1); // wrap s, t: repeat
    w.u32(1);
    w.u32(5); // min filter: linear, mip linear
    w.u32(1); // mag filter: linear
    w.f32(0.0); // lod bias
    w.u8(0); // edge lod
    w.u8(0); // min lod
    w.u8(max_lod as u8);
    w.u8(0);
    w.zeros(8);
    debug_assert_eq!(w.pos() - start, 0x80);
    w.bytes(&pixels);
    w.align(32);
}

fn padded(image: &RgbaImage, to: u32) -> RgbaImage {
    let (w, h) = image.dimensions();
    let (pw, ph) = (w.div_ceil(to) * to, h.div_ceil(to) * to);
    if (pw, ph) == (w, h) {
        return image.clone();
    }
    RgbaImage::from_fn(pw, ph, |x, y| *image.get_pixel(x.min(w - 1), y.min(h - 1)))
}

/// CMPR: DXT1 blocks in groups of 2 by 2, the colours big endian and each row's pixels the other
/// way round
fn encode_cmpr(image: &RgbaImage, out: &mut Vec<u8>) {
    let image = padded(image, 8);
    let (w, h) = (image.width() as usize, image.height() as usize);
    let mut dxt = vec![0u8; squish::Format::Bc1.compressed_size(w, h)];
    squish::Format::Bc1.compress(
        image.as_raw(),
        w,
        h,
        squish::Params {
            algorithm: squish::Algorithm::ClusterFit,
            weights: squish::COLOUR_WEIGHTS_UNIFORM,
            weigh_colour_by_alpha: false,
        },
        &mut dxt,
    );

    let blocks_x = w / 4;
    for group_y in 0..h / 8 {
        for group_x in 0..w / 8 {
            for (sub_x, sub_y) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                let block = (group_y * 2 + sub_y) * blocks_x + group_x * 2 + sub_x;
                let b = &dxt[block * 8..block * 8 + 8];
                out.extend_from_slice(&[b[1], b[0], b[3], b[2]]);
                for row in &b[4..8] {
                    out.push(
                        (row & 0x03) << 6 | (row & 0x0C) << 2 | (row & 0x30) >> 2 | (row & 0xC0) >> 6,
                    );
                }
            }
        }
    }
}

/// RGBA8: blocks of 4 by 4, alpha and red for the 16 pixels, then green and blue
fn encode_rgba8(image: &RgbaImage, out: &mut Vec<u8>) {
    let image = padded(image, 4);
    for block_y in (0..image.height()).step_by(4) {
        for block_x in (0..image.width()).step_by(4) {
            for y in 0..4 {
                for x in 0..4 {
                    let p = image.get_pixel(block_x + x, block_y + y);
                    out.extend_from_slice(&[p[3], p[0]]);
                }
            }
            for y in 0..4 {
                for x in 0..4 {
                    let p = image.get_pixel(block_x + x, block_y + y);
                    out.extend_from_slice(&[p[1], p[2]]);
                }
            }
        }
    }
}
