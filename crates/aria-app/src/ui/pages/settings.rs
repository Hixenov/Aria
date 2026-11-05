//! The Settings page. So far it has only its heading.

use eframe::egui::Ui;

use super::PageCtx;
use crate::{nav::Route, ui::widgets::section_header};

/// Draws the page.
pub fn show(ui: &mut Ui, ctx: &mut PageCtx<'_>) {
    section_header(ui, &ctx.tr.tr(Route::Settings.label_key()));
}
