//! The relay's wire frames (`docs/cloud-relay.md` in Wristper,
//! `MacLocal/WristperCore/RelayProtocol.swift`): a watch request arrives as
//! one text frame `{"type":"req",…}` followed by its body in binary frames
//! of at most 256 KiB; Hex answers with one text frame
//! `{"type":"res","id","status","body"}`. The assembler is the only state,
//! one per connection. Nothing here opens a socket.

use std::collections::BTreeMap;

use serde_json::{Map, Value, json};

use super::refusal::Refusal;

/// A recording body (`/audio`, and the audio inside `/e2ee`).
pub(crate) const MAX_AUDIO_BYTES: usize = 20 * 1024 * 1024;
/// What an `/e2ee` envelope may add over the audio: the 4-byte head length,
/// the head JSON (at most 4096 bytes itself), and the GCM tag.
pub(crate) const E2EE_OVERHEAD: usize = 4096;
pub(crate) const E2EE_BODY_LIMIT: usize = MAX_AUDIO_BYTES + E2EE_OVERHEAD;
pub(crate) const QUERY_BODY_LIMIT: usize = 8192;
pub(crate) const PAIRING_REQUEST_BODY_LIMIT: usize = 4096;
pub(crate) const SMALL_BODY_LIMIT: usize = 1024;

pub(crate) const E2EE_PATH: &str = "/e2ee";
pub(crate) const E2EE_QUERY_PATH: &str = "/e2ee/query";

/// Who may call an endpoint (`AccessTier` in Wristper).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Tier {
    /// No token: pairing, and the envelopes, which carry their own.
    Open,
    /// A paired watch.
    Device,
    /// The Mac itself; never reachable from the cloud.
    Master,
}

/// An endpoint the cloud may reach, with the checks the network layer runs
/// before reading its body (`ReceiverRoute`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Route {
    pub(crate) method: &'static str,
    pub(crate) path: &'static str,
    pub(crate) tier: Tier,
    /// 0 for a GET: no body is read at all.
    pub(crate) body_limit: usize,
    pub(crate) body_required: bool,
    /// Answered `e2ee-required` (403) for every authenticated caller before
    /// its body is read. Plaintext `/audio` only ever came from pairings
    /// without a key, which Hex does not support; the watch reads the 403
    /// as "pair again".
    pub(crate) encrypted_only: bool,
}

const fn get(path: &'static str, tier: Tier) -> Route {
    Route {
        method: "GET",
        path,
        tier,
        body_limit: 0,
        body_required: false,
        encrypted_only: false,
    }
}

const fn post(path: &'static str, tier: Tier, body_limit: usize, body_required: bool) -> Route {
    Route {
        method: "POST",
        path,
        tier,
        body_limit,
        body_required,
        encrypted_only: false,
    }
}

const fn encrypted_only(route: Route) -> Route {
    Route {
        encrypted_only: true,
        ..route
    }
}

/// Phase 1: pairing, health, dictation, and the encrypted envelopes. The
/// relay forwards anything under `/w/<room>/`; everything not listed here
/// (or in `AGENT_ROUTES` once phase 2 lands) answers 404 before any handler
/// runs, so the Mac-only `master` endpoints never face the internet.
pub(crate) const CLOUD_ROUTES: &[Route] = &[
    get("/pairing/result", Tier::Open),
    get("/health", Tier::Device),
    get("/status", Tier::Device),
    post(
        "/pairing/request",
        Tier::Open,
        PAIRING_REQUEST_BODY_LIMIT,
        true,
    ),
    post("/ping", Tier::Device, SMALL_BODY_LIMIT, false),
    post("/focus", Tier::Device, SMALL_BODY_LIMIT, true),
    post("/enter", Tier::Device, SMALL_BODY_LIMIT, true),
    post("/undo", Tier::Device, SMALL_BODY_LIMIT, true),
    encrypted_only(post("/audio", Tier::Device, MAX_AUDIO_BYTES, true)),
    post(E2EE_PATH, Tier::Open, E2EE_BODY_LIMIT, true),
    post(E2EE_QUERY_PATH, Tier::Open, QUERY_BODY_LIMIT, true),
];

