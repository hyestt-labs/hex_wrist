#!/usr/bin/env bash
# Runs scripts/merge-gate.sh against a fake `gh` whose Compile jobs all pass, its
# check log is good, and its PR body has an unverified section, so only check (c),
# the mutation check, decides each case.
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
  "run list"*) echo 4242 ;;
  api*) echo success ;;
  *"--json body"*) printf '## 未驗證\n- nothing\n' ;;
  *) echo "fake gh: unexpected \$*" >&2; exit 1 ;;
esac
FAKE
chmod +x "$tmp/bin/gh"
printf 'sha=%s\nok\nexit=0\n' "$sha" > "$tmp/check.log"
printf '        FAIL [   0.038s] (1/1) hex::bin/hex some::test\n' > "$tmp/with-failed.md"
printf 'sha=%s\nexit=1\n' "$sha" > "$tmp/bad-check.log"
printf 'All tests pass, no mutation run.\n' > "$tmp/no-failed.md"

# run <name> <expected exit> <expected output regex> <gate args...>
run() {
  local name=$1 want=$2 pattern=$3; shift 3
  local out rc
  out=$(PATH="$tmp/bin:$PATH" "$gate" 19 --check-log "$tmp/check.log" "$@" 2>&1); rc=$?
  echo "--- $name"
  echo "$out" | sed 's/^/    /'
  echo "    exit=$rc"
  if [ "$rc" = "$want" ] && echo "$out" | grep -Eq "$pattern"; then echo "PASS  $name"
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

[ "$fail" = 0 ] && echo "ALL PASS" || { echo "SOME FAILED"; exit 1; }
