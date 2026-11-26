//! The update checker in the window: the one-time question, the status text, and starting the
//! installer once Aria has shut down.

use std::io;
use std::path::Path;

use aria_core::i18n::{FluentArgs, Localizer};
use aria_core::settings::{Settings, UpdateChecks};
use aria_core::update_service::UpdateStatus;
use aria_update::UpdateError;
use eframe::egui::{self, Frame, Label, Margin, RichText, Ui};

use crate::ui::theme::{Palette, fonts, visuals::SPACE};

/// What the installer is started with: no questions, no reboot, and Aria started again when done
/// (the `[Run]` entry in `packaging/windows/aria.iss` that only `/RELAUNCH=1` enables).
pub const INSTALLER_ARGS: [&str; 4] = ["/SILENT", "/SUPPRESSMSGBOXES", "/NORESTART", "/RELAUNCH=1"];

/// Starts the installer that the update service checked; Aria quits right after.
pub type Launcher = Box<dyn Fn(&Path) -> io::Result<()>>;

/// The real [`Launcher`]: runs `installer` with [`INSTALLER_ARGS`] and does not wait for it.
pub fn launch_installer(installer: &Path) -> io::Result<()> {
    std::process::Command::new(installer)
        .args(INSTALLER_ARGS)
        .spawn()
        .map(drop)
}

/// The key that names why a check failed.
fn reason_key(error: &UpdateError) -> &'static str {
    match error {
        UpdateError::Offline => "update-reason-offline",
        UpdateError::Busy => "update-reason-busy",
        UpdateError::Answer | UpdateError::NotARelease | UpdateError::Missing => {
            "update-reason-answer"
        }
        UpdateError::TooBig => "update-reason-too-big",
        UpdateError::Checksum => "update-reason-checksum",
        UpdateError::Disk(_) => "update-reason-disk",
    }
}

/// The line the Settings row shows for `status`.
pub fn status_text(tr: &Localizer, status: &UpdateStatus) -> String {
    let with_version = |key: &str, version: &aria_update::Version| {
        let mut args = FluentArgs::new();
        args.set("version", version.to_string());
        tr.tr_args(key, &args).into_owned()
    };
    match status {
        UpdateStatus::Idle => tr.tr("update-idle").into_owned(),
        UpdateStatus::Checking => tr.tr("update-checking").into_owned(),
        UpdateStatus::UpToDate => tr.tr("update-up-to-date").into_owned(),
        UpdateStatus::NotReady(version) => with_version("update-not-ready", version),
        UpdateStatus::Downloading { version, percent } => {
            let mut args = FluentArgs::new();
            args.set("version", version.to_string());
            args.set("percent", *percent);
            tr.tr_args("update-downloading", &args).into_owned()
        }
        UpdateStatus::Ready(version) => with_version("update-ready", version),
        UpdateStatus::Available { version, .. } => with_version("update-available", version),
        UpdateStatus::Failed(error) => {
            let mut args = FluentArgs::new();
            args.set("reason", tr.tr(reason_key(error)).into_owned());
            tr.tr_args("update-failed", &args).into_owned()
        }
    }
}

/// The text of the sidebar's button, when there is something to install or to see.
pub fn sidebar_text(tr: &Localizer, status: &UpdateStatus) -> Option<String> {
    match status {
        UpdateStatus::Ready(_) => Some(tr.tr("update-restart").into_owned()),
        UpdateStatus::Available { .. } => Some(status_text(tr, status)),
        _ => None,
    }
}

/// Asks once whether Aria may check for updates, while the user has not answered. Returns the
/// settings with the answer.
pub fn card(ui: &mut Ui, tr: &Localizer, p: &Palette, settings: &Settings) -> Option<Settings> {
    if settings.update_checks != UpdateChecks::NotAsked {
        return None;
    }
    let mut answer = None;
    let frame = Frame::new()
        .fill(p.surface)
        .corner_radius(8)
        .inner_margin(Margin::same(SPACE[3] as i8));
    frame.show(ui, |ui| {
        ui.set_width(ui.available_width());
        let title = RichText::new(tr.tr("update-card-title"))
            .font(fonts::semibold(15.0))
            .color(p.text);
        ui.add(Label::new(title).selectable(false));
        ui.add_space(SPACE[1]);
        let body = RichText::new(tr.tr("update-card-body")).color(p.text_muted);
        ui.add(Label::new(body).selectable(false).wrap());
        ui.add_space(SPACE[2]);
        ui.horizontal(|ui| {
            if ui.button(tr.tr("update-card-on")).clicked() {
                answer = Some(UpdateChecks::On);
            }
            if ui.button(tr.tr("update-card-later")).clicked() {
                answer = Some(UpdateChecks::Off);
            }
        });
    });
    ui.add_space(SPACE[3]);
    answer.map(|update_checks| Settings {
        update_checks,
        ..settings.clone()
    })
}

/// Opens `url` in the browser.
pub fn open_page(ui: &Ui, url: &str) {
    ui.ctx().open_url(egui::OpenUrl::new_tab(url));
}

#[cfg(test)]
mod tests {
    use super::*;
    use aria_core::settings::Language;

    #[test]
    fn every_failure_has_its_reason() {
        let tr = Localizer::new(Language::En).unwrap();
        for (error, reason) in [
            (UpdateError::Offline, "no connection to GitHub"),
            (
                UpdateError::Busy,
                "GitHub is busy right now, try again later",
            ),
            (
                UpdateError::NotARelease,
                "GitHub's answer didn't make sense",
            ),
            (
                UpdateError::Checksum,
                "the download didn't match its checksum",
            ),
            (
                UpdateError::Disk(io::ErrorKind::Other),
                "it couldn't be saved on this computer",
            ),
        ] {
            assert_eq!(
                status_text(&tr, &UpdateStatus::Failed(error)),
                format!("Couldn't check: {reason}")
            );
        }
    }

    #[test]
    fn the_installer_is_told_to_start_aria_again() {
        assert!(INSTALLER_ARGS.contains(&"/RELAUNCH=1"));
        assert!(INSTALLER_ARGS.contains(&"/SILENT"));
    }
}
