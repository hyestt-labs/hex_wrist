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
  echo "          [--keep-running] [--selftest]"
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
    -h|--help) sed -n 2,36p "$0"; echo; usage; exit 0 ;;
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
unset HEX_APPLICATION_SUPPORT_DIR HEX_WATCH_RELAY_BASE

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
#   window    key = textedit:<window id> | terminal:<window id> | chrome:<pid>, extra = document name | tty | debug port
#   tty-proc  key = pid of the one process we run in a Terminal window, extra = that window's tty
#   path      key = a path under $tmp
#   testdata  key = one allowlisted file in Wristper Test's data folder (relay room reset)
#   wt        key = pid of the Wristper Test this run started, extra = its executable
#   sim       key = udid of the simulator this run created, extra = its name
#   ran       key = textedit | terminal | chrome, extra = running|not-running at the start (written to the log only; it decides nothing)

reg_add() { printf '%s\t%s\t%s\n' "$1" "$2" "${3:-}" >> "$tmp/registry.tsv"; }
reg_has() { [ -f "$tmp/registry.tsv" ] && awk -F'\t' -v k="$1" -v v="$2" '$1==k && $2==v {f=1} END{exit !f}' "$tmp/registry.tsv"; }
reg_get() { [ -f "$tmp/registry.tsv" ] && awk -F'\t' -v k="$1" -v v="$2" '$1==k && $2==v {x=$3} END{print x}' "$tmp/registry.tsv"; }
reg_list() { [ -f "$tmp/registry.tsv" ] && awk -F'\t' -v k="$1" '$1==k {print $2 "\t" $3}' "$tmp/registry.tsv"; return 0; }
reg_del() {
  [ -f "$tmp/registry.tsv" ] || return 0
  awk -F'\t' -v k="$1" -v v="$2" '!($1==k && $2==v)' "$tmp/registry.tsv" > "$tmp/registry.new" && mv -f "$tmp/registry.new" "$tmp/registry.tsv"
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
  perl -e 'use POSIX qw(setsid); setsid() or die "setsid: $!"; exec { $ARGV[0] } @ARGV or die "exec: $!"' "$@" &
  OWNED_PID=$!
  reg_add pid "$OWNED_PID" "$marker"
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
act_stop_group() {
  local g=$1 marker own i
  [[ $g =~ ^[0-9]+$ ]] && [ "$g" -gt 1 ] || { log "not stopping a group with the id '$g'"; return 1; }
  own=$(ps -o pgid= -p $$ | tr -d ' ')
  [ "$g" != "$own" ] || { log "not stopping this script's own process group"; return 1; }
  marker=$(reg_get pid "$g")
  if kill -0 "$g" 2>/dev/null && [[ $(ps -p "$g" -o command= 2>/dev/null) != *"$marker"* ]]; then
    log "pid $g no longer runs $marker (the number was reused); not stopping it"
    return 1
  fi
  group_alive "$g" || return 0
  kill -TERM -- "-$g" 2>/dev/null
  for i in $(seq 1 40); do group_alive "$g" || return 0; sleep 0.25; done
  kill -KILL -- "-$g" 2>/dev/null
  for i in $(seq 1 20); do group_alive "$g" || return 0; sleep 0.25; done
  log "process group $g would not stop"
  return 1
}
stop_pid() { owned pid "$1" act_stop_group "$1"; local rc=$?; reg_del pid "$1"; return $rc; }
reap_all() { # every registered process group
  local pid rest
  while IFS=$'\t' read -r pid rest; do [ -n "$pid" ] && stop_pid "$pid"; done < <(reg_list pid)
  return 0
}

# --- paths: only under the temp root, never through a link ------------------------------
# act_trash_path PATH: Trash PATH only when its real location is inside $tmp.
act_trash_path() {
  local p=$1 real
  [ -e "$p" ] || [ -L "$p" ] || return 0
  # A link is judged by where the link itself sits (Trash moves the link, never its target); anything
  # else, and any path that goes THROUGH a link, by its real location.
  if [ -L "$p" ]; then real="$(real_path "$(dirname -- "$p")")/$(basename -- "$p")"; else real=$(real_path "$p"); fi
  case "$real" in
    "$tmp_real"|"$tmp_real"/*) /usr/bin/trash "$p" >/dev/null 2>&1 ;;
    *) refuse path "$p" "its real path $real is outside the temp root $tmp_real"; return 99 ;;
  esac
}
trash_own() { owned path "$tmp" act_trash_path "$1"; }   # the registered root is the key; the act checks the real path is inside it
# The one thing Trashed outside $tmp: four files of Wristper Test's own data folder (its relay room),
# on the allowlist, only while this run's Wristper Test is stopped.
act_trash_testdata() {
  local p=$1 dir name
  dir=$(real_path "$(dirname -- "$p")"); name=$(basename -- "$p")
  [ "$dir" = "$(real_path "$TEST_DATA")" ] && [ "$(basename -- "$dir")" = "Wristper Test" ] || { refuse testdata "$p" "not inside Wristper Test's data folder"; return 99; }
  case "$name" in relay-room|relay-secret|paired-devices.json|delivered-ids.json) ;; *) refuse testdata "$p" "not on the allowlist"; return 99 ;; esac
  wt_running && { refuse testdata "$p" "Wristper Test is running"; return 99; }
  [ -e "$p" ] && /usr/bin/trash "$p" >/dev/null 2>&1
  return 0
}

# --- Wristper Test: only the one this run started ----------------------------------------
status_json() { "$hex" status --json 2>/dev/null; }
# Pids whose whole command line is exactly the app's executable (hex status, hex watch pair, … carry arguments).
wt_pids() { ps -axo pid=,command= | while read -r pid cmd; do [ "$cmd" = "$hex" ] && echo "$pid"; done; }
wt_running() { status_json >/dev/null || [ -n "$(wt_pids)" ]; }
act_quit_wt() { # pid
  local pid=$1 i
  "$hex" quit >/dev/null 2>&1
  for i in $(seq 1 40); do kill -0 "$pid" 2>/dev/null || return 0; sleep 0.25; done
  [ "$(ps -p "$pid" -o command= 2>/dev/null)" = "$hex" ] || return 0
  kill -TERM "$pid" 2>/dev/null
  for i in $(seq 1 20); do kill -0 "$pid" 2>/dev/null || return 0; sleep 0.25; done
  kill -KILL "$pid" 2>/dev/null
  return 0
}
stop_wristper_test() {
  local pid rest
  while IFS=$'\t' read -r pid rest; do
    [ -n "$pid" ] || continue
    owned wt "$pid" act_quit_wt "$pid"
    reg_del wt "$pid"
  done < <(reg_list wt)
  return 0
}

# --- simulator: only the one this run created ------------------------------------------
simctl_own() { # simctl-subcommand args... : on the simulator this run made, nothing else
  [ -n "${sim_udid:-}" ] || return 98
  owned sim "$sim_udid" xcrun simctl "$@"
}
delete_sim() {
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
# Handles:  textedit "<window id>|<document name>"   terminal "<window id>|<tty>"
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
# Live identity: the window still exists and is the one registered.
win_live() { # kind handle
  local kind=$1 h=$2 id got
  [ "$(reg_get window "$(win_key "$kind" "$h")")" = "$(win_extra "$kind" "$h")" ] || return 1
  case "$kind" in
    textedit)
      id=${h%%|*}
      got=$(osascript -e 'tell application "TextEdit"' -e "if not (exists window id $id) then return \"\"" \
        -e "return name of document of window id $id" -e 'end tell' 2>/dev/null)
      [ -n "$got" ] && [ "$got" = "${h#*|}" ] ;;
    terminal)
      id=${h%%|*}
      got=$(osascript -e 'tell application "Terminal"' -e "if not (exists window id $id) then return \"\"" \
        -e "return tty of selected tab of window id $id" -e 'end tell' 2>/dev/null)
      [ -n "$got" ] && [ "$got" = "${h#*|}" ] ;;
    chrome) chrome_ready "$(win_extra chrome "$h")" ;;
  esac
}

# front_check KIND HANDLE: is the app in front this run's target window? lsappinfo needs no permission.
front_check() {
  local kind=$1 h=$2 asn bid pid
  asn=$(lsappinfo front 2>/dev/null); [ -n "$asn" ] || return 1
  bid=$(lsappinfo info -only bundleid "$asn" 2>/dev/null | sed -nE 's/.*bundleID="([^"]*)".*/\1/p')
  pid=$(lsappinfo info -only pid "$asn" 2>/dev/null | sed -nE 's/.*pid = ([0-9]+).*/\1/p')
  case "$kind" in
    textedit)
      [ "$bid" = com.apple.TextEdit ] || return 1
      [ "$(osascript -e 'tell application "TextEdit" to return ((id of front window) as text) & "|" & (name of document of front window)' 2>/dev/null)" = "$h" ] ;;
    terminal)
      [ "$bid" = com.apple.Terminal ] || return 1
      [ "$(osascript -e 'tell application "Terminal" to return ((id of front window) as text) & "|" & (tty of selected tab of front window)' 2>/dev/null)" = "$h" ] ;;
    chrome) [ -n "$pid" ] && [ "$pid" = "${h%%:*}" ] ;;
    *) return 1 ;;
  esac
}
# The gate command watch-sim runs between /focus and /e2ee, and before /enter and /undo.
front_hook() { printf '%q ' /bin/bash "$self" --front-check "$1" "$2"; }

