use std::borrow::Cow;

pub const SIDEBAR_HTML: &str = include_str!("../assets/sidebar.html");
pub const NEWTAB_HTML: &str = include_str!("../assets/newtab.html");
pub const SETTINGS_HTML: &str = include_str!("../assets/settings.html");
pub const NOTES_HTML: &str = include_str!("../assets/notes.html");
pub const EASEL_HTML: &str = include_str!("../assets/easel.html");
pub const HISTORY_HTML: &str = include_str!("../assets/history.html");

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

/// The internal pages a tab can show.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InternalPage {
    NewTab,
    Settings,
    Notes,
    Easel,
    History,
}

impl InternalPage {
    fn from_route(route: &str) -> Option<Self> {
        match route {
            "settings" => Some(Self::Settings),
            "notes" => Some(Self::Notes),
            "easel" => Some(Self::Easel),
            "history" => Some(Self::History),
            "newtab" | "" => Some(Self::NewTab),
            _ => None,
        }
    }

    /// The page an eclipse:// URL serves (as `handle_eclipse_protocol` routes
    /// it), or None for anything else (other schemes, the icon, a 404).
    pub fn from_url(url: &str) -> Option<Self> {
        let parsed = url::Url::parse(url).ok()?;
        if parsed.scheme() != "eclipse" {
            return None;
        }
        Self::from_route(route(parsed.host_str().unwrap_or(""), parsed.path()))
    }

    fn html(self) -> &'static str {
        match self {
            Self::NewTab => NEWTAB_HTML,
            Self::Settings => SETTINGS_HTML,
            Self::Notes => NOTES_HTML,
            Self::Easel => EASEL_HTML,
            Self::History => HISTORY_HTML,
        }
    }
}

/// Which resource an eclipse:// request names: `eclipse://localhost/notes`
/// and `eclipse://notes` are the same page.
fn route<'a>(host: &'a str, path: &'a str) -> &'a str {
    let path = path.trim_matches('/');
    if !path.is_empty() && path != "localhost" {
        path
    } else if !host.is_empty() && host != "localhost" {
        host
    } else {
        path
    }
}

/// The sidebar is not served here: it is loaded with `with_html` in its own
/// webview, never in a tab.
pub fn handle_eclipse_protocol(
    req: wry::http::Request<Vec<u8>>,
) -> wry::http::Response<Cow<'static, [u8]>> {
    let route = route(req.uri().host().unwrap_or(""), req.uri().path());

    let page: &'static str = match route {
        "icon" => {
            return wry::http::Response::builder()
                .header("content-type", "image/png")
                .header("cache-control", "public, max-age=86400")
                .header("x-content-type-options", "nosniff")
                .body(Cow::Borrowed(ECLIPSE_ICON_PNG))
                .unwrap();
        }
        other => match InternalPage::from_route(other) {
            Some(page) => page.html(),
            None => {
                return wry::http::Response::builder()
                    .status(404)
                    .header("content-type", "text/plain; charset=utf-8")
                    .header("cache-control", "no-store, max-age=0")
                    .header("x-content-type-options", "nosniff")
                    .body(Cow::Borrowed(&b"Not found"[..]))
                    .unwrap();
            }
        },
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
            "eclipse://localhost/history",
            "eclipse://history",
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

    #[test]
    fn the_history_page_is_served_with_the_internal_policy() {
        let res = get("eclipse://localhost/history");
        assert_eq!(res.status(), 200);
        assert_eq!(res.headers()["content-type"], "text/html; charset=utf-8");
        assert_eq!(res.headers()["content-security-policy"], INTERNAL_PAGE_CSP);
        assert_eq!(res.headers()["x-frame-options"], "DENY");
        assert_eq!(res.body().as_ref(), HISTORY_HTML.as_bytes());
        // Self-contained: no external scripts, styles or fetches.
        assert!(!HISTORY_HTML.contains("<script src"));
        assert!(!HISTORY_HTML.contains("fetch("));
        assert!(!HISTORY_HTML.contains("XMLHttpRequest"));
    }

    #[test]
    fn internal_pages_are_named_by_their_route() {
        assert_eq!(InternalPage::from_url("eclipse://localhost/history"), Some(InternalPage::History));
        assert_eq!(InternalPage::from_url("eclipse://history"), Some(InternalPage::History));
        assert_eq!(InternalPage::from_url("eclipse://localhost/history/"), Some(InternalPage::History));
        assert_eq!(InternalPage::from_url("eclipse://localhost/settings"), Some(InternalPage::Settings));
        assert_eq!(InternalPage::from_url("eclipse://localhost/newtab"), Some(InternalPage::NewTab));
        assert_eq!(InternalPage::from_url("eclipse://localhost/"), Some(InternalPage::NewTab));
        assert_eq!(InternalPage::from_url("eclipse://localhost/icon"), None);
        assert_eq!(InternalPage::from_url("eclipse://localhost/whatever"), None);
        assert_eq!(InternalPage::from_url("https://evil.example/history"), None);
        assert_eq!(InternalPage::from_url("https://history/"), None);
        assert_eq!(InternalPage::from_url("about:blank"), None);
        assert_eq!(InternalPage::from_url(""), None);
    }
}
