use std::sync::Arc;
use std::time::Instant;
use tao::event_loop::EventLoopProxy;
use tao::window::Window;
use wry::{NewWindowResponse, PageLoadEvent, Rect, WebView, WebViewBuilder};

use crate::config::BrowserConfig;
use crate::events::{TabState, UserEvent};

pub struct Tab {
    pub id: usize,
    pub title: String,
    pub url: String,
    pub favicon: String,
    pub is_loading: bool,
    pub is_pinned: bool,
    pub is_hibernated: bool,
    pub workspace: String,
    pub webview: Option<WebView>,
    pub last_active: Instant,
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
        let title = if is_eclipse_url {
            "New Tab".to_string()
        } else {
            "Loading...".to_string()
        };

        let mut tab = Self {
            id,
            title,
            url: url.to_string(),
            favicon: get_default_favicon(url),
            is_loading: !is_eclipse_url,
            is_pinned: false,
            is_hibernated: false,
            workspace: workspace.to_string(),
            webview: None,
            last_active: Instant::now(),
        };

        let webview = tab.build_webview(window, proxy, bounds, config);
        tab.webview = Some(webview);
        tab
    }

    pub fn build_webview(
        &self,
        window: &Arc<Window>,
        proxy: EventLoopProxy<UserEvent>,
        bounds: Rect,
        config: &BrowserConfig,
    ) -> WebView {
        let tab_id = self.id;
        let proxy1 = proxy.clone();
        let proxy2 = proxy.clone();
        let proxy3 = proxy;

        // Hardware FairPlay DRM compatibility shim
        let drm_script = r#"
            (function() {
                if (window.__eclipse_drm_shimmed) return;
                window.__eclipse_drm_shimmed = true;
                if (!document.fullscreenEnabled && document.webkitFullscreenEnabled) {
                    document.fullscreenEnabled = true;
                }
            })();
        "#;

        // Advanced AdBlocker, Tracker Shield & YouTube Ad Auto-Skipper
        let adblock_script = if config.adblock_enabled {
            r#"
            (function() {
                if (window.__eclipse_shields_active) return;
                window.__eclipse_shields_active = true;

                // 1. DNT & Global Privacy Control Spoofing
                try {
                    Object.defineProperty(navigator, 'doNotTrack', { value: '1', configurable: false });
                    Object.defineProperty(navigator, 'globalPrivacyControl', { value: true, configurable: false });
                } catch(e) {}

                // 2. Neutralize common tracking analytics & telemetry functions
                window.ga = window.ga || function() {};
                window.gtag = window.gtag || function() {};
                window._fbq = window.fbq = window.fbq || function() {};
                window._hsq = window._hsq || [];

                // 3. Cosmetic Ad Blocking CSS injection
                const style = document.createElement('style');
                style.textContent = `
                    .adsbygoogle, [id^="google_ads_"], .ad-banner, .ad-unit,
                    .advertisement, .ad-container, [data-ad-slot], .ytp-ad-overlay-container,
                    .ytp-ad-message-container, ytd-promoted-sparkles-web-renderer,
                    ytd-display-ad-renderer, ytd-banner-promo-renderer,
                    ytd-ad-slot-renderer, #player-ads, .sparkles-light-cta,
                    div[id*="ad-slot"], div[class*="ad_unit"], div[class*="Sponsored"],
                    .trc_related_container, .outbrain, .taboola {
                        display: none !important;
                        visibility: hidden !important;
                        height: 0 !important;
                        max-height: 0 !important;
                        overflow: hidden !important;
                        opacity: 0 !important;
                        pointer-events: none !important;
                    }
                `;
                if (document.head) {
                    document.head.appendChild(style);
                } else {
                    document.addEventListener('DOMContentLoaded', () => document.head.appendChild(style));
                }

                // 4. YouTube Video Ad Auto-Skipper & Interruptor Neutralizer
                if (location.hostname.includes('youtube.com')) {
                    setInterval(() => {
                        // Click standard skip button if present
                        const skipBtn = document.querySelector('.ytp-ad-skip-button, .ytp-skip-ad-button, .ytp-ad-skip-button-modern');
                        if (skipBtn) {
                            skipBtn.click();
                        }
                        // Close overlay ads
                        const closeBtn = document.querySelector('.ytp-ad-overlay-close-button');
                        if (closeBtn) {
                            closeBtn.click();
                        }
                        // Skip video ad immediately to the end if ad is playing
                        const adShowing = document.querySelector('.ad-showing, .ad-interrupting');
                        const video = document.querySelector('video');
                        if (adShowing && video && isFinite(video.duration)) {
                            video.currentTime = video.duration;
                        }
                    }, 250);
                }
            })();
            "#
        } else {
            ""
        };

        let init_script = format!("{}\n{}", drm_script, adblock_script);

        let mut builder = WebViewBuilder::new()
            .with_bounds(bounds)
            .with_user_agent(config.active_user_agent())
            .with_autoplay(true)
            .with_devtools(true)
            .with_initialization_script(&init_script)
            .with_download_started_handler(|_url, path| {
                let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
                let mut download_dir = std::path::PathBuf::from(home);
                download_dir.push("Downloads");
                if let Some(file_name) = path.file_name() {
                    download_dir.push(file_name);
                    *path = download_dir;
                }
                true
            })
            .with_on_page_load_handler(move |event, url| {
                match event {
                    PageLoadEvent::Started => {
                        let _ = proxy1.send_event(UserEvent::PageStarted {
                            tab_id,
                            url: url.clone(),
                        });
                    }
                    PageLoadEvent::Finished => {
                        let _ = proxy1.send_event(UserEvent::PageFinished {
                            tab_id,
                            url: url.clone(),
                        });
                    }
                }
            })
            .with_document_title_changed_handler(move |title| {
                let _ = proxy2.send_event(UserEvent::TitleChanged {
                    tab_id,
                    title,
                });
            })
            .with_new_window_req_handler(move |url, _target| {
                let _ = proxy3.send_event(UserEvent::NewWindowRequested {
                    url,
                });
                NewWindowResponse::Deny
            })
            .with_custom_protocol("eclipse".into(), move |_id, req| {
                let path = req.uri().path();
                let body = match path {
                    "/newtab" | "newtab" => crate::ui_assets::NEWTAB_HTML.as_bytes().to_vec(),
                    "/settings" | "settings" => crate::ui_assets::SETTINGS_HTML.as_bytes().to_vec(),
                    _ => crate::ui_assets::NEWTAB_HTML.as_bytes().to_vec(),
                };
                wry::http::Response::builder()
                    .header("content-type", "text/html")
                    .body(std::borrow::Cow::Owned(body))
                    .unwrap()
            });

        let target_url = if self.url == "eclipse://newtab" || self.url.is_empty() {
            "eclipse://newtab".to_string()
        } else {
            self.url.clone()
        };

        builder = builder.with_url(&target_url);
        builder.build_as_child(&**window).expect("Failed to build tab webview as child")
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

    pub fn load_url(&mut self, url: &str) {
        self.url = url.to_string();
        self.is_hibernated = false;
        self.last_active = Instant::now();
        if let Some(ref wv) = self.webview {
            let _ = wv.load_url(url);
        }
    }

    pub fn reload(&self) {
        if let Some(ref wv) = self.webview {
            let _ = wv.evaluate_script("location.reload();");
        }
    }

    pub fn go_back(&self) {
        if let Some(ref wv) = self.webview {
            let _ = wv.evaluate_script("history.back();");
        }
    }

    pub fn go_forward(&self) {
        if let Some(ref wv) = self.webview {
            let _ = wv.evaluate_script("history.forward();");
        }
    }

    pub fn hibernate(&mut self) {
        if self.webview.is_some() {
            self.webview = None; // Drops WKWebView NSView immediately from memory
            self.is_hibernated = true;
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
            let webview = self.build_webview(window, proxy, bounds, config);
            self.webview = Some(webview);
            self.is_hibernated = false;
        }
        self.last_active = Instant::now();
    }

    pub fn to_state(&self) -> TabState {
        TabState {
            id: self.id,
            title: self.title.clone(),
            url: self.url.clone(),
            favicon: self.favicon.clone(),
            is_loading: self.is_loading,
            is_pinned: self.is_pinned,
            is_hibernated: self.is_hibernated,
            workspace: self.workspace.clone(),
        }
    }
}

pub fn get_default_favicon(url: &str) -> String {
    if url.contains("netflix.com") {
        "netflix".into()
    } else if url.contains("youtube.com") {
        "youtube".into()
    } else if url.contains("github.com") {
        "github".into()
    } else if url.contains("reddit.com") {
        "reddit".into()
    } else if url.contains("apple.com") {
        "apple".into()
    } else if url.contains("spotify.com") {
        "spotify".into()
    } else if url.starts_with("eclipse://") {
        "eclipse".into()
    } else {
        "globe".into()
    }
}
