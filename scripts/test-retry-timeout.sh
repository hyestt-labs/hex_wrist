#!/usr/bin/env bash
# Checks scripts/retry-timeout.sh: success, retry after failure, retry after a
# hang, and failure after every attempt fails or hangs. Uses the system
# `timeout`; on a Mac without GNU coreutils, put a `timeout` on PATH first.
set -u
here=$(cd "$(dirname "$0")" && pwd)
retry="$here/retry-timeout.sh"
tmp=$(mktemp -d)
trap '/usr/bin/trash "$tmp" 2>/dev/null || true' EXIT
fail=0
expect() { # name expected_exit actual_exit
  if [ "$2" = "$3" ]; then echo "PASS  $1 (exit $3)"; else echo "FAIL  $1: expected exit $2, got $3"; fail=1; fi
}

# Fails until the Nth call, counted in a file.
cat > "$tmp/flaky.sh" <<'FLAKY'
#!/usr/bin/env bash
count_file=$1 succeed_on=$2 mode=$3
n=$(( $(cat "$count_file" 2>/dev/null || echo 0) + 1 ))
echo "$n" > "$count_file"
if [ "$n" -ge "$succeed_on" ]; then echo "flaky: call $n ok"; exit 0; fi
[ "$mode" = hang ] && { echo "flaky: call $n hangs"; sleep 300; }
echo "flaky: call $n fails"; exit 1
FLAKY
chmod +x "$tmp/flaky.sh"

echo "--- 1. succeeds first time: no retry"
"$retry" 3 5 true; expect "first success" 0 $?

echo "--- 2. fails twice, succeeds on 3rd"
"$retry" 3 5 "$tmp/flaky.sh" "$tmp/c2" 3 fail; expect "retry after failure" 0 $?
expect "ran exactly 3 times" 3 "$(cat "$tmp/c2")"

echo "--- 3. first call hangs (sleep 300), killed by the limit, 2nd succeeds"
start=$(date +%s)
"$retry" 3 3 "$tmp/flaky.sh" "$tmp/c3" 2 hang; expect "retry after hang" 0 $?
elapsed=$(( $(date +%s) - start ))
[ "$elapsed" -lt 30 ] && echo "PASS  hang cut at the limit (${elapsed}s, not 300s)" || { echo "FAIL  took ${elapsed}s"; fail=1; }

echo "--- 4. always fails: stops after exactly 3 attempts, exits non-zero"
"$retry" 3 5 "$tmp/flaky.sh" "$tmp/c4" 99 fail; rc=$?
[ "$rc" != 0 ] && echo "PASS  exit non-zero ($rc)" || { echo "FAIL  exit 0"; fail=1; }
expect "ran exactly 3 times" 3 "$(cat "$tmp/c4")"

echo "--- 5. always hangs (sleep 300): 3 timeouts, then exit 124"
start=$(date +%s)
"$retry" 3 2 sleep 300; expect "all attempts time out" 124 $?
elapsed=$(( $(date +%s) - start ))
[ "$elapsed" -lt 30 ] && echo "PASS  3 x 2s limit took ${elapsed}s" || { echo "FAIL  took ${elapsed}s"; fail=1; }

echo "--- 6. bad arguments"
"$retry" 3 >/dev/null 2>&1; expect "missing command" 2 $?
"$retry" 0 5 true >/dev/null 2>&1; expect "zero attempts" 2 $?

[ "$fail" = 0 ] && echo "ALL PASS" || { echo "SOME FAILED"; exit 1; }
