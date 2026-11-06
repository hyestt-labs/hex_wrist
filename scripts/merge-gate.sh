#!/usr/bin/env bash
# Merge gate for a pull request on the fork. Fails unless every check holds:
#   1. one of the head SHA's 10 latest Compile runs has all four jobs (Linux,
#      Watch tests, macOS, Windows x64) at success; jobs from different runs
#      are never combined, and the passing run is printed
#   2. a local `scripts/check` log for that SHA ends with `exit=0`
#   3. the worker report holds the mutation check's FAILED lines, unless
#      `--no-mutation "<reason>"` skips it (for a PR that fixes no bug); the
#      skip and its reason are printed, and a missing reason fails
#   4. the PR body has an "unverified" section
# FAILED output counts at a line start (`FAIL`), as `test result: FAILED` or
# `... FAILED`, or anywhere after a backtick as `FAIL <test path>`, so a report
# may put the output in a Markdown table cell.
# Usage: scripts/merge-gate.sh <pr> --check-log <file> --report <file>
#          [--no-mutation "<reason>"] [--repo owner/name]
# scripts/test-merge-gate.sh exercises it against a fake `gh`.
# The check log is written by: scripts/check > log 2>&1; echo "exit=$?" >> log
# and must name the head SHA on its first line: echo "sha=<head>" > log first.
set -euo pipefail

pr="" check_log="" report="" repo="hyestt-labs/hex_wrist" no_mutation=0 no_mutation_reason=""
while [ $# -gt 0 ]; do
  case "$1" in
    --check-log) check_log="$2"; shift 2 ;;
    --report) report="$2"; shift 2 ;;
    --repo) repo="$2"; shift 2 ;;
    --no-mutation)
      no_mutation=1
      if [ $# -ge 2 ] && [[ "$2" != --* ]]; then no_mutation_reason="$2"; shift 2; else shift; fi ;;
    *) pr="$1"; shift ;;
  esac
done
[ -n "$pr" ] && [ -n "$check_log" ] && [ -n "$report" ] || {
  echo "usage: $0 <pr> --check-log <file> --report <file> [--no-mutation \"<reason>\"] [--repo owner/name]" >&2; exit 2; }

fail=0
bad() { echo "FAIL  $1"; fail=1; }
ok() { echo "ok    $1"; }

head_sha=$(gh pr view "$pr" --repo "$repo" --json headRefOid -q .headRefOid)
echo "PR #$pr head $head_sha"

compile_jobs=("Linux" "Watch tests" "macOS" "Windows x64")
run_ids=$(gh run list --repo "$repo" --workflow compile.yml --commit "$head_sha" --limit 10 \
  --json databaseId -q '.[].databaseId')
if [ -z "$run_ids" ]; then
  bad "no Compile run for $head_sha"
else
  good_run="" summaries=""
  for run_id in $run_ids; do
    jobs=$(gh api "repos/$repo/actions/runs/$run_id/jobs" \
      --jq '.jobs[] | "\(.name)\t\(.conclusion // "none")"')
    missing=""
    for job in "${compile_jobs[@]}"; do
      printf '%s\n' "$jobs" | grep -Fxq "$job"$'\t'success || missing="$missing $job;"
    done
    if [ -z "$missing" ]; then good_run=$run_id; break; fi
    summaries="$summaries"$'\n'"      run $run_id: not success:$missing"
  done
  if [ -n "$good_run" ]; then
    ok "Compile run $good_run has Linux, Watch tests, macOS, Windows x64 all success"
  else
    bad "no Compile run among the latest 10 for $head_sha has all four jobs at success (jobs of different runs are not combined):$summaries"
  fi
fi

if [ ! -f "$check_log" ]; then
  bad "check log $check_log is missing"
else
  log_sha=$(sed -n 's/^sha=//p' "$check_log" | head -1)
  [ "$log_sha" = "$head_sha" ] && ok "check log is for $head_sha" \
    || bad "check log names '${log_sha:-no sha}', not $head_sha"
  tail -1 "$check_log" | grep -qx 'exit=0' && ok "scripts/check exit=0" \
    || bad "scripts/check log does not end with exit=0"
fi

if [ "$no_mutation" = 1 ]; then
  if [ -z "${no_mutation_reason//[[:space:]]/}" ]; then
    bad "--no-mutation needs a reason: --no-mutation \"<why no bug fix to mutate>\""
  else
    echo "SKIP  (c) mutation check skipped: $no_mutation_reason"
  fi
elif [ ! -f "$report" ]; then
  bad "report $report is missing"
elif grep -Eq '^[[:space:]]*FAIL[[:space:]]|test result: FAILED|\.\.\. FAILED|`FAIL[[:space:]]+[^`]*[[:alnum:]_]' "$report"; then
  ok "report holds mutation FAILED output"
else
  bad "report has no mutation FAILED output (revert the fix, paste the failing test)"
fi

pr_body=$(gh pr view "$pr" --repo "$repo" --json body -q .body)
printf '%s' "$pr_body" | grep -Eiq '^#+ .*(unverified|未驗證)' && ok "PR body has an unverified section" \
  || bad "PR body has no 'unverified' / '未驗證' heading"

[ "$fail" = 0 ] && echo "GATE PASS" || { echo "GATE FAIL"; exit 1; }
