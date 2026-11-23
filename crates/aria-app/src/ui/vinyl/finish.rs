//! The record's finishes, rendered once into an image (off the UI thread), which the mode then
//! turns. Everything here is drawn by Aria from numbers: no image assets. The output is
//! deterministic for a given finish, colours and size.

use std::f32::consts::{PI, TAU};

use aria_core::settings::VinylFinish;
use image::{Rgba, RgbaImage};

/// The label (where the cover goes), as a share of the record's diameter.
pub const LABEL_RATIO: f32 = 0.36;
/// The size the mode renders finishes at.
pub const TEXTURE_SIZE: u32 = 1024;

/// What is drawn. [`VinylFinish::Random`] resolves to one of these.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Finish {
    Black,
    Tinted,
    Splatter,
    Marble,
}

const DRAWN: [Finish; 4] = [
    Finish::Black,
    Finish::Tinted,
    Finish::Splatter,
    Finish::Marble,
];

/// Colours taken from a cover: the main one and two accents. `neutral` when the cover has no
/// colour worth using (greyscale, near black or near white), and then they are greys.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct CoverColours {
    pub main: [u8; 3],
    pub accents: [[u8; 3]; 2],
    pub neutral: bool,
}

impl CoverColours {
    /// For no cover, or a cover without colour.
    pub fn neutral() -> CoverColours {
        CoverColours {
            main: [92, 92, 98],
            accents: [[170, 170, 176], [58, 58, 62]],
            neutral: true,
        }
    }
}

/// The share of sampled pixels that must have colour for the cover to count as coloured.
const MIN_COLOURED: f32 = 0.05;

/// The cover's colours, from a 32×32 sample of `rgba` (`width`×`height`, 4 bytes a pixel).
pub fn cover_colours(rgba: &[u8], width: u32, height: u32) -> CoverColours {
    if width == 0 || height == 0 || rgba.len() < (width * height * 4) as usize {
        return CoverColours::neutral();
    }
    const GRID: u32 = 32;
    // Buckets of 4 bits a channel: count and summed colour.
    let mut buckets: std::collections::HashMap<u16, (u32, [u32; 3])> =
        std::collections::HashMap::new();
    let mut coloured = 0u32;
    for gy in 0..GRID {
        for gx in 0..GRID {
            let x = (gx * width + width / 2) / GRID;
            let y = (gy * height + height / 2) / GRID;
            let i = ((y.min(height - 1) * width + x.min(width - 1)) * 4) as usize;
            let rgb = [rgba[i], rgba[i + 1], rgba[i + 2]];
            let (_, saturation, value) = hsv(rgb);
            if saturation >= 0.25 && (0.15..=0.95).contains(&value) {
                coloured += 1;
                let key = (u16::from(rgb[0] >> 4) << 8)
                    | (u16::from(rgb[1] >> 4) << 4)
                    | u16::from(rgb[2] >> 4);
                let entry = buckets.entry(key).or_insert((0, [0; 3]));
                entry.0 += 1;
                for (sum, channel) in entry.1.iter_mut().zip(rgb) {
                    *sum += u32::from(channel);
                }
            }
        }
    }
    if (coloured as f32) < MIN_COLOURED * (GRID * GRID) as f32 {
        return CoverColours::neutral();
    }
    let mut ranked: Vec<(u16, (u32, [u32; 3]))> = buckets.into_iter().collect();
    // Most common first; the key breaks ties so the result never depends on hash order.
    ranked.sort_by(|a, b| b.1.0.cmp(&a.1.0).then(a.0.cmp(&b.0)));
    let mean = |(count, sum): (u32, [u32; 3])| -> [u8; 3] {
        let mut out = [0u8; 3];
        for c in 0..3 {
            out[c] = u8::try_from(sum[c] / count.max(1)).unwrap_or(u8::MAX);
        }
        out
    };
    let main = mean(ranked[0].1);
    let mut accents: Vec<[u8; 3]> = Vec::new();
    for (_, entry) in ranked.iter().skip(1) {
        let colour = mean(*entry);
        let far = |other: [u8; 3]| distance(colour, other) > 60.0;
        if far(main) && accents.iter().all(|a| far(*a)) {
            accents.push(colour);
            if accents.len() == 2 {
                break;
            }
        }
    }
    while accents.len() < 2 {
        let factor = if accents.is_empty() { 1.6 } else { 0.5 };
        accents.push(scale(main, factor));
    }
    CoverColours {
        main,
        accents: [accents[0], accents[1]],
        neutral: false,
    }
}

