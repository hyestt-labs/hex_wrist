use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use super::fake::{FakeScreen, editable, text_area};
use super::*;

fn desktop(screen: &Arc<FakeScreen>) -> Desktop {
    Desktop::inline(screen.clone())
}

fn press(desktop: &Desktop, key: Key, expected: FieldSnapshot) -> KeyOutcome {
    let (done, outcome) = mpsc::channel();
    let valid_until = Instant::now() + Duration::from_secs(5);
    desktop.press(key, expected, valid_until, Ticket::new(0), move |result| {
        done.send(result).unwrap();
    });
    outcome.recv_timeout(Duration::from_secs(2)).unwrap()
}

fn capture(desktop: &Desktop) -> Option<FieldSnapshot> {
    let (done, snapshot) = mpsc::channel();
    desktop.capture(move |result| done.send(result).unwrap());
    snapshot.recv_timeout(Duration::from_secs(2)).unwrap()
}

#[test]
fn enter_presses_return_in_the_field_the_recording_started_in() {
    let screen = Arc::new(FakeScreen::trusted_with(Some(editable(text_area(7)))));
    let outcome = press(&desktop(&screen), Key::Enter, text_area(7));
    assert_eq!(outcome, KeyOutcome::Pressed);
    assert_eq!(screen.keys(), ["return"]);
}

#[test]
fn undo_sends_its_own_key_not_return() {
    let screen = Arc::new(FakeScreen::trusted_with(Some(editable(text_area(7)))));
    assert_eq!(
        press(&desktop(&screen), Key::Undo, text_area(7)),
        KeyOutcome::Pressed
    );
    assert_eq!(screen.keys(), ["undo"]);
}

#[test]
fn a_terminal_prompt_that_grew_still_counts_as_the_same_field() {
    // Typing grows a terminal's prompt line: the frame moves, the window
    // does not. Return must still go there.
    let mut grown = text_area(7);
    grown.frame.y -= 400.0;
    grown.frame.height += 400.0;
    let screen = Arc::new(FakeScreen::trusted_with(Some(editable(grown))));
    assert_eq!(
        press(&desktop(&screen), Key::Enter, text_area(7)),
        KeyOutcome::Pressed
    );
}

#[test]
fn another_application_in_front_gets_no_key() {
    let screen = Arc::new(FakeScreen::trusted_with(Some(editable(text_area(8)))));
    assert_eq!(
        press(&desktop(&screen), Key::Enter, text_area(7)),
        KeyOutcome::TargetChanged
    );
    assert!(screen.keys().is_empty());
}

#[test]
fn no_focus_at_all_is_a_changed_target() {
    let screen = Arc::new(FakeScreen::trusted_with(None));
    assert_eq!(
        press(&desktop(&screen), Key::Enter, text_area(7)),
        KeyOutcome::TargetChanged
    );
}

#[test]
fn secure_keyboard_entry_blocks_the_key_even_in_the_right_field() {
    let screen = Arc::new(FakeScreen::trusted_with(Some(editable(text_area(7)))));
    screen.secure.store(true, Ordering::SeqCst);
    assert_eq!(
        press(&desktop(&screen), Key::Enter, text_area(7)),
        KeyOutcome::Sensitive
    );
    assert!(screen.keys().is_empty());
}

#[test]
fn a_password_field_in_front_blocks_the_key() {
    let mut password = text_area(7);
    password.role = "AXSecureTextField".into();
    let screen = Arc::new(FakeScreen::trusted_with(Some(editable(password))));
    assert_eq!(
        press(&desktop(&screen), Key::Enter, text_area(7)),
        KeyOutcome::Sensitive
    );
}

#[test]
fn without_accessibility_a_key_fails_instead_of_posting_into_the_void() {
    let screen = Arc::new(FakeScreen::default());
    assert!(matches!(
        press(&desktop(&screen), Key::Enter, text_area(7)),
        KeyOutcome::Failed(_)
    ));
    assert!(screen.keys().is_empty());
}

#[test]
fn a_failed_key_event_is_reported() {
    let screen = Arc::new(FakeScreen::trusted_with(Some(editable(text_area(7)))));
    screen.fail_keys.store(true, Ordering::SeqCst);
    assert_eq!(
        press(&desktop(&screen), Key::Enter, text_area(7)),
        KeyOutcome::Failed("no event source".to_owned())
    );
}

#[test]
fn capture_reads_the_focused_field_only_with_accessibility() {
    let screen = Arc::new(FakeScreen::trusted_with(Some(editable(text_area(7)))));
    assert_eq!(capture(&desktop(&screen)), Some(text_area(7)));
    screen.trusted.store(false, Ordering::SeqCst);
    assert_eq!(capture(&desktop(&screen)), None);
}

#[test]
fn the_ax_thread_answers_off_the_callers_thread() {
    let screen = Arc::new(FakeScreen::trusted_with(Some(editable(text_area(7)))));
    let desktop = Desktop::start(screen).unwrap();
    let (done, name) = mpsc::channel();
    desktop.capture(move |_| {
        done.send(std::thread::current().name().map(str::to_owned))
            .unwrap();
    });
    let name = name.recv_timeout(Duration::from_secs(2)).unwrap();
    assert_eq!(name.as_deref(), Some("watch-ax"));
}

