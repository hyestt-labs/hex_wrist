# 單元 C 設計：配對頁、手錶引導、只用手錶不要求麥克風（macOS）

狀態：設計稿，2026-11-05，基準 `b12f05de`。只讀程式推得，沒有建置或實機測試。實作照 `spec-C.md`。

## 1. 難題：沒有麥克風權限時，手錶錄音怎麼轉錄

現況：
- 手錶錄音經 `RecognitionControl::WatchTranscribe` 進聽寫 listener（`src/recognition.rs:451-458`）。`ListenerDictation` 只把訊息 `try_send` 到控制佇列（`src/watch/service.rs:95-104`）。
- listener 執行緒要等 `setup_ready`（`src/macos_rune.rs:317,374-389`）。runtime 只在 `status().ready()` 時放行（`:801-807`）。`ready()` 要求麥克風、輔助使用、模型三項（`src/desktop/onboarding.rs:70-76`）。
- 結果：只用手錶的人沒有麥克風權限，listener 不啟動。手錶錄音堆在佇列（容量 8），不會被處理。
- `listen()` 啟動時依序建立 `InputMonitor`（事件 tap，需要輔助使用，`recognition.rs:334`）、`DictationWorker`（引擎與 Paster，`:350`）、`DictationAudio::open`（`:355-363`）。
- `DictationAudio::open` 在 `release_while_idle` 關閉時會立刻開麥克風（`src/dictation/audio.rs:212-216`）。這一步會讓 macOS 跳出麥克風權限提示。

### 方案 A（採用）：listener 只等輔助使用與模型；麥克風由音訊 owner 擋住
- 新增 `SetupStatus::listener_ready()`：輔助使用 Ready 且模型已安裝。`ready()` 不變，仍代表 Mac 聽寫就緒。
- `macos_rune.rs:317` 拆成兩個值：`listener_ready` 決定 listener 何時啟動（`:378`、`:441`、`:804`）與 `LaunchWindow`（`:470`）。`dictation_ready` 只給 `recheck_after_identity_change`（`:318`），行為不變。
- 麥克風權限由 `DictationAudio` 的 owner 管理，不放在 `recognition.rs`。理由：`recognition.rs:806` 每次重新讀設定都會送 `set_release_while_idle(policy)`。如果覆蓋寫在 listener 裡，任何設定存檔都會打開麥克風。
  - owner 記住 `wanted`（使用者設定）。實際值是 `wanted || !granted`（`audio.rs:677` 改這裡）。
  - 權限沒開時，輸入維持 `RecoveringAudioInput::closed`（`:213`），從不開串流。
  - `Start`（快捷鍵、`hex dictation`、`/dictations`）在權限沒開時回 `Ok(false)`，不錄音。
    - 第一次（NeedsRequest）：呼叫 `onboarding::request_microphone()`（`onboarding.rs:1016`）。這只跳系統提示，不開串流。
    - 已拒絕或待重開（NeedsSettings、NeedsRestart）：播錯誤音，並設一個一次性旗標。主執行緒的 pump 讀到後打開設定視窗的 General。
  - 快捷鍵被拒後沿用既有的 `hotkeys.suspend(shortcut)`（`recognition.rs:930-932`）。`/dictations` 沿用 `microphone-unavailable`（`audio.rs:314-320`）。不加新回覆碼，不升 `CONTROL_VERSION`。
  - 權限變成 Ready 後，下一次 `Start` 前 owner 重讀權限，改回 `wanted`。第一次按鍵像 release-while-idle：沒有 pre-roll。
  - 權限讀取用 `microphone_state()`（`onboarding.rs:1110`，改成 `pub(crate)`）。`HEX_AUDIO_FIXTURE` 時讀成 Ready（`:1112`），benchmark 不受影響。測試注入假的權限來源。
- 新程式放 `src/dictation/microphone_access.rs`：純政策函式加 macOS 讀取。`recognition.rs` 有 ceiling 2050 行，只改把 `release_while_idle` 傳給提示音的那幾行。
- 手錶在 listener 還不能跑時送來錄音（缺輔助使用或模型），`ListenerDictation::transcribe` 立刻回 `Err`，寫明缺哪一項。手錶收到既有的 500 `{"state":"failed","error":…}`（`src/watch/service/envelope.rs:401-408`）。不新增回應格式。

