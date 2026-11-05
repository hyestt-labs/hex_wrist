# Wristper on Hex：Mac 端 Rust 化技術方案

狀態：草案，2026-11-04。基準 Hex `afb11de5`（`dev`），Wristper main `4261853`。
來源：`artifacts/wristper-migration-20261104/` 的 PLAN、hex-ground、wristper-protocol、wristper-setup-licensing、wristper-agent 五份盤點。
本文件只讀程式與盤點推得，沒有建置、沒有連 relay、沒有實機測試。

## A. 一句話結論與階段總覽

結論：在 `hyestt-labs/hex_wrist` 新增 `src/watch/`，把手錶音檔接進 Hex 現有的 `DictationWorker`（同一個引擎、同一個 Paster），線路格式與 Wristper 4 版協定逐位元相容，買斷授權在 Mac 本機計額度。

第一階段分五個單元，每個單元一個 PR 到 `dev`，編譯與該單元測試通過後自行 merge（使用者 2026-11-04 授權）：

1. **單元 A：fork 身分與護欄。** bundle id、資料根目錄、URL scheme、關掉上游 updater 與回報入口、fork 版 `AGENTS.md`、`DIFFERENCES` 條目。PR 標題「fork: Wristper 身分與上游入口停用」。
2. **單元 B：協定核心（純邏輯）。** 框格式、E2EE、路由表、配對儲存與舊資料讀取、去重表、golden vectors。沒有網路、沒有 UI。PR「watch: 協定核心與 golden vectors」。
3. **單元 C：relay 連線與配對。** WebSocket 執行緒、心跳與退避、claim、`/pairing/*`、Apple Watch 設定頁、引導的配對頁。PR「watch: relay 連線與配對」。
4. **單元 D：送達管線。** m4a 解碼、`TranscriptionTarget::Watch`、AX 欄位快照與比對、讀回確認、`/status`、`/enter`、`/undo`、`/health`、HUD 狀態。PR「watch: 音檔送達與貼上驗證」。
5. **單元 E：授權與額度，引導收尾。** Polar 啟用流程、每日額度、20 秒上限、402、`wristper://activate`、QR 安裝頁、只用手錶不要求麥克風。PR「licensing: 買斷授權與免費額度」。

順序是 A → B → C → D → E。B 不依賴 A 的 UI，可以和 A 並行開發，但照順序 merge。

第二階段（Agent）：本次只交方案（第 I 節）。建議拆成 2a 背景接續、2b Claude 開著的 Session 核准轉手錶、2c OpenCode 加分項；「語音送進開著的 TUI」不排。

## 已定決策與理由

1. **新模組 `src/watch/`。** 跨平台核心放在 `watch/`；只有 AX 欄位、Enter、Undo 放 `watch/macos/`。理由：Windows、Linux 之後只補 OS adapter，核心不動。
2. **手錶音檔走 `DictationWorker`。** 新增 `TranscriptionTarget::Watch(WatchDelivery)`。理由：`src/parakeet.rs:367` `submit` 已處理序號、取消與 `engine.finish_with`；`finish_output`（`:945`）是唯一的 Paster owner 與保序點。接 `local_api` 的 `/transcriptions` 不行：`src/transcription/service.rs:460` 超過 30 秒或等待失敗時會載入第二份模型。
3. **手錶功能預設開啟，不進 Labs。** 這是刻意偏離 Hex「新功能進 Labs」規則（`AGENTS.md` Purpose 段）。理由：手錶是 Wristper 的付費核心。免費版 20 秒上限也刻意偏離 Hex「capture 沒有自動長度上限」。兩條都寫進 fork 版 `AGENTS.md`。
4. **授權照搬 Wristper 的 Polar customer-portal 流程。** 額度只在 Mac 計，唯一讀取點，修掉 Wristper 旗標不一致（`LicensingStore.swift:61-62` 預設開，`LicensingViews.swift:91,223,250` 讀成關）。
5. **沿用舊資料，不重新配對。** 讀 `~/Library/Application Support/Wristper/{relay-room,relay-secret,paired-devices.json}` 與 Keychain service `dev.wristper.mac.device`。
6. **身分改成 `dev.wristper.mac`、`wristper://`。** 上游 updater 與回報入口先停用；舊 0.1.x 使用者經既有 Sparkle appcast 升級到新 App。保留 Hex 的 MIT `LICENSE`。
7. **網站沿用 `wristper_web/`**，只換 DMG 連結。

使用者可推翻的決策（預設值見第 J 節）：第 5 點的「不支援無金鑰舊配對與 4 位數配對」、第 3 點的「Agent 也不進 Labs」、第 6 點的「更新先停用」、免費額度是否計入 `local_api` 轉錄、只用手錶時是否要求 Mac 麥克風。

## B. 架構與資料流

