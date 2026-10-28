use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use tao::event_loop::EventLoopProxy;
use tao::window::Window;
use url::Url;
use wry::{
    BackgroundThrottlingPolicy, NewWindowResponse, PageLoadEvent, Rect, WebView, WebViewBuilder,
};

use crate::config::BrowserConfig;
use crate::events::{TabState, UserEvent};

const BRIDGE_SCRIPT: &str = include_str!("bridge.js");
const SHIELDS_SCRIPT: &str = include_str!("shields.js");

pub const MIN_ZOOM: f64 = 0.5;
pub const MAX_ZOOM: f64 = 3.0;

/// Background colour used beneath every page while it loads. This avoids the
/// white flash you normally see between navigations in a dark browser.
const PAGE_BACKGROUND: (u8, u8, u8, u8) = (9, 10, 15, 255);

pub struct Tab {
    pub id: usize,
    pub title: String,
    pub url: String,
    pub favicon: String,
    pub favicon_url: Option<String>,
    pub is_loading: bool,
    pub is_pinned: bool,
    pub is_hibernated: bool,
    pub is_audible: bool,
    pub workspace: String,
    pub zoom: f64,
    pub token: String,
    pub webview: Option<WebView>,
    pub last_active: Instant,
    pub scroll_x: f64,
    pub scroll_y: f64,
    pub pending_scroll_restore: Option<(f64, f64)>,
    pub favorite_index: Option<usize>,
    pub group: Option<String>,
}

impl Tab {
    pub fn new(
        id: usize,
        url: &str,
        workspace: &str,
        window: &Arc<Window>,
        proxy: EventLoopProxy<UserEvent>,
        bounds: Rect,
        config: &BrowserConfig,
    ) -> Self {
        let is_eclipse_url = url.starts_with("eclipse://");
        let title = if is_eclipse_url && url.contains("settings") {
            "Preferences".to_string()
        } else if is_eclipse_url && url.contains("notes") {
            "Eclipse Notes".to_string()
        } else if is_eclipse_url && url.contains("easel") {
            "Eclipse Easel".to_string()
        } else if is_eclipse_url {
            "New Tab".to_string()
        } else {
            "Loading…".to_string()
        };

        let mut tab = Self {
            id,
            title,
            url: url.to_string(),
            favicon: get_default_favicon(url),
            favicon_url: get_favicon_url(url),
            is_loading: !is_eclipse_url,
            is_pinned: false,
            is_hibernated: false,
            is_audible: false,
            workspace: workspace.to_string(),
            zoom: 1.0,
            token: generate_ipc_token(),
            webview: None,
            last_active: Instant::now(),
            scroll_x: 0.0,
            scroll_y: 0.0,
            pending_scroll_restore: None,
            favorite_index: None,
            group: None,
        };

        let webview = tab.build_webview(window, proxy, bounds, config);
        tab.webview = Some(webview);
        tab
    }

    fn build_webview(
        &self,
        window: &Arc<Window>,
        proxy: EventLoopProxy<UserEvent>,
        bounds: Rect,
        config: &BrowserConfig,
    ) -> WebView {
        let tab_id = self.id;
        let proxy_load = proxy.clone();
        let proxy_title = proxy.clone();
        let proxy_newwin = proxy.clone();
        let proxy_ipc = proxy.clone();
        let proxy_download_started = proxy.clone();
        let proxy_download_finished = proxy.clone();

        let space_profile = config.profile_for_workspace(&self.workspace);
        let is_adblock_active =
            space_profile.adblock_enabled && is_adblock_active_for_url(&self.url, config);
        let active_ua = if !space_profile.custom_user_agent.trim().is_empty() {
            space_profile.custom_user_agent.trim()
        } else {
            config.active_user_agent()
        };

        let init_script = format!(
            "{}\n{}\n{}\n{}\n{}\n{}",
            build_config_seed_script(config, Some(&self.url), &self.workspace),
            build_bridge_script(&self.token),
            build_drm_script(config.drm_fairplay_enabled),
            build_shields_script(&self.token, is_adblock_active),
            build_custom_css_script(&config.custom_css),
            build_extension_support_script(&self.url, config, &self.workspace),
        );

        let target_url = tab_webview_url(&self.url);
        let webview_id = format!("tab_{tab_id}");

        let webview = WebViewBuilder::new()
            .with_id(&webview_id)
            .with_bounds(bounds)
            .with_user_agent(active_ua)
            .with_autoplay(config.autoplay_enabled)
            .with_background_color(PAGE_BACKGROUND)
            .with_back_forward_navigation_gestures(true)
            .with_background_throttling(BackgroundThrottlingPolicy::Throttle)
            .with_accept_first_mouse(true)
            .with_focused(false)
            .with_devtools(true)
            .with_initialization_script(&init_script)
            .with_ipc_handler(move |req| {
                let origin = req.uri().to_string();
                let body = req.body().clone();
                let _ = proxy_ipc.send_event(UserEvent::TabIpc {
                    tab_id,
                    msg: body,
                    origin,
                });
            })
            .with_download_started_handler(move |url, path| {
                let suggested = path
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or_default();

                let Some(clean_name) = resolve_download_name(suggested, &url) else {
                    return false;
                };

                if let Some(target) =
                    downloads_dir().map(|dir| unique_download_path(&dir, &clean_name))
                {
                    *path = target;
                }

                let final_path = path.to_string_lossy().to_string();
                let _ = proxy_download_started.send_event(UserEvent::DownloadStarted {
                    file_name: clean_name,
                    url: url.clone(),
                    path: final_path,
                });
                true
            })
            .with_download_completed_handler(move |url, path, success| {
                let suggested = path
                    .as_ref()
                    .and_then(|p| p.file_name())
                    .and_then(|s| s.to_str())
                    .unwrap_or_default();

                let resolved_path = path.as_ref().map(|p| p.to_string_lossy().to_string());
                if let Some(file_name) = resolve_download_name(suggested, &url) {
                    let _ = proxy_download_finished.send_event(UserEvent::DownloadCompleted {
                        url: url.clone(),
                        file_name,
                        path: resolved_path,
                        success,
                    });
                }
            })
            .with_on_page_load_handler(move |event, url| {
                let payload = match event {
                    PageLoadEvent::Started => UserEvent::PageStarted {
                        tab_id,
                        url: url.clone(),
                    },
                    PageLoadEvent::Finished => UserEvent::PageFinished {
                        tab_id,
                        url: url.clone(),
                    },
                };
                let _ = proxy_load.send_event(payload);
            })
            .with_document_title_changed_handler(move |title| {
                let _ = proxy_title.send_event(UserEvent::TitleChanged { tab_id, title });
            })
            .with_new_window_req_handler(move |url, _target| {
                let _ = proxy_newwin.send_event(UserEvent::NewWindowRequested { url });
                NewWindowResponse::Deny
            })
            .with_custom_protocol("eclipse".into(), move |_id, req| {
                crate::ui_assets::handle_eclipse_protocol(req)
            })
            .with_url(&target_url)
            .build_as_child(&**window)
            .expect("Failed to build tab webview as child");

        #[cfg(target_os = "macos")]
        {
            use wry::WebViewExtMacOS;
            let ns_view = webview.webview();
            let ptr = (&*ns_view) as *const _ as *mut std::ffi::c_void;
            crate::macos_menu::style_webview_card(ptr);
            crate::macos_menu::enable_swipe_gestures(ptr);
            crate::macos_menu::install_native_adblocker(ptr, is_adblock_active);
            crate::macos_menu::register_tab_view(ptr, tab_id);
        }

        webview
    }

