//! The status capsule follows the recording through the service, pushed
//! from the `watch` thread's own events and deadlines.

use super::*;
use crate::watch::desktop::fake::{editable, text_area};
use crate::watch::field::Withheld;
use crate::watch::status::{Capsule, Kind, READ_BACK_GRACE, Tone};
use crate::watch::suspect::SuspectReason;

type Seen = Arc<Mutex<Vec<Option<Capsule>>>>;

fn watching(h: &mut Harness) -> Seen {
    let seen: Seen = Arc::default();
    let sink = Arc::clone(&seen);
    h.core.status = Arc::new(move |capsule| sink.lock().unwrap().push(capsule));
    seen
}

fn kinds(seen: &Seen) -> Vec<Option<Kind>> {
    seen.lock()
        .unwrap()
        .iter()
        .map(|capsule| capsule.map(|capsule| capsule.kind))
        .collect()
}

/// `/focus` in a field and a recording on its way.
fn recording(h: &mut Harness, watch: &Watch) -> usize {
    h.screen.trusted.store(true, Ordering::SeqCst);
    h.screen.set_focus(Some(editable(text_area(7))));
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
    h.jobs.lock().unwrap().len() - 1
}

#[test]
fn the_capsule_follows_a_recording_from_focus_to_inserted() {
    let mut h = harness("capsule-flow");
    let seen = watching(&mut h);
    let watch = h.pair("watch-1");
    let index = recording(&mut h, &watch);
    assert_eq!(
        kinds(&seen),
        [Some(Kind::Recording), Some(Kind::Transcribing)]
    );
    h.deliver(
        index,
        report(DeliveryOutcome::Sent(Verification::Terminal), "ls"),
    );
    assert_eq!(kinds(&seen).last(), Some(&Some(Kind::InsertedInTerminal)));
    assert_eq!(
        seen.lock().unwrap().last().unwrap().unwrap().tone,
        Tone::Done
    );
}

#[test]
fn the_capsule_hides_itself_at_its_own_deadline_without_polling() {
    let mut h = harness("capsule-hide");
    let seen = watching(&mut h);
    let watch = h.pair("watch-1");
    let index = recording(&mut h, &watch);
    h.deliver(
        index,
        report(DeliveryOutcome::Sent(Verification::Terminal), "ls"),
    );
    let due = h.core.next_deadline().expect("the hide is a deadline");
    assert!(due <= h.now.at + Duration::from_millis(1200));
    h.advance(Duration::from_millis(1300));
    h.core.tick(h.now);
    assert_eq!(kinds(&seen).last(), Some(&None));
}

/// Catches flashing "couldn't confirm" for a paste the read-back is about
/// to confirm.
#[test]
fn an_unverified_paste_waits_out_the_read_back_grace() {
    let mut h = harness("capsule-grace");
    let seen = watching(&mut h);
    let watch = h.pair("watch-1");
    let index = recording(&mut h, &watch);
    h.deliver(
        index,
        report(DeliveryOutcome::Sent(Verification::None), "hello"),
    );
    assert_eq!(kinds(&seen).last(), Some(&Some(Kind::Transcribing)));
    h.advance(READ_BACK_GRACE + Duration::from_millis(10));
    h.core.tick(h.now);
    assert_eq!(kinds(&seen).last(), Some(&Some(Kind::Unconfirmed)));
    assert_eq!(
        seen.lock().unwrap().last().unwrap().unwrap().tone,
        Tone::Caution
    );
}

#[test]
fn a_read_back_inside_the_grace_shows_inserted_and_never_unconfirmed() {
    let mut h = harness("capsule-confirmed");
    let seen = watching(&mut h);
    let watch = h.pair("watch-1");
    let index = recording(&mut h, &watch);
    h.deliver(
        index,
        report(DeliveryOutcome::Sent(Verification::None), "hello"),
    );
    h.confirm(index, true);
    assert_eq!(kinds(&seen).last(), Some(&Some(Kind::Inserted)));
    h.advance(READ_BACK_GRACE + Duration::from_millis(10));
    h.core.tick(h.now);
    assert!(
        !kinds(&seen).contains(&Some(Kind::Unconfirmed)),
        "{:?}",
        kinds(&seen)
    );
}

#[test]
fn refusals_show_as_warnings() {
    for (outcome, kind) in [
        (
            DeliveryOutcome::Withheld(Withheld::TargetChanged),
            Kind::TargetChanged,
        ),
        (
            DeliveryOutcome::Withheld(Withheld::NotEditable),
            Kind::NotEditable,
        ),
        (
            DeliveryOutcome::Withheld(Withheld::SecureInput),
            Kind::SecureInput,
        ),
        (
            DeliveryOutcome::Withheld(Withheld::PasswordField),
            Kind::PasswordField,
        ),
        (
            DeliveryOutcome::Suspect(SuspectReason::Hallucination),
            Kind::Suspect,
        ),
    ] {
        let mut h = harness("capsule-warn");
        let seen = watching(&mut h);
        let watch = h.pair("watch-1");
        let index = recording(&mut h, &watch);
        h.deliver(index, report(outcome, "hello"));
        let last = seen.lock().unwrap().last().unwrap().unwrap();
        assert_eq!((last.kind, last.tone), (kind, Tone::Warning));
    }
}

#[test]
fn enter_and_undo_update_the_capsule() {
    let mut h = harness("capsule-keys");
    let seen = watching(&mut h);
    let watch = h.pair("watch-1");
    let index = recording(&mut h, &watch);
    h.deliver(
        index,
        report(DeliveryOutcome::Sent(Verification::None), "hello"),
    );
    h.confirm(index, true);
    h.query(&watch, "/enter", &[("id", ID)]).unwrap();
    assert_eq!(kinds(&seen).last(), Some(&Some(Kind::InsertedAndEntered)));

    let mut h = harness("capsule-undo");
    let seen = watching(&mut h);
    let watch = h.pair("watch-1");
    let index = recording(&mut h, &watch);
    h.deliver(
        index,
        report(DeliveryOutcome::Sent(Verification::None), "hello"),
    );
    h.confirm(index, true);
    h.query(&watch, "/undo", &[("id", ID)]).unwrap();
    assert_eq!(kinds(&seen).last(), Some(&Some(Kind::Undone)));
}

/// Catches a capsule that appears before the focused field was read: a
/// window that takes focus there would change the field it reports on.
#[test]
fn nothing_is_shown_until_the_focused_field_is_read() {
    let mut h = harness("capsule-late");
    h.core.desktop = Desktop::silent(h.screen.clone());
    let seen = watching(&mut h);
    let watch = h.pair("watch-1");
    h.screen.trusted.store(true, Ordering::SeqCst);
    let headers = h.device_headers(&watch);
    h.next_frame += 1;
    let head = RequestHead {
        id: "frame-x".into(),
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
    assert!(kinds(&seen).is_empty(), "still reading the field");
    h.advance(Duration::from_secs(2));
    h.core.tick(h.now);
    assert_eq!(kinds(&seen), [Some(Kind::Recording)]);
}

#[test]
fn a_failure_without_accessibility_says_so() {
    let mut h = harness("capsule-failed");
    let seen = watching(&mut h);
    let watch = h.pair("watch-1");
    let index = recording(&mut h, &watch);
    h.screen.trusted.store(false, Ordering::SeqCst);
    h.deliver(index, DeliveryReport::failed("no Accessibility"));
    assert_eq!(kinds(&seen).last(), Some(&Some(Kind::NoAccessibility)));
}
