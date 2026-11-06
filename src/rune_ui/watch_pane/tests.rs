//! The pane through its scenes: what it says, what pressing its controls
//! asks of the host, and what the host's answers change.

use rune::text::TextSystem;
use rune::{Env, Runtime, TextEdit};

use super::*;
use crate::rune_ui::platform::PlatformHost;
use crate::rune_ui::{SCENES, Service, accessibility_tests as a11y, offscreen, on_key, view};
use crate::watch::pairing::requests::Admission;

const SIZE: (f32, f32) = (980.0, 1400.0);

fn runtime(scene: &str) -> Runtime<Ui> {
    let mut state = Ui::with(
        crate::app_settings::AppSettings::default(),
        Service::Fixture(Vec::new()),
    );
    let (_, apply) = SCENES
        .iter()
        .find(|(name, _)| *name == scene)
        .unwrap_or_else(|| panic!("no scene {scene}"));
    apply(&mut state);
    let env = Env {
        system_dark: true,
        scale: 2.0,
    };
    let text = TextSystem::new(rune_theme_opencode::FONTS, rune_theme_opencode::FAMILIES);
    let mut rt = offscreen::runtime(state, view, SIZE, env, text);
    rt.poll = Some(crate::rune_ui::poll);
    rt.on_key = Some(on_key);
    rt.settle_all();
    rt.frame(0.0);
    rt
}

fn settle(rt: &mut Runtime<Ui>) {
    for _ in 0..60 {
        rt.frame(1.0 / 60.0);
    }
}

/// Redraws after a test changed state directly, as the host would.
fn refresh(rt: &mut Runtime<Ui>) {
    rt.invalidate();
    settle(rt);
}

fn tap(rt: &mut Runtime<Ui>, key: &str) {
    let (x, y) = rt.find(key).unwrap_or_else(|| panic!("no element {key}"));
    rt.pointer_down(x, y);
    rt.pointer_up(x, y);
    settle(rt);
}

fn asked(rt: &Runtime<Ui>) -> &[PlatformRequest] {
    match &rt.state.host {
        PlatformHost::Fixture(sent) => sent,
        _ => panic!("scenes record platform requests"),
    }
}

/// The codes sent to be claimed.
fn asked_codes(rt: &Runtime<Ui>) -> Vec<&str> {
    asked(rt)
        .iter()
        .filter_map(|request| match request {
            PlatformRequest::WatchPair { code, .. } => Some(code.as_str()),
            _ => None,
        })
        .collect()
}

/// The number of the claim most recently sent.
fn last_claim(rt: &Runtime<Ui>) -> u64 {
    asked(rt)
        .iter()
        .rev()
        .find_map(|request| match request {
            PlatformRequest::WatchPair { generation, .. } => Some(*generation),
            _ => None,
        })
        .expect("a claim was sent")
}

fn shown(rt: &Runtime<Ui>) -> Vec<String> {
    rt.elements()
        .into_iter()
        .filter(|e| !e.path.contains("/faces/size"))
        .filter_map(|e| e.text)
        .collect()
}

fn says(rt: &Runtime<Ui>, words: &str) -> bool {
    shown(rt).iter().any(|text| text == words)
}

#[test]
fn the_pane_lists_the_connection_the_watches_and_the_choices() {
    let rt = runtime("mac-watch");
    for words in [
        "Apple Watch",
        "Connected",
        "Test Mac",
        "Test watch",
        "Last seen 3m ago",
        "Pair it again to use it",
        "Status capsule",
    ] {
        assert!(says(&rt, words), "missing {words:?} in {:?}", shown(&rt));
    }
    assert!(rt.find("watch-code").is_some());
}

