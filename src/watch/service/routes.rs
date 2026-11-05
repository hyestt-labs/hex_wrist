//! Request dispatch (`dispatch` in Swift's `ReceiverService`): the route
//! table, authentication, the ratchet, and the handlers that answer at once.
//! The envelope handlers and the recording path are in `envelope`.

use std::collections::BTreeMap;

use serde_json::{Map, Value, json};

use super::pending::{Reply, Waiting};
use super::state::{CAPTURE_WAIT, Core, Now};
use crate::watch::frame::{
    E2EE_PATH, E2EE_QUERY_PATH, RequestHead, Route, Tier, body_refusal, cloud_route,
    plaintext_refusal,
};
use crate::watch::pairing::requests::{PairingRequest, PairingResult};
use crate::watch::pairing::{self, FILE_NAME as PAIRED_FILE};
use crate::watch::recording::{Caller, Channel, Record, State, refuse_non_owner};
use crate::watch::refusal::Refusal;
use crate::watch::service::Message;
use crate::watch::{
    MINIMUM_PEER_PROTOCOL, PROTOCOL_VERSION, PeerInfo, SENT_AT_HEADER, TOKEN_HEADER, e2ee,
    is_valid_recording_id,
};

/// Plaintext calls a watch that proved `requires_e2ee` no longer gets
/// (`ratchetRefusal`).
const RATCHETED_PATHS: &[&str] = &["/audio", "/focus", "/enter", "/undo"];

/// The paired watch a device-tier request came from.
pub(crate) struct Auth {
    pub(crate) device: String,
    /// The envelope that carried the request was opened with this watch's
    /// own key (Swift's `isInsideOwnEnvelope`); it has passed the ratchet
    /// and may read a record's text.
    pub(crate) inside: bool,
}

/// Swift's `jsonBody`: an empty body reads as `{}`; anything that is not
/// a JSON object reads as nothing.
pub(crate) fn json_body(body: &[u8]) -> Option<Map<String, Value>> {
    if body.is_empty() {
        return Some(Map::new());
    }
    match serde_json::from_slice::<Value>(body).ok()? {
        Value::Object(map) => Some(map),
        _ => None,
    }
}

fn string_field(payload: &Map<String, Value>, key: &str) -> Option<String> {
    payload.get(key).and_then(Value::as_str).map(str::to_owned)
}

impl Core {
    /// A request the relay forwarded.
    pub(crate) fn on_request(&mut self, head: RequestHead, body: Vec<u8>, now: Now) {
        let reply = Reply::new(&head.id, &head.method, &head.path);
        self.dispatch(head, body, reply, false, now);
    }

    /// A request an envelope carried has a sealed `reply`. `late_upload`
    /// marks `/audio` from a background upload, which reads `duplicate` for
    /// any recording already handled.
    pub(super) fn dispatch(
        &mut self,
        head: RequestHead,
        body: Vec<u8>,
        reply: Reply,
        late_upload: bool,
        now: Now,
    ) {
        let Some(route) = cloud_route(&head.method, &head.path) else {
            return self.refuse(reply, Refusal::NotFound);
        };
        if let Some(refusal) = body_refusal(route, body.len()) {
            return self.refuse(reply, refusal);
        }
        let envelope = reply.seal().map(|seal| seal.device_id.as_str());
        let auth = match route.tier {
            Tier::Open => None,
            Tier::Device => match self.authenticate(&head, route, envelope, now) {
                Ok(auth) => Some(auth),
                Err(refusal) => return self.refuse(reply, refusal),
            },
            Tier::Master => return self.refuse(reply, Refusal::NotFound),
        };
        let (device, inside) =
            auth.map_or((String::new(), false), |auth| (auth.device, auth.inside));
        let served = match (route.method, route.path) {
            ("GET", "/health") => self.health(),
            ("GET", "/status") => self.status(&head.query, &device, inside, now),
            ("POST", "/focus") => return self.focus(&body, &device, reply, now),
            ("POST", "/enter") => Ok(not_supported("Enter")),
            ("POST", "/undo") => Ok(not_supported("Undo")),
            ("POST", "/ping") => Ok(ping(&head, now)),
            ("POST", "/pairing/request") => self.pairing_request(&body, now),
            ("GET", "/pairing/result") => self.pairing_result(&head.query, now),
            ("POST", "/audio") => {
                return self.audio(&head, body, device, reply, late_upload, now);
            }
            ("POST", E2EE_PATH) => return self.encrypted_audio(&head, &body, reply, now),
            ("POST", E2EE_QUERY_PATH) => return self.encrypted_query(&head, &body, reply, now),
            _ => Err(Refusal::NotFound),
        };
        match served {
            Ok((status, body)) => self.answer(reply, status, body),
            Err(refusal) => self.refuse(reply, refusal),
        }
    }

