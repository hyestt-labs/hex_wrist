# 單元 F 設計：五個新語音模型與誠實的數字

狀態：設計，待審。實作交給 Sonnet。依據 `spec-F.md` 與 `models-comparison.md`（以下稱「比對檔」）。

## 0. 需要使用者決定的事（先看這裡）

1. **`src/rune_ui` 不能完全不碰。** `facts.rs:231-254` 的 `blurb` 是窮舉 match，新 ID 不加分支就編不過。每個新 `tr()` 字串都要有全部翻譯（`i18n.rs:1726` 的測試；表在 `i18n.rs` 與 `i18n/{russian,portuguese,french}.rs`）。建議：只加 5 個 blurb 分支與翻譯，其他不動。
2. **Goldens 會變。** `picker-custom-models` 場景畫出全部模型（`scenes.rs:150-154`）。Golden 只在 Linux 跑（AGENTS.md）。fork `hyestt-labs/hex_wrist` 能不能 dispatch `check-linux.yml` 還不知道。建議：能跑就取 `golden-actual`，不能就在 PR 標明「goldens 待補」。
3. **非 macOS 的速度標籤。** 建議 Linux／Windows 先顯示「—」，理由見第 4 節。
4. **不放 Labs。** 使用者已決定。AGENTS.md 要求新功能進 Labs，但 catalog 模型有先例：只有 Ultra 與匯入模型受 Labs 控制（`models.rs:100-102`），NB-Whisper、Granite、GigaAM 都沒有。建議在 AGENTS.md 的 fork 偏離段落補一句。

## 1. 範圍

- 新增：Whisper large-v3、Whisper medium、Whisper small、Breeze ASR 25、Qwen3-ASR 1.7B，全部 Q8_0。
- 不加 Moonshine（引擎家族清單沒有，`custom_models.rs:37-45`）。
- 不改：預設模型、`choices_for_runtime` 全表（含 `zh`）、`recommended_selection`、既有模型的 `realtime`、`word_error_rate`、`benchmark()` 紀錄、Turbo 的 pin、Vulkan shader 型別、OpenCC。
- `ModelDetails` 不給新模型。原因：`rune_ui/model_details.rs:108-110` 寫死「Hex has run it only on Linux (Vulkan)」，`:160-163` 把每個數字標成 `% WER`。中文是 CER，所以會顯示錯的字。

## 2. Pin 驗證（沒有下載整個檔）

檢查方法（2026-11-05 執行，腳本在 `/tmp/f-pins-scripts/`，不進 repo）：
1. `GET https://huggingface.co/api/models/handy-computer/<repo>/tree/<rev>`：讀 `lfs.oid` 與 `lfs.size`。
2. `HEAD .../resolve/<rev>/<file>`：讀 `x-linked-etag`、`x-linked-size`、`x-repo-commit`。
3. `GET .../api/models/<repo>/revision/<rev>`：回傳的 `sha` 等於 pin，所以 rev 是 40 位 commit，不是分支名。
4. `curl -r 0-16777215`：只讀前 16 MiB，解析 GGUF 檔頭。

| 模型 | repo（handy-computer/） | revision | 檔名 | bytes | sha256 |
|---|---|---|---|---|---|
| Whisper large-v3 | whisper-large-v3-gguf | e3e29bee6389c7da4a141406f07bb80ddac5337c | whisper-large-v3-Q8_0.gguf | 1,668,741,440 | 2fa1a5f179f8a511a53e2108db270aa4af3ce08cd976af4180e2854666bb4ba3 |
| Whisper medium | whisper-medium-gguf | ec78f06fded51aa82cde751678b78f76f78c8b7f | whisper-medium-Q8_0.gguf | 831,538,144 | 09e6a65e7de377aa5b10bae24608bc6f8ca2ed04b3993ef10d4a02bcd9a82adf |
| Whisper small | whisper-small-gguf | c0214bd34be9296695486f838e0142f900803159 | whisper-small-Q8_0.gguf | 269,751,136 | 9b9c8811bbcc82a7766f0fb0925614bdacb0923b2cc630daeac17108b655b860 |
| Breeze ASR 25 | Breeze-ASR-25-gguf | 1e5e8d7295110e1e14e305e9cf7411d82711beaa | Breeze-ASR-25-Q8_0.gguf | 1,667,964,224 | 1650c163cd1623d13b585368ffd87d7e669c5ff23014511e0fa91bb4ce994756 |
| Qwen3-ASR 1.7B | Qwen3-ASR-1.7B-gguf | 92282af1610a2db19d66f2bef1e260f5deca782d | Qwen3-ASR-1.7B-Q8_0.gguf | 2,185,030,624 | 9a0d81792dfea2d5f278b8a63deb3ea6e02139ce42c2301f32ea19c4f77526b7 |

