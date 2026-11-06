# 報告：Activity 場景固定時間（fix/activity-fixture-time）

## 結論

- `mac-activity` 場景現在不讀真實時鐘：fixture 用固定常數 `FIXTURE_NOW_MS = 1_793_746_397_394`，窗格自帶一個可凍結的時鐘（`ActivityPane::now_ms`）。真實窗格 `now_ms = None`，仍用真實時間。
- 預期**沒有 golden 要重生**：新常數讓最新事件的時間戳等於現有 golden 顯示的 `1793746393394`，年齡標籤 `4s/6s/6s/9s/12s/1m/1m` 也相同（見下方 `--snap` 結果）。這個預期未在 Linux 上驗證，要靠 CI 的嚴格 golden 檢查確認。

## 原因（查過原始碼）

畫面上有兩處讀時鐘：
1. 詳情的 `Timestamp` 與 Raw event JSON 顯示 `timestamp_ms`（`activity.rs` 的 `fact(...)` 與 `serde_json::to_string(event)`）。fixture 選第 0 列，所以每次都顯示。fixture 用 `now_ms()`，所以每次渲染不同。
2. 列表右側的 `event_age(ts)`（`src/desktop/i18n.rs`）用 `history::now_ms()` 算「Ns ago」。fixture 改成固定時間後，如果仍讀真實 now，會變成「Nd ago」且逐日變動，所以時鐘也必須凍結。

畫面**沒有**本地時鐘或時區：`event_summary`、`fact`、`event_age` 都不用 chrono／Local／時區（grep 無命中）。所以時區測試 (b) 不適用於程式邏輯，改以不同 `TZ` 重跑測試作為佐證。

## 改動

- `src/desktop/i18n.rs`：`event_age` 拆出 `event_age_at(timestamp, now)`；`event_age` 呼叫它並傳真實 now（History 等其他呼叫者行為不變）。
- `src/rune_ui/activity.rs`：加 `FIXTURE_NOW_MS`、`ActivityPane::now_ms`；`fixture()` 用常數；`pane()` 用 `pane.now_ms.unwrap_or_else(history::now_ms)`。
- `src/rune_ui/activity_clock_tests.rs`（新，掛在 `rune_ui.rs`）：兩個測試，走 `SCENES` 的 `mac-activity` 與視窗的 `view`。
  - `activity_scene_ignores_the_clock`（現在必須發生）：渲染兩次，中間睡 1.1 秒（跨過秒界線）；文字與像素逐位元相同，且最新事件讀「4s ago」。
  - `activity_pane_ages_real_events_against_now`（仍必須不發生）：用 `EventLog` 寫入 3 小時前與 2 天前的事件，真實窗格 `poll` 後必須顯示「3h ago」「2d ago」，不得顯示「4s ago」。
- 測試放在新檔而不是 `tests.rs`：`tests.rs` 的 cfg 是 `all(test, target_os = "linux")`，在 Mac 不會編譯（我試過暫時改 cfg，出現 18 個編譯錯誤），新檔在 Mac 與 Linux 都跑。

## 驗證輸出

`scripts/check`（Mac，完整，exit 0）：

```
     Summary [  30.891s] 2559 tests run: 2559 passed, 32 skipped
```

Mac 上 `tests.rs` 的 Linux-only golden 比對不會跑；新測試的像素比對在 Mac 用 Metal GPU 實跑（`HEX_REQUIRE_GPU=1` 設定下也通過，代表真的渲染了）：

```
=== fixed ===
     Summary [   1.557s] 2 tests run: 2 passed, 2046 skipped
```

不同時區（程式不讀時區，這是佐證）：

```
TZ=Pacific/Kiritimati   Summary [   1.573s] 2 tests run: 2 passed
TZ=Etc/GMT+12           Summary [   1.551s] 2 tests run: 2 passed
TZ=Asia/Taipei          Summary [   1.557s] 2 tests run: 2 passed
```

### 變異驗證

變異 1：fixture 改回 `now_ms()`（且不凍結時鐘）→ 場景測試失敗：

