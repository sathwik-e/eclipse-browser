use std::cell::{Cell, RefCell};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use tao::event_loop::EventLoopProxy;
use tao::window::Window;
use url::Url;
use wry::{
    BackgroundThrottlingPolicy, NewWindowResponse, PageLoadEvent, Rect, WebView, WebViewBuilder,
};

use crate::config::BrowserConfig;
use crate::events::{DocumentTrust, TabState, UserEvent};

const BRIDGE_SCRIPT: &str = include_str!("bridge.js");
const SHIELDS_SCRIPT: &str = include_str!("shields.js");

/// Title of a tab whose web page has not reported one yet.
pub const LOADING_TITLE: &str = "Loading…";

pub const MIN_ZOOM: f64 = 0.5;
pub const MAX_ZOOM: f64 = 3.0;

/// Revision passed by callers that do not track config revisions: the config
/// is always pushed.
const UNVERSIONED: u64 = u64::MAX;

/// Runs an internal page's or the sidebar's flush hook (settings, notes,
/// easel and the sidebar's quick note post their debounced edits
/// synchronously from it). Always evaluates to `true`, so the completion
/// callback fires even when the page has no hook.
pub(crate) const FLUSH_SCRIPT: &str = "(function () { try { if (typeof window.__eclipseFlush === 'function') window.__eclipseFlush(); } catch (_) {} return true; })()";

/// `NavState::native_adblock` values (0 until the webview is built).
const NATIVE_ADBLOCK_OFF: u8 = 1;
const NATIVE_ADBLOCK_ON: u8 = 2;

/// The ad-blocking rules that apply to one tab: its space's switch combined
/// with the global one, and the sites the user excluded.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct AdblockPolicy {
    enabled: bool,
    /// Lowercased, trimmed, never empty.
    disabled_domains: Vec<String>,
}

impl AdblockPolicy {
    fn for_space(config: &BrowserConfig, workspace: &str) -> Self {
        let profile = config.profile_for_workspace(workspace);
        Self {
            enabled: profile.adblock_enabled && config.adblock_enabled,
            disabled_domains: normalized_domains(&config.adblock_disabled_domains),
        }
    }

    fn blocks(&self, url: &str) -> bool {
        self.enabled && !url_host_in_domains(url, &self.disabled_domains)
    }
}

/// Navigation state shared between a tab and its webview's WebKit callbacks.
/// Every webview (including one rebuilt after hibernation) gets a fresh one.
#[derive(Default)]
struct NavState {
    /// Bumped on every committed navigation, so each value is one document.
    /// Zero means nothing has committed yet.
    epoch: AtomicU64,
    /// The epoch whose load has finished.
    finished_epoch: AtomicU64,
    /// URL of the committed main-frame document, as WebKit reports it.
    committed_url: Mutex<String>,
    /// The committed document is an internal eclipse:// page.
    current_is_internal: AtomicBool,
    /// The previous committed document was internal and the current one is
    /// not (a save the old page posted while unloading may still arrive).
    left_internal: AtomicBool,
    /// The eclipse:// URL the browser itself is loading (load_url, the start
    /// URL, a retry from the error page). The navigation policy lets exactly
    /// this URL through, once; commits and failures clear it.
    browser_target: Mutex<Option<String>>,
    /// A load the browser started (load_url, the start URL, a retry) has
    /// neither committed nor failed yet.
    browser_load_pending: AtomicBool,
    /// An error page is shown through load_html. Its about:blank load events
    /// are swallowed so the tab keeps reporting the URL that failed.
    showing_error_page: AtomicBool,
    /// Whether the native content-rule list is attached (NATIVE_ADBLOCK_*).
    native_adblock: AtomicU8,
    /// The tab's ad-blocking rules, for decisions made at navigation time.
    adblock: Mutex<AdblockPolicy>,
    /// The tab's WKWebView as an NSView pointer once it is built (0 before).
    /// It lives exactly as long as the webview that owns this state's
    /// navigation handler.
    view: AtomicUsize,
}

impl NavState {
    fn committed_url(&self) -> String {
        self.committed_url
            .lock()
            .map(|url| url.clone())
            .unwrap_or_default()
    }

    /// Navigation policy: web pages may not open internal pages (other than
    /// the new-tab page, which holds nothing) in the tab or in frames. An
    /// internal page may, and so may the browser itself: a main-frame load it
    /// started (exactly that URL, once; a subframe asking for the same URL
    /// neither gets nor spends the grant) or a main-frame back/forward
    /// navigation, which can only return to a page this tab already showed.
    fn allows_navigation(&self, url: &str, context: PolicyContext) -> bool {
        if !is_eclipse_scheme(url) {
            return true;
        }
        self.current_is_internal.load(Ordering::Relaxed)
            || (context.main_frame && context.back_forward)
            || is_newtab_url(url)
            || (context.main_frame && self.take_browser_target(url))
    }

    fn set_browser_target(&self, url: Option<String>) {
        if let Ok(mut target) = self.browser_target.lock() {
            *target = url;
        }
    }

    /// Consumes the browser's pending eclipse:// target if `url` is it.
    fn take_browser_target(&self, url: &str) -> bool {
        let Ok(mut target) = self.browser_target.lock() else {
            return false;
        };
        let matches = target
            .as_deref()
            .is_some_and(|wanted| same_eclipse_url(wanted, url));
        if matches {
            *target = None;
        }
        matches
    }

    /// Records a browser-started load of `url` (the tab's webview URL).
    fn begin_browser_load(&self, url: &str) {
        self.set_browser_target(is_eclipse_scheme(url).then(|| url.to_string()));
        self.browser_load_pending.store(true, Ordering::Relaxed);
    }

    /// A document committed: the previous one's grants and flags are over.
    fn note_commit(&self, url: &str) {
        self.showing_error_page.store(false, Ordering::Relaxed);
        self.set_browser_target(None);
        self.browser_load_pending.store(false, Ordering::Relaxed);
        let internal = is_eclipse_scheme(url);
        let was_internal = self.current_is_internal.swap(internal, Ordering::Relaxed);
        self.left_internal
            .store(was_internal && !internal, Ordering::Relaxed);
        self.set_committed_url(url);
        self.epoch.fetch_add(1, Ordering::Relaxed);
    }

    /// The committed document finished loading. The browser target is left
    /// alone: this document already cleared it when it committed, and a
    /// browser load started since (whose policy decision may still be
    /// pending) must keep its grant.
    fn note_finished(&self) {
        self.finished_epoch
            .store(self.epoch.load(Ordering::Relaxed), Ordering::Relaxed);
    }

    fn set_committed_url(&self, url: &str) {
        if let Ok(mut committed) = self.committed_url.lock() {
            committed.clear();
            committed.push_str(url);
        }
    }

    /// URL of the committed web page that would be a popup's opener: empty
    /// while an internal page or the error page is on screen.
    fn popup_opener_url(&self) -> String {
        if self.showing_error_page.load(Ordering::Relaxed)
            || self.current_is_internal.load(Ordering::Relaxed)
        {
            return String::new();
        }
        self.committed_url()
    }

    /// What the committed document is, for an IPC message arriving now.
    fn document_trust(&self) -> DocumentTrust {
        if self.showing_error_page.load(Ordering::Relaxed) {
            return DocumentTrust::default();
        }
        DocumentTrust {
            internal: self.current_is_internal.load(Ordering::Relaxed),
            left_internal: self.left_internal.load(Ordering::Relaxed),
        }
    }

    fn set_adblock_policy(&self, policy: &AdblockPolicy) {
        if let Ok(mut current) = self.adblock.lock()
            && *current != *policy
        {
            *current = policy.clone();
        }
    }

    /// Attaches or detaches the native rule list (only when that changes).
    fn set_native_adblock(&self, enabled: bool) {
        let ptr = self.view.load(Ordering::Relaxed) as *mut std::ffi::c_void;
        if ptr.is_null() {
            return;
        }
        let state = if enabled {
            NATIVE_ADBLOCK_ON
        } else {
            NATIVE_ADBLOCK_OFF
        };
        if self.native_adblock.swap(state, Ordering::Relaxed) == state {
            return;
        }
        #[cfg(target_os = "macos")]
        crate::macos_menu::set_native_adblocker_enabled(ptr, enabled);
    }

    /// A main-frame navigation to `url` was allowed: switch the rule list to
    /// the destination's site before its first request goes out, so a site
    /// the user excluded is never filtered on its first load (and the
    /// reverse).
    fn prepare_main_frame_load(&self, url: &str) {
        if !is_http_url(url) {
            return;
        }
        let blocks = self
            .adblock
            .lock()
            .map(|policy| policy.blocks(url))
            .unwrap_or(false);
        self.set_native_adblock(blocks);
    }
}

