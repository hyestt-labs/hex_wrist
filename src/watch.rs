//! The Apple Watch (Wristper) protocol core: what Hex must speak, byte for
//! byte, to the watch app that already ships. Wristper's Mac receiver is the
//! reference (`artifacts/wristper-migration-20261104/wristper-protocol.md`
//! cites it by file and line). Everything here is pure: no sockets, no
//! threads, no UI, no engine. The relay WebSocket client, the HTTP handlers,
//! and the paste pipeline sit on top in later units.
//!
//! - `audio`: the watch's AAC `.m4a` decoded into the engine's samples.
//! - `relay`: the relay WebSocket client on its own thread.
//! - `delivery`: the handle a recording travels with through the dictation worker.
//! - `desktop`: the focused field and the keys, behind `Screen` and one `watch-ax` thread.
//! - `service`: the one `watch` thread that owns the state and answers requests.
//! - `frame`: the relay's request/response frames and the body assembler.
//! - `e2ee`: P-256 ECDH + HKDF pairing keys and the AES-256-GCM envelopes.
//! - `pairing`: paired watches, their tokens and keys, and the claim approval.
//! - `delivered`: the 24 h dedupe table a resent recording is answered from.
//! - `recording`: a recording's state machine and the `/status` projection.
//! - `refusal`: every error body the watch reads.
//! - `suspect`: Wristper's silence-hallucination and script rules.

#![allow(dead_code)]

pub(crate) mod audio;
pub(crate) mod delivered;
pub(crate) mod delivery;
pub(crate) mod desktop;
pub(crate) mod e2ee;
pub(crate) mod field;
pub(crate) mod frame;
pub(crate) mod pairing;
pub(crate) mod recording;
pub(crate) mod refusal;
pub(crate) mod relay;
#[cfg(target_os = "macos")]
pub(crate) mod screen;
pub(crate) mod service;
pub(crate) mod suspect;
pub(crate) mod zh;

use std::io::{self, Write};
use std::path::Path;
use std::sync::OnceLock;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

static SERVICE: OnceLock<service::WatchService> = OnceLock::new();

/// Makes the running service reachable from `hex watch pair` and the window.
pub(crate) fn install(service: service::WatchService) {
    if SERVICE.set(service).is_err() {
        tracing::warn!("the watch service was installed twice; keeping the first");
    }
}

pub(crate) fn service() -> Option<&'static service::WatchService> {
    SERVICE.get()
}

/// Relay override for a local relay in tests; release builds take only
/// https/wss (`relay::resolve_base`).
pub(crate) const RELAY_BASE_ENV: &str = "HEX_WATCH_RELAY_BASE";

/// Starts the service for the macOS app and installs it. On by default, no
/// toggle: this fork is Wristper, and dictating from the watch is its
/// reason to exist. A failure is logged and the app runs without it.
#[cfg(target_os = "macos")]
pub(crate) fn start_in_app(
    controls: std::sync::mpsc::SyncSender<crate::recognition::RecognitionControl>,
) -> Option<service::WatchService> {
    use service::{ListenerDictation, WatchConfig, WatchService};
    let support_dir = match crate::app_paths::support_dir() {
        Ok(dir) => dir,
        Err(error) => {
            tracing::warn!(%error, "the Apple Watch service did not start: no data folder");
            return None;
        }
    };
    let timing = relay::RelayTiming::default();
    let relay_base = relay::resolve_base(
        &std::env::var(RELAY_BASE_ENV).unwrap_or_default(),
        timing.allow_insecure,
    );
    let config = WatchConfig {
        support_dir,
        relay_base,
        mac_name: computer_name(),
        app_version: env!("CARGO_PKG_VERSION").to_owned(),
        timing,
    };
    match WatchService::start(
        config,
        Box::new(ListenerDictation::new(controls)),
        Box::new(pairing::keychain::KeychainSecretStore::new()),
    ) {
        Ok(service) => {
            install(service.clone());
            Some(service)
        }
        Err(error) => {
            tracing::warn!(%error, "the Apple Watch service did not start");
            None
        }
    }
}

