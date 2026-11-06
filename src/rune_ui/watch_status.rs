//! The status capsule for Apple Watch recordings (macOS): one click-through
//! pill near the bottom of the screen that says what the recording is doing
//! (`watch::status` decides what and for how long). It never takes focus or
//! a click, so it cannot change the field it reports on, and it draws only
//! when the `watch` thread pushes a change: there is no polling.

use rune::{Anim, Cx, El, OverlayHandle, OverlayOptions, OverlaySpec, ScreenEdge, row, text};

use super::controls::{Paint, pulsing_dot, status_dot};
use crate::watch::status::{Capsule, Tone};

/// Room for the longest line (secure input) on two lines.
pub(crate) const SIZE: (f32, f32) = (520.0, 84.0);
/// The pill's own height for one line.
const PILL_H: f32 = 40.0;
/// Where the pill rests above the bottom edge of the screen.
const MARGIN: f32 = 28.0;

#[derive(Default)]
pub(crate) struct State {
    capsule: Option<Capsule>,
    chinese: bool,
}

/// Adds the capsule to the app and registers it with the watch service.
pub(crate) fn install<S: 'static>(app: &mut rune::App<S>) {
    let spec = OverlaySpec {
        name: "Hex watch status",
        size: SIZE,
        edge: ScreenEdge::Bottom,
        margin: MARGIN,
        align: rune::Align::Center,
        inset: 0.0,
        interactive: false,
        view,
    }
    .options(OverlayOptions::default())
    // Hidden, it holds no GPU surface.
    .release_gpu_when_hidden();
    let overlay = app.overlay(spec, State::default());
    crate::watch::status::watch(move |capsule| apply(&overlay, capsule));
}

fn apply(overlay: &OverlayHandle<State>, capsule: Option<Capsule>) {
    let handle = overlay.clone();
    match capsule {
        Some(capsule) => overlay.update(move |state, tx| {
            place(&handle);
            state.capsule = Some(capsule);
            state.chinese =
                crate::watch::status::prefers_chinese(&crate::watch::screen::preferred_languages());
            tx.instant();
            handle.show();
        }),
        None => overlay.update(move |state, tx| {
            state.capsule = None;
            tx.instant();
            handle.hide();
        }),
    }
}

/// Puts the capsule on the screen the pointer is on, or back at the bottom
/// of the main one, as the setting says. Read as each capsule shows.
#[cfg(target_os = "macos")]
fn place(overlay: &OverlayHandle<State>) {
    use rune::{DisplayTarget, OverlayPlacement};
    let pointer = crate::watch::settings::capsule_follows_pointer()
        .then(|| objc2::MainThreadMarker::new().and_then(crate::rune_hud::pointer_screen))
        .flatten();
    let Some((screen, primary_top)) = pointer else {
        overlay.set_placement(None);
        overlay.place(ScreenEdge::Bottom, MARGIN);
        return;
    };
    let size = (f64::from(SIZE.0), f64::from(SIZE.1));
    let (x, y) = crate::rune_hud::hud_placement::bottom_center(
        &screen,
        size,
        f64::from(MARGIN),
        primary_top,
    );
    overlay.set_display(DisplayTarget::Primary);
    overlay.set_placement(Some(OverlayPlacement::Rect {
        x,
        y,
        width: SIZE.0,
        height: SIZE.1,
    }));
}

#[cfg(not(target_os = "macos"))]
fn place(_: &OverlayHandle<State>) {}

fn view(state: &State, cx: &mut Cx<State>) -> El {
    let root = row()
        .key("watch-status")
        .w_full()
        .h_full()
        .center()
        .px(4.0)
        .hover_motion_within(Anim::INSTANT, Anim::INSTANT);
    let Some(capsule) = state.capsule else {
        return root;
    };
    let p = Paint::new(cx.system_dark(), cx.env().scale);
    let c = &p.k.c;
    let words = capsule.kind.text(state.chinese);
    let dot = match capsule.tone {
        Tone::Recording => pulsing_dot(p.danger()),
        Tone::Working => pulsing_dot(p.accent()),
        Tone::Done => status_dot(p.success()),
        Tone::Caution | Tone::Warning => status_dot(p.warning()),
    };
    root.child(
        row()
            .key("pill")
            .role("status")
            .label(words)
            .min_h(PILL_H)
            .max_w(SIZE.0 - 8.0)
            .px(16.0)
            .py(9.0)
            .gap(10.0)
            .center_y()
            .radius(PILL_H / 2.0)
            .fill(c.bg_base)
            .border(p.hair, c.border_base)
            .child(dot)
            .child(
                text(words.to_owned())
                    .key("words")
                    .font(p.k.ty.ui)
                    .color(p.text()),
            ),
    )
}

#[cfg(test)]
mod tests {
    use rune::Runtime;
    use rune::text::TextSystem;

    use super::*;
    use crate::watch::status::Kind;

    fn runtime(kind: Option<Kind>, tone: Tone, chinese: bool, dark: bool) -> Runtime<State> {
        let state = State {
            capsule: kind.map(|kind| Capsule { kind, tone }),
            chinese,
        };
        let mut rt = crate::rune_ui::offscreen::runtime(
            state,
            view,
            SIZE,
            rune::Env {
                system_dark: dark,
                scale: 2.0,
            },
            TextSystem::new(rune_theme_opencode::FONTS, rune_theme_opencode::FAMILIES),
        );
        rt.settle_all();
        rt.frame(0.0);
        rt
    }

    fn rect(rt: &Runtime<State>, key: &str) -> Option<[f32; 4]> {
        rt.elements()
            .into_iter()
            .find(|e| !e.exiting && e.key.as_deref() == Some(key))
            .map(|e| e.rect)
    }

    #[test]
    fn a_short_status_hugs_its_words_in_the_middle() {
        let rt = runtime(Some(Kind::Recording), Tone::Recording, false, false);
        let [x, _, w, _] = rect(&rt, "pill").expect("the pill draws");
        assert!(w < SIZE.0 / 2.0, "the pill hugs a short line: {w}");
        assert!((x + w / 2.0 - SIZE.0 / 2.0).abs() < 1.0, "centred: {x} {w}");
    }

    #[test]
    fn the_longest_status_stays_inside_the_overlay_in_both_languages() {
        for chinese in [false, true] {
            let rt = runtime(Some(Kind::SecureInput), Tone::Warning, chinese, true);
            let [x, y, w, h] = rect(&rt, "pill").expect("the pill draws");
            assert!(x >= 0.0 && x + w <= SIZE.0, "inside across: {x} {w}");
            assert!(y >= 0.0 && y + h <= SIZE.1, "inside down: {y} {h}");
        }
    }

    #[test]
    fn nothing_to_say_draws_nothing() {
        let rt = runtime(None, Tone::Done, false, false);
        assert!(rect(&rt, "pill").is_none());
    }
}
