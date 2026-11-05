//! Port of Wristper's `SuspectText` and the language tables it needs.
//!
//! Wristper classifies a letter by the first word of its Unicode character name.
//! Rust has no names table here, so `letter_scripts` uses the Script property
//! instead. Known divergence: U+30FC (prolonged sound mark) is a letter whose
//! Swift name prefix is "KATAKANA-HIRAGANA", and whose Script is Common; both
//! land outside every allowed set, so verdicts agree.

use std::collections::{BTreeSet, HashSet};
use std::sync::LazyLock;

use unicode_general_category::{GeneralCategory, get_general_category};
use unicode_normalization::UnicodeNormalization;
use unicode_script::{Script as UScript, UnicodeScript};
use unicode_segmentation::UnicodeSegmentation;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SuspectReason {
    Empty,
    Hallucination,
    ForeignScript,
    Kana,
}

impl SuspectReason {
    pub(crate) fn text(self, chinese: bool) -> &'static str {
        match (self, chinese) {
            (Self::Empty, true) => "沒有辨識出文字",
            (Self::Empty, false) => "No speech detected",
            (Self::Hallucination, true) => "疑似靜音時模型自己編出的字幕文字，未貼上",
            (Self::Hallucination, false) => {
                "Likely subtitle hallucination from silence; not pasted"
            }
            (Self::ForeignScript, true) => "辨識結果夾雜其他語言文字，通常代表收到的是噪音",
            (Self::ForeignScript, false) => {
                "Unexpected script in transcript, usually caused by background noise"
            }
            (Self::Kana, true) => "辨識結果不確定（出現日文假名，通常代表收到的是噪音）",
            (Self::Kana, false) => "Uncertain transcript (Japanese kana detected, usually noise)",
        }
    }
}

/// Wristper's script names; anything else keeps its Unicode script.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Script {
    Cjk,
    Ideographic,
    Latin,
    Bopomofo,
    Hiragana,
    Katakana,
    Hangul,
    Cyrillic,
    Greek,
    Hebrew,
    Arabic,
    Thai,
    Devanagari,
    Other(UScript),
}

impl Script {
    // unicode_script::Script has no Ord, so order Other by its full name.
    fn sort_key(self) -> (u8, &'static str) {
        match self {
            Self::Cjk => (0, ""),
            Self::Ideographic => (1, ""),
            Self::Latin => (2, ""),
            Self::Bopomofo => (3, ""),
            Self::Hiragana => (4, ""),
            Self::Katakana => (5, ""),
            Self::Hangul => (6, ""),
            Self::Cyrillic => (7, ""),
            Self::Greek => (8, ""),
            Self::Hebrew => (9, ""),
            Self::Arabic => (10, ""),
            Self::Thai => (11, ""),
            Self::Devanagari => (12, ""),
            Self::Other(script) => (13, script.full_name()),
        }
    }
}

impl Ord for Script {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.sort_key().cmp(&other.sort_key())
    }
}

