//! Shortcut presses while the Microphone is not allowed.
//!
//! The listener runs without the permission (Apple Watch recordings need
//! none), and the dictation shortcut is often Option alone, so a start is
//! also Option+e, an Option-click, or an Option-drag. A refused start
//! therefore shows nothing. The press that means dictation is the one Hex
//! owns: the machine's chord breaker moment (held past the threshold,
//! Toggle's grace), or a lock that a double tap or a Toggle release made.
//! Only there does the audio owner ask or sound.

use super::{Hotkeys, Shortcut};
use crate::dictation::audio::DictationAudio;
use crate::suppression::{DictationHotkey, HotkeyAction};

/// `Start` was refused. When the missing permission is the cause, the machine
/// stays down until its press becomes Hex's own; a gesture with no such
/// moment (a double-tap lock) is the try itself. Any other refusal stands
/// the machine down at once.
pub(super) fn start_refused(hotkeys: &mut Hotkeys, shortcut: Shortcut, input: &DictationAudio) {
    let waiting = hotkeys
        .machine(shortcut)
        .is_some_and(DictationHotkey::owned_pending);
    if input.microphone_gated() && !waiting {
        input.press_owned();
    }
    if !(input.microphone_gated() && waiting) {
        hotkeys.suspend(shortcut);
    }
}

/// A press the closed microphone never started has nothing to end or show.
pub(super) fn ends_nothing(action: HotkeyAction, input: &DictationAudio) -> bool {
    matches!(
        action,
        HotkeyAction::Finish
            | HotkeyAction::Discard(_)
            | HotkeyAction::Cancel
            | HotkeyAction::CancelShortcut
    ) && input.microphone_gated()
        && !input.is_recording()
}

