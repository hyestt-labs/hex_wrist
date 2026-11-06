#!/bin/bash
# One command that proves Wristper Test.app takes Apple Watch dictations over
# the production relay and pastes them into real apps, with no human in the
# loop once the one-time permissions are granted.
#
#   stage 1  watch-sim (the shipping watch protocol code as a macOS CLI):
#            pairing, then English and Mandarin into TextEdit and Terminal
#   stage 2  the real watch app (DEBUG build) in a watchOS Simulator, same cases
#   stage 3  scenarios no unit test covers: keep_on_clipboard with an app
#            switch, Enter and Undo (TextEdit, Terminal cat, a plain shell, a
#            recorder window), a double Enter, a Chromium text field, and with
#            --with-agents interactive claude and codex
#   stage 4  the B2 live tests (cargo nextest, #[ignore] watch_live). Opt in
#            with --with-live: they close the FRONT TextEdit document and the
#            FRONT Terminal window, so they are not part of --all.
#
# `--all` (the default) runs stages 1 to 3 and ends with one PASS/FAIL table.
#
# THE RULE THIS SCRIPT IS BUILT AROUND: it acts only on what it made itself and
# can prove it made. Every process, window, tty, temp path, simulator, and app
# it starts is written into one registry ($tmp/registry.tsv) at the moment it is
# made. Every close, kill, trash, focus, read, and input (audio, Enter, Undo)
# goes through the one gate, `owned`, which refuses anything not in the registry,
# logs it, and marks the row INCOMPLETE. Nothing relies on the front window, the
# front document, "whatever was running at the start", or a whole tty.
#
# TERMINAL GATE: Terminal has never been driven by this script on a real Mac, and the person's own terminal sessions
# live there. No stage that uses Terminal (stage 1 or 2 with --targets terminal, stage 3, stage 4) runs until
# `--check-terminal` has passed on this Mac for this version of the Terminal code: it opens ONE Terminal window of its
# own, records its tty, reads text back, closes it, and touches no other window. See terminal_gate_ok.
#
# Exit status: 0 every case passed, 1 a case failed, 2 bad usage or a tool is
# missing, 3 blocked on a human (a permission, a locked screen, an app already
# running; the output lists each step), 4 nothing failed but a row is BLOCKED
# or INCOMPLETE and so is not proven. The script never grants a permission.
#
# Never touches /Applications/Wristper.app, its data folder
# (~/Library/Application Support/Wristper), its relay room, or its Keychain
# items: everything here runs under Wristper Test (dev.wristper.mac.test).
set -uo pipefail

self=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)/$(basename -- "$0")
root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
mode=run
stage=all
build=1
targets=textedit,terminal
langs=en,zh
keep=0
with_agents=0
with_live=0
c1=0
preflight_only=0
only=
fc_kind=
fc_handle=
usage() {
  echo "usage: $0 [--all] [--stage 1|2|3|4|all] [--skip-build] [--targets textedit,terminal] [--langs en,zh]"
  echo "          [--only <regex on scenario names>] [--with-agents] [--with-live] [--c1] [--preflight-only]"
  echo "          [--keep-running] [--selftest] [--check-textedit] [--check-terminal]"
}
while [ $# -gt 0 ]; do
  case "$1" in
    --all) stage=all; shift ;;
    --stage) stage=${2:-}; shift 2 ;;
    --skip-build) build=0; shift ;;
    --targets) targets=$2; shift 2 ;;
    --langs) langs=$2; shift 2 ;;
    --only) only=$2; shift 2 ;;
    --with-agents) with_agents=1; shift ;;
    --with-live) with_live=1; shift ;;
    --c1) c1=1; shift ;;
    --preflight-only) preflight_only=1; shift ;;
    --keep-running) keep=1; shift ;;
    --selftest) mode=selftest; shift ;;
    # Internal: the gate watch-sim runs between /focus and /e2ee (and before /enter and /undo).
    --front-check) mode=front-check; fc_kind=${2:-}; fc_handle=${3:-}; shift 3 ;;
    # Internal: the registered switch-away action (needs this run's registry, found through WATCH_E2E_TMP).
    --switch-away) mode=switch-away; shift ;;
    # A 30-second smoke test of the TextEdit handling on a real Mac: open one document, check it by id, write, read back,
    # close that one, and compare the window list with the one from before. Touches nothing else.
    --check-textedit) mode=check-textedit; shift ;;
    # The smoke test of the Terminal handling. NOT YET RUN on this Mac. Until it passes, no Terminal stage runs (see terminal_gate_ok).
    --check-terminal) mode=check-terminal; shift ;;
    -h|--help) sed -n 2,40p "$0"; echo; usage; exit 0 ;;
    *) echo "unknown argument: $1" >&2; usage >&2; exit 2 ;;
  esac
done
case "$stage" in 1|2|3|4|all) ;; *) echo "--stage must be 1, 2, 3, 4, or all" >&2; exit 2 ;; esac
[ "$stage" != 4 ] || with_live=1
# stage_on N: does this run include stage N? Stage 4 only with --with-live (or --stage 4).
stage_on() {
  if [ "$1" = 4 ]; then [ "$with_live" = 1 ] && { [ "$stage" = all ] || [ "$stage" = 4 ]; }; return; fi
  [ "$stage" = all ] || [ "$stage" = "$1" ]
}

export DEVELOPER_DIR=${DEVELOPER_DIR:-/Applications/Xcode.app/Contents/Developer}
[ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"
unset HEX_APPLICATION_SUPPORT_DIR HEX_WATCH_RELAY_BASE WATCH_E2E_LIVE_TERMINAL_ONCE   # the one-time token is set only by issue_live_terminal_token

# The real watch Swift sources, at pinned commits (never a branch or a working tree, which move).
# WRISTPER_SRC is a checkout of the Wristper Swift repo; the script reads these two commits out of
# its git history (git archive) and builds from that export, so uncommitted edits never leak in.
#   WRISTPER_SRC_COMMIT    the watch app's protocol sources: watch-sim compiles WatchShared and WatchProbe from here
#   WRISTPER_HOOKS_COMMIT  that commit plus the DEBUG-only E2E hooks (branch test/sim-fixture-audio): stage 2 builds this app
WRISTPER_SRC=${WRISTPER_SRC:-/Users/glenhsu/orca/workspaces/wristper/coding}
WRISTPER_SRC_COMMIT=${WRISTPER_SRC_COMMIT:-426185302ac51f77b07fc59878491bdf9a16ce71}
WRISTPER_HOOKS_COMMIT=${WRISTPER_HOOKS_COMMIT:-cf8c14b38cfe40f00df53a35f9cd08149fd87cd4}
# Links the Rust test binary on this Mac (Xcode 26: -lswiftCompatibility56). Optional.
WATCH_E2E_B2_ENV=${WATCH_E2E_B2_ENV:-/Users/glenhsu/orca/workspaces/hex_wrist/migration_to_hex/artifacts/wristper-migration-20261104/b2-test-env/env.sh}
SIGN_IDENTITY=${SIGN_IDENTITY:-Developer ID Application: WEI CHIEH HSU (VQY3J86U79)}
SIM_TYPE=${WATCH_E2E_SIM_TYPE:-com.apple.CoreSimulator.SimDeviceType.Apple-Watch-Series-11-46mm}
WATCH_BUNDLE=dev.wristper.watch.watchkitapp
TEST_BUNDLE=dev.wristper.mac.test
TEST_DATA="$HOME/Library/Application Support/Wristper Test"
OLD_DATA="$HOME/Library/Application Support/Wristper"
EN_TEXT=${WATCH_E2E_EN_TEXT:-Please send the quarterly report to the design team before Friday.}
ZH_TEXT=${WATCH_E2E_ZH_TEXT:-今天下午三點在會議室討論新的產品設計。}
AGENT_TEXT=${WATCH_E2E_AGENT_TEXT:-Reply with only the word pong.}
# Pass thresholds for the read-back, in percent. The score is ordered: the longest common
# subsequence of words (English) or characters (Mandarin) over the expected length
# (watch-sim match). Words out of order do not count; an empty or too short read-back
# (fewer than half the expected units) scores 0. The default Mandarin sentence has 19
# characters, so 90 allows at most one wrong or missing character; the English one has 12
# words, so 80 allows two. Fields that start empty also need the same precision.
EN_PASS=${WATCH_E2E_EN_PASS:-80}
ZH_PASS=${WATCH_E2E_ZH_PASS:-90}
# A page with no user data: one empty textarea, focused on load. Its input
# handler copies the text into the page title, which the isolated Chrome's
# remote debugging port lists over plain HTTP (the read-back).
CHROME_URL='data:text/html,%3Ctextarea%20id%3Dt%20autofocus%20oninput%3D%22document.title%3Dthis.value%22%20style%3D%22width%3A90vw%3Bheight%3A80vh%22%3E%3C%2Ftextarea%3E'
CHROME_BIN="/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"
# Where claude and codex start (they ask to trust a folder they have not seen).
AGENT_DIR=${WATCH_E2E_AGENT_DIR:-$root}
AGENT_BIN=

app="$root/target/app/Wristper Test.app"
hex="$app/Contents/MacOS/hex"
sim="$root/target/watch-sim/watch-sim"
pbtool="$root/target/watch-e2e/pasteboard-tool"

# State set up by init (none of it exists before, so --front-check and --help touch nothing).
out=; results=; run_id=; sim_name=; tmp=; tmp_real=; SC_FILE=
log() { printf '[%s] %s\n' "$(date +%H:%M:%S)" "$*" | tee -a "${out:-/dev/null}/run.log" >&2; }
die() { log "ERROR: $*"; exit "${2:-1}"; }
now_ms() { python3 -c 'import time; print(int(time.time()*1000))'; }
# run_to SECONDS CMD...: a command that a pending permission prompt could hang forever.
run_to() { local s=$1; shift; perl -e 'alarm shift; exec @ARGV' "$s" "$@"; }
real_path() { python3 -c 'import os,sys; print(os.path.realpath(sys.argv[1]))' "$1"; }

# =============================================================================
# The registry and the gate
# =============================================================================
# One tab-separated file, because many calls run in $( ) subshells that cannot
# hand variables back. Fields: kind, key, extra.
#   pid       key = pid (also its process group: every process is started by owned_bg), extra = program marker
#   window    key = textedit:<window id> | terminal:<window id> | chrome:<pid>, extra = file path | tty | debug port
#   tty-proc  key = pid of the one process we run in a Terminal window, extra = that window's tty
#   path      key = one item under $tmp (file, link, or an empty-when-Trashed folder); the root has extra `root`
#   tree      key = a folder under $tmp that a tool fills (Chrome profile, watch-sim state, clipboard backup), extra = owner pid | * | -
#   te-before key = a TextEdit file this run made, extra = the window ids that existed before it was opened (one line)
#   te-keep   key = a TextEdit file that must NOT be Trashed, extra = why (overrides every Trash path)
#   testdata  key = one allowlisted file in Wristper Test's data folder (relay room reset)
#   wt        key = pid of the Wristper Test this run started, extra = its executable
#   sim       key = udid of the simulator this run created, extra = its name
#   switch    key = finder (the one app this run may bring to the front on purpose, in the app-switch scenario)
#   ran       key = app name, extra = running|not-running at the start (written to the log only; it decides nothing)

US=$'\037'   # separates the parts of a registry "extra" field
# A process is identified by its pid, its start time, and its command line together. A pid alone can be reused.
proc_start() { ps -o lstart= -p "$1" 2>/dev/null | sed 's/^ *//; s/ *$//'; }
proc_cmd() { ps -o command= -p "$1" 2>/dev/null; }
epoch_of() { date -j -f '%a %b %e %T %Y' "$1" +%s 2>/dev/null; }
# The one Trash command (a variable only so the self-test can make it fail).
TRASH=/usr/bin/trash

reg_add() { printf '%s\t%s\t%s\n' "$1" "$2" "${3:-}" >> "$tmp/registry.tsv"; }
reg_has() { [ -f "$tmp/registry.tsv" ] && K="$1" V="$2" awk -F'\t' '$1==ENVIRON["K"] && $2==ENVIRON["V"] {f=1} END{exit !f}' "$tmp/registry.tsv"; }
reg_get() { [ -f "$tmp/registry.tsv" ] && K="$1" V="$2" awk -F'\t' '$1==ENVIRON["K"] && $2==ENVIRON["V"] {x=$3} END{print x}' "$tmp/registry.tsv"; }
reg_list() { [ -f "$tmp/registry.tsv" ] && K="$1" awk -F'\t' '$1==ENVIRON["K"] {print $2 "\t" $3}' "$tmp/registry.tsv"; return 0; }
reg_del() {
  [ -f "$tmp/registry.tsv" ] || return 0
  K="$1" V="$2" awk -F'\t' '!($1==ENVIRON["K"] && $2==ENVIRON["V"])' "$tmp/registry.tsv" > "$tmp/registry.new" && mv -f "$tmp/registry.new" "$tmp/registry.tsv"
}

# Refusals: what the gate would not do. A refusal marks the current row INCOMPLETE
# (row-refused) and is listed at the end of the table (refusals.tsv).
refuse() { # kind key action
  log "REFUSED (not owned by this run, nothing done): $1 $2: $3"
  printf '%s\t%s\t%s\n' "$1" "$2" "$3" >> "$out/refusals.tsv"
  printf '%s %s: %s\n' "$1" "$2" "$3" >> "$tmp/row-refused"
}
row_begin() { : > "$tmp/row-refused"; ABORT_NOTE=; }
row_refused() { [ -s "$tmp/row-refused" ]; }

# THE GATE. Every close, kill, trash, focus, read, and input (audio, Enter, Undo)
# goes through here. owned KIND KEY CMD...: runs CMD only when (KIND, KEY) is in
# the registry; otherwise it does nothing, logs, and the row becomes INCOMPLETE.
# The action functions (act_*) then re-check the live identity before they act.
owned() {
  local kind=$1 key=$2
  shift 2
  if ! reg_has "$kind" "$key"; then refuse "$kind" "$key" "$*"; return 99; fi
  "$@"
}

# --- processes: each in a process group of its own --------------------------------------
# owned_bg MARKER CMD...: starts CMD in a new session (so its group is exactly pid), registers
# it, and leaves it running. The caller redirects stdout and stderr. Sets OWNED_PID.
# owned_fg MARKER CMD...: the same, then waits, and removes the registry entry.
OWNED_PID=
owned_bg() {
  local marker=$1
  shift
  # The new session registers ITSELF (pid, marker, start time) before it execs, so a signal that
  # arrives right after the fork cannot leave a process the registry does not know.
  WATCH_E2E_MARKER="$marker" perl -e '
    use POSIX qw(setsid);
    setsid() or die "setsid: $!";
    my $started = `ps -o lstart= -p $$ 2>/dev/null`; $started =~ s/^\s+|\s+$//g;
    my $marker = delete $ENV{WATCH_E2E_MARKER};
    open(my $fh, ">>", "$ENV{WATCH_E2E_TMP}/registry.tsv") or die "registry: $!";
    print $fh "pid\t$$\t$marker\x1f$started\n"; close $fh;
    exec { $ARGV[0] } @ARGV or die "exec: $!";' "$@" &
  OWNED_PID=$!
}
owned_fg() {
  local rc
  owned_bg "$@"
  wait "$OWNED_PID"; rc=$?
  reg_del pid "$OWNED_PID"
  return $rc
}
group_alive() { ps -axo pgid= | awk -v g="$1" '$1==g {f=1} END{exit !f}'; }
# act_stop_group PGID: TERM to the whole group, wait until no member is left, then KILL.
# act_stop_group PGID: TERM to the whole group, wait until no member is left, then KILL. The leader must
# still be the process this run started (pid, start time, and command), before TERM and again before KILL.
# If it cannot be proven (the leader is gone but members remain, or the numbers do not match) nothing is
# killed: the row becomes INCOMPLETE.
act_stop_group() {
  local g=$1 extra marker started own i
  [[ $g =~ ^[0-9]+$ ]] && [ "$g" -gt 1 ] || { refuse pid "$g" "not a usable process group id"; return 1; }
  own=$(ps -o pgid= -p $$ | tr -d ' ')
  [ "$g" != "$own" ] || { refuse pid "$g" "that is this script's own process group"; return 1; }
  extra=$(reg_get pid "$g"); marker=${extra%%"$US"*}; started=${extra#*"$US"}
  group_alive "$g" || return 0
  leader_is_ours "$g" "$marker" "$started" || return 1
  kill -TERM -- "-$g" 2>/dev/null
  for i in $(seq 1 40); do group_alive "$g" || return 0; sleep 0.25; done
  leader_is_ours "$g" "$marker" "$started" || return 1   # it may have exited and its pid been reused meanwhile
  kill -KILL -- "-$g" 2>/dev/null
  for i in $(seq 1 20); do group_alive "$g" || return 0; sleep 0.25; done
  refuse pid "$g" "the process group would not stop"
  return 1
}
leader_is_ours() { # pgid marker started
  if ! kill -0 "$1" 2>/dev/null; then
    refuse pid "$1" "its group still has members but the leader is gone, so the group cannot be proven this run's; not stopped"
    return 1
  fi
  if [ -z "$3" ] || [ "$(proc_start "$1")" != "$3" ]; then
    refuse pid "$1" "the start time does not match the registered one ('$(proc_start "$1")' now, '$3' registered); not stopped"
    return 1
  fi
  if [[ $(proc_cmd "$1") != *"$2"* ]]; then
    refuse pid "$1" "its command line no longer holds '$2'; not stopped"
    return 1
  fi
}
# The registry entry is dropped only when the group is really gone; a refusal or failure leaves it there.
stop_pid() { owned pid "$1" act_stop_group "$1"; local rc=$?; [ $rc = 0 ] && reg_del pid "$1"; return $rc; }
reap_all() { # every registered process group
  local pid rest
  while IFS=$'\t' read -r pid rest; do [ -n "$pid" ] && stop_pid "$pid"; done < <(reg_list pid)
  return 0
}

# --- paths: only what was registered one by one, only under the temp root, never through a link -------
# Three registry kinds decide what may be Trashed (the root being registered authorizes none of its contents):
#   path  one item made by this run: a file or a link (Trashed itself), or a folder that must be EMPTY when it is
#         Trashed (the TextEdit folders; anything left inside, such as a file TextEdit or you put there, keeps it)
#   tree  a folder a tool of this run fills with files this script cannot list one by one (Chrome's profile, the
#         pairing state of watch-sim, the clipboard backup). It is Trashed whole only if it is a real folder (not a
#         link) directly inside the temp root, and the process that fills it is stopped (extra = the owner's pid,
#         `*` = no registered process at all, `-` = no condition)
#   root  the temp root itself (kind path, extra `root`): Trashed last, and only when it is empty
# act_trash_path PATH: Trash one registered item; real location must be inside $tmp, never through a link.
act_trash_path() {
  local p=$1 real rc
  [ -e "$p" ] || [ -L "$p" ] || return 0
  # A file marked te-keep is never Trashed, by any path (the cleanup of the root included).
  if reg_has te-keep "$p"; then log "kept: $p ($(reg_get te-keep "$p"))"; return 97; fi
  # A link is judged by where the link itself sits (Trash moves the link, never its target); anything
  # else, and any path that goes THROUGH a link, by its real location.
  if [ -L "$p" ]; then real="$(real_path "$(dirname -- "$p")")/$(basename -- "$p")"; else real=$(real_path "$p"); fi
  case "$real" in
    "$tmp_real"|"$tmp_real"/*) ;;
    *) refuse path "$p" "its real path $real is outside the temp root $tmp_real"; return 99 ;;
  esac
  if [ -d "$p" ] && [ ! -L "$p" ] && [ -n "$(ls -A -- "$p" 2>/dev/null)" ]; then
    refuse path "$p" "the folder is not empty (it holds: $(ls -A -- "$p" | head -5 | tr '\n' ' ')); kept, only registered items are Trashed one by one"
    return 98
  fi
  "$TRASH" "$p" >/dev/null 2>&1; rc=$?
  if [ $rc -ne 0 ] || [ -e "$p" ] || [ -L "$p" ]; then
    refuse path "$p" "Trash failed (exit $rc) and the path is still there"
    return 1
  fi
}
trash_own() { owned path "$1" act_trash_path "$1"; }   # the item itself must be registered
# act_trash_tree PATH: Trash a registered tree whole, after the checks above.
act_trash_tree() {
  local p=$1 owner
  [ -e "$p" ] || [ -L "$p" ] || return 0
  [ -d "$p" ] && [ ! -L "$p" ] || { refuse tree "$p" "not a real folder (a link or a file)"; return 99; }
  [ "$(real_path "$(dirname -- "$p")")" = "$tmp_real" ] || { refuse tree "$p" "not directly inside the temp root"; return 99; }
  owner=$(reg_get tree "$p")
  case "$owner" in
    -) ;;
    '*') [ -z "$(reg_list pid)" ] || { refuse tree "$p" "a registered process may still be filling it"; return 98; } ;;
    *) ! reg_has pid "$owner" || { refuse tree "$p" "its owner process $owner is still registered (not stopped)"; return 98; } ;;
  esac
  "$TRASH" "$p" >/dev/null 2>&1; local rc=$?
  if [ $rc -ne 0 ] || [ -e "$p" ]; then refuse tree "$p" "Trash failed (exit $rc) and the folder is still there"; return 1; fi
}
trash_tree() { owned tree "$1" act_trash_tree "$1"; }
# The one thing Trashed outside $tmp: four files of Wristper Test's own data folder (its relay room),
# on the allowlist, only while this run's Wristper Test is stopped.
act_trash_testdata() {
  local p=$1 dir name rc
  dir=$(real_path "$(dirname -- "$p")"); name=$(basename -- "$p")
  [ "$dir" = "$(real_path "$TEST_DATA")" ] && [ "$(basename -- "$dir")" = "Wristper Test" ] || { refuse testdata "$p" "not inside Wristper Test's data folder"; return 99; }
  case "$name" in relay-room|relay-secret|paired-devices.json|delivered-ids.json) ;; *) refuse testdata "$p" "not on the allowlist"; return 99 ;; esac
  wt_running && { refuse testdata "$p" "Wristper Test is running"; return 99; }
  [ -e "$p" ] || return 0
  "$TRASH" "$p" >/dev/null 2>&1; rc=$?
  if [ $rc -ne 0 ] || [ -e "$p" ]; then
    refuse testdata "$p" "Trash failed (exit $rc) and the file is still there"
    return 1
  fi
}

# --- Wristper Test: only the one this run started ----------------------------------------
status_json() { "$hex" status --json 2>/dev/null; }
# Pids whose whole command line is exactly the app's executable (hex status, hex watch pair, … carry arguments).
wt_pids() { ps -axo pid=,command= | while read -r pid cmd; do [ "$cmd" = "$hex" ] && echo "$pid"; done; }
wt_running() { status_json >/dev/null || [ -n "$(wt_pids)" ]; }
# act_quit_wt PID: quit the Wristper Test this run started. The registered pid must still be that process
# (pid, start time, executable) BEFORE `hex quit` is sent, and `hex quit` goes only if the running service
# reports that same pid. The same check is repeated before TERM and before KILL.
act_quit_wt() {
  local pid=$1 extra exe started i
  extra=$(reg_get wt "$pid"); exe=${extra%%"$US"*}; started=${extra#*"$US"}
  kill -0 "$pid" 2>/dev/null || return 0   # already gone: nothing to quit
  wt_is_ours "$pid" "$exe" "$started" || return 1
  if [ "$(status_json | jq -r .pid 2>/dev/null)" = "$pid" ]; then
    "$hex" quit >/dev/null 2>&1
    for i in $(seq 1 40); do kill -0 "$pid" 2>/dev/null || return 0; sleep 0.25; done
  fi
  kill -0 "$pid" 2>/dev/null || return 0
  wt_is_ours "$pid" "$exe" "$started" || return 1
  kill -TERM "$pid" 2>/dev/null
  for i in $(seq 1 20); do kill -0 "$pid" 2>/dev/null || return 0; sleep 0.25; done
  wt_is_ours "$pid" "$exe" "$started" || return 1
  kill -KILL "$pid" 2>/dev/null
  for i in $(seq 1 20); do kill -0 "$pid" 2>/dev/null || return 0; sleep 0.25; done
  refuse wt "$pid" "Wristper Test would not stop"
  return 1
}
wt_is_ours() { # pid exe started
  if [ -z "$3" ] || [ "$(proc_start "$1")" != "$3" ] || [ "$(proc_cmd "$1")" != "$2" ]; then
    refuse wt "$1" "that pid is not the Wristper Test this run started (start time '$(proc_start "$1")' vs '$3', command '$(proc_cmd "$1")'); not touched"
    return 1
  fi
}
# A Wristper Test that was launched but not yet registered (a signal or failure between `open` and the
# registration) is found by its exact command line and a start time after the launch: start_app refused
# to continue if one was already running, so any such process is this run's.
adopt_launched_wt() {
  local launched pid started
  launched=$(reg_get wt-launch launch); [ -n "$launched" ] || return 0
  while read -r pid; do
    [ -n "$pid" ] || continue
    reg_has wt "$pid" && continue
    started=$(proc_start "$pid")
    [ "$(epoch_of "$started")" -ge "$((launched - 2))" ] 2>/dev/null && reg_add wt "$pid" "$hex$US$started"
  done < <(wt_pids)
  reg_del wt-launch launch
}
stop_wristper_test() {
  local pid rest
  adopt_launched_wt
  while IFS=$'\t' read -r pid rest; do
    [ -n "$pid" ] || continue
    owned wt "$pid" act_quit_wt "$pid" && reg_del wt "$pid"
  done < <(reg_list wt)
  return 0
}

# --- simulator: only the one this run created ------------------------------------------
simctl_own() { # simctl-subcommand args... : on the simulator this run made, nothing else
  [ -n "${sim_udid:-}" ] || return 98
  owned sim "$sim_udid" xcrun simctl "$@"
}
# A simulator that was created but not yet registered is found by its run-specific name (it holds the run id,
# so no other device can carry it).
adopt_created_sim() {
  [ -n "${sim_udid:-}" ] && return 0
  reg_has sim-creating "$sim_name" || return 0
  local udid
  udid=$(xcrun simctl list devices -j 2>/dev/null | jq -r --arg n "$sim_name" '[.devices[][] | select(.name==$n)][0].udid // empty')
  [ -n "$udid" ] || return 0
  sim_udid=$udid
  reg_add sim "$udid" "$sim_name"
}
delete_sim() {
  adopt_created_sim
  [ -n "${sim_udid:-}" ] || return 0
  owned sim "$sim_udid" xcrun simctl shutdown "$sim_udid" >/dev/null 2>&1
  owned sim "$sim_udid" xcrun simctl delete "$sim_udid" >/dev/null 2>&1
  log "deleted simulator $sim_name ($sim_udid)"
  reg_del sim "$sim_udid"
  sim_udid=
}
sim_udid=

# =============================================================================
# Windows and ttys this run made
# =============================================================================
# Handles:  textedit "<window id>|<file path>"   terminal "<window id>|<tty>"
#           chrome "<pid>:<port>:<profile dir>"
# Each is registered when made (window, and for Terminal also the one process we run
# in it). Nothing refers to the front window or document; before each use the live
# window is compared with the registered id and name or tty.

win_key() { # kind handle
  case "$1" in
    textedit|terminal) echo "$1:${2%%|*}" ;;
    chrome) echo "chrome:${2%%:*}" ;;
  esac
}
win_extra() { # kind handle: what the registry holds for this window
  case "$1" in
    textedit|terminal) echo "${2#*|}" ;;
    chrome) cut -d: -f2 <<<"$2" ;;
  esac
}
# Live identity, four answers: ours (the live window is the registered one), gone (no such window any
# more, or its app is not running: the person closed it, nothing is left to act on), other (the registry
# disagrees, or a Terminal window now shows another tty), unknown (the question itself failed). Only `ours`
# may be acted on; `other` and `unknown` are refusals; `gone` is a quiet success for cleanup.
# TextEdit: this run opens a file of its own (a .txt in a folder under $tmp, never an untitled document: TextEdit
# autosaves an edited untitled document to iCloud Drive and Trash cannot remove that file). The identity is the window
# id plus the registered file path: the live document must still be that file. A file-backed document keeps its name
# and path when it is edited (shown on a real Mac, see docs/features/watch.md); if the path ever differs, the window
# is not the registered one. Terminal: the tty of the tab never changes, so it is still compared.
# An app that is not running is never asked (that would launch it again). An empty answer is never "fine".
win_probe() { # kind handle
  local kind=$1 h=$2 id running got rc
  [ "$(reg_get window "$(win_key "$kind" "$h")")" = "$(win_extra "$kind" "$h")" ] || { echo other; return; }
  case "$kind" in
    textedit|terminal)
      local app=TextEdit; [ "$kind" = terminal ] && app=Terminal
      running=$(osascript -e "application \"$app\" is running" 2>/dev/null); rc=$?
      [ $rc = 0 ] || { echo unknown; return; }
      [ "$running" = true ] || { echo gone; return; }
      id=${h%%|*}
      if [ "$kind" = textedit ]; then
        got=$(osascript -e "tell application \"TextEdit\" to return exists window id $id" 2>/dev/null); rc=$?
        [ $rc = 0 ] || { echo unknown; return; }
        case "$got" in
          true) ;;
          false) echo gone; return ;;
          *) echo unknown; return ;;
        esac
        got=$(osascript -e "tell application \"TextEdit\" to return path of document of window id $id" 2>/dev/null); rc=$?
        [ $rc = 0 ] && [ -n "$got" ] || { echo unknown; return; }
        if [ "$(real_path "$got")" = "$(real_path "${h#*|}")" ]; then echo ours; else echo other; fi
        return
      fi
      got=$(osascript -e 'tell application "Terminal"' -e "if not (exists window id $id) then return \"\"" \
        -e "return tty of selected tab of window id $id" -e 'end tell' 2>/dev/null); rc=$?
      [ $rc = 0 ] || { echo unknown; return; }
      if [ -z "$got" ]; then echo gone; elif [ "$got" = "${h#*|}" ]; then echo ours; else echo other; fi ;;
    chrome) if chrome_ready "$(win_extra chrome "$h")"; then echo ours; else echo gone; fi ;;
  esac
}
win_live() { [ "$(win_probe "$1" "$2")" = ours ]; }

# front_check KIND HANDLE: is the app in front this run's target window? lsappinfo needs no permission.
front_check() {
  local kind=$1 h=$2 asn bid pid
  asn=$(lsappinfo front 2>/dev/null); [ -n "$asn" ] || return 1
  bid=$(lsappinfo info -only bundleid "$asn" 2>/dev/null | sed -nE 's/.*bundleID="([^"]*)".*/\1/p')
  pid=$(lsappinfo info -only pid "$asn" 2>/dev/null | sed -nE 's/.*pid = ([0-9]+).*/\1/p')
  case "$kind" in
    textedit)
      [ "$bid" = com.apple.TextEdit ] || return 1
      # The window id. And the first window is not always the document: about a second after a new document appears TextEdit shows a small
      # panel (no document, not closeable, 280x168) at the top of its window list. So the window asked about is the
      # first one that has a document, not `front window`.
      [ "$(te_first_document_window)" = "${h%%|*}" ] ;;
    terminal)
      [ "$bid" = com.apple.Terminal ] || return 1
      [ "$(osascript -e 'tell application "Terminal" to return ((id of front window) as text) & "|" & (tty of selected tab of front window)' 2>/dev/null)" = "$h" ] ;;
    chrome) [ -n "$pid" ] && [ "$pid" = "${h%%:*}" ] ;;
    *) return 1 ;;
  esac
}
# The one sanctioned way to move focus off the target: bring Finder to the front (the app-switch scenario).
# It is registered at init and run through the gate like everything else.
act_switch_away() { open -a Finder && sleep 2; }
# The id of TextEdit's first (frontmost) window that has a document. Windows without one (the panel TextEdit
# shows a second after a new document appears) are skipped. Empty when there is none.
te_first_document_window() {
  # The window list is taken once, as ids, and each id is then asked about by id: walking `windows` by index would
  # skip or repeat a window when the person reorders windows during the walk.
  osascript -e 'tell application "TextEdit"' -e 'set firstDocumentWindow to ""' \
    -e 'set allIds to id of every window' \
    -e 'repeat with wid in allIds' -e 'try' -e 'set d to document of window id (contents of wid)' \
    -e 'if d is not missing value then' -e 'set firstDocumentWindow to (contents of wid) as text' -e 'exit repeat' -e 'end if' \
    -e 'end try' -e 'end repeat' -e 'return firstDocumentWindow' -e 'end tell' 2>/dev/null
}
# The ids of TextEdit's windows that have a document, sorted. The comparison "the same windows as before" must not
# count the small document-less panel TextEdit shows for about a second after a new document appears (its id
# differs from one moment to the next), so it uses these ids and not `id of every window`.
te_document_window_ids() {
  osascript -e 'tell application "TextEdit"' -e 'set documentWindowIds to {}' \
    -e 'set allIds to id of every window' \
    -e 'repeat with wid in allIds' -e 'try' -e 'set d to document of window id (contents of wid)' \
    -e 'if d is not missing value then set end of documentWindowIds to (contents of wid)' \
    -e 'end try' -e 'end repeat' -e 'return documentWindowIds' -e 'end tell' 2>/dev/null |
    tr ',' '\n' | tr -d ' ' | grep . | sort -n | tr '\n' ' '
}
# Are two lists of ids the same set? (Order does not matter; one id more or less does.)
same_id_set() { [ "$(tr ' ,' '\n\n' <<<"$1" | grep . | sort -n)" = "$(tr ' ,' '\n\n' <<<"$2" | grep . | sort -n)" ]; }

