# Apple Watch dictation

A person pairs the Wristper watch app with this Mac once. After that, each watch recording is transcribed on the Mac and pasted at the Mac's cursor.

## Sub-features

- `watch.pair`: the watch shows a 6-digit code, and `hex watch pair <code>` claims it. The watch then gets a sealed token over P-256.
- `watch.dictate`: an encrypted `/focus` then `/e2ee` audio, transcribed and pasted at the cursor.
- `watch.status`: the encrypted `/status` reaches a terminal state with the transcript.
- `watch.enter`: the watch's Enter button. Return goes to the same field once, also when two `/enter` calls arrive together.
- `watch.undo`: the watch's Undo button. It removes a pasted text in TextEdit and is refused in a terminal.
- `watch.clipboard`: the clipboard before the dictation comes back, or with Labs › Keep dictation on clipboard the dictation stays and a withheld one leaves the old clipboard alone.
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
- **Enter and Undo.** Run `scripts/verify-watch-e2e.sh --skip-build --stage 3 --only 'enter|undo'`. The rows `textedit-enter`, `textedit-undo`, `cat-enter`, `cat-undo`, `shell-enter`, and `enter-twice-*` are PASS. `detail.enter`, `detail.enters`, and `detail.undo` hold the Mac's raw replies. In stage 2, `E2E enter outcome=…` shows the real watch app's result.
- **Clipboard.** Run `scripts/verify-watch-e2e.sh --skip-build --stage 3 --only clipboard`. The three `clipboard-*` rows are PASS (`clipboard-keep-gate-switch` covers the first gate only; see the gap in SKILL.md). `detail.fails` names the clipboard text that was found when one fails.
- **Chromium.** Run `scripts/verify-watch-e2e.sh --skip-build --stage 3 --only chrome`. The `chrome-textarea` rows read the text back from a throwaway page in a separate Chrome process with an empty profile.
- **Agents (opt in).** Add `--with-agents`. The `agent-claude` and `agent-codex` rows need those tools logged in and the folder trusted.
- **Live tests (B2).** Opt in: `scripts/verify-watch-e2e.sh --skip-build --stage 4`. Each test makes its own TextEdit document or Terminal window, records its window id (and the document name or tty), and closes only that window by id after checking it is still the one it made. Not part of `--all` because they send real keys and have not yet been seen to pass on a real Mac. **Off for now:** the script refuses stage 4 (BLOCKED, exit 3), because these tests still open untitled TextEdit documents, which TextEdit autosaves into iCloud Drive where Trash cannot remove them; they must first open a file in a temp folder, as stages 1 to 3 do. The Rust tests refuse too when run by hand (`cargo nextest`): the TextEdit one always, the Terminal one without the script's one-time token. Terminal also needs `--check-terminal` first. One row per `watch_live_*` test.

## Gotchas

- An English-only model turns the Mandarin case into a FAIL, so the script selects `WATCH_E2E_ZH_MODEL` for `zh`.
- Anything that steals focus during a delivery puts the text elsewhere. Examples are a notification, the Wristper Test onboarding window, TextEdit's Open panel on its first launch, or Simulator.app being brought forward. Keep the Mac idle and unlocked while it runs.
- A locked screen blocks the whole run (exit 3). The script never unlocks it. A Wristper Test that is already running also blocks it (exit 3): the script never stops or resets an app it did not start.
- Run `scripts/verify-watch-e2e.sh --selftest` after changing the script: it checks the ownership gate and the no-audio-on-focus-failure rule with stand-ins, with no app or window.
- With B2, audio that beats its `/focus` by more than 2 s, or a `/focus` that read nothing, is withheld as `target-changed`. It is not pasted at the cursor.
- The clipboard scenarios cover an app switch before the audio arrives. A switch inside the few tens of milliseconds between the clipboard write and ⌘V is not covered when Keep dictation on clipboard is on (no test at all), and has only a unit test when it is off.
- Each `/pair/start` uses the relay's per-IP budget, which is 20 per 10 minutes. Do not loop the pairing step.
- The real-watch flow pairs to the test Mac's own room. It never affects the installed app's pairing.
