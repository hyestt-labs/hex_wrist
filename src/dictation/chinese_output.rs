//! Which Chinese characters a dictation comes out in: Traditional,
//! Simplified, or as the model wrote them. OpenCC through `ferrous-opencc`,
//! pure Rust with the tables compiled in. Mandarin uses `s2tw` and `tw2s`,
//! Cantonese `s2hk` and `hk2s`: the configurations Wristper's Swift app (and
//! Handy) used, not the `*p` ones. The phrase step would also rewrite words
//! the speaker said (打開 becomes 開啟; 軟體 becomes 软件), and a speaker's own
//! terms are already what the model heard. Only Chinese characters change;
//! Latin letters, digits, and punctuation pass through, and text already in
//! the chosen script is unchanged.

use std::sync::LazyLock;
use std::sync::atomic::{AtomicU8, Ordering};
use std::time::Instant;

use ferrous_opencc::OpenCC;
use ferrous_opencc::config::BuiltinConfig;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ChineseOutput {
    Traditional,
    Simplified,
    AsHeard,
}

impl ChineseOutput {
    pub(crate) const fn encoded(self) -> u8 {
        match self {
            Self::AsHeard => 0,
            Self::Traditional => 1,
            Self::Simplified => 2,
        }
    }

    const fn decode(value: u8) -> Self {
        match value {
            1 => Self::Traditional,
            2 => Self::Simplified,
            _ => Self::AsHeard,
        }
    }
}

/// The resolved choice dictation reads. Until settings load nothing is
/// converted.
static LIVE: AtomicU8 = AtomicU8::new(ChineseOutput::AsHeard.encoded());

/// Publishes the resolved choice (`resolve`) for the next dictation.
pub(crate) fn set_live(output: ChineseOutput) {
    LIVE.store(output.encoded(), Ordering::Relaxed);
}

pub(crate) fn live() -> ChineseOutput {
    ChineseOutput::decode(LIVE.load(Ordering::Relaxed))
}

/// The choice for `saved`, which `None` leaves to the system: the first
/// Chinese or Cantonese entry of the preferred languages, in order, decides;
/// without one the text stays as heard.
pub(crate) fn resolve(saved: Option<ChineseOutput>, preferred: &[String]) -> ChineseOutput {
    saved
        .or_else(|| preferred.iter().find_map(|tag| output_for_locale(tag)))
        .unwrap_or(ChineseOutput::AsHeard)
}

/// What a locale such as `zh-Hant-TW`, `zh_TW.UTF-8`, or `yue` asks for;
/// `None` when it is not Chinese. An explicit script wins over the region
/// (`zh-Hans-HK` is Simplified). Without either, Mandarin is Simplified and
/// Cantonese Traditional.
fn output_for_locale(tag: &str) -> Option<ChineseOutput> {
    // POSIX locales carry an encoding and modifier: `zh_TW.UTF-8@euro`.
    let tag = tag.split(['.', '@']).next().unwrap_or_default();
    let subtags: Vec<String> = tag
        .split(['-', '_'])
        .map(|subtag| subtag.trim().to_ascii_lowercase())
        .collect();
    let language = subtags.first()?.as_str();
    if language != "zh" && language != "yue" {
        return None;
    }
    let script = subtags
        .iter()
        .skip(1)
        .find_map(|subtag| match subtag.as_str() {
            "hant" => Some(ChineseOutput::Traditional),
            "hans" => Some(ChineseOutput::Simplified),
            _ => None,
        });
    let region = subtags
        .iter()
        .skip(1)
        .find_map(|subtag| match subtag.as_str() {
            "tw" | "hk" | "mo" => Some(ChineseOutput::Traditional),
            "cn" | "sg" | "my" => Some(ChineseOutput::Simplified),
            _ => None,
        });
    Some(script.or(region).unwrap_or(if language == "yue" {
        ChineseOutput::Traditional
    } else {
        ChineseOutput::Simplified
    }))
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Dialect {
    Mandarin,
    Cantonese,
}

static TAIWAN: LazyLock<Option<OpenCC>> = LazyLock::new(|| load(BuiltinConfig::S2tw));
static TAIWAN_SIMPLIFIED: LazyLock<Option<OpenCC>> = LazyLock::new(|| load(BuiltinConfig::Tw2s));
static HONG_KONG: LazyLock<Option<OpenCC>> = LazyLock::new(|| load(BuiltinConfig::S2hk));
static HONG_KONG_SIMPLIFIED: LazyLock<Option<OpenCC>> = LazyLock::new(|| load(BuiltinConfig::Hk2s));

fn load(config: BuiltinConfig) -> Option<OpenCC> {
    let started = Instant::now();
    match OpenCC::from_config(config) {
        Ok(opencc) => {
            tracing::info!(
                config = ?config,
                load_ms = started.elapsed().as_millis() as u64,
                "loaded the OpenCC tables"
            );
            Some(opencc)
        }
        Err(error) => {
            tracing::error!(%error, config = ?config, "the OpenCC tables did not load; transcripts stay as heard");
            None
        }
    }
}

/// Loads the tables `output` needs, so the first dictation after a settings
/// change does not pay for them.
pub(crate) fn preload(output: ChineseOutput) {
    if output == ChineseOutput::AsHeard {
        return;
    }
    std::thread::Builder::new()
        .name("opencc-preload".into())
        .spawn(move || {
            for dialect in [Dialect::Mandarin, Dialect::Cantonese] {
                let _ = table(output, dialect);
            }
        })
        .ok();
}

fn table(output: ChineseOutput, dialect: Dialect) -> Option<&'static OpenCC> {
    match (output, dialect) {
        (ChineseOutput::AsHeard, _) => None,
        (ChineseOutput::Traditional, Dialect::Mandarin) => TAIWAN.as_ref(),
        (ChineseOutput::Simplified, Dialect::Mandarin) => TAIWAN_SIMPLIFIED.as_ref(),
        (ChineseOutput::Traditional, Dialect::Cantonese) => HONG_KONG.as_ref(),
        (ChineseOutput::Simplified, Dialect::Cantonese) => HONG_KONG_SIMPLIFIED.as_ref(),
    }
}

