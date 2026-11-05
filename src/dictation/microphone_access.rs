//! Who may open the Mac microphone for dictation.
//!
//! The listener starts without the Microphone permission, so Apple Watch
//! recordings transcribe and paste on a Mac that never granted it. The audio
//! owner therefore holds the input closed until macOS says Ready, and a
//! dictation start records nothing and says nothing: Option+e or a click
//! starts the shortcut machine too. Only a press Hex owns (held past the
//! hold threshold, Toggle's grace, `hex dictation`, `/dictations`) is an
//! intentional try, and only then does [`MicrophoneAccess::intentional`] ask.

use std::sync::atomic::{AtomicBool, Ordering};

use crate::desktop::onboarding::{self, PermissionState};
use crate::feedback::{self, Tone};

/// Set when an intentional try found the microphone refused and the Settings
/// window has not opened yet this launch; the main thread's pump takes it.
static SETTINGS_REQUESTED: AtomicBool = AtomicBool::new(false);

/// Whether an intentional try asked for the Settings window since the last
/// call.
pub(crate) fn take_settings_request() -> bool {
    SETTINGS_REQUESTED.swap(false, Ordering::AcqRel)
}

/// What the owner reads and does about the permission. Tests replace it.
pub(crate) trait Permission: Send {
    fn state(&self) -> PermissionState;
    /// macOS's own prompt. It opens no stream.
    fn ask(&self);
    /// The error tone: this try cannot record.
    fn tone(&self);
    /// Ask the main thread to open Settings.
    fn show_settings(&self);
}

/// The running Mac's permission. `HEX_AUDIO_FIXTURE` reads as granted.
pub(crate) struct System;

impl Permission for System {
    fn state(&self) -> PermissionState {
        onboarding::microphone_state()
    }

    fn ask(&self) {
        onboarding::request_microphone();
    }

    fn tone(&self) {
        feedback::play(Tone::Error);
    }

    fn show_settings(&self) {
        SETTINGS_REQUESTED.store(true, Ordering::Release);
    }
}

/// The owner's view of the microphone permission and of what the person
/// asked for (`Release microphone while idle`).
pub(crate) struct MicrophoneAccess {
    permission: Box<dyn Permission>,
    wanted: bool,
    /// macOS's prompt was shown this launch.
    asked: bool,
    /// The Settings window was requested this launch: at most once.
    window_requested: bool,
}

impl MicrophoneAccess {
    pub(crate) fn system(wanted: bool) -> Self {
        Self::with_permission(Box::new(System), wanted)
    }

    pub(crate) fn with_permission(permission: Box<dyn Permission>, wanted: bool) -> Self {
        Self {
            permission,
            wanted,
            asked: false,
            window_requested: false,
        }
    }

    pub(crate) fn set_wanted(&mut self, wanted: bool) {
        self.wanted = wanted;
    }

    pub(crate) fn granted(&self) -> bool {
        self.permission.state() == PermissionState::Ready
    }

    /// What the owner runs: a missing permission acts as the released
    /// microphone, which opens only when a capture asks.
    pub(crate) fn release_while_idle(&self) -> bool {
        self.wanted || !self.granted()
    }

    /// Whether a dictation start may open the microphone. A refusal is
    /// silent: the start may be a chord, a click, or typing.
    pub(crate) fn admit(&self) -> bool {
        self.granted()
    }

    /// The person meant to dictate and cannot. macOS's prompt the first time;
    /// after a refusal (or while a grant waits for a restart) the error tone
    /// on every try, and the Settings window once per launch.
    pub(crate) fn intentional(&mut self) {
        match self.permission.state() {
            PermissionState::Ready => {}
            PermissionState::NeedsRequest => {
                if !std::mem::replace(&mut self.asked, true) {
                    tracing::info!("dictation tried: asking macOS for the microphone");
                    self.permission.ask();
                }
            }
            state => {
                tracing::info!(?state, "dictation tried: the microphone is not allowed");
                self.permission.tone();
                if !std::mem::replace(&mut self.window_requested, true) {
                    self.permission.show_settings();
                }
            }
        }
    }
}