# Two texts are the same when they are identical, character for character. Nothing is trimmed or forgiven: a
# control experiment (TextEdit's own Edit > Paste of this sentence) reads back exactly the sentence, and Hex adds no
# trailing space (it only ever adds a LEADING space between consecutive dictations, src/paste.rs `join` and
# `with_smart_spacing`), so a read-back that differs in any character, a trailing space included, is a real difference.
# (The trailing space that once appeared in results.jsonl was this script's own: it stored the read-back with
# `tr '\r\n' '  '`, which turned the newline `osascript` always prints after its result into a space.)
same_text() { [ "$1" = "$2" ]; }
# The read-back exactly as TextEdit gave it: `$( )` removes only the final newline osascript adds, nothing else.
readback_of() { printf %s "$(cat "$1")"; }

# The gate command watch-sim runs between /focus and /e2ee, and before /enter and /undo.
front_hook() { printf '%q ' /bin/bash "$self" --front-check "$1" "$2"; }

act_focus() { # kind handle
  win_live "$1" "$2" || { refuse window "$(win_key "$1" "$2")" "the live window is not the one this run made ($(win_probe "$1" "$2")); not focused"; return 1; }
  case "$1" in
    textedit) osascript -e 'tell application "TextEdit"' -e 'activate' -e "set index of window id ${2%%|*} to 1" -e 'end tell' >/dev/null ;;
    terminal) osascript -e 'tell application "Terminal"' -e 'activate' -e "set index of window id ${2%%|*} to 1" -e 'end tell' >/dev/null ;;
    chrome)
      # Bring that Chrome process (by pid) to the front: it is not one the person may have open.
      osascript -e 'tell application "System Events"' -e "set frontmost of (first process whose unix id is ${2%%:*}) to true" -e 'end tell' >/dev/null
      sleep 1 ;;
  esac
}
act_read() { # kind handle
  win_live "$1" "$2" || { refuse window "$(win_key "$1" "$2")" "the live window is not the one this run made ($(win_probe "$1" "$2")); not read"; return 1; }
  case "$1" in
    textedit) osascript -e "tell application \"TextEdit\" to return text of document of window id ${2%%|*}" ;;
    terminal) osascript -e "tell application \"Terminal\" to return contents of selected tab of window id ${2%%|*}" ;;
    chrome) chrome_page "$(win_extra chrome "$2")" ;;
  esac
}
# act_stop_tty_proc PID TTY: HUP the one process this run started in its window (never the tty).
# act_stop_tty_proc PID TTY: HUP the one process this run started in its window. Besides the tty it must
# match the start time and command line recorded when the window was made (a reused pid on the same tty
# is not ours), before HUP and again before KILL. Never the whole tty.
act_stop_tty_proc() {
  local pid=$1 tty=$2 extra rtty started cmd i
  extra=$(reg_get tty-proc "$pid"); rtty=${extra%%"$US"*}; extra=${extra#*"$US"}; started=${extra%%"$US"*}; cmd=${extra#*"$US"}
  kill -0 "$pid" 2>/dev/null || return 0   # gone: nothing left to stop
  tty_proc_is_ours "$pid" "$tty" "$rtty" "$started" "$cmd" || return 1
  kill -HUP "$pid" 2>/dev/null
  for i in $(seq 1 20); do kill -0 "$pid" 2>/dev/null || return 0; sleep 0.25; done
  tty_proc_is_ours "$pid" "$tty" "$rtty" "$started" "$cmd" || return 1
  kill -KILL "$pid" 2>/dev/null
  for i in $(seq 1 20); do kill -0 "$pid" 2>/dev/null || return 0; sleep 0.25; done
  refuse tty-proc "$pid" "the process in $tty would not stop"
  return 1
}
tty_proc_is_ours() { # pid tty registered-tty started cmd
  local seen
  seen=$(ps -o tty= -p "$1" 2>/dev/null | tr -d ' ')
  if [ "$3" != "$2" ] || { [ "/dev/$seen" != "$2" ] && [ "$seen" != "${2#/dev/}" ]; }; then
    refuse tty-proc "$1" "it is on '$seen' now, not on the registered $2; not touched"; return 1
  fi
  if [ -z "$4" ] || [ "$(proc_start "$1")" != "$4" ] || [ "$(proc_cmd "$1")" != "$5" ]; then
    refuse tty-proc "$1" "its start time or command line differs from the registered ones (a reused pid?); not touched"; return 1
  fi
}
# act_write KIND HANDLE TEXT: put text into the window this run made (the smoke test; the watch itself pastes).
act_write() {
  win_live "$1" "$2" || { refuse window "$(win_key "$1" "$2")" "the live window is not the one this run made ($(win_probe "$1" "$2")); nothing written"; return 1; }
  case "$1" in
    textedit) osascript -e "tell application \"TextEdit\" to set text of document of window id ${2%%|*} to \"$3\"" ;;
    # Terminal: the text is sent as input to the one process running in this window (the smoke test's cat).
    terminal) osascript -e "tell application \"Terminal\" to do script \"$3\" in selected tab of window id ${2%%|*}" ;;
    *) return 1 ;;
  esac
}
write_target() { local kind; kind=$(kind_of "$1"); owned window "$(win_key "$kind" "$2")" act_write "$kind" "$2" "$3"; }
# What the temp file holds when the window closes: information for the log, never a decision. TextEdit may or may not
# have autosaved the paste into it by then; it is this run's own file either way.
note_te_file() { # handle
  local f=${1#*|} bytes
  bytes=$(wc -c < "$f" 2>/dev/null | tr -d ' ')
  log "TextEdit window ${1%%|*}: this run's file $f holds ${bytes:-?} byte(s) at close"
}

# --- the TextEdit file and the windows that show it ----------------------------------------------------------
# Rule: a TextEdit file and its folder are Trashed only when this run can PROVE that no window shows the file any
# more. Whatever cannot be proven (a failed or unreadable scan, a window that will not close, a window that was there
# before this run opened the file, an error between opening the file and registering its window) is KEPT, with the
# reason in the log and in the registry (te-keep), and te-keep beats every Trash path (act_trash_path checks it).
te_keep() { # file why
  reg_has te-keep "$1" || reg_add te-keep "$1" "$2"
  log "KEEPING $1 (not Trashed): $2"
}
# Characters a temp path may hold. Paths go into AppleScript text, so nothing needing escaping is accepted at all.
safe_temp_path() { [[ $1 =~ ^/[A-Za-z0-9._/-]+$ ]]; }
# Is TextEdit running? Says true, false, or nothing (the question failed: callers keep everything).
te_running() { local a; a=$(osascript -e 'application "TextEdit" is running' 2>/dev/null) || return 1; case "$a" in true|false) echo "$a" ;; *) return 1 ;; esac; }
# Every window id of TextEdit, space-separated; fails unless the answer is whole (ids one per line, END last).
te_window_ids() {
  local raw last line
  raw=$(osascript -e 'tell application "TextEdit"' -e 'set idsBeforeOpen to id of every window' \
    -e "set AppleScript's text item delimiters to linefeed" -e 'if (count of idsBeforeOpen) is 0 then return "END"' \
    -e 'return (idsBeforeOpen as text) & linefeed & "END"' -e 'end tell' 2>/dev/null) || return 1
  last=${raw##*$'\n'}; [ "$last" = END ] || return 1
  while IFS= read -r line; do
    [ "$line" = END ] && continue
    [[ $line =~ ^[0-9]+$ ]] || return 1
    printf '%s ' "$line"
  done <<<"$raw"
}
# One scan of the TextEdit windows that have a document: "<id>|<path>" per line. A failed osascript, no END line, or any
# line that is not "<digits>|<text, possibly empty>" is a failed scan (returns 1): nothing is guessed from a broken answer.
te_scan() {
  local raw last line
  raw=$(osascript -e 'tell application "TextEdit"' -e 'set found to {}' -e 'set orphanScan to id of every window' \
    -e 'repeat with wid in orphanScan' -e 'try' -e 'set d to document of window id (contents of wid)' \
    -e 'set end of found to ((contents of wid) as text) & "|" & (path of d)' \
    -e 'end try' -e 'end repeat' -e "set AppleScript's text item delimiters to linefeed" \
    -e 'if (count of found) is 0 then return "END"' -e 'return (found as text) & linefeed & "END"' -e 'end tell' 2>/dev/null) || return 1
  last=${raw##*$'\n'}; [ "$last" = END ] || return 1
  while IFS= read -r line; do
    [ "$line" = END ] && continue
    [[ $line =~ ^[0-9]+\| ]] || return 1   # an untitled document has an empty path: "<id>|"
    printf '%s\n' "$line"
  done <<<"$raw"
}
# The ids of the windows that show FILE (by real path), space-separated; fails when the scan fails. Other documents'
# paths are only compared, never put into a command.
te_windows_showing() { # file
  local f=$1 scan line id p found=
  scan=$(te_scan) || return 1
  while IFS= read -r line; do
    [ -n "$line" ] || continue
    id=${line%%|*}; p=${line#*|}
    [ "${p##*/}" = "${f##*/}" ] || continue
    [ "$(real_path "$p")" = "$(real_path "$f")" ] && found="$found$id "
  done <<<"$scan"
  printf '%s' "$found"
}
# Close one TextEdit window by id (the caller has proven it ours) and check it is gone.
te_close_by_id() { # id
  osascript -e "tell application \"TextEdit\" to close window id $1 saving no" >/dev/null 2>&1
  sleep 0.5
  [ "$(osascript -e "tell application \"TextEdit\" to return exists window id $1" 2>/dev/null)" = false ]
}
# Windows that appeared after this run opened FILE and show it (a second window on the same file) belong to this
# run and are registered; a window that was already there before shows a file that did not exist then, so it cannot
# be ours and nothing is closed or Trashed on its account. A scan that fails keeps the file.
te_adopt_orphans() {
  local f before ids id
  te_running >/dev/null || { while IFS=$'\t' read -r f _; do te_keep "$f" "could not ask whether TextEdit is running"; done < <(reg_list te-before); return 0; }
  [ "$(te_running)" = true ] || return 0
  while IFS=$'\t' read -r f before; do
    [ -n "$f" ] || continue
    ids=$(te_windows_showing "$f") || { te_keep "$f" "the scan of TextEdit's windows failed or could not be read"; continue; }
    for id in $ids; do
      if [[ " $before " == *" $id "* ]]; then
        te_keep "$f" "window $id was already open before this run opened the file; it is not this run's to close"
      elif ! reg_has window "textedit:$id"; then
        log "TextEdit window $id appeared after this run opened $f and shows it; registering it so it is closed"
        reg_add window "textedit:$id" "$f"
      fi
    done
  done < <(reg_list te-before)
}
# The last step for a TextEdit file: close every window that shows it, prove none is left, then Trash the file and
# (only when empty) its folder. Anything unproven keeps the file (see the rule above).
te_settle_file() { # file
  local f=$1 before ids id d rest running
  d=$(dirname -- "$f")
  if reg_has te-keep "$f"; then log "kept: $f ($(reg_get te-keep "$f"))"; return 1; fi
  running=$(te_running) || { te_keep "$f" "could not ask whether TextEdit is running"; return 1; }
  if [ "$running" = true ] && { [ -e "$f" ] || [ -L "$f" ]; }; then
    before=$(reg_get te-before "$f")
    ids=$(te_windows_showing "$f") || { te_keep "$f" "the scan of TextEdit's windows failed or could not be read"; return 1; }
    for id in $ids; do
      if [[ " $before " == *" $id "* ]]; then te_keep "$f" "window $id was already open before this run opened the file"; return 1; fi
      reg_has window "textedit:$id" || reg_add window "textedit:$id" "$f"
      if [ "$(win_probe textedit "$id|$f")" = ours ] && owned window "textedit:$id" te_close_by_id "$id"; then
        reg_del window "textedit:$id"
      else
        te_keep "$f" "window $id shows the file and did not close"; return 1
      fi
    done
    ids=$(te_windows_showing "$f") || { te_keep "$f" "the scan after closing failed or could not be read"; return 1; }
    [ -z "$ids" ] || { te_keep "$f" "window(s) ${ids}still show the file after closing"; return 1; }
  fi
  act_trash_te_file "$f" || return 1
}
# The Trash of the TextEdit file this run made, then of the folder it sits in. Each goes through the gate (only a
# registered item), act_trash_path (inside the temp root, never through a link, a folder only when empty, never a
# te-keep file), and a shape check here: a regular file named wristper-verify-*.txt in a textedit.* folder.
act_trash_te_file() { # path
  local f=$1 d
  d=$(dirname -- "$f")
  case "$(basename -- "$f")" in wristper-verify-*.txt) ;; *) refuse path "$f" "not a file this run's TextEdit step names (wristper-verify-*.txt)"; return 99 ;; esac
  case "$(basename -- "$d")" in textedit.*) ;; *) refuse path "$f" "not inside a textedit.* folder this run made"; return 99 ;; esac
  [ -L "$f" ] && { refuse path "$f" "it is a link"; return 99; }
  owned path "$f" act_trash_path "$f" || return 1
  owned path "$d" act_trash_path "$d"
}
act_close() { # kind handle
  local kind=$1 h=$2 key state pid rest tty found=0 failed=0
  key=$(win_key "$kind" "$h")
  state=$(win_probe "$kind" "$h")
  case "$state" in
    ours|gone) ;;
    *) refuse window "$key" "the window's identity is '$state' (not the one this run made); not closed"; return 1 ;;
  esac
  case "$kind" in
    textedit)
      # Close the window (only when it is proven ours; checked gone), then settle its file: every window that shows it
      # must be closed and proven gone before the file and its (empty) folder are Trashed; else they stay.
      if [ "$state" = ours ]; then
        note_te_file "$h"
        te_close_by_id "${h%%|*}" \
          || { refuse window "$key" "TextEdit did not close the window; its file is left in place"; te_keep "${h#*|}" "window ${h%%|*} did not close"; return 1; }
      fi
      te_settle_file "${h#*|}" || return 1 ;;
    terminal)
      # The window is closed only after the one process this run started in it is proven ours and stopped.
      # Without that proof, or if stopping fails, the window stays open and the row is INCOMPLETE. (If the
      # person already closed the window, the process is still stopped when it is proven ours; no proof
      # needed when nothing is left.)
      tty=${h#*|}
      while IFS=$'\t' read -r pid rest; do
        [ "${rest%%"$US"*}" = "$tty" ] || continue
        found=1
        if owned tty-proc "$pid" act_stop_tty_proc "$pid" "$tty"; then reg_del tty-proc "$pid"; else failed=1; fi
      done < <(reg_list tty-proc)
      if [ "$state" = ours ]; then
        if [ $found = 0 ]; then
          refuse window "$key" "no registered process proves who runs in $tty; the window is left open"
          return 1
        fi
        [ $failed = 0 ] || return 1
        sleep 1
        osascript -e "tell application \"Terminal\" to close window id ${h%%|*}" >/dev/null 2>&1
      else
        [ $failed = 0 ] || return 1
      fi ;;
    chrome) chrome_close "$h" ;;
  esac
  reg_del window "$key"
}
# The public names: all of them go through the gate.
focus_target() { # target handle
  local kind; kind=$(kind_of "$1")
  owned window "$(win_key "$kind" "$2")" act_focus "$kind" "$2"
}
read_target() {
  local kind; kind=$(kind_of "$1")
  owned window "$(win_key "$kind" "$2")" act_read "$kind" "$2"
}
close_target() {
  local kind; kind=$(kind_of "$1")
  owned window "$(win_key "$kind" "$2")" act_close "$kind" "$2"
}

# TextEdit: a document backed by a file this run made. An untitled document would be autosaved by TextEdit into iCloud
# Drive (and renamed after its first line), and /usr/bin/trash cannot remove that file (NSCocoaErrorDomain 513). So the
# script makes an empty .txt in its own folder under $tmp (in $TMPDIR, not in iCloud) and opens that file. The folder
# and the file are registered one by one, and so are the ids of all windows that exist BEFORE the file is opened (te-before),
# which is what later tells this run's windows from yours. The window is the new one (an id not in that list) whose
# document is named like the file. Prints "<window id>|<file path>". Any error after the file may have been opened and
# before its window is registered keeps the file (te-keep): nothing is Trashed on a guess.
te_open() {
  local dir file name before before_as ans live id
  dir=$(mktemp -d "$tmp/textedit.XXXXXX") || return 1
  reg_add path "$dir"
  name="wristper-verify-$RANDOM$RANDOM.txt"
  file="$dir/$name"
  safe_temp_path "$file" || { log "the TextEdit file path '$file' has characters outside A-Za-z0-9._/-; nothing is opened"; return 1; }
  before=$(te_window_ids) || { log "could not list TextEdit's windows before opening the file; nothing is opened"; return 1; }
  : > "$file" || return 1
  reg_add path "$file"
  reg_add te-before "$file" "$before"
  before_as="{$(tr -s ' ' '\n' <<<"$before" | grep . | paste -sd, -)}"
  ans=$(osascript -e 'tell application "TextEdit"' -e 'activate' \
    -e "set idsBefore to $before_as" \
    -e "open (POSIX file \"$file\")" \
    -e 'set wid to ""' \
    -e 'repeat 20 times' \
    -e 'set idsNow to id of every window' \
    -e 'repeat with i in idsNow' \
    -e 'if idsBefore does not contain (contents of i) then' \
    -e 'try' \
    -e "if (name of document of window id (contents of i)) is \"$name\" then set wid to (contents of i) as text" \
    -e 'end try' \
    -e 'end if' \
    -e 'end repeat' \
    -e 'if wid is not "" then exit repeat' \
    -e 'delay 0.25' \
    -e 'end repeat' \
    -e 'if wid is "" then error "TextEdit opened no window for the file"' \
    -e "return wid & \"|$file\"" \
    -e 'end tell' 2>&1) || { te_keep "$file" "opening the file did not give a window id ($ans)"; return 1; }
  [[ $ans =~ ^[0-9]+\|/ ]] || { te_keep "$file" "unreadable answer after opening the file ($ans)"; return 1; }
  id=${ans%%|*}
  live=$(osascript -e "tell application \"TextEdit\" to return path of document of window id $id" 2>/dev/null)
  [ -n "$live" ] && [ "$(real_path "$live")" = "$(real_path "$file")" ] || { te_keep "$file" "window $id does not show the file (it shows '$live')"; return 1; }
  echo "$ans"
}
# Terminal: a new window running CMD. The command first writes its own pid, then (usually) exec's the
# program, so the pid is the program's. Prints "<window id>|<tty>".
term_open() { # shell-command delay-seconds
  local cmd=${1//\\/\\\\}
  cmd=${cmd//\"/\\\"}   # the command goes inside an AppleScript string
  osascript -e 'tell application "Terminal"' -e 'activate' -e "set t to do script \"$cmd\"" -e "delay $2" \
    -e 'set theTTY to tty of t' \
    -e 'repeat with w in windows' -e 'try' -e 'if (tty of selected tab of w) is theTTY then return ((id of w) as text) & "|" & theTTY' \
    -e 'end try' -e 'end repeat' -e 'end tell'
}

kind_of() {
  case "$1" in
    textedit) echo textedit ;;
    terminal|shell|recorder|claude|codex) echo terminal ;;
    chrome) echo chrome ;;
  esac
}

