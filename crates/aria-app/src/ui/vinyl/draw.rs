//! Drawing the turntable: the record and its label turned as textured meshes, a still sheen
//! over them, the sleeve, and the tonearm. Finishes are rendered on a thread and cached.

use std::f32::consts::{FRAC_PI_2, TAU};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::thread;

use eframe::egui::{
    self, Align2, Color32, FontId, Painter, Pos2, Rect, Shape, Stroke, TextureHandle, Vec2,
    epaint::{Mesh, TextShape, Vertex},
    pos2, vec2,
};
use image::RgbaImage;

use super::finish::{CoverColours, Finish, LABEL_RATIO, TEXTURE_SIZE, render};
use super::layout::Layout;

/// For textures drawn smaller than they are and turning: mipmaps keep fine grooves from
/// shimmering.
pub(crate) const TURNING: egui::TextureOptions = egui::TextureOptions {
    mipmap_mode: Some(egui::TextureFilter::Linear),
    ..egui::TextureOptions::LINEAR
};
use crate::artwork::paint_cover;

/// Segments of a circle drawn as a mesh.
const SEGMENTS: usize = 128;
/// The stylus rests on the outer grooves at this share of the record's radius.
const STYLUS_AT: f32 = 0.92;

/// What the label shows.
pub enum Label<'a> {
    Cover(&'a TextureHandle),
    Plain {
        colour: Color32,
        title: &'a str,
        artist: &'a str,
    },
}

/// Colours of the turntable's parts, from the theme.
#[derive(Clone, Copy)]
pub struct ArmColours {
    pub body: Color32,
    pub tube: Color32,
    pub outline: Color32,
}

impl ArmColours {
    pub fn for_theme(dark: bool) -> ArmColours {
        if dark {
            ArmColours {
                body: Color32::from_rgb(30, 30, 33),
                tube: Color32::from_rgb(170, 172, 178),
                outline: Color32::from_rgba_unmultiplied(255, 255, 255, 28),
            }
        } else {
            ArmColours {
                body: Color32::from_rgb(244, 241, 235),
                tube: Color32::from_rgb(196, 198, 204),
                outline: Color32::from_rgba_unmultiplied(0, 0, 0, 30),
            }
        }
    }
}

fn rotate(v: Vec2, angle: f32) -> Vec2 {
    let (s, c) = angle.sin_cos();
    vec2(v.x * c - v.y * s, v.x * s + v.y * c)
}

/// The record: its shadow, the finish (or a plain black disc), the label, the spindle, and a
/// still sheen.
pub fn draw_record(
    painter: &Painter,
    record: Rect,
    angle: f32,
    finish: Option<&TextureHandle>,
    label: Label<'_>,
    sheen: Color32,
) {
    let centre = record.center();
    let r = record.width() / 2.0;
    for (grow, alpha) in [(1.04, 18), (1.02, 26), (1.0, 34)] {
        painter.circle_filled(
            centre + vec2(r * 0.02, r * 0.035),
            r * grow,
            Color32::from_black_alpha(alpha),
        );
    }
    match finish {
        Some(texture) => {
            let mut mesh = Mesh::with_texture(texture.id());
            let corners = [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)];
            for (x, y) in corners {
                let pos = centre + rotate(vec2(x * r, y * r), angle);
                let uv = pos2((x + 1.0) / 2.0, (y + 1.0) / 2.0);
                mesh.vertices.push(Vertex {
                    pos,
                    uv,
                    color: Color32::WHITE,
                });
            }
            mesh.add_triangle(0, 1, 2);
            mesh.add_triangle(0, 2, 3);
            painter.add(Shape::mesh(mesh));
        }
        None => {
            painter.circle_filled(centre, r, Color32::from_rgb(14, 14, 16));
        }
    }
    let label_r = r * LABEL_RATIO;
    match label {
        Label::Cover(texture) => {
            painter.add(Shape::mesh(cover_disc(texture, centre, label_r, angle)));
        }
        Label::Plain {
            colour,
            title,
            artist,
        } => {
            painter.circle_filled(centre, label_r, colour);
            let ink = if colour.intensity() > 0.55 {
                Color32::from_rgb(24, 24, 26)
            } else {
                Color32::from_rgb(240, 238, 232)
            };
            for (text, size, offset) in [
                (title, label_r * 0.16, -0.22),
                (artist, label_r * 0.11, 0.28),
            ] {
                let galley = painter.layout(
                    text.to_owned(),
                    FontId::proportional(size.max(8.0)),
                    ink,
                    label_r * 1.5,
                );
                let half = galley.size() / 2.0;
                let at = centre + rotate(vec2(0.0, offset * label_r), angle) - rotate(half, angle);
                painter.add(TextShape::new(at, galley, ink).with_angle(angle));
            }
        }
    }
    painter.circle_filled(centre, r * 0.02, Color32::from_rgb(20, 20, 22));
    painter.circle_stroke(
        centre,
        r * 0.02,
        Stroke::new(1.0, Color32::from_white_alpha(40)),
    );
    painter.add(Shape::mesh(sheen_ring(centre, label_r, r, sheen)));
}