#[test]
fn each_scene_says_its_state() {
    for (scene, words) in [
        ("mac-watch-empty", "No watches paired yet"),
        ("mac-watch-offline", "Offline: check the network"),
        ("mac-watch-pairing", "Pairing…"),
        ("mac-watch-waiting", "Finish on your watch…"),
        ("mac-watch-paired", "Paired with Test watch"),
        (
            "mac-watch-unavailable",
            "The Apple Watch service isn't running",
        ),
    ] {
        let rt = runtime(scene);
        assert!(
            says(&rt, words),
            "{scene}: missing {words:?} in {:?}",
            shown(&rt)
        );
    }
    let rt = runtime("mac-watch-unavailable");
    assert!(
        rt.find("watch-code").is_none(),
        "no pairing without a service"
    );
}

#[test]
fn six_digits_send_one_pair_request() {
    let mut rt = runtime("mac-watch");
    rt.state.watch.code = TextEdit::new("21740");
    refresh(&mut rt);
    assert!(asked(&rt).is_empty(), "five digits send nothing");
    rt.state.watch.code = TextEdit::new("217400");
    refresh(&mut rt);
    refresh(&mut rt);
    assert_eq!(asked_codes(&rt), ["217400"]);
    assert!(says(&rt, "Pairing…"));
}

fn claimed(generation: u64, device_id: &str) -> WatchEvent {
    WatchEvent::Claimed {
        generation,
        result: Ok(ClaimOk {
            device_id: device_id.into(),
            device_name: "Test watch".into(),
        }),
    }
}

/// Types the six digits and lets the window send the claim.
fn send_claim(rt: &mut Runtime<Ui>) -> u64 {
    rt.state.watch.code = TextEdit::new("217400");
    refresh(rt);
    refresh(rt);
    last_claim(rt)
}

#[test]
fn the_reply_to_the_claim_sent_moves_the_pane_to_waiting() {
    let mut rt = runtime("mac-watch");
    let generation = send_claim(&mut rt);
    rt.state
        .platform
        .watch_events
        .push(claimed(generation, "relay-mine"));
    refresh(&mut rt);
    assert!(says(&rt, "Finish on your watch…"), "{:?}", shown(&rt));
}

#[test]
fn a_late_reply_to_an_older_claim_changes_nothing_on_screen() {
    let mut rt = runtime("mac-watch");
    let older = send_claim(&mut rt);
    rt.state.platform.watch_events.push(WatchEvent::Claimed {
        generation: older,
        result: Err(ClaimError::Timeout),
    });
    refresh(&mut rt);
    let newer = send_claim(&mut rt);
    assert_ne!(older, newer);
    rt.state
        .platform
        .watch_events
        .push(claimed(older, "relay-old"));
    refresh(&mut rt);
    assert!(says(&rt, "Pairing…"), "still waiting for the newer claim");
    assert!(!says(&rt, "Finish on your watch…"));
}

#[test]
fn another_watch_pairing_while_this_claim_waits_does_not_settle_it() {
    let mut rt = runtime("mac-watch");
    let generation = send_claim(&mut rt);
    rt.state
        .platform
        .watch_events
        .push(claimed(generation, "relay-mine"));
    refresh(&mut rt);
    // A different watch finishes pairing in the same minutes.
    let mut other = crate::rune_ui::watch_pane::scene_watch();
    other.id = "device-0009".into();
    other.name = "Test watch 3".into();
    let mut snapshot = rt.state.platform.watch.clone().unwrap();
    snapshot.devices.push(other);
    snapshot.admissions.push(Admission {
        serial: 1,
        claimed_device_id: "relay-theirs".into(),
        device_id: "device-0009".into(),
    });
    rt.state.platform.watch = Some(snapshot);
    refresh(&mut rt);
    assert!(says(&rt, "Finish on your watch…"), "{:?}", shown(&rt));
    assert!(!says(&rt, "Paired with Test watch 3"));
    // The watch that claimed finishes, and the pane names it.
    let mut mine = crate::rune_ui::watch_pane::scene_watch();
    mine.id = "device-0010".into();
    mine.name = "Test watch 4".into();
    let mut snapshot = rt.state.platform.watch.clone().unwrap();
    snapshot.devices.push(mine);
    snapshot.admissions.push(Admission {
        serial: 2,
        claimed_device_id: "relay-mine".into(),
        device_id: "device-0010".into(),
    });
    rt.state.platform.watch = Some(snapshot);
    refresh(&mut rt);
    assert!(says(&rt, "Paired with Test watch 4"), "{:?}", shown(&rt));
}

