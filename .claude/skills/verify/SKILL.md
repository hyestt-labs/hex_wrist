---
name: verify
description: Drive the Wristper Mac app (this Hex fork) end to end on a real Mac to prove Apple Watch dictation works. A watch, either the protocol simulator or the real watch app in a watchOS Simulator, pairs through the production relay and sends recordings, and the text must appear in TextEdit and Terminal. Use it to verify any change to src/watch/, pairing, delivery, paste, or the test build. Also use it when someone asks whether the watch works.
---

# Verify Wristper on a real Mac

`scripts/verify-watch-e2e.sh` builds `Wristper Test.app`, launches it, pairs a watch through `https://relay.wristper.dev`, and sends spoken recordings. TextEdit or Terminal (running `cat`) has focus during each delivery. The script reads that app's text back and judges it. It runs without a human once the one-time permissions are granted. The feature recipes are in [features/README.md](features/README.md).

- Stage 1 uses `watch-sim`. It is a macOS command-line watch compiled from the shipping watch Swift sources (`scripts/watch-sim/build.sh`). It is fast and deterministic.
- Stage 2 uses the real watch app. It is a DEBUG build from the `test/sim-fixture-audio` branch, running in a watchOS Simulator named `Wristper E2E`. Launch arguments make it pair and record without taps. It records a fixture m4a instead of the microphone.

## Safety rules

- Never touch `/Applications/Wristper.app`, `~/Library/Application Support/Wristper`, or its relay room.
- Never touch the Keychain items of service `dev.wristper.mac.device`. The test build uses `dev.wristper.mac.test.device`.
- Everything runs under `Wristper Test` (bundle `dev.wristper.mac.test`, data folder `~/Library/Application Support/Wristper Test`, its own relay room).
- Do not deploy the relay.
- Never delete with `rm`. Use `/usr/bin/trash`.

## Launch (one command)

Run it from the repository root, with no TTY needed:

```sh
scripts/verify-watch-e2e.sh                 # build everything, both stages
scripts/verify-watch-e2e.sh --skip-build    # reuse target/app and the watch build
scripts/verify-watch-e2e.sh --stage 1       # protocol simulator only (about 2 min after build)
scripts/verify-watch-e2e.sh --stage 2 --targets textedit --langs en
```

The script uses these exit codes:

- `0` means every case passed.
- `1` means a case failed.
- `2` means bad usage or a missing tool or source tree.
- `3` means it is blocked on a one-time permission. The output prints the exact clicks.

The last line is `RESULT: PASS` or `RESULT: FAIL`. Before it, a table prints stage, target, language, result, match %, state, milliseconds, and the text read back.

You can set these environment variables:

- `WRISTPER_SRC` is the Wristper checkout that watch-sim compiles from. It is read only. The default is `/Users/glenhsu/orca/workspaces/wristper/coding`.
- `WRISTPER_HOOKS_SRC` is the watch app with the E2E hooks. The default is `/Users/glenhsu/orca/workspaces/wristper/sim-hooks`, the worktree of branch `test/sim-fixture-audio`.
- `WATCH_E2E_EN_MODEL` and `WATCH_E2E_ZH_MODEL` set the model for each language. The defaults are `parakeet_unified_en` and `nemotron_3_5_asr`. The watch sends language `auto`, so the Mac transcribes with the selected model and language. The script selects the model before each language with `hex model install <id> --language <lang>`. The default model is English only.
- `WATCH_E2E_EN_PASS` and `WATCH_E2E_ZH_PASS` set the pass thresholds in percent. The defaults are 80 and 60.
- `SIGN_IDENTITY` is the identity the app is re-signed with. The default is `Developer ID Application: WEI CHIEH HSU (VQY3J86U79)`. `build-app.sh` signs a test build ad hoc. An ad hoc identity changes with every build, so macOS would drop the permissions after each rebuild. Re-signing keeps one identity.

## For Terra (codex exec)

Run it like this, and do not run anything interactive:

```sh
cd /Users/glenhsu/orca/workspaces/hex_wrist/verify
export DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer
. "$HOME/.cargo/env"
scripts/verify-watch-e2e.sh 2>&1 | tee /tmp/watch-e2e.txt; echo "exit=${PIPESTATUS[0]}"
```

1. If the exit code is `3`, stop and paste the `BLOCKED` block to the user. A human must click. Do not try to grant the permission yourself.
2. If the exit code is `1`, open `target/watch-e2e/<run>/results.jsonl` and `run.log`, and report each FAIL row with its `detail`.
3. On a pass, report the table, the run folder, and the `RESULT` line.
4. Do not edit `src/` to make a case pass. Report the failure instead.