/// The finish drawn for `finish`: [`VinylFinish::Random`] picks one per track, from a stable
/// hash of `identity` (a Spotify URI or a file path), the same across runs.
pub fn resolve(finish: VinylFinish, identity: &str) -> Finish {
    match finish {
        VinylFinish::Black => Finish::Black,
        VinylFinish::Tinted => Finish::Tinted,
        VinylFinish::Splatter => Finish::Splatter,
        VinylFinish::Marble => Finish::Marble,
        VinylFinish::Random => {
            let index = usize::try_from(fnv1a(identity.as_bytes()) % 4).unwrap_or(0);
            DRAWN[index]
        }
    }
}

/// FNV-1a, 64 bits.
fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    hash
}

/// Renders `finish` into a `size`×`size` image: a disc, transparent outside it and inside the
/// label, where the cover is drawn separately.
pub fn render(finish: Finish, colours: &CoverColours, size: u32) -> RgbaImage {
    let size = size.max(16);
    let centre = size as f32 / 2.0;
    let outer = centre - 1.0;
    let label = LABEL_RATIO * size as f32 / 2.0;
    let streaks = match finish {
        Finish::Splatter => Streaks::new(colours, outer, label),
        _ => Streaks::default(),
    };
    let mut image = RgbaImage::new(size, size);
    for y in 0..size {
        for x in 0..size {
            let dx = x as f32 + 0.5 - centre;
            let dy = y as f32 + 0.5 - centre;
            let r = (dx * dx + dy * dy).sqrt();
            // Antialiased edges of the disc and of the label hole.
            let coverage = (outer - r + 0.5).clamp(0.0, 1.0) * (r - label + 0.5).clamp(0.0, 1.0);
            if coverage <= 0.0 {
                continue;
            }
            let t = (r - label) / (outer - label);
            let groove = grooves(r, t, size);
            let grain = noise2(x as f32 * 0.9, y as f32 * 0.9, 7) - 0.5;
            let (rgb, alpha) = match finish {
                Finish::Black => {
                    let v = 14.0 + groove * 14.0 + grain * 6.0;
                    ([v * 0.95, v * 0.95, v * 1.05], 1.0)
                }
                Finish::Tinted => {
                    let base = tint(colours);
                    let shade = 0.78 + groove * 0.3 + grain * 0.05;
                    (
                        [base[0] * shade, base[1] * shade, base[2] * shade],
                        0.72 + groove * 0.1,
                    )
                }
                Finish::Splatter => {
                    let smoke = [52.0, 48.0, 52.0];
                    let shade = 0.8 + groove * 0.35 + grain * 0.06;
                    let mut rgb = [smoke[0] * shade, smoke[1] * shade, smoke[2] * shade];
                    let mut alpha = 0.78 + groove * 0.08;
                    let angle = dy.atan2(dx);
                    if let Some((colour, amount)) = streaks.at(r, angle) {
                        for c in 0..3 {
                            rgb[c] = rgb[c] * (1.0 - amount) + colour[c] * amount;
                        }
                        alpha = alpha * (1.0 - amount) + 0.97 * amount;
                    }
                    (rgb, alpha)
                }
                Finish::Marble => {
                    let rgb = marble(x as f32, y as f32, size, colours);
                    let shade = 0.82 + groove * 0.3 + grain * 0.05;
                    ([rgb[0] * shade, rgb[1] * shade, rgb[2] * shade], 1.0)
                }
            };
            image.put_pixel(x, y, Rgba(pixel(rgb, alpha * coverage)));
        }
    }
    image
}

fn pixel(rgb: [f32; 3], alpha: f32) -> [u8; 4] {
    let byte = |v: f32| v.round().clamp(0.0, 255.0) as u8;
    [
        byte(rgb[0]),
        byte(rgb[1]),
        byte(rgb[2]),
        byte(alpha * 255.0),
    ]
}

/// The groove pattern at radius `r` (`t` from the label, 0, to the edge, 1): about 0..1, with a
/// few smooth wider gaps between "tracks", a lead-in near the edge and a run-out by the label.
fn grooves(r: f32, t: f32, size: u32) -> f32 {
    let pitch = (size as f32 / 340.0).max(2.2);
    let fine = 0.5 + 0.5 * (r / pitch * TAU).sin();
    let mut amount = 0.6;
    for gap in [0.24, 0.43, 0.6, 0.78] {
        let d = (t - gap).abs();
        if d < 0.012 {
            amount *= d / 0.012;
        }
    }
    if t > 0.965 || t < 0.05 {
        amount *= 0.25;
    }
    0.2 + fine * amount
}

