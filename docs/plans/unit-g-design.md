# Unit G：中文輸出設定（繁體／簡體／照辨識結果）

狀態：設計，未實作。G1＝邏輯、設定、CLI、測試，不碰 `rune_ui`；G2＝設定列，等 C2／F 合併。

## 0. 結論

- 轉換只放一處：`Profiles::process_cancellable` 的第一步（`src/dictation/processor.rs:318`）。
- 三個 OS 的「要處理」的聽寫都經過這裡：macOS 的 Paste／Print／Watch（`parakeet.rs:106`、`:641`），Linux（`linux/dictation.rs:239`）與 Windows（`windows/dictation.rs:914`）經 `ModeRuntime::process`（`modes.rs:48`）。
- 設定 `chinese_output`：`traditional`／`simplified`／`as_heard`，沒選時跟系統語言，檔案裡省略、不寫 `null`。
- 刪掉 B2 的手錶專用轉換（`watch/output.rs:189-202`、`src/watch/zh.rs`）。手錶改走同一處。
- 表格：國語繁 `s2tw`、國語簡 `tw2s`、粵語繁 `s2hk`、粵語簡 `hk2s`。兩個方向都不轉用語。
- 不放在轉錄引擎：`Service` 目標（本機 API）共用引擎（`parakeet.rs:93`、`:801`），不能被轉。

## 1. 管線位置與順序

### 1.1 順序（每個「處理」目標）

```ts
模型輸出（已 trim，parakeet.rs:777-782）
├── 1. 中文輸出轉換            ← 新增；結果叫 heard
├── 2. Labs：口語格式、口語符號、刪贅詞（processor.rs:326-349）
├── 3. 全域取代，再套模式的修正（processor.rs:350-352）
├── 4. OpenCode 改寫（選用，processor.rs:385）
│   └── 4a. 改寫結果再轉換一次    ← 新增；LLM 常換字體
├── 5. 自訂轉換（apply_transformations，processor.rs:821）  ← 不再轉換，使用者的程式優先
└── 6. 輸出：貼上／印出／手錶（手錶的可疑文字檢查在這之後，watch/output.rs:226）
```

- 轉換在取代之前，原因和 Wristper 一樣：使用者寫的取代詞用他看到的字體比對（`HandyText.swift:531`）。
- 4a 是與 Wristper 的差異。Wristper 在簡體時跳過整理（`ReceiverService.swift:1448`、`TranscriptPolish.swift:544`），因為它的整理提示只會寫台灣繁體。Hex 的改寫提示由使用者決定，所以不跳過，只把結果轉回設定的字體。
- 可疑文字檢查不受影響：轉換不改變文字系統（Han／Kana／Latin）。

### 1.2 Linux／Windows 的順序變動（行為變更）

- 今天 `modes.rs:55` 先跑全域取代，再進 `process_cancellable` 跑 Labs 步驟。macOS 是 Labs 步驟在全域取代之前（`processor.rs:351`）。
- G1 改成 `Profiles::new(profile).global_replacements(self.replacements.clone())`（`processor.rs:266`），全域取代移進 `process_cancellable`。
- 結果：只有一個轉換點，三個 OS 順序一致。代價：Linux／Windows 開了 Labs 口語步驟時，全域取代改在它們之後。這三個 Labs 預設關閉。

### 1.3 存哪一份文字

- `Processed` 新增 `heard: String`（步驟 1 的結果；`Processed::unchanged` 的 `heard` 等於輸入）。History 的 `raw_text` 存 `heard`。改 `parakeet.rs:617` 的 `raw: job.text` 與 `modes::history_draft` 的呼叫端（`linux/dictation.rs:249`、`windows/dictation.rs:932`）。
- 理由：存未轉換的簡體，每筆 large-v3 聽寫都會顯示「原文與結果不同」。`final_text` 與 Paste Last 存最終文字。
- 手錶：`deliver` 直接用 `request.text`／`request.raw`，`raw = (request.raw != request.text)`。`WatchReportDetails.raw` 跟著一致。