    pub fn set_visible(&self, visible: bool) {
        if let Some(ref wv) = self.webview {
            let _ = wv.set_visible(visible);
        }
    }

    pub fn set_bounds(&self, bounds: Rect) {
        if let Some(ref wv) = self.webview {
            let _ = wv.set_bounds(bounds);
        }
    }

    pub fn animate_bounds(&self, bounds: Rect, duration: f64) {
        #[cfg(target_os = "macos")]
        if let Some(ref wv) = self.webview {
            use wry::WebViewExtMacOS;
            let ns_view = wv.webview();
            let ptr = (&*ns_view) as *const _ as *mut std::ffi::c_void;
            let (x, y) = bounds.position.to_logical::<f64>(1.0).into();
            let (w, h) = bounds.size.to_logical::<f64>(1.0).into();
            crate::macos_menu::animate_view_frame(ptr, x, y, w, h, duration);
            return;
        }
        self.set_bounds(bounds);
    }

    pub fn focus(&self) {
        if let Some(ref wv) = self.webview {
            let _ = wv.focus();
        }
    }

    pub fn evaluate_script(&self, js: &str) {
        if let Some(ref wv) = self.webview {
            let _ = wv.evaluate_script(js);
        }
    }

    pub fn evaluate_with_callback<F>(&self, js: &str, callback: F)
    where
        F: Fn(String) + Send + 'static,
    {
        if let Some(ref wv) = self.webview {
            let _ = wv.evaluate_script_with_callback(js, callback);
        }
    }

    pub fn set_loading_indicator(&self, is_loading: bool) {
        let flag = if is_loading { "true" } else { "false" };
        self.evaluate_script(&format!(
            "window.__eclipseSetLoading && window.__eclipseSetLoading({flag});"
        ));
    }

    pub fn load_url(&mut self, url: &str) {
        let canonical = canonicalize_eclipse_url(url);
        self.url = canonical.clone();
        self.favicon = get_default_favicon(&canonical);
        self.favicon_url = get_favicon_url(&canonical);
        self.is_loading = !canonical.starts_with("eclipse://");
        self.is_audible = false;
        self.scroll_x = 0.0;
        self.scroll_y = 0.0;
        self.pending_scroll_restore = None;
        self.last_active = Instant::now();

        if let Some(ref wv) = self.webview {
            self.is_hibernated = false;
            if self.is_loading {
                self.set_loading_indicator(true);
            }
            let _ = wv.load_url(&tab_webview_url(&canonical));
        }
    }

    pub fn reload(&self) {
        if let Some(ref wv) = self.webview {
            self.set_loading_indicator(true);
            let _ = wv.reload();
        }
    }

    pub fn stop_loading(&mut self) {
        self.evaluate_script("window.stop(); window.__eclipseSetLoading && window.__eclipseSetLoading(false);");
        self.is_loading = false;
    }

    pub fn go_back(&self) {
        if let Some(ref wv) = self.webview {
            if wv.can_go_back().unwrap_or(false) {
                let _ = wv.go_back();
            } else {
                let _ = wv.evaluate_script("if (window.history && window.history.length > 1) { window.history.back(); } else { window.history.back(); }");
            }
        }
    }

