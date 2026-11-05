use std::sync::atomic::Ordering;
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};

use super::*;
use crate::watch::delivery::{DeliveryEvent, DeliveryReport, Job};
use crate::watch::desktop::Desktop;
use crate::watch::desktop::fake::{FakeElement, FakeScreen, editable, text_area};
use crate::watch::field::{ElementRef, Expectation, FieldSnapshot, Focused, Withheld};

#[derive(Default)]
struct FakeInserter {
    inserted: Vec<String>,
    copied: Vec<String>,
    /// What `insert` reports as the text that went in.
    rewrite: Option<String>,
    fail: bool,
    /// The delays the clipboard restore was asked for.
    restores: Arc<Mutex<Vec<Duration>>>,
}

impl Inserter for FakeInserter {
    fn insert(&mut self, text: &str, commit: &dyn Fn() -> bool) -> Result<Inserted, String> {
        if self.fail || !commit() {
            return Err("paste was cancelled".to_owned());
        }
        self.inserted.push(text.to_owned());
        let restores = Arc::clone(&self.restores);
        Ok(Inserted {
            text: self.rewrite.clone().unwrap_or_else(|| text.to_owned()),
            restore: Some(Box::new(move |delay| restores.lock().unwrap().push(delay))),
        })
    }

    fn copy(&mut self, text: &str, _commit: &dyn Fn() -> bool) -> Result<(), String> {
        self.copied.push(text.to_owned());
        Ok(())
    }
}

struct Rig {
    screen: Arc<FakeScreen>,
    delivery: WatchDelivery,
    events: Receiver<DeliveryEvent>,
    inserter: FakeInserter,
}

fn rig(language: &str, expectation: Expectation, focus: Option<Focused>) -> Rig {
    let screen = Arc::new(FakeScreen::trusted_with(focus));
    let (sink, events) = mpsc::channel();
    let delivery = WatchDelivery::new(
        Job {
            id: "6f9619ff-8b86-d011-b42d-00c04fc964ff".into(),
            language: language.into(),
            expectation,
            desktop: Desktop::inline(screen.clone()),
        },
        move |event| {
            let _ = sink.send(event);
        },
    );
    Rig {
        screen,
        delivery,
        events,
        inserter: FakeInserter::default(),
    }
}

fn run(rig: &mut Rig, text: &str, raw: &str, copy_only: bool) -> Result<Delivered, String> {
    deliver(
        &mut rig.inserter,
        &rig.delivery,
        &Request {
            text,
            raw,
            copy_only,
            preferred: &["en-US".to_owned()],
        },
        &|| true,
    )
}

fn say(rig: &mut Rig, text: &str) -> Delivered {
    run(rig, text, text, false).expect("delivered")
}

fn in_field() -> Rig {
    rig(
        "en",
        Expectation::Field(text_area(7)),
        Some(editable(text_area(7))),
    )
}

fn terminal_field() -> FieldSnapshot {
    FieldSnapshot {
        bundle_id: Some("com.apple.Terminal".into()),
        role: "AXTextArea".into(),
        ..text_area(7)
    }
}

#[test]
fn a_matching_editable_field_gets_the_paste() {
    let mut rig = in_field();
    let delivered = say(&mut rig, "hello there");
    assert_eq!(rig.inserter.inserted, ["hello there"]);
    assert_eq!(delivered.outcome, DeliveryOutcome::Sent(Verification::None));
    assert!(delivered.follow_up.is_some());
}

#[test]
fn a_terminal_is_final_at_once() {
    let mut rig = rig(
        "en",
        Expectation::Field(terminal_field()),
        Some(editable(terminal_field())),
    );
    let delivered = say(&mut rig, "ls -la");
    assert_eq!(
        delivered.outcome,
        DeliveryOutcome::Sent(Verification::Terminal)
    );
}

/// Catches pasting into whatever has focus now instead of the field the
/// watch started in.
#[test]
fn another_application_in_front_withholds_the_paste() {
    let mut rig = rig(
        "en",
        Expectation::Field(text_area(7)),
        Some(editable(text_area(8))),
    );
    let delivered = say(&mut rig, "secret plans");
    assert_eq!(
        delivered.outcome,
        DeliveryOutcome::Withheld(Withheld::TargetChanged)
    );
    assert!(rig.inserter.inserted.is_empty());
    assert_eq!(delivered.text, "secret plans", "the record keeps the text");
}

