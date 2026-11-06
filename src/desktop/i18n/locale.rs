//! The person's preferred locales, in full (`zh-Hant-TW`).

/// The first entry of `NSLocale.preferredLanguages`, such as `zh-Hant-TW`.
#[cfg(target_os = "macos")]
pub(super) fn macos_preferred_locale() -> Option<String> {
    macos_preferred_locales().into_iter().next()
}

/// All of `NSLocale.preferredLanguages`, in the person's order.
#[cfg(target_os = "macos")]
fn macos_preferred_locales() -> Vec<String> {
    use objc2::msg_send;
    use objc2::rc::Retained;
    use objc2::runtime::AnyClass;
    use objc2_foundation::{NSArray, NSString};

    let Some(class) = AnyClass::get(c"NSLocale") else {
        return Vec::new();
    };
    // SAFETY: `preferredLanguages` is a class method returning an NSArray of NSString.
    let languages: Option<Retained<NSArray<NSString>>> =
        unsafe { msg_send![class, preferredLanguages] };
    languages
        .map(|languages| {
            languages
                .iter()
                .map(|language| language.to_string())
                .collect()
        })
        .unwrap_or_default()
}

/// The person's preferred locales in order, in full (`zh-Hant-TW`). macOS
/// gives its whole language list; Linux and Windows give only the first
/// locale (`LC_ALL`/`LC_MESSAGES`/`LANG`, the Windows user locale), because
/// neither has its full list read yet (`platform::DIFFERENCES`).
pub(crate) fn preferred_locales() -> Vec<String> {
    #[cfg(target_os = "macos")]
    {
        let locales = macos_preferred_locales();
        if !locales.is_empty() {
            return locales;
        }
    }
    vec![super::system_locale()]
}
