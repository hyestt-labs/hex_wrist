---
name: verify
description: Drive the Wristper Mac app (this Hex fork) end to end on a real Mac to prove Apple Watch dictation works. A watch, either the protocol simulator or the real watch app in a watchOS Simulator, pairs through the production relay and sends recordings, and the text must appear in TextEdit, Terminal, and Chrome, with Enter, Undo, and the clipboard checked too. Use it to verify any change to src/watch/, pairing, delivery, paste, or the test build. Also use it when someone asks whether the watch works.
---

# Verify Wristper on a real Mac

`scripts/verify-watch-e2e.sh --all` is the one command. It builds `Wristper Test.app`, launches it, pairs a watch through `https://relay.wristper.dev`, sends spoken recordings, reads the text back from the app that had focus, and prints one PASS/FAIL table. It runs without a human once the one-time permissions are granted. The feature recipes are in [features/README.md](features/README.md).

The script runs four stages:

1. **Protocol watch.** `watch-sim` is a macOS command-line watch compiled from the shipping watch Swift sources (`scripts/watch-sim/build.sh`). It pairs, then sends English and Mandarin into TextEdit and Terminal.
2. **Real watch app.** A DEBUG build from the pinned hooks commit (branch `test/sim-fixture-audio`) runs in a watchOS Simulator that the run creates (`Wristper E2E <run id>`) and deletes. Launch arguments make it pair and record without taps, with a fixture m4a instead of the microphone. It runs the same cases as stage 1.
3. **Scenarios no unit test covers**, through `watch-sim`:
   - `clipboard-keep-gate-switch`: Labs › Keep dictation on clipboard is on, the clipboard holds a sentinel, and the script switches to Finder after the recording starts and before its audio arrives. The first gate withholds the dictation as `target-changed`, nothing may reach the document, and the clipboard must still hold the sentinel.
   - `clipboard-keep-paste`: the same with no switch. The dictation stays on the clipboard.
   - `clipboard-default-restore`: keep is off. The sentinel comes back after the paste.
   - `textedit-enter`, `textedit-undo`: Return is typed once. Undo removes the text.
   - `cat-enter`, `cat-undo`: in Terminal running `cat`, Return makes `cat` echo the line, and Undo is refused.
   - `shell-enter`: in a plain zsh, Return runs the line once (`command not found` appears once).
   - `recorder-enter`: a Terminal window runs `while IFS= read -r l; do echo "ENTER:$l"; done`, which prints one `ENTER:<line>` per Return it receives. One Enter must give exactly one `ENTER:` line with the text and no empty one.
   - `enter-twice-textedit`, `enter-twice-shell`: two `/enter` calls at the same moment type Return once. The TextEdit row counts line breaks. The shell row uses the recorder window, so a second Return shows as a bare `ENTER:` and fails the row.
   - `mic-stays-closed`: **depends on C1.** After a successful watch dictation, `hex status` must still say `microphone: needs-request`, Hex's `process.log` since launch must hold no microphone-stream line, and the macOS TCC log must hold no microphone request from this app. Today the build requires the Microphone grant, so this row reads `BLOCKED` ("depends on C1"). `--c1` drops the Microphone requirement from the preflight and turns BLOCKED into FAIL. The only microphone lines Hex logs today are failure and recovery ones (`src/audio.rs`: `dictation microphone reopened`, `could not open dictation microphone at startup`, `microphone stream stopped; reopening it`, `microphone stream failed`, `the default microphone changed`). **No line is logged for a successful first open**, so a clean log is weak proof until C1 adds one. The script already looks for `microphone stream opened`, so C1 should log exactly that when it opens the input stream.
   - `chrome-textarea`: a throwaway `data:text/html` page with one textarea, in a separate Chrome process that has its own empty profile. The page copies its text into its title, and the script reads the title from that Chrome's remote debugging port (`curl http://127.0.0.1:<port>/json`). Nothing in the user's Chrome is used, opened, or changed, and Chrome gets no Apple event.
   - `agent-claude`, `agent-codex` (only with `--with-agents`): an interactive agent in Terminal receives "Reply with only the word pong." and Return submits it, proven by the agent's reply.
4. **B2 live tests.** `cargo nextest run --locked --bin hex --run-ignored only -E 'test(watch_live)' -j 1`. They read the focused field through Accessibility in TextEdit, Terminal, and Finder.