#[test]
fn nothing_focused_at_all_is_a_changed_target_when_a_field_was_locked() {
    let mut rig = rig("en", Expectation::Field(text_area(7)), None);
    assert_eq!(
        say(&mut rig, "hello").outcome,
        DeliveryOutcome::Withheld(Withheld::TargetChanged)
    );
}

#[test]
fn secure_keyboard_entry_withholds_even_the_right_field() {
    let mut rig = in_field();
    rig.screen.secure.store(true, Ordering::SeqCst);
    assert_eq!(
        say(&mut rig, "hunter2").outcome,
        DeliveryOutcome::Withheld(Withheld::SecureInput)
    );
    assert!(rig.inserter.inserted.is_empty());
}

#[test]
fn a_password_field_withholds_even_at_the_cursor() {
    let mut password = text_area(7);
    password.role = "AXSecureTextField".into();
    let mut rig = rig("en", Expectation::Current, Some(editable(password)));
    assert_eq!(
        say(&mut rig, "hunter2").outcome,
        DeliveryOutcome::Withheld(Withheld::PasswordField)
    );
}

#[test]
fn a_field_that_cannot_be_edited_withholds() {
    let mut rig = rig(
        "en",
        Expectation::Field(text_area(7)),
        Some(Focused {
            field: text_area(7),
            element: None,
            editable: false,
        }),
    );
    assert_eq!(
        say(&mut rig, "hello").outcome,
        DeliveryOutcome::Withheld(Withheld::NotEditable)
    );
}

#[test]
fn audio_without_a_focus_snapshot_is_withheld() {
    let mut rig = rig("en", Expectation::Missing, Some(editable(text_area(7))));
    assert_eq!(
        say(&mut rig, "hello").outcome,
        DeliveryOutcome::Withheld(Withheld::NoSnapshot)
    );
}

#[test]
fn at_the_cursor_with_nothing_focused_only_copies() {
    let mut rig = rig("en", Expectation::Current, None);
    let delivered = say(&mut rig, "hello");
    assert_eq!(delivered.outcome, DeliveryOutcome::Copied);
    assert_eq!(rig.inserter.copied, ["hello"]);
    assert!(rig.inserter.inserted.is_empty());
}

#[test]
fn at_the_cursor_pastes_without_matching_a_field() {
    let mut rig = rig("en", Expectation::Current, Some(editable(text_area(9))));
    assert_eq!(
        say(&mut rig, "hello").outcome,
        DeliveryOutcome::Sent(Verification::None)
    );
}

#[test]
fn without_accessibility_the_paste_fails_instead_of_posting_into_the_void() {
    let mut rig = in_field();
    rig.screen.trusted.store(false, Ordering::SeqCst);
    let error = run(&mut rig, "hello", "hello", false).err().unwrap();
    assert!(error.contains("Accessibility"), "{error}");
    assert!(rig.inserter.inserted.is_empty());
}

#[test]
fn copy_only_needs_no_accessibility() {
    let mut rig = in_field();
    rig.screen.trusted.store(false, Ordering::SeqCst);
    let delivered = run(&mut rig, "hello", "hello", true).unwrap();
    assert_eq!(delivered.outcome, DeliveryOutcome::Copied);
    assert_eq!(rig.inserter.copied, ["hello"]);
}

#[test]
fn a_failed_insert_fails_the_delivery_and_frees_the_keyboard() {
    let mut rig = in_field();
    rig.inserter.fail = true;
    assert!(run(&mut rig, "hello", "hello", false).is_err());
    // The lock came back: a key press would not wait.
    let desktop = rig.delivery.job().desktop.clone();
    drop(desktop.lock());
}

#[test]
fn a_silence_hallucination_is_suspect_and_never_pasted() {
    let mut rig = in_field();
    let delivered = say(&mut rig, "Thank you.");
    assert_eq!(
        delivered.outcome,
        DeliveryOutcome::Suspect(suspect::SuspectReason::Hallucination)
    );
    assert!(rig.inserter.inserted.is_empty());
}

#[test]
fn japanese_kana_in_a_chinese_recording_is_suspect() {
    let mut rig = rig("zh", Expectation::Current, Some(editable(text_area(7))));
    assert_eq!(
        say(&mut rig, "今天のテスト").outcome,
        DeliveryOutcome::Suspect(suspect::SuspectReason::Kana)
    );
}

