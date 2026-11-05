//! `Core`: everything the `watch` thread owns, and the inbox messages that
//! are not requests (relay events, claims, deliveries, the clock).

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::mpsc::{self, SyncSender};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

use super::pending::{Outbound, Reply, Waiting};
use super::{ClaimError, ClaimOk, Dictation, Message, WatchSnapshot};
use crate::watch::Timestamp;
use crate::watch::delivered::DeliveredIds;
use crate::watch::delivery::{DeliveryOutcome, DeliveryReport};
use crate::watch::e2ee::NonceCache;
use crate::watch::field::Withheld;
use crate::watch::frame::{Inbound, claim_frame};
use crate::watch::pairing::PairedDevices;
use crate::watch::pairing::requests::PairingRequests;
use crate::watch::recording::{PasteMarks, Record, State, Verification};
use crate::watch::refusal::Refusal;
use crate::watch::relay::{RelayEvent, RelayState};

/// Live records older than this are forgotten; the dedupe table keeps
/// answering `/status` for them.
pub(crate) const RECORD_RETENTION: Duration = Duration::from_secs(86_400);
pub(crate) const MAX_LIVE_RECORDS: usize = 500;
/// `/e2ee` is answered `transcribing` after this long at the latest.
pub(crate) const TRANSCRIPT_WAIT: Duration = Duration::from_secs(50);

/// The moment a message is handled, on both clocks: `at` for deadlines,
/// `wall` for timestamps and freshness. Tests pass their own.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Now {
    pub(crate) at: Instant,
    pub(crate) wall: SystemTime,
}

impl Now {
    pub(crate) fn current() -> Self {
        Self {
            at: Instant::now(),
            wall: SystemTime::now(),
        }
    }

    pub(crate) fn epoch(&self) -> f64 {
        self.wall
            .duration_since(UNIX_EPOCH)
            .unwrap_or(Duration::ZERO)
            .as_secs_f64()
    }
}

/// Who this Mac says it is.
pub(crate) struct Identity {
    pub(crate) mac_id: String,
    pub(crate) mac_name: String,
    pub(crate) app_version: String,
}

/// A recording plus what `/status` does not show.
pub(crate) struct LiveRecord {
    pub(crate) record: Record,
    /// When the audio was accepted; `receive_to_paste_ms` counts from here.
    pub(crate) received_at: Option<Instant>,
    pub(crate) touched: Instant,
}

impl LiveRecord {
    pub(crate) fn new(id: &str, owner: &str, now: Now) -> Self {
        Self {
            record: Record {
                id: id.to_owned(),
                state: State::AwaitingAudio,
                paste: PasteMarks::default(),
                language: String::new(),
                text: String::new(),
                raw_text: None,
                updated_at: Timestamp::string(now.wall),
                completed_at: None,
                transcribe_ms: None,
                paste_ms: None,
                receive_to_paste_ms: None,
                reason: None,
                reset: None,
                paste_reason: None,
                error: None,
                truncated: false,
                owner: Some(owner.to_owned()),
            },
            received_at: None,
            touched: now.at,
        }
    }
}

struct PendingClaim {
    code: String,
    reply: SyncSender<Result<ClaimOk, ClaimError>>,
    deadline: Instant,
}

pub(crate) enum Flow {
    Continue,
    Exit,
}

pub(crate) struct Core {
    pub(super) identity: Identity,
    pub(super) paired: PairedDevices,
    pub(super) requests: PairingRequests,
    pub(super) delivered: DeliveredIds,
    pub(super) delivered_path: PathBuf,
    pub(super) nonces: NonceCache,
    pub(super) records: BTreeMap<String, LiveRecord>,
    pub(super) waiting: Vec<Waiting>,
    claims: Vec<PendingClaim>,
    pub(super) relay_state: RelayState,
    pub(super) outbound: Box<dyn Outbound>,
    pub(super) dictation: Box<dyn Dictation>,
    /// Deliveries come back through here.
    pub(super) inbox: mpsc::Sender<Message>,
}

impl Core {
    pub(crate) fn new(
        identity: Identity,
        paired: PairedDevices,
        delivered: DeliveredIds,
        delivered_path: PathBuf,
        outbound: Box<dyn Outbound>,
        dictation: Box<dyn Dictation>,
        inbox: mpsc::Sender<Message>,
    ) -> Self {
        Self {
            identity,
            paired,
            requests: PairingRequests::new(),
            delivered,
            delivered_path,
            nonces: NonceCache::default(),
            records: BTreeMap::new(),
            waiting: Vec::new(),
            claims: Vec::new(),
            relay_state: RelayState::Offline,
            outbound,
            dictation,
            inbox,
        }
    }

