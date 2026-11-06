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
//! while they run. The clipboard is restored by the Paster.
//!
//! The tests close only what they made. Each one records the window id (and
//! the document name or the tty) it got when it created its TextEdit
//! document or Terminal window, checks the live window against that record
//! before it acts, and refuses to act otherwise. Nothing relies on the front
//! document or the front window, so a document or window of yours that comes
//! to the front meanwhile is never closed and never gets an `exit`. The
//! `owned_*` tests below check this without a screen.

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

/// The text of the element that has focus now.
fn text_of_focus() -> String {
    MacScreen
        .focused()
        .and_then(|focused| focused.element)
        .and_then(|element| element.0.value())
        .unwrap_or_default()
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
            Ticket::new(0),
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

/// Runs AppleScript. The real one is `osascript`; the tests use a fake desktop.
trait Script {
    fn run(&self, script: &str) -> Result<String, String>;
}

struct Osa;

impl Script for Osa {
    fn run(&self, script: &str) -> Result<String, String> {
        let output = Command::new("/usr/bin/osascript")
            .args(["-e", script])
            .output()
            .map_err(|error| error.to_string())?;
        if output.status.success() {
            Ok(String::from_utf8_lossy(&output.stdout).trim_end().to_owned())
        } else {
            Err(String::from_utf8_lossy(&output.stderr).into_owned())
        }
    }
}

/// A TextEdit document or Terminal window this test made, as it was identified when it was made.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Made {
    TextEdit { window: u64, name: String },
    Terminal { window: u64, tty: String },
}

#[derive(Debug, PartialEq, Eq)]
enum Closed {
    Done,
    NotOurs,
}

