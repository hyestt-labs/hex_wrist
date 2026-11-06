//! The Chinese characters setting (`chinese_output`): its live projection
//! and the one-time read of Wristper's `output.json`.

use std::path::Path;

use super::AppSettings;
use crate::dictation::chinese_output::ChineseOutput;

/// Wristper's Chinese script was read into `chinese_output` once. Left out
/// of `MIGRATIONS` so new settings read it too.
pub(super) const WRISTPER_IMPORT_MIGRATION: &str = "wristper-chinese-script";

impl AppSettings {
    /// What dictation converts to: the saved choice, or the system's when
    /// none is saved.
    pub fn resolved_chinese_output(&self) -> ChineseOutput {
        crate::dictation::chinese_output::resolve(
            self.chinese_output,
            &crate::desktop::i18n::preferred_locales(),
        )
    }

    /// Publishes the resolved choice to dictation, and warms the tables it
    /// needs.
    pub(super) fn apply_chinese_output(&self) {
        let output = self.resolved_chinese_output();
        if crate::dictation::chinese_output::live() != output {
            crate::dictation::chinese_output::preload(output);
        }
        crate::dictation::chinese_output::set_live(output);
    }

    /// Reads the Chinese script Wristper saved in `<dir>/output.json`
    /// (`traditional`, `simplified`, or `as-transcribed`) into an unchosen
    /// `chinese_output`, then records the migration so it never runs again.
    /// Wristper's own folder is the Hex folder on a Mac, so the file is read
    /// where it is and never written. A choice already made is kept, and an
    /// absent or unreadable file leaves the setting for the system to decide.
    pub(super) fn import_wristper_chinese_script(&mut self, dir: &Path) {
        if self.has_migration(WRISTPER_IMPORT_MIGRATION) {
            return;
        }
        self.migrations.push(WRISTPER_IMPORT_MIGRATION.into());
        if self.chinese_output.is_some() {
            return;
        }
        let Ok(bytes) = std::fs::read(dir.join("output.json")) else {
            return;
        };
        self.chinese_output = wristper_chinese_script(&bytes);
    }
}

fn wristper_chinese_script(bytes: &[u8]) -> Option<ChineseOutput> {
    let value: serde_json::Value = serde_json::from_slice(bytes).ok()?;
    match value.get("chinese_script")?.as_str()? {
        "traditional" => Some(ChineseOutput::Traditional),
        "simplified" => Some(ChineseOutput::Simplified),
        "as-transcribed" => Some(ChineseOutput::AsHeard),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dictation::chinese_output::ChineseOutput::{AsHeard, Simplified, Traditional};
    use std::sync::atomic::{AtomicU64, Ordering};

    struct Folder(std::path::PathBuf);

    impl Folder {
        fn with_output(contents: Option<&str>) -> Self {
            static SEQUENCE: AtomicU64 = AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "hex-wristper-import-{}-{}",
                std::process::id(),
                SEQUENCE.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir_all(&path).unwrap();
            if let Some(contents) = contents {
                std::fs::write(path.join("output.json"), contents).unwrap();
            }
            Self(path)
        }
    }

    impl Drop for Folder {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn wristper_chinese_script_is_imported_once() {
        for (script, expected) in [
            ("traditional", Traditional),
            ("simplified", Simplified),
            ("as-transcribed", AsHeard),
        ] {
            let folder = Folder::with_output(Some(&format!(
                r#"{{"chinese_script":"{script}","other":1}}"#
            )));
            let mut settings = AppSettings::default();
            settings.import_wristper_chinese_script(&folder.0);
            assert_eq!(settings.chinese_output, Some(expected), "{script}");
            assert!(settings.has_migration(WRISTPER_IMPORT_MIGRATION));
            // The person changes it back to following the system; a second
            // load does not bring Wristper's value back.
            settings.chinese_output = None;
            settings.import_wristper_chinese_script(&folder.0);
            assert_eq!(settings.chinese_output, None, "{script}");
        }
    }

    #[test]
    fn wristper_import_keeps_a_choice_already_made_and_never_writes_the_file() {
        let contents = r#"{"chinese_script":"traditional"}"#;
        let folder = Folder::with_output(Some(contents));
        let mut settings = AppSettings {
            chinese_output: Some(Simplified),
            ..Default::default()
        };
        settings.import_wristper_chinese_script(&folder.0);
        assert_eq!(settings.chinese_output, Some(Simplified));
        assert!(settings.has_migration(WRISTPER_IMPORT_MIGRATION));
        assert_eq!(
            std::fs::read_to_string(folder.0.join("output.json")).unwrap(),
            contents
        );
    }

    #[test]
    fn wristper_import_without_a_usable_file_leaves_the_system_to_decide() {
        for contents in [
            None,
            Some("not json"),
            Some("{}"),
            Some(r#"{"chinese_script":"klingon"}"#),
            Some(r#"{"chinese_script":3}"#),
        ] {
            let folder = Folder::with_output(contents);
            let mut settings = AppSettings::default();
            settings.import_wristper_chinese_script(&folder.0);
            assert_eq!(settings.chinese_output, None, "{contents:?}");
            assert!(settings.has_migration(WRISTPER_IMPORT_MIGRATION));
        }
    }

    #[test]
    fn changing_chinese_output_applies_to_the_next_dictation() {
        use crate::dictation::chinese_output::apply;
        let mut settings = AppSettings {
            chinese_output: Some(Simplified),
            ..Default::default()
        };
        settings.apply_chinese_output();
        assert_eq!(apply("頭髮", "zh"), "头发");
        settings.chinese_output = Some(Traditional);
        settings.apply_chinese_output();
        assert_eq!(apply("头发", "zh"), "頭髮");
        settings.chinese_output = Some(AsHeard);
        settings.apply_chinese_output();
        assert_eq!(apply("头发", "zh"), "头发");
    }
}