#[test]
fn each_claim_error_shows_its_message() {
    let errors = [
        ClaimError::InvalidCode,
        ClaimError::RelayOffline,
        ClaimError::Timeout,
        ClaimError::CodeNotFound,
        ClaimError::TooManyAttempts,
        ClaimError::Relay("boom".into()),
        ClaimError::Unavailable,
    ];
    let mut messages = Vec::new();
    for error in errors {
        let mut rt = runtime("mac-watch");
        rt.state.watch.pair = Pair::Pairing {
            generation: 7,
            seen: 0,
        };
        rt.state.platform.watch_events.push(WatchEvent::Claimed {
            generation: 7,
            result: Err(error.clone()),
        });
        refresh(&mut rt);
        let message = claim_error(&error);
        assert!(
            says(&rt, &message),
            "{error:?}: {message:?} in {:?}",
            shown(&rt)
        );
        messages.push(message);
    }
    messages.sort();
    messages.dedup();
    assert_eq!(
        messages.len(),
        errors_len(),
        "every error has its own words"
    );
}

fn errors_len() -> usize {
    7
}

#[test]
fn remove_needs_two_presses() {
    let mut rt = runtime("mac-watch");
    tap(&mut rt, "watch-remove");
    assert!(asked(&rt).is_empty(), "one press only asks to confirm");
    assert!(rt.find("watch-remove-now").is_some());
    tap(&mut rt, "watch-remove-cancel");
    assert!(rt.find("watch-remove-now").is_none());
    assert!(asked(&rt).is_empty());
    tap(&mut rt, "watch-remove");
    tap(&mut rt, "watch-remove-now");
    assert_eq!(
        asked(&rt),
        [PlatformRequest::WatchRevoke {
            id: "device-0001".into(),
            name: "Test watch".into()
        }]
    );
    assert!(
        rt.find("watch-remove-now").is_none(),
        "the confirmation closes"
    );
}

#[test]
fn a_remove_that_could_not_write_says_why_under_the_watch_it_kept() {
    let mut rt = runtime("mac-watch");
    rt.state.platform.watch_events.push(WatchEvent::Revoked {
        id: "device-0001".into(),
        name: "Test watch".into(),
        result: Err("could not write paired-devices.json: disk full".into()),
    });
    refresh(&mut rt);
    assert!(says(
        &rt,
        "Couldn't remove Test watch: could not write paired-devices.json: disk full"
    ));
    let y = |rt: &Runtime<Ui>, key: &str| {
        rt.presented(key)
            .map(|(rect, _)| rect[1])
            .unwrap_or_else(|| panic!("no {key}"))
    };
    assert!(y(&rt, "watch-device-device-0001") < y(&rt, "watch-remove-error"));
    assert!(
        y(&rt, "watch-remove-error") < y(&rt, "watch-device-device-0002"),
        "the error sits under its own watch, not at the end of the card"
    );
    // Remove stays there to try again, and a success clears the error.
    assert!(rt.find("watch-remove").is_some());
    rt.state.platform.watch_events.push(WatchEvent::Revoked {
        id: "device-0001".into(),
        name: "Test watch".into(),
        result: Ok(true),
    });
    refresh(&mut rt);
    assert!(
        rt.find("watch-remove-error").is_none(),
        "a success clears it"
    );
}

fn revoked(id: &str, name: &str, result: Result<bool, String>) -> WatchEvent {
    WatchEvent::Revoked {
        id: id.into(),
        name: name.into(),
        result,
    }
}

