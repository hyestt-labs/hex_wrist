//! How a pairing goes, apart from how it looks: the digits typed, the claim
//! the relay answers, and the wait for the watch to finish on its side.

use std::time::{Duration, Instant, SystemTime};

use rune::TextEdit;

use super::{WatchView, claim_error};
use crate::desktop::i18n::{tr, tr_fill};
use crate::watch::Timestamp;
use crate::watch::service::{ClaimError, ClaimOk, WatchSnapshot};

/// How long the watch has, after the relay took the code, to ask this Mac
/// for its token (the Mac's own approval window is 120 s).
const FINISH_WAIT: Duration = Duration::from_secs(120);

/// How good a status is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::rune_ui) enum Tone {
    Good,
    Waiting,
    Bad,
}

/// The paired watches as (id, when each last paired), to tell a new pairing
/// from one that was already there.
type Known = Vec<(String, String)>;

#[derive(Clone, Debug, Default, PartialEq)]
pub(in crate::rune_ui) enum Pair {
    #[default]
    Idle,
    /// The code went to the relay.
    Pairing {
        before: Known,
    },
    /// The relay took the code; the watch has until `since + FINISH_WAIT`.
    Waiting {
        before: Known,
        since: Instant,
    },
    Paired(String),
    Failed(Failure),
}

#[derive(Clone, Debug, PartialEq)]
pub(in crate::rune_ui) enum Failure {
    Claim(ClaimError),
    /// The relay took the code but the watch never asked for its token.
    WatchSilent,
}

pub(super) fn known(snapshot: Option<&WatchSnapshot>) -> Known {
    snapshot
        .map(|snapshot| {
            snapshot
                .devices
                .iter()
                .map(|device| (device.id.clone(), device.last_paired_at.clone()))
                .collect()
        })
        .unwrap_or_default()
}

impl Pair {
    pub(in crate::rune_ui) fn busy(&self) -> bool {
        matches!(self, Self::Pairing { .. } | Self::Waiting { .. })
    }

    /// The relay answered the claim.
    pub(super) fn claimed(
        &self,
        result: Result<ClaimOk, ClaimError>,
        snapshot: Option<&WatchSnapshot>,
    ) -> Self {
        let before = match self {
            Self::Pairing { before } | Self::Waiting { before, .. } => before.clone(),
            _ => known(snapshot),
        };
        match result {
            Ok(_) => {
                let waiting = Self::Waiting {
                    before,
                    since: Instant::now(),
                };
                waiting.settled(snapshot, Instant::now())
            }
            Err(error) => Self::Failed(Failure::Claim(error)),
        }
    }

    /// Paired once a watch that wasn't there before is, failed once the
    /// watch has had its time.
    pub(super) fn settled(&self, snapshot: Option<&WatchSnapshot>, now: Instant) -> Self {
        let Self::Waiting { before, since } = self else {
            return self.clone();
        };
        let arrived = snapshot.and_then(|snapshot| {
            snapshot.devices.iter().find(|device| {
                !before.contains(&(device.id.clone(), device.last_paired_at.clone()))
            })
        });
        if let Some(device) = arrived {
            Self::Paired(device.name.clone())
        } else if now.saturating_duration_since(*since) >= FINISH_WAIT {
            Self::Failed(Failure::WatchSilent)
        } else {
            self.clone()
        }
    }

    /// The line under the code field and how good it is.
    pub(in crate::rune_ui) fn status(&self) -> Option<(String, Tone)> {
        Some(match self {
            Self::Idle => return None,
            Self::Pairing { .. } => (tr("Pairing…").to_owned(), Tone::Waiting),
            Self::Waiting { .. } => (tr("Finish on your watch…").to_owned(), Tone::Waiting),
            Self::Paired(name) => (tr_fill("Paired with {}", name), Tone::Good),
            Self::Failed(Failure::Claim(error)) => (claim_error(error), Tone::Bad),
            Self::Failed(Failure::WatchSilent) => (
                tr("The watch didn't finish pairing. Update Wristper on the watch and try again")
                    .to_owned(),
                Tone::Bad,
            ),
        })
    }
}

