//! The encrypted envelopes (`/e2ee`, `/e2ee/query`) and the recording path
//! they carry (`encryptedAudio`, `encryptedQuery`, `audio`, and the
//! admission half of `transcribeAndPaste` in Swift).

use std::collections::BTreeMap;
use std::time::Duration;

use serde_json::json;

use super::pending::{AudioJob, Reply, Seal, Waiting};
use super::state::{Core, LiveRecord, Now, TRANSCRIPT_WAIT};
use crate::watch::delivery::{DeliveryEvent, Job, WatchDelivery};
use crate::watch::e2ee::{self, Aad, QueryShape, SharedKey};
use crate::watch::field::Expectation;
use crate::watch::frame::{MAX_AUDIO_BYTES, RequestHead};
use crate::watch::pairing::tokens_equal;
use crate::watch::recording::{
    Caller, DUPLICATE_AUDIO_WAIT_SECONDS, FOCUS_WAIT_SECONDS, Record, State, refuse_non_owner,
};
use crate::watch::refusal::Refusal;
use crate::watch::service::Message;
use crate::watch::{
    APP_VERSION_HEADER, DEVICE_HEADER, FOCUS_HEADER, ID_HEADER, LANGUAGE_HEADER, NONCE_HEADER,
    PROTOCOL_HEADER, SENT_AT_HEADER, TOKEN_HEADER, Timestamp, audio, is_valid_recording_id,
};

const ROUTE_HEADER: &str = "x-wristper-route";
const SESSION_HEADER: &str = "x-wristper-session";
const PROJECT_HEADER: &str = "x-wristper-project";
const APPROVAL_HEADER: &str = "x-wristper-approval";

/// An envelope opened: who sent it and what seals the answer.
struct Opened {
    device: String,
    nonce_base64: String,
    key: SharedKey,
    plaintext: Vec<u8>,
}

enum Kind {
    Audio,
    Query,
}

impl Core {
    /// The checks both envelopes share, up to the plaintext. Every failure
    /// but a locked Keychain is one 400 that says no more.
    fn open_envelope(
        &mut self,
        head: &RequestHead,
        body: &[u8],
        kind: Kind,
    ) -> Result<Opened, Refusal> {
        let device = head.header(DEVICE_HEADER).unwrap_or("").to_owned();
        let nonce_base64 = head.header(NONCE_HEADER).unwrap_or("").to_owned();
        if self.paired.secrets_unavailable_for(&device) {
            return Err(Refusal::SecretsUnavailable);
        }
        if device.is_empty() {
            return Err(Refusal::InvalidE2ee);
        }
        let key = self
            .paired
            .e2ee_key_for(&device)
            .ok_or(Refusal::InvalidE2ee)?;
        let nonce = e2ee::decode_nonce(&nonce_base64).ok_or(Refusal::InvalidE2ee)?;
        let aad = match kind {
            Kind::Audio => Aad::Request {
                device_id: &device,
                nonce_base64: &nonce_base64,
            },
            Kind::Query => Aad::Query {
                device_id: &device,
                nonce_base64: &nonce_base64,
            },
        };
        let plaintext = e2ee::open(body, &nonce, &key, &aad).ok_or(Refusal::InvalidE2ee)?;
        Ok(Opened {
            device,
            nonce_base64,
            key,
            plaintext,
        })
    }

    pub(super) fn encrypted_audio(
        &mut self,
        head: &RequestHead,
        body: &[u8],
        mut reply: Reply,
        now: Now,
    ) {
        let opened = match self.open_envelope(head, body, Kind::Audio) {
            Ok(opened) => opened,
            Err(refusal) => return self.refuse(reply, refusal),
        };
        let decoded = e2ee::decode_audio(&opened.plaintext)
            .filter(|(head, audio)| audio.len() <= MAX_AUDIO_BYTES && head.is_fresh(now.epoch()));
        let Some((audio_head, audio_bytes)) = decoded else {
            return self.refuse(reply, Refusal::InvalidE2ee);
        };
        if !self.nonces.consume(&opened.device, &opened.nonce_base64) {
            return self.refuse(reply, Refusal::Replayed);
        }
        reply.seal_with(Seal {
            key: opened.key,
            device_id: opened.device.clone(),
            request_nonce_base64: opened.nonce_base64,
        });
        self.ratchet(audio_head.requires_e2ee, &opened.device, &audio_head.token);
        let late_upload = audio_head.may_arrive_late();
        let mut headers = BTreeMap::from([
            (TOKEN_HEADER.to_owned(), audio_head.token),
            (ID_HEADER.to_owned(), audio_head.id),
            (LANGUAGE_HEADER.to_owned(), audio_head.language),
            (DEVICE_HEADER.to_owned(), opened.device),
            (SENT_AT_HEADER.to_owned(), audio_head.sent_at.to_string()),
            (FOCUS_HEADER.to_owned(), audio_head.focus),
        ]);
        for (name, value) in [
            (ROUTE_HEADER, audio_head.route),
            (SESSION_HEADER, audio_head.session),
            (PROJECT_HEADER, audio_head.project),
            (APPROVAL_HEADER, audio_head.approval),
        ] {
            if let Some(value) = value {
                headers.insert(name.to_owned(), value);
            }
        }
        copy_version_headers(head, &mut headers);
        let inner = RequestHead {
            id: head.id.clone(),
            method: "POST".to_owned(),
            path: "/audio".to_owned(),
            query: BTreeMap::new(),
            headers,
            body_len: audio_bytes.len(),
        };
        self.dispatch(inner, audio_bytes.to_vec(), reply, late_upload, now);
    }