#[test]
fn suspect_rules_read_what_was_heard_not_what_the_output_steps_made() {
    // The rewrite step turned a hallucinated caption into plausible text.
    let mut rig = in_field();
    let delivered = run(&mut rig, "A polite closing remark.", "Thank you.", false).unwrap();
    assert!(matches!(delivered.outcome, DeliveryOutcome::Suspect(_)));
}

#[test]
fn an_empty_result_is_suspect() {
    let mut rig = in_field();
    assert_eq!(
        say(&mut rig, "  ").outcome,
        DeliveryOutcome::Suspect(suspect::SuspectReason::Empty)
    );
}

#[test]
fn chinese_is_pasted_in_traditional_and_english_is_left_alone() {
    let mut rig = rig("zh", Expectation::Current, Some(editable(text_area(7))));
    let delivered = say(&mut rig, "我想要这个 OpenAI 的 GPT-5 头发");
    assert_eq!(rig.inserter.inserted, ["我想要這個 OpenAI 的 GPT-5 頭髮"]);
    assert_eq!(delivered.text, "我想要這個 OpenAI 的 GPT-5 頭髮");
}

#[test]
fn an_english_recording_is_never_converted() {
    let mut rig = in_field();
    say(&mut rig, "头发 stays as spoken");
    assert_eq!(rig.inserter.inserted, ["头发 stays as spoken"]);
}

#[test]
fn raw_text_is_converted_too_and_dropped_when_equal() {
    let mut rig = rig("zh", Expectation::Current, Some(editable(text_area(7))));
    let delivered = run(&mut rig, "头发", "头发", false).unwrap();
    assert_eq!(delivered.raw, None);
    // Each delivery holds the keyboard until its follow-up ends.
    drop(delivered);
    let delivered = run(&mut rig, "頭髮好", "头发好", false).unwrap();
    assert_eq!(delivered.raw, None, "both read 頭髮好");
    drop(delivered);
    let delivered = run(&mut rig, "頭髮", "头发 uh", false).unwrap();
    assert_eq!(delivered.raw.as_deref(), Some("頭髮 uh"));
}

#[test]
fn restore_waits_four_seconds_unless_the_read_back_confirmed() {
    assert_eq!(restore_delay(false, true), Duration::ZERO);
    assert_eq!(restore_delay(false, false), RESTORE_GRACE);
    // A terminal is never read back; even a stray confirmation waits.
    assert_eq!(restore_delay(true, false), RESTORE_GRACE);
    assert_eq!(restore_delay(true, true), RESTORE_GRACE);
}

fn confirmations(rig: &Rig) -> Vec<bool> {
    rig.events
        .try_iter()
        .filter_map(|event| match event {
            DeliveryEvent::Confirmed { verified } => Some(verified),
            DeliveryEvent::Report(_) => None,
        })
        .collect()
}

fn reported(rig: &Rig, delivered: &Delivered) {
    rig.delivery.deliver(DeliveryReport {
        outcome: delivered.outcome.clone(),
        text: delivered.text.clone(),
        raw_text: None,
        transcribe_ms: 0,
        paste_ms: 0,
    });
}

#[test]
fn a_seen_paste_is_confirmed_and_the_clipboard_comes_back_at_once() {
    let mut rig = in_field();
    rig.screen.script_values([Some(""), Some("hello there")]);
    let delivered = say(&mut rig, "hello there");
    reported(&rig, &delivered);
    delivered.follow_up.unwrap().run(&rig.delivery, |_| {});
    assert_eq!(confirmations(&rig), [true]);
    assert_eq!(*rig.inserter.restores.lock().unwrap(), [Duration::ZERO]);
}

#[test]
fn a_paste_nobody_can_see_is_unverified_and_the_clipboard_waits() {
    let mut rig = in_field();
    rig.screen.script_values([Some("")]);
    let delivered = say(&mut rig, "hello there");
    reported(&rig, &delivered);
    delivered.follow_up.unwrap().run(&rig.delivery, |_| {});
    assert_eq!(confirmations(&rig), [false]);
    assert_eq!(*rig.inserter.restores.lock().unwrap(), [RESTORE_GRACE]);
}