    pub fn go_forward(&self) {
        if let Some(ref wv) = self.webview {
            if wv.can_go_forward().unwrap_or(false) {
                let _ = wv.go_forward();
            } else {
                let _ = wv.evaluate_script("if (window.history) { window.history.forward(); }");
            }
        }
    }

    pub fn can_go_back(&self) -> bool {
        self.webview
            .as_ref()
            .and_then(|wv| wv.can_go_back().ok())
            .unwrap_or(false)
    }

    pub fn can_go_forward(&self) -> bool {
        self.webview
            .as_ref()
            .and_then(|wv| wv.can_go_forward().ok())
            .unwrap_or(false)
    }

    pub fn set_zoom(&mut self, zoom: f64) -> f64 {
        self.zoom = ((zoom.clamp(MIN_ZOOM, MAX_ZOOM) * 100.0).round() / 100.0).max(MIN_ZOOM);
        if let Some(ref wv) = self.webview {
            let _ = wv.zoom(self.zoom);
        }
        self.zoom
    }

    pub fn find(&self, action: &str) {
        let js = format!(
            "window.__eclipseFind && window.__eclipseFind({}, {});",
            js_string(&self.token),
            js_string(action)
        );
        self.evaluate_script(&js);
    }

    pub fn open_devtools(&self) {
        if let Some(ref wv) = self.webview {
            wv.open_devtools();
        }
    }

    pub fn apply_custom_css(&self, css: &str) {
        self.evaluate_script(&build_custom_css_script(css));
    }

    /// Push the current configuration into this page without leaking the
    /// browser config to ordinary websites.
    pub fn sync_config(&self, config: &BrowserConfig, active_url: Option<&str>) {
        let Ok(json) = serde_json::to_string(config) else {
            return;
        };
        let space_profile = config.profile_for_workspace(&self.workspace);
        let is_adblock_active =
            space_profile.adblock_enabled && is_adblock_active_for_url(&self.url, config);
        let act_url = active_url.unwrap_or(&self.url);
        let script = format!(
            r##"(function () {{
                var cfg = {json};
                var actUrl = {act_url_js};
                try {{
                    if (location.protocol === 'eclipse:') {{
                        window.__ECLIPSE_CONFIG__ = cfg;
                        window.__ECLIPSE_ACTIVE_URL__ = actUrl;
                        if (typeof window.onEclipseConfigUpdated === 'function') {{
                            window.onEclipseConfigUpdated(cfg, actUrl);
                        }}
                    }}
                }} catch (_) {{}}
                try {{
                    if (typeof window.__eclipseShields === 'function') {{
                        window.__eclipseShields({secret}, {is_adblock_active});
                    }}
                }} catch (_) {{}}
            }})();"##,
            secret = js_string(&self.token),
            act_url_js = js_string(act_url),
            is_adblock_active = if is_adblock_active { "true" } else { "false" }
        );
        self.evaluate_script(&script);

        #[cfg(target_os = "macos")]
        if let Some(ref wv) = self.webview {
            use wry::WebViewExtMacOS;
            let ns_view = wv.webview();
            let ptr = (&*ns_view) as *const _ as *mut std::ffi::c_void;
            crate::macos_menu::set_native_adblocker_enabled(ptr, is_adblock_active);
        }
    }

    pub fn hibernate(&mut self) {
        if self.webview.is_some() {
            self.pending_scroll_restore = Some((self.scroll_x, self.scroll_y));
            self.webview = None;
            self.is_hibernated = true;
            self.is_loading = false;
            self.is_audible = false;
        }
    }

    pub fn restore_if_needed(
        &mut self,
        window: &Arc<Window>,
        proxy: EventLoopProxy<UserEvent>,
        bounds: Rect,
        config: &BrowserConfig,
    ) {
        if self.is_hibernated || self.webview.is_none() {
            let saved_scroll = (self.scroll_x, self.scroll_y);
            let webview = self.build_webview(window, proxy, bounds, config);
            if (self.zoom - 1.0).abs() > f64::EPSILON {
                let _ = webview.zoom(self.zoom);
            }
            self.webview = Some(webview);
            self.is_hibernated = false;
            if saved_scroll.0 > 0.0 || saved_scroll.1 > 0.0 {
                self.pending_scroll_restore = Some(saved_scroll);
            }
        }
        self.last_active = Instant::now();
    }

    pub fn restore_scroll_if_pending(&mut self) {
        if let Some((x, y)) = self.pending_scroll_restore.take() {
            self.evaluate_script(&format!(
                "window.scrollTo({x:.1}, {y:.1}); setTimeout(function () {{ window.scrollTo({x:.1}, {y:.1}); }}, 80);"
            ));
        }
    }

    pub fn to_state(&self) -> TabState {
        TabState {
            id: self.id,
            title: self.title.clone(),
            url: self.url.clone(),
            favicon: self.favicon.clone(),
            favicon_url: self.favicon_url.clone(),
            is_loading: self.is_loading,
            is_pinned: self.is_pinned,
            is_hibernated: self.is_hibernated,
            is_audible: self.is_audible,
            zoom: self.zoom,
            workspace: self.workspace.clone(),
            favorite_index: self.favorite_index,
            group: self.group.clone(),
        }
    }
}

pub fn tab_webview_url(url: &str) -> String {
    match url {
        "eclipse://newtab" | "" => "eclipse://localhost/newtab".to_string(),
        "eclipse://settings" => "eclipse://localhost/settings".to_string(),
        "eclipse://notes" => "eclipse://localhost/notes".to_string(),
        "eclipse://easel" => "eclipse://localhost/easel".to_string(),
        other => other.to_string(),
    }
}

