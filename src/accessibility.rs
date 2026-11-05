use std::ffi::{CStr, c_char, c_void};
use std::ptr;

type CfTypeRef = *const c_void;
type CfStringRef = *const c_void;
type AxUiElementRef = *const c_void;

const CF_STRING_ENCODING_UTF8: u32 = 0x0800_0100;
const MESSAGING_TIMEOUT_SECS: f32 = 0.25;

#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn AXUIElementCreateApplication(pid: i32) -> AxUiElementRef;
    fn AXUIElementCopyAttributeValue(
        element: AxUiElementRef,
        attribute: CfStringRef,
        value: *mut CfTypeRef,
    ) -> i32;
    fn AXUIElementSetMessagingTimeout(element: AxUiElementRef, timeout_in_seconds: f32) -> i32;
    fn AXUIElementSetAttributeValue(
        element: AxUiElementRef,
        attribute: CfStringRef,
        value: CfTypeRef,
    ) -> i32;
    fn AXUIElementCopyParameterizedAttributeValue(
        element: AxUiElementRef,
        parameterized_attribute: CfStringRef,
        parameter: CfTypeRef,
        result: *mut CfTypeRef,
    ) -> i32;
    fn AXValueGetTypeID() -> usize;
    fn AXValueCreate(value_type: u32, value: *const c_void) -> CfTypeRef;
    fn AXValueGetValue(value: CfTypeRef, value_type: u32, value_out: *mut c_void) -> bool;
}

/// `kAXValueTypeCFRange`.
const AX_VALUE_CF_RANGE: u32 = 4;

/// Core Foundation's `CFRange`, in UTF-16 code units.
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct CfRange {
    location: isize,
    length: isize,
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFGetTypeID(value: CfTypeRef) -> usize;
    fn CFStringGetTypeID() -> usize;
    fn CFStringGetLength(value: CfStringRef) -> isize;
    fn CFStringGetMaximumSizeForEncoding(length: isize, encoding: u32) -> isize;
    fn CFStringGetCString(
        value: CfStringRef,
        buffer: *mut c_char,
        buffer_size: isize,
        encoding: u32,
    ) -> bool;
    fn CFStringCreateWithCString(
        allocator: *const c_void,
        string: *const c_char,
        encoding: u32,
    ) -> CfStringRef;
    fn CFURLGetTypeID() -> usize;
    fn CFURLGetString(url: CfTypeRef) -> CfStringRef;
    fn CFRelease(value: CfTypeRef);
    fn CFRetain(value: CfTypeRef) -> CfTypeRef;
    fn CFEqual(left: CfTypeRef, right: CfTypeRef) -> u8;
    fn CFArrayGetTypeID() -> usize;
    fn CFArrayGetCount(array: CfTypeRef) -> isize;
    fn CFArrayGetValueAtIndex(array: CfTypeRef, index: isize) -> CfTypeRef;
    static kCFBooleanTrue: CfTypeRef;
}

pub fn focused_window_title(pid: i32) -> Option<String> {
    // SAFETY: AXUIElementCreateApplication returns a retained Core Foundation object.
    let application = bounded_element(unsafe { AXUIElementCreateApplication(pid) })?;
    let window = copy_element(&application, c"AXFocusedWindow")?;
    copy_string(&window, c"AXTitle").filter(|title| !title.is_empty())
}

/// The page address of the web area holding the application's focused
/// element, for browsers without a scripting dictionary (Firefox, Zen).
/// The outermost web area wins, so focus inside an iframe still reports
/// the page itself.
pub fn focused_web_url(pid: i32) -> Option<String> {
    const MAX_DEPTH: usize = 64;
    // SAFETY: AXUIElementCreateApplication returns a retained Core Foundation object.
    let application = bounded_element(unsafe { AXUIElementCreateApplication(pid) })?;
    let mut element = copy_element(&application, c"AXFocusedUIElement")?;
    let mut url = None;
    for _ in 0..MAX_DEPTH {
        if copy_string(&element, c"AXRole").as_deref() == Some("AXWebArea") {
            url = copy_url(&element, c"AXURL").or(url);
        }
        match copy_element(&element, c"AXParent") {
            Some(parent) => element = parent,
            None => break,
        }
    }
    url
}

