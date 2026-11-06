//! The watch service: one `watch` thread that owns every piece of protocol
//! state (paired watches, the pairing window, the dedupe table, the nonce
//! cache, live recordings) and answers the relay's requests. The relay
//! thread only moves frames; the dictation worker only transcribes and
//! pastes; both talk to this thread through channels.
//!
//! - `state`: `Core`, the thread's state, and the non-request messages.
//! - `routes`: dispatch, authentication, and the handlers that answer at once.
//! - `envelope`: `/e2ee`, `/e2ee/query`, and the recording path.
//! - `pending`: replies held for a delivery, a duplicate, or a `/focus`.

mod capsule;
mod envelope;
#[cfg(test)]
mod integration;
mod keys;
mod pending;
mod routes;
mod state;
#[cfg(test)]
mod tests;

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::{Duration, Instant};

use super::delivery::{DeliveryReport, WatchDelivery};
use super::desktop::Desktop;
use super::field;
use super::pairing::{self, PairedDevices, SecretStore};
use super::relay::{NotConnected, Relay, RelayConfig, RelayEvent, RelayTiming};
#[cfg(target_os = "macos")]
use crate::recognition::RecognitionControl;
use state::{Core, Flow, Identity, Links, Now};

/// How long a claim may take from `claim`'s call to the relay's
/// `claim_ok` or `claim_failed`, time in the inbox included.
pub(crate) const CLAIM_TIMEOUT: Duration = Duration::from_secs(10);
/// How long `claim` waits for the `watch` thread, which answers by the
/// claim's deadline, `CLAIM_TIMEOUT` after the call.
pub(crate) const CLAIM_WAIT: Duration = CLAIM_TIMEOUT.saturating_add(Duration::from_secs(1));
/// The loop wakes at least this often, so bookkeeping never waits on a frame.
const TICK: Duration = Duration::from_secs(1);
const DEVICE_ID_FILE: &str = "device-id";
const DELIVERED_FILE: &str = "delivered-ids.json";

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

/// The speech model as `/health` names it (Swift's `ReceiverModels`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ModelInfo {
    /// `missing` (nothing installed), `cold` (installed), or `warm` (loaded).
    pub(crate) state: &'static str,
    pub(crate) id: String,
    pub(crate) name: String,
}

/// Hands a decoded recording to the dictation pipeline. The app sends
/// `RecognitionControl::WatchTranscribe`; tests answer at once.
pub(crate) trait Dictation: Send {
    /// `Err` names why the recording was not accepted (the listener is not
    /// running, its queue is full); the delivery is then never answered by
    /// the pipeline.
    fn transcribe(&self, samples: Vec<f32>, delivery: WatchDelivery) -> Result<(), String>;

    fn model(&self) -> ModelInfo;
}

/// The app's `Dictation`: the listener's control queue (macOS only, like
/// the listener itself).
#[cfg(target_os = "macos")]
pub(crate) struct ListenerDictation {
    controls: mpsc::SyncSender<RecognitionControl>,
    /// Whether the listener can run yet; before that nothing reads the queue.
    ready: fn() -> Result<(), String>,
}

#[cfg(target_os = "macos")]
impl ListenerDictation {
    pub(crate) fn new(
        controls: mpsc::SyncSender<RecognitionControl>,
        ready: fn() -> Result<(), String>,
    ) -> Self {
        Self { controls, ready }
    }
}

#[cfg(target_os = "macos")]
impl Dictation for ListenerDictation {
    fn transcribe(&self, samples: Vec<f32>, delivery: WatchDelivery) -> Result<(), String> {
        // The listener starts once Accessibility and the model are ready, not
        // the microphone. Until then a queued recording would wait for the
        // watch's timeout; refuse it with the reason instead.
        (self.ready)()?;
        self.controls
            .try_send(RecognitionControl::WatchTranscribe { samples, delivery })
            .map_err(|error| match error {
                mpsc::TrySendError::Full(_) => "the dictation queue is full".to_owned(),
                mpsc::TrySendError::Disconnected(_) => "the listener is not running".to_owned(),
            })
    }