### 1.4 不轉換的路徑

| 路徑 | 轉嗎 | 原因 |
|---|---|---|
| 熱鍵聽寫、`hex dictation stop --print`、手錶 | 轉 | 都是 `is_processed`（`parakeet.rs:106`） |
| 不處理的聽寫（`--raw`、`RawPaste`／`RawPrint`） | 不轉 | 契約是「照 Hex 聽到的貼上」（AGENTS.md:961、`dictation.md:227`）；轉錯時它是逃生口。見問題 2 |
| 本機 API `/transcriptions`、`/dictations`、`/streams` | 不轉 | 給程式的原始轉錄，客戶端自己決定；`Service` 不經處理 |
| 會議逐字稿 | G1 不轉 | 走 `meeting/transcript`，牽涉接縫去重、回音判斷、Markdown、MCP；另開單元。見問題 3 |
| Annotate 的字詞時間軸 | 不轉 | 時間戳對齊原文 |
| Labs › 即時文字（HUD） | G1 不轉 | 只顯示、不貼上；後續可用同一函式 |
| History › 重新轉錄（`history/recordings.rs:117`） | 轉 | 一行；讓同一筆的各版本用同一字體比較 |

## 2. 設定

### 2.1 形狀

```rust
// src/dictation/chinese_output.rs（新檔，取代 src/watch/zh.rs）
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ChineseOutput { Traditional, Simplified, AsHeard }
pub(crate) fn resolve(saved: Option<ChineseOutput>, preferred: &[String]) -> ChineseOutput;
pub(crate) fn convert(text: &str, output: ChineseOutput, language: &str) -> String;
```

- `AppSettings` 新增 `#[serde(skip_serializing_if = "Option::is_none")] pub chinese_output: Option<ChineseOutput>`。
- `None`＝跟系統，檔案裡省略（同 `meetings.retention`）。舊版忽略未知鍵；不合法的值由 `app_settings/persistence` 換成 `None` 並記錄，不擋啟動。
- 即時生效：`apply_runtime`（`app_settings.rs:712`）把解析後的值與 `transcription.language` 發布到 live projection（同 `RECORDING_AUDIO_BEHAVIOR`，`:740`）。`process_cancellable` 每次讀它，存檔後下一筆就生效，不用重建 `Profiles`。

### 2.2 「跟系統」的對應

照偏好語言清單的順序，找第一個 `zh` 或 `yue`；沒有就是 `as_heard`（Wristper `TextRules.swift:349-363`）：

- `zh-Hant*` → 繁體；`zh-Hans*` → 簡體。明確的文字代碼勝過地區（`zh-Hans-HK` → 簡體）。
- 沒寫文字代碼：地區 `TW`／`HK`／`MO` → 繁體；`CN`／`SG`／`MY`、只寫 `zh` → 簡體。
- `yue` 沒寫 `Hans` → 繁體；`yue-Hans` → 簡體。
- 判斷沿用 `i18n::is_traditional_chinese`（`desktop/i18n.rs:125`），另加 `yue` 的規則。

需要**整份**清單。`i18n::preferred_locale()`（`i18n.rs:170`）只回第一個；英文在前、`zh-Hant` 第二的 Mac 會被誤判成 `as_heard`，手錶就不再轉繁體。G1 新增 `desktop::i18n::preferred_locales() -> Vec<String>`：macOS 用 `NSLocale.preferredLanguages`（搬 `watch/screen.rs:82` 共用）；Windows 用 `GetUserPreferredUILanguages`，再補使用者地區（`i18n.rs:226`）；Linux 用 `LANGUAGE`（冒號分隔），再 `LC_ALL`、`LC_MESSAGES`、`LANG`。

### 2.3 `hex settings`

