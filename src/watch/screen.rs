//! The macOS `Screen`: Accessibility for the focused field, `keyboard` for
//! the keys. Every read is bounded by `accessibility`'s messaging timeout.

use super::desktop::Screen;
use super::field::{FieldSnapshot, Focused, Rect};

pub(crate) struct MacScreen;

fn rect(frame: [f64; 4]) -> Rect {
    Rect {
        x: frame[0],
        y: frame[1],
        width: frame[2],
        height: frame[3],
    }
}

/// The bundle identifier and name of the process that owns the field.
fn application(pid: i32) -> (Option<String>, Option<String>) {
    use objc2::rc::autoreleasepool;
    use objc2_app_kit::NSRunningApplication;
    autoreleasepool(
        |_| match NSRunningApplication::runningApplicationWithProcessIdentifier(pid) {
            Some(application) => (
                application.bundleIdentifier().map(|id| id.to_string()),
                application.localizedName().map(|name| name.to_string()),
            ),
            None => (None, None),
        },
    )
}

impl Screen for MacScreen {
    fn trusted(&self) -> bool {
        crate::accessibility::is_trusted()
    }

    fn secure_input(&self) -> bool {
        crate::accessibility::secure_input_enabled()
    }

    fn focused(&self) -> Option<Focused> {
        let field = crate::accessibility::focused_field()?;
        let (bundle_id, app_name) = application(field.pid);
        Some(Focused {
            editable: field.editable,
            field: FieldSnapshot {
                pid: field.pid,
                role: field.role,
                subrole: field.subrole,
                identifier: field.identifier,
                title: field.title,
                description: field.description,
                role_description: field.role_description,
                frame: field.frame.map(rect).unwrap_or_default(),
                window: field.window.map(rect),
                bundle_id,
                app_name,
            },
        })
    }

    fn value(&self, pid: i32) -> Option<String> {
        crate::accessibility::focused_value(pid)
    }

    fn press_return(&self) -> Result<(), String> {
        crate::keyboard::post_return().map_err(|error| error.to_string())
    }

    fn press_undo(&self) -> Result<(), String> {
        crate::keyboard::post_command('z').map_err(|error| error.to_string())
    }
}

/// `NSLocale.preferredLanguages`, for the suspect-text script rules.
pub(crate) fn preferred_languages() -> Vec<String> {
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