/// Hex owns `shortcut`'s press now. If nothing records for lack of the
/// permission, this is an intentional try; the machine stands down so it holds
/// no Toggle Escape.
pub(super) fn owned(hotkeys: &mut Hotkeys, shortcut: Shortcut, input: &DictationAudio) {
    if input.microphone_gated() && !input.is_recording() {
        input.press_owned();
        hotkeys.suspend(shortcut);
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::app_settings::{FUNCTION_KEY_MASK, OPTION_KEY_MASK, RuntimeHotkey, SHIFT_KEY_MASK};
    use crate::audio::CaptureInstant;
    use crate::desktop::onboarding::PermissionState;
    use crate::dictation::microphone_access::fakes::{Fake, Opens, open_audio};
    use crate::suppression::{DictationHotkey, InputEvent};

    const E_KEY: u16 = 14;
    const F5_KEY: u16 = 96;

    /// The shortcut bindings: Option alone (a bare modifier, which passes
    /// through), and the three that do not: a function key, a letter with a
    /// modifier, and Fn (Globe) alone.
    #[derive(Clone, Copy, Debug)]
    enum Binding {
        Option,
        F5,
        OptionE,
        Globe,
    }

    impl Binding {
        fn hotkey(self) -> RuntimeHotkey {
            let option = crate::app_settings::HotkeyModifiers::option();
            let none = crate::app_settings::HotkeyModifiers::default();
            let function = crate::app_settings::HotkeyModifiers {
                function: true,
                ..none
            };
            let (modifiers, key_code) = match self {
                Self::Option => (option, None),
                Self::F5 => (none, Some(F5_KEY)),
                Self::OptionE => (option, Some(E_KEY)),
                Self::Globe => (function, None),
            };
            RuntimeHotkey {
                modifiers,
                key_code,
            }
        }

        fn press(self) -> Vec<InputEvent> {
            let key = |code, flags| InputEvent::Key {
                code,
                down: true,
                flags,
            };
            match self {
                Self::Option => vec![InputEvent::Flags(OPTION_KEY_MASK)],
                Self::F5 => vec![key(F5_KEY, 0)],
                Self::OptionE => vec![
                    InputEvent::Flags(OPTION_KEY_MASK),
                    key(E_KEY, OPTION_KEY_MASK),
                ],
                Self::Globe => vec![InputEvent::Flags(FUNCTION_KEY_MASK)],
            }
        }

        fn release(self) -> Vec<InputEvent> {
            let key = |code, flags| InputEvent::Key {
                code,
                down: false,
                flags,
            };
            match self {
                Self::Option | Self::Globe => vec![InputEvent::Flags(0)],
                Self::F5 => vec![key(F5_KEY, 0)],
                Self::OptionE => vec![key(E_KEY, OPTION_KEY_MASK), InputEvent::Flags(0)],
            }
        }
    }

    /// The real shortcut machine, the real audio owner on a fake permission,
    /// and the three calls `recognition::listen` makes around them. The HID
    /// state `break_chord` reads (to post the 0x44 marker) is the only part not
    /// driven: the due moment is the machine's own `owned_now`.
    struct Rig {
        hotkeys: Hotkeys,
        audio: DictationAudio,
        permission: Fake,
        opens: Opens,
        base: CaptureInstant,
        binding: Binding,
        /// Actions that would have reached the capture code: none may.
        leaked: Vec<HotkeyAction>,
    }

    impl Rig {
        fn new(state: PermissionState, toggle: bool) -> Self {
            Self::with_binding(state, toggle, Binding::Option)
        }

        fn with_binding(state: PermissionState, toggle: bool, binding: Binding) -> Self {
            let base = CaptureInstant::from_nanos(60_000_000_000);
            let mut dictation = DictationHotkey::new(base, false, binding.hotkey());
            dictation.set_toggle(toggle);
            let permission = Fake::new(state);
            let opens = Opens::default();
            Self {
                hotkeys: Hotkeys {
                    dictation,
                    second: None,
                    raw: None,
                    raw_capture: false,
                },
                audio: open_audio(&permission, &opens, false),
                permission,
                opens,
                base,
                binding,
                leaked: Vec::new(),
            }
        }

        fn press(&mut self, ms: u64) {
            for event in self.binding.press() {
                self.send(event, ms);
            }
        }

        fn release(&mut self, ms: u64) {
            for event in self.binding.release() {
                self.send(event, ms);
            }
        }

        fn at(&self, ms: u64) -> CaptureInstant {
            self.base + Duration::from_millis(ms)
        }

        /// Hex's owned moments due by `ms`, as the loop's `break_chords` finds them.
        fn tick(&mut self, ms: u64) {
            let now = self.at(ms);
            let machine = &mut self.hotkeys.dictation;
            if machine.owned_now(now) {
                owned(&mut self.hotkeys, Shortcut::Dictation, &self.audio);
            }
        }

        fn send(&mut self, event: InputEvent, ms: u64) {
            self.tick(ms);
            let at = self.at(ms);
            for (shortcut, action) in self.hotkeys.process(event, at) {
                if action == HotkeyAction::Start {
                    if !self.audio.start(at).unwrap() {
                        start_refused(&mut self.hotkeys, shortcut, &self.audio);
                    }
                } else if !ends_nothing(action, &self.audio) {
                    self.leaked.push(action);
                }
            }
        }

        fn option(&mut self, ms: u64) {
            self.send(InputEvent::Flags(OPTION_KEY_MASK), ms);
        }

        /// Everything the owner has been told is handled, then what the
        /// person could notice: prompts, error tones, Settings windows.
        fn noticed(&self) -> (usize, usize, usize) {
            self.audio.invalidate_recognition().unwrap();
            self.permission.noticed()
        }
    }

    const REFUSED: [PermissionState; 3] = [
        PermissionState::NeedsRequest,
        PermissionState::NeedsSettings,
        PermissionState::NeedsRestart,
    ];

    /// Watch-only users hold Option for other reasons. None of these may
    /// prompt, sound, or open a window, and nothing may open the microphone.
    /// `gap` is when the second event comes: inside the hold threshold, and
    /// inside Toggle's 40 ms grace (any later Toggle press is the person's try).
    #[test]
    fn ordinary_option_use_is_silent_without_the_microphone() {
        type Script = fn(&mut Rig, u64);
        let gestures: [(&str, Script); 4] = [
            ("Option+e", |rig, gap| {
                rig.option(0);
                rig.send(
                    InputEvent::Key {
                        code: E_KEY,
                        down: true,
                        flags: OPTION_KEY_MASK,
                    },
                    gap,
                );
            }),
            ("Option-click", |rig, gap| {
                rig.option(0);
                rig.send(InputEvent::MouseDown, gap);
            }),
            ("Option-drag", |rig, gap| {
                rig.option(0);
                rig.send(InputEvent::MouseDown, gap);
                rig.send(InputEvent::Flags(0), 900);
            }),
            ("chord before the threshold", |rig, gap| {
                rig.option(0);
                rig.send(InputEvent::Flags(OPTION_KEY_MASK | SHIFT_KEY_MASK), gap);
                rig.send(InputEvent::Flags(0), gap + 50);
            }),
        ];
        for state in REFUSED {
            for (toggle, gap) in [(false, 250), (true, 20)] {
                for (name, gesture) in gestures {
                    let mut rig = Rig::new(state, toggle);
                    gesture(&mut rig, gap);
                    rig.tick(2_000);
                    let label = format!("{name}, {state:?}, toggle {toggle}");
                    assert_eq!(rig.noticed(), (0, 0, 0), "{label}");
                    assert_eq!(rig.opens.count(), 0, "{label}");
                    assert!(rig.leaked.is_empty(), "{label}: {:?}", rig.leaked);
                    assert!(!rig.audio.is_recording(), "{label}");
                }
            }
        }
    }

    /// In Hold mode a short tap is nothing; a Toggle tap is the gesture.
    #[test]
    fn a_short_tap_is_silent_in_hold_mode_and_a_try_in_toggle_mode() {
        for state in REFUSED {
            let mut rig = Rig::new(state, false);
            rig.option(0);
            rig.send(InputEvent::Flags(0), 120);
            rig.tick(2_000);
            assert_eq!(rig.noticed(), (0, 0, 0), "{state:?}");
            assert!(rig.leaked.is_empty());
        }
        let mut rig = Rig::new(PermissionState::NeedsSettings, true);
        rig.option(0);
        rig.send(InputEvent::Flags(0), 120);
        rig.tick(2_000);
        assert_eq!(rig.noticed(), (0, 1, 1));
    }

    #[test]
    fn a_hold_past_the_threshold_asks_exactly_once() {
        let mut rig = Rig::new(PermissionState::NeedsRequest, false);
        rig.option(0);
        rig.tick(299);
        assert_eq!(rig.noticed(), (0, 0, 0), "not yet the person's try");
        rig.tick(300);
        assert_eq!(rig.noticed(), (1, 0, 0), "the macOS prompt");
        assert!(!rig.hotkeys.any_recording(), "the machine stands down");
        rig.send(InputEvent::Flags(0), 1_000);
        rig.tick(2_000);
        assert_eq!(rig.noticed(), (1, 0, 0), "release and waiting add nothing");
        assert!(rig.leaked.is_empty());
        assert_eq!(rig.opens.count(), 0);
    }

    #[test]
    fn toggles_start_after_its_grace_asks_exactly_once() {
        let mut rig = Rig::new(PermissionState::NeedsRequest, true);
        rig.option(0);
        rig.tick(39);
        assert_eq!(rig.noticed(), (0, 0, 0));
        rig.tick(40);
        assert_eq!(rig.noticed(), (1, 0, 0));
        assert!(!rig.hotkeys.any_recording(), "Toggle holds no Escape");
        rig.send(InputEvent::Flags(0), 100);
        rig.tick(5_000);
        assert_eq!(rig.noticed(), (1, 0, 0));
        assert!(rig.leaked.is_empty());
    }

    #[test]
    fn a_refused_microphone_sounds_on_each_long_press_and_opens_settings_once() {
        let mut rig = Rig::new(PermissionState::NeedsSettings, false);
        for round in 1..=3_u64 {
            let start = round * 10_000;
            rig.option(start);
            rig.tick(start + 300);
            assert_eq!(rig.noticed(), (0, round as usize, 1), "press {round}");
            rig.send(InputEvent::Flags(0), start + 900);
        }
        assert!(rig.leaked.is_empty());
        assert_eq!(rig.opens.count(), 0);
    }

    /// A double-tap lock has no owned moment; the gesture is the try.
    #[test]
    fn a_double_tap_lock_is_an_intentional_try() {
        let mut rig = Rig::new(PermissionState::NeedsSettings, false);
        rig.hotkeys.dictation = DictationHotkey::new(
            rig.base,
            true,
            RuntimeHotkey {
                modifiers: crate::app_settings::HotkeyModifiers::option(),
                key_code: None,
            },
        );
        rig.option(0);
        rig.send(InputEvent::Flags(0), 50);
        rig.option(100);
        rig.send(InputEvent::Flags(0), 150);
        assert!(
            rig.hotkeys.dictation.is_locked(),
            "the second tap locked it"
        );
        rig.tick(160);
        assert_eq!(rig.noticed(), (0, 1, 1));
        assert!(!rig.hotkeys.any_recording());
    }
    const OTHER_BINDINGS: [Binding; 3] = [Binding::F5, Binding::OptionE, Binding::Globe];

    /// A function key, a letter with a modifier, and Fn have no chord breaker,
    /// so the owned moment cannot lean on it: a short tap or an early release
    /// is nobody's try in Hold mode. In Toggle mode a tap of the binding is the
    /// gesture itself, so it is one try (and nothing before its 40 ms grace is
    /// asked of a press still held).
    #[test]
    fn other_bindings_are_silent_until_the_press_is_owned() {
        for binding in OTHER_BINDINGS {
            for state in REFUSED {
                let mut rig = Rig::with_binding(state, false, binding);
                rig.press(0);
                rig.release(250);
                rig.tick(2_000);
                let label = format!("{binding:?}, {state:?}");
                assert_eq!(rig.noticed(), (0, 0, 0), "{label}");
                assert_eq!(rig.opens.count(), 0, "{label}");
                assert!(rig.leaked.is_empty(), "{label}: {:?}", rig.leaked);
                assert!(!rig.hotkeys.any_recording(), "{label}");

                let mut held = Rig::with_binding(state, true, binding);
                held.press(0);
                held.tick(39);
                assert_eq!(
                    held.noticed(),
                    (0, 0, 0),
                    "{label}: toggle, before the grace"
                );
            }
        }
    }

    #[test]
    fn other_bindings_ask_exactly_once_for_a_hold_past_the_threshold() {
        for binding in OTHER_BINDINGS {
            let mut rig = Rig::with_binding(PermissionState::NeedsRequest, false, binding);
            rig.press(0);
            rig.tick(299);
            assert_eq!(rig.noticed(), (0, 0, 0), "{binding:?}: not yet");
            rig.tick(300);
            assert_eq!(rig.noticed(), (1, 0, 0), "{binding:?}: the prompt");
            assert!(!rig.hotkeys.any_recording(), "{binding:?}");
            rig.release(1_000);
            rig.tick(2_000);
            assert_eq!(rig.noticed(), (1, 0, 0), "{binding:?}: nothing more");
            assert!(rig.leaked.is_empty(), "{binding:?}");

            let mut toggle = Rig::with_binding(PermissionState::NeedsRequest, true, binding);
            toggle.press(0);
            toggle.tick(39);
            assert_eq!(toggle.noticed(), (0, 0, 0), "{binding:?} toggle");
            toggle.tick(40);
            assert_eq!(toggle.noticed(), (1, 0, 0), "{binding:?} toggle");
        }
    }

    #[test]
    fn other_bindings_sound_on_each_long_press_after_a_refusal() {
        for binding in OTHER_BINDINGS {
            let mut rig = Rig::with_binding(PermissionState::NeedsSettings, false, binding);
            for round in 1..=3_u64 {
                let start = round * 10_000;
                rig.press(start);
                rig.tick(start + 300);
                assert_eq!(rig.noticed(), (0, round as usize, 1), "{binding:?} {round}");
                rig.release(start + 900);
            }
            assert!(rig.leaked.is_empty(), "{binding:?}");
            assert_eq!(rig.opens.count(), 0);
        }
    }
}