/// Both Removes were sent before either answered; the answers come in
/// `order` (0 = the one that fails, 1 = the one that succeeds).
fn two_removes_in_flight(order: [usize; 2]) -> Runtime<Ui> {
    let mut rt = runtime("mac-watch");
    let replies = [
        revoked("device-0001", "Test watch", Err("disk full".into())),
        revoked("device-0002", "Test watch 2", Ok(true)),
    ];
    for index in order {
        rt.state.platform.watch_events.push(replies[index].clone());
        refresh(&mut rt);
    }
    // The service pushes the list without the watch it removed.
    let kept = vec![crate::rune_ui::watch_pane::scene_watch()];
    rt.state.platform.watch = Some(scene_snapshot(kept));
    refresh(&mut rt);
    rt
}

#[test]
fn a_failed_remove_keeps_its_error_whichever_remove_answers_last() {
    for order in [[0, 1], [1, 0]] {
        let rt = two_removes_in_flight(order);
        assert!(
            says(&rt, "Couldn't remove Test watch: disk full"),
            "{order:?}: the failed watch keeps its error: {:?}",
            shown(&rt)
        );
        assert!(
            rt.find("watch-device-device-0002").is_none(),
            "{order:?}: the removed watch's row is gone"
        );
        assert!(rt.find("watch-device-device-0001").is_some(), "{order:?}");
    }
}

#[test]
fn each_failed_remove_says_its_own_reason_under_its_own_watch() {
    for order in [[0, 1], [1, 0]] {
        let mut rt = runtime("mac-watch");
        let replies = [
            revoked("device-0001", "Test watch", Err("disk full".into())),
            revoked(
                "device-0002",
                "Test watch 2",
                Err("read-only volume".into()),
            ),
        ];
        for index in order {
            rt.state.platform.watch_events.push(replies[index].clone());
            refresh(&mut rt);
        }
        for words in [
            "Couldn't remove Test watch: disk full",
            "Couldn't remove Test watch 2: read-only volume",
        ] {
            assert!(says(&rt, words), "{order:?}: {words:?} in {:?}", shown(&rt));
        }
    }
}

#[test]
fn the_revoke_error_scene_keeps_the_watch_and_says_why() {
    let rt = runtime("mac-watch-revoke-error");
    assert!(rt.find("watch-device-device-0001").is_some());
    assert!(rt.find("watch-remove-error").is_some());
}

#[test]
fn the_toggles_save_their_own_setting() {
    let mut rt = runtime("mac-watch");
    tap(&mut rt, "watch-status-capsule");
    assert!(!rt.state.settings.watch.status_capsule);
    tap(&mut rt, "watch-capsule-pointer");
    assert!(rt.state.settings.watch.capsule_follows_pointer);
    tap(&mut rt, "watch-keep-on-clipboard");
    assert!(rt.state.settings.watch.keep_on_clipboard);
    assert!(
        !rt.state.settings.labs.keep_on_clipboard,
        "Labs' own switch is not it"
    );
}

/// The first text input at or under `node`.
fn text_input(node: &serde_json::Value) -> Option<serde_json::Value> {
    if node["role"] == "text-input" {
        return Some(node.clone());
    }
    node["children"].as_array()?.iter().find_map(text_input)
}

#[test]
fn every_watch_control_has_a_name() {
    for scene in ["mac-watch", "mac-watch-remove", "mac-watch-pairing"] {
        let rt = runtime(scene);
        let input = text_input(&a11y::node(&rt, "watch-code")).expect("the field has an input");
        assert_eq!(
            input["name"], "Pairing code",
            "{scene}: the code field's name"
        );
        for key in [
            "watch-app-open",
            "watch-status-capsule",
            "watch-capsule-pointer",
            "watch-keep-on-clipboard",
        ] {
            let node = a11y::node(&rt, key);
            let name = node["name"].as_str().unwrap_or_default().trim().to_owned();
            assert!(!name.is_empty(), "{scene}: {key} has no name");
        }
    }
    let rt = runtime("mac-watch-remove");
    for key in ["watch-remove-cancel", "watch-remove-now"] {
        let node = a11y::node(&rt, key);
        assert!(
            !node["name"].as_str().unwrap_or_default().trim().is_empty(),
            "{key}"
        );
    }
}