    pub(crate) fn handle(&mut self, message: Message, now: Now) -> Flow {
        match message {
            Message::Relay(event) => self.on_relay(event, now),
            Message::Delivered { id, report } => self.on_delivered(&id, report, now),
            Message::Confirmed { id, verified } => self.on_confirmed(&id, verified, now),
            Message::Claim {
                code,
                reply,
                deadline,
            } => self.on_claim(code, reply, deadline, now),
            Message::Snapshot { reply } => {
                let _ = reply.try_send(WatchSnapshot {
                    relay: self.relay_state,
                    devices: self.paired.summary(),
                });
            }
            Message::Stop => return Flow::Exit,
        }
        Flow::Continue
    }

    pub(crate) fn shutdown(self) {
        for waiting in self.waiting {
            match waiting {
                Waiting::Transcript { reply, .. }
                | Waiting::Duplicate { reply, .. }
                | Waiting::Focus { reply, .. } => reply.abandon(),
            }
        }
        self.outbound.stop();
    }

    /// The nearest moment `tick` has something to do.
    pub(crate) fn next_deadline(&self) -> Option<Instant> {
        self.waiting
            .iter()
            .map(Waiting::deadline)
            .chain(self.claims.iter().map(|claim| claim.deadline))
            .min()
    }

    pub(crate) fn tick(&mut self, now: Now) {
        let (due, later): (Vec<_>, Vec<_>) = std::mem::take(&mut self.claims)
            .into_iter()
            .partition(|claim| claim.deadline <= now.at);
        self.claims = later;
        for claim in due {
            let _ = claim.reply.try_send(Err(ClaimError::Timeout));
        }
        let (due, later): (Vec<_>, Vec<_>) = std::mem::take(&mut self.waiting)
            .into_iter()
            .partition(|waiting| waiting.deadline() <= now.at);
        self.waiting = later;
        for waiting in due {
            match waiting {
                Waiting::Transcript { reply, .. } => {
                    self.answer(reply, 200, json!({"ok": true, "state": "transcribing"}));
                }
                Waiting::Duplicate { reply, .. } => self.answer(
                    reply,
                    200,
                    json!({"ok": true, "duplicate": true, "state": "transcribing"}),
                ),
                Waiting::Focus { id, job, reply, .. } => {
                    tracing::info!(
                        id,
                        "no /focus arrived for this recording; pasting at the cursor"
                    );
                    self.accept_audio(job, reply, false, now);
                }
            }
        }
        self.prune_records(now);
    }

    fn prune_records(&mut self, now: Now) {
        let cutoff = now.at.checked_sub(RECORD_RETENTION);
        self.records.retain(|_, live| {
            live.record.state == State::Transcribing
                || cutoff.is_none_or(|cutoff| live.touched > cutoff)
        });
        while self.records.len() > MAX_LIVE_RECORDS {
            let oldest = self
                .records
                .iter()
                .filter(|(_, live)| live.record.state != State::Transcribing)
                .min_by_key(|(_, live)| live.touched)
                .map(|(id, _)| id.clone());
            match oldest {
                Some(id) => {
                    self.records.remove(&id);
                }
                None => break,
            }
        }
    }

    pub(super) fn answer(&mut self, reply: Reply, status: u16, body: Value) {
        reply.answer(self.outbound.as_mut(), status, body);
    }

    pub(super) fn refuse(&mut self, reply: Reply, refusal: Refusal) {
        reply.refuse(self.outbound.as_mut(), refusal);
    }