```ts
Wristper.app（dev.wristper.mac，一個常駐程序）
├─ watch/relay           執行緒 "watch-relay"：tungstenite（sync）＋ native-tls
│  ├─ 連 wss://relay.wristper.dev/m/<room>，header X-Wristper-Mac-Secret
│  ├─ 每 30 s 送 "ping"；60 s 沒收到任何框就重連；退避 1,2,4…30 s
│  └─ 組 req 框＋二進位分段 → RelayRequest；寫回 res 框
├─ watch/core            執行緒 "watch"：唯一狀態 owner（單一 mpsc 收件匣）
│  ├─ routes             路由表：tier、body 上限、新鮮度
│  ├─ e2ee               p256 ECDH、HKDF-SHA256、AES-256-GCM
│  ├─ pairing            paired-devices.json ＋ Keychain dev.wristper.mac.device
│  ├─ recordings         錄音狀態機＋ delivered-ids.json（24 h／5000 筆）
│  └─ audio              symphonia：AAC .m4a → 16 kHz mono f32
├─ watch/macos           AX 欄位快照、48 px 比對、讀回、Enter、⌘Z
├─ licensing             Polar 啟用／驗證／停用、Plan、Quota（唯一讀取點）
├─ parakeet::DictationWorker   既有：序號、引擎、output worker、Paster
├─ rune_hud              既有：加外部來源的錄音／轉錄／結果狀態
└─ rune_ui               既有視窗：新增 Apple Watch 頁與方案列；引導加兩頁
```

一次手錶聽寫的資料流：

```ts
Watch 開始錄音
└─ POST /w/<room>/e2ee/query {path:/focus}         → relay → Mac
   └─ watch：解信封 → 新鮮度 60 s → 查額度 → AX 快照 → Recording::AwaitingAudio
Watch 錄完
└─ POST /w/<room>/e2ee（AudioHead ＋ m4a）         → relay（最多等 60 s）→ Mac
   └─ watch：解信封 → 去重 → 擁有者檢查 → 查額度 → 解碼 m4a
      └─ DictationWorker::submit_watch(samples, WatchDelivery)
         └─ engine 轉錄 → processor（模式、修正）→ output worker（保序）
            └─ finish_output：Target::Watch
               ├─ 可疑文字 → Suspect，不貼上
               ├─ 比對快照（pid、role、identifier／title、48 px）
               ├─ 寫 Committing 日誌（fsync）→ Paster 貼上
               └─ DeliveryOutcome 經 WatchDelivery 回 watch 執行緒
                  ├─ 回 res 框（密封的 {state,verified,…}）→ relay → Watch
                  └─ 背景讀回 ≤ 1 s → 升級成 pasted＋read-back
Watch 1.5 s 後 POST e2ee/query {path:/status} → 拿到最終狀態
```

## C. Rust 核心型別草圖

原則：線路字串只在 `watch/wire.rs` 轉換；內部用列舉，非法組合寫不出來。

```rust
// watch/ids.rs：邊界解析後才存在的品牌型別
pub struct RecordingId(uuid::Uuid);        // 解析自 36 字元 UUID
pub struct DeviceId(String);               // 手錶裝置 ID
pub struct Room(String);                   // ^[A-Za-z0-9_-]{22,64}$
pub struct RelaySecret(Zeroizing<String>); // ≥ 32 字元
pub struct Token([u8; 32]);                // base64url，固定時間比對
pub struct E2eeKey(Zeroizing<[u8; 32]>);
pub struct Nonce([u8; 12]);
pub struct ProtocolVersion(u8);            // 本機 4，最低對方 1
```

```rust
// watch/routes.rs：雲端可達的路由只有 Open 與 Device
pub enum CloudTier { Open, Device }
pub enum Freshness { Skew300s, Action60s, Background24h }
pub struct Route {
    pub method: Method,
    pub path: &'static str,
    pub tier: CloudTier,
    pub body_limit: usize,
    pub inside_e2ee_only: bool,
}
pub const CLOUD_ROUTES: &[Route] = &[/* 見第 D 節白名單 */];
```

Wristper 的 `master` tier 在 Hex 不上線路。配對認領、解除配對、授權操作是程序內的函式呼叫，CLI 走既有 `/control`（bearer token）。所以型別上 relay 請求不可能到達 master 動作。

```rust
// watch/envelope.rs
pub struct AudioHead {
    pub id: RecordingId, pub token: Token, pub language: Language,
    pub sent_at: f64, pub focus: FocusRequest,
    pub agent: Option<AgentRouteRequest>,  // 第一階段：Some → 400 invalid-route
    pub requires_e2ee: bool, pub background: bool,
}
pub enum FocusRequest { Requested, Current }
pub struct QueryHead { pub token: Token, pub sent_at: f64, pub path: InnerPath, pub query: Map }
pub enum Sealed { Audio { head: AudioHead, m4a: Vec<u8> }, Query(QueryHead) }
pub fn open(device: &DeviceId, nonce: Nonce, body: &[u8], key: &E2eeKey) -> Result<Sealed, E2eeError>;
pub fn seal_response(device: &DeviceId, req: Nonce, status: u16, json: &[u8], key: &E2eeKey) -> Vec<u8>;
```

