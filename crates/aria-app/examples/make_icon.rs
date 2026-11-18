//! Draws Aria's icon and writes it to `assets/icon/`: `aria.ico`, holding every size Windows asks
//! for (16 to 256 pixels), and `aria-256.png`, the window's icon.
//!
//! Run it from the repository root:
//!
//! ```text
//! cargo run --release -p aria-app --example make_icon
//! ```
//!
//! The icon is drawn from a few shapes, the same on every run, on a clear background: a bold,
//! rounded "A" in bright neon cyan, almost white at the top and deeper at the feet, whose
//! crossbar is a sound wave. A thin, nearly white rim runs inside its edge, like a lit tube, and a
//! soft cyan glow surrounds it. Each size is drawn on its own, so the small ones stay sharp: on
//! those the glow and the rim are left out, and the wave is a plain crossbar.

use std::{
    fs, io,
    path::{Path, PathBuf},
};

use image::{ExtendedColorType, ImageEncoder, codecs::png::PngEncoder};

/// The sizes in the `.ico`, in pixels.
const SIZES: [u32; 7] = [16, 24, 32, 48, 64, 128, 256];
/// Samples per pixel along each side, for smooth edges.
const SAMPLES: u32 = 8;

/// The letter's fill: at its apex, in its middle, and at its feet.
const FILL_TOP: [f32; 3] = [0xC4 as f32, 0xFF as f32, 0xF8 as f32];
const FILL_MIDDLE: [f32; 3] = [0x3B as f32, 0xF0 as f32, 0xE6 as f32];
const FILL_BOTTOM: [f32; 3] = [0x12 as f32, 0xAE as f32, 0xD9 as f32];
/// The bright rim inside the letter's edge.
const RIM: [f32; 3] = [0xF0 as f32, 0xFF as f32, 0xFE as f32];
/// The glow around the letter.
const GLOW: [f32; 3] = [0x1F as f32, 0xE6 as f32, 0xF2 as f32];

/// The "A" at full size, before it is shrunk to leave room for the glow: its apex, its feet,
/// and the crossbar's height.
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

/// The "A" and its wave, placed for one size.
struct Glyph {
    apex: (f32, f32),
    feet: [(f32, f32); 2],
    /// Half the width of a leg.
    leg: f32,
    /// Half the width of the wave.
    line: f32,
    /// The wave, as a line of points from leg to leg.
    points: Vec<(f32, f32)>,
    /// How far the wave reaches above and below `BAR_Y`, its width included.
    reach: f32,
    /// The bar's height once placed.
    bar_y: f32,
    /// How wide the bright rim is, inside the edge; 0 for none.
    rim: f32,
    /// How far the glow spreads; 0 for none.
    glow: f32,
}

impl Glyph {
    /// The letter for an icon of `size` pixels. From 32 pixels up it is shrunk to leave room for
    /// the glow and has its rim; on the smallest, the strokes are kept at least two pixels wide,
    /// and the wave is a plain crossbar.
    fn for_size(size: u32) -> Self {
        let pixel = 1.0 / size as f32;
        let small = size < 32;
        let scale = if small { 1.0 } else { 0.84 };
        let place = |(x, y): (f32, f32)| (0.5 + (x - 0.5) * scale, 0.5 + (y - 0.5) * scale);
        let apex = place(APEX);
        let feet = [place(FOOT_LEFT), place(FOOT_RIGHT)];
        let bar_y = place((0.5, BAR_Y)).1;
        let leg = (0.086 * scale).max(1.1 * pixel);
        let line = if small {
            0.07f32.max(0.85 * pixel)
        } else {
            0.052 * scale
        };
        let height = if small { 0.0 } else { 0.055 * scale };
        let leg_x =
            |foot: (f32, f32)| apex.0 + (foot.0 - apex.0) * (bar_y - apex.1) / (foot.1 - apex.1);
        let (from, to) = (leg_x(feet[0]), leg_x(feet[1]));
        let steps = 64;
        let points = (0..=steps)
            .map(|step| {
                let t = step as f32 / steps as f32;
                let x = from + (to - from) * t;
                let y = bar_y - height * (std::f32::consts::TAU * t).sin();
                (x, y)
            })
            .collect();
        let glow = if small { 0.0 } else { 0.03 };
        Self {
            apex,
            feet,
            leg,
            line,
            points,
            reach: height + line + 3.0 * glow,
            bar_y,
            rim: if small {
                0.0
            } else {
                (0.009f32).max(0.9 * pixel)
            },
            glow,
        }
    }

    /// How far `(u, v)` is outside the letter; below 0 inside it.
    fn distance(&self, u: f32, v: f32) -> f32 {
        let legs = segment_distance(u, v, self.apex, self.feet[0]).min(segment_distance(
            u,
            v,
            self.apex,
            self.feet[1],
        )) - self.leg;
        if (v - self.bar_y).abs() > self.reach {
            return legs;
        }
        let wave = self
            .points
            .windows(2)
            .map(|pair| segment_distance(u, v, pair[0], pair[1]))
            .fold(f32::MAX, f32::min)
            - self.line;
        legs.min(wave)
    }

    /// The colour at `(u, v)`, both `0.0..1.0` from the top left, with its alpha `0.0..=1.0`.
    fn sample(&self, u: f32, v: f32) -> [f32; 4] {
        let d = self.distance(u, v);
        if d <= 0.0 {
            let top = self.apex.1 - self.leg;
            let t = ((v - top) / (self.feet[0].1 + self.leg - top)).clamp(0.0, 1.0);
            let fill = if t < 0.45 {
                mix(FILL_TOP, FILL_MIDDLE, t / 0.45)
            } else {
                mix(FILL_MIDDLE, FILL_BOTTOM, (t - 0.45) / 0.55)
            };
            let colour = if self.rim > 0.0 {
                // Brightest right at the edge, fading in over the rim's width.
                let edge = (1.0 + d / self.rim).clamp(0.0, 1.0);
                mix(fill, RIM, 0.7 * edge * edge)
            } else {
                fill
            };
            return [colour[0], colour[1], colour[2], 1.0];
        }
        if self.glow > 0.0 {
            let alpha = 0.9 * (-(d / self.glow).powi(2)).exp();
            if alpha > 0.002 {
                return [GLOW[0], GLOW[1], GLOW[2], alpha];
            }
        }
        [0.0; 4]
    }
}

/// How far `(u, v)` is from the segment from `a` to `b`.
fn segment_distance(u: f32, v: f32, a: (f32, f32), b: (f32, f32)) -> f32 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let length = dx * dx + dy * dy;
    let t = if length > 0.0 {
        (((u - a.0) * dx + (v - a.1) * dy) / length).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let (px, py) = (a.0 + dx * t - u, a.1 + dy * t - v);
    (px * px + py * py).sqrt()
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