結果：5 個檔的 tree `lfs.oid`、`x-linked-etag`、Wristper（`SpeechModels.swift:80-84, 93-97, 145-149, 171-175, 184-188`）三方一致。大小也和 transcribe.cpp catalog 的 `downloads` 一致。

GGUF 檔頭（`artifacts.rs` 的 `architecture`／`variant` 照抄這裡，不能猜）：

| 模型 | general.architecture | stt.variant | 張量 | general.languages | lang_detect |
|---|---|---|---|---|---|
| large-v3 | whisper | whisper-large-v3 | 748 F32、513 Q8_0 | 100（含 yue、tl；無 fil） | true |
| medium | whisper | whisper-medium | 564 F32、385 Q8_0 | 99（缺 yue；無 fil） | true |
| small | whisper | whisper-small | 288 F32、193 Q8_0 | 99（缺 yue；無 fil） | true |
| Breeze | whisper | breeze-asr-25 | 748 F32、513 Q8_0 | 99（缺 yue） | true |
| Qwen 1.7B | qwen3_asr | qwen3-asr-1.7b | 365 F32、344 Q8_0 | 30，等於 `QWEN_LANGUAGES` | true |

和比對檔的差異：
- 比對檔的 Metal「14.8–23.9x」只來自 M4 Max。catalog 在 Hex 固定的 transcribe.cpp `2778ae4`（`Cargo.toml:45`）還有 base M4（`db678a03`，2026-10-22），慢約 2.3 倍。例如 large-v3 在 M4 只有 6.6–8.3x。
- 準確度紀錄是 `legacy-published`，`engine_sha` 與 `measured_on` 都是 null。L40S／CUDA 是 profile 的設定，不是這些紀錄的事實，所以 `hardware` 填 None。
- variant 字串會撞名：`whisper-medium` 等於 NB-Whisper（`artifacts.rs:190`），`qwen3-asr-1.7b` 等於 QwenCleo（`artifacts.rs:74`）。不影響：載入只比對自己的 artifact（`gguf_session.rs:63-68`），HF 快取比對用 sha 或 repo＋檔名（`found_models.rs:286-303`）。
- 上游 HEAD 與 catalog 記錄的 `upstream_commit` 一致，授權都是 apache-2.0（HF API `cardData.license`）。

## 3. 引擎家族與語言

- 家族由 `GgufArtifact.architecture` 決定，不另加欄位。`vocabulary_use` 依家族給 `Prompt`（`models.rs:345-357`），詞彙上限 whisper 500、qwen 1,000（`vocabulary.rs:251-252`）。
- large-v3／medium／small：`languages: &["*"]`、hint 開、偵測開，和 Turbo 相同（`models.rs:435-445`）。
- **缺陷 1：粵語。** Settings 列了 `yue`，`"*"` 會讓 medium／small 提供它。引擎遇到不支援的語言會報錯（transcribe.cpp `src/arch/whisper/model.cpp:1447`）。修法：`supports_language` 對 `WhisperMedium | WhisperSmall` 排除 `yue`，寫法同 `runtime_language` 的 id 分支。
- **缺陷 2：菲律賓語。** Settings 用 `fil`，Whisper 檔只有 `tl`。`fil→tl` 現在只對 Turbo 生效（`models.rs:272`）。修法：改成所有 `"*"` 的 whisper 模型都對應。
- Breeze：`languages: &["zh", "en"]`（catalog `breeze-asr-25.json`）、hint 開、偵測開。檔案宣告支援偵測。偵測關掉的話，選 Auto 的人換到 Breeze 會退回 `en`（`models.rs:306-312`）。
- Qwen 1.7B：`QWEN_LANGUAGES`、hint 開、偵測開，同 0.6B（`models.rs:446-456`）。長音訊策略是 `hard-cap`，和 0.6B 走同一條分段路徑。
- ID 與存檔名：`WhisperLargeV3`／`whisper_large_v3`、`WhisperMedium`／`whisper_medium`、`WhisperSmall`／`whisper_small`、`BreezeAsr25`／`breeze_asr_25`、`Qwen3Asr17B`／`qwen3_asr_1_7b`。
- `MODELS` 的位置：5 筆加在 `AppleSpeech` 之前。順序決定選單排序與匯入模型的退路（`custom_models.rs:452-470`），放最後就不會改變既有模型的相對順序。

## 4. 資料形狀與讀取端的遷移

