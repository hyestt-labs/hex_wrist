# 單元 D 設計：買斷授權與免費額度（macOS）

狀態：設計草案，2026-11-06，只有文件，沒有程式。基準 `dev` `31c8a7c5`；Wristper `sim-hooks` `cf8c14b`（唯讀）。
名稱：技術方案 `wristper-on-hex.md` 第 A 節的「單元 E：授權與額度」就是本單元。本文件推翻該文件第 C、G 節的三處，見 §0 第 4 點。
標記：「沿用」＝與 Wristper 原始碼相同；「新決定」＝本文件新定，需使用者或協調者確認；「推測」＝沒查證，附確認方法。
修訂：2026-11-06 依 Terra 事實查核 `artifacts/wristper-migration-20261104/terra-factcheck-D.md`（在 `d-check` worktree）修正 8 處，修正處標「查核」。Swift 路徑相對 `wristper/sim-hooks`（`cf8c14b`）。

## 0. 結論

1. 額度只在 Mac 計數（沿用）。一段＝一次送達（`pasted`、`unverified`、`copied`、`saved`，見 §1 第 1 列）；每天 10 段、每段 20 秒；`America/Los_Angeles` 午夜重設；手錶與 Mac 快捷鍵合計（沿用，`Licensing.swift:74-77,123-151`）。
2. 授權照搬 Polar customer-portal 的 activate／validate／deactivate（沿用，`PolarClient.swift:28-81`）。離線寬限 14 天（恰 14 天仍有效）；驗證回 404 或 200 帶 `revoked`／`disabled` 會撤銷，本機 `expiresAt` 到期也算撤銷（沿用，見 §1 第 15、17 列）。
3. 新模組 `src/licensing/` 是唯一的方案判定點。`watch` 執行緒與 output worker 只讀記憶體快照；檔案與網路由 `licensing` 執行緒處理。
4. 推翻技術方案三處：
   - G.1「同名同資料夾就地升級」：舊 Swift App 固定用 `~/Library/Application Support/Wristper`（`EmbeddedReceiver.swift:14-16`）。Hex 正式版預設也選 `Wristper`，所以未覆寫時會讀寫到同一個資料夾；測試版預設是 `Wristper Test`，`HEX_APPLICATION_SUPPORT_DIR` 可覆寫兩者（`src/app_paths.rs:15-18,31-42`）。新狀態改寫 `licensing-state.json`；舊授權從明確的「舊來源路徑」唯讀匯入，見 §2 第 3 點（查核：`terra-factcheck-D.md`）。
   - G.9「按下時播錯誤音」：預設快捷鍵是單按 Option，Option+e 也會啟動狀態機。改成沿用 `src/recognition/gated_press.rs`：Hex 擁有這次按壓時才出聲。
   - D.13 `/health` 的 `licensing`：照 Swift `LicensingStatus.json` 的 7 個鍵，不是技術方案列的 5 個。
5. 拆成 4 個 PR，都在 G2 之後：D1 純邏輯與儲存、D2 擋點與計數、D3 Polar 與 CLI 與深連結、D4 介面。

## 1. 行為規格