# Chrome runs as its own process (own session, own profile folder under $tmp): the user's Chrome, its
# windows, profile, and settings are never used or changed. No Apple event is sent to Chrome.
chrome_open() {
  local dir port pid i
  [ -x "$CHROME_BIN" ] || return 1
  dir=$(mktemp -d "$tmp/chrome.XXXXXX") || return 1
  reg_add path "$dir.log"
  port=$(python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1",0)); print(s.getsockname()[1])')
  # --use-mock-keychain and basic password store: a fresh profile must not touch the login Keychain.
  owned_bg "Google Chrome" "$CHROME_BIN" --user-data-dir="$dir" --remote-debugging-port="$port" --no-first-run \
    --no-default-browser-check --use-mock-keychain --password-store=basic --disable-extensions --disable-sync \
    --window-size=900,700 --new-window "$CHROME_URL" > "$dir.log" 2>&1
  pid=$OWNED_PID
  reg_add tree "$dir" "$pid"
  reg_add window "chrome:$pid" "$port"
  for i in $(seq 1 40); do
    chrome_ready "$port" && { echo "$pid:$port:$dir"; return 0; }
    kill -0 "$pid" 2>/dev/null || break
    sleep 0.5
  done
  chrome_close "$pid:$port:$dir"
  return 1
}
# True once the debugging port lists the throwaway page.
chrome_ready() {
  curl -s -m 3 "http://127.0.0.1:$1/json" 2>/dev/null |
    jq -e 'map(select(.type=="page" and (.url | startswith("data:text/html")))) | length > 0' >/dev/null 2>&1
}
# The throwaway page's title as the debugging port lists it ("" until text is typed).
chrome_page() {
  curl -s -m 3 "http://127.0.0.1:$1/json" 2>/dev/null |
    jq -r '[.[] | select(.type=="page")][0] | .title // empty | select(startswith("data:text/html") | not)' 2>/dev/null
}
chrome_close() { # handle pid:port:dir
  local pid=${1%%:*} rest=${1#*:} dir
  dir=${rest#*:}
  stop_pid "$pid"
  trash_tree "$dir"; trash_own "$dir.log"
  reg_del window "chrome:$pid"
}

# --- the Terminal gate ----------------------------------------------------------------------------------------
# A stamp file proves that `--check-terminal` passed on THIS Mac, for THIS script, within the last 24 hours. It sits at
# one fixed place (no environment variable names another) and holds the Mac's hardware UUID (IOPlatformUUID), the
# sha256 of the whole script file, and the time it was issued. The gate checks all three: another Mac's stamp, a stamp
# for a script that has been edited since (so every commit that touches this file closes the gate again), an expired
# stamp, and one dated in the future are all refused; a Mac whose UUID cannot be read never opens it.
# TERMINAL_SMOKE is set only inside check_terminal (the smoke test has to open its own window before any stamp exists).
TERMINAL_STAMP="$root/target/watch-e2e/terminal-smoke.ok"
TERMINAL_STAMP_MAX_AGE=86400
TERMINAL_SMOKE=0
machine_id() { ioreg -rd1 -c IOPlatformExpertDevice 2>/dev/null | sed -n 's/.*"IOPlatformUUID" = "\([0-9A-Fa-f-][0-9A-Fa-f-]*\)".*/\1/p' | head -1; }
script_sha() { shasum -a 256 "${1:-$self}" 2>/dev/null | cut -d' ' -f1; }
# Prints why the gate is closed, or nothing when it is open.
terminal_gate_problem() {
  local host sha issued me now age
  [ -f "$TERMINAL_STAMP" ] || { echo "there is no stamp ($TERMINAL_STAMP): --check-terminal has not passed"; return 0; }
  host=$(sed -n 's/^host=//p' "$TERMINAL_STAMP"); sha=$(sed -n 's/^script=//p' "$TERMINAL_STAMP"); issued=$(sed -n 's/^issued=//p' "$TERMINAL_STAMP")
  me=$(machine_id)
  [ -n "$me" ] || { echo "this Mac's hardware UUID cannot be read, so no stamp can be tied to it"; return 0; }
  [ "$host" = "$me" ] || { echo "the stamp is from another Mac (its host '$host', this one '$me')"; return 0; }
  [ -n "$sha" ] && [ "$sha" = "$(script_sha)" ] || { echo "the script has changed since the stamp was issued (the stamp covers the whole file's sha256)"; return 0; }
  [[ $issued =~ ^[0-9]+$ ]] || { echo "the stamp has no valid issue time"; return 0; }
  now=$(date +%s); age=$((now - issued))
  [ $age -ge 0 ] || { echo "the stamp is dated in the future"; return 0; }
  [ $age -le $TERMINAL_STAMP_MAX_AGE ] || { echo "the stamp is $((age / 3600)) hours old (it lasts $((TERMINAL_STAMP_MAX_AGE / 3600)) hours)"; return 0; }
  return 0
}
terminal_gate_ok() { [ -z "$(terminal_gate_problem)" ]; }
# The one-time token for the Rust live tests (src/watch/live_tests.rs). It exists only after the Terminal gate is open,
# in a file of this run's temp root and in an environment variable of the cargo child; the test that uses it empties the
# file (consumes it), so a second use, or a cargo run started by hand, has none.
issue_live_terminal_token() {
  local token
  terminal_gate_ok || return 1
  token=$(od -An -N16 -tx1 /dev/urandom | tr -d ' \n')
  [ ${#token} = 32 ] || return 1
  reg_add path "$tmp/live-terminal-once"
  printf %s "$token" > "$tmp/live-terminal-once" || return 1
  export WATCH_E2E_LIVE_TERMINAL_ONCE="$token"
}
# Stage 4's Rust tests (src/watch/live_tests.rs) still make UNTITLED TextEdit documents, which TextEdit autosaves into
# iCloud Drive where Trash cannot remove them (the problem the file-backed documents above avoid). Until they open a
# file of their own, stage 4 does not run. Returns 0 (and adds a blocked step) when this run includes stage 4.
stage4_block() {
  stage_on 4 || return 1
  need "Stage 4 (--with-live, --stage 4) is off for now: its Rust tests still open untitled TextEdit documents, which TextEdit saves into iCloud Drive and /usr/bin/trash cannot remove. They must open a file in a temp folder first (as scripts/verify-watch-e2e.sh does for stages 1 to 3). Nothing was run."
}
# Adds one blocked step and returns 0 when this run uses Terminal and the smoke test has not passed.
terminal_gate_block() {
  uses_app Terminal || return 1
  terminal_gate_ok && return 1
  need "Terminal has not been checked on this Mac (or the check is stale): $(terminal_gate_problem). Your own terminal sessions are there. Run \"$self\" --check-terminal first (it opens one Terminal window of its own, reads text back, closes it, and touches no other window), then run again. Until it passes, no stage that uses Terminal runs; --stage 1 --targets textedit does not need it."
}

# Prints a handle for the opened target and registers what was made. open_target runs in a subshell,
# which is why the registry is a file.
open_target() {
  local h= tty pf pid seen
  case "$1" in
    textedit)
      h=$(te_open) || return 1
      # Only a real "<window id>|<file path>" is registered. Anything else (an error text, an empty answer) is
      # not a made window: nothing is registered and the row is INCOMPLETE. The file is already registered
      # (te_open); the path must be inside this run's temp root, else the window is not registered at all.
      [[ $h =~ ^[0-9]+\|/.+$ ]] || { log "TextEdit did not return a window id and file path: '${h:0:120}'"; return 1; }
      case "$(real_path "${h#*|}")" in "$tmp_real"/*) ;; *) te_keep "${h#*|}" "the file is not inside the temp root; its window was not registered"; log "TextEdit's file ${h#*|} is not inside the temp root; not registered"; return 1 ;; esac
      reg_add window "textedit:${h%%|*}" "${h#*|}" ;;
    terminal|shell|recorder|claude|codex)
      if [ "$TERMINAL_SMOKE" != 1 ] && ! terminal_gate_ok; then
        log "REFUSED: no Terminal window is opened until --check-terminal has passed on this Mac for this version of the script"
        return 1
      fi
      pf=$(mktemp "$tmp/pid.XXXXXX") || return 1
      reg_add path "$pf"
      case "$1" in
        terminal) h=$(term_open "echo \$\$ > '$pf'; clear; exec cat" 1) ;;
        shell) h=$(term_open "echo \$\$ > '$pf'; clear" 1.5) ;;
        # Prints one ENTER:<line> for every Return it receives, so an extra, empty Return shows as "ENTER:".
        recorder) h=$(term_open "echo \$\$ > '$pf'; clear; while IFS= read -r l; do echo \"ENTER:\$l\"; done" 1.5) ;;
        claude|codex) h=$(term_open "echo \$\$ > '$pf'; cd '$AGENT_DIR' && clear && exec '$AGENT_BIN'" 12) ;;
      esac
      [ -n "$h" ] || return 1
      tty=${h#*|}
      reg_add window "terminal:${h%%|*}" "$tty"
      pid=$(tr -d ' \n' < "$pf" 2>/dev/null)
      seen=$(ps -o tty= -p "${pid:-0}" 2>/dev/null | tr -d ' ')
      if [[ ${pid:-} =~ ^[0-9]+$ ]] && { [ "/dev/$seen" = "$tty" ] || [ "$seen" = "${tty#/dev/}" ]; }; then
        reg_add tty-proc "$pid" "$tty$US$(proc_start "$pid")$US$(proc_cmd "$pid")"
      else
        log "could not prove which process runs in $tty; that window will be left open at the end (INCOMPLETE)"
      fi ;;
    chrome) chrome_open; return ;;
  esac
  [ -n "$h" ] || return 1
  echo "$h"
}

# =============================================================================
# Rows
# =============================================================================
blocked=()
need() { blocked+=("$1"); }
FAILS=
BLOCKED_NOTE=
ABORT_NOTE=
SC_REPORT=null; SC_READBACK=; SC_MS=0; SC_HANDLE=; SC_TARGET=; SC_PERCENT=0; SC_SWITCH=0; SC_EXACT=

# record STAGE TARGET LANG RESULT PERCENT STATE MS TEXT DETAIL_JSON
record() {
  jq -cn --arg stage "$1" --arg target "$2" --arg lang "$3" --arg result "$4" --arg percent "$5" \
    --arg state "$6" --arg ms "$7" --arg text "$8" --argjson detail "${9:-null}" \
    '{stage:$stage,target:$target,lang:$lang,result:$result,match_percent:$percent,state:$state,ms:$ms,text:$text,detail:$detail}' \
    >> "$results"
  log "stage $1 $2 $3: $4 (match ${5}%, state $6, ${7} ms)"
}
rj() { jq -r "$1" <<<"$SC_REPORT" 2>/dev/null; }          # a field of the report
chk() { [ "$1" = ok ] || FAILS="$FAILS; $2"; }              # chk ok|bad "what failed"
is() { if "$@"; then echo ok; else echo bad; fi; }
breaks() { tr -cd '\r\n' < "$SC_FILE" | wc -c | tr -d ' '; }  # line breaks in the read-back
count() { grep -o -- "$1" "$SC_FILE" | wc -l | tr -d ' '; }  # occurrences of a string
# The recorder window echoes ENTER:<line> for each Return. (grep -c, not -q: no SIGPIPE under pipefail.)
enter_lines() { grep -c '^ENTER:[^[:space:]]' "$SC_FILE"; }          # Returns that carried the pasted text
empty_enter_lines() { grep -c '^ENTER:[[:space:]]*$' "$SC_FILE"; }  # Returns on an empty line

# finish STAGE SCENARIO LANG PERCENT: INCOMPLETE when the run aborted the row or the gate refused something
# (nothing was proven); else FAIL when FAILS has anything; else BLOCKED when BLOCKED_NOTE names a missing
# dependency (FAIL under --c1, which asserts the dependency is met); else PASS.
finish() {
  local result=PASS note=${FAILS#; } refused=
  row_refused && refused=$(tr '\n' ';' < "$tmp/row-refused")
  if [ -n "$ABORT_NOTE" ] || [ -n "$refused" ]; then
    result=INCOMPLETE; note="${ABORT_NOTE}${ABORT_NOTE:+ }${refused:+(gate refused: $refused)}"
  elif [ -n "$FAILS" ]; then
    result=FAIL
  elif [ -n "$BLOCKED_NOTE" ]; then
    result=BLOCKED; note=$BLOCKED_NOTE
    [ "$c1" = 1 ] && result=FAIL
  fi
  record "$1" "$2" "$3" "$result" "$4" "$(rj '.state // .event')" "$SC_MS" \
    "$(readback_of "$SC_FILE" | tr '\r\n' '  ' | tail -c 200)" \
    "$(jq -c --arg f "$note" --arg e "$SC_EXACT" --arg r "$(readback_of "$SC_FILE" | head -c 400)" '. + {fails:$f, readback_is_exactly_the_sentence:$e, readback:$r}' <<<"$SC_REPORT")"
  FAILS=; BLOCKED_NOTE=; ABORT_NOTE=; SC_SWITCH=0; SC_EXACT=
}

# =============================================================================
# Teardown: everything this run started, found in the registry
# =============================================================================
CLIP_STATE=none   # none | saved | restored | failed
CLIP_DETAIL=
# Changing the clipboard is an input too: only while a verified backup of the person's clipboard exists.
act_clip_set() { "$pbtool" set "$1"; }
clip_set() { owned clipboard backup act_clip_set "$1"; }
clip_backup() {
  [ "$CLIP_STATE" = none ] || return 0
  local msg rc
  msg=$("$pbtool" save "$tmp/clipboard" 2>&1); rc=$?
  case $rc in
    0) ;;
    3) need "The clipboard holds a concealed item (a password manager's copy). Copy something ordinary, or clear the clipboard, then run again: this script overwrites the clipboard during the run and must be able to put it back."; print_blocked ;;
    4) need "The clipboard holds something this script cannot read completely ($msg). It will not make a partial backup and will not overwrite it. Copy something ordinary, or clear the clipboard, then run again."; print_blocked ;;
    *) die "could not back up the clipboard ($msg); nothing was changed" 2 ;;
  esac
  CLIP_STATE=saved
  reg_add clipboard backup "$tmp/clipboard"
  reg_add tree "$tmp/clipboard" -
  log "clipboard backed up ($msg)"
}
clip_restore() {
  [ "$CLIP_STATE" = saved ] || return 0
  local msg
  if msg=$("$pbtool" restore "$tmp/clipboard" 2>&1) && CLIP_DETAIL=$("$pbtool" verify "$tmp/clipboard" 2>&1); then
    CLIP_STATE=restored
  else
    CLIP_STATE=failed; CLIP_DETAIL="${CLIP_DETAIL:-$msg}"
  fi
  log "clipboard restore: $CLIP_STATE ($CLIP_DETAIL)"
}

# The temp root, item by item. Each root entry is Trashed only if THIS run registered it (path or tree) and the
# checks of act_trash_path / act_trash_tree pass; an entry nobody registered is kept and named in the log, and so is
# the root then. If the clipboard could not be put back, its backup stays. The registry and the row marker go last,
# and the root only when it is empty.
cleanup_root() {
  local entry kept=0 internal
  for entry in "$tmp"/* "$tmp"/.[!.]*; do
    [ -e "$entry" ] || [ -L "$entry" ] || continue
    case "$entry" in "$tmp/registry.tsv"|"$tmp/registry.new"|"$tmp/row-refused") continue ;; esac
    if [ "$entry" = "$tmp/clipboard" ] && { [ "$CLIP_STATE" = failed ] || [ "$CLIP_STATE" = saved ]; }; then kept=1; continue; fi
    if reg_has tree "$entry"; then trash_tree "$entry" || kept=1
    elif reg_has path "$entry"; then trash_own "$entry" || kept=1
    else log "kept: $entry (this run did not register it, so it is not Trashed)"; kept=1
    fi
  done
  if [ "$CLIP_STATE" = failed ] || [ "$CLIP_STATE" = saved ]; then
    log "YOUR CLIPBOARD WAS NOT PUT BACK. The backup is kept at $tmp/clipboard. Restore it with: \"$pbtool\" restore \"$tmp/clipboard\""
    echo "YOUR CLIPBOARD WAS NOT PUT BACK. The backup is kept at $tmp/clipboard" >&2
    echo "Restore it with: \"$pbtool\" restore \"$tmp/clipboard\"" >&2
  fi
  [ $kept = 0 ] || { log "the temp root $tmp stays: it still holds kept items (named above)"; return 0; }
  # Everything else is gone: the script's own three bookkeeping files, then the empty root.
  for internal in registry.tsv registry.new row-refused; do
    [ -e "$tmp/$internal" ] && { "$TRASH" "$tmp/$internal" >/dev/null 2>&1 || { log "kept: $tmp/$internal (Trash failed)"; return 0; }; }
  done
  [ -z "$(ls -A -- "$tmp" 2>/dev/null)" ] && "$TRASH" "$tmp" >/dev/null 2>&1
  return 0
}

keep_setting_original=
TORN_DOWN=0
# A signal that arrives while the cleanup runs must not cut it short.
mask_signals() { trap '' INT TERM HUP; }
teardown() {
  [ "$TORN_DOWN" = 0 ] || return 0
  mask_signals
  TORN_DOWN=1
  [ -n "$tmp" ] || return 0
  local key extra kind id name entry f
  # 1. processes, each as a whole group, until nothing of the group is left
  reap_all
  # 2. TextEdit documents, Terminal windows (and the one process we ran in each), Chrome windows
  te_adopt_orphans
  while IFS=$'\t' read -r key extra; do
    [ -n "$key" ] || continue
    kind=${key%%:*}; id=${key#*:}
    case "$kind" in
      textedit) owned window "$key" act_close textedit "$id|$extra" ;;
      terminal) owned window "$key" act_close terminal "$id|$extra" ;;
    esac
  done < <(reg_list window)
  # 3. the person's clipboard and the setting this run changed
  clip_restore
  if [ -n "$keep_setting_original" ] && [ -x "$hex" ]; then
    "$hex" settings set labs.keep_on_clipboard "$keep_setting_original" >/dev/null 2>&1
  fi
  # 4. Wristper Test (only the one this run started) and the simulator (only the one this run made)
  if [ "$keep" = 0 ]; then
    stop_wristper_test
    delete_sim
  elif [ -n "$sim_udid" ]; then
    log "--keep-running: simulator $sim_name ($sim_udid) stays; remove it with: xcrun simctl delete $sim_udid"
  fi
  # 5. every TextEdit file of this run is settled (windows proven gone, or the file kept), then the temp root is
  # cleaned item by item (see cleanup_root).
  while IFS=$'\t' read -r key extra; do te_settle_file "$key"; done < <(reg_list te-before)
  cleanup_root
  return 0
}
cleanup() {
  trap - EXIT
  mask_signals
  teardown
}

# Where this run may make its temp folder. Prints why DIR is refused, or nothing when it is fine. The folder holds
# the clipboard backup and the TextEdit files, and its path goes into AppleScript text, so: an absolute path of the
# characters A-Za-z0-9._/- only (checked on the value and on its real path, so a link cannot smuggle others in), an
# existing folder, and not under the user's Documents, Desktop, iCloud Drive (Mobile Documents, CloudStorage), or any
# path with iCloud in its name (compared on real paths).
tmpdir_problem() { # dir home
  local d=$1 home=$2 real r rr
  [[ $d =~ ^/[A-Za-z0-9._/-]+$ ]] || { echo "the temp folder '$d' is not an absolute path made only of A-Za-z0-9._/- (the path is put into AppleScript text; nothing is escaped)"; return 0; }
  [ -d "$d" ] || { echo "the temp folder '$d' is not an existing folder"; return 0; }
  real=$(real_path "$d")
  [[ $real =~ ^/[A-Za-z0-9._/-]+$ ]] || { echo "the real path '$real' of the temp folder has characters outside A-Za-z0-9._/-"; return 0; }
  for r in "$home/Library/Mobile Documents" "$home/Library/CloudStorage" "$home/Documents" "$home/Desktop"; do
    rr=$(real_path "$r")
    case "$real/" in "$r"/*|"$rr"/*) echo "the temp folder '$d' is under $r (a folder that is synced or that you use): refused"; return 0 ;; esac
  done
  case "$real" in *[Ii][Cc]loud*) echo "the temp folder '$d' has iCloud in its path: refused"; return 0 ;; esac
  return 0
}

init() { # prefix
  # The traps come first, before anything is made, so a signal or a die at any later point still runs
  # the cleanup (which only ever touches what the registry holds).
  trap cleanup EXIT
  trap 'exit 130' INT
  trap 'exit 143' TERM HUP
  local base=${TMPDIR:-/tmp} problem
  problem=$(tmpdir_problem "$base" "$HOME")
  [ -z "$problem" ] || { echo "$problem" >&2; exit 2; }
  out="$root/target/watch-e2e/${1}$(date +%Y%m%d-%H%M%S)"
  mkdir -p "$out"
  results="$out/results.jsonl"
  : > "$results"
  run_id=$(basename "$out")
  # A simulator is made fresh for this run and deleted at its end. It is never reused or erased.
  sim_name="Wristper E2E $run_id"
  # Secrets live only here: the clipboard backup, the watch's pairing keys, raw logs that hold pairing
  # codes, the registry, Chrome's profile. Mode 0700, moved to the Trash at exit.
  tmp=$(mktemp -d "$base/wristper-e2e.XXXXXX") || { echo "cannot make a temp folder" >&2; exit 2; }
  tmp_real=$(real_path "$tmp")
  : > "$tmp/registry.tsv"; : > "$out/refusals.tsv"; : > "$tmp/row-refused"
  reg_add path "$tmp" root
  reg_add switch finder "the app-switch scenario"
  export WATCH_E2E_TMP="$tmp" WATCH_E2E_OUT="$out"   # the gate's helper commands find the registry through these
  SC_FILE="$out/readback.txt"
}

# =============================================================================
# Preflight
# =============================================================================
# Everything that needs a human is collected here and reported once, as a BLOCKED
# block with exit status 3. Nothing here tries to grant anything.

# The app that runs this script: TCC attributes Automation and Accessibility to it.
runner_app() {
  local pid=$$ comm name i
  for i in 1 2 3 4 5 6 7 8 9 10 11 12; do
    pid=$(ps -o ppid= -p "$pid" 2>/dev/null | tr -d ' ')
    [ -n "$pid" ] && [ "$pid" -gt 1 ] || break
    comm=$(ps -o comm= -p "$pid" 2>/dev/null)
    case "$comm" in
      *.app/*) name=${comm%%.app/*}; echo "${name##*/}"; return ;;
    esac
  done
  echo "the app that runs this script (Terminal, iTerm, Orca, or codex's host)"
}