act_focus() { # kind handle
  win_live "$1" "$2" || return 1
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
  win_live "$1" "$2" || return 1
  case "$1" in
    textedit) osascript -e "tell application \"TextEdit\" to return text of document of window id ${2%%|*}" ;;
    terminal) osascript -e "tell application \"Terminal\" to return contents of selected tab of window id ${2%%|*}" ;;
    chrome) chrome_page "$(win_extra chrome "$2")" ;;
  esac
}
# act_stop_tty_proc PID TTY: HUP the one process this run started in its window (never the tty).
act_stop_tty_proc() {
  local pid=$1 tty=$2 i seen
  seen=$(ps -o tty= -p "$pid" 2>/dev/null | tr -d ' ')
  [ -n "$seen" ] || return 0
  [ "/dev/$seen" = "$tty" ] || [ "$seen" = "${tty#/dev/}" ] || { refuse tty-proc "$pid" "it is on $seen now, not $tty"; return 99; }
  kill -HUP "$pid" 2>/dev/null
  for i in $(seq 1 20); do kill -0 "$pid" 2>/dev/null || return 0; sleep 0.25; done
  seen=$(ps -o tty= -p "$pid" 2>/dev/null | tr -d ' ')
  { [ "/dev/$seen" = "$tty" ] || [ "$seen" = "${tty#/dev/}" ]; } && kill -KILL "$pid" 2>/dev/null
  return 0
}
act_close() { # kind handle
  local kind=$1 h=$2 pid rest tty
  win_live "$kind" "$h" || return 1
  case "$kind" in
    textedit) osascript -e "tell application \"TextEdit\" to close window id ${h%%|*} saving no" >/dev/null 2>&1 ;;
    terminal)
      tty=${h#*|}
      while IFS=$'\t' read -r pid rest; do
        [ "$rest" = "$tty" ] || continue
        owned tty-proc "$pid" act_stop_tty_proc "$pid" "$tty"
        reg_del tty-proc "$pid"
      done < <(reg_list tty-proc)
      sleep 1
      osascript -e "tell application \"Terminal\" to close window id ${h%%|*}" >/dev/null 2>&1 ;;
    chrome) chrome_close "$h" ;;
  esac
  reg_del window "$(win_key "$kind" "$h")"
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