/// The agent routes Wristper also admits from the cloud. Phase 2 moves them
/// into `CLOUD_ROUTES` with their handlers; until then they 404 like any
/// unknown path, and the watch hides its agent entry because `/health`
/// reports no agent support.
pub(crate) const AGENT_ROUTES: &[Route] = &[
    get("/agent/projects", Tier::Device),
    get("/agent/sessions", Tier::Device),
    get("/agent/session", Tier::Device),
    get("/agent/recent", Tier::Device),
    get("/agent/usage", Tier::Device),
    post("/agent/confirm", Tier::Device, SMALL_BODY_LIMIT, true),
];

pub(crate) fn cloud_route(method: &str, path: &str) -> Option<&'static Route> {
    CLOUD_ROUTES
        .iter()
        .find(|route| route.method == method && route.path == path)
}

/// What an authenticated plaintext call to `route` gets before its body
/// is read.
pub(crate) fn plaintext_refusal(route: &Route) -> Option<Refusal> {
    route.encrypted_only.then_some(Refusal::E2eeRequired)
}

/// The body checks the local server and the relay gateway share
/// (`RelayGateway.respond`), run once the whole body is in hand.
pub(crate) fn body_refusal(route: &Route, body_len: usize) -> Option<Refusal> {
    if body_len > route.body_limit {
        Some(Refusal::TooLarge)
    } else if body_len == 0 && route.body_required {
        Some(Refusal::EmptyBody)
    } else {
        None
    }
}

/// A pairing code as the watch shows it. Wristper's 4-digit codes came from
/// watch apps that pair without a key, which Hex does not support.
pub(crate) const CLAIM_CODE_LENGTH: usize = 6;

pub(crate) fn is_valid_claim_code(code: &str) -> bool {
    code.len() == CLAIM_CODE_LENGTH && code.bytes().all(|byte| byte.is_ascii_digit())
}

/// The JSON part of a request frame.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RequestHead {
    pub(crate) id: String,
    pub(crate) method: String,
    pub(crate) path: String,
    pub(crate) query: BTreeMap<String, String>,
    /// Lower-cased names; the relay forwards only `content-type` and
    /// `x-wristper-*`.
    pub(crate) headers: BTreeMap<String, String>,
    pub(crate) body_len: usize,
}

impl RequestHead {
    pub(crate) fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(name).map(String::as_str)
    }
}

/// What one frame from the relay amounts to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Inbound {
    /// A request whose body is complete.
    Request(RequestHead, Vec<u8>),
    /// A frame that broke the framing rules: answer with this status and
    /// error and hand nothing to the service.
    Rejected {
        id: String,
        status: u16,
        error: &'static str,
    },
    ClaimOk {
        code: String,
        device_id: String,
        device_name: String,
    },
    ClaimFailed {
        code: String,
        error: String,
    },
    Pong,
}

/// Rebuilds requests from a request frame plus the binary frames after it.
#[derive(Debug, Default)]
pub(crate) struct Assembler {
    pending: Option<(RequestHead, Vec<u8>)>,
    /// Bytes of an oversized body still to arrive. They are swallowed so
    /// they never read as the next request's body.
    discarding: usize,
}

impl Assembler {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Reads fields as Wristper's `JSONSerialization` does: a field of the
    /// wrong type counts as absent instead of voiding the whole frame.
    pub(crate) fn receive_text(&mut self, text: &str) -> Vec<Inbound> {
        if text == "pong" {
            return vec![Inbound::Pong];
        }
        let Ok(Value::Object(frame)) = serde_json::from_str::<Value>(text) else {
            return Vec::new();
        };
        let field = |key: &str| frame.get(key).and_then(Value::as_str).map(str::to_owned);
        match field("type").as_deref() {
            Some("req") => self.receive_request(&frame),
            Some("claim_ok") => vec![Inbound::ClaimOk {
                code: field("code").unwrap_or_default(),
                device_id: field("device_id").unwrap_or_default(),
                device_name: field("name").unwrap_or_default(),
            }],
            Some("claim_failed") => vec![Inbound::ClaimFailed {
                code: field("code").unwrap_or_default(),
                error: field("error").unwrap_or_else(|| "code_not_found".to_owned()),
            }],
            _ => Vec::new(),
        }
    }

