use std::borrow::Cow;

pub const SIDEBAR_HTML: &str = include_str!("../assets/sidebar.html");
pub const NEWTAB_HTML: &str = include_str!("../assets/newtab.html");
pub const SETTINGS_HTML: &str = include_str!("../assets/settings.html");
pub const NOTES_HTML: &str = include_str!("../assets/notes.html");
pub const EASEL_HTML: &str = include_str!("../assets/easel.html");

/// Runtime brand mark (a 256 px PNG derived from the supplied artwork).
/// Keeping the small copy embedded costs ~40 KB and makes the sidebar, new-tab
/// page, settings page, and internal tab icons share one source of truth.
pub const ECLIPSE_ICON_PNG: &[u8] = include_bytes!("../assets/eclipse-icon-256.png");

pub fn handle_eclipse_protocol(
    req: wry::http::Request<Vec<u8>>,
) -> wry::http::Response<Cow<'static, [u8]>> {
    let host = req.uri().host().unwrap_or("");
    let path = req.uri().path().trim_matches('/');
    let route = if !path.is_empty() && path != "localhost" {
        path
    } else if !host.is_empty() && host != "localhost" {
        host
    } else {
        path
    };

    let (body, content_type): (&'static [u8], &'static str) = match route {
        "icon" => (ECLIPSE_ICON_PNG, "image/png"),
        "sidebar" => (SIDEBAR_HTML.as_bytes(), "text/html; charset=utf-8"),
        "settings" => (SETTINGS_HTML.as_bytes(), "text/html; charset=utf-8"),
        "notes" => (NOTES_HTML.as_bytes(), "text/html; charset=utf-8"),
        "easel" => (EASEL_HTML.as_bytes(), "text/html; charset=utf-8"),
        "newtab" | "" => (NEWTAB_HTML.as_bytes(), "text/html; charset=utf-8"),
        _ => (NEWTAB_HTML.as_bytes(), "text/html; charset=utf-8"),
    };

    let cache_control = if route == "icon" {
        "public, max-age=86400"
    } else {
        "no-store, max-age=0"
    };

    wry::http::Response::builder()
        .header("content-type", content_type)
        .header("cache-control", cache_control)
        .header("x-content-type-options", "nosniff")
        .header("access-control-allow-origin", "*")
        .body(Cow::Borrowed(body))
        .unwrap()
}
