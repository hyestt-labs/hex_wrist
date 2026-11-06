//! The window's view of the service is pushed from the `watch` thread when
//! the relay, the pairing, or the list of watches changes.

use std::rc::Rc;

use super::*;
use crate::watch::pairing::{DeviceStatus, PairedDevices};

type Seen = Arc<Mutex<Vec<WatchSnapshot>>>;

fn watching(h: &mut Harness) -> Seen {
    let seen: Seen = Arc::default();
    let sink = Arc::clone(&seen);
    h.core.view = Arc::new(move |snapshot| sink.lock().unwrap().push(snapshot));
    seen
}

fn revoke(h: &mut Harness, id: &str) -> Result<bool, String> {
    let (reply, answer) = mpsc::sync_channel(1);
    h.core.handle(
        Message::Revoke {
            id: id.to_owned(),
            reply,
        },
        h.now,
    );
    answer.try_recv().unwrap()
}

/// Swaps in a keychain the test can read, before any watch pairs.
fn shared_keychain(h: &mut Harness) -> Rc<MemorySecretStore> {
    let store = Rc::new(MemorySecretStore::default());
    h.core.paired = PairedDevices::load(&h.dir.join(pairing::FILE_NAME), Box::new(store.clone()));
    store
}

/// The status `/health` answers the watch's token.
fn health(h: &mut Harness, watch: &Watch) -> u16 {
    let headers = h.device_headers(watch);
    h.request("GET", "/health", &[], &headers, &[]).unwrap().0
}

fn listed(seen: &Seen) -> Vec<String> {
    let last = seen.lock().unwrap().last().cloned().unwrap();
    last.devices.into_iter().map(|device| device.id).collect()
}

#[test]
fn a_claim_then_the_watch_request_pushes_the_new_device() {
    let mut h = harness("view-claim");
    let seen = watching(&mut h);
    h.core.handle(
        Message::Relay(RelayEvent::State(RelayState::Connected)),
        h.now,
    );
    assert_eq!(
        seen.lock().unwrap().last().unwrap().relay,
        RelayState::Connected
    );
    let (reply, answer) = mpsc::sync_channel(1);
    h.core.handle(
        Message::Claim {
            code: "217400".to_owned(),
            reply,
            deadline: h.now.at + CLAIM_TIMEOUT,
        },
        h.now,
    );
    h.core.handle(
        Message::Relay(RelayEvent::Inbound(Inbound::ClaimOk {
            code: "217400".to_owned(),
            device_id: "watch-9".to_owned(),
            device_name: "Series 9".to_owned(),
        })),
        h.now,
    );
    assert!(answer.try_recv().unwrap().is_ok());
    assert!(
        seen.lock()
            .unwrap()
            .iter()
            .all(|snapshot| snapshot.devices.is_empty()),
        "a claim alone enrolls nothing: the watch has not asked yet"
    );
    h.pair("watch-9");
    let last = seen.lock().unwrap().last().cloned().unwrap();
    assert_eq!(last.mac_name, MAC_NAME);
    let [device] = &last.devices[..] else {
        panic!("one watch is paired: {:?}", last.devices);
    };
    assert_eq!(device.id, "watch-9");
    assert_eq!(device.status, DeviceStatus::Ready);
}

#[test]
fn another_mac_taking_the_room_is_shown_until_this_one_serves_again() {
    let mut h = harness("view-replaced");
    let seen = watching(&mut h);
    let last = |seen: &Seen| seen.lock().unwrap().last().cloned().unwrap();
    h.core.handle(Message::Relay(RelayEvent::Replaced), h.now);
    assert!(last(&seen).replaced);
    h.core.handle(
        Message::Relay(RelayEvent::State(RelayState::Connecting)),
        h.now,
    );
    assert!(last(&seen).replaced, "still waiting to reconnect");
    h.core.handle(
        Message::Relay(RelayEvent::State(RelayState::Connected)),
        h.now,
    );
    assert!(!last(&seen).replaced);
}