| # | 項目 | 規格 | 來源 |
|---|---|---|---|
| 1 | 單位 | 一段＝一次送達：手錶 `pasted`、`unverified`、`copied`、`saved`；Mac 的 Paste、RawPaste、Print、RawPrint。`suspect`、`failed`、`target-changed`、`not-editable`、`sensitive` 不算。`saved`（只存紀錄、不貼上）在 Swift 也計數；Hex 的 `DeliveryOutcome` 目前沒有產生 `saved` 的路徑（`src/watch/delivery.rs:20`），日後加了也要計數 | 沿用 `TrialStore.swift:25-27`；`ReceiverService.swift:1367-1379,1406-1411`（查核：`terra-factcheck-D.md`） |
| 1b | Agent 流程 | Swift 在 Agent 錄音進入「等待確認」時就計一段，不等 Agent 執行（`ReceiverService.swift:1451-1475`）。第一階段 Hex 對帶 route 的錄音回 400 `invalid-route`，不會走到這裡；第二階段照 Swift 計數或改規則，屆時再決定 | 計數時點沿用，延後到第二階段是新決定（需使用者確認）（查核：`terra-factcheck-D.md`） |
| 2 | 上限 | 10 段／天、20 秒／段 | 沿用 `Licensing.swift:74-75` |
| 3 | 重設 | `America/Los_Angeles` 午夜，跨夏令時間 | 沿用 `Licensing.swift:123-136` |
| 4 | 誰計數 | 每台 Mac 各自計數；relay 不計。前提是每台 Mac 各自保存自己的資料夾，共用或覆寫的資料夾不在此保證內 | 沿用（relay 沒有額度路由，`worker.js:74-131`）（查核：`terra-factcheck-D.md`） |
| 5 | 鎖住時仍開放 | `/health`、`/status`、`/enter`、`/undo`、Paste Last 不被額度拒絕；都不計數。`/status` 的測試只證明沒回 402（未知 ID 仍回 404） | 沿用 `ReceiverLicensingTests.swift:91-98,113-134`（測試存在，本次沒有跑）（查核：`terra-factcheck-D.md`） |
| 6 | 手錶擋點順序 | 驗證身分 → 426 → 去重 → 402。已送達的同 ID 重送一律回 `duplicate:true` | 426 先於 402 沿用；去重先於 402 是新決定（Wristper `audio()` 先擋授權，第 10 筆後重送會拿 402） |
| 7 | 手錶看到什麼 | 用既有 `Refusal::DailyLimit`（`src/watch/refusal.rs:20`）：402 `{"ok":false,"error":"license_required","reason":"daily_limit_reached","resets_at"}`，E2EE 時密封在信封內。不新增錯誤碼 | 沿用 `ReceiverLicensing.swift:4-10` |
| 8 | commit 點競態 | `/focus` 與音檔入口各檢查一次；output worker 貼上前再檢查一次，額度已滿就不貼上。手錶紀錄 `failed`＋`license_required`＋`daily_limit_reached`，回 402 | 沿用 `ReceiverService.swift:1355-1359` |
| 9 | 被拒文字的去處 | 因額度被拒的文字不進 Paste Last、不進 History | 新決定（否則 Paste Last 可繞過額度） |
| 10 | 手錶 20 秒 | 解碼後只取前 320 000 個樣本（20 s × 16 kHz），紀錄 `truncated:true` | 沿用 `SpeechEngine.swift:212-217`；`ReceiverService.swift:1285-1310`。Swift 的 Mac PCM 另在解碼前依 byte 截斷（`1285-1288`），Hex 不需要（查核：`terra-factcheck-D.md`） |
| 11 | Mac 快捷鍵 20 秒 | 整段音訊（含 450 ms 預錄）到 20 s 時當程式停止：結束 capture、播停止音、轉錄並貼上；狀態機 `suspend`，之後放開鍵不開始新錄音。Toggle 與鎖定模式下，自動停止後的下一次按壓會開始新錄音（預期行為） | Wristper 也會自動停止（`MacMicInput.swift:685-698`）；接到 Hex 的方式是新決定 |
| 12 | Mac 快捷鍵額度用完 | `Start` 不開始錄音、不出聲。Hex 擁有按壓時（超過門檻、Toggle 寬限、鎖定）才播錯誤音並顯示 HUD「今日免費額度已用完」 | 新決定（Wristper 的快捷鍵是 ⌥Space，Hex 預設是單按 Option） |
| 12b | 程式啟動的聽寫 | 額度用完時，`hex dictation start|toggle` 與 `/dictations` 立刻拒絕，回錯誤碼 `daily-limit-reached`（附 `resets_at`），不開麥克風 | 新決定（否則會錄完才在貼上前丟掉） |
| 13 | 買斷啟用 | 按鈕打開 `https://relay.wristper.dev/checkout?products=e571f4c4-…&reference_id=<裝置雜湊>`；付款後，relay 只有在驗證 checkout 成功並查到授權碼時才 302 到 `wristper://activate?license_key=<key>`，否則導向 `https://wristper.dev/thanks`，使用者手動貼收據信裡的授權碼 | 沿用 `LicensingStore.swift:48-51,116-122`（組織 ID、結帳網址、客戶入口）；`worker.js:33-34,242-264`（查核：`terra-factcheck-D.md`）。`reference_id` 沿用，但 relay 的 `checkoutStart` 沒讀它（`worker.js:180-214`），不能當作綁定 |
| 14 | 驗證 | 啟動後 5 s 檢查一次，之後每小時；距上次驗證滿 24 h 才呼叫 Polar | 沿用 `LicensingStore.swift:209-224` |
| 15 | 離線 | 驗證時網路錯誤、403、422、5xx 都不改狀態（不撤銷、也不更新驗證時間）。距上次成功驗證恰 14 天仍有效，超過才退回免費（`overdue`），之後驗證成功即恢復。所以這些錯誤不代表能一直維持付費 | 沿用 `Licensing.swift:73-77,154-161,168-182`；`PolarClient.swift:67-74,103-110`；`LicensingStore.swift:193-205`（查核：`terra-factcheck-D.md`） |
| 16 | 轉移 | Swift 只在 activate 回 403 且 detail 含 `limit` 時，顯示「這張授權已啟用 3 台 Mac」＋客戶入口；Swift 自己不數台數。Polar 後台實際上限是否為 3、是否由 Polar 強制：推測，確認方法見 §6 風險 5。停用這台：deactivate，404 視為已停用 | 沿用 `PolarClient.swift:16-19,103-110`；`LicensingStore.swift:72-75,153-162,173-191`（查核：`terra-factcheck-D.md`） |
| 17 | 退款／撤銷 | 兩種來源都算 `revoked`（退回免費）：(a) 驗證回 404，或 HTTP 200 帶 `status` 為 `revoked`／`disabled`；(b) 本機 `revokedAt` 存在，或 `expiresAt` 已到。撤銷後保留本機授權，只給「移除」；移除只清本機、不呼叫 Polar。之後的一般驗證成功也不會清除撤銷 | 沿用 `Licensing.swift:154-161,168-182`；`PolarClient.swift:67-74`；`LicensePlan.swift:22-35`；`LicensingStore.swift:173-180`（查核：`terra-factcheck-D.md`） |
| 18 | 試用 | 目前 Swift 的方案沒有試用：沒有授權就是免費額度（`Licensing.swift:63-70,168-182`）。relay 仍實作 `/trial/start`、`/trial/export`（`worker.js:107-112,134-159`），Mac 端沒有呼叫者；本單元不呼叫、不改 relay。試用以前是否為 7 天、何時被免費額度取代：無法從目前原始碼確認 | 沿用（查核：`terra-factcheck-D.md`） |
| 19 | 損毀的狀態檔 | 原檔保留為 `licensing-state.json.unreadable-<sha256>`（照 `app_settings/persistence.rs:265`）。今日額度視為用完。鑰匙圈有授權憑證時立刻重新驗證，等結果期間算寬限（付費使用者不被擋） | 前半沿用技術方案 G.2；後半是新決定 |
| 19b | 只有鑰匙圈憑證 | 狀態檔與舊檔都不存在、鑰匙圈卻有憑證（例如使用者刪了狀態檔）：立刻重新驗證，等結果期間算寬限 | 新決定，與第 19 列一致 |
| 20 | 正式版旗標 | 正式版永遠啟用。只有 `dev_build::is_dev_build()` 時，`WRISTPER_LICENSING=off` 能關掉；關掉時 `/health` 回 `state: unlimited`、額度欄位全為 null、不帶 `truncated`（沿用 `testUnlimitedAndLicensedQuotaFieldsAreNull`、`testFlagOffOmitsTruncated…`）。舊 defaults 鍵 `licensingEnabled` 不讀 | 沿用技術方案 C 節；修掉 Wristper 的雙讀取點（`LicensingStore.swift:62` 與 `LicensingViews.swift:91,223,250`） |
| 21 | 不受額度影響 | 更新器、會議、Annotate、History 的「Transcribe again」 | 新決定，見 §7 問題 B2 |