pub fn canonicalize_eclipse_url(url: &str) -> String {
    if url.starts_with("eclipse://") {
        if url.contains("settings") {
            return "eclipse://settings".to_string();
        }
        if url.contains("notes") {
            return "eclipse://notes".to_string();
        }
        if url.contains("easel") {
            return "eclipse://easel".to_string();
        }
        if url.contains("sidebar") {
            return "eclipse://sidebar".to_string();
        }
        return "eclipse://newtab".to_string();
    }
    url.to_string()
}

pub fn is_adblock_active_for_url(url: &str, config: &BrowserConfig) -> bool {
    if !config.adblock_enabled {
        return false;
    }
    if let Ok(parsed) = url::Url::parse(url) {
        if let Some(host) = parsed.host_str() {
            let host_clean = host.to_lowercase();
            for domain in &config.adblock_disabled_domains {
                let d_clean = domain.trim().to_lowercase();
                if !d_clean.is_empty()
                    && (host_clean == d_clean || host_clean.ends_with(&format!(".{d_clean}")))
                {
                    return false;
                }
            }
        }
    }
    true
}

fn wildcard_match(pattern: &str, text: &str) -> bool {
    if pattern == "*" {
        return true;
    }
    let parts: Vec<&str> = pattern.split('*').collect();
    if parts.len() == 1 {
        return pattern == text;
    }
    let mut rem = text;
    for (i, part) in parts.iter().enumerate() {
        if part.is_empty() {
            continue;
        }
        if i == 0 {
            if !rem.starts_with(part) {
                return false;
            }
            rem = &rem[part.len()..];
        } else if i == parts.len() - 1 && !pattern.ends_with('*') {
            return rem.ends_with(part);
        } else if let Some(pos) = rem.find(part) {
            rem = &rem[pos + part.len()..];
        } else {
            return false;
        }
    }
    true
}

fn extension_matches_url(pattern: &str, url: &str) -> bool {
    let p = pattern.trim();
    if p.is_empty() {
        return false;
    }
    if p == "<all_urls>" || p == "*" || p == "*/*" || p == "*://*/*" {
        return url.starts_with("http://") || url.starts_with("https://");
    }
    if let Some((scheme_pat, rest)) = p.split_once("://") {
        let Ok(parsed) = Url::parse(url) else {
            return false;
        };
        let scheme = parsed.scheme();
        let scheme_ok = match scheme_pat {
            "*" | "http*" => scheme == "http" || scheme == "https",
            other => scheme.eq_ignore_ascii_case(other),
        };
        if !scheme_ok {
            return false;
        }
        let (host_pat, path_pat) = match rest.find('/') {
            Some(idx) => (&rest[..idx], &rest[idx..]),
            None => (rest, "/*"),
        };
        let host = parsed.host_str().unwrap_or("").to_ascii_lowercase();
        let host_pat_lower = host_pat.to_ascii_lowercase();
        let host_ok = if host_pat_lower == "*" {
            !host.is_empty()
        } else if let Some(domain) = host_pat_lower.strip_prefix("*.") {
            host == domain || host.ends_with(&format!(".{domain}"))
        } else {
            host == host_pat_lower
        };
        if !host_ok {
            return false;
        }
        let full_path = match parsed.query() {
            Some(q) => format!("{}?{q}", parsed.path()),
            None => parsed.path().to_string(),
        };
        return wildcard_match(path_pat, &full_path) || wildcard_match(path_pat, parsed.path());
    }
    if p.contains('*') {
        return wildcard_match(p, url);
    }
    url.contains(p)
}