/// Every page a browser's windows show, for a call's address or tab title
/// when its scripting dictionary can't be asked (#831). Chromium (Arc,
/// Chrome, Brave, Edge) builds its web accessibility tree only for an
/// assistive app that asks, which `AXManualAccessibility` does, once per
/// process; without it a browser behind another app shows windows but no
/// web areas.
pub fn browser_pages(pid: i32) -> crate::dictation::browser_page::BrowserPages {
    // SAFETY: AXUIElementCreateApplication returns a retained Core Foundation object.
    let Some(application) = bounded_element(unsafe { AXUIElementCreateApplication(pid) }) else {
        return Default::default();
    };
    request_web_accessibility(pid, &application);
    let mut windows: Vec<CfObject> = Vec::new();
    let candidates = [
        copy_element(&application, c"AXFocusedWindow"),
        copy_element(&application, c"AXMainWindow"),
    ]
    .into_iter()
    .flatten()
    .chain(copy_elements(&application, c"AXWindows"));
    for window in candidates {
        // SAFETY: Both are live AXUIElements.
        if !windows
            .iter()
            .any(|known| unsafe { CFEqual(known.0, window.0) } != 0)
        {
            windows.push(window);
        }
    }
    let mut pages = crate::dictation::browser_page::pages(windows.into_iter().map(Element));
    // The focused web area, when there is one, is the likeliest page.
    if let Some(url) = focused_web_url(pid) {
        pages.urls.retain(|known| known != &url);
        pages.urls.insert(0, url);
    }
    pages
}

/// Asks a Chromium browser for its web accessibility tree, once per
/// process. Other apps ignore the attribute.
fn request_web_accessibility(pid: i32, application: &CfObject) {
    static ASKED: std::sync::Mutex<Vec<i32>> = std::sync::Mutex::new(Vec::new());
    let mut asked = ASKED.lock().unwrap_or_else(|error| error.into_inner());
    if asked.contains(&pid) {
        return;
    }
    asked.push(pid);
    let Some(attribute) = cf_string_literal(c"AXManualAccessibility").map(CfObject) else {
        return;
    };
    // SAFETY: A live application element, a CFString name, and the shared
    // kCFBooleanTrue constant.
    let status =
        unsafe { AXUIElementSetAttributeValue(application.0, attribute.0, kCFBooleanTrue) };
    if status != 0 {
        tracing::debug!(
            status,
            pid,
            "the browser has no manual accessibility switch"
        );
    }
}

/// An element as `browser_page` walks it.
struct Element(CfObject);

impl crate::dictation::browser_page::AxNode for Element {
    fn role(&self) -> Option<String> {
        copy_string(&self.0, c"AXRole")
    }

    fn title(&self) -> Option<String> {
        copy_string(&self.0, c"AXTitle")
    }

    fn url(&self) -> Option<String> {
        copy_url(&self.0, c"AXURL")
    }

    fn children(&self) -> Vec<Self> {
        copy_elements(&self.0, c"AXChildren")
            .into_iter()
            .map(Element)
            .collect()
    }
}

/// An attribute holding elements (`AXWindows`, `AXChildren`), each bounded.
fn copy_elements(element: &CfObject, name: &CStr) -> Vec<CfObject> {
    let Some(array) = copy_attribute(element, name) else {
        return Vec::new();
    };
    if unsafe { CFGetTypeID(array.0) } != unsafe { CFArrayGetTypeID() } {
        return Vec::new();
    }
    let count = unsafe { CFArrayGetCount(array.0) };
    (0..count)
        .filter_map(|index| {
            // SAFETY: Get rule: the array owns the item, so it is retained
            // before the array goes.
            let item = unsafe { CFArrayGetValueAtIndex(array.0, index) };
            if item.is_null() {
                return None;
            }
            let item = CfObject(unsafe { CFRetain(item) });
            (unsafe { AXUIElementSetMessagingTimeout(item.0, MESSAGING_TIMEOUT_SECS) } == 0)
                .then_some(item)
        })
        .collect()
}

/// `kAXValueTypeCGPoint` and `kAXValueTypeCGSize`.
const AX_VALUE_CG_POINT: u32 = 1;
const AX_VALUE_CG_SIZE: u32 = 2;

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct CgPair(f64, f64);

#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    safe fn AXIsProcessTrusted() -> bool;
    fn AXUIElementGetPid(element: AxUiElementRef, pid: *mut i32) -> i32;
    fn AXUIElementIsAttributeSettable(
        element: AxUiElementRef,
        attribute: CfStringRef,
        settable: *mut bool,
    ) -> i32;
}

#[link(name = "Carbon", kind = "framework")]
unsafe extern "C" {
    safe fn IsSecureEventInputEnabled() -> bool;
}

/// Whether Hex may use Accessibility. Asks without prompting.
pub fn is_trusted() -> bool {
    AXIsProcessTrusted()
}

/// Secure keyboard entry is on (a password prompt, or a terminal's Secure
/// Keyboard Entry): synthetic keys reach no app.
pub fn secure_input_enabled() -> bool {
    IsSecureEventInputEnabled()
}