    /// The saved selection; whether the engine holds it is not asked here,
    /// so an installed model reads `cold`.
    fn model(&self) -> ModelInfo {
        use crate::transcription::models::{definition, is_installed};
        let (_, selection) = crate::app_settings::transcription_selection();
        let model = definition(selection.model);
        ModelInfo {
            state: if is_installed(model, &selection.language) {
                "cold"
            } else {
                "missing"
            },
            id: selection.model.as_str().to_owned(),
            name: model.name.to_owned(),
        }
    }
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
            Self::RelayOffline => {
                "the relay is not connected; check the network and try again".into()
            }
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
    /// What the watch shows for this Mac after pairing.
    pub(crate) mac_name: String,
    /// Another Mac took this relay room and this one has not served it
    /// since: its pairing now reaches the other Mac.
    pub(crate) replaced: bool,
    pub(crate) devices: Vec<super::pairing::DeviceSummary>,
    /// Which pairing request spent which claim's approval, so the window
    /// settles a claim on the watch that answered it, not on any watch that
    /// paired while it waited.
    pub(crate) admissions: Vec<super::pairing::requests::Admission>,
}

/// The handle every other thread keeps. Cloneable; `stop` ends the thread.
#[derive(Clone)]
pub(crate) struct WatchService {
    inbox: mpsc::Sender<Message>,
}

pub(crate) enum Message {
    Claim {
        code: String,
        /// Which claim this is, so the pairing it earns names it
        /// (`Admission::claim_id`).
        claim_id: u64,
        reply: mpsc::SyncSender<Result<ClaimOk, ClaimError>>,
        /// When the caller stops waiting, less the margin; a claim still in
        /// the inbox then is never sent.
        deadline: Instant,
    },
    Snapshot {
        reply: mpsc::SyncSender<WatchSnapshot>,
    },
    /// Removes a paired watch. `Ok(false)`: it was not paired. `Err` names
    /// why the list was not written; the watch is denied either way.
    Revoke {
        id: String,
        reply: mpsc::SyncSender<Result<bool, String>>,
    },
    Stop,
    Relay(RelayEvent),
    /// The `watch-ax` thread read the focused field for `/focus` of `id`.
    Captured {
        id: String,
        field: Option<field::FieldSnapshot>,
    },
    /// The `watch-ax` thread pressed `key` for `id`, or says why not.
    Keyed {
        id: String,
        key: super::desktop::Key,
        /// Which request this answers; an answer to a request already
        /// given up on frees nothing of a later one's.
        op: u64,
        outcome: super::desktop::KeyOutcome,
    },
    /// The dictation worker finished recording `id`.
    Delivered {
        id: String,
        report: DeliveryReport,
    },
    /// The read-back after a paste of `id` found the text, or did not.
    Confirmed {
        id: String,
        verified: bool,
    },
}

impl WatchService {
    /// Starts the relay and the `watch` thread. Fails only when the room
    /// files cannot be created.
    pub(crate) fn start(
        config: WatchConfig,
        dictation: Box<dyn Dictation>,
        store: Box<dyn SecretStore + Send>,
        desktop: Desktop,
    ) -> std::io::Result<Self> {
        let room = super::relay::read_or_create_room(&config.support_dir)?;
        let mac_id = read_or_create_device_id(&config.support_dir.join(DEVICE_ID_FILE))?;
        let (inbox, messages) = mpsc::channel();
        let (events, relay_events) = mpsc::channel();
        let relay = Relay::start(
            RelayConfig {
                base: config.relay_base,
                room,
                timing: config.timing,
            },
            events,
        );
        let forward = inbox.clone();
        std::thread::Builder::new()
            .name("watch-relay-events".to_owned())
            .spawn(move || {
                for event in relay_events {
                    if forward.send(Message::Relay(event)).is_err() {
                        break;
                    }
                }
            })
            .map_err(std::io::Error::other)?;
        let identity = Identity {
            mac_id,
            mac_name: config.mac_name,
            app_version: config.app_version,
        };
        let support_dir = config.support_dir;
        let core_inbox = inbox.clone();
        // `PairedDevices` holds the store, which is not `Send` once boxed
        // without the bound, so the thread builds the core itself.
        std::thread::Builder::new()
            .name("watch".to_owned())
            .spawn(move || {
                let paired = PairedDevices::load(&support_dir.join(pairing::FILE_NAME), store);
                for note in &paired.notes {
                    tracing::info!(note, "paired watches");
                }
                let delivered_path = support_dir.join(DELIVERED_FILE);
                let delivered = super::delivered::load(&delivered_path);
                let core = Core::new(
                    identity,
                    paired,
                    delivered,
                    delivered_path,
                    Links {
                        outbound: Box::new(relay),
                        dictation,
                        desktop,
                    },
                    core_inbox,
                );
                run(core, messages);
            })
            .map_err(std::io::Error::other)?;
        Ok(Self { inbox })
    }

