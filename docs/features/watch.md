# Apple Watch (Wristper)

Dictate on the Apple Watch with the Wristper watch app; this Mac receives the
recording through the Cloudflare relay, transcribes it with the dictation
model, and pastes the text at the current cursor. The watch app does not
change: the Mac speaks Wristper protocol 4 byte for byte
(`artifacts/wristper-migration-20261104/wristper-protocol.md` is the contract,
with Swift line references). macOS only in this unit; Linux and Windows carry
`watch` as a pending difference in `rune_ui::platform::DIFFERENCES`.

Status: units B1 (relay, pairing, delivery, `/status`) and B2 (field
snapshot, paste gate, read-back, Enter, Undo, suspect text, Traditional
Chinese, status capsule). Unit C adds the Settings pane; unit D licensing and
the free quota.

## How To Get To It

```ts
Apple Watch                               // watch.pair, watch.dictate
├── Pair
│   ├── Watch: Pair a Mac -> shows a 6-digit code        // the watch asks the relay
│   ├── Mac: hex watch pair <code>                       // B1's only entry; C adds the pane
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
│   │   ├── Traditional Chinese (zh or auto)             -> s2tw, watch recordings only
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

## Behavior

- **Service.** `watch::service::WatchService` starts with the macOS app
  (`macos_rune::run`), on by default, no toggle. It reads or creates
  `relay-room`, `relay-secret`, and `device-id` in the data folder
  (`~/Library/Application Support/Wristper`), and reads `paired-devices.json`
  with the device secrets in Keychain service `dev.wristper.mac.device`; files
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
  change case) in the focused field for 1 s; a change plus the words confirms
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
- **Traditional Chinese.** `watch::zh`: `ferrous-opencc` 0.2 `s2tw`
  (Taiwan characters, no phrase step), only for `zh` or auto recordings, only
  for the watch. Other dictation is unchanged.
- **Suspect text.** `watch::suspect` runs on what was heard (before the Output
  pane's steps) with the system's preferred languages.
- **Status capsule.** `watch::status` maps the record to a capsule and its
  lifetime (Recording ≤ 180 s, Transcribing ≤ 60 s, results 1.2–5 s) in
  Traditional Chinese or English by the system's language order; the `watch`
  thread pushes each change and schedules the hide and the 1.4 s read-back
  grace as its own deadlines. `rune_ui::watch_status` draws a click-through
  pill at the bottom of the primary screen. It shows only after the field was
  read.
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
  `watch::output` (every gate result, read-back, restore delays, Traditional
  Chinese, suspect text), `watch::status`, `paste::deferred` (private
  clipboard items), and the service tests under
  `watch::service::tests::{field,keys,capsule}`.
- Live, by hand (they drive TextEdit, Terminal, and Finder, so they are
  `#[ignore]`): `cargo nextest run --locked --bin hex --run-ignored only -E
  'test(watch_live)'` from a terminal that has Accessibility and Automation
  access. They cover snapshot, paste, read-back, Enter, Undo, target change
  in TextEdit, and a terminal paste with Enter in Terminal.
- `control::` covers `watch-pair` on the wire and `unsupported` on Linux and
  Windows hosts; `platform::parity_tests` records the `watch` difference.
- Not verified: no real Apple Watch, no connection to the production relay,
  no TLS or proxy path (tests use `ws://127.0.0.1`), no end-to-end paste on a
  real Mac with the model loaded. A fixture recorded from a development watch
  is still to be added.

## Gaps And Constraints

- The live checks above have not been observed to pass in this unit's
  worktree: no real Apple Watch, no real Accessibility session, no Orca,
  Ghostty, Warp, or Claude Code and Codex inside a terminal yet. Linux and
  Windows compile through CI only; the whole feature is macOS.
- `/enter` and `/undo` need the live record, which Hex keeps 24 h; after a
  restart a recording reads `unknown-id` for both (the dedupe table keeps its
  state, not its field). A `focus: current` resend has no snapshot, so Enter
  for it reads `target-changed`, as in Wristper.
- The capsule is on the primary screen, not on the one under the pointer, and
  speaks only Traditional Chinese and English.
- Enter and Undo compare stricter than Wristper's `sameFieldAfterTyping`:
  same process, role, subrole, and window (both windows must be readable),
  the identifier, and without one the title, description, and role
  description plus a frame that still overlaps the original. A terminal whose
  prompt frame jumps clear of the original, or whose tab switch changes only
  what AX does not expose, reads `target-changed` or goes to the other tab;
  this needs the live Terminal, Orca, and Ghostty check.
- A small race remains by design: the focus is read and judged twice (the
  second time right before ⌘V, Return, or ⌘Z), the key's 5 s deadline and
  cancellation are checked after both reads, but Accessibility cannot pin the
  target, so a switch of app inside those last microseconds still sends the
  key to the new app. Needs the live check to size it.
- The read-back reads the focused element of the app in front, not the
  original element object Wristper kept; an app that moves focus inside its
  own window right after ⌘V stays `unverified`.
- `/health`'s `model`, `model_id`, and `model_name` follow the saved
  transcription selection, not whether the model is loaded.
