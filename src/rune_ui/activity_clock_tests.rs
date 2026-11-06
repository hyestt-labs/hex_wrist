//! The Activity scene is a fixture at a frozen moment, and the real pane
//! still reads the real clock. Rendered through the scene table and the
//! window's `view`, as the goldens and `--snap` draw it.

use rune::render::Gpu;
use rune::text::TextSystem;
use rune::{Env, Runtime};

use super::*;

const SIZE: (f32, f32) = (980.0, 760.0);
const SCALE: f32 = 2.0;

fn runtime_for(state: Ui) -> Runtime<Ui> {
    let env = Env {
        system_dark: false,
        scale: SCALE,
    };
    let text = TextSystem::new(rune_theme_opencode::FONTS, rune_theme_opencode::FAMILIES);
    let mut rt = offscreen::runtime(state, view, SIZE, env, text);
    rt.poll = Some(poll);
    rt.on_key = Some(on_key);
    rt.settle_all();
    rt.frame(0.0);
    rt
}

fn scene_state() -> Ui {
    let mut state = Ui::with(AppSettings::default(), Service::Fixture(Vec::new()));
    let (_, apply) = SCENES
        .iter()
        .find(|(name, _)| *name == "mac-activity")
        .expect("the mac-activity scene");
    apply(&mut state);
    state
}

fn texts(rt: &Runtime<Ui>) -> Vec<String> {
    rt.elements()
        .into_iter()
        .filter(|e| !e.path.contains("/faces/size"))
        .filter_map(|e| e.text)
        .collect()
}

/// The scene's pixels, or `None` without a GPU adapter.
fn pixels(mut rt: Runtime<Ui>) -> Option<Vec<u8>> {
    let gpu = Gpu::try_new(Gpu::instance(), None).ok()?;
    let mut renderer = offscreen::renderer(&gpu.device);
    let (width, height) = ((SIZE.0 * SCALE) as u32, (SIZE.1 * SCALE) as u32);
    let target = offscreen::target(&gpu.device, width, height);
    renderer.prepare(
        &gpu.device,
        &gpu.queue,
        &mut rt.text.atlas,
        &rt.instances,
        (width, height),
    );
    renderer.upload_images(&gpu.device, &gpu.queue, &mut rt.images);
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    rune::compose::Compositor::render(
        &mut WindowCompositor::default(),
        &mut rune::compose::Frame {
            device: &gpu.device,
            queue: &gpu.queue,
            encoder: &mut encoder,
            target: &target.view,
            format: offscreen::format(),
            size: (width, height),
            dt: 0.0,
            update_ms: 0.0,
            rt: &mut rt,
            ui: &mut renderer,
        },
    );
    target.copy(&mut encoder);
    renderer.submit(&gpu.queue, encoder);
    Some(target.read(&gpu.device))
}

/// The clock moving between two renders changes neither the scene's text
/// nor its pixels.
#[test]
fn activity_scene_ignores_the_clock() {
    let first_text = texts(&runtime_for(scene_state()));
    let first_pixels = pixels(runtime_for(scene_state()));
    // Past a second boundary, so a live "4s ago" would read "5s ago" and a
    // live timestamp would differ.
    std::thread::sleep(std::time::Duration::from_millis(1_100));
    let second_text = texts(&runtime_for(scene_state()));
    let second_pixels = pixels(runtime_for(scene_state()));
    assert_eq!(
        first_text, second_text,
        "the scene's text followed the clock"
    );
    assert!(
        first_text.iter().any(|t| t == "4s ago"),
        "the newest fixture event reads 4s ago: {first_text:?}"
    );
    match (first_pixels, second_pixels) {
        (Some(first), Some(second)) => {
            assert!(first == second, "the scene's pixels followed the clock");
        }
        _ => assert!(
            std::env::var_os("HEX_REQUIRE_GPU").is_none(),
            "no GPU adapter, and HEX_REQUIRE_GPU is set"
        ),
    }
}

/// The real Activity pane still reads the real clock: events read from a
/// log are aged against now, whatever the fixture froze.
#[test]
fn activity_pane_ages_real_events_against_now() {
    use crate::desktop::events::{DictationPhase, EventLog, VoiceEvent, now_ms};
    let now = now_ms();
    // Named by the moment, so a run never meets an earlier run's log.
    let path = std::env::temp_dir()
        .join(format!("hex-rune-activity-{now}-{}", std::process::id()))
        .join("live.ndjson");
    let log = EventLog::create(&path).unwrap();
    for ago_ms in [3 * 3_600_000, 2 * 86_400_000] {
        log.emit(&VoiceEvent::Dictation {
            timestamp_ms: now - ago_ms,
            phase: DictationPhase::Started,
            text: String::new(),
            processing: None,
            reason: None,
        })
        .unwrap();
    }
    log.flush().unwrap();
    let mut state = scene_state();
    state.activity = Default::default();
    assert!(state.activity.poll(Some(&path), true));
    let shown = texts(&runtime_for(state));
    assert!(shown.iter().any(|t| t == "3h ago"), "{shown:?}");
    assert!(shown.iter().any(|t| t == "2d ago"), "{shown:?}");
    assert!(!shown.iter().any(|t| t == "4s ago"), "{shown:?}");
}
