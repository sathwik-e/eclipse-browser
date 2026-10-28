use std::ffi::{CStr, c_char};
use std::sync::Mutex;

use tao::event_loop::EventLoopProxy;

use crate::events::UserEvent;

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
}

/// `kind`: 0 = provisional navigation failed, 1 = committed navigation failed.
/// Strings are UTF-8, may be null. WebContent crashes are not reported here;
/// tabs get them from wry's web-content-process-terminate handler.
type NavigationCallback =
    extern "C" fn(tab_id: u64, kind: i32, code: i64, description: *const c_char, url: *const c_char);

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
    // UserEvent::NavigationFailed treats both kinds alike.
    _kind: i32,
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
