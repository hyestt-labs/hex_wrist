# 報告 G2：Output 頁「Chinese characters」設定列

分支 `wristper/unit-g2-chinese-row`（從 origin/dev，含 C2、G1），commit `3fe1096e`。
未合併、未重生 golden。

## 改了什麼
- `src/rune_ui/output.rs`：新卡 `chinese_card`，放在「Every dictation」之上（`pane` 多收 `layer`，選單掛在上層）。
  - 顯示條件 `shows_chinese_characters`：`chinese_output` 已存值，或目前模型 `supports_language("zh")` / `("yue")`。
  - 選項：`System ({})`（填入解析結果）、`Traditional`、`Simplified`、`As heard`；System 存 `None`，其餘存 `Some(..)`；選已選項不存檔。
  - 存檔走 `Ui::update`（與 Speech engine 同一條路），欄位是 G1 的 `AppSettings.chinese_output`，沒有新增平行路徑。`hex settings chinese_output` 讀到同一個值（測試有驗）。
  - 夾具（`ui.is_fixture()`）的 System 固定解析為 Traditional，不讀本機語系，所以場景與測試在每台機器上相同。
  - 選擇器：`select_sized` 的欄位名稱 `tr("Chinese characters")`、值為目前選項；四個選項是 `menuitemcheckbox`，各有名稱（全部走 `tr`）。
- `src/desktop/i18n/chinese_output.rs`（新）：六個字串 × 九欄，欄位與文字照設計 §5 的表；zh-Hant 以台灣用語直接寫（中文字體／只影響中文與粵語／跟隨系統（{}）／繁體中文／簡體中文／照辨識結果），不是字元轉換。接線：`i18n.rs`（`tables()`、全語言測試）、`portuguese.rs`、`french.rs`、`russian.rs`、`traditional_chinese.rs`。
- 場景 `output-chinese`（`scenes.rs`，Breeze ASR 25、語言 zh、跟系統＝繁體），加入 playground 路徑 `Window/Output/Chinese characters` 與 `goldens()` 清單（light、dark）。
- 測試 `src/rune_ui/chinese_output_tests.rs`：放在各 OS 都編譯的位置。原因：`rune_ui::tests` 只在 Linux 編譯，放那裡這台 Mac 跑不到。
- 文件：`docs/features/dictation.md`（Chinese Characters 小節：卡、顯示條件、檢查）、`docs/features/README.md`、`AGENTS.md`（Wristper 偏離段加一句）。

## 場景與預期變動的 golden
- 新增場景：`output-chinese`（一個；沒有再加其他場景）。
- 新增 golden 兩張（目前**不存在**，Linux 上會失敗，等你重生）：`tests/golden/rune-output-chinese-light.png`、`tests/golden/rune-output-chinese-dark.png`（PANE_SIZE）。
- 現有 golden：預期**零張**改變。`output`、`output-trust`（portal 夾具，模型 ParakeetUnifiedEnglish）沒有中文也沒存值，卡不顯示（測試 `an_english_model_with_nothing_saved_hides_the_card` 對 `output`、`output-trust`、`output-empty` 斷言）。預設模型沒有中文的結論：這三個場景都由 `portal_fixture()` 設成 ParakeetUnifiedEnglish，未使用 `AppSettings::default()` 的模型；因此沒有「預設模型有中文」的情況要回報。
- 驗收規則（側欄與其他窗格在容許內）：Output 頁以外沒動；Output 頁的新卡在第一張卡之後。其餘變動範圍預期只有 Output 頁內容區、新卡以下。
- CI 實證（Check Linux run 42226350400，head `3fe1096e`）：`rune_ui::tests::goldens` 失敗，且**只列兩個缺檔**：
  - `tests/golden/rune-output-chinese-dark.png is missing`
  - `tests/golden/rune-output-chinese-light.png is missing`
  其餘 golden 都在容許內；`2048 passed, 1 failed`（那一個就是 goldens）。

## `--snap` 目視（Mac，`--size 980x760 --scale 2`，淺色與深色各一）
- `output-chinese`：卡在 agent 卡與 Every dictation 之間；標題「Chinese characters」＋說明靠左，選擇器「System (Traditional)」靠右，沒有截斷、對齊正確；深色與淺色的邊框、字色都正常。
- 額外（暫時場景，已移除，不在 commit）：俄文「Как в системе (Традиционные)」放得進 260 pt 欄位；zh-Hant 顯示「中文字體／只影響中文與粵語／照辨識結果」。
- 圖在 /tmp/g2snap/（output-chinese-light.png、output-chinese-dark.png），未提交。

## 檢查輸出原文
`scripts/check`（fmt、code-shape、clippy `--all-targets --features onnx-benchmark -D warnings`、全部測試）：
```
     Summary [  36.415s] 2636 tests run: 2636 passed, 32 skipped
exit 0
```
`cargo clippy --locked --bin hex --tests -- -D warnings`：
```
   Compiling hex v3.3.0 (/Users/glenhsu/orca/workspaces/hex_wrist/g2)
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 13.51s
```
`git diff --check`：無輸出。
`scripts/check chinese_output_tests`（新 UI 測試）：`Summary [   0.820s] 6 tests run: 6 passed, 2662 skipped`。
GitHub：Compile（run 42226354521）success；Check Linux（run 42226350400）：nix success，test 失敗，原因只有上面那兩張缺的 golden。

