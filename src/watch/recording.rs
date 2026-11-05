//! A recording's lifecycle as the watch sees it: the state machine, the
//! `/status` projection, and the replies to `/enter` and `/undo`. The state
//! strings are shared with Wristper's watch app and the Python receiver
//! (`TrialState`), so they must not change.

use std::time::{Duration, SystemTime};

use serde_json::{Value, json};

use super::Timestamp;
use super::delivered::Entry;
use super::refusal::Refusal;

/// `/enter` works this long after the paste completed.
pub(crate) const ENTER_WINDOW_SECONDS: u64 = 30;
/// `/enter` waits this long for a recording still in progress to settle.
pub(crate) const ENTER_WAIT_SECONDS: u64 = 15;
pub(crate) const UNDO_WINDOW_SECONDS: u64 = 60;
/// How long audio waits for the `/focus` snapshot that precedes it.
pub(crate) const FOCUS_WAIT_SECONDS: u64 = 2;
/// A second upload of a recording still being processed waits this long for
/// the first before answering `duplicate`.
pub(crate) const DUPLICATE_AUDIO_WAIT_SECONDS: u64 = 45;

/// How the paste was confirmed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Verification {
    /// The text was read back from the focused field.
    ReadBack,
    /// The target is a terminal, which cannot be read back; sent is final.
    Terminal,
    #[default]
    None,
}

impl Verification {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::ReadBack => "read-back",
            Self::Terminal => "terminal",
            Self::None => "none",
        }
    }

    fn try_parse(text: &str) -> Option<Self> {
        match text {
            "read-back" => Some(Self::ReadBack),
            "terminal" => Some(Self::Terminal),
            "none" => Some(Self::None),
            _ => Option::None,
        }
    }

    pub(crate) fn is_verified(self) -> bool {
        self != Self::None
    }
}

/// What a paste left on its record. Kept as stored, not derived from the
/// state: an undone paste, for one, still reports how it was verified.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct PasteMarks {
    pub(crate) verified: bool,
    pub(crate) verification: Verification,
    pub(crate) entered: bool,
}

impl PasteMarks {
    /// A paste that just went out.
    pub(crate) fn sent(verification: Verification, entered: bool) -> Self {
        Self {
            verified: verification.is_verified(),
            verification,
            entered,
        }
    }

    /// Stored fields as Wristper reads them (`verification(of:)`): a
    /// `verification` string it does not know reads `read-back` when
    /// `verified`, else `none`.
    pub(crate) fn stored(verified: bool, verification: &str, entered: bool) -> Self {
        Self {
            verified,
            verification: Verification::try_parse(verification).unwrap_or(if verified {
                Verification::ReadBack
            } else {
                Verification::None
            }),
            entered,
        }
    }
}

/// The agent states (`TrialState.agent*`). Phase 1 runs no agent, but the
/// dedupe table Wristper wrote may hold them, and `/status` must still find
/// those recordings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AgentState {
    AwaitingConfirm,
    Running,
    Done,
    /// The Mac received and transcribed the audio, so for the watch this
    /// is final and delivered; only a plain `failed` makes it resend.
    Failed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum State {
    AwaitingAudio,
    Transcribing,
    Transcribed,
    Pasted,
    /// Sent, but the read-back has not (or could not) confirm it.
    Unverified,
    TargetChanged,
    NotEditable,
    Sensitive,
    Copied,
    Saved,
    Suspect,
    Failed,
    Undone,
    Segmented,
    Agent(AgentState),
}

impl State {
    const ALL: [State; 18] = [
        Self::AwaitingAudio,
        Self::Transcribing,
        Self::Transcribed,
        Self::Pasted,
        Self::Unverified,
        Self::TargetChanged,
        Self::NotEditable,
        Self::Sensitive,
        Self::Copied,
        Self::Saved,
        Self::Suspect,
        Self::Failed,
        Self::Undone,
        Self::Segmented,
        Self::Agent(AgentState::AwaitingConfirm),
        Self::Agent(AgentState::Running),
        Self::Agent(AgentState::Done),
        Self::Agent(AgentState::Failed),
    ];

