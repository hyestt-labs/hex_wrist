# 報告：單元 D 設計（授權與免費額度）

工作目錄：`/Users/glenhsu/orca/workspaces/hex_wrist/d-design`，分支 `wristper/unit-d-design`（從 `origin/dev` `31c8a7c5`）。
產出：`docs/plans/unit-d-design.md`（221 行，2026-11-06 依 Terra 事實查核修訂）。只有文件，沒有程式、沒有改 UI。

## 改了什麼

1. 新增 `docs/plans/unit-d-design.md`：行為規格（21 列，逐列標沿用或新決定）、儲存、模組接點、離線影響表、安全、測試計畫、4 個 PR 的拆分與行數預算、風險、6 個待決問題。
2. 新增本報告。

## 讀過的來源

- Hex：`docs/plans/wristper-on-hex.md`（A、C、D、E、G、J 節）、`src/watch/refusal.rs`、`src/watch/service/{routes,envelope}.rs`、`src/watch/delivery.rs`、`src/watch/pairing/keychain.rs`、`src/parakeet.rs` `finish_output`、`src/recognition.rs`、`src/recognition/gated_press.rs`、`src/dictation/audio.rs`、`src/app_paths.rs`、`src/rune_ui/{deep_link,story_link,platform}.rs`、`src/dev_build.rs`、`scripts/code-shape.py`、`scripts/file-size-exceptions.txt`。
- Wristper（`sim-hooks` `cf8c14b`，唯讀）：`Licensing.swift`、`LicensingStore.swift`、`PolarClient.swift`、`ReceiverLicensing.swift`、`LicensePlan.swift`、`FreeLimitNotice.swift`、`KeychainSecretStore.swift`、`ReceiverService.swift` 的擋點、`EmbeddedReceiver.swift`、`MacMicInput.swift`、`relay/worker.js`，以及 6 個授權測試檔的測試名稱。
- 協調資料：`PLAN.md`、`TRAIL.md`、`spec-common.md`、`wristper-setup-licensing.md`（在 `migration_to_hex` worktree）。

## 主要發現（原始碼查證）

1. （查核後修正）只有未覆寫的正式版預設與舊 Swift App 用同一個資料夾 `~/Library/Application Support/Wristper`；測試版預設 `Wristper Test`，`HEX_APPLICATION_SUPPORT_DIR` 可覆寫（`src/app_paths.rs:15-18,31-42`）。設計改成新檔 `licensing-state.json`，舊授權從固定的舊來源路徑唯讀匯入。
2. 舊授權碼不在鑰匙圈。Wristper 把完整授權碼明文放在 `licensing.json`（`LicensingStore.swift:26-30`）。`dev.wristper.mac.device*` 只放手錶密鑰與遷移記號。
3. 預設快捷鍵是單按 Option，Option+e 也會啟動狀態機。所以「額度用完」的錯誤音要等 Hex 擁有按壓才播，沿用 C1 的 `gated_press.rs`。
4. Wristper 的 `audio()` 先擋授權再去重（`ReceiverService.swift:893`），第 10 段之後重送同 ID 會拿 402。設計改成先去重。
5. `/health` 的 `licensing` 在 Swift 有 7 個鍵（`Licensing.swift` `LicensingStatus.json`），技術方案只列 5 個；Hex 目前寫死 `licensed`（`src/watch/service/routes.rs:187-195`）。
6. relay `/checkout` 不讀 `reference_id`（`worker.js:180-214`），它不是購買與裝置的綁定。
7. `src/parakeet.rs` 非測試行數 1386，硬上限 1500。計數接點最多加 20 行。
8. `DeliveryOutcome` 沒有額度結果（`src/watch/delivery.rs:20`），D2 要新增一種。
9. `commit()` 在 `verify()` 之前執行（`src/paste.rs:476` `commit_prepared_paste`，讀原始碼確認，不是推測）。所以計數不能放在 commit closure 裡，否則 target-changed 也會被扣額；設計改成貼上前 `admit()`、結果確定送達後才 `record_delivery()`。
10. e2e 驗證用 Wristper Test；啟用授權後每天第 11 次起會拿 402。設計要求 e2e 用暫存資料夾或 dev 專用開關。

## 依 Terra 事實查核的修正（2026-11-06）

依據：`/Users/glenhsu/orca/workspaces/hex_wrist/d-check/artifacts/wristper-migration-20261104/terra-factcheck-D.md`（結論「設計有事實錯誤」）。文件內每處修正都標「查核」並附 Swift 檔:行。

