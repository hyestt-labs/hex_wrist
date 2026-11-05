//! Where each catalog model's error rate comes from, as Labs › Benchmark
//! details shows it, and whether two figures may be compared.

mod catalog;

use super::models::{ModelDefinition, TranscriptionModelId, choices_for_runtime};

/// The error measure a benchmark reports; lower is better for both.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ErrorMetric {
    /// Word error rate.
    Wer,
    /// Character error rate, for languages written without spaces.
    Cer,
}

impl ErrorMetric {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Wer => "WER",
            Self::Cer => "CER",
        }
    }
}

/// Where one model's error rate comes from, as Labs › Benchmark details
/// shows it. Every `None` is something the source did not record and the
/// picker says is unknown; nothing is guessed. Speed figures (`realtime`)
/// have no record: they are upstream figures whose hardware and method are
/// unknown.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BenchmarkRecord {
    pub metric: ErrorMetric,
    /// In percent.
    pub value: f64,
    /// The language code the benchmark measured.
    pub language: &'static str,
    /// The dataset and its split, such as "FLEURS test".
    pub dataset: Option<&'static str>,
    /// Plain text naming where the figure comes from.
    pub source: Option<&'static str>,
    /// The source record's revision; not an inference run.
    pub source_revision: Option<&'static str>,
    pub utterances: Option<u32>,
    pub batch_size: Option<u32>,
    /// Whether the run asked for timestamps.
    pub timestamps: Option<bool>,
    pub hardware: Option<&'static str>,
    /// The weights' quantization, such as "Q8_0".
    #[cfg_attr(not(test), allow(dead_code))] // the picker reads it in a UI unit
    pub quantization: Option<&'static str>,
    /// The 95% confidence interval, in percent, when the source gives one.
    #[cfg_attr(not(test), allow(dead_code))]
    pub ci95: Option<(f64, f64)>,
}

impl BenchmarkRecord {
    /// A record whose source, dataset, and conditions were not recorded.
    const fn unrecorded(metric: ErrorMetric, value: f64, language: &'static str) -> Self {
        Self {
            metric,
            value,
            language,
            dataset: None,
            source: None,
            source_revision: None,
            utterances: None,
            batch_size: None,
            timestamps: None,
            hardware: None,
            quantization: None,
            ci95: None,
        }
    }

    /// The raw figure: `7.57% CER`.
    pub fn rate_label(&self) -> String {
        format!("{:.2}% {}", self.value, self.metric.label())
    }

    /// Whether the two figures measure the same thing on the same data, so
    /// one may be called better than the other. An unknown dataset matches
    /// nothing.
    pub fn comparable(&self, other: &Self) -> bool {
        self.metric == other.metric
            && self.language == other.language
            && self.dataset.is_some()
            && self.dataset == other.dataset
    }
}

/// The transcription runtime's structured records for its Mandarin FLEURS
/// runs, both legacy-published with no engine revision: the original run
/// IDs and scoring or artifact revisions are unknown. The catalog's
/// `word_error_rate` keeps the narrative figures (7.64 and 10.11) as
/// alternate legacy values; cards with the Lab on show these instead.
const MANDARIN_FLEURS_SOURCE: &str = "transcribe.cpp benchmark record, legacy-published";
const MANDARIN_FLEURS_REVISION: &str = "7045d1c";

const QWEN3_ASR_RECORD: BenchmarkRecord = BenchmarkRecord {
    metric: ErrorMetric::Cer,
    value: 7.57,
    language: "zh",
    dataset: Some("FLEURS test"),
    source: Some(MANDARIN_FLEURS_SOURCE),
    source_revision: Some(MANDARIN_FLEURS_REVISION),
    utterances: Some(945),
    batch_size: Some(8),
    timestamps: Some(false),
    hardware: None,
    quantization: None,
    ci95: None,
};
const SENSEVOICE_RECORD: BenchmarkRecord = BenchmarkRecord {
    value: 10.12,
    ..QWEN3_ASR_RECORD
};
const QWENCLEO_RECORD: BenchmarkRecord = BenchmarkRecord {
    dataset: Some("Author's Egyptian Arabic test"),
    source: Some("The model author's own evaluation"),
    utterances: Some(3_699),
    ..BenchmarkRecord::unrecorded(ErrorMetric::Wer, 19.85, "ar")
};

