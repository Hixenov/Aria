//! The Spotify card: connect and disconnect, the account, the device to play on, and what
//! Spotify's Development mode and Premium mean for it.

use aria_core::i18n::FluentArgs;
use aria_core::spotify_service::{SpotifyAccount, SpotifyRequest};
use eframe::egui::{ComboBox, Label, RichText, TextStyle, Ui};

use super::{
    COMBO_WIDTH, Dialog, PageAction, PageCtx, buttons_row, caption, divider, group, open_dialog,
    row,
};

pub(super) fn show(ui: &mut Ui, ctx: &mut PageCtx<'_>) {
    let tr = ctx.tr;
    let p = ctx.palette;
    let spotify = ctx.spotify;
    group(ui, &p, &tr.tr("settings-spotify"), |ui| {
        match &spotify.account {
            SpotifyAccount::SignedOut { .. } => {
                buttons_row(ui, |ui| {
                    if ui.button(tr.tr("settings-spotify-connect")).clicked() {
                        ctx.out.push(PageAction::Spotify(SpotifyRequest::SignIn {
                            done_page: String::new(),
                            failed_page: String::new(),
                        }));
                    }
                });
                caption(
                    ui,
                    &p,
                    &tr.tr("settings-spotify-allow-list"),
                    TextStyle::Small,
                );
            }
            SpotifyAccount::SigningIn => {
                row(ui, &p, &tr.tr("settings-spotify-waiting"), |ui| {
                    if ui.button(tr.tr("settings-spotify-cancel")).clicked() {
                        ctx.out
                            .push(PageAction::Spotify(SpotifyRequest::CancelSignIn));
                    }
                });
            }
            SpotifyAccount::SignedIn { name } => {
                let mut args = FluentArgs::new();
                args.set("name", name.as_str());
                let connected = tr.tr_args("settings-spotify-connected", &args);
                ui.add_space(super::SPACE[1]);
                ui.add(Label::new(RichText::new(&*connected).color(p.text)).selectable(false));
                ui.add_space(super::SPACE[1]);
                divider(ui);
                let label = tr.tr("settings-spotify-device");
                row(ui, &p, &label, |ui| device_combo(ui, ctx, &label));
                divider(ui);
                buttons_row(ui, |ui| {
                    if ui.button(tr.tr("settings-spotify-refresh")).clicked() {
                        ctx.out.push(PageAction::Spotify(SpotifyRequest::Refresh));
                    }
                    if ui.button(tr.tr("settings-spotify-disconnect")).clicked() {
                        open_dialog(ui, ctx.modal_open, Dialog::SpotifyDisconnect);
                    }
                });
            }
        }
        caption(ui, &p, &tr.tr("settings-spotify-premium"), TextStyle::Small);
        if !spotify.keyring {
            caption(
                ui,
                &p,
                &tr.tr("settings-spotify-no-keyring"),
                TextStyle::Small,
            );
        }
    });
}

/// "The active device", then Spotify's devices by name; restricted ones cannot be chosen.
fn device_combo(ui: &mut Ui, ctx: &mut PageCtx<'_>, name: &str) {
    let tr = ctx.tr;
    let chosen = ctx.settings.spotify_device.clone();
    let active = tr.tr("settings-spotify-active-device").into_owned();
    let selected = chosen
        .as_ref()
        .and_then(|id| {
            ctx.spotify
                .devices
                .iter()
                .find(|d| d.id.as_ref() == Some(id))
                .map(|d| d.name.clone())
        })
        .unwrap_or_else(|| active.clone());
    let mut picked: Option<Option<String>> = None;
    ComboBox::from_id_salt("aria_spotify_device")
        .width(COMBO_WIDTH)
        .selected_text(selected)
        .show_ui(ui, |ui| {
            if ui.selectable_label(chosen.is_none(), &active).clicked() {
                picked = Some(None);
            }
            for device in &ctx.spotify.devices {
                let Some(id) = &device.id else {
                    continue;
                };
                let enabled = !device.restricted;
                let is_chosen = chosen.as_ref() == Some(id);
                let response = ui.add_enabled(
                    enabled,
                    eframe::egui::Button::selectable(is_chosen, &device.name),
                );
                if response.clicked() {
                    picked = Some(Some(id.clone()));
                }
            }
        })
        .response
        .accessible_name(name);
    if let Some(id) = picked
        && id != chosen
    {
        ctx.out
            .push(PageAction::Spotify(SpotifyRequest::Device(id)));
    }
}

#[cfg(test)]
mod tests {
    use aria_core::SpDevice;
    use aria_core::spotify_service::{SpotifyAccount, SpotifyRequest};
    use aria_library::Collections;
    use eframe::egui::accesskit::Role;
    use egui_kittest::kittest::Queryable as _;

    use super::super::super::{
        LibraryView,
        test_support::{World, harness},
    };
    use super::super::show as settings_page;
    use super::*;

