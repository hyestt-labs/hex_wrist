//! The Apple Watch settings (Wristper fork): the status capsule, where it
//! shows, whether a watch dictation stays on the clipboard, and the extra
//! terminal apps. The service itself has no switch.

use super::*;

/// The most extra terminal bundle ids `watch.terminal_apps` keeps.
const MAX_TERMINAL_APPS: usize = 32;
/// The longest bundle id `watch.terminal_apps` accepts.
const MAX_TERMINAL_APP_LEN: usize = 255;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
pub struct WatchSettings {
    /// Show the status capsule while a watch records, transcribes, or pastes.
    pub status_capsule: bool,
    /// The capsule shows on the screen the pointer is on, not the main one.
    pub capsule_follows_pointer: bool,
    /// A watch dictation stays on the clipboard after it pastes
    /// (Labs › Keep dictation on clipboard does the same for every dictation).
    pub keep_on_clipboard: bool,
    /// Bundle ids that count as terminals besides the built-in list: a paste
    /// there is final, since its text can't be read back. Always written, as
    /// `[]` when empty.
    #[serde(deserialize_with = "deserialize_terminal_apps")]
    pub terminal_apps: Vec<String>,
}

impl Default for WatchSettings {
    fn default() -> Self {
        Self {
            status_capsule: true,
            capsule_follows_pointer: false,
            keep_on_clipboard: false,
            terminal_apps: Vec::new(),
        }
    }
}

/// Why `apps` can't be `watch.terminal_apps`, if it can't.
fn validate_terminal_apps(apps: &[String]) -> Result<(), String> {
    if apps.len() > MAX_TERMINAL_APPS {
        return Err(format!(
            "at most {MAX_TERMINAL_APPS} bundle ids, not {}",
            apps.len()
        ));
    }
    for app in apps {
        if app.trim().is_empty() {
            return Err("a bundle id can't be empty".into());
        }
        if app.chars().count() > MAX_TERMINAL_APP_LEN {
            return Err(format!(
                "a bundle id has at most {MAX_TERMINAL_APP_LEN} characters"
            ));
        }
    }
    Ok(())
}

// A bad list fails the field, so loading drops it alone (`persistence::decode`)
// and `hex settings set` refuses it with the same reason.
fn deserialize_terminal_apps<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Vec<String>, D::Error> {
    let apps = Vec::<String>::deserialize(deserializer)?;
    validate_terminal_apps(&apps).map_err(serde::de::Error::custom)?;
    Ok(apps)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_show_the_capsule_and_keep_nothing() {
        let defaults = WatchSettings::default();
        assert!(defaults.status_capsule);
        assert!(!defaults.capsule_follows_pointer);
        assert!(!defaults.keep_on_clipboard);
        let written = serde_json::to_value(&defaults).unwrap();
        // An empty list is written, never left out or null.
        assert_eq!(written["terminal_apps"], serde_json::json!([]));
    }

    #[test]
    fn terminal_apps_rejects_bad_entries() {
        let mut too_many = vec!["a.b".to_owned(); MAX_TERMINAL_APPS + 1];
        assert!(validate_terminal_apps(&too_many).is_err());
        too_many.truncate(MAX_TERMINAL_APPS);
        assert!(validate_terminal_apps(&too_many).is_ok());
        assert!(validate_terminal_apps(&["".to_owned()]).is_err());
        assert!(validate_terminal_apps(&["  ".to_owned()]).is_err());
        assert!(validate_terminal_apps(&["x".repeat(MAX_TERMINAL_APP_LEN)]).is_ok());
        assert!(validate_terminal_apps(&["x".repeat(MAX_TERMINAL_APP_LEN + 1)]).is_err());
        // The field's deserializer holds the same rule, so a file with a bad
        // list fails that field alone.
        assert!(serde_json::from_str::<WatchSettings>(r#"{"terminal_apps":[""]}"#).is_err());
        assert!(serde_json::from_str::<WatchSettings>(r#"{"terminal_apps":["a.b"]}"#).is_ok());
    }
}
