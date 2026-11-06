//! The Apple Watch pane (macOS): the relay connection, this Mac's name on the
//! watch, a six-digit code field that pairs a watch, the paired watches with
//! a two-step Remove, and the capsule and paste choices. The service's state
//! arrives as pushed snapshots (`watch::view`) in `ui.platform.watch`; the
//! window never polls it. Pairing and Remove go out as platform requests and
//! come back as [`WatchEvent`]s. First-run setup shares the code field, its
//! status line, and the app link (`watch_pane::pairing`).

mod fixtures;
#[cfg(target_os = "macos")]
pub(in crate::rune_ui) mod host;
mod pairing;
#[cfg(test)]
mod tests;

use std::time::SystemTime;

use rune::{El, TextEdit, row, text};
use rune_kit::{Button, ButtonVariant, TextInput};

use super::Ui;
use super::controls::{
    Paint, PushSize, bare_page, folding, note, page, section, setting_row, status_dot, toggle_row,
};
use super::lists::small_button;
use super::platform::PlatformRequest;
use crate::desktop::i18n::{tr, tr_fill, tr_named};
use crate::watch::pairing::{DeviceStatus, DeviceSummary};
use crate::watch::relay::RelayState;
use crate::watch::service::{ClaimError, ClaimOk, WatchSnapshot};
pub(super) use fixtures::{Scene, first_run, scene};
#[cfg(test)]
pub(super) use fixtures::{scene_snapshot, scene_watch};
pub(super) use pairing::{Pair, Tone};

/// Where the watch app comes from.
pub(super) const APP_PAGE: &str = "https://wristper.dev/watch";

/// What the host reports back to the pane.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
#[derive(Clone, Debug, PartialEq)]
pub(super) enum WatchEvent {
    Claimed(Result<ClaimOk, ClaimError>),
    Revoked {
        id: String,
        name: String,
        result: Result<bool, String>,
    },
}

/// The pane's own state: the code being typed, how pairing is going, and the
/// two-step Remove. The service's state is `ui.platform.watch`.
#[derive(Default)]
pub(super) struct WatchView {
    pub(super) code: TextEdit,
    pub(super) pair: Pair,
    /// The watch whose Remove was pressed once.
    removing: Option<String>,
    /// A Remove that could not write the list: the watch's id, and what to
    /// say under its row.
    remove_error: Option<(String, String)>,
    /// What "last seen" counts from; scenes fix it so their images don't move.
    pub(super) now: Option<SystemTime>,
}

/// Adopts what the host reported and advances pairing. True when anything
/// shown changed.
pub(super) fn poll(ui: &mut Ui) -> bool {
    if !ui.platform.caps.watch {
        return false;
    }
    let mut changed = false;
    for event in std::mem::take(&mut ui.platform.watch_events) {
        changed |= ui.watch.apply(event, ui.platform.watch.as_ref());
    }
    changed |= ui
        .watch
        .settle(ui.platform.watch.as_ref(), std::time::Instant::now());
    if let Some(code) = ui.watch.typed(ui.platform.watch.as_ref()) {
        ui.ask(PlatformRequest::WatchPair(code));
        changed = true;
    }
    changed
}

fn removal_pressed(ui: &mut Ui, id: &str, name: &str) {
    if ui.watch.removing.as_deref() == Some(id) {
        ui.watch.removing = None;
        ui.watch.remove_error = None;
        ui.ask(PlatformRequest::WatchRevoke {
            id: id.to_owned(),
            name: name.to_owned(),
        });
    } else {
        ui.watch.removing = Some(id.to_owned());
    }
}

impl WatchView {
    fn apply(&mut self, event: WatchEvent, snapshot: Option<&WatchSnapshot>) -> bool {
        match event {
            WatchEvent::Claimed(result) => {
                self.code = TextEdit::default();
                self.pair = self.pair.claimed(result, snapshot);
            }
            WatchEvent::Revoked { id, name, result } => {
                self.remove_error = result.err().map(|error| {
                    let words = tr_named(
                        "Couldn't remove {name}: {error}",
                        &[("name", &name), ("error", &error)],
                    );
                    (id, words)
                });
            }
        }
        true
    }
}