# Is the app running right now? (Asking does not launch it.)
app_running() { [ "$(osascript -e "application \"$1\" is running" 2>/dev/null)" = true ]; }

# (A pipe into grep -q would die of SIGPIPE under pipefail, so capture first.)
screen_locked() { local o; o=$(ioreg -n Root -d1 2>/dev/null); [[ $o == *'"IOConsoleLocked" = Yes'* ]]; }

# probe_automation APP: can the runner send Apple events to APP? Adds to `blocked` when not.
probe_automation() {
  local appname=$1 err rc runner
  runner=$(runner_app)
  err=$(run_to 25 osascript -e "tell application \"$appname\" to count windows" 2>&1 >/dev/null); rc=$?
  [ $rc = 0 ] && return 0
  need "Automation: in System Settings > Privacy & Security > Automation, turn on \"$appname\" under \"$runner\" (or click OK when macOS asks). osascript said: ${err:-timed out waiting for the prompt}"
  return 1
}

# The runner needs Accessibility for the Chrome scenario (bringing the throwaway Chrome to the front)
# and for the stage 4 live tests, and Automation of System Events to prove it.
probe_runner_accessibility() {
  local err rc runner
  runner=$(runner_app)
  err=$(run_to 25 osascript -e 'tell application "System Events" to get name of first process whose frontmost is true' 2>&1 >/dev/null); rc=$?
  [ $rc = 0 ] && return 0
  if grep -qi 'assistive' <<<"$err"; then
    need "Accessibility: in System Settings > Privacy & Security > Accessibility, turn on \"$runner\" (the Chrome scenario brings its throwaway Chrome to the front with it, and the stage 4 live tests read the focused field through it)."
  else
    need "Automation: in System Settings > Privacy & Security > Automation, turn on \"System Events\" under \"$runner\", and in Accessibility turn on \"$runner\". osascript said: ${err:-timed out waiting for the prompt}"
  fi
}

# The Chromium scenario starts its own Chrome process with a throwaway profile (see chrome_open) and
# reads it back over its debugging port. It changes nothing in the user's Chrome and sends Chrome no
# Apple event. It only needs Chrome installed.
probe_chrome() {
  local handle
  [ -x "$CHROME_BIN" ] || { need "Install Google Chrome in /Applications (the Chromium scenario starts a separate Chrome process with a throwaway profile; your own Chrome is not touched)."; return; }
  handle=$(chrome_open) || true
  if [ -z "$handle" ]; then
    need "A separate Chrome process with a throwaway profile did not start (see the chrome.*.log files in $tmp); check Google Chrome launches."
    return
  fi
  chrome_close "$handle"
}

print_blocked() {
  local i=1 line
  {
    echo
    echo "BLOCKED: ${#blocked[@]} thing(s) need a human on this Mac before the run can continue."
    echo "The script never grants permissions itself and never touches an app it did not start. Do each step once, then run again with --skip-build:"
    for line in "${blocked[@]}"; do echo "  $i. $line"; i=$((i + 1)); done
    echo "Any Wristper Test this run started was stopped on the way out. To click its Microphone Allow, start it first: open -g \"$app\""
  } | tee -a "$out/run.log"
  exit 3
}

# =============================================================================
# Build
# =============================================================================
build_mac_app() {
  log "building Wristper Test.app (HEX_TEST_BUILD=1 HEX_LOCAL_BUILD=1)"
  (cd "$root" && HEX_TEST_BUILD=1 HEX_LOCAL_BUILD=1 owned_fg build-app scripts/build-app.sh) > "$out/build-app.log" 2>&1 \
    || die "build-app.sh failed; see $out/build-app.log"
  # build-app.sh signs a test build ad hoc, whose identity changes with every
  # build, so macOS forgets its Accessibility and Microphone grants each
  # time. Re-signing with the Developer ID keeps one identity across builds.
  if [[ $(security find-identity -v -p codesigning) == *"$SIGN_IDENTITY"* ]]; then
    log "re-signing with $SIGN_IDENTITY"
    {
      codesign --force --options runtime --sign "$SIGN_IDENTITY" "$app/Contents/Library/HexMicrophone.driver" &&
      codesign --force --options runtime --sign "$SIGN_IDENTITY" --identifier "$TEST_BUNDLE.scrawl-helper" "$app/Contents/MacOS/scrawl-helper" &&
      codesign --force --options runtime --sign "$SIGN_IDENTITY" --entitlements "$root/.github/release/hex.entitlements" "$app" &&
      codesign --verify --deep --strict "$app"
    } >> "$out/build-app.log" 2>&1 || die "re-signing failed; see $out/build-app.log"
  else
    log "WARNING: $SIGN_IDENTITY not found; the app stays ad hoc signed and needs its permissions granted again after every build"
  fi
}

# export_pinned COMMIT NAME [paths...]: the commit's files (or just those paths) under
# target/watch-e2e/pinned/NAME-COMMIT, made once and never changed. Prints the folder.
export_pinned() {
  local commit=$1 name=$2 dest
  shift 2
  [[ $commit =~ ^[0-9a-f]{40}$ ]] || die "$name commit must be a full 40-character sha, got '$commit'" 2
  git -C "$WRISTPER_SRC" cat-file -e "$commit^{commit}" 2>/dev/null \
    || die "commit $commit is not in $WRISTPER_SRC's git history; fetch it or set WRISTPER_SRC" 2
  dest="$root/target/watch-e2e/pinned/$name-$commit"
  if [ ! -f "$dest/.complete" ]; then
    mkdir -p "$dest"
    git -C "$WRISTPER_SRC" archive "$commit" "$@" | tar -x -C "$dest" || die "could not export $name at $commit" 2
    : > "$dest/.complete"
  fi
  echo "$dest"
}

build_watch_sim() {
  local src
  src=$(export_pinned "$WRISTPER_SRC_COMMIT" watch-src WatchProbe WatchShared) || exit $?
  log "building watch-sim from the watch sources at $WRISTPER_SRC_COMMIT"
  WRISTPER_SRC="$src" owned_fg build.sh "$root/scripts/watch-sim/build.sh" "$sim" > "$out/build-watch-sim.log" 2>&1 \
    || die "watch-sim build failed; see $out/build-watch-sim.log"
  echo "$WRISTPER_SRC_COMMIT" > "$(dirname "$sim")/built-from-commit"
}

# The clipboard backup tool (scripts/pasteboard-tool): saves and restores every type on the pasteboard.
build_pbtool() {
  if [ ! -x "$pbtool" ] || [ "$root/scripts/pasteboard-tool/main.swift" -nt "$pbtool" ]; then
    mkdir -p "$(dirname "$pbtool")"
    xcrun swiftc -O -o "$pbtool" "$root/scripts/pasteboard-tool/main.swift" > "$out/build-pasteboard-tool.log" 2>&1 \
      || die "pasteboard-tool build failed; see $out/build-pasteboard-tool.log"
  fi
}

# =============================================================================
# Wristper Test
# =============================================================================
fresh_room() {
  local f old_room= failed=0
  case "$TEST_DATA" in */"Wristper Test") ;; *) die "refusing to reset a data folder that is not Wristper Test: $TEST_DATA" 2 ;; esac
  [ "$TEST_DATA" != "$OLD_DATA" ] || die "refusing to reset the installed app's data folder" 2
  [ ! -s "$TEST_DATA/relay-room" ] || old_room=$(cat "$TEST_DATA/relay-room")
  reg_add path "$tmp/old-room"
  printf %s "$old_room" > "$tmp/old-room"
  for f in relay-room relay-secret paired-devices.json delivered-ids.json; do
    reg_add testdata "$TEST_DATA/$f" allowlist
    owned testdata "$TEST_DATA/$f" act_trash_testdata "$TEST_DATA/$f" || failed=1
  done
  return $failed
}

launch_wristper_test() {
  local pid started
  log "launching Wristper Test in the background"
  # Written BEFORE the launch: if a signal or a failure comes between `open` and the registration, the
  # teardown still finds the app by this launch time (see adopt_launched_wt).
  reg_add wt-launch launch "$(date +%s)"
  open -g "$app" || die "could not open $app"
  for _ in $(seq 1 60); do status_json >/dev/null && break; sleep 1; done
  status_json >/dev/null || die "Wristper Test did not answer hex status within 60 s"
  for _ in $(seq 1 30); do [ -s "$TEST_DATA/relay-room" ] && break; sleep 1; done
  [ -s "$TEST_DATA/relay-room" ] || die "no relay room in $TEST_DATA; the watch service did not start (see $TEST_DATA/logs/process.log)"
  # Its pid from its own status, and only if that pid runs exactly the app's executable.
  pid=$(status_json | jq -r .pid)
  [ "$(proc_cmd "$pid")" = "$hex" ] \
    || die "Wristper Test started, but its process could not be identified; this script will not stop it unless it can prove it started it. Quit it with: \"$hex\" quit"
  started=$(proc_start "$pid")
  reg_add wt "$pid" "$hex$US$started"
  reg_del wt-launch launch
}

# An app that was already running is never stopped, reset, or driven: refuse, and say why.
start_app() {
  if wt_running; then
    need "Wristper Test is already running (pid $(wt_pids | tr '\n' ' ')). This script only stops and resets a Wristper Test that it started itself, so it refuses. Quit it yourself (\"$hex\" quit), then run again. Nothing was stopped or reset."
    print_blocked
  fi
  reg_add ran "Wristper Test" not-running
  launch_wristper_test
  status_json > "$out/status-start.json"
}

# After the preflight passed: stop it (it is ours), give it a new relay room, start it again, check the room changed.
restart_with_new_room() {
  stop_wristper_test
  for _ in $(seq 1 30); do status_json >/dev/null || break; sleep 1; done
  wt_running && die "Wristper Test did not stop (or another instance appeared); not resetting its data"
  fresh_room || die "could not reset the relay room (see the REFUSED lines in $out/run.log); nothing was started on top of the old one"
  # Where Hex's log and the macOS log stood before this launch (mic-stays-closed reads what came after).
  proc_log_offset=$(stat -f%z "$TEST_DATA/logs/process.log" 2>/dev/null || echo 0)
  launch_time=$(date '+%Y-%m-%d %H:%M:%S')
  launch_wristper_test
  [ "$(cat "$TEST_DATA/relay-room")" != "$(cat "$tmp/old-room")" ] || die "the relay room is the same as the previous run's; it was not renewed"
  status_json > "$out/status-start.json"
  log "relay room: new for this run (differs from the previous one)"
}

# The watch sends language "auto"; the Mac transcribes with the selected
# model and language, so each language runs on a model that speaks it.
model_for() { [ "$1" = zh ] && echo "${WATCH_E2E_ZH_MODEL:-nemotron_3_5_asr}" || echo "${WATCH_E2E_EN_MODEL:-parakeet_unified_en}"; }

select_model() { # lang
  local want s
  want=$(model_for "$1")
  s=$(status_json)
  if [ "$(jq -r .model.id <<<"$s")" != "$want" ] || [ "$(jq -r .model.language <<<"$s")" != "$1" ]; then
    log "selecting $want for $1: hex model install $want --language $1"
    owned_fg hex "$hex" model install "$want" --language "$1" 2>&1 | tee -a "$out/run.log"
  fi
  for _ in $(seq 1 720); do
    s=$(status_json)
    [ "$(jq -r '.model.id + "/" + .model.language + "/" + (.model.preparing // "") + "/" + (.permissions.transcription_model|tostring)' <<<"$s")" = "$want/$1//true" ] && break
    [ -n "$(jq -r '.model.error // empty' <<<"$s")" ] && die "model $want failed: $(jq -r .model.error <<<"$s")"
    sleep 5
  done
  [ "$(jq -r '.model.id + "/" + (.model.preparing // "")' <<<"$s")" = "$want/" ] || die "model $want was not ready within 60 min: $(jq -c .model <<<"$s")"
  log "model: $(jq -r '.model.name + " (" + .model.id + ", " + .model.language + ")"' <<<"$s")"
}

# Every missing permission, listed together. Until unit C adds the setup page
# to the Wristper build, the listener also waits for Microphone.
check_permissions() {
  local s mic ax t
  s=$(status_json)
  echo "$s" > "$out/status-ready.json"
  mic=$(jq -r .permissions.microphone <<<"$s")
  ax=$(jq -r .permissions.accessibility <<<"$s")
  # With --c1 (unit C1 merged) the watch must work without the Microphone grant, so it is not required.
  if [ "$mic" != ready ] && [ "$c1" = 0 ]; then
    need "Microphone for Wristper Test.app ($mic): run open -g \"$app\", then \"$hex\" window show, and click Allow for Microphone in the setup page."
  fi
  if [ "$ax" != ready ]; then
    need "Accessibility for Wristper Test.app ($ax): System Settings > Privacy & Security > Accessibility, turn on \"Wristper Test\"; if it is not listed click + and add $app. Then run \"$hex\" restart if the status stays needs-restart."
  fi
  local chrome_wanted=0
  if stage_on 3 && { [ -z "$only" ] || [[ chrome =~ $only ]]; }; then chrome_wanted=1; fi
  [ "$chrome_wanted" = 1 ] && probe_chrome
  if ! screen_locked; then
    uses_app TextEdit && probe_automation TextEdit
    uses_app Terminal && probe_automation Terminal
    if stage_on 4 || [ "$chrome_wanted" = 1 ]; then probe_runner_accessibility; fi
  fi
  [ ${#blocked[@]} -eq 0 ] || print_blocked
  log "preflight: permissions ready"
}

# Does this run drive that app? (Probing launches it, so only the apps in use are probed.)
uses_app() {
  case "$1" in
    TextEdit) stage_on 3 || stage_on 4 || { { stage_on 1 || stage_on 2; } && [[ ",$targets," == *,textedit,* ]]; } ;;
    Terminal) stage_on 3 || stage_on 4 || { { stage_on 1 || stage_on 2; } && [[ ",$targets," == *,terminal,* ]]; } ;;
  esac
}

wait_listener() {
  local s
  for _ in $(seq 1 30); do
    [ "$(status_json | jq -r .listener.running)" = true ] && break
    sleep 1
  done
  s=$(status_json)
  [ "$(jq -r .listener.running <<<"$s")" = true ] || die "listener not running: $(jq -c .listener <<<"$s")"
  log "listener running"
}

# =============================================================================
# Fixtures and judging
# =============================================================================
voice_for() { say -v '?' | awk -v loc="$1" '$0 ~ loc {print $1; exit}'; }

make_fixtures() {
  local en zh l
  en=$(say -v '?' | awk '/^Samantha /{print "Samantha"; exit}'); [ -n "$en" ] || en=$(voice_for en_US)
  zh=$(say -v '?' | awk '/^Meijia /{print "Meijia"; exit}'); [ -n "$zh" ] || zh=$(voice_for zh_TW)
  [ -n "$zh" ] || die "no zh_TW voice for say; install one in System Settings > Accessibility > Spoken Content" 2
  say -v "$en" -o "$out/en.aiff" "$EN_TEXT" && say -v "$zh" -o "$out/zh.aiff" "$ZH_TEXT" \
    && say -v "$en" -o "$out/agent.aiff" "$AGENT_TEXT" || die "say failed"
  # The watch's recorder: 16 kHz mono AAC at 32 kbps in .m4a (WatchProbe/ProbeModel.swift recorderSettings).
  for l in en zh agent; do
    afconvert -f m4af -d aac@16000 -c 1 -b 32000 "$out/$l.aiff" "$out/$l.m4a" || die "afconvert failed for $l"
  done
  log "fixtures: en ($en) $(stat -f%z "$out/en.m4a") B, zh ($zh) $(stat -f%z "$out/zh.m4a") B, agent $(stat -f%z "$out/agent.m4a") B"
}

text_for() { case "$1" in zh) echo "$ZH_TEXT" ;; agent) echo "$AGENT_TEXT" ;; *) echo "$EN_TEXT" ;; esac; }
pass_for() { [ "$1" = zh ] && echo "$ZH_PASS" || echo "$EN_PASS"; }

# Pairing codes are single use and expire in 2 minutes, but they are still kept out of the saved logs.
redact_pairing() { sed -E 's/"code":"[0-9]{6}"/"code":"******"/; s/pair-code [0-9]{6}/pair-code ******/'; }

judge() { # lang-or-kind readback -> percent (ordered recall)
  "$sim" match --expected "$(text_for "$1")" --actual "$2" | jq -r .percent
}
# Precision: the share of the read-back that is the sentence. Only for fields that start empty
# (a TextEdit document, the Chrome textarea): a sentence pasted twice scores 100 on recall but 50 here.
precision_of() { # lang-or-kind readback -> percent
  "$sim" match --expected "$(text_for "$1")" --actual "$2" | jq -r .precision
}
starts_empty() { [ "$1" = textedit ] || [ "$1" = chrome ]; }

want() { [ -z "$only" ] || [[ $1 =~ $only ]]; }

judged() { # lang -> percent, with the pass threshold check (recall, plus precision for empty-start fields)
  local precision
  SC_PERCENT=$(judge "$1" "$SC_READBACK"); SC_PERCENT=${SC_PERCENT:-0}
  SC_EXACT=false; same_text "$SC_READBACK" "$(text_for "$1")" && SC_EXACT=true   # information: the read-back is exactly the sentence
  chk "$(is [ "$SC_PERCENT" -ge "$(pass_for "$1")" ])" "read-back matches only ${SC_PERCENT}%"
  if starts_empty "$SC_TARGET"; then
    precision=$(precision_of "$1" "$SC_READBACK"); precision=${precision:-0}
    chk "$(is [ "$precision" -ge "$(pass_for "$1")" ])" "only ${precision}% of the read-back is the sentence (pasted twice, or extra text?)"
  fi
}

# =============================================================================
# watch-sim cases (stages 1 and 3)
# =============================================================================
proc_log_offset=0
launch_time=
sim_paired=0
pair_sim() { # stage
  # The watch's pairing state (device id, token, private key) and the pairing code live only in $tmp.
  mkdir -m 700 -p "$tmp/watch-sim-state"
  reg_add tree "$tmp/watch-sim-state" '*'
  export WATCH_SIM_STATE="$tmp/watch-sim-state"
  local raw="$tmp/stage$1-pair.jsonl"
  reg_add path "$raw"
  log "stage $1: pairing watch-sim through the production relay"
  owned_bg watch-sim "$sim" pair --timeout 150 > "$raw" 2>&1
  local pid=$OWNED_PID code=
  for _ in $(seq 1 30); do
    code=$(jq -r 'select(.event=="code") | .code' "$raw" 2>/dev/null | head -1)
    [ -n "$code" ] && break
    sleep 0.5
  done
  [ -n "$code" ] || { redact_pairing < "$raw" | tee "$out/stage$1-pair.jsonl" >&2; die "watch-sim got no pairing code"; }
  log "watch-sim got a pairing code; hex watch pair ******"
  owned_fg hex "$hex" watch pair "$code" 2>&1 | tee -a "$out/run.log"
  wait "$pid"; reg_del pid "$pid"
  redact_pairing < "$raw" > "$out/stage$1-pair.jsonl"
  jq -e 'select(.event=="paired") | .ok' "$raw" >/dev/null \
    || { cat "$out/stage$1-pair.jsonl" >&2; record "$1" pairing - FAIL 0 unpaired 0 "" "null"; return 1; }
  record "$1" pairing - PASS 100 paired "$(jq -r 'select(.event=="paired") | .ms' "$raw")" "" \
    "$(jq -c 'select(.event=="paired")' "$out/stage$1-pair.jsonl")"
  sim_paired=1
}

# act_sim_deliver KIND HANDLE AUDIO args...: the only place watch-sim deliver runs, behind the gate.
# watch-sim sends /focus, runs the front check, and only if it passes sends the audio; the same check
# runs before each /enter and /undo.
act_sim_deliver() {
  local kind=$1 h=$2 audio=$3 after before
  shift 3
  win_live "$kind" "$h" || { refuse window "$(win_key "$kind" "$h")" "the live window is not the one this run made; nothing sent"; return 98; }
  before=$(front_hook "$kind" "$h")
  after=$before
  # The app-switch scenario: verify the target is in front, then move away on purpose, so the audio meets
  # the first gate with another app in front and must be withheld.
  [ "$SC_SWITCH" = 1 ] && after="$before && $(printf '%q ' /bin/bash "$self" --switch-away)"
  owned_fg watch-sim "$sim" deliver --audio "$out/$audio.m4a" --timeout 90 --after-focus "$after" --before-action "$before" "$@"
}
sim_deliver() { # kind handle audio args...
  owned window "$(win_key "$1" "$2")" act_sim_deliver "$@"
}

# sim_run TARGET AUDIO [watch-sim deliver args...]
# Opens the target (registered), focuses it (must succeed), checks it is in front, delivers AUDIO from
# watch-sim (gated again between /focus and /e2ee), reads the target back. Sets SC_REPORT (JSON),
# SC_READBACK, SC_MS, SC_HANDLE. A failed step sets ABORT_NOTE: nothing is sent and the row is INCOMPLETE.
sim_run() {
  local target=$1 audio=$2 started kind
  shift 2
  SC_TARGET=$target; SC_REPORT=null; SC_READBACK=; SC_MS=0; SC_HANDLE=
  row_begin
  kind=$(kind_of "$target")
  SC_HANDLE=$(open_target "$target") || SC_HANDLE=
  if [ -z "$SC_HANDLE" ]; then
    ABORT_NOTE="could not open $target"; SC_REPORT='{"event":"aborted","e2ee_sent":false}'; : > "$SC_FILE"; return 1
  fi
  sleep 1
  if ! focus_target "$target" "$SC_HANDLE"; then
    ABORT_NOTE="could not focus the $target window this run made; nothing was sent"
    SC_REPORT='{"event":"aborted","e2ee_sent":false}'; : > "$SC_FILE"; return 1
  fi
  if ! front_check "$kind" "$SC_HANDLE"; then
    ABORT_NOTE="the app in front is not the $target window this run made; nothing was sent"
    SC_REPORT='{"event":"aborted","e2ee_sent":false}'; : > "$SC_FILE"; return 1
  fi
  started=$(now_ms)
  SC_REPORT=$(sim_deliver "$kind" "$SC_HANDLE" "$audio" "$@" | tail -1)
  SC_MS=$(( $(now_ms) - started ))
  jq -e . >/dev/null 2>&1 <<<"$SC_REPORT" || SC_REPORT=$(jq -Rn --arg r "$SC_REPORT" '{raw:$r}')
  if [ "$(rj '.aborted // empty')" != "" ]; then
    ABORT_NOTE="watch-sim stopped before sending: $(rj '.aborted')"
  fi
  sleep 1
  read_target "$target" "$SC_HANDLE" > "$SC_FILE" 2>/dev/null
  SC_READBACK=$(cat "$SC_FILE")
}

# After sim_run: close what it opened (through the gate; a refusal marks the row INCOMPLETE).
sim_close() { [ -n "$SC_HANDLE" ] && close_target "$1" "$SC_HANDLE"; return 0; }

