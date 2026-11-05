#!/bin/bash
# One command that proves Wristper Test.app takes Apple Watch dictations over
# the production relay and pastes them into real apps, with no human in the
# loop once the one-time permissions are granted.
#
#   stage 1  watch-sim (the shipping watch protocol code as a macOS CLI):
#            pairing, then English and Mandarin into TextEdit and Terminal
#   stage 2  the real watch app (DEBUG build) in a watchOS Simulator, same cases
#   stage 3  scenarios no unit test covers: keep_on_clipboard with an app
#            switch, Enter and Undo (TextEdit, Terminal cat, a plain shell),
#            a double Enter, a Chromium text field, and with --with-agents
#            interactive claude and codex
#   stage 4  the B2 live tests (cargo nextest, #[ignore] watch_live)
#
# `--all` (the default) runs every stage and ends with one PASS/FAIL table.
#
# Exit status: 0 every case passed, 1 a case failed, 2 bad usage or a tool is
# missing, 3 blocked on a one-time permission or a locked screen (the output
# lists each click). The script never tries to grant a permission itself.
#
# Never touches /Applications/Wristper.app, its data folder
# (~/Library/Application Support/Wristper), its relay room, or its Keychain
# items: everything here runs under Wristper Test (dev.wristper.mac.test).
set -uo pipefail

root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
stage=all
build=1
targets=textedit,terminal
langs=en,zh
keep=0
with_agents=0
preflight_only=0
only=
usage() {
  echo "usage: $0 [--all] [--stage 1|2|3|4|all] [--skip-build] [--targets textedit,terminal] [--langs en,zh]"
  echo "          [--only <regex on scenario names>] [--with-agents] [--preflight-only] [--keep-running]"
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
    --preflight-only) preflight_only=1; shift ;;
    --keep-running) keep=1; shift ;;
    -h|--help) sed -n 2,24p "$0"; echo; usage; exit 0 ;;
    *) echo "unknown argument: $1" >&2; usage >&2; exit 2 ;;
  esac
done
case "$stage" in 1|2|3|4|all) ;; *) echo "--stage must be 1, 2, 3, 4, or all" >&2; exit 2 ;; esac
stage_on() { [ "$stage" = all ] || [ "$stage" = "$1" ]; }

