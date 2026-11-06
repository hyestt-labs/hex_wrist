# Apple Watch (Wristper)

Dictate on the Apple Watch with the Wristper watch app; this Mac receives the
recording through the Cloudflare relay, transcribes it with the dictation
model, and pastes the text at the current cursor. The watch app does not
change: the Mac speaks Wristper protocol 4 byte for byte
(`artifacts/wristper-migration-20261104/wristper-protocol.md` is the contract,
with Swift line references). macOS only in this unit; Linux and Windows carry
`watch` as a pending difference in `rune_ui::platform::DIFFERENCES`.

Status: units B1 (relay, pairing, delivery, `/status`) and B2 (field
snapshot, paste gate, read-back, Enter, Undo, suspect text, status
capsule). Chinese characters are the one setting every dictation follows
(unit G; [dictation.md](dictation.md#chinese-characters)). Unit C1 lets a Mac
without the Microphone transcribe; unit C2 adds the Settings pane, the
`watch.*` settings, and the first run's Apple Watch page. Unit D adds
licensing and the free quota.

## How To Get To It

```ts
Apple Watch                               // watch.pair, watch.dictate
├── Pair
│   ├── Watch: Pair a Mac -> shows a 6-digit code        // the watch asks the relay
│   ├── Mac: Settings › Apple Watch › Pairing code      // six digits send the claim by themselves (C2)
│   ├── Mac: hex watch pair <code>                       // B1's entry; the pane uses the same claim
│   │   ├── done  -> "Claimed the code for Apple Watch (…)"; the watch finishes by itself
│   │   ├── invalid_code | code_not_found | too_many_attempts | relay_offline | timeout
│   │   └── unavailable -> the service did not start (see the log)
│   └── Watch: POST /pairing/request (P-256 key) -> GET /pairing/result (sealed token)
├── Dictate
│   ├── Watch starts recording -> /e2ee/query {/focus}   -> watch-ax reads the focused field
│   │   ├── field read   -> captured:true, role          -> record awaiting-audio, capsule "Recording"
│   │   └── nothing read (no Accessibility, no focus, app silent for 1.5 s) -> captured:false;
│   │                                                 its recording is withheld as target-changed (Wristper pasted at the cursor)
│   ├── Watch stops -> /e2ee (AudioHead + m4a)           -> decode -> DictationWorker
│   │   ├── Chinese characters (Settings: chinese_output) -> the one pipeline step, same as every dictation
│   │   ├── suspect text (silence caption, wrong script) -> suspect, not pasted
│   │   ├── Labs › Copy dictation only on                -> copied
│   │   ├── gate against the field /focus locked
│   │   │   ├── secure input or password field           -> sensitive
│   │   │   ├── another field or app                     -> target-changed
│   │   │   ├── cannot be edited                         -> not-editable
│   │   │   ├── audio beat /focus by > 2 s               -> target-changed + reason
│   │   │   ├── nothing focused, focus: current          -> copied
│   │   │   └── same field                               -> ⌘V
│   │   │       ├── terminal app                         -> pasted + terminal (final)
│   │   │       └── other app -> unverified; ≤ 1 s later -> pasted + read-back, or stays unverified
│   │   └── no Accessibility / decode / inference failed -> 500 failed
│   └── Watch 1.5 s later: /e2ee/query {/status}         -> the same fields as Wristper
├── Enter  (/enter, ≤ 30 s after the paste, pasted or unverified, once)
│   ├── recording still in progress                      -> waits ≤ 15 s
│   ├── same app, role, and window still in front        -> Return -> entered
│   └── else -> {ok:false, reason: not-pasted | expired | target-changed | sensitive | failed}
├── Undo   (/undo, ≤ 60 s, not after Enter, read-back confirmed, never a terminal)
│   └── same field in front                              -> ⌘Z -> undone
└── Resend (watch's unsent list)
    ├── same id while transcribing -> waits ≤ 45 s, else {duplicate, transcribing}
    └── same id after delivery     -> {duplicate, state, verified, verification}
```

## Settings And First Run (C2)

```ts
Settings › Apple Watch (macOS, below General)         // pane key nav-watch
├── This Mac
│   ├── Connection: Connected | Connecting… | Offline: check the network
│   │   └── another Mac took the room -> "Another Mac took over this pairing" until this Mac serves again
│   └── Name on your watch                             // what the watch shows for this Mac
├── Pair a watch
│   ├── Pairing code (six digits; only digits stay)    // the sixth digit sends the claim once
│   │   ├── Pairing… -> Finish on your watch… (≤ 120 s) -> Paired with <name>
│   │   ├── invalid_code | relay_offline | timeout | code_not_found | too_many_attempts | relay refusal -> its own sentence
│   │   ├── the relay took the code but that watch did not ask for its token in 120 s -> "The watch didn't finish pairing. Update Wristper on the watch and try again"
│   │   └── a reply to an older claim, or a watch that pairs meanwhile, settles nothing: only the claimed watch does
│   └── Get the watch app -> https://wristper.dev/watch   // a link, no QR code: no QR crate is in Cargo.lock
├── Paired watches (name, Last seen <age>)
│   ├── NeedsRepair -> "Pair it again to use it"; SecretsUnavailable -> "Waiting for the keychain"
│   ├── Remove… -> Cancel | Remove (two presses)       // Message::Revoke
│   │   └── the list could not be written -> nothing changes: the watch stays listed and served, its Keychain item stays,
│   │       and "Couldn't remove <name>: <the error>" shows under its row; Remove tries again
│   │       (errors are kept per watch id: another watch's Remove answering first or last never clears or replaces it)
│   └── none -> "No watches paired yet"
├── Status and paste                                    // watch.* settings, each applied at once
│   ├── Status capsule (on)            watch.status_capsule
│   ├── Follow the pointer (off)       watch.capsule_follows_pointer
│   └── Keep dictation on clipboard    watch.keep_on_clipboard  // Labs' switch still keeps every dictation
└── service not started -> "The Apple Watch service isn't running"

hex settings set watch.terminal_apps '["com.example.term"]'   // CLI only: extra terminal bundle ids (≤ 32, ≤ 255 characters each)

First run (macOS, the service has reported once)
├── model step / download -> Set up your Apple Watch -> demo -> permissions -> Almost ready -> practice -> Hex is ready
│   ├── Pair later (no Skip; it records nothing)       -> demo
│   └── a watch paired (not needing repair) -> Continue -> demo
├── a watch paired and no Microphone: Let Hex type for you (Accessibility only) -> no practice -> Hex is ready
└── resumed at setup: no watch paired -> Apple Watch page; paired -> permissions
    └── no snapshot yet -> permissions; a first snapshot within 1.5 s (no watch) -> Apple Watch page; a later one changes nothing
```

## Behavior

- **No Microphone needed (C1).** The listener starts once Accessibility and
  the model are ready (`SetupStatus::listener_ready`), so a Mac that never
  granted the Microphone still transcribes and pastes a watch recording. Until
  the model and Accessibility are ready, `ListenerDictation::transcribe`
  refuses a recording at once with the reason (`watch::listener_ready`), and
  the watch reads the existing 500 `failed`. The Mac microphone stays closed
  (`dictation::microphone_access`, held by the `DictationAudio` owner): the
  shortcut start itself is silent (Option+e and a click start the machine too);
  only an owned press (held past the threshold, Toggle's start after 40 ms, a
  lock), `hex dictation`, or `/dictations` is a try: macOS's prompt once, then
  the error tone on each try after a refusal and the Settings window once per
  launch. A granted permission restores the saved Release-while-idle policy
  after the first capture, which opens cold like a released microphone. A
  Mac-only user whose microphone was revoked after setup now meets this at the
  shortcut instead of a window at launch. Every real stream open logs
  `microphone stream opened`. Onboarding no longer needs it once a watch is
  paired: see "Settings And First Run".
- **Service.** `watch::service::WatchService` starts with the macOS app
  (`macos_rune::run`), on by default, no toggle. It reads or creates
  `relay-room`, `relay-secret`, and `device-id` in the data folder
  (`~/Library/Application Support/Wristper`; `Wristper Test` for a test build), and reads `paired-devices.json`
  with the device secrets in Keychain service `dev.wristper.mac.device` (`dev.wristper.mac.test.device` for a test build, so its pairings and migration marker never reach the installed app); files
  Wristper wrote are reused, so existing pairings keep working. Source:
  `src/watch/service.rs`, `src/watch/relay.rs`.
- **Relay.** One WebSocket to `wss://relay.wristper.dev/m/<room>` with the
  `X-Wristper-Mac-Secret` header; `ping` every 30 s; 60 s without a frame
  reconnects; backoff 1, 2, 4 … 30 s; another Mac taking the room (`replaced`)
  is logged and the client keeps retrying. TLS is the platform's; an HTTP
  proxy from `net::proxy_for` is used through CONNECT. `/health` reports
  `relay: connected|connecting|offline`.
- **Pairing.** Only 6-digit, E2EE pairings: the claim through the relay
  approves one device id for 120 s, `/pairing/request` needs a valid P-256
  public key (else 426 `watch_outdated`, approval kept), and the token leaves
  only sealed. Plaintext `/audio` is always 403 `e2ee-required`.
- **Delivery.** The m4a is decoded (`watch::audio`, symphonia) to 16 kHz mono
  and submitted as `TranscriptionTarget::Watch` on the running listener's
  `DictationWorker`: same engine, same ordered output, same Paster, the Output
  pane's processing, History kind Dictation, and Paste Last. Its inference
  is a foreground engine job, not yielding background work: a watch
  recording is the user's own dictation, so a Mac shortcut dictation
  released while it transcribes waits its turn in submission order, and
  capture itself is never blocked. The reply to
  `/e2ee` waits for the result up to 50 s, then answers `transcribing` so the
  relay's 60 s never 504s; the watch re-queries `/status`.
- **Idempotency.** The recording id is the key: in-flight duplicates wait up
  to 45 s; delivered ones answer from the live record, or from
  `delivered-ids.json` (24 h, 5000 entries) once the record is gone. Each
  entry Hex writes names its watch, so another watch gets neither a
  duplicate answer nor the status (404 `unknown-id`). Known limit: the old
  Swift app drops that `owner` key if it rewrites the file, which only
  happens when both apps run against one data folder; they share a bundle
  id, so installing Wristper replaces the Swift app. Audio
  marked `focus: requested` that beats its `/focus` waits 2 s, then proceeds
  and is withheld as `target-changed` with a reason (nothing is pasted without
  the field the watch started in).
- **What the watch reads.** `/status` has Wristper's field set (`ok, id,
  state, language, verified, verification, entered, updated_at, route,
  transcribe_ms, paste_ms, receive_to_paste_ms, reason, reset, paste_reason,
  error, completed_at, completed_age, agent_ms, session_id`, plus `text`,
  `raw_text`, and the `agent_*` nulls inside an envelope). `/health` reports
  `licensing` as licensed and unlimited until unit D.
- **Field snapshot.** `/focus` hands the read to the `watch-ax` thread
  (`watch::desktop`) and parks its reply; the answer is `captured:false` after
  1.5 s if the focused application does not answer. Each Accessibility element
  is bounded at 250 ms and the system-wide element is never used. The snapshot
  (pid, role, subrole, identifier, title, frame, window, bundle id, app name)
  lives on the live record, not on the wire. Audio with `focus: requested`
  that beat its `/focus` waits for the capture, not just the request.
  Source: `src/watch/desktop.rs`, `src/watch/screen.rs`,
  `src/accessibility.rs`, `src/watch/field.rs`.
- **Paste gate.** The output worker (`watch::output`) takes the keyboard lock,
  reads the focus, and decides with `field::gate`: pid and role must match;
  identifiers decide when both exist; else titles must not conflict and the
  frame must be within 48 px. A `focus: current` resend pastes at the cursor.
  Withheld and suspect texts stay on the watch's record; Paste Last (⌥⇧V)
  recovers a withheld text, a suspect one is never offered, and neither
  reaches History.
- **Read-back and clipboard.** After ⌘V a background thread looks for the
  text that went in (not the text sent: a continuation join may add a space or
  change case) in the element that was pasted into for 1 s; a change plus the words confirms
  (`pasted`, `read-back`). The clipboard comes back at once on confirmation,
  else after 4 s, through the same generation guard as every Hex paste (a
  newer paste or an outside change cancels it); an original carrying
  `ConcealedType` or `TransientType` is not restored. The keyboard lock is held
  until the read-back ends, so `/enter` waits for it.
- **Enter and Undo.** `watch::service::keys`: the policy is
  `recording::check_enter` and `check_undo`; the key goes through `watch-ax`,
  which compares the focus with the recording's field first (process, role, subrole,
  window, identifier or names plus an overlapping frame). A request that waited past its 5 s
  deadline is not pressed late. Return is held 30 ms: Electron terminals
  ignore an instant press.
- **Chinese characters.** Not watch-specific since unit G: the recording goes
  through `dictation::chinese_output` like every processed dictation
  (`s2tw`, `tw2s`, `s2hk`, `hk2s`; no phrase step). The recording's own
  language (`zh`, `yue`, `en`, …) picks the table when it names one; `auto`
  or empty falls back to the model's language. A setting of Simplified pastes
  Simplified; unset follows the system's language order (an Apple Watch
  user with a Traditional Chinese system still gets Traditional). What was
  heard is stored converted, so `raw` is `None` unless a step changed the text.
- **Suspect text.** `watch::suspect` runs on what was heard (before the Output
  pane's steps) with the system's preferred languages.
- **Status capsule.** `watch::status` maps the record to a capsule and its
  lifetime (Recording ≤ 180 s, Transcribing ≤ 60 s, results 1.2–5 s) in
  Traditional Chinese or English by the system's language order; the `watch`
  thread pushes each change and schedules the hide and the 1.4 s read-back
  grace as its own deadlines. `rune_ui::watch_status` draws a click-through
  pill at the bottom of the primary screen. It shows only after the field was
  read.
- **Settings (C2).** `app_settings::WatchSettings` (`watch` in
  `settings.json`; every OS reads it, only macOS acts on it). An older file
  without `watch` loads the defaults and is not rewritten; one bad field falls
  back alone (a bad `terminal_apps` list is dropped whole). `watch::settings`
  holds the applied values: the capsule setting is read each time the service
  publishes (`watch::status::publish` withholds a capsule and only hides);
  the screen follows the pointer when the capsule shows
  (`rune_hud::hud_placement::bottom_center`, from the pointer's `NSScreen`);
  `paste::Keep` is read as each paste commits (`Keep::Watch` adds the watch
  setting to Labs' switch; a Mac dictation never reads it); the terminal list
  is read at every gate (`field::is_terminal`). Every key has a
  `hex settings` key; each needs the `watch` capability.
- **Remove (C2b).** `PairedDevices::revoke` writes the list without the
  watch first. Only once it is written does Hex forget the watch in memory
  and delete its Keychain item. A list it cannot write changes nothing: the
  watch stays listed, served, and in the snapshot, with the error under its
  row, and Remove can try again. A Keychain item that cannot be deleted after
  the list is written is only logged: an item without a list entry never
  authenticates.
- **Window state (C2).** The `watch` thread publishes a `WatchSnapshot`
  (relay state, Mac name, `replaced`, devices) through `watch::view` when the
  relay state, `replaced`, a pairing, or a Remove changes; an unchanged one is
  not sent again, and a window opened later gets the newest at once. The
  macOS host (`rune_ui::watch_pane::host`) hands snapshots to the window and
  runs a claim or a Remove on a thread of its own (a claim waits up to 11 s on
  the relay); the answers come back as `WatchEvent`s in `PlatformState`.
  Linux and Windows hosts do nothing with the two requests.
- **Pairing in the window (C2d).** The pane numbers every claim
  (`GENERATION`, one counter for the process, so a reply that reaches a
  reopened window cannot match a new claim) and sends the number with the
  code; `hex watch pair` sends `NO_CLAIM` (0). The service keeps it with the
  claim, the approval it earns, and the admission: `PairingRequests::admit`
  records each request that spends the 120 s approval as an `Admission`
  (that claim's number and the entry it enrolled, which is an older one when
  `previous_token` named it). The snapshot carries the newest eight. A reply
  settles only the `Pairing` that sent that number and is dropped otherwise;
  the pane then waits for the admission with its own number and shows the
  enrolled watch's name. A pairing another claim earned (another watch, or an
  older window's claim of the same watch) settles nothing, and a watch paired
  again with its old token settles at once. A request that fails its
  key agreement or Keychain write records no admission, so the pane shows
  the timeout sentence after 120 s. The Mac cannot see the watch's own 426
  `watch_outdated`, which also shows that sentence.
- **Replies own only their request (C2d).** A Remove's reply names its watch
  and changes only that watch's error (`WatchView::remove_errors`, by id); a
  success clears its own entry only, and an entry for a watch that is no
  longer listed is dropped. Each Remove also carries a number (`REMOVAL`,
  process-wide); the pane keeps the newest sent for each watch and drops the
  answer to an older one, so whichever answer arrives last, the newest
  request decides what the row says. A claim's reply carries its number (above).
- **Health.** `/health` `paste_ready` is the Accessibility permission, with a
  `paste_detail` that names where to grant it.

## Checks

- `scripts/check watch` runs the protocol core (unit A) and this unit's tests:
  `watch::relay` against an in-process fake relay (handshake header and path,
  request and response frames, chunked bodies, pings, half-open detection,
  `replaced`, stop, CONNECT tunnel), `watch::audio` against real `afconvert`
  fixtures in `tests/fixtures/watch-audio/` (16 kHz and 44.1 kHz tones,
  hostile input), `watch::delivery`, and `watch::service` (every handler, the
  deferred replies at their deadlines, and the end-to-end integration test:
  claim, pairing with a real P-256 key, sealed `/focus`, sealed `/e2ee` with a
  real m4a through a fake transcriber, `/status` with the Swift field set,
  duplicate resend, plaintext `/status`, plaintext `/audio` refused).
- B2: `scripts/check watch` also covers `watch::field` (matching and the
  gate), `watch::desktop` (keys, lock, deadlines, with a scripted `Screen`),
  `watch::output` (every gate result, read-back, restore delays, suspect
  text), `watch::status`, `paste::deferred` (private
  clipboard items), and the service tests under
  `watch::service::tests::{field,keys,capsule}`.
- Live, by hand (they drive TextEdit, Terminal, and Finder, so they are
  `#[ignore]`): `cargo nextest run --locked --bin hex --run-ignored only -E
  'test(watch_live)'` from a terminal that has Accessibility and Automation
  access. They cover snapshot, paste, read-back, Enter, Undo, target change
  in TextEdit, and a terminal paste with Enter in Terminal.
- `control::` covers `watch-pair` on the wire and `unsupported` on Linux and
  Windows hosts; `platform::parity_tests` records the `watch` difference.
- C1: `dictation::microphone_access` covers the gate (a refused start
  records nothing, opens nothing, and says nothing; an intentional try asks
  once, then sounds; a grant restores the saved policy; a revoke closes the
  warm input) and `recognition::gated_press` drives the real shortcut machine
  with no permission (Option+e, click, drag, short tap, an early chord: no
  prompt, tone, or window; a hold past the threshold or Toggle's grace: one
  try) and `no_open_path_opens_the_microphone_without_permission` drives
  startup, settings reloads, route and device changes, selection changes,
  retry time, and every kind of start with a stream counter at zero.
  `dictation::audio` adds the interrupted-stream path and the one stated
  exception, Annotate's capture sink. `desktop::onboarding` and `macos_rune`
  pin the Mac-only behavior (`a_mac_only_*`) and the watch-only launch.
- C2: `app_settings::persistence` (an old file loads the defaults without a
  write; one bad field falls back alone), `control::settings` (each key sets
  its own field; bad `terminal_apps` lists are refused with a reason;
  `every_persisted_setting_has_a_cli_key`), `watch::field`
  (`a_custom_terminal_bundle_id_pastes_as_final`), `paste::tests`
  (`watch_keep_on_clipboard_applies_only_to_watch_recordings`),
  `watch::status::tests` (a turned-off capsule), `rune_hud::hud_placement`,
  `watch::service::tests::view` (a claim and the watch's request push the new
  device; a Remove that cannot write the list keeps the watch listed, served,
  and in the Keychain, and a retry removes it; a watch paired again with its
  old token pushes its new pairing time; the snapshot names which claim each
  admitted request spent, and a refused request records none; `replaced`),
  `rune_ui::watch_pane` (the pairing state machine and last-seen ages; the
  scenes' texts; six digits send one `WatchPair`; every claim error has its
  words; Remove takes two presses and shows the revoke error under the watch
  it kept; two Removes in flight, in either order, keep each watch's own
  outcome; a late reply to an older claim is dropped and another watch pairing
  meanwhile settles nothing; a watch paired again settles at once; a
  watch-only setup can Continue without a model and is not offered it while
  a download runs; every control has a name;
  the first run's page, Pair later, the watch-only permissions page and
  finish, the resumed setup with a snapshot inside or after the 1.5 s wait),
  `watch::pairing` (a Remove that cannot write changes nothing, also after a
  restart; a retry removes it), `desktop::i18n::apple_watch` (every string
  has one Taiwan-wording Traditional Chinese entry, and the zh-Hant table has
  none of the mainland words), and `desktop::onboarding`
  (`permission_warnings` leaves the Microphone out with a paired watch). The
  UI tests are plain `#[test]`s that run on macOS too; the goldens (the ten
  `mac-watch*` scenes, `mac-watch-revoke-error` included, and the three
  `mac-onboarding-*watch*` ones) compare on Linux only.
- Not verified: no real Apple Watch, no connection to the production relay,
  no TLS or proxy path (tests use `ws://127.0.0.1`), no end-to-end paste on a
  real Mac with the model loaded. A fixture recorded from a development watch
  is still to be added.

## Gaps And Constraints

- Known limit (C1, accepted): `listener_ready::current()` says Accessibility and
  the model are ready, but the listener thread starts on the runtime's next
  tick, after setup turns ready and after its own startup; no bound is
  measured. While the listener's queue (capacity 8) has a receiver, a
  recording that arrives in that gap is queued and waits until the listener
  reads it. If the queue is full or the receiver is gone, `try_send` fails at
  once and the recording is refused with the reason (500 `failed`), not
  queued. If a queued recording is still waiting at 50 s, the Mac watch
  service answers `transcribing` (its own reply deadline, not a wait on the
  watch); the watch then polls `/status`. No test drives that race.
- Known limit (C1, accepted): a press counts as owned when the event that ends
  it (the release, or a chord breaker such as a key joining a held
  modifier) is stamped at or after the owned moment, judged by that event's
  own timestamp inside the shortcut machine (`DictationHotkey::process`). So a
  release queued behind later events in one batch is judged by its own stamp:
  a tap stamped 100 ms is silent whatever else the batch holds, and one
  stamped 400 ms asks once. A release that arrives in a later batch than the
  clock tick that passed the owned moment is not rewound: the tick has
  already counted the press. A key joining a Hold press after the threshold
  is a try, as the tick would make it. HID key evidence that falls back to
  arrival time because its clock can't be read (`src/audio/instant.rs`)
  predates C1 and can order wrongly; C1 does not change it.
- A permission revoked while the microphone is warm is applied when a capture
  starts or ends or a setting reloads, and every open the input starts by
  itself (route, device-list, selection change, stream retry) asks the
  permission again first and is dropped when it is gone; the warm stream itself
  closes at the next such moment.
- Known limit (C1, accepted): the permission is read on the owner thread just
  before an open starts (`RecoveringAudioInput::start_replacement`, and at
  startup before the warm open), but the opener runs a moment later on its own
  thread. A revoke inside that gap still reaches macOS, which decides whether
  the stream opens. Not testable without a real Mac; it is on the real-Mac
  verification list (open after a revoke, the prompt timing, the log line).
- The live checks above have not been observed to pass in this unit's
  worktree: no real Apple Watch, no real Accessibility session, no Orca,
  Ghostty, Warp, or Claude Code and Codex inside a terminal yet. Linux and
  Windows compile through CI only; the whole feature is macOS.
- `/enter` and `/undo` need the live record, which Hex keeps 24 h; after a
  restart a recording reads `unknown-id` for both (the dedupe table keeps its
  state, not its field). A `focus: current` resend has no snapshot, so Enter
  for it reads `target-changed`, as in Wristper.
- The capsule is on the primary screen unless `watch.capsule_follows_pointer`
  is on (never exercised on a real multi-display Mac), and speaks only
  Traditional Chinese and English.
- Not in C2, each an open question: `hex watch list` and `revoke`; a pane
  editor for `terminal_apps`; a "Pair Apple Watch…" menu-bar item; a login
  item registered when onboarding finishes; the practice with a watch; a QR
  code; showing the watch's 426 `watch_outdated` on the Mac; licensing and the
  quota (units D, E); the capsule in other languages; Windows and Linux.
- A paired watch's "Last seen" is as old as the last pushed snapshot: a
  request from the watch does not push one (`PairedDevices::touch` is silent),
  so the age moves only when the relay, a pairing, or a Remove changes.
- C2's own code that has not run on a real Mac: where the capsule lands with
  Follow the pointer on (and the default placement coming back when it is
  turned off), and the threads that run a claim and a Remove. The Apple Watch
  strings in Traditional Chinese are written in Taiwan wording
  (`apple_watch::TRADITIONAL`), not converted, and are not reviewed by a
  native speaker; the other languages' strings have no
  native review either.
- Not observed on a real Mac: whether listing devices without opening a
  stream raises the Microphone prompt, `requestAccess`' timing, the first key
  after a grant, and the path that opens Settings after a refusal (C1's list),
  nor the resumed-setup decision, which waits up to 1.5 s for the service's
  first snapshot.
- Field matching is a best effort over what Accessibility exposes, not proof
  of identity. Paste: same process and role, the identifier when both sides
  have one (and then the window rectangle must agree too, or the field's frame
  when a window cannot be read), else titles that do not conflict and a frame
  within 48 points. Enter and Undo are stricter than Wristper's
  `sameFieldAfterTyping`: same process, role, subrole, and window rectangle
  (both readable), the identifier, and without one the title, description,
  and role description plus a frame that still overlaps the original. Known
  limits, not excluded: two windows of one app with the same position and size
  (within 48 points) and the same identifiers or names look identical, as do
  two tabs of one terminal window, two fields of one window with the same
  names and overlapping frames, and a window moved by less than the
  tolerance. A terminal whose prompt frame jumps clear of the original reads
  `target-changed`. The Terminal, Orca, and Ghostty live check shows how often
  these happen.
- Focus is judged more than once, and windows remain. Paste: the gate reads
  and judges the focus and the field's text; then inside the Paster, after
  the clipboard snapshot and write (the slow steps, tens of ms, hundreds if
  the clipboard holds large data), a last comparison reads the focus again
  and runs immediately before ⌘V. A switch that happened before that read
  completes sends nothing and the clipboard goes back as it was (also with
  Labs › Keep dictation on clipboard: with ⌘V never sent, the earlier
  clipboard returns unless something else changed it meanwhile). Keys
  (Return, ⌘Z): the focus is read and judged twice, and the deadline and the
  ticket are settled after both reads, right before the key. Not closed, for
  paste and keys alike: the time between the last focus read finishing and
  the key event being posted (the read is not atomic with the post; the
  user can switch in that gap), and from the event being posted until the
  target handles it (a few ms, longer if the app is busy); the key then lands
  in the new app. Accessibility cannot pin the target, so the live check has
  to size these; they are not excluded.
- A key's claim is settled atomically: the `watch-ax` thread claims it right
  before the key event, the service cancels it on timeout, a restart of the
  recording, or shutdown. If the key already went out when the service gives
  up waiting for the answer, the record counts it as pressed (Enter
  `entered`, Undo `undone`) without confirmation and a retry is refused,
  never pressed a second time.
- The clipboard restore has a window, as in Wristper's Swift version: it
  checks the generation and the pasteboard `changeCount`, then writes the old
  contents back, and a copy the user makes between that check and the write
  is overwritten by the restored clipboard (no atomic compare-and-write
  exists on `NSPasteboard`). The paste's own write and the restore both run
  under the same lock, so only an outside copy in those microseconds loses.
- The read-back keeps the AX element the paste went into and reads only that
  element, as Wristper does with its first candidate; it does not also read
  whatever has focus later. An app that replaces the field object after ⌘V
  (some Electron apps, possibly Orca) therefore stays `unverified`, which
  also keeps Undo closed. The live Orca and Slack check will show how often.
- `/health`'s `model`, `model_id`, and `model_name` follow the saved
  transcription selection, not whether the model is loaded.
