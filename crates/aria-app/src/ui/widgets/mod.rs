//! Reusable widgets. Colors come only from the active [`Palette`](crate::ui::theme::Palette).

mod empty_state;
mod icon_button;
mod nav_item;
mod scrub;
mod section_header;
mod toast;

pub use empty_state::{empty_state, empty_state_with};
pub use icon_button::IconButton;
pub use nav_item::{NAV_ROW_HEIGHT, nav_item};
pub use scrub::{Moved, Scrub};
pub use section_header::section_header;
pub use toast::Toasts;

/// Stroke width of the keyboard focus ring.
const FOCUS_RING_WIDTH: f32 = 1.5;