/// Hex's own run of transcribe.cpp's WER harness (`scripts/wer/run.py` and
/// `score.py`, language `he`, greedy, no timestamps) over the full FLEURS
/// Hebrew test split on apollo. Stock Whisper large-v3-turbo scored 29.69%
/// on the same run, so the fine-tune makes 38% fewer word errors; the
/// reporter of #752 measured 18.46% against 29.51% (37%) on an RTX 3090.
const IVRIT_WHISPER_RECORD: BenchmarkRecord = BenchmarkRecord {
    metric: ErrorMetric::Wer,
    value: 18.45,
    language: "he",
    dataset: Some("FLEURS test"),
    source: Some("Hex's run of the transcribe.cpp WER harness"),
    source_revision: Some("7045d1c"),
    utterances: Some(792),
    batch_size: Some(1),
    timestamps: Some(false),
    hardware: Some("Radeon 8060S (Vulkan)"),
    quantization: None,
    ci95: None,
};

/// transcribe.cpp's published conversion benchmark for the Q8_0 file
/// (handy-computer/parakeet-ultra-gguf's card): LibriSpeech test-clean in
/// full, batch 8, no timestamps, the `en` hint, decoded on CUDA at
/// transcribe.cpp 021aa17d. The benchmark artifact's digest and the scorer
/// versions are unrecorded; not measured in Hex.
const PARAKEET_ULTRA_RECORD: BenchmarkRecord = BenchmarkRecord {
    metric: ErrorMetric::Wer,
    value: 1.80,
    language: "en",
    dataset: Some("LibriSpeech test-clean"),
    source: Some("transcribe.cpp's published conversion benchmark"),
    source_revision: Some("021aa17d"),
    utterances: Some(2_620),
    batch_size: Some(8),
    timestamps: Some(false),
    hardware: Some("CUDA"),
    quantization: None,
    ci95: None,
};

/// Hex's run of the same harness over LibriSpeech test-clean with the
/// `en-US` prompt on apollo. Its FLEURS test runs scored 7.91% English and
/// 10.39% German with the language set, 8.77% and 10.46% with Auto.
const NEMOTRON_3_5_ASR_RECORD: BenchmarkRecord = BenchmarkRecord {
    metric: ErrorMetric::Wer,
    value: 3.05,
    language: "en",
    dataset: Some("LibriSpeech test-clean"),
    source: Some("Hex's run of the transcribe.cpp WER harness"),
    source_revision: Some("7045d1c"),
    utterances: Some(2_620),
    batch_size: Some(1),
    timestamps: Some(false),
    hardware: Some("Radeon 8060S (Vulkan)"),
    quantization: None,
    ci95: None,
};

/// Hex's run of the same harness over the full FLEURS Russian test split
/// on apollo (#952). Parakeet v3 scored 6.52% and Whisper large-v3-turbo
/// (with `ru`) 5.36% on the same run; transcribe.cpp publishes 5.35%.
const GIGAAM_V3_RECORD: BenchmarkRecord = BenchmarkRecord {
    metric: ErrorMetric::Wer,
    value: 5.37,
    language: "ru",
    dataset: Some("FLEURS test"),
    source: Some("Hex's run of the transcribe.cpp WER harness"),
    source_revision: Some("7045d1c"),
    utterances: Some(775),
    batch_size: Some(1),
    timestamps: Some(false),
    hardware: Some("Radeon 8060S (Vulkan)"),
    quantization: None,
    ci95: None,
};

/// Hex's run of the same harness over the full FLEURS Norwegian (Bokmål)
/// test split on apollo, with the `no` hint (#1150). On the same run Whisper
/// large-v3-turbo scored 8.94%, NB-Whisper's distilled turbo beta 8.65%,
/// its Medium converted to Q8_0 6.75%, and its Large (Q8_0, twice the size
/// and half the speed) 5.62%.
const NB_WHISPER_MEDIUM_RECORD: BenchmarkRecord = BenchmarkRecord {
    value: 6.68,
    language: "no",
    utterances: Some(357),
    ..GIGAAM_V3_RECORD
};

/// Hex's run of the same harness over the full FLEURS English test split on
/// apollo (#1252). On the same run Parakeet Unified English scored 4.01%,
/// Whisper large-v3-turbo 4.38%, Parakeet v3 4.88%, and Cohere 5.08%; in
/// French, German, Spanish, Portuguese, and Italian Granite trailed Parakeet
/// v3 (7.08, 5.51, 4.69, 8.21, and 4.24% against 5.35, 5.17, 3.78, 4.91, and
/// 3.06%), and in Japanese it tied Whisper turbo (5.04% CER against 5.02%).
const GRANITE_SPEECH_5_RECORD: BenchmarkRecord = BenchmarkRecord {
    value: 4.77,
    language: "en",
    utterances: Some(647),
    ..GIGAAM_V3_RECORD
};

