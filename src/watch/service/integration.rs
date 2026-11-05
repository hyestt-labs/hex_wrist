//! The whole service against an in-process fake relay: a watch pairs,
//! records, and reads its status exactly as Wristper's watch app does.

use std::collections::BTreeSet;
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use p256::SecretKey;
use p256::elliptic_curve::sec1::ToSec1Point;
use serde_json::{Value, json};
use tungstenite::{Message as Frame, WebSocket};

use super::*;
use crate::watch::delivery::{DeliveryOutcome, DeliveryReport};
use crate::watch::e2ee::{self, Aad, AudioHead, QueryHead, SharedKey};
use crate::watch::pairing::MemorySecretStore;
use crate::watch::relay::RelayState;
use crate::watch::{DEVICE_HEADER, NONCE_HEADER, PROTOCOL_HEADER, TOKEN_HEADER};

const M4A: &[u8] = include_bytes!("../../../tests/fixtures/watch-audio/tone-440-16k.m4a");
const ID: &str = "6f9619ff-8b86-d011-b42d-00c04fc964ff";
const WAIT: Duration = Duration::from_secs(5);
const CHUNK: usize = 256 * 1024;

enum Out {
    Text(String),
    Binary(Vec<u8>),
}

/// The relay as the Mac sees it: one socket, pongs its pings, hands every
/// other text frame to the test, and sends what the test queues.
struct FakeRelay {
    to_mac: Sender<Out>,
    from_mac: Receiver<String>,
    next_id: u64,
}

fn is_idle(error: &std::io::Error) -> bool {
    matches!(
        error.kind(),
        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
    )
}

fn serve(listener: TcpListener, to_mac: Receiver<Out>, from_mac: Sender<String>) {
    let (stream, _) = listener.accept().unwrap();
    stream
        .set_read_timeout(Some(Duration::from_millis(5)))
        .unwrap();
    let mut socket: WebSocket<TcpStream> = tungstenite::accept(stream).unwrap();
    loop {
        while let Ok(out) = to_mac.try_recv() {
            let frame = match out {
                Out::Text(text) => Frame::text(text),
                Out::Binary(bytes) => Frame::binary(bytes),
            };
            if socket.send(frame).is_err() {
                return;
            }
        }
        match socket.read() {
            Ok(Frame::Text(text)) => {
                if text.as_str() == "ping" {
                    let _ = socket.send(Frame::text("pong"));
                } else if from_mac.send(text.to_string()).is_err() {
                    return;
                }
            }
            Ok(Frame::Close(_)) => return,
            Ok(_) => {}
            Err(tungstenite::Error::Io(error)) if is_idle(&error) => {}
            Err(_) => return,
        }
    }
}

fn start_fake_relay() -> (FakeRelay, u16) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let (to_mac, outgoing) = mpsc::channel();
    let (incoming, from_mac) = mpsc::channel();
    std::thread::spawn(move || serve(listener, outgoing, incoming));
    (
        FakeRelay {
            to_mac,
            from_mac,
            next_id: 0,
        },
        port,
    )
}

impl FakeRelay {
    fn send_text(&self, text: String) {
        self.to_mac.send(Out::Text(text)).unwrap();
    }

    /// The next frame the Mac sent that is not a reply to another request.
    fn next_text(&self) -> String {
        self.from_mac
            .recv_timeout(WAIT)
            .expect("a frame from the Mac")
    }

    fn request(
        &mut self,
        method: &str,
        path: &str,
        query: &[(&str, &str)],
        headers: &[(&str, &str)],
        body: &[u8],
    ) -> (u16, Value) {
        self.next_id += 1;
        let id = format!("req-{}", self.next_id);
        let query: serde_json::Map<String, Value> = query
            .iter()
            .map(|(k, v)| ((*k).to_owned(), Value::from(*v)))
            .collect();
        let headers: serde_json::Map<String, Value> = headers
            .iter()
            .map(|(k, v)| ((*k).to_owned(), Value::from(*v)))
            .collect();
        self.send_text(
            json!({
                "type": "req",
                "id": id,
                "method": method,
                "path": path,
                "query": query,
                "headers": headers,
                "body_len": body.len(),
            })
            .to_string(),
        );
        for chunk in body.chunks(CHUNK) {
            self.to_mac.send(Out::Binary(chunk.to_vec())).unwrap();
        }
        let deadline = Instant::now() + WAIT;
        while Instant::now() < deadline {
            let text = self
                .from_mac
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .expect("a reply from the Mac");
            let frame: Value = serde_json::from_str(&text).unwrap();
            if frame["type"] == "res" && frame["id"] == id {
                let status = frame["status"].as_u64().unwrap() as u16;
                let body = serde_json::from_str(frame["body"].as_str().unwrap()).unwrap();
                return (status, body);
            }
        }
        panic!("no reply to {method} {path} within {WAIT:?}");
    }
}

/// Answers every delivery from a helper thread, as the output worker would.
struct ScriptedDictation {
    samples: Arc<Mutex<Vec<usize>>>,
}