export DEVELOPER_DIR=${DEVELOPER_DIR:-/Applications/Xcode.app/Contents/Developer}
[ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"
unset HEX_APPLICATION_SUPPORT_DIR HEX_WATCH_RELAY_BASE

WRISTPER_SRC=${WRISTPER_SRC:-/Users/glenhsu/orca/workspaces/wristper/coding}
WRISTPER_HOOKS_SRC=${WRISTPER_HOOKS_SRC:-/Users/glenhsu/orca/workspaces/wristper/sim-hooks}
# Links the Rust test binary on this Mac (Xcode 26: -lswiftCompatibility56). Optional.
WATCH_E2E_B2_ENV=${WATCH_E2E_B2_ENV:-/Users/glenhsu/orca/workspaces/hex_wrist/migration_to_hex/artifacts/wristper-migration-20261104/b2-test-env/env.sh}
SIGN_IDENTITY=${SIGN_IDENTITY:-Developer ID Application: WEI CHIEH HSU (VQY3J86U79)}
SIM_NAME=${WATCH_E2E_SIM_NAME:-Wristper E2E}
SIM_TYPE=${WATCH_E2E_SIM_TYPE:-com.apple.CoreSimulator.SimDeviceType.Apple-Watch-Series-11-46mm}
WATCH_BUNDLE=dev.wristper.watch.watchkitapp
TEST_BUNDLE=dev.wristper.mac.test
TEST_DATA="$HOME/Library/Application Support/Wristper Test"
OLD_DATA="$HOME/Library/Application Support/Wristper"
EN_TEXT=${WATCH_E2E_EN_TEXT:-Please send the quarterly report to the design team before Friday.}
ZH_TEXT=${WATCH_E2E_ZH_TEXT:-今天下午三點在會議室討論新的產品設計。}
AGENT_TEXT=${WATCH_E2E_AGENT_TEXT:-Reply with only the word pong.}
EN_PASS=${WATCH_E2E_EN_PASS:-80}
ZH_PASS=${WATCH_E2E_ZH_PASS:-60}
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
out="$root/target/watch-e2e/$(date +%Y%m%d-%H%M%S)"
mkdir -p "$out"
results="$out/results.jsonl"
: > "$results"
log() { printf '[%s] %s\n' "$(date +%H:%M:%S)" "$*" | tee -a "$out/run.log" >&2; }
die() { log "ERROR: $*"; exit "${2:-1}"; }
now_ms() { python3 -c 'import time; print(int(time.time()*1000))'; }
# run_to SECONDS CMD...: a command that a pending permission prompt could hang forever.
run_to() { local s=$1; shift; perl -e 'alarm shift; exec @ARGV' "$s" "$@"; }

for tool in jq python3 perl osascript say afconvert xcrun codesign pbcopy pbpaste; do
  command -v "$tool" >/dev/null || die "missing tool: $tool" 2
done
if stage_on 4; then
  command -v cargo >/dev/null || die "missing tool: cargo (stage 4)" 2
  cargo nextest --version >/dev/null 2>&1 || die "missing cargo-nextest (brew install cargo-nextest)" 2
fi

# Safety: the test build's data root is its own, and nothing below names the
# installed app's folder.
[ "$TEST_DATA" != "$OLD_DATA" ] || die "test data root equals the installed app's" 2

log_pid=
sim_udid=
keep_setting_original=
cleanup() {
  [ -n "$log_pid" ] && kill "$log_pid" 2>/dev/null
  if [ -s "$out/chrome-handles" ]; then
    while IFS= read -r h; do chrome_close "$h"; done < "$out/chrome-handles"
  fi
  if [ -n "$keep_setting_original" ] && [ -x "$hex" ]; then
    "$hex" settings set labs.keep_on_clipboard "$keep_setting_original" >/dev/null 2>&1
  fi
  if [ "$keep" = 0 ]; then
    [ -x "$hex" ] && "$hex" quit >/dev/null 2>&1
    [ -n "$sim_udid" ] && xcrun simctl shutdown "$sim_udid" >/dev/null 2>&1
  fi
}
trap cleanup EXIT

# --- preflight --------------------------------------------------------------
# Everything that needs a human is collected here and reported once, as a
# BLOCKED block with exit status 3. Nothing here tries to grant anything.

blocked=()
need() { blocked+=("$1"); }

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

# The runner needs Accessibility for the B2 live tests and the Chrome scenario (bringing
# the throwaway Chrome to the front), and Automation of System Events to prove it.
probe_runner_accessibility() {
  local err rc runner
  runner=$(runner_app)
  err=$(run_to 25 osascript -e 'tell application "System Events" to get name of first process whose frontmost is true' 2>&1 >/dev/null); rc=$?
  [ $rc = 0 ] && return 0
  if grep -qi 'assistive' <<<"$err"; then
    need "Accessibility: in System Settings > Privacy & Security > Accessibility, turn on \"$runner\" (the stage 4 live tests read the focused field through it, and the Chrome scenario brings its throwaway Chrome to the front with it)."
  else
    need "Automation: in System Settings > Privacy & Security > Automation, turn on \"System Events\" under \"$runner\", and in Accessibility turn on \"$runner\". osascript said: ${err:-timed out waiting for the prompt}"
  fi
}

# The Chromium scenario starts its own Chrome process with a throwaway profile
# (see chrome_open) and reads it back over its debugging port. It changes nothing
# in the user's Chrome and sends Chrome no Apple event. It only needs Chrome
# installed, and (to bring that process to the front) the same System Events
# access the stage 4 live tests need.
probe_chrome() {
  local handle
  [ -x "$CHROME_BIN" ] || { need "Install Google Chrome in /Applications (the Chromium scenario starts a separate Chrome process with a throwaway profile; your own Chrome is not touched)."; return; }
  handle=$(chrome_open) || true
  if [ -z "$handle" ]; then
    need "A separate Chrome process with a throwaway profile did not start (see ${TMPDIR:-/tmp}/wristper-e2e-chrome.*.log); check Google Chrome launches."
    return
  fi
  chrome_close "$handle"
}

print_blocked() {
  local i=1 line
  {
    echo
    echo "BLOCKED: ${#blocked[@]} thing(s) need a human on this Mac before the run can continue."
    echo "The script never grants permissions itself. Do each step once, then run again with --skip-build:"
    for line in "${blocked[@]}"; do echo "  $i. $line"; i=$((i + 1)); done
    echo "Wristper Test is left running so you can grant its permissions."
  } | tee -a "$out/run.log"
  keep=1
  exit 3
}

# A locked screen hides every window from Accessibility and AppleScript. It is
# only recorded here: the build below still runs, and check_permissions then
# reports it with everything else. The script never tries to unlock it.
if screen_locked; then
  need "The screen is locked (IOConsoleLocked). Unlock this Mac and keep it awake and idle for the run (about 15 min; System Settings > Lock Screen can keep it from locking again). Nothing here unlocks it."
  log "screen is locked: GUI scenarios cannot run; building only"
fi

# --- build ------------------------------------------------------------------

build_mac_app() {
  log "building Wristper Test.app (HEX_TEST_BUILD=1 HEX_LOCAL_BUILD=1)"
  (cd "$root" && HEX_TEST_BUILD=1 HEX_LOCAL_BUILD=1 scripts/build-app.sh) > "$out/build-app.log" 2>&1 \
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

build_watch_sim() {
  log "building watch-sim from $WRISTPER_SRC"
  "$root/scripts/watch-sim/build.sh" "$sim" > "$out/build-watch-sim.log" 2>&1 \
    || die "watch-sim build failed; see $out/build-watch-sim.log"
}

[ "$build" = 1 ] && build_mac_app
[ -x "$hex" ] || die "$app is missing; run without --skip-build" 2
[ "$(/usr/libexec/PlistBuddy -c 'Print :CFBundleIdentifier' "$app/Contents/Info.plist")" = "$TEST_BUNDLE" ] \
  || die "$app is not $TEST_BUNDLE" 2
if { [ "$build" = 1 ] || [ ! -x "$sim" ]; } && { stage_on 1 || stage_on 3; }; then build_watch_sim; fi

# --- the test app -----------------------------------------------------------

status_json() { "$hex" status --json 2>/dev/null; }

start_app() {
  if [ "$build" = 1 ] && status_json >/dev/null; then
    log "restarting Wristper Test so the new build runs (hex quit)"
    "$hex" quit >/dev/null 2>&1
    for _ in $(seq 1 30); do status_json >/dev/null || break; sleep 1; done
  fi
  if status_json >/dev/null; then
    log "Wristper Test is already running"
  else
    log "launching Wristper Test in the background"
    open -g "$app" || die "could not open $app"
  fi
  for _ in $(seq 1 60); do status_json >/dev/null && break; sleep 1; done
  status_json > "$out/status-start.json" || die "Wristper Test did not answer hex status within 60 s"
  [ -s "$TEST_DATA/relay-room" ] || die "no relay room in $TEST_DATA; the watch service did not start (see $TEST_DATA/logs/process.log)"
  log "relay room: $(cut -c1-6 "$TEST_DATA/relay-room")… (Wristper Test's own)"
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
    "$hex" model install "$want" --language "$1" 2>&1 | tee -a "$out/run.log"
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
  if [ "$mic" != ready ]; then
    need "Microphone for Wristper Test.app ($mic): run \"$hex\" window show, then click Allow for Microphone in the setup page."
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

# --- fixtures ---------------------------------------------------------------

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

# --- targets ----------------------------------------------------------------
# textedit: a new document. terminal: a Terminal window running cat.
# shell: a Terminal window at a plain zsh prompt. chrome: a throwaway page.
# claude, codex: a Terminal window running that agent.

kind_of() {
  case "$1" in
    textedit) echo textedit ;;
    terminal|shell|claude|codex) echo terminal ;;
    chrome) echo chrome ;;
  esac
}