pub fn build_extension_support_script(
    url: &str,
    config: &BrowserConfig,
    workspace: &str,
) -> String {
    if !config.webextensions_enabled {
        return String::new();
    }

    let mut ext_scripts = Vec::new();
    let mut ext_styles = Vec::new();

    for ext in &config.installed_extensions {
        if !ext.enabled {
            continue;
        }
        let matches = ext
            .matches
            .iter()
            .any(|pattern| extension_matches_url(pattern, url));
        if matches {
            for script in &ext.content_scripts {
                if !script.trim().is_empty() {
                    ext_scripts.push(script.clone());
                }
            }
            for css in &ext.content_css {
                if !css.trim().is_empty() {
                    ext_styles.push(css.clone());
                }
            }
        }
    }

    let has_custom_js = !config.custom_js.trim().is_empty();
    if ext_scripts.is_empty() && ext_styles.is_empty() && !has_custom_js {
        return String::new();
    }

    let encoded_custom_js =
        serde_json::to_string(&config.custom_js).unwrap_or_else(|_| "\"\"".into());
    let encoded_ext_scripts =
        serde_json::to_string(&ext_scripts).unwrap_or_else(|_| "[]".into());
    let encoded_ext_styles =
        serde_json::to_string(&ext_styles).unwrap_or_else(|_| "[]".into());
    let storage_key = format!("__eclipse_ext_storage_{workspace}");
    let encoded_storage_key =
        serde_json::to_string(&storage_key).unwrap_or_else(|_| "\"__eclipse_ext_storage\"".into());

    format!(
        r##"(function () {{
            'use strict';
            try {{
                if (!window.chrome) window.chrome = {{}};
                if (!window.chrome.runtime) {{
                    var storageKey = {encoded_storage_key};
                    var storageMap = {{}};
                    try {{
                        storageMap = JSON.parse(localStorage.getItem(storageKey) || '{{}}');
                    }} catch (_) {{}}

                    var runtime = {{
                        id: 'eclipse-webextension-engine',
                        getURL: function(path) {{
                            return 'eclipse://extensions/' + String(path || '').replace(/^\//, '');
                        }},
                        getManifest: function() {{
                            return {{
                                name: 'Eclipse WebExtension Engine',
                                version: '2.0.0',
                                manifest_version: 3
                            }};
                        }},
                        sendMessage: function(msg, cb) {{
                            if (typeof cb === 'function') {{
                                setTimeout(function() {{ cb(undefined); }}, 0);
                            }}
                            return Promise.resolve(undefined);
                        }},
                        onMessage: {{
                            addListener: function() {{}},
                            removeListener: function() {{}},
                            hasListener: function() {{ return false; }}
                        }},
                        connect: function() {{
                            var port = {{
                                name: '',
                                postMessage: function() {{}},
                                onMessage: {{ addListener: function() {{}}, removeListener: function() {{}} }},
                                onDisconnect: {{
                                    addListener: function(fn) {{
                                        if (typeof fn === 'function') {{
                                            setTimeout(function() {{ fn(port); }}, 0);
                                        }}
                                    }},
                                    removeListener: function() {{}}
                                }}
                            }};
                            return port;
                        }}
                    }};

                    var storage = {{
                        local: {{
                            get: function(keys, cb) {{
                                var result = {{}};
                                if (typeof keys === 'string') {{
                                    result[keys] = storageMap[keys];
                                }} else if (Array.isArray(keys)) {{
                                    keys.forEach(function(k) {{ result[k] = storageMap[k]; }});
                                }} else if (keys && typeof keys === 'object') {{
                                    Object.keys(keys).forEach(function(k) {{
                                        result[k] = (storageMap[k] !== undefined) ? storageMap[k] : keys[k];
                                    }});
                                }} else {{
                                    result = Object.assign({{}}, storageMap);
                                }}
                                if (typeof cb === 'function') cb(result);
                                return Promise.resolve(result);
                            }},
                            set: function(items, cb) {{
                                if (items && typeof items === 'object') {{
                                    Object.assign(storageMap, items);
                                    try {{
                                        localStorage.setItem(storageKey, JSON.stringify(storageMap));
                                    }} catch (_) {{}}
                                }}
                                if (typeof cb === 'function') cb();
                                return Promise.resolve();
                            }},
                            remove: function(keys, cb) {{
                                var arr = Array.isArray(keys) ? keys : [keys];
                                arr.forEach(function(k) {{ delete storageMap[k]; }});
                                try {{
                                    localStorage.setItem(storageKey, JSON.stringify(storageMap));
                                }} catch (_) {{}}
                                if (typeof cb === 'function') cb();
                                return Promise.resolve();
                            }},
                            clear: function(cb) {{
                                storageMap = {{}};
                                try {{
                                    localStorage.removeItem(storageKey);
                                }} catch (_) {{}}
                                if (typeof cb === 'function') cb();
                                return Promise.resolve();
                            }}
                        }},
                        sync: null
                    }};
                    storage.sync = storage.local;

                    var tabs = {{
                        query: function(info, cb) {{
                            var activeTab = [{{
                                id: 1,
                                active: true,
                                url: location.href,
                                title: document.title
                            }}];
                            if (typeof cb === 'function') cb(activeTab);
                            return Promise.resolve(activeTab);
                        }},
                        sendMessage: function(tabId, msg, cb) {{
                            if (typeof cb === 'function') cb(undefined);
                            return Promise.resolve(undefined);
                        }},
                        create: function(props, cb) {{
                            if (props && props.url) window.open(props.url, '_blank');
                            if (typeof cb === 'function') cb({{ id: 2, url: props ? props.url : '' }});
                            return Promise.resolve({{ id: 2, url: props ? props.url : '' }});
                        }}
                    }};

                    var action = {{
                        setTitle: function() {{}},
                        setIcon: function() {{}},
                        setBadgeText: function() {{}},
                        setBadgeBackgroundColor: function() {{}},
                        onClicked: {{ addListener: function() {{}}, removeListener: function() {{}} }}
                    }};

                    window.chrome.runtime = runtime;
                    window.chrome.storage = storage;
                    window.chrome.tabs = tabs;
                    window.chrome.action = action;
                    window.chrome.browserAction = action;
                    window.chrome.extension = {{
                        getURL: runtime.getURL,
                        inIncognitoContext: false
                    }};
                    if (!window.browser) {{
                        window.browser = window.chrome;
                    }}
                }}
            }} catch (_) {{}}

            var extStyles = {encoded_ext_styles};
            if (Array.isArray(extStyles) && extStyles.length > 0) {{
                function injectExtStyles() {{
                    extStyles.forEach(function(css) {{
                        var styleEl = document.createElement('style');
                        styleEl.className = '__eclipse_extension_style';
                        styleEl.textContent = css;
                        (document.head || document.documentElement).appendChild(styleEl);
                    }});
                }}
                if (document.head || document.documentElement) injectExtStyles();
                else document.addEventListener('DOMContentLoaded', injectExtStyles, {{ once: true }});
            }}

            var extScripts = {encoded_ext_scripts};
            if (Array.isArray(extScripts) && extScripts.length > 0) {{
                extScripts.forEach(function(code) {{
                    try {{
                        (new Function(code))();
                    }} catch (err) {{
                        console.warn('[Eclipse Extension Script Error]', err);
                    }}
                }});
            }}

            var customScript = {encoded_custom_js};
            if (customScript && customScript.trim()) {{
                try {{
                    var runCustom = new Function(customScript);
                    if (document.readyState === 'loading') {{
                        document.addEventListener('DOMContentLoaded', function() {{ runCustom(); }}, {{ once: true }});
                    }} else {{
                        runCustom();
                    }}
                }} catch (err) {{
                    console.warn('[Eclipse UserScript Error]', err);
                }}
            }}
        }})();"##
    )
}