    pub(crate) fn receive_binary(&mut self, bytes: &[u8]) -> Vec<Inbound> {
        if self.discarding > 0 {
            self.discarding = self.discarding.saturating_sub(bytes.len());
            return Vec::new();
        }
        let Some((head, mut body)) = self.pending.take() else {
            return Vec::new();
        };
        body.extend_from_slice(bytes);
        if body.len() > head.body_len {
            return vec![Inbound::Rejected {
                id: head.id,
                status: 400,
                error: "body-overflow",
            }];
        }
        if body.len() == head.body_len {
            return vec![Inbound::Request(head, body)];
        }
        self.pending = Some((head, body));
        Vec::new()
    }

    fn receive_request(&mut self, frame: &Map<String, Value>) -> Vec<Inbound> {
        let field = |key: &str| frame.get(key).and_then(Value::as_str).map(str::to_owned);
        let (Some(id), Some(method), Some(path), Some(body_len)) = (
            field("id"),
            field("method"),
            field("path"),
            frame.get("body_len").and_then(body_length),
        ) else {
            return Vec::new();
        };
        if id.is_empty() {
            return Vec::new();
        }
        let mut output = Vec::new();
        // A new request before the previous body finished: give up on the
        // old one now rather than let the relay wait out its 60 s.
        if let Some((stale, _)) = self.pending.take() {
            output.push(Inbound::Rejected {
                id: stale.id,
                status: 400,
                error: "incomplete-body",
            });
        }
        self.discarding = 0;
        let head = RequestHead {
            id,
            method,
            query: string_map(frame.get("query")),
            headers: header_map(frame.get("headers")),
            body_len,
            path,
        };
        let maximum = if head.path == E2EE_PATH {
            E2EE_BODY_LIMIT
        } else {
            MAX_AUDIO_BYTES
        };
        if body_len > maximum {
            self.discarding = body_len;
            output.push(Inbound::Rejected {
                id: head.id,
                status: 413,
                error: "too-large",
            });
        } else if body_len == 0 {
            output.push(Inbound::Request(head, Vec::new()));
        } else {
            self.pending = Some((head, Vec::new()));
        }
        output
    }
}

/// A JSON number that is a whole, non-negative byte count. `5.0` counts,
/// as `NSNumber.intValue` reads it; `5.5` does not, since truncating it
/// would misframe every body after it.
fn body_length(value: &Value) -> Option<usize> {
    if let Some(whole) = value.as_u64() {
        return usize::try_from(whole).ok();
    }
    let float = value.as_f64()?;
    (float >= 0.0 && float.fract() == 0.0 && float <= usize::MAX as f64).then_some(float as usize)
}

fn string_entries(value: Option<&Value>) -> impl Iterator<Item = (&String, &str)> {
    value
        .and_then(Value::as_object)
        .into_iter()
        .flatten()
        .filter_map(|(key, value)| Some((key, value.as_str()?)))
}

fn string_map(value: Option<&Value>) -> BTreeMap<String, String> {
    string_entries(value)
        .map(|(key, text)| (key.clone(), text.to_owned()))
        .collect()
}

/// Header names are case-insensitive: lower-cased here, and the first of
/// two names that differ only in case wins (`RelayGateway.respond`).
fn header_map(value: Option<&Value>) -> BTreeMap<String, String> {
    let mut headers = BTreeMap::new();
    for (key, text) in string_entries(value) {
        headers
            .entry(key.to_ascii_lowercase())
            .or_insert_with(|| text.to_owned());
    }
    headers
}

/// The reply frame. The body travels as a JSON string inside the frame;
/// the relay hands it to the watch as `application/json`.
pub(crate) fn response_frame(id: &str, status: u16, body: &[u8]) -> String {
    json!({
        "type": "res",
        "id": id,
        "status": status,
        "body": String::from_utf8_lossy(body),
    })
    .to_string()
}

pub(crate) fn response_frame_json(id: &str, status: u16, body: &Value) -> String {
    response_frame(id, status, body.to_string().as_bytes())
}

