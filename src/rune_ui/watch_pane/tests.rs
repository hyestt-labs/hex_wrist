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