- `optional("chinese_output", Kind::Choice(&["traditional", "simplified", "as_heard"]), "Chinese characters in dictation; none follows the system language")`，放在 `ui_language` 旁（`control/settings.rs:607`）。
- 加欄位後 `every_persisted_setting_has_a_cli_key`（`:1499`）會先失敗，加 key 才通過。測試裡要把它設成 `Some(..)`，因為 `None` 不會出現在 JSON。

### 2.4 B2 的手錶轉換與不退化計畫

- 刪 `src/watch/zh.rs` 與 `heard_as_traditional`。日文保護（假名就不轉）搬進 `chinese_output`，對所有路徑生效。
- 手錶的 `job.language`（`watch/delivery.rs:76`）是比設定更準的提示。parakeet 的處理 worker 從 `TranscriptionTarget::Watch` 取出，傳給 `process_cancellable`。
- 一次性匯入：macOS 的資料夾就是 Wristper 的（`app_paths.rs:39`）。`chinese_output` 沒選、`<support>/output.json` 有 `chinese_script` 時，照值匯入（`as-transcribed` → `as_heard`），記 migration `wristper-chinese-script`，做法同 Scrawl 匯入（`app_settings.rs:564-584`）。這台 Mac 的值是 `traditional`。
- 不退化：系統清單有繁體中文，或匯入了 `traditional` → 手錶輸出和 B2 相同。退化的只有「系統沒有任何中文、也沒有 Wristper 設定」的人：手錶從強制繁體變成照辨識結果。這和 Wristper 的預設一致。
- 回歸測試：B2 的四組測試（`watch/zh.rs` 的 tests）原樣搬到 `chinese_output`，期待值不變。

## 3. 判斷「這段是中文」

語言提示 = 手錶的 `job.language`（非空、非 `auto` 時），否則 live 的 `transcription.language`。

```ts
convert(text, output, language)
├── output == as_heard                     -> 原樣
├── 主代碼是 zh                             -> 國語表
├── 主代碼是 yue                            -> 粵語表
├── 其他明確語言（ja、ko、en…）              -> 原樣
└── auto 或空：文字裡有漢字，且沒有假名（U+3040–30FF、U+31F0–31FF、U+FF66–FF9F）也沒有諺文 -> 國語表
```

- 中英夾雜：OpenCC 只換漢字；英文、數字、程式碼、標點原樣。照轉。
- 日文：選 `ja` 一定不轉。自動偵測時有假名就不轉。
- 粵語：提示是 `yue` 才用 `s2hk`／`hk2s`（`里面` → `裏面`，國語表是 `裡面`）。自動偵測時 Hex 拿不到偵測語言（`transcription/transcript.rs:15` 沒有 language），只用國語表；差別只有少數異體字。
- 誤轉風險：自動偵測下，只有漢字的日文短句（`東京大学`）會被轉成 `東京大學`。機率低；使用者可選 `ja` 避開。
- 效能：一段句子的轉換是微秒級。字典第一次用時才載入（`LazyLock`，B2 已這樣做）。G1 記錄第一次載入的毫秒數；超過 20 ms 就在 `apply_runtime` 解析出非 `as_heard` 時預先載入。

## 4. 相依套件

- `Cargo.toml:60` 現在是 `ferrous-opencc` 0.4.0、只開 `s2t-conversion`。G1 加 `t2s-conversion`（`tw2s`、`hk2s` 需要）。
- 只換 feature，`Cargo.lock` 與 `nix/package.nix` 的 hash 不變。大小（估計，未實測）：`build.rs` 對 t2s 多編 `TSPhrases`、`TWPhrasesRev`、`TWVariantsRev`、`TWVariantsRevPhrases`、`HKVariantsRev`、`HKVariantsRevPhrases`，原文共約 23 KB。現有 `.ocb` 是原文的 1.8–2 倍（`STPhrases` 1.06 MB → 1.87 MB），所以約 +45 KB；release 執行檔約 67 MB。`TSCharacters` 已在 s2t 時編入。編譯時間增加可忽略。
- 為什麼簡體用 `tw2s` 不用 `tw2sp`：Wristper 測試要求 `軟體與網路` → `软体与网路`（`OpenCCTests.swift:15`）。0.4.0 的 `tw2sp` 含 `TWPhrasesRev`，會變成 `软件`。`tw2s` 與繁體的 `s2tw` 一樣不轉用語。

