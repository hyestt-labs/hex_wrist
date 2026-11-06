# merge-gate.sh 兩個修正：Compile 只要有一個 run 四個 job 都成功；(c) 接受表格儲存格的 FAIL

分支 `ci/gate-any-run`（從 origin/dev `66e3effa`），未合併。

## 改了什麼

`scripts/merge-gate.sh`
1. 檢查 (a)：`gh run list --commit <sha> --limit 10` 取該 SHA 最近 10 個 Compile run。對每個 run 呼叫一次 `gh api …/runs/<id>/jobs`，印成 `名稱<TAB>conclusion`，逐名比對 `Linux`、`Watch tests`、`macOS`、`Windows x64` 是否為 `success`。第一個四個都成功的 run 通過並印出 `ok    Compile run <id> has …`；沒有就失敗，並逐 run 列出缺哪些 job。比對只在單一 run 內做，不把不同 run 的 job 拼起來。沒有任何 run 仍印 `no Compile run for <sha>`。
2. 檢查 (c) 的 regex 多收一種：反引號後接 `FAIL ` 加內容（`` `FAIL[[:space:]]+[^`]*[[:alnum:]_] ``）。行首 `FAIL`、`test result: FAILED`、`... FAILED` 照舊。裸字 `FAIL`（沒反引號、或反引號內只有 `FAIL`）不算。

`scripts/test-merge-gate.sh`：假 `gh` 改成讀 `$SCEN/runs`（新到舊）與 `$SCEN/jobs-<run id>`。原 7 案不變；新增 7 案（8–14）。測試輔助函式把輸出攤平成一行再比對，好讓多行失敗訊息可以斷言。

| 案 | 修正 | 方向 | 情境 |
|---|---|---|---|
| 8 | (a) | 必須發生 | 最新 run 的 macOS 失敗、次新 run 的 Windows 取消、最舊 run 四個都成功 → 通過，印 run 1 |
| 9 | (a) | 必須不發生 | 兩個 run 各缺一個 job（macOS 失敗、Windows skipped），合起來四個都有 → 失敗，列出兩個 run 各缺什麼 |
| 10 | (a) | 補充 | 缺 Linux 的 run 被跳過，用較舊的 run |
| 11 | (a) | 補充 | 沒有 Compile run → 失敗 |
| 12 | (c) | 必須發生 | 表格儲存格 `` `FAIL [ 0.038s] hex::bin/hex some::a_test` `` → 通過 |
| 13 | (c) | 必須不發生 | 表格內只有裸字 `FAIL` 與 `` `FAIL` `` → 失敗 |
| 14 | (c) | 必須不發生 | 完全沒有 FAIL → 失敗 |

兩個測試都走正式的 `scripts/merge-gate.sh`，沒有只存在於測試輔助器的邏輯（假的只有 `gh`）。

## 輸出原文

`scripts/test-merge-gate.sh`（修正後）：
```
PASS  1. report has FAILED output, no flag: passes
PASS  2. no FAILED output, no flag: fails
PASS  3. flag with a reason: skips (c), prints the skip and the reason
PASS  4. flag without a reason: fails
PASS  5. flag with an empty reason: fails
PASS  6. flag followed by another option: fails
PASS  7. flag does not hide the other checks (bad check log): fails
PASS  8. newest run fails, an earlier run has all four jobs: passes and names that run
PASS  9. four jobs success but spread over two runs (each missing one): fails
PASS  10. a run missing Linux is skipped, the older run is used
PASS  11. no Compile run at all: fails
PASS  12. FAIL line inside a table cell: passes
PASS  13. table with no FAIL test output (bare word FAIL only): fails
PASS  14. no FAIL anywhere, as in case 2: still fails
ALL PASS
```

`scripts/check`（head `b86b6b28`，`sha=` 開頭、`exit=` 結尾的 log）結尾：
```
    Finished `test` profile [unoptimized + debuginfo] target(s) in 34.46s
────────────
 Nextest run ID a9cf0981-dbde-4f54-a164-8a7871fbb1de with nextest profile: default
    Starting 2636 tests across 6 binaries (32 tests skipped)
────────────
     Summary [  35.001s] 2636 tests run: 2636 passed, 32 skipped
exit=0
```