```rust
// watch/pairing.rs
pub struct PairedDevice {
    pub id: DeviceId, pub name: String,
    pub paired_at: Timestamp, pub last_seen: Option<Timestamp>,
    pub secrets: DeviceSecrets,           // 第一階段必有金鑰
    pub push: Option<PushToken>,          // 第二階段才用
}
pub struct DeviceSecrets { token: Token, key: E2eeKey }
pub enum LoadedDevice { Ready(PairedDevice), NeedsRepair { id: DeviceId, name: String, why: RepairReason } }
pub enum RepairReason { NoE2eeKey, KeychainMissing }
pub enum PairingWindow { Closed, Approved { device: DeviceId, until: Instant } } // 單次、120 s
```

```rust
// watch/recordings.rs：一筆手錶錄音的生命週期
pub enum Recording {
    AwaitingAudio { owner: DeviceId, target: FieldTarget, since: Instant },
    Transcribing  { owner: DeviceId, target: FieldTarget, job: DictationJobId },
    Committing    { owner: DeviceId, text: Transcript },          // 已寫日誌，⌘V 送出中
    Done          { owner: DeviceId, text: Transcript, outcome: DeliveryOutcome,
                    completed_at: Timestamp, follow_up: FollowUp },
    Failed        { owner: DeviceId, error: FailureReason },
}
pub enum FieldTarget { Snapshot(FieldSnapshot), MissingFocus, Current }
pub enum FollowUp { None, Entered, Undone }   // Enter 與 Undo 互斥
```

```rust
// watch/delivery.rs：送達結果，對應線路 state
pub enum DeliveryOutcome {
    Pasted(Verification),      // "pasted"，verification "read-back"|"terminal"
    Unverified,                // "unverified"：已送 ⌘V，讀回未確認
    Copied,                    // "copied"：無焦點也無快照
    NotPasted(Refusal),        // 不貼上，只存文字
    Suspect(SuspectReason),    // "suspect"
}
pub enum Verification { ReadBack, Terminal }
pub enum Refusal { TargetChanged(TargetDiff), NotEditable, Sensitive(SensitiveKind), DailyLimit }
pub enum FailureReason { Decode, Inference(String), PasteFailed(String), Cancelled }

pub struct WatchDelivery(Arc<DeliverySlot>);   // TranscriptionTarget::Watch 帶的 handle
struct DeliverySlot { id: RecordingId, target: FieldTarget, reply: Mutex<Option<oneshot::Sender<DeliveryOutcome>>> }
```

`TranscriptionTarget` 目前是 `#[derive(Clone, Copy)]`（`src/parakeet.rs:86`）。加上帶 `Arc` 的變體後改成只有 `Clone`；`recognition.rs` 的比對點（`:1340`、`:1762`）跟著改。`DictationClip` 沒有從樣本建構的公開入口（`src/dictation.rs:320`），單元 D 加 `DictationClip::external(samples)`，沒有 pre-roll、tail 與 capture marker。

```rust
// licensing/model.rs：唯一的方案判定
pub enum Plan {
    Licensed { license: License, health: LicenseHealth },
    Free { usage: DailyUsage, lapsed: Option<Lapse> },
}
pub enum LicenseHealth { Validated { at: Timestamp }, Grace { until: Timestamp } } // ≤ 14 天
pub enum Lapse { Revoked, Expired, GraceOver }
pub struct DailyUsage { day: LaDate, used: u8 }          // 上限 10，America/Los_Angeles
pub enum Admission { Unlimited, Free { max: Duration } } // Free 一律 20 s
pub struct QuotaExhausted { resets_at: Timestamp }
pub fn plan(stored: &LicensingFile, now: Timestamp) -> Plan;
pub fn admit(plan: &Plan, now: Timestamp) -> Result<Admission, QuotaExhausted>;
pub fn consume(file: &mut LicensingFile, now: Timestamp) -> Result<(), QuotaExhausted>; // 在 commit 點
```

`licensing::enabled()` 是唯一旗標：正式版永遠 `true`；只有 `dev_build::is_dev_build()` 可用環境變數 `WRISTPER_LICENSING=off` 關掉。UI、深連結、接收端都只問 `Plan`，不讀 defaults。

## D. 線路相容性清單

逐字保留以下值，集中在 `watch/wire.rs` 的常數：