/// What the current document of a tab has already received, so repeated
/// sync calls (page events, config changes, tab switches) only send changes.
#[derive(Default)]
struct SyncState {
    /// Navigation epoch the fields below belong to.
    epoch: Cell<u64>,
    /// Config revision and active URL last pushed to this internal page after
    /// it finished loading.
    config: RefCell<Option<(u64, String)>>,
    /// In-page shields state of this document (None: the init-script default).
    shields: Cell<Option<bool>>,
    /// Extension content scripts / custom JS were injected into this document.
    page_scripts_done: Cell<bool>,
}

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
    /// Secret for the in-page control hooks (`__eclipseFind`, `__eclipseShields`).
    /// It is not the IPC token, so a page that pre-empts a hook learns nothing
    /// it could post with.
    control_token: String,
    nav: Arc<NavState>,
    sync: SyncState,
    /// Ad-blocking rules, DRM shim and custom CSS baked into the webview's
    /// init scripts (fixed at creation; later documents are topped up in
    /// sync). The in-page shields start from `init_adblock` applied to each
    /// document's own host.
    init_adblock: AdblockPolicy,
    init_drm: bool,
    init_custom_css: String,
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
            LOADING_TITLE.to_string()
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
            control_token: generate_ipc_token(),
            nav: Arc::default(),
            sync: SyncState::default(),
            init_adblock: AdblockPolicy::default(),
            init_drm: false,
            init_custom_css: String::new(),
        };

        let webview = tab.build_webview(window, proxy, bounds, config);
        tab.webview = Some(webview);
        tab
    }

    /// A tab restored from the saved session without a webview yet; selecting
    /// it builds the webview (see `restore_if_needed`).
    #[allow(clippy::too_many_arguments)]
    pub fn hibernated(
        id: usize,
        url: &str,
        title: String,
        workspace: &str,
        is_pinned: bool,
        zoom: f64,
        favorite_index: Option<usize>,
    ) -> Self {
        Self {
            id,
            title,
            url: url.to_string(),
            favicon: get_default_favicon(url),
            favicon_url: get_favicon_url(url),
            is_loading: false,
            is_pinned,
            is_hibernated: true,
            is_audible: false,
            workspace: workspace.to_string(),
            zoom: zoom.clamp(MIN_ZOOM, MAX_ZOOM),
            token: generate_ipc_token(),
            webview: None,
            last_active: Instant::now(),
            scroll_x: 0.0,
            scroll_y: 0.0,
            pending_scroll_restore: None,
            favorite_index,
            group: None,
            control_token: generate_ipc_token(),
            nav: Arc::default(),
            sync: SyncState::default(),
            init_adblock: AdblockPolicy::default(),
            init_drm: false,
            init_custom_css: String::new(),
        }
    }

    fn build_webview(
        &mut self,
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
        let adblock = AdblockPolicy::for_space(config, &self.workspace);
        let is_adblock_active = adblock.blocks(&self.url);
        let active_ua = if !space_profile.custom_user_agent.trim().is_empty() {
            space_profile.custom_user_agent.trim()
        } else {
            config.active_user_agent()
        };

        let target_url = tab_webview_url(&self.url);
        let nav = Arc::new(NavState::default());
        nav.begin_browser_load(&target_url);
        nav.set_adblock_policy(&adblock);
        self.nav = nav.clone();
        self.sync = SyncState::default();
        self.init_drm = config.drm_fairplay_enabled;
        self.init_custom_css = config.custom_css.clone();

        // Each part is its own WKUserScript inside its own try/catch, so a
        // syntax or runtime error in one (for example a Trusted Types site
        // refusing a DOM write) cannot stop the others.
        let init_scripts = [
            build_config_seed_script(config, &self.url),
            build_bridge_script(&self.token, &self.control_token),
            build_drm_script(config.drm_fairplay_enabled),
            build_shields_script(&self.control_token, &adblock),
            build_custom_css_script(&config.custom_css),
        ];

        self.init_adblock = adblock;
        let webview_id = format!("tab_{tab_id}");
        let nav_load = nav.clone();
        let nav_policy = nav.clone();
        let nav_ipc = nav.clone();
        let nav_popup = nav.clone();

        let mut builder = WebViewBuilder::new()
            .with_id(&webview_id)
            .with_bounds(bounds)
            .with_user_agent(active_ua)
            .with_autoplay(config.autoplay_enabled)
            .with_back_forward_navigation_gestures(true)
            .with_background_throttling(BackgroundThrottlingPolicy::Throttle)
            .with_accept_first_mouse(true)
            .with_focused(false)
            .with_devtools(true);
        for script in init_scripts
            .iter()
            .filter(|script| !script.trim().is_empty())
        {
            builder = builder.with_initialization_script(guard_script(script));
        }

        let builder = builder
            .with_ipc_handler(move |req| {
                // Read now: a later commit must not change how a message the
                // previous document posted is trusted.
                let document = nav_ipc.document_trust();
                let origin = req.uri().to_string();
                let body = req.body().clone();
                let _ = proxy_ipc.send_event(UserEvent::TabIpc {
                    tab_id,
                    msg: body,
                    origin,
                    document,
                });
            })
            .with_navigation_handler(move |url| {
                let context = navigation_policy_context();
                let allowed = nav_policy.allows_navigation(&url, context);
                if allowed && context.main_frame {
                    nav_policy.prepare_main_frame_load(&url);
                }
                allowed
            })
            .with_download_started_handler(move |url, path| {
                let suggested = path
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or_default()
                    .to_string();
                let mut pending = pending_downloads();

                let seq = NEXT_DOWNLOAD_SEQ.fetch_add(1, Ordering::Relaxed);
                let Some(clean_name) = resolve_download_name(&suggested, &url) else {
                    // A page load WebKit could not display, not a file.
                    remember_download(&mut pending, seq, url, None);
                    return false;
                };

                if let Some(dir) = downloads_dir() {
                    let reserved: Vec<&Path> =
                        pending.iter().filter_map(|d| d.path.as_deref()).collect();
                    *path = unique_download_path(&dir, &clean_name, &reserved);
                }

                let file_name = path
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or(clean_name);
                remember_download(&mut pending, seq, url.clone(), Some(path.clone()));
                drop(pending);

                let _ = proxy_download_started.send_event(UserEvent::DownloadStarted {
                    seq,
                    file_name,
                    url,
                    path: path.to_string_lossy().to_string(),
                });
                true
            })
            .with_download_completed_handler(move |url, path, success| {
                // wry reports neither a path nor an identity on macOS: the
                // completion is matched to the oldest download of that URL
                // still pending, and its sequence number lets the app update
                // exactly the list item that download created.
                let started = {
                    let mut pending = pending_downloads();
                    pending
                        .iter()
                        .position(|d| d.url == url)
                        .map(|index| pending.remove(index))
                };
                let (seq, final_path) = match started {
                    Some(PendingDownload { path: None, .. }) => return,
                    Some(PendingDownload {
                        seq, path: Some(p), ..
                    }) => (Some(seq), path.or(Some(p))),
                    None => (None, path),
                };
                let file_name = final_path
                    .as_ref()
                    .and_then(|p| p.file_name())
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_else(|| download_file_name(&url));
                let _ = proxy_download_finished.send_event(UserEvent::DownloadCompleted {
                    seq,
                    url,
                    file_name,
                    path: final_path.map(|p| p.to_string_lossy().to_string()),
                    success,
                });
            })
            .with_on_page_load_handler(move |event, url| {
                let error_page_load =
                    nav_load.showing_error_page.load(Ordering::Relaxed) && url == "about:blank";
                if error_page_load {
                    return;
                }
                let payload = match event {
                    PageLoadEvent::Started => {
                        nav_load.note_commit(&url);
                        UserEvent::PageStarted { tab_id, url }
                    }
                    PageLoadEvent::Finished => {
                        nav_load.note_finished();
                        UserEvent::PageFinished { tab_id, url }
                    }
                };
                let _ = proxy_load.send_event(payload);
            })
            .with_document_title_changed_handler(move |title| {
                let _ = proxy_title.send_event(UserEvent::TitleChanged { tab_id, title });
            })
            // Sized popups from a web page (sign-in and payment flows) open in
            // a Little Eclipse window that keeps `window.opener`; everything
            // else opens as an ordinary tab without one. The popup's webview
            // is built on the configuration WebKit hands over, which shares
            // this tab's WKUserContentController: it runs this tab's init
            // scripts and its messages reach this tab's IPC handler, where
            // they are trusted only as a website from another origin (see
            // app::handle_tab_ipc).
            .with_new_window_req_handler(move |url, features| {
                #[cfg(target_os = "macos")]
                if opens_little_window(
                    &nav_popup.popup_opener_url(),
                    features.size.is_some(),
                    &url,
                ) && let Some(webview) = crate::macos_menu::open_little_window(&features)
                {
                    return NewWindowResponse::Create { webview };
                }
                #[cfg(not(target_os = "macos"))]
                let _ = (&nav_popup, features);
                let _ = proxy_newwin.send_event(UserEvent::NewWindowRequested { url });
                NewWindowResponse::Deny
            })
            .with_custom_protocol("eclipse".into(), move |_id, req| {
                crate::ui_assets::handle_eclipse_protocol(req)
            })
            .with_url(&target_url);

        #[cfg(target_os = "macos")]
        let builder = {
            use wry::WebViewBuilderExtDarwin;
            let proxy_terminated = proxy.clone();
            builder.with_on_web_content_process_terminate_handler(move || {
                let _ = proxy_terminated.send_event(UserEvent::WebContentTerminated { tab_id });
            })
        };

        // No background colour: wry would turn off drawsBackground, and pages
        // without their own CSS background must render on white, like Safari.
        let webview = builder
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
            nav.view.store(ptr as usize, Ordering::Relaxed);
            nav.native_adblock.store(
                if is_adblock_active {
                    NATIVE_ADBLOCK_ON
                } else {
                    NATIVE_ADBLOCK_OFF
                },
                Ordering::Relaxed,
            );
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

    /// The committed document is an internal eclipse:// page (not the
    /// error page, and not merely the target of a pending load).
    pub fn committed_is_internal(&self) -> bool {
        self.webview.is_some() && self.nav.document_trust().internal
    }

    /// URL of the document on screen, as WebKit reports it (empty before
    /// the first commit; the failed URL while the error page is shown).
    pub fn committed_url(&self) -> String {
        self.nav.committed_url()
    }

    /// The committed document changed its own URL (pushState, a fragment).
    pub fn note_same_document_url(&self, url: &str) {
        self.nav.set_committed_url(url);
    }

    /// Asks the internal page on screen to post its pending (debounced)
    /// edits now. `on_done` runs once the hook has returned, after every
    /// message it posted has been delivered. Returns false, without calling
    /// `on_done`, when no internal page is on screen.
    pub fn flush_internal_page<F>(&self, on_done: F) -> bool
    where
        F: Fn(String) + Send + 'static,
    {
        if !self.committed_is_internal() {
            return false;
        }
        self.evaluate_with_callback(FLUSH_SCRIPT, on_done);
        true
    }

    /// Before the browser navigates the tab away from an internal page: its
    /// edits go out while it is still the committed document, so they are
    /// still trusted as coming from it.
    fn flush_before_leaving(&self) {
        if self.committed_is_internal() {
            self.evaluate_script(FLUSH_SCRIPT);
        }
    }

    /// A media (or other plug-in) document took over the load: WebKit
    /// reports no didFinish for it, so the load is marked finished here.
    pub fn mark_load_finished(&self) {
        self.nav.set_browser_target(None);
        self.nav
            .browser_load_pending
            .store(false, Ordering::Relaxed);
        self.nav.note_finished();
    }

    /// The load the browser started ended without a new document (it became
    /// a download, the policy refused it, or it was stopped): the tab goes
    /// back to describing the document still on screen. Returns false when
    /// no browser-started load was pending (nothing changed). The caller
    /// refreshes the title.
    pub fn abandon_pending_load(&mut self) -> bool {
        if !self.nav.browser_load_pending.swap(false, Ordering::Relaxed) {
            return false;
        }
        self.nav.set_browser_target(None);
        self.is_loading = false;
        self.set_loading_indicator(false);
        let showing_error = self.nav.showing_error_page.load(Ordering::Relaxed);
        let committed = self.nav.committed_url();
        if self.webview.is_none() || committed.is_empty() {
            // Nothing on screen yet: the tab keeps the URL it was opened for.
            return true;
        }
        let canonical = canonicalize_eclipse_url(&committed);
        self.favicon = get_default_favicon(&canonical);
        self.favicon_url = if showing_error {
            None
        } else {
            get_favicon_url(&canonical)
        };
        self.url = canonical;
        true
    }

    pub fn load_url(&mut self, url: &str) {
        self.flush_before_leaving();
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
            let target = tab_webview_url(&canonical);
            self.nav.begin_browser_load(&target);
            let _ = wv.load_url(&target);
        }
    }

    /// Reloads the page. A reload of an internal page passes the policy as
    /// the current document; a WebKit reload never needs a grant.
    pub fn reload(&self) {
        if let Some(ref wv) = self.webview {
            self.set_loading_indicator(true);
            if self.nav.showing_error_page.load(Ordering::Relaxed) {
                // Reloading would only redisplay the error page.
                let target = tab_webview_url(&self.url);
                self.nav.begin_browser_load(&target);
                let _ = wv.load_url(&target);
                return;
            }
            let _ = wv.reload();
        }
    }

    /// Stops loading. Returns true when a load the browser had started was
    /// abandoned, so the tab went back to the document on screen.
    pub fn stop_loading(&mut self) -> bool {
        self.evaluate_script(
            "window.stop(); window.__eclipseSetLoading && window.__eclipseSetLoading(false);",
        );
        self.is_loading = false;
        self.abandon_pending_load()
    }

    /// History navigation. WebKit's back/forward reaches the navigation
    /// policy as a main-frame back/forward action, which may return to an
    /// internal page without any grant from here.
    pub fn go_back(&self) {
        if let Some(ref wv) = self.webview {
            if wv.can_go_back().unwrap_or(false) {
                self.flush_before_leaving();
                let _ = wv.go_back();
            } else {
                let _ = wv.evaluate_script("if (window.history) { window.history.back(); }");
            }
        }
    }

    pub fn go_forward(&self) {
        if let Some(ref wv) = self.webview {
            if wv.can_go_forward().unwrap_or(false) {
                self.flush_before_leaving();
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
            js_string(&self.control_token),
            js_string(action)
        );
        self.evaluate_script(&js);
    }

    /// Opens the native print panel for the page (WebKit ignores a scripted
    /// `window.print()` without a UI-delegate hook).
    pub fn print(&self) {
        if let Some(ref wv) = self.webview {
            let _ = wv.print();
        }
    }

    pub fn open_devtools(&self) {
        if let Some(ref wv) = self.webview {
            wv.open_devtools();
        }
    }

    pub fn apply_custom_css(&self, css: &str) {
        self.evaluate_script(&build_custom_css_script(css));
    }

    /// Replaces the page with a self-contained error page for a navigation
    /// that failed. The tab keeps reporting `failed_url` as its URL and title,
    /// and "Try Again" navigates there again (plain JS, no IPC).
    pub fn show_error_page(&mut self, failed_url: &str, description: &str) {
        if self.webview.is_none() {
            return;
        }
        self.nav.showing_error_page.store(true, Ordering::Relaxed);
        self.nav.current_is_internal.store(false, Ordering::Relaxed);
        self.nav.left_internal.store(false, Ordering::Relaxed);
        self.nav.set_browser_target(None);
        self.nav
            .browser_load_pending
            .store(false, Ordering::Relaxed);
        // What the tab reports while the error page is up (and what an
        // abandoned load from it returns to).
        self.nav.set_committed_url(failed_url);
        self.url = failed_url.to_string();
        self.title = failed_url.to_string();
        self.favicon = get_default_favicon(failed_url);
        self.favicon_url = None;
        self.is_loading = false;
        self.is_audible = false;
        self.set_loading_indicator(false);
        if let Some(ref wv) = self.webview {
            let _ = wv.load_html(&build_error_page_html(failed_url, description));
        }
    }

    /// Brings the page in line with `config` (C9). Ordinary websites never
    /// receive config or user data: they only get the shields switch and the
    /// native ad blocker, each only when its effective state changes, plus
    /// matching extension content scripts and custom JS once per document.
    /// Internal eclipse:// pages get `config.to_page_json()` through
    /// `onEclipseConfigUpdated`, once per (revision, active URL) per document.
    pub fn sync_config(&self, config: &BrowserConfig, revision: u64, active_url: Option<&str>) {
        if self.webview.is_none() {
            return;
        }
        let adblock = AdblockPolicy::for_space(config, &self.workspace);
        self.nav.set_adblock_policy(&adblock);

        // The rule list applies to the tab's next loads, so it follows the
        // tab's target URL rather than the document still on screen (links
        // a page follows are switched when the navigation is allowed).
        if !self.url.starts_with("eclipse://") {
            self.nav.set_native_adblock(adblock.blocks(&self.url));
        }

        let nav = &self.nav;
        let epoch = nav.epoch.load(Ordering::Relaxed);
        // Nothing has committed yet (the init scripts cover the first
        // document), or an error page is up.
        if epoch == 0 || nav.showing_error_page.load(Ordering::Relaxed) {
            return;
        }
        if self.sync.epoch.get() != epoch {
            self.sync.epoch.set(epoch);
            self.sync.config.replace(None);
            self.sync.shields.set(None);
            self.sync.page_scripts_done.set(false);
        }
        // A push made before the document finished loading may have run
        // before the page's own scripts; it is repeated once on finish.
        let finished = nav.finished_epoch.load(Ordering::Relaxed) == epoch;

        if nav.current_is_internal.load(Ordering::Relaxed) {
            let act_url = active_url.unwrap_or(&self.url);
            let up_to_date = revision != UNVERSIONED
                && matches!(&*self.sync.config.borrow(), Some((rev, url)) if *rev == revision && url == act_url);
            if !up_to_date {
                self.evaluate_script(&build_config_push_script(config, act_url));
                if finished {
                    self.sync
                        .config
                        .replace(Some((revision, act_url.to_string())));
                }
            }
            return;
        }

        let committed = nav.committed_url();
        let desired = adblock.blocks(&committed);
        let current = self
            .sync
            .shields
            .get()
            .unwrap_or_else(|| self.init_adblock.blocks(&committed));
        if current != desired {
            self.evaluate_script(&format!(
                "typeof window.__eclipseShields === 'function' && window.__eclipseShields({}, {desired});",
                js_string(&self.control_token)
            ));
            if finished {
                self.sync.shields.set(Some(desired));
            }
        }

        if !self.sync.page_scripts_done.get() {
            self.sync.page_scripts_done.set(true);
            for script in build_page_scripts(&committed, config) {
                self.evaluate_script(&script);
            }
            if config.drm_fairplay_enabled && !self.init_drm {
                self.evaluate_script(&build_drm_script(true));
            }
            if config.custom_css != self.init_custom_css {
                self.evaluate_script(&build_custom_css_script(&config.custom_css));
            }
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

fn is_eclipse_scheme(url: &str) -> bool {
    url.get(..8)
        .is_some_and(|scheme| scheme.eq_ignore_ascii_case("eclipse:"))
}

fn is_newtab_url(url: &str) -> bool {
    matches!(
        url.trim_end_matches('/'),
        "eclipse://localhost/newtab" | "eclipse://newtab" | "eclipse://localhost"
    )
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
        return "eclipse://newtab".to_string();
    }
    url.to_string()
}

/// Trimmed, lowercased, non-empty entries of a site list.
fn normalized_domains(domains: &[String]) -> Vec<String> {
    domains
        .iter()
        .map(|domain| domain.trim().to_lowercase())
        .filter(|domain| !domain.is_empty())
        .collect()
}

/// `url`'s host is one of `domains` (normalized) or a subdomain of one.
fn url_host_in_domains(url: &str, domains: &[String]) -> bool {
    let Some(host) = Url::parse(url)
        .ok()
        .and_then(|parsed| parsed.host_str().map(str::to_lowercase))
    else {
        return false;
    };
    domains
        .iter()
        .any(|domain| host == *domain || host.ends_with(&format!(".{domain}")))
}

fn is_http_url(url: &str) -> bool {
    url.starts_with("http://") || url.starts_with("https://")
}

/// Whether a `window.open` request opens in a Little Eclipse window (a small
/// native window that keeps `window.opener`, see menu.m) instead of a tab:
/// the opener's committed document is a web page (`opener_document` is
/// empty for internal pages and the error page), the page asked for a size
/// (sign-in and payment popups do; plain `target=_blank` links do not), and
/// the target is a web page or blank (popups often open empty and navigate
/// once the opener has written to them).
pub(crate) fn opens_little_window(opener_document: &str, has_size: bool, target_url: &str) -> bool {
    let is_web = |url: &str| {
        Url::parse(url).is_ok_and(|parsed| matches!(parsed.scheme(), "http" | "https"))
    };
    has_size
        && is_web(opener_document)
        && (target_url.is_empty() || target_url == "about:blank" || is_web(target_url))
}

/// Two eclipse:// URLs name the same page (WebKit may add a trailing slash).
fn same_eclipse_url(a: &str, b: &str) -> bool {
    a.trim_end_matches('/')
        .eq_ignore_ascii_case(b.trim_end_matches('/'))
}

/// What the navigation policy is being asked about (see menu.m).
#[derive(Debug, Clone, Copy, Default)]
pub struct PolicyContext {
    /// The navigation targets the tab's main frame (not a subframe or a new
    /// window).
    pub main_frame: bool,
    /// A main-frame back/forward history navigation (toolbar, swipe, context
    /// menu or `history.back()`): it can only reach an entry the tab already
    /// visited.
    pub back_forward: bool,
}

fn navigation_policy_context() -> PolicyContext {
    #[cfg(target_os = "macos")]
    {
        crate::macos_menu::navigation_policy_context()
    }
    #[cfg(not(target_os = "macos"))]
    {
        PolicyContext::default()
    }
}

/// What a main-frame navigation error WebKit reported means for the tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NavigationError {
    /// Not this load failing: it was superseded by another navigation, or
    /// the WebContent process died (reported by the terminate handler).
    Ignore,
    /// WebKitErrorPlugInWillHandleLoad (204): a standalone media document
    /// (a direct .mp4/.mp3 URL) took over the load after it committed. The
    /// page is fine and plays; the load counts as finished.
    HandledByPlugin,
    /// The load ended without a new document and with nothing to report: it
    /// became a download or the navigation policy refused it (102), or
    /// WebKit cannot open the scheme (mailto:, app links).
    Interrupted,
    /// A real failure (DNS, offline, TLS, ...). `committed`: the new
    /// document had already committed, so (part of) it is on screen.
    Failed { committed: bool },
}

pub fn classify_navigation_error(
    committed: bool,
    domain: &str,
    code: i64,
    url: &str,
) -> NavigationError {
    match (domain, code) {
        // NSURLErrorCancelled.
        ("NSURLErrorDomain", -999) => return NavigationError::Ignore,
        // WKErrorWebContentProcessTerminated.
        ("WKErrorDomain", 2) => return NavigationError::Ignore,
        ("WebKitErrorDomain", 204) => return NavigationError::HandledByPlugin,
        // WebKitErrorFrameLoadInterruptedByPolicyChange.
        ("WebKitErrorDomain", 102) => return NavigationError::Interrupted,
        _ => {}
    }
    if let Ok(parsed) = Url::parse(url)
        && !matches!(parsed.scheme(), "http" | "https" | "file")
    {
        return NavigationError::Interrupted;
    }
    NavigationError::Failed { committed }
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

/// Extension content scripts, their CSS and the user's custom JS for one
/// committed document, each as its own script for `evaluate_script` (which
/// page CSP does not apply to, unlike `new Function`). Matching happens per
/// navigation against the real URL, so installs, toggles and new-tab tabs
/// pick up changes on the next load. Only http(s) pages get any of it: never
/// eclipse:// pages, which hold the privileged IPC helper.
///
/// `chrome.storage` here is per document and in memory: persisting it would
/// mean storing extension data in the website's own localStorage, where the
/// site can read and rewrite it.
fn build_page_scripts(url: &str, config: &BrowserConfig) -> Vec<String> {
    let mut scripts = Vec::new();
    if !config.webextensions_enabled {
        return scripts;
    }
    let Ok(parsed) = Url::parse(url) else {
        return scripts;
    };
    if parsed.scheme() != "http" && parsed.scheme() != "https" {
        return scripts;
    }
    let host_js = js_string(parsed.host_str().unwrap_or(""));
    let guard = format!(
        "if (location.protocol !== 'http:' && location.protocol !== 'https:') return;\n\
         if (location.hostname !== {host_js}) return;"
    );

    let mut ext_scripts = Vec::new();
    let mut ext_styles = Vec::new();
    for ext in config.installed_extensions.iter().filter(|ext| ext.enabled) {
        if !ext
            .matches
            .iter()
            .any(|pattern| extension_matches_url(pattern, url))
        {
            continue;
        }
        ext_scripts.extend(ext.content_scripts.iter().filter(|s| !s.trim().is_empty()));
        ext_styles.extend(ext.content_css.iter().filter(|s| !s.trim().is_empty()));
    }

    if !ext_styles.is_empty() {
        let encoded_styles = serde_json::to_string(&ext_styles).unwrap_or_else(|_| "[]".into());
        scripts.push(format!(
            r##"(function () {{
                {guard}
                {encoded_styles}.forEach(function (css) {{
                    var styleEl = document.createElement('style');
                    styleEl.className = '__eclipse_extension_style';
                    styleEl.textContent = css;
                    (document.head || document.documentElement).appendChild(styleEl);
                }});
            }})();"##
        ));
    }

    if !ext_scripts.is_empty() {
        scripts.push(format!(
            "(function () {{\n{guard}\n{EXTENSION_RUNTIME_SHIM}\n}})();"
        ));
        for code in ext_scripts {
            scripts.push(wrap_user_code(
                &guard,
                code,
                "Eclipse Extension Script Error",
            ));
        }
    }

    if !config.custom_js.trim().is_empty() {
        scripts.push(wrap_user_code(
            &guard,
            &config.custom_js,
            "Eclipse UserScript Error",
        ));
    }
    scripts
}