pub fn build_custom_css_script(css: &str) -> String {
    let encoded_css = serde_json::to_string(css).unwrap_or_else(|_| "\"\"".into());
    format!(
        r##"(function () {{
            var cssText = {encoded_css};
            function applyUserCss() {{
                var el = document.getElementById('__eclipse_user_custom_css');
                if (!cssText.trim()) {{
                    if (el) el.remove();
                    return;
                }}
                if (!el) {{
                    el = document.createElement('style');
                    el.id = '__eclipse_user_custom_css';
                    (document.head || document.documentElement).appendChild(el);
                }}
                el.textContent = cssText;
            }}
            if (document.head || document.documentElement) {{
                applyUserCss();
            }} else {{
                document.addEventListener('DOMContentLoaded', applyUserCss, {{ once: true }});
            }}
        }})();"##
    )
}

fn build_config_seed_script(
    config: &BrowserConfig,
    active_url: Option<&str>,
    workspace: &str,
) -> String {
    let json = serde_json::to_string(config).unwrap_or_else(|_| "{}".into());
    let act_url = js_string(active_url.unwrap_or(""));
    let ws_js = js_string(workspace);
    format!(
        r##"(function () {{
            try {{
                window.__ECLIPSE_SPACE__ = {ws_js};
                if (location.protocol === 'eclipse:') {{
                    window.__ECLIPSE_CONFIG__ = {json};
                    window.__ECLIPSE_ACTIVE_URL__ = {act_url};
                }}
            }} catch (_) {{}}
        }})();"##
    )
}

fn build_drm_script(enabled: bool) -> String {
    if !enabled {
        return String::new();
    }
    r##"(function () {
        if (window.__eclipse_drm_shimmed) return;
        window.__eclipse_drm_shimmed = true;
        try {
            if (!document.fullscreenEnabled && document.webkitFullscreenEnabled) {
                Object.defineProperty(document, 'fullscreenEnabled', { get: function () { return true; } });
            }
        } catch (_) {}
    })();"##
        .to_string()
}

fn build_bridge_script(token: &str) -> String {
    BRIDGE_SCRIPT.replace("__ECLIPSE_TOKEN__", token)
}

fn build_shields_script(token: &str, enabled: bool) -> String {
    SHIELDS_SCRIPT.replace("__ECLIPSE_TOKEN__", token).replace(
        "__ECLIPSE_ENABLED__",
        if enabled { "true" } else { "false" },
    )
}

/// 128 bits of OS randomness. The token never leaves our injected closure, so
/// untrusted pages cannot drive the browser chrome through `window.ipc`.
pub fn generate_ipc_token() -> String {
    let mut bytes = [0u8; 16];
    if let Ok(mut file) = fs::File::open("/dev/urandom")
        && file.read_exact(&mut bytes).is_ok()
    {
        use std::fmt::Write as _;
        let mut token = String::with_capacity(32);
        for byte in bytes {
            let _ = write!(token, "{byte:02x}");
        }
        return token;
    }

    use std::hash::{BuildHasher, Hasher, RandomState};
    let mut token = String::with_capacity(32);
    for _ in 0..2 {
        let mut hasher = RandomState::new().build_hasher();
        hasher.write_u64(std::process::id() as u64);
        hasher.write_u128(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|duration| duration.as_nanos())
                .unwrap_or_default(),
        );
        use std::fmt::Write as _;
        let _ = write!(token, "{:016x}", hasher.finish());
    }
    token
}

pub fn get_default_favicon(url: &str) -> String {
    if url.starts_with("eclipse://") {
        return "eclipse".into();
    }
    if let Ok(parsed) = Url::parse(url)
        && let Some(host) = parsed.host_str()
    {
        let h = host.to_lowercase();
        if h.contains("netflix.com") {
            return "netflix".into();
        } else if h.contains("youtube.com") || h.contains("youtu.be") {
            return "youtube".into();
        } else if h.contains("github.com") {
            return "github".into();
        } else if h.contains("reddit.com") {
            return "reddit".into();
        } else if h.contains("apple.com") {
            return "apple".into();
        } else if h.contains("spotify.com") {
            return "spotify".into();
        }
    }
    "globe".into()
}

pub fn get_favicon_url(url: &str) -> Option<String> {
    if url.starts_with("eclipse://") {
        return None;
    }
    let parsed = Url::parse(url).ok()?;
    if parsed.scheme() != "http" && parsed.scheme() != "https" {
        return None;
    }
    let host = parsed.host_str()?;
    if host.is_empty() || host == "localhost" {
        return None;
    }
    parsed.join("/favicon.ico").ok().map(String::from)
}

fn js_string(value: &str) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "\"\"".into())
}

pub fn downloads_dir() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from)?;
    let dir = home.join("Downloads");
    fs::create_dir_all(&dir).ok()?;
    Some(dir)
}

