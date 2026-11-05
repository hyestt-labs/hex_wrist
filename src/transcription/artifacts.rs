//! The catalog's pinned model files and the language lists its entries share.

use super::models::{GgufArtifact, TranscriptionModelId};

pub(super) const PARAKEET_V3_LANGUAGES: &[&str] = &[
    "bg", "hr", "cs", "da", "nl", "en", "et", "fi", "fr", "de", "el", "hu", "it", "lv", "lt", "mt",
    "pl", "pt", "ro", "ru", "sk", "sl", "es", "sv", "uk",
];
pub(super) const QWEN_LANGUAGES: &[&str] = &[
    "zh", "en", "yue", "ar", "de", "fr", "es", "pt", "id", "it", "ko", "ru", "th", "vi", "ja",
    "tr", "hi", "ms", "nl", "sv", "da", "fi", "pl", "cs", "fil", "fa", "el", "ro", "hu", "mk",
];
pub(super) const COHERE_LANGUAGES: &[&str] = &[
    "en", "fr", "de", "es", "it", "pt", "nl", "pl", "el", "ar", "ja", "zh", "vi", "ko",
];

pub(super) const PARAKEET_V2_ARTIFACT: GgufArtifact = GgufArtifact {
    filename: "parakeet-tdt-0.6b-v2-Q8_0.gguf",
    revision: "07cee0616125a08ef619729bb47f40ef747e4bc4",
    repository: "handy-computer/parakeet-tdt-0.6b-v2-gguf",
    bytes: 729_574_912,
    sha256: "f0d0e99cebb6d3b83f1f7069b82b5d3c2e39a54545b0da039cb4bafd9c4e5caa",
    architecture: "parakeet",
    variant: "tdt-0.6b-v2",
};
pub(super) const PARAKEET_UNIFIED_ENGLISH_ARTIFACT: GgufArtifact = GgufArtifact {
    filename: "parakeet-unified-en-0.6b-Q8_0.gguf",
    revision: "7e948f21b7bdbac698d3318db9d350f1096f3b6c",
    repository: "handy-computer/parakeet-unified-en-0.6b-gguf",
    bytes: 731_357_568,
    sha256: "4b50b6dd862bf6e346929aaf4f5eaacec003bfa3f56462d6c874b41ef2f38795",
    architecture: "parakeet",
    variant: "unified-en-0.6b",
};
pub(super) const PARAKEET_V3_ARTIFACT: GgufArtifact = GgufArtifact {
    filename: "parakeet-tdt-0.6b-v3-Q8_0.gguf",
    revision: "85ac09ea12fc4b1112fa76810059364bc6adc9de",
    repository: "handy-computer/parakeet-tdt-0.6b-v3-gguf",
    bytes: 739_508_576,
    sha256: "5859f77944efcd8eafa23a6350731960b2b55b2203df51f319665c807d802cc7",
    architecture: "parakeet",
    variant: "tdt-0.6b-v3",
};
pub(super) const WHISPER_ARTIFACT: GgufArtifact = GgufArtifact {
    filename: "whisper-large-v3-turbo-Q8_0.gguf",
    revision: "d222c9f621c1128299248f2ded4d8a1820519780",
    repository: "handy-computer/whisper-large-v3-turbo-gguf",
    bytes: 886_381_824,
    sha256: "d5e65f2b0828802ae2c231673d31982cebe3a778c95d9494a9f3efee6bd17448",
    architecture: "whisper",
    variant: "whisper-large-v3-turbo",
};
pub(super) const QWEN_ARTIFACT: GgufArtifact = GgufArtifact {
    filename: "Qwen3-ASR-0.6B-Q8_0.gguf",
    revision: "e4e16599b900eb0cb36e524514756bb92eb092b7",
    repository: "handy-computer/Qwen3-ASR-0.6B-gguf",
    bytes: 850_423_456,
    sha256: "f081b2d5e23bd669d92cc331d722a8a0681943b8e6f34b48996fd5c319b5acd8",
    architecture: "qwen3_asr",
    variant: "qwen3-asr-0.6b",
};
// Mohammed Aly's Qwen3-ASR 1.7B fine-tune for Egyptian Arabic and
// Arabic/English code-switching (Apache-2.0), from mohammedaly22/QwenCleo-ASR
// at 50f6b901. Converted with transcribe.cpp 7045d1c's Qwen3-ASR converter,
// with the fine-tune's attribution in `general.*`, and quantized to Q8_0;
// rebuilding it that way reproduces this file byte for byte.
pub(super) const QWENCLEO_ARTIFACT: GgufArtifact = GgufArtifact {
    filename: "QwenCleo-ASR-1.7B-Q8_0.gguf",
    revision: "08c66976a0496e1e3f1a88965eea123631e634fa",
    repository: "reomar15/QwenCleo-ASR-transcribe-cpp-GGUF",
    bytes: 2_185_030_816,
    sha256: "43bae9d5d7a26684b4a0faf1abc6e872b5825cc9147dbb7200da9ff4c5d12f87",
    architecture: "qwen3_asr",
    variant: "qwen3-asr-1.7b",
};
// ivrit.ai's Hebrew fine-tune of Whisper large-v3-turbo (Apache-2.0), from
// ivrit-ai/whisper-large-v3-turbo at f33172a8. Converted with transcribe.cpp
// 7045d1c's Whisper converter, with the variant's display name added
// (`ivrit.ai Whisper Large v3 Turbo`; author and repository come from the
// source), and quantized to Q8_0; rebuilding it that way reproduces this
// file byte for byte.
pub(super) const IVRIT_WHISPER_ARTIFACT: GgufArtifact = GgufArtifact {
    filename: "ivrit-whisper-large-v3-turbo-Q8_0.gguf",
    revision: "a12befaff80add995ac6a8fe10cbb7f38438c7b5",
    repository: "adolan/ivrit-whisper-large-v3-turbo-gguf",
    bytes: 886_381_536,
    sha256: "23756afa3d3c52cd3cb55e259c8373373eba6d19c3858a67531ba2772b7f272f",
    architecture: "whisper",
    variant: "ivrit-whisper-large-v3-turbo",
};
pub(super) const SENSEVOICE_ARTIFACT: GgufArtifact = GgufArtifact {
    filename: "SenseVoiceSmall-Q8_0.gguf",
    revision: "4a08b8e900b38a977e32eb08d5d0697d6e72ba04",
    repository: "handy-computer/SenseVoiceSmall-gguf",
    bytes: 252_684_608,
    sha256: "6c759ee4c9748c9b3f7a5a60ca74f0f7e685fb9d45d1378fce7cfd62f59adf29",
    architecture: "sensevoice",
    variant: "sensevoice-small",
};
pub(super) const COHERE_ARTIFACT: GgufArtifact = GgufArtifact {
    filename: "cohere-transcribe-03-2026-Q8_0.gguf",
    revision: "dfa4adebb64f3076b7b6b90b721275cc069cb421",
    repository: "handy-computer/cohere-transcribe-03-2026-gguf",
    bytes: 2_410_655_232,
    sha256: "931916663432fd895423a4291a8400221802b288967ca2d435fc5e3141c9e71e",
    architecture: "cohere_asr",
    variant: "cohere-transcribe-03-2026",
};
// Conway Research's Parakeet TDT v3 fine-tunes (CC-BY-4.0), converted to
// Q8_0 with transcribe.cpp's Parakeet converter; each repository holds the
// recipe.
pub(super) const UNDERDOG_BARK_3_PRO_ARTIFACT: GgufArtifact = GgufArtifact {
    filename: "underdog-bark-3.0-pro-Q8_0.gguf",
    revision: "5c10586263806666c0a8b4905a3346a7c679803e",
    repository: "joshthomasdev/underdog-bark-3.0-pro-gguf",
    bytes: 739_508_480,
    sha256: "472d151cc1546bd30629c4c25df3a8e555e8990845843ddafe76800c640e9865",
    architecture: "parakeet",
    variant: "tdt-0.6b-bark-3.0-pro",
};
pub(super) const UNDERDOG_BARK_3_CLEAN_ARTIFACT: GgufArtifact = GgufArtifact {
    filename: "underdog-bark-3.0-clean-Q8_0.gguf",
    revision: "868701fd53d62c59d759ed1d79e095118f3fa7b9",
    repository: "joshthomasdev/underdog-bark-3.0-clean-gguf",
    bytes: 739_508_480,
    sha256: "42ec2f70a3d14842e322391baf5186b2829f271cc7e396aed25580135ae46fae",
    architecture: "parakeet",
    variant: "tdt-0.6b-bark-3.0-clean",
};
// Moondream's Parakeet Ultra, a post-trained Parakeet TDT 0.6B v3
// (CC-BY-4.0), converted to GGUF and quantized to Q8_0 by handy-computer
// (430 F32, 225 Q8_0, and 48 F16 tensors). Behind Labs › Parakeet Ultra.
pub(super) const PARAKEET_ULTRA_ARTIFACT: GgufArtifact = GgufArtifact {
    filename: "parakeet-ultra-Q8_0.gguf",
    revision: "a1539be1b849ea7463b88343e5277b373d0c4ea9",
    repository: "handy-computer/parakeet-ultra-gguf",
    bytes: 740_363_168,
    sha256: "007a59761e9258779f189df396b50d12b83bb5b79e66df4b955c230d2f2a0a59",
    architecture: "parakeet",
    variant: "tdt-0.6b-ultra",
};
// NVIDIA's Nemotron 3.5 ASR Streaming 0.6B (OpenMDW-1.1), a cache-aware
// FastConformer with a language-prompted RNN-T. Hex converted
// nvidia/nemotron-3.5-asr-streaming-0.6b at 24b151a (the .nemo,
// 210214ed…) with transcribe.cpp 7045d1c's Parakeet converter and quantized
// it to Q8_0: all 653 tensors match this file byte for byte, and the
// metadata differs only by two capability flags the newer converter writes
// as false (translate, diarization). The pinned file is handy-computer's,
// which ships the licence and notice beside it.
pub(super) const NEMOTRON_3_5_ASR_ARTIFACT: GgufArtifact = GgufArtifact {
    filename: "nemotron-3.5-asr-streaming-0.6b-Q8_0.gguf",
    revision: "53ce288f69326b415aeef030cfcb02a08a8096dd",
    repository: "handy-computer/nemotron-3.5-asr-streaming-0.6b-gguf",
    bytes: 751_094_240,
    sha256: "b94545b313b3223fda7b2857a52681da813935c2127643d1e9ff0c23d988089c",
    architecture: "parakeet",
    variant: "nemotron-3.5-asr-streaming-0.6b",
};
// ai-sage's GigaAM v3 e2e RNN-T (MIT), Russian only, cased and punctuated.
// Hex converted the `v3_e2e_rnnt` checkpoint that salute-developers/GigaAM
// at 6e4b027c downloads and checks (MD5 2730de75…) with transcribe.cpp
// 7045d1c's GigaAM converter and quantized it to Q8_0: all 560 tensors
// match this file byte for byte, and the metadata differs only by four
// capability flags the newer converter writes as false. The pinned file is
// handy-computer's (from ai-sage/GigaAM-v3 at ec1dc1f), which ships the
// licence and notice beside it.
pub(super) const GIGAAM_V3_E2E_RNNT_ARTIFACT: GgufArtifact = GgufArtifact {
    filename: "gigaam-v3-e2e-rnnt-Q8_0.gguf",
    revision: "3efd5b39882ceadca5570a8fb9fa1a2c801d6d6d",
    repository: "handy-computer/gigaam-v3-e2e-rnnt-gguf",
    bytes: 273_724_832,
    sha256: "78d63b47723b7f8d78c6113a6ef983b5a86e2a86f6c273e1f5cb6967b1c4467a",
    architecture: "gigaam",
    variant: "gigaam-v3-e2e-rnnt",
};
// The National Library of Norway's NB-Whisper Medium (Apache-2.0), a
// Whisper medium fine-tune for Norwegian: NbAiLab's own whisper.cpp Q5_0
// file at nb-whisper-medium 0ed074d5, which transcribe.cpp loads as Whisper.
// Hex also converted the same revision's weights with transcribe.cpp
// 7045d1c's Whisper converter at Q8_0 (832 MB): it scored 6.75% on FLEURS
// Norwegian against this file's 6.68%, so the authors' smaller file stays.
pub(super) const NB_WHISPER_MEDIUM_ARTIFACT: GgufArtifact = GgufArtifact {
    filename: "ggml-model-q5_0.bin",
    revision: "0ed074d5985bd56ca4140159a9dbffbc3fb5117e",
    repository: "NbAiLab/nb-whisper-medium",
    bytes: 539_212_484,
    sha256: "18733de634af639a43b0f8c5f5a2ea0920de4c5b32a5570ec130981581c0e5e7",
    architecture: "whisper",
    // transcribe.cpp names a legacy Whisper file by its geometry.
    variant: "whisper-medium",
};
// IBM's Granite Speech 5.0 470M TurboCTC (Apache-2.0), the multilingual,
// cased and punctuated release at ibm-granite 1dd23f46 (handy-computer's
// file is the earlier English-only, lowercase checkpoint). Hex converted it
// with transcribe.cpp 7045d1c's Granite 5 converter, patched only to accept
// the release's missing legacy `preprocessor_config.json` and to list its
// seven languages, and quantized it to Q8_0: the result matches this file
// byte for byte. The pinned file is 17slever17's, which ships the licence
// and notice beside it.
pub(super) const GRANITE_SPEECH_5_ARTIFACT: GgufArtifact = GgufArtifact {
    filename: "granite-speech-5.0-470m-turboctc-multilingual-Q8_0.gguf",
    revision: "62f64d9d5ad35989b06caf199cafe7155944a22b",
    repository: "17slever17/granite-speech-5.0-470m-turboctc-multilingual-GGUF",
    bytes: 505_663_136,
    sha256: "f3efdc05b85b3a764bb5bb7022e69f61cafcecc11605c881c6396a6ed07ec91b",
    architecture: "granite_speech5_ctc",
    variant: "granite-speech-5.0-470m-turboctc",
};
pub(super) const GRANITE_SPEECH_5_LANGUAGES: &[&str] = &["en", "fr", "de", "es", "pt", "it", "ja"];
/// The languages Nemotron 3.5 transcribes that Hex names, as the locales
/// its language prompt takes (`runtime_language`). Its card also lists
/// Norwegian Bokmål, which Hex doesn't offer it for yet.
pub(super) const NEMOTRON_3_5_LOCALES: &[(&str, &str)] = &[
    ("en", "en-US"),
    ("es", "es-ES"),
    ("fr", "fr-FR"),
    ("de", "de-DE"),
    ("it", "it-IT"),
    ("pt", "pt-BR"),
    ("nl", "nl-NL"),
    ("tr", "tr-TR"),
    ("ru", "ru-RU"),
    ("uk", "uk-UA"),
    ("pl", "pl-PL"),
    ("cs", "cs-CZ"),
    ("sk", "sk-SK"),
    ("bg", "bg-BG"),
    ("hr", "hr-HR"),
    ("ro", "ro-RO"),
    ("hu", "hu-HU"),
    ("sv", "sv-SE"),
    ("da", "da-DK"),
    ("fi", "fi-FI"),
    ("et", "et-EE"),
    ("ar", "ar-AR"),
    ("hi", "hi-IN"),
    ("ja", "ja-JP"),
    ("ko", "ko-KR"),
    ("zh", "zh-CN"),
    ("vi", "vi-VN"),
];
pub(super) const NEMOTRON_3_5_LANGUAGES: &[&str] = &[
    "en", "es", "fr", "de", "it", "pt", "nl", "tr", "ru", "uk", "pl", "cs", "sk", "bg", "hr", "ro",
    "hu", "sv", "da", "fi", "et", "ar", "hi", "ja", "ko", "zh", "vi",
];
// handy-computer's Q8_0 conversions of OpenAI's Whisper large-v3, medium, and
// small (Apache-2.0), and of MediaTek Research's Breeze ASR 25, a Whisper
// large-v2 fine-tune (Apache-2.0), all converted with transcribe.cpp's
// Whisper converter. Each pin is a 40-digit commit; its size and SHA-256
// match the Hub's LFS record, and the header names the architecture and
// variant below (748 F32 and 513 Q8_0 tensors for large-v3 and Breeze, 564
// and 385 for medium, 288 and 193 for small).
pub(super) const WHISPER_LARGE_V3_ARTIFACT: GgufArtifact = GgufArtifact {
    filename: "whisper-large-v3-Q8_0.gguf",
    revision: "e3e29bee6389c7da4a141406f07bb80ddac5337c",
    repository: "handy-computer/whisper-large-v3-gguf",
    bytes: 1_668_741_440,
    sha256: "2fa1a5f179f8a511a53e2108db270aa4af3ce08cd976af4180e2854666bb4ba3",
    architecture: "whisper",
    variant: "whisper-large-v3",
};
pub(super) const WHISPER_MEDIUM_ARTIFACT: GgufArtifact = GgufArtifact {
    filename: "whisper-medium-Q8_0.gguf",
    revision: "ec78f06fded51aa82cde751678b78f76f78c8b7f",
    repository: "handy-computer/whisper-medium-gguf",
    bytes: 831_538_144,
    sha256: "09e6a65e7de377aa5b10bae24608bc6f8ca2ed04b3993ef10d4a02bcd9a82adf",
    architecture: "whisper",
    // NB-Whisper Medium's legacy file carries the same variant name; a
    // header's variant only has to match the model's own artifact.
    variant: "whisper-medium",
};
pub(super) const WHISPER_SMALL_ARTIFACT: GgufArtifact = GgufArtifact {
    filename: "whisper-small-Q8_0.gguf",
    revision: "c0214bd34be9296695486f838e0142f900803159",
    repository: "handy-computer/whisper-small-gguf",
    bytes: 269_751_136,
    sha256: "9b9c8811bbcc82a7766f0fb0925614bdacb0923b2cc630daeac17108b655b860",
    architecture: "whisper",
    variant: "whisper-small",
};
pub(super) const BREEZE_ASR_25_ARTIFACT: GgufArtifact = GgufArtifact {
    filename: "Breeze-ASR-25-Q8_0.gguf",
    revision: "1e5e8d7295110e1e14e305e9cf7411d82711beaa",
    repository: "handy-computer/Breeze-ASR-25-gguf",
    bytes: 1_667_964_224,
    sha256: "1650c163cd1623d13b585368ffd87d7e669c5ff23014511e0fa91bb4ce994756",
    architecture: "whisper",
    variant: "breeze-asr-25",
};
// handy-computer's Q8_0 conversion of Alibaba's Qwen3-ASR 1.7B (Apache-2.0),
// 365 F32 and 344 Q8_0 tensors. QwenCleo, a fine-tune of it, carries the same
// variant name.
pub(super) const QWEN_1_7B_ARTIFACT: GgufArtifact = GgufArtifact {
    filename: "Qwen3-ASR-1.7B-Q8_0.gguf",
    revision: "92282af1610a2db19d66f2bef1e260f5deca782d",
    repository: "handy-computer/Qwen3-ASR-1.7B-gguf",
    bytes: 2_185_030_624,
    sha256: "9a0d81792dfea2d5f278b8a63deb3ea6e02139ce42c2301f32ea19c4f77526b7",
    architecture: "qwen3_asr",
    variant: "qwen3-asr-1.7b",
};

