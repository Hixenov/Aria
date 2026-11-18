//! Draws Aria's icon and writes it to `assets/icon/`: `aria.ico`, holding every size Windows asks
//! for (16 to 256 pixels), and `aria-256.png`, the window's icon.
//!
//! Run it from the repository root:
//!
//! ```text
//! cargo run -p aria-app --example make_icon
//! ```
//!
//! The icon is drawn from a few shapes, the same on every run, on a clear background: a bold,
//! rounded "A" in Aria's teal, lighter at the top and deeper at the bottom, whose crossbar is a
//! sound wave. Each size is drawn on its own, so the small ones stay sharp; on the smallest, the
//! wave is a plain crossbar.

use std::{
    fs, io,
    path::{Path, PathBuf},
};

use image::{ExtendedColorType, ImageEncoder, codecs::png::PngEncoder};

/// The sizes in the `.ico`, in pixels.
const SIZES: [u32; 7] = [16, 24, 32, 48, 64, 128, 256];
/// Samples per pixel along each side, for smooth edges.
const SAMPLES: u32 = 8;

/// The letter's colour, at its apex and at its feet.
const TOP: [f32; 3] = [0x4D as f32, 0xE2 as f32, 0xD1 as f32];
const BOTTOM: [f32; 3] = [0x0E as f32, 0x63 as f32, 0x77 as f32];

/// The "A": its apex, its feet, and the crossbar's height.
const APEX: (f32, f32) = (0.5, 0.13);
const FOOT_LEFT: (f32, f32) = (0.19, 0.87);
const FOOT_RIGHT: (f32, f32) = (0.81, 0.87);
const BAR_Y: f32 = 0.64;

fn main() -> io::Result<()> {
    let out = Path::new("assets").join("icon");
    fs::create_dir_all(&out)?;
    let mut images = Vec::new();
    for size in SIZES {
        let png = png(size, &draw(size))?;
        if size == 256 {
            fs::write(out.join("aria-256.png"), &png)?;
        }
        images.push((size, png));
    }
    let ico = ico(&images);
    let path: PathBuf = out.join("aria.ico");
    fs::write(&path, ico)?;
    println!("wrote {} and aria-256.png", path.display());
    Ok(())
}

/// The icon at `size` pixels, as straight (not premultiplied) RGBA.
fn draw(size: u32) -> Vec<u8> {
    let glyph = Glyph::for_size(size);
    let mut rgba = Vec::with_capacity((size * size * 4) as usize);
    let samples = (SAMPLES * SAMPLES) as f32;
    for y in 0..size {
        for x in 0..size {
            // Premultiplied sums over the pixel's samples.
            let mut sum = [0.0f32; 4];
            for sy in 0..SAMPLES {
                for sx in 0..SAMPLES {
                    let u = (x as f32 + (sx as f32 + 0.5) / SAMPLES as f32) / size as f32;
                    let v = (y as f32 + (sy as f32 + 0.5) / SAMPLES as f32) / size as f32;
                    let [r, g, b, a] = glyph.sample(u, v);
                    sum[0] += r * a;
                    sum[1] += g * a;
                    sum[2] += b * a;
                    sum[3] += a;
                }
            }
            let alpha = sum[3] / samples;
            for channel in &sum[..3] {
                let value = if sum[3] > 0.0 { channel / sum[3] } else { 0.0 };
                rgba.push(value.round().clamp(0.0, 255.0) as u8);
            }
            rgba.push((alpha * 255.0).round().clamp(0.0, 255.0) as u8);
        }
    }
    rgba
}

fn mix(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    let t = t.clamp(0.0, 1.0);
    [
        a[0] + (b[0] - a[0]) * t,
        a[1] + (b[1] - a[1]) * t,
        a[2] + (b[2] - a[2]) * t,
    ]
}

/// The "A" and its wave.
struct Glyph {
    /// Half the width of a leg.
    leg: f32,
    /// Half the width of the wave.
    line: f32,
    /// The wave, as a line of points from leg to leg.
    points: Vec<(f32, f32)>,
}

