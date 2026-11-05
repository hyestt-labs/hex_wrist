//! A paste whose clipboard restore the caller times: a watch recording
//! restores at once when the read-back sees the text land, and after
//! `READ_BACK_GRACE` when it cannot. It rides the same `ClipboardRestore`
//! generation as every other paste, so a newer paste or an outside
//! clipboard change still cancels it.

use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use super::{ClipboardRestore, ClipboardSnapshot, NSPasteboard};

/// When the clipboard comes back after ⌘V.
#[derive(Clone, Copy)]
pub(super) enum Restore {
    /// A fixed delay after the key press (dictation).
    After(Duration),
    /// The caller decides through the returned handle.
    Deferred,
}

/// How a clipboard paste ended.
pub(super) enum Pasted {
    /// ⌘V was sent; the handle, if any, is the restore left to time.
    Sent(Option<RestoreHandle>),
    /// The caller's last check refused it right before ⌘V; the clipboard
    /// is back as it was and nothing was sent.
    Refused,
}

/// What a paste inserted, and the restore left to time.
pub(crate) struct Sent {
    /// The text as it went in, after the continuation join and spacing.
    pub(crate) inserted: String,
    /// `None` when the clipboard needs no restore: typed text, Keep on
    /// clipboard, or nothing worth putting back.
    pub(crate) restore: Option<RestoreHandle>,
}

pub(crate) struct RestoreHandle {
    clipboard_restore: Arc<Mutex<ClipboardRestore>>,
    generation: u64,
}

impl RestoreHandle {
    pub(super) fn new(clipboard_restore: Arc<Mutex<ClipboardRestore>>, generation: u64) -> Self {
        Self {
            clipboard_restore,
            generation,
        }
    }

    /// Restores after `delay` (at once when zero) unless a newer paste or
    /// an outside clipboard change got there first.
    pub(crate) fn restore_after(self, delay: Duration) {
        restore_after(self.clipboard_restore, self.generation, delay);
    }
}

pub(super) fn restore_after(
    clipboard_restore: Arc<Mutex<ClipboardRestore>>,
    generation: u64,
    delay: Duration,
) {
    let restore = move || {
        let mut restore = clipboard_restore
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if restore.generation != generation {
            return;
        }
        let clipboard = NSPasteboard::generalPasteboard();
        if restore.last_change_count != Some(clipboard.changeCount()) {
            restore.original = None;
            restore.last_change_count = None;
            return;
        }
        let Some(previous) = restore.original.take() else {
            return;
        };
        let result = super::restore_clipboard(&clipboard, &previous);
        restore.last_change_count = None;
        if let Err(error) = result {
            tracing::warn!(%error, "could not restore clipboard after paste");
        }
    };
    if delay.is_zero() {
        restore();
    } else {
        thread::spawn(move || {
            thread::sleep(delay);
            restore();
        });
    }
}

/// nspasteboard.org markers for clipboard content that must not be copied
/// elsewhere: a password manager's secret, a one-shot transfer.
const PRIVATE_FLAVORS: [&str; 2] = [
    "org.nspasteboard.ConcealedType",
    "org.nspasteboard.TransientType",
];

/// What a watch paste may put back: items carrying a private marker are
/// dropped, and when nothing else was there the restore is skipped, so a
/// copied password is neither kept in memory nor written back
/// (`ClipboardRestorePolicy.canBackUp`).
pub(super) fn restorable(original: ClipboardSnapshot) -> Option<ClipboardSnapshot> {
    let had_items = !original.items.is_empty();
    let items: Vec<_> = original
        .items
        .into_iter()
        .filter(|item| {
            !item
                .iter()
                .any(|flavor| PRIVATE_FLAVORS.contains(&flavor.data_type.as_str()))
        })
        .collect();
    (!had_items || !items.is_empty()).then_some(ClipboardSnapshot { items })
}

#[cfg(test)]
mod tests {
    use super::super::ClipboardFlavor;
    use super::*;

    fn item(types: &[&str]) -> Vec<ClipboardFlavor> {
        types
            .iter()
            .map(|data_type| ClipboardFlavor {
                data_type: (*data_type).to_owned(),
                data: b"x".to_vec(),
                flags: 0,
            })
            .collect()
    }

    fn snapshot(items: Vec<Vec<ClipboardFlavor>>) -> ClipboardSnapshot {
        ClipboardSnapshot { items }
    }

    #[test]
    fn an_ordinary_clipboard_comes_back_whole() {
        let original = snapshot(vec![item(&["public.utf8-plain-text"])]);
        assert_eq!(restorable(original.clone()), Some(original));
    }

    #[test]
    fn a_concealed_password_is_never_restored() {
        let original = snapshot(vec![item(&[
            "public.utf8-plain-text",
            "org.nspasteboard.ConcealedType",
        ])]);
        assert_eq!(restorable(original), None);
    }

    #[test]
    fn a_transient_item_is_dropped_and_the_rest_stays() {
        let keep = item(&["public.utf8-plain-text"]);
        let original = snapshot(vec![
            item(&["public.png", "org.nspasteboard.TransientType"]),
            keep.clone(),
        ]);
        assert_eq!(restorable(original), Some(snapshot(vec![keep])));
    }

    #[test]
    fn an_empty_clipboard_restores_to_empty() {
        assert_eq!(restorable(snapshot(vec![])), Some(snapshot(vec![])));
    }
}
