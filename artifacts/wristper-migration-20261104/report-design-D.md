# 報告：單元 D 設計（授權與免費額度）

工作目錄：`/Users/glenhsu/orca/workspaces/hex_wrist/d-design`，分支 `wristper/unit-d-design`（從 `origin/dev` `31c8a7c5`）。
產出：`docs/plans/unit-d-design.md`（198 行）。只有文件，沒有程式、沒有改 UI。

## 改了什麼

1. 新增 `docs/plans/unit-d-design.md`：行為規格（21 列，逐列標沿用或新決定）、儲存、模組接點、離線影響表、安全、測試計畫、4 個 PR 的拆分與行數預算、風險、6 個待決問題。
2. 新增本報告。

## 讀過的來源

- Hex：`docs/plans/wristper-on-hex.md`（A、C、D、E、G、J 節）、`src/watch/refusal.rs`、`src/watch/service/{routes,envelope}.rs`、`src/watch/delivery.rs`、`src/watch/pairing/keychain.rs`、`src/parakeet.rs` `finish_output`、`src/recognition.rs`、`src/recognition/gated_press.rs`、`src/dictation/audio.rs`、`src/app_paths.rs`、`src/rune_ui/{deep_link,story_link,platform}.rs`、`src/dev_build.rs`、`scripts/code-shape.py`、`scripts/file-size-exceptions.txt`。
- Wristper（`sim-hooks` `cf8c14b`，唯讀）：`Licensing.swift`、`LicensingStore.swift`、`PolarClient.swift`、`ReceiverLicensing.swift`、`LicensePlan.swift`、`FreeLimitNotice.swift`、`KeychainSecretStore.swift`、`ReceiverService.swift` 的擋點、`EmbeddedReceiver.swift`、`MacMicInput.swift`、`relay/worker.js`，以及 6 個授權測試檔的測試名稱。
- 協調資料：`PLAN.md`、`TRAIL.md`、`spec-common.md`、`wristper-setup-licensing.md`（在 `migration_to_hex` worktree）。

## 主要發現（原始碼查證）

1. macOS 資料根目錄是 `~/Library/Application Support/Wristper`（`src/app_paths.rs:39`），和舊 App 同一個資料夾。技術方案 G.1 的「同名就地升級」會改寫舊 `licensing.json`。設計改成新檔 `licensing-state.json`，舊檔只讀。
2. 舊授權碼不在鑰匙圈。Wristper 把完整授權碼明文放在 `licensing.json`（`LicensingStore.swift:26-30`）。`dev.wristper.mac.device*` 只放手錶密鑰與遷移記號。
3. 預設快捷鍵是單按 Option，Option+e 也會啟動狀態機。所以「額度用完」的錯誤音要等 Hex 擁有按壓才播，沿用 C1 的 `gated_press.rs`。
4. Wristper 的 `audio()` 先擋授權再去重（`ReceiverService.swift:893`），第 10 段之後重送同 ID 會拿 402。設計改成先去重。
5. `/health` 的 `licensing` 在 Swift 有 7 個鍵（`Licensing.swift` `LicensingStatus.json`），技術方案只列 5 個；Hex 目前寫死 `licensed`（`src/watch/service/routes.rs:187-195`）。
6. relay `/checkout` 不讀 `reference_id`（`worker.js:180-214`），它不是購買與裝置的綁定。
7. `src/parakeet.rs` 非測試行數 1386，硬上限 1500。計數接點最多加 20 行。
8. `DeliveryOutcome` 沒有額度結果（`src/watch/delivery.rs:20`），D2 要新增一種。
9. `commit()` 在 `verify()` 之前執行（`src/paste.rs:476` `commit_prepared_paste`，讀原始碼確認，不是推測）。所以計數不能放在 commit closure 裡，否則 target-changed 也會被扣額；設計改成貼上前 `admit()`、結果確定送達後才 `record_delivery()`。
10. e2e 驗證用 Wristper Test；啟用授權後每天第 11 次起會拿 402。設計要求 e2e 用暫存資料夾或 dev 專用開關。

## 待使用者決定的問題（每題附建議）

1. `local_api` 的 `/transcriptions`、`/streams`：建議免費版回 402，付費才能用。
2. 會議、Annotate、History 的「Transcribe again」：建議不算額度。
3. 舊授權：建議自動、唯讀匯入，不先問。
4. 授權碼：建議放鑰匙圈（`dev.wristper.mac.license`），不照 Wristper 放明文檔。
5. 深連結遇到已有有效授權：建議先問，不靜默替換。
6. Mac 快捷鍵 20 秒：建議到時自動停止並貼上，不在使用者不知情時只截轉錄。

以上六題不回覆就照建議做。

協調者可自行決定、但文件已寫出理由的技術選擇：手寫美國太平洋時間規則（不加 tz 相依）、先去重再擋額度、commit 點嚴格扣額、損毀狀態檔時付費使用者靠鑰匙圈憑證進入寬限、匯入時先寫鑰匙圈再寫狀態檔、程式啟動的聽寫在額度用完時立刻拒絕。

## 檢查輸出

```
$ wc -l docs/plans/unit-d-design.md
     198 docs/plans/unit-d-design.md
$ git diff --check
（無輸出）
```

## 沒驗證的項目

1. 沒有建置、沒有跑測試、沒有連 relay 或 Polar。所有行為都來自讀原始碼。
2. Polar 後台的啟用上限是否真的設為 3，沒查（要使用者看後台）。
3. Polar sandbox 與正式版的 license key 端點行為是否相同，沒查（設計的真實驗證第 3 步會確認）。
4. 新增 `hex license` 的 `ControlRequest` 是否要升 `CONTROL_VERSION`，留給 D3 依 `docs/cli.md` 判斷。
5. 「鎖內只做記憶體運算」是否被協調者接受為符合「`watch` 執行緒只等 inbox」，需要審查確認。
