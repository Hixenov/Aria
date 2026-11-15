//! Proves that a system-wide hotkey reaches an eframe window, also while the window is not
//! focused or is minimized. The soundboard's hotkeys depend on this.
//!
//! Run: `cargo run -p aria-app --example hotkey_probe`, then press Ctrl+Alt+F15. The probe
//! prints `hotkey pressed` and `hotkey released` with the time since start, and closes after
//! 20 seconds, or shortly after the first press (once the release arrived, at most 2 s later).

use std::time::{Duration, Instant};

use global_hotkey::hotkey::{Code, HotKey, Modifiers};
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};

const LIFETIME: Duration = Duration::from_secs(20);
const RELEASE_GRACE: Duration = Duration::from_secs(2);
const REPAINT_EVERY: Duration = Duration::from_millis(50);

struct Probe {
    started: Instant,
    hotkey_id: u32,
    first_press: Option<Instant>,
    released: bool,
    // Dropping the manager unregisters the hotkey, so it lives as long as the window.
    _manager: GlobalHotKeyManager,
}

impl eframe::App for Probe {
    fn ui(&mut self, ui: &mut eframe::egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        while let Ok(event) = GlobalHotKeyEvent::receiver().try_recv() {
            if event.id != self.hotkey_id {
                continue;
            }
            let at = self.started.elapsed().as_secs_f32();
            match event.state {
                HotKeyState::Pressed => {
                    println!("hotkey pressed (t = {at:.2} s)");
                    self.first_press.get_or_insert_with(Instant::now);
                }
                HotKeyState::Released => {
                    println!("hotkey released (t = {at:.2} s)");
                    self.released = true;
                }
            }
        }
        let done = self.started.elapsed() >= LIFETIME
            || self
                .first_press
                .is_some_and(|at| self.released || at.elapsed() >= RELEASE_GRACE);
        if done {
            ctx.send_viewport_cmd(eframe::egui::ViewportCommand::Close);
        }
        ui.label("Press Ctrl+Alt+F15");
        ctx.request_repaint_after(REPAINT_EVERY);
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // The manager must be created on the main thread, before the event loop starts.
    let manager = GlobalHotKeyManager::new()?;
    let hotkey = HotKey::new(Some(Modifiers::CONTROL | Modifiers::ALT), Code::F15);
    manager.register(hotkey)?;
    println!(
        "registered Ctrl+Alt+F15; waiting up to {} s",
        LIFETIME.as_secs()
    );
    let probe = Probe {
        started: Instant::now(),
        hotkey_id: hotkey.id(),
        first_press: None,
        released: false,
        _manager: manager,
    };
    eframe::run_native(
        "Aria hotkey probe",
        eframe::NativeOptions::default(),
        Box::new(|_cc| Ok(Box::new(probe))),
    )?;
    Ok(())
}