    fn on_relay(&mut self, event: RelayEvent, now: Now) {
        match event {
            RelayEvent::State(state) => {
                self.relay_state = state;
                if state == RelayState::Offline {
                    for claim in self.claims.drain(..) {
                        let _ = claim.reply.try_send(Err(ClaimError::RelayOffline));
                    }
                }
            }
            RelayEvent::Replaced => {
                tracing::warn!("another Mac took this relay room; the watch now reaches it");
            }
            RelayEvent::Inbound(Inbound::Request(head, body)) => self.on_request(head, body, now),
            RelayEvent::Inbound(Inbound::Rejected { id, status, error }) => {
                let reply = Reply::new(&id, "?", "?");
                self.answer(reply, status, json!({"ok": false, "error": error}));
            }
            RelayEvent::Inbound(Inbound::ClaimOk {
                code,
                device_id,
                device_name,
            }) => {
                let Some(claim) = self.take_claim(&code) else {
                    return;
                };
                if claim.deadline <= now.at {
                    tracing::info!(
                        code,
                        "a pairing claim was answered after its caller gave up"
                    );
                    let _ = claim.reply.try_send(Err(ClaimError::Timeout));
                    return;
                }
                self.requests.approve(&device_id, now.at);
                let _ = claim.reply.try_send(Ok(ClaimOk {
                    device_id,
                    device_name,
                }));
            }
            RelayEvent::Inbound(Inbound::ClaimFailed { code, error }) => {
                let Some(claim) = self.take_claim(&code) else {
                    return;
                };
                let error = match error.as_str() {
                    "code_not_found" => ClaimError::CodeNotFound,
                    "too_many_attempts" => ClaimError::TooManyAttempts,
                    other => ClaimError::Relay(other.to_owned()),
                };
                let _ = claim.reply.try_send(Err(error));
            }
            RelayEvent::Inbound(Inbound::Pong) => {}
        }
    }

    fn take_claim(&mut self, code: &str) -> Option<PendingClaim> {
        let position = self.claims.iter().position(|claim| claim.code == code);
        if position.is_none() {
            tracing::info!(code, "the relay answered a claim nobody is waiting for");
        }
        position.map(|position| self.claims.remove(position))
    }

    /// Sends the claim and waits for the relay until `deadline`, the
    /// caller's. A claim that sat in the inbox past it is dropped unsent,
    /// so no watch is approved for a caller that already gave up.
    fn on_claim(
        &mut self,
        code: String,
        reply: SyncSender<Result<ClaimOk, ClaimError>>,
        deadline: Instant,
        now: Now,
    ) {
        if deadline <= now.at {
            tracing::info!(code, "a pairing claim expired before it was sent");
            let _ = reply.try_send(Err(ClaimError::Timeout));
            return;
        }
        if self.relay_state != RelayState::Connected {
            let _ = reply.try_send(Err(ClaimError::RelayOffline));
            return;
        }
        if let Some(position) = self.claims.iter().position(|claim| claim.code == code) {
            let earlier = self.claims.remove(position);
            let _ = earlier.reply.try_send(Err(ClaimError::Timeout));
        }
        if self
            .outbound
            .send(claim_frame(&code, &self.identity.mac_name))
            .is_err()
        {
            let _ = reply.try_send(Err(ClaimError::RelayOffline));
            return;
        }
        self.claims.push(PendingClaim {
            code,
            reply,
            deadline,
        });
    }

    /// The dictation worker's verdict on `id`: the record takes it, the
    /// dedupe table remembers it, and every reply held for `id` goes out.
    fn on_delivered(&mut self, id: &str, report: DeliveryReport, now: Now) {
        let Some(live) = self.records.get_mut(id) else {
            tracing::warn!(
                id,
                "a delivery arrived for a recording Hex no longer tracks"
            );
            return;
        };
        if live.record.state != State::Transcribing {
            tracing::info!(
                id,
                state = live.record.state.name(),
                "a late delivery was ignored"
            );
            return;
        }
        let record = &mut live.record;
        let chinese = record.language.starts_with("zh");
        record.paste = PasteMarks::default();
        match report.outcome {
            DeliveryOutcome::Sent(verification) => {
                record.state = if verification == Verification::Terminal {
                    State::Pasted
                } else {
                    State::Unverified
                };
                record.paste = PasteMarks::sent(verification, false);
            }
            DeliveryOutcome::Copied => {
                record.state = State::Copied;
                record.paste = PasteMarks::sent(Verification::None, false);
            }
            DeliveryOutcome::Withheld(withheld) => {
                record.state = match withheld {
                    Withheld::TargetChanged | Withheld::NoSnapshot => State::TargetChanged,
                    Withheld::NotEditable => State::NotEditable,
                    Withheld::SecureInput | Withheld::PasswordField => State::Sensitive,
                };
                record.paste_reason = withheld.paste_reason().map(str::to_owned);
                record.reason = withheld.reason(chinese).map(str::to_owned);
            }
            DeliveryOutcome::Suspect(reason) => {
                record.state = State::Suspect;
                record.reason = Some(reason.text(chinese).to_owned());
            }
            DeliveryOutcome::Failed(error) => {
                record.state = State::Failed;
                record.error = Some(error);
            }
        }
        record.text = report.text;
        record.raw_text = report.raw_text;
        record.transcribe_ms = Some(report.transcribe_ms);
        record.paste_ms = Some(report.paste_ms);
        record.receive_to_paste_ms = live
            .received_at
            .map(|received| now.at.saturating_duration_since(received).as_millis() as u64);
        let stamp = Timestamp::string(now.wall);
        if record.state != State::Failed {
            record.completed_at = Some(stamp.clone());
        }
        record.updated_at = stamp;
        live.touched = now.at;
        let (status, body) = transcript_reply(record);
        let duplicate = record.duplicate_json();
        self.remember_handled(id, now);
        let (mine, others): (Vec<_>, Vec<_>) = std::mem::take(&mut self.waiting)
            .into_iter()
            .partition(|waiting| waiting.id() == id && !matches!(waiting, Waiting::Focus { .. }));
        self.waiting = others;
        for waiting in mine {
            match waiting {
                Waiting::Transcript { reply, .. } => self.answer(reply, status, body.clone()),
                Waiting::Duplicate { reply, .. } => self.answer(reply, 200, duplicate.clone()),
                Waiting::Focus { .. } => unreachable!("focus waiters were kept"),
            }
        }
    }