# Chrome runs as its own process with a throwaway profile: the user's Chrome,
# its windows, its profile, and its settings are never used or changed. No
# Apple event is sent to Chrome at all. The handle is "pid:port:profile-dir".
chrome_open() {
  local dir port pid i
  [ -x "$CHROME_BIN" ] || return 1
  dir=$(mktemp -d "${TMPDIR:-/tmp}/wristper-e2e-chrome.XXXXXX") || return 1
  port=$(python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1",0)); print(s.getsockname()[1])')
  # --use-mock-keychain and basic password store: a fresh profile must not touch the login Keychain.
  "$CHROME_BIN" --user-data-dir="$dir" --remote-debugging-port="$port" --no-first-run --no-default-browser-check \
    --use-mock-keychain --password-store=basic --disable-extensions --disable-sync --window-size=900,700 \
    --new-window "$CHROME_URL" > "$dir.log" 2>&1 &
  pid=$!
  echo "$pid:$port:$dir" >> "$out/chrome-handles"   # the exit trap closes any that are left
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
# Stops only the process this script started, then moves its profile to the Trash.
chrome_close() {
  local pid=${1%%:*} rest=${1#*:} dir i
  dir=${rest#*:}
  # Only a process whose command line carries this throwaway profile: never the user's Chrome.
  if [[ $(ps -p "$pid" -o command= 2>/dev/null) == *"--user-data-dir=$dir"* ]]; then
    kill "$pid" 2>/dev/null
    for i in $(seq 1 20); do kill -0 "$pid" 2>/dev/null || break; sleep 0.5; done
    kill -0 "$pid" 2>/dev/null && kill -9 "$pid" 2>/dev/null
  fi
  case "$dir" in */wristper-e2e-chrome.*) /usr/bin/trash "$dir" "$dir.log" >/dev/null 2>&1 ;; esac
}

# Prints a handle for the opened target.
open_target() {
  case "$1" in
    textedit)
      osascript -e 'tell application "TextEdit"' -e 'activate' -e 'set d to make new document' \
        -e 'return name of d' -e 'end tell' ;;
    terminal)
      osascript -e 'tell application "Terminal"' -e 'activate' -e 'set t to do script "clear; cat"' \
        -e 'delay 1' -e 'return tty of t' -e 'end tell' ;;
    shell)
      osascript -e 'tell application "Terminal"' -e 'activate' -e 'set t to do script "clear"' \
        -e 'delay 1.5' -e 'return tty of t' -e 'end tell' ;;
    claude|codex)
      osascript -e 'tell application "Terminal"' -e 'activate' \
        -e "set t to do script \"cd '$AGENT_DIR' && clear && '$AGENT_BIN'\"" \
        -e 'delay 12' -e 'return tty of t' -e 'end tell' ;;
    chrome) chrome_open ;;
  esac
}