impl Dictation for ScriptedDictation {
    fn transcribe(&self, samples: Vec<f32>, delivery: WatchDelivery) -> Result<(), String> {
        self.samples.lock().unwrap().push(samples.len());
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(50));
            delivery.deliver(DeliveryReport {
                outcome: DeliveryOutcome::Unverified,
                text: "hello from the watch".to_owned(),
                raw_text: None,
                transcribe_ms: 120,
                paste_ms: 30,
            });
        });
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

fn epoch() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs_f64()
}

fn fresh_nonce() -> String {
    BASE64.encode(e2ee::random_nonce().unwrap())
}

/// Seals `plaintext` for `path` and opens the sealed reply.
fn sealed_request(
    relay: &mut FakeRelay,
    watch: &Watch,
    path: &str,
    plaintext: &[u8],
) -> (u16, Value) {
    let nonce_base64 = fresh_nonce();
    let nonce = e2ee::decode_nonce(&nonce_base64).unwrap();
    let aad = if path == "/e2ee" {
        Aad::Request {
            device_id: &watch.device,
            nonce_base64: &nonce_base64,
        }
    } else {
        Aad::Query {
            device_id: &watch.device,
            nonce_base64: &nonce_base64,
        }
    };
    let body = e2ee::seal(plaintext, &watch.key, &nonce, &aad);
    let (status, reply) = relay.request(
        "POST",
        path,
        &[],
        &[
            (DEVICE_HEADER, &watch.device),
            (NONCE_HEADER, &nonce_base64),
            (PROTOCOL_HEADER, "4"),
        ],
        &body,
    );
    let sealed = &reply["e2ee"];
    assert!(sealed.is_object(), "{status} {reply}");
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
            request_nonce_base64: &nonce_base64,
            status,
        },
    )
    .expect("the reply opens with the pairing key");
    let mut body: Value = serde_json::from_slice(&plain).unwrap();
    assert_eq!(body["e2ee_actions"], true);
    body.as_object_mut().unwrap().remove("e2ee_actions");
    (status, body)
}

fn query(relay: &mut FakeRelay, watch: &Watch, path: &str, id: &str) -> (u16, Value) {
    let head = QueryHead {
        version: e2ee::VERSION,
        token: watch.token.clone(),
        sent_at: epoch(),
        path: path.to_owned(),
        query: [("id".to_owned(), id.to_owned())].into_iter().collect(),
        requires_e2ee: None,
    };
    sealed_request(
        relay,
        watch,
        "/e2ee/query",
        &serde_json::to_vec(&head).unwrap(),
    )
}

fn audio(relay: &mut FakeRelay, watch: &Watch, id: &str) -> (u16, Value) {
    let head = AudioHead {
        version: e2ee::VERSION,
        token: watch.token.clone(),
        id: id.to_owned(),
        language: "en".to_owned(),
        sent_at: epoch(),
        focus: "requested".to_owned(),
        route: None,
        session: None,
        project: None,
        requires_e2ee: None,
        background: None,
        approval: None,
    };
    let plaintext = e2ee::encode_audio(&head, M4A).unwrap();
    sealed_request(relay, watch, "/e2ee", &plaintext)
}

