//! The shell's panels around the page: sidebar, player bar and right panel.
//!
//! Each panel is filled with `surface`, and egui draws a 1 px `border` line on its inner
//! edge.

pub mod player_bar;
pub mod right_panel;
pub mod sidebar;

use eframe::egui::{Frame, Margin};

use crate::ui::theme::Palette;

/// A panel frame filled with `surface`.
fn surface_frame(p: &Palette, margin: impl Into<Margin>) -> Frame {
    Frame::new().fill(p.surface).inner_margin(margin)
}