/// The cover as a disc turned by `angle`, cropped to its middle square.
fn cover_disc(texture: &TextureHandle, centre: Pos2, radius: f32, angle: f32) -> Mesh {
    let [w, h] = texture.size();
    let (w, h) = (w.max(1) as f32, h.max(1) as f32);
    let (kx, ky) = if w > h { (h / w, 1.0) } else { (1.0, w / h) };
    let mut mesh = Mesh::with_texture(texture.id());
    mesh.vertices.push(Vertex {
        pos: centre,
        uv: pos2(0.5, 0.5),
        color: Color32::WHITE,
    });
    for i in 0..=SEGMENTS {
        let a = i as f32 / SEGMENTS as f32 * TAU;
        let (s, c) = a.sin_cos();
        mesh.vertices.push(Vertex {
            pos: centre + rotate(vec2(c, s) * radius, angle),
            uv: pos2(0.5 + 0.5 * c * kx, 0.5 + 0.5 * s * ky),
            color: Color32::WHITE,
        });
        if i > 0 {
            mesh.add_triangle(0, i as u32, i as u32 + 1);
        }
    }
    mesh
}

/// A soft highlight that stays put while the record turns: brightest at two opposite angles,
/// as a single light gives on real vinyl.
fn sheen_ring(centre: Pos2, inner: f32, outer: f32, colour: Color32) -> Mesh {
    let mut mesh = Mesh::default();
    let light = -0.75_f32;
    for i in 0..=SEGMENTS {
        let a = i as f32 / SEGMENTS as f32 * TAU;
        let strength = (2.0 * (a - light)).cos().max(0.0).powf(3.0);
        let c = colour.gamma_multiply(strength);
        let dir = vec2(a.cos(), a.sin());
        mesh.colored_vertex(centre + dir * inner, c.gamma_multiply(0.4));
        mesh.colored_vertex(centre + dir * outer, c);
        if i > 0 {
            let k = i as u32 * 2;
            mesh.add_triangle(k - 2, k - 1, k);
            mesh.add_triangle(k - 1, k, k + 1);
        }
    }
    mesh
}

/// The sleeve: the large cover with a soft shadow.
pub fn draw_sleeve(painter: &Painter, sleeve: Rect, cover: Option<&TextureHandle>, blank: Color32) {
    for (grow, alpha) in [(10.0, 14), (5.0, 22), (2.0, 30)] {
        painter.rect_filled(
            sleeve.translate(vec2(4.0, 8.0)).expand(grow),
            8.0,
            Color32::from_black_alpha(alpha),
        );
    }
    match cover {
        Some(texture) => paint_cover(painter, sleeve, 6, texture),
        None => {
            painter.rect_filled(sleeve, 6.0, blank);
        }
    }
}

/// The tonearm, from rest (`arm = 0`, off the record) to playing (`arm = 1`, on the outer
/// grooves).
pub fn draw_arm(painter: &Painter, layout: &Layout, arm: f32, colours: ArmColours) {
    let d = layout.record.width();
    let pivot = layout.arm_pivot;
    let length = 0.62 * d;
    let rest = FRAC_PI_2 - 0.1;
    let play = play_angle(layout, length, rest);
    let angle = rest + (play - rest) * arm.clamp(0.0, 1.0);
    let dir = vec2(angle.cos(), angle.sin());
    let tip = pivot + dir * length;
    // The base, the tube, and the headshell.
    painter.circle_filled(pivot, d * 0.07, colours.body);
    painter.circle_stroke(pivot, d * 0.07, Stroke::new(1.0, colours.outline));
    painter.line_segment([pivot, tip], Stroke::new(d * 0.014, colours.tube));
    painter.circle_filled(pivot, d * 0.035, colours.tube);
    let across = vec2(-dir.y, dir.x);
    let head = [
        tip - across * d * 0.03,
        tip + across * d * 0.03,
        tip + across * d * 0.03 + dir * d * 0.07,
        tip - across * d * 0.03 + dir * d * 0.07,
    ];
    painter.add(Shape::convex_polygon(
        head.to_vec(),
        colours.body,
        Stroke::new(1.0, colours.outline),
    ));
}

