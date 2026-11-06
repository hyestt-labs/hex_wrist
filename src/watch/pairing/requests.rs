//! The claim approval and the pairing requests it admits
//! (`PairingRequests` in Wristper): typing the code approves one watch for
//! 120 s; its request spends the approval, derives the key, enrolls the
//! device, and leaves a result readable for 300 s.

use std::collections::BTreeMap;
use std::time::{Duration, Instant, SystemTime};

use super::super::e2ee::{self, Agreement, SharedKey};
use super::{
    APPROVAL_WINDOW, ENCRYPTION_FAILED, KEYCHAIN_UNAVAILABLE, PairedDevices, REQUEST_LIFETIME,
};

struct Approval {
    device_id: String,
    until: Instant,
}

enum Outcome {
    Issued {
        token: String,
        mac_public_key: String,
        shared_key: SharedKey,
    },
    Failed(&'static str),
}

struct Request {
    device_id: String,
    created: Instant,
    outcome: Outcome,
    counted: bool,
}

/// How many admissions the service remembers for the window.
const ADMISSIONS_KEPT: usize = 8;

/// Which `/pairing/request` spent which claim's approval: the id the relay
/// gave the watch that claimed, and the entry the request enrolled (an
/// older one when `previous_token` named it). `serial` rises with each, so
/// a window that waits for one tells it from any it saw before.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Admission {
    pub(crate) serial: u64,
    pub(crate) claimed_device_id: String,
    pub(crate) device_id: String,
}

/// A `POST /pairing/request` after its body was read.
#[derive(Clone, Copy, Debug)]
pub(crate) struct PairingRequest<'a> {
    /// The id the request says; compared with the approval.
    pub(crate) claimed_device_id: &'a str,
    /// The entry to write: an older one when `previous_token` named it
    /// (a reinstalled watch gets a new id but keeps its token).
    pub(crate) device_id: &'a str,
    pub(crate) name: &'a str,
    /// X9.63 point, standard base64.
    pub(crate) watch_public_key: &'a str,
}

/// What `GET /pairing/result` answers. The token only ever leaves sealed:
/// pairing without a key (Wristper's 4-digit codes) is not supported.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum PairingResult {
    Unknown,
    /// Only a watch holding the same key can open it.
    Sealed {
        sealed: String,
        mac_public_key: String,
    },
}

/// The one open approval and the recent pairing requests.
pub(crate) struct PairingRequests {
    items: BTreeMap<String, Request>,
    order: Vec<String>,
    approval: Option<Approval>,
    finished: u32,
    admissions: Vec<Admission>,
    serial: u64,
}

impl Default for PairingRequests {
    fn default() -> Self {
        Self::new()
    }
}

impl PairingRequests {
    pub(crate) fn new() -> Self {
        Self {
            items: BTreeMap::new(),
            order: Vec::new(),
            approval: None,
            finished: 0,
            admissions: Vec::new(),
            serial: 0,
        }
    }

    /// The newest admissions, oldest first.
    pub(crate) fn admissions(&self) -> &[Admission] {
        &self.admissions
    }

    /// Called on `claim_ok`. One watch at a time; a later claim replaces
    /// the earlier one.
    pub(crate) fn approve(&mut self, device_id: &str, now: Instant) {
        self.approval = Some(Approval {
            device_id: device_id.to_owned(),
            until: now + APPROVAL_WINDOW,
        });
        self.clear_failures();
    }

    pub(crate) fn clear_failures(&mut self) {
        let failed: Vec<String> = self
            .items
            .iter()
            .filter(|(_, item)| matches!(item.outcome, Outcome::Failed(_)))
            .map(|(key, _)| key.clone())
            .collect();
        for key in failed {
            self.remove(&key);
        }
    }

    pub(crate) fn close_window(&mut self) {
        self.approval = None;
    }

    pub(crate) fn window_remaining(&self, now: Instant) -> Duration {
        self.approval
            .as_ref()
            .map(|approval| approval.until.saturating_duration_since(now))
            .unwrap_or(Duration::ZERO)
    }

    fn prune(&mut self, now: Instant) {
        let expired: Vec<String> = self
            .items
            .iter()
            .filter(|(_, item)| now.saturating_duration_since(item.created) >= REQUEST_LIFETIME)
            .map(|(key, _)| key.clone())
            .collect();
        for key in expired {
            self.remove(&key);
        }
    }

    fn remove(&mut self, key: &str) {
        self.items.remove(key);
        self.order.retain(|item| item != key);
    }