/// Catches confirming a paste that never landed because the same words
/// were already in the field.
#[test]
fn words_that_were_already_there_do_not_confirm() {
    let mut rig = in_field();
    rig.screen.script_values([Some("hello there")]);
    let delivered = say(&mut rig, "hello there");
    reported(&rig, &delivered);
    delivered.follow_up.unwrap().run(&rig.delivery, |_| {});
    assert_eq!(confirmations(&rig), [false]);
}

/// Catches comparing against the text the watch sent when the paste added
/// a joining space or changed the first letter.
#[test]
fn the_read_back_looks_for_the_text_that_went_in() {
    let mut rig = in_field();
    rig.inserter.rewrite = Some(" Hello there".into());
    rig.screen.script_values([Some(""), Some("Hello there")]);
    let delivered = say(&mut rig, "hello there");
    reported(&rig, &delivered);
    delivered.follow_up.unwrap().run(&rig.delivery, |_| {});
    assert_eq!(confirmations(&rig), [true]);
}

#[test]
fn a_terminal_is_not_read_back_and_its_clipboard_waits() {
    let mut rig = rig(
        "en",
        Expectation::Field(terminal_field()),
        Some(editable(terminal_field())),
    );
    let delivered = say(&mut rig, "ls");
    reported(&rig, &delivered);
    delivered.follow_up.unwrap().run(&rig.delivery, |_| {});
    assert!(confirmations(&rig).is_empty());
    assert_eq!(*rig.inserter.restores.lock().unwrap(), [RESTORE_GRACE]);
}

/// Catches a confirmation reaching the watch thread before the report it
/// refines, which `WatchDelivery::confirm` would drop.
#[test]
fn a_confirmation_before_the_report_is_dropped_not_misordered() {
    let mut rig = in_field();
    rig.screen.script_values([Some(""), Some("hello there")]);
    let delivered = say(&mut rig, "hello there");
    delivered.follow_up.unwrap().run(&rig.delivery, |_| {});
    assert!(confirmations(&rig).is_empty());
}

#[test]
fn the_keyboard_stays_held_until_the_read_back_is_done() {
    let mut rig = in_field();
    rig.screen.script_values([Some(""), Some("hello there")]);
    let delivered = say(&mut rig, "hello there");
    let desktop = rig.delivery.job().desktop.clone();
    let (acquired, waiting) = mpsc::channel();
    let probe = {
        let desktop = desktop.clone();
        std::thread::spawn(move || {
            drop(desktop.lock());
            acquired.send(()).unwrap();
        })
    };
    assert!(
        waiting.recv_timeout(Duration::from_millis(150)).is_err(),
        "an Enter must wait for the read-back"
    );
    delivered.follow_up.unwrap().run(&rig.delivery, |_| {});
    waiting.recv_timeout(Duration::from_secs(2)).unwrap();
    probe.join().unwrap();
}

/// Catches an Enter, parked on the keyboard lock, pressing before the
/// confirmation went out: the record would then stay `unverified` for good.
#[test]
fn the_confirmation_goes_out_before_the_keyboard_is_released() {
    let screen = Arc::new(FakeScreen::trusted_with(Some(editable(text_area(7)))));
    screen.script_values([Some(""), Some("hello there")]);
    let seen: Arc<Mutex<Vec<bool>>> = Arc::default();
    let log = Arc::clone(&seen);
    let delivery = WatchDelivery::new(
        Job {
            id: "6f9619ff-8b86-d011-b42d-00c04fc964ff".into(),
            language: "en".into(),
            expectation: Expectation::Field(text_area(7)),
            desktop: Desktop::inline(screen.clone()),
        },
        move |event| {
            if let DeliveryEvent::Confirmed { verified } = event {
                log.lock().unwrap().push(verified);
            }
        },
    );
    let mut inserter = FakeInserter::default();
    let delivered = deliver(
        &mut inserter,
        &delivery,
        &Request {
            text: "hello there",
            raw: "hello there",
            copy_only: false,
            preferred: &[],
        },
        &|| true,
    )
    .unwrap();
    delivery.deliver(DeliveryReport::failed("placeholder report"));
    // The thread that wins the lock after the read-back looks at what the
    // watch thread has heard by then.
    let desktop = delivery.job().desktop.clone();
    let heard = Arc::clone(&seen);
    let waiter = std::thread::spawn(move || {
        let _held = desktop.lock();
        heard.lock().unwrap().clone()
    });
    std::thread::sleep(Duration::from_millis(100));
    delivered.follow_up.unwrap().run(&delivery, |_| {});
    assert_eq!(waiter.join().unwrap(), [true]);
}

