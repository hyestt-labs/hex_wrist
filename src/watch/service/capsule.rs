//! Drives the status capsule from the `watch` thread: each change to a
//! recording is announced as it happens, and the hide and the read-back
//! grace are deadlines of the thread's own loop, so nothing polls.

use std::time::Instant;

use super::state::{Core, Now};
use crate::watch::recording::State;
use crate::watch::status::{self, READ_BACK_GRACE};

/// What the capsule is waiting on.
#[derive(Default)]
pub(crate) struct Timers {
    /// When the capsule on screen goes away.
    hide_at: Option<Instant>,
    /// A paste not yet read back: shown as unconfirmed at this moment if
    /// the read-back has not settled it.
    grace: Option<(String, Instant)>,
}

impl Timers {
    pub(crate) fn next_deadline(&self) -> Option<Instant> {
        [self.hide_at, self.grace.as_ref().map(|(_, at)| *at)]
            .into_iter()
            .flatten()
            .min()
    }
}

impl Core {
    /// `id` changed state: show the capsule it earns, unless it is a sent
    /// paste still inside the read-back grace.
    pub(super) fn announce(&mut self, id: &str, now: Now) {
        self.capsule.grace = None;
        let Some(live) = self.records.get(id) else {
            return;
        };
        let record = &live.record;
        if record.state == State::Unverified && !record.paste.entered {
            self.capsule.grace = Some((id.to_owned(), now.at + READ_BACK_GRACE));
            return;
        }
        self.show(id, now);
    }

    fn show(&mut self, id: &str, now: Now) {
        let Some(live) = self.records.get(id) else {
            return;
        };
        let Some(display) = status::display(&live.record, self.desktop.trusted()) else {
            return;
        };
        (self.status)(Some(display.capsule));
        self.capsule.hide_at = Some(now.at + display.hide_after);
    }

    pub(super) fn tick_capsule(&mut self, now: Now) {
        if self.capsule.hide_at.is_some_and(|at| at <= now.at) {
            self.capsule.hide_at = None;
            (self.status)(None);
        }
        if let Some((id, at)) = self.capsule.grace.clone()
            && at <= now.at
        {
            self.capsule.grace = None;
            self.show(&id, now);
        }
    }

    pub(super) fn hide_capsule(&mut self) {
        self.capsule = Timers::default();
        (self.status)(None);
    }
}