#[test]
fn a_key_waits_for_a_paste_that_holds_the_lock() {
    let screen = Arc::new(FakeScreen::trusted_with(Some(editable(text_area(7)))));
    let desktop = Desktop::start(screen.clone()).unwrap();
    let guard = desktop.lock();
    let (done, outcome) = mpsc::channel();
    let valid_until = Instant::now() + Duration::from_secs(5);
    desktop.press(
        Key::Enter,
        text_area(7),
        valid_until,
        Ticket::new(0),
        move |result| {
            done.send(result).unwrap();
        },
    );
    // Held: nothing is pressed yet.
    assert!(outcome.recv_timeout(Duration::from_millis(150)).is_err());
    assert!(screen.keys().is_empty());
    drop(guard);
    assert_eq!(
        outcome.recv_timeout(Duration::from_secs(2)).unwrap(),
        KeyOutcome::Pressed
    );
}

/// Catches an Enter pressed after the watch was told the request failed.
#[test]
fn a_key_for_a_request_given_up_on_is_not_pressed() {
    let screen = Arc::new(FakeScreen::trusted_with(Some(editable(text_area(7)))));
    let desktop = desktop(&screen);
    let (done, outcome) = mpsc::channel();
    desktop.press(
        Key::Enter,
        text_area(7),
        Instant::now(),
        Ticket::new(0),
        move |result| {
            done.send(result).unwrap();
        },
    );
    assert!(matches!(
        outcome.recv_timeout(Duration::from_secs(2)).unwrap(),
        KeyOutcome::Failed(_)
    ));
    assert!(screen.keys().is_empty());
}

/// Catches an Enter pressed for a request the service already cancelled
/// (a recording that started over, a reply given up on).
#[test]
fn a_cancelled_ticket_presses_nothing() {
    let screen = Arc::new(FakeScreen::trusted_with(Some(editable(text_area(7)))));
    let desktop = desktop(&screen);
    let ticket = Ticket::new(0);
    ticket.cancel();
    let (done, outcome) = mpsc::channel();
    let valid_until = Instant::now() + Duration::from_secs(5);
    desktop.press(
        Key::Enter,
        text_area(7),
        valid_until,
        ticket,
        move |result| {
            done.send(result).unwrap();
        },
    );
    assert!(matches!(
        outcome.recv_timeout(Duration::from_secs(2)).unwrap(),
        KeyOutcome::Failed(_)
    ));
    assert!(screen.keys().is_empty());
}

/// Catches a Return sent to the app the user switched to while the focus
/// was being read: the target is read again right before the key.
#[test]
fn the_target_is_read_again_right_before_the_key() {
    let screen = Arc::new(FakeScreen::trusted_with(Some(editable(text_area(8)))));
    screen
        .focus_script
        .lock()
        .unwrap()
        .push_back(Some(editable(text_area(7))));
    assert_eq!(
        press(&desktop(&screen), Key::Enter, text_area(7)),
        KeyOutcome::TargetChanged
    );
    assert!(screen.keys().is_empty());
}

/// Catches a Return sent after the reply was given up on because reading
/// the focus took longer than the request had left.
#[test]
fn a_slow_focus_read_does_not_press_after_the_deadline() {
    let screen = Arc::new(FakeScreen::trusted_with(Some(editable(text_area(7)))));
    *screen.focus_delay.lock().unwrap() = Duration::from_millis(120);
    let desktop = desktop(&screen);
    let (done, outcome) = mpsc::channel();
    let valid_until = Instant::now() + Duration::from_millis(100);
    desktop.press(
        Key::Enter,
        text_area(7),
        valid_until,
        Ticket::new(0),
        move |result| {
            done.send(result).unwrap();
        },
    );
    assert!(matches!(
        outcome.recv_timeout(Duration::from_secs(2)).unwrap(),
        KeyOutcome::Failed(_)
    ));
    assert!(screen.keys().is_empty());
}

#[test]
fn a_pressed_key_marks_its_ticket_sent_and_it_can_no_longer_be_cancelled() {
    let screen = Arc::new(FakeScreen::trusted_with(Some(editable(text_area(7)))));
    let desktop = desktop(&screen);
    let ticket = Ticket::new(1);
    let (done, outcome) = mpsc::channel();
    let valid_until = Instant::now() + Duration::from_secs(5);
    desktop.press(
        Key::Enter,
        text_area(7),
        valid_until,
        ticket.clone(),
        move |r| {
            done.send(r).unwrap();
        },
    );
    assert_eq!(outcome.recv().unwrap(), KeyOutcome::Pressed);
    assert!(ticket.was_sent());
    assert!(
        !ticket.cancel(),
        "the caller learns the key already went out"
    );
}

#[test]
fn a_cancelled_ticket_is_never_sent() {
    let ticket = Ticket::new(1);
    assert!(ticket.cancel());
    assert!(!ticket.claim());
    assert!(!ticket.was_sent());
}
