//! `/enter` and `/undo` through the service: the windows and states are
//! `recording`'s; these check that the service hands the right field to
//! the `watch-ax` thread and records what came back.

use super::*;
use crate::watch::desktop::fake::{editable, text_area};
use crate::watch::recording::{ENTER_WAIT_SECONDS, UNDO_WINDOW_SECONDS};
use crate::watch::service::keys::KEY_WAIT;

fn terminal(pid: i32) -> crate::watch::field::FieldSnapshot {
    crate::watch::field::FieldSnapshot {
        bundle_id: Some("com.apple.Terminal".into()),
        ..text_area(pid)
    }
}

/// `/focus` in a field, then a recording delivered into it.
fn pasted_into(
    h: &mut Harness,
    watch: &Watch,
    field: crate::watch::field::FieldSnapshot,
    outcome: DeliveryOutcome,
) {
    h.screen.trusted.store(true, Ordering::SeqCst);
    h.screen.set_focus(Some(editable(field)));
    let headers = h.device_headers(watch);
    h.request(
        "POST",
        "/focus",
        &[],
        &headers,
        format!(r#"{{"id":"{ID}"}}"#).as_bytes(),
    )
    .unwrap();
    let head = audio_head(watch, ID, "requested", h.now.epoch());
    assert!(h.send_audio(watch, &head, M4A).is_none());
    let index = h.jobs.lock().unwrap().len() - 1;
    h.deliver(index, report(outcome, "hello there"));
}

fn press(h: &mut Harness, watch: &Watch, path: &str) -> Answer {
    h.query(watch, path, &[("id", ID)]).expect("answered")
}

fn read_back_paste(h: &mut Harness, watch: &Watch) {
    pasted_into(
        h,
        watch,
        text_area(7),
        DeliveryOutcome::Sent(Verification::None),
    );
    let index = h.jobs.lock().unwrap().len() - 1;
    h.confirm(index, true);
}

#[test]
fn enter_presses_return_once_and_a_second_one_is_a_duplicate() {
    let mut h = harness("enter");
    let watch = h.pair("watch-1");
    read_back_paste(&mut h, &watch);
    let entered = press(&mut h, &watch, "/enter");
    assert_eq!(entered.body, json!({"ok": true, "state": "entered"}));
    assert_eq!(h.screen.keys(), ["return"]);
    let again = press(&mut h, &watch, "/enter");
    assert_eq!(
        again.body,
        json!({"ok": true, "state": "entered", "duplicate": true})
    );
    assert_eq!(h.screen.keys(), ["return"], "Return went in only once");
    let status = h.status_of(&watch, ID);
    assert_eq!(status["entered"], true);
}

/// Claude Code and Codex run in a terminal, which is never read back: its
/// paste is `pasted` at once and Enter must work on it.
#[test]
fn enter_works_on_a_terminal_paste() {
    let mut h = harness("enter-terminal");
    let watch = h.pair("watch-1");
    pasted_into(
        &mut h,
        &watch,
        terminal(7),
        DeliveryOutcome::Sent(Verification::Terminal),
    );
    let entered = press(&mut h, &watch, "/enter");
    assert_eq!(entered.body["state"], "entered");
    assert_eq!(h.screen.keys(), ["return"]);
}

#[test]
fn enter_works_on_a_paste_the_read_back_could_not_confirm() {
    let mut h = harness("enter-unverified");
    let watch = h.pair("watch-1");
    pasted_into(
        &mut h,
        &watch,
        text_area(7),
        DeliveryOutcome::Sent(Verification::None),
    );
    assert_eq!(press(&mut h, &watch, "/enter").body["state"], "entered");
}

#[test]
fn enter_is_refused_after_30_seconds() {
    let mut h = harness("enter-expired");
    let watch = h.pair("watch-1");
    read_back_paste(&mut h, &watch);
    h.advance(Duration::from_secs(31));
    let refused = press(&mut h, &watch, "/enter");
    assert_eq!(refused.body["reason"], "expired");
    assert!(h.screen.keys().is_empty());
}

#[test]
fn enter_after_a_withheld_paste_is_not_pasted() {
    let mut h = harness("enter-withheld");
    let watch = h.pair("watch-1");
    pasted_into(
        &mut h,
        &watch,
        text_area(7),
        DeliveryOutcome::Withheld(crate::watch::field::Withheld::TargetChanged),
    );
    assert_eq!(press(&mut h, &watch, "/enter").body["reason"], "not-pasted");
    assert!(h.screen.keys().is_empty());
}

/// The user moved to another window after dictating: Return goes nowhere.
#[test]
fn enter_in_another_application_is_refused_and_presses_nothing() {
    let mut h = harness("enter-moved");
    let watch = h.pair("watch-1");
    read_back_paste(&mut h, &watch);
    h.screen.set_focus(Some(editable(text_area(8))));
    let refused = press(&mut h, &watch, "/enter");
    assert_eq!(refused.body["reason"], "target-changed");
    assert!(h.screen.keys().is_empty());
    let status = h.status_of(&watch, ID);
    assert_eq!(status["entered"], false, "a refused Enter marks nothing");
}

#[test]
fn enter_in_a_password_prompt_reads_sensitive() {
    let mut h = harness("enter-secure");
    let watch = h.pair("watch-1");
    read_back_paste(&mut h, &watch);
    h.screen.secure.store(true, Ordering::SeqCst);
    assert_eq!(press(&mut h, &watch, "/enter").body["reason"], "sensitive");
}

#[test]
fn a_failed_key_event_is_a_500_failed() {
    let mut h = harness("enter-failed");
    let watch = h.pair("watch-1");
    read_back_paste(&mut h, &watch);
    h.screen.fail_keys.store(true, Ordering::SeqCst);
    let failed = press(&mut h, &watch, "/enter");
    assert_eq!(
        (failed.status, failed.body["reason"].as_str()),
        (500, Some("failed"))
    );
}

/// The watch sends Enter with the audio; it waits for the paste to land.
#[test]
fn enter_waits_for_a_recording_still_being_processed() {
    let mut h = harness("enter-waits");
    let watch = h.pair("watch-1");
    h.screen.trusted.store(true, Ordering::SeqCst);
    h.screen.set_focus(Some(editable(text_area(7))));
    let headers = h.device_headers(&watch);
    h.request(
        "POST",
        "/focus",
        &[],
        &headers,
        format!(r#"{{"id":"{ID}"}}"#).as_bytes(),
    )
    .unwrap();
    let head = audio_head(&watch, ID, "requested", h.now.epoch());
    assert!(h.send_audio(&watch, &head, M4A).is_none());
    assert!(
        h.query(&watch, "/enter", &[("id", ID)]).is_none(),
        "it waits"
    );
    assert!(h.screen.keys().is_empty());
    h.deliver(
        0,
        report(DeliveryOutcome::Sent(Verification::Terminal), "ls"),
    );
    let frame = h.last_frame_for_request();
    assert!(
        frame.body.get("e2ee").is_some(),
        "the Enter was answered, sealed"
    );
    assert_eq!(h.screen.keys(), ["return"]);
}

#[test]
fn a_waiting_enter_gives_up_after_15_seconds() {
    let mut h = harness("enter-gives-up");
    let watch = h.pair("watch-1");
    h.screen.trusted.store(true, Ordering::SeqCst);
    let headers = h.device_headers(&watch);
    h.request(
        "POST",
        "/focus",
        &[],
        &headers,
        format!(r#"{{"id":"{ID}"}}"#).as_bytes(),
    )
    .unwrap();
    assert!(h.query(&watch, "/enter", &[("id", ID)]).is_none());
    h.advance(Duration::from_secs(ENTER_WAIT_SECONDS + 1));
    h.core.tick(h.now);
    let frame = h.last_frame_for_request();
    assert!(frame.body.get("e2ee").is_some());
    assert!(h.screen.keys().is_empty());
}

#[test]
fn undo_sends_command_z_after_a_confirmed_paste_and_marks_it_undone() {
    let mut h = harness("undo");
    let watch = h.pair("watch-1");
    read_back_paste(&mut h, &watch);
    let undone = press(&mut h, &watch, "/undo");
    assert_eq!(undone.body, json!({"ok": true, "state": "undone"}));
    assert_eq!(h.screen.keys(), ["undo"]);
    assert_eq!(h.status_of(&watch, ID)["state"], "undone");
}

/// Command-Z in a terminal would undo something unknown.
#[test]
fn undo_is_refused_for_a_terminal_and_for_an_unconfirmed_paste() {
    let mut h = harness("undo-terminal");
    let watch = h.pair("watch-1");
    pasted_into(
        &mut h,
        &watch,
        terminal(7),
        DeliveryOutcome::Sent(Verification::Terminal),
    );
    assert_eq!(
        press(&mut h, &watch, "/undo").body["reason"],
        "not-undoable"
    );

    let mut h = harness("undo-unconfirmed");
    let watch = h.pair("watch-1");
    pasted_into(
        &mut h,
        &watch,
        text_area(7),
        DeliveryOutcome::Sent(Verification::None),
    );
    assert_eq!(
        press(&mut h, &watch, "/undo").body["reason"],
        "not-undoable"
    );
    assert!(h.screen.keys().is_empty());
}

#[test]
fn undo_is_refused_after_enter_and_after_60_seconds() {
    let mut h = harness("undo-late");
    let watch = h.pair("watch-1");
    read_back_paste(&mut h, &watch);
    press(&mut h, &watch, "/enter");
    assert_eq!(press(&mut h, &watch, "/undo").body["reason"], "entered");

    let mut h = harness("undo-expired");
    let watch = h.pair("watch-1");
    read_back_paste(&mut h, &watch);
    h.advance(Duration::from_secs(UNDO_WINDOW_SECONDS + 1));
    assert_eq!(press(&mut h, &watch, "/undo").body["reason"], "expired");
}

#[test]
fn undo_in_another_application_reads_target_changed_even_for_a_password() {
    let mut h = harness("undo-moved");
    let watch = h.pair("watch-1");
    read_back_paste(&mut h, &watch);
    h.screen.secure.store(true, Ordering::SeqCst);
    assert_eq!(
        press(&mut h, &watch, "/undo").body["reason"],
        "target-changed"
    );
    assert!(h.screen.keys().is_empty());
}

#[test]
fn another_watch_cannot_press_keys_for_a_recording() {
    let mut h = harness("enter-owner");
    let first = h.pair("watch-1");
    let second = h.pair("watch-2");
    read_back_paste(&mut h, &first);
    let refused = h.query(&second, "/enter", &[("id", ID)]).unwrap();
    assert_eq!(
        (refused.status, refused.body["error"].as_str()),
        (403, Some("not-owner"))
    );
    assert!(h.screen.keys().is_empty());
}

/// A plaintext Enter can park before the envelope that ratchets its watch
/// to encrypted-only; it must be refused when the wait ends.
#[test]
fn a_parked_plaintext_enter_is_refused_once_the_watch_is_encrypted_only() {
    let mut h = harness("enter-ratchet");
    let watch = h.pair("watch-1");
    h.screen.trusted.store(true, Ordering::SeqCst);
    let headers = h.device_headers(&watch);
    h.request(
        "POST",
        "/focus",
        &[],
        &headers,
        format!(r#"{{"id":"{ID}"}}"#).as_bytes(),
    )
    .unwrap();
    let head = audio_head(&watch, ID, "requested", h.now.epoch());
    assert!(h.send_audio(&watch, &head, M4A).is_none());
    let parked = h.request(
        "POST",
        "/enter",
        &[],
        &headers,
        format!(r#"{{"id":"{ID}"}}"#).as_bytes(),
    );
    assert!(parked.is_none(), "the plaintext Enter waits");
    let frame = format!("frame-{}", h.next_frame);
    assert!(h.core.paired.require_e2ee(&watch.device));
    h.deliver(
        0,
        report(DeliveryOutcome::Sent(Verification::Terminal), "ls"),
    );
    let (status, body) = h.take(&frame).expect("answered when released");
    assert_eq!(
        (status, body["error"].as_str()),
        (403, Some("e2ee-required"))
    );
    assert!(h.screen.keys().is_empty());
}

/// Two Enters that arrive before the first key reached the app must press
/// Return once: in a terminal a second Return runs an extra command.
#[test]
fn two_enters_in_flight_press_return_once() {
    let mut h = harness("enter-twice");
    let watch = h.pair("watch-1");
    read_back_paste(&mut h, &watch);
    h.core.desktop = Desktop::manual(h.screen.clone());
    h.frames.lock().unwrap().clear();
    let first = h.query(&watch, "/enter", &[("id", ID)]);
    let second = h.query(&watch, "/enter", &[("id", ID)]);
    assert!(first.is_none() && second.is_none(), "both wait on the key");
    h.core.desktop.run_pending();
    h.pump();
    h.core.desktop.run_pending();
    h.pump();
    assert_eq!(h.screen.keys(), ["return"], "Return went in once");
    let frames = h.pending_frame_ids();
    assert_eq!(frames.len(), 2, "both requests were answered");
}

/// An Undo behind an Enter that is still in flight sees the Enter's result.
#[test]
fn an_undo_behind_an_enter_in_flight_is_refused_as_entered() {
    let mut h = harness("enter-undo");
    let watch = h.pair("watch-1");
    read_back_paste(&mut h, &watch);
    h.core.desktop = Desktop::manual(h.screen.clone());
    assert!(h.query(&watch, "/enter", &[("id", ID)]).is_none());
    assert!(h.query(&watch, "/undo", &[("id", ID)]).is_none());
    for _ in 0..3 {
        h.core.desktop.run_pending();
        h.pump();
    }
    assert_eq!(h.screen.keys(), ["return"], "no Command-Z after Enter");
}

/// The first Enter times out, a retry is queued, then the first job's late
/// answer arrives. It must free only its own claim: matching the callback by
/// recording and key kind let it answer the retry and release the keyboard
/// claim, so a third Enter pressed Return again.
#[test]
fn a_late_answer_to_a_timed_out_enter_cannot_free_the_retrys_claim() {
    let mut h = harness("enter-late");
    let watch = h.pair("watch-1");
    read_back_paste(&mut h, &watch);
    h.core.desktop = Desktop::manual(h.screen.clone());
    h.frames.lock().unwrap().clear();
    // 1: queued, never answered in time.
    assert!(h.query(&watch, "/enter", &[("id", ID)]).is_none());
    h.advance(KEY_WAIT + Duration::from_secs(1));
    h.core.tick(h.now);
    // 2: the retry, queued behind the cancelled first job.
    assert!(h.query(&watch, "/enter", &[("id", ID)]).is_none());
    // The first job's late answer and the retry's answer come in together.
    h.core.desktop.run_pending();
    h.pump();
    // 3: a further Enter, after both.
    let third = h.query(&watch, "/enter", &[("id", ID)]);
    h.core.desktop.run_pending();
    h.pump();
    assert_eq!(h.screen.keys(), ["return"], "Return went in once");
    drop(third);
    let status = h.status_of(&watch, ID);
    assert_eq!(status["entered"], true);
}

/// The first Enter's Return was already pressed when the service gave up
/// waiting for its answer. The timeout must not read that as "never sent":
/// a retry would press Return again.
#[test]
fn an_enter_that_was_sent_but_answered_late_is_never_pressed_again() {
    let mut h = harness("enter-sent-late");
    let watch = h.pair("watch-1");
    read_back_paste(&mut h, &watch);
    h.core.desktop = Desktop::manual(h.screen.clone());
    h.frames.lock().unwrap().clear();
    assert!(h.query(&watch, "/enter", &[("id", ID)]).is_none());
    // The watch-ax thread presses Return; its answer waits in the inbox.
    h.core.desktop.run_pending();
    assert_eq!(h.screen.keys(), ["return"]);
    h.advance(KEY_WAIT + Duration::from_secs(1));
    h.core.tick(h.now);
    // A retry and a third request, with the late answer arriving between.
    let retry = h.query(&watch, "/enter", &[("id", ID)]);
    h.core.desktop.run_pending();
    h.pump();
    let third = h.query(&watch, "/enter", &[("id", ID)]);
    h.core.desktop.run_pending();
    h.pump();
    drop((retry, third));
    assert_eq!(h.screen.keys(), ["return"], "Return went in once");
    assert_eq!(h.status_of(&watch, ID)["entered"], true);
}

/// Same for Command-Z: sent, then timed out, must read as undone.
#[test]
fn an_undo_that_was_sent_but_answered_late_is_never_pressed_again() {
    let mut h = harness("undo-sent-late");
    let watch = h.pair("watch-1");
    read_back_paste(&mut h, &watch);
    h.core.desktop = Desktop::manual(h.screen.clone());
    h.frames.lock().unwrap().clear();
    assert!(h.query(&watch, "/undo", &[("id", ID)]).is_none());
    h.core.desktop.run_pending();
    h.advance(KEY_WAIT + Duration::from_secs(1));
    h.core.tick(h.now);
    let retry = h.query(&watch, "/undo", &[("id", ID)]);
    h.core.desktop.run_pending();
    h.pump();
    drop(retry);
    assert_eq!(h.screen.keys(), ["undo"], "Command-Z went in once");
    assert_eq!(h.status_of(&watch, ID)["state"], "undone");
}

/// Stopping the service cancels keys still queued or waiting for the
/// keyboard lock: they must not be pressed afterwards.
#[test]
fn a_queued_key_is_not_pressed_after_the_service_stops() {
    let mut h = harness("enter-shutdown");
    let watch = h.pair("watch-1");
    read_back_paste(&mut h, &watch);
    h.core.desktop = Desktop::manual(h.screen.clone());
    assert!(h.query(&watch, "/enter", &[("id", ID)]).is_none());
    let desktop = h.core.desktop.clone();
    h.core.shutdown();
    desktop.run_pending();
    assert!(h.screen.keys().is_empty(), "no Return after shutdown");
}
