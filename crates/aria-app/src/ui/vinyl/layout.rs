//! Where things go on the screen: the sleeve and the record side by side on a wide window, the
//! record large and partly off the left edge on a narrow one.

use eframe::egui::{Pos2, Rect, Vec2, pos2, vec2};

/// From this width-to-height ratio on, the sleeve lies beside the record.
pub const WIDE_RATIO: f32 = 1.4;
/// The margin around the title.
const MARGIN: f32 = 48.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Layout {
    /// The sleeve, on a wide window.
    pub sleeve: Option<Rect>,
    /// The record's square.
    pub record: Rect,
    /// Where the tonearm turns, outside the record at its top right.
    pub arm_pivot: Pos2,
    /// The bottom left of the title block.
    pub title: Pos2,
}

pub fn layout(screen: Rect) -> Layout {
    let (w, h) = (screen.width(), screen.height());
    let title = pos2(screen.left() + MARGIN, screen.bottom() - MARGIN * 2.2);
    let (sleeve, record) = if w >= WIDE_RATIO * h {
        let side = 0.62 * h;
        // The sleeve, then the record over its right edge, reaching a quarter past it.
        let group = side * 1.25;
        let left = screen.center().x - 0.05 * w - group / 2.0;
        let top = screen.center().y - 0.04 * h - side / 2.0;
        let sleeve = Rect::from_min_size(pos2(left, top), Vec2::splat(side));
        let centre = pos2(sleeve.right() - side / 4.0, sleeve.center().y);
        (
            Some(sleeve),
            Rect::from_center_size(centre, Vec2::splat(side)),
        )
    } else {
        let d = 1.15 * w.min(h);
        let centre = pos2(screen.left() + 0.3 * d, screen.top() + 0.4 * h);
        (None, Rect::from_center_size(centre, Vec2::splat(d)))
    };
    let d = record.width();
    let arm_pivot = record.center() + vec2(0.4 * d, -0.4 * d);
    Layout {
        sleeve,
        record,
        arm_pivot,
        title,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn screen(w: f32, h: f32) -> Rect {
        Rect::from_min_size(Pos2::ZERO, vec2(w, h))
    }

    /// The share of the record's disc inside `screen`, sampled on a grid.
    fn inside(record: Rect, screen: Rect) -> f32 {
        let (mut all, mut seen) = (0.0, 0.0);
        let r = record.width() / 2.0;
        for i in 0..100 {
            for j in 0..100 {
                let p =
                    record.min + vec2(i as f32 + 0.5, j as f32 + 0.5) * (record.width() / 100.0);
                if p.distance(record.center()) <= r {
                    all += 1.0;
                    if screen.contains(p) {
                        seen += 1.0;
                    }
                }
            }
        }
        seen / all
    }

    #[test]
    fn layout_is_wide_from_1_4() {
        assert!(layout(screen(1400.0, 1000.0)).sleeve.is_some());
        assert!(layout(screen(1399.0, 1000.0)).sleeve.is_none());
    }

    #[test]
    fn the_record_is_on_screen_enough() {
        let narrow = screen(800.0, 1200.0);
        assert!(inside(layout(narrow).record, narrow) >= 0.7);
        let wide = screen(1920.0, 1080.0);
        let l = layout(wide);
        let sleeve = l.sleeve.unwrap();
        assert!(
            sleeve.intersects(l.record),
            "the record lies over the sleeve"
        );
        for rect in [sleeve, l.record] {
            assert!(rect.top() >= 0.0 && rect.bottom() <= 1080.0, "{rect:?}");
        }
        assert!(inside(l.record, wide) > 0.99);
        assert!(wide.contains(l.arm_pivot));
    }
}
