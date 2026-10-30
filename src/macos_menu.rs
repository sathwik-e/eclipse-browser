use std::ffi::{CStr, CString, c_char};
use std::sync::Mutex;

use tao::event_loop::EventLoopProxy;

use crate::app::CommandBarSuggestion;
use crate::events::UserEvent;
use crate::tab::PolicyContext;

type MenuCallback = extern "C" fn(*const c_char);

unsafe extern "C" {
    fn eclipse_install_menu(callback: MenuCallback);
    fn eclipse_toggle_fullscreen();
    fn eclipse_is_fullscreen() -> bool;
    fn eclipse_bring_view_to_front(view_ptr: *mut std::ffi::c_void);
    fn eclipse_configure_window(window_ptr: *mut std::ffi::c_void);
    fn eclipse_style_webview_card(view_ptr: *mut std::ffi::c_void);
    fn eclipse_style_sidebar_view(view_ptr: *mut std::ffi::c_void);
    fn eclipse_enable_swipe_gestures(view_ptr: *mut std::ffi::c_void);
    fn eclipse_animate_view_frame(
        view_ptr: *mut std::ffi::c_void,
        x: f64,
        y: f64,
        width: f64,
        height: f64,
        duration: f64,
    );
    fn eclipse_install_edge_monitor(expand_cb: extern "C" fn(), collapse_cb: extern "C" fn());
    fn eclipse_set_hover_expanded(is_expanded: i32, sidebar_width: f64);
    fn eclipse_install_native_adblocker(view_ptr: *mut std::ffi::c_void, enabled: i32);
    fn eclipse_set_native_adblocker_enabled(view_ptr: *mut std::ffi::c_void, enabled: i32);
    fn eclipse_install_navigation_hooks(callback: NavigationCallback);
    fn eclipse_register_tab_view(view_ptr: *mut std::ffi::c_void, tab_id: u64);
    fn eclipse_recheck_hover();
    fn eclipse_navigation_policy_context() -> i32;
    fn eclipse_script_message_from_little_window() -> i32;
    fn eclipse_download_from_little_window() -> i32;
    fn eclipse_little_window_count_for_view(view_ptr: *mut std::ffi::c_void) -> i32;
    fn eclipse_popup_request_wants_popup() -> i32;
    fn eclipse_open_little_window(
        configuration: *mut std::ffi::c_void,
        opener: *mut std::ffi::c_void,
        width: f64,
        height: f64,
        has_position: i32,
        x: f64,
        y: f64,
    ) -> *mut std::ffi::c_void;
    fn eclipse_open_standalone_little_window(
        url: *const c_char,
        user_agent: *const c_char,
        adblock: i32,
        focus_address: i32,
    ) -> u64;
    fn eclipse_little_window_load(window_id: u64, url: *const c_char) -> i32;
    fn eclipse_show_little_command_bar(
        titles: *const *const c_char,
        urls: *const *const c_char,
        icons: *const *const c_char,
        count: usize,
        red: f64,
        green: f64,
        blue: f64,
    ) -> i32;
    fn eclipse_set_little_window_theme(red: f64, green: f64, blue: f64, dark: i32);
    fn eclipse_close_all_little_windows();
}

/// `kind`: 0 = provisional navigation failed, 1 = committed navigation failed.
/// `domain` is the NSError domain. Strings are UTF-8, may be null. Every
/// error is reported; `tab::classify_navigation_error` decides what it means.
type NavigationCallback = extern "C" fn(
    tab_id: u64,
    kind: i32,
    domain: *const c_char,
    code: i64,
    description: *const c_char,
    url: *const c_char,
);

/// Bits of `eclipse_navigation_policy_context` (keep in sync with menu.m).
const POLICY_MAIN_FRAME: i32 = 1;
const POLICY_BACK_FORWARD: i32 = 2;
const POLICY_OTHER_VIEW: i32 = 4;
const POLICY_USER_GESTURE: i32 = 8;

static MENU_PROXY: Mutex<Option<EventLoopProxy<UserEvent>>> = Mutex::new(None);
static EDGE_PROXY: Mutex<Option<EventLoopProxy<UserEvent>>> = Mutex::new(None);
static NAV_PROXY: Mutex<Option<EventLoopProxy<UserEvent>>> = Mutex::new(None);