## 2. 資料與儲存

```ts
<新資料根目錄>                     正式版預設 ~/Library/Application Support/Wristper，
                                   測試版預設 …/Wristper Test，HEX_APPLICATION_SUPPORT_DIR 可覆寫（app_paths.rs:15-18,31-42）
├─ licensing-state.json    新 App 的狀態；durable 原子寫入，0600
│  ├─ license: display_key, customer_email, activated_at, last_validated_at, expires_at, revoked_at
│  ├─ usage: { day: "YYYY-MM-DD"（洛杉磯日期）, used }
│  └─ imported_from_legacy: <舊檔 SHA-256>   （只匯入一次）
└─ device-id               Hex 的 Mac ID；SHA-256 後當 Polar meta.device
<舊來源路徑>                       舊 Swift App 固定用 ~/Library/Application Support/Wristper（EmbeddedReceiver.swift:14-16）
├─ licensing.json          舊 Swift 寫的；新 App 只讀，永不寫、不改名、不刪
└─ device-id               舊 Mac ID
鑰匙圈（新設計的名稱，不是沿用）  service dev.wristper.mac.license（測試版 dev.wristper.mac.test.license）
└─ account "license" → {"key":…,"activation_id":…}
relay：本單元不在 relay 存任何東西（relay 本身是否留授權資料，見 §4 第 3 點）
settings.json：不放授權（不是使用者設定，所以也沒有 hex settings 鍵）
```

1. **鑰匙圈只放憑證（新設計）。** Swift 的授權資料全在 `licensing.json`；Swift 的鑰匙圈只放手錶裝置密鑰（service `dev.wristper.mac.device`，account 是裝置 ID）與遷移記號（service `dev.wristper.mac.device.state`，account `migration`）（`KeychainSecretStore.swift:16-17,49-57,62-82`）（查核：`terra-factcheck-D.md`）。上面的 `dev.wristper.mac.license` 與 account `license` 是本設計新取的名稱。授權碼與 `activation_id` 存成一筆。方案判定只讀狀態檔，所以鑰匙圈鎖住時，只會延後重新驗證、停用與「複製授權碼」，不會擋輸入。讀寫照 `watch/pairing/keychain.rs`：關掉互動、不彈框、失敗立即返回。service 名稱不在 `dev.wristper.mac.device*` 底下。讀取分三種結果：
   1. 讀到：照常。
   2. 找不到（`errSecItemNotFound`）：狀態檔若顯示已授權，代表憑證遺失；照 Polar 404 處理成撤銷，提示重新輸入授權碼。
   3. 讀不到（鎖住或簽章不符）：不改狀態，當作連不上，下一小時再試；14 天寬限照算。Wristper 原本把完整授權碼明文放在 `licensing.json`（`LicensingStore.swift:26-30`），見 §7 問題 A2。
