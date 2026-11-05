//! A scripted `Screen` for tests: no Accessibility, no key events.

use std::collections::VecDeque;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::watch::desktop::Screen;
use crate::watch::field::{FieldSnapshot, Focused, Rect};

#[derive(Default)]
pub(crate) struct FakeScreen {
    pub(crate) trusted: AtomicBool,
    pub(crate) secure: AtomicBool,
    pub(crate) fail_keys: AtomicBool,
    pub(crate) focus: Mutex<Option<Focused>>,
    /// What `value` answers, one entry per call; the last repeats.
    pub(crate) values: Mutex<VecDeque<Option<String>>>,
    /// The keys pressed: `return` or `undo`.
    pub(crate) keys: Mutex<Vec<&'static str>>,
}

impl FakeScreen {
    pub(crate) fn trusted_with(focus: Option<Focused>) -> Self {
        let screen = Self::default();
        screen.trusted.store(true, Ordering::SeqCst);
        *screen.focus.lock().unwrap() = focus;
        screen
    }

    pub(crate) fn keys(&self) -> Vec<&'static str> {
        self.keys.lock().unwrap().clone()
    }

    pub(crate) fn set_focus(&self, focus: Option<Focused>) {
        *self.focus.lock().unwrap() = focus;
    }

    pub(crate) fn script_values(&self, values: impl IntoIterator<Item = Option<&'static str>>) {
        *self.values.lock().unwrap() = values
            .into_iter()
            .map(|value| value.map(str::to_owned))
            .collect();
    }
}

impl Screen for FakeScreen {
    fn trusted(&self) -> bool {
        self.trusted.load(Ordering::SeqCst)
    }

    fn secure_input(&self) -> bool {
        self.secure.load(Ordering::SeqCst)
    }

    fn focused(&self) -> Option<Focused> {
        self.focus.lock().unwrap().clone()
    }

    fn value(&self, _pid: i32) -> Option<String> {
        let mut values = self.values.lock().unwrap();
        if values.len() > 1 {
            values.pop_front().flatten()
        } else {
            values.front().cloned().flatten()
        }
    }

    fn press_return(&self) -> Result<(), String> {
        if self.fail_keys.load(Ordering::SeqCst) {
            return Err("no event source".to_owned());
        }
        self.keys.lock().unwrap().push("return");
        Ok(())
    }

    fn press_undo(&self) -> Result<(), String> {
        if self.fail_keys.load(Ordering::SeqCst) {
            return Err("no event source".to_owned());
        }
        self.keys.lock().unwrap().push("undo");
        Ok(())
    }
}

/// A text area in a 800x600 window.
pub(crate) fn text_area(pid: i32) -> FieldSnapshot {
    FieldSnapshot {
        pid,
        role: "AXTextArea".into(),
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
        bundle_id: Some("com.apple.TextEdit".into()),
        app_name: Some("TextEdit".into()),
        ..Default::default()
    }
}

pub(crate) fn editable(field: FieldSnapshot) -> Focused {
    Focused {
        field,
        editable: true,
    }
}
