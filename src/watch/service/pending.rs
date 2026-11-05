//! Replies the `watch` thread owes the relay but cannot give yet, and the
//! `Reply` handle that answers one frame exactly once, sealed when the
//! request came inside an envelope.

use std::time::Instant;

use serde_json::Value;

use crate::watch::e2ee::{self, SharedKey};
use crate::watch::frame::response_frame_json;
use crate::watch::refusal::Refusal;
use crate::watch::relay::NotConnected;

/// Where response frames go: the relay in the app, a sink in tests.
pub(crate) trait Outbound: Send {
    fn send(&mut self, text: String) -> Result<(), NotConnected>;
    fn stop(self: Box<Self>);
}

/// What sealing a reply needs: the request's key, device, and nonce.
pub(crate) struct Seal {
    pub(crate) key: SharedKey,
    pub(crate) device_id: String,
    pub(crate) request_nonce_base64: String,
}

/// One frame's reply. Answering consumes it; dropping one unanswered is a
/// bug, logged (and asserted in debug builds) because the relay would wait
/// out its 60 s and the watch would read a 504.
pub(crate) struct Reply {
    frame_id: String,
    /// `METHOD /path`, for the log.
    label: String,
    seal: Option<Seal>,
    answered: bool,
}

impl Reply {
    pub(crate) fn new(frame_id: &str, method: &str, path: &str) -> Self {
        Self {
            frame_id: frame_id.to_owned(),
            label: format!("{method} {path}"),
            seal: None,
            answered: false,
        }
    }

    /// From here every answer travels sealed.
    pub(crate) fn seal_with(&mut self, seal: Seal) {
        self.seal = Some(seal);
    }

    pub(crate) fn seal(&self) -> Option<&Seal> {
        self.seal.as_ref()
    }

    pub(crate) fn answer(mut self, outbound: &mut dyn Outbound, status: u16, body: Value) {
        self.answered = true;
        let (status, body) = match &self.seal {
            Some(seal) => sealed(seal, status, &body),
            None => (status, body),
        };
        let frame = response_frame_json(&self.frame_id, status, &body);
        if outbound.send(frame).is_err() {
            tracing::info!(
                request = %self.label,
                status,
                "reply lost: the relay is not connected; the watch re-queries /status"
            );
        }
    }

    pub(crate) fn refuse(self, outbound: &mut dyn Outbound, refusal: Refusal) {
        self.answer(outbound, refusal.status(), refusal.body());
    }

    /// The service is stopping: the relay drops the frame anyway and the
    /// watch re-queries `/status`.
    pub(crate) fn abandon(mut self) {
        self.answered = true;
        tracing::info!(request = %self.label, "reply abandoned at shutdown");
    }
}

impl Drop for Reply {
    fn drop(&mut self) {
        if !self.answered && !std::thread::panicking() {
            tracing::error!(request = %self.label, "a watch reply was dropped unanswered");
        }
    }
}

/// A sealing failure answers 500 `e2ee-failed` in the clear, as Swift's
/// `sealed` does.
fn sealed(seal: &Seal, status: u16, body: &Value) -> (u16, Value) {
    let envelope = e2ee::random_nonce().and_then(|nonce| {
        e2ee::seal_response(
            body,
            status,
            &seal.key,
            &seal.device_id,
            &seal.request_nonce_base64,
            &nonce,
        )
    });
    match envelope {
        Some(envelope) => (status, envelope.json()),
        None => {
            let refusal = Refusal::E2eeFailed;
            (refusal.status(), refusal.body())
        }
    }
}

/// An authenticated recording request once its envelope and headers are
/// read: what the transcription needs, nothing the wire layer cares about.
pub(crate) struct AudioJob {
    pub(crate) device: String,
    pub(crate) id: String,
    pub(crate) language: String,
    pub(crate) audio: Vec<u8>,
    /// `focus: current`: a resend from the watch's unsent list, pasted
    /// wherever the cursor is now.
    pub(crate) at_cursor: bool,
    /// `focus: requested` and no `/focus` arrived in time: nothing pastes.
    pub(crate) snapshot_missing: bool,
}

/// A reply held for something that has not happened yet.
pub(crate) enum Waiting {
    /// Audio accepted: answered when the dictation worker delivers. At the
    /// deadline (50 s after receipt) it answers 200 `transcribing` so the
    /// relay's 60 s never turns into a 504; the watch then polls `/status`.
    Transcript {
        id: String,
        reply: Reply,
        deadline: Instant,
    },
    /// The same recording was sent again while the first copy is still in
    /// progress: answered with `duplicate_json()` when it finishes, or
    /// `duplicate` + `transcribing` at the deadline
    /// (`DUPLICATE_AUDIO_WAIT_SECONDS`).
    Duplicate {
        id: String,
        reply: Reply,
        deadline: Instant,
    },
    /// Audio with `focus: requested` arrived before its `/focus`: run when
    /// that `/focus` lands or at the deadline (`FOCUS_WAIT_SECONDS`).
    Focus {
        id: String,
        job: AudioJob,
        reply: Reply,
        deadline: Instant,
    },
    /// `/focus` while the `watch-ax` thread reads the focused field: answered
    /// when it lands, or `captured: false` at the deadline if the focused
    /// application does not answer.
    Capture {
        id: String,
        device: String,
        reply: Reply,
        deadline: Instant,
    },
}

impl Waiting {
    pub(crate) fn id(&self) -> &str {
        match self {
            Self::Transcript { id, .. }
            | Self::Duplicate { id, .. }
            | Self::Focus { id, .. }
            | Self::Capture { id, .. } => id,
        }
    }

    pub(crate) fn deadline(&self) -> Instant {
        match self {
            Self::Transcript { deadline, .. }
            | Self::Duplicate { deadline, .. }
            | Self::Focus { deadline, .. }
            | Self::Capture { deadline, .. } => *deadline,
        }
    }
}