/// The translucent colour of the Tinted finish.
fn tint(colours: &CoverColours) -> [f32; 3] {
    if colours.neutral {
        return [70.0, 70.0, 76.0];
    }
    let (h, s, _) = hsv(colours.main);
    // Keep the hue, enough saturation to read as coloured vinyl, dark enough to read as vinyl.
    let rgb = from_hsv(h, s.max(0.45), 0.55);
    [f32::from(rgb[0]), f32::from(rgb[1]), f32::from(rgb[2])]
}

/// Marble: swirls of the cover's colours over black, from warped value noise.
fn marble(x: f32, y: f32, size: u32, colours: &CoverColours) -> [f32; 3] {
    let scale = 6.0 / size as f32;
    let (px, py) = (x * scale, y * scale);
    let warp_x = fbm(px + 3.1, py + 1.7, 11);
    let warp_y = fbm(px + 8.3, py + 2.9, 13);
    let n = fbm(px + 2.6 * warp_x, py + 2.6 * warp_y, 17);
    let band = (n * 9.0 * PI).sin() * 0.5 + 0.5;
    let pick = fbm(px * 0.7 + 5.0, py * 0.7 + 4.0, 19);
    let veins = if pick > 0.5 {
        colours.accents[0]
    } else {
        colours.main
    };
    let black = [10.0, 10.0, 12.0];
    let vein = [
        f32::from(veins[0]) * 0.8,
        f32::from(veins[1]) * 0.8,
        f32::from(veins[2]) * 0.8,
    ];
    let amount = smooth(0.55, 0.95, band) * 0.85;
    [
        black[0] + (vein[0] - black[0]) * amount,
        black[1] + (vein[1] - black[1]) * amount,
        black[2] + (vein[2] - black[2]) * amount,
    ]
}

