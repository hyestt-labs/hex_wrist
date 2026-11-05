//! The field a watch recording pastes into: the snapshot `/focus` takes of
//! the focused element, Wristper's matching rules, and the decision a paste
//! makes against them before it touches the clipboard (Swift's
//! `FieldSnapshot`, `PasteOutcomeDecision`, `TerminalApps`, and the gate at
//! the top of `FieldPaster.paste`). Pure: the macOS adapter (`watch::ax`)
//! only fills `Focused` in and carries the verdict out.

use super::recording::Verification;

/// Points on the screen, as the Accessibility API reports them.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Rect {
    pub(crate) x: f64,
    pub(crate) y: f64,
    pub(crate) width: f64,
    pub(crate) height: f64,
}

impl Rect {
    /// Within `SLACK` on every side.
    fn near(&self, other: &Rect) -> bool {
        (self.x - other.x).abs() <= SLACK
            && (self.y - other.y).abs() <= SLACK
            && (self.width - other.width).abs() <= SLACK
            && (self.height - other.height).abs() <= SLACK
    }
}

/// How far a field (or its window) may move or grow and still be the same
/// one: a resized sidebar, a window nudged while the watch recorded.
const SLACK: f64 = 48.0;

/// The focused element as `/focus` saw it.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct FieldSnapshot {
    pub(crate) pid: i32,
    pub(crate) role: String,
    pub(crate) subrole: String,
    pub(crate) identifier: String,
    pub(crate) title: String,
    pub(crate) frame: Rect,
    pub(crate) window: Option<Rect>,
    pub(crate) bundle_id: Option<String>,
    pub(crate) app_name: Option<String>,
}

impl FieldSnapshot {
    /// Whether the focus now is the field remembered at the recording's
    /// start: same process and role; the same identifier when both have
    /// one; else no conflicting title and the frame within `SLACK`.
    pub(crate) fn matches(&self, other: &FieldSnapshot) -> bool {
        if self.pid != other.pid || self.role != other.role {
            return false;
        }
        if !self.identifier.is_empty() && !other.identifier.is_empty() {
            return self.identifier == other.identifier;
        }
        if !self.title.is_empty() && !other.title.is_empty() && self.title != other.title {
            return false;
        }
        self.frame.near(&other.frame)
    }

    /// For Enter and Undo after the paste: the field's own frame moved with
    /// the caret (a terminal grows its prompt line), so only the process,
    /// role, identifier, and window are compared.
    pub(crate) fn same_field_after_typing(&self, other: &FieldSnapshot) -> bool {
        if self.pid != other.pid || self.role != other.role {
            return false;
        }
        if !self.identifier.is_empty() && !other.identifier.is_empty() {
            return self.identifier == other.identifier;
        }
        match (&self.window, &other.window) {
            (Some(window), Some(other_window)) => window.near(other_window),
            _ => self.matches(other),
        }
    }

    /// A password field: `AXSecureTextField` as role or subrole.
    pub(crate) fn is_sensitive(&self) -> bool {
        self.role == "AXSecureTextField" || self.subrole == "AXSecureTextField"
    }

    pub(crate) fn is_terminal(&self) -> bool {
        is_terminal(self.bundle_id.as_deref())
    }
}

/// Apps whose text cannot be read back through Accessibility; a paste sent
/// to one is final (`TerminalApps.builtIn`).
const TERMINAL_APPS: &[&str] = &[
    "com.apple.Terminal",
    "com.googlecode.iterm2",
    "dev.warp.Warp-Stable",
    "dev.warp.Warp",
    "com.mitchellh.ghostty",
    "net.kovidgoyal.kitty",
    "org.alacritty",
    "io.alacritty",
    "com.github.wez.wezterm",
    "co.zeit.hyper",
    "com.stablyai.orca",
];

pub(crate) fn is_terminal(bundle_id: Option<&str>) -> bool {
    bundle_id.is_some_and(|id| !id.is_empty() && TERMINAL_APPS.contains(&id))
}