stage1() {
  pair_sim 1 || return
  local target lang
  for lang in ${langs//,/ }; do
    select_model "$lang"
    for target in ${targets//,/ }; do
      sim_run "$target" "$lang"
      sim_close "$target"
      judged "$lang"
      chk "$(is [ "$(rj '.terminal')" = true ])" "no terminal state ($(rj '.state'))"
      finish 1 "$target" "$lang" "$SC_PERCENT"
    done
  done
}

# =============================================================================
# Stage 3: scenarios no test covers
# =============================================================================
set_keep() { "$hex" settings set labs.keep_on_clipboard "$1" >/dev/null 2>&1; sleep 1; }

scenario_clipboard() {
  local sentinel="WATCH-E2E-SENTINEL-$RANDOM$RANDOM" board sc
  if [ "$CLIP_STATE" != saved ]; then
    # Without a verified backup the clipboard scenarios do not touch the clipboard at all.
    for sc in clipboard-keep-gate-switch clipboard-keep-paste clipboard-default-restore; do
      want "$sc" || continue
      row_begin; SC_REPORT='{"event":"refused"}'; SC_MS=0; : > "$SC_FILE"
      ABORT_NOTE="no clipboard backup exists, so this scenario will not overwrite your clipboard"
      finish 3 "$sc" en 0
    done
    return 0
  fi
  keep_setting_original=$("$hex" settings get labs.keep_on_clipboard 2>/dev/null)
  [ "$keep_setting_original" = true ] && keep_setting_original=on || keep_setting_original=off

  if want clipboard-keep-gate-switch; then
    # keep_on_clipboard on, the person switches app after the recording started
    # and before its audio arrives: the first gate withholds the dictation, so
    # the clipboard is never written. This does NOT reach the final focus check
    # (after the clipboard write, right before the paste key): that window is
    # tens of ms and /status reports both refusals the same way (target-changed).
    set_keep on; clip_set "$sentinel"
    SC_SWITCH=1
    sim_run textedit en
    board=$("$pbtool" get)
    sim_close textedit
    chk "$(is [ "$(rj '.state')" = target-changed ])" "state is $(rj '.state'), want target-changed"
    chk "$(is [ "$board" = "$sentinel" ])" "clipboard is '${board:0:60}', want the sentinel back"
    chk "$(is [ "$(count Friday)" = 0 ])" "text reached the document anyway"
    SC_PERCENT=0; finish 3 clipboard-keep-gate-switch en "$SC_PERCENT"
  fi

  if want clipboard-keep-paste; then
    # keep_on_clipboard on, no switch: the dictation stays on the clipboard.
    set_keep on; clip_set "$sentinel"
    sim_run textedit en
    sleep 5; board=$("$pbtool" get)
    sim_close textedit
    judged en
    chk "$(is [ "$(judge en "$board")" -ge "$EN_PASS" ])" "clipboard is '${board:0:60}', want the dictation"
    finish 3 clipboard-keep-paste en "$SC_PERCENT"
  fi

  if want clipboard-default-restore; then
    # keep_on_clipboard off (the default): the clipboard comes back after the paste.
    set_keep off; clip_set "$sentinel"
    sim_run textedit en
    sleep 6; board=$("$pbtool" get)
    sim_close textedit
    judged en
    chk "$(is [ "$board" = "$sentinel" ])" "clipboard is '${board:0:60}', want the sentinel restored"
    finish 3 clipboard-default-restore en "$SC_PERCENT"
  fi
  set_keep "$keep_setting_original"
}

scenario_enter_undo() {
  if want textedit-enter; then
    sim_run textedit en --enter; sim_close textedit
    judged en
    chk "$(is [ "$(rj '.enter.body.ok')" = true ])" "enter reply: $(rj '.enter|tostring')"
    chk "$(is [ "$(( $(breaks) - 1 ))" = 1 ])" "Return typed $(( $(breaks) - 1 )) times, want 1"
    finish 3 textedit-enter en "$SC_PERCENT"
  fi
  if want textedit-undo; then
    sim_run textedit en --undo; sim_close textedit
    SC_PERCENT=$(judge en "$SC_READBACK"); SC_PERCENT=${SC_PERCENT:-0}
    chk "$(is [ "$(rj '.undo.body.ok')" = true ])" "undo reply: $(rj '.undo|tostring')"
    chk "$(is [ "$SC_PERCENT" -lt 20 ])" "text is still in the document after undo (${SC_PERCENT}%)"
    finish 3 textedit-undo en "$SC_PERCENT"
  fi
  if want cat-enter; then
    sim_run terminal en --enter; sim_close terminal
    judged en
    chk "$(is [ "$(rj '.enter.body.ok')" = true ])" "enter reply: $(rj '.enter|tostring')"
    chk "$(is [ "$(count quarterly)" -ge 2 ])" "cat did not echo the line (Return not typed)"
    finish 3 cat-enter en "$SC_PERCENT"
  fi
  if want cat-undo; then
    # Undo is never offered in a terminal: the reply must refuse and the line must stay.
    sim_run terminal en --undo; sim_close terminal
    judged en
    chk "$(is [ "$(rj '.undo.body.ok')" = false ])" "undo in a terminal was not refused: $(rj '.undo|tostring')"
    finish 3 cat-undo en "$SC_PERCENT"
  fi
  if want recorder-enter; then
    sim_run recorder en --enter; sim_close recorder
    judged en
    chk "$(is [ "$(rj '.enter.body.ok')" = true ])" "enter reply: $(rj '.enter|tostring')"
    chk "$(is [ "$(enter_lines)" = 1 ])" "$(enter_lines) Return(s) reached the shell with text, want exactly 1"
    chk "$(is [ "$(empty_enter_lines)" = 0 ])" "$(empty_enter_lines) empty Return(s) reached the shell"
    finish 3 recorder-enter en "$SC_PERCENT"
  fi
  if want shell-enter; then
    sim_run shell en --enter; sim_close shell
    judged en
    chk "$(is [ "$(rj '.enter.body.ok')" = true ])" "enter reply: $(rj '.enter|tostring')"
    chk "$(is [ "$(count 'command not found')" = 1 ])" "the shell ran the line $(count 'command not found') times, want 1"
    finish 3 shell-enter en "$SC_PERCENT"
  fi
}

scenario_double_enter() {
  if want enter-twice-textedit; then
    sim_run textedit en --enter-twice; sim_close textedit
    judged en
    # The two replies are kept in the detail; the proof is how often Return was typed.
    chk "$(is [ "$(( $(breaks) - 1 ))" = 1 ])" "Return typed $(( $(breaks) - 1 )) times, want 1"
    finish 3 enter-twice-textedit en "$SC_PERCENT"
  fi
  if want enter-twice-shell; then
    # The recorder window prints one ENTER:<line> per Return it receives, so a second Return
    # (which sends an empty line) shows up as a bare "ENTER:".
    sim_run recorder en --enter-twice; sim_close recorder
    judged en
    chk "$(is [ "$(enter_lines)" = 1 ])" "$(enter_lines) Return(s) reached the shell with text, want exactly 1"
    chk "$(is [ "$(empty_enter_lines)" = 0 ])" "$(empty_enter_lines) extra empty Return(s): the second Enter was typed too"
    finish 3 enter-twice-shell en "$SC_PERCENT"
  fi
}

scenario_chrome() {
  local lang
  for lang in ${langs//,/ }; do
    want "chrome-$lang" || want chrome || continue
    select_model "$lang"
    sim_run chrome "$lang"; sim_close chrome
    judged "$lang"
    chk "$(is [ "$(rj '.terminal')" = true ])" "no terminal state ($(rj '.state'))"
    finish 3 chrome-textarea "$lang" "$SC_PERCENT"
  done
}

# The first claude or codex on PATH that is not under ~/.wristper (the installed
# Wristper's own wrappers, which must not run in this test).
agent_bin() {
  local p
  while IFS= read -r p; do
    case "$p" in "$HOME/.wristper/"*) continue ;; esac
    echo "$p"; return 0
  done < <(type -aP "$1" 2>/dev/null)
  return 1
}

# Off by default: needs the user's claude and codex logins and spends tokens.
scenario_agents() {
  local agent n
  for agent in claude codex; do
    want "agent-$agent" || continue
    AGENT_BIN=$(agent_bin "$agent")
    if [ -z "$AGENT_BIN" ]; then
      row_begin; SC_REPORT='{"event":"missing"}'; SC_MS=0; : > "$SC_FILE"; FAILS="no $agent outside ~/.wristper on PATH"
      finish 3 "agent-$agent" en 0; continue
    fi
    sim_run "$agent" agent --enter
    judged agent
    chk "$(is [ "$(rj '.enter.body.ok')" = true ])" "enter reply: $(rj '.enter|tostring')"
    # The agent answers "pong" only if Return really submitted the prompt.
    for n in $(seq 1 45); do
      read_target "$agent" "$SC_HANDLE" > "$SC_FILE" 2>/dev/null
      [ "$(grep -io 'pong' "$SC_FILE" | wc -l | tr -d ' ')" -ge 2 ] && break
      sleep 2
    done
    chk "$(is [ "$(grep -io 'pong' "$SC_FILE" | wc -l | tr -d ' ')" -ge 2 ])" "no reply from $agent within 90 s: Return did not submit, or $agent needs a login or a folder trust in $AGENT_DIR"
    sim_close "$agent"
    finish 3 "agent-$agent" en "$SC_PERCENT"
  done
}

# Depends on unit C1 (the watch works without the Microphone grant and without opening the input
# stream). What this can see today:
#  - `hex status` permissions.microphone must stay needs-request after a successful watch dictation;
#  - Hex's process.log, from this launch on, must have no microphone-stream line. The exact lines
#    that exist today (src/audio.rs) are only failure and recovery ones: "dictation microphone
#    reopened", "could not open dictation microphone at startup", "microphone stream stopped;
#    reopening it", "microphone stream failed", "the default microphone changed". No line is
#    logged for a successful first open, so absence here is weak evidence until C1 adds one
#    (this pattern already expects "microphone stream opened");
#  - the macOS TCC log, from this launch on, must hold no kTCCServiceMicrophone request naming
#    this app (informational: that pattern has never been seen to match on this Mac).
MIC_LOG_RE='microphone stream opened|dictation microphone (opened|reopened)|could not open dictation microphone|microphone stream (stopped|failed)|the default microphone changed'
scenario_mic() {
  want mic-stays-closed || return 0
  local mic now_mic lines tcc
  mic=$(status_json | jq -r .permissions.microphone)
  if [ "$mic" != needs-request ]; then
    row_begin
    SC_REPORT=$(jq -cn --arg m "$mic" '{event:"blocked",microphone:$m}'); SC_MS=0; : > "$SC_FILE"; SC_PERCENT=0
    BLOCKED_NOTE="depends on C1: the Microphone grant is '$mic' here and the build still requires it, so a watch dictation cannot be shown to leave the microphone closed. Run --c1 once C1 is merged on a Mac where Wristper Test was never given Microphone."
    finish 3 mic-stays-closed en 0
    return 0
  fi
  sim_run textedit en; sim_close textedit
  judged en
  chk "$(is [ "$(rj '.terminal')" = true ])" "the dictation did not finish ($(rj '.state')): without Microphone the watch does not work yet (C1)"
  now_mic=$(status_json | jq -r .permissions.microphone)
  chk "$(is [ "$now_mic" = needs-request ])" "hex status microphone is '$now_mic' after the dictation, want needs-request"
  lines=$(tail -c +"$((proc_log_offset + 1))" "$TEST_DATA/logs/process.log" 2>/dev/null | grep -E "$MIC_LOG_RE"; true)
  chk "$(is [ -z "$lines" ])" "Hex's log shows microphone stream activity since launch: $(head -c 200 <<<"$lines" | tr '\n' '|')"
  tcc=$(run_to 60 log show --start "$launch_time" --style compact --predicate \
    'subsystem == "com.apple.TCC" AND eventMessage CONTAINS "kTCCServiceMicrophone" AND eventMessage CONTAINS "dev.wristper.mac.test"' 2>/dev/null | grep -c kTCCServiceMicrophone; true)
  chk "$(is [ "${tcc:-0}" = 0 ])" "the macOS TCC log shows $tcc microphone request(s) from this app since launch"
  SC_REPORT=$(jq -c --arg l "${lines:-}" --arg t "${tcc:-0}" '. + {mic_log_lines:$l, tcc_requests:$t}' <<<"$SC_REPORT")
  finish 3 mic-stays-closed en "$SC_PERCENT"
}

stage3() {
  [ "$sim_paired" = 1 ] || pair_sim 3 || return
  select_model en
  scenario_mic
  scenario_clipboard
  scenario_enter_undo
  scenario_double_enter
  scenario_chrome
  [ "$with_agents" = 1 ] && { select_model en; scenario_agents; }
}

# =============================================================================
# Stage 2: the real watch app in a watchOS Simulator
# =============================================================================
# Always a new device: made here, registered, deleted by teardown.
sim_device() {
  local udid runtime type
  runtime=$(xcrun simctl list runtimes -j | jq -r '[.runtimes[] | select(.platform=="watchOS" and .isAvailable)] | last | .identifier')
  [ -n "$runtime" ] && [ "$runtime" != null ] || die "no watchOS Simulator runtime installed (Xcode > Settings > Components)" 2
  type=$SIM_TYPE
  xcrun simctl list devicetypes -j | jq -e --arg t "$type" '.devicetypes[] | select(.identifier==$t)' >/dev/null 2>&1 ||
    type=$(xcrun simctl list devicetypes -j | jq -r '[.devicetypes[] | select(.productFamily=="Apple Watch")] | last | .identifier')
  # Written BEFORE the create: if a signal comes between the create and the registration, the teardown
  # still finds the device by this run's unique name (adopt_created_sim).
  reg_add sim-creating "$sim_name"
  udid=$(xcrun simctl create "$sim_name" "$type" "$runtime") || die "simctl create failed"
  echo "$udid"
}

wait_log() { # pattern timeout-seconds start-line -> matching line
  local i m
  for i in $(seq 1 $(( $2 * 2 ))); do
    m=$(tail -n +"$3" "$tmp/stage2-watch.log" | grep -E "$1"; true)
    if [ -n "$m" ]; then echo "${m%%$'\n'*}"; return 0; fi
    sleep 0.5
  done
  return 1
}

# act_open_gate KIND HANDLE GATEFILE: lets the watch app upload, only if the target is still the window in front.
act_open_gate() {
  win_live "$1" "$2" || { refuse window "$(win_key "$1" "$2")" "the live window is not the one this run made; the watch app was not allowed to upload"; return 1; }
  front_check "$1" "$2" || { refuse window "$(win_key "$1" "$2")" "the window this run made is not in front; the watch app was not allowed to upload"; return 1; }
  : > "$3"
}

stage2() {
  local hooks_src
  hooks_src=$(export_pinned "$WRISTPER_HOOKS_COMMIT" watch-app) || exit $?
  [ -d "$hooks_src/WristperProbe.xcodeproj" ] || die "commit $WRISTPER_HOOKS_COMMIT has no WristperProbe.xcodeproj" 2
  [[ $(cat "$hooks_src/WatchProbe/E2EHooks.swift" 2>/dev/null) == *e2eGateFile* ]] \
    || die "commit $WRISTPER_HOOKS_COMMIT lacks the gate in WatchProbe/E2EHooks.swift (branch test/sim-fixture-audio)" 2
  sim_udid=$(sim_device)
  reg_add sim "$sim_udid" "$sim_name"
  log "stage 2: watchOS Simulator $sim_name ($sim_udid), made for this run"
  simctl_own boot "$sim_udid" >/dev/null 2>&1
  simctl_own bootstatus "$sim_udid" -b >/dev/null 2>&1 || die "simulator did not boot"
  local dd="$root/target/watch-e2e/watch-dd-$WRISTPER_HOOKS_COMMIT" watch_app
  if [ "$build" = 1 ] || [ ! -d "$dd/Build/Products/Debug-watchsimulator/WristperProbe.app" ]; then
    # Ad hoc signing is required: an unsigned simulator build has no application-identifier,
    # so its Keychain refuses the pairing key ("Could not securely save the encryption key").
    log "building the watch app (Debug, E2E hooks) at $WRISTPER_HOOKS_COMMIT"
    owned_fg xcodebuild xcodebuild -project "$hooks_src/WristperProbe.xcodeproj" -scheme WristperProbe -configuration Debug \
      -destination "id=$sim_udid" -derivedDataPath "$dd" build \
      CODE_SIGN_IDENTITY=- CODE_SIGNING_ALLOWED=YES CODE_SIGNING_REQUIRED=NO DEVELOPMENT_TEAM= > "$out/build-watch.log" 2>&1 \
      || die "watch app build failed; see $out/build-watch.log"
  fi
  watch_app="$dd/Build/Products/Debug-watchsimulator/WristperProbe.app"
  # The device is new, so its app data and Keychain are empty: nothing to reset or erase.
  simctl_own install "$sim_udid" "$watch_app" || die "simctl install failed"
  simctl_own privacy "$sim_udid" grant microphone "$WATCH_BUNDLE" >/dev/null 2>&1

  reg_add path "$tmp/stage2-watch.log"
  : > "$tmp/stage2-watch.log"
  owned_bg simctl xcrun simctl spawn "$sim_udid" log stream --style compact \
    --predicate 'subsystem == "dev.wristper.probe"' > "$tmp/stage2-watch.log" 2>&1
  sleep 2

  local start line code
  start=$(( $(wc -l < "$tmp/stage2-watch.log") + 1 ))
  simctl_own launch --terminate-running-process "$sim_udid" "$WATCH_BUNDLE" -e2ePair >/dev/null \
    || die "simctl launch failed"
  line=$(wait_log 'E2E pair-code [0-9]{6}' 30 "$start") || { record 2 pairing - FAIL 0 no-code 0 "" null; return; }
  code=$(grep -oE '[0-9]{6}$' <<<"$line")
  log "watch app showed a pairing code; hex watch pair ******"
  local t0; t0=$(now_ms)
  owned_fg hex "$hex" watch pair "$code" 2>&1 | tee -a "$out/run.log"
  line=$(wait_log 'E2E (paired|pair-failed)' 150 "$start")
  if grep -q "E2E paired" <<<"$line"; then
    record 2 pairing - PASS 100 paired $(( $(now_ms) - t0 )) "" "$(jq -cn --arg l "$line" '{log:$l}')"
  else
    record 2 pairing - FAIL 0 "${line:-timeout}" $(( $(now_ms) - t0 )) "" null
    return
  fi

  local container target lang handle readback percent result started ms enter_arg detail res_line phase mac_state precision gate kind note
  container=$(xcrun simctl get_app_container "$sim_udid" "$WATCH_BUNDLE" data) || die "no app container"
  mkdir -p "$container/tmp"
  for lang in ${langs//,/ }; do
    select_model "$lang"
    for target in ${targets//,/ }; do
      row_begin; FAILS=; SC_REPORT=null; SC_MS=0; : > "$SC_FILE"
      kind=$(kind_of "$target")
      cp "$out/$lang.m4a" "$container/tmp/fixture-$lang.m4a"
      gate="$container/tmp/gate-$lang-$target"
      handle=$(open_target "$target") || handle=
      if [ -z "$handle" ]; then record 2 "$target" "$lang" INCOMPLETE 0 no-target 0 "" '{"fails":"could not open the target; nothing was sent"}'; continue; fi
      sleep 1
      if ! focus_target "$target" "$handle"; then
        close_target "$target" "$handle"
        record 2 "$target" "$lang" INCOMPLETE 0 no-focus 0 "" '{"fails":"could not focus the window this run made; the watch app was not started"}'; continue
      fi
      enter_arg=; [ "$target" = terminal ] && enter_arg=-e2eEnter
      start=$(( $(wc -l < "$tmp/stage2-watch.log") + 1 ))
      started=$(now_ms)
      simctl_own launch --terminate-running-process "$sim_udid" "$WATCH_BUNDLE" \
        -e2eRecord -fixtureAudio "$container/tmp/fixture-$lang.m4a" -e2eGateFile "$gate" $enter_arg >/dev/null
      # The watch app records, then waits for the gate file before it uploads. The file is made only if
      # the window this run made is still the one in front at that moment.
      note=
      if wait_log 'E2E gate-wait' 60 "$start" >/dev/null; then
        owned window "$(win_key "$kind" "$handle")" act_open_gate "$kind" "$handle" "$gate" \
          || note="the window this run made was not in front when the watch app was about to upload; it was stopped and nothing was sent"
      else
        note="the watch app never reached its upload gate"
      fi
      if [ -n "$note" ]; then
        simctl_own terminate "$sim_udid" "$WATCH_BUNDLE" >/dev/null 2>&1
        close_target "$target" "$handle"
        record 2 "$target" "$lang" INCOMPLETE 0 aborted $(( $(now_ms) - started )) "" "$(jq -cn --arg f "$note" '{fails:$f}')"
        continue
      fi
      wait_log 'E2E done' 150 "$start" >/dev/null
      ms=$(( $(now_ms) - started ))
      sleep 1
      readback=$(read_target "$target" "$handle")
      close_target "$target" "$handle"
      detail=$(tail -n +"$start" "$tmp/stage2-watch.log" | grep -E 'E2E (fixture|result|enter|gate)' | jq -R . | jq -sc '{log:.}')
      percent=$(judge "$lang" "$readback"); percent=${percent:-0}
      # PASS needs both: the Mac's own final state (from the watch app's E2E result line) and the text.
      res_line=$(tail -n +"$start" "$tmp/stage2-watch.log" | grep 'E2E result'; true); res_line=${res_line%%$'\n'*}
      phase=$(sed -nE 's/.*phase=([a-z]+).*/\1/p' <<<"$res_line")
      mac_state=$(sed -nE 's/.*macState\{state=([a-z-]+).*/\1/p' <<<"$res_line")
      chk "$(is [ "$phase" = finished ])" "watch phase is '${phase:-none}', want finished"
      case "$mac_state" in pasted|unverified) ;; *) chk bad "Mac state is '${mac_state:-none}', want pasted or unverified" ;; esac
      chk "$(is [ "$percent" -ge "$(pass_for "$lang")" ])" "read-back matches only ${percent}% (ordered, need $(pass_for "$lang"))"
      if starts_empty "$target"; then
        precision=$(precision_of "$lang" "$readback"); precision=${precision:-0}
        chk "$(is [ "$precision" -ge "$(pass_for "$lang")" ])" "only ${precision}% of the read-back is the sentence (pasted twice, or extra text?)"
      fi
      result=PASS; [ -z "$FAILS" ] || result=FAIL
      row_refused && result=INCOMPLETE
      record 2 "$target" "$lang" "$result" "$percent" "${mac_state:-no-result}" "$ms" \
        "$(printf %s "$readback" | tr '\r\n' '  ' | tail -c 200)" "$(jq -c --arg f "${FAILS#; }" --arg r "$(printf %s "$readback" | head -c 400)" '. + {fails:$f, readback:$r}' <<<"$detail")"
      FAILS=
    done
  done
}

archive_stage2_log() { [ -s "$tmp/stage2-watch.log" ] && redact_pairing < "$tmp/stage2-watch.log" > "$out/stage2-watch.log"; return 0; }

# =============================================================================
# Stage 4: the B2 live tests (opt in: --with-live)
# =============================================================================
# These Rust tests (src/watch/live_tests.rs) drive real windows. Each one records the window id (and the
# document name or tty) of the TextEdit document or Terminal window it makes, checks the live window against
# that before it pastes, presses, or closes, and closes only that window by id. They never touch a front
# document or front window, so there is nothing for this script to check beforehand. Still opt in (--with-live):
# they send real keys (Return, Command-Z) and have not yet been seen to pass on a real Mac.
stage4() {
  local log4="$out/stage4-live.log" name res rc
  row_begin
  issue_live_terminal_token || { record 4 live - INCOMPLETE 0 gate-closed 0 "" '{"fails":"the Terminal gate is closed, so no one-time token was issued for the Rust live tests"}'; return; }
  log "stage 4: cargo nextest run --locked --bin hex --run-ignored only -E 'test(watch_live)'"
  [ -f "$WATCH_E2E_B2_ENV" ] && . "$WATCH_E2E_B2_ENV"
  # One test at a time: both drive the same screen.
  (cd "$root" && owned_fg cargo cargo nextest run --locked --bin hex --run-ignored only -E 'test(watch_live)' --no-fail-fast -j 1) > "$log4" 2>&1
  rc=$?
  # Lines like "        PASS [   3.2s] hex watch::live_tests::watch_live_…"; failures print again in the summary.
  while read -r res name; do
    [ "$res" = PASS ] || res=FAIL
    record 4 "${name##*::}" - "$res" "$([ "$res" = PASS ] && echo 100 || echo 0)" "$(tr 'A-Z' 'a-z' <<<"$res")" 0 "" "null"
  done < <(sed -nE 's/^ *(PASS|FAIL|SIGABRT|SIGSEGV|SIGKILL|TIMEOUT|LEAK) +\[[^]]*\] +(.* )?([^ ]*watch_live[^ ]*).*$/\1 \3/p' "$log4" | awk '!seen[$2]++')
  if ! jq -e 'select(.stage=="4")' "$results" >/dev/null 2>&1; then
    record 4 live - FAIL 0 "no-test-ran(exit $rc)" 0 "$(tail -c 200 "$log4" | tr '\n' ' ')" null
  elif [ $rc -ne 0 ] && ! jq -e 'select(.stage=="4" and .result!="PASS")' "$results" >/dev/null; then
    record 4 live - FAIL 0 "nextest-exit-$rc" 0 "$(tail -c 200 "$log4" | tr '\n' ' ')" null
  fi
}

# =============================================================================
# The report
# =============================================================================
print_table() {
  echo
  printf '%-5s %-26s %-4s %-10s %6s %-14s %7s  %s\n' STAGE SCENARIO LANG RESULT MATCH STATE MS "READ BACK"
  # Fields are joined with the unit separator, not a tab: `read` would collapse the empty ones (an empty
  # read-back) and shift the notes into the wrong column.
  jq -r '[.stage,.target,.lang,.result,(.match_percent+"%"),.state,.ms,(.text|.[0:60]),((.detail.fails // "")|tostring)] | join("\u001f")' "$results" |
    while IFS=$'\037' read -r a b c d e f g h i; do
      printf '%-5s %-26s %-4s %-10s %6s %-14s %7s  %s\n' "$a" "$b" "$c" "$d" "$e" "$f" "$g" "$h"
      [ -n "$i" ] && printf '        └ %s\n' "$i"
    done
  echo
  echo "Details: $results"
}

# The final verdict from the rows. FAIL beats everything; BLOCKED or INCOMPLETE rows mean "not proven".
verdict() {
  local not_proven
  if jq -e 'select(.result=="FAIL")' "$results" >/dev/null; then
    echo "RESULT: FAIL"; return 1
  fi
  [ -s "$results" ] || { echo "RESULT: FAIL (no cases ran)"; return 1; }
  not_proven=$(jq -r 'select(.result=="BLOCKED" or .result=="INCOMPLETE") | .result + ":" + .target' "$results" | tr '\n' ' ')
  if [ -n "$not_proven" ]; then
    echo "RESULT: INCOMPLETE (no row failed, but not proven: ${not_proven}; BLOCKED rows need C1, run --c1 after it merges)"
    return 4
  fi
  echo "RESULT: PASS"
}

# =============================================================================
# main
# =============================================================================
main() {
  local rc t
  init ""
  for t in jq python3 perl osascript say afconvert xcrun codesign lsappinfo; do
    command -v "$t" >/dev/null || die "missing tool: $t" 2
  done
  if stage_on 4; then
    command -v cargo >/dev/null || die "missing tool: cargo (stage 4)" 2
    cargo nextest --version >/dev/null 2>&1 || die "missing cargo-nextest (brew install cargo-nextest)" 2
  fi
  # The Terminal gate comes first, before anything is built, started, or opened.
  stage4_block
  if [ "$mode" != check-terminal ] && terminal_gate_block; then :; fi
  [ ${#blocked[@]} -eq 0 ] || print_blocked
  # Safety: the test build's data root is its own, and nothing below names the installed app's folder.
  [ "$TEST_DATA" != "$OLD_DATA" ] || die "test data root equals the installed app's" 2

  # Which apps were running at the start: written to the log only. It decides nothing: this run closes only
  # windows it made and quits only the one Wristper Test it started.
  for t in TextEdit Terminal "Google Chrome"; do
    if app_running "$t"; then reg_add ran "$t" running; else reg_add ran "$t" not-running; fi
  done
  log "apps running before this run (information only): $(reg_list ran | tr '\t\n' ': ' | sed 's/:$//')"

  # An app that was already running is never stopped, reset, driven, or rebuilt underneath: refuse before
  # anything else (the build below would overwrite and re-sign its bundle).
  if wt_running; then
    need "Wristper Test is already running (pid $(wt_pids | tr '\n' ' ')). This script only builds, stops, and resets a Wristper Test that it started itself, so it refuses. Quit it yourself (\"$hex\" quit), then run again. Nothing was built, stopped, or reset."
    print_blocked
  fi

  # A locked screen hides every window from Accessibility and AppleScript. It is only recorded here: the
  # build below still runs, and check_permissions then reports it with everything else. Nothing unlocks it.
  if screen_locked; then
    need "The screen is locked (IOConsoleLocked). Unlock this Mac and keep it awake and idle for the run (about 15 min; System Settings > Lock Screen can keep it from locking again). Nothing here unlocks it."
    log "screen is locked: GUI scenarios cannot run; building only"
  fi

  build_pbtool
  [ "$build" = 1 ] && build_mac_app
  [ -x "$hex" ] || die "$app is missing; run without --skip-build" 2
  [ "$(/usr/libexec/PlistBuddy -c 'Print :CFBundleIdentifier' "$app/Contents/Info.plist")" = "$TEST_BUNDLE" ] \
    || die "$app is not $TEST_BUNDLE" 2
  if { [ "$build" = 1 ] || [ ! -x "$sim" ] || [ "$(cat "$(dirname "$sim")/built-from-commit" 2>/dev/null)" != "$WRISTPER_SRC_COMMIT" ] \
       || [ "$root/scripts/watch-sim/main.swift" -nt "$sim" ] || [ "$root/scripts/watch-sim/WatchStoreShim.swift" -nt "$sim" ]; } &&
     { stage_on 1 || stage_on 3; }; then build_watch_sim; fi
  log "watch sources pinned: watch-sim $WRISTPER_SRC_COMMIT, stage 2 app $WRISTPER_HOOKS_COMMIT"

  start_app
  check_permissions
  [ "$preflight_only" = 1 ] && { log "preflight OK"; echo "PREFLIGHT: OK"; exit 0; }
  clip_backup
  restart_with_new_room
  select_model "${langs%%,*}"
  wait_listener
  make_fixtures
  stage_on 1 && stage1
  stage_on 2 && { stage2; archive_stage2_log; }
  stage_on 3 && stage3
  stage_on 4 && stage4

  teardown   # before the table, so what it did and refused is in the table
  case "$CLIP_STATE" in
    restored) record x clipboard-restored - PASS 100 verified 0 "" "$(jq -cn --arg d "$CLIP_DETAIL" '{detail:$d}')" ;;
    failed) record x clipboard-restored - FAIL 0 not-restored 0 "" "$(jq -cn --arg d "$CLIP_DETAIL" --arg p "$tmp/clipboard" '{fails:("your clipboard was NOT restored: " + $d + ". Backup kept at " + $p)}')" ;;
  esac
  if [ -s "$out/refusals.tsv" ]; then
    record x ownership-guard - INCOMPLETE 0 refused 0 "" "$(jq -cn --arg r "$(tr '\t\n' ' ;' < "$out/refusals.tsv")" '{fails:("the gate refused to act on something not in this run'"'"'s registry: " + $r)}')"
  fi
  print_table
  verdict; rc=$?
  exit $rc
}

# =============================================================================
# check-textedit: the TextEdit handling, on a real Mac, in about 5 seconds
# =============================================================================
check_textedit() {
  local before_ids= before_names= after_ids after_names h id text back ok=1 started asn icloud_dir icloud_before icloud_after
  local file dir name live_name live_path filetext tmp_for_check
  started=$(now_ms)
  init "check-textedit-"
  step() { printf '  %-4s %s\n' "$1" "$2"; [ "$1" = PASS ] || ok=0; }
  echo "check-textedit: one TextEdit document, backed by a file in this run's own temp folder, nothing else"
  if app_running TextEdit; then
    before_ids=$(te_document_window_ids)
    before_names=$(osascript -e 'tell application "TextEdit" to return name of every document' 2>/dev/null)
    echo "  (TextEdit is running with $(tr ' ' '\n' <<<"$before_ids" | grep -c .) document window(s); their ids and names are recorded and must be the same at the end)"
  else
    echo "  (TextEdit is not running; the run starts it and leaves it running)"
  fi
  icloud_dir="$HOME/Library/Mobile Documents/com~apple~TextEdit/Documents"
  icloud_before=$(ls -1 "$icloud_dir" 2>/dev/null)
  echo "  (TextEdit's iCloud folder holds $(grep -c . <<<"$icloud_before") file(s) before; the list must be the same at the end)"
  tmp_for_check=$tmp
  row_begin
  h=$(open_target textedit) || h=
  if [ -n "$h" ]; then step PASS "opened a document from a file: window ${h%%|*}"; else step FAIL "could not open a document ($(tail -1 "$out/run.log" 2>/dev/null))"; fi
  if [ -n "$h" ]; then
    id=${h%%|*}; file=${h#*|}; dir=$(dirname -- "$file"); name=$(basename -- "$file")
    case "$(real_path "$file")" in "$tmp_real"/*) step PASS "the file is inside this run's temp folder ($file)" ;; *) step FAIL "the file is outside the temp folder: $file" ;; esac
    case "$(real_path "$file")" in "$(real_path "$HOME/Library/Mobile Documents")"/*) step FAIL "the file is in iCloud Drive" ;; *) step PASS "the file is not in iCloud Drive" ;; esac
    [ -f "$file" ] && [ ! -s "$file" ] && step PASS "the file exists and is empty" || step FAIL "the file is missing or not empty"
    reg_has path "$file" && reg_has path "$dir" && step PASS "the registry holds the file and its folder" || step FAIL "the registry does not hold the file and folder"
    [ "$(win_probe textedit "$h")" = ours ] && step PASS "the live window $id shows the registered file" || step FAIL "the live window is not the registered one"
    [ "$(reg_get window "textedit:$id")" = "$file" ] && step PASS "the registry holds window $id and the file path" || step FAIL "the registry does not hold the window"
    [ "$(win_probe textedit "999999|$file")" = other ] && step PASS "an id that is not in the registry is refused" || step FAIL "an unregistered id was not refused"
    [ "$(win_probe textedit "$id|$dir/not-the-registered-file.txt")" = other ] && step PASS "the registered id with a path that is not the registered one is refused" || step FAIL "a wrong path was not refused"
    # The two checks stage 1 makes just before audio goes out: focus the window by id, then the front check.
    focus_target textedit "$h" && step PASS "focused window $id by its id" || step FAIL "could not focus window $id"
    sleep 0.5
    if front_check textedit "$h"; then
      step PASS "the front check says window $id is the one in front (by window id)"
    else
      asn=$(lsappinfo front 2>/dev/null)
      step FAIL "the front check does not see window $id in front (front app: $(lsappinfo info -only bundleid "$asn" 2>/dev/null | tr -s ' ' | tr -d '\n'); TextEdit's first document window: $(te_first_document_window))"
    fi
    text="e2e-smoke-$RANDOM$RANDOM$RANDOM"   # exists only at run time
    write_target textedit "$h" "$text" >/dev/null && step PASS "wrote text into window $id" || step FAIL "could not write into the window"
    back=$(read_target textedit "$h")
    same_text "$back" "$text" && step PASS "read back the same text" || step FAIL "read back '${back:0:60}', wrote '$text'"
    # Time for TextEdit's autosave (an untitled document was saved to iCloud within about a second): wait, then look.
    sleep 5
    live_name=$(osascript -e "tell application \"TextEdit\" to return name of document of window id $id" 2>/dev/null)
    live_path=$(osascript -e "tell application \"TextEdit\" to return path of document of window id $id" 2>/dev/null)
    [ "$live_name" = "$name" ] && step PASS "5 s after the edit the document still has its name ($live_name)" || step FAIL "the document name changed: '$live_name', registered '$name'"
    [ -n "$live_path" ] && [ "$(real_path "$live_path")" = "$(real_path "$file")" ] && step PASS "…and still has its path" || step FAIL "the document path changed: '$live_path', registered '$file'"
    [ "$(win_probe textedit "$h")" = ours ] && step PASS "…and the window is still recognised as ours" || step FAIL "the window is no longer recognised as ours after the edit"
    filetext=$(cat "$file" 2>/dev/null; echo x); filetext=${filetext%x}
    if [ -z "$filetext" ]; then step PASS "the file is still empty (TextEdit had not saved the edit into it)"
    elif same_text "$filetext" "$text"; then step PASS "the file holds exactly the text (TextEdit saved the edit into the file)"
    else step FAIL "the file holds something else: '${filetext:0:60}'"; fi
    icloud_after=$(ls -1 "$icloud_dir" 2>/dev/null)
    [ "$icloud_after" = "$icloud_before" ] \
      && step PASS "5 s after the edit no file appeared in TextEdit's iCloud Drive folder" \
      || step FAIL "a file appeared in $icloud_dir: $(diff <(echo "$icloud_before") <(echo "$icloud_after") | grep '^>' | tr '\n' ' ')"
    close_target textedit "$h"
    # Asked directly: after the close the registry no longer holds the window, so win_probe would say "other".
    [ "$(osascript -e "tell application \"TextEdit\" to return exists window id $id" 2>/dev/null)" = false ] \
      && step PASS "window $id is gone after the close" || step FAIL "window $id is still there"
    [ ! -e "$file" ] && step PASS "the file was moved to the Trash" || step FAIL "the file is still at $file"
    [ ! -e "$dir" ] && step PASS "the file's folder was moved to the Trash" || step FAIL "the folder is still at $dir"
    reg_has window "textedit:$id" && step FAIL "the registry still holds window $id" || step PASS "the registry dropped window $id"
    icloud_after=$(ls -1 "$icloud_dir" 2>/dev/null)
    [ "$icloud_after" = "$icloud_before" ] \
      && step PASS "after the close the iCloud folder list is exactly the one from before" \
      || step FAIL "the iCloud folder list changed: $(diff <(echo "$icloud_before") <(echo "$icloud_after") | grep '^[<>]' | tr '\n' ' ')"
  fi
  if app_running TextEdit; then
    after_ids=$(te_document_window_ids)
    same_id_set "$after_ids" "$before_ids" || { sleep 1; after_ids=$(te_document_window_ids); }   # the person may be moving windows: look once more
    after_names=$(osascript -e 'tell application "TextEdit" to return name of every document' 2>/dev/null)
    same_id_set "$after_ids" "$before_ids" && step PASS "the windows that have a document are exactly the ones from before (the document-less panel TextEdit shows briefly is not counted)" || step FAIL "document window ids differ: before [$before_ids] after [$after_ids]"
    [ "$after_names" = "$before_names" ] && step PASS "TextEdit's document names are exactly the ones from before" || step FAIL "document names differ"
  fi
  row_refused && step FAIL "the gate refused something: $(tr '\n' ';' < "$tmp/row-refused")" || step PASS "the gate refused nothing"
  echo "  elapsed $(( ($(now_ms) - started) / 1000 )) s"
  teardown
  [ ! -e "$tmp_for_check" ] && step PASS "the temp folder is gone after the cleanup" || step FAIL "the temp folder is still at $tmp_for_check"
  if [ "$ok" = 1 ]; then echo "check-textedit: PASS"; return 0; fi
  echo "check-textedit: FAIL"; return 1
}

# =============================================================================
# check-terminal: the Terminal handling, on a real Mac, in about 10 seconds. WRITTEN, NOT YET RUN on this Mac.
# =============================================================================
# One Terminal window of this run's own. It records the window id and the tty, runs `cat` in it (the one registered
# process), sends one line, reads the window back, and closes it by id after stopping the registered process.
# Every other Terminal window is only listed (id and tty, read-only) before and after, and the two lists must match.
# On a pass it writes the stamp that opens the Terminal gate.
check_terminal() {
  local before after h id tty text back ok=1 started pid line
  started=$(now_ms)
  init "check-terminal-"
  step() { printf '  %-4s %s\n' "$1" "$2"; [ "$1" = PASS ] || ok=0; }
  TERMINAL_SMOKE=1
  echo "check-terminal: one Terminal window of this run's own, nothing else"
  terminal_window_list() {
    [ "$(osascript -e 'application "Terminal" is running' 2>/dev/null)" = true ] || return 0
    osascript -e 'tell application "Terminal"' -e 'set out to {}' -e 'set allIds to id of every window' \
      -e 'repeat with wid in allIds' -e 'try' -e 'set end of out to ((contents of wid) as text) & ":" & (tty of selected tab of window id (contents of wid))' \
      -e 'end try' -e 'end repeat' -e 'return out' -e 'end tell' 2>/dev/null | tr ',' '\n' | tr -d ' ' | grep . | sort
  }
  if app_running Terminal; then
    before=$(terminal_window_list)
    echo "  (Terminal is running with $(grep -c . <<<"$before") window(s); their ids and ttys are recorded, read-only, and must be the same at the end)"
  else
    before=
    echo "  (Terminal is not running; the run starts it and leaves it running)"
  fi
  row_begin
  h=$(open_target terminal) || h=
  if [ -n "$h" ]; then step PASS "opened a window: $h"; else step FAIL "could not open a window ($(tail -1 "$out/run.log" 2>/dev/null))"; fi
  if [ -n "$h" ]; then
    id=${h%%|*}; tty=${h#*|}
    [[ $id =~ ^[0-9]+$ ]] && [[ $tty == /dev/ttys* ]] && step PASS "window id $id and tty $tty were recorded" || step FAIL "the handle is not a window id and a tty: $h"
    [ "$(reg_get window "terminal:$id")" = "$tty" ] && step PASS "the registry holds window $id and its tty" || step FAIL "the registry does not hold the window"
    pid=$(reg_list tty-proc | awk -F'\t' -v t="$tty" '{split($2,a,"\037"); if (a[1]==t) print $1}' | head -1)
    [ -n "$pid" ] && step PASS "the one process in the window ($pid) is registered with the tty" || step FAIL "no process is registered for the tty"
    [ "$(win_probe terminal "$h")" = ours ] && step PASS "the live window shows the registered tty" || step FAIL "the live window is not the registered one"
    [ "$(win_probe terminal "999999|$tty")" = other ] && step PASS "an id that is not in the registry is refused" || step FAIL "an unregistered id was not refused"
    [ "$(win_probe terminal "$id|/dev/ttys999")" = other ] && step PASS "the registered id with another tty is refused" || step FAIL "a wrong tty was not refused"
    focus_target terminal "$h" && step PASS "focused window $id by its id" || step FAIL "could not focus window $id"
    sleep 0.5
    front_check terminal "$h" && step PASS "the front check says window $id is the one in front" || step FAIL "the front check does not see window $id in front"
    text="e2e-term-$RANDOM$RANDOM"
    write_target terminal "$h" "$text" >/dev/null && step PASS "sent one line to the window's process" || step FAIL "could not send a line to the window"
    sleep 1
    back=$(read_target terminal "$h")
    [[ $back == *"$text"* ]] && step PASS "read the line back from the window" || step FAIL "the window does not show '$text'"
    close_target terminal "$h"
    [ "$(osascript -e "tell application \"Terminal\" to return exists window id $id" 2>/dev/null)" = false ] \
      && step PASS "window $id is gone after the close" || step FAIL "window $id is still there"
    [ -n "$pid" ] && ! kill -0 "$pid" 2>/dev/null && step PASS "the registered process ($pid) is gone" || step FAIL "the registered process is still running"
    reg_has window "terminal:$id" && step FAIL "the registry still holds window $id" || step PASS "the registry dropped window $id"
  fi
  if app_running Terminal; then
    after=$(terminal_window_list)
    [ "$after" = "$before" ] && step PASS "the other Terminal windows (ids and ttys) are exactly the ones from before" || step FAIL "the Terminal window list differs: before [$(tr '\n' ' ' <<<"$before")] after [$(tr '\n' ' ' <<<"$after")]"
  fi
  row_refused && step FAIL "the gate refused something: $(tr '\n' ';' < "$tmp/row-refused")" || step PASS "the gate refused nothing"
  echo "  elapsed $(( ($(now_ms) - started) / 1000 )) s"
  teardown
  TERMINAL_SMOKE=0
  if [ "$ok" = 1 ]; then
    mkdir -p "$(dirname -- "$TERMINAL_STAMP")"
    printf 'host=%s\nscript=%s\nissued=%s\nat=%s\n' "$(machine_id)" "$(script_sha)" "$(date +%s)" "$(date '+%Y-%m-%d %H:%M:%S')" > "$TERMINAL_STAMP"
    echo "check-terminal: PASS (stamp written: $TERMINAL_STAMP)"; return 0
  fi
  [ -f "$TERMINAL_STAMP" ] && "$TRASH" "$TERMINAL_STAMP" >/dev/null 2>&1   # a stale stamp from an older version is not left to open the gate
  echo "check-terminal: FAIL (no stamp; Terminal stages stay refused)"; return 1
}

# =============================================================================
# selftest: no GUI, no app, no network
# =============================================================================
# `--selftest` runs the real functions above against stand-in `osascript`, `lsappinfo`, and watch-sim
# (shell scripts in a PATH shim folder), and checks the ownership rules: the gate refuses what is not
# registered, a focus failure sends no audio, and the positive control proves "no audio" can be seen.
ST_PASS=0; ST_FAIL=0
st_check() { # name expected-true-command...
  local name=$1; shift
  if "$@"; then ST_PASS=$((ST_PASS + 1)); printf '  PASS  %s\n' "$name"; else ST_FAIL=$((ST_FAIL + 1)); printf '  FAIL  %s\n' "$name"; fi
}
st_shims() {
  mkdir -p "$tmp/bin"
  # osascript stand-in. $STUB_DIR/live: live|gone (does the window still exist?). $STUB_DIR/front: words, one per
  # call (last one repeats): ok|bad (is it in front?). Every call is logged.
  cat > "$tmp/bin/osascript" <<'EOF'
#!/bin/bash
args="$*"
echo "osascript $args" >> "$STUB_DIR/asc.log"
case "$args" in
  *"POSIX file"*)
    # te_open: the stand-in "opens" the file it was given and answers like the real script does
    case "$(cat "$STUB_DIR/te-open" 2>/dev/null)" in
      error) echo "TextEdit got an error: Can’t make document into type reference. (-1700)" >&2; exit 1 ;;
      garbage) echo "missing value" ;;
      empty) : ;;
      outside) echo "101|/etc/hosts" ;;
      *) f=$(sed -E 's/.*POSIX file "([^"]*)".*/\1/' <<<"$args"); echo "$f" > "$STUB_DIR/te-path"; echo "101|$f"
         # a new window that shows the file, in TextEdit's window list
         [ -f "$STUB_DIR/open-no-window" ] || echo "101|$f" >> "$STUB_DIR/scan" ;;
    esac ;;
  *"front window"*|*firstDocumentWindow*)
    read -r w rest < "$STUB_DIR/front"; [ -z "$rest" ] || echo "$rest" > "$STUB_DIR/front"
    case "$args" in
      *TextEdit*) [ "$w" = ok ] && echo "101" || echo "999" ;;
      *) [ "$w" = ok ] && echo "55|/dev/ttys004" || echo "999|/dev/ttys999" ;;
    esac
    exit 0 ;;
  *"return exists window id 102"*) grep -q '^102|' "$STUB_DIR/scan" 2>/dev/null && echo true || echo false ;;
  *"close window id 102"*)
    [ -f "$STUB_DIR/noclose102" ] && exit 0
    grep -v '^102|' "$STUB_DIR/scan" > "$STUB_DIR/scan.new" 2>/dev/null; mv -f "$STUB_DIR/scan.new" "$STUB_DIR/scan" 2>/dev/null ;;
  *"path of document of window id 102"*) grep '^102|' "$STUB_DIR/scan" | head -1 | cut -d'|' -f2- ;;
  *"return exists window id 101"*) [ "$(cat "$STUB_DIR/live")" = live ] && echo true || echo false ;;
  *"return exists window id 202"*) echo false ;;
  *"close window id 101"*)
    [ -f "$STUB_DIR/noclose" ] && exit 0
    echo gone > "$STUB_DIR/live"; grep -v '^101|' "$STUB_DIR/scan" > "$STUB_DIR/scan.new" 2>/dev/null; mv -f "$STUB_DIR/scan.new" "$STUB_DIR/scan" 2>/dev/null ;;
  *"tty of selected tab of window id 303"*) [ "$(cat "$STUB_DIR/live")" = live ] && echo "/dev/ttys303" || echo "" ;;
  *"name of document of window id 101"*)
    [ "$(cat "$STUB_DIR/live")" = live ] || { echo ""; exit 0; }
    basename "$(cat "$STUB_DIR/te-path")" ;;
  *"path of document of window id 101"*)
    [ "$(cat "$STUB_DIR/name" 2>/dev/null)" = moved ] && echo "/stub/Elsewhere.txt" || cat "$STUB_DIR/te-path" ;;
  *orphanScan*)
    [ -f "$STUB_DIR/scan-fail" ] && { echo "scan failed" >&2; exit 1; }
    [ -f "$STUB_DIR/scan-garbage" ] && { echo "this is not a window list"; echo END; exit 0; }
    if [ -f "$STUB_DIR/scan-fail-first" ]; then
      n=$(cat "$STUB_DIR/scan-count" 2>/dev/null || echo 0); n=$((n + 1)); echo "$n" > "$STUB_DIR/scan-count"
      [ "$n" -eq 1 ] && { echo "scan failed" >&2; exit 1; }
    fi
    if [ -f "$STUB_DIR/scan-late" ]; then
      # a window that shows the file appears between the first scan and the one after closing
      n=$(cat "$STUB_DIR/scan-count" 2>/dev/null || echo 0); n=$((n + 1)); echo "$n" > "$STUB_DIR/scan-count"
      cat "$STUB_DIR/scan" 2>/dev/null
      [ "$n" -ge 2 ] && echo "106|$(cat "$STUB_DIR/te-path")"
      echo END; exit 0
    fi
    cat "$STUB_DIR/scan" 2>/dev/null
    [ -f "$STUB_DIR/scan-noend" ] || echo END ;;
  *idsBeforeOpen*)
    [ -f "$STUB_DIR/ids-fail" ] && exit 1
    cat "$STUB_DIR/before-ids" 2>/dev/null; [ -f "$STUB_DIR/ids-noend" ] || echo END ;;
  *documentWindowIds*) grep ':doc' "$STUB_DIR/windows" 2>/dev/null | cut -d: -f1 | paste -sd, - | sed 's/,/, /g' ;;
  *"id of every window"*) cut -d: -f1 "$STUB_DIR/windows" 2>/dev/null | paste -sd, - | sed 's/,/, /g' ;;
  *"is running"*) cat "$STUB_DIR/running" ;;
  *) : ;;
