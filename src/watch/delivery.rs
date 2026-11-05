//! How a watch recording's result travels back from Hex's dictation
//! pipeline. The `watch` thread hands the output worker a `WatchDelivery`
//! inside `TranscriptionTarget::Watch`; the worker pastes through the one
//! Paster, in order with every other dictation, and answers through it
//! once. A delivery dropped unanswered (a cancelled job, a worker that
//! stopped) still reports, so no recording waits forever.

use std::fmt;
use std::sync::{Arc, Mutex};

use super::suspect::SuspectReason;

/// What the output worker did with the transcript.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum DeliveryOutcome {
    /// ⌘V was sent at the current focus. Hex's paste is not a confirmation
    /// (`paste.rs`), and the read-back that could upgrade it lands later,
    /// so the watch reads `unverified`.
    Unverified,
    /// Labs › Copy dictation only: on the clipboard, not pasted.
    Copied,
    /// Not pasted: the text failed Wristper's text rules (today only an
    /// empty transcript).
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

type Reply = Box<dyn FnOnce(DeliveryReport) + Send>;

struct Slot {
    id: String,
    reply: Mutex<Option<Reply>>,
}

impl Drop for Slot {
    fn drop(&mut self) {
        if let Some(reply) = self.reply.get_mut().unwrap_or_else(|e| e.into_inner()).take() {
            reply(DeliveryReport::failed("dictation was dropped before it finished"));
        }
    }
}

/// The handle a watch recording travels with through the dictation worker.
#[derive(Clone)]
pub(crate) struct WatchDelivery(Arc<Slot>);

impl fmt::Debug for WatchDelivery {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("WatchDelivery").field(&self.0.id).finish()
    }
}

impl WatchDelivery {
    pub(crate) fn new(id: &str, reply: impl FnOnce(DeliveryReport) + Send + 'static) -> Self {
        Self(Arc::new(Slot {
            id: id.to_owned(),
            reply: Mutex::new(Some(Box::new(reply))),
        }))
    }

    pub(crate) fn id(&self) -> &str {
        &self.0.id
    }

    /// Answers the watch thread. The first report wins; later ones are
    /// dropped, so a clone that outlives the output cannot re-answer.
    pub(crate) fn deliver(&self, report: DeliveryReport) {
        let reply = self
            .0
            .reply
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .take();
        if let Some(reply) = reply {
            reply(report);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;

    use super::*;

    #[test]
    fn delivers_once_and_clones_share_the_slot() {
        let (sender, receiver) = mpsc::channel();
        let delivery = WatchDelivery::new("rec-1", move |report| {
            let _ = sender.send(report);
        });
        let other = delivery.clone();
        assert_eq!(delivery.id(), "rec-1");
        other.deliver(DeliveryReport {
            outcome: DeliveryOutcome::Unverified,
            text: "hello".into(),
            raw_text: None,
            transcribe_ms: 12,
            paste_ms: 3,
        });
        delivery.deliver(DeliveryReport::failed("second"));
        drop(delivery);
        drop(other);
        let report = receiver.recv().unwrap();
        assert_eq!(report.outcome, DeliveryOutcome::Unverified);
        assert_eq!(report.text, "hello");
        assert!(receiver.try_recv().is_err(), "only the first report arrives");
    }

    #[test]
    fn a_dropped_delivery_reports_failure() {
        let (sender, receiver) = mpsc::channel();
        let delivery = WatchDelivery::new("rec-2", move |report| {
            let _ = sender.send(report);
        });
        drop(delivery);
        let report = receiver.recv().unwrap();
        assert!(matches!(report.outcome, DeliveryOutcome::Failed(_)));
        assert_eq!(report.text, "");
    }
}