// The scenes take their OS's capabilities from the build, so only a Linux
// build draws a Linux window.
#[cfg(target_os = "linux")]
#[test]
fn linux_and_windows_sidebars_have_no_watch() {
    for scene in ["portal", "windows-settings"] {
        let rt = runtime(scene);
        assert!(rt.find("nav-watch").is_none(), "{scene} lists Apple Watch");
    }
}

#[test]
fn a_mac_lists_apple_watch_in_its_sidebar() {
    let rt = runtime("mac-settings");
    assert!(rt.find("nav-watch").is_some());
}

#[test]
fn the_sidebar_puts_apple_watch_right_after_general() {
    let rt = runtime("mac-settings");
    let y = |key: &str| rt.presented(key).map(|(rect, _)| rect[1]).unwrap();
    assert!(y("nav-general") < y("nav-watch"));
    assert!(y("nav-watch") < y("nav-output"));
}

mod first_run {
    use super::*;
    use crate::desktop::onboarding::{PermissionState, Step as TourStep};
    use crate::rune_ui::onboarding_kit::live::{self, Page};

    fn page(rt: &Runtime<Ui>) -> Option<Page> {
        live::page(&rt.state)
    }

    #[test]
    fn the_watch_page_follows_the_model_step_and_pairs() {
        let rt = runtime("mac-onboarding-watch");
        assert_eq!(page(&rt), Some(Page::Watch));
        for words in ["Set up your Apple Watch", "Get the watch app", "Pair later"] {
            assert!(says(&rt, words), "missing {words:?} in {:?}", shown(&rt));
        }
        assert!(rt.find("watch-onboard-code").is_some());
        assert!(rt.find("continue").is_none(), "nothing paired yet");
        assert!(rt.find("skip").is_none(), "this page has no Skip");
    }

    #[test]
    fn pair_later_goes_to_the_demo_and_records_nothing() {
        let mut rt = runtime("mac-onboarding-watch");
        tap(&mut rt, "pair-later");
        assert_eq!(page(&rt), Some(Page::Demo), "the demo follows");
        assert!(rt.state.first_run.is_some(), "onboarding stays open");
        let tour = rt.state.tour.as_ref().expect("the tour is still running");
        assert_eq!(tour.flow.step(), TourStep::Setup, "setup is not done");
        // No platform request, no completion: a fixture never writes, and a
        // skipped watch asks the host for nothing.
        assert!(asked(&rt).is_empty(), "{:?}", asked(&rt));
    }

    #[test]
    fn a_paired_watch_shows_continue_not_pair_later() {
        let mut rt = runtime("mac-onboarding-watch-paired");
        assert!(rt.find("pair-later").is_none());
        assert!(says(&rt, "Paired with Test watch"));
        tap(&mut rt, "continue");
        assert_eq!(page(&rt), Some(Page::Demo));
    }

    #[test]
    fn six_digits_on_the_watch_page_send_one_pair_request() {
        let mut rt = runtime("mac-onboarding-watch");
        rt.state.watch.code = TextEdit::new("217400");
        refresh(&mut rt);
        refresh(&mut rt);
        assert_eq!(asked_codes(&rt), ["217400"]);
    }

