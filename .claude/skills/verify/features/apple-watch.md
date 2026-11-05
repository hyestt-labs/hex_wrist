# Apple Watch dictation

A person pairs the Wristper watch app with this Mac once. After that, each watch recording is transcribed on the Mac and pasted at the Mac's cursor.

## Sub-features

- `watch.pair`: the watch shows a 6-digit code, and `hex watch pair <code>` claims it. The watch then gets a sealed token over P-256.
- `watch.dictate`: an encrypted `/focus` then `/e2ee` audio, transcribed and pasted at the cursor.
- `watch.status`: the encrypted `/status` reaches a terminal state with the transcript.
- `watch.enter`: the watch's Enter button. It is pending until B2, and B1 answers `not-supported`.
- `watch.resend`: the same recording id again gets a duplicate answer. Only the Rust tests cover it (`scripts/check watch::service`).

## How to get to it (user POV)

- On the watch: Pair Mac shows a code. On the Mac: `hex watch pair <code>`. The Settings pane arrives in unit C.
- On the watch: tap record, speak, and stop. The text appears where the Mac's cursor is.
- On the watch: tap the Enter button on the result.

## Driving it with verify-watch-e2e.sh

Preconditions: the baseline in [README.md](./README.md).

- **Pair (stage 1).** Run `scripts/verify-watch-e2e.sh --skip-build --stage 1`. `stage1-pair.jsonl` shows the events `code`, `claimed`, and `paired` with `encrypted: true`, and the table row `1 pairing` is PASS.
- **Pair (stage 2).** Run `scripts/verify-watch-e2e.sh --skip-build --stage 2`. `stage2-watch.log` shows `E2E pair-code NNNNNN`, then `E2E paired … encrypted=true`.
- **Dictate into TextEdit.** Each stage's `textedit en` and `textedit zh` rows are PASS. Their read-back column holds the sentence.
- **Dictate into Terminal.** The `terminal en` and `terminal zh` rows are PASS. Their read-back holds the sentence that `cat` echoed.
- **Status.** In stage 1 `results.jsonl`, `detail.status` is the decrypted `/status` with `state`, `text`, and `transcribe_ms`.
- **Enter.** In stage 1 terminal rows, `detail.enter` shows the watch-side Enter result. In stage 2, `E2E enter outcome=…` shows it. Until B2, expect a refusal.

## Gotchas

- An English-only model turns the Mandarin case into a FAIL, so the script selects `WATCH_E2E_ZH_MODEL` for `zh`.
- Anything that steals focus during a delivery puts the text elsewhere. Examples are a notification, the Wristper Test onboarding window, or Simulator.app being brought forward. Keep the Mac idle while it runs.
- B1 pastes at the current cursor even when `/focus` arrives late. It never reports `target-changed`.
- Each `/pair/start` uses the relay's per-IP budget, which is 20 per 10 minutes. Do not loop the pairing step.
- The real-watch flow pairs to the test Mac's own room. It never affects the installed app's pairing.