# TextEdit: a new document, found again by its window id.
te_open() {
  osascript -e 'tell application "TextEdit"' -e 'activate' -e 'set d to make new document' \
    -e 'set w to first window whose document is d' -e 'return ((id of w) as text) & "|" & (name of d)' -e 'end tell'
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
  reg_add path "$dir"; reg_add path "$dir.log"
  port=$(python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1",0)); print(s.getsockname()[1])')
  # --use-mock-keychain and basic password store: a fresh profile must not touch the login Keychain.
  owned_bg "Google Chrome" "$CHROME_BIN" --user-data-dir="$dir" --remote-debugging-port="$port" --no-first-run \
    --no-default-browser-check --use-mock-keychain --password-store=basic --disable-extensions --disable-sync \
    --window-size=900,700 --new-window "$CHROME_URL" > "$dir.log" 2>&1
  pid=$OWNED_PID
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
  trash_own "$dir"; trash_own "$dir.log"
  reg_del window "chrome:$pid"
}

# Prints a handle for the opened target and registers what was made. open_target runs in a subshell,
# which is why the registry is a file.
open_target() {
  local h= tty pf pid seen
  case "$1" in
    textedit)
      h=$(te_open) || return 1
      [ -n "$h" ] || return 1
      reg_add window "textedit:${h%%|*}" "${h#*|}" ;;
    terminal|shell|recorder|claude|codex)
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
        reg_add tty-proc "$pid" "$tty"
      else
        log "could not prove which process runs in $tty; it will be left running (its window is still closed)"
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
SC_REPORT=null; SC_READBACK=; SC_MS=0; SC_HANDLE=; SC_TARGET=; SC_PERCENT=0; SC_SWITCH=0

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
    "$(tr '\r\n' '  ' <"$SC_FILE" | tail -c 200)" "$(jq -c --arg f "$note" '. + {fails:$f}' <<<"$SC_REPORT")"
  FAILS=; BLOCKED_NOTE=; ABORT_NOTE=; SC_SWITCH=0
}