/// Who made a catalog model, under which licence, and where its pinned file
/// came from, read from the pinned file's own header and its transcribe.cpp
/// catalog entry (2778ae4).
#[cfg_attr(not(test), allow(dead_code))] // read by the picker's details in a UI unit
pub struct ModelSource {
    pub author: &'static str,
    pub license_spdx: &'static str,
    pub license_url: &'static str,
    pub upstream_repo: &'static str,
    /// The upstream commit the catalog records, as the catalog abbreviates it.
    pub upstream_revision: &'static str,
    pub converted_by: &'static str,
}

const APACHE_2_0: &str = "https://www.apache.org/licenses/LICENSE-2.0";
const HANDY_COMPUTER: &str = "handy-computer, with transcribe.cpp";

const WHISPER_LARGE_V3_SOURCE: ModelSource = ModelSource {
    author: "OpenAI",
    license_spdx: "Apache-2.0",
    license_url: APACHE_2_0,
    upstream_repo: "openai/whisper-large-v3",
    upstream_revision: "06f233f",
    converted_by: HANDY_COMPUTER,
};
const WHISPER_MEDIUM_SOURCE: ModelSource = ModelSource {
    author: "OpenAI",
    license_spdx: "Apache-2.0",
    license_url: APACHE_2_0,
    upstream_repo: "openai/whisper-medium",
    upstream_revision: "abdf7c3",
    converted_by: HANDY_COMPUTER,
};
const WHISPER_SMALL_SOURCE: ModelSource = ModelSource {
    author: "OpenAI",
    license_spdx: "Apache-2.0",
    license_url: APACHE_2_0,
    upstream_repo: "openai/whisper-small",
    upstream_revision: "973afd2",
    converted_by: HANDY_COMPUTER,
};
const BREEZE_ASR_25_SOURCE: ModelSource = ModelSource {
    author: "MediaTek Research",
    license_spdx: "Apache-2.0",
    license_url: APACHE_2_0,
    upstream_repo: "MediaTek-Research/Breeze-ASR-25",
    upstream_revision: "cffe7cc",
    converted_by: HANDY_COMPUTER,
};
const QWEN_1_7B_SOURCE: ModelSource = ModelSource {
    author: "Alibaba Qwen Team",
    license_spdx: "Apache-2.0",
    license_url: APACHE_2_0,
    upstream_repo: "Qwen/Qwen3-ASR-1.7B",
    upstream_revision: "7278e1e",
    converted_by: HANDY_COMPUTER,
};