## 變異驗證（commit 後在原位改 `scripts/merge-gate.sh`，每次跑完用 `git checkout` 還原；以下只列未通過的案）

M1a：檢查 (a) 還原成 origin/dev 版（只看最新一個 run）。
```
FAIL  1. report has FAILED output, no flag: passes (wanted exit 0 and /ok    report holds mutation FAILED output.*|GATE PASS/)
FAIL  3. flag with a reason: skips (c), prints the skip and the reason (wanted exit 0 and /SKIP  \(c\) mutation check skipped: CI-only change, no bug fix/)
FAIL  8. newest run fails, an earlier run has all four jobs: passes and names that run (wanted exit 0 and /ok    Compile run 1 has Linux, Watch tests, macOS, Windows x64 all success/)
FAIL  9. four jobs success but spread over two runs (each missing one): fails (wanted exit 1 and /FAIL  no Compile run among the latest 10 .*run 3: not success: macOS;.*run 2: not success: Windows x64;/)
FAIL  10. a run missing Linux is skipped, the older run is used (wanted exit 0 and /ok    Compile run 1 /)
FAIL  12. FAIL line inside a table cell: passes (wanted exit 0 and /ok    report holds mutation FAILED output/)
SOME FAILED
```
（1、3、12 一併失敗，因為舊版的 `gh api` 呼叫對新假 `gh` 讀不到 `success`，整個 gate 在 (a) 失敗；案 8、9、10 是這次修正要擋的行為。）

M1b：錯誤做法：把各 run 的成功 job 累積起來拼湊（`seen` 變數跨 run 累加）。
```
FAIL  8. newest run fails, an earlier run has all four jobs: passes and names that run (wanted exit 0 and /ok    Compile run 1 has Linux, Watch tests, macOS, Windows x64 all success/)
FAIL  9. four jobs success but spread over two runs (each missing one): fails (wanted exit 1 and /FAIL  no Compile run among the latest 10 .*run 3: not success: macOS;.*run 2: not success: Windows x64;/)
SOME FAILED
```
（案 8 失敗是因為拼湊在 run 2 就湊齊，印出 run 2 而不是 run 1；案 9 失敗是因為拼湊讓 gate 通過。）

M2a：檢查 (c) regex 還原成只收行首 `FAIL`、`test result: FAILED`、`... FAILED`。
```
FAIL  12. FAIL line inside a table cell: passes (wanted exit 0 and /ok    report holds mutation FAILED output/)
SOME FAILED
```

M2b：錯誤做法：任何位置出現裸字 `FAIL` 就通過。
```
FAIL  13. table with no FAIL test output (bare word FAIL only): fails (wanted exit 1 and /FAIL  report has no mutation FAILED output/)
SOME FAILED
```

還原後 `git status` 乾淨，`scripts/test-merge-gate.sh` 結尾 `ALL PASS`。

## 未驗證

- 沒對真的 `gh`／GitHub 跑過新 (a)：`gh run list --limit 10 --json databaseId -q '.[].databaseId'` 與 `gh api … --jq '.jobs[] | "\(.name)\t\(.conclusion // "none")"'` 只在假 `gh` 下測。`gh api` 預設一頁 30 個 job，Compile 每 run 只有 4 個，沒做分頁。
- 沒測同一 run 重跑（re-run attempt）時 jobs API 回傳哪一次 attempt 的 job；推測預設回最新 attempt，要確認需在真實重跑過的 run 上查。
- 「最近 10 個」依 `gh run list` 預設的新到舊順序，沒另外排序。
- 這個 PR 自己的 gate：Compile 對只動 `scripts/` 與 `artifacts/` 的 PR 可能被 path filter 跳過（見先前 PR #15 的經驗），需手動 dispatch 或沿用 PR 說明。
- (c) 的新 regex 會收「反引號後接 `FAIL ` 與任意含字母數字的文字」，不驗證那段文字真的是測試路徑，也不驗證它真的來自還原修正後的輸出。