/// The pane: the connection, pairing, paired watches, and the choices.
pub(super) fn pane(ui: &Ui, p: Paint) -> El {
    let Some(snapshot) = &ui.platform.watch else {
        return bare_page("watch-content").pt(24.0).child(
            text(tr("The Apple Watch service isn't running"))
                .key("watch-unavailable")
                .font(p.k.ty.ui)
                .color(p.muted())
                .wrap(),
        );
    };
    let content = page("watch-content")
        .child(section(
            p,
            "watch-mac",
            tr("This Mac"),
            mac_rows(p, snapshot),
        ))
        .child(section(
            p,
            "watch-pair",
            tr("Pair a watch"),
            pair_rows(ui, p),
        ))
        .child(section(
            p,
            "watch-devices",
            tr("Paired watches"),
            device_rows(ui, p, snapshot),
        ))
        .child(section(
            p,
            "watch-paste",
            tr("Status and paste"),
            paste_rows(ui, p),
        ));
    super::scroll_body(p.k, "watch-scroll", content)
}

fn mac_rows(p: Paint, snapshot: &WatchSnapshot) -> Vec<El> {
    let (label, tone) = relay_status(snapshot);
    let color = match tone {
        Tone::Good => p.success(),
        Tone::Waiting => p.warning(),
        Tone::Bad => p.danger(),
    };
    let status = row()
        .key("watch-relay-status")
        .gap(8.0)
        .center_y()
        .child(status_dot(color))
        .child(text(label).key("label").font(p.k.ty.ui).color(p.muted()));
    vec![
        setting_row(p, "watch-relay", tr("Connection"), None, status),
        setting_row(
            p,
            "watch-mac-name",
            tr("Name on your watch"),
            Some(tr("What the watch shows for this Mac")),
            text(snapshot.mac_name.clone())
                .key("mac-name")
                .font(p.k.ty.ui)
                .color(p.muted())
                .truncate(),
        ),
    ]
}

/// The relay's state in words, and how good it is.
pub(super) fn relay_status(snapshot: &WatchSnapshot) -> (&'static str, Tone) {
    match (snapshot.relay, snapshot.replaced) {
        (RelayState::Connected, _) => (tr("Connected"), Tone::Good),
        (_, true) => (tr("Another Mac took over this pairing"), Tone::Bad),
        (RelayState::Connecting, false) => (tr("Connecting…"), Tone::Waiting),
        (RelayState::Offline, false) => (tr("Offline: check the network"), Tone::Bad),
    }
}

/// The six-digit field, shared with first-run setup (`key` tells them apart).
pub(super) fn code_input(code: &TextEdit, busy: bool, key: &'static str, p: Paint) -> El {
    TextInput::new(code)
        .label(tr("Pairing code"))
        .placeholder("000000")
        .width(Some(120.0))
        .disabled(busy)
        .on_edit(|ui: &mut Ui| &mut ui.watch.code)
        .build(p.k)
        .key(key)
}

/// Whether a paired watch can be served (not needing repair); false until
/// the service has reported.
pub(super) fn paired(ui: &Ui) -> bool {
    crate::watch::view::paired(ui.platform.watch.as_ref()).unwrap_or(false)
}

/// Opens the watch app's page, as a live window does (scenes record nothing).
pub(super) fn open_app_page(ui: &mut Ui) {
    if ui.live()
        && let Err(error) = crate::desktop::opencode_catalog::open_url(APP_PAGE)
    {
        ui.error = Some(format!("{error:#}"));
    }
}

/// The status line under the code field and its color, if there is one.
pub(super) fn code_status(ui: &Ui, p: Paint) -> Option<(String, rune::Color)> {
    let (words, tone) = ui.watch.pair.status()?;
    let color = match tone {
        Tone::Good => p.success(),
        Tone::Waiting => p.muted(),
        Tone::Bad => p.danger(),
    };
    Some((words, color))
}

fn pair_rows(ui: &Ui, p: Paint) -> Vec<super::controls::CardRow> {
    let status = code_status(ui, p).map(|(words, color)| note("watch-code-note", &words, color));
    vec![
        setting_row(
            p,
            "watch-pair-code",
            tr("Pairing code"),
            Some(tr(
                "On your watch, open Wristper › Pair a Mac and type the code it shows",
            )),
            code_input(&ui.watch.code, ui.watch.pair.busy(), "watch-code", p),
        )
        .into(),
        folding("watch-code-fold", status),
        setting_row(
            p,
            "watch-app",
            tr("Get the watch app"),
            Some("wristper.dev/watch"),
            small_button(p, "watch-app-open", tr("Open"), |ui: &mut Ui, _| {
                open_app_page(ui)
            }),
        )
        .into(),
    ]
}

