//! The macOS host's side of the Apple Watch pane: it hands the service's
//! pushed snapshots to the window, and runs pairing and Remove on threads of
//! their own, since a claim waits on the relay for up to 11 s.

use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};

use super::platform::{PlatformRequest, PlatformState};
use super::watch_pane::WatchEvent;
use crate::watch::service::{ClaimError, WatchSnapshot};

pub(super) struct WatchFeed {
    events: Sender<WatchEvent>,
    replies: Receiver<WatchEvent>,
    /// The newest snapshot the service pushed and the window has not taken.
    pushed: Arc<Mutex<Option<WatchSnapshot>>>,
    wake: Arc<dyn Fn() + Send + Sync>,
}

impl WatchFeed {
    pub(super) fn new(wake: Arc<dyn Fn() + Send + Sync>) -> Self {
        let (events, replies) = mpsc::channel();
        let pushed: Arc<Mutex<Option<WatchSnapshot>>> = Arc::default();
        let (slot, wake_window) = (Arc::clone(&pushed), Arc::clone(&wake));
        // Runs on the service's thread: it only hands the snapshot over.
        crate::watch::view::watch(move |snapshot| {
            *slot.lock().unwrap_or_else(|error| error.into_inner()) = Some(snapshot);
            wake_window();
        });
        Self {
            events,
            replies,
            pushed,
            wake,
        }
    }

    /// Starts the work for a pairing or Remove request. False for any other.
    pub(super) fn send(&self, request: &PlatformRequest) -> bool {
        let (events, wake) = (self.events.clone(), Arc::clone(&self.wake));
        match request.clone() {
            PlatformRequest::WatchPair(code) => spawn("hex-watch-pair", move || {
                let result = match crate::watch::service() {
                    Some(service) => service.claim(&code),
                    None => Err(ClaimError::Unavailable),
                };
                let _ = events.send(WatchEvent::Claimed(result));
                wake();
            }),
            PlatformRequest::WatchRevoke { id, name } => spawn("hex-watch-revoke", move || {
                let result = match crate::watch::service() {
                    Some(service) => service.revoke(&id),
                    None => Err(ClaimError::Unavailable.message()),
                };
                let _ = events.send(WatchEvent::Revoked { id, name, result });
                wake();
            }),
            _ => return false,
        }
        true
    }

    /// Moves what arrived into the window's state.
    pub(super) fn poll(&mut self, state: &mut PlatformState) -> bool {
        let mut changed = false;
        let pushed = self
            .pushed
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .take();
        if let Some(snapshot) = pushed
            && state.watch.as_ref() != Some(&snapshot)
        {
            state.watch = Some(snapshot);
            changed = true;
        }
        while let Ok(event) = self.replies.try_recv() {
            state.watch_events.push(event);
            changed = true;
        }
        changed
    }
}

fn spawn(name: &str, work: impl FnOnce() + Send + 'static) {
    if let Err(error) = std::thread::Builder::new().name(name.into()).spawn(work) {
        tracing::warn!(%error, thread = name, "could not start the Apple Watch worker");
    }
}
