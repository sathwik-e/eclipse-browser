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

/// Policy for the internal pages. They are self-contained (inline scripts and
/// styles, no network fetches); images may come from sites (favicons) or be
/// pasted (data:/blob:). Nothing may frame them, so a website cannot overlay
/// eclipse://settings for clickjacking.
const INTERNAL_PAGE_CSP: &str = "default-src 'none'; script-src 'unsafe-inline'; \
     style-src 'unsafe-inline'; img-src 'self' eclipse: http: https: data: blob:; \
     font-src 'self' data:; media-src 'self' data: blob:; connect-src 'none'; \
     frame-src 'none'; object-src 'none'; base-uri 'none'; form-action 'none'; \
     frame-ancestors 'none'";

/// The sidebar is not served here: it is loaded with `with_html` in its own
/// webview, never in a tab.
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

    let page: &'static str = match route {
        "icon" => {
            return wry::http::Response::builder()
                .header("content-type", "image/png")
                .header("cache-control", "public, max-age=86400")
                .header("x-content-type-options", "nosniff")
                .body(Cow::Borrowed(ECLIPSE_ICON_PNG))
                .unwrap();
        }
        "settings" => SETTINGS_HTML,
        "notes" => NOTES_HTML,
        "easel" => EASEL_HTML,
        "newtab" | "" => NEWTAB_HTML,
        _ => {
            return wry::http::Response::builder()
                .status(404)
                .header("content-type", "text/plain; charset=utf-8")
                .header("cache-control", "no-store, max-age=0")
                .header("x-content-type-options", "nosniff")
                .body(Cow::Borrowed(&b"Not found"[..]))
                .unwrap();
        }
    };

    wry::http::Response::builder()
        .header("content-type", "text/html; charset=utf-8")
        .header("cache-control", "no-store, max-age=0")
        .header("x-content-type-options", "nosniff")
        .header("content-security-policy", INTERNAL_PAGE_CSP)
        .header("x-frame-options", "DENY")
        .header("referrer-policy", "no-referrer")
        .body(Cow::Borrowed(page.as_bytes()))
        .unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn get(uri: &str) -> wry::http::Response<Cow<'static, [u8]>> {
        let req = wry::http::Request::builder()
            .uri(uri)
            .body(Vec::new())
            .unwrap();
        handle_eclipse_protocol(req)
    }

    #[test]
    fn serves_only_known_routes() {
        for uri in [
            "eclipse://localhost/newtab",
            "eclipse://localhost/settings",
            "eclipse://localhost/notes",
            "eclipse://localhost/easel",
            "eclipse://localhost/",
            "eclipse://newtab",
        ] {
            let res = get(uri);
            assert_eq!(res.status(), 200, "{uri}");
            assert!(res.headers().get("access-control-allow-origin").is_none());
            let csp = res.headers()["content-security-policy"].to_str().unwrap();
            assert!(csp.contains("frame-ancestors 'none'"));
        }
        assert_eq!(get("eclipse://localhost/icon").status(), 200);
        assert_eq!(get("eclipse://localhost/sidebar").status(), 404);
        assert_eq!(get("eclipse://localhost/../../etc/passwd").status(), 404);
        assert_eq!(get("eclipse://localhost/whatever").status(), 404);
    }
}
