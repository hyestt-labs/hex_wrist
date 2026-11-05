use super::*;
use crate::watch::recording::PasteMarks;

fn record(state: State, paste: PasteMarks) -> Record {
    Record {
        id: "6f9619ff-8b86-d011-b42d-00c04fc964ff".into(),
        state,
        paste,
        language: "en".into(),
        text: String::new(),
        raw_text: None,
        updated_at: String::new(),
        completed_at: None,
        transcribe_ms: None,
        paste_ms: None,
        receive_to_paste_ms: None,
        reason: None,
        reset: None,
        paste_reason: None,
        error: None,
        truncated: false,
        owner: None,
    }
}

fn shown(state: State, paste: PasteMarks) -> (Kind, Tone, f64) {
    let display = display(&record(state, paste), true).expect("announced");
    (
        display.capsule.kind,
        display.capsule.tone,
        display.hide_after.as_secs_f64(),
    )
}

fn plain() -> PasteMarks {
    PasteMarks::default()
}

#[test]
fn a_recording_shows_for_at_most_three_minutes() {
    assert_eq!(
        shown(State::AwaitingAudio, plain()),
        (Kind::Recording, Tone::Recording, 180.0)
    );
    assert_eq!(
        shown(State::Transcribing, plain()),
        (Kind::Transcribing, Tone::Working, 60.0)
    );
}

#[test]
fn a_pasted_recording_reads_inserted_and_a_terminal_says_so() {
    let read_back = PasteMarks::sent(Verification::ReadBack, false);
    assert_eq!(
        shown(State::Pasted, read_back),
        (Kind::Inserted, Tone::Done, 1.2)
    );
    let terminal = PasteMarks::sent(Verification::Terminal, false);
    assert_eq!(shown(State::Pasted, terminal).0, Kind::InsertedInTerminal);
}

#[test]
fn pressing_enter_changes_the_words() {
    let entered = PasteMarks::sent(Verification::ReadBack, true);
    assert_eq!(shown(State::Pasted, entered).0, Kind::InsertedAndEntered);
    let terminal = PasteMarks::sent(Verification::Terminal, true);
    assert_eq!(
        shown(State::Pasted, terminal).0,
        Kind::InsertedInTerminalAndEntered
    );
    let unconfirmed = PasteMarks::sent(Verification::None, true);
    assert_eq!(
        shown(State::Unverified, unconfirmed).0,
        Kind::UnconfirmedAndEntered
    );
}

#[test]
fn an_unconfirmed_paste_is_a_caution_not_a_success() {
    let sent = PasteMarks::sent(Verification::None, false);
    assert_eq!(
        shown(State::Unverified, sent),
        (Kind::Unconfirmed, Tone::Caution, 2.5)
    );
}

#[test]
fn every_refusal_is_a_warning() {
    for (state, kind) in [
        (State::TargetChanged, Kind::TargetChanged),
        (State::NotEditable, Kind::NotEditable),
        (State::Suspect, Kind::Suspect),
    ] {
        let (found, tone, _) = shown(state, plain());
        assert_eq!((found, tone), (kind, Tone::Warning));
    }
}

#[test]
fn secure_input_stays_longer_than_a_password_field() {
    let mut secure = record(State::Sensitive, plain());
    secure.paste_reason = Some("secure-input".into());
    let secure = display(&secure, true).unwrap();
    assert_eq!(secure.capsule.kind, Kind::SecureInput);
    assert_eq!(secure.hide_after, Duration::from_secs(5));
    let password = display(&record(State::Sensitive, plain()), true).unwrap();
    assert_eq!(password.capsule.kind, Kind::PasswordField);
    assert_eq!(password.hide_after, Duration::from_millis(2500));
}

#[test]
fn a_failure_names_missing_accessibility_when_that_is_the_cause() {
    let failed = record(State::Failed, plain());
    assert_eq!(display(&failed, true).unwrap().capsule.kind, Kind::Failed);
    assert_eq!(
        display(&failed, false).unwrap().capsule.kind,
        Kind::NoAccessibility
    );
}

#[test]
fn states_the_mac_does_not_announce_show_nothing() {
    assert!(display(&record(State::Saved, plain()), true).is_none());
    assert!(display(&record(State::Segmented, plain()), true).is_none());
}

#[test]
fn the_first_of_chinese_or_english_in_the_system_list_wins() {
    let list = |tags: &[&str]| tags.iter().map(|t| (*t).to_owned()).collect::<Vec<_>>();
    assert!(prefers_chinese(&list(&["zh-Hant-TW", "en-US"])));
    assert!(prefers_chinese(&list(&["fr-FR", "yue-Hant", "en"])));
    assert!(!prefers_chinese(&list(&["en-US", "zh-Hant"])));
    assert!(!prefers_chinese(&list(&["de-DE"])));
    assert!(!prefers_chinese(&[]));
}

#[test]
fn both_languages_have_words_for_every_capsule() {
    for kind in [
        Kind::Recording,
        Kind::Transcribing,
        Kind::Inserted,
        Kind::InsertedInTerminal,
        Kind::InsertedAndEntered,
        Kind::InsertedInTerminalAndEntered,
        Kind::Unconfirmed,
        Kind::UnconfirmedAndEntered,
        Kind::Undone,
        Kind::Copied,
        Kind::TargetChanged,
        Kind::NotEditable,
        Kind::SecureInput,
        Kind::PasswordField,
        Kind::Suspect,
        Kind::Failed,
        Kind::NoAccessibility,
    ] {
        assert!(!kind.text(true).is_empty() && !kind.text(false).is_empty());
        assert_ne!(kind.text(true), kind.text(false));
    }
}