## 5. 介面（G2）

- 位置：Output 頁，獨立一張卡放在「Every dictation」之上（`rune_ui/output.rs:342`）。理由：它是輸出步驟，「不處理的聽寫」跳過它，和 Output 頁的說明一致。
- 只在目前模型的語言含 `zh` 或 `yue`、或已存了值時顯示。英文模型的使用者看不到它，也不會改變現有 golden。
- 文案（sentence case）：標題 `Chinese characters`；說明 `Only changes Chinese and Cantonese text`；選項 `System ({})`（填入解析結果）、`Traditional`、`Simplified`、`As heard`。
- 翻譯（`i18n.rs:337` 六欄＋pt／fr／ru 各自的表；zh-Hant 由簡中欄自動轉出）：

| 英文 | pl | zh | ja | de | es | uk | pt | fr | ru |
|---|---|---|---|---|---|---|---|---|---|
| Chinese characters | Znaki chińskie | 中文输出 | 中国語の字体 | Chinesische Schrift | Caracteres chinos | Китайські ієрогліфи | Caracteres chineses | Caractères chinois | Китайские иероглифы |
| Only changes Chinese and Cantonese text | Zmienia tylko tekst chiński i kantoński | 只影响中文与粤语 | 中国語と広東語だけに適用 | Betrifft nur Chinesisch und Kantonesisch | Solo cambia el chino y el cantonés | Змінює лише китайський і кантонський текст | Só altera chinês e cantonês | Ne modifie que le chinois et le cantonais | Меняет только китайский и кантонский текст |
| System ({}) | Jak w systemie ({}) | 跟随系统（{}） | システムに合わせる（{}） | Wie das System ({}) | Como el sistema ({}) | Як у системі ({}) | Como o sistema ({}) | Comme le système ({}) | Как в системе ({}) |
| Traditional | Tradycyjne | 繁体中文 | 繁体字 | Traditionell | Tradicional | Традиційні | Tradicional | Traditionnel | Традиционные |
| Simplified | Uproszczone | 简体中文 | 簡体字 | Vereinfacht | Simplificado | Спрощені | Simplificado | Simplifié | Упрощённые |
| As heard | Jak usłyszano | 照辨识结果 | 認識結果のまま | Wie erkannt | Tal como se oyó | Як розпізнано | Como foi ouvido | Tel qu'entendu | Как распознано |

- 場景：新增 `output-chinese`（中文模型、跟系統＝繁體），淺色與深色；`--snap` 目視。預期新增兩張 golden；`output`、`output-trust` 不變（預設模型沒有中文）。若預設模型有中文，這兩張也會變，G2 要回報。
- golden 只在 Linux 比對。照 F 的做法：Mac 上 `--snap --size 980x760 --scale 2` 目視，回報要重畫的檔名，由協調者跑 `check-linux.yml` 取 `golden-actual`。
- 選擇器名稱走 `tr`（`rune_ui/accessibility_tests`）。G2 等 C2／F 合併後才開始：i18n 表與 `scenes.rs` 一定衝突。

## 6. 測試、工作拆分、風險、問題

### 6.1 先失敗的測試（G1）

