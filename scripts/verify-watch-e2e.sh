#!/bin/bash
# End-to-end proof that Wristper Test.app receives Apple Watch dictations over
# the production relay and pastes them into a real app, with no human in the
# loop once the one-time permissions are granted.
#
#   stage 1  watch-sim: the shipping watch protocol code as a macOS CLI
#   stage 2  the real watch app (DEBUG build) in a watchOS Simulator
#
# Each stage pairs a fresh watch to the test app's own relay room, sends an
# English and a Mandarin recording while TextEdit or Terminal (running cat)
# has focus, and reads the text back from that app.
#
# Exit status: 0 every case passed, 1 a case failed, 2 bad usage or a tool
# is missing, 3 blocked on a one-time permission (the output says which).
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
while [ $# -gt 0 ]; do
  case "$1" in
    --stage) stage=$2; shift 2 ;;
    --skip-build) build=0; shift ;;
    --targets) targets=$2; shift 2 ;;
    --langs) langs=$2; shift 2 ;;
    --keep-running) keep=1; shift ;;
    -h|--help) sed -n 2,20p "$0"; echo; echo "usage: $0 [--stage 1|2|all] [--skip-build] [--targets textedit,terminal] [--langs en,zh] [--keep-running]"; exit 0 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done
case "$stage" in 1|2|all) ;; *) echo "--stage must be 1, 2, or all" >&2; exit 2 ;; esac