impl Glyph {
    /// The letter for an icon of `size` pixels: on the smallest, the strokes are kept at least
    /// two pixels wide, and the wave is a plain crossbar.
    fn for_size(size: u32) -> Self {
        let pixel = 1.0 / size as f32;
        let small = size < 32;
        let leg = 0.086f32.max(1.1 * pixel);
        let line = if small {
            0.07f32.max(0.85 * pixel)
        } else {
            0.052
        };
        let height = if small { 0.0 } else { 0.055 };
        let (from, to) = (leg_x(BAR_Y, FOOT_LEFT), leg_x(BAR_Y, FOOT_RIGHT));
        let steps = 64;
        let points = (0..=steps)
            .map(|step| {
                let t = step as f32 / steps as f32;
                let x = from + (to - from) * t;
                let y = BAR_Y - height * (std::f32::consts::TAU * t).sin();
                (x, y)
            })
            .collect();
        Self { leg, line, points }
    }

    /// The colour at `(u, v)`, both `0.0..1.0` from the top left, with its alpha `0.0..=1.0`.
    fn sample(&self, u: f32, v: f32) -> [f32; 4] {
        let on_leg =
            near(u, v, APEX, FOOT_LEFT, self.leg) || near(u, v, APEX, FOOT_RIGHT, self.leg);
        let on_wave = || {
            self.points
                .windows(2)
                .any(|pair| near(u, v, pair[0], pair[1], self.line))
        };
        if !on_leg && !on_wave() {
            return [0.0; 4];
        }
        let [r, g, b] = mix(
            TOP,
            BOTTOM,
            (v - APEX.1 + self.leg) / (FOOT_LEFT.1 - APEX.1 + 2.0 * self.leg),
        );
        [r, g, b, 1.0]
    }
}

/// Where the leg from the apex to `foot` crosses height `y`.
fn leg_x(y: f32, foot: (f32, f32)) -> f32 {
    APEX.0 + (foot.0 - APEX.0) * (y - APEX.1) / (foot.1 - APEX.1)
}

/// Whether `(u, v)` is within `radius` of the segment from `a` to `b`.
fn near(u: f32, v: f32, a: (f32, f32), b: (f32, f32), radius: f32) -> bool {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let length = dx * dx + dy * dy;
    let t = if length > 0.0 {
        (((u - a.0) * dx + (v - a.1) * dy) / length).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let (px, py) = (a.0 + dx * t - u, a.1 + dy * t - v);
    px * px + py * py <= radius * radius
}

fn png(size: u32, rgba: &[u8]) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    PngEncoder::new(&mut bytes)
        .write_image(rgba, size, size, ExtendedColorType::Rgba8)
        .map_err(io::Error::other)?;
    Ok(bytes)
}

/// An `.ico` file holding `images`, each a PNG, as Windows Vista and later read them.
fn ico(images: &[(u32, Vec<u8>)]) -> Vec<u8> {
    let count = images.len() as u16;
    let mut ico = Vec::new();
    ico.extend_from_slice(&0u16.to_le_bytes()); // reserved
    ico.extend_from_slice(&1u16.to_le_bytes()); // an icon, not a cursor
    ico.extend_from_slice(&count.to_le_bytes());
    let mut offset = 6 + 16 * u32::from(count);
    for (size, png) in images {
        // 256 is written as 0.
        let side = if *size >= 256 { 0 } else { *size as u8 };
        ico.extend_from_slice(&[side, side, 0, 0]);
        ico.extend_from_slice(&1u16.to_le_bytes()); // colour planes
        ico.extend_from_slice(&32u16.to_le_bytes()); // bits per pixel
        ico.extend_from_slice(&(png.len() as u32).to_le_bytes());
        ico.extend_from_slice(&offset.to_le_bytes());
        offset += png.len() as u32;
    }
    for (_, png) in images {
        ico.extend_from_slice(png);
    }
    ico
}
