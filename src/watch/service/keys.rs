//! `/enter` and `/undo`: the watch presses Return or ⌘Z in the field its
//! recording pasted into (`enter` and `undo` in Swift's `ReceiverService`).
//! The policy (windows, states) is `recording::check_enter` and
//! `check_undo`; the key itself goes through the `watch-ax` thread, which
//! compares the focus with the recording's field first.

use std::time::Duration;

use serde_json::Value;

use super::pending::{Reply, Waiting};
use super::state::{Core, Now};
use crate::watch::Timestamp;
use crate::watch::desktop::{Key, KeyOutcome};
use crate::watch::recording::{
    Caller, ENTER_WAIT_SECONDS, EnterRefusal, EnterReply, State, UndoRefusal, UndoReply,
    check_enter, check_undo, refuse_non_owner,
};
use crate::watch::refusal::Refusal;
use crate::watch::service::Message;
use crate::watch::{is_valid_recording_id, service::routes::json_body};

/// How long a key press may take on the `watch-ax` thread, the wait for
/// the keyboard lock (a paste's read-back, up to 1 s) included. The
/// watch's own timeout is 20 s.
pub(crate) const KEY_WAIT: Duration = Duration::from_secs(5);

impl Core {
    /// `POST /enter` or `/undo` with `{"id"}`.
    pub(super) fn key_request(
        &mut self,
        key: Key,
        body: &[u8],
        caller: (&str, bool),
        reply: Reply,
        now: Now,
    ) {
        let (device, inside) = caller;
        let id = match self.key_target(body, device, now) {
            Ok(id) => id,
            Err(refusal) => return self.refuse(reply, refusal),
        };
        // Enter waits for a recording still being processed: the watch sends
        // it with the audio, and the text must land first.
        let in_progress = self
            .records
            .get(&id)
            .is_some_and(|live| live.record.state.is_in_progress());
        if key == Key::Enter && in_progress {
            self.waiting.push(Waiting::Settle {
                id,
                key,
                device: device.to_owned(),
                inside,
                reply,
                deadline: now.at + Duration::from_secs(ENTER_WAIT_SECONDS),
            });
            return;
        }
        self.press_key(key, &id, (device, inside), reply, now);
    }

    fn key_target(&self, body: &[u8], device: &str, now: Now) -> Result<String, Refusal> {
        let payload = json_body(body).ok_or(Refusal::InvalidJson)?;
        let id = payload
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        if !is_valid_recording_id(&id) {
            return Err(Refusal::InvalidId);
        }
        refuse_non_owner(self.owner_of(&id, now), Caller::Device(device))?;
        Ok(id)
    }

    /// The recording settled (or the wait ran out): check the policy and
    /// hand the key to the `watch-ax` thread.
    pub(super) fn press_key(
        &mut self,
        key: Key,
        id: &str,
        caller: (&str, bool),
        reply: Reply,
        now: Now,
    ) {
        // One key at a time per recording: a second request waits for the
        // first one's result and is judged against it (a second Enter is a
        // duplicate, an Undo after an Enter is refused).
        if self
            .records
            .get(id)
            .is_some_and(|live| live.in_flight.is_some())
        {
            self.waiting.push(Waiting::Settle {
                id: id.to_owned(),
                key,
                device: caller.0.to_owned(),
                inside: caller.1,
                reply,
                deadline: now.at + KEY_WAIT,
            });
            return;
        }
        let live = self.records.get(id);
        let record = live.map(|live| &live.record);
        let verdict = match key {
            Key::Enter => check_enter(record, now.wall)
                .err()
                .map(|reply| match reply {
                    EnterReply::AlreadyEntered => (200, reply.json()),
                    refused => (refused.status(), refused.json()),
                }),
            Key::Undo => check_undo(record, now.wall)
                .err()
                .map(|refusal| refused_undo(UndoReply::Refused(refusal))),
        };
        if let Some((status, body)) = verdict {
            return self.answer(reply, status, body);
        }
        let Some(field) = live.and_then(|live| live.field.clone()) else {
            let (status, body) = match key {
                Key::Enter => {
                    let refused = EnterReply::Refused(EnterRefusal::TargetChanged);
                    (refused.status(), refused.json())
                }
                Key::Undo => refused_undo(UndoReply::Refused(UndoRefusal::TargetChanged)),
            };
            return self.answer(reply, status, body);
        };
        let valid_until = now.at + KEY_WAIT;
        let op = self.next_op;
        self.next_op += 1;
        let ticket = crate::watch::desktop::Ticket::new(op);
        if let Some(live) = self.records.get_mut(id) {
            live.in_flight = Some(ticket.clone());
        }
        self.waiting.push(Waiting::Key {
            id: id.to_owned(),
            key,
            op,
            reply,
            deadline: valid_until,
        });
        let inbox = self.inbox.clone();
        let id = id.to_owned();
        self.desktop
            .press(key, field, valid_until, ticket, move |outcome| {
                let _ = inbox.send(Message::Keyed {
                    id,
                    key,
                    op,
                    outcome,
                });
            });
    }

