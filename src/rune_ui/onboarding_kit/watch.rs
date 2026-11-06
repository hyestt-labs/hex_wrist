//! The Apple Watch page of the first run (macOS, with a running watch
//! service) and what the story knows about watches. It follows the model
//! step: the same six-digit code field and status line as Settings' Apple
//! Watch pane, the app's link, and Pair later. Pair later (or Continue, once
//! a watch is paired) goes on to the demo; neither records completion.

use rune::{El, TextEdit, Transition, col, row, text};
use rune_kit::{Button, ButtonVariant};

use super::super::controls::{Paint, PushSize};
use super::super::watch_pane::{self, Tone};
use super::flows::continue_button;
use super::landing::{arrives, heading};
use super::live::{Live, press_continue};
use super::stories::Onboard;
use crate::desktop::i18n::tr;

/// How long a resumed setup waits for the service's first snapshot to decide
/// between this page and the permissions.
pub(super) const RESUME_WAIT: f64 = 1.5;

/// What the story knows about watches, refreshed from the window each frame.
#[derive(Default)]
pub(crate) struct Page {
    /// The service has reported: this Mac can pair, so the page shows.
    available: bool,
    /// A watch that isn't waiting to be paired again; `None` until the
    /// service reports.
    pub(super) paired: Option<bool>,
    /// The Mac's microphone is allowed.
    mic_ready: bool,
    pub(super) code: TextEdit,
    status: Option<(String, Tone)>,
    busy: bool,
    /// What the tour was last told (`Input::Watch`).
    pub(super) told: Option<bool>,
    /// A resumed setup picks this page or the permissions once the service
    /// has reported, or at this time on the story's clock.
    pub(super) resume_until: Option<f64>,
}

/// What the window's pane knows, as the page reads it.
pub(super) struct Window {
    pub(super) available: bool,
    pub(super) paired: Option<bool>,
    pub(super) mic_ready: bool,
    pub(super) code: TextEdit,
    pub(super) status: Option<(String, Tone)>,
    pub(super) busy: bool,
}

impl Page {
    pub(super) fn offered(&self) -> bool {
        self.available
    }

    /// A paired watch dictates without the Mac's microphone.
    pub(super) fn microphone_deferred(&self) -> bool {
        self.paired == Some(true) && !self.mic_ready
    }

    pub(super) fn is_paired(&self) -> bool {
        self.paired == Some(true)
    }

    pub(super) fn refresh(&mut self, window: Window) {
        self.available = window.available;
        self.paired = window.paired;
        self.mic_ready = window.mic_ready;
        self.code = window.code;
        self.status = window.status;
        self.busy = window.busy;
    }
}

pub(super) fn page(p: Paint, s: &Onboard, live: &Live) -> El {
    let watch = &live.watch;
    let words = |key: &'static str, copy: &str, color| {
        text(copy.to_owned())
            .key(key)
            .font(p.k.ty.ui)
            .color(color)
            .align(rune::Align::Center)
            .wrap()
            .max_w(440.0)
    };
    let status = watch.status.as_ref().map(|(copy, tone)| {
        let color = match tone {
            Tone::Good => p.success(),
            Tone::Waiting => p.muted(),
            Tone::Bad => p.danger(),
        };
        words("watch-status", copy, color).transition(Transition::opacity())
    });
    let entry = col()
        .key("watch-entry")
        .gap(10.0)
        .items_center()
        .child(words(
            "watch-how",
            tr("On your watch, open Wristper › Pair a Mac and type the code it shows"),
            p.muted(),
        ))
        .child(watch_pane::code_input(
            &watch.code,
            watch.busy,
            "watch-onboard-code",
            p,
        ))
        .children(status)
        .child(
            Button::new(tr("Get the watch app"))
                .variant(ButtonVariant::Ghost)
                .on_press(watch_pane::open_app_page)
                .build_push(p.k)
                .key("watch-app"),
        );
    let action = if watch.is_paired() {
        continue_button(p, s)
    } else {
        Button::new(tr("Pair later"))
            .variant(ButtonVariant::Neutral)
            .on_press(press_continue)
            .build_push(p.k)
            .key("pair-later")
            .transition(arrives())
    };
    col()
        .key("watch")
        .gap(22.0)
        .items_center()
        .child(heading(
            p,
            "watch-head",
            tr("Set up your Apple Watch"),
            tr("Dictate from your wrist and Hex pastes the words on this Mac."),
        ))
        .child(entry)
        .child(row().key("action").h(32.0).center().child(action))
}