/// What the paste must match, decided when the audio is accepted.
#[derive(Clone, Debug, PartialEq)]
// Kept inline: one lives per recording, and the tests read it by value.
#[allow(clippy::large_enum_variant)]
pub(crate) enum Expectation {
    /// The field `/focus` captured when the watch started recording.
    Field(FieldSnapshot),
    /// Paste wherever the cursor is: `focus: current` (a resend from the
    /// watch's unsent list), or a `/focus` that captured nothing.
    Current,
    /// `focus: requested`, but no `/focus` arrived in time: nothing is
    /// pasted, and the watch reads `target-changed` with a reason.
    Missing,
}

/// The focused element as the paste finds it.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Focused {
    pub(crate) field: FieldSnapshot,
    /// `kAXValueAttribute` is settable, or the role is a text field, text
    /// area, combo box, search field, or cell.
    pub(crate) editable: bool,
}

/// Why a paste was withheld, as the record keeps it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Withheld {
    /// The focus is not the field the recording started in.
    TargetChanged,
    /// `focus: requested` without a `/focus` snapshot.
    NoSnapshot,
    NotEditable,
    /// Secure keyboard entry is on: a password prompt, or a terminal's
    /// Secure Keyboard Entry; no app receives synthetic keys.
    SecureInput,
    /// A password field.
    PasswordField,
}

impl Withheld {
    /// `paste_reason` in the record (`PasteReason.secureInput`).
    pub(crate) fn paste_reason(self) -> Option<&'static str> {
        match self {
            Self::SecureInput => Some("secure-input"),
            _ => None,
        }
    }

    /// `reason` in the record: Wristper writes one only for the missing
    /// snapshot.
    pub(crate) fn reason(self, chinese: bool) -> Option<&'static str> {
        match (self, chinese) {
            (Self::NoSnapshot, true) => Some("沒有收到錄音開始時的欄位資訊，未自動貼上"),
            (Self::NoSnapshot, false) => {
                Some("Focused field was not captured when recording started; not auto-pasted")
            }
            _ => None,
        }
    }
}

/// What the output worker does with the text.
#[derive(Clone, Debug, PartialEq)]
#[allow(clippy::large_enum_variant)]
pub(crate) enum Gate {
    /// Send ⌘V. `Terminal` is final at once; `None` awaits the read-back.
    Paste {
        target: FieldSnapshot,
        verification: Verification,
    },
    /// No focused element and nothing expected: leave the text on the
    /// clipboard.
    Copy,
    Withhold(Withheld),
}

/// Swift's `FieldPaster.paste` up to the clipboard write.
pub(crate) fn gate(
    expectation: &Expectation,
    focused: Option<&Focused>,
    secure_input: bool,
) -> Gate {
    if secure_input {
        return Gate::Withhold(Withheld::SecureInput);
    }
    let expected = match expectation {
        Expectation::Missing => return Gate::Withhold(Withheld::NoSnapshot),
        Expectation::Field(field) => Some(field),
        Expectation::Current => None,
    };
    let Some(focused) = focused else {
        return match expected {
            Some(_) => Gate::Withhold(Withheld::TargetChanged),
            None => Gate::Copy,
        };
    };
    if focused.field.is_sensitive() {
        return Gate::Withhold(Withheld::PasswordField);
    }
    if let Some(expected) = expected {
        if !expected.matches(&focused.field) {
            return Gate::Withhold(Withheld::TargetChanged);
        }
        if !focused.editable {
            return Gate::Withhold(Withheld::NotEditable);
        }
    }
    let verification = if focused.field.is_terminal() {
        Verification::Terminal
    } else {
        Verification::None
    };
    Gate::Paste {
        target: focused.field.clone(),
        verification,
    }
}

/// Whether `/enter` or `/undo` may press its key now: the recording's
/// field is still the focus (`sameFieldAfterTyping`), and it is not a
/// password field.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum KeyGate {
    Press,
    TargetChanged,
    Sensitive,
}

pub(crate) fn key_gate(
    expectation: &Expectation,
    focused: Option<&FieldSnapshot>,
    secure_input: bool,
) -> KeyGate {
    if secure_input {
        return KeyGate::Sensitive;
    }
    let (Expectation::Field(expected), Some(focused)) = (expectation, focused) else {
        return KeyGate::TargetChanged;
    };
    if focused.is_sensitive() {
        return KeyGate::Sensitive;
    }
    if expected.same_field_after_typing(focused) {
        KeyGate::Press
    } else {
        KeyGate::TargetChanged
    }
}