2. **舊格式（逐字）。** 頂層只有兩個選填鍵：`license`、`usage`（`Licensing.swift:3-11,53-70`）。
   - `license` 必填 `key`、`activationID`、`displayKey`、`activatedAt`、`lastValidatedAt`；選填 `customerEmail`、`expiresAt`、`revokedAt`。
   - `usage` 是 `day`（洛杉磯日期字串）、`used`。
   - 編碼：Swift `JSONEncoder`，`.iso8601` 日期（無小數秒）、pretty-printed、sorted keys，nil 欄位省略（`LicensingStore.swift:26-42`）。
   - 目前編碼器不輸出 `trial`。更舊的檔可能帶 `trial`，讀取時忽略未知鍵（`testLegacySnapshotKeepsLicenseAndDropsTrial`）。
   - Swift 讀到損毀檔時，先刪舊 `.bak`，再把原檔改名成 `licensing.json.bak`，回傳空快照（`LicensingStore.swift:15-23`）（查核：`terra-factcheck-D.md`）。所以舊來源路徑可能只剩 `.bak`；新 App 不讀 `.bak`，當作沒有舊授權。
3. **舊授權的識別與匯入（唯讀）。**
   - 舊來源路徑固定是 `~/Library/Application Support/Wristper/licensing.json`，不從新資料根目錄推導（查核：`terra-factcheck-D.md`）。正式版未覆寫時兩者剛好相同；測試版或覆寫了 `HEX_APPLICATION_SUPPORT_DIR` 時不同。
   - 只有正式版、而且沒有覆寫 `HEX_APPLICATION_SUPPORT_DIR` 時才自動找舊來源。測試版與覆寫的情況一律不讀真實舊檔；要測匯入，用 `hex license import --dry-run --from <合成檔>`。
   - 舊 `device-id` 與新 `device-id` 不同時（例如資料夾不同），`meta.device` 會變；Polar 是否因此視為新裝置：推測，用 sandbox 測（§5 真實驗證第 3 步）。

   啟動時若 `licensing-state.json` 不存在而舊來源存在：
   1. 讀舊檔一次，記下 SHA-256。
   2. 解析成功：先把 `key` 與 `activationID` 寫進新的鑰匙圈項目，寫入成功後才寫新狀態檔（其餘欄位與今日的 `usage`）。鑰匙圈寫入失敗就這次不匯入，也不寫狀態檔，下次啟動再試；否則狀態檔顯示已授權卻無法驗證，14 天後付費使用者會被降成免費。保留原 `activationID`，所以不多占一個啟用名額。
   3. 解析失敗：不匯入、不動舊檔，當作全新安裝，log 寫「legacy licensing file unreadable」與 SHA-256 前 8 碼。
   4. 舊檔永遠逐位元組不變。好處：使用者退回 Wristper 0.1.x 時，舊 App 仍讀得到自己的授權。
4. **需要使用者同意的步驟。**
   1. 產品行為：匯入只在本機讀檔，不送出任何資料。是否自動匯入，見 §7 問題 A1（必須使用者明確同意）。
   2. 開發驗證：讀使用者真實的 `licensing.json`（內含完整授權碼）要先取得同意。做法：使用者自己 `cp` 一份到暫存資料夾，執行 `hex license import --dry-run --from <副本>`。輸出只有遮罩後的 `display_key` 與欄位是否齊全。開發者不開啟真實檔，也不連真實的 Polar 授權。

## 3. 與現有架構的接點

```ts
src/licensing.rs            公開 API：Licensing handle、admit()、record_delivery()、status_json()、enabled()
src/licensing/
├─ model.rs     Plan、Admission、Lapse、reduce(event)；純函式（照 Licensing.swift）
├─ pacific.rs   洛杉磯日期與下次午夜；手寫美國太平洋規則，不加 tz 相依
├─ store.rs     licensing-state.json 讀寫、unreadable 備份、舊檔唯讀匯入
├─ credential.rs  CredentialStore trait；macOS 鑰匙圈實作；測試用記憶體實作
├─ polar.rs     PolarClient（經 net，15 s 逾時）；trait 供假實作
└─ owner.rs     「licensing」執行緒：寫檔、驗證排程、activate／deactivate
```

