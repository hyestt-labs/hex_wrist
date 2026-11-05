//! End-to-end encryption between a watch and this Mac, byte-compatible with
//! Wristper's `PairingKey.swift` and `EncryptedBridge.swift`: a P-256 ECDH
//! shared secret run through HKDF-SHA256 gives one 32-byte AES-256-GCM key
//! per pairing. Every ciphertext is bound to its purpose by an AAD string,
//! so a recording envelope cannot be replayed as a query, nor a response as
//! a request. `tests/fixtures/watch-e2ee/vectors.json`
//! (`scripts/watch-e2ee-vectors.swift`) holds CryptoKit's output for fixed
//! keys and nonces; the tests prove both directions against it.
//!
//! Base64 everywhere here is the standard alphabet with padding
//! (`Data.base64EncodedString()`); only pairing tokens are base64url without
//! padding (`pairing`).

use std::collections::{BTreeMap, HashSet, VecDeque};
use std::fmt;

use aes_gcm::aead::{Aead, Payload};
use aes_gcm::{Aes256Gcm, KeyInit};
use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use hkdf::Hkdf;
use p256::elliptic_curve::sec1::ToSec1Point;
use p256::{PublicKey, SecretKey, ecdh};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::Sha256;

use super::frame::MAX_AUDIO_BYTES;

pub(crate) const VERSION: u32 = 1;
/// The AudioHead JSON may be at most this long; the 4-byte length prefix
/// says how long it is.
pub(crate) const MAX_HEAD_BYTES: usize = 4096;
pub(crate) const NONCE_BYTES: usize = 12;
pub(crate) const TAG_BYTES: usize = 16;
pub(crate) const KEY_BYTES: usize = 32;

/// A request's `sent_at` may differ from this clock by this much either way.
pub(crate) const MAX_CLOCK_SKEW_SECS: f64 = 300.0;
/// Actions the relay could hold back and release later (`/focus`, `/enter`,
/// `/undo`, agent confirmations) accept only this much age.
pub(crate) const ACTION_MAX_AGE_SECS: f64 = 60.0;
/// A background upload may arrive hours late; it is still bounded by the
/// dedupe table's retention so the same recording cannot run twice.
pub(crate) const BACKGROUND_MAX_AGE_SECS: f64 = 86_400.0;
pub(crate) const NONCE_CACHE_CAPACITY: usize = 4096;
/// Every response plaintext carries this field; the watch learns from it
/// that this Mac takes encrypted actions and stops falling back to plain.
pub(crate) const ENCRYPTED_ACTIONS_FIELD: &str = "e2ee_actions";

const KEY_SALT: &[u8] = b"Wristper E2EE v1";
const KEY_INFO: &[u8] = b"Wristper pairing key";
const RESULT_TAG: &str = "Wristper pairing result v1";
const REQUEST_TAG: &str = "Wristper E2EE request v1";
const QUERY_TAG: &str = "Wristper E2EE query v1";
const RESPONSE_TAG: &str = "Wristper E2EE response v1";

/// Inner paths a query envelope may name. Phase 1 serves the dictation
/// ones; the agent paths stay listed so the policy (POST or GET, short-lived
/// or not) is already right when phase 2 adds their handlers.
pub(crate) const QUERY_POST_PATHS: &[&str] = &["/focus", "/enter", "/undo"];
pub(crate) const QUERY_GET_PATHS: &[&str] = &["/status"];
pub(crate) const AGENT_QUERY_POST_PATHS: &[&str] = &[
    "/agent/confirm",
    "/agent/stop",
    "/agent/answer",
    "/agent/push-token",
];
pub(crate) const AGENT_QUERY_GET_PATHS: &[&str] = &[
    "/agent/projects",
    "/agent/sessions",
    "/agent/session",
    "/agent/recent",
    "/agent/usage",
    "/agent/runs",
];
/// Short-lived: the relay must not be able to hold these and release them
/// once the situation on the Mac changed.
pub(crate) const SHORT_LIVED_PATHS: &[&str] = &[
    "/agent/confirm",
    "/agent/stop",
    "/agent/answer",
    "/focus",
    "/enter",
    "/undo",
];

/// How a query envelope's inner request is carried to its handler.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum QueryShape {
    /// `query` becomes the JSON body.
    Post,
    /// `query` stays the query string.
    Get,
}

pub(crate) fn query_shape(path: &str) -> Option<QueryShape> {
    if QUERY_POST_PATHS.contains(&path) {
        Some(QueryShape::Post)
    } else if QUERY_GET_PATHS.contains(&path) {
        Some(QueryShape::Get)
    } else {
        None
    }
}

pub(crate) fn is_short_lived(path: &str) -> bool {
    SHORT_LIVED_PATHS.contains(&path)
}

