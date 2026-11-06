//! The pane's scenes: fixed snapshots and pairing states, so the images
//! and tests don't move with the clock or a real relay.

use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use super::pairing::{Failure, known};
use super::{Pair, Ui};
use crate::rune_ui::sidebar::Pane;
use crate::watch::Timestamp;
use crate::watch::pairing::{DeviceStatus, DeviceSummary};
use crate::watch::relay::RelayState;
use crate::watch::service::{ClaimError, WatchSnapshot};

/// What every scene counts "last seen" from: 2026-10-26, 12:00 UTC.
fn now() -> SystemTime {
    UNIX_EPOCH + Duration::from_secs(1_793_016_000)
}

fn device(id: &str, name: &str, status: DeviceStatus, seen_ago: u64) -> DeviceSummary {
    let seen = Timestamp::string(now() - Duration::from_secs(seen_ago));
    DeviceSummary {
        id: id.into(),
        name: name.into(),
        paired_at: seen.clone(),
        last_seen: seen,
        last_protocol: 4,
        last_app_version: "0.4.0".into(),
        status,
    }
}

fn watches() -> Vec<DeviceSummary> {
    vec![
        device("series-9", "Glen's Apple Watch", DeviceStatus::Ready, 180),
        device(
            "series-4",
            "Old Apple Watch",
            DeviceStatus::NeedsRepair,
            86_400 * 9,
        ),
    ]
}

pub(in crate::rune_ui) fn snapshot(
    relay: RelayState,
    devices: Vec<DeviceSummary>,
) -> WatchSnapshot {
    WatchSnapshot {
        relay,
        mac_name: "Glen's MacBook Pro".into(),
        replaced: false,
        devices,
    }
}

/// The pane on a Mac with the service connected, the way a scene names it.
pub(in crate::rune_ui) enum Scene {
    Connected,
    Empty,
    Offline,
    Pairing,
    Waiting,
    Paired,
    Error,
    Remove,
    Unavailable,
}

pub(in crate::rune_ui) fn scene(ui: &mut Ui, scene: Scene) {
    *ui = crate::rune_ui::mac_fixture();
    ui.pane = Pane::Watch;
    ui.watch.now = Some(now());
    let connected = RelayState::Connected;
    ui.platform.watch = Some(snapshot(connected, watches()));
    match scene {
        Scene::Connected => {}
        Scene::Empty => ui.platform.watch = Some(snapshot(connected, Vec::new())),
        Scene::Offline => ui.platform.watch = Some(snapshot(RelayState::Offline, watches())),
        Scene::Pairing => {
            ui.watch.code = rune::TextEdit::new("217400");
            ui.watch.pair = Pair::Pairing {
                before: known(ui.platform.watch.as_ref()),
            };
        }
        Scene::Waiting => {
            ui.watch.code = rune::TextEdit::new("217400");
            ui.watch.pair = Pair::Waiting {
                before: known(ui.platform.watch.as_ref()),
                since: Instant::now(),
            };
        }
        Scene::Paired => ui.watch.pair = Pair::Paired("Glen's Apple Watch".into()),
        Scene::Error => ui.watch.pair = Pair::Failed(Failure::Claim(ClaimError::CodeNotFound)),
        Scene::Remove => ui.watch.removing = Some("series-9".into()),
        Scene::Unavailable => ui.platform.watch = None,
    }
}

/// The first run on a Mac whose watch service reports, at `page`: with a
/// watch already paired (the Continue button, no Microphone row) or none.
pub(in crate::rune_ui) fn first_run(
    ui: &mut Ui,
    paired: bool,
    page: crate::rune_ui::onboarding_kit::live::Page,
) {
    use crate::rune_ui::onboarding;
    *ui = crate::rune_ui::mac_fixture();
    ui.watch.now = Some(now());
    let devices = if paired {
        vec![watches().remove(0)]
    } else {
        Vec::new()
    };
    ui.platform.watch = Some(snapshot(RelayState::Connected, devices));
    if paired {
        ui.watch.pair = Pair::Paired("Glen's Apple Watch".into());
    }
    onboarding::mac_fixture(ui, onboarding::MacStep::Setup);
    crate::rune_ui::onboarding_kit::live::fixture(ui, page);
}

/// A paired watch, for tests that build their own snapshot.
#[cfg(test)]
pub(in crate::rune_ui) fn scene_watch() -> DeviceSummary {
    watches().remove(0)
}

#[cfg(test)]
pub(in crate::rune_ui) fn scene_snapshot(devices: Vec<DeviceSummary>) -> WatchSnapshot {
    snapshot(RelayState::Connected, devices)
}