### 方案 B（不採用）：把 `DictationWorker` 從 `listen()` 拆出來，先於 listener 啟動
- Worker 需要 `InputMonitor` 的 activity（`recognition.rs:350-354`），Paster 等實體修飾鍵也靠它。沒有輔助使用時 `InputMonitor::start` 會失敗。
- listener 要改成「接手已存在的 worker」。關機順序也要改：DRAIN 註明「listener 擁有引擎，join 它才釋放模型」（`macos_rune.rs:561-562`）。
- `capture_host` 也拿 worker 的引擎（`recognition.rs:367`）。
- 會破壞的不變式：listener 啟動順序、單一引擎 owner、shutdown/drain 順序。改動面最大，風險最高。

### 方案 C（不採用）：手錶服務自己載模型、自己貼上
- 違反 AGENTS.md：「Hex never loads a second model or starts a second Paster for it」，也違反 `wristper-on-hex.md` K.8。
- 手錶與 Mac 聽寫不再同序，History 與 Paste Last 也會分成兩套。

### 方案 D（不採用）：listener 只在已配對手錶時免麥克風啟動
- runtime 要追蹤配對狀態，配對資料在 `watch` 執行緒與鑰匙圈。耦合比 A 多。
- 換來的只是「沒配對、沒麥克風的人按快捷鍵不會被問」。照使用者決定，這正是該問的時機。

### A 不破壞的不變式
- `keyboard::initialize_layout` 仍在主執行緒、設定之前（`macos_rune.rs:300`）。listener 仍等 `recognition_start` channel，只換放行條件。
- 事件 tap 仍只在輔助使用 Ready 後建立。模型仍要求已安裝。
- 關機：listener 早一點啟動，DRAIN 一樣 join 它（`:563-571`）。手錶服務仍先停（`:557-559`）。
- 「從不開麥克風」由 owner 保證，第 5 節步驟 3 逐一列出開啟路徑來驗證。例外：Annotate 擷取打開 capture sink（`audio.rs:685-688`），這算明確的 Mac 麥克風使用，macOS 自己會問。

## 2. 引導

### 位置
- story 新增 `Step::Watch`，放在 `Started` 和 `Parked` 之間（`src/rune_ui/onboarding_kit/stories.rs:59-73`）。順序：下載或模型步驟 → Apple Watch → 示範 → 權限 → Almost ready → 練習 → Hex is ready。
- 只在 `caps.watch`（macOS）出現。Linux、Windows 照舊。
- tour 的 `Step`（`onboarding.rs:113-118`）不增加。watch 頁和示範一樣只是 story 頁，tour 停在 Setup。
- `first_run_next`（`stories.rs:785-800`）：`Pick|Started` 放行後進 `Watch` 並回 `None`，等使用者動作。
- 頁面上兩個動作：「Pair later」（Secondary），以及配對完成後的 Continue（Primary，`.default_button()`）。兩者都進示範頁。Back 照既有規則回到模型清單（只在模型步驟出現過時）。
- watch 頁不是 `optional()`，沒有 Skip。AGENTS.md「Skip 只在示範與練習」不變。「Pair later」不記錄完成。

### 「已配對」與「就緒」
- 已配對：快照裡至少一支手錶的 `DeviceStatus` 不是 `NeedsRepair`（`src/watch/pairing.rs:216-227`）。`SecretsUnavailable` 算已配對，避免登入時鑰匙圈還鎖著就把麥克風列叫回來。
- `SetupStatus::ready_with_watch(paired)`：輔助使用與模型 Ready，而且麥克風 Ready 或 `paired` 為真。
- `Flow` 加 `watch: bool`，由新的 `Input::Watch(bool)` 更新。它和 `Input::Status` 共用同一段「未就緒→就緒」判斷（`onboarding.rs:418-441`），所以停在 Setup 的 tour 會收到 `AdvanceSoon`。不用 `set_accessibility_stale` 那種旁路 setter。原因：`Flow::new` 不知道手錶狀態，會把續跑的 tour 退回 Setup（`:308`）。
- `麥克風延後 = watch && microphone != Ready`。這時：
  - `permissions_ready()`（`:395`）不看麥克風。
  - `rows()`（`:446`）不放麥克風列。
  - `passes(TryIt)`（`:497`）為真，跳過練習。練習要用 Mac 快捷鍵。
  - `relaunch()`（`:565`）改看 `listener_ready()`。