    /// Spends the approval and issues a token. `None` when this is not the
    /// approved watch, leaving the approval for the real one. A failed key
    /// agreement or store still consumes the approval and records why.
    pub(crate) fn admit(
        &mut self,
        request: &PairingRequest<'_>,
        paired: &mut PairedDevices,
        now: Instant,
        wall: SystemTime,
    ) -> Option<String> {
        let PairingRequest {
            claimed_device_id,
            device_id,
            name,
            watch_public_key,
        } = *request;
        self.prune(now);
        let approval = self.approval.as_ref()?;
        if approval.until <= now || approval.device_id != claimed_device_id {
            return None;
        }
        self.approval = None;
        let stale: Vec<String> = self
            .items
            .iter()
            .filter(|(_, item)| item.device_id == device_id)
            .map(|(key, _)| key.clone())
            .collect();
        for key in stale {
            self.remove(&key);
        }
        let outcome = match e2ee::agree(watch_public_key) {
            Some(Agreement {
                key,
                public_key_base64,
            }) => match paired.enroll(device_id, name, &key, wall) {
                Some(token) => Outcome::Issued {
                    token,
                    mac_public_key: public_key_base64,
                    shared_key: key,
                },
                None => Outcome::Failed(KEYCHAIN_UNAVAILABLE),
            },
            None => Outcome::Failed(ENCRYPTION_FAILED),
        };
        let id = new_request_id()?;
        if matches!(outcome, Outcome::Issued { .. }) {
            self.serial += 1;
            self.admissions.push(Admission {
                serial: self.serial,
                claimed_device_id: claimed_device_id.to_owned(),
                device_id: device_id.to_owned(),
            });
            if self.admissions.len() > ADMISSIONS_KEPT {
                self.admissions.remove(0);
            }
        }
        self.items.insert(
            id.clone(),
            Request {
                device_id: device_id.to_owned(),
                created: now,
                outcome,
                counted: false,
            },
        );
        self.order.push(id.clone());
        Some(id)
    }

    /// Why `request_id` failed; `None` when it did not (or is unknown).
    pub(crate) fn failure(&mut self, request_id: &str, now: Instant) -> Option<&'static str> {
        self.prune(now);
        match self.items.get(request_id)?.outcome {
            Outcome::Failed(reason) => Some(reason),
            Outcome::Issued { .. } => None,
        }
    }

    /// The newest failure, for the Mac's pairing page.
    pub(crate) fn latest_failure(&mut self, now: Instant) -> Option<&'static str> {
        self.prune(now);
        self.order
            .iter()
            .rev()
            .find_map(|key| match self.items.get(key)?.outcome {
                Outcome::Failed(reason) => Some(reason),
                Outcome::Issued { .. } => None,
            })
    }

    /// Readable until the request expires. The sealed form uses a fresh
    /// nonce each read; the token inside is the same.
    pub(crate) fn result(&mut self, request_id: &str, now: Instant) -> PairingResult {
        self.prune(now);
        let Some(item) = self.items.get_mut(request_id) else {
            return PairingResult::Unknown;
        };
        let Outcome::Issued {
            token,
            mac_public_key,
            shared_key,
        } = &item.outcome
        else {
            return PairingResult::Unknown;
        };
        if !item.counted {
            item.counted = true;
            self.finished += 1;
        }
        match e2ee::random_nonce() {
            Some(nonce) => PairingResult::Sealed {
                sealed: e2ee::seal_pairing_result(token, shared_key, request_id, &nonce),
                mac_public_key: mac_public_key.clone(),
            },
            None => PairingResult::Unknown,
        }
    }

    /// The watch used its new token: its result need not be kept.
    pub(crate) fn completed(&mut self, device_id: &str) {
        let done: Vec<(String, bool)> = self
            .items
            .iter()
            .filter(|(_, item)| {
                item.device_id == device_id && matches!(item.outcome, Outcome::Issued { .. })
            })
            .map(|(key, item)| (key.clone(), item.counted))
            .collect();
        for (key, counted) in done {
            self.remove(&key);
            if !counted {
                self.finished += 1;
            }
        }
    }

    /// How many watches finished pairing since launch; the pairing page
    /// compares it with the count before the claim.
    pub(crate) fn completed_count(&self) -> u32 {
        self.finished
    }
}

/// 128 random bits as 32 lower-case hex digits, as Wristper's request ids.
fn new_request_id() -> Option<String> {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).ok()?;
    Some(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}