/// The element with keyboard focus, as a paste needs to know it. Frames are
/// `[x, y, width, height]` in points.
pub struct FocusedField {
    pub pid: i32,
    pub role: String,
    pub subrole: String,
    pub identifier: String,
    pub title: String,
    pub description: String,
    pub role_description: String,
    pub frame: Option<[f64; 4]>,
    pub window: Option<[f64; 4]>,
    /// The value is settable, or the role is a text input.
    pub editable: bool,
    /// The element itself, to read its value later.
    pub element: FieldElement,
}

/// A retained Accessibility element. Core Foundation objects may be used
/// from any thread; each read is bounded by the element's messaging timeout.
pub struct FieldElement(CfObject);

// SAFETY: A retained CFType; AXUIElement reads are thread-safe.
unsafe impl Send for FieldElement {}
unsafe impl Sync for FieldElement {}

impl FieldElement {
    /// The element's text value.
    pub fn value(&self) -> Option<String> {
        copy_string(&self.0, c"AXValue")
    }
}

/// The frontmost application's focused element. Every element is bounded
/// by the messaging timeout, and the system-wide element is never used: its
/// timeout is the process-wide default, and a hung app would stall every
/// other Accessibility read with it.
pub fn focused_field() -> Option<FocusedField> {
    let pid = crate::dictation::context::frontmost_pid()?;
    // SAFETY: AXUIElementCreateApplication returns a retained Core Foundation object.
    let application = bounded_element(unsafe { AXUIElementCreateApplication(pid) })?;
    let element = copy_element(&application, c"AXFocusedUIElement")?;
    let mut owner = 0;
    // SAFETY: A live element and an out parameter.
    if unsafe { AXUIElementGetPid(element.0, &mut owner) } != 0 {
        owner = pid;
    }
    // A role that cannot be read means the app is not answering: stop here
    // rather than spend a timeout on each remaining attribute.
    let role = copy_string(&element, c"AXRole")?;
    let window = copy_element(&element, c"AXWindow").and_then(|window| frame_of(&window));
    Some(FocusedField {
        pid: owner,
        subrole: copy_string(&element, c"AXSubrole").unwrap_or_default(),
        identifier: copy_string(&element, c"AXIdentifier").unwrap_or_default(),
        title: copy_string(&element, c"AXTitle").unwrap_or_default(),
        description: copy_string(&element, c"AXDescription").unwrap_or_default(),
        role_description: copy_string(&element, c"AXRoleDescription").unwrap_or_default(),
        frame: frame_of(&element),
        window,
        editable: is_editable(&element, &role),
        role,
        element: FieldElement(element),
    })
}

fn is_editable(element: &CfObject, role: &str) -> bool {
    const INPUT_ROLES: [&str; 5] = [
        "AXTextField",
        "AXTextArea",
        "AXComboBox",
        "AXSearchField",
        "AXCell",
    ];
    if INPUT_ROLES.contains(&role) {
        return true;
    }
    let Some(attribute) = cf_string_literal(c"AXValue").map(CfObject) else {
        return false;
    };
    let mut settable = false;
    // SAFETY: A live element, a CFString name, and an out parameter.
    let status = unsafe { AXUIElementIsAttributeSettable(element.0, attribute.0, &mut settable) };
    status == 0 && settable
}

fn frame_of(element: &CfObject) -> Option<[f64; 4]> {
    let position = copy_pair(element, c"AXPosition", AX_VALUE_CG_POINT)?;
    let size = copy_pair(element, c"AXSize", AX_VALUE_CG_SIZE)?;
    Some([position.0, position.1, size.0, size.1])
}

fn copy_pair(element: &CfObject, name: &CStr, value_type: u32) -> Option<CgPair> {
    let value = copy_attribute(element, name)?;
    if unsafe { CFGetTypeID(value.0) } != unsafe { AXValueGetTypeID() } {
        return None;
    }
    let mut pair = CgPair::default();
    // SAFETY: `value` is an AXValue of this type; CGPoint and CGSize are two doubles.
    unsafe { AXValueGetValue(value.0, value_type, (&mut pair as *mut CgPair).cast()) }
        .then_some(pair)
}

/// The character just before the insertion point of the application's
/// focused text element, for Labs › Smart spacing. `None` when text is
/// selected, the cursor is at the start, or the app doesn't say.
pub fn char_before_caret(pid: i32) -> Option<char> {
    // SAFETY: AXUIElementCreateApplication returns a retained Core Foundation object.
    let application = bounded_element(unsafe { AXUIElementCreateApplication(pid) })?;
    let focused = copy_element(&application, c"AXFocusedUIElement")?;
    let selection = copy_range(&focused, c"AXSelectedTextRange")?;
    if selection.length != 0 || selection.location <= 0 {
        return None;
    }
    // One UTF-16 unit, or two when it is half of a surrogate pair.
    (1..=2.min(selection.location)).find_map(|units| {
        let range = CfRange {
            location: selection.location - units,
            length: units,
        };
        copy_string_for_range(&focused, range)?.chars().next_back()
    })
}

