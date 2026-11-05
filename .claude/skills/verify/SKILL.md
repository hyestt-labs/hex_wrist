---
name: verify
description: Drive the Wristper Mac app (this Hex fork) end to end on a real Mac to prove Apple Watch dictation works. A watch, either the protocol simulator or the real watch app in a watchOS Simulator, pairs through the production relay and sends recordings, and the text must appear in TextEdit, Terminal, and Chrome, with Enter, Undo, and the clipboard checked too. Use it to verify any change to src/watch/, pairing, delivery, paste, or the test build. Also use it when someone asks whether the watch works.
---

# Verify Wristper on a real Mac

`scripts/verify-watch-e2e.sh --all` is the one command. It builds `Wristper Test.app`, launches it, pairs a watch through `https://relay.wristper.dev`, sends spoken recordings, reads the text back from the app that had focus, and prints one PASS/FAIL table. It runs without a human once the one-time permissions are granted. The feature recipes are in [features/README.md](features/README.md).

The script runs four stages:

1. **Protocol watch.** `watch-sim` is a macOS command-line watch compiled from the shipping watch Swift sources (`scripts/watch-sim/build.sh`). It pairs, then sends English and Mandarin into TextEdit and Terminal.
2. **Real watch app.** A DEBUG build from the `test/sim-fixture-audio` branch runs in a watchOS Simulator named `Wristper E2E`. Launch arguments make it pair and record without taps, with a fixture m4a instead of the microphone. It runs the same cases as stage 1.
3. **Scenarios no unit test covers**, through `watch-sim`:
   - `clipboard-keep-gate-switch`: Labs › Keep dictation on clipboard is on, the clipboard holds a sentinel, and the script switches to Finder after the recording starts and before its audio arrives. The first gate withholds the dictation as `target-changed`, nothing may reach the document, and the clipboard must still hold the sentinel.
   - `clipboard-keep-paste`: the same with no switch. The dictation stays on the clipboard.
   - `clipboard-default-restore`: keep is off. The sentinel comes back after the paste.
   - `textedit-enter`, `textedit-undo`: Return is typed once. Undo removes the text.
   - `cat-enter`, `cat-undo`: in Terminal running `cat`, Return makes `cat` echo the line, and Undo is refused.
   - `shell-enter`: in a plain zsh, Return runs the line once (`command not found` appears once).
   - `enter-twice-textedit`, `enter-twice-shell`: two `/enter` calls at the same moment type Return once.
   - `chrome-textarea`: a throwaway incognito `data:text/html` page with one textarea. The text is pasted and read back through AppleScript JavaScript.
   - `agent-claude`, `agent-codex` (only with `--with-agents`): an interactive agent in Terminal receives "Reply with only the word pong." and Return submits it, proven by the agent's reply.
4. **B2 live tests.** `cargo nextest run --locked --bin hex --run-ignored only -E 'test(watch_live)' -j 1`. They read the focused field through Accessibility in TextEdit, Terminal, and Finder.

**Gap.** `clipboard-keep-gate-switch` reaches only the first gate, so the clipboard is never written and the sentinel survives even if the restore code were broken. It does not prove the restore in the final focus check (a switch after the clipboard write, right before ⌘V, with keep on; commit e0e36e16). `/status` reports both refusals as `target-changed`, and a script cannot time a window of tens of milliseconds. That restore has no automated test (report-B2-fixes3 item 3). The keep-off variant has the unit test `watch::output::tests::a_switch_during_the_clipboard_write_sends_no_paste`. To check the keep-on restore by hand, turn on Labs › Keep dictation on clipboard, put text on the clipboard, start a watch recording in TextEdit, and switch app at the moment the audio arrives, many times; the old clipboard must come back whenever the paste was withheld.

## Safety rules