/// Asks the relay to claim the code the watch shows, naming this Mac.
pub(crate) fn claim_frame(code: &str, name: &str) -> String {
    json!({"type": "claim", "code": code, "name": name}).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request_frame(id: &str, method: &str, path: &str, body_len: usize) -> String {
        format!(
            r#"{{"type":"req","id":"{id}","method":"{method}","path":"{path}","query":{{"id":"abc"}},"headers":{{"x-wristper-token":"t","content-type":"audio/mp4"}},"body_len":{body_len}}}"#
        )
    }

    fn only_request(events: Vec<Inbound>) -> (RequestHead, Vec<u8>) {
        match events.as_slice() {
            [Inbound::Request(head, body)] => (head.clone(), body.clone()),
            other => panic!("expected one request, got {other:?}"),
        }
    }

    #[test]
    fn claim_codes_are_exactly_six_ascii_digits() {
        for valid in ["000000", "217400", "004217"] {
            assert!(is_valid_claim_code(valid), "{valid}");
        }
        for invalid in [
            "",
            "0000",
            "2174",
            "217",
            "21745",
            "2174000",
            "21a4",
            "２１７４",
            " 2174",
            "-174",
        ] {
            assert!(!is_valid_claim_code(invalid), "{invalid}");
        }
    }

    #[test]
    fn only_phase_one_watch_routes_are_allowed_from_the_cloud() {
        for (method, path) in [
            ("GET", "/pairing/result"),
            ("GET", "/health"),
            ("GET", "/status"),
            ("POST", "/pairing/request"),
            ("POST", "/focus"),
            ("POST", "/enter"),
            ("POST", "/undo"),
            ("POST", "/audio"),
            ("POST", "/e2ee"),
            ("POST", "/e2ee/query"),
            ("POST", "/ping"),
        ] {
            assert!(cloud_route(method, path).is_some(), "{method} {path}");
        }
        for (method, path) in [
            ("GET", "/paired"),
            ("GET", "/history"),
            ("GET", "/activity"),
            ("GET", "/output"),
            ("POST", "/output"),
            ("POST", "/pairing/claim"),
            ("POST", "/paired/revoke"),
            ("POST", "/mac-audio"),
            ("POST", "/models/select"),
            ("GET", "/license"),
            ("POST", "/status"),
            ("GET", "/audio"),
            ("GET", "/agent/sessions"),
            ("POST", "/agent/confirm"),
        ] {
            assert!(cloud_route(method, path).is_none(), "{method} {path}");
        }
        for route in AGENT_ROUTES {
            assert!(route.path.starts_with("/agent/"));
            assert!(cloud_route(route.method, route.path).is_none());
        }
    }

    #[test]
    fn plaintext_audio_is_refused_as_e2ee_required_and_nothing_else_is() {
        let audio = cloud_route("POST", "/audio").unwrap();
        let refusal = plaintext_refusal(audio).unwrap();
        assert_eq!(
            (refusal.status(), refusal.body()["error"].as_str()),
            (403, Some("e2ee-required"))
        );
        for route in CLOUD_ROUTES.iter().filter(|route| route.path != "/audio") {
            assert_eq!(plaintext_refusal(route), None, "{}", route.path);
        }
    }

    #[test]
    fn route_body_limits_match_wristper() {
        let limit = |method, path| cloud_route(method, path).unwrap().body_limit;
        assert_eq!(limit("POST", "/audio"), 20 * 1024 * 1024);
        assert_eq!(limit("POST", "/e2ee"), 20 * 1024 * 1024 + 4096);
        assert_eq!(limit("POST", "/e2ee/query"), 8192);
        assert_eq!(limit("POST", "/focus"), 1024);
        assert_eq!(limit("POST", "/pairing/request"), 4096);
        let focus = cloud_route("POST", "/focus").unwrap();
        assert_eq!(body_refusal(focus, 1025), Some(Refusal::TooLarge));
        assert_eq!(body_refusal(focus, 0), Some(Refusal::EmptyBody));
        assert_eq!(body_refusal(focus, 1024), None);
        let ping = cloud_route("POST", "/ping").unwrap();
        assert_eq!(body_refusal(ping, 0), None);
    }

    #[test]
    fn request_without_body_completes_on_the_text_frame() {
        let mut assembler = Assembler::new();
        let (head, body) =
            only_request(assembler.receive_text(&request_frame("r1", "GET", "/health", 0)));
        assert_eq!(head.path, "/health");
        assert_eq!(head.query.get("id").map(String::as_str), Some("abc"));
        assert_eq!(head.header("x-wristper-token"), Some("t"));
        assert!(body.is_empty());
    }

    #[test]
    fn body_chunks_are_joined_in_order() {
        let mut assembler = Assembler::new();
        assert!(
            assembler
                .receive_text(&request_frame("r1", "POST", "/audio", 6))
                .is_empty()
        );
        assert!(assembler.receive_binary(b"abc").is_empty());
        let (_, body) = only_request(assembler.receive_binary(b"def"));
        assert_eq!(body, b"abcdef");
    }

    #[test]
    fn two_requests_back_to_back_stay_separate() {
        let mut assembler = Assembler::new();
        let _ = assembler.receive_text(&request_frame("a", "POST", "/audio", 2));
        let (one, body_one) = only_request(assembler.receive_binary(b"xy"));
        let _ = assembler.receive_text(&request_frame("b", "POST", "/audio", 1));
        let (two, body_two) = only_request(assembler.receive_binary(b"z"));
        assert_eq!((one.id.as_str(), two.id.as_str()), ("a", "b"));
        assert_eq!((body_one, body_two), (b"xy".to_vec(), b"z".to_vec()));
    }

    #[test]
    fn extra_body_bytes_reject_the_request() {
        let mut assembler = Assembler::new();
        let _ = assembler.receive_text(&request_frame("r1", "POST", "/audio", 2));
        assert_eq!(
            assembler.receive_binary(b"abc"),
            vec![Inbound::Rejected {
                id: "r1".into(),
                status: 400,
                error: "body-overflow"
            }]
        );
        assert!(assembler.receive_binary(b"late").is_empty());
    }

    #[test]
    fn new_request_abandons_an_incomplete_one() {
        let mut assembler = Assembler::new();
        let _ = assembler.receive_text(&request_frame("old", "POST", "/audio", 5));
        let events = assembler.receive_text(&request_frame("new", "GET", "/health", 0));
        assert_eq!(
            events.first(),
            Some(&Inbound::Rejected {
                id: "old".into(),
                status: 400,
                error: "incomplete-body"
            })
        );
        match events.last() {
            Some(Inbound::Request(head, _)) => assert_eq!(head.id, "new"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn oversized_body_is_rejected_and_its_chunks_are_swallowed() {
        let mut assembler = Assembler::new();
        let too_big = MAX_AUDIO_BYTES + 1;
        assert_eq!(
            assembler.receive_text(&request_frame("r1", "POST", "/audio", too_big)),
            vec![Inbound::Rejected {
                id: "r1".into(),
                status: 413,
                error: "too-large"
            }]
        );
        assert!(assembler.receive_binary(&vec![0; too_big]).is_empty());
        let (head, _) =
            only_request(assembler.receive_text(&request_frame("n", "GET", "/health", 0)));
        assert_eq!(head.id, "n");
    }

    #[test]
    fn e2ee_gets_the_envelope_overhead_and_nothing_else_does() {
        let mut assembler = Assembler::new();
        let with_overhead = MAX_AUDIO_BYTES + E2EE_OVERHEAD;
        assert!(
            assembler
                .receive_text(&request_frame("e", "POST", "/e2ee", with_overhead))
                .is_empty()
        );
        let mut other = Assembler::new();
        assert_eq!(
            other.receive_text(&request_frame("a", "POST", "/audio", with_overhead)),
            vec![Inbound::Rejected {
                id: "a".into(),
                status: 413,
                error: "too-large"
            }]
        );
        let mut over = Assembler::new();
        assert_eq!(
            over.receive_text(&request_frame("e2", "POST", "/e2ee", with_overhead + 1)),
            vec![Inbound::Rejected {
                id: "e2".into(),
                status: 413,
                error: "too-large"
            }]
        );
    }

    #[test]
    fn control_frames_and_garbage() {
        let mut assembler = Assembler::new();
        assert_eq!(assembler.receive_text("pong"), vec![Inbound::Pong]);
        assert_eq!(
            assembler.receive_text(
                r#"{"type":"claim_ok","code":"217400","device_id":"w1","name":"Apple Watch"}"#
            ),
            vec![Inbound::ClaimOk {
                code: "217400".into(),
                device_id: "w1".into(),
                device_name: "Apple Watch".into()
            }]
        );
        assert_eq!(
            assembler.receive_text(
                r#"{"type":"claim_failed","code":"2174","error":"too_many_attempts"}"#
            ),
            vec![Inbound::ClaimFailed {
                code: "2174".into(),
                error: "too_many_attempts".into()
            }]
        );
        assert_eq!(
            assembler.receive_text(r#"{"type":"claim_failed","code":"2174"}"#),
            vec![Inbound::ClaimFailed {
                code: "2174".into(),
                error: "code_not_found".into()
            }]
        );
        assert!(assembler.receive_text("not json").is_empty());
        assert!(
            assembler
                .receive_text(r#"{"type":"req","id":"x"}"#)
                .is_empty()
        );
        assert!(
            assembler
                .receive_text(
                    r#"{"type":"req","id":"","method":"GET","path":"/health","body_len":0}"#
                )
                .is_empty()
        );
        assert!(
            assembler
                .receive_text(
                    r#"{"type":"req","id":"x","method":"GET","path":"/health","body_len":-1}"#
                )
                .is_empty()
        );
        assert!(
            assembler
                .receive_text(r#"{"type":"push_result","id":"p"}"#)
                .is_empty()
        );
        assert!(assembler.receive_binary(b"orphan").is_empty());
    }

    #[test]
    fn mistyped_fields_read_as_absent_like_jsonserialization() {
        let mut assembler = Assembler::new();
        assert_eq!(
            assembler.receive_text(r#"{"type":"claim_failed","code":2174,"error":7}"#),
            vec![Inbound::ClaimFailed {
                code: String::new(),
                error: "code_not_found".into()
            }]
        );
        assert_eq!(
            assembler
                .receive_text(r#"{"type":"claim_ok","code":"217400","device_id":null,"name":3}"#),
            vec![Inbound::ClaimOk {
                code: "217400".into(),
                device_id: String::new(),
                device_name: String::new()
            }]
        );
    }

    #[test]
    fn a_whole_float_body_length_counts_and_a_fractional_one_does_not() {
        let frame = |length: &str| {
            format!(
                r#"{{"type":"req","id":"f","method":"POST","path":"/focus","body_len":{length}}}"#
            )
        };
        let mut assembler = Assembler::new();
        assert!(assembler.receive_text(&frame("2.0")).is_empty());
        let (head, body) = only_request(assembler.receive_binary(b"ok"));
        assert_eq!((head.body_len, body.as_slice()), (2, b"ok".as_slice()));
        for bad in ["2.5", "-1.0", "\"2\"", "true"] {
            assert!(
                Assembler::new().receive_text(&frame(bad)).is_empty(),
                "{bad}"
            );
        }
    }

    #[test]
    fn header_names_are_lower_cased_and_the_first_spelling_wins() {
        let mut assembler = Assembler::new();
        let (head, _) = only_request(assembler.receive_text(
            r#"{"type":"req","id":"h","method":"GET","path":"/status","headers":{"X-Wristper-Token":"first","x-wristper-token":"second","Content-Type":"audio/mp4"},"body_len":0}"#,
        ));
        assert_eq!(head.header("x-wristper-token"), Some("first"));
        assert_eq!(head.header("content-type"), Some("audio/mp4"));
        assert_eq!(head.headers.len(), 2);
    }

    #[test]
    fn non_string_query_and_header_values_are_dropped() {
        let mut assembler = Assembler::new();
        let (head, _) = only_request(assembler.receive_text(
            r#"{"type":"req","id":"q","method":"GET","path":"/status","query":{"id":"abc","n":1},"headers":{"x-wristper-token":"t","bad":true},"body_len":0}"#,
        ));
        assert_eq!(head.query.len(), 1);
        assert_eq!(head.headers.len(), 1);
    }

    #[test]
    fn response_frame_carries_status_and_json_body_string() {
        let frame: Value = serde_json::from_str(&response_frame_json(
            "r1",
            200,
            &json!({"ok": true, "path": "/a/b"}),
        ))
        .unwrap();
        assert_eq!(frame["type"], "res");
        assert_eq!(frame["id"], "r1");
        assert_eq!(frame["status"], 200);
        let body: Value = serde_json::from_str(frame["body"].as_str().unwrap()).unwrap();
        assert_eq!(body["ok"], true);
        assert_eq!(body["path"], "/a/b");
        assert_eq!(
            response_frame("r1", 200, br#"{"ok":true}"#),
            r#"{"body":"{\"ok\":true}","id":"r1","status":200,"type":"res"}"#
        );
    }

    #[test]
    fn claim_frame_names_the_mac() {
        let frame: Value = serde_json::from_str(&claim_frame("2174", "Glen 的 MacBook")).unwrap();
        assert_eq!(frame["type"], "claim");
        assert_eq!(frame["code"], "2174");
        assert_eq!(frame["name"], "Glen 的 MacBook");
        assert_eq!(frame.as_object().unwrap().len(), 3);
    }
}
