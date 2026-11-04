//! Aria desktop music player entry point.

use eframe::egui;

/// Application name; also the window title.
const APP_NAME: &str = "Aria";

struct AriaApp;

impl eframe::App for AriaApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ui, |_ui| {});
    }
}

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(APP_NAME)
            .with_inner_size([1200.0, 780.0])
            .with_min_inner_size([900.0, 600.0]),
        persist_window: true,
        renderer: eframe::Renderer::Glow,
        ..Default::default()
    };
    eframe::run_native(APP_NAME, options, Box::new(|_cc| Ok(Box::new(AriaApp))))
}