原則：新 API 用「依 id 查詢的函式」，不在 `ModelDefinition` 加欄位。加欄位要改 17 筆 `MODELS` 與 `custom_models.rs:339`。舊 API 留到 UI 單元遷移完再刪。

```rust
// model_benchmarks.rs
pub enum Backend { Metal, Vulkan, Cpu }
pub struct SpeedRecord {
    pub backend: Backend, pub machine: &'static str, // "Apple M4", "Apple M4 Max", "Ryzen 7 4750U"
    pub sample: &'static str, pub sample_seconds: f64, // "dots" 35.332, "jfk" 11.0
    pub realtime: f64,                                  // xrt_wall
    pub engine_revision: &'static str, pub measured_on: &'static str,
}
pub fn speed_records(id: TranscriptionModelId) -> &'static [SpeedRecord];
/// 35 秒樣本、該 backend 最慢的參考機器；沒有紀錄回 None（顯示「—」）。
pub fn speed(id: TranscriptionModelId, backend: Backend) -> Option<&'static SpeedRecord>;
pub fn accuracy(id: TranscriptionModelId) -> &'static [BenchmarkRecord]; // 一個語言一筆
pub fn accuracy_in(id: TranscriptionModelId, language: &str) -> Option<BenchmarkRecord>;
// artifacts.rs：每個 id 一個明確分支（沒有 `_`），既有模型先回 None
pub struct ModelSource { author, license_spdx, license_url, upstream_repo, upstream_revision, converted_by }
pub fn source(id: TranscriptionModelId) -> Option<&'static ModelSource>;
```

- `BenchmarkRecord`（`model_benchmarks.rs:29-47`）加 `quantization: Option<&str>` 與 `ci95: Option<(f64, f64)>`。既有常數用 `..` 展開或 `unrecorded()`，`rune_ui/picker.rs` 的測試也用展開，所以都能編譯。
- 新模型的準確度：transcribe.cpp catalog `2778ae4` 的 FLEURS test、Q8_0、batch 8、不要時間戳。`source` 填「transcribe.cpp catalog, legacy-published」，`source_revision` 填 `2778ae4`，`hardware` 填 None。只收 Settings `LANGUAGES` 裡、模型也支援的語言。Breeze 的 `zh` 有兩筆（batch 8 與 batch 1，後者沒有 ci95），取 batch 8，才能和其他模型比。錯誤率 ≥100% 的列（例如 am 128.3%）照存；UI 單元再決定怎麼顯示。
- 這兩張表用腳本 `scripts/models/catalog-records.py` 從固定的 catalog 產生，輸出到 `src/transcription/model_benchmarks/catalog.rs`，檔頭寫明 rev。不放成第三個 `model_*.rs`（會觸發 code-shape 的前綴規則）。

| 舊讀取端 | 單元 F 做法 | UI 單元做法 |
|---|---|---|
| `facts.rs:106-117` `wait()` 解析 `def.realtime` | 新模型的 `realtime` 依 OS 編譯期選：macOS 用 M4 Metal dots，其他 `"Unknown"`（顯示「—」） | 改讀 `speed(id, 目前 backend)`，再刪 `realtime` |
| `facts.rs:154-158` `errors()`、`list.rs:281`、`picker.rs:250-265` 讀 `benchmark()` | 新模型的 `benchmark()` 回代表語言：三個 Whisper 是 `en`，Breeze 與 Qwen 1.7B 是 `zh` | 改讀 `accuracy_in(id, 語言)` |
| `model_benchmarks.rs:208-239` `benchmark()`、`most_accurate_is_comparable` | 新模型加明確分支；比較邏輯不變 | 同上 |
| `custom_models.rs:339-340` | 不動（`"Unknown"`、None） | 不動 |
| `ModelDetails`（`model_details.rs:72-77`） | 新模型回 None | 改 UI 文案後再用 `source()` |

macOS 的 `realtime` 標籤：large-v3 `~8x`、medium `~15x`、small `~40x`、Breeze `~8x`、Qwen 1.7B `~11x`。選 base M4 的理由：既有標籤來源不明（Turbo 標 `~19x`，`models.rs:438`；catalog 是 M4 15.0x、M4 Max 51.1x、4750U Vulkan 3.7x）。用 M4 Max 會讓 large-v3 看起來和 Turbo 一樣快；用 4750U 在 Linux 會和既有標籤差約 5 倍。large-v3 與 Breeze 的 blurb 寫明「Mac 以外很慢」。

## 5. 數字的誠實規則

- 每筆數字都要有來源、資料集、量化、機器、backend、engine rev。不知道就填 None，不猜。
- 速度（xRT，`xrt_wall`；jfk 11 秒／dots 35.3 秒）：