/// The user switched apps after the gate read the focus and before ⌘V:
/// the focus is read again right before the paste.
#[test]
fn the_focus_is_read_again_right_before_the_paste() {
    let mut rig = in_field();
    rig.screen
        .focus_script
        .lock()
        .unwrap()
        .push_back(Some(editable(text_area(7))));
    rig.screen.set_focus(Some(editable(text_area(8))));
    let delivered = say(&mut rig, "hello there");
    assert_eq!(
        delivered.outcome,
        DeliveryOutcome::Withheld(Withheld::TargetChanged)
    );
    assert!(rig.inserter.inserted.is_empty());
    assert!(delivered.follow_up.is_none());
}

/// Moving between two fields of the same app counts too.
#[test]
fn a_switch_to_a_password_field_before_the_paste_is_sensitive() {
    let mut rig = in_field();
    let mut password = text_area(7);
    password.role = "AXSecureTextField".into();
    rig.screen
        .focus_script
        .lock()
        .unwrap()
        .push_back(Some(editable(text_area(7))));
    rig.screen.set_focus(Some(editable(password)));
    assert_eq!(
        say(&mut rig, "hunter2").outcome,
        DeliveryOutcome::Withheld(Withheld::PasswordField)
    );
    assert!(rig.inserter.inserted.is_empty());
}

fn focused_through(field: FieldSnapshot, element: &Arc<FakeElement>) -> Focused {
    Focused {
        field,
        element: Some(ElementRef(element.clone())),
        editable: true,
    }
}

/// Catches confirming a paste because another field, which already held
/// the words, had focus when the read-back looked: undo would then be
/// offered for a paste that never reached the field it is undone in.
#[test]
fn the_read_back_reads_the_element_that_was_pasted_into() {
    let pasted_into = FakeElement::scripted([Some("")]);
    let elsewhere = FakeElement::scripted([Some("hello there")]);
    let mut rig = in_field();
    {
        let mut script = rig.screen.focus_script.lock().unwrap();
        script.push_back(Some(focused_through(text_area(7), &pasted_into)));
        script.push_back(Some(focused_through(text_area(7), &pasted_into)));
    }
    // After the paste, focus has moved to another field of the same app.
    rig.screen
        .set_focus(Some(focused_through(text_area(7), &elsewhere)));
    let delivered = say(&mut rig, "hello there");
    reported(&rig, &delivered);
    delivered.follow_up.unwrap().run(&rig.delivery, |_| {});
    assert_eq!(confirmations(&rig), [false]);
    assert_eq!(*rig.inserter.restores.lock().unwrap(), [RESTORE_GRACE]);
}

#[test]
fn the_read_back_confirms_when_the_pasted_element_shows_the_words() {
    let pasted_into = FakeElement::scripted([Some(""), Some("hello there")]);
    let mut rig = in_field();
    {
        let mut script = rig.screen.focus_script.lock().unwrap();
        script.push_back(Some(focused_through(text_area(7), &pasted_into)));
        script.push_back(Some(focused_through(text_area(7), &pasted_into)));
    }
    let delivered = say(&mut rig, "hello there");
    reported(&rig, &delivered);
    delivered.follow_up.unwrap().run(&rig.delivery, |_| {});
    assert_eq!(confirmations(&rig), [true]);
}

/// Catches the deferred clipboard restore being dropped with the read-back
/// thread that could not start: the transcript would stay on the clipboard.
#[test]
fn a_read_back_that_cannot_start_still_restores_the_clipboard() {
    let mut rig = in_field();
    let delivered = say(&mut rig, "hello there");
    let desktop = rig.delivery.job().desktop.clone();
    delivered
        .follow_up
        .unwrap()
        .spawn_with(rig.delivery.clone(), |_job| {
            Err(std::io::Error::other("no threads left"))
        });
    assert_eq!(*rig.inserter.restores.lock().unwrap(), [Duration::ZERO]);
    // The keyboard is free again.
    drop(desktop.lock());
}