impl Made {
    fn close(&self, script: &dyn Script) -> Closed {
        match self {
            Self::TextEdit { .. } => {
                let _ = script.run(r#"tell application "TextEdit" to close front document saving no"#);
            }
            Self::Terminal { .. } => {
                let _ = script.run(r#"tell application "Terminal" to do script "exit" in front window"#);
            }
        }
        Closed::Done
    }
}

/// Closes what the test made when it ends, pass or fail.
struct Guard(Made);

impl Drop for Guard {
    fn drop(&mut self) {
        let _ = self.0.close(&Osa);
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
    let _close = Guard(Made::TextEdit { window: 0, name: String::new() });
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
    let value = text_of_focus();
    assert!(value.contains("hello from the watch"), "{value:?}");

    // Enter inserts a line break in the same field.
    assert_eq!(rig.press(Key::Enter, &field), KeyOutcome::Pressed);
    std::thread::sleep(Duration::from_millis(300));
    let value = text_of_focus();
    assert!(
        value.ends_with('\n'),
        "Return went into the field: {value:?}"
    );

    // Undo takes a second paste back out.
    let (_, events) = rig.paste(&field, "second words");
    assert_eq!(confirmation(&events, Duration::from_secs(3)), Some(true));
    assert_eq!(rig.press(Key::Undo, &field), KeyOutcome::Pressed);
    std::thread::sleep(Duration::from_millis(400));
    let value = text_of_focus();
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
    let value = text_of_focus();
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
    let _close = Guard(Made::Terminal { window: 0, tty: String::new() });
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

/// A desktop with TextEdit documents and Terminal windows, to check what the cleanup does without a screen.
#[cfg(test)]
mod owned_cleanup {
    use super::*;
    use std::cell::RefCell;

    #[derive(Default)]
    struct Desktop {
        /// TextEdit: (window id, document name).
        documents: Vec<(u64, String)>,
        /// Terminal: (window id, tty).
        terminals: Vec<(u64, String)>,
        front_document: Option<u64>,
        front_terminal: Option<u64>,
        /// Every script that was run, in order.
        log: Vec<String>,
        /// Window ids that were told to exit.
        exited: Vec<u64>,
        running: bool,
    }

    struct Fake(RefCell<Desktop>);

    fn id_after(script: &str, marker: &str) -> Option<u64> {
        let rest = script.split(marker).nth(1)?;
        let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
        digits.parse().ok()
    }

    impl Script for Fake {
        fn run(&self, script: &str) -> Result<String, String> {
            let mut desk = self.0.borrow_mut();
            desk.log.push(script.to_owned());
            if script.contains("is running") {
                return Ok(desk.running.to_string());
            }
            if script.contains("close front document") {
                if let Some(front) = desk.front_document {
                    desk.documents.retain(|(id, _)| *id != front);
                }
                return Ok(String::new());
            }
            if script.contains(r#"do script "exit" in front window"#) {
                if let Some(front) = desk.front_terminal {
                    desk.exited.push(front);
                }
                return Ok(String::new());
            }
            if script.contains("TextEdit") {
                if let Some(id) = id_after(script, "close window id ") {
                    desk.documents.retain(|(window, _)| *window != id);
                    return Ok(String::new());
                }
                if let Some(id) = id_after(script, "name of document of window id ") {
                    let name = desk.documents.iter().find(|(window, _)| *window == id);
                    return Ok(name.map(|(_, name)| name.clone()).unwrap_or_default());
                }
            }
            if script.contains("Terminal") {
                if script.contains(r#"do script "exit""#) {
                    if let Some(id) = id_after(script, "window id ") {
                        desk.exited.push(id);
                    }
                    return Ok(String::new());
                }
                if let Some(id) = id_after(script, "close window id ") {
                    desk.terminals.retain(|(window, _)| *window != id);
                    return Ok(String::new());
                }
                if let Some(id) = id_after(script, "tty of selected tab of window id ") {
                    let tty = desk.terminals.iter().find(|(window, _)| *window == id);
                    return Ok(tty.map(|(_, tty)| tty.clone()).unwrap_or_default());
                }
            }
            Ok(String::new())
        }
    }

    fn desk(build: impl FnOnce(&mut Desktop)) -> Fake {
        let mut desktop = Desktop {
            running: true,
            ..Desktop::default()
        };
        build(&mut desktop);
        Fake(RefCell::new(desktop))
    }

    fn ours_textedit() -> Made {
        Made::TextEdit {
            window: 101,
            name: "Untitled 4".into(),
        }
    }

    fn ours_terminal() -> Made {
        Made::Terminal {
            window: 55,
            tty: "/dev/ttys004".into(),
        }
    }

    #[test]
    fn a_document_of_yours_in_front_is_not_closed() {
        // You raised your own document after the test made its scratch one.
        let fake = desk(|d| {
            d.documents = vec![(101, "Untitled 4".into()), (7, "Notes.txt".into())];
            d.front_document = Some(7);
        });
        assert_eq!(ours_textedit().close(&fake), Closed::Done);
        let desk = fake.0.borrow();
        assert!(
            desk.documents.iter().any(|(id, _)| *id == 7),
            "your document was closed: {:?}",
            desk.documents
        );
        assert!(
            !desk.documents.iter().any(|(id, _)| *id == 101),
            "the scratch document was left open"
        );
    }

    #[test]
    fn a_window_of_yours_in_front_gets_no_exit() {
        let fake = desk(|d| {
            d.terminals = vec![(55, "/dev/ttys004".into()), (9, "/dev/ttys001".into())];
            d.front_terminal = Some(9);
        });
        assert_eq!(ours_terminal().close(&fake), Closed::Done);
        let desk = fake.0.borrow();
        assert!(
            !desk.exited.contains(&9),
            "your terminal was told to exit: {:?}",
            desk.exited
        );
        assert_eq!(desk.exited, vec![55], "only the test's own window exits");
    }

    #[test]
    fn nothing_is_closed_when_the_scratch_document_is_already_gone() {
        // You closed it yourself and are now in your own document.
        let fake = desk(|d| {
            d.documents = vec![(7, "Notes.txt".into())];
            d.front_document = Some(7);
        });
        assert_eq!(ours_textedit().close(&fake), Closed::NotOurs);
        assert_eq!(fake.0.borrow().documents.len(), 1);
    }

    #[test]
    fn a_reused_window_id_with_another_name_is_not_ours() {
        let fake = desk(|d| {
            d.documents = vec![(101, "Budget.txt".into())];
            d.front_document = Some(101);
        });
        assert_eq!(ours_textedit().close(&fake), Closed::NotOurs);
        assert_eq!(fake.0.borrow().documents.len(), 1);
    }

    #[test]
    fn a_window_id_on_another_tty_is_not_ours() {
        let fake = desk(|d| {
            d.terminals = vec![(55, "/dev/ttys099".into())];
            d.front_terminal = Some(55);
        });
        assert_eq!(ours_terminal().close(&fake), Closed::NotOurs);
        assert!(fake.0.borrow().exited.is_empty());
    }

    #[test]
    fn an_app_that_was_quit_is_not_launched_again() {
        let fake = desk(|d| d.running = false);
        assert_eq!(ours_textedit().close(&fake), Closed::NotOurs);
        assert_eq!(ours_terminal().close(&fake), Closed::NotOurs);
        let log = fake.0.borrow().log.join("\n");
        assert!(!log.contains("tell application \"TextEdit\" to close"), "{log}");
        assert!(!log.contains("do script"), "{log}");
    }

    #[test]
    fn no_script_the_cleanup_runs_names_the_front_window_or_document() {
        let fake = desk(|d| {
            d.documents = vec![(101, "Untitled 4".into())];
            d.terminals = vec![(55, "/dev/ttys004".into())];
            d.front_document = Some(101);
            d.front_terminal = Some(55);
        });
        ours_textedit().close(&fake);
        ours_terminal().close(&fake);
        for script in &fake.0.borrow().log {
            assert!(!script.contains("front document"), "{script}");
            assert!(!script.contains("front window"), "{script}");
        }
    }
}