/// The 32-byte pairing key. Debug prints nothing of it.
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct SharedKey([u8; KEY_BYTES]);

impl fmt::Debug for SharedKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SharedKey(…)")
    }
}

impl SharedKey {
    /// Only an exact 32-byte standard-base64 value is a key; a pairing from
    /// before encryption has none and must pair again.
    pub(crate) fn from_base64(text: &str) -> Option<Self> {
        let bytes = BASE64.decode(text).ok()?;
        Some(Self(bytes.try_into().ok()?))
    }

    pub(crate) fn to_base64(&self) -> String {
        BASE64.encode(self.0)
    }

    pub(crate) fn as_bytes(&self) -> &[u8; KEY_BYTES] {
        &self.0
    }
}

/// A finished key agreement: the key, and the Mac public key the watch
/// needs to derive the same one.
pub(crate) struct Agreement {
    pub(crate) key: SharedKey,
    /// X9.63 uncompressed point (`04 ‖ X ‖ Y`), as `Data.base64EncodedString()`.
    pub(crate) public_key_base64: String,
}

pub(crate) fn is_valid_public_key(base64: &str) -> bool {
    BASE64
        .decode(base64)
        .ok()
        .is_some_and(|bytes| PublicKey::from_sec1_bytes(&bytes).is_ok())
}

/// Derives the pairing key against the watch's public key with a fresh
/// ephemeral Mac key.
pub(crate) fn agree(peer_public_key_base64: &str) -> Option<Agreement> {
    let mut scalar = [0u8; KEY_BYTES];
    for _ in 0..8 {
        getrandom::fill(&mut scalar).ok()?;
        if let Ok(secret) = SecretKey::from_slice(&scalar) {
            return agree_with(&secret, peer_public_key_base64);
        }
    }
    None
}

/// `PairingKey.sharedKey`: ECDH, then HKDF-SHA256 with the fixed salt and
/// info. Deterministic for a given key pair, which the vectors exercise.
pub(crate) fn agree_with(secret: &SecretKey, peer_public_key_base64: &str) -> Option<Agreement> {
    let peer_bytes = BASE64.decode(peer_public_key_base64).ok()?;
    let peer = PublicKey::from_sec1_bytes(&peer_bytes).ok()?;
    let shared = ecdh::diffie_hellman(secret.to_nonzero_scalar(), peer.as_affine());
    let public = secret.public_key().to_sec1_point(false);
    Some(Agreement {
        key: derive_key(shared.raw_secret_bytes(), KEY_SALT, KEY_INFO)?,
        public_key_base64: BASE64.encode(public.as_bytes()),
    })
}

fn derive_key(shared_secret: &[u8], salt: &[u8], info: &[u8]) -> Option<SharedKey> {
    let mut key = [0u8; KEY_BYTES];
    Hkdf::<Sha256>::new(Some(salt), shared_secret)
        .expand(info, &mut key)
        .ok()?;
    Some(SharedKey(key))
}

/// What a ciphertext is for. The exact strings are the wire contract.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Aad<'a> {
    PairingResult {
        request_id: &'a str,
    },
    Request {
        device_id: &'a str,
        nonce_base64: &'a str,
    },
    Query {
        device_id: &'a str,
        nonce_base64: &'a str,
    },
    Response {
        device_id: &'a str,
        request_nonce_base64: &'a str,
        status: u16,
    },
}

impl Aad<'_> {
    pub(crate) fn bytes(&self) -> Vec<u8> {
        match self {
            Aad::PairingResult { request_id } => format!("{RESULT_TAG}\n{request_id}"),
            Aad::Request {
                device_id,
                nonce_base64,
            } => format!("{REQUEST_TAG}\n{device_id}\n{nonce_base64}"),
            Aad::Query {
                device_id,
                nonce_base64,
            } => format!("{QUERY_TAG}\n{device_id}\n{nonce_base64}"),
            Aad::Response {
                device_id,
                request_nonce_base64,
                status,
            } => format!("{RESPONSE_TAG}\n{device_id}\n{request_nonce_base64}\n{status}"),
        }
        .into_bytes()
    }
}

pub(crate) fn random_nonce() -> Option<[u8; NONCE_BYTES]> {
    let mut nonce = [0u8; NONCE_BYTES];
    getrandom::fill(&mut nonce).ok()?;
    Some(nonce)
}

/// AES-256-GCM; returns `ciphertext ‖ tag`, as CryptoKit's
/// `box.ciphertext + box.tag`.
pub(crate) fn seal(
    plaintext: &[u8],
    key: &SharedKey,
    nonce: &[u8; NONCE_BYTES],
    aad: &Aad<'_>,
) -> Vec<u8> {
    let cipher = Aes256Gcm::new(key.as_bytes().into());
    cipher
        .encrypt(
            nonce.into(),
            Payload {
                msg: plaintext,
                aad: &aad.bytes(),
            },
        )
        // AES-GCM encryption fails only for a message past 2^36 bytes;
        // the body limits keep every input far below.
        .unwrap_or_default()
}

