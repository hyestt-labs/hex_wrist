#!/usr/bin/env bash
# Runs scripts/merge-gate.sh against a fake `gh`. By default one Compile run has
# all four jobs at success, the check log is good, and the PR body has an
# unverified section, so a case changes one thing: the report (check (c)) or the
# Compile runs (check (a)). The fake reads the runs from $SCEN/runs (newest first)
# and each run's jobs from $SCEN/jobs-<run id> as "name<TAB>conclusion" lines.
set -u
here=$(cd "$(dirname "$0")" && pwd)
gate="$here/merge-gate.sh"
tmp=$(mktemp -d)
trap '/usr/bin/trash "$tmp" 2>/dev/null || true' EXIT
sha=0123456789abcdef0123456789abcdef01234567
fail=0

mkdir "$tmp/bin"
cat > "$tmp/bin/gh" <<FAKE
#!/usr/bin/env bash
case "\$*" in
  *headRefOid*) echo $sha ;;
  "run list"*) cat "\$SCEN/runs" ;;
  api*) id=\${2#*/runs/}; id=\${id%%/*}; cat "\$SCEN/jobs-\$id" ;;
  *"--json body"*) printf '## 未驗證\n- nothing\n' ;;
  *) echo "fake gh: unexpected \$*" >&2; exit 1 ;;
esac
FAKE
chmod +x "$tmp/bin/gh"

# scenario <name> <run id>...: runs listed newest first, all four jobs success
scenario() {
  local dir="$tmp/scen-$1"; shift
  mkdir -p "$dir"; : > "$dir/runs"
  for id in "$@"; do
    echo "$id" >> "$dir/runs"
    printf 'Linux\tsuccess\nWatch tests\tsuccess\nmacOS\tsuccess\nWindows x64\tsuccess\n' > "$dir/jobs-$id"
  done
  echo "$dir"
}
# setjob <scenario dir> <run id> <job> <conclusion>
setjob() {
  local f="$1/jobs-$2" t
  t=$(grep -Fv "$3"$'\t' "$f"); printf '%s\n%s\t%s\n' "$t" "$3" "$4" > "$f"
}
scen_ok=$(scenario ok 4242)
scen_latest_bad=$(scenario latest-bad 3 2 1); setjob "$scen_latest_bad" 3 macOS failure
setjob "$scen_latest_bad" 2 "Windows x64" cancelled
scen_split=$(scenario split 3 2); setjob "$scen_split" 3 macOS failure; setjob "$scen_split" 2 "Windows x64" skipped
scen_none=$(scenario none)
scen_old=$(scenario old 2 1); setjob "$scen_old" 2 Linux failure
printf 'sha=%s\nok\nexit=0\n' "$sha" > "$tmp/check.log"
printf '        FAIL [   0.038s] (1/1) hex::bin/hex some::test\n' > "$tmp/with-failed.md"
printf 'sha=%s\nexit=1\n' "$sha" > "$tmp/bad-check.log"
printf 'All tests pass, no mutation run.\n' > "$tmp/no-failed.md"

# run <name> <expected exit> <expected output regex> <gate args...>
run() {
  local name=$1 want=$2 pattern=$3; shift 3
  local out rc
  out=$(SCEN="${scen:-$scen_ok}" PATH="$tmp/bin:$PATH" "$gate" 19 --check-log "$tmp/check.log" "$@" 2>&1); rc=$?
  echo "--- $name"
  echo "$out" | sed 's/^/    /'
  echo "    exit=$rc"
  if [ "$rc" = "$want" ] && echo "$out" | tr "\n" " " | grep -Eq "$pattern"; then echo "PASS  $name"
  else echo "FAIL  $name (wanted exit $want and /$pattern/)"; fail=1; fi
}

run "1. report has FAILED output, no flag: passes" 0 'ok    report holds mutation FAILED output.*|GATE PASS' \
  --report "$tmp/with-failed.md"
run "2. no FAILED output, no flag: fails" 1 'FAIL  report has no mutation FAILED output' \
  --report "$tmp/no-failed.md"
run "3. flag with a reason: skips (c), prints the skip and the reason" 0 'SKIP  \(c\) mutation check skipped: CI-only change, no bug fix' \
  --report "$tmp/no-failed.md" --no-mutation "CI-only change, no bug fix"
run "4. flag without a reason: fails" 1 'FAIL  --no-mutation needs a reason' \
  --report "$tmp/no-failed.md" --no-mutation
run "5. flag with an empty reason: fails" 1 'FAIL  --no-mutation needs a reason' \
  --report "$tmp/no-failed.md" --no-mutation ""
run "6. flag followed by another option: fails" 1 'FAIL  --no-mutation needs a reason' \
  --no-mutation --report "$tmp/no-failed.md"
run "7. flag does not hide the other checks (bad check log): fails" 1 'FAIL  scripts/check log does not end with exit=0' \
  --report "$tmp/no-failed.md" --no-mutation "CI-only change" --check-log "$tmp/bad-check.log"

# Check (a): any single Compile run among the latest ten with all four jobs.
# scen_latest_bad: newest run 3 fails macOS, run 2 fails Windows, run 1 is all green.
scen=$scen_latest_bad
run "8. newest run fails, an earlier run has all four jobs: passes and names that run" 0 'ok    Compile run 1 has Linux, Watch tests, macOS, Windows x64 all success' \
  --report "$tmp/with-failed.md"
scen=$scen_split
run "9. four jobs success but spread over two runs (each missing one): fails" 1 'FAIL  no Compile run among the latest 10 .*run 3: not success: macOS;.*run 2: not success: Windows x64;' \
  --report "$tmp/with-failed.md"
scen=$scen_old
run "10. a run missing Linux is skipped, the older run is used" 0 'ok    Compile run 1 ' \
  --report "$tmp/with-failed.md"
scen=$scen_none
run "11. no Compile run at all: fails" 1 'FAIL  no Compile run for' \
  --report "$tmp/with-failed.md"
scen=

# Check (c): FAILED output in a Markdown table cell counts; its absence fails.
printf '| step | output |\n|---|---|\n| revert fix | `FAIL [   0.038s] hex::bin/hex some::a_test` |\n' > "$tmp/table-cell.md"
printf '| step | output |\n|---|---|\n| revert fix | the test FAIL and nothing else |\n| note | `FAIL` is what the gate looks for |\n' > "$tmp/table-no-fail.md"
run "12. FAIL line inside a table cell: passes" 0 'ok    report holds mutation FAILED output' \
  --report "$tmp/table-cell.md"
run "13. table with no FAIL test output (bare word FAIL only): fails" 1 'FAIL  report has no mutation FAILED output' \
  --report "$tmp/table-no-fail.md"
run "14. no FAIL anywhere, as in case 2: still fails" 1 'FAIL  report has no mutation FAILED output' \
  --report "$tmp/no-failed.md"

[ "$fail" = 0 ] && echo "ALL PASS" || { echo "SOME FAILED"; exit 1; }