    /// Swift's `authorize` plus the checks `dispatch` runs for a device
    /// caller: token, ratchet, pairing completion, protocol age, `touch`.
    /// `envelope` names the watch whose key opened the carrying envelope;
    /// another watch's token inside it counts as plaintext.
    fn authenticate(
        &mut self,
        head: &RequestHead,
        route: &Route,
        envelope: Option<&str>,
        now: Now,
    ) -> Result<Auth, Refusal> {
        let token = head.header(TOKEN_HEADER).unwrap_or("");
        let device = match self.paired.device_for_token(token) {
            Some(device) => device,
            None if self.paired.secrets_unavailable() => {
                tracing::info!(path = %head.path, "device secrets are unreadable; answering 503");
                return Err(Refusal::SecretsUnavailable);
            }
            None => {
                if route.path == "/audio" {
                    tracing::info!("a recording was refused: its token matches no watch");
                }
                return Err(Refusal::Forbidden);
            }
        };
        let inside = envelope == Some(device.as_str());
        if envelope.is_some() && !inside {
            tracing::info!(path = %head.path, "an envelope carried another watch's token");
        }
        if !inside {
            if let Some(refusal) = plaintext_refusal(route) {
                return Err(refusal);
            }
            if RATCHETED_PATHS.contains(&route.path) && self.paired.requires_e2ee(&device) {
                return Err(Refusal::E2eeRequired);
            }
        }
        // The first call with the new token proves the watch read its
        // pairing result.
        self.requests.completed(&device);
        let peer = PeerInfo::from_headers(|name| head.header(name));
        if peer.is_too_old() {
            return Err(Refusal::watch_outdated(
                MINIMUM_PEER_PROTOCOL,
                &self.identity.app_version,
            ));
        }
        self.paired.touch(&device, Some(&peer), now.wall);
        Ok(Auth { device, inside })
    }

    fn health(&self) -> Result<(u16, Value), Refusal> {
        let model = self.dictation.model();
        let trusted = self.desktop.trusted();
        Ok((
            200,
            json!({
                "ok": true,
                "paste_ready": trusted,
                "paste_detail": if trusted {
                    ""
                } else {
                    "Accessibility permission is not granted (System Settings › Privacy & Security › Accessibility)"
                },
                "model": model.state,
                "model_id": model.id,
                "model_name": model.name,
                "relay": self.relay_state.name(),
                "protocol": PROTOCOL_VERSION,
                "min_peer_protocol": MINIMUM_PEER_PROTOCOL,
                "app_version": self.identity.app_version,
                "licensing": {
                    "state": "licensed",
                    "daily_limit": null,
                    "used_today": null,
                    "remaining_today": null,
                    "resets_at": null,
                    "max_seconds": null,
                    "license_status": "active",
                },
            }),
        ))
    }

    /// Another watch's recording reads as unknown, not `not-owner`: the
    /// watch takes a 403 here as a revoked token and forgets the pairing,
    /// while a 404 only resends the audio, which `/audio` refuses.
    fn status(
        &self,
        query: &BTreeMap<String, String>,
        device: &str,
        inside: bool,
        now: Now,
    ) -> Result<(u16, Value), Refusal> {
        let id = query.get("id").map(String::as_str).unwrap_or("");
        if !is_valid_recording_id(id) {
            return Err(Refusal::InvalidId);
        }
        if refuse_non_owner(self.owner_of(id, now), Caller::Device(device)).is_err() {
            return Err(Refusal::UnknownId);
        }
        let channel = if inside {
            Channel::InsideE2ee
        } else {
            Channel::Plaintext
        };
        if let Some(live) = self.records.get(id) {
            return Ok((200, live.record.status_json(channel, now.wall)));
        }
        self.delivered
            .get(id, now.wall)
            .and_then(Record::from_delivered)
            .map(|record| (200, record.status_json(channel, now.wall)))
            .ok_or(Refusal::UnknownId)
    }

    /// `/focus`: the watch started recording `id`. The `watch-ax` thread
    /// reads the focused field; the reply waits for it (`Waiting::Capture`)
    /// and then marks the recording awaiting its audio.
    fn focus(&mut self, body: &[u8], device: &str, reply: Reply, now: Now) {
        let checked = (|| {
            let payload = json_body(body).ok_or(Refusal::InvalidJson)?;
            let id = string_field(&payload, "id").unwrap_or_default();
            if !is_valid_recording_id(&id) {
                return Err(Refusal::InvalidId);
            }
            refuse_non_owner(self.owner_of(&id, now), Caller::Device(device))?;
            Ok(id)
        })();
        let id = match checked {
            Ok(id) => id,
            Err(refusal) => return self.refuse(reply, refusal),
        };
        let existing = self.records.get(&id).map(|live| &live.record);
        if let Some(record) = existing
            && record.state != State::AwaitingAudio
        {
            let state = record.state.name();
            return self.answer(
                reply,
                200,
                json!({"ok": true, "captured": false, "state": state}),
            );
        }
        self.waiting.push(Waiting::Capture {
            id: id.clone(),
            device: device.to_owned(),
            reply,
            deadline: now.at + CAPTURE_WAIT,
        });
        let inbox = self.inbox.clone();
        self.desktop.capture(move |field| {
            let _ = inbox.send(Message::Captured { id, field });
        });
    }