/// Opens `ciphertext ‖ tag`. `None` for a wrong key, nonce, AAD, or a
/// damaged body; the caller answers `invalid-e2ee` and says no more.
pub(crate) fn open(
    ciphertext_and_tag: &[u8],
    nonce: &[u8],
    key: &SharedKey,
    aad: &Aad<'_>,
) -> Option<Vec<u8>> {
    let nonce: &[u8; NONCE_BYTES] = nonce.try_into().ok()?;
    if ciphertext_and_tag.len() < TAG_BYTES {
        return None;
    }
    let cipher = Aes256Gcm::new(key.as_bytes().into());
    cipher
        .decrypt(
            nonce.into(),
            Payload {
                msg: ciphertext_and_tag,
                aad: &aad.bytes(),
            },
        )
        .ok()
}

/// `PairingKey.sealResult`: the new token in CryptoKit's combined form
/// (`nonce ‖ ciphertext ‖ tag`), base64.
pub(crate) fn seal_pairing_result(
    token: &str,
    key: &SharedKey,
    request_id: &str,
    nonce: &[u8; NONCE_BYTES],
) -> String {
    let mut combined = nonce.to_vec();
    combined.extend(seal(
        token.as_bytes(),
        key,
        nonce,
        &Aad::PairingResult { request_id },
    ));
    BASE64.encode(combined)
}

/// The watch's side of `seal_pairing_result`, kept for the vectors and for
/// a future pairing self-test.
pub(crate) fn open_pairing_result(
    sealed: &str,
    key: &SharedKey,
    request_id: &str,
) -> Option<String> {
    let combined = BASE64.decode(sealed).ok()?;
    let (nonce, rest) = combined.split_at_checked(NONCE_BYTES)?;
    let token = open(rest, nonce, key, &Aad::PairingResult { request_id })?;
    String::from_utf8(token).ok()
}

/// The JSON head inside a `/e2ee` recording envelope.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct AudioHead {
    pub(crate) version: u32,
    pub(crate) token: String,
    pub(crate) id: String,
    pub(crate) language: String,
    pub(crate) sent_at: f64,
    /// `requested` (a `/focus` was sent) or `current` (paste at the cursor).
    pub(crate) focus: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) route: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) session: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) project: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) requires_e2ee: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) background: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) approval: Option<String>,
}

impl AudioHead {
    /// A background upload without an agent route may arrive hours late.
    pub(crate) fn may_arrive_late(&self) -> bool {
        self.background == Some(true) && self.route.is_none()
    }

    /// `ReceiverService.isFresh`: not from the future beyond the skew, and
    /// not older than its kind allows.
    pub(crate) fn is_fresh(&self, now_epoch_secs: f64) -> bool {
        let age = now_epoch_secs - self.sent_at;
        let maximum = if self.may_arrive_late() {
            BACKGROUND_MAX_AGE_SECS
        } else {
            MAX_CLOCK_SKEW_SECS
        };
        age >= -MAX_CLOCK_SKEW_SECS && age <= maximum
    }
}

/// The JSON inside a `/e2ee/query` envelope: an action or a read on the
/// watch's behalf.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct QueryHead {
    pub(crate) version: u32,
    pub(crate) token: String,
    pub(crate) sent_at: f64,
    pub(crate) path: String,
    pub(crate) query: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) requires_e2ee: Option<bool>,
}

impl QueryHead {
    pub(crate) fn is_fresh(&self, now_epoch_secs: f64) -> bool {
        (now_epoch_secs - self.sent_at).abs() <= MAX_CLOCK_SKEW_SECS
    }

    /// A short-lived action that took longer than `ACTION_MAX_AGE_SECS` to
    /// arrive: answered `confirm-stale`, after the nonce was consumed.
    pub(crate) fn is_stale_action(&self, now_epoch_secs: f64) -> bool {
        is_short_lived(&self.path) && (now_epoch_secs - self.sent_at).abs() > ACTION_MAX_AGE_SECS
    }
}

/// Parses a `/e2ee/query` plaintext: version 1 and a path the envelope
/// may name, else nothing (`invalid-e2ee`). The agent paths are refused
/// here until phase 2 serves them; `/health` reports no agent support, so
/// the watch does not send them meanwhile.
pub(crate) fn decode_query(plaintext: &[u8]) -> Option<QueryHead> {
    let head: QueryHead = serde_json::from_slice(plaintext).ok()?;
    (head.version == VERSION && query_shape(&head.path).is_some()).then_some(head)
}