/// A permission that is always granted: the owner as it was before the gate.
#[cfg(test)]
pub(crate) struct Granted;

#[cfg(test)]
impl Permission for Granted {
    fn state(&self) -> PermissionState {
        PermissionState::Ready
    }

    fn ask(&self) {}

    fn tone(&self) {}

    fn show_settings(&self) {}
}

#[cfg(test)]
impl MicrophoneAccess {
    pub(crate) fn unrestricted(wanted: bool) -> Self {
        Self::with_permission(Box::new(Granted), wanted)
    }
}

/// Test doubles for the permission and for the stream opener.
#[cfg(test)]
pub(crate) mod fakes {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::mpsc::Sender;
    use std::sync::{Arc, Mutex};

    use super::Permission;
    use crate::audio::{AudioInput, CaptureInstant, Opened, Opener};
    use crate::desktop::onboarding::PermissionState;

    /// A permission the test turns on and off, counting what the owner did.
    #[derive(Clone)]
    pub(crate) struct Fake {
        state: Arc<Mutex<PermissionState>>,
        asked: Arc<AtomicUsize>,
        tones: Arc<AtomicUsize>,
        windows: Arc<AtomicUsize>,
    }

    impl Fake {
        pub(crate) fn new(state: PermissionState) -> Self {
            Self {
                state: Arc::new(Mutex::new(state)),
                asked: Arc::default(),
                tones: Arc::default(),
                windows: Arc::default(),
            }
        }

        pub(crate) fn set(&self, state: PermissionState) {
            *self.state.lock().unwrap() = state;
        }

        pub(crate) fn asked(&self) -> usize {
            self.asked.load(Ordering::SeqCst)
        }

        pub(crate) fn tones(&self) -> usize {
            self.tones.load(Ordering::SeqCst)
        }

        pub(crate) fn windows(&self) -> usize {
            self.windows.load(Ordering::SeqCst)
        }

        /// Prompts, tones, and windows: everything the person could notice.
        pub(crate) fn noticed(&self) -> (usize, usize, usize) {
            (self.asked(), self.tones(), self.windows())
        }
    }

    impl Permission for Fake {
        fn state(&self) -> PermissionState {
            *self.state.lock().unwrap()
        }

        fn ask(&self) {
            self.asked.fetch_add(1, Ordering::SeqCst);
        }

        fn tone(&self) {
            self.tones.fetch_add(1, Ordering::SeqCst);
        }

        fn show_settings(&self) {
            self.windows.fetch_add(1, Ordering::SeqCst);
        }
    }

    type Chunk = (Vec<f32>, CaptureInstant);

    /// Counts every stream the owner asks to open, and keeps each fake
    /// stream's sender alive so it stays healthy.
    #[derive(Clone, Default)]
    pub(crate) struct Opens {
        count: Arc<AtomicUsize>,
        senders: Arc<Mutex<Vec<Sender<Chunk>>>>,
    }

    impl Opens {
        pub(crate) fn opener(&self) -> Opener {
            let opens = self.clone();
            Arc::new(move |_| {
                opens.count.fetch_add(1, Ordering::SeqCst);
                let (input, sender) = AudioInput::channel_for_test();
                opens.senders.lock().unwrap().push(sender);
                Ok(Opened::Input(Box::new(input)))
            })
        }

        pub(crate) fn count(&self) -> usize {
            self.count.load(Ordering::SeqCst)
        }
    }