    /// `/pairing/request`: only the watch whose code was just typed, within
    /// the approval window, gets a token.
    fn pairing_request(&mut self, body: &[u8], now: Now) -> Result<(u16, Value), Refusal> {
        let payload = json_body(body).ok_or(Refusal::InvalidJson)?;
        if self.requests.window_remaining(now.at).is_zero() {
            return Err(Refusal::PairingClosed);
        }
        let mut device_id: String = match payload.get("device_id") {
            None | Some(Value::Null) => String::new(),
            Some(Value::String(text)) => text.chars().take(64).collect(),
            Some(other) => other.to_string().chars().take(64).collect(),
        };
        if device_id.is_empty() {
            device_id = random_device_id().ok_or(Refusal::PairingClosed)?;
        }
        // The approval names the id the relay gave the watch; compare it
        // before `previous_token` swaps in the older entry.
        let claimed = device_id.clone();
        if let Some(previous) = string_field(&payload, "previous_token")
            && !previous.is_empty()
            && let Some(owner) = self.paired.device_for_token(&previous)
        {
            device_id = owner;
        }
        let name: String = string_field(&payload, "name")
            .unwrap_or_else(|| "Apple Watch".to_owned())
            .chars()
            .take(64)
            .collect();
        let watch_public_key = string_field(&payload, "e2ee_public_key").unwrap_or_default();
        if !e2ee::is_valid_public_key(&watch_public_key) {
            tracing::info!(name, "a pairing request carried no usable public key");
            return Err(Refusal::watch_outdated(
                PROTOCOL_VERSION,
                &self.identity.app_version,
            ));
        }
        let request = PairingRequest {
            claimed_device_id: &claimed,
            device_id: &device_id,
            name: &name,
            watch_public_key: &watch_public_key,
        };
        let Some(request_id) = self
            .requests
            .admit(&request, &mut self.paired, now.at, now.wall)
        else {
            tracing::info!(
                name,
                claimed,
                "a pairing request from a watch other than the claimed one was refused"
            );
            return Err(Refusal::PairingClosed);
        };
        tracing::info!(
            name,
            device_id,
            file = PAIRED_FILE,
            "a watch was approved with the pairing code"
        );
        Ok((
            200,
            json!({
                "ok": true,
                "name": self.identity.mac_name,
                "device_id": self.identity.mac_id,
                "request_id": request_id,
            }),
        ))
    }

    fn pairing_result(
        &mut self,
        query: &BTreeMap<String, String>,
        now: Now,
    ) -> Result<(u16, Value), Refusal> {
        let request_id = query.get("request_id").map(String::as_str).unwrap_or("");
        if let Some(failure) = self.requests.failure(request_id, now.at) {
            return Ok((
                200,
                json!({
                    "ok": false,
                    "approved": false,
                    "error": pairing::watch_facing_failure(failure),
                }),
            ));
        }
        match self.requests.result(request_id, now.at) {
            PairingResult::Unknown => Err(Refusal::UnknownRequest),
            PairingResult::Sealed {
                sealed,
                mac_public_key,
            } => Ok((
                200,
                json!({
                    "ok": true,
                    "approved": true,
                    "sealed_result": sealed,
                    "e2ee_public_key": mac_public_key,
                }),
            )),
        }
    }
}

/// `/enter` and `/undo` wait for B2 (they need the field snapshot and the
/// read-back); the watch reads `reason` and shows it.
fn not_supported(action: &str) -> (u16, Value) {
    (
        200,
        json!({
            "ok": false,
            "reason": "not-supported",
            "error": format!("{action} from the watch is not supported yet"),
        }),
    )
}

/// The relay's edge normally answers `/ping` itself; this is the direct
/// shape for completeness.
fn ping(head: &RequestHead, now: Now) -> (u16, Value) {
    (
        200,
        json!({
            "ok": true,
            "server_time": now.epoch(),
            "one_way_ms": one_way_ms(head, now),
        }),
    )
}

pub(crate) fn one_way_ms(head: &RequestHead, now: Now) -> Option<i64> {
    let sent = head.header(SENT_AT_HEADER)?.parse::<f64>().ok()?;
    Some(((now.epoch() - sent) * 1_000.0).round() as i64)
}

/// Wristper's fallback id for a request without one: 32 lower-case hex
/// digits.
fn random_device_id() -> Option<String> {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).ok()?;
    Some(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}