/// What the watch shows for this Mac: the computer name from System
/// Settings (`SCDynamicStoreCopyComputerName` in Wristper), else `Mac`.
#[cfg(target_os = "macos")]
fn computer_name() -> String {
    use objc2_core_foundation::{CFRetained, CFString};
    use std::ptr::NonNull;
    #[link(name = "SystemConfiguration", kind = "framework")]
    unsafe extern "C" {
        fn SCDynamicStoreCopyComputerName(
            store: *const std::ffi::c_void,
            encoding: *mut u32,
        ) -> *mut CFString;
    }
    // SAFETY: a null store and null encoding are documented as allowed; the
    // returned string follows the Create rule, so `from_raw` takes the +1.
    let name = unsafe {
        NonNull::new(SCDynamicStoreCopyComputerName(
            std::ptr::null(),
            std::ptr::null_mut(),
        ))
        .map(|raw| CFRetained::from_raw(raw))
    };
    name.map(|name| name.to_string())
        .filter(|name| !name.trim().is_empty())
        .unwrap_or_else(|| "Mac".to_owned())
}

/// `hex watch pair <code>`.
pub(crate) fn pair(code: &str) -> crate::control::ControlReply {
    use crate::control::ControlReply;
    let Some(service) = service() else {
        return ControlReply::error(
            service::ClaimError::Unavailable.wire(),
            service::ClaimError::Unavailable.message(),
        );
    };
    match service.claim(code.trim()) {
        Ok(claimed) => ControlReply::done(format!(
            "Claimed the code for {} ({}); the watch finishes pairing by itself within 2 minutes",
            claimed.device_name, claimed.device_id
        )),
        Err(error) => ControlReply::error(error.wire(), error.message()),
    }
}

/// `X-Wristper-Protocol` this receiver speaks.
pub(crate) const PROTOCOL_VERSION: u32 = 4;
/// The oldest watch protocol still served; 1 is a watch that sends no
/// version header at all.
pub(crate) const MINIMUM_PEER_PROTOCOL: u32 = 1;

pub(crate) const PROTOCOL_HEADER: &str = "x-wristper-protocol";
pub(crate) const APP_VERSION_HEADER: &str = "x-wristper-app-version";
pub(crate) const TOKEN_HEADER: &str = "x-wristper-token";
pub(crate) const DEVICE_HEADER: &str = "x-wristper-device";
pub(crate) const NONCE_HEADER: &str = "x-wristper-nonce";
pub(crate) const SENT_AT_HEADER: &str = "x-wristper-sent-at";
pub(crate) const ID_HEADER: &str = "x-wristper-id";
pub(crate) const LANGUAGE_HEADER: &str = "x-wristper-language";
pub(crate) const FOCUS_HEADER: &str = "x-wristper-focus";

/// Replaces `path` atomically with a file that is owner-only from its
/// first byte: the staging file is created 0600, its folder 0700, as
/// Wristper writes `delivered-ids.json` and `paired-devices.json`.
pub(crate) fn write_owner_only(path: &Path, contents: &[u8]) -> io::Result<()> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let mut folder = std::fs::DirBuilder::new();
    folder.recursive(true);
    let mut staging_options = std::fs::OpenOptions::new();
    staging_options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
        folder.mode(0o700);
        staging_options.mode(0o600);
    }
    folder.create(parent)?;
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    let mut nonce = [0u8; 8];
    getrandom::fill(&mut nonce).map_err(io::Error::other)?;
    name.push(format!(".{:016x}.tmp", u64::from_le_bytes(nonce)));
    let staging = path.with_file_name(name);
    let written = (|| {
        let mut file = staging_options.open(&staging)?;
        file.write_all(contents)?;
        file.sync_all()?;
        std::fs::rename(&staging, path)
    })();
    if let Err(error) = written {
        let _ = std::fs::remove_file(&staging);
        return Err(error);
    }
    crate::durable::sync_directory(parent)
}

/// A recording's id: the UUID the watch generated, 36 ASCII hex digits and
/// hyphens (`TrialState.isValidID`).
pub(crate) fn is_valid_recording_id(value: &str) -> bool {
    value.len() == 36
        && value
            .bytes()
            .all(|byte| byte == b'-' || byte.is_ascii_hexdigit())
}

/// What a watch reported about itself in its request headers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PeerInfo {
    pub(crate) protocol_version: u32,
    /// At most 32 characters; absent when the header is missing or empty.
    pub(crate) app_version: Option<String>,
}

impl PeerInfo {
    /// `headers` must already be lower-cased. A number below 1 (`-1`) reads
    /// as 0, too old, as Swift's `Int(_:)` would compare it; text that is
    /// no number at all reads as 1, a watch that sent no header.
    pub(crate) fn from_headers<'a>(mut header: impl FnMut(&str) -> Option<&'a str>) -> Self {
        let protocol_version = header(PROTOCOL_HEADER)
            .and_then(|value| value.parse::<i64>().ok())
            .map_or(1, |version| version.clamp(0, i64::from(u32::MAX)) as u32);
        let app_version = header(APP_VERSION_HEADER)
            .filter(|value| !value.is_empty())
            .map(|value| value.chars().take(32).collect());
        Self {
            protocol_version,
            app_version,
        }
    }

    pub(crate) fn is_too_old(&self) -> bool {
        self.protocol_version < MINIMUM_PEER_PROTOCOL
    }
}