# =============================================================================
# Teardown: everything this run started, found in the registry
# =============================================================================
CLIP_STATE=none   # none | saved | restored | failed
CLIP_DETAIL=
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

keep_setting_original=
TORN_DOWN=0
teardown() {
  [ "$TORN_DOWN" = 0 ] || return 0
  TORN_DOWN=1
  [ -n "$tmp" ] || return 0
  local key extra kind id name entry f
  # 1. processes, each as a whole group, until nothing of the group is left
  reap_all
  # 2. TextEdit documents, Terminal windows (and the one process we ran in each), Chrome windows
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
  # 5. the temp root. If the clipboard could not be put back, its backup stays (everything else goes).
  if [ "$CLIP_STATE" = failed ] || [ "$CLIP_STATE" = saved ]; then
    for entry in "$tmp"/* "$tmp"/.[!.]*; do
      [ -e "$entry" ] || continue
      # The backup stays. The registry and the row marker stay too: the gate still needs them to run, and
      # they hold no secret.
      case "$entry" in "$tmp/clipboard"|"$tmp/registry.tsv"|"$tmp/registry.new"|"$tmp/row-refused") continue ;; esac
      trash_own "$entry"
    done
    log "YOUR CLIPBOARD WAS NOT PUT BACK. The backup is kept at $tmp/clipboard. Restore it with: \"$pbtool\" restore \"$tmp/clipboard\""
    echo "YOUR CLIPBOARD WAS NOT PUT BACK. The backup is kept at $tmp/clipboard" >&2
    echo "Restore it with: \"$pbtool\" restore \"$tmp/clipboard\"" >&2
  else
    trash_own "$tmp"
  fi
  return 0
}
cleanup() {
  trap - EXIT
  teardown
}

init() { # prefix
  out="$root/target/watch-e2e/${1}$(date +%Y%m%d-%H%M%S)"
  mkdir -p "$out"
  results="$out/results.jsonl"
  : > "$results"
  run_id=$(basename "$out")
  # A simulator is made fresh for this run and deleted at its end. It is never reused or erased.
  sim_name="Wristper E2E $run_id"
  # Secrets live only here: the clipboard backup, the watch's pairing keys, raw logs that hold pairing
  # codes, the registry, Chrome's profile. Mode 0700, moved to the Trash at exit.
  tmp=$(mktemp -d "${TMPDIR:-/tmp}/wristper-e2e.XXXXXX") || { echo "cannot make a temp folder" >&2; exit 2; }
  tmp_real=$(real_path "$tmp")
  : > "$tmp/registry.tsv"; : > "$out/refusals.tsv"; : > "$tmp/row-refused"
  reg_add path "$tmp" root
  SC_FILE="$out/readback.txt"
  trap cleanup EXIT
  # A signal must still run the cleanup (it stops what this run started).
  trap 'exit 130' INT
  trap 'exit 143' TERM HUP
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
  local f old_room=
  case "$TEST_DATA" in */"Wristper Test") ;; *) die "refusing to reset a data folder that is not Wristper Test: $TEST_DATA" 2 ;; esac
  [ "$TEST_DATA" != "$OLD_DATA" ] || die "refusing to reset the installed app's data folder" 2
  [ ! -s "$TEST_DATA/relay-room" ] || old_room=$(cat "$TEST_DATA/relay-room")
  printf %s "$old_room" > "$tmp/old-room"
  for f in relay-room relay-secret paired-devices.json delivered-ids.json; do
    reg_add testdata "$TEST_DATA/$f" allowlist
    owned testdata "$TEST_DATA/$f" act_trash_testdata "$TEST_DATA/$f"
  done
}