fn smooth(edge0: f32, edge1: f32, v: f32) -> f32 {
    let t = ((v - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Splatter's streaks, kept by angle so each pixel checks only its neighbours.
#[derive(Default)]
struct Streaks {
    bins: Vec<Vec<Streak>>,
}

#[derive(Clone, Copy)]
struct Streak {
    angle: f32,
    start: f32,
    length: f32,
    width: f32,
    /// How much the streak bends, in radians over its length.
    bend: f32,
    /// A seed for the waviness of its edge.
    seed: u64,
    colour: [f32; 3],
}

const BINS: usize = 360;

impl Streaks {
    fn new(colours: &CoverColours, outer: f32, label: f32) -> Streaks {
        let mut rng = XorShift(0x9e37_79b9_7f4a_7c15);
        let accent = colours.main;
        let mut bins = vec![Vec::new(); BINS];
        // Streaks thrown outwards, then small droplets.
        for i in 0..300 {
            let droplet = i >= 170;
            let angle = rng.next() * TAU - PI;
            let start = label + 4.0 + rng.next() * (outer - label) * 0.8;
            let (length, width) = if droplet {
                let w = outer * (0.002 + rng.next() * 0.005);
                (w * 2.0, w)
            } else {
                (
                    (outer - label) * (0.05 + rng.next() * 0.3),
                    outer * (0.004 + rng.next() * 0.012),
                )
            };
            let white = rng.next() < 0.72;
            let colour = if white {
                [238.0, 236.0, 232.0]
            } else {
                [
                    f32::from(accent[0]),
                    f32::from(accent[1]),
                    f32::from(accent[2]),
                ]
            };
            let bend = (rng.next() - 0.5) * 0.12;
            let seed = u64::from(i as u32) + 101;
            let streak = Streak {
                angle,
                start,
                length,
                width,
                bend,
                seed,
                colour,
            };
            // A streak covers at most its width at its start radius: a few bins around it.
            let span = (width * 1.6 / (start - width * 1.6).max(1.0)).atan() + bend.abs() + 0.02;
            let lo = bin(angle - span);
            let hi = bin(angle + span);
            let mut b = lo;
            loop {
                bins[b].push(streak);
                if b == hi {
                    break;
                }
                b = (b + 1) % BINS;
            }
        }
        Streaks { bins }
    }

    /// The streak colour at radius `r` and `angle`, and how much of it covers the pixel.
    fn at(&self, r: f32, angle: f32) -> Option<([f32; 3], f32)> {
        let list = self.bins.get(bin(angle))?;
        let mut best: Option<([f32; 3], f32)> = None;
        for s in list {
            let dr = r - s.start;
            let head = s.width * 1.35;
            if dr < -head - 2.0 || dr > s.length + 2.0 {
                continue;
            }
            let along = (dr / s.length).clamp(0.0, 1.0);
            let mut d = angle - (s.angle + s.bend * along);
            d = (d + PI).rem_euclid(TAU) - PI;
            let across = (d * r).abs();
            // A round drop at the start, thinning to a point as it is thrown outwards, its edge
            // a little uneven and soft.
            let wobble = 0.75 + 0.5 * noise2(along * 7.0, 0.5, s.seed);
            let half = s.width * (1.0 - along).powf(0.8) * wobble;
            let body = if (0.0..=s.length).contains(&dr) {
                (half - across) / 1.6 + 0.5
            } else {
                0.0
            };
            let drop = (head - (dr * dr + across * across).sqrt()) / 1.6 + 0.5;
            let amount = body.max(drop).clamp(0.0, 1.0) * 0.94;
            if amount > best.map_or(0.0, |b| b.1) {
                best = Some((s.colour, amount));
            }
        }
        best
    }
}

fn bin(angle: f32) -> usize {
    let a = (angle + PI).rem_euclid(TAU) / TAU;
    ((a * BINS as f32) as usize).min(BINS - 1)
}

/// A small deterministic generator: no `rand` dependency.
struct XorShift(u64);

impl XorShift {
    /// The next number in `0..1`.
    fn next(&mut self) -> f32 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        (x >> 40) as f32 / (1u64 << 24) as f32
    }
}

/// Value noise in `0..1` at (`x`, `y`), from a hashed lattice, smoothly interpolated.
fn noise2(x: f32, y: f32, seed: u64) -> f32 {
    let (x0, y0) = (x.floor(), y.floor());
    let (fx, fy) = (x - x0, y - y0);
    let corner = |dx: f32, dy: f32| {
        let h = lattice((x0 + dx) as i64, (y0 + dy) as i64, seed);
        (h >> 40) as f32 / (1u64 << 24) as f32
    };
    let sx = fx * fx * (3.0 - 2.0 * fx);
    let sy = fy * fy * (3.0 - 2.0 * fy);
    let top = corner(0.0, 0.0) + (corner(1.0, 0.0) - corner(0.0, 0.0)) * sx;
    let bottom = corner(0.0, 1.0) + (corner(1.0, 1.0) - corner(0.0, 1.0)) * sx;
    top + (bottom - top) * sy
}

/// A well-mixed hash of a lattice point (splitmix64's finaliser).
fn lattice(x: i64, y: i64, seed: u64) -> u64 {
    let mut z = (x as u64)
        .wrapping_mul(0x9e37_79b9_7f4a_7c15)
        .wrapping_add((y as u64).wrapping_mul(0xc2b2_ae3d_27d4_eb4f))
        .wrapping_add(seed.wrapping_mul(0x1656_67b1_9e37_79f9));
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

/// Three octaves of [`noise2`], in about `0..1`.
fn fbm(x: f32, y: f32, seed: u64) -> f32 {
    let mut total = 0.0;
    let mut amplitude = 0.5;
    let mut frequency = 1.0;
    for octave in 0..3u64 {
        total += amplitude * noise2(x * frequency, y * frequency, seed + octave);
        amplitude *= 0.5;
        frequency *= 2.0;
    }
    total / 0.875
}

fn hsv(rgb: [u8; 3]) -> (f32, f32, f32) {
    let [r, g, b] = rgb.map(|c| f32::from(c) / 255.0);
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let delta = max - min;
    let hue = if delta == 0.0 {
        0.0
    } else if max == r {
        60.0 * ((g - b) / delta).rem_euclid(6.0)
    } else if max == g {
        60.0 * ((b - r) / delta + 2.0)
    } else {
        60.0 * ((r - g) / delta + 4.0)
    };
    let saturation = if max == 0.0 { 0.0 } else { delta / max };
    (hue, saturation, max)
}

fn from_hsv(hue: f32, saturation: f32, value: f32) -> [u8; 3] {
    let c = value * saturation;
    let h = hue.rem_euclid(360.0) / 60.0;
    let x = c * (1.0 - (h.rem_euclid(2.0) - 1.0).abs());
    let (r, g, b) = match h as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = value - c;
    [r, g, b].map(|v| ((v + m) * 255.0).round().clamp(0.0, 255.0) as u8)
}

fn distance(a: [u8; 3], b: [u8; 3]) -> f32 {
    let d: f32 = (0..3)
        .map(|c| (f32::from(a[c]) - f32::from(b[c])).powi(2))
        .sum();
    d.sqrt()
}

fn scale(rgb: [u8; 3], factor: f32) -> [u8; 3] {
    rgb.map(|c| (f32::from(c) * factor).round().clamp(0.0, 255.0) as u8)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Small enough for a debug build to render every finish quickly.
    const TEST_SIZE: u32 = 384;

    fn alpha(image: &RgbaImage, x: u32, y: u32) -> u8 {
        image.get_pixel(x, y).0[3]
    }

    fn solid(rgb: [u8; 3]) -> Vec<u8> {
        (0..64 * 64)
            .flat_map(|_| [rgb[0], rgb[1], rgb[2], 255])
            .collect()
    }

    #[test]
    fn each_finish_is_a_disc_of_the_size() {
        let colours = cover_colours(&solid([200, 30, 30]), 64, 64);
        for finish in DRAWN {
            let image = render(finish, &colours, TEST_SIZE);
            assert_eq!(image.dimensions(), (TEST_SIZE, TEST_SIZE));
            let c = TEST_SIZE / 2;
            assert_eq!(alpha(&image, 0, 0), 0, "{finish:?} corner");
            assert_eq!(alpha(&image, c, c), 0, "{finish:?} label hole");
            let groove = c + (TEST_SIZE as f32 * 0.45) as u32;
            assert!(alpha(&image, groove, c) > 0, "{finish:?} grooves");
        }
        assert_eq!(
            render(Finish::Black, &colours, TEXTURE_SIZE).dimensions(),
            (1024, 1024)
        );
    }

    #[test]
    fn rendering_is_deterministic() {
        let colours = cover_colours(&solid([30, 120, 200]), 64, 64);
        for finish in DRAWN {
            assert_eq!(
                render(finish, &colours, TEST_SIZE),
                render(finish, &colours, TEST_SIZE),
                "{finish:?}"
            );
        }
    }

    /// The mean alpha over a ring well inside the grooves.
    fn ring_alpha(image: &RgbaImage) -> f32 {
        let c = image.width() as f32 / 2.0;
        let (mut sum, mut n) = (0.0, 0.0);
        for (x, y, p) in image.enumerate_pixels() {
            let r = ((x as f32 + 0.5 - c).powi(2) + (y as f32 + 0.5 - c).powi(2)).sqrt();
            if (0.45 * c..0.9 * c).contains(&r) {
                sum += f32::from(p.0[3]);
                n += 1.0;
            }
        }
        sum / n
    }

    #[test]
    fn tinted_is_see_through() {
        let colours = cover_colours(&solid([200, 30, 30]), 64, 64);
        assert!(ring_alpha(&render(Finish::Tinted, &colours, TEST_SIZE)) < 230.0);
        assert!(ring_alpha(&render(Finish::Black, &colours, TEST_SIZE)) > 254.0);
    }

    #[test]
    fn a_grey_cover_is_neutral() {
        assert!(cover_colours(&solid([128, 128, 128]), 64, 64).neutral);
        assert!(cover_colours(&solid([3, 3, 3]), 64, 64).neutral);
        assert!(cover_colours(&[], 0, 0).neutral);
    }

    #[test]
    fn a_red_cover_gives_red() {
        let colours = cover_colours(&solid([200, 30, 30]), 64, 64);
        assert!(!colours.neutral);
        for (got, want) in colours.main.iter().zip([200u8, 30, 30]) {
            assert!(got.abs_diff(want) <= 20, "{:?}", colours.main);
        }
    }

    #[test]
    fn random_is_stable_per_track() {
        let a = resolve(VinylFinish::Random, "spotify:track:x");
        assert_eq!(a, resolve(VinylFinish::Random, "spotify:track:x"));
        let seen: std::collections::HashSet<Finish> = (0..40)
            .map(|i| resolve(VinylFinish::Random, &format!("C:/Music/{i}.flac")))
            .collect();
        assert!(seen.len() >= 3, "{seen:?}");
        assert_eq!(resolve(VinylFinish::Marble, "x"), Finish::Marble);
    }
}