- 完成記錄：`rune_ui/onboarding.rs:146`、`live.rs:435`、`live.rs:602` 從 `status.ready()` 改成 `ready_with_watch(ui.watch.paired())`。
- story 的 `granted()`（`live.rs:153`）加 `paired` 參數。`poll` 每次都會重算它（`live.rs:498,517`），所以在 watch 頁配對後，權限頁會跟著變。
- 權限頁在麥克風延後時只顯示輔助使用：
  - 標題 "Let Hex type for you"。
  - 說明 "Pastes what you dictate on your watch into the app you're using"。
  - 這只在手錶模式出現。現有 `mac-onboarding-*` 的 golden 不變。
- 續跑：`TourStep::Setup` 時（`live.rs:256`），如果 `caps.watch` 而且還沒配對，跳到 watch 頁；否則跳到權限頁。代價：Accessibility 要重開的人，重開後會再看到一次 watch 頁。

### AGENTS.md 的修改（照抄）
1. 「On a new Mac, release dictation starts only after…」改成：
   "The listener starts once Accessibility and the selected model are ready, so watch recordings transcribe and paste without the Microphone. Until the Microphone is granted, `DictationAudio` never opens it: a dictation start (shortcut, `hex dictation`, `/dictations`) records nothing and asks: macOS's prompt the first time, the Settings window once refused. A window no longer opens at launch only because the Microphone is missing."
2. 引導段落 "Start using Hex records completion only once setup is ready" 後面加：
   "Setup is ready when Accessibility and the model are, and the Microphone is granted or a watch is paired (not needing repair). With a paired watch and no Microphone, the permissions page asks only for Accessibility and the practice is passed over. On macOS the Apple Watch page follows the model step: Pair later or Continue moves on, it has no Skip, and it never records completion."
3. Purpose 段的 Wristper deviation 加一句："Watch-only setup asks for no Microphone (user decision 2026-11-04)."
4. `watch` 模組條目加 unit C：`watch::view`、`Message::Revoke`、`rune_ui::watch_pane`、`watch.*` 設定。

## 3. 設定

- `app_settings/watch.rs` 新增 `WatchSettings`，加 `#[serde(default)]`。`AppSettings` 加 `pub watch: WatchSettings`。所有 OS 都編譯，不加 cfg，因為 `settings.json` 跨 OS 共用。
  - `status_capsule: bool`：預設 true。狀態膠囊顯示與否。
  - `capsule_follows_pointer: bool`：預設 false。膠囊跟著滑鼠所在螢幕，用 `rune_hud::hud_placement`。
  - `keep_on_clipboard: bool`：預設 false。手錶聽寫貼上後文字留在剪貼簿。手錶錄音實際用 `labs.keep_on_clipboard || watch.keep_on_clipboard`。
  - `terminal_apps: Vec<String>`：預設空，一律序列化成 `[]`。內建清單（`src/watch/field.rs:125-141`）之外，再當成終端機的 bundle id。最多 32 筆，每筆非空、≤ 255 字元。
- 遷移安全：
  - 舊檔沒有 `watch`，讀到預設值。`migrated` 比對前後值相同，不回寫（`persistence.rs:109-110`）。
  - 單一欄位壞掉，由 `decode` 逐欄刪除並套預設（`persistence.rs:186-215`）。
  - 舊版 build 遇到未知的 `watch` 鍵會忽略。不寫 `null`。
