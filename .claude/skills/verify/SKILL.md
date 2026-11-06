---
name: verify
description: Drive the Wristper Mac app (this Hex fork) end to end on a real Mac to prove Apple Watch dictation works. A watch, either the protocol simulator or the real watch app in a watchOS Simulator, pairs through the production relay and sends recordings, and the text must appear in TextEdit, Terminal, and Chrome, with Enter, Undo, and the clipboard checked too. Use it to verify any change to src/watch/, pairing, delivery, paste, or the test build. Also use it when someone asks whether the watch works.
---

# Verify Wristper on a real Mac

`scripts/verify-watch-e2e.sh --all` is the one command. It builds `Wristper Test.app`, launches it, pairs a watch through `https://relay.wristper.dev`, sends spoken recordings, reads the text back from the window it opened, and prints one PASS/FAIL table. It runs without a human once the one-time permissions are granted. The feature recipes are in [features/README.md](features/README.md).

**Before the first real run, on a Mac with someone at it:** `scripts/verify-watch-e2e.sh --check-textedit` (about 3 seconds). It opens one TextEdit document of its own and checks it by window id; checks that a window whose live name differs from the registered one (same id) is still recognised, and that an unregistered id is refused; focuses it; runs the front check stage 1 uses; then writes a random string, reads it back, and closes that one document without saving, and compares TextEdit's window ids and document names, and the files in TextEdit's iCloud Drive folder, with the state from before. It touches nothing else, and it needs only Automation for TextEdit.

Three things about TextEdit that the script is built around (each was found on a real Mac):

- **A new document is found as "the one window id that was not in the list before".** `make new document` returns a document object that cannot be made into a window reference (`first window whose document is d` fails with -1700).
- **The identity of a TextEdit window is its window id, not its name.** TextEdit renames an edited untitled document after its first line and autosaves it (run 2: `Untitled 23` became `Quarterly Report to Design Team`, and `/Users/…/Library/Mobile Documents/com~apple~TextEdit/Documents/Quarterly Report to Design Team.rtf` appeared one second after the paste). The name is recorded once, when the window is made, and noted in the log if it changes. A different id, or an id that is not in the registry, is still refused.
- **The front window is not always the document.** About a second after a new document appears, TextEdit shows a small panel (no document, not closeable, 280×168) at the top of its window list. The front check therefore asks for the first window that has a document (`te_first_document_window`), not `front window`.
- **Autosave leaves a file.** A document that has text is saved to iCloud Drive a few seconds after an edit, and closing with "saving no" keeps the file. The script does not delete it (it is a file in your own folder). The final table lists it as a `NOTE` row (`leftover-files`), which does not stop a `PASS`. The `--check-textedit` smoke test writes last and closes at once so it creates none, and fails if one appears.

**First real run (screen unlocked, nothing else open in TextEdit):** do not start with `--all`. Run `scripts/verify-watch-e2e.sh --stage 1 --targets textedit --langs en`. It touches one TextEdit window and nothing else. Widen the run only after it passes.

## The ownership rule

The script acts only on what it made itself and can prove it made. Terra's reviews found one root cause behind every unsafe finding: acting on desktop state the script did not create and did not re-check (the front window, a path saved later, the list of apps at the start, a whole tty). So the structure is:

- **One registry** (`$tmp/registry.tsv`, a file, because many calls run in subshells). Everything the script starts goes in at the moment it is made, and the dangerous ones register themselves *before* they run: each process starts in its own session and writes its own pid, marker, and start time into the registry before it execs; Wristper Test and the simulator are noted before they are launched or created, so the teardown can still find them if a signal or a failure comes right after. The registry holds each process group, each TextEdit or Terminal window id (with the document name or the tty), the one process it runs inside a Terminal window (its pid, tty, start time, and command line), each temp path, the Wristper Test it launched, the simulator it created. The apps that were running at the start are logged as information only. They decide nothing.
- **One gate**, the function `owned KIND KEY CMD...`. Every close, kill, Trash, focus, read, and input (audio, Enter, Undo) goes through it. If the object is not in the registry it does nothing, logs `REFUSED`, and marks the row `INCOMPLETE` (an overall `ownership-guard` row lists every refusal). The `act_*` functions it runs then re-check the live identity: the window id and name or tty, the process command line, the real path.
- **No broad actions, and no action on a guess.** The script stops no process by name, never touches a whole tty (only the one registered process on it, with HUP), never quits TextEdit or Terminal, never closes the front window or document, never Trashes a document the test saved, and never sends anything to a window it did not register. A process counts as ours only when its pid, its start time, and its command line all match what was registered, and this is checked again between TERM and KILL, and before `hex quit`. If a group still has members but its leader is gone, nothing is killed (it cannot be proven ours) and the row is `INCOMPLETE`. After a refusal or a failed action the script does not do something else instead: a Terminal window is not closed unless its registered process was proven ours and stopped; a failed Stop or Trash is reported and keeps its registry entry; a failed relay-room reset stops the run. A window the person already closed counts as gone (nothing to do), not as a refusal.
- **Input is checked at the moment it goes out.** watch-sim runs a front check (`lsappinfo` for the app, then AppleScript for the window id and the document name or tty) after `/focus` and before `/e2ee`, and before each `/enter` and `/undo`. A failed check sends nothing, and watch-sim fails closed: with no check given it sends nothing. In stage 2 the watch app records, then waits for a gate file before it uploads; the script creates the file only if the registered window is still in front. A failed `focus_target` aborts the scenario before any audio.

Run `scripts/verify-watch-e2e.sh --selftest` after any change to the script (74 checks, no app, window, or network). It runs the real functions against stand-in `osascript`, `lsappinfo`, and `open`, and it runs the **real** `watch-sim` binary in its dry-run mode (`WATCH_SIM_DRY_RUN=1`: nothing goes over the network, every step is recorded in the order it happens), so it checks the real order and the real exit codes: the audio goes only after the after-focus gate, each `/enter` (also each of two at the same moment) and `/undo` has its own gate first, a failing or missing gate sends nothing and exits 1, and `action` needs a gate too. A control feeds the order check a trace that sends before its gate and expects it to be caught. It also checks that the gate refuses an unregistered window, process group, and path, a path through a link out of the temp root, a start time or command line that does not match, a group whose leader is gone, a Wristper Test pid that is not the app (no `hex quit` is sent), a Terminal window with no proven process, and a Trash that fails; that a focus failure, a front mismatch, and an app that moves away between `/focus` and `/e2ee` each send no audio; that signals are masked during the cleanup; and, as the positive control, that audio is sent when all is well, so "not sent" can be told apart.

## What the stages do

1. **Protocol watch.** `watch-sim` is a macOS command-line watch compiled from the shipping watch Swift sources (`scripts/watch-sim/build.sh`). It pairs, then sends English and Mandarin into TextEdit and Terminal.
2. **Real watch app.** A DEBUG build from the pinned hooks commit runs in a watchOS Simulator that the run creates (`Wristper E2E <run id>`) and deletes. Launch arguments make it pair and record without taps, with a fixture m4a instead of the microphone, and wait for the gate file before uploading. It runs the same cases as stage 1.
3. **Scenarios no unit test covers**, through `watch-sim`:
   - `clipboard-keep-gate-switch`: Labs › Keep dictation on clipboard is on, the clipboard holds a sentinel, and the script switches to Finder after the recording starts and before its audio arrives (the front check passes first, then the switch). The first gate withholds the dictation as `target-changed`, nothing may reach the document, and the clipboard must still hold the sentinel.
   - `clipboard-keep-paste`: the same with no switch. The dictation stays on the clipboard.
   - `clipboard-default-restore`: keep is off. The sentinel comes back after the paste.
   - `textedit-enter`, `textedit-undo`: Return is typed once. Undo removes the text.
   - `cat-enter`, `cat-undo`: in Terminal running `cat`, Return makes `cat` echo the line, and Undo is refused.
   - `shell-enter`: in a plain zsh, Return runs the line once (`command not found` appears once).
   - `recorder-enter`: a Terminal window runs `while IFS= read -r l; do echo "ENTER:$l"; done`, which prints one `ENTER:<line>` per Return it receives. One Enter must give exactly one `ENTER:` line with the text and no empty one.
   - `enter-twice-textedit`, `enter-twice-shell`: two `/enter` calls at the same moment type Return once. The TextEdit row counts line breaks. The shell row uses the recorder window, so a second Return shows as a bare `ENTER:` and fails the row.
   - `mic-stays-closed`: **depends on C1.** After a successful watch dictation, `hex status` must still say `microphone: needs-request`, Hex's `process.log` since launch must hold no microphone-stream line, and the macOS TCC log must hold no microphone request from this app. Today the build requires the Microphone grant, so this row reads `BLOCKED` ("depends on C1"). `--c1` drops the Microphone requirement from the preflight and turns BLOCKED into FAIL. The only microphone lines Hex logs today are failure and recovery ones (`src/audio.rs`: `dictation microphone reopened`, `could not open dictation microphone at startup`, `microphone stream stopped; reopening it`, `microphone stream failed`, `the default microphone changed`). **No line is logged for a successful first open**, so a clean log is weak proof until C1 adds one. The script already looks for `microphone stream opened`, so C1 should log exactly that when it opens the input stream.
   - `chrome-textarea`: a throwaway `data:text/html` page with one textarea, in a separate Chrome process that has its own empty profile folder under the temp root. The page copies its text into its title, and the script reads the title from that Chrome's remote debugging port (`curl http://127.0.0.1:<port>/json`). Nothing in the user's Chrome is used, opened, or changed, and Chrome gets no Apple event.
   - `agent-claude`, `agent-codex` (only with `--with-agents`): an interactive agent in Terminal receives "Reply with only the word pong." and Return submits it, proven by the agent's reply.