1. **relay 連線。** `GET wss://<base>/m/<room>`，header `X-Wristper-Mac-Secret`。文字框 `ping`／`pong`。單一訊息上限 1 MiB。close 1000 `replaced` 代表被新連線取代，不重連搶回，只記 log 並在 UI 顯示。
2. **請求框。** `{"type":"req","id","method","path","query","headers","body_len"}` 後接二進位分段（每段 ≤ 256 KiB）。回應 `{"type":"res","id","status","body":<JSON 字串>}`。claim：送 `{"type":"claim","code","name"}`，收 `claim_ok{code,device_id,name}` 或 `claim_failed{code,error}`，等 10 s。
3. **組框錯誤。** 400 `body-overflow`、400 `incomplete-body`、413 `too-large`（一般 20 MiB，`/e2ee` 20 MiB＋4096）。
4. **雲端白名單。** GET `/pairing/result`、`/health`、`/status`；POST `/pairing/request`、`/focus`、`/enter`、`/undo`、`/e2ee`、`/e2ee/query`、`/ping`。`/audio` 第一階段回 403 `e2ee-required`（見第 J 節）。`/agent/*` 第一階段回 404，與 Wristper 白名單外的路徑相同。
5. **header。** `X-Wristper-Protocol`（4）、`X-Wristper-App-Version`、`X-Wristper-Token`、`X-Wristper-Device`、`X-Wristper-Sent-At`、`X-Wristper-Nonce`、`X-Wristper-ID`、`X-Wristper-Language`、`X-Wristper-Focus`（`requested`|`current`）、`X-Wristper-Route|Session|Project|Approval`。內部標頭 `x-wristper-inside-e2ee` 一律先刪再由解信封加上。
6. **HKDF。** salt `Wristper E2EE v1`，info `Wristper pairing key`，輸出 32 bytes。公鑰是 P-256 x963（65 bytes，`0x04‖X‖Y`）的 base64。
7. **AAD。** 配對結果 `Wristper pairing result v1\n<request_id>`（AES-GCM combined：nonce‖ct‖tag）；錄音 `Wristper E2EE request v1\n<deviceID>\n<nonce base64>`（body 為 ct‖tag，nonce 在 header）；查詢 `Wristper E2EE query v1\n…`；回應 `Wristper E2EE response v1\n<deviceID>\n<請求 nonce>\n<status>`。
8. **信封明文。** 4 bytes 大端序 head 長度（≤ 4096）＋ AudioHead JSON ＋ 音訊。回應 `{"ok":true,"e2ee":{"nonce","ciphertext"}}`，外層狀態碼等於內層，明文帶 `e2ee_actions:true`。
9. **新鮮度。** 一般 ±300 s；`/focus`、`/enter`、`/undo` 60 s（超過 409 `confirm-stale`）；`background:true` 且無 route 24 h。nonce 記最近 4096 個，重放 409 `replayed`。
10. **棘輪。** 密文帶 `requires_e2ee:true` 時記到裝置；之後明文動作回 403 `e2ee-required`。
11. **錯誤碼。** 402 `{"ok":false,"error":"license_required","reason":"daily_limit_reached","resets_at"}`；403 `forbidden`、`not-owner`、`pairing-closed`；400 `invalid-e2ee`、`invalid-id`、`invalid-metadata`、`invalid-route`、`invalid-session`、`invalid_code`；404 `unknown-id`；426 `{"error":"watch_outdated","min_protocol","mac_protocol","mac_version"}`；500 `paste-failed`、`e2ee-failed`、`{state:"failed",error}`；503 `secrets-unavailable`。配對本機錯誤 `code_not_found`、`too_many_attempts`、`relay_offline`、`timeout`。
12. **逾時。** relay 等 Mac 60 s；Watch 70 s；`/focus` 8 s；音檔先到等 `/focus` 2 s；同 ID 處理中等 45 s 後回 `{duplicate:true,state:"transcribing"}`；讀回 1 s；`/enter` 等處理 15 s、完成後 30 s 內、Watch 20 s；`/undo` 60 s（Watch 只顯示 10 s，兩值都保留）；配對核准 120 s、結果可重讀 300 s。
13. **JSON 形狀。** `/status`、`/health`（含 `licensing{state,daily_limit,remaining_today,resets_at,max_seconds}`）、`/focus` 回 `{ok,captured,role}` 照 `ReceiverService.swift:616-677,845-868` 的欄位名。

**golden vector 驗證方法。**

1. 在 Wristper repo 加一個 Swift 測試產生器，重用 `PairingKey.swift` 與 `EncryptedBridge.swift`。用固定私鑰、固定 nonce、固定 request_id，輸出 JSON 到 `tests/fixtures/wristper-protocol/`。
2. 涵蓋：配對結果封裝、錄音信封（含 head 長度前綴）、查詢信封、回應信封（200、402、403 各一）、x963 公鑰、HKDF 輸出。
3. Rust 測試雙向驗：解開 Swift 產的密文，並把 Rust 封的密文與 fixture 逐 byte 比對（固定 nonce 時密文是確定的）。
4. 反向：Swift 測試解開 Rust 產的 fixture，證明 Watch 能讀。
5. 每個 AAD 各做一個「換錯 AAD 必須失敗」的測試，證明四種密文不能互冒。
6. base64 變體（標準或 url-safe、有無 padding）由 fixture 決定，不靠猜。
7. 單元 D 合併前，用一支開發用 Watch 經 dev relay 錄一段真的信封存成 fixture。這一步需要使用者的裝置，未做之前標「未實機驗證」。

## E. 失敗、重試、崩潰、未確定貼上