/// Runs third-party code once the DOM is ready (like a content script's
/// default `document_idle`). The code is inlined as a function body, so a
/// syntax error only fails this one evaluation.
fn wrap_user_code(guard: &str, code: &str, label: &str) -> String {
    let label_js = js_string(label);
    format!(
        "(function () {{\n{guard}\nfunction run() {{\ntry {{\n(function () {{\n{code}\n}}).call(window);\n}} catch (err) {{\nconsole.warn({label_js}, err);\n}}\n}}\nif (document.readyState === 'loading') document.addEventListener('DOMContentLoaded', run, {{ once: true }});\nelse run();\n}})();"
    )
}

const EXTENSION_RUNTIME_SHIM: &str = r##"if (!window.chrome) window.chrome = {};
if (!window.chrome.runtime) {
    var storageMap = {};
    var runtime = {
        id: 'eclipse-webextension-engine',
        getURL: function (path) {
            return 'eclipse://extensions/' + String(path || '').replace(/^\//, '');
        },
        getManifest: function () {
            return { name: 'Eclipse WebExtension Engine', version: '2.0.0', manifest_version: 3 };
        },
        sendMessage: function (msg, cb) {
            if (typeof cb === 'function') setTimeout(function () { cb(undefined); }, 0);
            return Promise.resolve(undefined);
        },
        onMessage: {
            addListener: function () {},
            removeListener: function () {},
            hasListener: function () { return false; }
        },
        connect: function () {
            var port = {
                name: '',
                postMessage: function () {},
                onMessage: { addListener: function () {}, removeListener: function () {} },
                onDisconnect: {
                    addListener: function (fn) {
                        if (typeof fn === 'function') setTimeout(function () { fn(port); }, 0);
                    },
                    removeListener: function () {}
                }
            };
            return port;
        }
    };
    var storage = {
        local: {
            get: function (keys, cb) {
                var result = {};
                if (typeof keys === 'string') {
                    result[keys] = storageMap[keys];
                } else if (Array.isArray(keys)) {
                    keys.forEach(function (k) { result[k] = storageMap[k]; });
                } else if (keys && typeof keys === 'object') {
                    Object.keys(keys).forEach(function (k) {
                        result[k] = storageMap[k] !== undefined ? storageMap[k] : keys[k];
                    });
                } else {
                    result = Object.assign({}, storageMap);
                }
                if (typeof cb === 'function') cb(result);
                return Promise.resolve(result);
            },
            set: function (items, cb) {
                if (items && typeof items === 'object') Object.assign(storageMap, items);
                if (typeof cb === 'function') cb();
                return Promise.resolve();
            },
            remove: function (keys, cb) {
                (Array.isArray(keys) ? keys : [keys]).forEach(function (k) { delete storageMap[k]; });
                if (typeof cb === 'function') cb();
                return Promise.resolve();
            },
            clear: function (cb) {
                storageMap = {};
                if (typeof cb === 'function') cb();
                return Promise.resolve();
            }
        },
        sync: null
    };
    storage.sync = storage.local;
    var tabs = {
        query: function (info, cb) {
            var activeTab = [{ id: 1, active: true, url: location.href, title: document.title }];
            if (typeof cb === 'function') cb(activeTab);
            return Promise.resolve(activeTab);
        },
        sendMessage: function (tabId, msg, cb) {
            if (typeof cb === 'function') cb(undefined);
            return Promise.resolve(undefined);
        },
        create: function (props, cb) {
            if (props && props.url) window.open(props.url, '_blank');
            if (typeof cb === 'function') cb({ id: 2, url: props ? props.url : '' });
            return Promise.resolve({ id: 2, url: props ? props.url : '' });
        }
    };
    var action = {
        setTitle: function () {},
        setIcon: function () {},
        setBadgeText: function () {},
        setBadgeBackgroundColor: function () {},
        onClicked: { addListener: function () {}, removeListener: function () {} }
    };
    window.chrome.runtime = runtime;
    window.chrome.storage = storage;
    window.chrome.tabs = tabs;
    window.chrome.action = action;
    window.chrome.browserAction = action;
    window.chrome.extension = { getURL: runtime.getURL, inIncognitoContext: false };
    if (!window.browser) window.browser = window.chrome;
}"##;

pub fn build_custom_css_script(css: &str) -> String {
    let encoded_css = serde_json::to_string(css).unwrap_or_else(|_| "\"\"".into());
    format!(
        r##"(function () {{
            if (location.protocol === 'eclipse:') return;
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

/// Seeds `window.__ECLIPSE_CONFIG__` at document start for tabs created on an
/// internal page, so those pages render with settings on first paint. Tabs
/// created on a website get no seed at all; their internal pages receive the
/// config from `sync_config` once they commit.
fn build_config_seed_script(config: &BrowserConfig, tab_url: &str) -> String {
    if !tab_url.starts_with("eclipse://") {
        return String::new();
    }
    let json = config.to_page_json();
    let act_url = js_string(tab_url);
    format!(
        r##"(function () {{
            if (location.protocol !== 'eclipse:') return;
            window.__ECLIPSE_CONFIG__ = {json};
            window.__ECLIPSE_ACTIVE_URL__ = {act_url};
        }})();"##
    )
}