/// `EncryptedBridge.decodeAudio`: `len(4, big-endian) ‖ head JSON ‖ audio`.
/// The audio is held to the plaintext `/audio` limit: the envelope's body
/// limit leaves room for a head, not for a longer recording.
pub(crate) fn decode_audio(plaintext: &[u8]) -> Option<(AudioHead, &[u8])> {
    let (length, rest) = plaintext.split_at_checked(4)?;
    let head_len = u32::from_be_bytes(length.try_into().ok()?) as usize;
    if head_len == 0 || head_len > MAX_HEAD_BYTES {
        return None;
    }
    let (head_json, audio) = rest.split_at_checked(head_len)?;
    if audio.len() > MAX_AUDIO_BYTES {
        return None;
    }
    let head: AudioHead = serde_json::from_slice(head_json).ok()?;
    (head.version == VERSION).then_some((head, audio))
}

/// The watch's `EncryptedWatchMacBridge.seal` layout, for tests and a
/// future pairing self-test. The bytes differ from the watch's: its
/// `JSONEncoder` orders the head's keys differently in every process.
pub(crate) fn encode_audio(head: &AudioHead, audio: &[u8]) -> Option<Vec<u8>> {
    let head_json = serde_json::to_vec(head).ok()?;
    if head_json.len() > MAX_HEAD_BYTES {
        return None;
    }
    let mut plaintext = (head_json.len() as u32).to_be_bytes().to_vec();
    plaintext.extend(head_json);
    plaintext.extend_from_slice(audio);
    Some(plaintext)
}

/// A sealed response: the body the watch receives in place of the plain
/// JSON, under the same HTTP status.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ResponseEnvelope {
    pub(crate) nonce_base64: String,
    pub(crate) ciphertext_base64: String,
}

impl ResponseEnvelope {
    pub(crate) fn json(&self) -> Value {
        json!({
            "ok": true,
            "e2ee": {"nonce": self.nonce_base64, "ciphertext": self.ciphertext_base64},
        })
    }
}

/// Seals a reply for `device_id` to the request that carried
/// `request_nonce_base64`, bound to `status`, with `e2ee_actions: true`
/// added to the plaintext. `reply` must be a JSON object.
pub(crate) fn seal_response(
    reply: &Value,
    status: u16,
    key: &SharedKey,
    device_id: &str,
    request_nonce_base64: &str,
    nonce: &[u8; NONCE_BYTES],
) -> Option<ResponseEnvelope> {
    let mut marked = reply.as_object()?.clone();
    marked.insert(ENCRYPTED_ACTIONS_FIELD.to_owned(), Value::Bool(true));
    let plaintext = Value::Object(marked).to_string();
    let ciphertext = seal(
        plaintext.as_bytes(),
        key,
        nonce,
        &Aad::Response {
            device_id,
            request_nonce_base64,
            status,
        },
    );
    Some(ResponseEnvelope {
        nonce_base64: BASE64.encode(nonce),
        ciphertext_base64: BASE64.encode(ciphertext),
    })
}

/// Decodes the `X-Wristper-Nonce` header. The text form is also part of
/// the AAD, so callers keep both.
pub(crate) fn decode_nonce(base64: &str) -> Option<[u8; NONCE_BYTES]> {
    BASE64.decode(base64).ok()?.try_into().ok()
}

/// The nonces already accepted, per device, bounded to the newest
/// `NONCE_CACHE_CAPACITY`. Memory only, as in Wristper: across a restart
/// the recording id dedupe (`delivered`) and the `entered` flag stop the
/// replays that matter.
#[derive(Debug)]
pub(crate) struct NonceCache {
    seen: HashSet<String>,
    order: VecDeque<String>,
    capacity: usize,
}

impl Default for NonceCache {
    fn default() -> Self {
        Self::with_capacity(NONCE_CACHE_CAPACITY)
    }
}

impl NonceCache {
    pub(crate) fn with_capacity(capacity: usize) -> Self {
        Self {
            seen: HashSet::new(),
            order: VecDeque::new(),
            capacity,
        }
    }

    /// `true` the first time a device presents a nonce; `false` on a replay.
    pub(crate) fn consume(&mut self, device_id: &str, nonce_base64: &str) -> bool {
        let value = format!("{device_id}:{nonce_base64}");
        if !self.seen.insert(value.clone()) {
            return false;
        }
        self.order.push_back(value);
        if self.order.len() > self.capacity
            && let Some(oldest) = self.order.pop_front()
        {
            self.seen.remove(&oldest);
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const VECTORS: &str = include_str!("../../tests/fixtures/watch-e2ee/vectors.json");

    fn vectors() -> Value {
        serde_json::from_str(VECTORS).unwrap()
    }

    fn text<'a>(value: &'a Value, path: &[&str]) -> &'a str {
        path.iter()
            .fold(value, |value, key| &value[*key])
            .as_str()
            .unwrap_or_else(|| panic!("{path:?}"))
    }