focus_target() {
  case "$(kind_of "$1")" in
    textedit)
      osascript -e "tell application \"TextEdit\"" -e "activate" \
        -e "set index of (first window whose name is \"$2\") to 1" -e "end tell" >/dev/null ;;
    terminal)
      osascript -e 'tell application "Terminal"' -e 'activate' -e 'repeat with w in windows' \
        -e "if tty of selected tab of w is \"$2\" then set index of w to 1" -e 'end repeat' -e 'end tell' >/dev/null ;;
    chrome)
      # Bring this Chrome process (by its pid) to the front; it is not the one the user may have open.
      osascript -e 'tell application "System Events"' \
        -e "set frontmost of (first process whose unix id is ${2%%:*}) to true" -e 'end tell' >/dev/null
      sleep 1 ;;
  esac
}

read_target() {
  case "$(kind_of "$1")" in
    textedit) osascript -e "tell application \"TextEdit\" to return text of document \"$2\"" ;;
    terminal)
      osascript -e 'tell application "Terminal"' -e 'repeat with w in windows' \
        -e "if tty of selected tab of w is \"$2\" then return contents of selected tab of w" \
        -e 'end repeat' -e 'end tell' ;;
    chrome) chrome_page "$(cut -d: -f2 <<<"$2")" ;;
  esac
}

close_target() {
  case "$(kind_of "$1")" in
    textedit) osascript -e "tell application \"TextEdit\" to close document \"$2\" saving no" >/dev/null 2>&1 ;;
    terminal)
      # Only processes on this window's own tty; never by name across the system.
      pkill -t "${2#/dev/}" -f 'cat|claude|codex' 2>/dev/null
      osascript -e 'tell application "Terminal"' -e 'repeat with w in windows' \
        -e "if tty of selected tab of w is \"$2\" then do script \"exit\" in selected tab of w" \
        -e 'end repeat' -e 'end tell' >/dev/null 2>&1 ;;
    chrome) chrome_close "$2" ;;
  esac
}

