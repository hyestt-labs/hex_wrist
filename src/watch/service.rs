//! The watch service: one `watch` thread that owns every piece of protocol
//! state (paired watches, the pairing window, the dedupe table, the nonce
//! cache, live recordings) and answers the relay's requests. The relay
//! thread only moves frames; the dictation worker only transcribes and
//! pastes; both talk to this thread through channels.

use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Duration;

use super::delivery::WatchDelivery;
use super::pairing::SecretStore;
use super::relay::RelayTiming;

/// How long `claim` waits for the relay's `claim_ok` or `claim_failed`.
pub(crate) const CLAIM_TIMEOUT: Duration = Duration::from_secs(10);

/// Where the service's files live and who this Mac says it is.
pub(crate) struct WatchConfig {
    /// Holds `relay-room`, `relay-secret`, `device-id`, `paired-devices.json`,
    /// and `delivered-ids.json`; Wristper's own files are reused unchanged.
    pub(crate) support_dir: PathBuf,
    /// Already resolved (`relay::resolve_base`).
    pub(crate) relay_base: String,
    /// What the watch shows for this Mac after pairing.
    pub(crate) mac_name: String,
    pub(crate) app_version: String,
    pub(crate) timing: RelayTiming,
}

/// Hands a decoded recording to the dictation pipeline. The app sends
/// `RecognitionControl::WatchTranscribe`; tests answer at once.
pub(crate) trait Dictation: Send {
    /// `Err` names why the recording was not accepted (the listener is not
    /// running, its queue is full); the delivery is then never answered by
    /// the pipeline.
    fn transcribe(&self, samples: Vec<f32>, delivery: WatchDelivery) -> Result<(), String>;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ClaimOk {
    pub(crate) device_id: String,
    pub(crate) device_name: String,
}

/// Why a pairing code was not claimed; `wire()` is the `error` Wristper's
/// UI maps to its texts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ClaimError {
    /// Not six digits.
    InvalidCode,
    RelayOffline,
    Timeout,
    CodeNotFound,
    TooManyAttempts,
    /// Another relay error, as the relay named it.
    Relay(String),
    /// The service is not running.
    Unavailable,
}

impl ClaimError {
    pub(crate) fn wire(&self) -> &str {
        match self {
            Self::InvalidCode => "invalid_code",
            Self::RelayOffline => "relay_offline",
            Self::Timeout => "timeout",
            Self::CodeNotFound => "code_not_found",
            Self::TooManyAttempts => "too_many_attempts",
            Self::Relay(error) => error,
            Self::Unavailable => "unavailable",
        }
    }

    pub(crate) fn message(&self) -> String {
        match self {
            Self::InvalidCode => "the pairing code is the six digits the watch shows".into(),
            Self::RelayOffline => "the relay is not connected; check the network and try again".into(),
            Self::Timeout => "the relay did not answer within 10 s".into(),
            Self::CodeNotFound => "no watch is showing that code; it may have expired".into(),
            Self::TooManyAttempts => "too many wrong codes; wait 10 minutes".into(),
            Self::Relay(error) => format!("the relay refused the code ({error})"),
            Self::Unavailable => "the Apple Watch service is not running".into(),
        }
    }
}

/// A snapshot for `hex status` and the window.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct WatchSnapshot {
    pub(crate) relay: super::relay::RelayState,
    pub(crate) devices: Vec<super::pairing::DeviceSummary>,
}

/// The handle every other thread keeps. Cloneable; `stop` ends the thread.
#[derive(Clone)]
pub(crate) struct WatchService {
    inbox: mpsc::Sender<Message>,
}

pub(crate) enum Message {
    Claim {
        code: String,
        reply: mpsc::SyncSender<Result<ClaimOk, ClaimError>>,
    },
    Snapshot {
        reply: mpsc::SyncSender<WatchSnapshot>,
    },
    Stop,
}

impl WatchService {
    /// Starts the relay and the `watch` thread. Fails only when the room
    /// files cannot be created.
    pub(crate) fn start(
        _config: WatchConfig,
        _dictation: Box<dyn Dictation>,
        _store: Box<dyn SecretStore + Send>,
    ) -> std::io::Result<Self> {
        Err(std::io::Error::other("the watch service is not implemented yet"))
    }

    /// Claims `code` through the relay, waiting at most `CLAIM_TIMEOUT`.
    pub(crate) fn claim(&self, code: &str) -> Result<ClaimOk, ClaimError> {
        if !super::frame::is_valid_claim_code(code) {
            return Err(ClaimError::InvalidCode);
        }
        let (reply, answer) = mpsc::sync_channel(1);
        if self
            .inbox
            .send(Message::Claim {
                code: code.to_owned(),
                reply,
            })
            .is_err()
        {
            return Err(ClaimError::Unavailable);
        }
        answer
            .recv_timeout(CLAIM_TIMEOUT + Duration::from_secs(1))
            .unwrap_or(Err(ClaimError::Timeout))
    }

    pub(crate) fn snapshot(&self) -> Option<WatchSnapshot> {
        let (reply, answer) = mpsc::sync_channel(1);
        self.inbox.send(Message::Snapshot { reply }).ok()?;
        answer.recv_timeout(Duration::from_secs(2)).ok()
    }

    pub(crate) fn stop(&self) {
        let _ = self.inbox.send(Message::Stop);
    }
}