    /// A dictation owner on the fake permission and the counting opener.
    pub(crate) fn open_audio(
        permission: &Fake,
        opens: &Opens,
        wanted: bool,
    ) -> crate::dictation::audio::DictationAudio {
        use crate::audio::recording_environment::RecordingEnvironmentController;
        use crate::suppression::PendingInputEvents;
        crate::dictation::audio::DictationAudio::open_through(
            super::MicrophoneAccess::with_permission(Box::new(permission.clone()), wanted),
            opens.opener(),
            None,
            0,
            None,
            RecordingEnvironmentController::for_test(),
            PendingInputEvents::default(),
            None,
        )
        .unwrap()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    use super::fakes::{Fake, Opens, open_audio};
    use super::*;
    use crate::audio::CaptureInstant;
    use crate::dictation::Finish;
    use crate::dictation::audio::DictationAudio;

    fn at() -> CaptureInstant {
        CaptureInstant::from_nanos(60_000_000_000)
    }

    /// Everything queued before has been handled by the owner.
    fn settle(audio: &DictationAudio) {
        audio.invalidate_recognition().unwrap();
    }

    fn wait_until_open(audio: &DictationAudio) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while audio.sample_rate() == 0 {
            assert!(Instant::now() < deadline, "the microphone never opened");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    const REFUSED: [PermissionState; 3] = [
        PermissionState::NeedsRequest,
        PermissionState::NeedsSettings,
        PermissionState::NeedsRestart,
    ];

    #[test]
    fn a_start_without_permission_records_nothing_and_never_opens_the_input() {
        for state in REFUSED {
            for wanted in [false, true] {
                let permission = Fake::new(state);
                let opens = Opens::default();
                let audio = open_audio(&permission, &opens, wanted);

                assert!(!audio.start(at()).unwrap(), "{state:?} wanted {wanted}");
                assert_eq!(permission.noticed(), (0, 0, 0), "a start says nothing");
                assert!(!audio.is_recording());
                assert_eq!(
                    audio.capture_generation(),
                    0,
                    "a refused start counts as none"
                );
                assert_eq!(
                    audio.start_programmatic(at()).unwrap_err().to_string(),
                    "microphone-unavailable"
                );
                assert!(!audio.is_recovering());
                assert!(matches!(audio.finish(at()).unwrap(), Finish::Discard));
                audio.cancel().unwrap();
                assert_eq!(opens.count(), 0, "{state:?} wanted {wanted}");
            }
        }
    }

    #[test]
    fn a_settings_reload_without_permission_keeps_the_input_closed() {
        // Every reload sends the saved policy; with the microphone wanted
        // warm, an owner that took it literally would open the stream.
        for state in REFUSED {
            let permission = Fake::new(state);
            let opens = Opens::default();
            let audio = open_audio(&permission, &opens, false);
            assert!(audio.microphone_gated());
            for release in [true, false, false, true, false] {
                audio.set_release_while_idle(release);
                settle(&audio);
                assert_eq!(opens.count(), 0, "{state:?} release {release}");
                assert!(audio.microphone_gated());
                assert!(!audio.is_recovering());
            }
        }
    }

    #[test]
    fn granting_permission_restores_the_saved_policy() {
        for (wanted, opens_after_two_presses) in [(false, 1), (true, 2)] {
            let permission = Fake::new(PermissionState::NeedsSettings);
            let opens = Opens::default();
            let audio = open_audio(&permission, &opens, wanted);
            assert!(!audio.start(at()).unwrap());

            permission.set(PermissionState::Ready);
            // The first press after the grant is a cold open: it records once
            // the stream is up, like a released microphone.
            assert!(audio.start(at()).unwrap());
            assert!(audio.microphone_gated(), "stable during the capture");
            wait_until_open(&audio);
            assert_eq!(opens.count(), 1);
            audio.cancel().unwrap();
            assert!(!audio.microphone_gated(), "the grant applies once idle");
            settle(&audio);

            assert!(audio.start(at()).unwrap());
            let deadline = Instant::now() + Duration::from_secs(5);
            while opens.count() < opens_after_two_presses {
                assert!(Instant::now() < deadline, "wanted {wanted}: no second open");
                std::thread::sleep(Duration::from_millis(5));
            }
            std::thread::sleep(Duration::from_millis(100));
            assert_eq!(opens.count(), opens_after_two_presses, "wanted {wanted}");
        }
    }

    #[test]
    fn a_revoked_permission_closes_the_warm_input_once_idle() {
        for ending in ["cancel", "refused start"] {
            let permission = Fake::new(PermissionState::Ready);
            let opens = Opens::default();
            let audio = open_audio(&permission, &opens, false);
            assert_eq!(
                opens.count(),
                1,
                "granted and warm: open at startup, as before"
            );
            assert!(!audio.microphone_gated());

            permission.set(PermissionState::NeedsSettings);
            if ending == "cancel" {
                audio.cancel().unwrap();
            } else {
                assert!(!audio.start(at()).unwrap());
            }
            assert!(audio.microphone_gated(), "{ending}");
            assert_eq!(opens.count(), 1, "{ending}");

            // The stream was closed: the next grant needs a fresh open.
            permission.set(PermissionState::Ready);
            assert!(audio.start(at()).unwrap());
            let deadline = Instant::now() + Duration::from_secs(5);
            while opens.count() < 2 {
                assert!(Instant::now() < deadline, "{ending}: the input stayed open");
                std::thread::sleep(Duration::from_millis(5));
            }
        }
    }

    /// A start is silent whatever the permission; only an owned press or a
    /// programmatic start is an intentional try.
    #[test]
    fn a_start_alone_asks_and_sounds_nothing() {
        for state in REFUSED {
            let permission = Fake::new(state);
            let audio = open_audio(&permission, &Opens::default(), false);
            for _ in 0..3 {
                assert!(!audio.start(at()).unwrap());
                audio.cancel().unwrap();
            }
            settle(&audio);
            assert_eq!(permission.noticed(), (0, 0, 0), "{state:?}");
        }
    }

    #[test]
    fn the_first_intentional_try_asks_and_a_refused_one_sounds_every_time() {
        let permission = Fake::new(PermissionState::NeedsRequest);
        let audio = open_audio(&permission, &Opens::default(), false);
        for _ in 0..3 {
            audio.press_owned();
        }
        settle(&audio);
        assert_eq!(permission.noticed(), (1, 0, 0), "the system prompt, once");

        // The person refuses it. Each later try sounds; Settings opens once.
        permission.set(PermissionState::NeedsSettings);
        for _ in 0..3 {
            audio.press_owned();
        }
        settle(&audio);
        assert_eq!(permission.noticed(), (1, 3, 1));

        for state in [
            PermissionState::NeedsSettings,
            PermissionState::NeedsRestart,
        ] {
            let permission = Fake::new(state);
            let audio = open_audio(&permission, &Opens::default(), false);
            for _ in 0..3 {
                audio.press_owned();
            }
            settle(&audio);
            assert_eq!(permission.noticed(), (0, 3, 1), "{state:?}");
        }
    }

    /// `/dictations` is intentional by nature: its refused start tries at once.
    #[test]
    fn a_programmatic_start_is_an_intentional_try() {
        let permission = Fake::new(PermissionState::NeedsRequest);
        let audio = open_audio(&permission, &Opens::default(), false);
        assert!(audio.start_programmatic(at()).is_err());
        assert_eq!(permission.noticed(), (1, 0, 0));
        permission.set(PermissionState::NeedsSettings);
        for _ in 0..2 {
            assert!(audio.start_programmatic(at()).is_err());
        }
        assert_eq!(permission.noticed(), (1, 2, 1));
    }

    /// A grant then a revoke is one more refusal, not a new window.
    #[test]
    fn granting_then_revoking_keeps_the_window_to_once_per_launch() {
        let permission = Fake::new(PermissionState::NeedsSettings);
        let opens = Opens::default();
        let audio = open_audio(&permission, &opens, false);
        audio.press_owned();
        settle(&audio);
        permission.set(PermissionState::Ready);
        assert!(audio.start(at()).unwrap());
        wait_until_open(&audio);
        audio.cancel().unwrap();
        permission.set(PermissionState::NeedsSettings);
        audio.press_owned();
        settle(&audio);
        assert_eq!(permission.noticed(), (0, 2, 1));
    }

    /// Every way the listener's microphone could open after `listen()`, driven
    /// with the permission missing and a stream counter: none may open one.
    /// Annotate's capture sink is the stated exception (a user action macOS
    /// asks about itself) and has its own test in `dictation::audio`.
    ///
    /// What a test cannot drive: the CoreAudio and lid listeners themselves
    /// (they register in `watch_input_devices`, called only from a real
    /// stream open, so none exists here) and a real permission prompt. The
    /// route change they report is driven through `notify_input_route_changed`.
    #[test]
    fn no_open_path_opens_the_microphone_without_permission() {
        for state in REFUSED {
            for wanted in [false, true] {
                let label = format!("{state:?}, wanted {wanted}");
                let permission = Fake::new(state);
                let opens = Opens::default();

                // Warm open at startup (wanted false) and cold start (true).
                let audio = open_audio(&permission, &opens, wanted);
                assert_eq!(opens.count(), 0, "startup: {label}");

                // Settings reload, release-while-idle on and off.
                for release in [!wanted, wanted, !wanted, wanted] {
                    audio.set_release_while_idle(release);
                }
                // Default input changed, device list changed, lid closed and
                // opened: each ends in the same route-change report, and the
                // avoid-Bluetooth setting reports through it too.
                for _ in 0..3 {
                    crate::audio::notify_input_route_changed();
                }
                crate::audio::set_avoid_bluetooth_microphone(false);
                crate::audio::set_avoid_bluetooth_microphone(true);
                // Selection changed in Settings, to a device and back.
                audio.request_selection(1, Some("Studio Display Microphone"));
                audio.request_selection(2, None);
                settle(&audio);
                // Cold-open retry: any timed attempt would fire inside this.
                std::thread::sleep(Duration::from_millis(600));
                settle(&audio);
                assert_eq!(opens.count(), 0, "reloads, route and selection: {label}");
                assert!(!audio.is_recovering(), "{label}");

                // The shortcut, `hex dictation`, `/dictations`, and the end of
                // a capture that never began.
                assert!(!audio.start(at()).unwrap(), "{label}");
                assert!(audio.start_programmatic(at()).is_err(), "{label}");
                assert!(!audio.is_recording(), "{label}");
                let _ = audio.finish(at()).unwrap();
                audio.cancel().unwrap();
                settle(&audio);
                assert_eq!(opens.count(), 0, "starts: {label}");
                assert!(!audio.is_recovering(), "{label}");
            }
        }
    }
    /// A log line collector for one test thread.
    #[derive(Clone, Default)]
    struct Lines(Arc<Mutex<Vec<u8>>>);

    impl std::io::Write for Lines {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Lines {
        type Writer = Lines;

        fn make_writer(&'a self) -> Lines {
            self.clone()
        }
    }

    /// Lines logged on this thread while `work` runs.
    fn logged(work: impl FnOnce()) -> String {
        let lines = Lines::default();
        let subscriber = tracing_subscriber::fmt()
            .with_writer(lines.clone())
            .with_ansi(false)
            .finish();
        tracing::subscriber::with_default(subscriber, work);
        let bytes = lines.0.lock().unwrap().clone();
        String::from_utf8(bytes).unwrap()
    }

    /// The live check greps the process log for this exact message to tell
    /// whether the Mac opened a microphone stream.
    #[test]
    fn an_opened_stream_logs_one_line_and_a_refused_microphone_logs_none() {
        const LINE: &str = "microphone stream opened";
        let opened = |state| {
            let opens = Opens::default();
            let log = logged(|| {
                // Wanted warm, so a granted permission opens at startup on this
                // thread; refused ones must not.
                let audio = open_audio(&Fake::new(state), &opens, false);
                let _ = audio.start(at());
                settle(&audio);
            });
            (log.matches(LINE).count(), opens.count())
        };
        assert_eq!(opened(PermissionState::Ready), (1, 1), "one open, one line");
        for state in REFUSED {
            assert_eq!(
                opened(state),
                (0, 0),
                "{state:?}: nothing opened, nothing logged"
            );
        }
    }
}