    #[test]
    fn watch_only_permissions_ask_only_for_accessibility() {
        let rt = runtime("mac-onboarding-permissions-watch");
        assert_eq!(page(&rt), Some(Page::Permissions));
        assert!(rt.find("mic-row").is_none(), "no Microphone row");
        assert!(rt.find("ax-row").is_some(), "Accessibility is still asked");
        assert!(says(&rt, "Let Hex type for you"));
        assert!(says(
            &rt,
            "Pastes what you dictate on your watch into the app you're using"
        ));
        // Without a watch the page asks for both, as it always did.
        let rt = runtime("mac-onboarding-permissions");
        assert!(rt.find("mic-row").is_some());
        assert!(says(&rt, "Let Hex hear you and type for you"));
    }

    #[test]
    fn watch_only_finishes_without_the_practice() {
        let mut rt = runtime("mac-onboarding-permissions-watch");
        // Accessibility granted and the model installed; the Microphone
        // never is.
        rt.update(|ui, tx| {
            ui.platform.permissions.accessibility = PermissionState::Ready;
            ui.picker.installed = Some(vec![ui.settings.transcription.model]);
            if let Some(view) = &mut ui.view {
                view.transcription.preparing = None;
            }
            live::poll(ui, tx);
        });
        assert_eq!(
            rt.state.platform.permissions.microphone,
            PermissionState::NeedsRequest
        );
        for _ in 0..40 {
            rt.update(|ui, tx| {
                if let Some(s) = ui.first_run.as_mut() {
                    s.now += 1.0;
                    s.move_on();
                }
                live::poll(ui, tx);
            });
            refresh(&mut rt);
            if page(&rt) == Some(Page::Ready) {
                break;
            }
            assert_ne!(page(&rt), Some(Page::Try), "the practice was not skipped");
        }
        assert_eq!(page(&rt), Some(Page::Ready));
        assert!(
            rt.find("try-again").is_none(),
            "a Mac dictation can't be tried yet"
        );
        assert!(rt.find("start-using-hex").is_some());
    }

    #[test]
    fn a_setup_resumed_before_pairing_asks_about_the_watch_again() {
        // The tour quit at setup: unpaired shows the watch page, paired the permissions.
        for (paired, expected) in [(false, Page::Watch), (true, Page::Permissions)] {
            let mut rt = runtime("mac-onboarding-watch");
            rt.update(|ui, _| {
                let devices = if paired {
                    vec![crate::rune_ui::watch_pane::scene_watch()]
                } else {
                    Vec::new()
                };
                ui.platform.watch = Some(crate::rune_ui::watch_pane::scene_snapshot(devices));
                ui.first_run = None;
                live::open(ui);
            });
            refresh(&mut rt);
            assert_eq!(page(&rt), Some(expected), "paired: {paired}");
        }
    }

    /// A setup resumed (the tour quit at setup) before the service's first
    /// snapshot arrived: the permissions show while it waits.
    fn resumed_without_a_snapshot() -> Runtime<Ui> {
        let mut rt = runtime("mac-onboarding-watch");
        rt.update(|ui, _| {
            ui.platform.watch = None;
            ui.first_run = None;
            live::open(ui);
        });
        refresh(&mut rt);
        assert_eq!(page(&rt), Some(Page::Permissions));
        rt
    }

    /// The first snapshot, with no watch paired, `after` seconds later on
    /// the story's clock; nothing polled in between.
    fn first_snapshot_after(rt: &mut Runtime<Ui>, after: f64) {
        rt.update(|ui, tx| {
            if let Some(s) = ui.first_run.as_mut() {
                s.now += after;
            }
            ui.platform.watch = Some(crate::rune_ui::watch_pane::scene_snapshot(Vec::new()));
            live::poll(ui, tx);
        });
        refresh(rt);
    }

    #[test]
    fn a_resumed_setup_moves_to_the_watch_page_when_the_service_reports_in_time() {
        let mut rt = resumed_without_a_snapshot();
        first_snapshot_after(&mut rt, 1.0);
        assert_eq!(page(&rt), Some(Page::Watch));
    }