fn device_rows(ui: &Ui, p: Paint, snapshot: &WatchSnapshot) -> Vec<super::controls::CardRow> {
    let now = ui.watch.now.unwrap_or_else(SystemTime::now);
    let mut rows = Vec::new();
    for device in &snapshot.devices {
        let id = device.id.clone();
        let name = device.name.clone();
        let control = if ui.watch.removing.as_deref() == Some(&device.id) {
            confirm_remove(p, &id, &name)
        } else {
            let (id, name) = (id.clone(), name.clone());
            small_button(p, "watch-remove", tr("Remove…"), move |ui: &mut Ui, _| {
                removal_pressed(ui, &id, &name)
            })
        };
        rows.push(
            setting_row(
                p,
                &format!("watch-device-{}", device.id),
                &device.name,
                Some(&device_note(device, now)),
                control,
            )
            .into(),
        );
        // A Remove that changed nothing says why under the watch it kept.
        if let Some((_, words)) = ui
            .watch
            .remove_error
            .as_ref()
            .filter(|(failed, _)| *failed == device.id)
        {
            rows.push(note("watch-remove-error", words, p.danger()).into());
        }
    }
    if snapshot.devices.is_empty() {
        rows.push(note("watch-none", tr("No watches paired yet"), p.muted()).into());
    }
    rows
}

/// The second press of Remove: Cancel, and a Danger Remove.
fn confirm_remove(p: Paint, id: &str, name: &str) -> El {
    let (id, name) = (id.to_owned(), name.to_owned());
    row()
        .key("watch-remove-confirm")
        .gap(6.0)
        .center_y()
        .child(small_button(
            p,
            "watch-remove-cancel",
            tr("Cancel"),
            |ui: &mut Ui, _| ui.watch.removing = None,
        ))
        .child(
            Button::new(tr("Remove"))
                .variant(ButtonVariant::Danger)
                .on_press_tx(move |ui: &mut Ui, _| removal_pressed(ui, &id, &name))
                .build_small_push(p.k)
                .key("watch-remove-now"),
        )
}

/// What a paired watch's row says under its name.
pub(super) fn device_note(device: &DeviceSummary, now: SystemTime) -> String {
    match device.status {
        DeviceStatus::Ready => tr_fill("Last seen {}", &pairing::last_seen(&device.last_seen, now)),
        DeviceStatus::NeedsRepair => tr("Pair it again to use it").to_owned(),
        DeviceStatus::SecretsUnavailable => tr("Waiting for the keychain").to_owned(),
    }
}

fn paste_rows(ui: &Ui, p: Paint) -> Vec<El> {
    let watch = &ui.settings.watch;
    vec![
        toggle_row(
            p,
            "watch-status-capsule",
            tr("Status capsule"),
            tr("Shows what a watch dictation is doing, near the bottom of the screen"),
            watch.status_capsule,
            |ui: &mut Ui| {
                ui.update(|settings| {
                    settings.watch.status_capsule = !settings.watch.status_capsule
                });
            },
        ),
        toggle_row(
            p,
            "watch-capsule-pointer",
            tr("Follow the pointer"),
            tr("Shows the capsule on the screen the pointer is on"),
            watch.capsule_follows_pointer,
            |ui: &mut Ui| {
                ui.update(|settings| {
                    settings.watch.capsule_follows_pointer = !settings.watch.capsule_follows_pointer
                });
            },
        ),
        toggle_row(
            p,
            "watch-keep-on-clipboard",
            tr("Keep dictation on clipboard"),
            tr("A watch dictation stays on the clipboard after it pastes"),
            watch.keep_on_clipboard,
            |ui: &mut Ui| {
                ui.update(|settings| {
                    settings.watch.keep_on_clipboard = !settings.watch.keep_on_clipboard
                });
            },
        ),
    ]
}

/// The pane's words for a pairing code the relay or the watch refused.
pub(super) fn claim_error(error: &ClaimError) -> String {
    match error {
        ClaimError::InvalidCode => tr("Type the six digits your watch shows").to_owned(),
        ClaimError::RelayOffline => {
            tr("Hex can't reach the relay. Check the network and try again").to_owned()
        }
        ClaimError::Timeout => tr("The relay didn't answer. Try again").to_owned(),
        ClaimError::CodeNotFound => {
            tr("No watch is showing that code. It may have expired").to_owned()
        }
        ClaimError::TooManyAttempts => tr("Too many wrong codes. Wait 10 minutes").to_owned(),
        ClaimError::Relay(error) => tr_fill("The relay refused the code ({})", error),
        ClaimError::Unavailable => tr("The Apple Watch service isn't running").to_owned(),
    }
}