export DEVELOPER_DIR=${DEVELOPER_DIR:-/Applications/Xcode.app/Contents/Developer}
[ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"
unset HEX_APPLICATION_SUPPORT_DIR HEX_WATCH_RELAY_BASE

WRISTPER_SRC=${WRISTPER_SRC:-/Users/glenhsu/orca/workspaces/wristper/coding}
WRISTPER_HOOKS_SRC=${WRISTPER_HOOKS_SRC:-/Users/glenhsu/orca/workspaces/wristper/sim-hooks}
SIGN_IDENTITY=${SIGN_IDENTITY:-Developer ID Application: WEI CHIEH HSU (VQY3J86U79)}
SIM_NAME=${WATCH_E2E_SIM_NAME:-Wristper E2E}
SIM_TYPE=${WATCH_E2E_SIM_TYPE:-com.apple.CoreSimulator.SimDeviceType.Apple-Watch-Series-11-46mm}
WATCH_BUNDLE=dev.wristper.watch.watchkitapp
TEST_BUNDLE=dev.wristper.mac.test
TEST_DATA="$HOME/Library/Application Support/Wristper Test"
OLD_DATA="$HOME/Library/Application Support/Wristper"
EN_TEXT=${WATCH_E2E_EN_TEXT:-Please send the quarterly report to the design team before Friday.}
ZH_TEXT=${WATCH_E2E_ZH_TEXT:-今天下午三點在會議室討論新的產品設計。}
EN_PASS=${WATCH_E2E_EN_PASS:-80}
ZH_PASS=${WATCH_E2E_ZH_PASS:-60}

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

for tool in jq python3 osascript say afconvert xcrun codesign; do
  command -v "$tool" >/dev/null || die "missing tool: $tool" 2
done

# Safety: the test build's data root is its own, and nothing below names the
# installed app's folder.
[ "$TEST_DATA" != "$OLD_DATA" ] || die "test data root equals the installed app's" 2

log_pid=
sim_udid=
cleanup() {
  [ -n "$log_pid" ] && kill "$log_pid" 2>/dev/null
  if [ "$keep" = 0 ]; then
    [ -x "$hex" ] && "$hex" quit >/dev/null 2>&1
    [ -n "$sim_udid" ] && xcrun simctl shutdown "$sim_udid" >/dev/null 2>&1
  fi
}
trap cleanup EXIT

# --- build ------------------------------------------------------------------

build_mac_app() {
  log "building Wristper Test.app (HEX_TEST_BUILD=1 HEX_LOCAL_BUILD=1)"
  (cd "$root" && HEX_TEST_BUILD=1 HEX_LOCAL_BUILD=1 scripts/build-app.sh) > "$out/build-app.log" 2>&1 \
    || die "build-app.sh failed; see $out/build-app.log"
  # build-app.sh signs a test build ad hoc, whose identity changes with every
  # build, so macOS forgets its Accessibility and Microphone grants each
  # time. Re-signing with the Developer ID keeps one identity across builds.
  if security find-identity -v -p codesigning | grep -qF "$SIGN_IDENTITY"; then
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
{ [ "$build" = 1 ] || [ ! -x "$sim" ]; } && build_watch_sim

# --- the test app -----------------------------------------------------------

status_json() { "$hex" status --json 2>/dev/null; }

start_app() {
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

ensure_ready() {
  local s mic ax
  s=$(status_json)
  echo "$s" > "$out/status-ready.json"
  mic=$(jq -r .permissions.microphone <<<"$s")
  ax=$(jq -r .permissions.accessibility <<<"$s")
  if [ "$mic" != ready ] || [ "$ax" != ready ]; then
    cat <<EOF | tee -a "$out/run.log"

BLOCKED: Wristper Test needs a one-time permission (microphone=$mic, accessibility=$ax).
The listener that turns watch audio into pasted text starts only after
Microphone, Accessibility, and the speech model are all ready.
Do this once on this Mac, then run the script again with --skip-build:
  1. Run: "$hex" window show
  2. In the Wristper Test window, click Allow for Microphone in the setup page.
  3. In System Settings > Privacy & Security > Accessibility, turn on "Wristper Test".
     If it is not listed, click +, and add: $app
  4. If the status still says needs-restart, run: "$hex" restart
Wristper Test is left running so you can grant them.
EOF
    keep=1
    exit 3
  fi
  for _ in $(seq 1 30); do
    [ "$(status_json | jq -r .listener.running)" = true ] && break
    sleep 1
  done
  s=$(status_json)
  [ "$(jq -r .listener.running <<<"$s")" = true ] || die "listener not running: $(jq -c .listener <<<"$s")"
  log "listener running; permissions ready"
}

# --- fixtures ---------------------------------------------------------------

voice_for() { say -v '?' | awk -v loc="$1" '$0 ~ loc {print $1; exit}'; }

make_fixtures() {
  local en zh
  en=$(say -v '?' | awk '/^Samantha /{print "Samantha"; exit}'); [ -n "$en" ] || en=$(voice_for en_US)
  zh=$(say -v '?' | awk '/^Meijia /{print "Meijia"; exit}'); [ -n "$zh" ] || zh=$(voice_for zh_TW)
  [ -n "$zh" ] || die "no zh_TW voice for say; install one in System Settings > Accessibility > Spoken Content" 2
  say -v "$en" -o "$out/en.aiff" "$EN_TEXT" && say -v "$zh" -o "$out/zh.aiff" "$ZH_TEXT" || die "say failed"
  # The watch's recorder: 16 kHz mono AAC at 32 kbps in .m4a (WatchProbe/ProbeModel.swift recorderSettings).
  for l in en zh; do
    afconvert -f m4af -d aac@16000 -c 1 -b 32000 "$out/$l.aiff" "$out/$l.m4a" || die "afconvert failed for $l"
  done
  log "fixtures: en ($en) $(stat -f%z "$out/en.m4a") B, zh ($zh) $(stat -f%z "$out/zh.m4a") B"
}

text_for() { [ "$1" = zh ] && echo "$ZH_TEXT" || echo "$EN_TEXT"; }
pass_for() { [ "$1" = zh ] && echo "$ZH_PASS" || echo "$EN_PASS"; }

# --- targets ----------------------------------------------------------------

# Prints a handle for the opened target.
open_target() {
  case "$1" in
    textedit)
      osascript -e 'tell application "TextEdit"' -e 'activate' -e 'set d to make new document' \
        -e 'return name of d' -e 'end tell' ;;
    terminal)
      osascript -e 'tell application "Terminal"' -e 'activate' -e 'set t to do script "clear; cat"' \
        -e 'delay 1' -e 'return tty of t' -e 'end tell' ;;
  esac
}

focus_target() {
  case "$1" in
    textedit)
      osascript -e "tell application \"TextEdit\"" -e "activate" \
        -e "set index of (first window whose name is \"$2\") to 1" -e "end tell" >/dev/null ;;
    terminal)
      osascript -e 'tell application "Terminal"' -e 'activate' -e 'repeat with w in windows' \
        -e "if tty of selected tab of w is \"$2\" then set index of w to 1" -e 'end repeat' -e 'end tell' >/dev/null ;;
  esac
}

read_target() {
  case "$1" in
    textedit) osascript -e "tell application \"TextEdit\" to return text of document \"$2\"" ;;
    terminal)
      osascript -e 'tell application "Terminal"' -e 'repeat with w in windows' \
        -e "if tty of selected tab of w is \"$2\" then return contents of selected tab of w" \
        -e 'end repeat' -e 'end tell' ;;
  esac
}