    /// The read-back's verdict (`recordConfirmation`): a confirmed paste
    /// still `unverified` and not yet entered becomes `pasted` with
    /// `read-back`; anything else keeps its state.
    fn on_confirmed(&mut self, id: &str, verified: bool, now: Now) {
        let Some(live) = self.records.get_mut(id) else {
            return;
        };
        let record = &mut live.record;
        if !verified || record.state != State::Unverified || record.paste.entered {
            return;
        }
        record.state = State::Pasted;
        record.paste = PasteMarks::sent(Verification::ReadBack, false);
        record.updated_at = Timestamp::string(now.wall);
        live.touched = now.at;
        self.remember_handled(id, now);
    }

    /// The watch `id` belongs to: the live record's, else the dedupe
    /// table's, so ownership outlives pruning and restarts.
    pub(super) fn owner_of(&self, id: &str, now: Now) -> Option<&str> {
        match self.records.get(id) {
            Some(live) => live.record.owner.as_deref(),
            None => self.delivered.get(id, now.wall)?.owner.as_deref(),
        }
    }

    /// Writes `id`'s live state into the dedupe table, so a resend after
    /// the live record is gone still reads `duplicate`.
    pub(super) fn remember_handled(&mut self, id: &str, now: Now) {
        let Some(live) = self.records.get(id) else {
            return;
        };
        let record = &live.record;
        self.delivered.record(
            id,
            record.owner.as_deref(),
            record.state,
            record.paste,
            record.truncated,
            record.completed_at.clone(),
            now.wall,
        );
        if let Err(error) = self.delivered.save(&self.delivered_path) {
            tracing::warn!(%error, path = %self.delivered_path.display(), "delivered-ids.json was not saved");
        }
    }

    /// Marks `id` failed with `error` (`fail` in Swift) and remembers it.
    pub(super) fn fail(&mut self, id: &str, device: &str, error: String, now: Now) -> Refusal {
        let live = self
            .records
            .entry(id.to_owned())
            .or_insert_with(|| LiveRecord::new(id, device, now));
        live.record.state = State::Failed;
        live.record.error = Some(error.clone());
        live.record.updated_at = Timestamp::string(now.wall);
        live.touched = now.at;
        self.remember_handled(id, now);
        Refusal::Failed { error }
    }
}

/// The answer to the `/e2ee` that carried the audio (Swift's
/// `transcribeAndPaste` tail, the `suspect` branch, and `fail`).
fn transcript_reply(record: &Record) -> (u16, Value) {
    match record.state {
        State::Failed => {
            let refusal = Refusal::Failed {
                error: record.error.clone().unwrap_or_default(),
            };
            (refusal.status(), refusal.body())
        }
        State::Suspect => {
            let mut body = json!({
                "ok": true,
                "state": "suspect",
                "text": record.text,
                "reason": record.reason,
            });
            if record.truncated {
                body["truncated"] = Value::Bool(true);
            }
            (200, body)
        }
        _ => {
            let mut body = json!({
                "ok": true,
                "state": record.state.name(),
                "verified": record.paste.verified,
                "verification": record.paste.verification.as_str(),
                "text": record.text,
                "transcribe_ms": record.transcribe_ms,
                "paste_ms": record.paste_ms,
                "receive_to_paste_ms": record.receive_to_paste_ms,
            });
            if let Some(reason) = &record.paste_reason {
                body["paste_reason"] = Value::from(reason.as_str());
            }
            if record.truncated {
                body["truncated"] = Value::Bool(true);
            }
            (200, body)
        }
    }
}
