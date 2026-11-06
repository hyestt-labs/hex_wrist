//! How a pairing goes, apart from how it looks: the digits typed, the claim
//! the relay answers, and the wait for the watch to finish on its side.

use std::sync::atomic::{AtomicU64, Ordering};
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

/// Numbers each claim sent, across windows, so a reply that comes back to a
/// window that has moved on (or to a newer one) is told from the answer it
/// waits for.
static GENERATION: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug, Default, PartialEq)]
pub(in crate::rune_ui) enum Pair {
    #[default]
    Idle,
    /// Claim `generation` went to the relay.
    Pairing {
        generation: u64,
    },
    /// The relay took claim `generation`; its watch has until
    /// `since + FINISH_WAIT` to ask this Mac for its token.
    Waiting {
        generation: u64,
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

impl Pair {
    pub(in crate::rune_ui) fn busy(&self) -> bool {
        matches!(self, Self::Pairing { .. } | Self::Waiting { .. })
    }

    /// The relay answered claim `generation`. Only the claim this window
    /// waits for counts; any other reply changes nothing.
    pub(super) fn claimed(
        &self,
        generation: u64,
        result: Result<ClaimOk, ClaimError>,
        snapshot: Option<&WatchSnapshot>,
    ) -> Self {
        let Self::Pairing {
            generation: waiting,
        } = self
        else {
            return self.clone();
        };
        if *waiting != generation {
            return self.clone();
        }
        match result {
            Ok(_) => Self::Waiting {
                generation,
                since: Instant::now(),
            }
            .settled(snapshot, Instant::now()),
            Err(error) => Self::Failed(Failure::Claim(error)),
        }
    }

    /// Paired once the service admits a pairing request that spent this
    /// claim's approval, failed once the watch has had its time. A pairing
    /// another claim earned, a watch's or an older window's, is not this one's.
    pub(super) fn settled(&self, snapshot: Option<&WatchSnapshot>, now: Instant) -> Self {
        let Self::Waiting { generation, since } = self else {
            return self.clone();
        };
        let arrived = snapshot.and_then(|snapshot| {
            let admission = snapshot
                .admissions
                .iter()
                .find(|admission| admission.claim_id == *generation)?;
            snapshot
                .devices
                .iter()
                .find(|device| device.id == admission.device_id)
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
    /// code once, with the number of its claim. Nothing is sent while a
    /// pairing is under way.
    pub(super) fn typed(&mut self) -> Option<(String, u64)> {
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
        let generation = GENERATION.fetch_add(1, Ordering::Relaxed) + 1;
        self.pair = Pair::Pairing { generation };
        Some((digits, generation))
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
    use super::super::WatchEvent;
    use super::*;
    use crate::watch::pairing::requests::{Admission, NO_CLAIM};
    use crate::watch::pairing::{DeviceStatus, DeviceSummary};
    use crate::watch::relay::RelayState;

    fn device(id: &str, name: &str) -> DeviceSummary {
        DeviceSummary {
            id: id.into(),
            name: name.into(),
            last_paired_at: "2026-11-01T10:00:00.000000+00:00".into(),
            last_seen: "2026-11-01T10:00:00.000000+00:00".into(),
            last_protocol: 4,
            last_app_version: "0.4.0".into(),
            status: DeviceStatus::Ready,
        }
    }

    /// The claim number of a pairing from before the claim under test; claims
    /// count up from 1 in each process, so a test's own never reaches it.
    const EARLIER: u64 = 10_000;

    fn admission(claim_id: u64, enrolled: &str) -> Admission {
        Admission {
            claim_id,
            device_id: enrolled.into(),
        }
    }

    fn snapshot(devices: Vec<DeviceSummary>, admissions: Vec<Admission>) -> WatchSnapshot {
        WatchSnapshot {
            relay: RelayState::Connected,
            mac_name: "Test Mac".into(),
            replaced: false,
            devices,
            admissions,
        }
    }

    fn ok() -> Result<ClaimOk, ClaimError> {
        Ok(ClaimOk {
            device_id: "relay-id".into(),
            device_name: "Test watch".into(),
        })
    }

    /// Types `text` and returns the code and claim number a send would carry.
    fn typed(view: &mut WatchView, text: &str) -> Option<(String, u64)> {
        view.code = TextEdit::new(text);
        view.typed()
    }

    fn claim(view: &mut WatchView) -> u64 {
        typed(view, "217400").expect("six digits send").1
    }

    /// A claim the relay took: the view waits for its watch.
    fn waiting(snapshot: Option<&WatchSnapshot>) -> (WatchView, u64) {
        let mut view = WatchView::default();
        let generation = claim(&mut view);
        view.apply(
            WatchEvent::Claimed {
                generation,
                result: ok(),
            },
            snapshot,
        );
        (view, generation)
    }

    #[test]
    fn six_digits_send_one_code_and_nothing_else_does() {
        let mut view = WatchView::default();
        assert_eq!(typed(&mut view, "21740"), None);
        assert_eq!(view.pair, Pair::Idle);
        let (code, _) = typed(&mut view, "217400").unwrap();
        assert_eq!(code, "217400");
        assert!(view.pair.busy());
        // The same digits again, while it runs, send nothing more.
        assert_eq!(typed(&mut view, "217400"), None);
        assert_eq!(typed(&mut view, "999999"), None);
    }

    #[test]
    fn every_claim_has_its_own_number() {
        let mut view = WatchView::default();
        let first = claim(&mut view);
        view.pair = Pair::Idle;
        let second = claim(&mut view);
        assert_ne!(first, second);
        assert_ne!(first, 0, "0 belongs to scenes");
        assert_ne!(first, NO_CLAIM, "a window's claim is never the CLI's");
    }

    #[test]
    fn the_field_keeps_only_six_digits() {
        let mut view = WatchView::default();
        assert_eq!(typed(&mut view, "2-1 7a4"), None);
        assert_eq!(view.code.text, "2174");
        let (code, _) = typed(&mut view, "1234567890").unwrap();
        assert_eq!(code, "123456");
        assert_eq!(view.code.text, "123456");
    }

    #[test]
    fn a_refused_code_shows_its_reason_and_the_next_six_digits_try_again() {
        let mut view = WatchView::default();
        let generation = claim(&mut view);
        view.apply(
            WatchEvent::Claimed {
                generation,
                result: Err(ClaimError::CodeNotFound),
            },
            None,
        );
        let (words, tone) = view.pair.status().unwrap();
        assert_eq!(words, claim_error(&ClaimError::CodeNotFound));
        assert_eq!(tone, Tone::Bad);
        assert!(!view.pair.busy());
        assert!(typed(&mut view, "111111").is_some());
    }

    #[test]
    fn the_reply_to_the_current_claim_moves_pairing_on() {
        let mut view = WatchView::default();
        let generation = claim(&mut view);
        let changed = view.apply(
            WatchEvent::Claimed {
                generation,
                result: ok(),
            },
            None,
        );
        assert!(changed);
        assert!(matches!(view.pair, Pair::Waiting { .. }), "{:?}", view.pair);
        assert!(
            view.code.text.is_empty(),
            "the field clears once it is taken"
        );
    }

    #[test]
    fn the_reply_to_an_older_claim_is_dropped() {
        let mut view = WatchView::default();
        let older = claim(&mut view);
        view.apply(
            WatchEvent::Claimed {
                generation: older,
                result: Err(ClaimError::Timeout),
            },
            None,
        );
        // A newer claim is under way when the older one's late reply lands.
        let newer = claim(&mut view);
        assert_ne!(older, newer);
        let changed = view.apply(
            WatchEvent::Claimed {
                generation: older,
                result: ok(),
            },
            None,
        );
        assert!(!changed, "nothing shown changes");
        assert_eq!(view.pair, Pair::Pairing { generation: newer });
        assert_eq!(view.code.text, "217400", "the field is not cleared");
        // A window that has no claim under way ignores a reply too.
        let mut idle = WatchView::default();
        assert!(!idle.apply(
            WatchEvent::Claimed {
                generation: older,
                result: ok(),
            },
            None,
        ));
        assert_eq!(idle.pair, Pair::Idle);
    }

    #[test]
    fn the_pairing_this_claim_earned_settles_it() {
        let before = snapshot(
            vec![device("old", "Old watch")],
            vec![admission(EARLIER, "old")],
        );
        let (mut view, generation) = waiting(Some(&before));
        // The earlier pairing is not this one.
        assert!(!view.settle(Some(&before), Instant::now()));
        let after = snapshot(
            vec![device("old", "Old watch"), device("new", "Series 9")],
            vec![admission(EARLIER, "old"), admission(generation, "new")],
        );
        assert!(view.settle(Some(&after), Instant::now()));
        assert_eq!(view.pair, Pair::Paired("Series 9".into()));
        assert_eq!(view.pair.status().unwrap().1, Tone::Good);
    }

    #[test]
    fn a_reinstalled_watch_settles_under_the_entry_its_token_named() {
        // The service enrolled this claim's watch under the older entry "w";
        // the window finds it by the admission, not by a new id.
        let before = snapshot(vec![device("w", "Series 9")], vec![admission(EARLIER, "w")]);
        let (mut view, generation) = waiting(Some(&before));
        assert!(!view.settle(Some(&before), Instant::now()), "same pairing");
        let again = snapshot(
            vec![device("w", "Series 9")],
            vec![admission(EARLIER, "w"), admission(generation, "w")],
        );
        assert!(view.settle(Some(&again), Instant::now()), "no 120 s wait");
        assert_eq!(view.pair, Pair::Paired("Series 9".into()));
    }

    #[test]
    fn another_claims_pairing_during_the_wait_does_not_settle_this_one() {
        let (mut view, generation) = waiting(None);
        // Another watch, or an older window's claim of this very watch,
        // finishes pairing in the same window of time.
        let other = snapshot(
            vec![device("theirs", "Test watch 2")],
            vec![admission(EARLIER, "theirs")],
        );
        assert!(!view.settle(Some(&other), Instant::now()));
        assert!(matches!(view.pair, Pair::Waiting { .. }), "{:?}", view.pair);
        // This claim's own pairing then does, and the status names it.
        let both = snapshot(
            vec![
                device("theirs", "Test watch 2"),
                device("mine", "Test watch"),
            ],
            vec![admission(EARLIER, "theirs"), admission(generation, "mine")],
        );
        assert!(view.settle(Some(&both), Instant::now()));
        assert_eq!(view.pair, Pair::Paired("Test watch".into()));
    }

    #[test]
    fn a_watch_that_never_asks_times_out_with_its_own_message() {
        let mut view = WatchView {
            pair: Pair::Waiting {
                generation: 1,
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