fn copy_range(element: &CfObject, name: &CStr) -> Option<CfRange> {
    let value = copy_attribute(element, name)?;
    if unsafe { CFGetTypeID(value.0) } != unsafe { AXValueGetTypeID() } {
        return None;
    }
    let mut range = CfRange::default();
    // SAFETY: `value` is an AXValue and `range` has CFRange's layout.
    unsafe {
        AXValueGetValue(
            value.0,
            AX_VALUE_CF_RANGE,
            (&mut range as *mut CfRange).cast(),
        )
    }
    .then_some(range)
}

fn copy_string_for_range(element: &CfObject, range: CfRange) -> Option<String> {
    // SAFETY: AXValueCreate copies the CFRange and returns a retained object.
    let parameter = unsafe { AXValueCreate(AX_VALUE_CF_RANGE, (&range as *const CfRange).cast()) };
    if parameter.is_null() {
        return None;
    }
    let parameter = CfObject(parameter);
    let attribute = CfObject(cf_string_literal(c"AXStringForRange")?);
    let mut value = ptr::null();
    let status = unsafe {
        AXUIElementCopyParameterizedAttributeValue(element.0, attribute.0, parameter.0, &mut value)
    };
    if status != 0 || value.is_null() {
        return None;
    }
    let value = CfObject(value);
    cf_string(value.0).filter(|text| !text.is_empty())
}

struct CfObject(CfTypeRef);

impl Drop for CfObject {
    fn drop(&mut self) {
        unsafe { CFRelease(self.0) };
    }
}

fn bounded_element(element: AxUiElementRef) -> Option<CfObject> {
    if element.is_null() {
        return None;
    }
    let element = CfObject(element);
    (unsafe { AXUIElementSetMessagingTimeout(element.0, MESSAGING_TIMEOUT_SECS) } == 0)
        .then_some(element)
}

fn copy_attribute(element: &CfObject, name: &CStr) -> Option<CfObject> {
    let attribute = CfObject(cf_string_literal(name)?);
    let mut value = ptr::null();
    let status = unsafe { AXUIElementCopyAttributeValue(element.0, attribute.0, &mut value) };
    // `then`, not `then_some`: dropping a `CfObject(null)` would CFRelease(NULL).
    (status == 0 && !value.is_null()).then(|| CfObject(value))
}

fn copy_element(element: &CfObject, name: &CStr) -> Option<CfObject> {
    let child = copy_attribute(element, name)?;
    (unsafe { AXUIElementSetMessagingTimeout(child.0, MESSAGING_TIMEOUT_SECS) } == 0)
        .then_some(child)
}

fn copy_string(element: &CfObject, name: &CStr) -> Option<String> {
    let value = copy_attribute(element, name)?;
    cf_string(value.0)
}

fn copy_url(element: &CfObject, name: &CStr) -> Option<String> {
    let value = copy_attribute(element, name)?;
    if unsafe { CFGetTypeID(value.0) } != unsafe { CFURLGetTypeID() } {
        return cf_string(value.0);
    }
    // SAFETY: CFURLGetString follows the Get rule; `value` keeps it alive.
    let string = unsafe { CFURLGetString(value.0) };
    (!string.is_null()).then(|| cf_string(string)).flatten()
}

fn cf_string_literal(value: &CStr) -> Option<CfStringRef> {
    let string =
        unsafe { CFStringCreateWithCString(ptr::null(), value.as_ptr(), CF_STRING_ENCODING_UTF8) };
    (!string.is_null()).then_some(string)
}

fn cf_string(value: CfTypeRef) -> Option<String> {
    if unsafe { CFGetTypeID(value) } != unsafe { CFStringGetTypeID() } {
        return None;
    }
    let value = value.cast();
    let length = unsafe { CFStringGetLength(value) };
    let capacity = unsafe { CFStringGetMaximumSizeForEncoding(length, CF_STRING_ENCODING_UTF8) }
        .saturating_add(1);
    let mut bytes = vec![0_u8; capacity as usize];
    if !unsafe {
        CFStringGetCString(
            value,
            bytes.as_mut_ptr().cast(),
            capacity,
            CF_STRING_ENCODING_UTF8,
        )
    } {
        return None;
    }
    let length = bytes
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(bytes.len());
    bytes.truncate(length);
    String::from_utf8(bytes).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_attribute_is_none() {
        assert_eq!(focused_window_title(i32::MAX), None);
        assert_eq!(focused_web_url(i32::MAX), None);
        assert_eq!(char_before_caret(i32::MAX), None);
    }
}