    pub(crate) fn name(&self) -> &'static str {
        match self {
            Self::AwaitingAudio => "awaiting-audio",
            Self::Transcribing => "transcribing",
            Self::Transcribed => "transcribed",
            Self::Pasted => "pasted",
            Self::Unverified => "unverified",
            Self::TargetChanged => "target-changed",
            Self::NotEditable => "not-editable",
            Self::Sensitive => "sensitive",
            Self::Copied => "copied",
            Self::Saved => "saved",
            Self::Suspect => "suspect",
            Self::Failed => "failed",
            Self::Undone => "undone",
            Self::Segmented => "segmented",
            Self::Agent(AgentState::AwaitingConfirm) => "agent-awaiting-confirm",
            Self::Agent(AgentState::Running) => "agent-running",
            Self::Agent(AgentState::Done) => "agent-done",
            Self::Agent(AgentState::Failed) => "agent-failed",
        }
    }

    /// A stored state name; `None` for one no Wristper version writes.
    pub(crate) fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|state| state.name() == name)
    }

    /// `TrialState.pasteSent`: the text reached the target.
    pub(crate) fn is_paste_sent(&self) -> bool {
        matches!(self, Self::Pasted | Self::Unverified)
    }

    /// `TrialState.delivered`: the watch may drop its audio.
    pub(crate) fn is_delivered(&self) -> bool {
        self.is_paste_sent() || matches!(self, Self::Copied | Self::Saved)
    }

    /// `TrialState.inProgress`.
    pub(crate) fn is_in_progress(&self) -> bool {
        matches!(
            self,
            Self::AwaitingAudio | Self::Transcribing | Self::Transcribed
        )
    }

    /// `TrialState.recoverable`: Paste Last may take the text.
    pub(crate) fn is_recoverable(&self) -> bool {
        matches!(
            self,
            Self::TargetChanged
                | Self::NotEditable
                | Self::Sensitive
                | Self::Unverified
                | Self::Transcribed
                | Self::Pasted
                | Self::Failed
                | Self::Copied
                | Self::Saved
                | Self::Undone
                | Self::Agent(AgentState::Failed)
        )
    }
}

/// Where a `/status` reply travels. Outside the envelope the relay could
/// read it, so a dictation record's text and agent reply stay out
/// (`hideReply`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Channel {
    Plaintext,
    InsideE2ee,
}

/// One recording as `/status` reads it.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Record {
    pub(crate) id: String,
    pub(crate) state: State,
    pub(crate) paste: PasteMarks,
    pub(crate) language: String,
    pub(crate) text: String,
    pub(crate) raw_text: Option<String>,
    pub(crate) updated_at: String,
    pub(crate) completed_at: Option<String>,
    pub(crate) transcribe_ms: Option<u64>,
    pub(crate) paste_ms: Option<u64>,
    pub(crate) receive_to_paste_ms: Option<u64>,
    pub(crate) reason: Option<String>,
    pub(crate) reset: Option<Value>,
    pub(crate) paste_reason: Option<String>,
    pub(crate) error: Option<String>,
    pub(crate) truncated: bool,
    /// The paired watch whose `/focus` or `/audio` created it; `None` when
    /// the Mac did, or for a Swift-written dedupe entry.
    pub(crate) owner: Option<String>,
}

impl Record {
    /// What `/status` answers from the dedupe table when the live record is
    /// gone. `None` only for a state name no Wristper version writes.
    pub(crate) fn from_delivered(entry: &Entry) -> Option<Record> {
        Some(Record {
            id: entry.id.clone(),
            state: State::parse(&entry.state)?,
            paste: PasteMarks::stored(entry.verified, &entry.verification, entry.entered),
            language: String::new(),
            text: String::new(),
            raw_text: None,
            updated_at: entry
                .completed_at
                .clone()
                .unwrap_or_else(|| entry.recorded_at.clone()),
            completed_at: entry.completed_at.clone(),
            transcribe_ms: None,
            paste_ms: None,
            receive_to_paste_ms: None,
            reason: None,
            reset: None,
            paste_reason: None,
            error: None,
            truncated: entry.truncated,
            owner: entry.owner.clone(),
        })
    }

