//! The Artists page. Its content comes in the phase named by [`Route::phase`].

use eframe::egui::Ui;

use super::PageCtx;
use crate::nav::Route;

/// Draws the page.
pub fn show(ui: &mut Ui, ctx: &mut PageCtx<'_>) {
    super::pending(ui, ctx, Route::Artists);
}