    fn hex(text: &str) -> Vec<u8> {
        (0..text.len())
            .step_by(2)
            .map(|index| u8::from_str_radix(&text[index..index + 2], 16).unwrap())
            .collect()
    }

    fn vector_key(vectors: &Value) -> SharedKey {
        SharedKey::from_base64(text(vectors, &["shared_key_base64"])).unwrap()
    }

    fn nonce(base64: &str) -> [u8; NONCE_BYTES] {
        decode_nonce(base64).unwrap()
    }

    #[test]
    fn both_sides_derive_cryptokits_key() {
        let vectors = vectors();
        let expected = hex(text(&vectors, &["shared_key_hex"]));
        let mac = SecretKey::from_slice(&hex(text(&vectors, &["mac_private_key_hex"]))).unwrap();
        let from_mac = agree_with(&mac, text(&vectors, &["watch_public_key_x963_base64"])).unwrap();
        assert_eq!(from_mac.key.as_bytes().as_slice(), expected.as_slice());
        assert_eq!(
            from_mac.public_key_base64,
            text(&vectors, &["mac_public_key_x963_base64"])
        );
        let watch =
            SecretKey::from_slice(&hex(text(&vectors, &["watch_private_key_hex"]))).unwrap();
        let from_watch =
            agree_with(&watch, text(&vectors, &["mac_public_key_x963_base64"])).unwrap();
        assert_eq!(from_watch.key, from_mac.key);
        assert_eq!(
            from_mac.key.to_base64(),
            text(&vectors, &["shared_key_base64"])
        );
    }

    #[test]
    fn the_key_needs_wristpers_exact_salt_and_info() {
        let vectors = vectors();
        let expected = hex(text(&vectors, &["shared_key_hex"]));
        let mac = SecretKey::from_slice(&hex(text(&vectors, &["mac_private_key_hex"]))).unwrap();
        let peer = BASE64
            .decode(text(&vectors, &["watch_public_key_x963_base64"]))
            .unwrap();
        let peer = PublicKey::from_sec1_bytes(&peer).unwrap();
        let shared = ecdh::diffie_hellman(mac.to_nonzero_scalar(), peer.as_affine());
        let raw = shared.raw_secret_bytes().as_slice();
        let derive = |salt: &[u8], info: &[u8]| derive_key(raw, salt, info).unwrap();
        assert_eq!(
            derive(KEY_SALT, KEY_INFO).as_bytes().as_slice(),
            expected.as_slice()
        );
        assert_eq!(
            (KEY_SALT, KEY_INFO),
            (
                b"Wristper E2EE v1".as_slice(),
                b"Wristper pairing key".as_slice()
            )
        );
        assert_ne!(
            derive(b"", KEY_INFO),
            derive(KEY_SALT, KEY_INFO),
            "salt is used"
        );
        assert_ne!(
            derive(b"Wristper E2EE v2", KEY_INFO),
            derive(KEY_SALT, KEY_INFO)
        );
        assert_ne!(
            derive(KEY_SALT, b""),
            derive(KEY_SALT, KEY_INFO),
            "info is used"
        );
        assert_ne!(raw, expected.as_slice(), "the ECDH secret is never the key");
    }

    #[test]
    fn agree_uses_a_fresh_key_each_time_and_rejects_bad_public_keys() {
        let vectors = vectors();
        let peer = text(&vectors, &["watch_public_key_x963_base64"]);
        let one = agree(peer).unwrap();
        let two = agree(peer).unwrap();
        assert_ne!(one.key, two.key);
        assert_ne!(one.public_key_base64, two.public_key_base64);
        assert!(is_valid_public_key(peer));
        assert!(!is_valid_public_key(""));
        assert!(!is_valid_public_key("not base64!"));
        assert!(!is_valid_public_key(&BASE64.encode([4u8; 65])));
        assert!(agree(&BASE64.encode([4u8; 65])).is_none());
    }

    #[test]
    fn pairing_result_matches_cryptokit_and_opens() {
        let vectors = vectors();
        let key = vector_key(&vectors);
        let result = &vectors["pairing_result"];
        let request_id = text(result, &["request_id"]);
        let token = text(result, &["token"]);
        let nonce: [u8; 12] = hex(text(result, &["nonce_hex"])).try_into().unwrap();
        let sealed = seal_pairing_result(token, &key, request_id, &nonce);
        assert_eq!(sealed, text(result, &["combined_base64"]));
        assert_eq!(
            open_pairing_result(text(result, &["combined_base64"]), &key, request_id).as_deref(),
            Some(token)
        );
        assert_eq!(open_pairing_result(&sealed, &key, "another-request"), None);
        let other = SharedKey([7u8; 32]);
        assert_eq!(open_pairing_result(&sealed, &other, request_id), None);
    }