1. **冪等鍵是錄音 ID。** 同 ID 處理中，等 45 s 後回 `transcribing`；已完成就回原結果加 `duplicate:true`。重送永遠不會貼第二次。
2. **先寫日誌再貼。** output worker 在送 ⌘V 前，把 `{id, state:"committing"}` 經 `durable` 寫進 `delivered-ids.json` 並 fsync。貼上後改成最終狀態。
3. **崩潰復原。** 啟動時把殘留的 `committing` 一律改成 `unverified`。理由：⌘V 可能已送出。Watch 查 `/status` 得到「已送出，Mac 無法確認」，不會自動重送。這是唯一正確的未確定語意：寧可少貼，不可重貼。
4. **崩潰在轉錄中。** 記錄只在記憶體，重啟後 `/status` 回 `unknown-id`。Watch 依 `automaticRetry` 重送，帶 `focus:"current"`，貼到使用者目前游標。
5. **relay 斷線。** relay 對等待中的請求回 503 `mac_offline`。Mac 重連後不補送舊回應（relay 會丟掉），Watch 用 `/status` 補查。
6. **回應逾時。** Mac 對 `/e2ee` 最晚在 50 s 回應；還沒結果就回 `{ok:true,state:"transcribing"}`，讓 relay 的 60 s 不先觸發 504。
7. **Keychain 讀不到。** 回 503 `secrets-unavailable`，不回 403，避免 Watch 誤判配對失效。
8. **轉錄或貼上失敗。** 回 500 `{state:"failed",error}`；Watch 視為已送達的終態。
9. **額度競態。** `/focus`、`/e2ee` 入口各查一次；commit 點用 `consume` 原子扣額。扣不到就是 `NotPasted(DailyLimit)`，紀錄 `failed`＋`reason:daily_limit_reached`，與 Wristper 第三觸發點相同。
10. **讀回不阻塞輸出。** ⌘V 後 output worker 立即處理下一筆；讀回 1 s 由 watch 執行緒做，成功才升級成 `pasted`＋`read-back`。
11. **剪貼簿還原。** 沿用 Hex `paste` 的世代規則（`src/paste.rs`），不搬 Wristper「讀回成功立刻還原、否則 4 s」。差異寫進 feature map。
12. **⌘V 不是確認。** `src/paste.rs:461` 已明說。沒有讀回或終端機判定時，結果一律是 `unverified`。
13. **Agent route 不降級。** AudioHead 帶 route 時第一階段回 400 `invalid-route`，不當一般聽寫貼上。

## F. 設定與引導

1. **Apple Watch 設定頁。** 側欄常駐項目（不在 Labs）。內容：relay 狀態（已連線、連線中、離線、被取代）、已配對手錶清單（名稱、最後上線、解除配對）、「需要重新配對」的舊裝置、「配對 Apple Watch」按鈕。
2. **配對輸入。** 6 位數字，滿 6 位自動送出。不收 4 位數（見第 J 節）。錯誤文案對應 `code_not_found`、`too_many_attempts`、`relay_offline`、`timeout`。配對入口三處：引導、設定頁、選單列「配對 Apple Watch…」。
3. **CLI。** `hex watch pair <code>`、`hex watch list`、`hex watch revoke <id>` 走 `/control`。每個新持久設定都要有 `hex settings` 鍵。
4. **引導新增兩頁。** 在 `src/desktop/onboarding.rs:133` 的 `SetupItem` 加 `Watch`。頁面一：QR code 指向 `https://wristper.dev/watch`，請使用者用 iPhone 掃描安裝手錶 App。頁面二：輸入手錶顯示的 6 位數。兩頁都有「之後再配對」。
5. **麥克風權限。** Hex 現在把 Microphone 列為必要（`SetupItem::Microphone`）。fork 改成：選「只用手錶」時跳過麥克風；第一次按 Mac 快捷鍵或在「試一次」選 Mac 麥克風時才要求。這偏離 Hex「麥克風、輔助使用、模型齊了才開始聽寫」的不變式，要寫進 fork 版 `AGENTS.md`。輔助使用與模型仍是必要（貼上與轉錄都需要）。
6. **試一次。** 擴充既有練習：可選「用手錶說一句」，等到一筆 `Done` 紀錄就算完成。
7. **登入項目。** 引導完成時登記 `SMAppService`。理由：手錶要能隨時送達，App 必須常駐。
8. **HUD。** `/focus` 送入 `DictationIndicatorEvent` 新的外部錄音事件，HUD 顯示「手錶錄音中」，最多 180 s；之後沿用轉錄中與結果狀態，`target-changed`、`not-editable`、`sensitive`、`suspect` 顯示對應提示。HUD 仍不搶焦點、點擊穿透。
9. **可疑文字。** 移植 Wristper `TextRules.swift:22-60,116-143` 的五類規則成純函式，只套用在手錶來源。模型不同（Hex 預設 Parakeet，Wristper 用 Whisper），幻覺字幕清單要用 Hex 模型重新取樣。

## G. 授權與額度