| 模型 | M4 Max Metal | M4 Metal | 4750U Vulkan | M4 Max CPU | M4 CPU | 4750U CPU |
|---|---|---|---|---|---|---|
| large-v3 | 14.8／19.7 | 6.6／8.3 | 1.7／2.3 | 3.0／4.4 | 1.9／2.9 | 0.8／1.1 |
| medium | 32.0／34.6 | 12.3／14.9 | 3.6／4.6 | 5.8／8.7 | 3.6／5.3 | 1.6／2.3 |
| small | 89.3／88.1 | 34.8／39.8 | 10.8／12.6 | 17.0／24.3 | 10.8／15.6 | 5.1／6.7 |
| Breeze | 20.7／23.9 | 6.8／8.5 | 1.7／2.4 | 3.0／4.5 | 1.9／2.9 | 0.8／1.1 |
| Qwen 1.7B | 44.3／36.2 | 13.2／10.5 | 4.4／3.7 | 10.3／8.8 | 6.8／6.1 | 2.7／2.3 |

  engine rev：M4 Max `77b0c93`（2026-09-14）、4750U `218aeae3`（Qwen 是 `cd0ea568`，2026-09-14）、M4 `db678a03`（2026-10-22）。
- 準確度（FLEURS test、Q8_0、batch 8）：large-v3 en 4.03% WER、zh 7.98% CER；medium 4.64／13.13；small 6.51／23.06；Breeze 4.14／8.10；Qwen 1.7B 3.23／7.14。en 647 句，zh 945 句。
- 新模型的 `word_error_rate` 等於它 `benchmark()` 代表紀錄的值，由測試綁住。
- 這台 Mac 是 M5 Pro，不是參考機器。實跑的 RTF 標成「Hex's run」，只寫進報告，不寫進 catalog 紀錄。
- 沒有紀錄的 backend 顯示「—」，不拿別的 backend 的數字代替。

## 6. Vulkan 量化

- 5 個檔只有 F32 與 Q8_0 張量（第 2 節檔頭），在 `GGML_VULKAN_SHADER_TYPES` 的範圍內（`.cargo/config.toml`、AGENTS.md）。不用加型別。
- 既有測試 `gguf_artifacts_are_q8_0_for_the_vulkan_shader_subset`（`models.rs:1638-1652`）只看檔名；新檔名都以 `-Q8_0.gguf` 結尾，會通過。

## 7. 必須不變的預設與推薦

- 預設 `ParakeetUnifiedEnglish`（`models.rs:33-36`）、預設語言 `en`（`:171-178`）、`choices_for_runtime`（`:652-727`）、`recommended_selection`（`:1272-1289`）。
- 既有測試（`:1549`、`:1564`、`:1881`）沒有凍結整張表。所以第一個 commit 先加特徵測試 `recommendations_stay_as_before_unit_f`：列出每個 `LANGUAGES`（含 `auto`）的 `(id, badge)` 清單、`recommended_selection(lang).model`、`TranscriptionModelId::default()`。它在 base commit 上必須是綠的。

## 8. 先失敗的測試（每個測試抓一個缺陷）

| 測試 | 抓的缺陷 |
|---|---|
| `recommendations_stay_as_before_unit_f`（先 commit、先綠） | 新模型混進推薦或預設 |
| `new_pins_are_the_verified_files` | pin 打錯字，或 rev 不是 40 位小寫 hex |
| `catalog_files_are_unique` | (repo, 檔名) 或 sha 重複，例如把 QwenCleo 的 pin 貼給 Qwen 1.7B |
| `every_model_id_round_trips`（含新 5 個） | 漏了 `from_str` 分支，存好的選擇讀不回來 |
| `whisper_medium_and_small_do_not_offer_cantonese` | 選單提供 `yue`，引擎在 `model.cpp:1447` 報錯 |
| `every_any_language_whisper_maps_filipino_to_tl` | `fil` 送進 large-v3／medium／small，引擎報錯 |
| `speed_picks_the_slowest_reference_machine_per_backend` | 拿 M4 Max 或別的 backend 的數字當等待時間 |
| `realtime_label_matches_this_os_backend`（cfg 分 macOS 與其他） | 標籤和紀錄不一致，或在 Vulkan 顯示 Metal 數字 |
| `accuracy_has_one_record_per_supported_language` | Breeze 的重複 `zh`、模型不支援的語言卻有紀錄 |
| `headline_benchmark_matches_word_error_rate` | 兩個數字各改各的 |
| `new_models_have_a_source_but_no_details` | 缺授權或作者；或 `details()` 讓 UI 顯示錯的 WER／Linux 文案 |
| `new_models_read_the_vocabulary_as_a_prompt` | 家族字串打錯，詞彙沒送進模型 |