## One-time manual steps (a human, on this Mac)

The listener turns watch audio into pasted text. It starts only after Microphone, Accessibility, and a speech model are all ready (`desktop::onboarding::SetupStatus::ready`). Watch-only use still needs the Microphone grant.

1. Run `scripts/verify-watch-e2e.sh --stage 1`. It exits with `3` and prints the steps.
2. Run `"target/app/Wristper Test.app/Contents/MacOS/hex" window show`. In the setup page, click Allow for Microphone.
3. In System Settings > Privacy & Security > Accessibility, turn on Wristper Test. If it is not listed, add `target/app/Wristper Test.app`.
4. The first time, macOS asks whether your terminal app (the one that runs the script) may control TextEdit and Terminal (Automation). Click OK.
5. Run the script again with `--skip-build`.

The grants survive rebuilds only while the Developer ID re-sign succeeds. The script logs a WARNING when it does not.

## Doctor

Run these read-only checks first when anything looks off:

```sh
H="target/app/Wristper Test.app/Contents/MacOS/hex"
"$H" status --json | jq '{permissions, listener, model}'
cat ~/Library/Application\ Support/Wristper\ Test/relay-room | cut -c1-6
tail -50 ~/Library/Application\ Support/Wristper\ Test/logs/process.log | grep -i watch
```

- `permissions.ready` must be `true`.
- `listener.running` must be `true`.
- `process.log` should show the relay as connected.
- `"$H" status` must answer. If it does not, Wristper Test is not running. Start it with `open -g "target/app/Wristper Test.app"`.

## Evidence (what counts as proof)

A case is proof only when the text read back from TextEdit or Terminal matches the spoken sentence. Matching ignores case, punctuation, spacing, and Traditional versus Simplified characters (`watch-sim match`). A 200 reply or a `state` of `unverified` alone is not proof. B1 cannot read a field back, so the Mac reports `unverified` even after a good paste.

Each run writes `target/watch-e2e/<YYYYmmdd-HHMMSS>/`, and cleanup never removes it. The folder holds these files:

- `results.jsonl` has one row per case. For stage 1, its `detail` holds the decrypted `/status` JSON, `send_ms`, `transcribe_ms`, `total_ms`, and the Enter reply. For stage 2, it holds the watch app's `E2E result` log line.
- `run.log` has the timeline.
- `stage1-pair.jsonl` and `stage2-watch.log` hold the pairing evidence (code, claim, paired).
- `en.m4a` and `zh.m4a` are the fixtures. They are 16 kHz mono AAC at 32 kbps, which is the watch recorder's setting in `WatchProbe/ProbeModel.swift`.
- The `build-*.log` files are the build logs.

Enter is pending until B2. B1 answers `/enter` with `{"ok":false,"reason":"not-supported"}`. The Terminal cases press Enter (stage 1 `--enter`, stage 2 `-e2eEnter`) and record the reply. They do not fail on it.

## Real-watch check (optional, a human with a watch)

1. Start the test app with `open -g "target/app/Wristper Test.app"`.
2. On the watch, open Wristper and choose Pair Mac. The watch shows a 6-digit code.
3. Run `"target/app/Wristper Test.app/Contents/MacOS/hex" watch pair <code>`. The watch finishes pairing by itself.
4. Click into TextEdit, then record on the watch. The text must appear at the cursor.

The watch can hold several Macs. Pick the test Mac on the watch while you test, then switch back.

## Cleanup

The script's exit trap quits Wristper Test (`hex quit`) and shuts down the `Wristper E2E` simulator. It does both unless you pass `--keep-running` or the run is blocked. It closes the TextEdit documents it opened without saving and ends the `cat` it started in Terminal. It never kills by process name, except `pkill -t <its own tty> cat`. To remove a stale simulator, run `xcrun simctl delete "Wristper E2E"`. To start over from a fresh test data folder, run `/usr/bin/trash ~/Library/Application\ Support/Wristper\ Test`. That also loses the test room and the downloaded test models.

## Helpers

- `scripts/watch-sim/build.sh [out]` builds `target/watch-sim/watch-sim`.
- `watch-sim pair [--timeout s]` prints JSON lines. Lines with `event` set to `code`, `claimed`, and `paired` appear in that order.
- `watch-sim deliver --audio <m4a> [--language auto] [--enter] [--timeout s]` sends the encrypted `/focus`, then the encrypted `/e2ee` with a fresh UUID. It then polls the encrypted `/status` until the state is terminal. Set `WATCH_SIM_STATE=<dir>` to keep the watch's pairing in that folder.
- `watch-sim match --expected <text> --actual <text>` prints `{"percent":N}`.
- `watch-sim health` sends `/health` with the stored token.