# record STAGE TARGET LANG RESULT PERCENT STATE MS TEXT DETAIL_JSON
record() {
  jq -cn --arg stage "$1" --arg target "$2" --arg lang "$3" --arg result "$4" --arg percent "$5" \
    --arg state "$6" --arg ms "$7" --arg text "$8" --argjson detail "${9:-null}" \
    '{stage:$stage,target:$target,lang:$lang,result:$result,match_percent:$percent,state:$state,ms:$ms,text:$text,detail:$detail}' \
    >> "$results"
  log "stage $1 $2 $3: $4 (match ${5}%, state $6, ${7} ms)"
}

judge() { # lang-or-kind readback -> percent
  "$sim" match --expected "$(text_for "$1")" --actual "$2" | jq -r .percent
}

want() { [ -z "$only" ] || [[ $1 =~ $only ]]; }

# --- watch-sim cases (stages 1 and 3) ---------------------------------------

sim_paired=0
pair_sim() { # stage
  export WATCH_SIM_STATE="$out/watch-sim-state"
  log "stage $1: pairing watch-sim through the production relay"
  "$sim" pair --timeout 150 > "$out/stage$1-pair.jsonl" 2>&1 &
  local pid=$! code=
  for _ in $(seq 1 30); do
    code=$(jq -r 'select(.event=="code") | .code' "$out/stage$1-pair.jsonl" 2>/dev/null | head -1)
    [ -n "$code" ] && break
    sleep 0.5
  done
  [ -n "$code" ] || { cat "$out/stage$1-pair.jsonl" >&2; die "watch-sim got no pairing code"; }
  log "watch-sim code $code; hex watch pair $code"
  "$hex" watch pair "$code" 2>&1 | tee -a "$out/run.log"
  wait "$pid"
  jq -e 'select(.event=="paired") | .ok' "$out/stage$1-pair.jsonl" >/dev/null \
    || { cat "$out/stage$1-pair.jsonl" >&2; record "$1" pairing - FAIL 0 unpaired 0 "" "null"; return 1; }
  record "$1" pairing - PASS 100 paired "$(jq -r 'select(.event=="paired") | .ms' "$out/stage$1-pair.jsonl")" "" \
    "$(jq -c 'select(.event=="paired")' "$out/stage$1-pair.jsonl")"
  sim_paired=1
}

# sim_run TARGET AUDIO [watch-sim deliver args...]
# Opens the target, focuses it, delivers AUDIO (en, zh, agent) from watch-sim,
# reads the target back. Sets SC_REPORT (JSON), SC_READBACK, SC_MS, SC_HANDLE.
SC_REPORT=null; SC_READBACK=; SC_MS=0; SC_HANDLE=; SC_FILE="$out/readback.txt"
sim_run() {
  local target=$1 audio=$2 started
  shift 2
  SC_REPORT=null; SC_READBACK=; SC_MS=0
  SC_HANDLE=$(open_target "$target") || SC_HANDLE=
  [ -n "$SC_HANDLE" ] || { SC_REPORT='{"event":"no-target"}'; : > "$SC_FILE"; return 1; }
  sleep 1
  focus_target "$target" "$SC_HANDLE"
  started=$(now_ms)
  SC_REPORT=$("$sim" deliver --audio "$out/$audio.m4a" --timeout 90 "$@" | tail -1)
  SC_MS=$(( $(now_ms) - started ))
  jq -e . >/dev/null 2>&1 <<<"$SC_REPORT" || SC_REPORT=$(jq -Rn --arg r "$SC_REPORT" '{raw:$r}')
  sleep 1
  read_target "$target" "$SC_HANDLE" > "$SC_FILE" 2>/dev/null
  SC_READBACK=$(cat "$SC_FILE")
}

# After sim_run for a target that stays open (agents), close it.
sim_close() { close_target "$1" "$SC_HANDLE"; }

rj() { jq -r "$1" <<<"$SC_REPORT" 2>/dev/null; }          # a field of the report
FAILS=
chk() { [ "$1" = ok ] || FAILS="$FAILS; $2"; }              # chk ok|bad "what failed"
is() { if "$@"; then echo ok; else echo bad; fi; }
breaks() { tr -cd '\r\n' < "$SC_FILE" | wc -c | tr -d ' '; }  # line breaks in the read-back
count() { grep -o -- "$1" "$SC_FILE" | wc -l | tr -d ' '; }  # occurrences of a string

