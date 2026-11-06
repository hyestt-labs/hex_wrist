# PR #19 擴大：apt 重試、clippy --tests、merge-gate --no-mutation

分支 `ci/check-linux-timeout`，head `6b2b90b1`（已 push，未合併）。

## 改了什麼

1. `.github/workflows/compile.yml`：Linux 與 Windows 的 clippy 改成 `cargo clippy --locked --bin hex --tests -- -D warnings`。macOS 不動（規格只要求 Linux 與 Windows）。
2. `scripts/retry-timeout.sh <次數> <秒> <指令…>`：每次嘗試套 `timeout -k 10 <秒>`，失敗或逾時才重試，三次都失敗才以最後一次的退出碼失敗。`compile.yml` 的 Linux、Watch tests 兩個 job 與 `check-linux.yml` 的 `Install native libraries` 改成 `scripts/retry-timeout.sh 3 240 sh -c 'sudo apt-get update && sudo apt-get install -y …'`，步驟層 `timeout-minutes: 10` 保留。
3. `scripts/merge-gate.sh`：新增 `--no-mutation "<原因>"`。有原因就跳過 (c) 並印出 `SKIP  (c) mutation check skipped: <原因>`；沒給、空白或後面接另一個選項都失敗。其他檢查照跑。
4. 測試腳本：`scripts/test-retry-timeout.sh`、`scripts/test-merge-gate.sh`（用假 `gh`，走正式的 `merge-gate.sh`）。

## 本機輸出：重試與逾時（`scripts/test-retry-timeout.sh`）

本機 macOS 沒有 GNU `timeout`，測試時 PATH 前面放了一個 perl 寫的替身（`/tmp/ci19-shim/timeout`，只模擬 `timeout -k K 秒 指令`，逾時退出碼 124）。真正的 GNU `timeout` 只在 CI 的 Linux 上跑過成功路徑（見下）。卡住用 `sleep 300`，限制縮成 2–3 秒以免等 12 分鐘。

```
--- 1. succeeds first time: no retry
retry-timeout: attempt 1/3 (limit 5s): true
retry-timeout: attempt 1 succeeded
PASS  first success (exit 0)
--- 2. fails twice, succeeds on 3rd
retry-timeout: attempt 1/3 (limit 5s): /var/folders/1b/c2p5vjv93xbf9_6w3pxxbq200000gn/T/tmp.2sT4h41iBl/flaky.sh /var/folders/1b/c2p5vjv93xbf9_6w3pxxbq200000gn/T/tmp.2sT4h41iBl/c2 3 fail
flaky: call 1 fails
retry-timeout: attempt 1 failed with exit 1
retry-timeout: attempt 2/3 (limit 5s): /var/folders/1b/c2p5vjv93xbf9_6w3pxxbq200000gn/T/tmp.2sT4h41iBl/flaky.sh /var/folders/1b/c2p5vjv93xbf9_6w3pxxbq200000gn/T/tmp.2sT4h41iBl/c2 3 fail
flaky: call 2 fails
retry-timeout: attempt 2 failed with exit 1
retry-timeout: attempt 3/3 (limit 5s): /var/folders/1b/c2p5vjv93xbf9_6w3pxxbq200000gn/T/tmp.2sT4h41iBl/flaky.sh /var/folders/1b/c2p5vjv93xbf9_6w3pxxbq200000gn/T/tmp.2sT4h41iBl/c2 3 fail
flaky: call 3 ok
retry-timeout: attempt 3 succeeded
PASS  retry after failure (exit 0)
PASS  ran exactly 3 times (exit 3)
--- 3. first call hangs (sleep 300), killed by the limit, 2nd succeeds
retry-timeout: attempt 1/3 (limit 3s): /var/folders/1b/c2p5vjv93xbf9_6w3pxxbq200000gn/T/tmp.2sT4h41iBl/flaky.sh /var/folders/1b/c2p5vjv93xbf9_6w3pxxbq200000gn/T/tmp.2sT4h41iBl/c3 2 hang
flaky: call 1 hangs
retry-timeout: attempt 1 timed out after 3s and was killed
retry-timeout: attempt 2/3 (limit 3s): /var/folders/1b/c2p5vjv93xbf9_6w3pxxbq200000gn/T/tmp.2sT4h41iBl/flaky.sh /var/folders/1b/c2p5vjv93xbf9_6w3pxxbq200000gn/T/tmp.2sT4h41iBl/c3 2 hang
flaky: call 2 ok
retry-timeout: attempt 2 succeeded
PASS  retry after hang (exit 0)
PASS  hang cut at the limit (4s, not 300s)
--- 4. always fails: stops after exactly 3 attempts, exits non-zero
retry-timeout: attempt 1/3 (limit 5s): /var/folders/1b/c2p5vjv93xbf9_6w3pxxbq200000gn/T/tmp.2sT4h41iBl/flaky.sh /var/folders/1b/c2p5vjv93xbf9_6w3pxxbq200000gn/T/tmp.2sT4h41iBl/c4 99 fail
flaky: call 1 fails
retry-timeout: attempt 1 failed with exit 1
retry-timeout: attempt 2/3 (limit 5s): /var/folders/1b/c2p5vjv93xbf9_6w3pxxbq200000gn/T/tmp.2sT4h41iBl/flaky.sh /var/folders/1b/c2p5vjv93xbf9_6w3pxxbq200000gn/T/tmp.2sT4h41iBl/c4 99 fail
flaky: call 2 fails
retry-timeout: attempt 2 failed with exit 1
retry-timeout: attempt 3/3 (limit 5s): /var/folders/1b/c2p5vjv93xbf9_6w3pxxbq200000gn/T/tmp.2sT4h41iBl/flaky.sh /var/folders/1b/c2p5vjv93xbf9_6w3pxxbq200000gn/T/tmp.2sT4h41iBl/c4 99 fail
flaky: call 3 fails
retry-timeout: attempt 3 failed with exit 1
retry-timeout: all 3 attempts failed (last exit 1)
PASS  exit non-zero (1)
PASS  ran exactly 3 times (exit 3)
--- 5. always hangs (sleep 300): 3 timeouts, then exit 124
retry-timeout: attempt 1/3 (limit 2s): sleep 300
retry-timeout: attempt 1 timed out after 2s and was killed
retry-timeout: attempt 2/3 (limit 2s): sleep 300
retry-timeout: attempt 2 timed out after 2s and was killed
retry-timeout: attempt 3/3 (limit 2s): sleep 300
retry-timeout: attempt 3 timed out after 2s and was killed
retry-timeout: all 3 attempts failed (last exit 124)
PASS  all attempts time out (exit 124)
PASS  3 x 2s limit took 6s
--- 6. bad arguments
PASS  missing command (exit 2)
PASS  zero attempts (exit 2)
ALL PASS
rc=0
```