1. 資料根目錄：刪掉「新舊 App 共用」。改寫成正式版預設 `Wristper`、測試版預設 `Wristper Test`、可被環境變數覆寫；遷移明定舊來源路徑 `~/Library/Application Support/Wristper/licensing.json`，只在未覆寫的正式版自動找（§0 第 4 點、§2 樹狀圖與第 3 點）。
2. 送達集合：補上 `saved`（`TrialStore.swift:25-27`；`ReceiverService.swift:1367-1379`）；新增第 1b 列寫 Agent 進入等待確認就計數（`ReceiverService.swift:1451-1475`），延到第二階段標為新決定。我另外查到 Hex 的 `DeliveryOutcome` 目前沒有產生 `saved` 的路徑（`src/watch/delivery.rs:20`），`State::Saved` 只來自舊紀錄（`src/watch/recording.rs:119,184`）。
3. `licensing.json` 格式：改成頂層選填 `license`、`usage`，`license` 必填 5 個、選填 3 個；目前編碼器不輸出 `trial`；Swift 讀到損毀檔會改名成 `.bak` 並回空快照（`LicensingStore.swift:15-23`）。
4. 鑰匙圈：寫明 `dev.wristper.mac.license`、`dev.wristper.mac.test.license`、account `license` 是新設計的名稱；Swift 的鑰匙圈只有裝置密鑰與 `.state` 遷移記號（`KeychainSecretStore.swift:16-17,49-57,62-82`）。
5. 寬限與撤銷：恰 14 天仍有效；`expiresAt` 到期與 `revokedAt` 存在都算撤銷；只有 404 或 200 帶 `revoked`／`disabled` 觸發撤銷；403、422、5xx 不改狀態，但 14 天後仍 overdue（第 15、17 列、離線表）。
6. 試用：改成 relay 仍實作 `/trial/start`、`/trial/export`，本單元不呼叫、不改；七天試用的歷史無法從原始碼確認（第 18 列）。
7. 推測項：Polar 上限 3 台、sandbox 與正式版相同、relay 不留授權資料、改時鐘的影響，都改標推測並寫確認方法（只用 sandbox、只讀 repo、注入時鐘的單元測試）（第 16 列、§4 第 3 點、§6 風險 4–6）。
8. 深連結：寫明 Swift 現在直接 activate、不檢查既有授權；checkout 後只有查到授權碼才導向帶 key 的連結，否則導向感謝頁（第 13 列、§3 第 7 點、§4 第 3 點）。

## 待使用者決定的問題（分組，每題附建議）

A 組：必須使用者明確回答「是」，不得靜默採用預設。

1. A1 舊授權自動匯入：建議正式版自動、唯讀。未同意前只做 `--dry-run --from`。
2. A2 授權碼放鑰匙圈（新 service）或照 Wristper 放檔案：建議鑰匙圈。
3. A3 深連結遇到已有授權：建議先問，不靜默替換。
4. A4 用 Polar sandbox 做真實驗證：建議可以，只用 sandbox，不碰正式服務。

B 組：不回覆就照建議做。

1. B1 `local_api` 的 `/transcriptions`、`/streams`：建議免費版回 402。
2. B2 會議、Annotate、Transcribe again：建議不算額度。
3. B3 Mac 快捷鍵 20 秒：建議自動停止並貼上。
4. B4 第二階段 Agent 計數時點：建議照 Swift，進入等待確認就計數。

協調者可自行決定、但文件已寫出理由的技術選擇：手寫美國太平洋時間規則（不加 tz 相依）、先去重再擋額度、貼上前檢查加送達後計數、損毀狀態檔時付費使用者靠鑰匙圈憑證進入寬限、匯入時先寫鑰匙圈再寫狀態檔、程式啟動的聽寫在額度用完時立刻拒絕。

## 檢查輸出

```
$ wc -l docs/plans/unit-d-design.md
     221 docs/plans/unit-d-design.md
$ git diff --check
（無輸出）
```

## 沒驗證的項目

1. 沒有建置、沒有跑測試、沒有連 relay 或 Polar。所有行為都來自讀原始碼。
2. Polar 後台的啟用上限是否真的設為 3，沒查（要使用者看後台）。
3. Polar sandbox 與正式版的 license key 端點行為是否相同，沒查（設計的真實驗證第 3 步會確認）。
4. 新增 `hex license` 的 `ControlRequest` 是否要升 `CONTROL_VERSION`，留給 D3 依 `docs/cli.md` 判斷。
5. 「鎖內只做記憶體運算」是否被協調者接受為符合「`watch` 執行緒只等 inbox」，需要審查確認。
6. relay 是否留存授權資料、改系統時鐘的實際影響，都還是推測（確認方法見設計 §4 第 3 點）。
7. 本次修訂只重讀了查核報告引用的 Swift 行；我自己另外核對了 `TrialStore.swift:25-27`、`ReceiverService.swift:1365-1380`、`EmbeddedReceiver.swift:14-16`、`worker.js:107-112`，其餘行號照查核報告引用，沒有逐行重讀。