impl WatchView {
    /// Keeps only digits (six at most) in the field; the sixth sends the
    /// code once. Nothing is sent while a pairing is under way.
    pub(super) fn typed(&mut self, snapshot: Option<&WatchSnapshot>) -> Option<String> {
        let digits: String = self
            .code
            .text
            .chars()
            .filter(char::is_ascii_digit)
            .take(6)
            .collect();
        if digits != self.code.text {
            self.code = TextEdit::new(digits.clone());
        }
        if digits.len() < 6 || self.pair.busy() {
            return None;
        }
        self.removing = None;
        self.pair = Pair::Pairing {
            before: known(snapshot),
        };
        Some(digits)
    }

    /// Moves a pairing on with the newest snapshot and the clock.
    pub(super) fn settle(&mut self, snapshot: Option<&WatchSnapshot>, now: Instant) -> bool {
        let next = self.pair.settled(snapshot, now);
        let changed = next != self.pair;
        self.pair = next;
        changed
    }
}

/// When a watch was last heard from, as an age; the text if it isn't a time.
pub(super) fn last_seen(stamp: &str, now: SystemTime) -> String {
    let Some(then) = Timestamp::parse(stamp) else {
        return stamp.to_owned();
    };
    let seconds = now.duration_since(then).unwrap_or_default().as_secs();
    let (template, value) = match seconds {
        0..=59 => ("{}s ago", seconds),
        60..=3_599 => ("{}m ago", seconds / 60),
        3_600..=86_399 => ("{}h ago", seconds / 3_600),
        _ => ("{}d ago", seconds / 86_400),
    };
    tr_fill(template, &value.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::watch::e2ee::SharedKey;
    use crate::watch::pairing::{DeviceStatus, DeviceSummary, MemorySecretStore, PairedDevices};
    use crate::watch::relay::RelayState;

    fn device(id: &str, name: &str, paired_at: &str) -> DeviceSummary {
        DeviceSummary {
            id: id.into(),
            name: name.into(),
            last_paired_at: paired_at.into(),
            last_seen: paired_at.into(),
            last_protocol: 4,
            last_app_version: "0.4.0".into(),
            status: DeviceStatus::Ready,
        }
    }

    fn snapshot(devices: Vec<DeviceSummary>) -> WatchSnapshot {
        WatchSnapshot {
            relay: RelayState::Connected,
            mac_name: "Glen's Mac".into(),
            replaced: false,
            devices,
        }
    }

    fn typed(view: &mut WatchView, text: &str, snapshot: Option<&WatchSnapshot>) -> Option<String> {
        view.code = TextEdit::new(text);
        view.typed(snapshot)
    }

    #[test]
    fn six_digits_send_one_code_and_nothing_else_does() {
        let mut view = WatchView::default();
        assert_eq!(typed(&mut view, "21740", None), None);
        assert_eq!(view.pair, Pair::Idle);
        assert_eq!(typed(&mut view, "217400", None), Some("217400".into()));
        assert!(view.pair.busy());
        // The same digits again, while it runs, send nothing more.
        assert_eq!(typed(&mut view, "217400", None), None);
        assert_eq!(typed(&mut view, "999999", None), None);
    }

    #[test]
    fn the_field_keeps_only_six_digits() {
        let mut view = WatchView::default();
        assert_eq!(typed(&mut view, "2-1 7a4", None), None);
        assert_eq!(view.code.text, "2174");
        assert_eq!(typed(&mut view, "1234567890", None), Some("123456".into()));
        assert_eq!(view.code.text, "123456");
    }

    #[test]
    fn a_refused_code_shows_its_reason_and_the_next_six_digits_try_again() {
        let mut view = WatchView::default();
        typed(&mut view, "217400", None);
        view.pair = view.pair.claimed(Err(ClaimError::CodeNotFound), None);
        let (words, tone) = view.pair.status().unwrap();
        assert_eq!(words, claim_error(&ClaimError::CodeNotFound));
        assert_eq!(tone, Tone::Bad);
        assert!(!view.pair.busy());
        assert_eq!(typed(&mut view, "111111", None), Some("111111".into()));
    }

    #[test]
    fn a_taken_code_waits_for_a_watch_that_was_not_paired_before() {
        let before = snapshot(vec![device(
            "old",
            "Old watch",
            "2026-11-01T10:00:00.000000+00:00",
        )]);
        let mut view = WatchView::default();
        typed(&mut view, "217400", Some(&before));
        let ok = ClaimOk {
            device_id: "relay-id".into(),
            device_name: "Series 9".into(),
        };
        view.pair = view.pair.claimed(Ok(ok), Some(&before));
        assert!(matches!(view.pair, Pair::Waiting { .. }), "{:?}", view.pair);
        // The old watch alone is not the new pairing.
        assert!(!view.settle(Some(&before), Instant::now()));
        let after = snapshot(vec![
            device("old", "Old watch", "2026-11-01T10:00:00.000000+00:00"),
            device("new", "Series 9", "2026-11-05T09:00:00.000000+00:00"),
        ]);
        assert!(view.settle(Some(&after), Instant::now()));
        assert_eq!(view.pair, Pair::Paired("Series 9".into()));
        assert_eq!(view.pair.status().unwrap().1, Tone::Good);
    }

    #[test]
    fn pairing_a_watch_again_settles_as_soon_as_it_has() {
        // The list a real `PairedDevices` reports: a watch paired again
        // (with `previous_token`, the same entry) keeps its first
        // `paired_at`.
        let mut nonce = [0u8; 8];
        getrandom::fill(&mut nonce).unwrap();
        let dir = std::env::temp_dir().join(format!(
            "hex-watch-pane-repair-{}-{:016x}",
            std::process::id(),
            u64::from_le_bytes(nonce)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let mut paired = PairedDevices::load(
            &dir.join(crate::watch::pairing::FILE_NAME),
            Box::new(MemorySecretStore::default()),
        );
        let key = SharedKey::from_base64("w9fFiEsXKqe7xFZ8w+SKs5nTn9bYRhMYXEOB17J8uO0=").unwrap();
        let then = Timestamp::parse("2026-11-01T10:00:00.000000+00:00").unwrap();
        paired.enroll("w", "Series 9", &key, then).unwrap();
        let before = snapshot(paired.summary());
        let mut view = WatchView::default();
        typed(&mut view, "217400", Some(&before));
        let ok = ClaimOk {
            device_id: "w-reinstalled".into(),
            device_name: "Series 9".into(),
        };
        view.pair = view.pair.claimed(Ok(ok), Some(&before));
        assert!(!view.settle(Some(&before), Instant::now()), "same pairing");
        paired
            .enroll("w", "Series 9", &key, then + Duration::from_secs(60))
            .unwrap();
        let again = snapshot(paired.summary());
        assert!(view.settle(Some(&again), Instant::now()), "no 120 s wait");
        assert_eq!(view.pair, Pair::Paired("Series 9".into()));
    }

    #[test]
    fn a_watch_that_never_asks_times_out_with_its_own_message() {
        let mut view = WatchView {
            pair: Pair::Waiting {
                before: Vec::new(),
                since: Instant::now(),
            },
            ..Default::default()
        };
        let soon = Instant::now() + FINISH_WAIT - Duration::from_secs(1);
        assert!(!view.settle(None, soon));
        assert!(view.settle(None, Instant::now() + FINISH_WAIT));
        assert_eq!(view.pair, Pair::Failed(Failure::WatchSilent));
        let (words, tone) = view.pair.status().unwrap();
        assert!(words.contains("Update Wristper"), "{words}");
        assert_eq!(tone, Tone::Bad);
    }

    #[test]
    fn last_seen_counts_from_the_clock_it_is_given() {
        let now = Timestamp::parse("2026-11-05T12:00:00.000000+00:00").unwrap();
        let ago =
            |seconds: u64| last_seen(&Timestamp::string(now - Duration::from_secs(seconds)), now);
        assert_eq!(ago(5), "5s ago");
        assert_eq!(ago(180), "3m ago");
        assert_eq!(ago(7_300), "2h ago");
        assert_eq!(ago(200_000), "2d ago");
        assert_eq!(last_seen("not a time", now), "not a time");
    }
}
