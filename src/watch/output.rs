//! What the output worker does with a watch recording's text, in
//! Wristper's order (`ReceiverService.transcribeAndPaste` and
//! `FieldPaster.paste`): Traditional Chinese, the suspect-text rules, the
//! gate against the field `/focus` locked, then ⌘V, a read-back that looks
//! for the text, and the clipboard restore the read-back times. Nothing
//! here touches AppKit: `Inserter` is the Paster and `Screen` is
//! Accessibility, so every decision runs under test.

use std::time::{Duration, Instant};

use super::delivery::{DeliveryOutcome, WatchDelivery};
use super::desktop::{Held, Screen};
use super::field::{Gate, gate, read_back_confirms};
use super::recording::Verification;
use super::{suspect, zh};

/// How long the read-back looks for the pasted text.
pub(crate) const READ_BACK_TIMEOUT: Duration = Duration::from_secs(1);
const READ_BACK_POLL: Duration = Duration::from_millis(30);
/// How long the clipboard keeps the transcript when the read-back cannot
/// confirm the paste (a terminal, or an app that shows nothing).
pub(crate) const RESTORE_GRACE: Duration = Duration::from_secs(4);

const NOT_TRUSTED: &str = "Accessibility permission is not granted \
    (enable it in System Settings › Privacy & Security › Accessibility)";

/// Puts the clipboard back after the given delay; zero means now.
pub(crate) type RestoreFn = Box<dyn FnOnce(Duration) + Send>;

pub(crate) struct Inserted {
    /// The text as it went in; the read-back looks for this.
    pub(crate) text: String,
    pub(crate) restore: Option<RestoreFn>,
}

/// The paste seam: `paste::Paster` in the app.
pub(crate) trait Inserter {
    /// Sends ⌘V with `text`, leaving the clipboard restore to the caller.
    fn insert(&mut self, text: &str, commit: &dyn Fn() -> bool) -> Result<Inserted, String>;
    /// Leaves `text` on the clipboard and pastes nothing.
    fn copy(&mut self, text: &str, commit: &dyn Fn() -> bool) -> Result<(), String>;
}

pub(crate) struct Request<'a> {
    /// The final text, after the Output pane's steps.
    pub(crate) text: &'a str,
    /// The corrected transcript before them.
    pub(crate) raw: &'a str,
    /// Labs › Copy dictation only: no gate, no paste.
    pub(crate) copy_only: bool,
    /// The system's preferred languages, for the script rules.
    pub(crate) preferred: &'a [String],
}

pub(crate) struct Delivered {
    pub(crate) outcome: DeliveryOutcome,
    /// The text after the Traditional Chinese conversion.
    pub(crate) text: String,
    /// What was heard, converted the same way; `None` when it equals `text`.
    pub(crate) raw: Option<String>,
    pub(crate) follow_up: Option<FollowUp>,
}

/// Runs after the watch has its answer: the read-back, then the restore.
/// Holds the keyboard lock until the read-back is done.
pub(crate) struct FollowUp {
    held: Held,
    desktop: super::desktop::Desktop,
    pid: i32,
    terminal: bool,
    text: String,
    before: Option<String>,
    restore: Option<RestoreFn>,
}

impl FollowUp {
    /// Starts the read-back on its own thread. Call it after
    /// `WatchDelivery::deliver`: a confirmation only refines a report that
    /// already went out.
    pub(crate) fn spawn(self, delivery: WatchDelivery) {
        let started = std::thread::Builder::new()
            .name("watch-readback".to_owned())
            .spawn(move || self.run(&delivery, std::thread::sleep));
        if let Err(error) = started {
            tracing::warn!(%error, "the paste read-back could not start");
        }
    }

    fn run(self, delivery: &WatchDelivery, sleep: impl FnMut(Duration)) {
        let Self {
            held,
            desktop,
            pid,
            terminal,
            text,
            before,
            restore,
        } = self;
        let confirmed = !terminal
            && wait_for_text(
                desktop.screen(),
                pid,
                &text,
                before.as_deref(),
                READ_BACK_TIMEOUT,
                sleep,
            );
        drop(held);
        if let Some(restore) = restore {
            restore(restore_delay(terminal, confirmed));
        }
        if !terminal {
            delivery.confirm(confirmed);
        }
    }
}

/// `ClipboardRestorePolicy.decide`: at once when the read-back saw the
/// text land, else after the grace period.
pub(crate) fn restore_delay(terminal: bool, read_back_confirmed: bool) -> Duration {
    if read_back_confirmed && !terminal {
        Duration::ZERO
    } else {
        RESTORE_GRACE
    }
}