/// Wristper's timestamp text, `2026-11-04T12:34:56.123456+00:00`: Python's
/// `datetime.now(timezone.utc).isoformat()`, so records sort as strings and
/// files written by the Swift app read back unchanged.
pub(crate) struct Timestamp;

impl Timestamp {
    pub(crate) fn string(at: SystemTime) -> String {
        let since_epoch = at.duration_since(UNIX_EPOCH).unwrap_or(Duration::ZERO);
        let seconds = since_epoch.as_secs();
        let micros = since_epoch.subsec_micros();
        let (year, month, day) = civil_from_days((seconds / 86_400) as i64);
        let of_day = seconds % 86_400;
        format!(
            "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{micros:06}+00:00",
            of_day / 3_600,
            of_day % 3_600 / 60,
            of_day % 60
        )
    }

    /// Reads `string`'s output, with or without the fraction, ending in `Z`
    /// or `±HH:MM` (`Timestamp.parsers` in Wristper). Like its
    /// `DateFormatter`, a date that does not exist (Feb 30, 24:00, second
    /// 60) is not one.
    pub(crate) fn parse(text: &str) -> Option<SystemTime> {
        let bytes = text.as_bytes();
        let digits = |range: std::ops::Range<usize>| -> Option<u64> {
            let slice = text.get(range)?;
            if slice.bytes().all(|byte| byte.is_ascii_digit()) {
                slice.parse().ok()
            } else {
                None
            }
        };
        let separators = [(4, b'-'), (7, b'-'), (10, b'T'), (13, b':'), (16, b':')];
        if bytes.len() < 20
            || separators
                .iter()
                .any(|&(at, separator)| bytes[at] != separator)
        {
            return None;
        }
        let (year, month, day) = (digits(0..4)?, digits(5..7)?, digits(8..10)?);
        let (hour, minute, second) = (digits(11..13)?, digits(14..16)?, digits(17..19)?);
        let mut cursor = 19;
        let mut micros = 0u64;
        if bytes.get(cursor) == Some(&b'.') {
            let start = cursor + 1;
            let end = start + text[start..].bytes().take_while(u8::is_ascii_digit).count();
            let fraction = text.get(start..end)?;
            if fraction.is_empty() {
                return None;
            }
            let padded = format!("{fraction:0<6}");
            micros = padded.get(..6)?.parse().ok()?;
            cursor = end;
        }
        let offset_seconds: i64 = match bytes.get(cursor) {
            Some(b'Z') if cursor + 1 == bytes.len() => 0,
            Some(sign @ (b'+' | b'-')) => {
                if bytes.get(cursor + 3) != Some(&b':') || cursor + 6 != bytes.len() {
                    return None;
                }
                let hours = digits(cursor + 1..cursor + 3)?;
                let minutes = digits(cursor + 4..cursor + 6)?;
                if hours > 23 || minutes > 59 {
                    return None;
                }
                let magnitude = (hours * 3_600 + minutes * 60) as i64;
                if *sign == b'+' { magnitude } else { -magnitude }
            }
            _ => return None,
        };
        if !(1..=12).contains(&month)
            || day == 0
            || day > days_in_month(year, month)
            || hour > 23
            || minute > 59
            || second > 59
        {
            return None;
        }
        let days = days_from_civil(year as i64, month as u32, day as u32);
        let local = days * 86_400 + (hour * 3_600 + minute * 60 + second) as i64;
        let utc = local - offset_seconds;
        if utc < 0 {
            return None;
        }
        Some(UNIX_EPOCH + Duration::new(utc as u64, (micros * 1_000) as u32))
    }
}

fn days_in_month(year: u64, month: u64) -> u64 {
    match month {
        2 if year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400)) => {
            29
        }
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