    fn page(account: SpotifyAccount) -> egui_kittest::Harness<'static, World> {
        let mut world = World::new(Collections::default(), LibraryView::default());
        if let SpotifyAccount::SignedOut { keyring } = &account {
            world.spotify.keyring = *keyring;
        }
        world.spotify.account = account;
        world.spotify.devices = vec![
            SpDevice {
                id: Some("phone".to_owned()),
                name: "My Phone".to_owned(),
                kind: "Smartphone".to_owned(),
                active: true,
                restricted: false,
                volume: Some(50),
                supports_volume: true,
            },
            SpDevice {
                id: Some("car".to_owned()),
                name: "Car".to_owned(),
                kind: "Automobile".to_owned(),
                active: false,
                restricted: true,
                volume: None,
                supports_volume: false,
            },
        ];
        harness(world, |ui, world| world.page(ui, settings_page))
    }

    fn click_on(harness: &mut egui_kittest::Harness<'static, World>, role: Role, label: &str) {
        harness.get_by_role_and_label(role, label).scroll_to_me();
        harness.run();
        harness.get_by_role_and_label(role, label).click();
        harness.run();
    }

    fn signed_in() -> SpotifyAccount {
        SpotifyAccount::SignedIn {
            name: "Ann".to_owned(),
        }
    }

    #[test]
    fn connect_sends_sign_in() {
        let mut harness = page(SpotifyAccount::SignedOut { keyring: true });
        let label = harness.state().english("settings-spotify-connect");
        click_on(&mut harness, Role::Button, &label);
        assert!(matches!(
            harness.state().out.as_slice(),
            [PageAction::Spotify(SpotifyRequest::SignIn { .. })]
        ));
        let allow = harness.state().english("settings-spotify-allow-list");
        harness.get_by_label(&allow);
    }

    #[test]
    fn signing_in_shows_cancel() {
        let mut harness = page(SpotifyAccount::SigningIn);
        let waiting = harness.state().english("settings-spotify-waiting");
        harness.get_by_label(&waiting);
        let cancel = harness.state().english("settings-spotify-cancel");
        click_on(&mut harness, Role::Button, &cancel);
        assert_eq!(
            harness.state().out,
            [PageAction::Spotify(SpotifyRequest::CancelSignIn)]
        );
    }

    #[test]
    fn signed_in_shows_the_name_and_devices() {
        let mut harness = page(signed_in());
        let mut args = FluentArgs::new();
        args.set("name", "Ann");
        let connected = harness
            .state()
            .english_with("settings-spotify-connected", &args);
        harness.get_by_label(&connected);
        let device = harness.state().english("settings-spotify-device");
        click_on(&mut harness, Role::ComboBox, &device);
        let list = harness.get_by_role(Role::ListBox);
        list.get_by_role_and_label(Role::Button, "My Phone");
        let active = harness.state().english("settings-spotify-active-device");
        harness
            .get_by_role(Role::ListBox)
            .get_by_role_and_label(Role::Button, &active);
    }

    #[test]
    fn choosing_a_device_sends_it() {
        let mut harness = page(signed_in());
        let device = harness.state().english("settings-spotify-device");
        click_on(&mut harness, Role::ComboBox, &device);
        harness
            .get_by_role(Role::ListBox)
            .get_by_role_and_label(Role::Button, "My Phone")
            .click();
        harness.run();
        assert_eq!(
            harness.state().out,
            [PageAction::Spotify(SpotifyRequest::Device(Some(
                "phone".to_owned()
            )))]
        );
    }

    #[test]
    fn a_restricted_device_cannot_be_chosen() {
        let mut harness = page(signed_in());
        let device = harness.state().english("settings-spotify-device");
        click_on(&mut harness, Role::ComboBox, &device);
        harness
            .get_by_role(Role::ListBox)
            .get_by_role_and_label(Role::Button, "Car")
            .click();
        harness.run();
        assert!(harness.state().out.is_empty(), "{:?}", harness.state().out);
    }

    #[test]
    fn disconnect_asks_first() {
        let mut harness = page(signed_in());
        let disconnect = harness.state().english("settings-spotify-disconnect");
        click_on(&mut harness, Role::Button, &disconnect);
        assert!(harness.state().out.is_empty(), "nothing before the answer");
        let body = harness
            .state()
            .english("settings-spotify-disconnect-confirm");
        harness.get_by_label(&body);
        let confirm = harness.state().english("settings-confirm");
        harness
            .get_by_role_and_label(Role::Button, &confirm)
            .click();
        harness.run();
        assert_eq!(
            harness.state().out,
            [PageAction::Spotify(SpotifyRequest::SignOut)]
        );
    }

    #[test]
    fn the_no_keyring_caption_shows() {
        let harness = page(SpotifyAccount::SignedOut { keyring: false });
        let caption = harness.state().english("settings-spotify-no-keyring");
        harness.get_by_label(&caption);
        let harness = page(SpotifyAccount::SignedOut { keyring: true });
        assert!(harness.query_by_label(&caption).is_none());
    }
}