    /// The `watch-ax` thread pressed the key, or says why it did not.
    /// Answers are matched by operation number: one for a request already
    /// given up on finds no waiting reply and frees nothing.
    pub(super) fn on_keyed(&mut self, id: &str, key: Key, op: u64, outcome: KeyOutcome, now: Now) {
        let position = self
            .waiting
            .iter()
            .position(|waiting| matches!(waiting, Waiting::Key { op: held, .. } if *held == op));
        let Some(position) = position else {
            tracing::info!(id, "a key answer arrived after its request was answered");
            return;
        };
        let Waiting::Key { reply, .. } = self.waiting.remove(position) else {
            return;
        };
        self.end_key(id, op, false, now);
        let (status, body) = self.keyed(id, key, outcome, now);
        self.answer(reply, status, body);
        self.release_settled(id, now);
    }

    /// Operation `op`'s claim on `id`'s keyboard is over; another
    /// operation's claim is left alone. `cancel` also stops the `watch-ax`
    /// thread from pressing it late.
    pub(super) fn end_key(&mut self, id: &str, op: u64, cancel: bool, now: Now) {
        if let Some(live) = self.records.get_mut(id)
            && live
                .in_flight
                .as_ref()
                .is_some_and(|ticket| ticket.op() == op)
            && let Some(ticket) = live.in_flight.take()
            && cancel
        {
            ticket.cancel();
        }
        if cancel {
            self.release_settled(id, now);
        }
    }

    /// The reply at the deadline when the `watch-ax` thread has not answered.
    /// If its key already went out (it claimed the ticket) the key counts as
    /// pressed even though the result is unconfirmed: the record says so, so
    /// no retry presses it again. Only a key that never went out is
    /// cancelled and reads as failed.
    pub(super) fn key_timed_out(&mut self, id: &str, key: Key, op: u64, now: Now) -> (u16, Value) {
        let sent = self
            .records
            .get(id)
            .and_then(|live| live.in_flight.as_ref())
            .filter(|ticket| ticket.op() == op)
            .is_some_and(|ticket| !ticket.cancel());
        if sent {
            tracing::warn!(
                id,
                "a key was sent but its answer is late; counting it as pressed"
            );
            return self.keyed(id, key, KeyOutcome::Pressed, now);
        }
        match key {
            Key::Enter => {
                let failed = EnterReply::Refused(EnterRefusal::Failed);
                (failed.status(), failed.json())
            }
            Key::Undo => refused_undo(UndoReply::Refused(UndoRefusal::Failed)),
        }
    }

    fn keyed(&mut self, id: &str, key: Key, outcome: KeyOutcome, now: Now) -> (u16, Value) {
        match (key, outcome) {
            (Key::Enter, KeyOutcome::Pressed) => {
                if let Some(live) = self.records.get_mut(id) {
                    live.record.paste.entered = true;
                    live.record.updated_at = Timestamp::string(now.wall);
                    live.touched = now.at;
                }
                self.remember_handled(id, now);
                self.announce(id, now);
                (200, EnterReply::Entered.json())
            }
            (Key::Undo, KeyOutcome::Pressed) => {
                if let Some(live) = self.records.get_mut(id) {
                    live.record.state = State::Undone;
                    live.record.updated_at = Timestamp::string(now.wall);
                    live.touched = now.at;
                }
                self.remember_handled(id, now);
                self.announce(id, now);
                (200, UndoReply::Undone.json())
            }
            (Key::Enter, KeyOutcome::TargetChanged) => enter_refused(EnterRefusal::TargetChanged),
            (Key::Enter, KeyOutcome::Sensitive) => enter_refused(EnterRefusal::Sensitive),
            // A password field reads as a changed target for Undo.
            (Key::Undo, KeyOutcome::TargetChanged | KeyOutcome::Sensitive) => {
                refused_undo(UndoReply::Refused(UndoRefusal::TargetChanged))
            }
            (Key::Enter, KeyOutcome::Failed(error)) => {
                tracing::warn!(id, error, "Return could not be pressed");
                enter_refused(EnterRefusal::Failed)
            }
            (Key::Undo, KeyOutcome::Failed(error)) => {
                tracing::warn!(id, error, "Command-Z could not be pressed");
                refused_undo(UndoReply::Refused(UndoRefusal::Failed))
            }
        }
    }

    /// An `/enter` that waited for its recording: run it now that the
    /// recording has a verdict.
    pub(super) fn release_settled(&mut self, id: &str, now: Now) {
        let (mine, others): (Vec<_>, Vec<_>) = std::mem::take(&mut self.waiting)
            .into_iter()
            .partition(|waiting| matches!(waiting, Waiting::Settle { id: held, .. } if held == id));
        self.waiting = others;
        for waiting in mine {
            if let Waiting::Settle {
                key,
                device,
                inside,
                reply,
                ..
            } = waiting
            {
                self.resume_settled(key, id, &device, inside, reply, now);
            }
        }
    }

    /// Swift's `ratchetRefusal` after `waitUntilSettled`: a plaintext
    /// `/enter` can park before the envelope that makes its watch
    /// encrypted-only, so the check runs again once the wait is over.
    pub(super) fn resume_settled(
        &mut self,
        key: Key,
        id: &str,
        device: &str,
        inside: bool,
        reply: Reply,
        now: Now,
    ) {
        if !inside && self.paired.requires_e2ee(device) {
            return self.refuse(reply, Refusal::E2eeRequired);
        }
        self.press_key(key, id, (device, inside), reply, now);
    }
}

fn enter_refused(refusal: EnterRefusal) -> (u16, Value) {
    let refused = EnterReply::Refused(refusal);
    (refused.status(), refused.json())
}

fn refused_undo(reply: UndoReply) -> (u16, Value) {
    (reply.status(), reply.json())
}