// Howard Hinnant's proleptic Gregorian conversions.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let yoe = year.rem_euclid(400);
    let mp = if month > 2 { month - 3 } else { month + 9 } as i64;
    let doy = (153 * mp + 2) / 5 + day as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timestamp_matches_wristpers_isoformat() {
        let at = UNIX_EPOCH + Duration::new(1_762_259_696, 123_456_789);
        assert_eq!(Timestamp::string(at), "2025-11-04T12:34:56.123456+00:00");
        assert_eq!(
            Timestamp::string(UNIX_EPOCH),
            "1970-01-01T00:00:00.000000+00:00"
        );
    }

    #[test]
    fn timestamp_parses_both_wristper_shapes_and_round_trips() {
        let at = UNIX_EPOCH + Duration::new(1_762_259_696, 123_456_000);
        assert_eq!(Timestamp::parse(&Timestamp::string(at)), Some(at));
        assert_eq!(
            Timestamp::parse("2025-11-04T12:34:56+00:00"),
            Some(UNIX_EPOCH + Duration::from_secs(1_762_259_696))
        );
        assert_eq!(
            Timestamp::parse("2025-11-04T13:34:56Z"),
            Some(UNIX_EPOCH + Duration::from_secs(1_762_263_296))
        );
        assert_eq!(
            Timestamp::parse("2025-11-04T14:34:56+02:00"),
            Some(UNIX_EPOCH + Duration::from_secs(1_762_259_696))
        );
        assert_eq!(
            Timestamp::parse("2024-02-29T00:00:00+00:00"),
            Some(UNIX_EPOCH + Duration::from_secs(1_709_164_800))
        );
        assert_eq!(
            Timestamp::parse("2000-02-29T00:00:00Z"),
            Some(UNIX_EPOCH + Duration::from_secs(951_782_400))
        );
        for bad in [
            "",
            "2025-11-04",
            "2025-13-04T00:00:00+00:00",
            "yesterday",
            "2025-11-04T12:34:56+00:00junk",
            "2025-11-04T12:34:56.5+00:007",
            "2025-11-04T12:34:56Zjunk",
            "2025-11-04T12:34:56+00junk",
            "2025-11-04T12:34:56+00",
            "2025-11-04T12:34:56+0000",
            "2025-11-04T12:34:56+24:00",
            "2025-11-04T12:34:56+00:60",
            "2025-02-30T00:00:00+00:00",
            "2025-02-29T00:00:00+00:00",
            "1900-02-29T00:00:00+00:00",
            "2025-04-31T00:00:00+00:00",
            "2025-11-00T00:00:00+00:00",
            "2025-11-04T24:00:00+00:00",
            "2025-11-04T12:34:60+00:00",
        ] {
            assert_eq!(Timestamp::parse(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn timestamp_separators_must_sit_where_wristper_writes_them() {
        for bad in [
            "2025-11-04T12-34:56+00:00",
            "2025-11-04T12:34-56+00:00",
            "2025-11-04T12x34x56Z",
            "2025-11-04T12345:56Z",
            "2025-11-04T12:34:56,5+00:00",
            "2025/11/04T12:34:56Z",
            "2025-11-04 12:34:56Z",
        ] {
            assert_eq!(Timestamp::parse(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn recording_ids_are_36_hex_and_hyphen_characters() {
        assert!(is_valid_recording_id(
            "6F9619FF-8B86-D011-B42D-00C04FC964FF"
        ));
        assert!(is_valid_recording_id(
            "6f9619ff-8b86-d011-b42d-00c04fc964ff"
        ));
        assert!(!is_valid_recording_id("6f9619ff"));
        assert!(!is_valid_recording_id(
            "6f9619ff-8b86-d011-b42d-00c04fc964fg"
        ));
        assert!(!is_valid_recording_id(
            "6f9619ff-8b86-d011-b42d-00c04fc964ff-"
        ));
    }

    #[test]
    fn peer_info_defaults_to_protocol_1_and_caps_the_app_version() {
        let headers = [
            (PROTOCOL_HEADER, "4"),
            (
                APP_VERSION_HEADER,
                "1.2.3-really-long-version-string-beyond-32",
            ),
        ];
        let lookup = |name: &str| {
            headers
                .iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| *value)
        };
        let peer = PeerInfo::from_headers(lookup);
        assert_eq!(peer.protocol_version, 4);
        assert_eq!(peer.app_version.as_deref().map(str::len), Some(32));
        let bare = PeerInfo::from_headers(|_| None);
        assert_eq!(bare.protocol_version, 1);
        assert_eq!(bare.app_version, None);
        assert!(!bare.is_too_old());
        let negative = PeerInfo::from_headers(|name| (name == PROTOCOL_HEADER).then_some("-1"));
        assert!(
            negative.is_too_old(),
            "Int(\"-1\") < minimumPeer in Wristper"
        );
        let garbage = PeerInfo::from_headers(|name| (name == PROTOCOL_HEADER).then_some("four"));
        assert_eq!(garbage.protocol_version, 1);
    }
}
