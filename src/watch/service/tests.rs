use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use p256::SecretKey;
use p256::elliptic_curve::sec1::ToSec1Point;
use serde_json::{Value, json};

use super::pending::Outbound;
use super::state::{Core, Identity, MAX_LIVE_RECORDS, Now, TRANSCRIPT_WAIT};
use super::*;
use crate::watch::delivery::{DeliveryOutcome, DeliveryReport};
use crate::watch::desktop::Desktop;
use crate::watch::desktop::fake::FakeScreen;
use crate::watch::e2ee::{self, Aad, AudioHead, SharedKey};
use crate::watch::frame::{Inbound, RequestHead};
use crate::watch::pairing::MemorySecretStore;
use crate::watch::recording::{DUPLICATE_AUDIO_WAIT_SECONDS, Verification};
use crate::watch::relay::{NotConnected, RelayEvent, RelayState};
use crate::watch::suspect::SuspectReason;
use crate::watch::{DEVICE_HEADER, NONCE_HEADER, PROTOCOL_HEADER, TOKEN_HEADER};

mod capsule;
mod field;
mod keys;

const M4A: &[u8] = include_bytes!("../../../tests/fixtures/watch-audio/tone-440-16k.m4a");
const ID: &str = "6f9619ff-8b86-d011-b42d-00c04fc964ff";
const OTHER_ID: &str = "11111111-2222-3333-4444-555555555555";
const MAC_NAME: &str = "Glen's Mac";
const APP_VERSION: &str = "3.0.0-test";

struct Sink {
    frames: Arc<Mutex<Vec<String>>>,
    connected: Arc<AtomicBool>,
}

impl Outbound for Sink {
    fn send(&mut self, text: String) -> Result<(), NotConnected> {
        if !self.connected.load(Ordering::SeqCst) {
            return Err(NotConnected);
        }
        self.frames.lock().unwrap().push(text);
        Ok(())
    }

    fn stop(self: Box<Self>) {}
}

type Jobs = Arc<Mutex<Vec<(Vec<f32>, WatchDelivery)>>>;

struct FakeDictation {
    jobs: Jobs,
    refuse: Arc<AtomicBool>,
}

impl Dictation for FakeDictation {
    fn transcribe(&self, samples: Vec<f32>, delivery: WatchDelivery) -> Result<(), String> {
        if self.refuse.load(Ordering::SeqCst) {
            return Err("the listener is not running".to_owned());
        }
        self.jobs.lock().unwrap().push((samples, delivery));
        Ok(())
    }

    fn model(&self) -> ModelInfo {
        ModelInfo {
            state: "cold",
            id: "parakeet-tdt-0.6b-v3".to_owned(),
            name: "Parakeet v3".to_owned(),
        }
    }
}

struct Watch {
    device: String,
    token: String,
    key: SharedKey,
}

struct Answer {
    status: u16,
    body: Value,
    sealed: bool,
}

struct Harness {
    core: Core,
    messages: Receiver<Message>,
    frames: Arc<Mutex<Vec<String>>>,
    connected: Arc<AtomicBool>,
    jobs: Jobs,
    refuse: Arc<AtomicBool>,
    screen: Arc<FakeScreen>,
    now: Now,
    next_frame: u64,
    dir: PathBuf,
}