extern "C" fn handle_menu_action(action: *const c_char) {
    if action.is_null() {
        return;
    }
    let action = unsafe { CStr::from_ptr(action) }
        .to_string_lossy()
        .into_owned();
    let proxy = MENU_PROXY
        .lock()
        .ok()
        .and_then(|guard| guard.as_ref().cloned());
    if let Some(proxy) = proxy {
        let _ = proxy.send_event(UserEvent::MenuAction(action));
    }
}

pub fn install(proxy: EventLoopProxy<UserEvent>) {
    if let Ok(mut guard) = MENU_PROXY.lock() {
        *guard = Some(proxy);
    }
    unsafe {
        eclipse_install_menu(handle_menu_action);
    }
}

extern "C" fn handle_edge_expand() {
    let proxy = EDGE_PROXY
        .lock()
        .ok()
        .and_then(|guard| guard.as_ref().cloned());
    if let Some(proxy) = proxy {
        let _ = proxy.send_event(UserEvent::HoverExpandSidebar);
    }
}

extern "C" fn handle_edge_collapse() {
    let proxy = EDGE_PROXY
        .lock()
        .ok()
        .and_then(|guard| guard.as_ref().cloned());
    if let Some(proxy) = proxy {
        let _ = proxy.send_event(UserEvent::HoverCollapseSidebar);
    }
}

pub fn install_edge_monitor(proxy: EventLoopProxy<UserEvent>) {
    if let Ok(mut guard) = EDGE_PROXY.lock() {
        *guard = Some(proxy);
    }
    unsafe {
        eclipse_install_edge_monitor(handle_edge_expand, handle_edge_collapse);
    }
}

pub fn set_hover_expanded(is_expanded: bool, sidebar_width: f64) {
    unsafe {
        eclipse_set_hover_expanded(if is_expanded { 1 } else { 0 }, sidebar_width);
    }
}

pub fn toggle_fullscreen() {
    unsafe {
        eclipse_toggle_fullscreen();
    }
}

pub fn is_fullscreen() -> bool {
    unsafe { eclipse_is_fullscreen() }
}

pub fn bring_view_to_front(view_ptr: *mut std::ffi::c_void) {
    if !view_ptr.is_null() {
        unsafe {
            eclipse_bring_view_to_front(view_ptr);
        }
    }
}

pub fn configure_window(window_ptr: *mut std::ffi::c_void) {
    if !window_ptr.is_null() {
        unsafe {
            eclipse_configure_window(window_ptr);
        }
    }
}

pub fn style_webview_card(view_ptr: *mut std::ffi::c_void) {
    if !view_ptr.is_null() {
        unsafe {
            eclipse_style_webview_card(view_ptr);
        }
    }
}

pub fn style_sidebar_view(view_ptr: *mut std::ffi::c_void) {
    if !view_ptr.is_null() {
        unsafe {
            eclipse_style_sidebar_view(view_ptr);
        }
    }
}

pub fn animate_view_frame(
    view_ptr: *mut std::ffi::c_void,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    duration: f64,
) {
    if !view_ptr.is_null() {
        unsafe {
            eclipse_animate_view_frame(view_ptr, x, y, width, height, duration);
        }
    }
}

pub fn enable_swipe_gestures(view_ptr: *mut std::ffi::c_void) {
    if !view_ptr.is_null() {
        unsafe {
            eclipse_enable_swipe_gestures(view_ptr);
        }
    }
}

pub fn install_native_adblocker(view_ptr: *mut std::ffi::c_void, enabled: bool) {
    if !view_ptr.is_null() {
        unsafe {
            eclipse_install_native_adblocker(view_ptr, if enabled { 1 } else { 0 });
        }
    }
}

pub fn set_native_adblocker_enabled(view_ptr: *mut std::ffi::c_void, enabled: bool) {
    if !view_ptr.is_null() {
        unsafe {
            eclipse_set_native_adblocker_enabled(view_ptr, if enabled { 1 } else { 0 });
        }
    }
}

fn c_string(ptr: *const c_char) -> String {
    if ptr.is_null() {
        String::new()
    } else {
        unsafe { CStr::from_ptr(ptr) }.to_string_lossy().into_owned()
    }
}

extern "C" fn handle_navigation_event(
    tab_id: u64,
    kind: i32,
    domain: *const c_char,
    code: i64,
    description: *const c_char,
    url: *const c_char,
) {
    let proxy = NAV_PROXY
        .lock()
        .ok()
        .and_then(|guard| guard.as_ref().cloned());
    let Some(proxy) = proxy else {
        return;
    };
    let _ = proxy.send_event(UserEvent::NavigationFailed {
        tab_id: tab_id as usize,
        committed: kind == 1,
        domain: c_string(domain),
        code,
        description: c_string(description),
        url: c_string(url),
    });
}