- `hex settings` 鍵（`src/control/settings.rs`）：
  - `watch.status_capsule`、`watch.capsule_follows_pointer`、`watch.keep_on_clipboard`：`Kind::Bool`。
  - `watch.terminal_apps`：`Kind::Json`，驗證規則同上。
  - `NEEDS` 加 `("watch.", "watch")`（`:690`），非 macOS 回 unsupported。
  - `every_persisted_setting_has_a_cli_key`（`:1499`）會先失敗，加鍵後通過。
- 即時套用：在 `apply_runtime`（`app_settings.rs:755` 旁）加 `crate::watch::settings::apply(&self.watch)`，存成 atomic 與 `RwLock<Vec<String>>`。使用時才讀：
  - 膠囊：每次發布時讀。
  - keep：每次貼上 commit 時讀。`paste.rs:387` 改成由呼叫端傳入 `keep`。
  - 終端機清單：每次 gate 時讀。
  - 四項都不需要重開，也不需要 listener 重新載入。

## 4. Apple Watch 頁

- 側欄新項目 `Pane::Watch`，key `nav-watch`，標題 `tr("Apple Watch")`，加一個新的線條手錶圖示。放在 General 後面（`rune_ui/labs.rs:35-43`），只在 `caps.watch` 顯示。
- 頁面程式在新檔 `rune_ui/watch_pane.rs`。`rune_ui.rs` 只剩約 12 行額度，只加一個 `watch: WatchView` 欄位和 dispatch。
- 資料來源：
  - `watch` 執行緒在每次 relay 狀態、配對或撤銷改變時，推送 `WatchSnapshot`（新增 `mac_name`、`replaced`）到 `watch::view`，模式同 `watch::status::watch`。視窗安裝 watcher，從不輪詢。
  - 配對：`PlatformRequest::WatchPair(code)`。macOS host 開執行緒呼叫 `claim`（`service.rs:294`），結果用 `window.update` 送回。
  - 撤銷：新增 `Message::Revoke{id, reply}`，呼叫 `PairedDevices::revoke`（`pairing.rs:602`）。
  - Linux 與 Windows 的 host 對這兩個 request 不做事。
- 版面（`page` + `section`，不加 Primary，因為頁面的主要動作是輸入碼）：
  1. **This Mac**：
     - `watch-relay` 列：`status_dot` 加 Connected、Connecting…、Offline: check the network、Another Mac took over this pairing。
     - `watch-mac-name` 列：手錶上顯示的 Mac 名稱。
  2. **Pair a watch**：
     - 說明 "On your watch, open Wristper › Pair a Mac and type the code it shows"。
     - 6 位數輸入欄 `watch-code`：只收數字，滿 6 位自動送出。
     - 狀態行 `watch-code-note`（`note`）：Pairing…、Finish on your watch…（等快照出現該裝置，最多 120 s）、Paired with <name>，或錯誤。
     - `link_row` "Get the watch app" 連到 https://wristper.dev/watch。
  3. **Paired watches**：
     - 每支手錶一列，key `watch-device-<id>`。描述依狀態：Last seen <時間>、Pair it again to use it（NeedsRepair）、Waiting for the keychain（SecretsUnavailable）。
     - `small_button` "Remove…" 是兩步確認，第二步是 Danger "Remove"。
     - 撤銷失敗時，在該列下方用 `note` 顯示 `revoke` 回的錯誤。
     - 沒有手錶時用 `placeholder` 顯示 "No watches paired yet"。
  4. **Status and paste**：三個 `toggle_row`，key 分別是 `watch-status-capsule`、`watch-capsule-pointer`、`watch-keep-on-clipboard`。`terminal_apps` 這次只做 CLI。
  5. 服務沒啟動（沒有快照）：整頁用 `message` 顯示 "The Apple Watch service isn't running"。