1. **儲存。** `<data>/licensing.json`，經 `durable` atomic 寫入，權限 0600。內容是授權（key、activation_id、email、日期、最後驗證時間）與 `usage{day,used}`。第一次啟動時讀舊 Wristper 檔（同名、同資料夾）並就地升級 schema。
2. **損毀。** 照 Hex `settings.json.unreadable-<sha256>` 的作法保留原檔。損毀時把今日額度視為用完，並觸發一次重新驗證。不照 Wristper 的「改名成 .bak 歸零」，那等於刪檔就重設額度。
3. **Polar 呼叫。** `https://api.polar.sh/v1/customer-portal/license-keys/{activate,validate,deactivate}`，body 帶 `key`、`organization_id`、`label`、`activation_id`、`meta.device`（Mac ID 的 SHA-256）。全部經 Hex `net`，不 shell 出 curl。sandbox 只在 dev build 可切。
4. **狀態規則。** 啟動 5 s 後檢查，之後每小時檢查是否滿 24 h 該驗證。網路錯誤不改狀態，靠 14 天寬限。404 或 `revoked`／`disabled` → `Lapse::Revoked` → 免費。403 detail 含 `limit` → 「這張授權已用在 3 台 Mac」並給客戶入口連結。停用這台：deactivate，404 視為已停用。
5. **額度。** 每天 10 段，`America/Los_Angeles` 午夜重設，跨夏令時間。需要時區資料庫：新增 `jiff`（含 tzdb）或沿用 lock 檔已有的 `chrono` 搭 `chrono-tz`，單元 E 選一個並寫測試覆蓋 DST 切換日。
6. **哪些算一段。** commit 點成功送出的都算：手錶的 `pasted`、`unverified`、`copied`；Mac 快捷鍵的 Paste、RawPaste、Print、RawPrint。`suspect`、`failed`、`NotPasted` 不算。`local_api` 的 Service 轉錄預設也算（見第 J 節）。
7. **計數點。** `finish_output` 內、`commit()` 成功之後呼叫 `licensing::consume`。Mac 快捷鍵與手錶共用這一點，自然達成「合計」。
8. **20 秒上限。** 手錶：解碼後只取前 20 s，`/status` 帶 `truncated:true`。Mac 快捷鍵：在 capture 層到 20 s 時自動結束，當作一次正常放開（有 release tail、會轉錄貼上）。這偏離 Hex「capture 沒有自動上限」。
9. **額度用完。** 手錶收到 402，Watch 把錄音留在未送出清單。Mac 快捷鍵按下時不開始錄音，播錯誤音，HUD 顯示「今日免費額度已用完」，並在選單列提供購買。這偏離 Hex「有意的聽寫一律完成」，寫進 fork 版 `AGENTS.md`。
10. **購買。** 「一次買斷」按鈕打開 relay `/checkout?products=<id>`，附 `reference_id`。付款後 relay 302 到 `wristper://activate?license_key=<key>`。Hex 在 `app/Info.plist` 註冊 `wristper` scheme，經 `src/rune_ui/deep_link.rs` 路由；key 長度 ≤ 256。手動貼授權碼走同一個 `activate`。購買按鈕是該頁唯一的 Primary。
11. **不動 relay。** `/checkout`、`/purchase/complete`、webhook 原樣保留；`/trial/*` 沒有呼叫者，不在本計畫內處理。

## H. 身分、更新、發行

1. **bundle id 與網域。** `app/Info.plist` 改 `dev.wristper.mac`；defaults 網域、`DisableUpdates`、policy 鍵跟著改。SDK helper 與麥克風驅動改 `dev.wristper.mac.service`、`dev.wristper.mac.microphone.driver`。
2. **輔助使用權限沿用。** macOS 的 TCC 授權綁 bundle id 加簽章的 designated requirement。必須用 Wristper 原本的 Developer ID 團隊簽，舊使用者才不用重新授權（未驗證，見第 J 節）。
3. **資料根目錄。** `src/app_paths.rs:38-40` 的 `hex` 改 `Wristper`（測試版 `Wristper-test`），`~/.config/hex` 改 `~/.config/wristper`。理由：舊配對檔就在 `Wristper/`；也避免和使用者另外安裝的 Hex 搶 `desktop/instance` 的資料夾鎖與 `settings.json`。
4. **URL scheme。** 只註冊 `wristper`。移除正式版的 `hex` scheme；測試版保留 `hex-test://story/…` 給 playground。
5. **更新。** 上游 `src/updater.rs:43` 的 `ORIGIN`、`src/updater/macos.rs:24` 的 `BUNDLE_ID`、`app/update-requirement.txt`、`src/updater/notes.rs` 的備援網址都指向 Anomaly。單元 A 改成「沒有更新來源」：更新列顯示「到網站下載新版」，連到 `https://wristper.dev/download`，永不自動下載。Wristper 自己的 release service 建好後再接回 Hex updater。
6. **舊 0.1.x 升級橋。** 在 `download.wristper.dev/appcast.xml` 發一筆新 App 的 zip，用 Wristper 的 EdDSA 私鑰簽。新 App 的 `Info.plist` 把上游 Kit 的 `SUPublicEDKey`（`:52-53`）換成 Wristper 的公鑰，`validate-app.sh` 跟著改；新 App 本身不含 Sparkle。
7. **回報入口。** `src/labs.rs:31` 的 `FEEDBACK_URL` 改 `mailto:support@wristper.dev`；`src/rune_ui/output.rs:40` 的上游文件連結移除或改指 Wristper 文件。
8. **CI 與發行。** fork 不跑 anoma.ly 的 release、site、installers、legacy-appcast workflow。Mac 版用 `scripts/build-app.sh` 搭 `HEX_TEAM_ID`、`HEX_NOTARY_PROFILE=wristper-notary` 本機發行，產出 DMG 上傳 R2 `wristper-downloads`。
9. **網站。** 沿用 `wristper_web/`，只把 Download 換成新 DMG。最低系統改成 macOS 15、Apple silicon（Hex README 的基準）。
10. **授權通知。** 保留 `LICENSE`（MIT，Christopher Langton）並打包進 bundle；合併 Hex 與 Wristper 的 `THIRD_PARTY_NOTICES.md`，模型授權逐項核對。
11. **fork 版 `AGENTS.md`。** 單元 A 改寫：分支與 Dax 規則、release freeze、Labs 例外、20 秒例外、麥克風例外、資料根目錄、`DIFFERENCES` 新條目。
12. **平台差異。** `src/rune_ui/platform.rs:808` 的 `DIFFERENCES` 新增 `watch_field_verify`、`watch_enter_undo` 為 `Difference::Pending("AX 欄位快照只有 macOS adapter")`；relay、E2EE、配對、授權本身是跨平台，不列。