/// Hooks WebKit navigation failures (failed provisional or committed loads)
/// into the event loop.
pub fn install_navigation_hooks(proxy: EventLoopProxy<UserEvent>) {
    if let Ok(mut guard) = NAV_PROXY.lock() {
        *guard = Some(proxy);
    }
    unsafe {
        eclipse_install_navigation_hooks(handle_navigation_event);
    }
}

/// Associates a tab's native view with its id so navigation failures can be
/// reported for the right tab.
pub fn register_tab_view(view_ptr: *mut std::ffi::c_void, tab_id: usize) {
    if !view_ptr.is_null() {
        unsafe {
            eclipse_register_tab_view(view_ptr, tab_id as u64);
        }
    }
}

/// Asks the edge monitor to re-evaluate the pointer position now (for example
/// after the sidebar stops needing to stay open) and collapse if it is outside.
pub fn recheck_hover() {
    unsafe {
        eclipse_recheck_hover();
    }
}

/// What WebKit is deciding a navigation policy for while wry's navigation
/// handler runs (menu.m wraps the delegate method; outside a decision both
/// flags are false).
pub fn navigation_policy_context() -> PolicyContext {
    let bits = unsafe { eclipse_navigation_policy_context() };
    PolicyContext {
        main_frame: bits & POLICY_MAIN_FRAME != 0,
        back_forward: bits & POLICY_BACK_FORWARD != 0,
        other_view: bits & POLICY_OTHER_VIEW != 0,
        user_gesture: bits & POLICY_USER_GESTURE != 0,
    }
}

/// Whether the script message wry's IPC handler is delivering right now was
/// posted by a Little Eclipse window's webview (menu.m wraps wry's message
/// handler; outside a delivery this is false). Such a popup shares its opener
/// tab's WKUserContentController, so its messages reach the tab's handler.
pub fn script_message_from_little_window() -> bool {
    unsafe { eclipse_script_message_from_little_window() != 0 }
}

/// Whether the download wry's download handler is deciding right now came
/// from a Little Eclipse window, which handed it to a tab's webview (menu.m
/// wraps wry's download delegate for those; outside a decision this is
/// false).
pub fn download_from_little_window() -> bool {
    unsafe { eclipse_download_from_little_window() != 0 }
}

/// How many Little Eclipse windows (nested ones included) are open for the
/// tab whose webview is `view_ptr`.
pub fn little_window_count_for_view(view_ptr: *mut std::ffi::c_void) -> usize {
    let count = unsafe { eclipse_little_window_count_for_view(view_ptr) };
    usize::try_from(count).unwrap_or(0)
}

/// Whether the window.open wry's new-window handler is deciding asked for a
/// window of its own rather than a tab: a popup window by WebKit's reading
/// of the HTML rule (a `popup` feature, a size or position, or turned-off
/// toolbars), or, as in Arc, a script's `window.open` with no features whose
/// page keeps its opener. target=_blank links and `noopener` windows are
/// tabs. Outside the handler this is false.
pub fn popup_request_wants_popup() -> bool {
    unsafe { eclipse_popup_request_wants_popup() != 0 }
}

/// Opens a Little Eclipse window (menu.m) for a popup `features` describes,
/// built on the configuration WebKit handed the opener, and returns its
/// webview for `NewWindowResponse::Create`. `None` when the request came
/// without a user gesture, would exceed the popup limits, or the native
/// window could not be made; the caller then opens it as a tab. Must be
/// called from wry's new-window handler (menu.m reads the gesture state of
/// the request being handled).
pub fn open_little_window(
    features: &wry::NewWindowFeatures,
) -> Option<objc2::rc::Retained<objc2_web_kit::WKWebView>> {
    use objc2::rc::Retained;

    // A popup that gave no size gets menu.m's default size (0 x 0 here).
    let (width, height) = features
        .size
        .map_or((0.0, 0.0), |size| (size.width, size.height));
    let (has_position, x, y) = match features.position {
        Some(position) => (1, position.x, position.y),
        None => (0, 0.0, 0.0),
    };
    let configuration = Retained::as_ptr(&features.opener.target_configuration)
        .cast_mut()
        .cast::<std::ffi::c_void>();
    let opener = Retained::as_ptr(&features.opener.webview)
        .cast_mut()
        .cast::<std::ffi::c_void>();
    // Both pointers stay valid for the call: `features` holds a reference to
    // each object.
    let raw = unsafe {
        eclipse_open_little_window(
            configuration,
            opener,
            width,
            height,
            has_position,
            x,
            y,
        )
    };
    // SAFETY: menu.m returns either null or a WKWebView retained once for
    // the caller (`__bridge_retained`); `from_raw` takes over exactly that
    // reference, and dropping the `Retained` releases it.
    unsafe { Retained::from_raw(raw.cast::<objc2_web_kit::WKWebView>()) }
}