fn unique_download_path(dir: &Path, file_name: &str) -> PathBuf {
    let safe = sanitize_file_name(file_name);
    let mut candidate = dir.join(&safe);
    if !candidate.exists() {
        return candidate;
    }
    let path = Path::new(&safe);
    let stem = path
        .file_stem()
        .map(|value| value.to_string_lossy().into_owned())
        .unwrap_or_else(|| "download".into());
    let extension = path
        .extension()
        .map(|value| value.to_string_lossy().into_owned());
    for index in 1..1000 {
        let name = match extension {
            Some(ref ext) => format!("{stem} ({index}).{ext}"),
            None => format!("{stem} ({index})"),
        };
        candidate = dir.join(name);
        if !candidate.exists() {
            return candidate;
        }
    }
    dir.join(safe)
}

fn sanitize_file_name(file_name: &str) -> String {
    let name = Path::new(file_name)
        .file_name()
        .map(|value| value.to_string_lossy().into_owned())
        .unwrap_or_default();
    let cleaned: String = name
        .chars()
        .map(|ch| match ch {
            '/' | '\\' | ':' | '\0' => '_',
            ch if ch.is_control() => '_',
            ch => ch,
        })
        .collect();
    let trimmed = cleaned.trim().trim_matches('.');
    if trimmed.is_empty() {
        "download".into()
    } else {
        trimmed.chars().take(180).collect()
    }
}

const NON_DOWNLOADABLE_EXTENSIONS: &[&str] = &[
    "html", "htm", "xhtml", "php", "asp", "aspx", "jsp", "cgi", "js", "mjs", "cjs", "css", "less",
    "scss", "ts", "tsx", "map",
];

const TOP_LEVEL_DOMAINS: &[&str] = &[
    "com", "org", "net", "edu", "gov", "mil", "io", "co", "ai", "app", "dev", "in", "uk", "de",
    "fr", "ca", "au", "jp", "cn", "ru", "br", "it", "nl", "se", "no", "es", "ch", "at", "dk", "fi",
    "pl", "be", "ie", "nz", "kr", "sg", "hk", "tw", "za", "mx", "ar", "cl", "xyz", "top", "site",
    "online", "info", "biz", "pro", "me", "tv", "cc", "fm", "am", "us", "eu",
];

const LEGITIMATE_DOWNLOAD_EXTENSIONS: &[&str] = &[
    "zip", "tar", "gz", "tgz", "bz2", "tbz2", "xz", "txz", "7z", "rar", "z", "lz", "lzma", "cab",
    "iso", "img", "bin", "dmg", "pkg", "toast", "vcd", "exe", "msi", "msix", "appx", "deb", "rpm",
    "apk", "aab", "ipa", "jar", "war", "crx", "xpi", "torrent", "pdf", "epub", "mobi", "azw",
    "azw3", "csv", "tsv", "xlsx", "xls", "docx", "doc", "pptx", "ppt", "odt", "ods", "odp", "rtf",
    "tex", "sqlite", "sqlite3", "db", "sql", "parquet", "patch", "diff", "txt", "mp3", "wav",
    "flac", "aac", "ogg", "m4a", "wma", "opus", "aiff", "mp4", "mkv", "avi", "mov", "wmv", "flv",
    "webm", "m4v", "psd", "ai", "blend", "stl", "obj", "fbx", "step", "iges",
];

#[allow(dead_code)]
pub fn is_legitimate_download(suggested_name: &str, url_str: &str) -> bool {
    resolve_download_name(suggested_name, url_str).is_some()
}

pub fn resolve_download_name(suggested_name: &str, url_str: &str) -> Option<String> {
    if let Some(candidate) = extract_valid_download_name(suggested_name) {
        return Some(candidate);
    }

    if let Ok(parsed) = Url::parse(url_str)
        && let Some(segment) = parsed
            .path_segments()
            .and_then(|mut segments| segments.next_back())
    {
        let decoded = percent_decode(segment);
        if !decoded.is_empty()
            && decoded != "/"
            && let Some(candidate) = extract_valid_download_name(&decoded)
        {
            return Some(candidate);
        }
    }

    None
}

fn extract_valid_download_name(candidate: &str) -> Option<String> {
    let decoded = percent_decode(candidate);
    let clean = sanitize_file_name(&decoded);
    if clean.is_empty()
        || clean == "download"
        || clean == "unknown"
        || clean.starts_with("download-")
    {
        return None;
    }

    let path = Path::new(&clean);
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    if ext.is_empty() {
        return None;
    }

    if NON_DOWNLOADABLE_EXTENSIONS.contains(&ext.as_str()) {
        return None;
    }

    if TOP_LEVEL_DOMAINS.contains(&ext.as_str()) {
        return None;
    }

    if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
        let stem_lower = stem.to_ascii_lowercase();
        for tld in TOP_LEVEL_DOMAINS {
            if stem_lower.ends_with(&format!(".{tld}")) || stem_lower == *tld {
                return None;
            }
        }
    }

    if LEGITIMATE_DOWNLOAD_EXTENSIONS.contains(&ext.as_str()) {
        return Some(clean);
    }

    if ext.len() >= 2 && ext.len() <= 6 && ext.chars().all(|c| c.is_ascii_alphanumeric()) {
        return Some(clean);
    }

    None
}

#[cfg(test)]
#[allow(dead_code)]
pub fn download_file_name(url: &str) -> String {
    resolve_download_name("", url).unwrap_or_else(|| "download".into())
}

