//! The status capsule on the Mac: what a watch recording is doing, in
//! Wristper's words (`StatusOverlay.poll` and `PasteDisplay`). This is the
//! pure part: which capsule a recording's record earns, how long it stays,
//! and the two languages it speaks. The `watch` thread pushes each change
//! through [`watch`]ed sinks as it happens; the overlay that draws it is
//! `rune_ui::watch_status`.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::recording::{Record, State, Verification};

/// A paste still `unverified` this long after it was sent is shown as
/// unconfirmed; the read-back usually settles it sooner.
pub(crate) const READ_BACK_GRACE: Duration = Duration::from_millis(1400);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Tone {
    Recording,
    Working,
    Done,
    Caution,
    Warning,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    Recording,
    Transcribing,
    Inserted,
    InsertedInTerminal,
    InsertedAndEntered,
    InsertedInTerminalAndEntered,
    Unconfirmed,
    UnconfirmedAndEntered,
    Undone,
    Copied,
    TargetChanged,
    NotEditable,
    SecureInput,
    PasswordField,
    Suspect,
    Failed,
    NoAccessibility,
}

impl Kind {
    /// Traditional Chinese or English, by the system's language order.
    pub(crate) fn text(self, chinese: bool) -> &'static str {
        match (self, chinese) {
            (Self::Recording, true) => "錄音中",
            (Self::Recording, false) => "Recording",
            (Self::Transcribing, true) => "轉錄中…",
            (Self::Transcribing, false) => "Transcribing…",
            (Self::Inserted, true) => "已插入",
            (Self::Inserted, false) => "Inserted",
            (Self::InsertedInTerminal, true) => "已貼上（終端機）",
            (Self::InsertedInTerminal, false) => "Pasted (Terminal)",
            (Self::InsertedAndEntered, true) => "已插入並按 Enter",
            (Self::InsertedAndEntered, false) => "Inserted & Sent Enter",
            (Self::InsertedInTerminalAndEntered, true) => "已貼上（終端機）並按 Enter",
            (Self::InsertedInTerminalAndEntered, false) => "Pasted (Terminal) & Sent Enter",
            (Self::Unconfirmed, true) => "已送出，無法確認",
            (Self::Unconfirmed, false) => "Sent; couldn't confirm",
            (Self::UnconfirmedAndEntered, true) => "已送出並按 Enter，無法確認",
            (Self::UnconfirmedAndEntered, false) => "Sent & Pressed Enter; couldn't confirm",
            (Self::Undone, true) => "已撤回",
            (Self::Undone, false) => "Undone",
            (Self::Copied, true) => "已複製，請手動貼上",
            (Self::Copied, false) => "Copied; please paste manually",
            (Self::TargetChanged, true) => "欄位已改變，未貼上；可用「貼上上一筆」取回",
            (Self::TargetChanged, false) => "Field changed; not pasted. Paste Last has the text",
            (Self::NotEditable, true) => "這裡不能輸入，未貼上；可用「貼上上一筆」取回",
            (Self::NotEditable, false) => "Not editable; not pasted. Paste Last has the text",
            (Self::SecureInput, true) => {
                "安全輸入開啟中（可能是密碼提示，或終端機開了「安全鍵盤輸入」），未貼上"
            }
            (Self::SecureInput, false) => {
                "Secure input is on (a password prompt, or Secure Keyboard Entry in a terminal); not pasted"
            }
            (Self::PasswordField, true) => "密碼欄位不貼上",
            (Self::PasswordField, false) => "Password field; not pasted",
            (Self::Suspect, true) => "沒聽清楚，沒有貼上",
            (Self::Suspect, false) => "Unclear audio; not pasted",
            (Self::Failed, true) => "處理失敗，請看紀錄",
            (Self::Failed, false) => "Failed; check the log",
            (Self::NoAccessibility, true) => "沒有輔助使用權限，沒有貼上",
            (Self::NoAccessibility, false) => "No Accessibility permission; not pasted",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Capsule {
    pub(crate) kind: Kind,
    pub(crate) tone: Tone,
}

/// A capsule and how long it stays up.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Display {
    pub(crate) capsule: Capsule,
    pub(crate) hide_after: Duration,
}

fn show(kind: Kind, tone: Tone, seconds: f64) -> Option<Display> {
    Some(Display {
        capsule: Capsule { kind, tone },
        hide_after: Duration::from_secs_f64(seconds),
    })
}

/// The capsule `record` earns now. `accessibility` is whether Hex may use
/// Accessibility, which names the cause of a failure. `None` for states
/// the Mac does not announce.
pub(crate) fn display(record: &Record, accessibility: bool) -> Option<Display> {
    let entered = record.paste.entered;
    let terminal = record.paste.verification == Verification::Terminal;
    match record.state {
        State::AwaitingAudio => show(Kind::Recording, Tone::Recording, 180.0),
        State::Transcribing | State::Transcribed => show(Kind::Transcribing, Tone::Working, 60.0),
        State::Pasted => {
            let kind = match (terminal, entered) {
                (false, false) => Kind::Inserted,
                (true, false) => Kind::InsertedInTerminal,
                (false, true) => Kind::InsertedAndEntered,
                (true, true) => Kind::InsertedInTerminalAndEntered,
            };
            show(kind, Tone::Done, 1.2)
        }
        State::Unverified => {
            let kind = if entered {
                Kind::UnconfirmedAndEntered
            } else {
                Kind::Unconfirmed
            };
            show(kind, Tone::Caution, 2.5)
        }
        State::Undone => show(Kind::Undone, Tone::Done, 1.2),
        State::Copied => show(Kind::Copied, Tone::Done, 2.5),
        State::TargetChanged => show(Kind::TargetChanged, Tone::Warning, 2.5),
        State::NotEditable => show(Kind::NotEditable, Tone::Warning, 2.5),
        State::Sensitive => {
            if record.paste_reason.as_deref() == Some("secure-input") {
                show(Kind::SecureInput, Tone::Warning, 5.0)
            } else {
                show(Kind::PasswordField, Tone::Warning, 2.5)
            }
        }
        State::Suspect => show(Kind::Suspect, Tone::Warning, 2.5),
        State::Failed => {
            let kind = if accessibility {
                Kind::Failed
            } else {
                Kind::NoAccessibility
            };
            show(kind, Tone::Warning, 3.0)
        }
        State::Saved | State::Segmented | State::Agent(_) => None,
    }
}

/// Whether the system lists Chinese before English
/// (`UILanguage.isChinese`).
pub(crate) fn prefers_chinese(preferred: &[String]) -> bool {
    for identifier in preferred {
        let base = identifier
            .to_lowercase()
            .replace('_', "-")
            .split('-')
            .next()
            .unwrap_or_default()
            .to_owned();
        match base.as_str() {
            "zh" | "yue" => return true,
            "en" => return false,
            _ => {}
        }
    }
    false
}

/// Where capsules go: `None` hides. Called from the `watch` thread, so a
/// sink hands the capsule to its own thread and never waits.
pub(crate) type Sink = Arc<dyn Fn(Option<Capsule>) + Send + Sync>;

static SINKS: Mutex<Vec<Sink>> = Mutex::new(Vec::new());

/// Registers the overlay that draws capsules.
pub(crate) fn watch(sink: impl Fn(Option<Capsule>) + Send + Sync + 'static) {
    SINKS
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .push(Arc::new(sink));
}

/// Tells every registered sink; the service's default sink.
pub(crate) fn publish(capsule: Option<Capsule>) {
    let sinks: Vec<Sink> = SINKS
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .clone();
    for sink in sinks {
        sink(capsule);
    }
}

#[cfg(test)]
mod tests;