## 本機輸出：merge-gate（`scripts/test-merge-gate.sh`）

```
--- 1. report has FAILED output, no flag: passes
    PR #19 head 0123456789abcdef0123456789abcdef01234567
    ok    Compile / Linux success
    ok    Compile / Watch tests success
    ok    Compile / macOS success
    ok    Compile / Windows x64 success
    ok    check log is for 0123456789abcdef0123456789abcdef01234567
    ok    scripts/check exit=0
    ok    report holds mutation FAILED output
    ok    PR body has an unverified section
    GATE PASS
    exit=0
PASS  1. report has FAILED output, no flag: passes
--- 2. no FAILED output, no flag: fails
    PR #19 head 0123456789abcdef0123456789abcdef01234567
    ok    Compile / Linux success
    ok    Compile / Watch tests success
    ok    Compile / macOS success
    ok    Compile / Windows x64 success
    ok    check log is for 0123456789abcdef0123456789abcdef01234567
    ok    scripts/check exit=0
    FAIL  report has no mutation FAILED output (revert the fix, paste the failing test)
    ok    PR body has an unverified section
    GATE FAIL
    exit=1
PASS  2. no FAILED output, no flag: fails
--- 3. flag with a reason: skips (c), prints the skip and the reason
    PR #19 head 0123456789abcdef0123456789abcdef01234567
    ok    Compile / Linux success
    ok    Compile / Watch tests success
    ok    Compile / macOS success
    ok    Compile / Windows x64 success
    ok    check log is for 0123456789abcdef0123456789abcdef01234567
    ok    scripts/check exit=0
    SKIP  (c) mutation check skipped: CI-only change, no bug fix
    ok    PR body has an unverified section
    GATE PASS
    exit=0
PASS  3. flag with a reason: skips (c), prints the skip and the reason
--- 4. flag without a reason: fails
    PR #19 head 0123456789abcdef0123456789abcdef01234567
    ok    Compile / Linux success
    ok    Compile / Watch tests success
    ok    Compile / macOS success
    ok    Compile / Windows x64 success
    ok    check log is for 0123456789abcdef0123456789abcdef01234567
    ok    scripts/check exit=0
    FAIL  --no-mutation needs a reason: --no-mutation "<why no bug fix to mutate>"
    ok    PR body has an unverified section
    GATE FAIL
    exit=1
PASS  4. flag without a reason: fails
--- 5. flag with an empty reason: fails
    PR #19 head 0123456789abcdef0123456789abcdef01234567
    ok    Compile / Linux success
    ok    Compile / Watch tests success
    ok    Compile / macOS success
    ok    Compile / Windows x64 success
    ok    check log is for 0123456789abcdef0123456789abcdef01234567
    ok    scripts/check exit=0
    FAIL  --no-mutation needs a reason: --no-mutation "<why no bug fix to mutate>"
    ok    PR body has an unverified section
    GATE FAIL
    exit=1
PASS  5. flag with an empty reason: fails
--- 6. flag followed by another option: fails
    PR #19 head 0123456789abcdef0123456789abcdef01234567
    ok    Compile / Linux success
    ok    Compile / Watch tests success
    ok    Compile / macOS success
    ok    Compile / Windows x64 success
    ok    check log is for 0123456789abcdef0123456789abcdef01234567
    ok    scripts/check exit=0
    FAIL  --no-mutation needs a reason: --no-mutation "<why no bug fix to mutate>"
    ok    PR body has an unverified section
    GATE FAIL
    exit=1
PASS  6. flag followed by another option: fails
--- 7. flag does not hide the other checks (bad check log): fails
    PR #19 head 0123456789abcdef0123456789abcdef01234567
    ok    Compile / Linux success
    ok    Compile / Watch tests success
    ok    Compile / macOS success
    ok    Compile / Windows x64 success
    ok    check log is for 0123456789abcdef0123456789abcdef01234567
    FAIL  scripts/check log does not end with exit=0
    SKIP  (c) mutation check skipped: CI-only change
    ok    PR body has an unverified section
    GATE FAIL
    exit=1
PASS  7. flag does not hide the other checks (bad check log): fails
ALL PASS
rc=0
```