1. **執行緒與鎖。** 共享狀態是 `Mutex<State>`。鎖內只做記憶體運算：不做 I/O、不連網、不取其他鎖，所以持鎖時間是微秒級。`watch` 執行緒與 listener 只呼叫 `admit()`（唯讀）；只有 output worker 呼叫 `record_delivery()`。兩者都不等 `licensing` 執行緒。`record_delivery()` 改完記憶體就送一則「保存」訊息給 `licensing` 執行緒，由它合併後寫檔。這符合「`watch` 執行緒只等 inbox」。
2. **手錶擋點（`src/watch/service/`）。**
   - `routes.rs` `focus()`：身分驗證與 426 之後檢查 `admit()`，鎖住回 `Refusal::DailyLimit`。
   - `envelope.rs` `accept_audio()`：去重之後、`start_transcription()` 之前檢查一次；免費時 `start_transcription()` 截到 20 s 並標 `truncated`。
   - `routes.rs` `health()`：`licensing` 改成 `licensing::status_json()` 的 7 個鍵。目前寫死 `licensed`（`routes.rs:187-195`）。
3. **計數點（`src/parakeet.rs` `finish_output`）。** 不在 commit closure 裡扣額：`commit()` 在 `verify()` 之前執行（`src/paste.rs:476` `commit_prepared_paste`），`verify()` 回 false 時結果是 Withheld，不該計數。做法：
   1. 貼上前呼叫 `admit()`，額度已滿就不呼叫 Paster。
   2. 結果確定是送達集合（第 1 列）後才呼叫 `record_delivery()`。
   3. 所有計數來源（手錶、Paste、Print、Service、`/dictations`）都經過這一個依序執行的 output worker，所以「先查、後記」之間沒有別的計數者，不需要退還。

   額度已滿時：手錶回新增的 `DeliveryOutcome::QuotaExhausted { resets_at }`（`src/watch/delivery.rs:20` 現在沒有這種結果）；Mac 播錯誤音、HUD 提示；兩者都不寫 `last_transcript` 與 History。`parakeet.rs` 非測試行數 1386，上限 1500，這裡最多加 20 行，判斷放在 `licensing::output`。
4. **Mac 快捷鍵（`src/recognition.rs`、`src/recognition/gated_press.rs`）。** `HotkeyAction::Start` 前問 `admit()`。鎖住時沿用 `start_refused`／`owned` 的流程，新增第二個拒絕原因 `QuotaExhausted`：Hex 擁有按壓時播錯誤音並送 HUD 事件，然後 `hotkeys.suspend(shortcut)`。20 s 上限由 listener 迴圈依 capture 開始時間檢查，到時以程式停止結束（沒有 release tail，不捏造放開時間戳）。
5. **`local_api` 與 `hex dictation`。** `/dictations` 與 `hex dictation` 走同一個 worker，自動計數；開始時先問 `admit()`，鎖住就照第 12b 列拒絕。`/transcriptions` 與 `/streams` 見 §7 問題 B1。
6. **CLI。** 不加 `hex settings` 鍵。新增 `hex license status [--json]`、`hex license activate`（從 stdin 讀授權碼，不放 argv，避免出現在 `ps` 與 shell 歷史）、`hex license deactivate`、`hex license revalidate`、`hex license import --dry-run --from <path>`。前四個走 `/control`；新增 `ControlRequest` 是否要升 `CONTROL_VERSION`，依 `docs/cli.md` 判斷。
7. **深連結（`src/rune_ui/deep_link.rs`）。** Swift 現況：收到就直接 activate，不檢查既有授權（`LicensingViews.swift:222-235`）（查核：`terra-factcheck-D.md`）。新增 `Target::Activate(key)`，讀 `<scheme>://activate?license_key=`（正式版 `wristper`，測試版 `wristper-test`，`story_link.rs:39-47`）。授權碼長度 ≤ 256。已有有效授權時不直接替換，見 §7 問題 A3：D3 先只開設定頁並提示「已有授權」，確認視窗在 D4 加。不 log 完整 URL。
8. **更新器。** `updater` 不讀 `licensing`。免費、撤銷、過期的使用者都照常收到更新。D2 加一個測試鎖住這點。
9. **平台。** `rune_ui/platform.rs` 的 `Capabilities` 新增 `licensing`：macOS `true`，Linux 與 Windows `false`，`DIFFERENCES` 加 `Difference::Pending("the plan logic is cross-platform; only macOS has a credential store and a buy flow")`。Linux／Windows 不擋輸入，`hex license` 回 `unsupported`。`platform.rs` 已有 844 行（超過 800 的警告線），只加這兩處。
10. **fork 版 `AGENTS.md` 要改的不變式。** 「Capture has no automatic duration limit」加免費版 20 s 例外；「Every other intentional dictation finishes and pastes」加額度例外；Architecture 加 `licensing` 一段；Purpose 的 Wristper 偏離段補一句。`docs/features/watch.md:121` 與 `dictation.md` 同 PR 更新。

**離線與失敗的影響**