    pub(super) fn encrypted_query(
        &mut self,
        head: &RequestHead,
        body: &[u8],
        mut reply: Reply,
        now: Now,
    ) {
        let opened = match self.open_envelope(head, body, Kind::Query) {
            Ok(opened) => opened,
            Err(refusal) => return self.refuse(reply, refusal),
        };
        let Some(query_head) =
            e2ee::decode_query(&opened.plaintext).filter(|query| query.is_fresh(now.epoch()))
        else {
            return self.refuse(reply, Refusal::InvalidE2ee);
        };
        if !self.nonces.consume(&opened.device, &opened.nonce_base64) {
            return self.refuse(reply, Refusal::Replayed);
        }
        reply.seal_with(Seal {
            key: opened.key,
            device_id: opened.device.clone(),
            request_nonce_base64: opened.nonce_base64,
        });
        self.ratchet(query_head.requires_e2ee, &opened.device, &query_head.token);
        if query_head.is_stale_action(now.epoch()) {
            tracing::info!(
                device = opened.device,
                path = query_head.path,
                "an action arrived too late"
            );
            return self.refuse(reply, Refusal::ConfirmStale);
        }
        let mut headers = BTreeMap::from([
            (TOKEN_HEADER.to_owned(), query_head.token),
            (DEVICE_HEADER.to_owned(), opened.device),
            (SENT_AT_HEADER.to_owned(), query_head.sent_at.to_string()),
        ]);
        copy_version_headers(head, &mut headers);
        let (method, query, body) = match e2ee::query_shape(&query_head.path) {
            Some(QueryShape::Post) => (
                "POST",
                BTreeMap::new(),
                serde_json::to_vec(&query_head.query).unwrap_or_default(),
            ),
            // `decode_query` admitted only paths with a shape.
            Some(QueryShape::Get) | None => ("GET", query_head.query, Vec::new()),
        };
        let inner = RequestHead {
            id: head.id.clone(),
            method: method.to_owned(),
            path: query_head.path,
            query,
            headers,
            body_len: body.len(),
        };
        self.dispatch(inner, body, reply, false, now);
    }

    /// Only a field inside the ciphertext proves the watch wants the
    /// ratchet; the outer headers are the relay's to change.
    fn ratchet(&mut self, proof: Option<bool>, device: &str, token: &str) {
        if proof != Some(true) {
            return;
        }
        let expected = self.paired.token_for(device).unwrap_or("").to_owned();
        if tokens_equal(token, &expected) && self.paired.require_e2ee(device) {
            tracing::info!(device, "this watch now takes encrypted actions only");
        }
    }

    fn handled_state(&self, id: &str, now: Now) -> Option<String> {
        if let Some(live) = self.records.get(id)
            && live.record.state != State::AwaitingAudio
        {
            return Some(live.record.state.name().to_owned());
        }
        self.delivered
            .get(id, now.wall)
            .map(|entry| entry.state.clone())
    }

    /// The inner `/audio`, reached only through `/e2ee` (plaintext `/audio`
    /// is refused before any body is read) and only once its token and
    /// envelope agree.
    pub(super) fn audio(
        &mut self,
        head: &RequestHead,
        body: Vec<u8>,
        device: String,
        reply: Reply,
        late_upload: bool,
        now: Now,
    ) {
        let id = head.header(ID_HEADER).unwrap_or("");
        if !is_valid_recording_id(id) {
            return self.refuse(reply, Refusal::InvalidMetadata);
        }
        if head
            .header(ROUTE_HEADER)
            .is_some_and(|route| !route.is_empty())
        {
            return self.refuse(reply, Refusal::InvalidRoute);
        }
        let focus_requested = match head.header(FOCUS_HEADER) {
            Some("requested") => true,
            Some("current") => false,
            _ => return self.refuse(reply, Refusal::InvalidMetadata),
        };
        if let Err(refusal) = refuse_non_owner(self.owner_of(id, now), Caller::Device(&device)) {
            return self.refuse(reply, refusal);
        }
        // Nonces live in memory and get pushed out; a background upload may
        // outlive them, so the recording's own state stops its replay.
        if late_upload && let Some(state) = self.handled_state(id, now) {
            return self.answer(
                reply,
                200,
                json!({"ok": true, "duplicate": true, "state": state}),
            );
        }
        let job = AudioJob {
            device,
            id: id.to_owned(),
            language: head.header(LANGUAGE_HEADER).unwrap_or("").to_owned(),
            audio: body,
            at_cursor: !focus_requested,
            snapshot_missing: false,
        };
        self.accept_audio(job, reply, focus_requested, now);
    }

