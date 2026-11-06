//! The window's view of the service is pushed from the `watch` thread when
//! the relay, the pairing, or the list of watches changes.

use super::*;
use crate::watch::pairing::DeviceStatus;

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
    let seen = watching(&mut h);
    let watch = h.pair("watch-1");
    assert_eq!(seen.lock().unwrap().last().unwrap().devices.len(), 1);
    assert_eq!(
        h.core.paired.device_for_token(&watch.token).as_deref(),
        Some("watch-1")
    );
    assert_eq!(revoke(&mut h, "watch-1"), Ok(true));
    assert!(seen.lock().unwrap().last().unwrap().devices.is_empty());
    assert_eq!(
        h.core.paired.device_for_token(&watch.token),
        None,
        "the revoked watch's token no longer works"
    );
    assert_eq!(
        revoke(&mut h, "watch-1"),
        Ok(false),
        "revoking again changes nothing"
    );
}

#[test]
fn revoke_reports_the_list_it_could_not_write() {
    let mut h = harness("view-revoke-unsaved");
    let seen = watching(&mut h);
    let watch = h.pair("watch-1");
    h.core.paired.refuse_saves("disk full");
    let error = revoke(&mut h, "watch-1").expect_err("a list that was not written is not removed");
    assert!(error.contains("disk full"), "{error}");
    // The service stops serving it at once even so, and says so.
    assert!(seen.lock().unwrap().last().unwrap().devices.is_empty());
    assert_eq!(h.core.paired.device_for_token(&watch.token), None);
}