close_target() {
  case "$1" in
    textedit) osascript -e "tell application \"TextEdit\" to close document \"$2\" saving no" >/dev/null 2>&1 ;;
    terminal)
      pkill -t "${2#/dev/}" cat 2>/dev/null
      osascript -e 'tell application "Terminal"' -e 'repeat with w in windows' \
        -e "if tty of selected tab of w is \"$2\" then do script \"exit\" in selected tab of w" \
        -e 'end repeat' -e 'end tell' >/dev/null 2>&1 ;;
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

judge() { # lang readback -> percent
  "$sim" match --expected "$(text_for "$1")" --actual "$2" | jq -r .percent
}

# --- stage 1: watch-sim -----------------------------------------------------

stage1() {
  export WATCH_SIM_STATE="$out/watch-sim-state"
  log "stage 1: pairing watch-sim through the production relay"
  "$sim" pair --timeout 150 > "$out/stage1-pair.jsonl" 2>&1 &
  local pid=$! code=
  for _ in $(seq 1 30); do
    code=$(jq -r 'select(.event=="code") | .code' "$out/stage1-pair.jsonl" 2>/dev/null | head -1)
    [ -n "$code" ] && break
    sleep 0.5
  done
  [ -n "$code" ] || { cat "$out/stage1-pair.jsonl" >&2; die "watch-sim got no pairing code"; }
  log "watch-sim code $code; hex watch pair $code"
  "$hex" watch pair "$code" 2>&1 | tee -a "$out/run.log"
  wait "$pid"
  jq -e 'select(.event=="paired") | .ok' "$out/stage1-pair.jsonl" >/dev/null \
    || { cat "$out/stage1-pair.jsonl" >&2; record 1 pairing - FAIL 0 unpaired 0 "" "null"; return; }
  record 1 pairing - PASS 100 paired "$(jq -r 'select(.event=="paired") | .ms' "$out/stage1-pair.jsonl")" "" \
    "$(jq -c 'select(.event=="paired")' "$out/stage1-pair.jsonl")"

  local target lang handle report readback percent state started ms result enter
  for lang in ${langs//,/ }; do
    select_model "$lang"
    for target in ${targets//,/ }; do
      handle=$(open_target "$target") || { record 1 "$target" "$lang" FAIL 0 no-target 0 "" null; continue; }
      sleep 1
      focus_target "$target" "$handle"
      enter=; [ "$target" = terminal ] && enter=--enter
      started=$(now_ms)
      report=$("$sim" deliver --audio "$out/$lang.m4a" --timeout 90 $enter | tail -1)
      ms=$(( $(now_ms) - started ))
      sleep 1
      readback=$(read_target "$target" "$handle")
      close_target "$target" "$handle"
      percent=$(judge "$lang" "$readback")
      state=$(jq -r '.state // .event' <<<"$report")
      result=FAIL; [ "${percent:-0}" -ge "$(pass_for "$lang")" ] && result=PASS
      record 1 "$target" "$lang" "$result" "$percent" "$state" "$ms" \
        "$(tr '\n' ' ' <<<"$readback" | tail -c 200)" "$report"
    done
  done
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
  local i
  for i in $(seq 1 $(( $2 * 2 ))); do
    if tail -n +"$3" "$out/stage2-watch.log" | grep -m1 -E "$1"; then return 0; fi
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

# --- run --------------------------------------------------------------------

start_app
select_model "${langs%%,*}"
ensure_ready
make_fixtures
case "$stage" in
  1) stage1 ;;
  2) stage2 ;;
  all) stage1; stage2 ;;
esac

echo
printf '%-5s %-9s %-4s %-6s %6s %-12s %7s  %s\n' STAGE TARGET LANG RESULT MATCH STATE MS "READ BACK"
jq -r '[.stage,.target,.lang,.result,(.match_percent+"%"),.state,.ms,(.text|.[0:70])] | @tsv' "$results" |
  while IFS=$'\t' read -r a b c d e f g h; do printf '%-5s %-9s %-4s %-6s %6s %-12s %7s  %s\n' "$a" "$b" "$c" "$d" "$e" "$f" "$g" "$h"; done
echo
echo "Enter: Wristper B1 answers /enter with not-supported; the stage 1 terminal rows show it under detail.enter (B2 adds Enter)."
echo "Details: $results"
if jq -e 'select(.result!="PASS")' "$results" >/dev/null; then
  echo "RESULT: FAIL"; exit 1
fi
[ -s "$results" ] || { echo "RESULT: FAIL (no cases ran)"; exit 1; }
echo "RESULT: PASS"
