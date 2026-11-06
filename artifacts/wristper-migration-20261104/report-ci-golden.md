# 報告：嚴格、可重跑的 golden 檢查（ci/golden-strict）

## 結論

- 新增 `.github/workflows/golden-strict.yml`、`scripts/golden-diff.py`、`scripts/test-golden-diff.py`，並更新 `AGENTS.md`、`docs/design-system.md`。
- 實跑 dev（run 42162978501）的結果是 **FAIL，1 個場景超過容許**：`rune-mac-activity-light`，2619 個像素超過容許，最大通道差 233。其餘 126 個場景在容許內，48 個 capture-capsule 場景因為只能在 macOS 渲染而不檢查。
- 原因：`src/rune_ui/activity.rs:73` 的 `Activity::fixture()` 用 `now_ms()` 當事件時間，畫面詳情顯示 `timestamp_ms`（golden 是 1793746393394，這次渲染是 1793940006144）。這個 golden 每次渲染都不同，現有測試只因為差異低於 0.2% 上限（5958 像素）才放行。這是產品碼的 fixture 問題，我依規定沒改，也沒重生 golden。

## 建議的下一步

1. 另開 PR 讓 `Activity::fixture()` 用固定時間（例如固定常數），再重生 `rune-mac-activity-light` 與同場景的 dark golden。修好後這個 workflow 會全綠。
2. 合併本 PR 後，用 `gh workflow run golden-strict.yml --ref dev -f ref=<要檢查的分支>` 正式跑。

## 實跑

- 連結：https://github.com/hyestt-labs/hex_wrist/actions/runs/42162978501
- artifact：`golden-renders`（175 張渲染）、`golden-diffs`（`rune-mac-activity-light.png`：差異像素標紅，其餘為調暗的 golden）。
- `gh workflow run golden-strict.yml --ref ci/golden-strict -f ref=dev` 回 HTTP 404：GitHub 要求 `workflow_dispatch` 的檔案先在預設分支（dev）上。改法：在分支上暫時加 `push` 觸發（只限 `ci/golden-strict`）跑這次檢查，跑完已移除。最終版本（只有 `workflow_dispatch`）沒有在 CI 上跑過；與跑過的版本只差這個觸發設定，以及表格 skipped 說明文字少一個重複前綴。
- 前幾次 run：第一次工具不在 dev 上（現改為從 workflow 所在的 ref 取工具）；第二、三次 apt 安裝卡住 9–30 分鐘（移除 `vulkan-tools` 後正常，原因未查明）；第四次發現 capture-capsule golden 只在 macOS 編譯，加了 skip。

## 設計說明

- 規則：逐通道差 > 12 的像素算超過容許；任何場景有一個這樣的像素就失敗，沒有數量容許。不用「任何像素不同」，因為驅動雜訊讓每個場景有數千到十萬像素差 1–4。
- 渲染方式：先把 `tests/golden` 複製出來當核准版，建立時間標記，再用 `HEX_UPDATE_GOLDENS=1 HEX_REQUIRE_GPU=1 cargo test --locked --bin hex rune_ui` 在 runner 內重寫整個 `tests/golden`，之後比對核准版與重寫版。repo 不會被改。不用 `HEX_GOLDEN_RENDERS`，因為只有 121 張的測試會寫出，meeting capsule 的測試不寫。
- 不被靜默略過：`HEX_REQUIRE_GPU` 只有 `check_golden` 會檢查，meeting／screenshot／capture capsule 的測試沒有 GPU 時直接略過。所以工具檢查每張渲染的修改時間是否晚於標記，沒被重寫就失敗（「render is older than the run」）。第一次完整實跑就靠這個抓到 48 張沒被重寫的 capture-capsule。
- 不檢查清單：`--skip 'capture-capsule-*=…'`。這 48 張的測試是 `cfg(all(test, target_os = "macos"))`，只有 Mac 會渲染。表格明列為 skipped，不是悄悄通過。Mac 上的檢查需另跑，這次沒跑。
- 工具：純 Python 標準庫（沒有新增依賴），自帶 PNG 解碼（五種 filter）；兩張 1960×3440 的圖約 1.2 秒。
- 測試：`python3 scripts/test-golden-diff.py`（14 個，用程式內產生的合成 PNG）：完全相同、全圖差 = 12 通過、一個像素差 13 失敗、尺寸不同失敗、alpha 計入、缺 render／缺 golden、渲染時間早於標記、skip 規則、五種 filter 解碼。workflow 第一步會跑它（實跑中通過）。