## 9. Sonnet 的工作順序（每步以一個檢查結尾，每步一個 commit）

1. 加特徵測試。檢查：`scripts/check transcription::models` 綠。
2. 加 5 個 artifact 與 `source()` 表；加 ID、`as_str`、`from_str`、`MODELS` 項目、`blurb` 與翻譯。檢查：pin、唯一性、round-trip 測試綠，i18n 測試綠。
3. 修 `yue` 排除與 `fil→tl`。檢查：兩個測試先紅後綠。
4. 寫 `scripts/models/catalog-records.py`，產生 `model_benchmarks/catalog.rs`；加 `Backend`、`SpeedRecord`、`speed()`、`accuracy*()`、`benchmark()` 分支、依 OS 的 `realtime`。檢查：第 8 節其餘測試綠。
5. SDK `MODEL_IDS`（`sdk/typescript/src/types.ts:1-18`）、changeset（先例 `sdk/.changeset/nb-whisper-medium.md`）、`docs/specs/local-transcription-service.md:186`、功能地圖 `docs/features/README.md:318`、`dictation.md:822`、AGENTS.md 偏離說明。檢查：`cd sdk/typescript && bun run check && bun run test`。
6. 全部檢查：`cargo fmt --all --check`、`cargo clippy --locked --bin hex --all-targets -- -D warnings`、`cargo nextest run --locked --bin hex`、`python3 scripts/code-shape.py`。Linux 與 Windows 的 clippy 交給 `compile.yml`，或在 PR 註明沒跑。
7. 用 `--snap --scene picker-custom-models` 在 Mac 畫改前／改後各一張並比較；goldens 依第 0 節第 2 點處理。
8. 實跑（第 10 節）。

行數預估：`models.rs` 非測試行 1,378 → 約 1,460（硬上限 1,500，沒有例外）。超過就先把 `MODELS` 搬到 `artifacts.rs`，單獨一個 commit。`model_benchmarks.rs` 約 330 行，`catalog.rs` 約 380 行，`artifacts.rs` 約 340 行。

## 10. 實跑驗證

- 隔離：`HEX_APPLICATION_SUPPORT_DIR`、`HF_HUB_CACHE`、`HF_HOME` 都指到 `mktemp -d` 的資料夾；`HEX_DISABLE_UPDATES=1`。不設 `HF_*` 的話，`found_models` 會掃使用者真的 HF 快取，可能跳過要測的下載（`found_models.rs:78-79`）。
- 程式：`cargo build --release --bin hex`，再跑 `hex service`（無視窗、無快捷鍵）。從暫存資料夾的 `local-api.json` 取 port 與 token。先確認 log 裡沒有啟動 `watch` 執行緒。
- 下載：`POST /models/<id>/prepare`。這條路走 catalog 的大小與 sha 驗證。
- 轉錄：`POST /transcriptions?model=<id>&language=<lang>`，body 是 16 kHz 單聲道 WAV。
- 音檔：`say -v Samantha` 英文約 10 秒與約 30 秒各一段；`say -v Meijia`（zh_TW）中文約 10 秒一段。用 `afconvert -f WAVE -d LEI16@16000 -c 1` 轉檔。
- 模型：Whisper small（en、zh）與 Breeze ASR 25（zh、en）。Breeze 是唯一只支援中英的新模型，可以順便檢查「直接輸出繁體」與「沒有標點」。Qwen 1.7B 和 QwenCleo 同架構，Hex 已經在跑。
- 記錄：逐字稿、音檔秒數、轉錄秒數、RTF（含與不含載入）、機器 M5 Pro、backend Metal、任何錯誤原文。
- 收尾：用確切 PID 停 `hex service`；暫存資料夾用 `/usr/bin/trash` 移除。不碰 `~/Library/Application Support/hex`。

## 11. 風險與待決問題

- 從 Wristper 來的人：它的 Turbo pin 和 Hex 不同（比對檔第 1 節），會重下 886 MB。
- Whisper 讀詞彙提示，Wristper 看過「Glossary」被唸回來（比對檔第 4 節）。新加的 4 個 Whisper 檔也可能發生。
- 用舊版 Hex 打開新的存檔名會解不出來，舊版沿用預設、不改檔（`models.rs:29-32`）。可接受。
- 如果有人先前把同一個檔匯入成自訂模型，選單會出現兩筆。低風險，不處理。
- 待決：Breeze 對台灣國語要不要另外推薦？FLEURS zh 不是台灣國語，沒有可比的公開數字。本單元不改推薦。