## CI 輸出（run 42200634115，head `6b2b90b1`）

四個 Compile job 全部成功：Linux、Watch tests、macOS、Windows x64；`hex-agent-policy` 成功。

- Linux 與 Windows 的 clippy 步驟確實跑了 `--tests`，沒有警告：
```
Windows x64  Run cargo clippy --locked --bin hex --tests -- -D warnings
Windows x64  Finished `dev` profile [unoptimized + debuginfo] target(s) in 1m 03s
Linux        Run cargo clippy --locked --bin hex --tests -- -D warnings
Linux        Finished `dev` profile [unoptimized] target(s) in 31.37s
```
  所以 Windows 的 `--tests` 不用拆出去，兩邊一起改。
- Linux runner 用真正的 GNU `timeout` 跑 apt 步驟（兩個 job 都第一次就成功）：
```
Watch tests  retry-timeout: attempt 1/3 (limit 240s): sh -c
Watch tests  retry-timeout: attempt 1 succeeded
Linux        retry-timeout: attempt 1/3 (limit 240s): sh -c
Linux        retry-timeout: attempt 1 succeeded
```

## 沒驗證的項目

- 真正的 GNU `timeout` 在 apt 卡住時殺掉並重試：CI 只走過成功路徑；卡住路徑只用本機 perl 替身驗證。
- `check-linux.yml` 改過的步驟沒有跑（它只在 Release 的 check／stable 或手動執行時跑）；語法用 `ruby -ryaml` 讀出 `run` 腳本後 `bash -n` 檢查過。
- 步驟層 `timeout-minutes: 10`（600 秒）小於三次逾時的最壞情況（3 × 240 秒 + 殺程序寬限）：兩次都卡住時，第三次會被步驟層逾時中斷，步驟仍然失敗，不會多跑。
- apt 被殺在 dpkg 中途時，下一次嘗試是否因 lock 或半裝狀態失敗：沒測，推測 runner 是全新環境，影響小。
- macOS 的 clippy 沒加 `--tests`，沒檢查它有沒有既有警告。
- `.github/` 的修改依 AGENTS.md 需要 Dax 核可。
- `merge-gate.sh` 只對假 `gh` 測過，沒有對真實 PR 跑過。`--no-mutation` 的原因必須緊接在旗標後面。