/// Looks for `text` in the focused field of `pid` until `timeout`. The
/// field must differ from `before`, so text that was already there does not
/// confirm a paste that never landed.
pub(crate) fn wait_for_text(
    screen: &dyn Screen,
    pid: i32,
    text: &str,
    before: Option<&str>,
    timeout: Duration,
    mut sleep: impl FnMut(Duration),
) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        let value = screen.value(pid);
        if read_back_confirms(text, before, value.as_deref()) {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        sleep(READ_BACK_POLL);
    }
}

fn heard_as_traditional(text: &str, language: &str) -> String {
    if zh::applies(language) {
        zh::to_traditional(text)
    } else {
        text.to_owned()
    }
}

/// `Err` is a failure the watch reads as `failed`: the paste was not
/// attempted (no Accessibility) or did not complete.
pub(crate) fn deliver(
    inserter: &mut dyn Inserter,
    delivery: &WatchDelivery,
    request: &Request<'_>,
    commit: &dyn Fn() -> bool,
) -> Result<Delivered, String> {
    let job = delivery.job();
    let text = heard_as_traditional(request.text, &job.language);
    let heard = heard_as_traditional(request.raw, &job.language);
    let raw = (heard != text).then(|| heard.clone());
    let done = |outcome, follow_up| {
        Ok(Delivered {
            outcome,
            text: text.clone(),
            raw: raw.clone(),
            follow_up,
        })
    };
    if text.trim().is_empty() {
        return done(
            DeliveryOutcome::Suspect(suspect::SuspectReason::Empty),
            None,
        );
    }
    let preferred: Vec<&str> = request.preferred.iter().map(String::as_str).collect();
    let rules_text = if heard.trim().is_empty() {
        &text
    } else {
        &heard
    };
    if let Some(reason) =
        suspect::reason(rules_text, &job.language, Some(&job.language), &preferred)
    {
        return done(DeliveryOutcome::Suspect(reason), None);
    }
    if request.copy_only {
        inserter.copy(&text, commit)?;
        return done(DeliveryOutcome::Copied, None);
    }
    let desktop = &job.desktop;
    if !desktop.trusted() {
        return Err(NOT_TRUSTED.to_owned());
    }
    let held = desktop.lock();
    let screen = desktop.screen();
    let focused = screen.focused();
    match gate(&job.expectation, focused.as_ref(), screen.secure_input()) {
        Gate::Withhold(withheld) => done(DeliveryOutcome::Withheld(withheld), None),
        Gate::Copy => {
            inserter.copy(&text, commit)?;
            done(DeliveryOutcome::Copied, None)
        }
        Gate::Paste {
            target,
            verification,
        } => {
            let terminal = verification == Verification::Terminal;
            let before = if terminal {
                None
            } else {
                screen.value(target.pid)
            };
            let inserted = inserter.insert(&text, commit)?;
            let follow_up = FollowUp {
                held,
                desktop: desktop.clone(),
                pid: target.pid,
                terminal,
                text: inserted.text,
                before,
                restore: inserted.restore,
            };
            done(DeliveryOutcome::Sent(verification), Some(follow_up))
        }
    }
}

/// The output worker's one Paster, shared between its two callers.
#[cfg(target_os = "macos")]
pub(crate) struct Shared<'a>(pub(crate) &'a std::cell::RefCell<crate::paste::Paster>);

#[cfg(target_os = "macos")]
impl Inserter for Shared<'_> {
    fn insert(&mut self, text: &str, commit: &dyn Fn() -> bool) -> Result<Inserted, String> {
        self.0.borrow_mut().insert(text, commit)
    }

    fn copy(&mut self, text: &str, commit: &dyn Fn() -> bool) -> Result<(), String> {
        self.0.borrow_mut().copy(text, commit)
    }
}

#[cfg(target_os = "macos")]
impl Inserter for crate::paste::Paster {
    fn insert(&mut self, text: &str, commit: &dyn Fn() -> bool) -> Result<Inserted, String> {
        let sent = self
            .paste_deferred(text, || commit())
            .map_err(|error| error.to_string())?;
        Ok(Inserted {
            text: sent.inserted,
            restore: sent
                .restore
                .map(|handle| Box::new(move |delay| handle.restore_after(delay)) as RestoreFn),
        })
    }

    fn copy(&mut self, text: &str, commit: &dyn Fn() -> bool) -> Result<(), String> {
        self.paste(text, crate::paste::PasteMode::CopyOnly, || commit())
            .map_err(|error| error.to_string())
    }
}

#[cfg(test)]
mod tests;