fn percent_decode(input: &str) -> String {
    fn hex(byte: u8) -> Option<u8> {
        match byte {
            b'0'..=b'9' => Some(byte - b'0'),
            b'a'..=b'f' => Some(byte - b'a' + 10),
            b'A'..=b'F' => Some(byte - b'A' + 10),
            _ => None,
        }
    }

    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%'
            && index + 2 < bytes.len()
            && let (Some(high), Some(low)) = (hex(bytes[index + 1]), hex(bytes[index + 2]))
        {
            out.push(high * 16 + low);
            index += 3;
            continue;
        }
        out.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_urls() {
        assert_eq!(
            canonicalize_eclipse_url("eclipse://localhost/newtab"),
            "eclipse://newtab"
        );
        assert_eq!(
            canonicalize_eclipse_url("eclipse://localhost/settings"),
            "eclipse://settings"
        );
        assert_eq!(
            canonicalize_eclipse_url("eclipse://localhost/notes"),
            "eclipse://notes"
        );
        assert_eq!(
            canonicalize_eclipse_url("eclipse://localhost/easel"),
            "eclipse://easel"
        );
        assert_eq!(
            canonicalize_eclipse_url("https://example.com"),
            "https://example.com"
        );
    }

    #[test]
    fn tab_urls_are_local() {
        assert_eq!(
            tab_webview_url("eclipse://newtab"),
            "eclipse://localhost/newtab"
        );
        assert_eq!(
            tab_webview_url("eclipse://settings"),
            "eclipse://localhost/settings"
        );
        assert_eq!(
            tab_webview_url("eclipse://notes"),
            "eclipse://localhost/notes"
        );
        assert_eq!(
            tab_webview_url("eclipse://easel"),
            "eclipse://localhost/easel"
        );
        assert_eq!(
            tab_webview_url("https://example.com"),
            "https://example.com"
        );
    }

    #[test]
    fn tokens_are_unique() {
        let first = generate_ipc_token();
        let second = generate_ipc_token();
        assert_eq!(first.len(), 32);
        assert_ne!(first, second);
    }

    #[test]
    fn favicon_fallback_stays_on_site() {
        assert_eq!(
            get_favicon_url("https://example.com/a/b?q=1"),
            Some("https://example.com/favicon.ico".to_string())
        );
        assert_eq!(get_favicon_url("eclipse://newtab"), None);
    }

    #[test]
    fn download_validation_rejects_pages_and_accepts_files() {
        assert!(!is_legitimate_download(
            "www.youtube.com",
            "https://www.youtube.com/"
        ));
        assert!(!is_legitimate_download(
            "youtube.com",
            "https://youtube.com/"
        ));
        assert!(!is_legitimate_download(
            "download",
            "https://www.youtube.com/videoplayback"
        ));
        assert!(!is_legitimate_download(
            "index.html",
            "https://example.com/index.html"
        ));
        assert!(!is_legitimate_download(
            "login.php",
            "https://example.com/login.php"
        ));
        assert!(!is_legitimate_download(
            "bundle.js",
            "https://example.com/bundle.js"
        ));
        assert!(!is_legitimate_download("", "https://example.com/"));

        assert!(is_legitimate_download(
            "report.pdf",
            "https://example.com/files/report.pdf"
        ));
        assert!(is_legitimate_download(
            "archive.zip",
            "https://example.com/archive.zip"
        ));
        assert!(is_legitimate_download(
            "Eclipse-Installer.dmg",
            "https://example.com/dl"
        ));
        assert!(is_legitimate_download(
            "data.csv",
            "https://example.com/export?format=csv"
        ));

        assert_eq!(
            resolve_download_name(
                "report%20final.pdf",
                "https://example.com/files/report%20final.pdf"
            ),
            Some("report final.pdf".into())
        );
        assert_eq!(
            resolve_download_name("", "https://example.com/files/archive.tar.gz"),
            Some("archive.tar.gz".into())
        );
        assert_eq!(resolve_download_name("", "https://example.com/"), None);
        assert!(!sanitize_file_name("../../etc/passwd").contains('/'));
    }

    #[test]
    fn test_adblock_site_toggling_and_extension_matching() {
        let mut cfg = BrowserConfig::default();
        cfg.adblock_enabled = true;
        cfg.adblock_disabled_domains = vec!["net77.cc".into(), "youtube.com".into()];

        assert!(!is_adblock_active_for_url("https://net77.cc/home", &cfg));
        assert!(!is_adblock_active_for_url("https://sub.net77.cc/video", &cfg));
        assert!(!is_adblock_active_for_url(
            "https://www.youtube.com/watch?v=123",
            &cfg
        ));

        assert!(is_adblock_active_for_url("https://github.com", &cfg));
        assert!(is_adblock_active_for_url("https://apple.com", &cfg));

        cfg.adblock_enabled = false;
        assert!(!is_adblock_active_for_url("https://github.com", &cfg));

        assert!(extension_matches_url(
            "*://*.youtube.com/*",
            "https://www.youtube.com/watch?v=abc"
        ));
        assert!(extension_matches_url(
            "https://*.youtube.com/*",
            "https://www.youtube.com/watch?v=abc"
        ));
        assert!(extension_matches_url(
            "https://*.youtube.com/watch*",
            "https://youtube.com/watch?v=abc"
        ));
        assert!(extension_matches_url("<all_urls>", "https://example.org"));
        assert!(!extension_matches_url(
            "*://*.youtube.com/*",
            "https://vimeo.com/123"
        ));
        assert!(!extension_matches_url(
            "https://*.youtube.com/*",
            "http://www.youtube.com/watch?v=abc"
        ));
    }
}