esac
EOF
  # lsappinfo stand-in: who is in front.
  cat > "$tmp/bin/lsappinfo" <<'EOF'
#!/bin/bash
case "$1" in
  front) echo "ASN:0x0-0x1:" ;;
  info) case "$*" in *bundleid*) echo '    bundleID="com.apple.TextEdit"' ;; *pid*) echo '    pid = 1234' ;; esac ;;
esac
EOF
  # watch-sim stand-in with the real contract: log the call, run the gate, send only if it passes.
  cat > "$tmp/fake-sim" <<'EOF'
#!/bin/bash
echo "$*" >> "$STUB_DIR/sim-calls"
[ "$1" = deliver ] || exit 0
hook=; while [ $# -gt 0 ]; do [ "$1" = --after-focus ] && hook=$2; shift; done
if /bin/sh -c "${hook:-false}"; then
  echo "e2ee-sent" >> "$STUB_DIR/sim-calls"
  echo '{"event":"delivered","e2ee_sent":true,"terminal":true,"state":"pasted"}'
else
  echo '{"event":"delivered","e2ee_sent":false,"aborted":"after-focus gate failed"}'
fi
EOF
  # open stand-in: the switch-away action runs `open -a Finder`.
  printf '#!/bin/bash\necho "open $*" >> "$STUB_DIR/open.log"\n' > "$tmp/bin/open"
  chmod +x "$tmp/bin/osascript" "$tmp/bin/lsappinfo" "$tmp/fake-sim" "$tmp/bin/open"
  reg_has tree "$tmp/bin" || reg_add tree "$tmp/bin" -
  reg_has path "$tmp/fake-sim" || reg_add path "$tmp/fake-sim"
}
st_set() { printf '%s\n' "$2" > "$STUB_DIR/$1"; }
st_not() { ! "$@"; }
# Every send must come after its own gate: the audio after the after-focus gate, and each /enter or /undo after
# one more gate of its kind than there were sends before it.
st_order_ok() {
  awk '/^gate:after-focus$/{ga=1} /^send:\/e2ee$/{if(!ga)bad=1}
       /^gate:\/enter$/{ge++} /^send:\/enter$/{se++; if(se>ge)bad=1}
       /^gate:\/undo$/{gu++} /^send:\/undo$/{su++; if(su>gu)bad=1}
       END{exit bad+0}' <<<"$1"
}
st_sent() { [ -f "$STUB_DIR/sim-calls" ] && grep -c '^e2ee-sent' "$STUB_DIR/sim-calls"; return 0; }
# A second, private temp root with its own registry, to see what the cleanup does without touching the real one.
st_sub() { # name
  SUB="$tmp/sub-$1"; mkdir "$SUB"; : > "$SUB/registry.tsv"; : > "$SUB/row-refused"
  printf 'path\t%s\troot\n' "$SUB" >> "$SUB/registry.tsv"
}
st_run_sub() { ( tmp=$SUB; tmp_real=$(real_path "$SUB"); CLIP_STATE=none; "$@" ); }
# The tests above keep TextEdit files on purpose (te-keep). They are the test's own: Trash them with the real command and
# drop the stale registry lines, so the test's temp root can be cleaned at the end.
st_sweep_te() {
  local d
  for d in "$tmp"/textedit.*; do [ -d "$d" ] && "$TRASH" "$d"; done
  awk -F'\t' '!($1=="window" && $2 ~ /^textedit:/)' "$tmp/registry.tsv" > "$tmp/registry.new" && mv -f "$tmp/registry.new" "$tmp/registry.tsv"
  : > "$tmp/row-refused"
}
st_reset() { : > "$STUB_DIR/sim-calls"; : > "$STUB_DIR/asc.log"; : > "$tmp/row-refused"; ABORT_NOTE=; }