4. **B2 live tests, opt in with `--with-live` (or `--stage 4`); not part of `--all`.** `cargo nextest run --locked --bin hex --run-ignored only -E 'test(watch_live)' -j 1`. They read and press in real TextEdit and Terminal windows. Each test makes its own document or window and records the window id (and the document name or the tty) it got; before it pastes, presses, or closes, it checks that the live window still matches (an empty or failed answer counts as "not ours", and an app that is not running is not asked), it raises that window and asserts it is in front, and it closes only that window by id. They no longer use `close front document` or send `exit` to the front window. `owned_cleanup` tests in the same file (no screen needed, run by `scripts/check watch`) simulate a document or window of yours coming to the front before the cleanup and assert it is not closed and gets no `exit`. The stage stays opt in because it sends real keys (Return, Command-Z) and has never been seen to pass on a real Mac, not because of how it closes.

**Gap.** `clipboard-keep-gate-switch` reaches only the first gate, so the clipboard is never written and the sentinel survives even if the restore code were broken. It does not prove the restore in the final focus check (a switch after the clipboard write, right before ⌘V, with keep on; commit e0e36e16). `/status` reports both refusals as `target-changed`, and a script cannot time a window of tens of milliseconds. That restore has no automated test (report-B2-fixes3 item 3). The keep-off variant has the unit test `watch::output::tests::a_switch_during_the_clipboard_write_sends_no_paste`. To check the keep-on restore by hand, turn on Labs › Keep dictation on clipboard, put text on the clipboard, start a watch recording in TextEdit, and switch app at the moment the audio arrives, many times; the old clipboard must come back whenever the paste was withheld.

## Safety rules

What the script touches, and what it must leave alone:

- Never touch `/Applications/Wristper.app`, `~/Library/Application Support/Wristper`, or its relay room.
- Never touch the Keychain items of service `dev.wristper.mac.device*`. The test build uses `dev.wristper.mac.test.device`.
- Everything runs under `Wristper Test` (bundle `dev.wristper.mac.test`, data folder `~/Library/Application Support/Wristper Test`).
- **A Wristper Test that is already running makes the script stop with exit 3.** It prints why and changes nothing: it does not stop that app and does not reset its pairing data. The script only stops and resets a Wristper Test that it launched itself, and it identifies that one by the pid in its own `hex status` and by the exact executable path.
- **A new relay room every run, but only once the preflight passed.** A run that ends BLOCKED, and `--preflight-only`, reset nothing. After the preflight, the script stops its own Wristper Test, moves `relay-room`, `relay-secret`, `paired-devices.json`, and `delivered-ids.json` to the Trash (the one place it Trashes outside the temp root: four allowlisted files in `Wristper Test`, and only while its app is stopped), and starts it again so the service makes new ones. It checks the new room differs from the old one.
- **Secrets stay in one temp folder.** The clipboard backup, the watch's pairing state (device id, token, private key), the raw logs with pairing codes, the registry, and Chrome's profile live in `${TMPDIR}/wristper-e2e.XXXXXX` (mode 0700). It goes to the Trash at exit, through the gate, and only paths whose real location is inside it. The logs saved in the run folder have the pairing codes replaced by `******`.
- **Your clipboard comes back, or its backup stays.** Before the first paste, the script saves every item and type on the clipboard (`scripts/pasteboard-tool`), and it puts them back at exit and in the exit trap. It then compares the bytes. The table has a `clipboard-restored` row. If it cannot be restored, the backup stays in the temp folder (everything else there is Trashed), and the script prints the path and the exact `pasteboard-tool restore` command. If any type on the clipboard cannot be read, or an item is concealed (a password manager's copy), the script stops with exit 3 before it changes anything: there is never a partial backup.
- **Only what it made is closed, also after a failure.** TextEdit and Terminal windows are found by the id (and name or tty) from creation and checked again before every use and before closing. A Terminal window is closed by hanging up the one registered process on its tty and then closing that window by id.
- **Apps that were already running stay open.** The script never quits TextEdit or Terminal (probes may launch them; they are left running). It never drives your Chrome (see below).
- **Every child process is collected.** Each long or background process (watch-sim, the log stream, `hex watch pair`, `hex model install`, builds, cargo, Chrome) starts in its own session and registers itself. The traps (EXIT, INT, TERM, HUP) are installed before the temp folder is made; during the cleanup those signals are ignored so it cannot be cut short. The cleanup sends TERM to each registered group, waits until no member is left, then KILLs what remains, after proving the leader is still the process that was started. It stops Wristper Test with `hex quit` only after its pid is proven and `hex status` reports that same pid, then only the registered pid. Short CLI calls (`hex status`, `hex settings`, `simctl list`) are not registered.
- **A simulator of its own.** Stage 2 creates a new watchOS Simulator named `Wristper E2E <run id>`, registers it, and every `simctl` call on it goes through the gate. It deletes that one at exit. It never resets a Keychain, erases, shuts down, or reuses another device. With `--keep-running` it stays and the log prints the `xcrun simctl delete` command.
- The Chromium scenario starts its own Chrome process (`--user-data-dir=<folder under the temp root>`, `--use-mock-keychain`, `--remote-debugging-port`), in its own session. At the end it stops that group and moves the profile to the Trash.
- `--with-agents` skips any `claude` or `codex` under `~/.wristper/` (the installed Wristper's wrappers) and uses the first other one on PATH.
- The scenarios change Labs › Keep dictation on clipboard on the test app. The script restores that setting at exit.
- The script never tries to grant a permission or to unlock the screen.
- Do not deploy the relay.
- Delete only with `/usr/bin/trash`. The script stops no process by name.

## Launch (one command)

Run it from the repository root, with no TTY needed:

```sh
scripts/verify-watch-e2e.sh --stage 1 --targets textedit --langs en   # the first real run
scripts/verify-watch-e2e.sh --all                 # build everything, stages 1 to 3 (the default)
scripts/verify-watch-e2e.sh --all --skip-build    # reuse target/app, watch-sim, and the watch build
scripts/verify-watch-e2e.sh --selftest            # the ownership rules, no app or window needed
scripts/verify-watch-e2e.sh --preflight-only      # list every missing click; changes no state
scripts/verify-watch-e2e.sh --stage 3 --only 'enter|clipboard'    # scenarios whose name matches
scripts/verify-watch-e2e.sh --stage 2 --targets textedit --langs en
scripts/verify-watch-e2e.sh --all --with-agents   # also claude and codex in Terminal
scripts/verify-watch-e2e.sh --all --c1            # after unit C1 merged: no Microphone grant needed, mic-stays-closed must pass
scripts/verify-watch-e2e.sh --stage 4 --skip-build  # the B2 live tests (see stage 4 above)
```

The script uses these exit codes:

- `0` means every case passed.
- `1` means a case failed (with `--c1`, a BLOCKED row counts as a failure).
- `2` means bad usage or a missing tool or source tree.
- `3` means it is blocked on a human. The output prints a `BLOCKED` block that lists every missing step in one go. The causes are a locked screen, a Wristper Test that is already running, an unreadable or concealed clipboard, Microphone or Accessibility for Wristper Test, Automation for the app that runs the script (to control TextEdit and Terminal), Accessibility and System Events for that app (stage 4 and the Chrome scenario, which brings its throwaway Chrome to the front), and Google Chrome missing from `/Applications`.
- `4` means no case failed but a row reads `BLOCKED` (today `mic-stays-closed`, which depends on C1) or `INCOMPLETE` (the run aborted a row, or the gate refused something). The last line says `RESULT: INCOMPLETE (...)`. Report those rows by name: they are not passes, and the run is not a green light.

The last line is `RESULT: PASS`, `RESULT: INCOMPLETE (...)`, or `RESULT: FAIL`. Before it, a table prints stage, scenario, language, result (`PASS`, `FAIL`, `BLOCKED`, `INCOMPLETE`), match %, state, milliseconds, and the text read back. A non-passing row has a `└` line with the reasons.

Without a build step the script needs about 15 minutes. The first Mandarin run also downloads a model (the script waits up to 60 minutes).

You can set these environment variables:

- **Pinned watch sources.** The script never builds from a branch or a working tree. It reads two commits out of the Wristper Swift repo's git history with `git archive` and builds from that export (`target/watch-e2e/pinned/<name>-<commit>`, made once and never changed):
  - `WRISTPER_SRC_COMMIT` = `426185302ac51f77b07fc59878491bdf9a16ce71` (the watch app sources, Wristper PR #288 merge). `watch-sim` compiles `WatchShared` and `WatchProbe` from it.
  - `WRISTPER_HOOKS_COMMIT` = `cf8c14b38cfe40f00df53a35f9cd08149fd87cd4` (that commit plus the DEBUG-only E2E hooks, branch `test/sim-fixture-audio`, local commits not pushed). Stage 2 builds this app. The whole of `WatchProbe/E2EHooks.swift` sits inside `#if DEBUG` (one pair, lines 1 and 137), every caller is inside `#if DEBUG`, and a Release build of the watch app contains none of the hook strings (checked with `strings`). The hooks log the state the Mac actually sent on the wire, never the default `pasted` the watch app assumes when a reply has no state, and they upload only after the gate file appears.
  - Both are constants at the top of `scripts/verify-watch-e2e.sh`, and the run log prints them. An override must be a full 40-character sha. `WRISTPER_SRC` (default `/Users/glenhsu/orca/workspaces/wristper/coding`) is only the checkout whose history holds them. To move a pin, change the constant and this paragraph in the same commit.
- `WATCH_E2E_B2_ENV` is the file that links the Rust test binary on this Mac (stage 4). The default is `.../migration_to_hex/artifacts/wristper-migration-20261104/b2-test-env/env.sh`.
- `WATCH_E2E_EN_MODEL` and `WATCH_E2E_ZH_MODEL` set the model for each language. The defaults are `parakeet_unified_en` and `nemotron_3_5_asr`. The watch sends language `auto`, so the Mac transcribes with the selected model and language. The script selects the model before each language with `hex model install <id> --language <lang>`.
- `WATCH_E2E_EN_PASS` and `WATCH_E2E_ZH_PASS` set the pass thresholds in percent. The defaults are **80 and 90**. The score is ordered: the longest common subsequence of the words (English) or characters (Mandarin) over the expected length, so words out of order do not count. The default English sentence has 12 words, so 80 allows two lost words. The default Mandarin sentence has 19 characters, so 90 allows one wrong or missing character. An empty read-back, or one with fewer than half the expected words or characters, scores 0. A stage 2 row passes only when the watch app also reports phase `finished` and the Mac's own state, as it came over the wire, is `pasted` or `unverified`. For a field that starts empty (a TextEdit document, the Chrome textarea) the read-back must also be mostly the sentence: precision (common words over read-back words) must reach the same threshold, so a sentence pasted twice (recall 100, precision 50) fails. Terminal read-backs include the prompt, so they are judged on recall only.
- `WATCH_E2E_AGENT_DIR` is the folder where `--with-agents` starts claude and codex. The default is the repository. The agent must already trust that folder, or its trust prompt eats the first Return.
- `SIGN_IDENTITY` is the identity the app is re-signed with. The default is `Developer ID Application: WEI CHIEH HSU (VQY3J86U79)`. `build-app.sh` signs a test build ad hoc. An ad hoc identity changes with every build, so macOS would drop the permissions after each rebuild. Re-signing keeps one identity.

## For Terra (codex exec)

The user must be at the Mac with the screen unlocked, or have run the one-time steps below, and the Mac must stay idle during the run. **The first real run is stage 1 with TextEdit only.** Run it like this, and do not run anything interactive:

```sh
codex exec -m gpt-6.1-terra --sandbox danger-full-access \
  'Run this with bash, nothing else, and wait for it to finish: bash -c "cd /Users/glenhsu/orca/workspaces/hex_wrist/verify && export DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer && scripts/verify-watch-e2e.sh --selftest > /tmp/watch-e2e-selftest.txt 2>&1; echo selftest-exit=\$?; scripts/verify-watch-e2e.sh --stage 1 --targets textedit --langs en > /tmp/watch-e2e.txt 2>&1; echo exit=\$?; tail -60 /tmp/watch-e2e.txt". Then follow .claude/skills/verify/SKILL.md section "For Terra" and report both exit codes, the table, and the run folder. Do not run --all or any other stage unless the user asks. Never touch /Applications/Wristper.app or ~/Library/Application Support/Wristper. Delete only with /usr/bin/trash.'
```

The script runs under `bash -c` on purpose: the exit code must come from the script itself, and `PIPESTATUS` does not exist in zsh.

0. If the self-test exit code is not `0`, stop and report it. Do not run the real run.
1. If the exit code is `4`, report the table and the `INCOMPLETE` line, name each BLOCKED or INCOMPLETE row, and say the run is not proven. Do not call it a pass.
2. If the exit code is `3`, stop and paste the whole `BLOCKED` block to the user. A human must click, close, or quit. Do not try to grant any permission, unlock the screen, edit TCC, or quit an app the script says it did not start.
3. If the exit code is `1`, open `target/watch-e2e/<run>/results.jsonl`, `run.log`, `refusals.tsv`, and report each FAIL row with its `detail`.
4. On a pass, report the table, the run folder, and the `RESULT` line.
5. Do not edit `src/` to make a case pass. Report the failure instead.
6. Add `--with-agents` only when the user asks. It spends tokens on their claude and codex logins.

## One-time manual steps (a human, on this Mac)

The listener turns watch audio into pasted text. It starts only after Microphone, Accessibility, and a speech model are all ready (`desktop::onboarding::SetupStatus::ready`). Watch-only use still needs the Microphone grant until unit C1 lands.

1. Quit any Wristper Test that is running (`"target/app/Wristper Test.app/Contents/MacOS/hex" quit`). The script refuses to continue while one runs.
2. Run `scripts/verify-watch-e2e.sh --preflight-only`. It exits with `3` and lists the exact steps for this Mac.
3. Run `open -g "target/app/Wristper Test.app"`, then `"target/app/Wristper Test.app/Contents/MacOS/hex" window show`. In the setup page, click Allow for Microphone. Quit it again.
4. In System Settings > Privacy & Security > Accessibility, turn on Wristper Test. If it is not listed, add `target/app/Wristper Test.app`. Turn on the app that runs the script too (Terminal, Orca, iTerm) for stage 4 and the Chrome scenario.
5. The first time, macOS asks whether the app that runs the script may control TextEdit, Terminal, and System Events (Automation). Click OK each time. Chrome needs no setting.
6. Run the script again with `--skip-build`.

The grants survive rebuilds only while the Developer ID re-sign succeeds. The script logs a WARNING when it does not.

## Doctor

Run these read-only checks first when anything looks off:

```sh
H="target/app/Wristper Test.app/Contents/MacOS/hex"
"$H" status --json | jq '{permissions, listener, model, pid}'
tail -50 ~/Library/Application\ Support/Wristper\ Test/logs/process.log | grep -i watch
ioreg -n Root -d1 | grep IOConsoleLocked
lsappinfo front
```

- `permissions.ready` must be `true`.
- `listener.running` must be `true`.
- `process.log` should show the relay as connected.
- `IOConsoleLocked` must not read `Yes`.
- `lsappinfo front` names the app in front. On a locked screen it is `loginwindow`.
- `"$H" status` must answer. If it does not, Wristper Test is not running. Start it with `open -g "target/app/Wristper Test.app"`.

## Evidence (what counts as proof)

A case is proof only when the text read back from TextEdit, Terminal, or the throwaway Chrome matches the spoken sentence. Matching ignores case, punctuation, spacing, and Traditional versus Simplified characters (`watch-sim match`). A 200 reply or a `state` of `unverified` alone is not proof.

Each run writes `target/watch-e2e/<YYYYmmdd-HHMMSS>/`, and cleanup never removes it. The folder holds these files:

- `results.jsonl` has one row per case. For `watch-sim` cases, `detail` holds the decrypted `/status` JSON, `send_ms`, `transcribe_ms`, `total_ms`, `e2ee_sent`, the raw `/enter`, `/undo`, and double-Enter replies, and `fails`, the reasons of a FAIL, INCOMPLETE, or BLOCKED row. For stage 2, it holds the watch app's `E2E result` log line.
- `run.log` has the timeline, including every `REFUSED` line.
- `refusals.tsv` lists everything the gate refused (kind, key, action).
- `stage1-pair.jsonl`, `stage3-pair.jsonl` (if stage 3 paired alone), and `stage2-watch.log` hold the pairing evidence (claim, paired), with the pairing codes replaced by `******`.
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

The exit trap (it also runs on Ctrl-C and on a kill) does this, in this order, all through the gate and unless you pass `--keep-running`:

1. It stops every registered process group (TERM, wait until no member is left, then KILL) and the throwaway Chrome.
2. It closes the TextEdit documents and Terminal windows this run made (HUP of the one registered process in each Terminal window first).
3. It puts your clipboard back and restores Labs › Keep dictation on clipboard.
4. It quits the Wristper Test this run started and deletes the simulator this run made.
5. It moves the temp folder to the Trash. If the clipboard could not be put back, the backup stays and the script prints where.

The run folder is never removed. To start over from a fresh test data folder (this also loses the downloaded test models), run `/usr/bin/trash ~/Library/Application\ Support/Wristper\ Test`. An old simulator named `Wristper E2E` without a run id comes from an earlier version of this script; remove it with `xcrun simctl delete <udid>` after checking its name.

## Helpers

- `scripts/watch-sim/build.sh [out]` builds `target/watch-sim/watch-sim`.
- `watch-sim pair [--timeout s]` prints JSON lines. Lines with `event` set to `code`, `claimed`, and `paired` appear in that order.
- `watch-sim deliver --audio <m4a> [--language auto] --after-focus <sh> [--before-action <sh>] [--enter] [--enter-twice] [--undo] [--timeout s]` sends the encrypted `/focus`, then runs the `--after-focus` command (`/bin/sh -c`). If it exits non-zero, or is missing, watch-sim sends nothing, reports `e2ee_sent: false`, and exits 1. Otherwise it sends the encrypted `/e2ee` with a fresh UUID and polls the encrypted `/status` until the state is terminal. Then it sends `/enter` once, `/enter` twice at the same moment, or `/undo`, each only after `--before-action` passes, and keeps the Mac's reply verbatim. Set `WATCH_SIM_STATE=<dir>` to keep the watch's pairing in that folder.
- `watch-sim selftest-gate --hook <sh>` runs that same gate with no network. `WATCH_SIM_DRY_RUN=1` makes every command skip the network and print a `trace` of the steps in order (the self-test uses it).
- `watch-sim action --path /enter|/undo --id <uuid> --before-action <sh>` sends one action for an earlier recording, only after the gate passes. With no `--before-action` it sends nothing and exits 1.
- `watch-sim match --expected <text> --actual <text>` prints the ordered recall (`percent`) and `precision`.
- `watch-sim health` sends `/health` with the stored token.
- `scripts/verify-watch-e2e.sh --front-check <textedit|terminal|chrome> <handle>` is the gate command watch-sim runs. It exits 0 only if that window is in front.