## I. 第二階段：Agent

目標：在 Mac 上看 Claude Code、Codex 目前的 Session 狀態，從手錶控制。以下判定以 `wristper-agent.md` 為據。

**能支援（Rust 直接做）：**

1. 列出 Claude、Codex、Grok 對話與最近 5 則訊息。掃 `~/.claude/projects/*/*.jsonl`、`~/.codex/sessions/**/rollout-*.jsonl`，讀頭 512 KB、尾 1 MB，mtime 快取。用 fixture 測試格式。
2. 判斷對話是否開在 Mac 上。Claude 讀 `~/.claude/sessions/*.json` 加 pid 與啟動時間比對；Codex 查 rollout 檔的持有程序。用 `libproc` 取代 `ps`、`lsof` 子程序。查不到一律「不確定」並拒送。
3. 判斷 Claude 主控回合是否在跑（掃紀錄尾端），Codex 看 `task_started` 對 `task_complete`。
4. 背景接續已關閉的對話：Claude `claude -p --input-format stream-json --resume <id> --output-format stream-json --permission-prompt-tool stdio`；Codex `codex app-server` 的 `thread/resume`、`turn/start`。
5. 手錶核准與提問（背景路線）：Claude `can_use_tool`、`AskUserQuestion`；Codex `requestApproval`、`request_user_input`。只給 allow_once、deny 與選項。
6. 停止：Codex `turn/interrupt`；其他對程序群組送 SIGTERM。
7. 完成、失敗、等你確認、等你回答四種推播：Mac 經 relay WebSocket 送 `push` 框，relay 轉 APNs。
8. 開著的 Claude Session 的核准轉手錶：Claude `PermissionRequest` hook 呼叫 `hex agent hook claude`，經同 uid 的 Unix socket 送到 Hex，最多等 290 s，任何錯誤都不輸出、讓 Claude 照常在 Mac 上問。
9. 主動推「Mac 上的 Claude 在等核准」：狀態檔 `waiting` 加 hook 事件。
10. Cursor、Antigravity 對話清單與背景接續：ACP JSON-RPC over stdio。

**不能支援或不建議：**

1. 語音送進開著的終端機 TUI 而不關 TUI：Claude 與 Codex 都沒有官方注入介面；看畫面打字已在 Wristper 證實不穩並作廢。
2. 手錶按 Enter 送出 TUI 輸入框：等同模擬按鍵，同上。一般聽寫的 `/enter` 仍可當人工備援。
3. 防止同 uid 程式偽造核准：tmux send-keys、TIOCSTI 等路徑存在，只能提高門檻。
4. 乾淨地中斷開著的 Session：只能送訊號，會留下沒有結果的 tool_use。
5. Grok 手錶核准：Grok 只有 `--permission-mode auto`。

**需要廠商支援：**

1. Claude Code、Codex 對執行中互動 Session 注入使用者訊息的官方 API。
2. Grok 的外部核准通道（等同 `--permission-prompt-tool stdio`）。
3. Codex `PermissionRequest` hook 在目前版本是否穩定觸發：Wristper 沒有真機證據，要先實測。

**建議分版：**

1. 2a：背景路線，Claude 與 Codex（清單、內容、確認後接續、核准、提問、停止、推播）。Grok、Cursor、agy 放後面。
2. 2b：Claude 開著的 Session 核准轉手錶（hook）。Codex hook 實測通過才承諾。
3. 2c：OpenCode 開著的 Session 送話，經 Hex 既有 `desktop/opencode_service`，不必關 TUI。
4. 不排：Wristper 規劃中的「B 半自動版」（SIGTERM TUI、每句跑 `-p`、再開回 TUI），等有實測數據再談。

**會偏離 Hex 規則之處：**

