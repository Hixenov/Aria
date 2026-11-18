//! Draws Aria's icon and writes it to `assets/icon/`: `aria.ico`, holding every size Windows asks
//! for (16 to 256 pixels), and `aria-256.png`, the window's icon.
//!
//! Run it from the repository root:
//!
//! ```text
//! cargo run -p aria-app --example make_icon
//! ```
//!
//! The icon is drawn from a few shapes, the same on every run: a rounded square in Aria's teal,
//! darker towards the bottom, and a white "A" crossed by a sound wave, which runs out past both
//! legs. Each size is drawn on its own, so the small ones stay sharp; on the smallest, the wave is
//! a plain crossbar.

use std::{
    fs, io,
    path::{Path, PathBuf},
};

use image::{ExtendedColorType, ImageEncoder, codecs::png::PngEncoder};

/// The sizes in the `.ico`, in pixels.
const SIZES: [u32; 7] = [16, 24, 32, 48, 64, 128, 256];
/// Samples per pixel along each side, for smooth edges.
const SAMPLES: u32 = 8;

/// The top and the bottom of the background.
const TOP: [f32; 3] = [0x4D as f32, 0xE2 as f32, 0xD1 as f32];
const BOTTOM: [f32; 3] = [0x0E as f32, 0x63 as f32, 0x77 as f32];

/// The "A": its apex, its feet, and the crossbar's height.
const APEX: (f32, f32) = (0.5, 0.215);
const FOOT_LEFT: (f32, f32) = (0.29, 0.775);
const FOOT_RIGHT: (f32, f32) = (0.71, 0.775);
const BAR_Y: f32 = 0.6;
/// Where the wave starts and ends, and how many waves it makes on the way.
const WAVE_FROM: f32 = 0.165;
const WAVE_TO: f32 = 0.835;
const WAVES: f32 = 1.5;

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
    let shadow = size >= 48;
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
                    let [r, g, b, a] = sample(u, v, &glyph, shadow);
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

/// The colour at `(u, v)`, both `0.0..1.0` from the top left, with its alpha `0.0..=1.0`.
fn sample(u: f32, v: f32, glyph: &Glyph, shadow: bool) -> [f32; 4] {
    if !in_background(u, v) {
        return [0.0; 4];
    }
    let mut colour = background(u, v);
    if shadow && glyph.contains(u, v - 0.014) && !glyph.contains(u, v) {
        // A soft drop below the letter.
        colour = mix(colour, [0.0, 0.0, 0.0], 0.22);
    }
    if glyph.contains(u, v) {
        colour = [255.0, 255.0, 255.0];
    }
    [colour[0], colour[1], colour[2], 1.0]
}

/// Whether `(u, v)` is inside the rounded square: a superellipse, softer than a rounded rectangle.
fn in_background(u: f32, v: f32) -> bool {
    const HALF: f32 = 0.47;
    const POWER: f32 = 5.0;
    let x = ((u - 0.5) / HALF).abs();
    let y = ((v - 0.5) / HALF).abs();
    x.powf(POWER) + y.powf(POWER) <= 1.0
}

/// Teal, lighter at the top left, with a faint glow there.
fn background(u: f32, v: f32) -> [f32; 3] {
    let t = (0.3 * u + 0.7 * v).clamp(0.0, 1.0);
    let base = mix(TOP, BOTTOM, t);
    let glow = (1.0 - ((u - 0.3).powi(2) + (v - 0.18).powi(2)).sqrt() / 0.55).clamp(0.0, 1.0);
    mix(base, [255.0, 255.0, 255.0], 0.14 * glow)
}

fn mix(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [
        a[0] + (b[0] - a[0]) * t,
        a[1] + (b[1] - a[1]) * t,
        a[2] + (b[2] - a[2]) * t,
    ]
}

/// The "A": two strokes with round ends, and a wave between them.
struct Glyph {
    /// Half the width of a leg.
    leg: f32,
    /// Half the width of the wave.
    bar: f32,
    /// The wave's height above and below its line.
    wave: f32,
    /// The wave, as a line of points from leg to leg.
    points: Vec<(f32, f32)>,
}

impl Glyph {
    /// The letter for an icon of `size` pixels: on the smallest, the strokes are kept at least
    /// two pixels wide, and the wave is a plain crossbar from leg to leg.
    fn for_size(size: u32) -> Self {
        let pixel = 1.0 / size as f32;
        let leg = 0.056f32.max(1.05 * pixel);
        let small = size < 32;
        let bar = if small {
            0.05f32.max(0.8 * pixel)
        } else {
            0.03
        };
        let wave = if small { 0.0 } else { 0.05 };
        let (from, to, waves) = if small {
            (leg_x(BAR_Y, FOOT_LEFT), leg_x(BAR_Y, FOOT_RIGHT), 0.0)
        } else {
            (WAVE_FROM, WAVE_TO, WAVES)
        };
        let steps = 64;
        let points = (0..=steps)
            .map(|step| {
                let t = step as f32 / steps as f32;
                let x = from + (to - from) * t;
                let y = BAR_Y - wave * (std::f32::consts::TAU * waves * t).sin();
                (x, y)
            })
            .collect();
        Self {
            leg,
            bar,
            wave,
            points,
        }
    }

    fn contains(&self, u: f32, v: f32) -> bool {
        if near(u, v, APEX, FOOT_LEFT, self.leg) || near(u, v, APEX, FOOT_RIGHT, self.leg) {
            return true;
        }
        if (v - BAR_Y).abs() > self.wave + self.bar {
            return false;
        }
        self.points
            .windows(2)
            .any(|pair| near(u, v, pair[0], pair[1], self.bar))
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