    /// Ownership, idempotency, and the wait for `/focus`; then the
    /// transcription starts. Ownership comes first so a duplicate answer
    /// never reaches another watch.
    pub(super) fn accept_audio(
        &mut self,
        job: AudioJob,
        reply: Reply,
        may_wait_for_focus: bool,
        now: Now,
    ) {
        let owner = self.owner_of(&job.id, now);
        if let Err(refusal) = refuse_non_owner(owner, Caller::Device(&job.device)) {
            return self.refuse(reply, refusal);
        }
        let live = self.records.get(&job.id).map(|live| &live.record);
        match live {
            Some(record) if record.state == State::Transcribing => {
                self.waiting.push(Waiting::Duplicate {
                    id: job.id,
                    reply,
                    deadline: now.at + Duration::from_secs(DUPLICATE_AUDIO_WAIT_SECONDS),
                });
                return;
            }
            Some(record) if record.state.is_delivered() => {
                return self.answer(reply, 200, record.duplicate_json());
            }
            Some(_) => {}
            None => {
                let earlier = self
                    .delivered
                    .get(&job.id, now.wall)
                    .and_then(Record::from_delivered)
                    .filter(|record| record.state.is_delivered());
                if let Some(record) = earlier {
                    return self.answer(reply, 200, record.duplicate_json());
                }
            }
        }
        if may_wait_for_focus && live.is_none() {
            self.waiting.push(Waiting::Focus {
                id: job.id.clone(),
                job,
                reply,
                deadline: now.at + Duration::from_secs(FOCUS_WAIT_SECONDS),
            });
            return;
        }
        self.start_transcription(job, reply, now);
    }

    /// `/focus` for `id` landed: audio that waited for it runs now.
    pub(super) fn release_focus_wait(&mut self, id: &str, now: Now) {
        let position = self
            .waiting
            .iter()
            .position(|waiting| matches!(waiting, Waiting::Focus { id: held, .. } if held == id));
        if let Some(position) = position
            && let Waiting::Focus { job, reply, .. } = self.waiting.remove(position)
        {
            self.accept_audio(job, reply, false, now);
        }
    }

    fn start_transcription(&mut self, job: AudioJob, reply: Reply, now: Now) {
        let samples = match audio::decode_m4a(&job.audio) {
            Ok(samples) => samples,
            Err(error) => {
                tracing::warn!(id = job.id, %error, "a watch recording could not be decoded");
                let refusal = self.fail(&job.id, &job.device, error.to_string(), now);
                return self.refuse(reply, refusal);
            }
        };
        let live = self
            .records
            .entry(job.id.clone())
            .or_insert_with(|| LiveRecord::new(&job.id, &job.device, now));
        let expectation = if job.at_cursor {
            Expectation::Current
        } else if job.snapshot_missing {
            Expectation::Missing
        } else {
            // Wristper pastes at the cursor when `/focus` read nothing. Hex
            // does not: the user may have left the field while recording, so
            // only an explicit `focus: current` resend pastes there.
            live.field
                .clone()
                .map_or(Expectation::Missing, Expectation::Field)
        };
        // A resend of the same id must not keep the previous attempt's text.
        live.record = Record {
            state: State::Transcribing,
            language: job.language.clone(),
            owner: Some(job.device.clone()),
            updated_at: Timestamp::string(now.wall),
            ..LiveRecord::new(&job.id, &job.device, now).record
        };
        // A key still on its way belongs to the attempt that is starting over.
        if let Some(ticket) = live.in_flight.take() {
            ticket.cancel();
        }
        live.received_at = Some(now.at);
        live.touched = now.at;
        self.announce(&job.id, now);
        let inbox = self.inbox.clone();
        let id = job.id.clone();
        let delivery = WatchDelivery::new(
            Job {
                id: job.id.clone(),
                language: job.language.clone(),
                expectation,
                desktop: self.desktop.clone(),
            },
            move |event| {
                let message = match event {
                    DeliveryEvent::Report(report) => Message::Delivered {
                        id: id.clone(),
                        report,
                    },
                    DeliveryEvent::Confirmed { verified } => Message::Confirmed {
                        id: id.clone(),
                        verified,
                    },
                };
                let _ = inbox.send(message);
            },
        );
        if let Err(why) = self.dictation.transcribe(samples, delivery) {
            tracing::warn!(
                id = job.id,
                why,
                "the dictation pipeline refused a watch recording"
            );
            let refusal = self.fail(&job.id, &job.device, why, now);
            return self.refuse(reply, refusal);
        }
        self.waiting.push(Waiting::Transcript {
            id: job.id,
            reply,
            deadline: now.at + TRANSCRIPT_WAIT,
        });
    }
}

fn copy_version_headers(outer: &RequestHead, inner: &mut BTreeMap<String, String>) {
    for name in [PROTOCOL_HEADER, APP_VERSION_HEADER] {
        if let Some(value) = outer.header(name) {
            inner.insert(name.to_owned(), value.to_owned());
        }
    }
}