launch_wristper_test() {
  local pid
  log "launching Wristper Test in the background"
  open -g "$app" || die "could not open $app"
  for _ in $(seq 1 60); do status_json >/dev/null && break; sleep 1; done
  status_json >/dev/null || die "Wristper Test did not answer hex status within 60 s"
  for _ in $(seq 1 30); do [ -s "$TEST_DATA/relay-room" ] && break; sleep 1; done
  [ -s "$TEST_DATA/relay-room" ] || die "no relay room in $TEST_DATA; the watch service did not start (see $TEST_DATA/logs/process.log)"
  # Its pid from its own status, and only if that pid runs exactly the app's executable.
  pid=$(status_json | jq -r .pid)
  [ "$(ps -p "$pid" -o command= 2>/dev/null)" = "$hex" ] \
    || die "Wristper Test started, but its process could not be identified; it was not registered, so this script will not stop it. Quit it with: \"$hex\" quit"
  reg_add wt "$pid" "$hex"
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
  fresh_room
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
    for t in TextEdit Terminal; do probe_automation "$t"; done
    if stage_on 4 || [ "$chrome_wanted" = 1 ]; then probe_runner_accessibility; fi
  fi
  [ ${#blocked[@]} -eq 0 ] || print_blocked
  log "preflight: permissions ready"
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
  reg_add path "$tmp/watch-sim-state"
  export WATCH_SIM_STATE="$tmp/watch-sim-state"
  local raw="$tmp/stage$1-pair.jsonl"
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
  win_live "$kind" "$h" || return 98
  before=$(front_hook "$kind" "$h")
  after=$before
  # The app-switch scenario: verify the target is in front, then move away on purpose, so the audio meets
  # the first gate with another app in front and must be withheld.
  [ "$SC_SWITCH" = 1 ] && after="$before && open -a Finder && sleep 2"
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
    set_keep on; "$pbtool" set "$sentinel"
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
    set_keep on; "$pbtool" set "$sentinel"
    sim_run textedit en
    sleep 5; board=$("$pbtool" get)
    sim_close textedit
    judged en
    chk "$(is [ "$(judge en "$board")" -ge "$EN_PASS" ])" "clipboard is '${board:0:60}', want the dictation"
    finish 3 clipboard-keep-paste en "$SC_PERCENT"
  fi

  if want clipboard-default-restore; then
    # keep_on_clipboard off (the default): the clipboard comes back after the paste.
    set_keep off; "$pbtool" set "$sentinel"
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
  win_live "$1" "$2" || return 1
  front_check "$1" "$2" || return 1
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
        "$(tr '\r\n' '  ' <<<"$readback" | tail -c 200)" "$(jq -c --arg f "${FAILS#; }" '. + {fails:$f}' <<<"$detail")"
      FAILS=
    done
  done
}

archive_stage2_log() { [ -s "$tmp/stage2-watch.log" ] && redact_pairing < "$tmp/stage2-watch.log" > "$out/stage2-watch.log"; return 0; }

# =============================================================================
# Stage 4: the B2 live tests (opt in: --with-live)
# =============================================================================
# These Rust tests (src/watch/live_tests.rs) close the FRONT TextEdit document and send `exit` to the
# FRONT Terminal window. This script cannot make them act on only what they made, so it refuses
# unless TextEdit has no document open and Terminal has no window open.
stage4() {
  local log4="$out/stage4-live.log" name res rc docs wins
  row_begin
  docs=0; wins=0
  app_running TextEdit && docs=$(osascript -e 'tell application "TextEdit" to count documents' 2>/dev/null)
  app_running Terminal && wins=$(osascript -e 'tell application "Terminal" to count windows' 2>/dev/null)
  if [ "${docs:-0}" != 0 ] || [ "${wins:-0}" != 0 ]; then
    record 4 live - INCOMPLETE 0 refused 0 "" "$(jq -cn --arg f "refused: the live tests close the FRONT TextEdit document and the FRONT Terminal window, and you have $docs document(s) and $wins window(s) open. Close them yourself, or leave this stage out." '{fails:$f}')"
    return 0
  fi
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
  # Safety: the test build's data root is its own, and nothing below names the installed app's folder.
  [ "$TEST_DATA" != "$OLD_DATA" ] || die "test data root equals the installed app's" 2

  # Which apps were running at the start: written to the log only. It decides nothing: this run closes only
  # windows it made and quits only the one Wristper Test it started.
  for t in TextEdit Terminal "Google Chrome"; do
    if app_running "$t"; then reg_add ran "$t" running; else reg_add ran "$t" not-running; fi
  done
  log "apps running before this run (information only): $(reg_list ran | tr '\t\n' ': ' | sed 's/:$//')"

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
  if { [ "$build" = 1 ] || [ ! -x "$sim" ] || [ "$(cat "$(dirname "$sim")/built-from-commit" 2>/dev/null)" != "$WRISTPER_SRC_COMMIT" ]; } &&
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
  *"front window"*)
    read -r w rest < "$STUB_DIR/front"; [ -z "$rest" ] || echo "$rest" > "$STUB_DIR/front"
    [ "$w" = ok ] && echo "101|Untitled 9" || echo "999|Elsewhere"; exit 0 ;;
  *"make new document"*) echo "101|Untitled 9" ;;
  *"name of document of window id 101"*) [ "$(cat "$STUB_DIR/live")" = live ] && echo "Untitled 9" || echo "" ;;
  *"is running"*) echo false ;;
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
  chmod +x "$tmp/bin/osascript" "$tmp/bin/lsappinfo" "$tmp/fake-sim"
}
st_set() { printf '%s\n' "$2" > "$STUB_DIR/$1"; }
st_not() { ! "$@"; }
st_sent() { [ -f "$STUB_DIR/sim-calls" ] && grep -c '^e2ee-sent' "$STUB_DIR/sim-calls"; return 0; }
st_reset() { : > "$STUB_DIR/sim-calls"; : > "$STUB_DIR/asc.log"; : > "$tmp/row-refused"; ABORT_NOTE=; }