/// Japanese shares kanji with Chinese, and OpenCC would rewrite them (学 to
/// 學), so a text that carries kana is left as heard.
fn has_kana(text: &str) -> bool {
    text.chars().any(|character| {
        matches!(character,
            '\u{3040}'..='\u{30FF}' | '\u{31F0}'..='\u{31FF}' | '\u{FF66}'..='\u{FF9F}')
    })
}

fn has_hangul(text: &str) -> bool {
    text.chars().any(|character| {
        matches!(character, '\u{1100}'..='\u{11FF}' | '\u{3130}'..='\u{318F}' | '\u{AC00}'..='\u{D7AF}')
    })
}

fn has_han(text: &str) -> bool {
    text.chars().any(|character| {
        matches!(character, '\u{3400}'..='\u{4DBF}' | '\u{4E00}'..='\u{9FFF}' | '\u{F900}'..='\u{FAFF}')
    })
}

/// Which table the text calls for, from the language the model ran in
/// (`zh`, `yue`, `auto`, or empty). An explicit other language is left
/// alone; an undecided one converts text that has Han characters and
/// neither kana nor Hangul, with Mandarin's table: dictation reports no
/// detected language, and the tables differ only in a few variant forms.
fn dialect(text: &str, language: &str) -> Option<Dialect> {
    if has_kana(text) {
        return None;
    }
    let primary = language
        .split(['-', '_'])
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    match primary.as_str() {
        "zh" => Some(Dialect::Mandarin),
        "yue" => Some(Dialect::Cantonese),
        "" | "auto" => (has_han(text) && !has_hangul(text)).then_some(Dialect::Mandarin),
        _ => None,
    }
}

/// `text` in the characters `output` asks for.
pub(crate) fn convert(text: &str, output: ChineseOutput, language: &str) -> String {
    let converted = dialect(text, language)
        .and_then(|dialect| table(output, dialect))
        .map(|opencc| opencc.convert(text));
    converted.unwrap_or_else(|| text.to_owned())
}

/// A dictation's text in the live choice. `hint` is the language the
/// recording itself names (a watch's), used when it is a real language;
/// otherwise the language the model runs in decides.
pub(crate) fn apply(text: &str, hint: &str) -> String {
    #[cfg(test)]
    CALLS.with(|calls| calls.set(calls.get() + 1));
    let output = live();
    if output == ChineseOutput::AsHeard {
        return text.to_owned();
    }
    let hint = hint.trim();
    if hint.is_empty() || hint.eq_ignore_ascii_case("auto") {
        convert(
            text,
            output,
            &crate::app_settings::transcription_selection().1.language,
        )
    } else {
        convert(text, output, hint)
    }
}

