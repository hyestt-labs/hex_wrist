#!/usr/bin/env bash
# Run a command with a hard time limit per attempt and retry it when it fails.
# Usage: scripts/retry-timeout.sh <attempts> <seconds> <command> [args...]
# Each attempt runs under `timeout -k 10 <seconds>`, so a hung command is killed
# and tried again. The exit code is 0 on the first success, else the last
# attempt's code (124 means the final attempt timed out). CI uses it for
# `apt-get`, which sometimes hangs for 20+ minutes on a runner.
# scripts/test-retry-timeout.sh exercises it.
set -u

[ $# -ge 3 ] || { echo "usage: $0 <attempts> <seconds> <command> [args...]" >&2; exit 2; }
attempts=$1 seconds=$2
shift 2
case "$attempts$seconds" in *[!0-9]* | "") echo "attempts and seconds must be whole numbers" >&2; exit 2 ;; esac
[ "$attempts" -ge 1 ] && [ "$seconds" -ge 1 ] || { echo "attempts and seconds must be at least 1" >&2; exit 2; }

if command -v timeout >/dev/null 2>&1; then limiter=timeout
elif command -v gtimeout >/dev/null 2>&1; then limiter=gtimeout
else echo "retry-timeout: neither timeout nor gtimeout is installed" >&2; exit 2; fi

rc=0
n=1
while [ "$n" -le "$attempts" ]; do
  echo "retry-timeout: attempt $n/$attempts (limit ${seconds}s): $*"
  "$limiter" -k 10 "$seconds" "$@"
  rc=$?
  if [ "$rc" = 0 ]; then
    echo "retry-timeout: attempt $n succeeded"
    exit 0
  fi
  if [ "$rc" = 124 ] || [ "$rc" = 137 ]; then
    echo "retry-timeout: attempt $n timed out after ${seconds}s and was killed"
  else
    echo "retry-timeout: attempt $n failed with exit $rc"
  fi
  n=$((n + 1))
done
echo "retry-timeout: all $attempts attempts failed (last exit $rc)" >&2
exit "$rc"