selftest_main() {
  init "selftest-"
  STUB_DIR="$tmp/stub"; mkdir -p "$STUB_DIR"
  export STUB_DIR
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
  ln -s /etc "$tmp/dirlink"
  trash_own "$tmp/dirlink/hosts"
  st_check "a path that goes through a link out of the temp root is refused" row_refused
  st_check "…and /etc/hosts is still there" test -f /etc/hosts
  st_reset
  trash_own "$tmp/dirlink"
  st_check "a link inside the temp root is Trashed as a link (never its target)" bash -c "[ ! -L '$tmp/dirlink' ] && [ -f /etc/hosts ] && [ ! -s '$tmp/row-refused' ]"
  : > "$tmp/victim"; reg_add path "$tmp/victim"
  st_reset
  trash_own "$tmp/victim"
  st_check "a registered file inside the temp root is Trashed" bash -c "[ ! -e '$tmp/victim' ] && [ ! -s '$tmp/row-refused' ]"
  st_reset
  owned_bg sleep sleep 120 > /dev/null 2>&1
  pid=$OWNED_PID
  reg_del pid "$pid"
  owned pid "$pid" act_stop_group "$pid"
  st_check "stopping an unregistered process group is refused" row_refused
  st_check "…and the process is still running" kill -0 "$pid"
  reg_add pid "$pid" sleep
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

  echo "[7] the clipboard cannot be put back: its backup stays, the other secrets go, the gate refuses nothing"
  mkdir -p "$tmp/clipboard"; echo '{"items":[]}' > "$tmp/clipboard/manifest.json"
  mkdir -p "$tmp/watch-sim-state"; echo "private key" > "$tmp/watch-sim-state/watch-state.json"
  printf '#!/bin/sh\nexit 1\n' > "$tmp/failing-pbtool"; chmod +x "$tmp/failing-pbtool"
  pbtool="$tmp/failing-pbtool"; CLIP_STATE=saved
  : > "$out/refusals.tsv"
  teardown
  st_check "the restore is reported as failed" test "$CLIP_STATE" = failed
  st_check "the clipboard backup is still on disk" test -f "$tmp/clipboard/manifest.json"
  st_check "the pairing keys were moved to the Trash" bash -c "[ ! -e '$tmp/watch-sim-state' ]"
  st_check "the cleanup was not refused by the gate" bash -c "[ ! -s '$out/refusals.tsv' ]"
  # Leave nothing behind: the kept backup is part of this test, so Trash the folder now.
  trash_own "$tmp"
  st_check "(test cleanup) the temp folder is gone" bash -c "[ ! -e '$tmp' ]"

  print_table
  echo
  echo "selftest: $ST_PASS passed, $ST_FAIL failed"
  [ "$ST_FAIL" = 0 ]
}

# =============================================================================
case "$mode" in
  front-check) front_check "$fc_kind" "$fc_handle"; exit $? ;;
  selftest) selftest_main; exit $? ;;
  *) main ;;
esac
