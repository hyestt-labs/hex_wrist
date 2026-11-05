//! Traditional Chinese with Taiwan character variants for watch
//! transcripts: OpenCC's `s2tw` through `ferrous-opencc`, pure Rust with
//! the tables compiled in. It is the configuration Wristper's Swift app
//! (and Handy) used, not `s2twp`: the phrase step would also rewrite words
//! the speaker said in Traditional (打開 becomes 開啟), and a Taiwanese
//! speaker's own terms (軟體, 滑鼠) are already what the model heard. Only
//! Chinese characters change; Latin letters, digits, and punctuation pass
//! through, and text already Traditional is unchanged.

use std::sync::LazyLock;

use ferrous_opencc::OpenCC;
use ferrous_opencc::config::BuiltinConfig;

static TAIWAN: LazyLock<Option<OpenCC>> = LazyLock::new(|| {
    match OpenCC::from_config(BuiltinConfig::S2tw) {
        Ok(opencc) => Some(opencc),
        Err(error) => {
            tracing::error!(%error, "the OpenCC s2tw tables did not load; transcripts stay as heard");
            None
        }
    }
});

/// Whether a recording's transcript is converted: the watch asked for
/// Chinese, or let the model decide (`auto` or nothing).
pub(crate) fn applies(language: &str) -> bool {
    language.is_empty() || language == "auto" || language.starts_with("zh")
}

/// Japanese shares kanji with Chinese, and s2tw would rewrite them (学 to
/// 學), so a recording that carries kana is left as heard.
fn has_kana(text: &str) -> bool {
    text.chars()
        .any(|character| ('\u{3040}'..='\u{30FF}').contains(&character))
}

/// What the watch pastes: `text` in Traditional Chinese when the recording
/// is Chinese or undecided and not Japanese, else unchanged.
pub(crate) fn for_recording(text: &str, language: &str) -> String {
    if applies(language) && !has_kana(text) {
        to_traditional(text)
    } else {
        text.to_owned()
    }
}

/// `text` in Traditional Chinese with Taiwan character variants.
pub(crate) fn to_traditional(text: &str) -> String {
    match TAIWAN.as_ref() {
        Some(opencc) => opencc.convert(text),
        None => text.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simplified_becomes_traditional_with_taiwan_variants() {
        assert_eq!(to_traditional("头发后来干净"), "頭髮後來乾淨");
        assert_eq!(to_traditional("只有一只猫在里面"), "只有一隻貓在裡面");
        // The phrase step is off: the speaker's own words stay.
        assert_eq!(to_traditional("幫我打開軟體"), "幫我打開軟體");
    }

    #[test]
    fn latin_digits_and_traditional_text_pass_through() {
        for text in [
            "Hello, world 123",
            "明天 3:30 開會，記得帶 MacBook。",
            "幫我打開 Cursor 的 settings",
            "",
        ] {
            assert_eq!(to_traditional(text), text);
        }
        assert_eq!(
            to_traditional("把 config.json 复制到 ~/.config"),
            "把 config.json 複製到 ~/.config"
        );
    }

    #[test]
    fn chinese_and_auto_convert_and_other_languages_do_not() {
        assert!(applies("zh"));
        assert!(applies("zh-TW"));
        assert!(applies("auto"));
        assert!(applies(""));
        assert!(!applies("en"));
        assert!(!applies("ja"));
    }

    #[test]
    fn japanese_is_left_alone_even_when_the_language_is_undecided() {
        // 学 is simplified-looking Japanese kanji; s2tw would make it 學.
        assert_eq!(for_recording("学校へ行きます", "auto"), "学校へ行きます");
        assert_eq!(for_recording("学校", ""), "學校");
        assert_eq!(for_recording("学校", "en"), "学校");
    }
}
