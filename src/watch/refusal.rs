//! Every error body the watch reads, as one closed set. The watch keys its
//! behavior on the `error` string (and, for 402 and 426, on the status), so
//! each variant owns its status and its exact wire text in one place.

use serde_json::{Value, json};

use super::PROTOCOL_VERSION;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Refusal {
    /// 403: the token does not match.
    Forbidden,
    /// 403: the watch demands encryption and sent plaintext.
    E2eeRequired,
    /// 403: another watch owns this recording.
    NotOwner,
    /// 403: not the approved watch, or the approval expired.
    PairingClosed,
    /// 402: the free daily limit is used up.
    DailyLimit {
        resets_at: String,
    },
    /// 426: the watch speaks a protocol older than the Mac serves.
    WatchOutdated {
        min_protocol: u32,
        mac_protocol: u32,
        mac_version: String,
    },
    InvalidE2ee,
    InvalidId,
    InvalidMetadata,
    InvalidRoute,
    InvalidSession,
    InvalidJson,
    EmptyBody,
    BodyOverflow,
    IncompleteBody,
    NotFound,
    UnknownId,
    UnknownRequest,
    /// 409: the nonce was seen before.
    Replayed,
    /// 409: an action arrived after its freshness window.
    ConfirmStale,
    TooLarge,
    PasteFailed,
    /// 500: transcription or paste failed midway. The watch counts this as
    /// delivered, so the body carries the recording's `failed` state.
    Failed {
        error: String,
    },
    E2eeFailed,
    /// 503: the Keychain could not be read. Old watches read a 403 as
    /// "unpaired", so this must stay a 503.
    SecretsUnavailable,
}

impl Refusal {
    /// The `426` for a watch whose protocol is below `min_protocol`:
    /// `MINIMUM_PEER_PROTOCOL` for a device call, `PROTOCOL_VERSION` for a
    /// pairing request without a public key (`ReceiverService.swift:540`).
    pub(crate) fn watch_outdated(min_protocol: u32, mac_version: &str) -> Self {
        Self::WatchOutdated {
            min_protocol,
            mac_protocol: PROTOCOL_VERSION,
            mac_version: mac_version.to_owned(),
        }
    }

    pub(crate) fn status(&self) -> u16 {
        match self {
            Self::DailyLimit { .. } => 402,
            Self::Forbidden | Self::E2eeRequired | Self::NotOwner | Self::PairingClosed => 403,
            Self::InvalidE2ee
            | Self::InvalidId
            | Self::InvalidMetadata
            | Self::InvalidRoute
            | Self::InvalidSession
            | Self::InvalidJson
            | Self::EmptyBody
            | Self::BodyOverflow
            | Self::IncompleteBody => 400,
            Self::NotFound | Self::UnknownId | Self::UnknownRequest => 404,
            Self::Replayed | Self::ConfirmStale => 409,
            Self::TooLarge => 413,
            Self::WatchOutdated { .. } => 426,
            Self::PasteFailed | Self::Failed { .. } | Self::E2eeFailed => 500,
            Self::SecretsUnavailable => 503,
        }
    }

    pub(crate) fn body(&self) -> Value {
        match self {
            Self::DailyLimit { resets_at } => json!({
                "ok": false,
                "error": "license_required",
                "reason": "daily_limit_reached",
                "resets_at": resets_at,
            }),
            Self::WatchOutdated {
                min_protocol,
                mac_protocol,
                mac_version,
            } => json!({
                "ok": false,
                "error": "watch_outdated",
                "min_protocol": min_protocol,
                "mac_protocol": mac_protocol,
                "mac_version": mac_version,
            }),
            Self::Failed { error } => json!({
                "ok": false,
                "state": "failed",
                "error": error,
            }),
            other => json!({ "ok": false, "error": other.error_name() }),
        }
    }