- 錯誤文案由 pane 用 `tr` 對應每個 `ClaimError`（`service.rs:132-170`）。CLI 的 `message()` 不變。等手錶完成逾時：顯示 "The watch didn't finish pairing. Update Wristper on the watch and try again"。
- 新字串在五種語言檔都要補（`desktop/i18n/*.rs`）。`i18n.rs` 本身只剩約 42 行，不要動。
- QR：`Cargo.lock` 裡查過 `qr`、`qrcode`、`fast_qr`，都沒有，所以只放文字連結。
- `--snap` 場景，都用 `mac_fixture`，Linux 也能畫：
  - 設定頁：`mac-watch`（已連線，一支 Ready、一支 NeedsRepair）、`mac-watch-empty`、`mac-watch-offline`、`mac-watch-pairing`、`mac-watch-waiting`、`mac-watch-paired`、`mac-watch-error`（code_not_found）、`mac-watch-remove`（確認狀態）、`mac-watch-unavailable`。
  - 引導：`mac-onboarding-watch`、`mac-onboarding-watch-paired`、`mac-onboarding-permissions-watch`。

## 5. 實作步驟（依序；每步 commit，結尾跑檢查）

先 source spec-C 的環境，每步都跑 `cargo fmt --all --check` 和 `cargo clippy --locked --bin hex --all-targets -- -D warnings`。

1. **就緒判斷**（`desktop/onboarding.rs`，約 80 行）。先寫會失敗的測試：
   - `listener_ready_needs_accessibility_and_model_not_the_microphone`：抓「listener 仍要求麥克風」。
   - `a_paired_watch_makes_setup_ready_without_the_microphone`：抓「只用手錶永遠不會完成、每次開機都重開引導」。
   - `without_a_watch_the_microphone_is_still_required`：抓「把麥克風對所有人都拿掉」。
   - `a_resumed_tour_advances_when_the_watch_input_arrives`：抓「旁路 setter 讓 tour 卡在 Setup」。
   - `a_needs_repair_watch_does_not_count`、`a_keychain_wait_still_counts`、`deferred_microphone_hides_its_row_and_passes_the_practice`。
   - 檢查：`scripts/check desktop::onboarding`。
2. **麥克風 owner**（新檔 `dictation/microphone_access.rs`，加 `audio.rs`）。先寫會失敗的測試，用注入的權限與假輸入：
   - `a_start_without_permission_records_nothing_and_never_opens_the_input`：抓「靜音錄音或開了串流」。
   - `a_settings_reload_without_permission_keeps_the_input_closed`：抓「存任何設定就開麥克風」。
   - `granting_permission_restores_the_saved_policy_at_the_next_start`、`the_first_refusal_asks_and_a_denied_one_wants_settings`。
   - 檢查：`scripts/check dictation::audio`。
3. **開啟路徑盤點**：列出 `listen()` 之後所有會開輸入的路徑，寫進 commit 說明。至少有：
   - warm open、cold-open 重試、預設輸入或裝置清單變動後重開、選擇變更（`audio.rs:674`）、`audio.rs:605,901` 的 `request_open`、蓋上螢幕、capture sink。
   - 每條都要有測試或寫明的例外。capture sink 是 Annotate 的例外。
   - 檢查：用 grep 找 `request_open`、`RecoveringAudioInput::open`，確定都在 owner 的 gate 後面。
4. **listener 放行**（`macos_rune.rs` ≤ 20 行，`launch.rs`，`watch/service.rs`）：
   - 測試 `a_watch_only_setup_launches_quietly`（`LaunchWindow`）：抓「每次開機都開視窗」。
   - 測試 `a_recording_before_the_listener_can_run_is_refused_at_once`（注入 readiness）：抓「錄音卡在佇列直到 50 s」。
   - 設定視窗旗標由 pump 讀取。
   - 檢查：`scripts/check macos_rune` 和 `scripts/check watch::service`。
5. **設定**（`app_settings/watch.rs`、`control/settings.rs`、`watch/field.rs`、`paste.rs`）。測試：
   - `an_old_file_without_watch_loads_defaults_and_is_not_rewritten`、`an_invalid_watch_field_falls_back_alone`、`terminal_apps_rejects_bad_entries`。
   - `a_custom_terminal_bundle_id_pastes_as_final`（抓「設定沒接上」）、`watch_keep_on_clipboard_applies_only_to_watch_recordings`、既有的 `every_persisted_setting_has_a_cli_key`。
   - 膠囊開關與跟隨滑鼠接到 `rune_ui::watch_status`。
   - 檢查：`scripts/check settings`、`scripts/check watch`。