/// The provenance Unit F recorded for the models it added. The older
/// models carry theirs in comments beside their artifacts, so they answer
/// `None` until someone moves it here; every id is named, so a new model
/// can't skip the question.
#[cfg_attr(not(test), allow(dead_code))]
pub fn source(id: TranscriptionModelId) -> Option<&'static ModelSource> {
    use TranscriptionModelId as M;
    match id {
        M::WhisperLargeV3 => Some(&WHISPER_LARGE_V3_SOURCE),
        M::WhisperMedium => Some(&WHISPER_MEDIUM_SOURCE),
        M::WhisperSmall => Some(&WHISPER_SMALL_SOURCE),
        M::BreezeAsr25 => Some(&BREEZE_ASR_25_SOURCE),
        M::Qwen3Asr17B => Some(&QWEN_1_7B_SOURCE),
        M::ParakeetUnifiedEnglish
        | M::ParakeetV2
        | M::ParakeetV3
        | M::WhisperLargeV3Turbo
        | M::Qwen3Asr06B
        | M::QwenCleoAsr17B
        | M::IvritWhisperLargeV3Turbo
        | M::SenseVoiceSmall
        | M::CohereTranscribe
        | M::UnderdogBark3Pro
        | M::UnderdogBark3Clean
        | M::ParakeetUltra
        | M::Nemotron35Asr
        | M::GigaAmV3
        | M::NbWhisperMedium
        | M::GraniteSpeech5
        | M::AppleSpeech
        | M::Custom(_) => None,
    }
}