fn temp_dir() -> std::path::PathBuf {
    let mut nonce = [0u8; 8];
    getrandom::fill(&mut nonce).unwrap();
    let dir = std::env::temp_dir().join(format!(
        "hex-watch-integration-{}-{:016x}",
        std::process::id(),
        u64::from_le_bytes(nonce)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn a_watch_pairs_records_and_reads_its_status_through_the_relay() {
    let started = Instant::now();
    let dir = temp_dir();
    let (mut relay, port) = start_fake_relay();
    let samples = Arc::new(Mutex::new(Vec::new()));
    let service = WatchService::start(
        WatchConfig {
            support_dir: dir.clone(),
            relay_base: format!("ws://127.0.0.1:{port}"),
            mac_name: "Test Mac".to_owned(),
            app_version: "3.0.0-test".to_owned(),
            timing: RelayTiming {
                ping_interval: Duration::from_millis(200),
                max_backoff: Duration::from_millis(50),
                allow_insecure: true,
            },
        },
        Box::new(ScriptedDictation {
            samples: Arc::clone(&samples),
        }),
        Box::new(MemorySecretStore::default()),
    )
    .unwrap();
    let deadline = Instant::now() + WAIT;
    while service.snapshot().map(|snapshot| snapshot.relay) != Some(RelayState::Connected) {
        assert!(Instant::now() < deadline, "the relay never connected");
        std::thread::sleep(Duration::from_millis(10));
    }
    for file in ["relay-room", "relay-secret", "device-id"] {
        assert!(dir.join(file).is_file(), "{file} was created");
    }

    // (a) `hex watch pair 217400`.
    let claimer = {
        let service = service.clone();
        std::thread::spawn(move || service.claim("217400"))
    };
    let claim: Value = serde_json::from_str(&relay.next_text()).unwrap();
    assert_eq!(
        claim,
        json!({"type": "claim", "code": "217400", "name": "Test Mac"})
    );
    relay.send_text(
        json!({"type": "claim_ok", "code": "217400", "device_id": "watch-1", "name": "Test Watch"})
            .to_string(),
    );
    assert_eq!(
        claimer.join().unwrap(),
        Ok(ClaimOk {
            device_id: "watch-1".to_owned(),
            device_name: "Test Watch".to_owned(),
        })
    );

    // (b) The watch pairs with a fresh P-256 key.
    let secret = loop {
        let mut scalar = [0u8; 32];
        getrandom::fill(&mut scalar).unwrap();
        if let Ok(secret) = SecretKey::from_slice(&scalar) {
            break secret;
        }
    };
    let public_key = BASE64.encode(secret.public_key().to_sec1_point(false).as_bytes());
    let (status, body) = relay.request(
        "POST",
        "/pairing/request",
        &[],
        &[("content-type", "application/json")],
        json!({"device_id": "watch-1", "name": "Test Watch", "previous_token": "", "e2ee_public_key": public_key})
            .to_string()
            .as_bytes(),
    );
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["name"], "Test Mac");
    let mac_id = std::fs::read_to_string(dir.join("device-id")).unwrap();
    assert_eq!(body["device_id"], mac_id.trim());
    let request_id = body["request_id"].as_str().unwrap().to_owned();
    let (status, body) = relay.request(
        "GET",
        "/pairing/result",
        &[("request_id", &request_id)],
        &[],
        &[],
    );
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["approved"], true);
    let agreement = e2ee::agree_with(&secret, body["e2ee_public_key"].as_str().unwrap()).unwrap();
    let token = e2ee::open_pairing_result(
        body["sealed_result"].as_str().unwrap(),
        &agreement.key,
        &request_id,
    )
    .unwrap();
    let watch = Watch {
        device: "watch-1".to_owned(),
        token,
        key: agreement.key,
    };

    // (c) Recording starts: `/focus` inside a query envelope.
    let (status, body) = query(&mut relay, &watch, "/focus", ID);
    assert_eq!(status, 200);
    assert_eq!(body, json!({"ok": true, "captured": false, "role": ""}));

    // (d) The recording itself.
    let (status, body) = audio(&mut relay, &watch, ID);
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["state"], "unverified");
    assert_eq!(body["text"], "hello from the watch");
    assert_eq!(body["verified"], false);
    assert_eq!(body["verification"], "none");
    assert_eq!(body["transcribe_ms"], 120);
    assert_eq!(body["paste_ms"], 30);
    assert!(body["receive_to_paste_ms"].as_u64().unwrap() >= 50);
    let heard = samples.lock().unwrap().clone();
    assert_eq!(heard.len(), 1);
    assert!(
        (15_000..=20_000).contains(&heard[0]),
        "{} samples",
        heard[0]
    );

    // (e) `/status` inside the envelope shows every field, text included.
    let (status, body) = query(&mut relay, &watch, "/status", ID);
    assert_eq!(status, 200);
    let expected: BTreeSet<&str> = "ok,id,state,language,verified,verification,entered,updated_at,route,transcribe_ms,paste_ms,receive_to_paste_ms,reason,reset,paste_reason,error,completed_at,completed_age,text,raw_text,agent_ms,session_id,agent_summary,agent_activity,agent_activity_at,agent_delivery"
        .split(',')
        .collect();
    let keys: BTreeSet<&str> = body
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(keys, expected);
    assert_eq!(body["state"], "unverified");
    assert_eq!(body["id"], ID);
    assert_eq!(body["text"], "hello from the watch");

    // (f) The same recording again is a duplicate, not a second paste.
    let (status, body) = audio(&mut relay, &watch, ID);
    assert_eq!(status, 200);
    assert_eq!(body["duplicate"], true);
    assert_eq!(body["state"], "unverified");
    assert_eq!(samples.lock().unwrap().len(), 1);

    // (g) Plaintext `/status` answers without the text.
    let (status, body) = relay.request(
        "GET",
        "/status",
        &[("id", ID)],
        &[(TOKEN_HEADER, &watch.token), (PROTOCOL_HEADER, "4")],
        &[],
    );
    assert_eq!(status, 200);
    assert_eq!(body["state"], "unverified");
    assert!(body.get("text").is_none());

    // (h) Plaintext `/audio` is refused before its body is read.
    let (status, body) = relay.request(
        "POST",
        "/audio",
        &[],
        &[(TOKEN_HEADER, &watch.token), (PROTOCOL_HEADER, "4")],
        M4A,
    );
    assert_eq!(
        (status, body["error"].as_str()),
        (403, Some("e2ee-required"))
    );

    let snapshot = service.snapshot().unwrap();
    assert_eq!(snapshot.devices.len(), 1);
    assert_eq!(snapshot.devices[0].id, "watch-1");
    service.stop();
    assert!(
        started.elapsed() < Duration::from_secs(10),
        "{:?}",
        started.elapsed()
    );
    let _ = std::fs::remove_dir_all(&dir);
}
