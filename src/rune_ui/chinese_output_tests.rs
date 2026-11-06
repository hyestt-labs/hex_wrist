//! The Output pane's Chinese characters card: who sees it, and what a
//! choice saves. Runs on every OS (the window's own tests are Linux only).

use super::*;
use crate::dictation::chinese_output::ChineseOutput;
use rune::text::TextSystem;
use rune::{Env, Runtime};

fn runtime_for(state: Ui) -> Runtime<Ui> {
    let text = TextSystem::new(rune_theme_opencode::FONTS, rune_theme_opencode::FAMILIES);
    let mut rt = crate::rune_ui::offscreen::runtime(
        state,
        view,
        (980.0, 760.0),
        Env {
            system_dark: false,
            scale: 1.0,
        },
        text,
    );
    rt.settle_all();
    rt.frame(0.0);
    rt
}

fn scene(name: &str) -> Runtime<Ui> {
    let mut state = Ui::with(AppSettings::default(), Service::Fixture(Vec::new()));
    SCENES.iter().find(|(scene, _)| *scene == name).unwrap().1(&mut state);
    runtime_for(state)
}

fn tap(rt: &mut Runtime<Ui>, key: &str) {
    let (x, y) = rt.find(key).unwrap_or_else(|| panic!("no element {key}"));
    rt.pointer_down(x, y);
    rt.pointer_up(x, y);
    rt.settle_all();
    rt.frame(0.0);
}

fn element(rt: &Runtime<Ui>, key: &str) -> rune::ElementInfo {
    let suffix = format!("/{key}");
    rt.elements()
        .into_iter()
        .find(|e| !e.exiting && (e.key.as_deref() == Some(key) || e.path.ends_with(&suffix)))
        .unwrap_or_else(|| panic!("no element {key}"))
}

/// The Output pane for a speech model and a saved choice.
fn output(model: TranscriptionModelId, saved: Option<ChineseOutput>) -> Runtime<Ui> {
    let mut state = scenes::portal_fixture();
    output::fixture(&mut state);
    state.pane = Pane::Output;
    state.settings.transcription.model = model;
    state.settings.transcription.language = "en".into();
    state.settings.chinese_output = saved;
    runtime_for(state)
}

fn shown(rt: &Runtime<Ui>) -> bool {
    rt.find("chinese-output-row").is_some()
}

/// Opens the card's select and returns its options' labels.
fn open_choices(rt: &mut Runtime<Ui>) -> Vec<String> {
    tap(rt, "chinese-output-field-trigger");
    assert_eq!(rt.state.open, Some(Menu::Pick("chinese-output")));
    (0..4)
        .map(|index| {
            element(rt, &format!("chinese-output-item{index}"))
                .label
                .unwrap_or_else(|| panic!("item{index} has no name"))
        })
        .collect()
}

#[test]
fn an_english_model_with_nothing_saved_hides_the_card() {
    let english = TranscriptionModelId::ParakeetUnifiedEnglish;
    assert!(!shown(&output(english, None)));
    // The same pane with a model that writes Chinese shows it.
    assert!(shown(&output(TranscriptionModelId::BreezeAsr25, None)));
    assert!(shown(&scene("output-chinese")));
    // The scenes English users see keep it hidden.
    for scene in ["output", "output-trust", "output-empty"] {
        assert!(!shown(&self::scene(scene)), "{scene}");
    }
}

#[test]
fn a_saved_choice_keeps_the_card_even_with_an_english_model() {
    let english = TranscriptionModelId::ParakeetUnifiedEnglish;
    for saved in [
        ChineseOutput::Traditional,
        ChineseOutput::Simplified,
        ChineseOutput::AsHeard,
    ] {
        assert!(shown(&output(english, Some(saved))), "{saved:?}");
    }
    assert!(!shown(&output(english, None)));
}

#[test]
fn the_options_are_named_and_system_says_what_it_resolves_to() {
    let mut rt = output(TranscriptionModelId::BreezeAsr25, None);
    assert_eq!(
        open_choices(&mut rt),
        [
            "System (Traditional)",
            "Traditional",
            "Simplified",
            "As heard"
        ]
    );
    // The field names what it chooses and reads the current choice.
    let field = element(&rt, "chinese-output-field-trigger");
    assert_eq!(field.label.as_deref(), Some("Chinese characters"));
}

#[test]
fn choosing_a_script_saves_it_and_system_saves_none() {
    let mut rt = output(TranscriptionModelId::BreezeAsr25, None);
    for (index, expected) in [
        (1, Some(ChineseOutput::Traditional)),
        (2, Some(ChineseOutput::Simplified)),
        (3, Some(ChineseOutput::AsHeard)),
        (0, None),
    ] {
        open_choices(&mut rt);
        tap(&mut rt, &format!("chinese-output-item{index}"));
        assert_eq!(rt.state.open, None);
        assert_eq!(rt.state.settings.chinese_output, expected, "option {index}");
    }
}

#[test]
fn choosing_what_is_already_chosen_saves_nothing() {
    let mut rt = output(
        TranscriptionModelId::BreezeAsr25,
        Some(ChineseOutput::Simplified),
    );
    let saved = rt.state.saved;
    open_choices(&mut rt);
    tap(&mut rt, "chinese-output-item2");
    assert_eq!(rt.state.saved, saved);
    assert_eq!(
        rt.state.settings.chinese_output,
        Some(ChineseOutput::Simplified)
    );
    // Leaving a saved choice for System is a save, and it forgets the value.
    open_choices(&mut rt);
    tap(&mut rt, "chinese-output-item0");
    assert_eq!(rt.state.saved, saved + 1);
    assert_eq!(rt.state.settings.chinese_output, None);
    // System again: already following the system.
    open_choices(&mut rt);
    tap(&mut rt, "chinese-output-item0");
    assert_eq!(rt.state.saved, saved + 1);
}

#[test]
fn a_choice_changes_nothing_else_and_goes_through_the_settings_key() {
    let mut rt = output(TranscriptionModelId::BreezeAsr25, None);
    let before = rt.state.settings.clone();
    open_choices(&mut rt);
    tap(&mut rt, "chinese-output-item1");
    let mut expected = before;
    expected.chinese_output = Some(ChineseOutput::Traditional);
    assert_eq!(rt.state.settings, expected);
    // `hex settings chinese_output` reads what the window saved.
    let shown = crate::control::settings::get(&rt.state.settings, "chinese_output").unwrap();
    assert_eq!(shown[0].value, serde_json::json!("traditional"));
}