```
        FAIL [   1.603s] (2/2) hex::bin/hex rune_ui::activity_clock_tests::activity_scene_ignores_the_clock
    thread 'rune_ui::activity_clock_tests::activity_scene_ignores_the_clock' (8896140) panicked at src/rune_ui/activity_clock_tests.rs:92:5:
    assertion `left == right` failed: the scene's text followed the clock
```

（文字斷言先於像素斷言，所以失敗訊息是文字；像素斷言沒有單獨被變異驗證。）

變異 2：真實窗格的時鐘也被凍結（`unwrap_or(FIXTURE_NOW_MS)`）→ 真實窗格測試失敗：

```
        FAIL [   0.157s] (1/2) hex::bin/hex rune_ui::activity_clock_tests::activity_pane_ages_real_events_against_now
    ["General", ..., "Activity", "Refresh", "Dictation started", "0s ago", "Dictation started", "0s ago", "Select an activity row."]
```

兩個變異都已還原（`git diff` 只剩上述改動）。

### `--snap` 對照

`hex app -- --snap /tmp/fix-activity-snap.png --scene mac-activity --theme light` 的 Activity 窗格文字與 `tests/golden/rune-mac-activity-light.png` 逐字相同（時間戳 1793746393394、Raw event JSON、各列年齡）。側欄差異（Quit Hex、Version）是 Mac 與 Linux 的平台差異，不是本改動造成。

## 第 3 項：其他 fixture 的時鐘依賴

`grep -rn 'now_ms\|SystemTime::now\|Utc::now\|Local::now' src/rune_ui`（另加 `Instant::now`）命中判斷：

| 命中 | 影響 golden？ |
| --- | --- |
| `activity.rs:73` | 是，已修。 |
| `history.rs:161`（`HistoryPane::fixture` 用 `history::now_ms()`，畫面用 `event_age`） | 否。事件時間是 now 之前「N 分鐘 + 30 秒」，顯示「Nm ago」，要渲染延遲超過 30 秒才會變。History golden 實跑最大差 1–2。沒改。 |
| `history.rs:396`（`entry.replace(..., now_ms())`） | 否，使用者操作路徑，不是 fixture。 |
| `practice.rs:20` | 否，只是記錄起點，不畫。 |
| `meetings_host.rs`（`now_ms`、`Instant::now`） | 否，真實 host，場景走 `Service::Fixture`。meetings golden 最大差 2。 |
| `meeting_capsules.rs:494`（`wall_clock_ms`） | 否，真實執行路徑。 |
| `meeting_capsules.rs:1503、1978`（測試 helper 的 `Instant::now() - 258s／4s`） | 低風險，未修。顯示的經過時間是 258 秒加渲染延遲，渲染拖過 1 秒才會變「4:19」，與牆鐘時間無關。meeting capsule golden 最大差 2。 |
| `modes.rs:193`、`onboarding.rs:203`、`models.rs`、`picker.rs:822`、`meeting_capsules.rs` 其餘 `Instant`、`macos_host.rs`、`linux_host.rs`、`tests.rs:658` | 否，排程、重試、動畫計時，不輸出到畫面文字。 |

## 會變的 golden

預期：無。若 CI 的嚴格檢查仍報 `rune-mac-activity-light`，代表 Linux 渲染與現有 golden 有我沒看到的差異，那時再重生。

## 未驗證

- Linux 渲染：沒跑 `HEX_UPDATE_GOLDENS=1 cargo test --locked --bin hex rune_ui`，也沒跑 `golden-strict.yml`。「golden 不變」是用 Mac 的 `--snap` 文字比對推得，不是 Linux 像素證據。
- 新測試在 Linux／Windows 沒跑過；程式碼沒用平台專屬 API，但 `cargo clippy --locked --bin hex -- -D warnings` 只在 Mac 跑過（`scripts/check` 內的 clippy）。
- 像素斷言沒有單獨的變異驗證（文字斷言先失敗）。