selftest_main() {
  init "selftest-"
  STUB_DIR="$tmp/stub"; mkdir -p "$STUB_DIR"; reg_add tree "$STUB_DIR" -
  export STUB_DIR
  st_set running true; st_set te-open ok; st_set name original; st_set te-path /nonexistent
  st_shims
  PATH="$tmp/bin:$PATH"; export PATH
  sim="$tmp/fake-sim"
  local real_sim="$root/target/watch-sim/watch-sim" h pid n before
  echo "selftest: the ownership rules, with stand-ins (no app, no window, no network)"

  echo "[1] the gate refuses what this run did not make"
  st_reset; st_set live live
  close_target textedit "999|Someone else's document"
  st_check "closing an unregistered window is refused" row_refused
  st_check "…and no close command ran" bash -c "! grep -q 'close window' '$STUB_DIR/asc.log'"
  st_reset
  owned path /etc/hosts act_trash_path /etc/hosts
  st_check "Trash of a path that is not registered is refused" row_refused
  st_reset
  ln -s /etc "$tmp/dirlink"; reg_add path "$tmp/dirlink"
  trash_own "$tmp/dirlink/hosts"
  st_check "a path that goes through a link out of the temp root is refused (not registered)" row_refused
  st_reset
  reg_add path "$tmp/dirlink/hosts"; trash_own "$tmp/dirlink/hosts"
  st_check "…and refused although registered: its real location is outside the temp root" row_refused
  st_check "…and /etc/hosts is still there" test -f /etc/hosts
  st_reset
  trash_own "$tmp/dirlink"
  st_check "a registered link inside the temp root is Trashed as a link (never its target)" bash -c "[ ! -L '$tmp/dirlink' ] && [ -f /etc/hosts ] && [ ! -s '$tmp/row-refused' ]"
  : > "$tmp/victim"; reg_add path "$tmp/victim"
  st_reset
  trash_own "$tmp/victim"
  st_check "a registered file inside the temp root is Trashed" bash -c "[ ! -e '$tmp/victim' ] && [ ! -s '$tmp/row-refused' ]"
  st_reset
  owned_bg sleep sleep 120 > /dev/null 2>&1
  pid=$OWNED_PID; sleep 0.5
  entry=$(reg_get pid "$pid")
  reg_del pid "$pid"
  owned pid "$pid" act_stop_group "$pid"
  st_check "stopping an unregistered process group is refused" row_refused
  st_check "…and the process is still running" kill -0 "$pid"
  reg_add pid "$pid" "$entry"
  stop_pid "$pid"
  st_check "a registered group is stopped until no member is left" bash -c "! kill -0 $pid 2>/dev/null"
  st_check "a bad group id is not accepted" st_not act_stop_group 1

  echo "[2] a focus failure sends no audio"
  st_reset; st_set live gone; st_set front "ok"
  : > "$tmp/row-refused"
  SC_FILE="$out/readback.txt"
  sim_run textedit en
  st_check "sim_run aborted" test -n "$ABORT_NOTE"
  st_check "no audio was sent (watch-sim stand-in saw no e2ee)" bash -c "[ \"$(st_sent)\" = 0 ]"
  st_check "watch-sim was not even started" bash -c "! grep -q '^deliver' '$STUB_DIR/sim-calls'"
  FAILS=; finish 9 focus-fails en 0
  st_check "the row is INCOMPLETE" bash -c "jq -e 'select(.target==\"focus-fails\" and .result==\"INCOMPLETE\")' '$results' >/dev/null"

  echo "[1b] only what this run registered, item by item, is Trashed (the registered root authorizes nothing inside it)"
  st_sub scope; sub=$SUB
  : > "$sub/mine.txt"; : > "$sub/stranger.txt"; printf 'path\t%s\t\n' "$sub/mine.txt" >> "$sub/registry.tsv"
  st_run_sub cleanup_root
  st_check "a file nobody registered, in a registered root, is kept" test -f "$sub/stranger.txt"
  st_check "…and the root stays with it" test -d "$sub"
  st_check "…while the registered file beside it was Trashed" bash -c "[ ! -e '$sub/mine.txt' ]"
  "$TRASH" "$sub"
  st_sub allmine; sub=$SUB
  : > "$sub/a.txt"; printf 'path\t%s\t\n' "$sub/a.txt" >> "$sub/registry.tsv"
  st_run_sub cleanup_root
  st_check "(control) when everything in it is registered, the items and then the root are Trashed" bash -c "[ ! -e '$sub/a.txt' ] && [ ! -e '$sub' ]"
  st_sub folder; sub=$SUB
  mkdir "$sub/textedit.AAAAAA"; : > "$sub/textedit.AAAAAA/wristper-verify-1.txt"; : > "$sub/textedit.AAAAAA/stranger.rtf"
  printf 'path\t%s\t\npath\t%s\t\n' "$sub/textedit.AAAAAA" "$sub/textedit.AAAAAA/wristper-verify-1.txt" >> "$sub/registry.tsv"
  printf 'te-before\t%s\t\n' "$sub/textedit.AAAAAA/wristper-verify-1.txt" >> "$sub/registry.tsv"
  st_set live live; st_set running true; : > "$STUB_DIR/scan"; rm -f "$STUB_DIR/scan-fail"
  st_run_sub te_settle_file "$sub/textedit.AAAAAA/wristper-verify-1.txt"
  st_check "a registered file is Trashed" bash -c "[ ! -e '$sub/textedit.AAAAAA/wristper-verify-1.txt' ]"
  st_check "…a file beside it that nobody registered stays" test -f "$sub/textedit.AAAAAA/stranger.rtf"
  st_check "…and so does the folder (it is Trashed only when empty)" test -d "$sub/textedit.AAAAAA"
  st_run_sub cleanup_root
  st_check "the cleanup of the root keeps that folder and the root too" bash -c "[ -f '$sub/textedit.AAAAAA/stranger.rtf' ] && [ -d '$sub' ]"
  "$TRASH" "$sub"
  st_sub tree; sub=$SUB
  mkdir "$sub/tool"; : > "$sub/tool/state.json"; printf 'tree\t%s\t4242\npid\t4242\tmarker\n' "$sub/tool" >> "$sub/registry.tsv"
  st_run_sub cleanup_root
  st_check "a tool folder whose owner process is still registered is kept" test -f "$sub/tool/state.json"
  printf 'tree\t%s\t4242\n' "$sub/tool" > "$sub/registry.tsv"; printf 'path\t%s\troot\n' "$sub" >> "$sub/registry.tsv"
  st_run_sub cleanup_root
  st_check "(control) once the owner is gone the whole tool folder is Trashed, then the root" bash -c "[ ! -e '$sub/tool' ] && [ ! -e '$sub' ]"
  st_sub treelink; sub=$SUB
  ln -s /etc "$sub/lnk"; printf 'tree\t%s\t-\n' "$sub/lnk" >> "$sub/registry.tsv"
  st_run_sub cleanup_root
  st_check "a registered tree that is a link is refused (and its target untouched)" bash -c "[ -L '$sub/lnk' ] && [ -f /etc/hosts ]"
  "$TRASH" "$sub"

  echo "[2c] TextEdit opens a file this run made (never an untitled document, which TextEdit autosaves into iCloud)"
  st_reset; st_set live live; st_set front ok; st_set name original; : > "$STUB_DIR/scan"; : > "$STUB_DIR/before-ids"; rm -f "$STUB_DIR/noclose" "$STUB_DIR/scan-fail"
  printf '7\n8\n' > "$STUB_DIR/before-ids"
  sim_run textedit en
  st_check "no abort" test -z "$ABORT_NOTE"
  tf=$(cat "$STUB_DIR/te-path"); td=$(dirname "$tf")
  st_check "the file is inside the temp folder, in a textedit.* folder, named wristper-verify-*.txt" bash -c "case '$tf' in '$tmp'/textedit.*/wristper-verify-*.txt) exit 0 ;; *) exit 1 ;; esac"
  st_check "the file exists and is empty" bash -c "[ -f '$tf' ] && [ ! -s '$tf' ]"
  st_check "the file, its folder, the window with the file path, and the window ids from before are registered" bash -c "grep -q \"^path.$tf\" '$tmp/registry.tsv' && grep -q \"^path.$td\" '$tmp/registry.tsv' && grep -q \"^window.textedit:101.$tf\" '$tmp/registry.tsv' && grep -q \"^te-before.$tf.7 8 \" '$tmp/registry.tsv'"
  st_check "the audio was sent (focus and the front check use the window id)" bash -c "[ \"$(st_sent)\" = 1 ]"
  st_check "the read-back was not refused" bash -c "[ ! -s '$tmp/row-refused' ]"
  sim_close textedit
  st_check "the window was closed by its id" bash -c "grep -q 'close window id 101' '$STUB_DIR/asc.log'"
  st_check "…its file was moved to the Trash" bash -c "[ ! -e '$tf' ]"
  st_check "…and its folder, now empty, was moved to the Trash" bash -c "[ ! -e '$td' ]"
  st_check "…with no refusal" bash -c "[ ! -s '$tmp/row-refused' ]"
  st_check "…and the registry dropped the window" bash -c "! grep -q 'textedit:101' '$tmp/registry.tsv'"
  FAILS=; finish 9 own-file en 0
  st_check "the row is PASS" bash -c "jq -e 'select(.target==\"own-file\" and .result==\"PASS\")' '$results' >/dev/null"
  # The registry decides: the same id with another path, and an id that is not registered, are refused.
  st_reset; st_set live live; st_set name original; : > "$STUB_DIR/scan"
  h=$(open_target textedit)
  tf=$(cat "$STUB_DIR/te-path"); td=$(dirname "$tf")
  st_check "(setup) a second document is registered" bash -c "[ '$h' = '101|$tf' ] && [ \"$(win_probe textedit "$h")\" = ours ]"
  st_check "a handle whose path is not the registered one is refused" bash -c "[ \"$(win_probe textedit "101|$td/wristper-verify-0.txt")\" = other ]"
  st_check "an id that is not in the registry is refused" bash -c "[ \"$(win_probe textedit "555|$tf")\" = other ]"
  st_reset
  close_target textedit "555|$tf"
  st_check "closing an unregistered id is refused even though TextEdit would answer for it" row_refused
  st_check "…and no close command was sent" bash -c "! grep -q 'close window id 555' '$STUB_DIR/asc.log'"
  st_check "…and the file is still there" test -f "$tf"
  # TextEdit shows the registered id with another file (the person saved it elsewhere): not ours, nothing closed or Trashed
  st_reset; st_set name moved
  st_check "a live document whose path is not the registered file is not ours" bash -c "[ \"$(win_probe textedit "$h")\" = other ]"
  close_target textedit "$h"
  st_check "closing it is refused" row_refused
  st_check "…no close command was sent, and the file was not Trashed" bash -c "! grep -q 'close window id 101' '$STUB_DIR/asc.log' && [ -f '$tf' ]"
  st_set name original; st_reset
  close_target textedit "$h"
  st_check "(test cleanup) the same window closes once it shows the registered file again" bash -c "[ ! -e '$tf' ] && [ ! -s '$tmp/row-refused' ]"

  echo "[2f] fail-closed: whatever cannot be proven keeps the TextEdit file, its folder, and the root"
  # (a) the scan after closing fails
  st_reset; st_set live live; st_set name original; : > "$STUB_DIR/scan"; : > "$STUB_DIR/before-ids"; rm -f "$STUB_DIR/noclose" "$STUB_DIR/noclose102"
  h=$(open_target textedit); tf=$(cat "$STUB_DIR/te-path"); td=$(dirname "$tf")
  touch "$STUB_DIR/scan-fail"
  close_target textedit "$h"
  st_check "(a) the scan fails: the file stays" test -f "$tf"
  st_check "(a) …and its folder" test -d "$td"
  st_check "(a) …with the reason in the registry" bash -c "grep -q \"^te-keep.$tf.\" '$tmp/registry.tsv'"
  rm -f "$STUB_DIR/scan-fail"
  keep_status=0; act_trash_path "$tf" || keep_status=$?
  st_check "(a) every other Trash path obeys it too: act_trash_path refuses a te-keep file" bash -c "[ '$keep_status' != 0 ] && [ -f '$tf' ]"
  te_settle_file "$tf" || true
  st_check "(a) …and a later settle, with a working scan, still keeps it" test -f "$tf"
  # (a2) the scan fails only the first time: the second scan, after closing, is clean, yet the file is not Trashed on it
  st_reset; st_set live live; : > "$STUB_DIR/scan"; rm -f "$STUB_DIR/scan-count"
  h=$(open_target textedit); tfa=$(cat "$STUB_DIR/te-path")
  touch "$STUB_DIR/scan-fail-first"
  close_target textedit "$h"
  rm -f "$STUB_DIR/scan-fail-first" "$STUB_DIR/scan-count"
  st_check "(a2) a scan that failed once keeps the file even though a later scan is clean" test -f "$tfa"
  # (b) the answer cannot be read: no END line, and a line that is not a window
  for flag in scan-noend scan-garbage; do
    st_reset; st_set live live; : > "$STUB_DIR/scan"; rm -f "$STUB_DIR/scan-fail"
    h=$(open_target textedit); tf2=$(cat "$STUB_DIR/te-path")
    printf '9|/Users/me/other.txt\n' >> "$STUB_DIR/scan"   # a valid line, so only the missing END line can fail the scan
    touch "$STUB_DIR/$flag"
    close_target textedit "$h"
    rm -f "$STUB_DIR/$flag"
    st_check "(b) [$flag] an unreadable scan keeps the file" bash -c "[ -f '$tf2' ] && grep -q \"^te-keep.$tf2.\" '$tmp/registry.tsv'"
  done
  # (c) a window that will not close
  st_reset; st_set live live; : > "$STUB_DIR/scan"
  h=$(open_target textedit); tf3=$(cat "$STUB_DIR/te-path")
  touch "$STUB_DIR/noclose"
  close_target textedit "$h"
  rm -f "$STUB_DIR/noclose"
  st_check "(c) a window that does not close keeps the file, and it is a refusal" bash -c "[ -f '$tf3' ] && [ -s '$tmp/row-refused' ]"
  # (d) the file was opened but TextEdit gave no window id: the error comes before the window is registered
  st_reset; st_set live live; st_set te-open error; : > "$STUB_DIR/scan"
  keeps_before=$(grep -c '^te-keep' "$tmp/registry.tsv")
  h=$(open_target textedit)
  keeps_after=$(grep -c '^te-keep' "$tmp/registry.tsv")
  st_check "(d) an error after the file was opened and before its window was registered keeps the file, with a reason" bash -c "[ -z '$h' ] && [ '$keeps_after' -gt '$keeps_before' ]"
  st_set te-open ok
  # (e) the window ids from before cannot be listed: nothing is opened at all
  st_reset; : > "$STUB_DIR/asc.log"; touch "$STUB_DIR/ids-fail"
  h=$(open_target textedit)
  rm -f "$STUB_DIR/ids-fail"
  st_check "(e) when TextEdit's windows cannot be listed first, the file is not opened" bash -c "[ -z '$h' ] && ! grep -q 'POSIX file' '$STUB_DIR/asc.log'"

  st_sweep_te
  # (e) the list of window ids from before has no END line: it is incomplete, so nothing is opened
  st_reset; : > "$STUB_DIR/asc.log"; printf '7\n' > "$STUB_DIR/before-ids"; touch "$STUB_DIR/ids-noend"
  h=$(open_target textedit)
  rm -f "$STUB_DIR/ids-noend"
  st_check "(e2) an incomplete list of the windows from before: the file is not opened" bash -c "[ -z '$h' ] && ! grep -q 'POSIX file' '$STUB_DIR/asc.log'"
  echo "[2g] a second window on the file is claimed only if it appeared after the file was opened, and all must close first"
  # (a) a new window 102 on the same file: both are closed, then the file is Trashed
  st_reset; st_set live live; st_set name original; : > "$STUB_DIR/scan"; printf '7\n' > "$STUB_DIR/before-ids"; rm -f "$STUB_DIR/noclose" "$STUB_DIR/noclose102"
  h=$(open_target textedit); tf=$(cat "$STUB_DIR/te-path"); td=$(dirname "$tf")
  printf '102|%s\n' "$tf" >> "$STUB_DIR/scan"
  : > "$STUB_DIR/asc.log"
  close_target textedit "$h"
  st_check "(a) both windows were closed by id" bash -c "grep -q 'close window id 101' '$STUB_DIR/asc.log' && grep -q 'close window id 102' '$STUB_DIR/asc.log'"
  st_check "(a) …and only then was the file Trashed (with its folder)" bash -c "[ ! -e '$tf' ] && [ ! -e '$td' ] && [ ! -s '$tmp/row-refused' ]"
  st_sweep_te
  # (b) the second window will not close: the file stays
  st_reset; st_set live live; : > "$STUB_DIR/scan"; printf '7\n' > "$STUB_DIR/before-ids"
  h=$(open_target textedit); tf=$(cat "$STUB_DIR/te-path")
  printf '102|%s\n' "$tf" >> "$STUB_DIR/scan"; touch "$STUB_DIR/noclose102"
  close_target textedit "$h"
  rm -f "$STUB_DIR/noclose102"
  st_check "(b) one window left open: the file is kept" test -f "$tf"
  st_sweep_te
  # (c) a window that was already open BEFORE the file was opened shows it: not ours, never closed
  st_reset; st_set live live; : > "$STUB_DIR/scan"; printf '7\n102\n' > "$STUB_DIR/before-ids"
  h=$(open_target textedit); tf=$(cat "$STUB_DIR/te-path")
  printf '102|%s\n' "$tf" >> "$STUB_DIR/scan"
  : > "$STUB_DIR/asc.log"
  close_target textedit "$h"
  st_check "(c) a window from before the run is not closed" bash -c "! grep -q 'close window id 102' '$STUB_DIR/asc.log'"
  st_check "(c) …and the file is kept, with the reason" bash -c "[ -f '$tf' ] && grep -q \"^te-keep.$tf.window 102\" '$tmp/registry.tsv'"
  st_sweep_te
  # (d0) untitled documents show up in a real scan as "<id>|" (empty path): that is a normal line
  st_reset; st_set live live; : > "$STUB_DIR/scan"; printf '7\n' > "$STUB_DIR/before-ids"
  h=$(open_target textedit); tf=$(cat "$STUB_DIR/te-path"); td=$(dirname "$tf")
  printf '178|\n2706|\n' >> "$STUB_DIR/scan"
  close_target textedit "$h"
  st_check "(d0) windows with an empty path (untitled documents) are normal lines: the file is Trashed" bash -c "[ ! -e '$tf' ] && [ ! -e '$td' ]"
  st_sweep_te
  # (d) a document with a comma, quotes, and a backslash in its path does not break the scan
  st_reset; st_set live live; : > "$STUB_DIR/scan"; printf '7\n' > "$STUB_DIR/before-ids"
  h=$(open_target textedit); tf=$(cat "$STUB_DIR/te-path"); td=$(dirname "$tf")
  printf '9|/Users/me/Notes, "draft" \\ v2.txt\n' >> "$STUB_DIR/scan"
  close_target textedit "$h"
  st_check "(d) another document's odd path neither breaks the scan nor keeps this run's file" bash -c "[ ! -e '$tf' ] && [ ! -e '$td' ]"
  st_sweep_te
  # (f) the file is proven free by a FRESH scan after closing: a window that shows it by then keeps the file
  st_reset; st_set live live; : > "$STUB_DIR/scan"; printf '7\n' > "$STUB_DIR/before-ids"; rm -f "$STUB_DIR/scan-count"
  h=$(open_target textedit); tf=$(cat "$STUB_DIR/te-path")
  touch "$STUB_DIR/scan-late"
  close_target textedit "$h"
  rm -f "$STUB_DIR/scan-late" "$STUB_DIR/scan-count"
  st_check "(f) a window that shows the file after the first scan keeps the file" test -f "$tf"
  st_sweep_te
  # (e) te_adopt_orphans: a window that appeared after the open, whose registration was lost, is registered; one from before is not
  st_reset; st_set live live; : > "$STUB_DIR/scan"; printf '7\n' > "$STUB_DIR/before-ids"
  h=$(open_target textedit); tf=$(cat "$STUB_DIR/te-path"); td=$(dirname "$tf")
  reg_del window textedit:101
  printf '103|%s\n' "$tf" >> "$STUB_DIR/scan"
  te_adopt_orphans
  st_check "(e) a window that appeared after the file was opened and shows it is registered" bash -c "grep -q \"^window.textedit:103.$tf\" '$tmp/registry.tsv'"
  printf '104|/Users/me/wristper-verify-1.txt\n' >> "$STUB_DIR/scan"
  te_adopt_orphans
  st_check "(e) a window that only looks like ours by name (another file, same prefix) is not registered" bash -c "! grep -q 'textedit:104' '$tmp/registry.tsv'"
  reg_del window textedit:103
  : > "$STUB_DIR/scan"; reg_del te-before "$tf"; te_keep "$tf" "(test fixture) done"
  "$TRASH" "$td"
  # (g) te_adopt_orphans: a window that was already open before the file was opened is never claimed
  st_sweep_te
  st_reset; st_set live live; : > "$STUB_DIR/scan"; printf '7\n105\n' > "$STUB_DIR/before-ids"
  h=$(open_target textedit); tf=$(cat "$STUB_DIR/te-path")
  reg_del window textedit:101
  printf '105|%s\n' "$tf" >> "$STUB_DIR/scan"
  te_adopt_orphans
  st_check "(g) a window from before the run is not registered for closing, and the file is kept" bash -c "! grep -q 'textedit:105' '$tmp/registry.tsv' && grep -q \"^te-keep.$tf.window 105\" '$tmp/registry.tsv'"

  st_sweep_te
  echo "[2i] te_open refuses a path it cannot put into AppleScript text"
  for odd in 'a,b' 'a"b' 'a\b' 'a b'; do
    st_sub "$odd"; sub=$SUB
    : > "$STUB_DIR/asc.log"
    h=$(st_run_sub te_open)
    st_check "[$odd] nothing is opened and no AppleScript text holds the path" bash -c "[ -z '$h' ] && ! grep -q 'POSIX file' '$STUB_DIR/asc.log'"
    "$TRASH" "$sub"
  done
  st_check "safe_temp_path accepts A-Za-z0-9._/- only (control: an ordinary path passes)" safe_temp_path "/var/folders/ab/T/wristper-e2e.x1/textedit.y2/wristper-verify-1.txt"
  for odd in '/a,b' '/a"b' '/a\b' '/a b' '/a'"'"'b' '/a$b' '/a;b' 'relative/path' '' $'/a\nb'; do
    st_check "safe_temp_path refuses [$odd]" st_not safe_temp_path "$odd"
  done

  echo "[2h] the Terminal gate: no Terminal stage runs before --check-terminal has passed on this Mac, for this script, within 24 hours"
  TERMINAL_STAMP="$tmp/terminal-smoke.ok"; reg_add path "$TERMINAL_STAMP"
  me=$(machine_id); sha=$(script_sha); now=$(date +%s)
  st_check "(setup) this Mac's hardware UUID can be read" test -n "$me"
  st_stamp() { printf 'host=%s\nscript=%s\nissued=%s\n' "$1" "$2" "$3" > "$TERMINAL_STAMP"; }
  st_reset; : > "$STUB_DIR/asc.log"
  [ -e "$TERMINAL_STAMP" ] && "$TRASH" "$TERMINAL_STAMP"
  st_check "no stamp: the gate is closed" st_not terminal_gate_ok
  h=$(open_target terminal)
  st_check "no stamp: open_target terminal refuses, and no Terminal window was asked for" bash -c "[ -z '$h' ] && ! grep -q 'do script' '$STUB_DIR/asc.log'"
  for k in shell recorder claude codex; do
    h=$(open_target $k)
    st_check "no stamp: open_target $k refuses too" bash -c "[ -z '$h' ] && ! grep -q 'do script' '$STUB_DIR/asc.log'"
  done
  st_stamp "$me" "$sha" "$now"
  st_check "(control) a stamp for this Mac, this script, issued now opens the gate" terminal_gate_ok
  open_target terminal >/dev/null
  st_check "(control) …and then open_target does ask Terminal for a window (so the refusals can be seen)" bash -c "grep -q 'do script' '$STUB_DIR/asc.log'"
  st_stamp "$me" "$sha" "$((now - 86400 + 120))"
  st_check "a stamp 2 minutes before its 24 hours are up still opens the gate" terminal_gate_ok
  st_stamp "$me" "$sha" "$((now - 86400 - 120))"
  st_check "an expired stamp (24 hours and 2 minutes old) is refused" st_not terminal_gate_ok
  st_stamp "$me" "$sha" "$((now - 90000))"
  st_check "…and says why" bash -c "[[ \"$(terminal_gate_problem)\" == *hours\ old* ]]"
  st_stamp "$me" "$sha" "$((now + 3600))"
  st_check "a stamp dated in the future is refused (it would never expire)" st_not terminal_gate_ok
  st_stamp "0000-OTHER-MAC" "$sha" "$now"
  st_check "a stamp from another Mac is refused" st_not terminal_gate_ok
  st_check "…and says why" bash -c "[[ \"$(terminal_gate_problem)\" == *another\ Mac* ]]"
  st_stamp "" "$sha" "$now"
  st_check "a stamp with an empty host is refused" st_not terminal_gate_ok
  st_stamp "$me" "$sha" "$now"
  st_check "a Mac whose UUID cannot be read never opens the gate, even for a stamp whose host is empty" bash -c "$(declare -f terminal_gate_problem terminal_gate_ok script_sha); machine_id() { :; }; TERMINAL_STAMP='$TERMINAL_STAMP'; self='$self'; TERMINAL_STAMP_MAX_AGE=86400; printf 'host=\nscript=%s\nissued=%s\n' \"\$(script_sha)\" '$now' > '$TERMINAL_STAMP'; ! terminal_gate_ok"
  st_stamp "$me" "$sha" "$now"
  cp "$self" "$tmp/edited-script.sh"; reg_add path "$tmp/edited-script.sh"; printf '# one more line\n' >> "$tmp/edited-script.sh"
  keep_self=$self; self="$tmp/edited-script.sh"
  st_check "the script changed after the stamp was issued: refused (the stamp covers the whole file)" st_not terminal_gate_ok
  self=$keep_self
  st_check "(control) with the script as it was, the same stamp opens the gate" terminal_gate_ok
  st_stamp "$me" "" "$now"
  st_check "a stamp with no script hash is refused" st_not terminal_gate_ok
  : > "$TERMINAL_STAMP"
  st_check "an empty stamp is refused" st_not terminal_gate_ok
  stamp_var="WATCH_E2E_TERMINAL""_STAMP"
  st_check "no environment variable names another stamp file: the path is fixed in the script" bash -c "! grep -q '$stamp_var' '$self'"
  # the one-time token the Rust live tests need
  st_stamp "$me" "$sha" "$now"
  tokenfile=$tmp/live-terminal-once
  tok=$( issue_live_terminal_token && echo "$WATCH_E2E_LIVE_TERMINAL_ONCE:$(cat "$tokenfile")" )
  st_check "with the gate open, a one-time token is issued: 32 hex characters, the same in the variable and the file" bash -c "[[ '${tok%%:*}' =~ ^[0-9a-f]{32}\$ ]] && [ '${tok%%:*}' = '${tok##*:}' ]"
  [ -e "$tokenfile" ] && "$TRASH" "$tokenfile"
  st_stamp "$me" "$sha" "$((now - 90000))"
  tok=$( issue_live_terminal_token && echo issued )
  st_check "with the gate closed, no token is issued and no file is made" bash -c "[ -z '$tok' ] && [ ! -e '$tokenfile' ]"
  st_check "(a stage run) stage 1 with --targets textedit does not use Terminal" bash -c "stage=1; with_live=0; targets=textedit; $(declare -f stage_on uses_app); ! uses_app Terminal"
  for cfg in "1 terminal" "1 textedit,terminal" "2 terminal" "3 textedit" "all textedit,terminal"; do
    st_check "(a stage run) stage ${cfg%% *} with targets ${cfg#* } uses Terminal" bash -c "stage=${cfg%% *}; with_live=0; targets=${cfg#* }; $(declare -f stage_on uses_app); uses_app Terminal"
  done
  st_check "(a stage run) stage 4 uses Terminal" bash -c "stage=4; with_live=1; targets=textedit; $(declare -f stage_on uses_app); uses_app Terminal"
  blocked=(); stage=1; targets=terminal; with_live=0
  terminal_gate_block
  st_check "a Terminal run without a valid stamp is blocked, with the --check-terminal instruction and the reason" bash -c "[ '${#blocked[@]}' = 1 ] && [[ '${blocked[0]}' == *--check-terminal* ]] && [[ '${blocked[0]}' == *hours\ old* ]]"
  blocked=(); targets=textedit
  st_check "a TextEdit-only run is not blocked by the gate" st_not terminal_gate_block
  blocked=(); targets=textedit,terminal; st_stamp "$me" "$sha" "$now"
  st_check "(control) a Terminal run with a valid stamp is not blocked" st_not terminal_gate_block
  blocked=(); stage=4; with_live=1
  stage4_block
  st_check "stage 4 is blocked (its Rust tests still make untitled TextEdit documents)" bash -c "[ '${#blocked[@]}' = 1 ] && [[ '${blocked[0]}' == *'untitled TextEdit'* ]]"
  blocked=(); stage=1; with_live=0
  st_check "stages 1 to 3 are not blocked by it" st_not stage4_block
  stage=all; targets=textedit,terminal; blocked=()
  # the real script, run from a copy whose root has no stamp: refuses a Terminal stage before building anything
  mkdir -p "$tmp/fakeroot/scripts"; reg_add tree "$tmp/fakeroot" -
  cp "$self" "$tmp/fakeroot/scripts/verify-watch-e2e.sh"
  st_check "the real script refuses a Terminal stage before building anything (exit 3, names --check-terminal)" bash -c "o=\$(/bin/bash '$tmp/fakeroot/scripts/verify-watch-e2e.sh' --stage 1 --targets terminal --skip-build 2>&1); rc=\$?; [ \$rc = 3 ] && grep -q -- '--check-terminal' <<<\"\$o\" && ! grep -q 'building Wristper Test' <<<\"\$o\""
  st_check "…and so does stage 4" bash -c "o=\$(/bin/bash '$tmp/fakeroot/scripts/verify-watch-e2e.sh' --stage 4 --skip-build 2>&1); rc=\$?; [ \$rc = 3 ] && grep -q 'Stage 4' <<<\"\$o\""

  echo "[2j] where the temp folder may be: a plain path of A-Za-z0-9._/- outside Documents, Desktop, and iCloud"
  mkdir -p "$tmp/home/Library/Mobile Documents/x" "$tmp/home/Library/CloudStorage/x" "$tmp/home/Documents/x" "$tmp/home/Desktop/x" "$tmp/home/plain" "$tmp/iCloudish" "$tmp/a,b" "$tmp/a\"b" "$tmp/a\\b" "$tmp/a b"
  reg_add tree "$tmp/home" -; reg_add tree "$tmp/iCloudish" -; reg_add tree "$tmp/a,b" -; reg_add tree "$tmp/a\"b" -; reg_add tree "$tmp/a\\b" -; reg_add tree "$tmp/a b" -
  ln -s "$tmp/home/Documents/x" "$tmp/linkdoc"; reg_add path "$tmp/linkdoc"
  st_tp_refused() { [ -n "$(tmpdir_problem "$1" "$tmp/home")" ]; }
  st_check "(control) an ordinary folder is accepted" st_not st_tp_refused "$tmp/home/plain"
  for odd in "home/Library/Mobile Documents/x" "home/Library/CloudStorage/x" "home/Documents/x" "home/Desktop/x" iCloudish 'a,b' 'a"b' 'a\b' 'a b' linkdoc nonexistent; do
    st_check "a temp folder at [$odd] is refused" st_tp_refused "$tmp/$odd"
  done
  st_check "a relative path is refused" st_tp_refused relative/path
  ln -s "$tmp/home/plain" "$tmp/l,ink"; reg_add path "$tmp/l,ink"
  ln -s "$tmp/a,b" "$tmp/plainlink"; reg_add path "$tmp/plainlink"
  st_check "a link whose own name has a comma, to a plain folder, is refused (the value is checked, not only the real path)" st_tp_refused "$tmp/l,ink"
  st_check "a plain-named link to a folder whose real path has a comma is refused (the real path is checked too)" st_tp_refused "$tmp/plainlink"
  mkdir -p "$tmp/home/Documents/tt"; reg_add tree "$tmp/home/Documents/tt" -
  st_check "the real script refuses such a TMPDIR at once (exit 2) and makes nothing in it" bash -c "HOME='$tmp/home' TMPDIR='$tmp/home/Documents/tt' /bin/bash '$self' --check-textedit >/dev/null 2>&1; rc=\$?; [ \$rc = 2 ] && [ -z \"\$(ls -A '$tmp/home/Documents/tt')\" ]"
  st_check "…and so does a TMPDIR with a comma in it" bash -c "HOME='$tmp/home' TMPDIR='$tmp/a,b' /bin/bash '$self' --check-textedit >/dev/null 2>&1; rc=\$?; [ \$rc = 2 ] && [ -z \"\$(ls -A '$tmp/a,b')\" ]"

  echo "[3] positive control: focus and front both fine, audio IS sent"
  st_reset; st_set live live; st_set front "ok"
  sim_run textedit en
  st_check "no abort" test -z "$ABORT_NOTE"
  st_check "the audio was sent (so the 'not sent' checks can fail)" bash -c "[ \"$(st_sent)\" = 1 ]"
  sim_close textedit
  st_check "its window was closed through the gate" bash -c "grep -q 'close window id 101' '$STUB_DIR/asc.log'"
  FAILS=; ABORT_NOTE=; finish 9 positive-control en 0

  echo "[4] the front app is not the target: no audio"
  st_reset; st_set live live; st_set front "bad"
  sim_run textedit en
  st_check "sim_run aborted" test -n "$ABORT_NOTE"
  st_check "no audio was sent" bash -c "[ \"$(st_sent)\" = 0 ]"
  sim_close textedit
  FAILS=; finish 9 front-bad en 0

  echo "[5] the app moves away between /focus and /e2ee: the gate inside watch-sim stops the audio"
  st_reset; st_set live live; st_set front "ok bad"   # the first check passes, the one inside watch-sim fails
  sim_run textedit en
  st_check "watch-sim was started" bash -c "grep -q '^deliver' '$STUB_DIR/sim-calls'"
  st_check "…but sent no audio" bash -c "[ \"$(st_sent)\" = 0 ]"
  st_check "…and the run noted the abort" test -n "$ABORT_NOTE"
  sim_close textedit
  FAILS=; finish 9 gate-in-sim en 0

  echo "[6] the real watch-sim gate contract"
  if [ -x "$real_sim" ]; then
    st_check "a failing gate: exit 1, e2ee_sent false" bash -c "! \"$real_sim\" selftest-gate --hook 'exit 1' | jq -e '.e2ee_sent' >/dev/null"
    st_check "no gate at all fails closed" bash -c "! \"$real_sim\" selftest-gate >/dev/null"
    st_check "a passing gate sends" bash -c "\"$real_sim\" selftest-gate --hook 'exit 0' | jq -e '.e2ee_sent' >/dev/null"
  else
    echo "  (skipped: $real_sim is not built; run the script once without --skip-build)"
  fi

  echo "[2b] TextEdit cannot make the document (the -1700 a first run hit): INCOMPLETE, never PASS, no audio"
  for mode in error garbage empty outside; do
    st_reset; st_set live live; st_set front ok; st_set te-open "$mode"
    sim_run textedit en
    st_check "[$mode] sim_run aborted" test -n "$ABORT_NOTE"
    st_check "[$mode] no audio was sent and watch-sim was not started" bash -c "[ \"$(st_sent)\" = 0 ] && ! grep -q '^deliver' '$STUB_DIR/sim-calls'"
    FAILS=; finish 9 "te-open-$mode" en 0
    st_check "[$mode] the recorded row is INCOMPLETE, not PASS" bash -c "jq -e 'select(.target==\"te-open-$mode\" and .result==\"INCOMPLETE\")' '$results' >/dev/null"
  done
  st_set te-open ok; st_sweep_te

  echo "[2d] the comparison of TextEdit's windows ignores the document-less panel, nothing else"
  st_reset
  printf '7:doc\n2845:panel\n101:doc\n' > "$STUB_DIR/windows"; before_ids=$(te_document_window_ids)
  printf '101:doc\n7:doc\n2890:panel\n' > "$STUB_DIR/windows"; after_ids=$(te_document_window_ids)
  st_check "(setup) the panel's id changed (2845 to 2890) and the order changed" bash -c "[ '$(osascript -e 'x id of every window')' != '' ]"
  st_check "a changed panel id (and a different order) is not a difference" same_id_set "$after_ids" "$before_ids"
  st_check "…the ids come from the document filter, not from 'id of every window'" bash -c "grep -q 'document of w' '$STUB_DIR/asc.log' && [ \"$before_ids\" = '7 101 ' ]"
  printf '7:doc\n2890:panel\n' > "$STUB_DIR/windows"; after_ids=$(te_document_window_ids)
  st_check "a document window that disappeared IS a difference" st_not same_id_set "$after_ids" "$before_ids"
  printf '7:doc\n101:doc\n55:doc\n' > "$STUB_DIR/windows"; after_ids=$(te_document_window_ids)
  st_check "a document window that appeared IS a difference" st_not same_id_set "$after_ids" "$before_ids"
  printf '7:doc\n102:doc\n' > "$STUB_DIR/windows"; after_ids=$(te_document_window_ids)
  st_check "one document window replaced by another IS a difference" st_not same_id_set "$after_ids" "$before_ids"
  st_check "an empty list is not the same as a non-empty one" st_not same_id_set "" "$before_ids"
  : > "$STUB_DIR/windows"

  echo "[2e] the read-back is kept and compared exactly (no trailing space is invented, none is forgiven)"
  SENT="Please send the quarterly report to the design team before Friday."
  st_check "the identical sentence is the same" same_text "$SENT" "$SENT"
  st_check "a trailing space is a difference (nothing is forgiven)" st_not same_text "$SENT " "$SENT"
  st_check "a trailing newline is a difference" st_not same_text "$SENT
" "$SENT"
  st_check "one different character in the middle is a difference" st_not same_text "Please send the quarterly reprot to the design team before Friday." "$SENT"
  st_check "a leading space is a difference" st_not same_text " $SENT" "$SENT"
  st_check "a doubled space in the middle is a difference" st_not same_text "Please send  the quarterly report to the design team before Friday." "$SENT"
  st_check "case counts" st_not same_text "please send the quarterly report to the design team before friday." "$SENT"
  st_check "empty is not the sentence" st_not same_text "" "$SENT"
  # What the results record: osascript prints a newline after its result; that newline is not part of the text.
  st_reset; SC_REPORT='{"state":"pasted"}'; SC_MS=0; ABORT_NOTE=; FAILS=
  printf '%s\n' "$SENT" > "$SC_FILE"; SC_READBACK=$(cat "$SC_FILE")
  finish 9 text-field-exact en 100
  st_check "the recorded text is exactly the read-back (osascript's newline did not become a space)" bash -c "[ \"\$(jq -r 'select(.target==\"text-field-exact\") | .text' '$results')\" = '$SENT' ]"
  st_check "…and the exact read-back is in the detail" bash -c "[ \"\$(jq -r 'select(.target==\"text-field-exact\") | .detail.readback' '$results')\" = '$SENT' ]"
  st_reset; SC_REPORT='{"state":"pasted"}'; SC_MS=0; ABORT_NOTE=; FAILS=
  printf '%s \n' "$SENT" > "$SC_FILE"; SC_READBACK=$(cat "$SC_FILE")
  finish 9 text-field-with-space en 100
  st_check "a read-back that really has a trailing space shows it (nothing is hidden)" bash -c "[ \"\$(jq -r 'select(.target==\"text-field-with-space\") | .text' '$results')\" = '$SENT ' ]"

  echo "[6b] the real watch-sim, dry run (no network): the real order of the real code, and its exit codes"
  if [ ! -x "$real_sim" ] || [ "$root/scripts/watch-sim/main.swift" -nt "$real_sim" ]; then
    keep_sim=$sim; sim=$real_sim; build_watch_sim; sim=$keep_sim
  fi
  export WATCH_SIM_DRY_RUN=1
  local o rc trace
  # a) every gate passes: gates come before their sends; the two concurrent Enters each have their own gate
  o=$("$real_sim" deliver --audio x --after-focus 'exit 0' --before-action 'exit 0' --enter --enter-twice --undo); rc=$?
  trace=$(jq -r '.trace[]' <<<"$o")
  st_check "all gates pass: exit 0" test "$rc" = 0
  st_check "…every send has its gate first (order of the real code)" st_order_ok "$trace"
  st_check "…three /enter sends, each with a gate" bash -c "[ \"\$(grep -c '^send:/enter' <<<'$trace')\" = 3 ] && [ \"\$(grep -c '^gate:/enter' <<<'$trace')\" = 3 ]"
  st_check "…audio is sent after the after-focus gate, /undo after its gate" bash -c "[ \"\$(grep -c '^send:/e2ee' <<<'$trace')\" = 1 ] && [ \"\$(grep -c '^send:/undo' <<<'$trace')\" = 1 ]"
  # the order check can fail: a trace that sends before its gate is caught
  st_check "(control) the order check catches a send that comes before its gate" st_not st_order_ok "$(printf 'send:/focus\nsend:/e2ee\ngate:after-focus\n')"
  st_check "(control) …and an Enter send without a gate" st_not st_order_ok "$(printf 'gate:after-focus\nsend:/e2ee\nsend:/enter\n')"
  # b) the after-focus gate fails
  o=$("$real_sim" deliver --audio x --after-focus 'exit 1' --before-action 'exit 0' --enter); rc=$?
  trace=$(jq -r '.trace[]' <<<"$o")
  st_check "after-focus gate fails: exit 1" test "$rc" = 1
  st_check "…no audio and no Enter were sent" bash -c "! grep -qE '^send:/(e2ee|enter)' <<<'$trace'"
  # c) no gate given at all
  o=$("$real_sim" deliver --audio x --before-action 'exit 0' --enter); rc=$?
  trace=$(jq -r '.trace[]' <<<"$o")
  st_check "no after-focus gate given: exit 1 (fails closed)" test "$rc" = 1
  st_check "…no audio was sent" bash -c "! grep -q '^send:/e2ee' <<<'$trace'"
  # d) the before-action gate fails: the audio went (its own gate passed), the keys did not
  o=$("$real_sim" deliver --audio x --after-focus 'exit 0' --before-action 'exit 1' --enter --enter-twice --undo); rc=$?
  trace=$(jq -r '.trace[]' <<<"$o")
  st_check "before-action gate fails: no /enter and no /undo were sent" bash -c "! grep -qE '^send:/(enter|undo)' <<<'$trace'"
  st_check "…each action still ran its own gate (1 + 2 + 1)" bash -c "[ \"\$(grep -c '^gate:/' <<<'$trace')\" = 4 ]"
  # e) the standalone action entry needs a gate too
  o=$("$real_sim" action --path /enter --id 11111111-1111-1111-1111-111111111111); rc=$?
  st_check "action with no gate: exit 1" test "$rc" = 1
  st_check "…and nothing was sent" bash -c "! grep -q '^send:' <<<'$(jq -r '.trace[]' <<<"$o")'"
  o=$("$real_sim" action --path /enter --id 11111111-1111-1111-1111-111111111111 --before-action 'exit 1'); rc=$?
  st_check "action with a failing gate: exit 1" test "$rc" = 1
  o=$("$real_sim" action --path /undo --id 11111111-1111-1111-1111-111111111111 --before-action 'exit 0'); rc=$?
  st_check "action with a passing gate: exit 0, gate first then send" bash -c "[ $rc = 0 ] && [ \"\$(jq -r '.trace|join(\" \")' <<<'$o')\" = 'gate:/undo send:/undo' ]"
  "$real_sim" action --path /focus --id 11111111-1111-1111-1111-111111111111 --before-action 'exit 0' >/dev/null 2>&1; rc=$?
  st_check "action only takes /enter or /undo" test "$rc" != 0
  unset WATCH_SIM_DRY_RUN

  echo "[7] the one process in a Terminal window: only that pid, only on that tty"
  st_reset
  owned_bg sleep sleep 120 > /dev/null 2>&1
  pid=$OWNED_PID; sleep 0.5
  entry=$(reg_get pid "$pid")
  reg_del pid "$pid"; reg_add tty-proc "$pid" "/dev/ttys999$US$(proc_start "$pid")$US$(proc_cmd "$pid")"
  owned tty-proc "$pid" act_stop_tty_proc "$pid" /dev/ttys999
  st_check "a process that is not on the registered tty is refused" row_refused
  st_check "…and it is still running" kill -0 "$pid"
  reg_del tty-proc "$pid"; reg_add pid "$pid" "$entry"
  stop_pid "$pid"
  st_check "(test cleanup) the process is stopped" bash -c "! kill -0 $pid 2>/dev/null"

  echo "[8] an app the person quit is not launched again by the cleanup"
  st_reset; st_set running false
  mkdir "$tmp/textedit.gone"; : > "$tmp/textedit.gone/wristper-verify-7.txt"
  reg_add path "$tmp/textedit.gone"; reg_add path "$tmp/textedit.gone/wristper-verify-7.txt"
  reg_add window textedit:202 "$tmp/textedit.gone/wristper-verify-7.txt"
  close_target textedit "202|$tmp/textedit.gone/wristper-verify-7.txt"
  st_check "no window command was sent to the quit TextEdit" bash -c "! grep -q 'tell application \"TextEdit\"' '$STUB_DIR/asc.log'"
  st_check "…its file was still Trashed (it is this run's own)" bash -c "[ ! -e '$tmp/textedit.gone' ]"
  st_check "…and a window that is simply gone is not a refusal (nothing was left to close)" bash -c "[ ! -s '$tmp/row-refused' ]"
  st_check "…its registry entry is dropped" bash -c "! grep -q 'textedit:202' '$tmp/registry.tsv'"
  st_set running true

  echo "[9] the switch-away action is a registered action"
  : > "$STUB_DIR/open.log"
  /bin/bash "$self" --switch-away
  st_check "registered: Finder is brought to the front" grep -q "open -a Finder" "$STUB_DIR/open.log"
  reg_del switch finder
  : > "$STUB_DIR/open.log"
  /bin/bash "$self" --switch-away
  st_check "unregistered: nothing is opened" bash -c "[ ! -s '$STUB_DIR/open.log' ]"
  reg_add switch finder "restored"

  echo "[10] identity: pid, start time, and command, not the pid alone"
  st_reset
  owned_bg sleep sleep 120 > /dev/null 2>&1
  pid=$OWNED_PID; sleep 0.5
  entry=$(reg_get pid "$pid")
  reg_del pid "$pid"; reg_add pid "$pid" "sleep${US}Mon Jan  1 00:00:00 2001"
  stop_pid "$pid"
  st_check "a registered start time that does not match refuses to stop it" row_refused
  st_check "…the process is still running" kill -0 "$pid"
  st_check "…and the registry entry is kept (a failed stop does not forget it)" reg_has pid "$pid"
  st_reset
  reg_del pid "$pid"; reg_add pid "$pid" "no-such-program${US}$(proc_start "$pid")"
  stop_pid "$pid"
  st_check "a command line that no longer holds the marker refuses to stop it" row_refused
  st_check "…the process is still running" kill -0 "$pid"
  reg_del pid "$pid"; reg_add pid "$pid" "$entry"
  stop_pid "$pid"
  st_check "(test cleanup) stopped with the right identity" bash -c "! kill -0 $pid 2>/dev/null"
  # A group whose leader has exited but still has a member: it cannot be proven ours, so nothing is killed.
  st_reset
  owned_bg sh sh -c 'sleep 120 & sleep 1' > /dev/null 2>&1
  pid=$OWNED_PID; sleep 2.5
  st_check "(setup) the leader is gone and a member remains" bash -c "! kill -0 $pid 2>/dev/null && ps -axo pgid= | grep -qx ' *$pid'"
  stop_pid "$pid"
  st_check "a group with a vanished leader is refused, not killed" row_refused
  st_check "…its member is still there" bash -c "ps -axo pgid= | grep -qx ' *$pid'"
  st_check "…and the registry keeps the entry" reg_has pid "$pid"
  kill -TERM -- "-$pid" 2>/dev/null   # the test's own straggler
  reg_del pid "$pid"

  echo "[11] Wristper Test: hex quit is not sent before the pid is proven"
  st_reset
  hex="$tmp/fake-hex"
  owned_bg sleep sleep 120 > /dev/null 2>&1
  pid=$OWNED_PID; sleep 0.5
  # The stand-in app reports THIS pid as its own, so `hex quit` would be sent if the identity check were missing.
  reg_add path "$tmp/fake-hex"
  printf '#!/bin/sh\nif [ "$1" = status ]; then echo "{\\"pid\\": %s}"; else echo "$*" >> "%s/hex.log"; fi\n' "$pid" "$STUB_DIR" > "$tmp/fake-hex"; chmod +x "$tmp/fake-hex"
  entry=$(reg_get pid "$pid"); reg_del pid "$pid"
  reg_add wt "$pid" "$hex${US}$(proc_start "$pid")"   # a live pid whose command is not the app's
  : > "$STUB_DIR/hex.log"
  owned wt "$pid" act_quit_wt "$pid"
  st_check "a pid that does not run the app's executable is refused" row_refused
  st_check "…no 'quit' was sent to the running service" bash -c "! grep -q quit '$STUB_DIR/hex.log'"
  st_check "…and the process is untouched" kill -0 "$pid"
  reg_del wt "$pid"; reg_add pid "$pid" "$entry"
  stop_pid "$pid"

  echo "[12] a Terminal window without a proven process is left open; a failed Trash is reported"
  st_reset; st_set live live
  reg_add window terminal:303 /dev/ttys303
  : > "$STUB_DIR/asc.log"
  close_target terminal "303|/dev/ttys303"
  st_check "closing a window with no registered process is refused" row_refused
  st_check "…and no close command was sent" bash -c "! grep -q 'close window' '$STUB_DIR/asc.log'"
  reg_del window terminal:303   # the test's own fixture
  st_reset
  : > "$tmp/stuck"; reg_add path "$tmp/stuck"
  printf '#!/bin/sh\nexit 1\n' > "$tmp/failing-trash"; chmod +x "$tmp/failing-trash"; reg_add path "$tmp/failing-trash"
  keep_trash=$TRASH; TRASH="$tmp/failing-trash"
  trash_own "$tmp/stuck"
  TRASH=$keep_trash
  st_check "a Trash that fails is refused and reported" row_refused
  st_check "…and the file is still there" test -e "$tmp/stuck"
  st_reset; trash_own "$tmp/stuck"

  echo "[13] a signal cannot cut the cleanup short"
  st_check "teardown masks INT, TERM, and HUP" bash -c "$(declare -f mask_signals); mask_signals; trap -p | grep -c SIG | grep -qx 3"

  echo "[13b] the cleanup keeps what it cannot prove: a window still open, a failing scan, a te-keep mark (file, folder, and root)"
  for flag in noclose scan-fail keep-mark; do
    st_sub "td-$flag"; sub=$SUB; f="$sub/textedit.BBBBBB/wristper-verify-9.txt"
    mkdir "$sub/textedit.BBBBBB"; : > "$f"
    printf 'path\t%s\t\npath\t%s\t\nte-before\t%s\t7 \nwindow\ttextedit:101\t%s\n' "$sub/textedit.BBBBBB" "$f" "$f" "$f" >> "$sub/registry.tsv"
    st_reset; st_set live live; st_set name original; st_set te-path "$f"; printf '101|%s\n' "$f" > "$STUB_DIR/scan"
    case $flag in noclose|scan-fail) [ $flag = noclose ] && touch "$STUB_DIR/noclose" || touch "$STUB_DIR/scan-fail" ;; keep-mark) printf 'te-keep\t%s\twhy not\n' "$f" >> "$sub/registry.tsv" ;; esac
    st_run_sub teardown
    rm -f "$STUB_DIR/noclose" "$STUB_DIR/scan-fail"
    st_check "[$flag] the file is still there after the whole teardown" test -f "$f"
    st_check "[$flag] …its folder too" test -d "$sub/textedit.BBBBBB"
    st_check "[$flag] …and the root" test -d "$sub"
    st_run_sub cleanup_root
    st_check "[$flag] a second cleanup of the root keeps them as well" test -f "$f"
    "$TRASH" "$sub"
  done
  st_sub td-clean; sub=$SUB; f="$sub/textedit.CCCCCC/wristper-verify-9.txt"
  mkdir "$sub/textedit.CCCCCC"; : > "$f"
  printf 'path\t%s\t\npath\t%s\t\nte-before\t%s\t7 \nwindow\ttextedit:101\t%s\n' "$sub/textedit.CCCCCC" "$f" "$f" "$f" >> "$sub/registry.tsv"
  st_reset; st_set live live; st_set name original; st_set te-path "$f"; printf '101|%s\n' "$f" > "$STUB_DIR/scan"
  st_run_sub teardown
  st_check "(control) with nothing unproven the teardown Trashes the file, the folder, and the root" bash -c "[ ! -e '$f' ] && [ ! -e '$sub' ]"
  [ -e "$sub" ] && "$TRASH" "$sub"

  echo "[14] the clipboard cannot be put back: its backup stays, the other secrets go, the gate refuses nothing"
  st_sub clip; sub=$SUB
  mkdir "$sub/clipboard" "$sub/watch-sim-state"; echo '{"items":[]}' > "$sub/clipboard/manifest.json"; echo "private key" > "$sub/watch-sim-state/watch-state.json"
  printf '#!/bin/sh\nexit 1\n' > "$sub/failing-pbtool"; chmod +x "$sub/failing-pbtool"
  printf 'tree\t%s\t-\ntree\t%s\t*\npath\t%s\t\n' "$sub/clipboard" "$sub/watch-sim-state" "$sub/failing-pbtool" >> "$sub/registry.tsv"
  : > "$out/refusals.tsv"
  st_clip_teardown() { pbtool="$SUB/failing-pbtool"; CLIP_STATE=saved; teardown; echo "$CLIP_STATE" > "$STUB_DIR/clip-state"; }
  st_run_sub st_clip_teardown
  st_check "the restore is reported as failed" bash -c "[ \"\$(cat '$STUB_DIR/clip-state')\" = failed ]"
  st_check "the clipboard backup is still on disk" test -f "$sub/clipboard/manifest.json"
  st_check "the pairing keys were moved to the Trash" bash -c "[ ! -e '$sub/watch-sim-state' ]"
  st_check "the cleanup was not refused by the gate" bash -c "[ ! -s '$out/refusals.tsv' ]"
  "$TRASH" "$sub"

  echo "[15] the temp folder of the selftest itself: everything it made was registered, so the cleanup removes it all"
  st_sweep_te
  teardown
  st_check "(test cleanup) the selftest's own temp folder is gone, nothing of it is left behind" bash -c "[ ! -e '$tmp' ]"

  print_table
  echo
  echo "selftest: $ST_PASS passed, $ST_FAIL failed"
  [ "$ST_FAIL" = 0 ]
}

# =============================================================================
case "$mode" in
  check-textedit) check_textedit; exit $? ;;
  check-terminal) check_terminal; exit $? ;;
  front-check) front_check "$fc_kind" "$fc_handle"; exit $? ;;
  switch-away) tmp=${WATCH_E2E_TMP:-}; out=${WATCH_E2E_OUT:-}; [ -f "$tmp/registry.tsv" ] || exit 1
               owned switch finder act_switch_away; exit $? ;;
  selftest) selftest_main; exit $? ;;
  *) main ;;
esac