1. `mac_mic_simplified_becomes_traditional`：`process_cancellable` 收 `今天天气很好`、語言 `zh`、繁體 → `今天天氣很好`。抓「Mac 麥克風從不轉換」。
2. `simplified_keeps_taiwan_words_in_simplified_characters`：`軟體與網路` → `软体与网路`。抓用了 `tw2sp`。
3. `japanese_is_never_converted`：`学校へ行きます`（auto）、`東京大学`（`ja`）不變。抓漢字被改。
4. `cantonese_uses_hong_kong_tables`：`yue` 下 `里面` → `裏面`。
5. `conversion_runs_before_replacements`：取代 `頭髮→hair`，輸入 `头发` → `hair`。抓順序錯。
6. `rewrite_output_is_converted_again`：用 `processor.rs:1518` 的假伺服器回簡體 → 貼繁體。
7. `raw_and_service_targets_are_not_converted`：沿用 `parakeet.rs:1401` 的 `processing_job`。
8. `history_raw_is_the_converted_heard_text`：`CompletedTranscript.raw` 是 `heard`，不是 `job.text`。
9. `watch_simplified_setting_is_not_forced_traditional`：簡體設定下手錶貼簡體，`WatchReportDetails.raw` 為 `None`。抓 B2 殘留。
10. `linux_windows_global_replacements_follow_labs_steps`：鎖住 1.2 的新順序。
11. `system_default_follows_first_chinese_preferred_language`：`[en-US, zh-Hant-TW]`→繁、`zh-Hans-HK`→簡、`zh-MO`→繁、`zh`→簡、`yue`→繁、`yue-Hans`→簡、`zh_TW.UTF-8`→繁、`[en, ja]`→照辨識。
12. 設定：`unchosen_chinese_output_is_omitted_not_null`、`invalid_chinese_output_falls_back_without_blocking_launch`、`wristper_chinese_script_is_imported_once`（只匯一次、不蓋掉已選的值）。
13. CLI 與即時：`chinese_output_cli_accepts_none_and_three_values`、會自動失敗的 `every_persisted_setting_has_a_cli_key`、`changing_chinese_output_applies_to_the_next_dictation`。

### 6.2 給 Sonnet 的 G1 步驟（每步一個 commit）

1. Cargo feature `t2s-conversion`；新 `dictation/chinese_output.rs`（表格、偵測、`resolve`）；搬 B2 測試；測試 2–4、11。
2. `AppSettings.chinese_output`、live projection、Wristper 匯入、`hex settings` key；測試 12–13。
3. `process_cancellable` 步驟 1 與 4a、`Processed.heard`、語言參數；`modes.rs` 改用 `global_replacements`；測試 1、5、6、10。
4. parakeet：手錶語言提示、History raw 改 `heard`、刪 `watch/zh.rs` 與 `heard_as_traditional`；Linux／Windows 的 `history_draft`；重新轉錄；測試 7–9。
5. 文件：AGENTS.md:632 與 :1237（「只給手錶」的不變式）、:50（若問題 1 照建議）、`docs/features/watch.md:33,134`、`docs/features/dictation.md`（處理順序）、`docs/cli.md`。跑 `scripts/check`。

### 6.3 風險

- 使用者以前為了修正 large-v3 的簡體輸出，寫了簡體的取代詞。轉成繁體後就比對不到。G1 在文件寫明；之後可在建取代表時一併轉換漢字。
- 鍵盤切換模型時，處理讀到的 live 語言可能已是新的：極窄的競態，最壞是用錯表或不轉。

### 6.4 要使用者決定的問題

1. **Labs 規則**：AGENTS.md 要求使用者要求的新功能放 Labs、預設關閉。放 Labs 的話，手錶會失去 B2 預設的繁體轉換。建議：寫進 2026-11-04 的 Wristper 例外，預設跟系統、不放 Labs。G1 的預設值取決於這題。
2. **不處理的聽寫要不要轉？** 建議不轉，保留「照 Hex 聽到的」契約與逃生口。Wristper 沒有這個模式。
3. **會議逐字稿要不要轉？** 建議 G1 不轉，需要時另開單元。