    /// The `error` string of the variants whose body is just `{ok,error}`.
    /// The three with richer bodies never reach here.
    fn error_name(&self) -> &'static str {
        match self {
            Self::Forbidden => "forbidden",
            Self::E2eeRequired => "e2ee-required",
            Self::NotOwner => "not-owner",
            Self::PairingClosed => "pairing-closed",
            Self::InvalidE2ee => "invalid-e2ee",
            Self::InvalidId => "invalid-id",
            Self::InvalidMetadata => "invalid-metadata",
            Self::InvalidRoute => "invalid-route",
            Self::InvalidSession => "invalid-session",
            Self::InvalidJson => "invalid-json",
            Self::EmptyBody => "empty-body",
            Self::BodyOverflow => "body-overflow",
            Self::IncompleteBody => "incomplete-body",
            Self::NotFound => "not-found",
            Self::UnknownId => "unknown-id",
            Self::UnknownRequest => "unknown-request",
            Self::Replayed => "replayed",
            Self::ConfirmStale => "confirm-stale",
            Self::TooLarge => "too-large",
            Self::PasteFailed => "paste-failed",
            Self::E2eeFailed => "e2ee-failed",
            Self::SecretsUnavailable => "secrets-unavailable",
            Self::DailyLimit { .. } => "license_required",
            Self::WatchOutdated { .. } => "watch_outdated",
            Self::Failed { .. } => "failed",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::watch::MINIMUM_PEER_PROTOCOL;

    #[test]
    fn daily_limit_matches_the_license_required_body() {
        let refusal = Refusal::DailyLimit {
            resets_at: "2026-11-05T00:00:00Z".into(),
        };
        assert_eq!(refusal.status(), 402);
        assert_eq!(
            refusal.body(),
            json!({
                "ok": false,
                "error": "license_required",
                "reason": "daily_limit_reached",
                "resets_at": "2026-11-05T00:00:00Z",
            })
        );
    }

    #[test]
    fn watch_outdated_names_both_protocols_and_the_mac_version() {
        let keyless_pairing = Refusal::watch_outdated(PROTOCOL_VERSION, "2.4.0");
        assert_eq!(keyless_pairing.body()["min_protocol"], 4);
        let refusal = Refusal::watch_outdated(MINIMUM_PEER_PROTOCOL, "2.4.0");
        assert_eq!(refusal.status(), 426);
        assert_eq!(
            refusal.body(),
            json!({
                "ok": false,
                "error": "watch_outdated",
                "min_protocol": MINIMUM_PEER_PROTOCOL,
                "mac_protocol": PROTOCOL_VERSION,
                "mac_version": "2.4.0",
            })
        );
    }

    #[test]
    fn failed_carries_the_failed_state_so_the_watch_counts_it_delivered() {
        let refusal = Refusal::Failed {
            error: "transcribe: boom".into(),
        };
        assert_eq!(refusal.status(), 500);
        assert_eq!(
            refusal.body(),
            json!({ "ok": false, "state": "failed", "error": "transcribe: boom" })
        );
    }

    #[test]
    fn every_plain_refusal_has_its_spec_status_and_error_string() {
        let table = [
            (Refusal::Forbidden, 403, "forbidden"),
            (Refusal::E2eeRequired, 403, "e2ee-required"),
            (Refusal::NotOwner, 403, "not-owner"),
            (Refusal::PairingClosed, 403, "pairing-closed"),
            (Refusal::InvalidE2ee, 400, "invalid-e2ee"),
            (Refusal::InvalidId, 400, "invalid-id"),
            (Refusal::InvalidMetadata, 400, "invalid-metadata"),
            (Refusal::InvalidRoute, 400, "invalid-route"),
            (Refusal::InvalidSession, 400, "invalid-session"),
            (Refusal::InvalidJson, 400, "invalid-json"),
            (Refusal::EmptyBody, 400, "empty-body"),
            (Refusal::BodyOverflow, 400, "body-overflow"),
            (Refusal::IncompleteBody, 400, "incomplete-body"),
            (Refusal::NotFound, 404, "not-found"),
            (Refusal::UnknownId, 404, "unknown-id"),
            (Refusal::UnknownRequest, 404, "unknown-request"),
            (Refusal::Replayed, 409, "replayed"),
            (Refusal::ConfirmStale, 409, "confirm-stale"),
            (Refusal::TooLarge, 413, "too-large"),
            (Refusal::PasteFailed, 500, "paste-failed"),
            (Refusal::E2eeFailed, 500, "e2ee-failed"),
            (Refusal::SecretsUnavailable, 503, "secrets-unavailable"),
        ];
        for (refusal, status, error) in table {
            assert_eq!(refusal.status(), status, "{error}");
            assert_eq!(
                refusal.body(),
                json!({ "ok": false, "error": error }),
                "{error}"
            );
        }
    }
}