- Never touch `/Applications/Wristper.app`, `~/Library/Application Support/Wristper`, or its relay room.
- `--with-agents` skips any `claude` or `codex` under `~/.wristper/` (the installed Wristper's wrappers) and uses the first other one on PATH.
- Never touch the Keychain items of service `dev.wristper.mac.device*`. The test build uses `dev.wristper.mac.test.device`.
- Everything runs under `Wristper Test` (bundle `dev.wristper.mac.test`, data folder `~/Library/Application Support/Wristper Test`, its own relay room).
- The script never tries to grant a permission or to unlock the screen.
- Do not deploy the relay.
- Never delete with `rm`. Use `/usr/bin/trash`.
- The scenarios overwrite the Mac's clipboard and change Labs › Keep dictation on clipboard on the test app. The script restores that setting when it exits. It cannot restore the clipboard.

## Launch (one command)

Run it from the repository root, with no TTY needed:

```sh
scripts/verify-watch-e2e.sh --all                 # build everything, all four stages (the default)
scripts/verify-watch-e2e.sh --all --skip-build    # reuse target/app, watch-sim, and the watch build
scripts/verify-watch-e2e.sh --preflight-only      # list every missing click, change nothing else
scripts/verify-watch-e2e.sh --stage 1             # one stage: 1, 2, 3, or 4
scripts/verify-watch-e2e.sh --stage 3 --only 'enter|clipboard'    # scenarios whose name matches
scripts/verify-watch-e2e.sh --stage 2 --targets textedit --langs en
scripts/verify-watch-e2e.sh --all --with-agents   # also claude and codex in Terminal
```

The script uses these exit codes:

- `0` means every case passed.
- `1` means a case failed.
- `2` means bad usage or a missing tool or source tree.
- `3` means it is blocked on a human. The output prints a `BLOCKED` block that lists every missing click in one go. The causes are a locked screen, Microphone or Accessibility for Wristper Test, Automation for the app that runs the script (to control TextEdit, Terminal, Google Chrome, and System Events), Accessibility for that app (stage 4), and Chrome's "Allow JavaScript from Apple Events".

The last line is `RESULT: PASS` or `RESULT: FAIL`. Before it, a table prints stage, scenario, language, result, match %, state, milliseconds, and the text read back. A failing row has a `└` line with the reasons.

Without a build step the script needs about 15 minutes. The first Mandarin run also downloads a model (the script waits up to 60 minutes).

You can set these environment variables:

- `WRISTPER_SRC` is the Wristper checkout that watch-sim compiles from. It is read only. The default is `/Users/glenhsu/orca/workspaces/wristper/coding`.
- `WRISTPER_HOOKS_SRC` is the watch app with the E2E hooks. The default is `/Users/glenhsu/orca/workspaces/wristper/sim-hooks`, the worktree of branch `test/sim-fixture-audio`.
- `WATCH_E2E_B2_ENV` is the file that links the Rust test binary on this Mac (stage 4). The default is `.../migration_to_hex/artifacts/wristper-migration-20261104/b2-test-env/env.sh`.
- `WATCH_E2E_EN_MODEL` and `WATCH_E2E_ZH_MODEL` set the model for each language. The defaults are `parakeet_unified_en` and `nemotron_3_5_asr`. The watch sends language `auto`, so the Mac transcribes with the selected model and language. The script selects the model before each language with `hex model install <id> --language <lang>`.
- `WATCH_E2E_EN_PASS` and `WATCH_E2E_ZH_PASS` set the pass thresholds in percent. The defaults are 80 and 60.
- `WATCH_E2E_AGENT_DIR` is the folder where `--with-agents` starts claude and codex. The default is the repository. The agent must already trust that folder, or its trust prompt eats the first Return.
- `SIGN_IDENTITY` is the identity the app is re-signed with. The default is `Developer ID Application: WEI CHIEH HSU (VQY3J86U79)`. `build-app.sh` signs a test build ad hoc. An ad hoc identity changes with every build, so macOS would drop the permissions after each rebuild. Re-signing keeps one identity.

## For Terra (codex exec)

The user must be at the Mac with the screen unlocked, or have run the one-time steps below, and the Mac must stay idle during the run. Run it like this, and do not run anything interactive:

```sh
codex exec -m gpt-6.1-terra --sandbox danger-full-access \
  'Run this with bash, nothing else, and wait for it to finish: bash -c "cd /Users/glenhsu/orca/workspaces/hex_wrist/verify && export DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer && scripts/verify-watch-e2e.sh --all > /tmp/watch-e2e.txt 2>&1; echo exit=\$?; tail -60 /tmp/watch-e2e.txt". Then follow .claude/skills/verify/SKILL.md section "For Terra" and report the table, the exit code, and the run folder. Never touch /Applications/Wristper.app or ~/Library/Application Support/Wristper. Delete only with /usr/bin/trash.'
```

The script runs under `bash -c` on purpose: the exit code must come from the script itself, and `PIPESTATUS` does not exist in zsh.

1. If the exit code is `3`, stop and paste the whole `BLOCKED` block to the user. A human must click. Do not try to grant any permission, unlock the screen, or edit TCC.
2. If the exit code is `1`, open `target/watch-e2e/<run>/results.jsonl`, `run.log`, `stage4-live.log`, and report each FAIL row with its `detail`.
3. On a pass, report the table, the run folder, and the `RESULT` line.
4. Do not edit `src/` to make a case pass. Report the failure instead.
5. Add `--with-agents` only when the user asks. It spends tokens on their claude and codex logins.

## One-time manual steps (a human, on this Mac)

The listener turns watch audio into pasted text. It starts only after Microphone, Accessibility, and a speech model are all ready (`desktop::onboarding::SetupStatus::ready`). Watch-only use still needs the Microphone grant until unit C lands.

1. Run `scripts/verify-watch-e2e.sh --preflight-only`. It exits with `3` and lists the exact steps for this Mac.
2. Run `"target/app/Wristper Test.app/Contents/MacOS/hex" window show`. In the setup page, click Allow for Microphone.
3. In System Settings > Privacy & Security > Accessibility, turn on Wristper Test. If it is not listed, add `target/app/Wristper Test.app`. Turn on the app that runs the script too (Terminal, Orca, iTerm) for stage 4.
4. The first time, macOS asks whether the app that runs the script may control TextEdit, Terminal, System Events, and Google Chrome (Automation). Click OK each time.
5. In Google Chrome, choose View > Developer > Allow JavaScript from Apple Events. This changes the user's real Chrome until they turn it off again.
6. Run the script again with `--skip-build`.

The grants survive rebuilds only while the Developer ID re-sign succeeds. The script logs a WARNING when it does not.

## Doctor

Run these read-only checks first when anything looks off:

```sh
H="target/app/Wristper Test.app/Contents/MacOS/hex"
"$H" status --json | jq '{permissions, listener, model}'
cat ~/Library/Application\ Support/Wristper\ Test/relay-room | cut -c1-6
tail -50 ~/Library/Application\ Support/Wristper\ Test/logs/process.log | grep -i watch
ioreg -n Root -d1 | grep IOConsoleLocked
```

- `permissions.ready` must be `true`.
- `listener.running` must be `true`.
- `process.log` should show the relay as connected.
- `IOConsoleLocked` must not read `Yes`.
- `"$H" status` must answer. If it does not, Wristper Test is not running. Start it with `open -g "target/app/Wristper Test.app"`.

## Evidence (what counts as proof)

A case is proof only when the text read back from TextEdit, Terminal, or Chrome matches the spoken sentence. Matching ignores case, punctuation, spacing, and Traditional versus Simplified characters (`watch-sim match`). A 200 reply or a `state` of `unverified` alone is not proof.

Each run writes `target/watch-e2e/<YYYYmmdd-HHMMSS>/`, and cleanup never removes it. The folder holds these files:

- `results.jsonl` has one row per case. For `watch-sim` cases, `detail` holds the decrypted `/status` JSON, `send_ms`, `transcribe_ms`, `total_ms`, the raw `/enter`, `/undo`, and double-Enter replies, and `fails`, the reasons of a FAIL. For stage 2, it holds the watch app's `E2E result` log line.
- `run.log` has the timeline.
- `stage1-pair.jsonl` and `stage2-watch.log` hold the pairing evidence (code, claim, paired).
- `stage4-live.log` holds the nextest output.
- `readback.txt` is the last read-back, raw.
- `en.m4a`, `zh.m4a`, and `agent.m4a` are the fixtures. They are 16 kHz mono AAC at 32 kbps, which is the watch recorder's setting in `WatchProbe/ProbeModel.swift`.
- The `build-*.log` files are the build logs.

## Real-watch check (optional, a human with a watch)

1. Start the test app with `open -g "target/app/Wristper Test.app"`.
2. On the watch, open Wristper and choose Pair Mac. The watch shows a 6-digit code.
3. Run `"target/app/Wristper Test.app/Contents/MacOS/hex" watch pair <code>`. The watch finishes pairing by itself.
4. Click into TextEdit, then record on the watch. The text must appear at the cursor.

The watch can hold several Macs. Pick the test Mac on the watch while you test, then switch back.

## Cleanup

The script's exit trap restores Labs › Keep dictation on clipboard, quits Wristper Test (`hex quit`), and shuts down the `Wristper E2E` simulator. It does these unless you pass `--keep-running` or the run is blocked. Each case closes the TextEdit document, Terminal window, or Chrome window it opened. It never kills by process name across the system. It stops `cat`, `claude`, and `codex` only on the tty of its own Terminal window. To remove a stale simulator, run `xcrun simctl delete "Wristper E2E"`. To start over from a fresh test data folder, run `/usr/bin/trash ~/Library/Application\ Support/Wristper\ Test`. That also loses the test room and the downloaded test models.

## Helpers

- `scripts/watch-sim/build.sh [out]` builds `target/watch-sim/watch-sim`.
- `watch-sim pair [--timeout s]` prints JSON lines. Lines with `event` set to `code`, `claimed`, and `paired` appear in that order.
- `watch-sim deliver --audio <m4a> [--language auto] [--after-focus <sh>] [--enter] [--enter-twice] [--undo] [--timeout s]` sends the encrypted `/focus`, runs the optional shell command (the app switch), then sends the encrypted `/e2ee` with a fresh UUID. It polls the encrypted `/status` until the state is terminal. Then it sends `/enter` once, `/enter` twice at the same moment, or `/undo`, and keeps the Mac's reply verbatim. Set `WATCH_SIM_STATE=<dir>` to keep the watch's pairing in that folder.
- `watch-sim action --path /enter|/undo --id <uuid>` sends one action for an earlier recording.
- `watch-sim match --expected <text> --actual <text>` prints `{"percent":N}`.
- `watch-sim health` sends `/health` with the stored token.