    /// The `/status` body of a dictation record (no agent route). Agent
    /// keys are null inside the envelope and, with the text, absent outside.
    pub(crate) fn status_json(&self, channel: Channel, now: SystemTime) -> Value {
        let mut body = json!({
            "ok": true,
            "id": self.id,
            "transcribe_ms": self.transcribe_ms,
            "paste_ms": self.paste_ms,
            "receive_to_paste_ms": self.receive_to_paste_ms,
            "reason": self.reason,
            "reset": self.reset,
            "paste_reason": self.paste_reason,
            "error": self.error,
            "completed_at": self.completed_at,
            "state": self.state.name(),
            "language": self.language,
            "verified": self.paste.verified,
            "verification": self.paste.verification.as_str(),
            "entered": self.paste.entered,
            "updated_at": self.updated_at,
            "route": null,
            "agent_ms": null,
            "session_id": null,
            "completed_age": self.completed_age(now),
        });
        if channel == Channel::InsideE2ee {
            for key in [
                "agent_summary",
                "agent_activity",
                "agent_activity_at",
                "agent_delivery",
            ] {
                body[key] = Value::Null;
            }
            body["text"] = Value::from(self.text.as_str());
            body["raw_text"] = json!(self.raw_text);
        }
        if self.truncated {
            body["truncated"] = Value::Bool(true);
        }
        body
    }

    /// The answer to a resent recording that was already handled.
    pub(crate) fn duplicate_json(&self) -> Value {
        let mut body = json!({
            "ok": true,
            "duplicate": true,
            "state": self.state.name(),
            "verified": self.paste.verified,
            "verification": self.paste.verification.as_str(),
        });
        if self.truncated {
            body["truncated"] = Value::Bool(true);
        }
        body
    }

    /// Seconds since completion, to two decimals; null without a parseable
    /// `completed_at`.
    fn completed_age(&self, now: SystemTime) -> Option<f64> {
        let completed = Timestamp::parse(self.completed_at.as_deref()?)?;
        Some((age_seconds(completed, now) * 100.0).round() / 100.0)
    }
}

/// Negative when `completed` lies in the future.
fn age_seconds(completed: SystemTime, now: SystemTime) -> f64 {
    match now.duration_since(completed) {
        Ok(age) => age.as_secs_f64(),
        Err(ahead) => -ahead.duration().as_secs_f64(),
    }
}

fn within(completed: SystemTime, now: SystemTime, window_seconds: u64) -> bool {
    age_seconds(completed, now) <= Duration::from_secs(window_seconds).as_secs_f64()
}

/// Who sent a request (`Caller` in Wristper, without the tool token).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Caller<'a> {
    Master,
    Device(&'a str),
}