**Gap.** `clipboard-keep-gate-switch` reaches only the first gate, so the clipboard is never written and the sentinel survives even if the restore code were broken. It does not prove the restore in the final focus check (a switch after the clipboard write, right before ⌘V, with keep on; commit e0e36e16). `/status` reports both refusals as `target-changed`, and a script cannot time a window of tens of milliseconds. That restore has no automated test (report-B2-fixes3 item 3). The keep-off variant has the unit test `watch::output::tests::a_switch_during_the_clipboard_write_sends_no_paste`. To check the keep-on restore by hand, turn on Labs › Keep dictation on clipboard, put text on the clipboard, start a watch recording in TextEdit, and switch app at the moment the audio arrives, many times; the old clipboard must come back whenever the paste was withheld.

## Safety rules

What the script touches, and what it must leave alone:

- Never touch `/Applications/Wristper.app`, `~/Library/Application Support/Wristper`, or its relay room.
- Never touch the Keychain items of service `dev.wristper.mac.device*`. The test build uses `dev.wristper.mac.test.device`.
- Everything runs under `Wristper Test` (bundle `dev.wristper.mac.test`, data folder `~/Library/Application Support/Wristper Test`).
- **A new relay room every run, but only once the preflight passed.** A run that ends BLOCKED, and `--preflight-only`, reset nothing. After the preflight, the script stops Wristper Test, moves its `relay-room`, `relay-secret`, `paired-devices.json`, and `delivered-ids.json` (all inside `Wristper Test`) to the Trash, and starts it again so the service makes new ones. It checks the new room differs from the old one. A stage 2 or stage 3 run can never see an earlier run's pairing.
- **Secrets stay in one temp folder.** The clipboard backup, the watch's pairing state (device id, token, private key), and the raw logs with pairing codes live in `${TMPDIR}/wristper-e2e.XXXXXX` (mode 0700). It goes to the Trash at exit. The logs saved in the run folder have the pairing codes replaced by `******`.
- **Your clipboard comes back.** Before the first paste, the script saves every item and type on the clipboard (`scripts/pasteboard-tool`), and it puts them back at exit and in the exit trap. It then compares the bytes. The table has a `clipboard-restored` row, and a FAIL there means your clipboard was not restored. If the clipboard holds a concealed item (a password manager's copy), the script stops with exit 3 instead of saving it.
- **Only what it made is closed, also after a failure.** Every TextEdit document and Terminal window it opens is noted in `target-handles`, and the exit trap closes any that are left, after the same identity check. TextEdit and Terminal windows are found by the window id (and tty) from creation and checked again before every use and before closing. Nothing relies on the front window or document. A Terminal window is closed by hanging up the processes on its own tty and then closing that window. The script stops no process by name.
- **Apps that were already running stay open.** The script records whether TextEdit, Terminal, and Chrome were running before the first probe. It quits an app only if it started it and nothing is left open in it. It never drives your Chrome (see below).
- **Every child process is collected.** The exit trap (also on INT, TERM, HUP) stops watch-sim, the log stream, `hex watch pair`, the throwaway Chrome, and Wristper Test (`hex quit`, then only the processes that run its exact executable).
- **A simulator of its own.** Stage 2 creates a new watchOS Simulator named `Wristper E2E <run id>` and deletes that one at exit (after checking its udid and name). It never resets a Keychain, erases, shuts down, or reuses another device. With `--keep-running` it stays and the log prints the `xcrun simctl delete` command.
- The Chromium scenario starts its own Chrome process (`--user-data-dir=<temp folder>`, `--use-mock-keychain`, `--remote-debugging-port`). It stops only that process, found by its profile folder on the command line, and moves the temp profile to the Trash.
- `--with-agents` skips any `claude` or `codex` under `~/.wristper/` (the installed Wristper's wrappers) and uses the first other one on PATH.
- The scenarios change Labs › Keep dictation on clipboard on the test app. The script restores that setting at exit.
- The script never tries to grant a permission or to unlock the screen.
- Do not deploy the relay.
- Delete only with `/usr/bin/trash`. Nothing else deletes anything here.

## Launch (one command)

Run it from the repository root, with no TTY needed:

```sh
scripts/verify-watch-e2e.sh --all                 # build everything, all four stages (the default)
scripts/verify-watch-e2e.sh --all --skip-build    # reuse target/app, watch-sim, and the watch build
scripts/verify-watch-e2e.sh --preflight-only      # list every missing click; changes no state (starts Wristper Test only if it was not running, then stops it again)
scripts/verify-watch-e2e.sh --stage 1             # one stage: 1, 2, 3, or 4
scripts/verify-watch-e2e.sh --stage 3 --only 'enter|clipboard'    # scenarios whose name matches
scripts/verify-watch-e2e.sh --stage 2 --targets textedit --langs en
scripts/verify-watch-e2e.sh --all --with-agents   # also claude and codex in Terminal
scripts/verify-watch-e2e.sh --all --c1            # after unit C1 merged: no Microphone grant needed, mic-stays-closed must pass
```

The script uses these exit codes:

- `0` means every case passed.
- `1` means a case failed (with `--c1`, a BLOCKED row counts as a failure).
- `4` means no case failed but a row reads `BLOCKED` (today only `mic-stays-closed`, which depends on C1). The last line says `RESULT: INCOMPLETE (...)`. Report the blocked rows by name: they are not passes, and the run is not a green light.
- `2` means bad usage or a missing tool or source tree.
- `3` means it is blocked on a human. The output prints a `BLOCKED` block that lists every missing click in one go. The causes are a locked screen, Microphone or Accessibility for Wristper Test, Automation for the app that runs the script (to control TextEdit, Terminal, and System Events), Accessibility for that app (stage 4 and the Chrome scenario, which brings its throwaway Chrome to the front), and Google Chrome missing from `/Applications`.

The last line is `RESULT: PASS`, `RESULT: INCOMPLETE (...)`, or `RESULT: FAIL`. Before it, a table prints stage, scenario, language, result, match %, state, milliseconds, and the text read back. A failing row has a `└` line with the reasons.

Without a build step the script needs about 15 minutes. The first Mandarin run also downloads a model (the script waits up to 60 minutes).

You can set these environment variables:

- **Pinned watch sources.** The script never builds from a branch or a working tree. It reads two commits out of the Wristper Swift repo's git history with `git archive` and builds from that export (`target/watch-e2e/pinned/<name>-<commit>`, made once and never changed):
  - `WRISTPER_SRC_COMMIT` = `426185302ac51f77b07fc59878491bdf9a16ce71` (the watch app sources, Wristper PR #288 merge). `watch-sim` compiles `WatchShared` and `WatchProbe` from it.
  - `WRISTPER_HOOKS_COMMIT` = `027800efddfc06a7088767faf61461a43b2a30c7` (that commit plus the DEBUG-only E2E hooks, branch `test/sim-fixture-audio`, two local commits not pushed). Stage 2 builds this app. The hooks log the state the Mac actually sent on the wire, never the default `pasted` the watch app assumes when a reply has no state.
  - Both are constants at the top of `scripts/verify-watch-e2e.sh`, and the run log prints them. An override must be a full 40-character sha. `WRISTPER_SRC` (default `/Users/glenhsu/orca/workspaces/wristper/coding`) is only the checkout whose history holds them. To move a pin, change the constant and this paragraph in the same commit.
- `WATCH_E2E_B2_ENV` is the file that links the Rust test binary on this Mac (stage 4). The default is `.../migration_to_hex/artifacts/wristper-migration-20261104/b2-test-env/env.sh`.
- `WATCH_E2E_EN_MODEL` and `WATCH_E2E_ZH_MODEL` set the model for each language. The defaults are `parakeet_unified_en` and `nemotron_3_5_asr`. The watch sends language `auto`, so the Mac transcribes with the selected model and language. The script selects the model before each language with `hex model install <id> --language <lang>`.
- `WATCH_E2E_EN_PASS` and `WATCH_E2E_ZH_PASS` set the pass thresholds in percent. The defaults are 80 and 60. The score is ordered: the longest common subsequence of the words (English) or characters (Mandarin) over the expected length, so words out of order do not count. An empty read-back, or one with fewer than half the expected words or characters, scores 0. A stage 2 row passes only when the watch app also reports phase `finished` and the Mac's own state, as it came over the wire, is `pasted` or `unverified`. For a field that starts empty (a TextEdit document, the Chrome textarea) the read-back must also be mostly the sentence: precision (common words over read-back words) must reach the same threshold, so a sentence pasted twice (recall 100, precision 50) fails. Terminal read-backs include the prompt, so they are judged on recall only.
- `WATCH_E2E_AGENT_DIR` is the folder where `--with-agents` starts claude and codex. The default is the repository. The agent must already trust that folder, or its trust prompt eats the first Return.
- `SIGN_IDENTITY` is the identity the app is re-signed with. The default is `Developer ID Application: WEI CHIEH HSU (VQY3J86U79)`. `build-app.sh` signs a test build ad hoc. An ad hoc identity changes with every build, so macOS would drop the permissions after each rebuild. Re-signing keeps one identity.

## For Terra (codex exec)

The user must be at the Mac with the screen unlocked, or have run the one-time steps below, and the Mac must stay idle during the run. Run it like this, and do not run anything interactive:

```sh
codex exec -m gpt-6.1-terra --sandbox danger-full-access \
  'Run this with bash, nothing else, and wait for it to finish: bash -c "cd /Users/glenhsu/orca/workspaces/hex_wrist/verify && export DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer && scripts/verify-watch-e2e.sh --all > /tmp/watch-e2e.txt 2>&1; echo exit=\$?; tail -60 /tmp/watch-e2e.txt". Then follow .claude/skills/verify/SKILL.md section "For Terra" and report the table, the exit code, and the run folder. Never touch /Applications/Wristper.app or ~/Library/Application Support/Wristper. Delete only with /usr/bin/trash.'
```

The script runs under `bash -c` on purpose: the exit code must come from the script itself, and `PIPESTATUS` does not exist in zsh.

0. If the exit code is `4`, report the table and the `INCOMPLETE` line, name each BLOCKED row, and say the run is not proven. Do not call it a pass.
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
4. The first time, macOS asks whether the app that runs the script may control TextEdit, Terminal, and System Events (Automation). Click OK each time. Chrome needs no setting.
5. Run the script again with `--skip-build`.

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

A case is proof only when the text read back from TextEdit, Terminal, or the throwaway Chrome matches the spoken sentence. Matching ignores case, punctuation, spacing, and Traditional versus Simplified characters (`watch-sim match`). A 200 reply or a `state` of `unverified` alone is not proof.

Each run writes `target/watch-e2e/<YYYYmmdd-HHMMSS>/`, and cleanup never removes it. The folder holds these files:

- `results.jsonl` has one row per case. For `watch-sim` cases, `detail` holds the decrypted `/status` JSON, `send_ms`, `transcribe_ms`, `total_ms`, the raw `/enter`, `/undo`, and double-Enter replies, and `fails`, the reasons of a FAIL. For stage 2, it holds the watch app's `E2E result` log line.
- `run.log` has the timeline.
- `stage1-pair.jsonl`, `stage3-pair.jsonl` (if stage 3 paired alone), and `stage2-watch.log` hold the pairing evidence (claim, paired), with the pairing codes replaced by `******`.
- `stage4-live.log` holds the nextest output.
- `readback.txt` is the last read-back, raw.
- `chrome-handles` lists the throwaway Chrome processes this run started.
- `en.m4a`, `zh.m4a`, and `agent.m4a` are the fixtures. They are 16 kHz mono AAC at 32 kbps, which is the watch recorder's setting in `WatchProbe/ProbeModel.swift`.
- The `build-*.log` files are the build logs.

## Real-watch check (optional, a human with a watch)

1. Start the test app with `open -g "target/app/Wristper Test.app"`.
2. On the watch, open Wristper and choose Pair Mac. The watch shows a 6-digit code.
3. Run `"target/app/Wristper Test.app/Contents/MacOS/hex" watch pair <code>`. The watch finishes pairing by itself.
4. Click into TextEdit, then record on the watch. The text must appear at the cursor.

The watch can hold several Macs. Pick the test Mac on the watch while you test, then switch back.

## Cleanup

The exit trap (it also runs on Ctrl-C and on a kill) does this, unless you pass `--keep-running`:

1. It stops the script's own child processes and the throwaway Chrome, and moves its temp profile to the Trash.
2. It puts your clipboard back and restores Labs › Keep dictation on clipboard.
3. It quits Wristper Test and deletes the simulator this run made.
4. It quits TextEdit and Terminal only if this run started them and nothing is open in them.
5. It moves the temp folder with the secrets to the Trash.

The run folder is never removed. To start over from a fresh test data folder (this also loses the downloaded test models), run `/usr/bin/trash ~/Library/Application\ Support/Wristper\ Test`. An old simulator named `Wristper E2E` without a run id comes from an earlier version of this script; remove it with `xcrun simctl delete <udid>` after checking its name.

## Helpers

- `scripts/watch-sim/build.sh [out]` builds `target/watch-sim/watch-sim`.
- `watch-sim pair [--timeout s]` prints JSON lines. Lines with `event` set to `code`, `claimed`, and `paired` appear in that order.
- `watch-sim deliver --audio <m4a> [--language auto] [--after-focus <sh>] [--enter] [--enter-twice] [--undo] [--timeout s]` sends the encrypted `/focus`, runs the optional shell command (the app switch), then sends the encrypted `/e2ee` with a fresh UUID. It polls the encrypted `/status` until the state is terminal. Then it sends `/enter` once, `/enter` twice at the same moment, or `/undo`, and keeps the Mac's reply verbatim. Set `WATCH_SIM_STATE=<dir>` to keep the watch's pairing in that folder.
- `watch-sim action --path /enter|/undo --id <uuid>` sends one action for an earlier recording.
- `watch-sim match --expected <text> --actual <text>` prints `{"percent":N}`.
- `watch-sim health` sends `/health` with the stored token.
