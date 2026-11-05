//! How a watch recording's result travels back from Hex's dictation
//! pipeline. The `watch` thread hands the output worker a `WatchDelivery`
//! inside `TranscriptionTarget::Watch`; the worker pastes through the one
//! Paster, in order with every other dictation, and answers through it
//! once. A read-back that finishes later confirms through the same handle.
//! A delivery dropped unanswered (a cancelled job, a worker that stopped)
//! still reports, so no recording waits forever.

use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use super::field::{Expectation, Withheld};
use super::recording::Verification;
use super::suspect::SuspectReason;

/// What the output worker did with the transcript.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum DeliveryOutcome {
    /// ⌘V was sent. `Terminal` is final (`pasted`); `None` reads
    /// `unverified` until the read-back confirms it, if it does.
    Sent(Verification),
    /// Labs › Copy dictation only, or no focused element at the current
    /// cursor: on the clipboard, not pasted.
    Copied,
    /// Not pasted; the record keeps the text for the watch and History.
    Withheld(Withheld),
    /// Not pasted: the text failed Wristper's text rules.
    Suspect(SuspectReason),
    /// Inference, processing, or the paste failed, or the job was
    /// cancelled or dropped.
    Failed(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DeliveryReport {
    pub(crate) outcome: DeliveryOutcome,
    /// The pasted text (after the Output pane's steps); empty on failure.
    pub(crate) text: String,
    /// The transcript as heard, when it differs from `text`.
    pub(crate) raw_text: Option<String>,
    pub(crate) transcribe_ms: u64,
    pub(crate) paste_ms: u64,
}

impl DeliveryReport {
    pub(crate) fn failed(error: impl Into<String>) -> Self {
        Self {
            outcome: DeliveryOutcome::Failed(error.into()),
            text: String::new(),
            raw_text: None,
            transcribe_ms: 0,
            paste_ms: 0,
        }
    }
}

/// What the `watch` thread hears back.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum DeliveryEvent {
    Report(DeliveryReport),
    /// The read-back after a `Sent(Verification::None)`: the field holds
    /// the text (`true`), or a second did not show it.
    Confirmed {
        verified: bool,
    },
}

/// What the output worker needs to know about the recording besides its
/// audio.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Job {
    pub(crate) id: String,
    /// The language the watch asked for (`zh`, `en`, `auto`, or empty).
    pub(crate) language: String,
    pub(crate) expectation: Expectation,
}

type Reply = Arc<dyn Fn(DeliveryEvent) + Send + Sync>;

struct Slot {
    job: Job,
    reply: Reply,
    reported: AtomicBool,
    confirmed: AtomicBool,
}

impl Drop for Slot {
    fn drop(&mut self) {
        if !self.reported.swap(true, Ordering::AcqRel) {
            (self.reply)(DeliveryEvent::Report(DeliveryReport::failed(
                "dictation was dropped before it finished",
            )));
        }
    }
}

/// The handle a watch recording travels with through the dictation worker.
#[derive(Clone)]
pub(crate) struct WatchDelivery(Arc<Slot>);

impl fmt::Debug for WatchDelivery {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("WatchDelivery")
            .field(&self.0.job.id)
            .finish()
    }
}

impl WatchDelivery {
    pub(crate) fn new(job: Job, reply: impl Fn(DeliveryEvent) + Send + Sync + 'static) -> Self {
        Self(Arc::new(Slot {
            job,
            reply: Arc::new(reply),
            reported: AtomicBool::new(false),
            confirmed: AtomicBool::new(false),
        }))
    }

    pub(crate) fn id(&self) -> &str {
        &self.0.job.id
    }

    pub(crate) fn job(&self) -> &Job {
        &self.0.job
    }

    /// Answers the watch thread. The first report wins; later ones are
    /// dropped, so a clone that outlives the output cannot re-answer.
    pub(crate) fn deliver(&self, report: DeliveryReport) {
        if !self.0.reported.swap(true, Ordering::AcqRel) {
            (self.0.reply)(DeliveryEvent::Report(report));
        }
    }

    /// The read-back's verdict, once, and only after the report it refines.
    pub(crate) fn confirm(&self, verified: bool) {
        if self.0.reported.load(Ordering::Acquire) && !self.0.confirmed.swap(true, Ordering::AcqRel)
        {
            (self.0.reply)(DeliveryEvent::Confirmed { verified });
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;

    use super::*;

    fn job(id: &str) -> Job {
        Job {
            id: id.into(),
            language: "en".into(),
            expectation: Expectation::Current,
        }
    }

    fn sent(text: &str) -> DeliveryReport {
        DeliveryReport {
            outcome: DeliveryOutcome::Sent(Verification::None),
            text: text.into(),
            raw_text: None,
            transcribe_ms: 12,
            paste_ms: 3,
        }
    }

    #[test]
    fn delivers_once_and_clones_share_the_slot() {
        let (sender, receiver) = mpsc::channel();
        let delivery = WatchDelivery::new(job("rec-1"), move |event| {
            let _ = sender.send(event);
        });
        let other = delivery.clone();
        assert_eq!(delivery.id(), "rec-1");
        assert_eq!(delivery.job().language, "en");
        other.deliver(sent("hello"));
        delivery.deliver(DeliveryReport::failed("second"));
        drop(delivery);
        drop(other);
        assert_eq!(
            receiver.recv().unwrap(),
            DeliveryEvent::Report(sent("hello"))
        );
        assert!(
            receiver.try_recv().is_err(),
            "only the first report arrives"
        );
    }

    #[test]
    fn a_dropped_delivery_reports_failure() {
        let (sender, receiver) = mpsc::channel();
        let delivery = WatchDelivery::new(job("rec-2"), move |event| {
            let _ = sender.send(event);
        });
        drop(delivery);
        let DeliveryEvent::Report(report) = receiver.recv().unwrap() else {
            panic!("a report");
        };
        assert!(matches!(report.outcome, DeliveryOutcome::Failed(_)));
        assert_eq!(report.text, "");
    }

    #[test]
    fn confirmation_follows_the_report_and_arrives_once() {
        let (sender, receiver) = mpsc::channel();
        let delivery = WatchDelivery::new(job("rec-3"), move |event| {
            let _ = sender.send(event);
        });
        // Before any report it is meaningless and dropped.
        delivery.confirm(true);
        delivery.deliver(sent("hi"));
        delivery.clone().confirm(true);
        delivery.confirm(false);
        drop(delivery);
        let events: Vec<_> = receiver.try_iter().collect();
        assert_eq!(
            events,
            [
                DeliveryEvent::Report(sent("hi")),
                DeliveryEvent::Confirmed { verified: true },
            ]
        );
    }
}