/// Where a speed figure was measured.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[cfg_attr(not(test), allow(dead_code))]
pub enum Backend {
    Metal,
    Vulkan,
    Cpu,
}

/// A reference machine: the name the catalog gives it and the engine build
/// that measured it.
#[cfg_attr(not(test), allow(dead_code))]
struct Machine {
    name: &'static str,
    engine_revision: &'static str,
    measured_on: &'static str,
}

/// One speed measurement from transcribe.cpp's catalog: a Q8_0 model
/// transcribing one clip on a reference machine. Not a Hex run, and not
/// this computer: `realtime` is how many seconds of audio ran per second of
/// wall time (`xrt_wall`, model load excluded).
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(not(test), allow(dead_code))] // the picker reads it in a UI unit
pub struct SpeedRecord {
    pub backend: Backend,
    pub machine: &'static str,
    pub sample: &'static str,
    pub sample_seconds: f64,
    pub realtime: f64,
    pub engine_revision: &'static str,
    pub measured_on: &'static str,
}

/// The long clip, which the catalog's speed figures are quoted on.
#[cfg_attr(not(test), allow(dead_code))]
const LONG_SAMPLE_SECONDS: f64 = 35.0;

/// Every catalog speed record for `id`: Q8_0 on both clips, each reference
/// machine, each backend it ran. Empty for a model with none.
#[cfg_attr(not(test), allow(dead_code))]
pub fn speed_records(id: TranscriptionModelId) -> &'static [SpeedRecord] {
    catalog::speed_of(id)
}

/// The figure that sets a wait on `backend`: the 35 s clip on the slowest
/// reference machine that ran it, so no machine's speed is claimed for a
/// slower one. `None` when no record exists for `backend`.
#[cfg_attr(not(test), allow(dead_code))]
pub fn speed(id: TranscriptionModelId, backend: Backend) -> Option<&'static SpeedRecord> {
    speed_records(id)
        .iter()
        .filter(|record| record.backend == backend && record.sample_seconds > LONG_SAMPLE_SECONDS)
        .min_by(|left, right| left.realtime.total_cmp(&right.realtime))
}

/// Every catalog accuracy record for `id`, one per language.
#[cfg_attr(not(test), allow(dead_code))]
pub fn accuracy(id: TranscriptionModelId) -> &'static [BenchmarkRecord] {
    catalog::accuracy_of(id)
}

/// `id`'s catalog accuracy record for `language`, if the catalog has one.
pub fn accuracy_in(id: TranscriptionModelId, language: &str) -> Option<BenchmarkRecord> {
    catalog::accuracy_of(id)
        .iter()
        .find(|record| record.language == language)
        .copied()
}

/// `model`'s benchmark record, or `None` when the system manages the model
/// and publishes no figure (Apple Speech).
pub fn benchmark(model: &ModelDefinition) -> Option<BenchmarkRecord> {
    match model.id {
        TranscriptionModelId::Qwen3Asr06B => Some(QWEN3_ASR_RECORD),
        TranscriptionModelId::SenseVoiceSmall => Some(SENSEVOICE_RECORD),
        TranscriptionModelId::QwenCleoAsr17B => Some(QWENCLEO_RECORD),
        TranscriptionModelId::IvritWhisperLargeV3Turbo => Some(IVRIT_WHISPER_RECORD),
        TranscriptionModelId::ParakeetUltra => Some(PARAKEET_ULTRA_RECORD),
        TranscriptionModelId::Nemotron35Asr => Some(NEMOTRON_3_5_ASR_RECORD),
        TranscriptionModelId::GigaAmV3 => Some(GIGAAM_V3_RECORD),
        TranscriptionModelId::NbWhisperMedium => Some(NB_WHISPER_MEDIUM_RECORD),
        TranscriptionModelId::GraniteSpeech5 => Some(GRANITE_SPEECH_5_RECORD),
        // The catalog's figure for the language the model is chosen for.
        TranscriptionModelId::WhisperLargeV3
        | TranscriptionModelId::WhisperMedium
        | TranscriptionModelId::WhisperSmall => accuracy_in(model.id, "en"),
        TranscriptionModelId::BreezeAsr25 | TranscriptionModelId::Qwen3Asr17B => {
            accuracy_in(model.id, "zh")
        }
        // The catalog's English figures: their dataset, source, and
        // conditions were not recorded.
        _ => model
            .word_error_rate
            .map(|rate| BenchmarkRecord::unrecorded(ErrorMetric::Wer, rate, "en")),
    }
}