    #[test]
    fn a_resumed_setup_stops_waiting_after_a_moment_and_carries_on() {
        let mut rt = resumed_without_a_snapshot();
        first_snapshot_after(&mut rt, 1.6);
        assert_eq!(
            page(&rt),
            Some(Page::Permissions),
            "a snapshot after the wait never pulls the page away"
        );
        // Not stuck: the permissions page goes on once they're granted.
        rt.update(|ui, tx| {
            ui.platform.permissions.microphone = PermissionState::Ready;
            ui.platform.permissions.accessibility = PermissionState::Ready;
            live::poll(ui, tx);
        });
        for _ in 0..40 {
            if page(&rt) != Some(Page::Permissions) {
                break;
            }
            rt.update(|ui, tx| {
                if let Some(s) = ui.first_run.as_mut() {
                    s.now += 1.0;
                    s.move_on();
                }
                live::poll(ui, tx);
            });
            refresh(&mut rt);
        }
        assert_ne!(page(&rt), Some(Page::Permissions));
        assert_ne!(page(&rt), Some(Page::Watch));
    }

    /// A watch-only setup (a paired watch, Accessibility, no Microphone)
    /// whose model download failed.
    fn watch_only_with_a_failed_download() -> Runtime<Ui> {
        let mut rt = runtime("mac-onboarding");
        rt.update(|ui, tx| {
            let paired = vec![crate::rune_ui::watch_pane::scene_watch()];
            ui.platform.watch = Some(crate::rune_ui::watch_pane::scene_snapshot(paired));
            ui.platform.permissions.accessibility = PermissionState::Ready;
            if let Some(view) = &mut ui.view {
                view.transcription.preparing = None;
                view.transcription.error = Some("The connection was reset.".into());
            }
            live::poll(ui, tx);
        });
        refresh(&mut rt);
        rt
    }

    #[test]
    fn a_watch_only_setup_can_continue_without_a_model() {
        let mut rt = watch_only_with_a_failed_download();
        assert_eq!(
            rt.state.platform.permissions.microphone,
            PermissionState::NeedsRequest,
            "the Microphone was never granted"
        );
        assert!(
            rt.find("continue-without-model").is_some(),
            "the way out is offered: {:?}",
            shown(&rt)
        );
        tap(&mut rt, "continue-without-model");
        assert!(rt.state.first_run.is_none(), "setup closed");
        assert!(rt.state.tour.is_none(), "and its tour");
        // Settings then says no model is installed.
        rt.state.pane = crate::rune_ui::sidebar::Pane::General;
        refresh(&mut rt);
        assert!(
            says(&rt, "No speech model installed: The connection was reset."),
            "{:?}",
            shown(&rt)
        );
        assert!(rt.find("transcription-download").is_some());
    }

    #[test]
    fn a_watch_only_setup_is_not_offered_the_way_out_while_the_model_downloads() {
        let mut rt = watch_only_with_a_failed_download();
        rt.update(|ui, tx| {
            if let Some(view) = &mut ui.view {
                view.transcription.error = None;
            }
            crate::rune_ui::onboarding::set_downloaded(ui, 31);
            live::poll(ui, tx);
        });
        refresh(&mut rt);
        assert!(
            rt.find("continue-without-model").is_none(),
            "a model still on its way is not given up on"
        );
        tap_if_any(&mut rt);
        assert!(rt.state.first_run.is_some(), "setup stays open");
    }

    /// Presses Continue without a model the way a stale pointer might,
    /// when it isn't there nothing happens.
    fn tap_if_any(rt: &mut Runtime<Ui>) {
        if rt.find("continue-without-model").is_some() {
            tap(rt, "continue-without-model");
        }
    }

    #[test]
    fn a_mac_without_a_running_service_never_sees_the_watch_page() {
        let mut rt = runtime("mac-onboarding");
        rt.update(|ui, _| live::fixture_resume(ui, Page::Demo));
        refresh(&mut rt);
        assert_eq!(page(&rt), Some(Page::Demo), "straight to the demo");
        assert!(rt.find("pair-later").is_none());
    }
}