## 變異驗證（原文，已全部還原）
UI 測試（六個，兩個一組：現在必須發生／仍必須不發生）：
1. `an_english_model_with_nothing_saved_hides_the_card`（英文且沒存 → 不顯示；中文模型 → 顯示；`output`／`output-trust`／`output-empty` 仍不顯示）
2. `a_saved_choice_keeps_the_card_even_with_an_english_model`（已存值＋英文模型 → 顯示；沒存值 → 仍不顯示）
3. `the_options_are_named_and_system_says_what_it_resolves_to`
4. `choosing_a_script_saves_it_and_system_saves_none`（Traditional／Simplified／As heard 各存對應值；System 存 None）
5. `choosing_what_is_already_chosen_saves_nothing`（選已選項不存檔；離開已存值選 System 存 1 次；再選 System 不存）
6. `a_choice_changes_nothing_else_and_goes_through_the_settings_key`

| 變異 | 結果（原文） |
|---|---|
| M1 去掉「已存值就顯示」 | `FAIL ... a_saved_choice_keeps_the_card_even_with_an_english_model`；`panicked at src/rune_ui/chinese_output_tests.rs:97:9`；`Summary 6 tests run: 5 passed, 1 failed` |
| M2 卡永遠顯示 | `FAIL ... an_english_model_with_nothing_saved_hides_the_card`（`chinese_output_tests.rs:79:5`）、`FAIL ... a_saved_choice_keeps_the_card_even_with_an_english_model`（`:99:5`）；`Summary 6 tests run: 4 passed, 2 failed` |
| M3 System 存解析值而非 None | `FAIL ... choosing_a_script_saves_it_and_system_saves_none`（`:131:9`，`left: Some(Traditional) right: None`）、`FAIL ... choosing_what_is_already_chosen_saves_nothing`（`:153:5`，`left: Some(Traditional) right: None`）；`Summary 6 tests run: 4 passed, 2 failed` |
| M4 As heard 存成 None | `FAIL ... choosing_a_script_saves_it_and_system_saves_none`（`:131:9`，`left: None right: Some(AsHeard)`）；`Summary 6 tests run: 5 passed, 1 failed` |
| M5 選已選項也存檔 | `FAIL ... choosing_what_is_already_chosen_saves_nothing`（`:144:5`，`left: 1 right: 0`）；`Summary 6 tests run: 5 passed, 1 failed` |

i18n 測試：
| 變異 | 結果（原文） |
|---|---|
| I1 zh-Hant 表不收台灣用語 | `FAIL ... traditional_chinese_reads_as_taiwan_wording`；`panicked at src/desktop/i18n/chinese_output.rs:121:31: no entry found for key` |
| I2 台灣用語字串含「信息」 | `FAIL ... traditional_chinese_reads_as_taiwan_wording`；`"Only changes Chinese and Cantonese text" shows "只影響中文與粵語信息" with "信息"` |
| I3 某欄留空（pl「Tradycyjne」→ ""） | `FAIL ... nothing_here_repeats_another_table_or_itself`（`chinese_output.rs:87:13`） |

zh-Hant 測試檢查不含：設置、設備、連接、粘貼、信息、軟件、網絡、手表。

## 沒驗證的項目
- Linux／Windows 上的畫面：只有 Linux CI 的 `goldens` 之外的測試通過；兩張新 golden 沒有生成，沒有人看過 Linux 渲染結果。Windows 沒跑。
- 在真實 Mac／Linux／Windows 視窗裡實際點開選單、選項、存檔、聽寫一次看輸出字體：沒做（只有無頭 offscreen 與 `--snap`）。
- 選單打開後的截圖：沒拍（選單內容靠元素樹測試，不靠圖）。
- 其他語言（pl、ja、de、es、uk、pt、fr）沒看圖；只看了 ru 與 zh-Hant。譯文沿用設計表，沒有母語者審閱。
- `Linux and Windows 只讀第一個 locale` 的限制（G1 已記錄）：在這兩個 OS，「System ({})」顯示的是只看第一個語系的結果。
- 顯示條件看「模型支援 zh/yue」，不看目前選的語言：多語言模型選了英文仍顯示此卡。這是依規格「模型語言含 zh/yue」的解讀，若要改成看已選語言需要你決定。

## 後續清單（範圍外）
- 若 Linux/Windows 要看完整語言序列，需另一個單元。
- `src/rune_ui` 資料夾現在 52 個檔案（warn 40、fail 60），新增的 `chinese_output_tests.rs` 讓它多一個；可日後把 `*_tests.rs` 收進 `tests/`，但那個資料夾只在 Linux 編譯，所以這次放外面。

有擋合併問題：無（待你重生並檢查兩張 golden 後才能讓 Linux goldens 轉綠）。
