# Apple Watch (Wristper)

Dictate on the Apple Watch with the Wristper watch app; this Mac receives the
recording through the Cloudflare relay, transcribes it with the dictation
model, and pastes the text at the current cursor. The watch app does not
change: the Mac speaks Wristper protocol 4 byte for byte
(`artifacts/wristper-migration-20261104/wristper-protocol.md` is the contract,
with Swift line references). macOS only in this unit; Linux and Windows carry
`watch` as a pending difference in `rune_ui::platform::DIFFERENCES`.

Status: unit B1 (relay, pairing, delivery, `/status`). Unit B2 adds the
accessibility field snapshot and read-back, Enter, Undo, the suspect-text
rules, and the HUD; unit C the Settings pane; unit D licensing and the free
quota.

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
│   ├── Watch starts recording -> /e2ee/query {/focus}   -> record awaiting-audio
│   ├── Watch stops -> /e2ee (AudioHead + m4a)           -> decode -> DictationWorker
│   │   ├── pasted through the one Paster at the current cursor -> unverified
│   │   ├── Labs › Copy dictation only on                -> copied
│   │   ├── empty transcript                             -> suspect
│   │   └── decode, inference, or paste failed           -> 500 failed
│   └── Watch 1.5 s later: /e2ee/query {/status}         -> the same fields as Wristper
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
  pane's processing, History kind Dictation, and Paste Last. The reply to
  `/e2ee` waits for the result up to 50 s, then answers `transcribing` so the
  relay's 60 s never 504s; the watch re-queries `/status`.
- **Idempotency.** The recording id is the key: in-flight duplicates wait up
  to 45 s; delivered ones answer from the live record, or from
  `delivered-ids.json` (24 h, 5000 entries) once the record is gone. Audio
  marked `focus: requested` that beats its `/focus` waits 2 s, then proceeds
  (B1 pastes at the current cursor either way; B2 withholds as
  `target-changed` once snapshots exist).
- **What the watch reads.** `/status` has Wristper's field set (`ok, id,
  state, language, verified, verification, entered, updated_at, route,
  transcribe_ms, paste_ms, receive_to_paste_ms, reason, reset, paste_reason,
  error, completed_at, completed_age, agent_ms, session_id`, plus `text`,
  `raw_text`, and the `agent_*` nulls inside an envelope). `/health` reports
  `licensing` as licensed and unlimited until unit D.
- **Not yet (B2).** No accessibility snapshot (`/focus` answers
  `captured:false`), no read-back (so no `pasted`), `/enter` and `/undo`
  answer `{"ok":false,"reason":"not-supported"}`, no HUD state, no suspect
  rules beyond the empty transcript, no Traditional Chinese conversion.

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
- `control::` covers `watch-pair` on the wire and `unsupported` on Linux and
  Windows hosts; `platform::parity_tests` records the `watch` difference.
- Not verified: no real Apple Watch, no connection to the production relay,
  no TLS or proxy path (tests use `ws://127.0.0.1`), no end-to-end paste on a
  real Mac with the model loaded. A fixture recorded from a development watch
  is still to be added.

## Gaps And Constraints

- The watch's `captured:false` means the Mac did not lock a field; Wristper
  pastes anyway and so does B1.
- `paste_ready` in `/health` is always true in B1; it should follow the
  Accessibility permission once the Settings pane exists.
- `/health`'s `model`, `model_id`, and `model_name` follow the saved
  transcription selection, not whether the model is loaded.