    #[test]
    fn audio_envelope_opens_decodes_and_reseals_identically() {
        let vectors = vectors();
        let key = vector_key(&vectors);
        let audio = &vectors["audio"];
        let device_id = text(audio, &["device_id"]);
        let nonce_base64 = text(audio, &["nonce_base64"]);
        let body = BASE64
            .decode(text(audio, &["ciphertext_and_tag_base64"]))
            .unwrap();
        let aad = Aad::Request {
            device_id,
            nonce_base64,
        };
        let plaintext = open(&body, &nonce(nonce_base64), &key, &aad).unwrap();
        assert_eq!(
            BASE64.encode(&plaintext),
            text(audio, &["plaintext_base64"])
        );
        let (head, bytes) = decode_audio(&plaintext).unwrap();
        assert_eq!(head.version, 1);
        assert_eq!(head.id, "6F9619FF-8B86-D011-B42D-00C04FC964FF");
        assert_eq!(head.language, "zh");
        assert_eq!(head.sent_at, 1_762_259_696.5);
        assert_eq!(head.focus, "requested");
        assert_eq!(head.requires_e2ee, Some(true));
        assert_eq!(head.background, Some(true));
        assert_eq!(head.route, None);
        assert!(head.may_arrive_late());
        assert_eq!(bytes, hex(text(audio, &["audio_hex"])).as_slice());
        assert_eq!(seal(&plaintext, &key, &nonce(nonce_base64), &aad), body);
    }

    #[test]
    fn audio_envelope_rejects_the_wrong_aad_key_nonce_or_tag() {
        let vectors = vectors();
        let key = vector_key(&vectors);
        let audio = &vectors["audio"];
        let device_id = text(audio, &["device_id"]);
        let nonce_base64 = text(audio, &["nonce_base64"]);
        let body = BASE64
            .decode(text(audio, &["ciphertext_and_tag_base64"]))
            .unwrap();
        let good = Aad::Request {
            device_id,
            nonce_base64,
        };
        assert!(open(&body, &nonce(nonce_base64), &key, &good).is_some());
        assert!(
            open(
                &body,
                &nonce(nonce_base64),
                &key,
                &Aad::Query {
                    device_id,
                    nonce_base64
                }
            )
            .is_none(),
            "a recording must not open as a query"
        );
        assert!(
            open(
                &body,
                &nonce(nonce_base64),
                &key,
                &Aad::Request {
                    device_id: "other-watch",
                    nonce_base64
                }
            )
            .is_none()
        );
        assert!(open(&body, &nonce(nonce_base64), &SharedKey([1u8; 32]), &good).is_none());
        assert!(open(&body, &[0u8; 12], &key, &good).is_none());
        assert!(open(&body, &[0u8; 11], &key, &good).is_none());
        let mut flipped = body.clone();
        *flipped.last_mut().unwrap() ^= 1;
        assert!(open(&flipped, &nonce(nonce_base64), &key, &good).is_none());
        assert!(open(&body[..15], &nonce(nonce_base64), &key, &good).is_none());
    }

    #[test]
    fn audio_plaintext_layout_is_checked() {
        let head = AudioHead {
            version: 1,
            token: "t".into(),
            id: "6F9619FF-8B86-D011-B42D-00C04FC964FF".into(),
            language: "en".into(),
            sent_at: 1.0,
            focus: "current".into(),
            route: None,
            session: None,
            project: None,
            requires_e2ee: None,
            background: None,
            approval: None,
        };
        let plaintext = encode_audio(&head, b"audio").unwrap();
        let json = serde_json::to_string(&head).unwrap();
        assert!(
            !json.contains("route"),
            "absent optionals stay absent: {json}"
        );
        let (decoded, audio) = decode_audio(&plaintext).unwrap();
        assert_eq!(decoded, head);
        assert_eq!(audio, b"audio");
        assert!(decode_audio(&plaintext[..3]).is_none());
        let mut zero = plaintext.clone();
        zero[..4].copy_from_slice(&0u32.to_be_bytes());
        assert!(decode_audio(&zero).is_none());
        let mut huge = plaintext.clone();
        huge[..4].copy_from_slice(&4097u32.to_be_bytes());
        assert!(decode_audio(&huge).is_none());
        let mut short = plaintext.clone();
        short.truncate(4 + json.len() - 1);
        assert!(decode_audio(&short).is_none());
        let mut v2 = head.clone();
        v2.version = 2;
        assert!(decode_audio(&encode_audio(&v2, b"").unwrap()).is_none());
        let mut big_head = head.clone();
        big_head.project = Some("x".repeat(MAX_HEAD_BYTES));
        assert!(encode_audio(&big_head, b"").is_none());
    }