/// The arm's angle at which the stylus sits on the outer grooves, found by bisection between
/// rest and a quarter turn towards the record.
fn play_angle(layout: &Layout, length: f32, rest: f32) -> f32 {
    let centre = layout.record.center();
    let target = layout.record.width() / 2.0 * STYLUS_AT;
    let at =
        |angle: f32| (layout.arm_pivot + vec2(angle.cos(), angle.sin()) * length).distance(centre);
    let (mut lo, mut hi) = (rest, rest + FRAC_PI_2);
    for _ in 0..40 {
        let mid = (lo + hi) / 2.0;
        if at(mid) > target {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    (lo + hi) / 2.0
}

/// Finishes rendered on a thread, keyed by what was rendered. The newest render for the current
/// key is shown; until it arrives, the last one shown stays.
pub struct FinishCache {
    wanted: Option<u64>,
    shown: Option<(u64, TextureHandle)>,
    sender: Sender<(u64, RgbaImage)>,
    arrived: Receiver<(u64, RgbaImage)>,
}

impl Default for FinishCache {
    fn default() -> FinishCache {
        let (sender, arrived) = channel();
        FinishCache {
            wanted: None,
            shown: None,
            sender,
            arrived,
        }
    }
}

impl FinishCache {
    /// Asks for `finish` in `colours`, unless `key` is already wanted.
    pub fn want(&mut self, ctx: &egui::Context, finish: Finish, colours: CoverColours, key: u64) {
        if self.wanted == Some(key) {
            return;
        }
        self.wanted = Some(key);
        let (sender, ctx) = (self.sender.clone(), ctx.clone());
        let spawned = thread::Builder::new()
            .name("aria-vinyl".to_owned())
            .spawn(move || {
                let image = render(finish, &colours, TEXTURE_SIZE);
                // Fails only once the mode has gone, when nobody wants it.
                let _ = sender.send((key, image));
                ctx.request_repaint();
            });
        if spawned.is_err() {
            tracing::debug!("no thread for a record finish");
        }
    }

    /// The texture to show: the current key's once rendered, else the last one shown.
    pub fn texture(&mut self, ctx: &egui::Context) -> Option<&TextureHandle> {
        while let Ok((key, image)) = self.arrived.try_recv() {
            if Some(key) != self.wanted {
                continue;
            }
            let size = [image.width() as usize, image.height() as usize];
            let pixels = egui::ColorImage::from_rgba_unmultiplied(size, image.as_raw());
            let texture = ctx.load_texture("aria-vinyl", pixels, TURNING);
            self.shown = Some((key, texture));
        }
        self.shown.as_ref().map(|(_, texture)| texture)
    }

    /// The key of the texture shown, if any.
    pub fn shown_key(&self) -> Option<u64> {
        self.shown.as_ref().map(|(key, _)| *key)
    }

    pub fn clear(&mut self) {
        self.wanted = None;
        self.shown = None;
    }
}

/// A key for a finish in some colours.
pub fn finish_key(finish: Finish, colours: &CoverColours) -> u64 {
    use std::hash::{DefaultHasher, Hash, Hasher};
    let mut hasher = DefaultHasher::new();
    (finish, colours).hash(&mut hasher);
    hasher.finish()
}

/// Where to anchor text at the bottom left, for the title block.
pub const TITLE_ANCHOR: Align2 = Align2::LEFT_BOTTOM;

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    #[test]
    fn a_stale_render_is_dropped() {
        let ctx = egui::Context::default();
        let mut cache = FinishCache::default();
        let neutral = CoverColours::neutral();
        let a = finish_key(Finish::Black, &neutral);
        let b = finish_key(Finish::Marble, &neutral);
        cache.want(&ctx, Finish::Black, neutral, a);
        cache.want(&ctx, Finish::Marble, neutral, b);
        let deadline = Instant::now() + Duration::from_secs(60);
        while cache.shown_key() != Some(b) {
            assert!(Instant::now() < deadline, "the render never came");
            let _ = cache.texture(&ctx);
            thread::sleep(Duration::from_millis(20));
        }
        thread::sleep(Duration::from_millis(500));
        let _ = cache.texture(&ctx);
        assert_eq!(
            cache.shown_key(),
            Some(b),
            "the older render never replaces it"
        );
    }

    #[test]
    fn the_record_texture_has_mipmaps() {
        // Grooves a few pixels apart, drawn smaller and turning, shimmer without them.
        let ctx = egui::Context::default();
        let mut cache = FinishCache::default();
        let neutral = CoverColours::neutral();
        cache.want(
            &ctx,
            Finish::Black,
            neutral,
            finish_key(Finish::Black, &neutral),
        );
        let deadline = Instant::now() + Duration::from_secs(60);
        let id = loop {
            if let Some(texture) = cache.texture(&ctx) {
                break texture.id();
            }
            assert!(Instant::now() < deadline, "the render never came");
            thread::sleep(Duration::from_millis(20));
        };
        let mode = ctx
            .tex_manager()
            .read()
            .meta(id)
            .map(|m| m.options.mipmap_mode);
        assert_eq!(mode, Some(Some(egui::TextureFilter::Linear)));
    }

    #[test]
    fn the_arm_reaches_the_outer_grooves() {
        let layout =
            super::super::layout::layout(Rect::from_min_size(Pos2::ZERO, vec2(1920.0, 1080.0)));
        let d = layout.record.width();
        let length = 0.62 * d;
        let rest = FRAC_PI_2 - 0.1;
        let play = play_angle(&layout, length, rest);
        let tip = layout.arm_pivot + vec2(play.cos(), play.sin()) * length;
        let from_centre = tip.distance(layout.record.center());
        assert!(
            (from_centre - d / 2.0 * STYLUS_AT).abs() < 1.0,
            "{from_centre}"
        );
        let rest_tip = layout.arm_pivot + vec2(rest.cos(), rest.sin()) * length;
        assert!(
            rest_tip.distance(layout.record.center()) > d / 2.0,
            "at rest the stylus is off the record"
        );
    }
}