#[test]
fn an_unchanged_state_is_not_pushed_again() {
    let mut h = harness("view-quiet");
    let seen = watching(&mut h);
    for _ in 0..3 {
        h.core.handle(
            Message::Relay(RelayEvent::State(RelayState::Connected)),
            h.now,
        );
    }
    assert_eq!(seen.lock().unwrap().len(), 1);
}

#[test]
fn revoke_removes_the_watch_and_pushes_the_list() {
    let mut h = harness("view-revoke");
    let store = shared_keychain(&mut h);
    let seen = watching(&mut h);
    let watch = h.pair("watch-1");
    assert_eq!(listed(&seen), ["watch-1"]);
    assert_eq!(health(&mut h, &watch), 200);
    assert_eq!(revoke(&mut h, "watch-1"), Ok(true));
    assert!(listed(&seen).is_empty());
    assert_eq!(
        health(&mut h, &watch),
        403,
        "the revoked watch's token no longer works"
    );
    assert_eq!(store.load("watch-1").unwrap(), None);
    assert_eq!(
        revoke(&mut h, "watch-1"),
        Ok(false),
        "revoking again changes nothing"
    );
}

#[test]
fn a_revoke_that_cannot_write_the_list_keeps_the_watch_listed_and_served() {
    let mut h = harness("view-revoke-unsaved");
    let store = shared_keychain(&mut h);
    let seen = watching(&mut h);
    let watch = h.pair("watch-1");
    h.core.paired.refuse_saves("disk full");
    let error = revoke(&mut h, "watch-1").expect_err("a list that was not written is not removed");
    assert!(error.contains("disk full"), "{error}");
    assert_eq!(listed(&seen), ["watch-1"], "the window still lists it");
    let snapshot = h.core.snapshot();
    assert_eq!(snapshot.devices.len(), 1);
    assert_eq!(snapshot.devices[0].status, DeviceStatus::Ready);
    assert_eq!(
        store.load("watch-1").unwrap().map(|secret| secret.token),
        Some(watch.token.clone()),
        "the keychain item was not touched"
    );
    assert_eq!(health(&mut h, &watch), 200, "its next request is served");
}

#[test]
fn a_revoke_tried_again_after_a_failure_removes_the_watch() {
    let mut h = harness("view-revoke-again");
    let store = shared_keychain(&mut h);
    let seen = watching(&mut h);
    let watch = h.pair("watch-1");
    h.core.paired.refuse_saves("disk full");
    assert!(revoke(&mut h, "watch-1").is_err());
    h.core.paired.allow_saves();
    assert_eq!(revoke(&mut h, "watch-1"), Ok(true));
    assert!(listed(&seen).is_empty());
    assert_eq!(store.load("watch-1").unwrap(), None);
    assert_eq!(health(&mut h, &watch), 403);
}

#[test]
fn pairing_a_watch_again_pushes_when_it_last_paired() {
    let mut h = harness("view-repair");
    let seen = watching(&mut h);
    let watch = h.pair("watch-1");
    let paired_at = |seen: &Seen| {
        let last = seen.lock().unwrap().last().cloned().unwrap();
        let [device] = &last.devices[..] else {
            panic!("one watch is listed: {:?}", last.devices);
        };
        (device.id.clone(), device.last_paired_at.clone())
    };
    let (_, first) = paired_at(&seen);
    h.advance(Duration::from_secs(60));
    // A reinstalled watch: a new id from the relay, and its old token.
    h.approve("watch-1b");
    let (status, body) = h
        .request(
            "POST",
            "/pairing/request",
            &[],
            &[],
            json!({
                "device_id": "watch-1b",
                "name": "Test Watch",
                "previous_token": watch.token,
                "e2ee_public_key": public_key_base64(&watch_secret()),
            })
            .to_string()
            .as_bytes(),
        )
        .unwrap();
    assert_eq!(status, 200, "{body}");
    let (id, again) = paired_at(&seen);
    assert_eq!(id, "watch-1", "the old entry is paired again");
    assert_ne!(again, first, "the window sees a new pairing at once");
}