impl PartialOrd for Script {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl From<UScript> for Script {
    fn from(script: UScript) -> Self {
        match script {
            UScript::Han => Self::Cjk,
            UScript::Latin => Self::Latin,
            UScript::Bopomofo => Self::Bopomofo,
            UScript::Hiragana => Self::Hiragana,
            UScript::Katakana => Self::Katakana,
            UScript::Hangul => Self::Hangul,
            UScript::Cyrillic => Self::Cyrillic,
            UScript::Greek => Self::Greek,
            UScript::Hebrew => Self::Hebrew,
            UScript::Arabic => Self::Arabic,
            UScript::Thai => Self::Thai,
            UScript::Devanagari => Self::Devanagari,
            other => Self::Other(other),
        }
    }
}

const EXACT: &[&str] = &[
    "詞曲 李宗盛",
    "词曲 李宗盛",
    "作詞 李宗盛 作曲 李宗盛",
    "作词 李宗盛 作曲 李宗盛",
    "中文字幕",
    "謝謝觀看",
    "谢谢观看",
    "感謝觀看",
    "感谢观看",
    "感謝收看",
    "感谢收看",
    "謝謝收看",
    "谢谢收看",
    "thank you for watching",
    "thanks for watching",
    "thank you",
    "you",
];
const CONTAINS: &[&str] = &[
    "amara.org",
    "明鏡與點點",
    "明镜与点点",
    "優優獨播劇場",
    "优优独播剧场",
    "YoYo Television Series Exclusive",
    "小編字幕",
    "小编字幕",
];
const SHORT_CONTAINS: &[&str] = &["字幕提供", "字幕由"];
const SHORT_LIMIT: usize = 20;

static EXACT_NORMALIZED: LazyLock<HashSet<String>> =
    LazyLock::new(|| EXACT.iter().map(|s| normalize(s)).collect());
static CONTAINS_NORMALIZED: LazyLock<Vec<String>> =
    LazyLock::new(|| CONTAINS.iter().map(|s| normalize(s)).collect());
static SHORT_NORMALIZED: LazyLock<Vec<String>> =
    LazyLock::new(|| SHORT_CONTAINS.iter().map(|s| normalize(s)).collect());

const FOREIGN_FOR_ENGLISH: &[Script] = &[
    Script::Cyrillic,
    Script::Hangul,
    Script::Arabic,
    Script::Hebrew,
    Script::Thai,
    Script::Devanagari,
];
const ALLOWED_FOR_CHINESE: &[Script] = &[
    Script::Cjk,
    Script::Ideographic,
    Script::Latin,
    Script::Bopomofo,
    Script::Hiragana,
    Script::Katakana,
];

fn is_dropped_by_normalize(c: char) -> bool {
    use GeneralCategory::*;
    matches!(
        get_general_category(c),
        ConnectorPunctuation
            | DashPunctuation
            | OpenPunctuation
            | ClosePunctuation
            | InitialPunctuation
            | FinalPunctuation
            | OtherPunctuation
            | MathSymbol
            | CurrencySymbol
            | ModifierSymbol
            | OtherSymbol
            | SpaceSeparator
            | LineSeparator
            | ParagraphSeparator
            | Control
            | Format
            | Surrogate
            | PrivateUse
            | Unassigned
    )
}

/// Fold width and case, drop whitespace, punctuation, symbols and controls.
pub(crate) fn normalize(text: &str) -> String {
    text.nfkc()
        .collect::<String>()
        .to_lowercase()
        .chars()
        .filter(|&c| !is_dropped_by_normalize(c))
        .collect()
}

pub(crate) fn is_silence_hallucination(text: &str) -> bool {
    let plain = normalize(text);
    if EXACT_NORMALIZED.contains(&plain)
        || CONTAINS_NORMALIZED
            .iter()
            .any(|p| plain.contains(p.as_str()))
    {
        return true;
    }
    plain.graphemes(true).count() <= SHORT_LIMIT
        && SHORT_NORMALIZED.iter().any(|p| plain.contains(p.as_str()))
}

/// The script of every letter in `text`.
pub(crate) fn letter_scripts(text: &str) -> BTreeSet<Script> {
    use GeneralCategory::*;
    text.nfkc()
        .filter(|&c| {
            matches!(
                get_general_category(c),
                UppercaseLetter | LowercaseLetter | TitlecaseLetter | ModifierLetter | OtherLetter
            )
        })
        .map(|c| Script::from(c.script()))
        .collect()
}

fn has_kana(text: &str) -> bool {
    text.chars()
        .any(|c| (0x3040..=0x30FF).contains(&(c as u32)))
}

pub(crate) fn reason(
    text: &str,
    language: &str,
    requested_language: Option<&str>,
    preferred: &[&str],
) -> Option<SuspectReason> {
    if text.trim().is_empty() {
        return Some(SuspectReason::Empty);
    }
    if is_silence_hallucination(text) {
        return Some(SuspectReason::Hallucination);
    }
    let scripts = letter_scripts(text);
    let intent = requested_language.unwrap_or(language);
    if intent == "auto" {
        let allowed = language::allowed_auto_languages(preferred);
        if !language.is_empty() && language != "auto" && !allowed.contains(language) {
            return Some(SuspectReason::ForeignScript);
        }
        if !allowed.contains("ja") && has_kana(text) {
            return Some(SuspectReason::Kana);
        }
        let allowed_scripts = language::allowed_scripts(&allowed);
        if !scripts.is_subset(&allowed_scripts) {
            return Some(SuspectReason::ForeignScript);
        }
    }
    if language == "zh" {
        // Pure Chinese never carries kana; the model is guessing.
        if has_kana(text) {
            return Some(SuspectReason::Kana);
        }
        if scripts.iter().any(|s| !ALLOWED_FOR_CHINESE.contains(s)) {
            return Some(SuspectReason::ForeignScript);
        }
    } else if language == "en" {
        if scripts.iter().any(|s| FOREIGN_FOR_ENGLISH.contains(s)) {
            return Some(SuspectReason::ForeignScript);
        }
    } else if !language::script_matches(text, language) {
        return Some(SuspectReason::ForeignScript);
    }
    None
}

pub(crate) mod language {
    use super::{BTreeSet, Script, has_kana, letter_scripts};
    use std::collections::BTreeSet as Set;