#[cfg(test)]
thread_local! {
    static CALLS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// How many times `apply` ran on this thread: tests pin that a dictation is
/// converted once, ahead of the replacements, on every OS.
#[cfg(test)]
pub(crate) fn calls() -> usize {
    CALLS.with(std::cell::Cell::get)
}

#[cfg(test)]
mod tests {
    use super::ChineseOutput::{AsHeard, Simplified, Traditional};
    use super::*;

    fn tags(tags: &[&str]) -> Vec<String> {
        tags.iter().map(|tag| (*tag).to_owned()).collect()
    }

    #[test]
    fn simplified_becomes_traditional_with_taiwan_variants() {
        let to = |text: &str| convert(text, Traditional, "zh");
        assert_eq!(to("头发后来干净"), "頭髮後來乾淨");
        assert_eq!(to("只有一只猫在里面"), "只有一隻貓在裡面");
        // The phrase step is off: the speaker's own words stay.
        assert_eq!(to("幫我打開軟體"), "幫我打開軟體");
        assert_eq!(to("今天天气很好"), "今天天氣很好");
    }

    #[test]
    fn simplified_keeps_taiwan_words_in_simplified_characters() {
        // `tw2sp` would make these 软件 and 网络.
        assert_eq!(convert("軟體與網路", Simplified, "zh"), "软体与网路");
        assert_eq!(convert("頭髮後來乾淨", Simplified, "zh"), "头发后来干净");
    }

    #[test]
    fn latin_digits_and_traditional_text_pass_through() {
        for text in [
            "Hello, world 123",
            "明天 3:30 開會，記得帶 MacBook。",
            "幫我打開 Cursor 的 settings",
            "",
        ] {
            assert_eq!(convert(text, Traditional, "zh"), text);
        }
        assert_eq!(
            convert("把 config.json 复制到 ~/.config", Traditional, "auto"),
            "把 config.json 複製到 ~/.config"
        );
    }

    #[test]
    fn as_heard_never_changes_text() {
        assert_eq!(convert("头发", AsHeard, "zh"), "头发");
        assert_eq!(convert("頭髮", AsHeard, "auto"), "頭髮");
    }

    #[test]
    fn chinese_and_undecided_convert_and_other_languages_do_not() {
        assert_eq!(convert("头发", Traditional, "zh"), "頭髮");
        assert_eq!(convert("头发", Traditional, "zh-TW"), "頭髮");
        assert_eq!(convert("头发", Traditional, "auto"), "頭髮");
        assert_eq!(convert("头发", Traditional, ""), "頭髮");
        assert_eq!(convert("头发", Traditional, "en"), "头发");
        assert_eq!(convert("头发", Traditional, "ko"), "头发");
    }

    #[test]
    fn japanese_is_never_converted() {
        // 学 and 大学 are Japanese kanji; Traditional output would make them 學.
        assert_eq!(
            convert("学校へ行きます", Traditional, "auto"),
            "学校へ行きます"
        );
        assert_eq!(convert("学校へ行きます", Traditional, ""), "学校へ行きます");
        assert_eq!(convert("東京大学", Traditional, "ja"), "東京大学");
        assert_eq!(convert("学校", Traditional, "en"), "学校");
        assert_eq!(convert("学校", Traditional, ""), "學校");
        // Half-width katakana and the phonetic extensions count as kana.
        assert_eq!(convert("学ﾃｽﾄ", Traditional, "auto"), "学ﾃｽﾄ");
        // Chinese output toward Simplified spares Japanese too.
        assert_eq!(convert("國の学校へ", Simplified, "auto"), "國の学校へ");
    }

    #[test]
    fn korean_is_never_converted_when_the_language_is_undecided() {
        assert_eq!(convert("학교 学校", Traditional, "auto"), "학교 学校");
    }

    #[test]
    fn cantonese_uses_hong_kong_tables() {
        // Mandarin's table gives 裡面, Hong Kong's 裏面.
        assert_eq!(convert("里面", Traditional, "yue"), "裏面");
        assert_eq!(convert("里面", Traditional, "zh"), "裡面");
        assert_eq!(convert("裏面", Simplified, "yue"), "里面");
    }

    #[test]
    fn system_default_follows_first_chinese_preferred_language() {
        let cases: &[(&[&str], ChineseOutput)] = &[
            (&["en-US", "zh-Hant-TW"], Traditional),
            (&["zh-Hans-HK"], Simplified),
            (&["zh-MO"], Traditional),
            (&["zh-HK"], Traditional),
            (&["zh-TW"], Traditional),
            (&["zh-CN"], Simplified),
            (&["zh-SG"], Simplified),
            (&["zh"], Simplified),
            (&["zh-Hans"], Simplified),
            (&["yue"], Traditional),
            (&["yue-Hant-HK"], Traditional),
            (&["yue-Hans"], Simplified),
            (&["zh_TW.UTF-8"], Traditional),
            (&["zh_CN.UTF-8@euro"], Simplified),
            (&["en", "ja"], AsHeard),
            (&["zh-Hans-CN", "zh-Hant-TW"], Simplified),
            (&[], AsHeard),
            (&["C"], AsHeard),
        ];
        for (preferred, expected) in cases {
            assert_eq!(resolve(None, &tags(preferred)), *expected, "{preferred:?}");
        }
    }

    #[test]
    fn a_saved_choice_wins_over_the_system() {
        assert_eq!(resolve(Some(AsHeard), &tags(&["zh-Hant-TW"])), AsHeard);
        assert_eq!(
            resolve(Some(Simplified), &tags(&["zh-Hant-TW"])),
            Simplified
        );
        assert_eq!(resolve(Some(Traditional), &tags(&["en"])), Traditional);
    }

    #[test]
    fn apply_follows_the_live_choice_and_the_recordings_own_language() {
        set_live(Traditional);
        assert_eq!(apply("头发", "zh"), "頭髮");
        assert_eq!(apply("头发", "en"), "头发");
        set_live(Simplified);
        assert_eq!(apply("頭髮", "zh"), "头发");
        set_live(AsHeard);
        assert_eq!(apply("头发", "zh"), "头发");
    }
}
