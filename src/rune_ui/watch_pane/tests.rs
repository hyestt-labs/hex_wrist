//! The pane through its scenes: what it says, what pressing its controls
//! asks of the host, and what the host's answers change.

use rune::text::TextSystem;
use rune::{Env, Runtime, TextEdit};

use super::*;
use crate::rune_ui::platform::PlatformHost;
use crate::rune_ui::{SCENES, Service, accessibility_tests as a11y, offscreen, on_key, view};

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
        "Glen's MacBook Pro",
        "Glen's Apple Watch",
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
        ("mac-watch-paired", "Paired with Glen's Apple Watch"),
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
    assert_eq!(asked(&rt), [PlatformRequest::WatchPair("217400".into())]);
    assert!(says(&rt, "Pairing…"));
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
        rt.state.watch.pair = Pair::Pairing { before: Vec::new() };
        rt.state
            .platform
            .watch_events
            .push(WatchEvent::Claimed(Err(error.clone())));
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
            id: "series-9".into(),
            name: "Glen's Apple Watch".into()
        }]
    );
    assert!(
        rt.find("watch-remove-now").is_none(),
        "the confirmation closes"
    );
}

#[test]
fn a_remove_that_could_not_write_says_why() {
    let mut rt = runtime("mac-watch");
    rt.state.platform.watch_events.push(WatchEvent::Revoked {
        id: "series-9".into(),
        name: "Glen's Apple Watch".into(),
        result: Err("could not write paired-devices.json: disk full".into()),
    });
    refresh(&mut rt);
    assert!(says(
        &rt,
        "Couldn't remove Glen's Apple Watch: could not write paired-devices.json: disk full"
    ));
    rt.state.platform.watch_events.push(WatchEvent::Revoked {
        id: "series-9".into(),
        name: "Glen's Apple Watch".into(),
        result: Ok(true),
    });
    refresh(&mut rt);
    assert!(
        rt.find("watch-remove-error").is_none(),
        "a success clears it"
    );
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
        assert!(says(&rt, "Paired with Glen's Apple Watch"));
        tap(&mut rt, "continue");
        assert_eq!(page(&rt), Some(Page::Demo));
    }

    #[test]
    fn six_digits_on_the_watch_page_send_one_pair_request() {
        let mut rt = runtime("mac-onboarding-watch");
        rt.state.watch.code = TextEdit::new("217400");
        refresh(&mut rt);
        refresh(&mut rt);
        assert_eq!(asked(&rt), [PlatformRequest::WatchPair("217400".into())]);
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

    #[test]
    fn a_mac_without_a_running_service_never_sees_the_watch_page() {
        let mut rt = runtime("mac-onboarding");
        rt.update(|ui, _| live::fixture_resume(ui, Page::Demo));
        refresh(&mut rt);
        assert_eq!(page(&rt), Some(Page::Demo), "straight to the demo");
        assert!(rt.find("pair-later").is_none());
    }
}