fn build_config_push_script(config: &BrowserConfig, active_url: &str) -> String {
    let json = config.to_page_json();
    let act_url = js_string(active_url);
    format!(
        r##"(function () {{
            if (location.protocol !== 'eclipse:') return;
            var cfg = {json};
            var actUrl = {act_url};
            window.__ECLIPSE_CONFIG__ = cfg;
            window.__ECLIPSE_ACTIVE_URL__ = actUrl;
            if (typeof window.onEclipseConfigUpdated === 'function') {{
                window.onEclipseConfigUpdated(cfg, actUrl);
            }}
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

fn build_bridge_script(token: &str, control: &str) -> String {
    BRIDGE_SCRIPT
        .replace("__ECLIPSE_TOKEN__", token)
        .replace("__ECLIPSE_CONTROL__", control)
}

/// The in-page shields of every document the webview loads start from
/// `policy` applied to that document's own host, so a site the user
/// excluded is not filtered even when the tab was created elsewhere.
fn build_shields_script(control: &str, policy: &AdblockPolicy) -> String {
    let initial = if policy.enabled {
        let domains =
            serde_json::to_string(&policy.disabled_domains).unwrap_or_else(|_| "[]".into());
        format!(
            "(function (host, domains) {{ host = String(host || '').toLowerCase(); \
             for (var i = 0; i < domains.length; i++) {{ var d = domains[i]; \
             if (host === d || (host.length > d.length && host.slice(-d.length - 1) === '.' + d)) return false; }} \
             return true; }})(location.hostname, {domains})"
        )
    } else {
        "false".to_string()
    };
    SHIELDS_SCRIPT
        .replace("__ECLIPSE_CONTROL__", control)
        .replace("__ECLIPSE_ENABLED__", &initial)
}

/// One init-script part, isolated so a runtime error in it cannot abort the
/// parts after it.
fn guard_script(script: &str) -> String {
    format!("try {{\n{script}\n}} catch (_eclipseInitError) {{}}")
}

/// A self-contained page for a failed navigation. Everything from the URL and
/// the error is escaped; "Try Again" is a plain navigation, with no IPC.
fn build_error_page_html(failed_url: &str, description: &str) -> String {
    let description = if description.trim().is_empty() {
        "The page could not be loaded."
    } else {
        description.trim()
    };
    let parsed = Url::parse(failed_url).ok();
    let host = parsed
        .as_ref()
        .and_then(|u| u.host_str())
        .unwrap_or(failed_url);
    let retry = parsed
        .as_ref()
        .is_some_and(|u| u.scheme() == "http" || u.scheme() == "https");
    let retry_markup = if retry {
        format!(
            r#"<button id="retry" type="button">Try Again</button>
<script>
document.getElementById('retry').addEventListener('click', function () {{ location.href = {}; }});
</script>"#,
            js_string(failed_url).replace('<', "\\u003c")
        )
    } else {
        String::new()
    };
    format!(
        r#"<!doctype html>
<html>
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>{title}</title>
<style>
:root {{ color-scheme: light dark; }}
body {{ margin: 0; min-height: 100vh; display: flex; align-items: center; justify-content: center;
  font: 14px -apple-system, BlinkMacSystemFont, "SF Pro Text", sans-serif; background: #f5f5f7; color: #1d1d1f; }}
main {{ max-width: 520px; padding: 32px; text-align: center; }}
h1 {{ font-size: 22px; font-weight: 600; margin: 0 0 12px; }}
p {{ margin: 0 0 10px; line-height: 1.45; color: #515154; }}
.url {{ font: 12px ui-monospace, SFMono-Regular, Menlo, monospace; word-break: break-all; color: #6e6e73; }}
button {{ margin-top: 18px; padding: 8px 18px; border: 0; border-radius: 8px; font: inherit; font-weight: 500;
  background: #0071e3; color: #fff; cursor: pointer; }}
@media (prefers-color-scheme: dark) {{
  body {{ background: #1c1c1e; color: #f5f5f7; }}
  p {{ color: #c7c7cc; }}
  .url {{ color: #98989d; }}
}}
</style>
</head>
<body>
<main>
<h1>Can’t open {host}</h1>
<p>{description}</p>
<p class="url">{url}</p>
{retry_markup}
</main>
</body>
</html>"#,
        title = html_escape(failed_url),
        host = html_escape(host),
        description = html_escape(description),
        url = html_escape(failed_url),
    )
}

fn html_escape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            ch => out.push(ch),
        }
    }
    out
}

/// 128 bits of OS randomness, used for the IPC token and the control secret.
/// Neither is ever placed on a page-reachable object (see bridge.js); Rust
/// also authorizes each message by the frame URL WebKit reports, not by the
/// token alone.
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

/// `host` is `domain` itself or one of its subdomains (not merely containing
/// it, so github.com.login-verify.io does not get the GitHub glyph).
fn host_is(host: &str, domain: &str) -> bool {
    host == domain
        || host
            .strip_suffix(domain)
            .is_some_and(|prefix| prefix.ends_with('.'))
}

pub fn get_default_favicon(url: &str) -> String {
    if url.starts_with("eclipse://") {
        return "eclipse".into();
    }
    if let Ok(parsed) = Url::parse(url)
        && let Some(host) = parsed.host_str()
    {
        let h = host.to_lowercase();
        if host_is(&h, "netflix.com") {
            return "netflix".into();
        } else if host_is(&h, "youtube.com") || host_is(&h, "youtu.be") {
            return "youtube".into();
        } else if host_is(&h, "github.com") {
            return "github".into();
        } else if host_is(&h, "reddit.com") {
            return "reddit".into();
        } else if host_is(&h, "apple.com") {
            return "apple".into();
        } else if host_is(&h, "spotify.com") {
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

/// A download WebKit announced, by original request URL. `path` is where it
/// is being written, or None when it was refused as a mis-detected page.
struct PendingDownload {
    seq: u64,
    url: String,
    path: Option<PathBuf>,
}

/// Identifies a download from its start to its completion event.
static NEXT_DOWNLOAD_SEQ: AtomicU64 = AtomicU64::new(1);

/// Shared by every tab: destinations stay reserved until WebKit reports the
/// download finished, so two concurrent downloads never pick the same file.
static PENDING_DOWNLOADS: Mutex<Vec<PendingDownload>> = Mutex::new(Vec::new());
const PENDING_DOWNLOAD_LIMIT: usize = 64;

fn pending_downloads() -> std::sync::MutexGuard<'static, Vec<PendingDownload>> {
    PENDING_DOWNLOADS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn remember_download(
    pending: &mut Vec<PendingDownload>,
    seq: u64,
    url: String,
    path: Option<PathBuf>,
) {
    if pending.len() >= PENDING_DOWNLOAD_LIMIT {
        pending.remove(0);
    }
    pending.push(PendingDownload { seq, url, path });
}

fn unique_download_path(dir: &Path, file_name: &str, reserved: &[&Path]) -> PathBuf {
    let taken = |candidate: &Path| candidate.exists() || reserved.contains(&candidate);
    let safe = sanitize_file_name(file_name);
    let mut candidate = dir.join(&safe);
    if !taken(&candidate) {
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
        if !taken(&candidate) {
            return candidate;
        }
    }
    dir.join(safe)
}

const MAX_FILE_NAME_CHARS: usize = 180;

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
        return "download".into();
    }
    if trimmed.chars().count() <= MAX_FILE_NAME_CHARS {
        return trimmed.to_string();
    }
    // Shorten the stem, keep the extension.
    let path = Path::new(trimmed);
    if let (Some(stem), Some(ext)) = (path.file_stem(), path.extension()) {
        let ext = ext.to_string_lossy();
        let ext_len = ext.chars().count();
        if ext_len <= 16 {
            let stem: String = stem
                .to_string_lossy()
                .chars()
                .take(MAX_FILE_NAME_CHARS - ext_len - 1)
                .collect();
            return format!("{}.{ext}", stem.trim_end());
        }
    }
    trimmed.chars().take(MAX_FILE_NAME_CHARS).collect()
}

/// The file name for a download, or None for a page load that WebKit could
/// not display and handed over as a "download" (named after the host, or
/// with no name at all). Real files are never refused for their extension or
/// length: attachments named .html/.js/.ai, extensionless binaries named by
/// Content-Disposition, and over-long names (shortened) all download.
pub fn resolve_download_name(suggested_name: &str, url_str: &str) -> Option<String> {
    let parsed = Url::parse(url_str).ok();
    let host = parsed
        .as_ref()
        .and_then(|u| u.host_str())
        .map(|h| h.to_ascii_lowercase())
        .unwrap_or_default();
    let last_segment = parsed
        .as_ref()
        .and_then(|u| u.path_segments())
        .and_then(|mut segments| segments.next_back())
        .map(percent_decode)
        .unwrap_or_default();

    // WebKit's suggestion comes from Content-Disposition, the <a download>
    // name, or the last URL segment. Without an extension it only counts when
    // it is not just that segment (a streaming endpoint such as YouTube's
    // "videoplayback" loaded in a frame).
    if let Some(name) = clean_download_name(suggested_name, &host) {
        let from_url = name == sanitize_file_name(&last_segment);
        if Path::new(&name).extension().is_some() || !from_url {
            return Some(name);
        }
    }
    if let Some(name) = clean_download_name(&last_segment, &host)
        && Path::new(&name).extension().is_some()
    {
        return Some(name);
    }
    // Script-generated files have no useful name but are always real.
    match parsed.as_ref().map(|u| u.scheme()) {
        Some("blob") | Some("data") => Some("download".into()),
        _ => None,
    }
}

fn clean_download_name(candidate: &str, host: &str) -> Option<String> {
    let clean = sanitize_file_name(&percent_decode(candidate));
    let lower = clean.to_ascii_lowercase();
    if lower == "download" || lower == "unknown" {
        return None;
    }
    let bare_host = host.trim_start_matches("www.");
    if !bare_host.is_empty() && lower.trim_start_matches("www.") == bare_host {
        return None;
    }
    Some(clean)
}

/// Name for a download whose real name is unknown (never empty).
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
    use crate::config::ExtensionConfig;

    #[test]
    fn sized_popups_from_web_pages_open_little_windows() {
        let page = "https://shop.example/checkout";
        assert!(opens_little_window(page, true, "https://accounts.example/oauth?x=1"));
        assert!(opens_little_window("http://127.0.0.1:8000/opener.html", true, "http://127.0.0.1:8000/popup.html"));
        // Popups that start blank and navigate later.
        assert!(opens_little_window(page, true, ""));
        assert!(opens_little_window(page, true, "about:blank"));
    }

    #[test]
    fn other_popups_keep_opening_as_tabs() {
        let page = "https://shop.example/checkout";
        // No size: an ordinary target=_blank link or window.open(url).
        assert!(!opens_little_window(page, false, "https://accounts.example/"));
        // Internal pages and the error page never get popups (their opener
        // URL is empty), nor does anything that is not a web page.
        assert!(!opens_little_window("", true, "https://accounts.example/"));
        assert!(!opens_little_window("eclipse://localhost/settings", true, "https://a.example/"));
        assert!(!opens_little_window("about:blank", true, "https://a.example/"));
        assert!(!opens_little_window("file:///tmp/a.html", true, "https://a.example/"));
        // Targets that are not web pages.
        assert!(!opens_little_window(page, true, "eclipse://settings"));
        assert!(!opens_little_window(page, true, "javascript:alert(1)"));
        assert!(!opens_little_window(page, true, "data:text/html,hi"));
        assert!(!opens_little_window(page, true, "file:///etc/passwd"));
        assert!(!opens_little_window(page, true, "about:srcdoc"));
        assert!(!opens_little_window(page, true, "mailto:a@b.example"));
    }

    #[test]
    fn popup_opener_is_only_a_committed_web_page() {
        let nav = NavState::default();
        assert_eq!(nav.popup_opener_url(), "");
        nav.note_commit("https://shop.example/");
        assert_eq!(nav.popup_opener_url(), "https://shop.example/");
        nav.showing_error_page.store(true, Ordering::Relaxed);
        assert_eq!(nav.popup_opener_url(), "");
        nav.note_commit("eclipse://localhost/newtab");
        assert_eq!(nav.popup_opener_url(), "");
    }

    fn is_legitimate_download(suggested_name: &str, url_str: &str) -> bool {
        resolve_download_name(suggested_name, url_str).is_some()
    }

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
            canonicalize_eclipse_url("eclipse://sidebar"),
            "eclipse://newtab"
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
    fn brand_favicons_need_the_real_domain() {
        assert_eq!(get_default_favicon("https://github.com/x"), "github");
        assert_eq!(get_default_favicon("https://gist.github.com/x"), "github");
        assert_eq!(get_default_favicon("https://www.apple.com"), "apple");
        assert_eq!(get_default_favicon("https://youtu.be/abc"), "youtube");
        assert_eq!(
            get_default_favicon("https://github.com.login-verify.io/"),
            "globe"
        );
        assert_eq!(get_default_favicon("https://pineapple.com/"), "globe");
        assert_eq!(
            get_default_favicon("https://applecare-support.com/"),
            "globe"
        );
        assert_eq!(get_default_favicon("https://notgithub.com/"), "globe");
    }

    #[test]
    fn download_validation_rejects_pages_and_accepts_files() {
        // Pages WebKit handed over as downloads.
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
            "videoplayback",
            "https://rr1.googlevideo.com/videoplayback?id=1"
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

        // Files the old extension/TLD heuristics cancelled.
        assert!(is_legitimate_download(
            "logo.ai",
            "https://cdn.example.com/x?id=3"
        ));
        assert!(is_legitimate_download(
            "download-manager-setup.dmg",
            "https://example.com/get"
        ));
        assert!(is_legitimate_download(
            "MyTool.app.zip",
            "https://example.com/t"
        ));
        assert!(is_legitimate_download(
            "my.site.zip",
            "https://example.com/z"
        ));
        assert!(is_legitimate_download(
            "index.html",
            "https://example.com/index.html"
        ));
        assert!(is_legitimate_download(
            "script.js",
            "https://example.com/get?f=1"
        ));
        assert!(is_legitimate_download(
            "theme.css",
            "https://example.com/theme.css"
        ));
        // Extensionless name from Content-Disposition.
        assert_eq!(
            resolve_download_name("ffmpeg", "https://example.com/download?id=7"),
            Some("ffmpeg".into())
        );
        // Script-generated exports.
        assert_eq!(
            resolve_download_name("Unknown", "blob:https://example.com/1f2e"),
            Some("download".into())
        );
        assert_eq!(
            resolve_download_name("export.csv", "blob:https://example.com/1f2e"),
            Some("export.csv".into())
        );

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
        assert_eq!(download_file_name("https://example.com/"), "download");
        assert_eq!(download_file_name("https://example.com/a/b.zip"), "b.zip");
    }

    #[test]
    fn long_download_names_keep_their_extension() {
        let long = format!("{}.pdf", "a".repeat(400));
        let name = resolve_download_name(&long, "https://example.com/x").unwrap();
        assert!(name.ends_with(".pdf"));
        assert_eq!(name.chars().count(), MAX_FILE_NAME_CHARS);
    }

    #[test]
    fn reserved_download_paths_are_skipped() {
        let dir = std::env::temp_dir().join(format!("eclipse-dl-test-{}", generate_ipc_token()));
        fs::create_dir_all(&dir).unwrap();
        let first = dir.join("report.pdf");
        assert_eq!(unique_download_path(&dir, "report.pdf", &[]), first);
        let second = unique_download_path(&dir, "report.pdf", &[first.as_path()]);
        assert_eq!(second, dir.join("report (1).pdf"));
        let _ = fs::remove_dir_all(&dir);
    }

    const MAIN: PolicyContext = PolicyContext {
        main_frame: true,
        back_forward: false,
    };
    const BACK: PolicyContext = PolicyContext {
        main_frame: true,
        back_forward: true,
    };
    const SUBFRAME: PolicyContext = PolicyContext {
        main_frame: false,
        back_forward: false,
    };

    #[test]
    fn web_pages_cannot_open_internal_pages() {
        let nav = NavState::default();
        assert!(nav.allows_navigation("https://example.com/", MAIN));
        assert!(nav.allows_navigation("about:blank", MAIN));
        assert!(nav.allows_navigation("eclipse://localhost/newtab", MAIN));
        assert!(!nav.allows_navigation("eclipse://localhost/settings", MAIN));
        assert!(!nav.allows_navigation("ECLIPSE://localhost/notes", MAIN));

        nav.current_is_internal.store(true, Ordering::Relaxed);
        assert!(nav.allows_navigation("eclipse://localhost/easel", MAIN));
    }

    #[test]
    fn browser_loads_grant_exactly_their_target_once() {
        let nav = NavState::default();
        nav.begin_browser_load("eclipse://localhost/settings");
        // Another internal page (a racing subframe of the old document).
        assert!(!nav.allows_navigation("eclipse://localhost/notes", MAIN));
        // The target itself, once (WebKit may add a trailing slash).
        assert!(nav.allows_navigation("eclipse://localhost/settings/", MAIN));
        assert!(!nav.allows_navigation("eclipse://localhost/settings", MAIN));

        // A grant left by a load that never committed is gone after the
        // next commit, and a web load leaves none.
        nav.begin_browser_load("eclipse://localhost/notes");
        nav.note_commit("https://example.com/");
        assert!(!nav.allows_navigation("eclipse://localhost/notes", MAIN));
        nav.begin_browser_load("https://example.com/page");
        assert!(!nav.allows_navigation("eclipse://localhost/notes", MAIN));
    }

    #[test]
    fn subframes_neither_use_nor_spend_the_browser_grant() {
        let nav = NavState::default();
        nav.note_commit("https://example.com/");
        nav.begin_browser_load("eclipse://localhost/settings");
        // The website's iframe asks for the exact target first.
        assert!(!nav.allows_navigation("eclipse://localhost/settings", SUBFRAME));
        // The browser's own main-frame load still gets through, once.
        assert!(nav.allows_navigation("eclipse://localhost/settings", MAIN));
        assert!(!nav.allows_navigation("eclipse://localhost/settings", MAIN));
        // Frames of an internal page may load internal pages.
        nav.note_commit("eclipse://localhost/settings");
        assert!(nav.allows_navigation("eclipse://localhost/notes", SUBFRAME));
    }

    #[test]
    fn the_old_documents_finish_does_not_end_a_new_grant() {
        let nav = NavState::default();
        nav.note_commit("https://example.com/");
        nav.begin_browser_load("eclipse://localhost/notes");
        // The old page's didFinish arrives before the policy decision.
        nav.note_finished();
        assert!(nav.allows_navigation("eclipse://localhost/notes", MAIN));
    }

    #[test]
    fn back_forward_may_return_to_internal_pages() {
        let nav = NavState::default();
        nav.note_commit("https://example.com/");
        // Swipe or context-menu Back from a website to Settings.
        assert!(nav.allows_navigation("eclipse://localhost/settings", BACK));
        // Not a history navigation: still refused.
        assert!(!nav.allows_navigation("eclipse://localhost/settings", MAIN));
    }

    #[test]
    fn ipc_trust_follows_the_committed_document() {
        let nav = NavState::default();
        assert_eq!(nav.document_trust(), DocumentTrust::default());
        nav.note_commit("eclipse://localhost/notes");
        assert_eq!(
            nav.document_trust(),
            DocumentTrust {
                internal: true,
                left_internal: false
            }
        );
        // A load that never commits changes nothing.
        nav.begin_browser_load("https://example.com/file.dmg");
        assert!(nav.document_trust().internal);
        nav.note_commit("https://example.com/");
        assert_eq!(
            nav.document_trust(),
            DocumentTrust {
                internal: false,
                left_internal: true
            }
        );
        nav.note_commit("https://example.com/next");
        assert_eq!(nav.document_trust(), DocumentTrust::default());
        // The error page is never trusted.
        nav.note_commit("eclipse://localhost/settings");
        nav.showing_error_page.store(true, Ordering::Relaxed);
        assert_eq!(nav.document_trust(), DocumentTrust::default());
    }

    #[test]
    fn navigation_errors_are_classified_by_domain_and_code() {
        // A direct media URL: the media document took over after commit.
        assert_eq!(
            classify_navigation_error(
                true,
                "WebKitErrorDomain",
                204,
                "https://cdn.example/clip.mp4"
            ),
            NavigationError::HandledByPlugin
        );
        // The same code in another domain is an ordinary failure.
        assert_eq!(
            classify_navigation_error(false, "NSURLErrorDomain", 204, "https://example.com/"),
            NavigationError::Failed { committed: false }
        );
        assert_eq!(
            classify_navigation_error(false, "WebKitErrorDomain", 102, "https://example.com/a.dmg"),
            NavigationError::Interrupted
        );
        assert_eq!(
            classify_navigation_error(false, "WebKitErrorDomain", 101, "mailto:a@example.com"),
            NavigationError::Interrupted
        );
        assert_eq!(
            classify_navigation_error(false, "NSURLErrorDomain", -999, "https://example.com/"),
            NavigationError::Ignore
        );
        assert_eq!(
            classify_navigation_error(true, "WKErrorDomain", 2, "https://example.com/"),
            NavigationError::Ignore
        );
        assert_eq!(
            classify_navigation_error(
                false,
                "NSURLErrorDomain",
                -1003,
                "https://nonexistent.invalid/"
            ),
            NavigationError::Failed { committed: false }
        );
        assert_eq!(
            classify_navigation_error(true, "NSURLErrorDomain", -1005, "https://example.com/"),
            NavigationError::Failed { committed: true }
        );
        assert_eq!(
            classify_navigation_error(false, "NSURLErrorDomain", -1009, ""),
            NavigationError::Failed { committed: false }
        );
    }

    #[test]
    fn error_page_escapes_everything() {
        let html = build_error_page_html(
            "https://example.com/\"';</script><script>alert(1)</script>",
            "<b>Server</b> & co",
        );
        assert!(!html.contains("<script>alert(1)"));
        assert!(!html.contains("<b>Server</b>"));
        assert!(html.contains("&lt;b&gt;Server&lt;/b&gt; &amp; co"));
        assert!(html.contains("Try Again"));
        assert!(!html.contains("ipc"));

        let html = build_error_page_html("javascript:alert(1)", "");
        assert!(!html.contains("Try Again"));
        assert!(html.contains("The page could not be loaded."));
    }

    fn extension(matches: &[&str], script: &str) -> ExtensionConfig {
        ExtensionConfig {
            id: "ext".into(),
            name: "Ext".into(),
            version: "1".into(),
            description: String::new(),
            enabled: true,
            path: None,
            matches: matches.iter().map(|m| m.to_string()).collect(),
            content_scripts: vec![script.into()],
            content_css: vec!["body { outline: 1px solid red; }".into()],
        }
    }

    #[test]
    fn page_scripts_match_per_navigation_and_skip_internal_pages() {
        let mut cfg = BrowserConfig {
            webextensions_enabled: true,
            installed_extensions: vec![extension(&["*://*.youtube.com/*"], "window.__ytExt = 1;")],
            custom_js: "window.__custom = 1;".into(),
            ..BrowserConfig::default()
        };

        let yt = build_page_scripts("https://www.youtube.com/watch?v=1", &cfg);
        assert!(yt.iter().any(|s| s.contains("window.__ytExt = 1;")));
        assert!(yt.iter().any(|s| s.contains("__eclipse_extension_style")));
        assert!(yt.iter().any(|s| s.contains("window.__custom = 1;")));
        assert!(yt.iter().all(|s| !s.contains("new Function")));
        assert!(yt.iter().all(|s| !s.contains("localStorage")));
        assert!(
            yt.iter()
                .all(|s| s.contains("location.hostname !== \"www.youtube.com\""))
        );

        let other = build_page_scripts("https://example.org/", &cfg);
        assert!(other.iter().all(|s| !s.contains("__ytExt")));
        assert!(other.iter().any(|s| s.contains("window.__custom = 1;")));

        assert!(build_page_scripts("eclipse://localhost/settings", &cfg).is_empty());
        assert!(build_page_scripts("about:blank", &cfg).is_empty());

        cfg.webextensions_enabled = false;
        assert!(build_page_scripts("https://www.youtube.com/", &cfg).is_empty());
    }

    #[test]
    fn config_seed_is_only_for_internal_tabs_and_strips_extension_code() {
        let cfg = BrowserConfig {
            installed_extensions: vec![extension(&["<all_urls>"], "SECRET_EXTENSION_SOURCE")],
            ..BrowserConfig::default()
        };
        assert!(build_config_seed_script(&cfg, "https://example.com/").is_empty());
        let seed = build_config_seed_script(&cfg, "eclipse://newtab");
        assert!(seed.contains("__ECLIPSE_CONFIG__"));
        assert!(!seed.contains("SECRET_EXTENSION_SOURCE"));
        assert!(!seed.contains("__ECLIPSE_SPACE__"));
        let push = build_config_push_script(&cfg, "https://example.com/");
        assert!(!push.contains("SECRET_EXTENSION_SOURCE"));
        assert!(push.contains("location.protocol !== 'eclipse:'"));
    }

    #[test]
    fn injected_scripts_keep_secrets_off_page_objects() {
        let bridge = build_bridge_script("aaaabbbb", "ccccdddd");
        assert!(!bridge.contains("__ECLIPSE_TOKEN__"));
        assert!(!bridge.contains("__ECLIPSE_CONTROL__"));
        assert!(!bridge.contains("innerHTML"));
        assert!(!bridge.contains("insertAdjacentHTML"));
        assert!(!bridge.contains("payload.token"));
        assert!(!bridge.contains("__eclipseCommandBar"));
        assert!(!bridge.contains("__eclipseLittleArc"));
        assert!(!bridge.contains("set_fullscreen"));
        let policy = AdblockPolicy {
            enabled: true,
            disabled_domains: vec!["youtube.com".into()],
        };
        let shields = build_shields_script("ccccdddd", &policy);
        assert!(shields.contains("location.hostname, [\"youtube.com\"]"));
        assert!(
            build_shields_script("ccccdddd", &AdblockPolicy::default())
                .contains("setEnabled(false)")
        );
        assert!(!shields.contains("__ECLIPSE_CONTROL__"));
        assert!(!shields.contains("__ECLIPSE_ENABLED__"));
        assert!(!shields.contains("aaaabbbb"));
        assert!(!shields.contains("innerHTML"));
        assert!(guard_script("x();").starts_with("try {"));
    }

    #[test]
    fn test_adblock_site_toggling_and_extension_matching() {
        let mut cfg = BrowserConfig {
            adblock_enabled: true,
            adblock_disabled_domains: vec!["net77.cc".into(), " YouTube.com ".into(), "".into()],
            ..BrowserConfig::default()
        };
        let policy = AdblockPolicy::for_space(&cfg, "Default");
        assert_eq!(policy.disabled_domains, vec!["net77.cc", "youtube.com"]);

        assert!(!policy.blocks("https://net77.cc/home"));
        assert!(!policy.blocks("https://sub.net77.cc/video"));
        assert!(!policy.blocks("https://www.youtube.com/watch?v=123"));
        assert!(policy.blocks("https://github.com"));
        assert!(policy.blocks("https://apple.com"));
        // A suffix that is not a subdomain.
        assert!(policy.blocks("https://notyoutube.com/"));

        cfg.adblock_enabled = false;
        assert!(!AdblockPolicy::for_space(&cfg, "Default").blocks("https://github.com"));

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
