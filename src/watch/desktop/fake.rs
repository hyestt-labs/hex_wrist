//! A scripted `Screen` for tests: no Accessibility, no key events.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use crate::watch::desktop::Screen;
use crate::watch::field::{Element, ElementRef, FieldSnapshot, Focused, Rect};

#[derive(Default)]
pub(crate) struct FakeScreen {
    pub(crate) trusted: AtomicBool,
    pub(crate) secure: AtomicBool,
    pub(crate) fail_keys: AtomicBool,
    pub(crate) focus: Mutex<Option<Focused>>,
    /// Answers for the next `focused` calls, one each, before `focus`
    /// takes over: a user who switches apps between two reads.
    pub(crate) focus_script: Mutex<VecDeque<Option<Focused>>>,
    /// How long each `focused` read takes.
    pub(crate) focus_delay: Mutex<std::time::Duration>,
    /// The element every focused field without its own is read through.
    pub(crate) element: Arc<FakeElement>,
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
        self.element.script(values);
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
        let mut focused = self.read_focus()?;
        if focused.element.is_none() {
            focused.element = Some(ElementRef(self.element.clone()));
        }
        Some(focused)
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

impl FakeScreen {
    fn read_focus(&self) -> Option<Focused> {
        let delay = *self.focus_delay.lock().unwrap();
        if !delay.is_zero() {
            std::thread::sleep(delay);
        }
        if let Some(scripted) = self.focus_script.lock().unwrap().pop_front() {
            return scripted;
        }
        self.focus.lock().unwrap().clone()
    }
}

/// A field's text, scripted: one answer per read, the last repeating.
#[derive(Default)]
pub(crate) struct FakeElement {
    values: Mutex<VecDeque<Option<String>>>,
}

impl FakeElement {
    pub(crate) fn scripted(values: impl IntoIterator<Item = Option<&'static str>>) -> Arc<Self> {
        let element = Arc::new(Self::default());
        element.script(values);
        element
    }

    fn script(&self, values: impl IntoIterator<Item = Option<&'static str>>) {
        *self.values.lock().unwrap() = values
            .into_iter()
            .map(|value| value.map(str::to_owned))
            .collect();
    }
}

impl Element for FakeElement {
    fn value(&self) -> Option<String> {
        let mut values = self.values.lock().unwrap();
        if values.len() > 1 {
            values.pop_front().flatten()
        } else {
            values.front().cloned().flatten()
        }
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
        element: None,
        editable: true,
    }
}
