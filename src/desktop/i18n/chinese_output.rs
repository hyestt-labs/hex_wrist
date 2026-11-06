//! The Chinese characters strings (Wristper fork): the Output pane's card
//! for `chinese_output`, in every interface language. They live apart from
//! [`super::TRANSLATIONS`] so that table stays under its size cap, like
//! [`super::apple_watch`], and each language's table takes its column from
//! here ([`column`]). Traditional Chinese is written out in Taiwan wording
//! ([`TRADITIONAL`]) rather than converted from the Simplified column.
//! Columns: Polish, Chinese (Simplified), Japanese, German, Spanish,
//! Ukrainian, Brazilian Portuguese, French, Russian. Not reviewed by native
//! speakers.

/// Columns of [`TRANSLATIONS`] in the order of [`super::column_for_code`]'s
/// first six languages, then Portuguese, French, and Russian.
pub(super) const LANGUAGES: usize = 9;
const PORTUGUESE: usize = 6;
const FRENCH: usize = 7;
const RUSSIAN: usize = 8;

/// This file's strings for `column` of the main table (0 through 5).
pub(super) fn column(column: usize) -> impl Iterator<Item = (&'static str, &'static str)> {
    TRANSLATIONS
        .iter()
        .map(move |(english, translated)| (*english, translated[column]))
}

pub(super) fn portuguese() -> impl Iterator<Item = (&'static str, &'static str)> {
    column(PORTUGUESE)
}

pub(super) fn french() -> impl Iterator<Item = (&'static str, &'static str)> {
    column(FRENCH)
}

pub(super) fn russian() -> impl Iterator<Item = (&'static str, &'static str)> {
    column(RUSSIAN)
}

/// The Traditional Chinese (Taiwan) strings.
pub(super) fn traditional() -> impl Iterator<Item = (&'static str, &'static str)> {
    TRADITIONAL.iter().copied()
}

#[rustfmt::skip]
pub(super) const TRANSLATIONS: &[(&str, [&str; LANGUAGES])] = &[
    ("Chinese characters", ["Znaki chińskie", "中文输出", "中国語の字体", "Chinesische Schrift", "Caracteres chinos", "Китайські ієрогліфи", "Caracteres chineses", "Caractères chinois", "Китайские иероглифы"]),
    ("Only changes Chinese and Cantonese text", ["Zmienia tylko tekst chiński i kantoński", "只影响中文与粤语", "中国語と広東語だけに適用", "Betrifft nur Chinesisch und Kantonesisch", "Solo cambia el chino y el cantonés", "Змінює лише китайський і кантонський текст", "Só altera chinês e cantonês", "Ne modifie que le chinois et le cantonais", "Меняет только китайский и кантонский текст"]),
    ("System ({})", ["Jak w systemie ({})", "跟随系统（{}）", "システムに合わせる（{}）", "Wie das System ({})", "Como el sistema ({})", "Як у системі ({})", "Como o sistema ({})", "Comme le système ({})", "Как в системе ({})"]),
    ("Traditional", ["Tradycyjne", "繁体中文", "繁体字", "Traditionell", "Tradicional", "Традиційні", "Tradicional", "Traditionnel", "Традиционные"]),
    ("Simplified", ["Uproszczone", "简体中文", "簡体字", "Vereinfacht", "Simplificado", "Спрощені", "Simplificado", "Simplifié", "Упрощённые"]),
    ("As heard", ["Jak usłyszano", "照辨识结果", "認識結果のまま", "Wie erkannt", "Tal como se oyó", "Як розпізнано", "Como foi ouvido", "Tel qu'entendu", "Как распознано"]),
];

/// Traditional Chinese in Taiwan wording, by the English source string.
#[rustfmt::skip]
const TRADITIONAL: &[(&str, &str)] = &[
    ("Chinese characters", "中文字體"),
    ("Only changes Chinese and Cantonese text", "只影響中文與粵語"),
    ("System ({})", "跟隨系統（{}）"),
    ("Traditional", "繁體中文"),
    ("Simplified", "簡體中文"),
    ("As heard", "照辨識結果"),
];

#[cfg(test)]
mod tests {
    use super::*;

    /// The strings the Output pane's card shows, in the order it shows them.
    fn english() -> Vec<&'static str> {
        TRANSLATIONS.iter().map(|(english, _)| *english).collect()
    }

    #[test]
    fn nothing_here_repeats_another_table_or_itself() {
        let elsewhere: std::collections::HashSet<_> = super::super::TRANSLATIONS
            .iter()
            .map(|(english, _)| *english)
            .chain(
                super::super::apple_watch::TRANSLATIONS
                    .iter()
                    .map(|(english, _)| *english),
            )
            .collect();
        let mut seen = std::collections::HashSet::new();
        for (english, translated) in TRANSLATIONS {
            assert!(!elsewhere.contains(english), "{english:?} is elsewhere");
            assert!(seen.insert(english), "{english:?} is listed twice");
            assert!(
                translated.iter().all(|text| !text.is_empty()),
                "{english:?}"
            );
            if english.contains("{}") {
                assert!(
                    translated.iter().all(|text| text.contains("{}")),
                    "{english:?} lost its placeholder"
                );
            }
        }
    }

    #[test]
    fn every_string_has_one_taiwan_translation_with_its_placeholder() {
        let mut seen = Vec::new();
        for (source, translated) in TRADITIONAL {
            assert!(english().contains(source), "{source:?} is not ours");
            assert!(!seen.contains(source), "{source:?} is listed twice");
            seen.push(*source);
            assert_eq!(
                source.contains("{}"),
                translated.contains("{}"),
                "{source:?}"
            );
        }
        assert_eq!(seen, english(), "every string has one, in order");
    }

    /// What zh-Hant shows, through the table the interface reads.
    #[test]
    fn traditional_chinese_reads_as_taiwan_wording() {
        let table = super::super::traditional_chinese::table();
        for (english, _) in TRANSLATIONS {
            let shown = &table[english];
            for mainland in [
                "設置", "設備", "連接", "粘貼", "信息", "軟件", "網絡", "手表",
            ] {
                assert!(
                    !shown.contains(mainland),
                    "{english:?} shows {shown:?} with {mainland:?}"
                );
            }
        }
        assert_eq!(table["Chinese characters"], "中文字體");
        assert_eq!(table["System ({})"], "跟隨系統（{}）");
        assert_eq!(table["Traditional"], "繁體中文");
        assert_eq!(table["Simplified"], "簡體中文");
        assert_eq!(table["As heard"], "照辨識結果");
    }
}