| 情境 | 行為 | 使用者影響 |
|---|---|---|
| Polar 連不上或回 5xx／403／422 | 不改狀態（fail-open） | 付費使用者 14 天內不受影響；超過後退回每天 10 段，連上即恢復 |
| Polar 回 404，或 200 帶 `revoked`／`disabled`；本機 `expiresAt` 已到 | 撤銷（fail-closed） | 退款或到期的使用者回到免費 |
| 啟用時離線 | 啟用失敗，顯示網路錯誤 | 要連網才能啟用 |
| relay 斷線 | 授權不受影響 | 只影響買斷按鈕的結帳頁 |
| 鑰匙圈鎖住 | 方案照狀態檔判定；驗證延到下一小時 | 無 |
| 狀態檔損毀 | 見 §1 第 19 列 | 免費使用者當天不能用；付費使用者不受影響 |
| 免費額度 | 全在本機 | 離線照常每天 10 段 |

## 4. 安全與隱私

1. **不送出：** 轉錄文字、音訊、Mac 名稱以外的裝置資訊、email。送給 Polar 的只有 `key`、`organization_id`、`label`（Mac 名稱）、`activation_id`、`meta.device`（Mac ID 的 SHA-256），與 Wristper 相同。
2. **log：** 不記授權碼、`activation_id`、email、`wristper://activate` 的完整 URL。只記遮罩後的 `display_key`、動作與結果碼（`invalid_license`、`activation_limit`、`network_error`）。HTTP 層只記主機、狀態碼與時間。
3. **偽造與重放：**
   - 同一使用者的程式能改狀態檔變成「已授權」，也能刪檔重設額度。PRD 接受這個風險（`PRD.md:131`），這裡沿用、不加 HMAC。
   - 手錶不能碰授權：授權動作只在程序內與 `/control`，relay 請求到不了（`Tier::Master` 在 relay 回 404，`routes.rs:86`）。D2 加測試：relay 送 `/license/*` 回 404。
   - 任何網頁都能開深連結。Swift 的 `wristper://activate` 收到就直接 activate，不先檢查既有授權（`LicensingViews.swift:222-235`）（查核：`terra-factcheck-D.md`）。本設計改成已有授權時不靜默替換（§7 問題 A3）。
   - 時鐘：Swift 以目前時間與 `lastValidatedAt` 相減判斷寬限，以當天洛杉磯日期字串判斷額度（`Licensing.swift:129-135,168-182`）。改系統時鐘能否重設額度或延長寬限：推測，沒有實測。確認方法：D1 的單元測試注入時鐘，模擬往前與往後調；不改真實系統時鐘。結果出來前不處理、也不宣稱可繞過。
   - relay 是否留存授權資料：推測。已知 webhook 不做授權邏輯，checkout 完成時向 Polar 查授權碼後轉址（`worker.js:221-235,242-264`）；但這不能證明部署設定、平台 log 或外部儲存都不留。確認方法：審讀 relay 原始碼與 `wrangler` 設定檔，只讀 repo，不連正式 relay、不看正式 log。
4. **測試資料：** 單元與整合測試只用假 Polar（trait 或 127.0.0.1 假伺服器）與記憶體鑰匙圈。真實驗證只用 Polar sandbox（`sandbox-api.polar.sh`，只有 dev build 能切）與 relay 測試房間。絕不連 `api.polar.sh` 正式授權，不讀寫 `dev.wristper.mac.device*`、`dev.wristper.mac.license`，不碰真實 `~/Library/Application Support/Wristper`。sandbox 授權碼由使用者提供，只放鑰匙圈或環境變數，不進 repo、prompt、log。

## 5. 測試與驗證

**單元（D1，照 Wristper 測試移植，名稱對應）**

1. `model`：全新安裝是免費；10 段後拒絕、洛杉磯午夜重設；台北午夜不是界線；秋季回撥日的重設時間；寬限邊界（恰 14 天仍有效、多 1 秒才 overdue）；撤銷後再驗證仍撤銷；重新啟用清除撤銷；`expiresAt` 已到算撤銷；`saved` 計數；注入時鐘往前、往後調時的額度與寬限（§4 第 3 點）；已授權送出不寫 `usage`，撤銷後恢復計數（`LicensingTests` 全部 18 個）。
2. `pacific`：2026–2030 每年兩個夏令時間切換日的日期與下次午夜，對照 Swift `Licensing.day`／`resetsAt` 產生的表。
3. `store`：合成舊檔匯入保留 `activationID`；匯入後舊檔 SHA-256 不變；有 `trial` 鍵也能匯入；只有 `license` 或只有 `usage`、或兩者都沒有的舊檔都能讀；舊來源只剩 `licensing.json.bak` 時當作沒有舊授權；測試版與覆寫 `HEX_APPLICATION_SUPPORT_DIR` 時不自動讀舊來源；舊檔壞掉不改舊檔、當全新安裝；新檔壞掉保留 `.unreadable-<sha256>` 且今日額度用完；權限 0600；未變更不重寫。
4. `status_json`：免費、已授權、撤銷、逾期、旗標關閉（`unlimited`，額度欄位全 null）五種的 7 個鍵；`resets_at` 格式與 Swift `ISO8601DateFormatter` 相同（golden）。
5. `polar`（假伺服器）：request body 形狀；403 含 `limit` 才算 `activation_limit`；404；422 不是無效授權碼；`status: revoked` 算拒絕；204 停用成功；網路錯誤（`PolarClientTests` 14 個）。

