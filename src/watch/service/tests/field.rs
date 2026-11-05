//! `/focus` captures the field the paste must match; the recording carries
//! it as the expectation the output worker gates on.

use super::*;
use crate::watch::desktop::fake::{FakeScreen, editable, text_area};
use crate::watch::field::Expectation;
use crate::watch::service::state::CAPTURE_WAIT;

fn focus(h: &mut Harness, watch: &Watch) -> (u16, Value) {
    let headers = h.device_headers(watch);
    h.request(
        "POST",
        "/focus",
        &[],
        &headers,
        format!(r#"{{"id":"{ID}"}}"#).as_bytes(),
    )
    .expect("the capture answers")
}

fn expectation(h: &Harness, index: usize) -> Expectation {
    h.jobs.lock().unwrap()[index].1.job().expectation.clone()
}

fn requested(h: &mut Harness, watch: &Watch) -> Option<Answer> {
    let head = audio_head(watch, ID, "requested", h.now.epoch());
    h.send_audio(watch, &head, M4A)
}

#[test]
fn focus_reports_the_role_it_captured() {
    let mut h = harness("focus-role");
    h.screen.trusted.store(true, Ordering::SeqCst);
    h.screen.set_focus(Some(editable(text_area(7))));
    let watch = h.pair("watch-1");
    let (status, body) = focus(&mut h, &watch);
    assert_eq!(status, 200);
    assert_eq!(
        body,
        json!({"ok": true, "captured": true, "role": "AXTextArea"})
    );
}

/// Catches pasting at the cursor when the watch locked a field: the audio
/// must carry the snapshot, not `Current`.
#[test]
fn audio_after_focus_must_match_the_captured_field() {
    let mut h = harness("focus-field");
    h.screen.trusted.store(true, Ordering::SeqCst);
    h.screen.set_focus(Some(editable(text_area(7))));
    let watch = h.pair("watch-1");
    focus(&mut h, &watch);
    assert!(requested(&mut h, &watch).is_none());
    assert_eq!(expectation(&h, 0), Expectation::Field(text_area(7)));
}

/// Wristper pastes at the cursor when `/focus` ran but saw no field
/// (nothing focused, or Accessibility off).
#[test]
fn a_focus_that_captured_nothing_pastes_at_the_cursor() {
    let mut h = harness("focus-empty");
    let watch = h.pair("watch-1");
    let (_, body) = focus(&mut h, &watch);
    assert_eq!(body["captured"], false);
    assert!(requested(&mut h, &watch).is_none());
    assert_eq!(expectation(&h, 0), Expectation::Current);
}

/// A resend from the unsent list names no field; whatever `/focus` saw
/// earlier must not pull it back to an old place.
#[test]
fn focus_current_ignores_an_earlier_snapshot() {
    let mut h = harness("focus-current");
    h.screen.trusted.store(true, Ordering::SeqCst);
    h.screen.set_focus(Some(editable(text_area(7))));
    let watch = h.pair("watch-1");
    focus(&mut h, &watch);
    h.start_recording(&watch, ID);
    assert_eq!(expectation(&h, 0), Expectation::Current);
}

/// Audio that beat its `/focus` and waited the full 2 s pastes nowhere.
#[test]
fn audio_without_any_focus_is_not_pasted() {
    let mut h = harness("focus-missing");
    let watch = h.pair("watch-1");
    assert!(requested(&mut h, &watch).is_none());
    assert!(h.jobs.lock().unwrap().is_empty(), "it waits for /focus");
    h.advance(Duration::from_secs(3));
    h.core.tick(h.now);
    assert_eq!(expectation(&h, 0), Expectation::Missing);
}

#[test]
fn audio_that_beat_its_focus_runs_with_the_field_once_it_lands() {
    let mut h = harness("focus-late");
    h.screen.trusted.store(true, Ordering::SeqCst);
    h.screen.set_focus(Some(editable(text_area(7))));
    let watch = h.pair("watch-1");
    assert!(requested(&mut h, &watch).is_none());
    assert!(h.jobs.lock().unwrap().is_empty());
    focus(&mut h, &watch);
    assert_eq!(expectation(&h, 0), Expectation::Field(text_area(7)));
}

/// The watch thread must not wait on a hung application: the reply goes
/// out at the capture deadline, and a capture that lands later changes
/// nothing.
#[test]
fn a_silent_application_answers_captured_false_at_the_deadline() {
    let mut h = harness("focus-silent");
    h.core.desktop = Desktop::silent(h.screen.clone());
    let watch = h.pair("watch-1");
    let headers = h.device_headers(&watch);
    let id = {
        h.next_frame += 1;
        let id = format!("frame-{}", h.next_frame);
        let head = RequestHead {
            id: id.clone(),
            method: "POST".into(),
            path: "/focus".into(),
            query: BTreeMap::new(),
            headers: headers
                .iter()
                .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
                .collect(),
            body_len: 0,
        };
        h.core
            .on_request(head, format!(r#"{{"id":"{ID}"}}"#).into_bytes(), h.now);
        id
    };
    assert!(h.take(&id).is_none(), "the reply is parked");
    h.advance(CAPTURE_WAIT + Duration::from_millis(10));
    h.core.tick(h.now);
    let (status, body) = h.take(&id).expect("answered at the deadline");
    assert_eq!(status, 200);
    assert_eq!(body, json!({"ok": true, "captured": false, "role": ""}));
    // It is awaiting audio all the same, and a late capture is ignored.
    let status = h.status_of(&watch, ID);
    assert_eq!(status["state"], "awaiting-audio");
    h.core.handle(
        Message::Captured {
            id: ID.to_owned(),
            field: Some(text_area(7)),
        },
        h.now,
    );
    assert!(h.pending_frame_ids().is_empty());
}

#[test]
fn health_follows_the_accessibility_permission() {
    let mut h = harness("health-ax");
    let watch = h.pair("watch-1");
    let headers = h.device_headers(&watch);
    let (_, body) = h.request("GET", "/health", &[], &headers, &[]).unwrap();
    assert_eq!(body["paste_ready"], false);
    h.screen.trusted.store(true, Ordering::SeqCst);
    let (_, body) = h.request("GET", "/health", &[], &headers, &[]).unwrap();
    assert_eq!(body["paste_ready"], true);
    assert_eq!(body["paste_detail"], "");
}

#[allow(dead_code)]
fn _uses(_: &FakeScreen) {}