    #[test]
    fn audio_past_the_recording_limit_is_refused_inside_the_envelope() {
        let head = AudioHead {
            version: 1,
            token: "t".into(),
            id: "6F9619FF-8B86-D011-B42D-00C04FC964FF".into(),
            language: "en".into(),
            sent_at: 1.0,
            focus: "current".into(),
            route: None,
            session: None,
            project: None,
            requires_e2ee: None,
            background: None,
            approval: None,
        };
        let at_limit = encode_audio(&head, &vec![0; MAX_AUDIO_BYTES]).unwrap();
        assert_eq!(decode_audio(&at_limit).unwrap().1.len(), MAX_AUDIO_BYTES);
        let over = encode_audio(&head, &vec![0; MAX_AUDIO_BYTES + 1]).unwrap();
        assert!(decode_audio(&over).is_none());
    }

    #[test]
    fn a_head_decodes_in_any_key_order_and_with_escaped_slashes() {
        let json = br#"{"focus":"requested","language":"zh","id":"6F96","sent_at":1.5,"route":"a\/b","token":"t","version":1}"#;
        let mut plaintext = (json.len() as u32).to_be_bytes().to_vec();
        plaintext.extend_from_slice(json);
        let (head, audio) = decode_audio(&plaintext).unwrap();
        assert_eq!(head.route.as_deref(), Some("a/b"));
        assert!(audio.is_empty());
    }

    #[test]
    fn query_envelope_opens_and_reseals_identically() {
        let vectors = vectors();
        let key = vector_key(&vectors);
        let query = &vectors["query"];
        let device_id = text(query, &["device_id"]);
        let nonce_base64 = text(query, &["nonce_base64"]);
        let body = BASE64
            .decode(text(query, &["ciphertext_and_tag_base64"]))
            .unwrap();
        let aad = Aad::Query {
            device_id,
            nonce_base64,
        };
        let plaintext = open(&body, &nonce(nonce_base64), &key, &aad).unwrap();
        assert_eq!(plaintext, text(query, &["plaintext"]).as_bytes());
        let head = decode_query(&plaintext).unwrap();
        assert_eq!(head.path, "/status");
        assert_eq!(head.sent_at, 1_762_259_700.25);
        assert_eq!(
            head.query.get("id").map(String::as_str),
            Some("6F9619FF-8B86-D011-B42D-00C04FC964FF")
        );
        assert_eq!(seal(&plaintext, &key, &nonce(nonce_base64), &aad), body);
        assert!(
            open(
                &body,
                &nonce(nonce_base64),
                &key,
                &Aad::Request {
                    device_id,
                    nonce_base64
                }
            )
            .is_none(),
            "a query must not open as a recording"
        );
    }