6. **服務 API**：`Message::Revoke`、`watch::view` 推送、快照加 `mac_name` 與 `replaced`。測試（`MemorySecretStore` 寫入失敗）：
   - `revoke_reports_the_list_it_could_not_write`（抓「畫面說已移除，檔案其實沒寫」）、`a_claim_then_the_watch_request_pushes_the_new_device`。
   - 檢查：`scripts/check watch::service`。
7. **Apple Watch 頁與場景**（`watch_pane.rs`、`sidebar.rs`、`scenes.rs`、i18n）。Linux 測試，加在 `tests/panes.rs`：
   - `six_digits_send_one_pair_request`、`each_claim_error_shows_its_message`、`remove_needs_two_presses`。
   - `every_watch_control_has_a_name`、`linux_and_windows_sidebars_have_no_watch`。
   - 檢查：本機 `--snap` 兩種主題逐張看，加上 `every_translated_string_has_every_language`。
8. **引導頁**（`stories.rs`、`live.rs`、`permissions.rs`、`first_run.rs`）：
   - 先讓 `fixture_resume` 把 watch 頁當成按了 Pair later 通過。檢查：現有 `mac-onboarding-*` 的 `--snap` 和改之前逐位元相同。
   - 再加新頁與新場景。Linux 測試：`pair_later_goes_to_the_demo_and_records_nothing`、`watch_only_permissions_ask_only_for_accessibility`、`watch_only_finishes_without_the_practice`。
   - 把新場景加進 `onboarding_check::SCENES`。
9. **文件**：照第 2 節修改 AGENTS.md，更新 `docs/features/watch.md` 與索引。最後跑 `cargo nextest run --locked --bin hex` 全套，再跑 `python3 scripts/code-shape.py`。

行數預算：`rune_ui.rs` 約 12 行、`macos_rune.rs` 約 54 行、`stories.rs` 與 `permissions.rs` 已超過 800 的警告線，盡量把新程式放新檔。

給協調者的 golden 清單：
- 新增：第 4 節列的 12 個場景，各有 light 和 dark。
- 會變：側欄多一項，所以 `panes.rs:1587` 起的 `mac-*` 和 `goldens()` 裡的 `mac-labs-link` 都會變。
- 不可變：所有 Linux 與 Windows 的 golden，以及現有的 `mac-onboarding-*`。

不在範圍內，各自列為待決問題：`hex watch list` 與 `revoke`；`terminal_apps` 的 UI 編輯器；選單列「Pair Apple Watch…」；引導完成時登記登入項目；用手錶做練習；QR code；426 `watch_outdated` 顯示在 Mac UI；授權與額度（單元 D、E）；膠囊的其他語言；Windows 與 Linux。

## 6. 風險與只能實機驗證的事

- 只有 macOS 實機能驗證：只列舉裝置、不開串流時會不會跳麥克風提示；`requestAccess` 的提示時機；權限變 Ready 後第一次按鍵能否開麥克風；拒絕後打開設定視窗的路徑。
- UI 與引導測試只在 Linux 跑（`#[cfg(all(test, target_os = "linux"))]`）。Mac 本機只能用 `--snap` 看。要靠協調者跑 `check-linux.yml` 確認。
- 對上游 Mac 使用者的行為改變：麥克風被撤銷後，開機不再開視窗，改在按快捷鍵時處理。輔助使用與模型就緒後，listener 就會啟動。
- Accessibility 重開的路徑會再顯示一次 watch 頁。這是刻意接受的代價。
- 配對要等手錶自己送出 `/pairing/request`，有 120 s 的空窗。Mac 看不到手錶端的 426 錯誤，只能等到逾時。
- 真的 relay、真的手錶、鑰匙圈鎖住的登入時刻都沒有測過。依規則，測試不碰正式 relay 和使用者的 Wristper 鑰匙圈。
- Linux 與 Windows 沒有在本機編譯。新程式的 cfg 要靠 CI 的 clippy 檢查。