1. Hex 規定「絕不啟動 OpenCode 執行檔」，啟動子程序只經 `desktop/background_process`。第二階段要啟動 `claude -p`、`codex app-server`、`grok -p`、`cursor-agent acp`、`agy acp`。新政策：只在使用者從手錶確認後啟動，固定路徑、引數陣列、不經 shell、程序群組可整組結束。
2. 讀使用者家目錄下其他工具的紀錄檔（`~/.claude`、`~/.codex`），只讀不寫。Wristper 曾在清理時誤刪 179 段 Codex 對話（`tasks/lessons.md:10`），測試一律用暫存 HOME。
3. 寫入使用者設定：Claude hook 需要 `--settings` 或改 `~/.claude/settings.json`；Codex 需改 `~/.codex/hooks.json`。預設不寫，由使用者在設定頁明確按「安裝 hook」，並可一鍵移除。
4. 預設開關：Agent 預設關閉，需要使用者在 Apple Watch 頁明確開啟。手錶聽寫不進 Labs，但 Agent 啟動外部程序，風險不同（見第 J 節）。
5. 新增 Unix socket 入口（hook），只收同 uid、有界框長與逾時。

## J. 風險、未驗證項目、待使用者決定

每項附預設值；不回覆就照預設做。

1. **TCC 是否沿用。** 風險：簽章團隊不同時，舊使用者要重新開輔助使用。預設：用 Wristper 原 Developer ID 團隊簽，單元 A 用乾淨帳號實測一次。
2. **無金鑰舊配對與 4 位數配對。**（已決定，使用者 2026-11-04 同意預設） 預設：不支援。無金鑰裝置顯示「需要重新配對」；明文 `/audio` 回 403 `e2ee-required`；4 位數碼不收。理由：`/pairing/request` 沒公鑰已一律 426，新配對早就全是 E2EE。可改為支援，代價是多一條明文路徑要維護。
3. **只用手錶時要不要求麥克風。**（已決定，使用者 2026-11-04 同意預設） 預設：不要求，第一次用 Mac 麥克風時才問。
4. **`local_api` 轉錄算不算額度。** 預設：算，否則免費版可繞過。
5. **Agent 預設開或關。** 預設：關，在 Apple Watch 頁開啟。
6. **更新來源。**（已決定，使用者 2026-11-04 同意預設） 預設：先停用自動更新，只給網站下載連結；之後自架 release service。
7. **預設模型與繁體輸出。** Wristper 用 Whisper Large v3 Turbo 並固定繁體（OpenCC）；Hex 依語言推薦模型。預設：照 Hex 語言推薦；繁體轉換不在第一階段（見第 K 節），單元 D 前用中文樣本實測辨識品質。
8. **nonce 快取只在記憶體。** 重開 Mac 後 5 分鐘內的前景信封可重放；靠錄音 ID 去重與 `entered` 欄位擋下。預設：照 Wristper 只放記憶體，單元 B 加這個情境的測試。
9. **回應大小。** Mac 端 1 MiB 上限；第二階段 `/agent/session` 可能超過，Cloudflare Durable Object 的訊息上限未查證。預設：回應超過 900 KiB 就截斷內容並標 `truncated`。
10. **相依套件版本。** Hex 用 `sha2 0.11`；`hkdf`、`p256`、`aes-gcm` 要選同一代 `digest`，避免兩份 sha2。`symphonia` 已在 lock 檔（間接相依），要加 `isomp4` 與 `aac` feature。`tungstenite` 用 `native-tls`，要套用 `net` 的代理設定。這些是預期，單元 B、C 編譯時確認。
11. **Hex `net` 規則。** 規則說新的對外請求都經 `net`；WebSocket 不在 ureq 能力內。預設：WebSocket 用 tungstenite，但 TLS 連線器與代理解析從 `net` 取，並在 fork 版 `AGENTS.md` 記一句例外。
12. **AX 呼叫的執行緒。** 快照在 watch 執行緒取，比對在 output worker 取。預設：每次 AX 呼叫設 250 ms messaging timeout，避免卡住保序輸出。未實測。
13. **relay 部署版本。** STATUS 說 #107 未部署，之後狀態沒查。單元 C 前先 `curl` dev relay 的行為核對。
14. **登入項目。** 預設：引導完成時自動登記。
15. **未驗證總表。** 沒有建置、沒有跑測試、沒有連 relay、沒有實機 Watch。所有逾時與行為來自原始碼閱讀。

## K. 不做的事

1. 不改 Watch、iPhone App，也不改 relay 協定。
2. 不實作 Windows、Linux 的 AX 欄位驗證、Enter、Undo；只在 `DIFFERENCES` 標 pending。
3. 不保留 `127.0.0.1:39271` 的設定 API；依賴它的 `wristper-sessions.py` 與 Wristper CLI 會失效。
4. 不移植 Sparkle 到新 App；appcast 只當舊版升級橋。
5. 不移植 Wristper 的 Foundation Models 潤稿（PolishGate）、OpenCC 固定繁體、終端機 shim；需要時改用 Hex 的 transformations。
6. 不處理 relay 的 `/trial/*` 舊路由。
7. 第一階段不做任何 Agent 路由；帶 route 的錄音回錯誤，不降級成貼上。
8. 不新增第二個模型 owner、第二個 Paster，也不經 `local_api /transcriptions` 轉錄手錶音檔。
9. 不發任何 Hex 上游 release，不推上游 `production`。