/// Whether the field now holds the pasted text: the value changed since
/// before the paste and contains the text, trimmed, since apps add a
/// trailing space or drop a newline.
pub(crate) fn read_back_confirms(text: &str, before: Option<&str>, value: Option<&str>) -> bool {
    let wanted = text.trim();
    if wanted.is_empty() {
        return false;
    }
    match value {
        Some(value) => value != before.unwrap_or("") && value.contains(wanted),
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn field(pid: i32, role: &str) -> FieldSnapshot {
        FieldSnapshot {
            pid,
            role: role.into(),
            frame: Rect {
                x: 100.0,
                y: 200.0,
                width: 300.0,
                height: 40.0,
            },
            window: Some(Rect {
                x: 0.0,
                y: 0.0,
                width: 800.0,
                height: 600.0,
            }),
            ..Default::default()
        }
    }

    fn text_area() -> FieldSnapshot {
        field(42, "AXTextArea")
    }

    fn focused(field: FieldSnapshot, editable: bool) -> Focused {
        Focused { field, editable }
    }

    #[test]
    fn matching_needs_the_same_process_and_role() {
        let expected = text_area();
        assert!(expected.matches(&text_area()));
        assert!(!expected.matches(&field(43, "AXTextArea")));
        assert!(!expected.matches(&field(42, "AXTextField")));
    }

    #[test]
    fn identifiers_decide_when_both_sides_have_one() {
        let mut expected = text_area();
        expected.identifier = "message".into();
        let mut other = text_area();
        other.identifier = "subject".into();
        // Same place, different identifier: a different field.
        assert!(!expected.matches(&other));
        other.identifier = "message".into();
        other.frame.x += 500.0;
        // Same identifier, moved far: still the field.
        assert!(expected.matches(&other));
        other.identifier.clear();
        // One side without an identifier falls back on the frame.
        assert!(!expected.matches(&other));
    }

    #[test]
    fn titles_conflict_only_when_both_are_set_and_differ() {
        let mut expected = text_area();
        expected.title = "Reply".into();
        let mut other = text_area();
        assert!(expected.matches(&other));
        other.title = "Search".into();
        assert!(!expected.matches(&other));
    }

    #[test]
    fn the_frame_may_drift_48_points_but_not_49() {
        let expected = text_area();
        let mut other = text_area();
        other.frame.y += 48.0;
        other.frame.width -= 48.0;
        assert!(expected.matches(&other));
        other.frame.height += 49.0;
        assert!(!expected.matches(&other));
    }

    #[test]
    fn after_typing_the_field_frame_is_ignored_but_the_window_is_not() {
        let expected = text_area();
        let mut other = text_area();
        other.frame.y += 400.0;
        other.frame.height += 200.0;
        assert!(
            !expected.matches(&other),
            "the paste match would refuse this"
        );
        assert!(expected.same_field_after_typing(&other));
        other.window = Some(Rect {
            x: 900.0,
            ..expected.window.unwrap()
        });
        assert!(!expected.same_field_after_typing(&other));
        // Without a window on either side the paste match applies.
        let mut windowless = text_area();
        windowless.window = None;
        let mut moved = windowless.clone();
        moved.frame.y += 400.0;
        assert!(!windowless.same_field_after_typing(&moved));
    }

    #[test]
    fn terminals_are_the_swift_list_and_nothing_else() {
        assert!(is_terminal(Some("com.apple.Terminal")));
        assert!(is_terminal(Some("com.stablyai.orca")));
        assert!(!is_terminal(Some("com.apple.TextEdit")));
        assert!(!is_terminal(Some("")));
        assert!(!is_terminal(None));
    }

    #[test]
    fn secure_input_withholds_before_anything_else() {
        let expectation = Expectation::Field(text_area());
        let gate = gate(&expectation, Some(&focused(text_area(), true)), true);
        assert_eq!(gate, Gate::Withhold(Withheld::SecureInput));
        assert_eq!(Withheld::SecureInput.paste_reason(), Some("secure-input"));
        assert_eq!(Withheld::PasswordField.paste_reason(), None);
    }

    #[test]
    fn a_missing_snapshot_pastes_nothing() {
        let gate = gate(
            &Expectation::Missing,
            Some(&focused(text_area(), true)),
            false,
        );
        assert_eq!(gate, Gate::Withhold(Withheld::NoSnapshot));
        assert!(Withheld::NoSnapshot.reason(false).is_some());
        assert!(Withheld::TargetChanged.reason(false).is_none());
    }

    #[test]
    fn no_focus_copies_without_an_expectation_and_refuses_with_one() {
        assert_eq!(gate(&Expectation::Current, None, false), Gate::Copy);
        assert_eq!(
            gate(&Expectation::Field(text_area()), None, false),
            Gate::Withhold(Withheld::TargetChanged)
        );
    }

    #[test]
    fn a_password_field_is_refused_even_at_the_current_cursor() {
        let mut password = text_area();
        password.subrole = "AXSecureTextField".into();
        assert_eq!(
            gate(&Expectation::Current, Some(&focused(password, true)), false),
            Gate::Withhold(Withheld::PasswordField)
        );
    }

    #[test]
    fn the_expected_field_must_match_and_be_editable() {
        let expectation = Expectation::Field(text_area());
        assert_eq!(
            gate(
                &expectation,
                Some(&focused(field(42, "AXStaticText"), true)),
                false
            ),
            Gate::Withhold(Withheld::TargetChanged)
        );
        assert_eq!(
            gate(&expectation, Some(&focused(text_area(), false)), false),
            Gate::Withhold(Withheld::NotEditable)
        );
        // At the current cursor, editability is not checked (Swift pastes).
        assert!(matches!(
            gate(
                &Expectation::Current,
                Some(&focused(text_area(), false)),
                false
            ),
            Gate::Paste { .. }
        ));
    }

    #[test]
    fn a_terminal_paste_is_final_and_others_await_the_read_back() {
        let mut terminal = text_area();
        terminal.bundle_id = Some("com.googlecode.iterm2".into());
        let Gate::Paste {
            target,
            verification,
        } = gate(
            &Expectation::Field(terminal.clone()),
            Some(&focused(terminal.clone(), true)),
            false,
        )
        else {
            panic!("a matching terminal field pastes");
        };
        assert_eq!(target, terminal);
        assert_eq!(verification, Verification::Terminal);
        let Gate::Paste { verification, .. } = gate(
            &Expectation::Field(text_area()),
            Some(&focused(text_area(), true)),
            false,
        ) else {
            panic!("a matching field pastes");
        };
        assert_eq!(verification, Verification::None);
    }

    #[test]
    fn enter_needs_the_snapshot_and_the_same_field_after_typing() {
        let mut moved = text_area();
        moved.frame.y += 300.0;
        assert_eq!(
            key_gate(&Expectation::Field(text_area()), Some(&moved), false),
            KeyGate::Press
        );
        assert_eq!(
            key_gate(&Expectation::Current, Some(&text_area()), false),
            KeyGate::TargetChanged
        );
        assert_eq!(
            key_gate(&Expectation::Field(text_area()), None, false),
            KeyGate::TargetChanged
        );
        assert_eq!(
            key_gate(
                &Expectation::Field(text_area()),
                Some(&field(7, "AXTextArea")),
                false
            ),
            KeyGate::TargetChanged
        );
        let mut password = text_area();
        password.role = "AXSecureTextField".into();
        assert_eq!(
            key_gate(
                &Expectation::Field(password.clone()),
                Some(&password),
                false
            ),
            KeyGate::Sensitive
        );
        assert_eq!(
            key_gate(&Expectation::Field(text_area()), Some(&text_area()), true),
            KeyGate::Sensitive
        );
    }

    #[test]
    fn read_back_needs_a_change_that_contains_the_text() {
        assert!(read_back_confirms("hello ", Some(""), Some("hello")));
        assert!(read_back_confirms(
            "hello",
            Some("old"),
            Some("old hello\n")
        ));
        // The field already held the words: not this paste's doing.
        assert!(!read_back_confirms("hello", Some("hello"), Some("hello")));
        assert!(!read_back_confirms("hello", None, Some("hell")));
        assert!(!read_back_confirms("hello", None, None));
        assert!(!read_back_confirms("   ", None, Some("   ")));
    }
}