    /// Claims `code` through the relay, waiting at most `CLAIM_TIMEOUT`.
    /// `claim_id` numbers the claim (`requests::NO_CLAIM` for one no window
    /// made); the pairing it approves carries it.
    pub(crate) fn claim(&self, code: &str, claim_id: u64) -> Result<ClaimOk, ClaimError> {
        if !super::frame::is_valid_claim_code(code) {
            return Err(ClaimError::InvalidCode);
        }
        let (reply, answer) = mpsc::sync_channel(1);
        if self
            .inbox
            .send(Message::Claim {
                code: code.to_owned(),
                claim_id,
                reply,
                deadline: Instant::now() + CLAIM_TIMEOUT,
            })
            .is_err()
        {
            return Err(ClaimError::Unavailable);
        }
        answer
            .recv_timeout(CLAIM_WAIT)
            .unwrap_or(Err(ClaimError::Timeout))
    }

    pub(crate) fn snapshot(&self) -> Option<WatchSnapshot> {
        let (reply, answer) = mpsc::sync_channel(1);
        self.inbox.send(Message::Snapshot { reply }).ok()?;
        answer.recv_timeout(Duration::from_secs(2)).ok()
    }

    /// Removes the paired watch `id` (Settings' Remove).
    pub(crate) fn revoke(&self, id: &str) -> Result<bool, String> {
        let (reply, answer) = mpsc::sync_channel(1);
        self.inbox
            .send(Message::Revoke {
                id: id.to_owned(),
                reply,
            })
            .map_err(|_| "the Apple Watch service is not running".to_owned())?;
        answer
            .recv_timeout(Duration::from_secs(5))
            .unwrap_or_else(|_| Err("the Apple Watch service did not answer".to_owned()))
    }

    pub(crate) fn stop(&self) {
        let _ = self.inbox.send(Message::Stop);
    }
}

/// The `watch` thread: one inbox, woken by the nearest deadline.
fn run(mut core: Core, messages: mpsc::Receiver<Message>) {
    core.publish_view();
    serve(&mut core, &messages, Now::current);
    core.shutdown();
}

/// Handles messages until `Stop` or a closed inbox. A due deadline is
/// served before the next message, so a busy inbox never holds one back.
fn serve(core: &mut Core, messages: &mpsc::Receiver<Message>, mut clock: impl FnMut() -> Now) {
    let mut ticked = clock().at;
    loop {
        let now = clock();
        let due = core
            .next_deadline()
            .map_or(ticked + TICK, |deadline| deadline.min(ticked + TICK));
        if due <= now.at {
            core.tick(now);
            ticked = now.at;
            continue;
        }
        match messages.recv_timeout(due - now.at) {
            Ok(message) => {
                if let Flow::Exit = core.handle(message, clock()) {
                    return;
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }
    }
}

impl pending::Outbound for Relay {
    fn send(&mut self, text: String) -> Result<(), NotConnected> {
        Relay::send(self, text)
    }

    fn stop(self: Box<Self>) {
        Relay::stop(*self);
    }
}

/// `device-id`: this Mac's id as the watch stores it, a lower-case UUID
/// (`EmbeddedReceiver.readOrCreate`). A file Wristper wrote is kept.
fn read_or_create_device_id(path: &Path) -> std::io::Result<String> {
    if let Ok(text) = std::fs::read_to_string(path) {
        let trimmed = text.trim();
        if is_uuid(trimmed) {
            return Ok(trimmed.to_owned());
        }
    }
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).map_err(std::io::Error::other)?;
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let hex: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
    let id = format!(
        "{}-{}-{}-{}-{}",
        &hex[..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..]
    );
    super::write_owner_only(path, format!("{id}\n").as_bytes())?;
    Ok(id)
}

/// `UUID(uuidString:)` accepts either case; the file keeps what it holds.
fn is_uuid(text: &str) -> bool {
    text.len() == 36
        && text.bytes().enumerate().all(|(index, byte)| match index {
            8 | 13 | 18 | 23 => byte == b'-',
            _ => byte.is_ascii_hexdigit(),
        })
}
