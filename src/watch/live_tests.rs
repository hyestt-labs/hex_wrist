//! Checks against real applications: Accessibility, the Paster, and key
//! events, in TextEdit and Terminal. They drive the screen, so they are
//! `#[ignore]`d and run by hand:
//!
//! ```sh
//! cargo nextest run --locked --bin hex --run-ignored only -E 'test(watch_live)'
//! ```
//!
//! The terminal that runs them needs Accessibility (System Settings ›
//! Privacy & Security › Accessibility) and Automation access to TextEdit,
//! Terminal, and Finder (the first run asks). Nothing else may take focus
//! while they run. The clipboard is restored by the Paster; TextEdit's
//! scratch document and Terminal's window are closed afterwards.

use std::process::Command;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use super::delivery::{DeliveryEvent, DeliveryOutcome, DeliveryReport, Job, WatchDelivery};
use super::desktop::{Desktop, Key, KeyOutcome, Screen, Ticket};
use super::field::{Expectation, FieldSnapshot, Withheld};
use super::output::{Delivered, Request, deliver};
use super::recording::Verification;
use super::screen::MacScreen;
use crate::paste::Paster;
use crate::suppression::InputActivity;

fn osascript(script: &str) -> String {
    let output = Command::new("/usr/bin/osascript")
        .args(["-e", script])
        .output()
        .expect("osascript runs");
    assert!(
        output.status.success(),
        "osascript failed (Automation access for this terminal?): {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn settle() {
    std::thread::sleep(Duration::from_millis(1200));
}

struct Rig {
    desktop: Desktop,
    paster: Paster,
}

impl Rig {
    fn new() -> Self {
        assert!(
            MacScreen.trusted(),
            "grant Accessibility to the terminal running this test"
        );
        Self {
            desktop: Desktop::start(Arc::new(MacScreen)).unwrap(),
            paster: Paster::new(InputActivity::default()),
        }
    }

    fn focused(&self) -> FieldSnapshot {
        MacScreen.focused().expect("a focused field").field
    }

    fn delivery(&self, expectation: Expectation) -> (WatchDelivery, Receiver<DeliveryEvent>) {
        let (sink, events) = mpsc::channel();
        let delivery = WatchDelivery::new(
            Job {
                id: "6f9619ff-8b86-d011-b42d-00c04fc964ff".into(),
                language: "en".into(),
                expectation,
                desktop: self.desktop.clone(),
            },
            move |event| {
                let _ = sink.send(event);
            },
        );
        (delivery, events)
    }

    /// The whole output step, then the read-back, as the output worker runs it.
    fn paste(
        &mut self,
        expected: &FieldSnapshot,
        text: &str,
    ) -> (Delivered, Receiver<DeliveryEvent>) {
        let (delivery, events) = self.delivery(Expectation::Field(expected.clone()));
        let mut delivered = deliver(
            &mut self.paster,
            &delivery,
            &Request {
                text,
                raw: text,
                copy_only: false,
                preferred: &["en-US".to_owned()],
            },
            &|| true,
        )
        .expect("delivered");
        delivery.deliver(DeliveryReport {
            outcome: delivered.outcome.clone(),
            text: delivered.text.clone(),
            raw_text: None,
            transcribe_ms: 0,
            paste_ms: 0,
        });
        if let Some(follow_up) = delivered.follow_up.take() {
            follow_up.spawn(delivery);
        }
        (delivered, events)
    }

    fn press(&self, key: Key, expected: &FieldSnapshot) -> KeyOutcome {
        let (done, outcome) = mpsc::channel();
        let valid_until = Instant::now() + Duration::from_secs(5);
        self.desktop.press(
            key,
            expected.clone(),
            valid_until,
            Ticket::new(),
            move |result| {
                let _ = done.send(result);
            },
        );
        outcome.recv_timeout(Duration::from_secs(6)).unwrap()
    }
}

fn confirmation(events: &Receiver<DeliveryEvent>, wait: Duration) -> Option<bool> {
    let deadline = Instant::now() + wait;
    while let Some(left) = deadline.checked_duration_since(Instant::now()) {
        match events.recv_timeout(left) {
            Ok(DeliveryEvent::Confirmed { verified }) => return Some(verified),
            Ok(DeliveryEvent::Report(_)) => {}
            Err(_) => return None,
        }
    }
    None
}

struct Cleanup(&'static str);

impl Drop for Cleanup {
    fn drop(&mut self) {
        let _ = Command::new("/usr/bin/osascript")
            .args(["-e", self.0])
            .output();
    }
}

#[test]
#[ignore = "drives TextEdit and Finder; run by hand (see the module doc)"]
fn watch_live_textedit_snapshot_paste_read_back_enter_undo_and_target_change() {
    let mut rig = Rig::new();
    osascript(
        r#"tell application "TextEdit"
            activate
            make new document
        end tell"#,
    );
    let _close = Cleanup(r#"tell application "TextEdit" to close front document saving no"#);
    settle();

    // Snapshot: the focused field is TextEdit's text area.
    let field = rig.focused();
    assert_eq!(field.role, "AXTextArea", "{field:?}");
    assert_eq!(field.bundle_id.as_deref(), Some("com.apple.TextEdit"));
    assert!(field.window.is_some(), "the window frame was read");

    // Paste, then the read-back upgrades it.
    let (delivered, events) = rig.paste(&field, "hello from the watch");
    assert_eq!(delivered.outcome, DeliveryOutcome::Sent(Verification::None));
    assert_eq!(
        confirmation(&events, Duration::from_secs(3)),
        Some(true),
        "the read-back saw the text"
    );
    let value = MacScreen.value(field.pid).unwrap_or_default();
    assert!(value.contains("hello from the watch"), "{value:?}");

    // Enter inserts a line break in the same field.
    assert_eq!(rig.press(Key::Enter, &field), KeyOutcome::Pressed);
    std::thread::sleep(Duration::from_millis(300));
    let value = MacScreen.value(field.pid).unwrap_or_default();
    assert!(
        value.ends_with('\n'),
        "Return went into the field: {value:?}"
    );

    // Undo takes a second paste back out.
    let (_, events) = rig.paste(&field, "second words");
    assert_eq!(confirmation(&events, Duration::from_secs(3)), Some(true));
    assert_eq!(rig.press(Key::Undo, &field), KeyOutcome::Pressed);
    std::thread::sleep(Duration::from_millis(400));
    let value = MacScreen.value(field.pid).unwrap_or_default();
    assert!(!value.contains("second words"), "undone: {value:?}");

    // Another application in front: nothing is pasted, nothing is pressed.
    osascript(r#"tell application "Finder" to activate"#);
    settle();
    let (delivery, _events) = rig.delivery(Expectation::Field(field.clone()));
    let moved = deliver(
        &mut rig.paster,
        &delivery,
        &Request {
            text: "should not land",
            raw: "should not land",
            copy_only: false,
            preferred: &["en-US".to_owned()],
        },
        &|| true,
    )
    .expect("delivered");
    assert_eq!(
        moved.outcome,
        DeliveryOutcome::Withheld(Withheld::TargetChanged)
    );
    assert_eq!(rig.press(Key::Enter, &field), KeyOutcome::TargetChanged);
    osascript(r#"tell application "TextEdit" to activate"#);
    settle();
    let value = MacScreen.value(field.pid).unwrap_or_default();
    assert!(!value.contains("should not land"), "{value:?}");
}

#[test]
#[ignore = "drives Terminal; run by hand (see the module doc)"]
fn watch_live_terminal_paste_is_final_and_enter_runs_the_command() {
    let mut rig = Rig::new();
    osascript(
        r#"tell application "Terminal"
            activate
            do script ""
        end tell"#,
    );
    let _close = Cleanup(r#"tell application "Terminal" to do script "exit" in front window"#);
    std::thread::sleep(Duration::from_millis(2000));

    let field = rig.focused();
    assert_eq!(
        field.bundle_id.as_deref(),
        Some("com.apple.Terminal"),
        "{field:?}"
    );
    assert!(field.is_terminal());

    // A terminal is never read back: sent is final and no confirmation follows.
    let (delivered, events) = rig.paste(&field, "echo hex-watch-live-marker");
    assert_eq!(
        delivered.outcome,
        DeliveryOutcome::Sent(Verification::Terminal)
    );
    assert_eq!(confirmation(&events, Duration::from_millis(600)), None);

    // Enter runs it: the marker shows twice, as typed and as printed.
    assert_eq!(rig.press(Key::Enter, &field), KeyOutcome::Pressed);
    std::thread::sleep(Duration::from_millis(1200));
    let history =
        osascript(r#"tell application "Terminal" to get history of selected tab of front window"#);
    assert!(
        history.matches("hex-watch-live-marker").count() >= 2,
        "the command ran: {history}"
    );
}