**整合（D2，`src/watch/service/integration.rs` 的 `FakeRelay`）**

1. 第 10 段貼上、第 11 段 402 密封在信封內、隔天（注入時鐘）可再送。
2. 第 10 段送達後重送同 ID：`duplicate:true`，不是 402。
3. 協定過舊的手錶先得 426，不是 402。
4. 鎖住時 `/health`、`/status`、`/enter`、`/undo` 照常；`/undo` 與 Paste Last 不計數。
5. 免費時 30 s 錄音只轉錄前 20 s，`/status` 有 `truncated:true`；已授權不截。
6. 貼上前額度已滿：手錶紀錄 `failed`＋`license_required`；文字不在 Paste Last、不在 History。
7. 第 10 段被 `verify()` 拒絕（target-changed）時不計數，下一段仍可送出（鎖住 `commit` 先於 `verify` 的情況）。
8. 額度用完時 `hex dictation start` 與 `/dictations` 立刻回 `daily-limit-reached`，不開麥克風。
9. Mac 快捷鍵：鎖住時 Option+e 不出聲、不改 HUD；超過門檻的按壓播錯誤音一次；20 s 自動停止後放開鍵不開始新錄音。
10. 更新器在撤銷狀態下仍檢查更新（D2）。

**真實驗證（用 `verify` 工具，Simulator 手錶＋relay 測試房間）**

1. 用暫存 `HEX_APPLICATION_SUPPORT_DIR` 啟動 Wristper Test，預先寫入 `used: 9` 的狀態檔。送兩段：第一段貼上，第二段手錶顯示額度用完。
2. 其他 e2e 執行一律用暫存資料夾或 `WRISTPER_LICENSING=off`，否則每天第 11 次起都會拿到 402。
3. Polar sandbox（要使用者同意 §7 問題 A4）：使用者在 sandbox 建產品與授權碼後，跑 activate → validate → deactivate，再啟用到第 4 台確認 `activation_limit`；另換一個 `device-id` 啟用同一授權碼，看是否多占名額。
4. Safari 開 `wristper-test://activate?license_key=<sandbox key>`，確認啟用且 log 沒有授權碼。
5. 只能實機做（使用者驗收）：真手錶看到 402 的畫面文字；用真的 0.1.x 資料升級後仍是已授權（要使用者先同意 §7 問題 A1，且先備份）。

**修 bug 的規則（spec-common）：** 每個修正附兩個測試（現在必須發生的、仍必須不發生的），都走正式程式路徑；還原修正與還原成舊錯誤做法各貼一次失敗輸出。

## 6. 實作步驟與預算

每個 PR 都要通過 `cargo fmt --all --check`、三平台 Compile、`scripts/check licensing`，順序 D1 → D2 → D3 → D4，全部在 G2 合併之後。

| PR | 內容 | 預估非測試行數 | 檔案 |
|---|---|---|---|
| D1 | `licensing` 純邏輯、`pacific`、`store`（含舊檔匯入）、`credential`（trait＋macOS＋記憶體）。不接任何呼叫者 | +650 | 新 `src/licensing.rs`、`src/licensing/{model,pacific,store,credential}.rs`，各 < 300 |
| D2 | 擋點與計數：手錶 `/focus`、音檔、`/health`、20 s 截斷；`finish_output` 計數；Mac 快捷鍵擋點與 20 s 停止；`Capabilities.licensing`；`AGENTS.md` 與 feature map | +250 | `watch/service/{routes,envelope}.rs` 各 ≤ +40；`parakeet.rs` ≤ +20；`recognition.rs` ≤ +60（ceiling 2050，現 1894）；`gated_press.rs` ≤ +30；`licensing/output.rs` 新 |
| D3 | `polar.rs`、`owner.rs`（驗證排程、activate／deactivate）、`hex license` CLI、深連結 `activate`、`docs/cli.md` | +550 | 新 `src/licensing/{polar,owner}.rs`、`src/control/` 加子命令、`deep_link.rs` ≤ +40 |
| D4 | 介面：設定頁方案卡（買斷是該頁唯一 Primary）、選單列、HUD 兩種提示（20 s 截斷、額度用完）、引導「試一次」的額度提示、goldens | 依 G2 後的畫面再估 | `rune_ui/` |