/// Opens a standalone Little Eclipse window (menu.m): no opener, the tabs'
/// website data store, `user_agent` and (if `adblock`) the native content
/// blocker, no tab scripts or IPC. `url` is an http(s) address, or empty for
/// a blank window with its address field focused (`focus_address` focuses it
/// either way). The new window's id (for `little_window_load`), or `None`
/// when no window opened (the overall limit, or `url` is not a web address);
/// the caller then opens a tab.
pub fn open_standalone_little_window(url: &str, user_agent: &str, adblock: bool, focus_address: bool) -> Option<u64> {
    let (Ok(url), Ok(user_agent)) = (CString::new(url), CString::new(user_agent)) else {
        return None;
    };
    let id = unsafe {
        eclipse_open_standalone_little_window(
            url.as_ptr(),
            user_agent.as_ptr(),
            i32::from(adblock),
            i32::from(focus_address),
        )
    };
    (id != 0).then_some(id)
}

/// Loads `url` (what the user typed in the address field of Little Eclipse
/// window `window_id`, resolved by `app::little_window_url_from_input`, or the
/// next link preview) into that window and brings it to the front; `None`
/// refuses (menu.m beeps). menu.m loads only http(s). `false` also when the
/// window has closed.
pub fn little_window_load(window_id: u64, url: Option<&str>) -> bool {
    let url = url.and_then(|url| CString::new(url).ok());
    let ptr = url.as_ref().map_or(std::ptr::null(), |url| url.as_ptr());
    unsafe { eclipse_little_window_load(window_id, ptr) != 0 }
}

/// Shows the New Little Eclipse Window bar (⌥⌘N, menu.m) with `suggestions`
/// (title, http(s) URL, icon) and `accent` for the chosen row. Choosing a
/// row or typing an address or search comes back as the menu action
/// `little_command_bar_open:<text>`. `false` when the bar could not show.
pub fn show_little_command_bar(suggestions: &[CommandBarSuggestion], accent: [u8; 3]) -> bool {
    // Titles and URLs from Rust never hold NUL; one that did is left out
    // (an icon with one is dropped).
    let rows: Vec<(CString, CString, Option<CString>)> = suggestions
        .iter()
        .filter_map(|s| {
            Some((
                CString::new(s.title.as_str()).ok()?,
                CString::new(s.url.as_str()).ok()?,
                s.icon.as_deref().and_then(|icon| CString::new(icon).ok()),
            ))
        })
        .collect();
    let titles: Vec<*const c_char> = rows.iter().map(|(title, _, _)| title.as_ptr()).collect();
    let urls: Vec<*const c_char> = rows.iter().map(|(_, url, _)| url.as_ptr()).collect();
    let icons: Vec<*const c_char> = rows
        .iter()
        .map(|(_, _, icon)| icon.as_ref().map_or(std::ptr::null(), |icon| icon.as_ptr()))
        .collect();
    let [r, g, b] = accent.map(|c| f64::from(c) / 255.0);
    // The pointers stay valid for the call: `rows` owns the strings.
    unsafe {
        eclipse_show_little_command_bar(titles.as_ptr(), urls.as_ptr(), icons.as_ptr(), titles.len(), r, g, b) != 0
    }
}

/// Frames every Little Eclipse window in the current Space's theme colour.
pub fn set_little_window_theme(rgb: [u8; 3], dark: bool) {
    let [r, g, b] = rgb.map(|c| f64::from(c) / 255.0);
    unsafe { eclipse_set_little_window_theme(r, g, b, i32::from(dark)) }
}

/// Closes every Little Eclipse window (the app is quitting).
pub fn close_all_little_windows() {
    unsafe { eclipse_close_all_little_windows() }
}