    /// Whisper language codes listed in settings.
    pub(crate) fn codes() -> &'static [&'static str] {
        &[
            "zh", "en", "ja", "ko", "yue", "es", "fr", "de", "it", "pt", "ru", "vi", "th", "id",
            "ms", "tl", "hi", "ar", "tr", "nl", "pl", "sv", "no", "da", "fi", "uk", "cs", "el",
            "he", "ro", "hu",
        ]
    }

    const ALIASES: &[(&str, &str)] = &[
        ("nb", "no"),
        ("nn", "no"),
        ("fil", "tl"),
        ("iw", "he"),
        ("in", "id"),
    ];

    /// Scripts a language normally writes; `None` for the Latin-only languages.
    fn scripts(language: &str) -> Option<&'static [Script]> {
        use Script::*;
        Some(match language {
            "zh" => &[Cjk, Ideographic, Bopomofo, Latin],
            "yue" => &[Cjk, Ideographic, Latin],
            "ja" => &[Cjk, Ideographic, Hiragana, Katakana, Latin],
            "ko" => &[Hangul, Cjk, Latin],
            "ru" | "uk" => &[Cyrillic, Latin],
            "el" => &[Greek, Latin],
            "he" => &[Hebrew, Latin],
            "ar" => &[Arabic, Latin],
            "th" => &[Thai, Latin],
            "hi" => &[Devanagari, Latin],
            _ => return None,
        })
    }

    /// False when the transcript's scripts do not fit the detected language.
    /// Unknown languages always match.
    pub(crate) fn script_matches(text: &str, language: &str) -> bool {
        if language.is_empty() || language == "auto" {
            return true;
        }
        if language == "zh" && has_kana(text) {
            return false;
        }
        let allowed: &[Script] = match scripts(language) {
            Some(known) => known,
            None if codes().contains(&language) => &[Script::Latin],
            None => return true,
        };
        letter_scripts(text).iter().all(|s| allowed.contains(s))
    }

    /// Whisper codes found in the system preferred-language list, in order.
    pub(crate) fn preferred_codes(preferred: &[&str]) -> Vec<&'static str> {
        let mut result: Vec<&'static str> = Vec::new();
        for identifier in preferred {
            let base = identifier
                .split(['-', '_'])
                .find(|s| !s.is_empty())
                .unwrap_or("")
                .to_lowercase();
            let code = ALIASES
                .iter()
                .find(|(alias, _)| *alias == base)
                .map_or(base.as_str(), |(_, code)| code);
            if let Some(known) = codes().iter().find(|c| **c == code)
                && !result.contains(known)
            {
                result.push(known);
            }
        }
        result
    }

    /// Languages auto-detect may return: preferred ones plus Chinese and English.
    pub(crate) fn allowed_auto_languages(preferred: &[&str]) -> Set<&'static str> {
        let mut set: Set<&'static str> = preferred_codes(preferred).into_iter().collect();
        set.extend(["zh", "yue", "en"]);
        set
    }

    pub(crate) fn allowed_scripts(languages: &Set<&'static str>) -> BTreeSet<Script> {
        let mut result = BTreeSet::from([Script::Latin]);
        for language in languages {
            if let Some(known) = scripts(language) {
                result.extend(known.iter().copied());
            }
        }
        result
    }

    /// Languages to retry with, in order, after auto-detect guessed `excluding`.
    pub(crate) fn retry_candidates(preferred: &[&str], excluding: &str) -> Vec<&'static str> {
        let mut result = preferred_codes(preferred);
        for fallback in ["zh", "en"] {
            if !result.contains(&fallback) {
                result.push(fallback);
            }
        }
        // Whispered Chinese is most often misheard as these; try Chinese first.
        if ["ko", "ja", "th", "vi", "yue"].contains(&excluding)
            && let Some(index) = result.iter().position(|c| *c == "zh")
            && index > 0
        {
            let zh = result.remove(index);
            result.insert(0, zh);
        }
        result.retain(|c| *c != excluding);
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HALLUCINATION: Option<SuspectReason> = Some(SuspectReason::Hallucination);
    const FOREIGN: Option<SuspectReason> = Some(SuspectReason::ForeignScript);

    #[test]
    fn known_chinese_hallucinations() {
        for text in [
            "詞曲 李宗盛",
            "词曲 李宗盛",
            "詞曲：李宗盛。",
            "字幕由Amara.org社區提供",
            "字幕由 Amara.org 社区提供",
            "請不吝點贊 訂閱 轉發 打賞支持明鏡與點點欄目",
            "请不吝点赞 订阅 转发 打赏支持明镜与点点栏目",
            "中文字幕",
            "字幕提供",
            "字幕由 XXX 提供",
            "謝謝觀看",
            "谢谢观看。",
            "感謝收看",
            "感谢收看",
            "謝謝收看！",
            "優優獨播劇場",
            "YoYo Television Series Exclusive",
            "小編字幕",
        ] {
            assert_eq!(reason(text, "zh", None, &[]), HALLUCINATION, "{text}");
        }
    }

    #[test]
    fn known_english_hallucinations() {
        for text in [
            "Thank you for watching",
            "Thanks for watching!",
            "Thank you.",
            "You",
            "Subtitles by the Amara.org community",
        ] {
            assert_eq!(reason(text, "en", None, &[]), HALLUCINATION, "{text}");
        }
    }

    #[test]
    fn foreign_scripts() {
        for (text, language) in [
            ("也行ест也行 fue靠我的期待。", "zh"),
            ("안녕하세요 오늘", "zh"),
            ("Γεια σου κόσμε", "zh"),
            ("مرحبا بالعالم", "zh"),
            ("สวัสดีครับ", "zh"),
            ("Привет мир", "en"),
            ("안녕하세요", "en"),
            ("שלום עולם", "en"),
            ("नमस्ते दुनिया", "en"),
        ] {
            assert_eq!(reason(text, language, None, &[]), FOREIGN, "{text}");
        }
    }

    #[test]
    fn kana_in_chinese() {
        assert_eq!(
            reason("これはテスト", "zh", None, &[]),
            Some(SuspectReason::Kana)
        );
    }

    #[test]
    fn empty() {
        assert_eq!(reason("  \n", "zh", None, &[]), Some(SuspectReason::Empty));
    }

    #[test]
    fn normal_text_is_not_suspect() {
        for (text, language) in [
            ("但還是沒有辦法成功的錄音。", "zh"),
            ("Hey, my name is WJ and I'm d…", "en"),
            ("所以說現在的模型是使用什麼?我可以替換模型嗎?", "zh"),
            ("幫我打開 Cursor 的 settings", "zh"),
            ("這部影片的字幕要改成繁體中文", "zh"),
            ("請幫我確認字幕由誰提供，還有要不要加中文字幕檔案", "zh"),
            ("I want to say thank you for watching my kids today", "en"),
            ("Thank you for the update, I will review it tonight", "en"),
            ("謝謝觀看者的回饋，我們會再調整畫面", "zh"),
            ("Please call Li Zongsheng about the lyrics", "en"),
            ("明天 3:30 開會，記得帶 MacBook。", "zh"),
            ("Café résumé naïve", "en"),
            ("我的名字是 John Müller", "zh"),
        ] {
            assert_eq!(reason(text, language, None, &[]), None, "{text}");
        }
    }

    #[test]
    fn normalize_folds_width_case_and_punctuation() {
        assert_eq!(normalize("Ｔｈａｎｋ　ｙｏｕ！"), "thankyou");
        assert_eq!(normalize("詞曲：李宗盛。"), "詞曲李宗盛");
    }

    #[test]
    fn auto_detect_rejects_unexpected_foreign_languages_and_scripts() {
        let preferred = ["en-US", "zh-Hant-US"];
        let auto = Some("auto");
        assert_eq!(reason("서용하네일", "ko", auto, &preferred), FOREIGN);
        assert_eq!(reason("Привет мир", "ru", auto, &preferred), FOREIGN);
        assert_eq!(reason("これはテスト", "ja", auto, &preferred), FOREIGN);
        // An explicit Korean request lets Korean through.
        assert_eq!(reason("서용하네일", "ko", Some("ko"), &preferred), None);
        // Korean in the system languages lets auto-detected Korean through.
        assert_eq!(reason("서용하네일", "ko", auto, &["en-US", "ko-KR"]), None);
        assert_eq!(language::retry_candidates(&preferred, "ko"), ["zh", "en"]);
    }

    #[test]
    fn language_aliases_and_separators() {
        assert_eq!(
            language::preferred_codes(&["nb_NO", "fil-PH", "xx"]),
            ["no", "tl"]
        );
    }
}