1. `src/` 頂層現有 39 個檔，加 `licensing.rs` 是 40，沒超過警告線（> 40）。
2. D2 可再拆成 D2a（手錶）與 D2b（Mac 快捷鍵），若 review 範圍太大。
3. D1、D3 沒有 UI，不影響 goldens；D2 改 HUD 事件但沒改畫面，D4 才重產 goldens。
4. 新的 HUD、錯誤與 CLI 字串都要有每一種介面語言，否則 `desktop/i18n` 的測試會失敗。D2、D3、D4 的預算各含這些翻譯（每句每種語言一份；語言清單以 `src/desktop/i18n.rs` 為準）。

**風險**

1. **繞道（接受，與 Wristper 相同）：** `target-changed`、`not-editable` 不計數，但文字可經 Paste Last 貼出。建議接受：要封住得在 Paste Last 加計數，規則變複雜。
2. **commit 點拒絕會丟掉 Mac 的一段話：** 只在手錶與 Mac 同時跨過第 10 段時發生。手錶有未送出清單可重送，Mac 沒有。接受，換來「已用次數永不超過上限」。
3. **舊檔與新檔分歧：** 匯入後兩者各自演進；退回 0.1.x 時舊檔的計數與驗證時間是舊的。影響小。
4. **推測：** Polar sandbox 的 license key 端點與正式版行為相同；Swift 只切換 base URL（`PolarClient.swift:30-31,37-40`）。確認方法：§5 真實驗證第 3 步，只用 sandbox；正式端點不測。
5. **推測：** Polar 後台的啟用上限實際設為 3，且由 Polar 強制。Swift 只在 403 detail 含 `limit` 時顯示「3 台」（查核：`terra-factcheck-D.md`）。確認方法：使用者在 Polar 後台看產品的 License Key 設定（唯讀），或在 sandbox 建同樣設定的產品，啟用到第 4 台。
6. **推測：** relay 不留授權資料、改系統時鐘的影響。確認方法見 §4 第 3 點。

## 7. 要使用者決定的問題

分兩組（Opus 要求）。A 組必須等使用者明確回答「是」，不得靜默採用預設；B 組不回覆就照建議做（沿用技術方案 J 節的慣例）。

**A 組：需要使用者明確同意（讀真實授權、資料、鑰匙圈，或涉及結帳與付款）**

1. **A1 舊授權要不要自動匯入？** 建議：正式版自動、唯讀匯入舊 `~/Library/Application Support/Wristper/licensing.json`。理由：使用者已付費，多一個對話框只增加摩擦；匯入不送出資料、不改舊檔。為什麼在 A 組：匯入會讀使用者真實的完整授權碼。未同意前，D1 只做 `--dry-run --from <路徑>`，不自動讀舊來源。
2. **A2 授權碼放鑰匙圈，還是照 Wristper 放檔案（0600）？** 建議：鑰匙圈（新 service `dev.wristper.mac.license`）。理由：不會跟著備份、同步或問題回報外流；狀態檔損毀時付費使用者靠鑰匙圈憑證恢復。代價：測試版每次重簽可能讀不到舊項目，要重新啟用。若選放檔案：憑證與狀態同檔，第 19、19b 列的恢復不成立，損毀時付費使用者也要重新輸入授權碼。為什麼在 A 組：會把使用者真實的授權碼寫進使用者的鑰匙圈。
3. **A3 深連結遇到已有有效授權時怎麼辦？** 建議：開視窗問「用這個授權碼取代目前的授權？」，不靜默替換。理由：任何網頁都能開 `wristper://activate`；Swift 現在直接替換（`LicensingViews.swift:222-235`）。為什麼在 A 組：付款後的啟用流程。
4. **A4 能不能用 Polar sandbox 做真實驗證？** 建議：可以，只用 sandbox 組織與 sandbox 授權碼，由使用者建立並提供；不連正式 `api.polar.sh`，不動正式產品、結帳或折扣。理由：§6 風險 4、5 只能靠 sandbox 確認。為什麼在 A 組：涉及結帳與授權服務的帳號。

**B 組：可照建議預設**

1. **B1 `local_api` 的 `/transcriptions`、`/streams` 怎麼算？** 建議：免費版回 402 `license_required`，付費才能用。理由：串流沒有「一段」可計，逐次計數又要另做 20 s 截斷；不擋就能用腳本繞過額度。另兩個選項：逐次計數，或不限。
2. **B2 會議、Annotate、History 的「Transcribe again」算不算額度？** 建議：不算。理由：Wristper 沒有這些功能；它們在上游 Hex 免費；額度是保護手錶聽寫這個付費核心。
3. **B3 Mac 快捷鍵到 20 秒時自動停止並貼上，還是錄完才只轉錄前 20 秒？** 建議：自動停止。理由：停止音讓使用者知道被截斷；只截轉錄會在使用者不知情時丟掉後面的話。
4. **B4 第二階段 Agent 的計數時點（§1 第 1b 列）。** 建議：照 Swift，進入等待確認就計一段。理由：與舊行為一致；第一階段用不到，第二階段設計時可再改。
