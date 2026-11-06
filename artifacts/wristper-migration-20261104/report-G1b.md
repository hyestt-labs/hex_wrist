# G1b 修正報告

分支 `wristper/unit-g-chinese-output`，修正 Terra 審查（terra-G1-review.md）的擋合併問題。

## 改動

1. `src/dictation/chinese_output.rs`：新增 `apply_in_language(text, language)`，語言已知（含 `auto`）時不回退。`auto` 只看文字內容（有漢字且無假名／諺文用國語表），不讀全域模型語言。`apply` 保留原回退行為（空或 `auto` 讀模型語言），改為呼叫新入口，手錶與一般聽寫不變。
2. `src/history/recordings.rs`：`retranscribe` 改呼叫 `apply_in_language`。
3. 測試：
   - `a_chosen_auto_ignores_the_dictation_models_language`：全域 `en` + 重新轉錄 `auto` 的中文 → 轉成繁體；手錶 `apply("头发","auto")` 在全域 `en` 仍不轉；全域 `zh` + 重新轉錄 `auto` 的「학교 学校」→ 不被當 zh 轉（zh 路徑會跳過諺文檢查，因此這是能區分的案例）；明確 `zh`／`en` 照舊；手錶 auto 在全域 `zh` 仍轉。
   - `explicit_chinese_with_kana_leaves_the_whole_sentence_alone`：`zh`、`yue`、`zh-TW`、`auto` 搭配「今天天气很好，去コンビニ」及繁體 yue 句，Traditional 與 Simplified 皆整句不轉。
4. 文件：`docs/features/dictation.md` 記載手錶 auto／空語言回退模型語言、`en` 下不轉是已知結果、含假名整句不轉的產品取捨（寧可不轉也不破壞日文漢字）、重新轉錄的 `auto` 規則、舊記錄不追溯、`--raw` 保留未轉結果、搜尋以新記錄轉換後的 raw 為準；`AGENTS.md` History 不變式補同一句。

## 變異驗證

把 `apply_in_language` 改回舊行為（`auto` 時讀 `transcription_selection().1.language`）後執行
`scripts/check chinese_output::tests::a_chosen_auto`：

```
        FAIL [   0.038s] (1/1) hex::bin/hex dictation::chinese_output::tests::a_chosen_auto_ignores_the_dictation_models_language
     Summary [   0.043s] 1 test run: 0 passed, 1 failed, 2588 skipped
    thread 'dictation::chinese_output::tests::a_chosen_auto_ignores_the_dictation_models_language' (7846624) panicked at src/dictation/chinese_output.rs:399:9:
    assertion `left == right` failed
      left: "头发"
     right: "頭髮"
```

還原後測試通過（已用 diff 確認檔案與修正版一致）。

## 驗證結果

- `cargo fmt --check`：乾淨。
- `git diff --check`：乾淨。
- `cargo clippy --locked --bin hex -- -D warnings`：乾淨。
- `scripts/check` 全套：`Summary [29.527s] 2557 tests run: 2557 passed, 32 skipped`，exit 0。

## 未做

- Linux／Windows 未編譯；實機手錶與 History 重新轉錄的真實轉錄未驗證。
- Terra 後續清單的其他項目（migration 標記持久性、預載競態等）不在本次範圍。
