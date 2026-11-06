//! Whether the dictation listener can take a watch recording now.
//!
//! The listener starts once Accessibility and the model are ready (the
//! microphone is not needed). Before that, nothing reads the control queue,
//! so a recording would sit there until the watch gave up. Refusing it at
//! once names what is missing and lets the watch say so.

use crate::desktop::onboarding::{PermissionState, SetupStatus};

/// What keeps the listener from starting, if anything.
pub(crate) fn blocker(status: SetupStatus) -> Option<&'static str> {
    if status.accessibility != PermissionState::Ready {
        Some("Hex needs Accessibility to paste on this Mac")
    } else if !status.transcription_model {
        Some("Hex has no speech model installed on this Mac")
    } else {
        None
    }
}

/// The running Mac's answer, read at each recording.
#[cfg(target_os = "macos")]
pub(crate) fn current() -> Result<(), String> {
    let (_, selection) = crate::app_settings::transcription_selection();
    match blocker(crate::desktop::onboarding::status(&selection)) {
        Some(why) => Err(why.to_owned()),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status(accessibility: PermissionState, model: bool) -> SetupStatus {
        SetupStatus {
            microphone: PermissionState::NeedsRequest,
            accessibility,
            transcription_model: model,
        }
    }

    #[test]
    fn only_accessibility_and_the_model_block_the_listener() {
        assert_eq!(blocker(status(PermissionState::Ready, true)), None);
        assert!(
            blocker(status(PermissionState::NeedsSettings, true))
                .unwrap()
                .contains("Accessibility")
        );
        assert!(
            blocker(status(PermissionState::NeedsRestart, true))
                .unwrap()
                .contains("Accessibility")
        );
        assert!(
            blocker(status(PermissionState::Ready, false))
                .unwrap()
                .contains("model")
        );
    }
}
