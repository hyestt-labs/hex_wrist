//! The Apple Watch settings as the running service reads them: copied here
//! when settings apply (`AppSettings::apply_runtime`) and read where each
//! one acts, so a change needs no restart and no listener reload.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{OnceLock, RwLock};

use crate::app_settings::WatchSettings;

static STATUS_CAPSULE: AtomicBool = AtomicBool::new(true);
static CAPSULE_FOLLOWS_POINTER: AtomicBool = AtomicBool::new(false);
static KEEP_ON_CLIPBOARD: AtomicBool = AtomicBool::new(false);
static TERMINAL_APPS: OnceLock<RwLock<Vec<String>>> = OnceLock::new();

pub(crate) fn apply(settings: &WatchSettings) {
    STATUS_CAPSULE.store(settings.status_capsule, Ordering::Relaxed);
    CAPSULE_FOLLOWS_POINTER.store(settings.capsule_follows_pointer, Ordering::Relaxed);
    KEEP_ON_CLIPBOARD.store(settings.keep_on_clipboard, Ordering::Relaxed);
    *TERMINAL_APPS
        .get_or_init(Default::default)
        .write()
        .unwrap_or_else(|error| error.into_inner()) = settings.terminal_apps.clone();
}

/// Whether the status capsule shows, read each time the service publishes.
pub(crate) fn status_capsule() -> bool {
    STATUS_CAPSULE.load(Ordering::Relaxed)
}

/// Whether the capsule shows on the pointer's screen rather than the main one.
pub(crate) fn capsule_follows_pointer() -> bool {
    CAPSULE_FOLLOWS_POINTER.load(Ordering::Relaxed)
}

/// Whether a watch dictation stays on the clipboard after it pastes. Labs'
/// switch for every dictation is read apart (`paste::Keep`).
pub(crate) fn keep_on_clipboard() -> bool {
    KEEP_ON_CLIPBOARD.load(Ordering::Relaxed)
}

/// Whether the saved list names `bundle_id` as a terminal.
pub(crate) fn is_extra_terminal(bundle_id: &str) -> bool {
    TERMINAL_APPS
        .get_or_init(Default::default)
        .read()
        .unwrap_or_else(|error| error.into_inner())
        .iter()
        .any(|app| app == bundle_id)
}