# finish STAGE SCENARIO LANG PERCENT: records PASS when FAILS is empty.
finish() {
  local result=PASS
  [ -z "$FAILS" ] || result=FAIL
  record "$1" "$2" "$3" "$result" "$4" "$(rj '.state // .event')" "$SC_MS" \
    "$(tr '\r\n' '  ' <"$SC_FILE" | tail -c 200)" "$(jq -c --arg f "${FAILS#; }" '. + {fails:$f}' <<<"$SC_REPORT")"
  FAILS=
}

judged() { # lang -> percent, with the pass threshold check
  SC_PERCENT=$(judge "$1" "$SC_READBACK"); SC_PERCENT=${SC_PERCENT:-0}
  chk "$(is [ "$SC_PERCENT" -ge "$(pass_for "$1")" ])" "read-back matches only ${SC_PERCENT}%"
}

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

# --- stage 3: scenarios no test covers --------------------------------------

set_keep() { "$hex" settings set labs.keep_on_clipboard "$1" >/dev/null 2>&1; sleep 1; }

scenario_clipboard() {
  local sentinel="WATCH-E2E-SENTINEL-$RANDOM$RANDOM" board
  keep_setting_original=$("$hex" settings get labs.keep_on_clipboard 2>/dev/null)
  [ "$keep_setting_original" = true ] && keep_setting_original=on || keep_setting_original=off

  if want clipboard-keep-gate-switch; then
    # keep_on_clipboard on, the person switches app after the recording started
    # and before its audio arrives: the first gate withholds the dictation, so
    # the clipboard is never written. This does NOT reach the final focus check
    # (after the clipboard write, right before the paste key): that window is
    # tens of ms and /status reports both refusals the same way (target-changed).
    set_keep on; printf %s "$sentinel" | pbcopy
    sim_run textedit en --after-focus 'open -a Finder; sleep 2'
    board=$(pbpaste)
    sim_close textedit
    chk "$(is [ "$(rj '.state')" = target-changed ])" "state is $(rj '.state'), want target-changed"
    chk "$(is [ "$board" = "$sentinel" ])" "clipboard is '${board:0:60}', want the sentinel back"
    chk "$(is [ "$(count Friday)" = 0 ])" "text reached the document anyway"
    SC_PERCENT=0; finish 3 clipboard-keep-gate-switch en "$SC_PERCENT"
  fi

  if want clipboard-keep-paste; then
    # keep_on_clipboard on, no switch: the dictation stays on the clipboard.
    set_keep on; printf %s "$sentinel" | pbcopy
    sim_run textedit en
    sleep 5; board=$(pbpaste)
    sim_close textedit
    judged en
    chk "$(is [ "$(judge en "$board")" -ge "$EN_PASS" ])" "clipboard is '${board:0:60}', want the dictation"
    finish 3 clipboard-keep-paste en "$SC_PERCENT"
  fi

  if want clipboard-default-restore; then
    # keep_on_clipboard off (the default): the clipboard comes back after the paste.
    set_keep off; printf %s "$sentinel" | pbcopy
    sim_run textedit en
    sleep 6; board=$(pbpaste)
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
    sim_run shell en --enter-twice; sim_close shell
    judged en
    chk "$(is [ "$(count 'command not found')" = 1 ])" "the shell ran the line $(count 'command not found') times, want 1"
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
      SC_REPORT='{"event":"missing"}'; SC_MS=0; : > "$SC_FILE"; FAILS="no $agent outside ~/.wristper on PATH"
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

stage3() {
  [ "$sim_paired" = 1 ] || pair_sim 3 || return
  select_model en
  scenario_clipboard
  scenario_enter_undo
  scenario_double_enter
  scenario_chrome
  [ "$with_agents" = 1 ] && { select_model en; scenario_agents; }
}

# --- stage 2: the real watch app in a watchOS Simulator ---------------------

sim_device() {
  local udid runtime
  udid=$(xcrun simctl list devices -j | jq -r --arg n "$SIM_NAME" \
    '[.devices[][] | select(.name==$n and .isAvailable)][0].udid // empty')
  if [ -z "$udid" ]; then
    runtime=$(xcrun simctl list runtimes -j | jq -r '[.runtimes[] | select(.platform=="watchOS" and .isAvailable)] | last | .identifier')
    [ -n "$runtime" ] && [ "$runtime" != null ] || die "no watchOS Simulator runtime installed (Xcode > Settings > Components)" 2
    udid=$(xcrun simctl create "$SIM_NAME" "$SIM_TYPE" "$runtime") || die "simctl create failed"
  fi
  echo "$udid"
}

wait_log() { # pattern timeout-seconds start-line -> matching line
  local i m
  for i in $(seq 1 $(( $2 * 2 ))); do
    m=$(tail -n +"$3" "$out/stage2-watch.log" | grep -E "$1"; true)
    if [ -n "$m" ]; then echo "${m%%$'\n'*}"; return 0; fi
    sleep 0.5
  done
  return 1
}

stage2() {
  [ -d "$WRISTPER_HOOKS_SRC/WristperProbe.xcodeproj" ] || die "WRISTPER_HOOKS_SRC=$WRISTPER_HOOKS_SRC has no WristperProbe.xcodeproj" 2
  grep -q "e2ePair" "$WRISTPER_HOOKS_SRC/WatchProbe/E2EHooks.swift" 2>/dev/null \
    || die "$WRISTPER_HOOKS_SRC lacks WatchProbe/E2EHooks.swift (branch test/sim-fixture-audio)" 2
  sim_udid=$(sim_device)
  log "stage 2: watchOS Simulator $SIM_NAME ($sim_udid)"
  xcrun simctl boot "$sim_udid" >/dev/null 2>&1
  xcrun simctl bootstatus "$sim_udid" -b >/dev/null 2>&1 || die "simulator did not boot"
  local dd="$root/target/watch-e2e/watch-dd" watch_app
  if [ "$build" = 1 ] || [ ! -d "$dd/Build/Products/Debug-watchsimulator/WristperProbe.app" ]; then
    # Ad hoc signing is required: an unsigned simulator build has no application-identifier,
    # so its Keychain refuses the pairing key ("Could not securely save the encryption key").
    log "building the watch app (Debug, E2E hooks) from $WRISTPER_HOOKS_SRC"
    xcodebuild -project "$WRISTPER_HOOKS_SRC/WristperProbe.xcodeproj" -scheme WristperProbe -configuration Debug \
      -destination "id=$sim_udid" -derivedDataPath "$dd" build \
      CODE_SIGN_IDENTITY=- CODE_SIGNING_ALLOWED=YES CODE_SIGNING_REQUIRED=NO DEVELOPMENT_TEAM= > "$out/build-watch.log" 2>&1 \
      || die "watch app build failed; see $out/build-watch.log"
  fi
  watch_app="$dd/Build/Products/Debug-watchsimulator/WristperProbe.app"
  # A fresh watch every run: no earlier pairing in the app or the simulator's Keychain.
  xcrun simctl terminate "$sim_udid" "$WATCH_BUNDLE" >/dev/null 2>&1
  xcrun simctl uninstall "$sim_udid" "$WATCH_BUNDLE" >/dev/null 2>&1
  xcrun simctl keychain "$sim_udid" reset >/dev/null 2>&1
  xcrun simctl install "$sim_udid" "$watch_app" || die "simctl install failed"
  xcrun simctl privacy "$sim_udid" grant microphone "$WATCH_BUNDLE" >/dev/null 2>&1

  : > "$out/stage2-watch.log"
  xcrun simctl spawn "$sim_udid" log stream --style compact \
    --predicate 'subsystem == "dev.wristper.probe"' > "$out/stage2-watch.log" 2>&1 &
  log_pid=$!
  sleep 2

  local start line code
  start=$(( $(wc -l < "$out/stage2-watch.log") + 1 ))
  xcrun simctl launch --terminate-running-process "$sim_udid" "$WATCH_BUNDLE" -e2ePair >/dev/null \
    || die "simctl launch failed"
  line=$(wait_log 'E2E pair-code [0-9]{6}' 30 "$start") || { record 2 pairing - FAIL 0 no-code 0 "" null; return; }
  code=$(grep -oE '[0-9]{6}$' <<<"$line")
  log "watch app code $code; hex watch pair $code"
  local t0; t0=$(now_ms)
  "$hex" watch pair "$code" 2>&1 | tee -a "$out/run.log"
  line=$(wait_log 'E2E (paired|pair-failed)' 150 "$start")
  if grep -q "E2E paired" <<<"$line"; then
    record 2 pairing - PASS 100 paired $(( $(now_ms) - t0 )) "" "$(jq -cn --arg l "$line" '{log:$l}')"
  else
    record 2 pairing - FAIL 0 "${line:-timeout}" $(( $(now_ms) - t0 )) "" null
    return
  fi

  local container target lang handle readback percent result started ms enter_arg detail
  container=$(xcrun simctl get_app_container "$sim_udid" "$WATCH_BUNDLE" data) || die "no app container"
  mkdir -p "$container/tmp"
  for lang in ${langs//,/ }; do
    select_model "$lang"
    for target in ${targets//,/ }; do
      cp "$out/$lang.m4a" "$container/tmp/fixture-$lang.m4a"
      handle=$(open_target "$target") || { record 2 "$target" "$lang" FAIL 0 no-target 0 "" null; continue; }
      sleep 1
      focus_target "$target" "$handle"
      enter_arg=; [ "$target" = terminal ] && enter_arg=-e2eEnter
      start=$(( $(wc -l < "$out/stage2-watch.log") + 1 ))
      started=$(now_ms)
      xcrun simctl launch --terminate-running-process "$sim_udid" "$WATCH_BUNDLE" \
        -e2eRecord -fixtureAudio "$container/tmp/fixture-$lang.m4a" $enter_arg >/dev/null
      wait_log 'E2E done' 150 "$start" >/dev/null
      ms=$(( $(now_ms) - started ))
      sleep 1
      readback=$(read_target "$target" "$handle")
      close_target "$target" "$handle"
      detail=$(tail -n +"$start" "$out/stage2-watch.log" | grep -E 'E2E (fixture|result|enter)' | jq -R . | jq -sc '{log:.}')
      percent=$(judge "$lang" "$readback")
      result=FAIL; [ "${percent:-0}" -ge "$(pass_for "$lang")" ] && result=PASS
      record 2 "$target" "$lang" "$result" "$percent" \
        "$(tail -n +"$start" "$out/stage2-watch.log" | grep -m1 -oE 'phase=[a-z]+' || echo no-result)" "$ms" \
        "$(tr '\n' ' ' <<<"$readback" | tail -c 200)" "$detail"
    done
  done
}

# --- stage 4: the B2 live tests ---------------------------------------------

stage4() {
  local log4="$out/stage4-live.log" name res rc
  log "stage 4: cargo nextest run --locked --bin hex --run-ignored only -E 'test(watch_live)'"
  [ -f "$WATCH_E2E_B2_ENV" ] && . "$WATCH_E2E_B2_ENV"
  # One test at a time: both drive the same screen.
  (cd "$root" && cargo nextest run --locked --bin hex --run-ignored only -E 'test(watch_live)' --no-fail-fast -j 1) > "$log4" 2>&1
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

# --- run --------------------------------------------------------------------

start_app
check_permissions
[ "$preflight_only" = 1 ] && { log "preflight OK"; echo "PREFLIGHT: OK"; exit 0; }
select_model "${langs%%,*}"
wait_listener
make_fixtures
stage_on 1 && stage1
stage_on 2 && stage2
stage_on 3 && stage3
stage_on 4 && stage4

echo
printf '%-5s %-26s %-4s %-6s %6s %-14s %7s  %s\n' STAGE SCENARIO LANG RESULT MATCH STATE MS "READ BACK"
jq -r '[.stage,.target,.lang,.result,(.match_percent+"%"),.state,.ms,(.text|.[0:60]),((.detail.fails // "")|tostring)] | @tsv' "$results" |
  while IFS=$'\t' read -r a b c d e f g h i; do
    printf '%-5s %-26s %-4s %-6s %6s %-14s %7s  %s\n' "$a" "$b" "$c" "$d" "$e" "$f" "$g" "$h"
    [ -n "$i" ] && printf '        └ %s\n' "$i"
  done
echo
echo "Details: $results"
if jq -e 'select(.result!="PASS")' "$results" >/dev/null; then
  echo "RESULT: FAIL"; exit 1
fi
[ -s "$results" ] || { echo "RESULT: FAIL (no cases ran)"; exit 1; }
echo "RESULT: PASS"