/// Whether `model` may keep a "Most accurate" badge for `language` with
/// Labs › Benchmark details on: it has a record, and every other
/// recommended model for the language that has one is comparable to it.
pub(crate) fn most_accurate_is_comparable(model: &ModelDefinition, language: &str) -> bool {
    let Some(record) = benchmark(model) else {
        return false;
    };
    record.dataset.is_some()
        && choices_for_runtime(language)
            .iter()
            .filter(|choice| choice.model.id != model.id)
            .filter_map(|choice| benchmark(choice.model))
            .all(|other| record.comparable(&other))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transcription::artifacts::QWEN_LANGUAGES;
    use crate::transcription::models::{LANGUAGES, MODELS, definition};
    use TranscriptionModelId::*;

    const NEW: [TranscriptionModelId; 5] = [
        WhisperLargeV3,
        WhisperMedium,
        WhisperSmall,
        BreezeAsr25,
        Qwen3Asr17B,
    ];

    /// Which reference machine sets each backend's wait, by hand from the
    /// catalog's "dots" rows at 2778ae4 (xrt_wall; the Metal and Vulkan
    /// columns are the base M4 and the Ryzen 4750U, the CPU one the 4750U).
    #[test]
    fn speed_picks_the_slowest_reference_machine_per_backend() {
        for (id, metal, vulkan, cpu) in [
            (WhisperLargeV3, 8.25, 2.32, 1.1),
            (WhisperMedium, 14.88, 4.55, 2.3),
            (WhisperSmall, 39.76, 12.57, 6.7),
            (BreezeAsr25, 8.46, 2.35, 1.1),
            (Qwen3Asr17B, 10.52, 3.68, 2.3),
        ] {
            let metal_record = speed(id, Backend::Metal).expect("a Metal record");
            assert_eq!(metal_record.machine, "Apple M4", "{id:?}");
            assert_eq!(metal_record.sample, "dots", "{id:?}");
            assert_eq!(metal_record.realtime, metal, "{id:?}");
            let vulkan_record = speed(id, Backend::Vulkan).expect("a Vulkan record");
            assert_eq!(vulkan_record.machine, "AMD Ryzen 7 PRO 4750U", "{id:?}");
            assert_eq!(vulkan_record.realtime, vulkan, "{id:?}");
            let cpu_record = speed(id, Backend::Cpu).expect("a CPU record");
            assert_eq!(cpu_record.machine, "AMD Ryzen 7 PRO 4750U", "{id:?}");
            assert!(
                (cpu_record.realtime - cpu).abs() < 0.06,
                "{id:?} {cpu_record:?}"
            );
            // Never the fastest machine, never the short clip.
            for backend in [Backend::Metal, Backend::Vulkan, Backend::Cpu] {
                let picked = speed(id, backend).unwrap();
                for record in speed_records(id)
                    .iter()
                    .filter(|record| record.backend == backend)
                {
                    assert!(
                        record.sample_seconds < LONG_SAMPLE_SECONDS
                            || record.realtime >= picked.realtime,
                        "{id:?} {backend:?}: {record:?} is slower than {picked:?}"
                    );
                }
            }
            // Every record names where and when it ran.
            for record in speed_records(id) {
                assert!(!record.engine_revision.is_empty() && !record.measured_on.is_empty());
                assert!(record.realtime > 0.0);
            }
        }
        // No record, no figure; no backend is borrowed from another.
        assert!(speed(ParakeetV3, Backend::Metal).is_none());
        assert!(speed_records(ParakeetV3).is_empty());
    }

    /// macOS labels a model with the base M4's Metal figure for the 35 s
    /// clip; everything else has no label until it is measured there.
    #[test]
    fn realtime_label_matches_this_os_backend() {
        for id in NEW {
            let label = definition(id).realtime;
            if cfg!(target_os = "macos") {
                let record = speed(id, Backend::Metal).unwrap();
                assert_eq!(label, format!("~{}x", record.realtime.round()), "{id:?}");
            } else {
                assert_eq!(label, "Unknown", "{id:?}");
            }
        }
    }

    #[test]
    fn accuracy_has_one_record_per_supported_language() {
        let known: Vec<&str> = LANGUAGES.iter().map(|(code, _)| *code).collect();
        for id in NEW {
            let model = definition(id);
            let records = accuracy(id);
            let mut languages: Vec<&str> = records.iter().map(|record| record.language).collect();
            let total = languages.len();
            languages.sort_unstable();
            languages.dedup();
            assert_eq!(languages.len(), total, "{id:?} repeats a language");
            for record in records {
                assert!(
                    known.contains(&record.language),
                    "{id:?} {}",
                    record.language
                );
                assert!(
                    model.supports_language(record.language),
                    "{id:?} has a record for {}, which it doesn't offer",
                    record.language
                );
                assert_eq!(record.dataset, Some("FLEURS test"));
                assert_eq!(record.quantization, Some("Q8_0"));
                assert_eq!(record.batch_size, Some(8));
                assert_eq!(record.timestamps, Some(false));
                assert_eq!(
                    record.hardware, None,
                    "L40S is a profile setting, not a record"
                );
                assert_eq!(record.source_revision, Some("2778ae4"));
                let (low, high) = record.ci95.expect("a confidence interval");
                assert!(
                    low <= record.value && record.value <= high,
                    "{id:?} {record:?}"
                );
            }
        }
        // Breeze's second Mandarin run (batch 1) is not a second record.
        let breeze: Vec<_> = accuracy(BreezeAsr25)
            .iter()
            .map(|r| (r.language, r.metric))
            .collect();
        assert_eq!(breeze, [("en", ErrorMetric::Wer), ("zh", ErrorMetric::Cer)]);
        let mut qwen: Vec<_> = accuracy(Qwen3Asr17B).iter().map(|r| r.language).collect();
        let mut expected = QWEN_LANGUAGES.to_vec();
        qwen.sort_unstable();
        expected.sort_unstable();
        assert_eq!(qwen, expected);
        // Medium and small have no Cantonese token; large-v3 has one.
        for id in [WhisperMedium, WhisperSmall] {
            assert!(accuracy_in(id, "yue").is_none(), "{id:?}");
            assert!(accuracy_in(id, "zh").is_some(), "{id:?}");
        }
        assert!(accuracy_in(WhisperLargeV3, "yue").is_some());
        // An error rate past 100% is stored as measured.
        assert_eq!(accuracy_in(WhisperSmall, "bn").unwrap().value, 103.04);
        assert_eq!(accuracy_in(WhisperMedium, "bn").unwrap().value, 102.52);
        assert!(accuracy_in(ParakeetV3, "en").is_none());
    }

    /// The catalog's figures for the languages the models are picked for,
    /// from FLEURS test (Q8_0, batch 8): 647 English and 945 Mandarin
    /// utterances.
    #[test]
    fn headline_benchmark_matches_word_error_rate() {
        for (id, language, metric, value, utterances) in [
            (WhisperLargeV3, "en", ErrorMetric::Wer, 4.03, 647),
            (WhisperMedium, "en", ErrorMetric::Wer, 4.64, 647),
            (WhisperSmall, "en", ErrorMetric::Wer, 6.51, 647),
            (BreezeAsr25, "zh", ErrorMetric::Cer, 8.1, 945),
            (Qwen3Asr17B, "zh", ErrorMetric::Cer, 7.14, 945),
        ] {
            let model = definition(id);
            let record = benchmark(model).expect("a headline record");
            assert_eq!(record.language, language, "{id:?}");
            assert_eq!(record.metric, metric, "{id:?}");
            assert_eq!(record.value, value, "{id:?}");
            assert_eq!(record.utterances, Some(utterances), "{id:?}");
            assert_eq!(model.word_error_rate, Some(record.value), "{id:?}");
        }
        // The same measurement is comparable across the new models.
        let large = benchmark(definition(WhisperLargeV3)).unwrap();
        let small = benchmark(definition(WhisperSmall)).unwrap();
        assert!(large.comparable(&small));
        let breeze = benchmark(definition(BreezeAsr25)).unwrap();
        assert!(
            !large.comparable(&breeze),
            "English WER is not Mandarin CER"
        );
    }

    /// Every catalog model still has a record, so no card loses its figure.
    #[test]
    fn every_catalog_model_keeps_a_benchmark() {
        for model in MODELS {
            assert_eq!(
                benchmark(model).is_none(),
                model.id == AppleSpeech,
                "{:?}",
                model.id
            );
        }
    }
}