    #[test]
    fn query_paths_are_bounded_to_the_allowed_set() {
        let head = |path: &str| {
            format!(r#"{{"version":1,"token":"t","sent_at":1.0,"path":"{path}","query":{{}}}}"#)
        };
        assert!(decode_query(head("/status").as_bytes()).is_some());
        assert!(decode_query(head("/focus").as_bytes()).is_some());
        assert!(decode_query(head("/health").as_bytes()).is_none());
        assert!(decode_query(head("/paired").as_bytes()).is_none());
        assert!(
            decode_query(head("/agent/sessions").as_bytes()).is_none(),
            "phase 2"
        );
        assert!(
            decode_query(br#"{"version":2,"token":"t","sent_at":1.0,"path":"/status","query":{}}"#)
                .is_none()
        );
        assert_eq!(query_shape("/focus"), Some(QueryShape::Post));
        assert_eq!(query_shape("/enter"), Some(QueryShape::Post));
        assert_eq!(query_shape("/undo"), Some(QueryShape::Post));
        assert_eq!(query_shape("/status"), Some(QueryShape::Get));
        for path in SHORT_LIVED_PATHS {
            assert!(is_short_lived(path));
        }
        assert!(!is_short_lived("/status"));
    }

    #[test]
    fn response_envelope_matches_cryptokit() {
        let vectors = vectors();
        let key = vector_key(&vectors);
        let response = &vectors["response"];
        let device_id = text(response, &["device_id"]);
        let request_nonce = text(response, &["request_nonce_base64"]);
        let status = response["status"].as_u64().unwrap() as u16;
        let nonce_base64 = text(response, &["nonce_base64"]);
        let reply = json!({"ok": true, "state": "unverified", "verified": false});
        let envelope = seal_response(
            &reply,
            status,
            &key,
            device_id,
            request_nonce,
            &nonce(nonce_base64),
        )
        .unwrap();
        assert_eq!(envelope.nonce_base64, nonce_base64);
        assert_eq!(
            envelope.ciphertext_base64,
            text(response, &["ciphertext_base64"])
        );
        let body = envelope.json();
        assert_eq!(body["ok"], true);
        assert_eq!(body["e2ee"]["nonce"], nonce_base64);
        assert_eq!(body["e2ee"]["ciphertext"], envelope.ciphertext_base64);
        assert_eq!(body.as_object().unwrap().len(), 2);
        let ciphertext = BASE64.decode(&envelope.ciphertext_base64).unwrap();
        let plaintext = open(
            &ciphertext,
            &nonce(nonce_base64),
            &key,
            &Aad::Response {
                device_id,
                request_nonce_base64: request_nonce,
                status,
            },
        )
        .unwrap();
        assert_eq!(plaintext, text(response, &["plaintext"]).as_bytes());
        let other_status = Aad::Response {
            device_id,
            request_nonce_base64: request_nonce,
            status: 403,
        };
        assert!(
            open(&ciphertext, &nonce(nonce_base64), &key, &other_status).is_none(),
            "a 200 reply must not pass as a 403"
        );
        assert!(
            seal_response(
                &json!([1]),
                200,
                &key,
                device_id,
                request_nonce,
                &nonce(nonce_base64)
            )
            .is_none()
        );
    }

    #[test]
    fn freshness_windows() {
        let head = |sent_at: f64, background: Option<bool>, route: Option<&str>| AudioHead {
            version: 1,
            token: "t".into(),
            id: "i".into(),
            language: "en".into(),
            sent_at,
            focus: "requested".into(),
            route: route.map(str::to_owned),
            session: None,
            project: None,
            requires_e2ee: None,
            background,
            approval: None,
        };
        let now = 1_000_000.0;
        assert!(head(now - 299.0, None, None).is_fresh(now));
        assert!(head(now + 299.0, None, None).is_fresh(now));
        assert!(!head(now - 301.0, None, None).is_fresh(now));
        assert!(!head(now + 301.0, None, None).is_fresh(now));
        assert!(head(now - 86_000.0, Some(true), None).is_fresh(now));
        assert!(!head(now - 86_401.0, Some(true), None).is_fresh(now));
        assert!(
            !head(now - 86_000.0, Some(true), Some("claude")).is_fresh(now),
            "an agent recording never gets the background window"
        );
        assert!(!head(now + 301.0, Some(true), None).is_fresh(now));

        let query = |sent_at: f64, path: &str| QueryHead {
            version: 1,
            token: "t".into(),
            sent_at,
            path: path.into(),
            query: BTreeMap::new(),
            requires_e2ee: None,
        };
        assert!(query(now - 299.0, "/status").is_fresh(now));
        assert!(!query(now - 301.0, "/status").is_fresh(now));
        assert!(!query(now - 299.0, "/status").is_stale_action(now));
        assert!(!query(now - 59.0, "/enter").is_stale_action(now));
        assert!(query(now - 61.0, "/enter").is_stale_action(now));
        assert!(query(now + 61.0, "/focus").is_stale_action(now));
    }

    #[test]
    fn nonce_cache_rejects_replays_per_device_and_stays_bounded() {
        let mut cache = NonceCache::with_capacity(3);
        assert!(cache.consume("w1", "n1"));
        assert!(
            !cache.consume("w1", "n1"),
            "a replayed nonce must be refused"
        );
        assert!(cache.consume("w2", "n1"), "nonces are per device");
        assert!(cache.consume("w1", "n2"));
        assert!(cache.consume("w1", "n3"));
        assert!(
            cache.consume("w1", "n1"),
            "the oldest entry was evicted at capacity"
        );
        assert!(!cache.consume("w1", "n3"));
        assert_eq!(NonceCache::default().capacity, 4096);
    }

    #[test]
    fn base64_is_standard_with_padding_for_keys_and_nonces() {
        let vectors = vectors();
        assert!(SharedKey::from_base64(text(&vectors, &["shared_key_base64"])).is_some());
        let url_safe = text(&vectors, &["shared_key_base64"])
            .replace('+', "-")
            .replace('/', "_")
            .replace('=', "");
        assert!(
            SharedKey::from_base64(&url_safe).is_none(),
            "url-safe keys are not Wristper's"
        );
        assert!(SharedKey::from_base64(&BASE64.encode([0u8; 31])).is_none());
        assert!(SharedKey::from_base64(&BASE64.encode([0u8; 33])).is_none());
        assert!(decode_nonce("ICEiIyQlJicoKSor").is_some());
        assert!(
            decode_nonce("ICEiIyQlJico").is_none(),
            "9 bytes is not a nonce"
        );
        assert!(
            decode_nonce("ICEiIyQlJicoKSo").is_none(),
            "padding is required"
        );
    }
}
