//! What the Settings window shows about the Apple Watch service: the relay
//! connection, this Mac's name, and the paired watches. The `watch` thread
//! pushes a [`WatchSnapshot`] each time one of them changes, as
//! [`status`](super::status) pushes capsules; the window installs a watcher
//! and never polls.

use std::sync::{Arc, Mutex};

use super::service::WatchSnapshot;

/// Where snapshots go. Called from the `watch` thread, so a watcher hands
/// the snapshot to its own thread and never waits.
pub(crate) type Sink = Arc<dyn Fn(WatchSnapshot) + Send + Sync>;

#[derive(Default)]
struct Hub {
    sinks: Vec<Sink>,
    latest: Option<WatchSnapshot>,
}

static HUB: Mutex<Hub> = Mutex::new(Hub {
    sinks: Vec::new(),
    latest: None,
});

/// Registers a watcher. A snapshot that already went out is handed to it at
/// once, so a window opened later starts from the current state.
pub(crate) fn watch(sink: impl Fn(WatchSnapshot) + Send + Sync + 'static) {
    let sink: Sink = Arc::new(sink);
    let latest = {
        let mut hub = HUB.lock().unwrap_or_else(|error| error.into_inner());
        hub.sinks.push(Arc::clone(&sink));
        hub.latest.clone()
    };
    if let Some(snapshot) = latest {
        sink(snapshot);
    }
}

/// Tells every watcher; the service's default sink.
pub(crate) fn publish(snapshot: WatchSnapshot) {
    let sinks = {
        let mut hub = HUB.lock().unwrap_or_else(|error| error.into_inner());
        hub.latest = Some(snapshot.clone());
        hub.sinks.clone()
    };
    for sink in sinks {
        sink(snapshot.clone());
    }
}

/// The newest snapshot, or `None` while the service has pushed none.
pub(crate) fn latest() -> Option<WatchSnapshot> {
    HUB.lock()
        .unwrap_or_else(|error| error.into_inner())
        .latest
        .clone()
}

/// Whether at least one paired watch can be served (not needing repair).
/// A watch waiting on the Keychain counts, so a locked Keychain at login
/// never brings the Microphone row back. `None` until the first snapshot.
pub(crate) fn paired(snapshot: Option<&WatchSnapshot>) -> Option<bool> {
    use super::pairing::DeviceStatus;
    snapshot.map(|snapshot| {
        snapshot
            .devices
            .iter()
            .any(|device| device.status != DeviceStatus::NeedsRepair)
    })
}