## 場景表格原文（run 42162978501 的 `GITHUB_STEP_SUMMARY`）

### Golden strict check: FAIL

175 scenes, 48 not checked, 1 failing. A scene fails when any pixel's channel differs by more than 12 (no count allowance).

| Scene | Over tolerance | Differing | Max channel diff | Verdict |
| --- | ---: | ---: | ---: | --- |
| capture-capsule-guess-dark |  |  |  | skipped: not checked: macOS-only goldens (cfg(target_os = "macos")); check on a Mac |
| capture-capsule-guess-five-dark |  |  |  | skipped: not checked: macOS-only goldens (cfg(target_os = "macos")); check on a Mac |
| capture-capsule-guess-five-light |  |  |  | skipped: not checked: macOS-only goldens (cfg(target_os = "macos")); check on a Mac |
| capture-capsule-guess-light |  |  |  | skipped: not checked: macOS-only goldens (cfg(target_os = "macos")); check on a Mac |
| capture-capsule-list-guess-dark |  |  |  | skipped: not checked: macOS-only goldens (cfg(target_os = "macos")); check on a Mac |
| capture-capsule-list-guess-light |  |  |  | skipped: not checked: macOS-only goldens (cfg(target_os = "macos")); check on a Mac |
| capture-capsule-no-answer-dark |  |  |  | skipped: not checked: macOS-only goldens (cfg(target_os = "macos")); check on a Mac |
| capture-capsule-no-answer-light |  |  |  | skipped: not checked: macOS-only goldens (cfg(target_os = "macos")); check on a Mac |
| capture-capsule-permission-dark |  |  |  | skipped: not checked: macOS-only goldens (cfg(target_os = "macos")); check on a Mac |
| capture-capsule-permission-light |  |  |  | skipped: not checked: macOS-only goldens (cfg(target_os = "macos")); check on a Mac |
| capture-capsule-permission-ready-dark |  |  |  | skipped: not checked: macOS-only goldens (cfg(target_os = "macos")); check on a Mac |
| capture-capsule-permission-ready-light |  |  |  | skipped: not checked: macOS-only goldens (cfg(target_os = "macos")); check on a Mac |
| capture-capsule-permission-restart-dark |  |  |  | skipped: not checked: macOS-only goldens (cfg(target_os = "macos")); check on a Mac |
| capture-capsule-permission-restart-light |  |  |  | skipped: not checked: macOS-only goldens (cfg(target_os = "macos")); check on a Mac |
| capture-capsule-picker-area-dark |  |  |  | skipped: not checked: macOS-only goldens (cfg(target_os = "macos")); check on a Mac |
| capture-capsule-picker-area-light |  |  |  | skipped: not checked: macOS-only goldens (cfg(target_os = "macos")); check on a Mac |
| capture-capsule-picker-iphone-dark |  |  |  | skipped: not checked: macOS-only goldens (cfg(target_os = "macos")); check on a Mac |
| capture-capsule-picker-iphone-light |  |  |  | skipped: not checked: macOS-only goldens (cfg(target_os = "macos")); check on a Mac |
| capture-capsule-picker-window-dark |  |  |  | skipped: not checked: macOS-only goldens (cfg(target_os = "macos")); check on a Mac |
| capture-capsule-picker-window-light |  |  |  | skipped: not checked: macOS-only goldens (cfg(target_os = "macos")); check on a Mac |
| capture-capsule-processing-dark |  |  |  | skipped: not checked: macOS-only goldens (cfg(target_os = "macos")); check on a Mac |
| capture-capsule-processing-light |  |  |  | skipped: not checked: macOS-only goldens (cfg(target_os = "macos")); check on a Mac |
| capture-capsule-queued-dark |  |  |  | skipped: not checked: macOS-only goldens (cfg(target_os = "macos")); check on a Mac |
| capture-capsule-queued-light |  |  |  | skipped: not checked: macOS-only goldens (cfg(target_os = "macos")); check on a Mac |
| capture-capsule-recording-chips-dark |  |  |  | skipped: not checked: macOS-only goldens (cfg(target_os = "macos")); check on a Mac |
| capture-capsule-recording-chips-iphone-dark |  |  |  | skipped: not checked: macOS-only goldens (cfg(target_os = "macos")); check on a Mac |
| capture-capsule-recording-chips-iphone-light |  |  |  | skipped: not checked: macOS-only goldens (cfg(target_os = "macos")); check on a Mac |
| capture-capsule-recording-chips-light |  |  |  | skipped: not checked: macOS-only goldens (cfg(target_os = "macos")); check on a Mac |
| capture-capsule-recording-dark |  |  |  | skipped: not checked: macOS-only goldens (cfg(target_os = "macos")); check on a Mac |
| capture-capsule-recording-discard-dark |  |  |  | skipped: not checked: macOS-only goldens (cfg(target_os = "macos")); check on a Mac |
| capture-capsule-recording-discard-light |  |  |  | skipped: not checked: macOS-only goldens (cfg(target_os = "macos")); check on a Mac |
| capture-capsule-recording-drawing-dark |  |  |  | skipped: not checked: macOS-only goldens (cfg(target_os = "macos")); check on a Mac |
| capture-capsule-recording-drawing-light |  |  |  | skipped: not checked: macOS-only goldens (cfg(target_os = "macos")); check on a Mac |
| capture-capsule-recording-light |  |  |  | skipped: not checked: macOS-only goldens (cfg(target_os = "macos")); check on a Mac |
| capture-capsule-rule-final-dark |  |  |  | skipped: not checked: macOS-only goldens (cfg(target_os = "macos")); check on a Mac |
| capture-capsule-rule-final-light |  |  |  | skipped: not checked: macOS-only goldens (cfg(target_os = "macos")); check on a Mac |
| capture-capsule-rule-resolving-dark |  |  |  | skipped: not checked: macOS-only goldens (cfg(target_os = "macos")); check on a Mac |
| capture-capsule-rule-resolving-light |  |  |  | skipped: not checked: macOS-only goldens (cfg(target_os = "macos")); check on a Mac |
| capture-capsule-rule-sent-dark |  |  |  | skipped: not checked: macOS-only goldens (cfg(target_os = "macos")); check on a Mac |
| capture-capsule-rule-sent-light |  |  |  | skipped: not checked: macOS-only goldens (cfg(target_os = "macos")); check on a Mac |
| capture-capsule-saved-dark |  |  |  | skipped: not checked: macOS-only goldens (cfg(target_os = "macos")); check on a Mac |
| capture-capsule-saved-light |  |  |  | skipped: not checked: macOS-only goldens (cfg(target_os = "macos")); check on a Mac |
| capture-capsule-sent-dark |  |  |  | skipped: not checked: macOS-only goldens (cfg(target_os = "macos")); check on a Mac |
| capture-capsule-sent-light |  |  |  | skipped: not checked: macOS-only goldens (cfg(target_os = "macos")); check on a Mac |
| capture-capsule-sent-rule-dark |  |  |  | skipped: not checked: macOS-only goldens (cfg(target_os = "macos")); check on a Mac |
| capture-capsule-sent-rule-light |  |  |  | skipped: not checked: macOS-only goldens (cfg(target_os = "macos")); check on a Mac |
| capture-capsule-shift-dark |  |  |  | skipped: not checked: macOS-only goldens (cfg(target_os = "macos")); check on a Mac |
| capture-capsule-shift-light |  |  |  | skipped: not checked: macOS-only goldens (cfg(target_os = "macos")); check on a Mac |
| meeting-capsule-auto-started-dark | 0 | 1652 | 2 | ok |
| meeting-capsule-auto-started-light | 0 | 1790 | 2 | ok |
| meeting-capsule-call-ended-dark | 0 | 2898 | 2 | ok |
| meeting-capsule-call-ended-light | 0 | 4118 | 2 | ok |
| meeting-capsule-ready-dark | 0 | 1832 | 2 | ok |
| meeting-capsule-ready-light | 0 | 2141 | 2 | ok |
| rune-commands-dark | 0 | 14300 | 2 | ok |
| rune-history-dark | 0 | 15845 | 2 | ok |
| rune-history-light | 0 | 4888 | 1 | ok |
| rune-language-menu-light | 0 | 18174 | 2 | ok |
| rune-mac-activity-light | 2619 | 7604 | 233 | FAIL: look at the image and regenerate |
| rune-mac-hud-lab-dark | 0 | 13757 | 2 | ok |
| rune-mac-hud-lab-light | 0 | 8846 | 3 | ok |
| rune-mac-input-source-models-dark | 0 | 41473 | 2 | ok |
| rune-mac-input-source-models-light | 0 | 37908 | 3 | ok |
| rune-mac-labs-link-dark | 0 | 13866 | 2 | ok |
| rune-mac-labs-link-light | 0 | 47177 | 3 | ok |
| rune-mac-local-build-dark | 0 | 11413 | 2 | ok |
| rune-mac-microphone-bluetooth-dark | 0 | 15735 | 2 | ok |
| rune-mac-onboarding-dark | 0 | 1365 | 1 | ok |
| rune-mac-onboarding-denied-dark | 0 | 6386 | 3 | ok |
| rune-mac-onboarding-denied-light | 0 | 10705 | 2 | ok |
| rune-mac-onboarding-light | 0 | 1649 | 2 | ok |
| rune-mac-onboarding-permissions-dark | 0 | 5164 | 3 | ok |
| rune-mac-onboarding-permissions-light | 0 | 7713 | 2 | ok |
| rune-mac-onboarding-ready-dark | 0 | 9480 | 3 | ok |
| rune-mac-onboarding-ready-light | 0 | 12233 | 3 | ok |
| rune-mac-onboarding-reopen-dark | 0 | 60225 | 3 | ok |
| rune-mac-onboarding-reopen-light | 0 | 96246 | 4 | ok |
| rune-mac-onboarding-restart-dark | 0 | 5643 | 3 | ok |
| rune-mac-onboarding-restart-light | 0 | 9192 | 2 | ok |
| rune-mac-onboarding-stale-dark | 0 | 4534 | 3 | ok |
| rune-mac-onboarding-stale-light | 0 | 5949 | 2 | ok |
| rune-mac-onboarding-try-dark | 0 | 59670 | 3 | ok |
| rune-mac-onboarding-try-light | 0 | 95489 | 4 | ok |
| rune-mac-settings-advanced-light | 0 | 17038 | 3 | ok |
| rune-mac-settings-dark | 0 | 11139 | 2 | ok |
| rune-mac-settings-ducking-dark | 0 | 11139 | 2 | ok |
| rune-mac-settings-ducking-light | 0 | 9615 | 3 | ok |
| rune-mac-settings-light | 0 | 9615 | 3 | ok |
| rune-mac-settings-permissions-dark | 0 | 11981 | 2 | ok |
| rune-mac-settings-permissions-stale-dark | 0 | 11571 | 2 | ok |
| rune-mac-settings-permissions-stale-light | 0 | 12730 | 3 | ok |
| rune-mac-settings-sides-dark | 0 | 13866 | 2 | ok |
| rune-mac-voice-filter-dark | 0 | 15672 | 2 | ok |
| rune-mac-voice-filter-driver-missing-light | 0 | 14739 | 2 | ok |
| rune-mac-voice-filter-light | 0 | 15197 | 2 | ok |
| rune-mac-voice-filter-muted-dark | 0 | 15713 | 2 | ok |
| rune-meetings-actions-dark | 0 | 27412 | 2 | ok |
| rune-meetings-calendar-dark | 0 | 23419 | 2 | ok |
| rune-meetings-calendar-denied-dark | 0 | 12317 | 2 | ok |
| rune-meetings-calendar-light | 0 | 13479 | 2 | ok |
| rune-meetings-dark | 0 | 26963 | 2 | ok |
| rune-meetings-delete-light | 0 | 9151 | 1 | ok |
| rune-meetings-empty-light | 0 | 5831 | 2 | ok |
| rune-meetings-light | 0 | 7960 | 1 | ok |
| rune-meetings-off-dark | 0 | 5962 | 1 | ok |
| rune-meetings-off-light | 0 | 4886 | 3 | ok |
| rune-meetings-platform-icons-light | 0 | 8306 | 1 | ok |
| rune-meetings-recording-dark | 0 | 20978 | 2 | ok |
| rune-meetings-recording-light | 0 | 6728 | 1 | ok |
| rune-meetings-rename-dark | 0 | 31523 | 2 | ok |
| rune-meetings-settings-dark | 0 | 12105 | 2 | ok |
| rune-meetings-settings-light | 0 | 12841 | 2 | ok |
| rune-meetings-summary-model-dark | 0 | 26789 | 2 | ok |
| rune-meetings-summary-model-light | 0 | 18731 | 2 | ok |
| rune-meetings-summary-model-not-running-dark | 0 | 28851 | 2 | ok |
| rune-meetings-summary-model-not-running-light | 0 | 19006 | 2 | ok |
| rune-menus-light | 0 | 16901 | 3 | ok |
| rune-modes-dark | 0 | 40293 | 2 | ok |
| rune-modes-light | 0 | 11664 | 1 | ok |
| rune-modes-opencode-not-running-light | 0 | 12118 | 4 | ok |
| rune-modes-picker-dark | 0 | 45946 | 3 | ok |
| rune-modes-picker-light | 0 | 13557 | 2 | ok |
| rune-onboarding-dark | 0 | 1338 | 1 | ok |
| rune-onboarding-demo-dark | 0 | 11487 | 3 | ok |
| rune-onboarding-demo-light | 0 | 27901 | 2 | ok |
| rune-onboarding-download-failed-dark | 0 | 3684 | 2 | ok |
| rune-onboarding-download-failed-light | 0 | 4298 | 2 | ok |
| rune-onboarding-light | 0 | 1629 | 2 | ok |
| rune-onboarding-model-found-back-dark | 0 | 11575 | 3 | ok |
| rune-onboarding-model-found-back-light | 0 | 27991 | 2 | ok |
| rune-onboarding-model-found-dark | 0 | 5262 | 2 | ok |
| rune-onboarding-model-found-downloading-dark | 0 | 5322 | 2 | ok |
| rune-onboarding-model-found-downloading-light | 0 | 9333 | 2 | ok |
| rune-onboarding-model-found-light | 0 | 9325 | 2 | ok |
| rune-onboarding-model-found-recommended-dark | 0 | 5250 | 2 | ok |
| rune-onboarding-model-found-recommended-light | 0 | 9313 | 2 | ok |
| rune-onboarding-no-model-dark | 0 | 12909 | 2 | ok |
| rune-onboarding-no-model-light | 0 | 12139 | 3 | ok |
| rune-onboarding-paste-dark | 0 | 4115 | 2 | ok |
| rune-onboarding-paste-light | 0 | 6670 | 2 | ok |
| rune-onboarding-ready-dark | 0 | 8673 | 3 | ok |
| rune-onboarding-ready-light | 0 | 10196 | 2 | ok |
| rune-onboarding-try-dark | 0 | 58785 | 3 | ok |
| rune-onboarding-try-light | 0 | 93298 | 4 | ok |
| rune-onboarding-waiting-dark | 0 | 1272 | 1 | ok |
| rune-onboarding-waiting-light | 0 | 1480 | 1 | ok |
| rune-output-dark | 0 | 22534 | 2 | ok |
| rune-output-light | 0 | 22707 | 2 | ok |
| rune-output-trust-light | 0 | 23662 | 2 | ok |
| rune-picker-custom-models-light | 0 | 0 | 0 | ok |
| rune-picker-downloading-dark | 0 | 0 | 0 | ok |
| rune-picker-downloading-light | 0 | 0 | 0 | ok |
| rune-picker-found-light | 0 | 0 | 0 | ok |
| rune-portal-dark | 0 | 9630 | 2 | ok |
| rune-portal-light | 0 | 11619 | 3 | ok |
| rune-portal-toggle-cancel-light | 0 | 11792 | 3 | ok |
| rune-shortcut-mode-chord-light | 0 | 105748 | 4 | ok |
| rune-shortcut-mode-dark | 0 | 13459 | 2 | ok |
| rune-shortcut-mode-key-dark | 0 | 17009 | 2 | ok |
| rune-shortcut-mode-key-light | 0 | 96416 | 4 | ok |
| rune-shortcut-mode-light | 0 | 79985 | 4 | ok |
| rune-sound-sets-dark | 0 | 10336 | 2 | ok |
| rune-sound-sets-light | 0 | 86190 | 2 | ok |
| rune-sound-sets-playing-dark | 0 | 16844 | 2 | ok |
| rune-sound-sets-playing-light | 0 | 100537 | 2 | ok |
| rune-toggle-cancel-dark | 0 | 12559 | 2 | ok |
| rune-toggle-cancel-light | 0 | 12339 | 3 | ok |
| rune-vocabulary-budget-light | 0 | 78231 | 3 | ok |
| rune-windows-onboarding-microphone-dark | 0 | 4451 | 3 | ok |
| rune-windows-onboarding-microphone-light | 0 | 7897 | 2 | ok |
| rune-windows-settings-light | 0 | 9922 | 3 | ok |
| screenshot-thumbnail-hover-dark | 0 | 0 | 0 | ok |
| screenshot-thumbnail-hover-light | 0 | 0 | 0 | ok |
| screenshot-thumbnail-rest-dark | 0 | 0 | 0 | ok |
| screenshot-thumbnail-rest-light | 0 | 0 | 0 | ok |