fn temp_dir(name: &str) -> PathBuf {
    let mut nonce = [0u8; 8];
    getrandom::fill(&mut nonce).unwrap();
    let dir = std::env::temp_dir().join(format!(
        "hex-watch-service-{name}-{}-{:016x}",
        std::process::id(),
        u64::from_le_bytes(nonce)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn harness(name: &str) -> Harness {
    let dir = temp_dir(name);
    let frames = Arc::new(Mutex::new(Vec::new()));
    let connected = Arc::new(AtomicBool::new(true));
    let jobs: Jobs = Arc::new(Mutex::new(Vec::new()));
    let refuse = Arc::new(AtomicBool::new(false));
    let (inbox, messages) = mpsc::channel();
    let screen = Arc::new(FakeScreen::default());
    let paired = PairedDevices::load(
        &dir.join(pairing::FILE_NAME),
        Box::new(MemorySecretStore::default()),
    );
    let core = Core::new(
        Identity {
            mac_id: "0f0f0f0f-0000-4000-8000-000000000001".to_owned(),
            mac_name: MAC_NAME.to_owned(),
            app_version: APP_VERSION.to_owned(),
        },
        paired,
        crate::watch::delivered::load(&dir.join(DELIVERED_FILE)),
        dir.join(DELIVERED_FILE),
        Box::new(Sink {
            frames: Arc::clone(&frames),
            connected: Arc::clone(&connected),
        }),
        Box::new(FakeDictation {
            jobs: Arc::clone(&jobs),
            refuse: Arc::clone(&refuse),
        }),
        Desktop::inline(screen.clone()),
        inbox,
    );
    Harness {
        core,
        messages,
        frames,
        connected,
        jobs,
        refuse,
        screen,
        now: Now::current(),
        next_frame: 0,
        dir,
    }
}

fn watch_secret() -> SecretKey {
    loop {
        let mut scalar = [0u8; 32];
        getrandom::fill(&mut scalar).unwrap();
        if let Ok(secret) = SecretKey::from_slice(&scalar) {
            return secret;
        }
    }
}

fn public_key_base64(secret: &SecretKey) -> String {
    BASE64.encode(secret.public_key().to_sec1_point(false).as_bytes())
}

fn audio_head(watch: &Watch, id: &str, focus: &str, sent_at: f64) -> AudioHead {
    AudioHead {
        version: e2ee::VERSION,
        token: watch.token.clone(),
        id: id.to_owned(),
        language: "en".to_owned(),
        sent_at,
        focus: focus.to_owned(),
        route: None,
        session: None,
        project: None,
        requires_e2ee: None,
        background: None,
        approval: None,
    }
}

fn report(outcome: DeliveryOutcome, text: &str) -> DeliveryReport {
    DeliveryReport {
        outcome,
        text: text.to_owned(),
        raw_text: None,
        transcribe_ms: 120,
        paste_ms: 30,
    }
}

fn keys(body: &Value) -> BTreeSet<&str> {
    body.as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect()
}

impl Harness {
    fn advance(&mut self, by: Duration) {
        self.now = Now {
            at: self.now.at + by,
            wall: self.now.wall + by,
        };
    }

    fn pump(&mut self) {
        while let Ok(message) = self.messages.try_recv() {
            self.core.handle(message, self.now);
        }
    }

    fn deliver(&mut self, index: usize, report: DeliveryReport) {
        let delivery = self.jobs.lock().unwrap()[index].1.clone();
        delivery.deliver(report);
        self.pump();
    }

    fn confirm(&mut self, index: usize, verified: bool) {
        let delivery = self.jobs.lock().unwrap()[index].1.clone();
        delivery.confirm(verified);
        self.pump();
    }

    fn status_of(&mut self, watch: &Watch, id: &str) -> Value {
        let answer = self.query(watch, "/status", &[("id", id)]).unwrap();
        assert_eq!((answer.status, answer.sealed), (200, true));
        answer.body
    }

    fn request(
        &mut self,
        method: &str,
        path: &str,
        query: &[(&str, &str)],
        headers: &[(&str, &str)],
        body: &[u8],
    ) -> Option<(u16, Value)> {
        self.next_frame += 1;
        let id = format!("frame-{}", self.next_frame);
        let head = RequestHead {
            id: id.clone(),
            method: method.to_owned(),
            path: path.to_owned(),
            query: query
                .iter()
                .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
                .collect(),
            headers: headers
                .iter()
                .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
                .collect(),
            body_len: body.len(),
        };
        self.core.on_request(head, body.to_vec(), self.now);
        // Captures answer through the inbox, even with the inline desktop.
        self.pump();
        self.take(&id)
    }

    fn take(&mut self, frame_id: &str) -> Option<(u16, Value)> {
        let mut frames = self.frames.lock().unwrap();
        let position = frames.iter().position(|text| {
            serde_json::from_str::<Value>(text).unwrap()["id"].as_str() == Some(frame_id)
        })?;
        let frame: Value = serde_json::from_str(&frames.remove(position)).unwrap();
        assert_eq!(frame["type"], "res");
        let status = frame["status"].as_u64().unwrap() as u16;
        let body: Value = serde_json::from_str(frame["body"].as_str().unwrap()).unwrap();
        Some((status, body))
    }

    fn pending_frame_ids(&self) -> Vec<String> {
        self.frames
            .lock()
            .unwrap()
            .iter()
            .map(|text| {
                serde_json::from_str::<Value>(text).unwrap()["id"]
                    .as_str()
                    .unwrap()
                    .to_owned()
            })
            .collect()
    }

    fn approve(&mut self, device: &str) {
        self.core.requests.approve(device, self.now.at);
    }

    /// The whole pairing exchange, as the watch runs it.
    fn pair(&mut self, device: &str) -> Watch {
        self.approve(device);
        let secret = watch_secret();
        let (status, body) = self
            .request(
                "POST",
                "/pairing/request",
                &[],
                &[],
                json!({
                    "device_id": device,
                    "name": "Test Watch",
                    "previous_token": "",
                    "e2ee_public_key": public_key_base64(&secret),
                })
                .to_string()
                .as_bytes(),
            )
            .unwrap();
        assert_eq!(status, 200, "{body}");
        assert_eq!(body["ok"], true);
        assert_eq!(body["name"], MAC_NAME);
        assert_eq!(body["device_id"], self.core.identity.mac_id);
        let request_id = body["request_id"].as_str().unwrap().to_owned();
        let (status, body) = self
            .request(
                "GET",
                "/pairing/result",
                &[("request_id", &request_id)],
                &[],
                &[],
            )
            .unwrap();
        assert_eq!(status, 200, "{body}");
        assert_eq!(body["approved"], true);
        let agreement =
            e2ee::agree_with(&secret, body["e2ee_public_key"].as_str().unwrap()).unwrap();
        let token = e2ee::open_pairing_result(
            body["sealed_result"].as_str().unwrap(),
            &agreement.key,
            &request_id,
        )
        .expect("the sealed token opens with the agreed key");
        Watch {
            device: device.to_owned(),
            token,
            key: agreement.key,
        }
    }

    fn device_headers<'a>(&self, watch: &'a Watch) -> Vec<(&'a str, &'a str)> {
        vec![(TOKEN_HEADER, watch.token.as_str()), (PROTOCOL_HEADER, "4")]
    }

    fn envelope(
        &mut self,
        watch: &Watch,
        path: &str,
        plaintext: &[u8],
        nonce_base64: &str,
        aad: &Aad<'_>,
    ) -> Option<Answer> {
        let nonce = e2ee::decode_nonce(nonce_base64).unwrap();
        let body = e2ee::seal(plaintext, &watch.key, &nonce, aad);
        let device = watch.device.clone();
        let (status, reply) = self.request(
            "POST",
            path,
            &[],
            &[
                (DEVICE_HEADER, device.as_str()),
                (NONCE_HEADER, nonce_base64),
                (PROTOCOL_HEADER, "4"),
            ],
            &body,
        )?;
        let Some(sealed) = reply.get("e2ee") else {
            return Some(Answer {
                status,
                body: reply,
                sealed: false,
            });
        };
        let response_nonce = e2ee::decode_nonce(sealed["nonce"].as_str().unwrap()).unwrap();
        let ciphertext = BASE64
            .decode(sealed["ciphertext"].as_str().unwrap())
            .unwrap();
        let plain = e2ee::open(
            &ciphertext,
            &response_nonce,
            &watch.key,
            &Aad::Response {
                device_id: &watch.device,
                request_nonce_base64: nonce_base64,
                status,
            },
        )
        .expect("the sealed reply opens with the pairing key");
        let mut body: Value = serde_json::from_slice(&plain).unwrap();
        assert_eq!(body["e2ee_actions"], true);
        body.as_object_mut().unwrap().remove("e2ee_actions");
        Some(Answer {
            status,
            body,
            sealed: true,
        })
    }

    fn fresh_nonce() -> String {
        BASE64.encode(e2ee::random_nonce().unwrap())
    }

    fn send_audio(&mut self, watch: &Watch, head: &AudioHead, m4a: &[u8]) -> Option<Answer> {
        let nonce = Self::fresh_nonce();
        let plaintext = e2ee::encode_audio(head, m4a).unwrap();
        self.envelope(
            watch,
            "/e2ee",
            &plaintext,
            &nonce,
            &Aad::Request {
                device_id: &watch.device,
                nonce_base64: &nonce,
            },
        )
    }

    fn query(&mut self, watch: &Watch, path: &str, query: &[(&str, &str)]) -> Option<Answer> {
        let nonce = Self::fresh_nonce();
        let head = e2ee::QueryHead {
            version: e2ee::VERSION,
            token: watch.token.clone(),
            sent_at: self.now.epoch(),
            path: path.to_owned(),
            query: query
                .iter()
                .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
                .collect(),
            requires_e2ee: None,
        };
        let plaintext = serde_json::to_vec(&head).unwrap();
        self.envelope(
            watch,
            "/e2ee/query",
            &plaintext,
            &nonce,
            &Aad::Query {
                device_id: &watch.device,
                nonce_base64: &nonce,
            },
        )
    }

    /// Sends a current-focus recording and returns once it is transcribing.
    fn start_recording(&mut self, watch: &Watch, id: &str) -> usize {
        let head = audio_head(watch, id, "current", self.now.epoch());
        assert!(
            self.send_audio(watch, &head, M4A).is_none(),
            "the reply waits for the delivery"
        );
        self.jobs.lock().unwrap().len() - 1
    }

    fn last_frame_for_request(&mut self) -> Answer {
        let id = self.pending_frame_ids().pop().expect("a frame went out");
        let (status, body) = self.take(&id).unwrap();
        Answer {
            status,
            body,
            sealed: false,
        }
    }

    fn open_last_sealed(&mut self, watch: &Watch, request_nonce: &str) -> Answer {
        let answer = self.last_frame_for_request();
        let sealed = &answer.body["e2ee"];
        let response_nonce = e2ee::decode_nonce(sealed["nonce"].as_str().unwrap()).unwrap();
        let ciphertext = BASE64
            .decode(sealed["ciphertext"].as_str().unwrap())
            .unwrap();
        let plain = e2ee::open(
            &ciphertext,
            &response_nonce,
            &watch.key,
            &Aad::Response {
                device_id: &watch.device,
                request_nonce_base64: request_nonce,
                status: answer.status,
            },
        )
        .unwrap();
        let mut body: Value = serde_json::from_slice(&plain).unwrap();
        body.as_object_mut().unwrap().remove("e2ee_actions");
        Answer {
            status: answer.status,
            body,
            sealed: true,
        }
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// Sends audio and keeps the request nonce so the deferred reply can be
/// opened later.
fn send_audio_keep_nonce(h: &mut Harness, watch: &Watch, head: &AudioHead) -> String {
    let nonce = Harness::fresh_nonce();
    let plaintext = e2ee::encode_audio(head, M4A).unwrap();
    let answer = h.envelope(
        watch,
        "/e2ee",
        &plaintext,
        &nonce,
        &Aad::Request {
            device_id: &watch.device,
            nonce_base64: &nonce,
        },
    );
    assert!(answer.is_none(), "the reply waits");
    nonce
}

#[test]
fn health_has_wristpers_shape() {
    let mut h = harness("health");
    let watch = h.pair("watch-1");
    let headers = h.device_headers(&watch);
    let (status, body) = h.request("GET", "/health", &[], &headers, &[]).unwrap();
    assert_eq!(status, 200);
    assert_eq!(
        keys(&body),
        [
            "ok",
            "paste_ready",
            "paste_detail",
            "model",
            "model_id",
            "model_name",
            "relay",
            "protocol",
            "min_peer_protocol",
            "app_version",
            "licensing"
        ]
        .into_iter()
        .collect()
    );
    assert_eq!(body["ok"], true);
    assert_eq!(body["paste_ready"], false, "Accessibility is not granted");
    assert!(
        body["paste_detail"]
            .as_str()
            .is_some_and(|detail| detail.contains("Accessibility"))
    );
    assert_eq!(body["model"], "cold");
    assert_eq!(body["model_id"], "parakeet-tdt-0.6b-v3");
    assert_eq!(body["model_name"], "Parakeet v3");
    assert_eq!(body["relay"], "offline");
    assert_eq!(body["protocol"], 4);
    assert_eq!(body["min_peer_protocol"], 1);
    assert_eq!(body["app_version"], APP_VERSION);
    assert_eq!(
        body["licensing"],
        json!({
            "state": "licensed",
            "daily_limit": null,
            "used_today": null,
            "remaining_today": null,
            "resets_at": null,
            "max_seconds": null,
            "license_status": "active",
        })
    );
    h.core.handle(
        Message::Relay(RelayEvent::State(RelayState::Connected)),
        h.now,
    );
    let (_, body) = h.request("GET", "/health", &[], &headers, &[]).unwrap();
    assert_eq!(body["relay"], "connected");
}

#[test]
fn device_routes_need_a_token_and_a_current_protocol() {
    let mut h = harness("auth");
    let (status, body) = h.request("GET", "/health", &[], &[], &[]).unwrap();
    assert_eq!((status, body["error"].as_str()), (403, Some("forbidden")));
    let (status, body) = h.request("GET", "/nothing", &[], &[], &[]).unwrap();
    assert_eq!((status, body["error"].as_str()), (404, Some("not-found")));
    let watch = h.pair("watch-1");
    let (status, body) = h
        .request(
            "GET",
            "/health",
            &[],
            &[(TOKEN_HEADER, &watch.token), (PROTOCOL_HEADER, "0")],
            &[],
        )
        .unwrap();
    assert_eq!(status, 426);
    assert_eq!(
        body,
        json!({
            "ok": false,
            "error": "watch_outdated",
            "min_protocol": 1,
            "mac_protocol": 4,
            "mac_version": APP_VERSION,
        })
    );
}

#[test]
fn focus_marks_awaiting_audio_and_a_late_focus_does_not_overwrite() {
    let mut h = harness("focus");
    let watch = h.pair("watch-1");
    let headers = h.device_headers(&watch);
    let (status, body) = h
        .request(
            "POST",
            "/focus",
            &[],
            &headers,
            br#"{"id":"6f9619ff-8b86-d011-b42d-00c04fc964ff"}"#,
        )
        .unwrap();
    assert_eq!(status, 200);
    assert_eq!(body, json!({"ok": true, "captured": false, "role": ""}));
    let (status, body) = h
        .request("GET", "/status", &[("id", ID)], &headers, &[])
        .unwrap();
    assert_eq!(status, 200);
    assert_eq!(body["state"], "awaiting-audio");
    assert!(
        body.get("text").is_none(),
        "plaintext /status hides the text"
    );
    assert!(body.get("agent_summary").is_none());

    h.start_recording(&watch, ID);
    let (status, body) = h
        .request(
            "POST",
            "/focus",
            &[],
            &headers,
            format!(r#"{{"id":"{ID}"}}"#).as_bytes(),
        )
        .unwrap();
    assert_eq!(status, 200);
    assert_eq!(
        body,
        json!({"ok": true, "captured": false, "state": "transcribing"})
    );

    let (status, body) = h
        .request("POST", "/focus", &[], &headers, br#"{"id":"short"}"#)
        .unwrap();
    assert_eq!((status, body["error"].as_str()), (400, Some("invalid-id")));
    let (status, body) = h
        .request("POST", "/focus", &[], &headers, b"not json")
        .unwrap();
    assert_eq!(
        (status, body["error"].as_str()),
        (400, Some("invalid-json"))
    );
}

#[test]
fn another_watch_cannot_touch_a_recording() {
    let mut h = harness("owner");
    let first = h.pair("watch-1");
    let second = h.pair("watch-2");
    let body = format!(r#"{{"id":"{ID}"}}"#);
    let headers = h.device_headers(&first);
    h.request("POST", "/focus", &[], &headers, body.as_bytes())
        .unwrap();
    let headers = h.device_headers(&second);
    let (status, reply) = h
        .request("POST", "/focus", &[], &headers, body.as_bytes())
        .unwrap();
    assert_eq!((status, reply["error"].as_str()), (403, Some("not-owner")));
    let head = audio_head(&second, ID, "current", h.now.epoch());
    let answer = h.send_audio(&second, &head, M4A).unwrap();
    assert_eq!(answer.status, 403);
    assert!(answer.sealed);
    assert_eq!(answer.body["error"], "not-owner");
}

/// Another paired watch, with its own token and key, learns nothing about
/// a recording it did not make: `/status` reads as an unknown id.
#[test]
fn status_hides_another_watchs_recording() {
    let mut h = harness("status-owner");
    let owner = h.pair("watch-1");
    let other = h.pair("watch-2");
    let index = h.start_recording(&owner, ID);
    h.deliver(index, report(DeliveryOutcome::Copied, "the owner's words"));
    h.frames.lock().unwrap().clear();

    let sealed = h.query(&other, "/status", &[("id", ID)]).unwrap();
    assert_eq!(
        (sealed.status, sealed.body["error"].as_str()),
        (404, Some("unknown-id")),
        "{}",
        sealed.body
    );
    let headers = h.device_headers(&other);
    let (status, body) = h
        .request("GET", "/status", &[("id", ID)], &headers, &[])
        .unwrap();
    assert_eq!((status, body["error"].as_str()), (404, Some("unknown-id")));

    let mine = h.query(&owner, "/status", &[("id", ID)]).unwrap();
    assert_eq!(mine.status, 200);
    assert_eq!(mine.body["text"], "the owner's words");
}

/// Swift's `isInsideOwnEnvelope`: a token from another watch inside an
/// envelope counts as plaintext, so it reads no text and passes no ratchet.
#[test]
fn an_envelope_only_speaks_for_the_watch_whose_key_opened_it() {
    let mut h = harness("envelope-binding");
    let mallory = h.pair("watch-1");
    let victim = h.pair("watch-2");
    let mut head = audio_head(&victim, ID, "current", h.now.epoch());
    head.requires_e2ee = Some(true);
    assert!(h.send_audio(&victim, &head, M4A).is_none());
    h.deliver(0, report(DeliveryOutcome::Copied, "the victim's words"));
    assert!(h.core.paired.requires_e2ee(&victim.device));
    let impostor = Watch {
        device: mallory.device.clone(),
        token: victim.token.clone(),
        key: mallory.key.clone(),
    };

    let status = h.query(&impostor, "/status", &[("id", ID)]).unwrap();
    assert_eq!((status.status, status.sealed), (200, true));
    assert_eq!(status.body["state"], "copied");
    assert!(status.body.get("text").is_none(), "{}", status.body);

    let focus = h.query(&impostor, "/focus", &[("id", OTHER_ID)]).unwrap();
    assert_eq!(
        (focus.status, focus.body["error"].as_str()),
        (403, Some("e2ee-required"))
    );
    assert!(!h.core.records.contains_key(OTHER_ID));

    let head = audio_head(&impostor, OTHER_ID, "current", h.now.epoch());
    let audio = h.send_audio(&impostor, &head, M4A).unwrap();
    assert_eq!(
        (audio.status, audio.body["error"].as_str()),
        (403, Some("e2ee-required"))
    );
    assert_eq!(h.jobs.lock().unwrap().len(), 1);
}

#[test]
fn plaintext_audio_is_always_e2ee_required() {
    let mut h = harness("plaintext-audio");
    let watch = h.pair("watch-1");
    let headers = h.device_headers(&watch);
    let (status, body) = h.request("POST", "/audio", &[], &headers, M4A).unwrap();
    assert_eq!(
        (status, body["error"].as_str()),
        (403, Some("e2ee-required"))
    );
    let (status, body) = h.request("POST", "/audio", &[], &[], M4A).unwrap();
    assert_eq!((status, body["error"].as_str()), (403, Some("forbidden")));
}

#[test]
fn the_ratchet_refuses_plaintext_actions_after_the_watch_proves_it() {
    let mut h = harness("ratchet");
    let watch = h.pair("watch-1");
    let headers = h.device_headers(&watch);
    let focus = format!(r#"{{"id":"{OTHER_ID}"}}"#);
    let (status, _) = h
        .request("POST", "/focus", &[], &headers, focus.as_bytes())
        .unwrap();
    assert_eq!(status, 200);
    let mut head = audio_head(&watch, ID, "current", h.now.epoch());
    head.requires_e2ee = Some(true);
    assert!(h.send_audio(&watch, &head, M4A).is_none());
    assert!(h.core.paired.requires_e2ee(&watch.device));
    let (status, body) = h
        .request("POST", "/focus", &[], &headers, focus.as_bytes())
        .unwrap();
    assert_eq!(
        (status, body["error"].as_str()),
        (403, Some("e2ee-required"))
    );
    let (status, _) = h.request("GET", "/health", &[], &headers, &[]).unwrap();
    assert_eq!(status, 200, "/health is not ratcheted");
    let answer = h.query(&watch, "/focus", &[("id", OTHER_ID)]).unwrap();
    assert_eq!(
        answer.status, 200,
        "inside the envelope the ratchet is satisfied"
    );
}

#[test]
fn envelopes_that_do_not_open_are_400_in_the_clear_and_replays_409() {
    let mut h = harness("envelope");
    let watch = h.pair("watch-1");
    let nonce = Harness::fresh_nonce();
    let plaintext =
        e2ee::encode_audio(&audio_head(&watch, ID, "current", h.now.epoch()), M4A).unwrap();
    let wrong_aad = h
        .envelope(
            &watch,
            "/e2ee",
            &plaintext,
            &nonce,
            &Aad::Query {
                device_id: &watch.device,
                nonce_base64: &nonce,
            },
        )
        .unwrap();
    assert_eq!((wrong_aad.status, wrong_aad.sealed), (400, false));
    assert_eq!(wrong_aad.body["error"], "invalid-e2ee");

    let stranger = Watch {
        device: watch.device.clone(),
        token: watch.token.clone(),
        key: SharedKey::from_base64(&BASE64.encode([7u8; 32])).unwrap(),
    };
    let nonce = Harness::fresh_nonce();
    let body = e2ee::seal(
        &plaintext,
        &stranger.key,
        &e2ee::decode_nonce(&nonce).unwrap(),
        &Aad::Request {
            device_id: &watch.device,
            nonce_base64: &nonce,
        },
    );
    let (status, reply) = h
        .request(
            "POST",
            "/e2ee",
            &[],
            &[(DEVICE_HEADER, &watch.device), (NONCE_HEADER, &nonce)],
            &body,
        )
        .unwrap();
    assert_eq!(
        (status, reply["error"].as_str()),
        (400, Some("invalid-e2ee"))
    );

    let (status, reply) = h
        .request(
            "POST",
            "/e2ee",
            &[],
            &[(DEVICE_HEADER, "nobody"), (NONCE_HEADER, &nonce)],
            &body,
        )
        .unwrap();
    assert_eq!(
        (status, reply["error"].as_str()),
        (400, Some("invalid-e2ee"))
    );

    let stale = audio_head(&watch, ID, "current", h.now.epoch() - 301.0);
    let answer = h.send_audio(&watch, &stale, M4A).unwrap();
    assert_eq!((answer.status, answer.sealed), (400, false));

    let nonce = Harness::fresh_nonce();
    let head = audio_head(&watch, ID, "current", h.now.epoch());
    let plaintext = e2ee::encode_audio(&head, M4A).unwrap();
    let aad = Aad::Request {
        device_id: &watch.device,
        nonce_base64: &nonce,
    };
    assert!(
        h.envelope(&watch, "/e2ee", &plaintext, &nonce, &aad)
            .is_none()
    );
    let replay = h
        .envelope(&watch, "/e2ee", &plaintext, &nonce, &aad)
        .unwrap();
    assert_eq!((replay.status, replay.sealed), (409, false));
    assert_eq!(replay.body["error"], "replayed");
}

#[test]
fn a_recording_is_transcribed_delivered_and_readable_through_status() {
    let mut h = harness("deliver");
    let watch = h.pair("watch-1");
    let head = audio_head(&watch, ID, "current", h.now.epoch());
    let nonce = send_audio_keep_nonce(&mut h, &watch, &head);
    let samples = h.jobs.lock().unwrap()[0].0.len();
    assert!((15_000..=20_000).contains(&samples), "{samples} samples");
    h.advance(Duration::from_millis(250));
    h.deliver(
        0,
        report(
            DeliveryOutcome::Sent(Verification::None),
            "hello from the watch",
        ),
    );
    let answer = h.open_last_sealed(&watch, &nonce);
    assert_eq!(answer.status, 200);
    assert_eq!(answer.body["ok"], true);
    assert_eq!(answer.body["state"], "unverified");
    assert_eq!(answer.body["verified"], false);
    assert_eq!(answer.body["verification"], "none");
    assert_eq!(answer.body["text"], "hello from the watch");
    assert_eq!(answer.body["transcribe_ms"], 120);
    assert_eq!(answer.body["paste_ms"], 30);
    assert_eq!(answer.body["receive_to_paste_ms"], 250);
    assert_eq!(
        keys(&answer.body),
        [
            "ok",
            "state",
            "verified",
            "verification",
            "text",
            "transcribe_ms",
            "paste_ms",
            "receive_to_paste_ms"
        ]
        .into_iter()
        .collect()
    );

    let status = h.query(&watch, "/status", &[("id", ID)]).unwrap();
    assert_eq!((status.status, status.sealed), (200, true));
    let expected: BTreeSet<&str> = "ok,id,state,language,verified,verification,entered,updated_at,route,transcribe_ms,paste_ms,receive_to_paste_ms,reason,reset,paste_reason,error,completed_at,completed_age,text,raw_text,agent_ms,session_id,agent_summary,agent_activity,agent_activity_at,agent_delivery"
        .split(',')
        .collect();
    assert_eq!(keys(&status.body), expected);
    assert_eq!(status.body["state"], "unverified");
    assert_eq!(status.body["language"], "en");
    assert_eq!(status.body["text"], "hello from the watch");
    assert_eq!(status.body["entered"], false);
    assert!(status.body["completed_at"].is_string());
    assert!(status.body["completed_age"].is_number());

    let headers = h.device_headers(&watch);
    let (status, body) = h
        .request("GET", "/status", &[("id", ID)], &headers, &[])
        .unwrap();
    assert_eq!(status, 200);
    assert!(body.get("text").is_none());
    assert_eq!(body["state"], "unverified");
    let (status, body) = h
        .request("GET", "/status", &[("id", OTHER_ID)], &headers, &[])
        .unwrap();
    assert_eq!((status, body["error"].as_str()), (404, Some("unknown-id")));
    let (status, body) = h
        .request("GET", "/status", &[("id", "nope")], &headers, &[])
        .unwrap();
    assert_eq!((status, body["error"].as_str()), (400, Some("invalid-id")));

    let delivered = crate::watch::delivered::load(&h.dir.join(DELIVERED_FILE));
    let entry = delivered
        .get(ID, SystemTime::now())
        .expect("the dedupe table was saved");
    assert_eq!(entry.state, "unverified");
}

/// Token, envelope, and owner are checked before any duplicate answer, so
/// another watch learns nothing from a recording id it guessed.
#[test]
fn duplicate_answers_come_only_after_token_and_owner_checks() {
    let mut h = harness("duplicate-owner");
    let owner = h.pair("watch-1");
    let other = h.pair("watch-2");
    let index = h.start_recording(&owner, ID);
    let resend = audio_head(&other, ID, "current", h.now.epoch());
    let while_transcribing = h.send_audio(&other, &resend, M4A).unwrap();
    assert_eq!(
        (
            while_transcribing.status,
            while_transcribing.body["error"].as_str()
        ),
        (403, Some("not-owner"))
    );

    h.deliver(index, report(DeliveryOutcome::Copied, "mine"));
    let delivered = h.send_audio(&other, &resend, M4A).unwrap();
    assert_eq!(
        (delivered.status, delivered.body["error"].as_str()),
        (403, Some("not-owner"))
    );

    let mut late = resend.clone();
    late.background = Some(true);
    let late_other = h.send_audio(&other, &late, M4A).unwrap();
    assert_eq!(
        (late_other.status, late_other.body["error"].as_str()),
        (403, Some("not-owner"))
    );
    late.token = "not-a-token".to_owned();
    let late_unknown = h.send_audio(&other, &late, M4A).unwrap();
    assert_eq!(
        (late_unknown.status, late_unknown.body["error"].as_str()),
        (403, Some("forbidden"))
    );
    assert_eq!(h.jobs.lock().unwrap().len(), 1);
}

/// Once the live record is pruned or Hex restarts, the dedupe table still
/// names the owner, so another watch gets neither a duplicate nor a status.
/// An entry Swift wrote has no owner and answers any watch, as before.
#[test]
fn ownership_survives_in_the_dedupe_table() {
    let mut h = harness("dedupe-owner");
    let owner = h.pair("watch-1");
    let other = h.pair("watch-2");
    let index = h.start_recording(&owner, ID);
    h.deliver(index, report(DeliveryOutcome::Copied, "mine"));
    h.frames.lock().unwrap().clear();
    h.core.records.clear();
    h.core.nonces = e2ee::NonceCache::default();
    h.core.delivered = crate::watch::delivered::load(&h.dir.join(DELIVERED_FILE));

    let status = h.query(&other, "/status", &[("id", ID)]).unwrap();
    assert_eq!(
        (status.status, status.body["error"].as_str()),
        (404, Some("unknown-id"))
    );
    let resend = audio_head(&other, ID, "current", h.now.epoch());
    let mut late = resend.clone();
    late.background = Some(true);
    for head in [&resend, &late] {
        let answer = h.send_audio(&other, head, M4A).unwrap();
        assert_eq!(
            (answer.status, answer.body["error"].as_str()),
            (403, Some("not-owner")),
            "{}",
            answer.body
        );
    }
    let focus = format!(r#"{{"id":"{ID}"}}"#);
    let headers = h.device_headers(&other);
    let (code, body) = h
        .request("POST", "/focus", &[], &headers, focus.as_bytes())
        .unwrap();
    assert_eq!((code, body["error"].as_str()), (403, Some("not-owner")));

    let mine = h.query(&owner, "/status", &[("id", ID)]).unwrap();
    assert_eq!(
        (mine.status, mine.body["state"].as_str()),
        (200, Some("copied"))
    );
    let again = audio_head(&owner, ID, "current", h.now.epoch());
    let duplicate = h.send_audio(&owner, &again, M4A).unwrap();
    assert_eq!(duplicate.body["duplicate"], true);
    assert_eq!(h.jobs.lock().unwrap().len(), 1);

    let swift = format!(
        r#"[{{"id":"{OTHER_ID}","state":"pasted","verified":true,"verification":"read-back",
            "truncated":false,"entered":false,"recorded_at":"{}"}}]"#,
        crate::watch::Timestamp::string(h.now.wall)
    );
    h.core.delivered = crate::watch::delivered::DeliveredIds::from_json(swift.as_bytes());
    let open = h.query(&other, "/status", &[("id", OTHER_ID)]).unwrap();
    assert_eq!(
        (open.status, open.body["state"].as_str()),
        (200, Some("pasted"))
    );
}

#[test]
fn a_duplicate_waits_for_the_first_copy_then_answers_from_it() {
    let mut h = harness("duplicate");
    let watch = h.pair("watch-1");
    h.start_recording(&watch, ID);
    let head = audio_head(&watch, ID, "current", h.now.epoch());
    let second_nonce = send_audio_keep_nonce(&mut h, &watch, &head);
    assert_eq!(
        h.jobs.lock().unwrap().len(),
        1,
        "the duplicate starts nothing"
    );
    h.deliver(0, report(DeliveryOutcome::Sent(Verification::None), "once"));
    let duplicate = h.open_last_sealed(&watch, &second_nonce);
    assert_eq!(duplicate.status, 200);
    assert_eq!(
        duplicate.body,
        json!({"ok": true, "duplicate": true, "state": "unverified", "verified": false, "verification": "none"})
    );
    assert!(
        h.pending_frame_ids().len() == 1,
        "the first copy's reply went out too"
    );

    let again = h.send_audio(&watch, &head, M4A).unwrap();
    assert_eq!(again.status, 200);
    assert_eq!(again.body["duplicate"], true);
    assert_eq!(again.body["state"], "unverified");

    h.core.records.clear();
    let from_table = h.send_audio(&watch, &head, M4A).unwrap();
    assert_eq!(from_table.body["duplicate"], true);
    assert_eq!(from_table.body["state"], "unverified");
    assert_eq!(h.jobs.lock().unwrap().len(), 1);
}

#[test]
fn waiting_replies_time_out_with_transcribing() {
    let mut h = harness("timeouts");
    let watch = h.pair("watch-1");
    let head = audio_head(&watch, ID, "current", h.now.epoch());
    let first = send_audio_keep_nonce(&mut h, &watch, &head);
    let second = send_audio_keep_nonce(&mut h, &watch, &head);
    h.advance(Duration::from_secs(DUPLICATE_AUDIO_WAIT_SECONDS) - Duration::from_millis(1));
    h.core.tick(h.now);
    assert!(h.pending_frame_ids().is_empty());
    h.advance(Duration::from_millis(1));
    h.core.tick(h.now);
    let duplicate = h.open_last_sealed(&watch, &second);
    assert_eq!(
        duplicate.body,
        json!({"ok": true, "duplicate": true, "state": "transcribing"})
    );
    h.advance(TRANSCRIPT_WAIT - Duration::from_secs(DUPLICATE_AUDIO_WAIT_SECONDS));
    h.core.tick(h.now);
    let transcript = h.open_last_sealed(&watch, &first);
    assert_eq!(transcript.status, 200);
    assert_eq!(
        transcript.body,
        json!({"ok": true, "state": "transcribing"})
    );
    h.deliver(0, report(DeliveryOutcome::Sent(Verification::None), "late"));
    assert!(
        h.pending_frame_ids().is_empty(),
        "nothing is left to answer"
    );
    let status = h.query(&watch, "/status", &[("id", ID)]).unwrap();
    assert_eq!(status.body["state"], "unverified");
}

/// An inbox that never empties must not hold back a due deadline: the
/// clock moves 100 ms per reading while a thousand messages queue ahead of
/// `Stop`, and the `/focus` and duplicate waits still end on time.
#[test]
fn deadlines_fire_while_the_inbox_stays_busy() {
    let mut h = harness("busy-inbox");
    let watch = h.pair("watch-1");
    let head = audio_head(&watch, ID, "current", h.now.epoch());
    send_audio_keep_nonce(&mut h, &watch, &head);
    let duplicate = send_audio_keep_nonce(&mut h, &watch, &head);
    let focus = audio_head(&watch, OTHER_ID, "requested", h.now.epoch());
    assert!(h.send_audio(&watch, &focus, M4A).is_none());
    assert_eq!(h.jobs.lock().unwrap().len(), 1);

    let inbox = h.core.inbox.clone();
    for _ in 0..1_000 {
        let (reply, _) = mpsc::sync_channel(1);
        inbox.send(Message::Snapshot { reply }).unwrap();
    }
    inbox.send(Message::Stop).unwrap();
    let start = h.now;
    let mut readings = 0u32;
    let clock = move || {
        readings += 1;
        let by = Duration::from_millis(100) * readings;
        Now {
            at: start.at + by,
            wall: start.wall + by,
        }
    };
    serve(&mut h.core, &h.messages, clock);

    assert_eq!(
        h.jobs.lock().unwrap().len(),
        2,
        "the /focus wait ended and its audio ran"
    );
    assert_eq!(
        h.pending_frame_ids().len(),
        3,
        "the duplicate and both transcript waits were answered"
    );
    h.frames.lock().unwrap().truncate(1);
    let answer = h.open_last_sealed(&watch, &duplicate);
    assert_eq!(
        answer.body,
        json!({"ok": true, "duplicate": true, "state": "transcribing"})
    );
}

#[test]
fn audio_that_asked_for_focus_waits_two_seconds_for_it() {
    let mut h = harness("focus-wait");
    let watch = h.pair("watch-1");
    let head = audio_head(&watch, ID, "requested", h.now.epoch());
    assert!(h.send_audio(&watch, &head, M4A).is_none());
    assert!(
        h.jobs.lock().unwrap().is_empty(),
        "nothing runs before /focus"
    );
    h.advance(Duration::from_millis(1999));
    h.core.tick(h.now);
    assert!(h.jobs.lock().unwrap().is_empty());
    h.advance(Duration::from_millis(1));
    h.core.tick(h.now);
    assert_eq!(
        h.jobs.lock().unwrap().len(),
        1,
        "the wait ended and the audio ran"
    );

    let head = audio_head(&watch, OTHER_ID, "requested", h.now.epoch());
    assert!(h.send_audio(&watch, &head, M4A).is_none());
    assert_eq!(h.jobs.lock().unwrap().len(), 1);
    let answer = h.query(&watch, "/focus", &[("id", OTHER_ID)]).unwrap();
    assert_eq!(answer.body["captured"], false);
    assert_eq!(
        h.jobs.lock().unwrap().len(),
        2,
        "/focus released the waiting audio"
    );
}

#[test]
fn undecodable_audio_and_refused_dictation_fail_with_500() {
    let mut h = harness("decode");
    let watch = h.pair("watch-1");
    let head = audio_head(&watch, ID, "current", h.now.epoch());
    let answer = h
        .send_audio(&watch, &head, b"this is not an m4a file at all")
        .unwrap();
    assert_eq!((answer.status, answer.sealed), (500, true));
    assert_eq!(answer.body["ok"], false);
    assert_eq!(answer.body["state"], "failed");
    assert!(
        answer.body["error"]
            .as_str()
            .unwrap()
            .starts_with("The recording could not be read")
    );
    let status = h.query(&watch, "/status", &[("id", ID)]).unwrap();
    assert_eq!(status.body["state"], "failed");
    assert_eq!(status.body["error"], answer.body["error"]);

    h.refuse.store(true, Ordering::SeqCst);
    let head = audio_head(&watch, OTHER_ID, "current", h.now.epoch());
    let answer = h.send_audio(&watch, &head, M4A).unwrap();
    assert_eq!(answer.status, 500);
    assert_eq!(answer.body["error"], "the listener is not running");
}

#[test]
fn a_failed_delivery_answers_500_and_a_suspect_one_its_reason() {
    let mut h = harness("failed-delivery");
    let watch = h.pair("watch-1");
    let head = audio_head(&watch, ID, "current", h.now.epoch());
    let nonce = send_audio_keep_nonce(&mut h, &watch, &head);
    h.deliver(0, DeliveryReport::failed("the model crashed"));
    let answer = h.open_last_sealed(&watch, &nonce);
    assert_eq!(answer.status, 500);
    assert_eq!(
        answer.body,
        json!({"ok": false, "state": "failed", "error": "the model crashed"})
    );

    let mut head = audio_head(&watch, OTHER_ID, "current", h.now.epoch());
    head.language = "zh".to_owned();
    let nonce = send_audio_keep_nonce(&mut h, &watch, &head);
    h.deliver(
        1,
        report(DeliveryOutcome::Suspect(SuspectReason::Empty), ""),
    );
    let answer = h.open_last_sealed(&watch, &nonce);
    assert_eq!(answer.status, 200);
    assert_eq!(
        answer.body,
        json!({"ok": true, "state": "suspect", "text": "", "reason": SuspectReason::Empty.text(true)})
    );
    let status = h.query(&watch, "/status", &[("id", OTHER_ID)]).unwrap();
    assert_eq!(status.body["reason"], SuspectReason::Empty.text(true));
}

#[test]
fn an_agent_route_is_refused_rather_than_downgraded() {
    let mut h = harness("route");
    let watch = h.pair("watch-1");
    let mut head = audio_head(&watch, ID, "current", h.now.epoch());
    head.route = Some("agent".to_owned());
    let answer = h.send_audio(&watch, &head, M4A).unwrap();
    assert_eq!((answer.status, answer.sealed), (400, true));
    assert_eq!(answer.body["error"], "invalid-route");
    let mut head = audio_head(&watch, OTHER_ID, "elsewhere", h.now.epoch());
    head.route = None;
    let answer = h.send_audio(&watch, &head, M4A).unwrap();
    assert_eq!(answer.body["error"], "invalid-metadata");
    assert!(h.jobs.lock().unwrap().is_empty());
}

#[test]
fn pairing_is_closed_without_an_approval_and_an_old_watch_keeps_it() {
    let mut h = harness("pairing");
    let secret = watch_secret();
    let request = json!({"device_id": "watch-1", "name": "Old Watch", "e2ee_public_key": public_key_base64(&secret)});
    let (status, body) = h
        .request(
            "POST",
            "/pairing/request",
            &[],
            &[],
            request.to_string().as_bytes(),
        )
        .unwrap();
    assert_eq!(
        (status, body["error"].as_str()),
        (403, Some("pairing-closed"))
    );

    h.approve("watch-1");
    let (status, body) = h
        .request(
            "POST",
            "/pairing/request",
            &[],
            &[],
            br#"{"device_id":"watch-1","name":"Old Watch"}"#,
        )
        .unwrap();
    assert_eq!(status, 426);
    assert_eq!(body["error"], "watch_outdated");
    assert_eq!(body["min_protocol"], 4);
    assert!(
        !h.core.requests.window_remaining(h.now.at).is_zero(),
        "the approval is not spent"
    );

    let (status, body) = h
        .request(
            "POST",
            "/pairing/request",
            &[],
            &[],
            json!({"device_id": "watch-2", "name": "Other", "e2ee_public_key": public_key_base64(&secret)})
                .to_string()
                .as_bytes(),
        )
        .unwrap();
    assert_eq!(
        (status, body["error"].as_str()),
        (403, Some("pairing-closed"))
    );

    let (status, body) = h
        .request(
            "GET",
            "/pairing/result",
            &[("request_id", "nope")],
            &[],
            &[],
        )
        .unwrap();
    assert_eq!(
        (status, body["error"].as_str()),
        (404, Some("unknown-request"))
    );

    let watch = h.pair("watch-1");
    let headers = h.device_headers(&watch);
    let (status, _) = h.request("GET", "/health", &[], &headers, &[]).unwrap();
    assert_eq!(status, 200, "the issued token authenticates");
    assert_eq!(h.core.requests.completed_count(), 1);
}

#[test]
fn claims_answer_ok_failed_timeout_and_offline() {
    let mut h = harness("claims");
    let claim = |h: &mut Harness, code: &str| {
        let (reply, answer) = mpsc::sync_channel(1);
        h.core.handle(
            Message::Claim {
                code: code.to_owned(),
                reply,
                deadline: h.now.at + CLAIM_TIMEOUT,
            },
            h.now,
        );
        answer
    };
    let offline = claim(&mut h, "111111");
    assert_eq!(offline.try_recv().unwrap(), Err(ClaimError::RelayOffline));

    h.core.handle(
        Message::Relay(RelayEvent::State(RelayState::Connected)),
        h.now,
    );
    let ok = claim(&mut h, "217400");
    let frame: Value = serde_json::from_str(&h.frames.lock().unwrap().pop().unwrap()).unwrap();
    assert_eq!(
        frame,
        json!({"type": "claim", "code": "217400", "name": MAC_NAME})
    );
    h.core.handle(
        Message::Relay(RelayEvent::Inbound(Inbound::ClaimOk {
            code: "217400".to_owned(),
            device_id: "watch-9".to_owned(),
            device_name: "Series 9".to_owned(),
        })),
        h.now,
    );
    assert_eq!(
        ok.try_recv().unwrap(),
        Ok(ClaimOk {
            device_id: "watch-9".to_owned(),
            device_name: "Series 9".to_owned()
        })
    );
    assert!(
        !h.core.requests.window_remaining(h.now.at).is_zero(),
        "the claim approved the watch"
    );

    let failed = claim(&mut h, "333333");
    h.frames.lock().unwrap().clear();
    h.core.handle(
        Message::Relay(RelayEvent::Inbound(Inbound::ClaimFailed {
            code: "333333".to_owned(),
            error: "too_many_attempts".to_owned(),
        })),
        h.now,
    );
    assert_eq!(failed.try_recv().unwrap(), Err(ClaimError::TooManyAttempts));
    let other = claim(&mut h, "444444");
    h.core.handle(
        Message::Relay(RelayEvent::Inbound(Inbound::ClaimFailed {
            code: "444444".to_owned(),
            error: "busy".to_owned(),
        })),
        h.now,
    );
    assert_eq!(
        other.try_recv().unwrap(),
        Err(ClaimError::Relay("busy".to_owned()))
    );

    let slow = claim(&mut h, "555555");
    let replaced = claim(&mut h, "555555");
    assert_eq!(
        slow.try_recv().unwrap(),
        Err(ClaimError::Timeout),
        "a repeated code times the first out"
    );
    h.advance(CLAIM_TIMEOUT);
    h.core.tick(h.now);
    assert_eq!(replaced.try_recv().unwrap(), Err(ClaimError::Timeout));

    let dropped = claim(&mut h, "666666");
    h.core.handle(
        Message::Relay(RelayEvent::State(RelayState::Offline)),
        h.now,
    );
    assert_eq!(dropped.try_recv().unwrap(), Err(ClaimError::RelayOffline));

    h.core.handle(
        Message::Relay(RelayEvent::State(RelayState::Connected)),
        h.now,
    );
    h.connected.store(false, Ordering::SeqCst);
    let unsent = claim(&mut h, "777777");
    assert_eq!(unsent.try_recv().unwrap(), Err(ClaimError::RelayOffline));
}

/// A claim is bounded by its caller's deadline, not by when the thread
/// got to it: one that waited in the inbox past it never reaches the
/// relay, and one that waited part of it gets only the rest.
#[test]
fn a_claim_is_never_sent_after_its_caller_gave_up() {
    let mut h = harness("claim-deadline");
    h.core.handle(
        Message::Relay(RelayEvent::State(RelayState::Connected)),
        h.now,
    );
    let claim = |h: &mut Harness, code: &str, queued: Duration| {
        let (reply, answer) = mpsc::sync_channel(1);
        let deadline = h.now.at + CLAIM_TIMEOUT;
        h.advance(queued);
        h.core.handle(
            Message::Claim {
                code: code.to_owned(),
                reply,
                deadline,
            },
            h.now,
        );
        answer
    };

    let expired = claim(&mut h, "123456", CLAIM_WAIT);
    assert!(
        h.frames.lock().unwrap().is_empty(),
        "no claim frame went out"
    );
    assert_eq!(expired.try_recv().unwrap(), Err(ClaimError::Timeout));
    h.core.handle(
        Message::Relay(RelayEvent::Inbound(Inbound::ClaimOk {
            code: "123456".to_owned(),
            device_id: "watch-9".to_owned(),
            device_name: "Series 9".to_owned(),
        })),
        h.now,
    );
    assert!(
        h.core.requests.window_remaining(h.now.at).is_zero(),
        "a late answer approves no watch"
    );

    let late = claim(&mut h, "654321", Duration::from_secs(4));
    assert_eq!(h.frames.lock().unwrap().len(), 1, "the claim went out");
    h.advance(CLAIM_TIMEOUT - Duration::from_secs(4));
    h.core.tick(h.now);
    assert_eq!(
        late.try_recv().unwrap(),
        Err(ClaimError::Timeout),
        "the relay wait ends at the caller's deadline"
    );
}

#[test]
fn a_claim_answer_handled_after_the_deadline_approves_no_watch() {
    let mut h = harness("claim-late-answer");
    h.core.handle(
        Message::Relay(RelayEvent::State(RelayState::Connected)),
        h.now,
    );
    let (reply, answer) = mpsc::sync_channel(1);
    h.core.handle(
        Message::Claim {
            code: "123456".to_owned(),
            reply,
            deadline: h.now.at + CLAIM_TIMEOUT,
        },
        h.now,
    );
    assert_eq!(h.frames.lock().unwrap().len(), 1, "the claim went out");
    h.advance(CLAIM_TIMEOUT + Duration::from_millis(1));
    h.core.handle(
        Message::Relay(RelayEvent::Inbound(Inbound::ClaimOk {
            code: "123456".to_owned(),
            device_id: "watch-9".to_owned(),
            device_name: "Series 9".to_owned(),
        })),
        h.now,
    );
    assert_eq!(answer.try_recv().unwrap(), Err(ClaimError::Timeout));
    assert!(
        h.core.requests.window_remaining(h.now.at).is_zero(),
        "an answer handled past the deadline approves no watch"
    );
}

#[test]
fn a_background_upload_replayed_a_day_later_reads_duplicate() {
    let mut h = harness("background");
    let watch = h.pair("watch-1");
    let mut head = audio_head(&watch, ID, "current", h.now.epoch() - 3_600.0);
    head.background = Some(true);
    assert!(
        h.send_audio(&watch, &head, M4A).is_none(),
        "an hour-old background upload is fresh"
    );
    h.deliver(0, report(DeliveryOutcome::Copied, "copied"));
    h.frames.lock().unwrap().clear();
    let replay = h.send_audio(&watch, &head, M4A).unwrap();
    assert_eq!(
        replay.body,
        json!({"ok": true, "duplicate": true, "state": "copied"})
    );
    h.core.records.clear();
    h.core.nonces = e2ee::NonceCache::default();
    let from_table = h.send_audio(&watch, &head, M4A).unwrap();
    assert_eq!(
        from_table.body,
        json!({"ok": true, "duplicate": true, "state": "copied"})
    );
    assert_eq!(h.jobs.lock().unwrap().len(), 1);
    let mut too_old = head.clone();
    too_old.sent_at = h.now.epoch() - 86_401.0;
    let refused = h.send_audio(&watch, &too_old, M4A).unwrap();
    assert_eq!((refused.status, refused.sealed), (400, false));
}

#[test]
fn stale_actions_and_unknown_recordings_inside_the_envelope() {
    let mut h = harness("query");
    let watch = h.pair("watch-1");
    let enter = h.query(&watch, "/enter", &[("id", ID)]).unwrap();
    assert_eq!(enter.status, 200);
    assert_eq!(
        enter.body,
        json!({"ok": false, "reason": "unknown-id", "error": "unknown-id"})
    );
    let undo = h.query(&watch, "/undo", &[("id", ID)]).unwrap();
    assert_eq!(undo.body, json!({"ok": false, "reason": "unknown-id"}));

    let nonce = Harness::fresh_nonce();
    let head = e2ee::QueryHead {
        version: e2ee::VERSION,
        token: watch.token.clone(),
        sent_at: h.now.epoch() - 61.0,
        path: "/focus".to_owned(),
        query: BTreeMap::from([("id".to_owned(), ID.to_owned())]),
        requires_e2ee: None,
    };
    let stale = h
        .envelope(
            &watch,
            "/e2ee/query",
            &serde_json::to_vec(&head).unwrap(),
            &nonce,
            &Aad::Query {
                device_id: &watch.device,
                nonce_base64: &nonce,
            },
        )
        .unwrap();
    assert_eq!((stale.status, stale.sealed), (409, true));
    assert_eq!(stale.body["error"], "confirm-stale");
    assert!(
        !h.core.records.contains_key(ID),
        "the stale /focus did nothing"
    );

    let unknown = e2ee::QueryHead {
        path: "/agent/projects".to_owned(),
        sent_at: h.now.epoch(),
        ..head.clone()
    };
    let nonce = Harness::fresh_nonce();
    let refused = h
        .envelope(
            &watch,
            "/e2ee/query",
            &serde_json::to_vec(&unknown).unwrap(),
            &nonce,
            &Aad::Query {
                device_id: &watch.device,
                nonce_base64: &nonce,
            },
        )
        .unwrap();
    assert_eq!((refused.status, refused.sealed), (400, false));
}

#[test]
fn rejected_frames_and_lost_replies_do_not_stall_the_thread() {
    let mut h = harness("rejected");
    h.core.handle(
        Message::Relay(RelayEvent::Inbound(Inbound::Rejected {
            id: "bad".to_owned(),
            status: 413,
            error: "too-large",
        })),
        h.now,
    );
    assert_eq!(
        h.take("bad").unwrap(),
        (413, json!({"ok": false, "error": "too-large"}))
    );
    let watch = h.pair("watch-1");
    h.connected.store(false, Ordering::SeqCst);
    let headers = h.device_headers(&watch);
    assert!(h.request("GET", "/health", &[], &headers, &[]).is_none());
    h.connected.store(true, Ordering::SeqCst);
    assert_eq!(
        h.request("GET", "/health", &[], &headers, &[]).unwrap().0,
        200
    );
}

#[test]
fn finished_records_are_pruned_but_the_table_still_answers() {
    let mut h = harness("prune");
    let watch = h.pair("watch-1");
    for index in 0..(MAX_LIVE_RECORDS + 5) {
        let id = format!("{index:08x}-0000-4000-8000-000000000000");
        let head = audio_head(&watch, &id, "current", h.now.epoch());
        assert!(h.send_audio(&watch, &head, M4A).is_none());
        h.deliver(
            index,
            report(DeliveryOutcome::Sent(Verification::None), "x"),
        );
        h.advance(Duration::from_millis(1));
    }
    h.core.tick(h.now);
    assert_eq!(h.core.records.len(), MAX_LIVE_RECORDS);
    assert!(
        !h.core
            .records
            .contains_key("00000000-0000-4000-8000-000000000000")
    );
    let headers = h.device_headers(&watch);
    let (status, body) = h
        .request(
            "GET",
            "/status",
            &[("id", "00000000-0000-4000-8000-000000000000")],
            &headers,
            &[],
        )
        .unwrap();
    assert_eq!((status, body["state"].as_str()), (200, Some("unverified")));
    h.advance(Duration::from_secs(86_401));
    h.core.tick(h.now);
    assert!(h.core.records.is_empty());
}

#[test]
fn snapshot_and_stop_reach_the_core() {
    let mut h = harness("snapshot");
    h.pair("watch-1");
    let (reply, answer) = mpsc::sync_channel(1);
    h.core.handle(Message::Snapshot { reply }, h.now);
    let snapshot = answer.try_recv().unwrap();
    assert_eq!(snapshot.relay, RelayState::Offline);
    assert_eq!(snapshot.devices.len(), 1);
    assert_eq!(snapshot.devices[0].name, "Test Watch");
    assert!(matches!(
        h.core.handle(Message::Stop, h.now),
        super::state::Flow::Exit
    ));
}

#[test]
fn device_id_file_is_a_lowercase_uuid_kept_across_runs() {
    let dir = temp_dir("device-id");
    let path = dir.join(DEVICE_ID_FILE);
    let first = read_or_create_device_id(&path).unwrap();
    assert!(is_uuid(&first));
    assert_eq!(first, first.to_lowercase());
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        format!("{first}\n")
    );
    assert_eq!(read_or_create_device_id(&path).unwrap(), first);
    std::fs::write(&path, "  6F9619FF-8B86-D011-B42D-00C04FC964FF\n").unwrap();
    assert_eq!(
        read_or_create_device_id(&path).unwrap(),
        "6F9619FF-8B86-D011-B42D-00C04FC964FF",
        "Wristper's file is kept as written"
    );
    std::fs::write(&path, "junk").unwrap();
    let replaced = read_or_create_device_id(&path).unwrap();
    assert!(is_uuid(&replaced) && replaced != first);
    let _ = std::fs::remove_dir_all(&dir);
}

#[cfg(target_os = "macos")]
#[test]
fn listener_dictation_names_a_full_queue_and_a_gone_listener() {
    let (controls, queue) = mpsc::sync_channel(1);
    let dictation = ListenerDictation::new(controls);
    let (sink, _reports) = mpsc::channel();
    let delivery = |sink: &mpsc::Sender<crate::watch::delivery::DeliveryEvent>| {
        let sink = sink.clone();
        WatchDelivery::new(
            crate::watch::delivery::Job {
                id: ID.to_owned(),
                language: "en".to_owned(),
                expectation: crate::watch::field::Expectation::Current,
                desktop: Desktop::null(),
            },
            move |event| {
                let _ = sink.send(event);
            },
        )
    };
    assert_eq!(dictation.transcribe(vec![0.0; 16], delivery(&sink)), Ok(()));
    assert_eq!(
        dictation.transcribe(vec![0.0; 16], delivery(&sink)),
        Err("the dictation queue is full".to_owned())
    );
    drop(queue);
    assert_eq!(
        dictation.transcribe(vec![0.0; 16], delivery(&sink)),
        Err("the listener is not running".to_owned())
    );
}

#[test]
fn a_terminal_paste_is_final_and_a_read_back_upgrades_an_unverified_one() {
    use crate::watch::delivery::DeliveryOutcome::Sent;
    let mut h = harness("terminal-and-read-back");
    let watch = h.pair("watch-1");
    let head = audio_head(&watch, ID, "current", h.now.epoch());
    h.send_audio(&watch, &head, M4A);
    h.deliver(0, report(Sent(Verification::Terminal), "ls -la"));
    let status = h.status_of(&watch, ID);
    assert_eq!(status["state"], "pasted");
    assert_eq!(status["verified"], true);
    assert_eq!(status["verification"], "terminal");
    // A terminal paste is never read back; a stray confirmation is ignored.
    h.confirm(0, true);
    assert_eq!(h.status_of(&watch, ID)["verification"], "terminal");

    let head = audio_head(&watch, OTHER_ID, "current", h.now.epoch());
    h.send_audio(&watch, &head, M4A);
    h.deliver(1, report(Sent(Verification::None), "hello"));
    assert_eq!(h.status_of(&watch, OTHER_ID)["state"], "unverified");
    h.advance(Duration::from_millis(800));
    h.confirm(1, true);
    let status = h.status_of(&watch, OTHER_ID);
    assert_eq!(status["state"], "pasted");
    assert_eq!(status["verified"], true);
    assert_eq!(status["verification"], "read-back");
    let remembered = h.core.delivered.get(OTHER_ID, h.now.wall).unwrap();
    assert_eq!(
        (remembered.state.as_str(), remembered.verified),
        ("pasted", true)
    );
    assert_eq!(remembered.verification, "read-back");
}

#[test]
fn a_failed_read_back_leaves_the_paste_unverified() {
    let mut h = harness("read-back-failed");
    let watch = h.pair("watch-1");
    let head = audio_head(&watch, ID, "current", h.now.epoch());
    h.send_audio(&watch, &head, M4A);
    h.deliver(
        0,
        report(DeliveryOutcome::Sent(Verification::None), "hello"),
    );
    h.confirm(0, false);
    let status = h.status_of(&watch, ID);
    assert_eq!(status["state"], "unverified");
    assert_eq!(status["verified"], false);
    let remembered = h.core.delivered.get(ID, h.now.wall).unwrap();
    assert_eq!(remembered.state, "unverified");
}

#[test]
fn a_withheld_paste_keeps_the_text_and_names_why() {
    use crate::watch::field::Withheld;
    let mut h = harness("withheld");
    let watch = h.pair("watch-1");
    let cases = [
        (
            Withheld::TargetChanged,
            "target-changed",
            Value::Null,
            false,
        ),
        (Withheld::NoSnapshot, "target-changed", Value::Null, true),
        (Withheld::NotEditable, "not-editable", Value::Null, false),
        (
            Withheld::SecureInput,
            "sensitive",
            json!("secure-input"),
            false,
        ),
        (Withheld::PasswordField, "sensitive", Value::Null, false),
    ];
    for (index, (withheld, state, paste_reason, has_reason)) in cases.into_iter().enumerate() {
        let id = format!("00000000-0000-4000-8000-00000000000{index}");
        let head = audio_head(&watch, &id, "current", h.now.epoch());
        let nonce = send_audio_keep_nonce(&mut h, &watch, &head);
        h.deliver(
            index,
            report(DeliveryOutcome::Withheld(withheld), "kept words"),
        );
        let answer = h.open_last_sealed(&watch, &nonce);
        assert_eq!(answer.body["state"], state, "{withheld:?}");
        assert_eq!(answer.body["ok"], true);
        assert_eq!(answer.body["text"], "kept words");
        assert_eq!(
            answer
                .body
                .get("paste_reason")
                .cloned()
                .unwrap_or(Value::Null),
            paste_reason
        );
        let status = h.status_of(&watch, &id);
        assert_eq!(status["state"], state, "{withheld:?}");
        assert_eq!(status["text"], "kept words");
        assert_eq!(status["paste_reason"], paste_reason);
        assert_eq!(status["reason"].is_string(), has_reason, "{withheld:?}");
        assert_eq!(status["verified"], false);
    }
    // Withheld is not delivered: the watch's resend (`focus: current`) is
    // transcribed again instead of answered `duplicate`.
    let jobs_before = h.jobs.lock().unwrap().len();
    let head = audio_head(
        &watch,
        "00000000-0000-4000-8000-000000000000",
        "current",
        h.now.epoch(),
    );
    let answer = h.send_audio(&watch, &head, M4A);
    assert!(answer.is_none(), "the resend waits for its transcript");
    assert_eq!(h.jobs.lock().unwrap().len(), jobs_before + 1);
}