/// `notOwnerRefusal`: one watch's token cannot `/focus`, `/enter`, or
/// `/undo` another watch's recording. `owner` is the recording's, live or
/// remembered. Checked before anything else about the record; a record
/// without an owner, an unknown id, and the Mac itself all pass.
pub(crate) fn refuse_non_owner(owner: Option<&str>, caller: Caller) -> Result<(), Refusal> {
    let Caller::Device(device) = caller else {
        return Ok(());
    };
    match owner {
        Some(owner) if !device.is_empty() && !owner.is_empty() && owner != device => {
            Err(Refusal::NotOwner)
        }
        _ => Ok(()),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum EnterRefusal {
    UnknownId,
    NotPasted,
    Expired,
    TargetChanged,
    /// A password field or secure keyboard entry has focus.
    Sensitive,
    /// The key press itself failed: a 500, unlike the others.
    Failed,
}

impl EnterRefusal {
    pub(crate) fn reason(self) -> &'static str {
        match self {
            Self::UnknownId => "unknown-id",
            Self::NotPasted => "not-pasted",
            Self::Expired => "expired",
            Self::TargetChanged => "target-changed",
            Self::Sensitive => "sensitive",
            Self::Failed => "failed",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum EnterReply {
    Entered,
    AlreadyEntered,
    Refused(EnterRefusal),
}

impl EnterReply {
    pub(crate) fn status(&self) -> u16 {
        match self {
            Self::Refused(EnterRefusal::Failed) => 500,
            _ => 200,
        }
    }

    /// `reason` is the field the watch compares; `error` repeats it so every
    /// endpoint's failure reads from `error`.
    pub(crate) fn json(&self) -> Value {
        match self {
            Self::Entered => json!({ "ok": true, "state": "entered" }),
            Self::AlreadyEntered => json!({ "ok": true, "state": "entered", "duplicate": true }),
            Self::Refused(refusal) => {
                json!({ "ok": false, "reason": refusal.reason(), "error": refusal.reason() })
            }
        }
    }
}

/// Whether `/enter` may press Return for `record` now. `Err` is the reply
/// to send instead (a refusal, or `AlreadyEntered`, which needs no press).
pub(crate) fn check_enter(record: Option<&Record>, now: SystemTime) -> Result<(), EnterReply> {
    let record = record.ok_or(EnterReply::Refused(EnterRefusal::UnknownId))?;
    if !record.state.is_paste_sent() {
        return Err(EnterReply::Refused(EnterRefusal::NotPasted));
    }
    if record.paste.entered {
        return Err(EnterReply::AlreadyEntered);
    }
    let expired = EnterReply::Refused(EnterRefusal::Expired);
    let completed = record
        .completed_at
        .as_deref()
        .and_then(Timestamp::parse)
        .ok_or(expired)?;
    if within(completed, now, ENTER_WINDOW_SECONDS) {
        Ok(())
    } else {
        Err(expired)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum UndoRefusal {
    UnknownId,
    NotPasted,
    Entered,
    Expired,
    /// A terminal paste, or one never read back: Command-Z would undo an
    /// unknown amount.
    NotUndoable,
    TargetChanged,
    /// The undo keystroke failed: a 500.
    Failed,
}

impl UndoRefusal {
    pub(crate) fn reason(self) -> &'static str {
        match self {
            Self::UnknownId => "unknown-id",
            Self::NotPasted => "not-pasted",
            Self::Entered => "entered",
            Self::Expired => "expired",
            Self::NotUndoable => "not-undoable",
            Self::TargetChanged => "target-changed",
            Self::Failed => "failed",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum UndoReply {
    Undone,
    Refused(UndoRefusal),
}

impl UndoReply {
    pub(crate) fn status(&self) -> u16 {
        match self {
            Self::Refused(UndoRefusal::Failed) => 500,
            _ => 200,
        }
    }

    pub(crate) fn json(&self) -> Value {
        match self {
            Self::Undone => json!({ "ok": true, "state": "undone" }),
            Self::Refused(refusal) => json!({ "ok": false, "reason": refusal.reason() }),
        }
    }
}

/// Whether `/undo` may send Command-Z for `record` now.
pub(crate) fn check_undo(record: Option<&Record>, now: SystemTime) -> Result<(), UndoRefusal> {
    let record = record.ok_or(UndoRefusal::UnknownId)?;
    if !record.state.is_paste_sent() {
        return Err(UndoRefusal::NotPasted);
    }
    if record.paste.entered {
        return Err(UndoRefusal::Entered);
    }
    let completed = record
        .completed_at
        .as_deref()
        .and_then(Timestamp::parse)
        .ok_or(UndoRefusal::Expired)?;
    if !within(completed, now, UNDO_WINDOW_SECONDS) {
        return Err(UndoRefusal::Expired);
    }
    if record.paste.verification == Verification::ReadBack {
        Ok(())
    } else {
        Err(UndoRefusal::NotUndoable)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::UNIX_EPOCH;

    fn at(seconds: u64) -> SystemTime {
        UNIX_EPOCH + Duration::from_secs(1_800_000_000 + seconds)
    }

    fn read_back(entered: bool) -> (State, PasteMarks) {
        (
            State::Pasted,
            PasteMarks::sent(Verification::ReadBack, entered),
        )
    }

    fn record((state, paste): (State, PasteMarks), completed_at: Option<SystemTime>) -> Record {
        Record {
            id: "11111111-1111-1111-1111-111111111111".into(),
            state,
            paste,
            language: "en".into(),
            text: "hello".into(),
            raw_text: Some("hello raw".into()),
            updated_at: "2026-11-04T00:00:00.000000+00:00".into(),
            completed_at: completed_at.map(Timestamp::string),
            transcribe_ms: Some(120),
            paste_ms: None,
            receive_to_paste_ms: Some(900),
            reason: None,
            reset: None,
            paste_reason: None,
            error: None,
            truncated: false,
            owner: Some("w1".into()),
        }
    }

    fn plain(state: State) -> (State, PasteMarks) {
        (state, PasteMarks::default())
    }

    fn unverified() -> (State, PasteMarks) {
        (
            State::Unverified,
            PasteMarks::sent(Verification::None, false),
        )
    }

    #[test]
    fn every_state_name_round_trips_including_the_agent_states() {
        for state in State::ALL {
            assert_eq!(State::parse(state.name()), Some(state), "{}", state.name());
        }
        assert_eq!(
            State::parse("agent-done"),
            Some(State::Agent(AgentState::Done))
        );
        assert_eq!(State::parse("agent-later"), None);
    }

    #[test]
    fn state_names_are_the_wire_strings() {
        let names: Vec<_> = State::ALL.iter().map(|s| s.name()).collect();
        assert_eq!(
            names,
            [
                "awaiting-audio",
                "transcribing",
                "transcribed",
                "pasted",
                "unverified",
                "target-changed",
                "not-editable",
                "sensitive",
                "copied",
                "saved",
                "suspect",
                "failed",
                "undone",
                "segmented",
                "agent-awaiting-confirm",
                "agent-running",
                "agent-done",
                "agent-failed",
            ]
        );
    }

    #[test]
    fn stored_marks_are_kept_and_an_unknown_verification_falls_back_on_verified() {
        assert_eq!(
            PasteMarks::stored(true, "garbage", false).verification,
            Verification::ReadBack
        );
        assert_eq!(
            PasteMarks::stored(false, "", false).verification,
            Verification::None
        );
        let odd = PasteMarks::stored(true, "none", true);
        assert_eq!(
            (odd.verified, odd.verification, odd.entered),
            (true, Verification::None, true),
            "a valid stored pair is not re-derived"
        );
        assert!(PasteMarks::sent(Verification::Terminal, false).verified);
    }

    #[test]
    fn state_sets_match_trial_state() {
        let named = |predicate: fn(&State) -> bool| -> Vec<&'static str> {
            let mut names: Vec<_> = State::ALL
                .iter()
                .filter(|s| predicate(s))
                .map(|s| s.name())
                .collect();
            names.sort();
            names
        };
        assert_eq!(named(State::is_paste_sent), ["pasted", "unverified"]);
        assert_eq!(
            named(State::is_delivered),
            ["copied", "pasted", "saved", "unverified"]
        );
        assert_eq!(
            named(State::is_in_progress),
            ["awaiting-audio", "transcribed", "transcribing"]
        );
        assert_eq!(
            named(State::is_recoverable),
            [
                "agent-failed",
                "copied",
                "failed",
                "not-editable",
                "pasted",
                "saved",
                "sensitive",
                "target-changed",
                "transcribed",
                "undone",
                "unverified",
            ]
        );
    }

    fn keys(body: &Value) -> Vec<String> {
        let mut keys: Vec<_> = body.as_object().unwrap().keys().cloned().collect();
        keys.sort();
        keys
    }

    const PLAINTEXT_KEYS: [&str; 20] = [
        "ok",
        "id",
        "transcribe_ms",
        "paste_ms",
        "receive_to_paste_ms",
        "reason",
        "reset",
        "paste_reason",
        "error",
        "completed_at",
        "state",
        "language",
        "verified",
        "verification",
        "entered",
        "updated_at",
        "route",
        "agent_ms",
        "session_id",
        "completed_age",
    ];

    #[test]
    fn status_inside_the_envelope_has_exactly_the_wristper_key_set() {
        let body = record(read_back(false), Some(at(0))).status_json(Channel::InsideE2ee, at(1));
        let mut expected: Vec<String> = PLAINTEXT_KEYS.iter().map(|key| key.to_string()).collect();
        expected.extend(
            [
                "agent_summary",
                "agent_activity",
                "agent_activity_at",
                "agent_delivery",
                "text",
                "raw_text",
            ]
            .map(String::from),
        );
        expected.sort();
        assert_eq!(keys(&body), expected);
        assert_eq!(body["paste_ms"], Value::Null);
        assert_eq!(body["transcribe_ms"], 120);
        assert_eq!(body["verification"], "read-back");
        assert_eq!(body["text"], "hello");
        assert_eq!(body["agent_summary"], Value::Null);
    }

    #[test]
    fn plaintext_status_omits_the_text_and_the_agent_reply() {
        let body = record(read_back(false), Some(at(0))).status_json(Channel::Plaintext, at(1));
        let mut expected: Vec<String> = PLAINTEXT_KEYS.iter().map(|key| key.to_string()).collect();
        expected.sort();
        assert_eq!(keys(&body), expected);
        assert!(!body.to_string().contains("hello"), "{body}");
    }

    #[test]
    fn status_reports_the_stored_marks_not_ones_derived_from_the_state() {
        let undone = record(
            (
                State::Undone,
                PasteMarks::sent(Verification::ReadBack, false),
            ),
            None,
        );
        let body = undone.status_json(Channel::InsideE2ee, at(0));
        assert_eq!(
            (&body["state"], &body["verified"], &body["verification"]),
            (&json!("undone"), &json!(true), &json!("read-back"))
        );
        assert_eq!(undone.duplicate_json()["verified"], true);
    }

    #[test]
    fn truncated_appears_only_when_true() {
        let mut item = record(read_back(false), None);
        assert!(
            item.status_json(Channel::Plaintext, at(0))
                .get("truncated")
                .is_none()
        );
        assert!(item.duplicate_json().get("truncated").is_none());
        item.truncated = true;
        assert_eq!(
            item.status_json(Channel::Plaintext, at(0))["truncated"],
            true
        );
        assert_eq!(item.duplicate_json()["truncated"], true);
    }

    #[test]
    fn completed_age_is_null_without_completion_and_two_decimals_with_it() {
        let none = record(plain(State::Transcribed), None).status_json(Channel::Plaintext, at(5));
        assert_eq!(none["completed_age"], Value::Null);
        let mut item = record(read_back(false), Some(at(0)));
        item.completed_at = Some(Timestamp::string(at(0) + Duration::from_micros(1_234_567)));
        let age = item.status_json(Channel::Plaintext, at(4))["completed_age"]
            .as_f64()
            .unwrap();
        assert_eq!(age, 2.77);
    }

    #[test]
    fn duplicate_json_carries_the_stored_verification() {
        let body = record(unverified(), None).duplicate_json();
        assert_eq!(
            body,
            json!({ "ok": true, "duplicate": true, "state": "unverified",
                    "verified": false, "verification": "none" })
        );
    }

    #[test]
    fn only_the_owning_watch_or_the_mac_may_act_on_a_live_recording() {
        let owned = record(read_back(false), Some(at(0)));
        let refused = refuse_non_owner(owned.owner.as_deref(), Caller::Device("w2")).unwrap_err();
        assert_eq!(refused, Refusal::NotOwner);
        assert_eq!(refused.status(), 403);
        assert_eq!(refused.body(), json!({ "ok": false, "error": "not-owner" }));
        assert_eq!(
            refuse_non_owner(owned.owner.as_deref(), Caller::Device("w1")),
            Ok(())
        );
        assert_eq!(
            refuse_non_owner(owned.owner.as_deref(), Caller::Master),
            Ok(())
        );
        assert_eq!(
            refuse_non_owner(owned.owner.as_deref(), Caller::Device("")),
            Ok(()),
            "an empty device id is no device (`deviceID(of:)`)"
        );
        assert_eq!(
            refuse_non_owner(None, Caller::Device("w2")),
            Ok(()),
            "an unknown id answers unknown-id later"
        );
        for legacy in [None, Some(String::new())] {
            let mut unowned = owned.clone();
            unowned.owner = legacy;
            assert_eq!(
                refuse_non_owner(unowned.owner.as_deref(), Caller::Device("w2")),
                Ok(()),
                "a record without an owner stays open, as in Swift"
            );
        }
    }

    #[test]
    fn from_delivered_uses_completed_at_else_recorded_at() {
        let mut entry = Entry {
            id: "11111111-1111-1111-1111-111111111111".into(),
            state: "pasted".into(),
            verified: true,
            verification: "read-back".into(),
            truncated: true,
            entered: true,
            completed_at: Some("completed".into()),
            recorded_at: "recorded".into(),
            owner: None,
        };
        let with = Record::from_delivered(&entry).unwrap();
        assert_eq!(with.updated_at, "completed");
        assert_eq!(with.completed_at.as_deref(), Some("completed"));
        assert!(with.truncated && with.paste.entered);
        assert_eq!((with.text.as_str(), with.language.as_str()), ("", ""));
        assert_eq!(with.owner, None, "a Swift-written entry has no owner");
        entry.completed_at = None;
        let without = Record::from_delivered(&entry).unwrap();
        assert_eq!(without.updated_at, "recorded");
        assert_eq!(without.completed_at, None);
        entry.state = "agent-done".into();
        let agent = Record::from_delivered(&entry).expect("Swift-written agent records stay known");
        assert_eq!(agent.state, State::Agent(AgentState::Done));
        assert_eq!(
            agent.status_json(Channel::Plaintext, at(0))["state"],
            "agent-done"
        );
        entry.state = "agent-later".into();
        assert!(Record::from_delivered(&entry).is_none());
    }

    #[test]
    fn enter_follows_the_wristper_order_of_refusals() {
        let now = at(100);
        let refused = |reason| Err(EnterReply::Refused(reason));
        assert_eq!(check_enter(None, now), refused(EnterRefusal::UnknownId));
        assert_eq!(
            check_enter(Some(&record(plain(State::Copied), Some(at(99)))), now),
            refused(EnterRefusal::NotPasted)
        );
        assert_eq!(
            check_enter(Some(&record(read_back(true), Some(at(0)))), now),
            Err(EnterReply::AlreadyEntered),
            "an entered record is a duplicate even after the window"
        );
        assert_eq!(
            check_enter(Some(&record(read_back(false), None)), now),
            refused(EnterRefusal::Expired)
        );
        assert_eq!(
            check_enter(Some(&record(read_back(false), Some(at(70)))), now),
            Ok(()),
            "exactly 30 s is still inside"
        );
        assert_eq!(
            check_enter(Some(&record(read_back(false), Some(at(69)))), now),
            refused(EnterRefusal::Expired)
        );
    }

    #[test]
    fn enter_allows_an_unverified_paste() {
        assert_eq!(
            check_enter(Some(&record(unverified(), Some(at(95)))), at(100)),
            Ok(())
        );
    }

    #[test]
    fn undo_refuses_what_command_z_could_not_safely_revert() {
        let now = at(100);
        let check = |state, completed| check_undo(Some(&record(state, completed)), now);
        assert_eq!(check_undo(None, now), Err(UndoRefusal::UnknownId));
        assert_eq!(
            check(plain(State::Saved), Some(at(99))),
            Err(UndoRefusal::NotPasted)
        );
        assert_eq!(
            check(read_back(true), Some(at(99))),
            Err(UndoRefusal::Entered)
        );
        assert_eq!(check(read_back(false), None), Err(UndoRefusal::Expired));
        assert_eq!(
            check(read_back(false), Some(at(39))),
            Err(UndoRefusal::Expired)
        );
        assert_eq!(check(read_back(false), Some(at(40))), Ok(()));
        let terminal = (
            State::Pasted,
            PasteMarks::sent(Verification::Terminal, false),
        );
        assert_eq!(check(terminal, Some(at(99))), Err(UndoRefusal::NotUndoable));
        assert_eq!(
            check(unverified(), Some(at(99))),
            Err(UndoRefusal::NotUndoable)
        );
    }

    #[test]
    fn reply_bodies_and_statuses_match_the_watch_contract() {
        assert_eq!(
            EnterReply::Entered.json(),
            json!({ "ok": true, "state": "entered" })
        );
        assert_eq!(
            EnterReply::AlreadyEntered.json(),
            json!({ "ok": true, "state": "entered", "duplicate": true })
        );
        let refused = EnterReply::Refused(EnterRefusal::TargetChanged);
        assert_eq!(
            refused.json(),
            json!({ "ok": false, "reason": "target-changed", "error": "target-changed" })
        );
        assert_eq!(refused.status(), 200);
        let failed = EnterReply::Refused(EnterRefusal::Failed);
        assert_eq!(failed.status(), 500);
        assert_eq!(failed.json()["error"], "failed");
        assert_eq!(
            UndoReply::Undone.json(),
            json!({ "ok": true, "state": "undone" })
        );
        let undo = UndoReply::Refused(UndoRefusal::NotUndoable);
        assert_eq!(
            undo.json(),
            json!({ "ok": false, "reason": "not-undoable" })
        );
        assert_eq!(UndoReply::Refused(UndoRefusal::Failed).status(), 500);
    }
}
