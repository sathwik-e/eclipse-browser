use std::collections::{HashMap, HashSet};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tao::dpi::{LogicalPosition, LogicalSize, Position, Size};
use tao::event_loop::EventLoopProxy;
use tao::window::Window;
use wry::{DragDropEvent, Rect, WebView, WebViewBuilder};

use crate::config::{
    BrowserConfig, DownloadItem, EaselItem, FavoriteShortcut, NoteItem, Session, SessionTab,
    normalize_site_domain, write_atomic,
};
use crate::events::{BrowserUiState, DocumentTrust, IpcCommand, UserEvent};
use crate::tab::{
    LOADING_TITLE, NavigationError, Tab, canonicalize_eclipse_url, classify_navigation_error,
    downloads_dir, generate_ipc_token, get_default_favicon, get_favicon_url,
};

pub const ARC_BORDER: f64 = 8.0;
const STATE_FRAME_INTERVAL: Duration = Duration::from_millis(16);
const CLOSED_TAB_LIMIT: usize = 25;
/// Config writes are coalesced: Notes/Easel autosave every few hundred ms.
const CONFIG_SAVE_DELAY: Duration = Duration::from_millis(800);
const SESSION_SAVE_DELAY: Duration = Duration::from_secs(2);
const MAX_SESSION_TABS: usize = 200;
/// How long the sidebar stays open after Rust reveals it for an interaction
/// (⌘L, ⌘J, Preferences) before the sidebar's own `sidebar_interaction` hold
/// has had a chance to arrive.
const TRANSIENT_HOVER_HOLD: Duration = Duration::from_millis(1500);
/// Matches the 0.20-0.22 s collapse animations; the offscreen sidebar is
/// hidden afterwards so its CSS animations stop compositing.
const SIDEBAR_HIDE_DELAY: Duration = Duration::from_millis(300);
const CRASH_WINDOW: Duration = Duration::from_secs(30);
const MAX_CRASHES_IN_WINDOW: usize = 2;
const WEB_IPC_LIMIT: usize = 64 * 1024;
const TRUSTED_IPC_LIMIT: usize = 16 * 1024 * 1024;
/// How long a closed internal tab may take to deliver its pending edits
/// before its webview is dropped anyway (a hung or crashed page).
const CLOSE_FLUSH_TIMEOUT: Duration = Duration::from_secs(1);
/// How long quitting waits for open internal pages to deliver their edits.
const QUIT_FLUSH_TIMEOUT: Duration = Duration::from_millis(500);

struct ClosedTab {
    url: String,
    workspace: String,
    is_pinned: bool,
}

/// Who posted an IPC message. Decides which commands it may run (C1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IpcSender {
    /// The sidebar webview, with its token.
    Sidebar,
    /// An eclipse:// frame while the tab's committed document is an internal
    /// page.
    Internal,
    /// An internal page that is being closed, or that the tab just left: it
    /// may only save the edits it still had pending.
    DepartingInternal,
    /// Any other page.
    Web,
    /// A page in a Little Eclipse popup window (menu.m), which posts through
    /// its opener tab's handler: it is not the tab's page, so it may only
    /// open http(s) links in new tabs.
    LittleWindow,
}

impl IpcSender {
    fn message_limit(self) -> usize {
        match self {
            IpcSender::Web | IpcSender::LittleWindow => WEB_IPC_LIMIT,
            IpcSender::Sidebar | IpcSender::Internal | IpcSender::DepartingInternal => {
                TRUSTED_IPC_LIMIT
            }
        }
    }
}

pub struct BrowserApp {
    pub window: Arc<Window>,
    pub proxy: EventLoopProxy<UserEvent>,
    pub sidebar_webview: WebView,
    pub sidebar_token: String,
    pub tabs: Vec<Tab>,
    pub active_tab_id: usize,
    pub next_tab_id: usize,
    pub next_download_id: usize,
    pub config: BrowserConfig,
    pub current_workspace: String,
    pub window_size: (f64, f64),
    closed_tabs: Vec<ClosedTab>,
    pub state_dirty: bool,
    pub next_state_flush: Instant,
    pub last_window_title: String,
    pub last_broadcast_memory_mb: f64,
    pub memory_mb: f64,
    pub started_at: Instant,
    pub perf: bool,
    pub first_page_logged: bool,
    pub sidebar_hover_expanded: bool,
    pub is_fullscreen: bool,
    pub collapse_generation: u64,
    /// Incremented on every config mutation; webviews only receive the config
    /// again when it moved past what they were last given.
    config_revision: u64,
    sidebar_pushed_revision: Option<u64>,
    /// (tab id, revision) of the internal page last given the full state.
    internal_pushed: Option<(usize, u64)>,
    config_save_due: Option<Instant>,
    session_save_due: Option<Instant>,
    last_session_json: String,
    /// The sidebar reported focus in a text field, an open popover or a drag.
    sidebar_interaction_active: bool,
    command_bar_open: bool,
    hover_hold_until: Option<Instant>,
    sidebar_hide_due: Option<(Instant, u64)>,
    crash_history: HashMap<usize, Vec<Instant>>,
    /// Last http(s) page the user looked at; internal pages (Preferences) use
    /// it as "the current site".
    last_web_url: String,
    /// Off for ECLIPSE_TEST_URL runs so tests never touch the user's session.
    persist_session: bool,
    /// Closed internal tabs whose page is still delivering its pending
    /// edits, with the time their webview is dropped regardless.
    closing_tabs: HashMap<usize, (Tab, Instant)>,
    /// Set once quitting was requested: the time the app exits regardless.
    quit_deadline: Option<Instant>,
    /// Internal tabs whose flush quitting still waits for.
    quit_waiting: HashSet<usize>,
    /// Quitting still waits for the sidebar's flush (its quick note).
    sidebar_flush_pending: bool,
    sidebar_crashes: Vec<Instant>,
    /// A sidebar in a crash loop is reloaded only after a pause.
    sidebar_reload_due: Option<Instant>,
    /// Download sequence number (from tab.rs) to the DownloadItem id it
    /// created, so a completion updates exactly that item.
    download_items: HashMap<u64, usize>,
    /// Links other apps asked Eclipse to open, opened from `pump`.
    pending_external_links: Vec<String>,
    /// The frame colour last given to Little Eclipse windows.
    little_theme_pushed: Option<([u8; 3], bool)>,
}

impl BrowserApp {
    pub fn new(
        window: Arc<Window>,
        proxy: EventLoopProxy<UserEvent>,
        initial_size: (f64, f64),
    ) -> Self {
        let started_at = Instant::now();
        let perf = std::env::var_os("ECLIPSE_PERF").is_some();
        let config = BrowserConfig::load();
        let sidebar_w = config.sidebar_width.clamp(200.0, 260.0);

        let sidebar_h = (initial_size.1 - (ARC_BORDER * 2.0)).max(100.0);
        let initial_sidebar_x = if config.sidebar_collapsed {
            -sidebar_w - 16.0
        } else {
            ARC_BORDER
        };
        let sidebar_bounds = Rect {
            position: Position::Logical(LogicalPosition::new(initial_sidebar_x, ARC_BORDER)),
            size: Size::Logical(LogicalSize::new(sidebar_w, sidebar_h)),
        };

        let sidebar_token = generate_ipc_token();
        let sidebar_bridge = build_sidebar_bridge_script(&sidebar_token);
        let proxy_clone = proxy.clone();
        // Set while a drag that carries files is over the sidebar.
        let dragging_files = std::cell::Cell::new(false);
        let sidebar_builder = WebViewBuilder::new()
            .with_id("sidebar")
            .with_bounds(sidebar_bounds)
            .with_visible(true)
            .with_transparent(true)
            .with_accept_first_mouse(true)
            .with_devtools(cfg!(debug_assertions))
            .with_initialization_script(sidebar_bridge)
            .with_custom_protocol("eclipse".into(), move |_id, req| {
                crate::ui_assets::handle_eclipse_protocol(req)
            })
            // The chrome document holds the sidebar token: it must never be
            // replaced by a dropped link/file or any other navigation.
            .with_navigation_handler(|url| is_sidebar_document_url(&url))
            // Swallow file drops (WebKit would open the file); drags without
            // files still reach the page so its own drag and drop works.
            .with_drag_drop_handler(move |event| match event {
                DragDropEvent::Enter { paths, .. } => {
                    dragging_files.set(!paths.is_empty());
                    dragging_files.get()
                }
                DragDropEvent::Over { .. } => dragging_files.get(),
                DragDropEvent::Drop { paths, .. } => dragging_files.replace(false) || !paths.is_empty(),
                DragDropEvent::Leave => dragging_files.replace(false),
                _ => false,
            })
            .with_html(crate::ui_assets::SIDEBAR_HTML)
            .with_ipc_handler(move |req| {
                let body = req.body().clone();
                let _ = proxy_clone.send_event(UserEvent::Ipc(body));
            });
        // wry's delegate claims the crash callback either way, so without a
        // handler WebKit would not reload a killed sidebar on its own.
        #[cfg(target_os = "macos")]
        let sidebar_builder = {
            use wry::WebViewBuilderExtDarwin;
            let proxy_terminated = proxy.clone();
            sidebar_builder.with_on_web_content_process_terminate_handler(move || {
                let _ = proxy_terminated.send_event(UserEvent::SidebarWebContentTerminated);
            })
        };
        let sidebar_webview = sidebar_builder
            .build_as_child(&*window)
            .expect("Failed to initialize sidebar webview");

        #[cfg(target_os = "macos")]
        {
            use wry::WebViewExtMacOS;
            let sidebar_view = sidebar_webview.webview();
            let ptr = (&*sidebar_view) as *const _ as *mut std::ffi::c_void;
            crate::macos_menu::style_sidebar_view(ptr);
        }

        let next_download_id = config
            .downloads
            .iter()
            .map(|d| d.id)
            .max()
            .unwrap_or(0)
            + 1;

        let mut app = Self {
            window: window.clone(),
            proxy: proxy.clone(),
            sidebar_webview,
            sidebar_token,
            tabs: Vec::new(),
            active_tab_id: 1,
            next_tab_id: 1,
            next_download_id,
            config,
            current_workspace: "Default".into(),
            window_size: initial_size,
            closed_tabs: Vec::new(),
            state_dirty: true,
            next_state_flush: Instant::now(),
            last_window_title: "Eclipse".into(),
            last_broadcast_memory_mb: 0.0,
            memory_mb: get_process_memory_mb(),
            started_at,
            perf,
            first_page_logged: false,
            sidebar_hover_expanded: false,
            is_fullscreen: false,
            collapse_generation: 0,
            config_revision: 1,
            sidebar_pushed_revision: None,
            internal_pushed: None,
            config_save_due: None,
            session_save_due: None,
            last_session_json: String::new(),
            sidebar_interaction_active: false,
            command_bar_open: false,
            hover_hold_until: None,
            sidebar_hide_due: None,
            crash_history: HashMap::new(),
            last_web_url: String::new(),
            persist_session: std::env::var_os("ECLIPSE_TEST_URL").is_none(),
            closing_tabs: HashMap::new(),
            quit_deadline: None,
            quit_waiting: HashSet::new(),
            sidebar_flush_pending: false,
            sidebar_crashes: Vec::new(),
            sidebar_reload_due: None,
            download_items: HashMap::new(),
            pending_external_links: Vec::new(),
            little_theme_pushed: None,
        };
        app.last_broadcast_memory_mb = app.memory_mb;

        #[cfg(target_os = "macos")]
        {
            use tao::platform::macos::WindowExtMacOS;
            let ns_win = window.ns_window();
            crate::macos_menu::configure_window(ns_win);
        }

        // ECLIPSE_TEST_URL runs (smoke tests) start clean instead of restoring.
        match std::env::var("ECLIPSE_TEST_URL") {
            Ok(url) => {
                app.create_tab(&url, true);
            }
            Err(_) => {
                if !app.restore_session() {
                    app.create_tab("eclipse://newtab", true);
                }
            }
        }
        if app.config.sidebar_collapsed {
            app.schedule_sidebar_hide();
        }
        // After the session restore, which applies the current Space's look.
        app.push_little_window_theme();

        if app.perf {
            eprintln!(
                "[perf] window + chrome + first tab created in {:.1} ms (rss {:.1} MB)",
                started_at.elapsed().as_secs_f64() * 1000.0,
                get_process_memory_mb()
            );
        }
        app
    }

    pub fn sidebar_width(&self) -> f64 {
        self.config.sidebar_width.clamp(200.0, 260.0)
    }

    /// Whether the sidebar webview should currently be on screen (docked, or
    /// hover-expanded over the content).
    fn sidebar_on_screen(&self) -> bool {
        !self.config.sidebar_collapsed || self.sidebar_hover_expanded
    }

    /// Native fullscreen does not change the layout: the window simply gets
    /// bigger, and a collapsed sidebar keeps working through edge hover.
    pub fn content_bounds(&self) -> Rect {
        let (content_x, content_w) = if self.config.sidebar_collapsed {
            (
                ARC_BORDER,
                (self.window_size.0 - (ARC_BORDER * 2.0)).max(100.0),
            )
        } else {
            let sidebar_w = self.sidebar_width();
            (
                ARC_BORDER + sidebar_w + ARC_BORDER,
                (self.window_size.0 - sidebar_w - (ARC_BORDER * 3.0)).max(100.0),
            )
        };
        let content_y = ARC_BORDER;
        let content_h = (self.window_size.1 - (ARC_BORDER * 2.0)).max(100.0);

        Rect {
            position: Position::Logical(LogicalPosition::new(content_x, content_y)),
            size: Size::Logical(LogicalSize::new(content_w, content_h)),
        }
    }

    pub fn sidebar_bounds(&self) -> Rect {
        let sidebar_w = self.sidebar_width();
        let sidebar_x = if self.config.sidebar_collapsed {
            if self.sidebar_hover_expanded {
                ARC_BORDER
            } else {
                -sidebar_w - 16.0
            }
        } else {
            ARC_BORDER
        };
        let sidebar_h = (self.window_size.1 - (ARC_BORDER * 2.0)).max(100.0);
        Rect {
            position: Position::Logical(LogicalPosition::new(sidebar_x, ARC_BORDER)),
            size: Size::Logical(LogicalSize::new(sidebar_w, sidebar_h)),
        }
    }

    pub fn active_url(&self) -> String {
        self.tabs
            .iter()
            .find(|tab| tab.id == self.active_tab_id)
            .map(|tab| tab.url.clone())
            .unwrap_or_default()
    }

    /// "The current site" as internal pages should see it: the active tab if
    /// it shows a web page, otherwise the last web page that was active.
    fn reference_url(&self) -> String {
        let active = self.active_url();
        if is_http_url(&active) {
            active
        } else {
            self.last_web_url.clone()
        }
    }

    /// Records a config mutation: bumps the revision, schedules a coalesced
    /// save, pushes the change to every tab (tab.rs only re-evaluates what a
    /// page actually needs) and refreshes the chrome.
    fn config_changed(&mut self) {
        self.config_revision += 1;
        self.config_save_due
            .get_or_insert_with(|| Instant::now() + CONFIG_SAVE_DELAY);
        let reference = self.reference_url();
        for tab in &mut self.tabs {
            sync_tab_config(tab, &self.config, self.config_revision, &reference);
        }
        self.push_little_window_theme();
        self.broadcast_state();
    }

    /// Frames Little Eclipse windows in the current Space's theme (the
    /// global theme and accent mirror the current Space's look).
    fn push_little_window_theme(&mut self) {
        let frame = little_window_frame_color(&self.config.theme, &self.config.accent_color);
        if self.little_theme_pushed == Some(frame) {
            return;
        }
        self.little_theme_pushed = Some(frame);
        #[cfg(target_os = "macos")]
        crate::macos_menu::set_little_window_theme(frame.0, frame.1);
    }

    /// Links other apps asked Eclipse to open (Eclipse as the default
    /// browser, `ECLIPSE_SMOKE_LITTLE_URL`). They are opened from
    /// `pump`, so a link that arrives while the app launches waits for the
    /// browser to be up.
    pub fn queue_external_links(&mut self, urls: impl IntoIterator<Item = String>) {
        self.pending_external_links.extend(urls);
    }

    fn open_pending_external_links(&mut self) {
        for url in std::mem::take(&mut self.pending_external_links) {
            self.open_external_link(&url);
        }
    }

    /// A link from another app: a standalone Little Eclipse window, like
    /// Little Arc, or a new tab when that setting is off or no more windows
    /// may open. Only http(s) links are opened.
    fn open_external_link(&mut self, url: &str) {
        match external_link_route(url, self.config.open_links_in_little_window) {
            ExternalLinkRoute::LittleWindow(url) => {
                if self.perf {
                    eprintln!("[little] link from another app");
                }
                if !self.open_standalone_little_window(&url, false) {
                    self.open_external_link_in_tab(&url);
                }
            }
            ExternalLinkRoute::Tab(url) => self.open_external_link_in_tab(&url),
            ExternalLinkRoute::Ignore => {
                if self.perf {
                    eprintln!("[little] ignored a link from another app that is not http(s)");
                }
            }
        }
    }

    fn open_external_link_in_tab(&mut self, url: &str) {
        self.create_tab(url, true);
        self.window.set_visible(true);
        self.window.set_focus();
    }

    /// Opens a standalone Little Eclipse window for `url` (empty: a blank one
    /// with its address field focused) with the current Space's user agent
    /// and ad blocking. `false` when none opened.
    fn open_standalone_little_window(&mut self, url: &str, focus_address: bool) -> bool {
        #[cfg(target_os = "macos")]
        {
            let user_agent = crate::tab::space_user_agent(&self.config, &self.current_workspace);
            let adblock = crate::tab::adblock_active_for(&self.config, &self.current_workspace, url);
            crate::macos_menu::open_standalone_little_window(url, &user_agent, adblock, focus_address)
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = (url, focus_address);
            false
        }
    }

    /// ⌥⌘N: the New Little Eclipse Window bar (menu.m), suggesting open tabs
    /// and favourites. Without it, a blank standalone window with its address
    /// field focused, and past the window limit a new tab.
    fn show_little_command_bar(&mut self) {
        #[cfg(target_os = "macos")]
        {
            let tabs: Vec<CommandBarTab> = self
                .tabs
                .iter()
                .map(|tab| CommandBarTab {
                    title: &tab.title,
                    url: &tab.url,
                    current_space: tab.workspace == self.current_workspace,
                    last_active: tab.last_active,
                })
                .collect();
            let suggestions = little_command_bar_suggestions(&tabs, &self.config.favorites);
            let accent = parse_hex_rgb(&self.config.accent_color).unwrap_or([0x5E, 0x9E, 0xFF]);
            if crate::macos_menu::show_little_command_bar(&suggestions, accent) {
                return;
            }
        }
        if !self.open_standalone_little_window("", true) {
            self.create_tab("eclipse://newtab", true);
        }
    }

    fn save_config_now(&mut self) {
        self.config_save_due = None;
        self.config.save();
    }

    pub fn bring_sidebar_to_front(&self) {
        #[cfg(target_os = "macos")]
        {
            use wry::WebViewExtMacOS;
            let sidebar_view = self.sidebar_webview.webview();
            let ptr = (&*sidebar_view) as *const _ as *mut std::ffi::c_void;
            crate::macos_menu::bring_view_to_front(ptr);
        }
    }

    pub fn animate_sidebar_bounds(&self, bounds: Rect, duration: f64) {
        #[cfg(target_os = "macos")]
        {
            use wry::WebViewExtMacOS;
            let sidebar_view = self.sidebar_webview.webview();
            let ptr = (&*sidebar_view) as *const _ as *mut std::ffi::c_void;
            let (x, y) = bounds.position.to_logical::<f64>(1.0).into();
            let (w, h) = bounds.size.to_logical::<f64>(1.0).into();
            crate::macos_menu::animate_view_frame(ptr, x, y, w, h, duration);
        }
        #[cfg(not(target_os = "macos"))]
        let _ = self.sidebar_webview.set_bounds(bounds);
    }

    pub fn create_tab(&mut self, url: &str, select: bool) -> usize {
        let id = self.next_tab_id;
        self.next_tab_id += 1;

        let normalized = if url.is_empty() {
            "eclipse://newtab".to_string()
        } else {
            normalize_url(url, &self.config)
        };

        let bounds = self.content_bounds();
        let tab = Tab::new(
            id,
            &normalized,
            &self.current_workspace,
            &self.window,
            self.proxy.clone(),
            bounds,
            &self.config,
        );

        self.tabs.push(tab);

        if select {
            self.select_tab(id);
            if normalized == "eclipse://newtab"
                && let Some(tab) = self.tabs.iter().find(|t| t.id == id)
            {
                tab.evaluate_script(
                    "setTimeout(function(){ var s = document.getElementById('search-box'); if(s){ s.focus(); s.select(); } }, 30);",
                );
            }
        } else {
            if let Some(tab) = self.tabs.iter().find(|tab| tab.id == id) {
                tab.set_visible(false);
            }
            self.broadcast_state();
        }

        id
    }

    pub fn select_tab(&mut self, id: usize) {
        if !self.tabs.iter().any(|tab| tab.id == id) {
            return;
        }

        let bounds = self.content_bounds();
        let window = self.window.clone();
        let proxy = self.proxy.clone();
        let mut target_workspace = None;
        let previous_id = self.active_tab_id;
        let now = Instant::now();

        for tab in &mut self.tabs {
            if tab.id == id {
                tab.restore_if_needed(&window, proxy.clone(), bounds, &self.config);
                // Background tabs are not re-laid out on resize; catch up here.
                tab.animate_bounds(bounds, 0.0);
                tab.set_visible(true);
                tab.last_active = now;
                if is_http_url(&tab.url) {
                    self.last_web_url = tab.url.clone();
                }
                if !tab.is_pinned {
                    target_workspace = Some(tab.workspace.clone());
                }
            } else {
                // The tab being left was in use until now: its idle time
                // (for hibernation) starts here, not when it was selected.
                if tab.id == previous_id {
                    tab.last_active = now;
                }
                tab.set_visible(false);
            }
        }

        self.active_tab_id = id;
        if let Some(workspace) = target_workspace {
            self.set_current_workspace(&workspace);
        }
        let reference = self.reference_url();
        if let Some(tab) = self.tabs.iter_mut().find(|tab| tab.id == id) {
            sync_tab_config(tab, &self.config, self.config_revision, &reference);
            tab.focus();
        }
        self.bring_sidebar_to_front();
        if self.config.sidebar_collapsed && self.sidebar_hover_expanded {
            self.hover_collapse_sidebar();
        }
        self.broadcast_state();
    }

    /// Tab ids in the order the sidebar shows them for the current space:
    /// favorites' tabs (by favorite position), pinned tabs, then regular tabs.
    fn display_order(&self) -> Vec<usize> {
        let keys: Vec<TabOrderKey> = self
            .tabs
            .iter()
            .map(|tab| TabOrderKey {
                id: tab.id,
                favorite_index: tab.favorite_index,
                is_pinned: tab.is_pinned,
                workspace: &tab.workspace,
            })
            .collect();
        display_order(&keys, &self.current_workspace)
    }

    /// ⌘1…⌘8 select by position; index 8 (⌘9) always selects the last tab.
    pub fn select_tab_by_workspace_index(&mut self, index: usize) {
        let visible_ids = self.display_order();
        let target = if index >= 8 {
            visible_ids.last()
        } else {
            visible_ids.get(index)
        };
        if let Some(&id) = target {
            self.select_tab(id);
        }
    }

    pub fn select_relative_tab(&mut self, delta: isize) {
        let visible_ids = self.display_order();
        if visible_ids.is_empty() {
            return;
        }
        let position = visible_ids
            .iter()
            .position(|&id| id == self.active_tab_id)
            .unwrap_or(0);
        let next = (position as isize + delta).rem_euclid(visible_ids.len() as isize) as usize;
        self.select_tab(visible_ids[next]);
    }

    pub fn close_tab(&mut self, id: usize) {
        let Some(position) = self.tabs.iter().position(|tab| tab.id == id) else {
            return;
        };
        let was_active = self.active_tab_id == id;
        let closed = self.tabs.remove(position);
        self.crash_history.remove(&id);
        // Pinned tabs show in every space, so after closing one stay in the
        // space the user is looking at.
        let fallback_workspace = if closed.is_pinned {
            self.current_workspace.clone()
        } else {
            closed.workspace.clone()
        };
        self.remember_closed_tab(&closed);
        self.retire_tab(closed);
        self.forget_closed_web_url();

        if was_active {
            let in_space = |tab: &Tab| {
                !tab.is_pinned && tab.favorite_index.is_none() && tab.workspace == fallback_workspace
            };
            let next_id = self.tabs[position..]
                .iter()
                .find(|&tab| in_space(tab))
                .or_else(|| self.tabs[..position].iter().rev().find(|&tab| in_space(tab)))
                .map(|tab| tab.id);
            match next_id {
                Some(next_id) => self.select_tab(next_id),
                // Never jump to another space: give this one a fresh tab.
                None => {
                    self.set_current_workspace(&fallback_workspace);
                    self.create_tab("eclipse://newtab", true);
                }
            }
        } else if self.tabs.is_empty() {
            self.create_tab("eclipse://newtab", true);
        } else {
            self.broadcast_state();
        }
    }

    /// Drops a tab that left `self.tabs`. An internal page first runs its
    /// flush hook, so edits still in its debounce window are saved; its IPC
    /// stays accepted (saves only) until the hook returns or times out.
    fn retire_tab(&mut self, tab: Tab) {
        tab.set_visible(false);
        let id = tab.id;
        let proxy = self.proxy.clone();
        let flushing = tab.flush_internal_page(move |_| {
            let _ = proxy.send_event(UserEvent::PageFlushed { tab_id: id });
        });
        if flushing {
            self.closing_tabs
                .insert(id, (tab, Instant::now() + CLOSE_FLUSH_TIMEOUT));
        }
    }

    /// An internal page's flush hook returned: a closed tab can go now, and
    /// quitting has one page less to wait for.
    pub fn handle_page_flushed(&mut self, tab_id: usize) {
        self.closing_tabs.remove(&tab_id);
        if self.quit_waiting.remove(&tab_id) && self.perf {
            eprintln!("[perf] tab {tab_id} flushed before quitting");
        }
    }

    /// The sidebar's flush hook returned: its pending quick-note save has
    /// been delivered.
    pub fn handle_sidebar_flushed(&mut self) {
        if std::mem::take(&mut self.sidebar_flush_pending) && self.perf {
            eprintln!("[perf] sidebar flushed before quitting");
        }
    }

    /// ⌘Q, the window's close control and smoke tests: open internal pages
    /// and the sidebar deliver their pending edits first (see
    /// `ready_to_quit`).
    pub fn request_quit(&mut self) {
        if self.quit_deadline.is_some() {
            return;
        }
        let proxy = self.proxy.clone();
        self.sidebar_flush_pending = self
            .sidebar_webview
            .evaluate_script_with_callback(crate::tab::FLUSH_SCRIPT, move |_| {
                let _ = proxy.send_event(UserEvent::SidebarFlushed);
            })
            .is_ok();
        for tab in &self.tabs {
            let id = tab.id;
            let proxy = self.proxy.clone();
            let flushing = tab.flush_internal_page(move |_| {
                let _ = proxy.send_event(UserEvent::PageFlushed { tab_id: id });
            });
            if flushing {
                self.quit_waiting.insert(id);
            }
        }
        self.quit_waiting.extend(self.closing_tabs.keys().copied());
        self.quit_deadline = Some(Instant::now() + QUIT_FLUSH_TIMEOUT);
        if self.perf {
            eprintln!("[perf] quitting: {} internal page(s) to flush", self.quit_waiting.len());
        }
    }

    /// Quitting was requested and every internal page and the sidebar have
    /// flushed (or the wait timed out).
    pub fn ready_to_quit(&self) -> bool {
        quit_ready(
            self.quit_deadline,
            !self.quit_waiting.is_empty() || self.sidebar_flush_pending,
            Instant::now(),
        )
    }

    /// After tabs were closed: Preferences' "current site" must not keep
    /// naming a page that is no longer open.
    fn forget_closed_web_url(&mut self) {
        let open: Vec<(&str, Instant)> = self
            .tabs
            .iter()
            .map(|tab| (tab.url.as_str(), tab.last_active))
            .collect();
        let next = surviving_web_url(&self.last_web_url, &open);
        if next == self.last_web_url {
            return;
        }
        self.last_web_url = next;
        let reference = self.reference_url();
        if let Some(tab) = self.tabs.iter_mut().find(|tab| tab.id == self.active_tab_id) {
            sync_tab_config(tab, &self.config, self.config_revision, &reference);
        }
    }

    fn remember_closed_tab(&mut self, tab: &Tab) {
        self.closed_tabs.push(ClosedTab {
            url: tab.url.clone(),
            workspace: tab.workspace.clone(),
            is_pinned: tab.is_pinned,
        });
        if self.closed_tabs.len() > CLOSED_TAB_LIMIT {
            self.closed_tabs.remove(0);
        }
    }

    pub fn reopen_closed_tab(&mut self) {
        let Some(record) = self.closed_tabs.pop() else {
            return;
        };
        self.set_current_workspace(&record.workspace);
        let id = self.create_tab(&record.url, true);
        if let Some(tab) = self.tabs.iter_mut().find(|tab| tab.id == id) {
            tab.is_pinned = record.is_pinned;
            tab.workspace = record.workspace;
        }
        self.broadcast_state();
    }

    /// Makes `workspace` current and applies its profile's look. Every path
    /// that changes space (space switcher, selecting a tab of another space,
    /// closing the last tab, reopening) goes through here.
    fn set_current_workspace(&mut self, workspace: &str) {
        if self.current_workspace == workspace {
            return;
        }
        self.current_workspace = workspace.to_string();
        // Only the chrome look is mirrored into the global config. Per-space
        // ad blocking and user agent are resolved per tab from the tab's own
        // space profile, so they never leak into other spaces.
        let profile = self.config.profile_for_workspace(workspace);
        if self.config.theme != profile.theme || self.config.accent_color != profile.accent_color {
            self.config.theme = profile.theme;
            self.config.accent_color = profile.accent_color;
            self.config_changed();
        }
        self.broadcast_state();
    }

    pub fn switch_workspace(&mut self, workspace: String) {
        let normalized_ws = if workspace == "Work" {
            "Default".to_string()
        } else {
            workspace
        };
        self.set_current_workspace(&normalized_ws);

        let active_in_ws = self
            .tabs
            .iter()
            .find(|tab| tab.id == self.active_tab_id)
            .map(|tab| {
                !tab.is_pinned && tab.favorite_index.is_none() && tab.workspace == normalized_ws
            })
            .unwrap_or(false);

        if active_in_ws {
            self.broadcast_state();
            return;
        }

        let candidate_id = self
            .tabs
            .iter()
            .filter(|tab| {
                !tab.is_pinned && tab.favorite_index.is_none() && tab.workspace == normalized_ws
            })
            .max_by_key(|tab| tab.last_active)
            .map(|tab| tab.id);

        if let Some(id) = candidate_id {
            self.select_tab(id);
        } else {
            self.create_tab("eclipse://newtab", true);
        }
    }

    pub fn navigate_active(&mut self, input: &str) {
        let url = normalize_url(input, &self.config);
        if let Some(tab) = self
            .tabs
            .iter_mut()
            .find(|tab| tab.id == self.active_tab_id)
        {
            tab.url = url.clone();
            tab.title = title_for_url(&url);
            tab.favicon = get_default_favicon(&url);
            tab.favicon_url = get_favicon_url(&url);
            tab.is_loading = !url.starts_with("eclipse://");
            tab.last_active = Instant::now();
            tab.load_url(&url);
            tab.focus();
        }
        self.broadcast_state();
    }

    pub fn open_favorite(&mut self, index: usize) {
        let fav = match self.config.favorites.get(index) {
            Some(f) => f.clone(),
            None => return,
        };

        let fav_host = url::Url::parse(&fav.url).ok().and_then(|u| {
            u.host_str()
                .map(|h| h.trim_start_matches("www.").to_string())
        });

        let mut target_tab_id = self
            .tabs
            .iter()
            .find(|t| t.favorite_index == Some(index))
            .map(|t| t.id);

        if target_tab_id.is_none()
            && let Some(ref host) = fav_host
        {
            target_tab_id = self
                .tabs
                .iter()
                .find(|t| {
                    // Only adopt a pinned tab that no other favorite owns.
                    if !t.is_pinned || t.favorite_index.is_some() {
                        return false;
                    }
                    url::Url::parse(&t.url)
                        .ok()
                        .and_then(|u| {
                            u.host_str()
                                .map(|h| h.trim_start_matches("www.").to_string())
                        })
                        .map(|h| h == *host)
                        .unwrap_or(false)
                })
                .map(|t| t.id);
        }

        if let Some(id) = target_tab_id {
            if let Some(tab) = self.tabs.iter_mut().find(|t| t.id == id) {
                tab.is_pinned = true;
                tab.favorite_index = Some(index);
            }
            self.select_tab(id);
        } else {
            let active_is_empty_newtab = self
                .tabs
                .iter()
                .find(|t| t.id == self.active_tab_id)
                .map(|t| !t.is_pinned && (t.url == "eclipse://newtab" || t.url.is_empty()))
                .unwrap_or(false);

            let target_url = normalize_url(&fav.url, &self.config);
            if active_is_empty_newtab {
                let active_id = self.active_tab_id;
                if let Some(tab) = self.tabs.iter_mut().find(|t| t.id == active_id) {
                    tab.is_pinned = true;
                    tab.favorite_index = Some(index);
                    tab.title = fav.title.clone();
                    tab.load_url(&target_url);
                }
                self.select_tab(active_id);
            } else {
                let new_id = self.create_tab(&target_url, true);
                if let Some(tab) = self.tabs.iter_mut().find(|t| t.id == new_id) {
                    tab.is_pinned = true;
                    tab.favorite_index = Some(index);
                    tab.title = fav.title.clone();
                }
                self.broadcast_state();
            }
        }
    }

    pub fn reorder_tab(&mut self, from_id: usize, to_id: usize, before: bool) {
        if from_id == to_id {
            return;
        }
        let Some(from_pos) = self.tabs.iter().position(|t| t.id == from_id) else {
            return;
        };
        let Some(_to_pos) = self.tabs.iter().position(|t| t.id == to_id) else {
            return;
        };
        let moving_tab = self.tabs.remove(from_pos);
        let new_to_pos = self
            .tabs
            .iter()
            .position(|t| t.id == to_id)
            .unwrap_or(self.tabs.len());
        let insert_idx = if before {
            new_to_pos
        } else {
            (new_to_pos + 1).min(self.tabs.len())
        };
        self.tabs.insert(insert_idx, moving_tab);
        self.broadcast_state();
    }

    pub fn reorder_favorite(&mut self, from_index: usize, to_index: usize) {
        let len = self.config.favorites.len();
        if from_index >= len || to_index >= len || from_index == to_index {
            return;
        }
        let item = self.config.favorites.remove(from_index);
        self.config.favorites.insert(to_index, item);
        // Bindings follow the favorite itself, never the tab's current host.
        for tab in &mut self.tabs {
            if let Some(index) = tab.favorite_index {
                tab.favorite_index = Some(index_after_move(index, from_index, to_index));
            }
        }
        self.config_changed();
    }

    pub fn tidy_tabs(&mut self) {
        let ws = self.current_workspace.clone();
        let active_id = self.active_tab_id;

        // 1. Deduplicate identical unpinned URLs in the current workspace
        let mut seen_urls = std::collections::HashSet::new();
        let mut duplicate_ids = Vec::new();

        // First mark active tab's URL as seen so we never close the active tab
        if let Some(active_tab) = self
            .tabs
            .iter()
            .find(|t| t.id == active_id && !t.is_pinned && t.favorite_index.is_none() && t.workspace == ws)
            && active_tab.url != "eclipse://newtab"
        {
            seen_urls.insert(active_tab.url.clone());
        }

        for tab in &self.tabs {
            if tab.is_pinned || tab.favorite_index.is_some() || tab.workspace != ws {
                continue;
            }
            // Never close the active tab or one that is playing audio.
            if tab.id == active_id || tab.is_audible {
                continue;
            }
            if tab.url == "eclipse://newtab" {
                duplicate_ids.push(tab.id);
                continue;
            }
            if !seen_urls.insert(tab.url.clone()) {
                duplicate_ids.push(tab.id);
            }
        }

        // Closed duplicates go on the reopen stack so ⇧⌘T can undo Tidy.
        let removed_count = duplicate_ids.len();
        for id in duplicate_ids {
            if let Some(position) = self.tabs.iter().position(|t| t.id == id) {
                let closed = self.tabs.remove(position);
                self.crash_history.remove(&id);
                self.remember_closed_tab(&closed);
                self.retire_tab(closed);
            }
        }
        if removed_count > 0 {
            self.forget_closed_web_url();
        }

        // Ensure at least 1 tab remains
        if !self.tabs.iter().any(|t| !t.is_pinned && t.workspace == ws) {
            self.create_tab("eclipse://newtab", true);
        }

        // 2. Categorize each unpinned tab in current workspace into a semantic group
        let mut groups_set = std::collections::HashSet::new();
        for tab in &mut self.tabs {
            if tab.is_pinned || tab.favorite_index.is_some() || tab.workspace != ws {
                continue;
            }
            let group_name = categorize_tab_group(&tab.url, &tab.title);
            groups_set.insert(group_name.clone());
            tab.group = Some(group_name);
        }

        // 3. Sort unpinned tabs in the current workspace by group, then title
        let mut ws_indices = Vec::new();
        let mut ws_tabs = Vec::new();
        for (idx, tab) in self.tabs.iter().enumerate() {
            if !tab.is_pinned && tab.favorite_index.is_none() && tab.workspace == ws {
                ws_indices.push(idx);
            }
        }
        for &idx in ws_indices.iter().rev() {
            ws_tabs.push(self.tabs.remove(idx));
        }
        ws_tabs.sort_by(|a, b| {
            let ga = a.group.as_deref().unwrap_or("Other");
            let gb = b.group.as_deref().unwrap_or("Other");
            ga.cmp(gb)
                .then_with(|| a.title.to_lowercase().cmp(&b.title.to_lowercase()))
        });
        for tab in ws_tabs {
            self.tabs.push(tab);
        }

        let group_count = groups_set.len().max(1);
        if removed_count > 0 {
            self.show_toast(&format!(
                "Tidied tabs into {group_count} group(s) • Closed {removed_count} duplicate(s) (⇧⌘T reopens)"
            ));
        } else {
            self.show_toast(&format!("Tidied tabs into {group_count} group(s)"));
        }
        self.broadcast_state();
    }

    /// Shows a hidden sidebar for a keyboard interaction (⌘L, ⌘J,
    /// Preferences) by hover-expanding it, never by docking it: the user's
    /// collapsed preference is left alone. A short hold covers the gap until
    /// the sidebar reports its own `sidebar_interaction`.
    fn reveal_sidebar_for_interaction(&mut self) {
        if self.config.sidebar_collapsed {
            self.hover_hold_until = Some(Instant::now() + TRANSIENT_HOVER_HOLD);
            self.hover_expand_sidebar();
        }
        let _ = self.sidebar_webview.focus();
    }

    pub fn focus_omnibar(&mut self) {
        self.reveal_sidebar_for_interaction();
        let _ = self
            .sidebar_webview
            .evaluate_script("window.focusOmnibar && window.focusOmnibar();");
    }

    pub fn open_settings(&mut self) {
        self.reveal_sidebar_for_interaction();
        let _ = self
            .sidebar_webview
            .evaluate_script("window.openPreferencesDrawer && window.openPreferencesDrawer();");
    }

    pub fn open_downloads_panel(&mut self) {
        self.reveal_sidebar_for_interaction();
        let _ = self
            .sidebar_webview
            .evaluate_script("window.openDownloadsDrawer && window.openDownloadsDrawer();");
    }

    pub fn open_settings_tab(&mut self) {
        if let Some(existing_id) = self
            .tabs
            .iter()
            .find(|tab| tab.url == "eclipse://settings" && tab.workspace == self.current_workspace)
            .map(|tab| tab.id)
        {
            self.select_tab(existing_id);
        } else {
            self.create_tab("eclipse://settings", true);
        }
    }

    pub fn open_notes_tab(&mut self) {
        if let Some(existing_id) = self
            .tabs
            .iter()
            .find(|tab| tab.url == "eclipse://notes" && tab.workspace == self.current_workspace)
            .map(|tab| tab.id)
        {
            self.select_tab(existing_id);
        } else {
            self.create_tab("eclipse://notes", true);
        }
    }

    pub fn open_easel_tab(&mut self) {
        if let Some(existing_id) = self
            .tabs
            .iter()
            .find(|tab| tab.url == "eclipse://easel" && tab.workspace == self.current_workspace)
            .map(|tab| tab.id)
        {
            self.select_tab(existing_id);
        } else {
            self.create_tab("eclipse://easel", true);
        }
    }

    pub fn toggle_sidebar(&mut self) {
        self.sidebar_hover_expanded = false;
        self.collapse_generation += 1;
        #[cfg(target_os = "macos")]
        crate::macos_menu::set_hover_expanded(false, self.sidebar_width());
        self.config.sidebar_collapsed = !self.config.sidebar_collapsed;
        self.apply_sidebar_collapsed_change();
        self.config_changed();
    }

    /// Re-lays out after `config.sidebar_collapsed` flipped (menu toggle or a
    /// settings patch).
    fn apply_sidebar_collapsed_change(&mut self) {
        if self.config.sidebar_collapsed {
            let _ = self
                .sidebar_webview
                .evaluate_script("window.popoutSidebar && window.popoutSidebar(false);");
            self.update_layout_animated(0.22);
            if let Some(tab) = self.tabs.iter().find(|tab| tab.id == self.active_tab_id) {
                tab.focus();
            }
        } else {
            let _ = self.sidebar_webview.set_visible(true);
            let _ = self.sidebar_webview.evaluate_script(
                "document.body.classList.remove('floating-popout', 'sliding-out', 'is-collapsed-state');",
            );
            self.update_layout_animated(0.22);
        }
    }

    pub fn hover_expand_sidebar(&mut self) {
        // A docked sidebar never hover-expands. (The monitor's own flag then
        // reads "expanded" until the pointer leaves, which only produces one
        // ignored collapse; resetting it here would re-fire at the poll rate.)
        if !self.config.sidebar_collapsed {
            return;
        }
        if self.sidebar_hover_expanded {
            // Expanded but the monitor thought otherwise (a collapse was
            // ignored while the sidebar was held): resync it.
            #[cfg(target_os = "macos")]
            crate::macos_menu::set_hover_expanded(true, self.sidebar_width());
        } else {
            self.collapse_generation += 1;
            self.sidebar_hover_expanded = true;
            self.sidebar_hide_due = None;
            #[cfg(target_os = "macos")]
            crate::macos_menu::set_hover_expanded(true, self.sidebar_width());
            let _ = self.sidebar_webview.set_visible(true);
            self.bring_sidebar_to_front();
            let _ = self
                .sidebar_webview
                .evaluate_script("window.popoutSidebar && window.popoutSidebar(true);");
            self.animate_sidebar_bounds(self.sidebar_bounds(), 0.20);
            self.flush_state();
        }
    }

    /// True while something in the sidebar needs it to stay open.
    fn sidebar_held(&self) -> bool {
        self.sidebar_interaction_active
            || self.command_bar_open
            || self
                .hover_hold_until
                .is_some_and(|until| Instant::now() < until)
    }

    /// Collapse request from the edge monitor (or after selecting a tab).
    /// Ignored while the sidebar is held; releasing the hold re-arms the
    /// monitor and re-checks the pointer.
    pub fn hover_collapse_sidebar(&mut self) {
        if !self.sidebar_hover_expanded || self.sidebar_held() {
            return;
        }
        self.collapse_generation += 1;
        self.sidebar_hover_expanded = false;
        #[cfg(target_os = "macos")]
        crate::macos_menu::set_hover_expanded(false, self.sidebar_width());
        let _ = self
            .sidebar_webview
            .evaluate_script("window.popoutSidebar && window.popoutSidebar(false);");
        self.animate_sidebar_bounds(self.sidebar_bounds(), 0.20);
        self.schedule_sidebar_hide();
        // Keystrokes must not keep going into the now-hidden sidebar.
        if let Some(tab) = self.tabs.iter().find(|tab| tab.id == self.active_tab_id) {
            tab.focus();
        }
        self.flush_state();
    }

    /// Hides the offscreen sidebar once its collapse animation is over, so
    /// its CSS animations stop compositing.
    fn schedule_sidebar_hide(&mut self) {
        self.sidebar_hide_due = Some((Instant::now() + SIDEBAR_HIDE_DELAY, self.collapse_generation));
    }

    /// The sidebar stopped needing to stay open: let the edge monitor decide
    /// again from the current pointer position.
    fn release_sidebar_hold_if_idle(&mut self) {
        if self.sidebar_hover_expanded && !self.sidebar_held() {
            // A collapse ignored during the hold left the monitor believing
            // the sidebar is collapsed; re-arm it before re-checking.
            #[cfg(target_os = "macos")]
            {
                crate::macos_menu::set_hover_expanded(true, self.sidebar_width());
                crate::macos_menu::recheck_hover();
            }
        }
    }

    pub fn set_sidebar_interaction(&mut self, active: bool) {
        if self.sidebar_interaction_active == active {
            return;
        }
        self.sidebar_interaction_active = active;
        if !active {
            self.release_sidebar_hold_if_idle();
        }
    }

    /// Window fullscreen only changes the window size (handled by `resize`);
    /// the sidebar keeps docking or edge-hovering as usual. The flag lets the
    /// sidebar hide its window controls.
    pub fn set_fullscreen(&mut self, fullscreen: bool) {
        if self.is_fullscreen != fullscreen {
            self.is_fullscreen = fullscreen;
            self.broadcast_state();
        }
    }

    /// ⌘K. On the new tab page its own search box is the command bar;
    /// everywhere else the command bar lives in the (trusted) sidebar, never
    /// inside the web page.
    pub fn open_command_bar(&mut self) {
        if let Some(tab) = self.tabs.iter().find(|tab| tab.id == self.active_tab_id)
            && tab.url.starts_with("eclipse://newtab")
        {
            tab.evaluate_script(
                "(function () { var s = document.getElementById('search-box'); if (s) { s.focus(); s.select(); } })();",
            );
            tab.focus();
            return;
        }
        self.command_bar_open = true;
        if self.config.sidebar_collapsed {
            self.hover_expand_sidebar();
        }
        let _ = self.sidebar_webview.focus();
        let _ = self
            .sidebar_webview
            .evaluate_script("window.showCommandBar && window.showCommandBar();");
    }

    fn command_bar_closed(&mut self) {
        self.command_bar_open = false;
        self.release_sidebar_hold_if_idle();
        if let Some(tab) = self.tabs.iter().find(|tab| tab.id == self.active_tab_id) {
            tab.focus();
        }
    }

    /// Messages from the sidebar webview. Only token-bearing messages are
    /// accepted; sidebar.html always posts through `window.__eclipseIpc`.
    pub fn handle_ipc(&mut self, raw: &str) {
        let Some((token, command)) = parse_ipc(raw, IpcSender::Sidebar.message_limit()) else {
            return;
        };
        if token == self.sidebar_token {
            self.handle_ipc_command(None, command);
        }
    }

    /// Messages from a tab. `origin` is the posting frame's URL as reported by
    /// WebKit, which together with the tab's committed document when the
    /// message arrived (`document`) decides whether the sender is an internal
    /// page or an ordinary website. A tab that is closing only delivers the
    /// saves its page flushes.
    pub fn handle_tab_ipc(
        &mut self,
        tab_id: usize,
        raw: &str,
        origin: &str,
        document: DocumentTrust,
        from_little_window: bool,
    ) {
        let (tab, closing) = match self.tabs.iter().find(|tab| tab.id == tab_id) {
            Some(tab) => (tab, false),
            None => match self.closing_tabs.get(&tab_id) {
                Some((tab, _)) => (tab, true),
                None => return,
            },
        };
        let sender = tab_ipc_sender(origin, document, closing, from_little_window);
        let Some((token, command)) = parse_ipc(raw, sender.message_limit()) else {
            return;
        };
        if token != tab.token {
            return;
        }
        // Only the committed web document may report its own state (its
        // same-origin URL, favicon, audio, scroll position): never a late
        // message from a page the tab already left. A Little Eclipse popup,
        // which posts through its opener tab's handler even when it is on
        // the tab's origin, is a sender of its own (see is_command_allowed).
        if sender == IpcSender::Web
            && !is_plausible_page_report(&command, origin, &tab.committed_url())
        {
            if self.perf {
                eprintln!(
                    "[ipc] ignored {} from another document in tab {tab_id} ({})",
                    command_name(&command),
                    truncate_message(origin, 120)
                );
            }
            return;
        }
        if !is_command_allowed(sender, &command) {
            if self.perf {
                eprintln!(
                    "[ipc] dropped {} from {sender:?} tab {tab_id} ({})",
                    command_name(&command),
                    truncate_message(origin, 120)
                );
            }
            return;
        }
        self.handle_ipc_command(Some(tab_id), command);
    }

    fn handle_ipc_command(&mut self, source_tab_id: Option<usize>, cmd: IpcCommand) {
        match cmd {
            IpcCommand::Navigate { url } => self.navigate_active(&url),
            IpcCommand::NewTab { url, background } => {
                let target = url.unwrap_or_else(|| "eclipse://newtab".into());
                let select = !background.unwrap_or(false);
                self.create_tab(&target, select);
            }
            IpcCommand::CleanMemory => self.clean_memory(),
            IpcCommand::SelectTab { id } => self.select_tab(id),
            IpcCommand::SelectTabIndex { index } => self.select_tab_by_workspace_index(index),
            IpcCommand::SelectNextTab => self.select_relative_tab(1),
            IpcCommand::SelectPrevTab => self.select_relative_tab(-1),
            IpcCommand::CloseTab { id } => {
                let target_id = id.or(source_tab_id).unwrap_or(self.active_tab_id);
                self.close_tab(target_id);
            }
            IpcCommand::ReopenClosedTab => self.reopen_closed_tab(),
            IpcCommand::GoBack => {
                if let Some(tab) = self.tabs.iter().find(|tab| tab.id == self.active_tab_id) {
                    tab.go_back();
                }
                self.broadcast_state();
            }
            IpcCommand::GoForward => {
                if let Some(tab) = self.tabs.iter().find(|tab| tab.id == self.active_tab_id) {
                    tab.go_forward();
                }
                self.broadcast_state();
            }
            IpcCommand::Reload => {
                if let Some(tab) = self.tabs.iter().find(|tab| tab.id == self.active_tab_id) {
                    tab.reload();
                }
            }
            IpcCommand::StopLoading => self.stop_active(),
            IpcCommand::ZoomIn => self.zoom_active(0.1),
            IpcCommand::ZoomOut => self.zoom_active(-0.1),
            IpcCommand::ZoomReset => self.zoom_active(0.0),
            IpcCommand::Print => self.print_active(),
            IpcCommand::FavoriteTab => self.favorite_active_tab(),
            IpcCommand::ToggleSidebar => self.toggle_sidebar(),
            IpcCommand::HoverExpandSidebar => self.hover_expand_sidebar(),
            IpcCommand::HoverCollapseSidebar => self.hover_collapse_sidebar(),
            IpcCommand::FocusOmnibar => self.focus_omnibar(),
            IpcCommand::OpenCommandBar => self.open_command_bar(),
            IpcCommand::NavigateOrNewTab { url } => {
                let current_url = self
                    .tabs
                    .iter()
                    .find(|t| t.id == self.active_tab_id)
                    .map(|t| t.url.as_str())
                    .unwrap_or("");
                if current_url.is_empty() || current_url.starts_with("eclipse://newtab") {
                    self.navigate_active(&url);
                } else {
                    self.create_tab(&url, true);
                }
            }
            IpcCommand::CommandBarOpenUrl {
                url,
                tab_id,
                command,
            } => {
                if let Some(tab_id) = tab_id {
                    self.select_tab(tab_id);
                } else if let Some(cmd) = command {
                    match cmd.as_str() {
                        "open_settings" => self.open_settings(),
                        "clean_memory" => self.clean_memory(),
                        "tidy_tabs" => self.tidy_tabs(),
                        "open_notes" => self.open_notes_tab(),
                        "open_easel" => self.open_easel_tab(),
                        "open_downloads" => self.open_downloads_panel(),
                        _ => {}
                    }
                } else if let Some(url) = url {
                    let current_url = self
                        .tabs
                        .iter()
                        .find(|t| t.id == self.active_tab_id)
                        .map(|t| t.url.as_str())
                        .unwrap_or("");
                    if current_url.is_empty() || current_url.starts_with("eclipse://newtab") {
                        self.navigate_active(&url);
                    } else {
                        self.create_tab(&url, true);
                    }
                }
            }
            IpcCommand::OpenSettings => self.open_settings(),
            IpcCommand::OpenSettingsTab => self.open_settings_tab(),
            IpcCommand::TogglePin { id } => {
                if let Some(tab) = self.tabs.iter_mut().find(|tab| tab.id == id) {
                    tab.is_pinned = !tab.is_pinned;
                    if !tab.is_pinned {
                        tab.favorite_index = None;
                    }
                }
                self.broadcast_state();
            }
            IpcCommand::OpenFavorite { index } => self.open_favorite(index),
            IpcCommand::SetWorkspace { workspace } => self.switch_workspace(workspace),
            IpcCommand::HibernateTab { id } => {
                if id != self.active_tab_id {
                    if let Some(tab) = self.tabs.iter_mut().find(|tab| tab.id == id) {
                        tab.hibernate();
                    }
                    self.broadcast_state();
                }
            }
            IpcCommand::UpdateSettings { settings } => self.apply_settings(&settings),
            IpcCommand::CommandBarClosed => self.command_bar_closed(),
            IpcCommand::SidebarInteraction { active } => self.set_sidebar_interaction(active),
            IpcCommand::ToggleSiteAdblock { domain, enabled } => {
                self.toggle_site_adblock(&domain, enabled);
            }
            IpcCommand::InstallExtension { path } => {
                match self.install_extension_from_path(&path, None) {
                    Ok(name) => self.show_toast(&format!("Installed {name}")),
                    Err(err) => self.show_toast(&err),
                }
            }
            IpcCommand::InstallFromWebStore { input } => self.start_web_store_install(&input),
            IpcCommand::ToggleExtension { id, enabled } => {
                self.toggle_extension(&id, enabled);
            }
            IpcCommand::DeleteExtension { id } => {
                self.delete_extension(&id);
            }
            IpcCommand::SaveFavorite { index, title, url } => {
                let clean_title = title.trim().to_string();
                let normalized = normalize_url(&url, &self.config);
                let icon = get_default_favicon(&normalized);
                let favorite = FavoriteShortcut {
                    title: if clean_title.is_empty() {
                        normalized.clone()
                    } else {
                        clean_title
                    },
                    url: normalized,
                    icon,
                };
                match index {
                    Some(index) if index < self.config.favorites.len() => {
                        // A favorite pointed somewhere else now: its old tab
                        // no longer represents it.
                        if self.config.favorites[index].url != favorite.url {
                            for tab in &mut self.tabs {
                                if tab.favorite_index == Some(index) {
                                    tab.favorite_index = None;
                                    tab.is_pinned = false;
                                }
                            }
                        }
                        self.config.favorites[index] = favorite;
                    }
                    _ => self.config.favorites.push(favorite),
                }
                self.config_changed();
            }
            IpcCommand::DeleteFavorite { index } => {
                if index < self.config.favorites.len() {
                    self.config.favorites.remove(index);
                    for tab in &mut self.tabs {
                        if let Some(fav_idx) = tab.favorite_index {
                            if fav_idx == index {
                                tab.favorite_index = None;
                                tab.is_pinned = false;
                            } else if fav_idx > index {
                                tab.favorite_index = Some(fav_idx - 1);
                            }
                        }
                    }
                    self.config_changed();
                }
            }
            IpcCommand::ReorderTab {
                from_id,
                to_id,
                before,
            } => {
                self.reorder_tab(from_id, to_id, before);
            }
            IpcCommand::ReorderFavorite {
                from_index,
                to_index,
            } => {
                self.reorder_favorite(from_index, to_index);
            }
            IpcCommand::TidyTabs => {
                self.tidy_tabs();
            }
            IpcCommand::OpenDownload { id } => {
                if let Some(dl) = self.config.downloads.iter().find(|d| d.id == id) {
                    if !dl.path.is_empty() {
                        let _ = std::process::Command::new("open").arg(&dl.path).spawn();
                    } else if let Some(dir) = downloads_dir() {
                        let candidate = dir.join(&dl.file_name);
                        let _ = std::process::Command::new("open").arg(candidate).spawn();
                    }
                }
            }
            IpcCommand::RevealDownload { id } => {
                if let Some(dl) = self.config.downloads.iter().find(|d| d.id == id) {
                    let path_to_reveal = if !dl.path.is_empty() {
                        std::path::PathBuf::from(&dl.path)
                    } else if let Some(dir) = downloads_dir() {
                        dir.join(&dl.file_name)
                    } else {
                        return;
                    };
                    let _ = std::process::Command::new("open")
                        .arg("-R")
                        .arg(path_to_reveal)
                        .spawn();
                }
            }
            IpcCommand::OpenDownloadsFolder => {
                if let Some(dir) = downloads_dir() {
                    let _ = std::process::Command::new("open").arg(dir).spawn();
                }
            }
            IpcCommand::ClearDownloads => {
                self.config
                    .downloads
                    .retain(|d| d.status == "downloading");
                self.config_changed();
            }
            IpcCommand::SaveNote {
                id,
                title,
                content,
                workspace,
            } => {
                let now = now_unix_secs();
                let ws = workspace.unwrap_or_else(|| self.current_workspace.clone());
                let note_id = id.unwrap_or_else(|| format!("note_{now}"));
                if let Some(existing) = self.config.notes.iter_mut().find(|n| n.id == note_id) {
                    existing.title = title;
                    existing.content = content;
                    existing.updated_at = now;
                } else {
                    self.config.notes.insert(
                        0,
                        NoteItem {
                            id: note_id,
                            title,
                            content,
                            workspace: ws,
                            updated_at: now,
                        },
                    );
                }
                self.config_changed();
            }
            IpcCommand::DeleteNote { id } => {
                self.config.notes.retain(|n| n.id != id);
                self.config_changed();
            }
            IpcCommand::OpenNotesTab => {
                self.open_notes_tab();
            }
            IpcCommand::SaveEasel {
                id,
                title,
                elements_json,
                workspace,
            } => {
                let now = now_unix_secs();
                let ws = workspace.unwrap_or_else(|| self.current_workspace.clone());
                let easel_id = id.unwrap_or_else(|| format!("easel_{now}"));
                if let Some(existing) = self.config.easels.iter_mut().find(|e| e.id == easel_id) {
                    existing.title = title;
                    existing.elements_json = elements_json;
                    existing.updated_at = now;
                } else {
                    self.config.easels.insert(
                        0,
                        EaselItem {
                            id: easel_id,
                            title,
                            elements_json,
                            workspace: ws,
                            updated_at: now,
                        },
                    );
                }
                self.config_changed();
            }
            IpcCommand::DeleteEasel { id } => {
                self.config.easels.retain(|e| e.id != id);
                self.config_changed();
            }
            IpcCommand::OpenEaselTab { id: _ } => {
                self.open_easel_tab();
            }
            IpcCommand::UpdateSpaceProfile { profile } => {
                if profile.workspace.trim().is_empty() {
                    return;
                }
                if let Some(existing) = self
                    .config
                    .space_profiles
                    .iter_mut()
                    .find(|p| p.workspace.eq_ignore_ascii_case(&profile.workspace))
                {
                    *existing = profile.clone();
                } else {
                    self.config.space_profiles.push(profile.clone());
                }
                // The look is mirrored for the current space; ad blocking and
                // the user agent stay in the profile and are resolved per tab.
                if self.current_workspace.eq_ignore_ascii_case(&profile.workspace) {
                    self.config.theme = profile.theme;
                    self.config.accent_color = profile.accent_color;
                }
                self.config_changed();
            }
            IpcCommand::UpdateScroll { x, y } => {
                let target_id = source_tab_id.unwrap_or(self.active_tab_id);
                if let Some(tab) = self.tabs.iter_mut().find(|tab| tab.id == target_id) {
                    tab.scroll_x = x;
                    tab.scroll_y = y;
                }
            }
            IpcCommand::UpdateFavicon { favicon } => {
                if !is_safe_icon_url(&favicon) {
                    return;
                }
                let target_id = source_tab_id.unwrap_or(self.active_tab_id);
                if let Some(tab) = self.tabs.iter_mut().find(|tab| tab.id == target_id)
                    && tab.favicon_url.as_deref() != Some(favicon.as_str())
                {
                    tab.favicon_url = Some(favicon);
                    self.broadcast_state();
                }
            }
            IpcCommand::SetAudible { audible } => {
                let target_id = source_tab_id.unwrap_or(self.active_tab_id);
                if let Some(tab) = self.tabs.iter_mut().find(|tab| tab.id == target_id)
                    && tab.is_audible != audible
                {
                    tab.is_audible = audible;
                    self.broadcast_state();
                }
            }
            IpcCommand::ChromeError { message } => {
                if cfg!(debug_assertions) || self.perf {
                    let origin = match source_tab_id {
                        Some(id) => format!("tab {id}"),
                        None => "sidebar".to_string(),
                    };
                    eprintln!("[chrome-js] {origin}: {}", truncate_message(&message, 500));
                }
            }
            IpcCommand::UrlChanged { url } => {
                if !is_http_url(&url) || url.len() > 8 * 1024 {
                    return;
                }
                let target_id = source_tab_id.unwrap_or(self.active_tab_id);
                if let Some(tab) = self.tabs.iter_mut().find(|tab| tab.id == target_id)
                    && tab.url != url
                {
                    tab.note_same_document_url(&url);
                    tab.url = url;
                    if tab.id == self.active_tab_id {
                        self.last_web_url = tab.url.clone();
                    }
                    self.broadcast_state();
                }
            }
            IpcCommand::WindowClose => {
                let _ = self.proxy.send_event(UserEvent::CloseWindow);
            }
            IpcCommand::WindowMinimize => {
                self.window.set_minimized(true);
            }
            // The green traffic light zooms like every other macOS window;
            // fullscreen has its own menu item (⌃⌘F).
            IpcCommand::WindowZoom => {
                self.window.set_maximized(!self.window.is_maximized());
            }
            IpcCommand::DragWindow => {
                let _ = self.window.drag_window();
            }
            IpcCommand::OpenDevtools => {
                if let Some(tab) = self.tabs.iter().find(|tab| tab.id == self.active_tab_id) {
                    tab.open_devtools();
                }
            }
            IpcCommand::Ready => {
                // A (re)loaded document has none of the previous state.
                match source_tab_id {
                    None => {
                        self.sidebar_pushed_revision = None;
                        // Nor does a fresh sidebar hold the command bar or a
                        // field open; it only reports holds that start.
                        self.command_bar_open = false;
                        self.sidebar_interaction_active = false;
                        self.release_sidebar_hold_if_idle();
                    }
                    Some(_) => self.internal_pushed = None,
                }
                self.flush_state();
                if self.perf && source_tab_id.is_none() {
                    self.probe_sidebar();
                }
            }
        }
    }

    /// `update_settings`: applies only whitelisted, validated keys (see
    /// `BrowserConfig::apply_settings_patch`), then runs the side effects of
    /// what changed.
    fn apply_settings(&mut self, patch: &serde_json::Map<String, serde_json::Value>) {
        let changed = self.config.apply_settings_patch(patch);
        if changed.is_empty() {
            return;
        }
        if self.perf {
            eprintln!("[settings] changed {}", changed.join(", "));
        }
        let has = |key: &str| changed.contains(&key);

        if has("theme") || has("accent_color") {
            let ws = self.current_workspace.clone();
            let (theme, accent) = (self.config.theme.clone(), self.config.accent_color.clone());
            if let Some(profile) = self
                .config
                .space_profiles
                .iter_mut()
                .find(|p| p.workspace.eq_ignore_ascii_case(&ws))
            {
                profile.theme = theme;
                profile.accent_color = accent;
            }
        }
        if has("custom_css") {
            for tab in &self.tabs {
                tab.apply_custom_css(&self.config.custom_css);
            }
        }
        if has("sidebar_collapsed") {
            self.sidebar_hover_expanded = false;
            self.collapse_generation += 1;
            #[cfg(target_os = "macos")]
            crate::macos_menu::set_hover_expanded(false, self.sidebar_width());
            self.apply_sidebar_collapsed_change();
        } else if has("sidebar_width") {
            self.update_layout();
        }
        self.config_changed();
    }

    pub fn toggle_site_adblock(&mut self, domain: &str, enabled: bool) {
        let Some(domain_clean) = normalize_site_domain(domain) else {
            return;
        };
        if domain_clean == "localhost" {
            return;
        }
        let domain_root = domain_clean.trim_start_matches("www.");
        if enabled {
            self.config.adblock_disabled_domains.retain(|d| {
                let dl = d.trim().to_lowercase();
                let dl_root = dl.trim_start_matches("www.");
                dl != domain_clean
                    && dl_root != domain_root
                    && !domain_clean.ends_with(&format!(".{dl}"))
            });
        } else if !self.config.adblock_disabled_domains.iter().any(|d| {
            let dl = d.trim().to_lowercase();
            dl == domain_clean || domain_clean.ends_with(&format!(".{dl}"))
        }) {
            self.config.adblock_disabled_domains.push(domain_clean);
        }
        self.config_changed();

        if let Some(tab) = self.tabs.iter_mut().find(|t| t.id == self.active_tab_id) {
            tab.reload();
        }
    }

    pub fn toggle_extension(&mut self, id: &str, enabled: bool) {
        if let Some(ext) = self
            .config
            .installed_extensions
            .iter_mut()
            .find(|e| e.id == id)
        {
            ext.enabled = enabled;
            self.config_changed();
        }
    }

    pub fn delete_extension(&mut self, id: &str) {
        self.config.installed_extensions.retain(|e| e.id != id);
        self.config_changed();
    }

    /// Installs an unpacked extension (a directory or its manifest.json).
    /// Only files inside the extension directory are ever read. `store_id` is
    /// the Chrome Web Store id when installing from the store. Returns the
    /// extension's display name.
    pub fn install_extension_from_path(
        &mut self,
        path_str: &str,
        store_id: Option<&str>,
    ) -> Result<String, String> {
        let p = Path::new(path_str.trim());
        let manifest_path = if p.is_dir() {
            p.join("manifest.json")
        } else {
            p.to_path_buf()
        };
        let base_dir = manifest_path
            .parent()
            .and_then(|dir| std::fs::canonicalize(dir).ok())
            .ok_or_else(|| "Extension folder not found.".to_string())?;
        let content = read_extension_file(&base_dir, "manifest.json")
            .ok_or_else(|| "No readable manifest.json in that folder.".to_string())?;
        let json: serde_json::Value = serde_json::from_str(&content)
            .map_err(|err| format!("Invalid manifest.json: {err}"))?;

        let name = manifest_text(&base_dir, &json, "name").unwrap_or_else(|| "Custom Extension".into());
        let version = json["version"].as_str().unwrap_or("1.0.0").to_string();
        let description = manifest_text(&base_dir, &json, "description")
            .unwrap_or_else(|| "Loaded unpacked Chrome extension".into());

        let mut matches = Vec::new();
        let mut scripts = Vec::new();
        let mut styles = Vec::new();
        if let Some(cs_arr) = json["content_scripts"].as_array() {
            for cs in cs_arr {
                for m in cs["matches"].as_array().into_iter().flatten() {
                    if let Some(m_str) = m.as_str() {
                        matches.push(m_str.to_string());
                    }
                }
                for js in cs["js"].as_array().into_iter().flatten() {
                    if let Some(code) = js.as_str().and_then(|rel| read_extension_file(&base_dir, rel)) {
                        scripts.push(code);
                    }
                }
                for css in cs["css"].as_array().into_iter().flatten() {
                    if let Some(code) = css.as_str().and_then(|rel| read_extension_file(&base_dir, rel)) {
                        styles.push(code);
                    }
                }
            }
        }
        if matches.is_empty() {
            matches.push("<all_urls>".into());
        }

        let canonical_path = base_dir.to_string_lossy().to_string();
        let id = match store_id {
            Some(store_id) => format!("cws_{store_id}"),
            None => format!(
                "ext_{}",
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|d| d.as_millis())
                    .unwrap_or(0)
            ),
        };
        // Reinstalling replaces the same extension (same store id or same
        // folder); a different extension with the same name is kept.
        self.config
            .installed_extensions
            .retain(|e| e.id != id && e.path.as_deref() != Some(canonical_path.as_str()));
        self.config.installed_extensions.push(crate::config::ExtensionConfig {
            id,
            name: name.clone(),
            version,
            description,
            enabled: true,
            path: Some(canonical_path),
            matches,
            content_scripts: scripts,
            content_css: styles,
        });
        self.config_changed();
        Ok(name)
    }

    /// Downloads and unpacks a Chrome Web Store extension on a worker thread;
    /// the result comes back as `UserEvent::ExtensionDownloaded`.
    pub fn start_web_store_install(&mut self, input: &str) {
        let Some(ext_id) = extract_chrome_extension_id(input) else {
            self.show_toast("Invalid Chrome Web Store URL or ID. Expected 32-character extension ID.");
            return;
        };
        self.show_toast("Installing extension…");
        let proxy = self.proxy.clone();
        std::thread::spawn(move || {
            let result = download_web_store_extension(&ext_id);
            let _ = proxy.send_event(UserEvent::ExtensionDownloaded { ext_id, result });
        });
    }

    pub fn handle_extension_downloaded(&mut self, ext_id: String, result: Result<PathBuf, String>) {
        let outcome = result.and_then(|dir| self.install_extension_from_path(&dir.to_string_lossy(), Some(&ext_id)));
        match outcome {
            Ok(name) => self.show_toast(&format!("Installed {name}")),
            Err(err) => self.show_toast(&err),
        }
    }

    pub fn handle_page_started(&mut self, tab_id: usize, url: String) {
        // Error pages are loaded with load_html and commit about:blank; the
        // tab keeps reporting the URL that failed.
        if url == "about:blank" {
            return;
        }
        let canonical = canonicalize_eclipse_url(&url);
        if self.perf {
            eprintln!(
                "[perf] tab {tab_id} started {canonical} at {:.1} ms",
                self.started_at.elapsed().as_secs_f64() * 1000.0
            );
        }
        // A new document knows nothing of earlier pushes.
        if self.internal_pushed.is_some_and(|(id, _)| id == tab_id) {
            self.internal_pushed = None;
        }
        if tab_id == self.active_tab_id && is_http_url(&canonical) {
            self.last_web_url = canonical.clone();
        }
        let reference = self.reference_url();
        if let Some(tab) = self.tabs.iter_mut().find(|tab| tab.id == tab_id) {
            // A Tidy group describes the site, not the tab.
            if tab.group.is_some() && url_host(&tab.url) != url_host(&canonical) {
                tab.group = None;
            }
            tab.url = canonical.clone();
            tab.is_loading = !canonical.starts_with("eclipse://");
            tab.is_audible = false;
            tab.favicon_url = get_favicon_url(&canonical);
            tab.favicon = get_default_favicon(&canonical);
            if canonical.starts_with("eclipse://") {
                tab.title = title_for_url(&canonical);
            } else {
                tab.set_loading_indicator(true);
            }
            if tab.id == self.active_tab_id {
                tab.last_active = Instant::now();
            }
            sync_tab_config(tab, &self.config, self.config_revision, &reference);
        }
        self.broadcast_state();
    }

    pub fn handle_page_finished(&mut self, tab_id: usize, url: String) {
        if url == "about:blank" {
            if let Some(tab) = self.tabs.iter_mut().find(|tab| tab.id == tab_id) {
                tab.is_loading = false;
            }
            self.broadcast_state();
            return;
        }
        let canonical = canonicalize_eclipse_url(&url);
        if self.perf {
            eprintln!(
                "[perf] tab {tab_id} finished {canonical} at {:.1} ms",
                self.started_at.elapsed().as_secs_f64() * 1000.0
            );
        }
        let reference = self.reference_url();
        if let Some(tab) = self.tabs.iter_mut().find(|tab| tab.id == tab_id) {
            tab.url = canonical.clone();
            tab.is_loading = false;
            tab.set_loading_indicator(false);
            tab.favicon = get_default_favicon(&canonical);
            if tab.favicon_url.is_none() {
                tab.favicon_url = get_favicon_url(&canonical);
            }
            tab.restore_scroll_if_pending();
            sync_tab_config(tab, &self.config, self.config_revision, &reference);

            if self.perf && tab.id == self.active_tab_id {
                tab.evaluate_with_callback(
                    r#"JSON.stringify({
                        bridge: window.__eclipseBridgeActive === true,
                        find: typeof window.__eclipseFind,
                        shields: typeof window.__eclipseShields,
                        config: location.protocol === 'eclipse:' ? !!window.__ECLIPSE_CONFIG__ : 'hidden-from-web',
                        space: typeof window.__ECLIPSE_SPACE__,
                        configHandler: location.protocol === 'eclipse:' ? typeof window.onEclipseConfigUpdated : 'n/a'
                    })"#,
                    |result| eprintln!("[perf] page bridge: {result}"),
                );
            }
        }

        if self.perf && !self.first_page_logged {
            self.first_page_logged = true;
            eprintln!(
                "[perf] first page finished in {:.1} ms",
                self.started_at.elapsed().as_secs_f64() * 1000.0
            );
            if let Ok(sim) = std::env::var("ECLIPSE_SIMULATE_NAV") {
                eprintln!("[sim] simulating navigation to {sim}");
                self.navigate_active(&sim);
            }
        }
        self.broadcast_state();
    }

    pub fn handle_title_changed(&mut self, tab_id: usize, title: String) {
        if let Some(tab) = self.tabs.iter_mut().find(|tab| tab.id == tab_id)
            && !title.is_empty()
            && tab.title != title
        {
            tab.title = title;
            self.broadcast_state();
        }
    }

    /// window.open / target=_blank from a page that did not open a Little
    /// Eclipse window (`popup`: the page asked for a window of its own).
    /// Pages may not open internal eclipse:// pages.
    pub fn handle_new_window(&mut self, url: String, popup: bool) {
        match new_window_fallback(&url, popup) {
            NewWindowFallback::NewTabPage => {
                self.create_tab("eclipse://newtab", true);
            }
            NewWindowFallback::Tab => {
                self.create_tab(&url, true);
            }
            NewWindowFallback::External => {
                let _ = std::process::Command::new("open").arg(&url).spawn();
            }
            NewWindowFallback::Blocked => {
                self.show_toast("Pop-up window blocked");
            }
            NewWindowFallback::Ignore => {}
        }
    }

    pub fn handle_download_started(
        &mut self,
        seq: u64,
        file_name: String,
        url: String,
        path: String,
    ) {
        let id = self.next_download_id;
        self.next_download_id += 1;
        self.download_items.insert(seq, id);
        let item = DownloadItem {
            id,
            file_name: file_name.clone(),
            url,
            path,
            status: "downloading".into(),
            timestamp: now_unix_secs(),
        };
        self.config.downloads.insert(0, item);
        if self.config.downloads.len() > 50 {
            self.config.downloads.truncate(50);
        }
        self.show_toast(&format!("Downloading {file_name}…"));
        self.config_changed();
    }

    /// A completion names the download it belongs to by `seq` when its start
    /// was seen; otherwise it carries the original request URL (the name
    /// WebKit finally used may differ from what the URL suggests), so match
    /// by URL first and only fall back to the file name.
    pub fn handle_download_completed(
        &mut self,
        seq: Option<u64>,
        url: String,
        file_name: String,
        path: Option<String>,
        success: bool,
    ) {
        let status = if success { "completed" } else { "failed" };
        let started_item = seq.and_then(|seq| self.download_items.remove(&seq));
        let index = started_item
            .and_then(|id| {
                self.config
                    .downloads
                    .iter()
                    .position(|d| d.id == id && d.status == "downloading")
            })
            .or_else(|| find_download_for_completion(&self.config.downloads, &url, &file_name));
        let display_name = match index {
            Some(index) => {
                let item = &mut self.config.downloads[index];
                item.status = status.into();
                if let Some(p) = path.filter(|p| !p.is_empty()) {
                    item.path = p;
                }
                item.file_name.clone()
            }
            None => {
                let id = self.next_download_id;
                self.next_download_id += 1;
                self.config.downloads.insert(
                    0,
                    DownloadItem {
                        id,
                        file_name: file_name.clone(),
                        url,
                        path: path.unwrap_or_default(),
                        status: status.into(),
                        timestamp: now_unix_secs(),
                    },
                );
                file_name
            }
        };
        if success {
            self.show_toast(&format!("Downloaded {display_name}"));
        } else {
            self.show_toast(&format!("Download failed: {display_name}"));
        }
        self.config_changed();
    }

    /// WebKit reported a navigation error (see `classify_navigation_error`).
    /// A real failure before commit (DNS, offline, TLS, …) shows an error
    /// page with a retry button instead of a blank "Loading…" tab; one after
    /// commit leaves the partly loaded page alone.
    pub fn handle_navigation_failed(
        &mut self,
        tab_id: usize,
        committed: bool,
        domain: &str,
        code: i64,
        description: String,
        url: String,
    ) {
        let outcome = classify_navigation_error(committed, domain, code, &url);
        if self.perf {
            eprintln!("[nav] tab {tab_id} {outcome:?} ({domain} {code}) {url}: {description}");
        }
        let Some(tab) = self.tabs.iter_mut().find(|tab| tab.id == tab_id) else {
            return;
        };
        match outcome {
            NavigationError::Ignore => return,
            NavigationError::HandledByPlugin => {
                tab.mark_load_finished();
                let committed_url = tab.committed_url();
                let finished_url = if committed_url.is_empty() { url } else { committed_url };
                // A bare media document may never report a title.
                if tab.title == LOADING_TITLE {
                    tab.title = crate::tab::download_file_name(&finished_url);
                }
                self.refresh_title_from_page(tab_id);
                self.handle_page_finished(tab_id, finished_url);
                return;
            }
            NavigationError::Interrupted => {
                if tab.abandon_pending_load() {
                    self.after_abandoned_load(tab_id);
                }
                return;
            }
            NavigationError::Failed { committed: true } => {
                tab.mark_load_finished();
                tab.is_loading = false;
                tab.set_loading_indicator(false);
                self.broadcast_state();
                return;
            }
            NavigationError::Failed { committed: false } => {}
        }
        let failed_url = if url.is_empty() { tab.url.clone() } else { url };
        let description = if description.trim().is_empty() {
            format!("The page could not be loaded (error {code}).")
        } else {
            description
        };
        tab.is_loading = false;
        tab.set_loading_indicator(false);
        tab.url = failed_url.clone();
        tab.show_error_page(&failed_url, &description);
        self.broadcast_state();
    }

    /// The browser-started load of a tab ended without a new document and the
    /// tab went back to the document on screen: refresh what describes it.
    fn after_abandoned_load(&mut self, tab_id: usize) {
        let Some(tab) = self.tabs.iter_mut().find(|tab| tab.id == tab_id) else {
            return;
        };
        let url = tab.url.clone();
        if url.starts_with("eclipse://") {
            tab.title = title_for_url(&url);
        } else if is_http_url(&url) {
            // The title was replaced by the abandoned target's.
            self.refresh_title_from_page(tab_id);
        }
        if tab_id == self.active_tab_id && is_http_url(&url) {
            self.last_web_url = url;
        }
        let reference = self.reference_url();
        if let Some(tab) = self.tabs.iter_mut().find(|tab| tab.id == tab_id) {
            sync_tab_config(tab, &self.config, self.config_revision, &reference);
        }
        self.broadcast_state();
    }

    /// Re-reads the page's title (WebKit only reports it when it changes).
    fn refresh_title_from_page(&self, tab_id: usize) {
        let Some(tab) = self.tabs.iter().find(|tab| tab.id == tab_id) else {
            return;
        };
        let proxy = self.proxy.clone();
        tab.evaluate_with_callback("document.title", move |result| {
            if let Ok(title) = serde_json::from_str::<String>(&result) {
                let _ = proxy.send_event(UserEvent::TitleChanged { tab_id, title });
            }
        });
    }

    fn stop_active(&mut self) {
        let tab_id = self.active_tab_id;
        let abandoned = self
            .tabs
            .iter_mut()
            .find(|tab| tab.id == tab_id)
            .is_some_and(Tab::stop_loading);
        if abandoned {
            self.after_abandoned_load(tab_id);
        } else {
            self.broadcast_state();
        }
    }

    /// The sidebar's WebContent process died: reload the chrome document
    /// (its token bridge is a persistent user script, so it comes back with
    /// it) unless it keeps crashing, in which case it is retried later. The
    /// new document announces itself with `ready` and gets the full state.
    pub fn handle_sidebar_terminated(&mut self) {
        let now = Instant::now();
        self.sidebar_crashes
            .retain(|at| now.duration_since(*at) < CRASH_WINDOW);
        self.sidebar_crashes.push(now);
        self.sidebar_pushed_revision = None;
        self.command_bar_open = false;
        self.sidebar_interaction_active = false;
        self.hover_hold_until = None;
        self.release_sidebar_hold_if_idle();
        if self.sidebar_crashes.len() > MAX_CRASHES_IN_WINDOW {
            eprintln!(
                "[eclipse] the sidebar keeps crashing; retrying in {}s",
                CRASH_WINDOW.as_secs()
            );
            self.sidebar_reload_due = Some(now + CRASH_WINDOW);
        } else {
            self.reload_sidebar();
        }
    }

    fn reload_sidebar(&mut self) {
        self.sidebar_reload_due = None;
        let _ = self.sidebar_webview.load_html(crate::ui_assets::SIDEBAR_HTML);
    }

    /// The tab's WebContent process died. The active tab reloads right away
    /// (unless it keeps crashing); a background tab is hibernated and
    /// reloads when it is next selected.
    pub fn handle_web_content_terminated(&mut self, tab_id: usize) {
        let now = Instant::now();
        let history = self.crash_history.entry(tab_id).or_default();
        history.retain(|at| now.duration_since(*at) < CRASH_WINDOW);
        history.push(now);
        let crash_loop = history.len() > MAX_CRASHES_IN_WINDOW;
        let is_active = tab_id == self.active_tab_id;

        let Some(tab) = self.tabs.iter_mut().find(|tab| tab.id == tab_id) else {
            return;
        };
        if self.perf {
            eprintln!("[nav] tab {tab_id} WebContent terminated (crash loop: {crash_loop})");
        }
        tab.is_loading = false;
        tab.is_audible = false;
        if crash_loop {
            let url = tab.url.clone();
            tab.show_error_page(
                &url,
                "This page crashed repeatedly, so Eclipse stopped reloading it.",
            );
        } else if is_active {
            tab.reload();
        } else {
            tab.hibernate();
        }
        self.broadcast_state();
    }

    /// Actions from the native menu (menu.m owns every browser shortcut).
    pub fn handle_menu_action(&mut self, action: &str) {
        // A target=_blank link or noopener window.open inside a Little
        // Eclipse window (menu.m) opens as a tab, like one from a tab. The URL comes
        // from a web page: only http(s) is opened.
        // Also "Open in Eclipse" (⌘O) in a Little Eclipse window.
        if let Some(url) = action.strip_prefix("little_window_new_tab:") {
            if is_http_url(url) {
                self.create_tab(url, true);
            }
            return;
        }
        // Return in a Little Eclipse window's address field: what the user
        // typed, resolved to a web address or a search (never eclipse://).
        if let Some(rest) = action.strip_prefix("little_window_address:") {
            if let Some((id, text)) = rest.split_once(':')
                && let Ok(id) = id.parse::<u64>()
            {
                let url = little_window_url_from_input(text, &self.config);
                #[cfg(target_os = "macos")]
                crate::macos_menu::little_window_load(id, url.as_deref());
                #[cfg(not(target_os = "macos"))]
                let _ = (id, url);
            }
            return;
        }
        // Return in the New Little Eclipse Window bar (⌥⌘N): a suggestion's
        // address or what the user typed, resolved like the address field
        // (never eclipse://), opens in a new standalone Little Eclipse
        // window, or a tab past the window limit.
        if let Some(text) = action.strip_prefix("little_command_bar_open:") {
            match little_window_url_from_input(text, &self.config) {
                Some(url) => {
                    if !self.open_standalone_little_window(&url, false) {
                        self.open_external_link_in_tab(&url);
                    }
                }
                None => {
                    if self.perf {
                        eprintln!("[little] the command bar's text is not a web page");
                    }
                }
            }
            return;
        }
        if let Some(index) = action.strip_prefix("select_tab_index:") {
            if let Ok(index) = index.parse::<usize>() {
                self.select_tab_by_workspace_index(index.min(7));
            }
            return;
        }
        match action {
            "new_tab" => {
                self.create_tab("eclipse://newtab", true);
            }
            "new_little_window" => {
                self.show_little_command_bar();
            }
            // Handled by menu.m while a Little Eclipse window is in front
            // (the menu item is disabled otherwise).
            "open_in_eclipse" => {}
            "command_bar" => {
                self.open_command_bar();
            }
            "open_notes" => {
                self.open_notes_tab();
            }
            "open_easel" => {
                self.open_easel_tab();
            }
            "tidy_tabs" => {
                self.tidy_tabs();
            }
            "open_downloads" => {
                self.open_downloads_panel();
            }
            "close_tab" => self.close_tab(self.active_tab_id),
            "reopen_closed_tab" => self.reopen_closed_tab(),
            "close_window" => {
                let _ = self.proxy.send_event(UserEvent::CloseWindow);
            }
            "quit" => self.request_quit(),
            "focus_omnibar" => self.focus_omnibar(),
            "open_settings" => self.open_settings(),
            "open_settings_tab" => self.open_settings_tab(),
            "reload" => {
                if let Some(tab) = self.tabs.iter().find(|tab| tab.id == self.active_tab_id) {
                    tab.reload();
                }
            }
            "stop_loading" => self.stop_active(),
            "go_back" => {
                if let Some(tab) = self.tabs.iter().find(|tab| tab.id == self.active_tab_id) {
                    tab.go_back();
                }
                self.broadcast_state();
            }
            "go_forward" => {
                if let Some(tab) = self.tabs.iter().find(|tab| tab.id == self.active_tab_id) {
                    tab.go_forward();
                }
                self.broadcast_state();
            }
            "home" => self.navigate_active("eclipse://newtab"),
            "zoom_in" => self.zoom_active(0.1),
            "zoom_out" => self.zoom_active(-0.1),
            "zoom_reset" => self.zoom_active(0.0),
            "select_last_tab" => self.select_tab_by_workspace_index(8),
            "next_tab" => self.select_relative_tab(1),
            "prev_tab" => self.select_relative_tab(-1),
            "toggle_sidebar" => self.toggle_sidebar(),
            "hover_expand_sidebar" => self.hover_expand_sidebar(),
            "hover_collapse_sidebar" => self.hover_collapse_sidebar(),
            "toggle_fullscreen" => {
                #[cfg(target_os = "macos")]
                crate::macos_menu::toggle_fullscreen();
            }
            "open_devtools" => {
                if let Some(tab) = self.tabs.iter().find(|tab| tab.id == self.active_tab_id) {
                    tab.open_devtools();
                }
            }
            "find" => self.find_active("open"),
            "find_next" => self.find_active("next"),
            "find_prev" => self.find_active("prev"),
            "print" => self.print_active(),
            "favorite_tab" => self.favorite_active_tab(),
            "duplicate_tab" => {
                if let Some(tab) = self.tabs.iter().find(|tab| tab.id == self.active_tab_id) {
                    let url = tab.url.clone();
                    self.create_tab(&url, true);
                }
            }
            other => {
                if self.perf {
                    eprintln!("[menu] unhandled action {other}");
                }
            }
        }
    }

    /// Native print panel for the active tab (WKWebView print operation, so
    /// it works without the page's cooperation).
    fn print_active(&mut self) {
        if let Some(tab) = self.tabs.iter().find(|tab| tab.id == self.active_tab_id) {
            tab.print();
        }
    }

    pub fn favorite_active_tab(&mut self) {
        if let Some(tab) = self.tabs.iter().find(|tab| tab.id == self.active_tab_id) {
            let url = tab.url.clone();
            let title = if tab.title.trim().is_empty() {
                url.clone()
            } else {
                tab.title.clone()
            };
            if !url.starts_with("eclipse://") && !self.config.favorites.iter().any(|f| f.url == url)
            {
                let icon = get_default_favicon(&url);
                self.config.favorites.push(FavoriteShortcut { title, url, icon });
                self.show_toast("Added to favorites");
                self.config_changed();
            }
        }
    }

    /// Hibernates a background tab if it can safely lose its page state. A
    /// tab with Little Eclipse popups open stays awake: hibernating it would
    /// close them (a sign-in in progress while the user fetches a code from
    /// another tab).
    fn can_hibernate(tab: &Tab, active_id: usize) -> bool {
        tab.id != active_id
            && !tab.is_hibernated
            && tab.webview.is_some()
            && !tab.is_audible
            && !tab.is_loading
            && !tab.has_little_windows()
    }

    /// "Clean memory": hibernate every background tab that is idle right now,
    /// regardless of the idle-time threshold.
    pub fn clean_memory(&mut self) {
        let active_id = self.active_tab_id;
        let mut count = 0;
        for tab in &mut self.tabs {
            if Self::can_hibernate(tab, active_id) {
                tab.hibernate();
                count += 1;
            }
        }
        let message = match count {
            0 => "No background tabs to hibernate".to_string(),
            1 => "Hibernated 1 background tab".to_string(),
            n => format!("Hibernated {n} background tabs"),
        };
        self.show_toast(&message);
        self.broadcast_state();
    }

    pub fn handle_maintenance_tick(&mut self) {
        let mut hibernated_any = false;
        let minutes = self.config.tab_hibernation_mins;

        if minutes > 0 {
            let threshold = Duration::from_secs((minutes as u64) * 60);
            let now = Instant::now();
            let active_id = self.active_tab_id;

            for tab in &mut self.tabs {
                if Self::can_hibernate(tab, active_id)
                    && now.duration_since(tab.last_active) > threshold
                {
                    tab.hibernate();
                    hibernated_any = true;
                }
            }
        }

        self.memory_mb = get_process_memory_mb();
        if hibernated_any {
            self.broadcast_state();
            return;
        }
        if (self.memory_mb - self.last_broadcast_memory_mb).abs() >= 1.0 {
            self.last_broadcast_memory_mb = self.memory_mb;
            let script = format!(
                "window.updateMemoryUsage && window.updateMemoryUsage({:.1});",
                self.memory_mb
            );
            let _ = self.sidebar_webview.evaluate_script(&script);
        }
    }

    pub fn update_layout(&mut self) {
        self.update_layout_animated(0.0);
    }

    /// Lays out the sidebar and the visible tab. Hidden tabs get their bounds
    /// when they are selected.
    pub fn update_layout_animated(&mut self, duration: f64) {
        if self.sidebar_on_screen() {
            self.sidebar_hide_due = None;
            let _ = self.sidebar_webview.set_visible(true);
            self.bring_sidebar_to_front();
        } else {
            self.schedule_sidebar_hide();
        }
        // Routed through the native helper even when not animating, so an
        // in-flight animation is cancelled instead of overriding the frame.
        self.animate_sidebar_bounds(self.sidebar_bounds(), duration);

        let content_bounds = self.content_bounds();
        if let Some(tab) = self.tabs.iter().find(|tab| tab.id == self.active_tab_id) {
            tab.animate_bounds(content_bounds, duration);
        }
        self.broadcast_state();
    }

    /// Live window resize: only the visible webviews move, and nothing in the
    /// UI state changes, so there is no state push.
    pub fn resize(&mut self, width: f64, height: f64) {
        self.window_size = (width, height);
        self.animate_sidebar_bounds(self.sidebar_bounds(), 0.0);
        let content_bounds = self.content_bounds();
        if let Some(tab) = self.tabs.iter().find(|tab| tab.id == self.active_tab_id) {
            tab.animate_bounds(content_bounds, 0.0);
        }
    }

    pub fn broadcast_state(&mut self) {
        self.state_dirty = true;
        self.session_save_due
            .get_or_insert_with(|| Instant::now() + SESSION_SAVE_DELAY);
    }

    /// Runs deferred work that is due (state push, config/session saves,
    /// sidebar hold expiry and hiding) and returns when to wake up next.
    pub fn pump(&mut self) -> Option<Instant> {
        if !self.pending_external_links.is_empty() {
            self.open_pending_external_links();
        }
        let now = Instant::now();
        if self.state_dirty && now >= self.next_state_flush {
            self.flush_state();
        }
        if self.config_save_due.is_some_and(|due| now >= due) {
            self.save_config_now();
        }
        if self.session_save_due.is_some_and(|due| now >= due) {
            self.save_session();
        }
        if self.hover_hold_until.is_some_and(|until| now >= until) {
            self.hover_hold_until = None;
            self.release_sidebar_hold_if_idle();
        }
        if let Some((due, generation)) = self.sidebar_hide_due
            && now >= due
        {
            self.sidebar_hide_due = None;
            if generation == self.collapse_generation && !self.sidebar_on_screen() {
                let _ = self.sidebar_webview.set_visible(false);
            }
        }
        if self.sidebar_reload_due.is_some_and(|due| now >= due) {
            self.reload_sidebar();
        }
        // Closed internal tabs whose page never confirmed its flush.
        self.closing_tabs.retain(|_, (_, deadline)| now < *deadline);

        [
            self.state_dirty.then_some(self.next_state_flush),
            self.config_save_due,
            self.session_save_due,
            self.hover_hold_until,
            self.sidebar_hide_due.map(|(due, _)| due),
            self.sidebar_reload_due,
            self.closing_tabs.values().map(|(_, deadline)| *deadline).min(),
            self.quit_deadline,
        ]
        .into_iter()
        .flatten()
        .min()
    }

    /// Writes everything that is pending; called when the app quits.
    pub fn shutdown(&mut self) {
        #[cfg(target_os = "macos")]
        crate::macos_menu::close_all_little_windows();
        self.save_session();
        if self.config_save_due.is_some() {
            self.save_config_now();
        }
    }

    pub fn flush_state(&mut self) {
        self.state_dirty = false;
        self.next_state_flush = Instant::now() + STATE_FRAME_INTERVAL;

        let active_tab = self.tabs.iter().find(|tab| tab.id == self.active_tab_id);
        let active_url = active_tab.map(|tab| tab.url.clone()).unwrap_or_default();
        let active_title = active_tab.map(|tab| tab.title.clone()).unwrap_or_default();
        let is_loading = active_tab.map(|tab| tab.is_loading).unwrap_or(false);
        let can_go_back = active_tab.map(Tab::can_go_back).unwrap_or(false);
        let can_go_forward = active_tab.map(Tab::can_go_forward).unwrap_or(false);
        self.last_broadcast_memory_mb = self.memory_mb;

        let mut ui_state = BrowserUiState {
            tabs: self.tabs.iter().map(Tab::to_state).collect(),
            active_tab_id: self.active_tab_id,
            active_url,
            active_title: active_title.clone(),
            can_go_back,
            can_go_forward,
            is_loading,
            sidebar_collapsed: self.config.sidebar_collapsed,
            sidebar_hover_expanded: self.sidebar_hover_expanded,
            is_fullscreen: self.is_fullscreen,
            current_workspace: self.current_workspace.clone(),
            memory_mb: self.memory_mb,
            config: None,
            favorites: None,
            downloads: None,
            notes: None,
            easels: None,
            space_profiles: None,
        };
        let light_json = serde_json::to_string(&ui_state).ok();
        let revision = self.config_revision;
        let sidebar_needs_config = self.sidebar_pushed_revision != Some(revision);
        let internal_tab = active_tab.filter(|tab| tab.url.starts_with("eclipse://"));
        let internal_needs_config =
            internal_tab.is_some_and(|tab| self.internal_pushed != Some((tab.id, revision)));
        let full_json = if sidebar_needs_config || internal_needs_config {
            ui_state.config = Some(self.config.page_safe());
            ui_state.favorites = Some(self.config.favorites.clone());
            ui_state.downloads = Some(self.config.downloads.clone());
            ui_state.notes = Some(self.config.notes.clone());
            ui_state.easels = Some(self.config.easels.clone());
            ui_state.space_profiles = Some(self.config.space_profiles.clone());
            serde_json::to_string(&ui_state).ok()
        } else {
            None
        };

        let sidebar_json = if sidebar_needs_config { &full_json } else { &light_json };
        if let Some(json) = sidebar_json {
            let script = format!("window.updateBrowserState && window.updateBrowserState({json});");
            let _ = self.sidebar_webview.evaluate_script(&script);
            self.sidebar_pushed_revision = Some(revision);
        }

        if let Some(tab) = internal_tab {
            let json = if internal_needs_config { &full_json } else { &light_json };
            if let Some(json) = json {
                // Checked inside the page too: never hand state to a web
                // document that is still current while a navigation is pending.
                tab.evaluate_script(&format!(
                    "if (location.protocol === 'eclipse:' && typeof window.onEclipseStateUpdated === 'function') {{ window.onEclipseStateUpdated({json}); }}"
                ));
                self.internal_pushed = Some((tab.id, revision));
            }
        }

        let window_title = if active_title.is_empty() {
            "Eclipse".to_string()
        } else {
            format!("{active_title} — Eclipse")
        };
        if window_title != self.last_window_title {
            self.window.set_title(&window_title);
            self.last_window_title = window_title;
        }
    }

    /// Saves the open tabs (url, title, space, pin, favorite, zoom) and the
    /// active tab, if anything changed since the last save.
    fn save_session(&mut self) {
        self.session_save_due = None;
        if !self.persist_session {
            return;
        }
        let mut session = Session {
            tabs: Vec::new(),
            active_index: None,
            current_workspace: self.current_workspace.clone(),
        };
        for tab in self.tabs.iter().filter(|tab| is_restorable_url(&tab.url)) {
            if session.tabs.len() >= MAX_SESSION_TABS {
                break;
            }
            if tab.id == self.active_tab_id {
                session.active_index = Some(session.tabs.len());
            }
            session.tabs.push(SessionTab {
                url: tab.url.clone(),
                title: tab.title.clone(),
                workspace: tab.workspace.clone(),
                is_pinned: tab.is_pinned,
                favorite_url: tab
                    .favorite_index
                    .and_then(|index| self.config.favorites.get(index))
                    .map(|favorite| favorite.url.clone()),
                zoom: tab.zoom,
            });
        }
        let Ok(json) = serde_json::to_string_pretty(&session) else {
            return;
        };
        if json != self.last_session_json {
            write_atomic(&Session::path(), &json);
            self.last_session_json = json;
        }
    }

    /// Recreates the saved tabs. Only the active one gets a webview now; the
    /// rest are restored hibernated and load when selected. Returns false if
    /// there was nothing to restore.
    fn restore_session(&mut self) -> bool {
        let Some(session) = Session::load() else {
            return false;
        };
        let workspace = session.current_workspace;
        let active_index = session.active_index;
        let mut records = Vec::new();
        // Enumerated before filtering: `active_index` is a position in the
        // file as written, so a hand-edited or stale entry cannot shift it.
        for (index, record) in session
            .tabs
            .into_iter()
            .enumerate()
            .filter(|(_, record)| is_restorable_url(&record.url))
            .take(MAX_SESSION_TABS)
        {
            let favorite_index = record
                .favorite_url
                .as_ref()
                .and_then(|url| self.config.favorites.iter().position(|f| &f.url == url));
            records.push((index, record, favorite_index));
        }
        let keys: Vec<RestoreKey> = records
            .iter()
            .map(|(index, record, favorite_index)| RestoreKey {
                index: *index,
                workspace: &record.workspace,
                in_space_list: !record.is_pinned && favorite_index.is_none(),
            })
            .collect();
        let Some(active_position) = restored_active_position(&keys, active_index, &workspace) else {
            return false;
        };
        let mut active_id = None;
        for (position, (_, record, favorite_index)) in records.into_iter().enumerate() {
            let id = self.next_tab_id;
            self.next_tab_id += 1;
            if position == active_position {
                active_id = Some(id);
            }
            self.tabs.push(hibernated_tab(id, record, favorite_index));
        }
        let Some(active_id) = active_id else {
            return false;
        };
        self.current_workspace = workspace.clone();
        let profile = self.config.profile_for_workspace(&workspace);
        self.config.theme = profile.theme;
        self.config.accent_color = profile.accent_color;
        self.select_tab(active_id);
        true
    }

    fn zoom_active(&mut self, delta: f64) {
        let Some(tab) = self
            .tabs
            .iter_mut()
            .find(|tab| tab.id == self.active_tab_id)
        else {
            return;
        };
        let next = if delta == 0.0 { 1.0 } else { tab.zoom + delta };
        let previous = tab.zoom;
        let zoom = tab.set_zoom(next);
        self.show_toast(&format!("Zoom {:.0}%", zoom * 100.0));
        // The site popover shows the zoom, and the session saves it.
        if (zoom - previous).abs() > f64::EPSILON {
            self.broadcast_state();
        }
    }

    fn find_active(&mut self, action: &str) {
        if let Some(tab) = self.tabs.iter().find(|tab| tab.id == self.active_tab_id) {
            tab.focus();
            tab.find(action);
        }
    }

    pub fn show_toast(&self, message: &str) {
        let Ok(encoded) = serde_json::to_string(message) else {
            return;
        };
        let script = format!("window.showToast && window.showToast({encoded});");
        let _ = self.sidebar_webview.evaluate_script(&script);
    }

    fn probe_sidebar(&self) {
        let script = r#"(function () {
            function text(id) {
                var el = document.getElementById(id);
                if (!el) return null;
                return el.value !== undefined ? el.value : el.textContent;
            }
            var drawer = document.getElementById('settings-drawer');
            return JSON.stringify({
                tokenBridge: !!window.__eclipseIpc,
                stateHandler: typeof window.updateBrowserState,
                commandBar: typeof window.showCommandBar,
                commandBarElement: !!document.getElementById('arc-command-bar'),
                tabCards: document.querySelectorAll('#tab-container .tab-card').length,
                pinnedItems: document.querySelectorAll('#pinned-container .pinned-item').length,
                omnibar: text('omnibar'),
                ram: text('ram-text'),
                workspace: text('workspace-label'),
                drawer: drawer ? getComputedStyle(drawer).display : null,
                sidebarWidth: window.innerWidth,
                overflowX: document.documentElement.scrollWidth > document.documentElement.clientWidth + 1
            });
        })()"#;
        let _ = self
            .sidebar_webview
            .evaluate_script_with_callback(script, |result| {
                eprintln!("[perf] sidebar probe: {result}");
            });
    }
}

/// Groups a tab for Tidy Tabs by its host (matched as a domain suffix, so
/// dropbox.com is not "x.com" and a `?next=localhost` query means nothing).
pub fn categorize_tab_group(url: &str, title: &str) -> String {
    if url.starts_with("eclipse://") {
        return "Eclipse Workspace".into();
    }
    let parsed = url::Url::parse(url).ok();
    let host = parsed
        .as_ref()
        .and_then(|u| u.host_str())
        .map(|h| h.to_ascii_lowercase())
        .unwrap_or_default();
    let path = parsed.as_ref().map(|u| u.path()).unwrap_or("");
    let on = |domains: &[&str]| domains.iter().any(|d| host_matches(&host, d));

    if on(&["github.com", "gitlab.com", "stackoverflow.com", "crates.io", "docs.rs", "vercel.com"])
        || host == "localhost"
        || host == "127.0.0.1"
        || title.to_lowercase().contains("pull request")
    {
        return "Development".into();
    }
    if on(&["youtube.com", "youtu.be", "netflix.com", "spotify.com", "twitch.tv", "vimeo.com"]) {
        return "Media & Stream".into();
    }
    if on(&["mail.google.com", "docs.google.com", "notion.so", "linear.app", "figma.com"])
        || host.starts_with("outlook.")
    {
        return "Work & Productivity".into();
    }
    if on(&["discord.com", "reddit.com", "instagram.com", "x.com", "twitter.com", "linkedin.com"]) {
        return "Social & Community".into();
    }
    if on(&["duckduckgo.com", "wikipedia.org", "arxiv.org", "chatgpt.com", "claude.ai"])
        || (on(&["google.com"]) && path.starts_with("/search"))
    {
        return "Research & Search".into();
    }
    let clean = host.trim_start_matches("www.");
    if let Some(first) = clean.split('.').next()
        && !first.is_empty()
    {
        let mut chars = first.chars();
        if let Some(c) = chars.next() {
            return format!("{}{}", c.to_uppercase(), chars.as_str());
        }
    }
    "General".into()
}

fn host_matches(host: &str, domain: &str) -> bool {
    host == domain
        || host
            .strip_suffix(domain)
            .is_some_and(|prefix| prefix.ends_with('.'))
}

fn url_host(url: &str) -> Option<String> {
    url::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(str::to_ascii_lowercase))
}

fn is_http_url(url: &str) -> bool {
    url.starts_with("http://") || url.starts_with("https://")
}

/// The only documents the sidebar webview may show: its initial HTML (loaded
/// with a nil base URL, i.e. about:blank) and eclipse:// resources.
fn is_sidebar_document_url(url: &str) -> bool {
    url == "about:blank" || url.starts_with("eclipse:")
}

/// Saved tabs worth restoring: web pages and the internal pages. Error pages
/// (load_html → about:blank), data:, blob:, file: and the like are skipped.
fn is_restorable_url(url: &str) -> bool {
    is_http_url(url)
        || matches!(
            url,
            "eclipse://newtab" | "eclipse://settings" | "eclipse://notes" | "eclipse://easel"
        )
}

fn now_unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Internal = an eclipse:// frame while the tab's committed document is an
/// internal page. The classification follows the committed document, never
/// the URL the tab is loading, so a load that never commits (a download, a
/// refused or stopped navigation) cannot demote the page still on screen,
/// and a web document still current while an eclipse:// load is pending is
/// never promoted. An eclipse:// frame right after the tab left an internal
/// page is that page saving as it unloads (web documents cannot host
/// eclipse:// frames). Everything else, including the error page, is a
/// website.
pub fn classify_tab_sender(origin: &str, document: DocumentTrust) -> IpcSender {
    let origin_is_eclipse = url::Url::parse(origin).is_ok_and(|u| u.scheme() == "eclipse");
    if !origin_is_eclipse {
        IpcSender::Web
    } else if document.internal {
        IpcSender::Internal
    } else if document.left_internal {
        IpcSender::DepartingInternal
    } else {
        IpcSender::Web
    }
}

/// Who posted a message through a tab's IPC handler. WebKit's word that a
/// Little Eclipse popup posted it wins over everything else: such a popup
/// shares the tab's handler and can be on the tab's own origin, so origin
/// checks cannot tell it from the tab's page. A message from a closing tab's
/// internal page may only save what it still had pending.
fn tab_ipc_sender(
    origin: &str,
    document: DocumentTrust,
    closing: bool,
    from_little_window: bool,
) -> IpcSender {
    if from_little_window {
        return IpcSender::LittleWindow;
    }
    match classify_tab_sender(origin, document) {
        IpcSender::Internal if closing => IpcSender::DepartingInternal,
        sender => sender,
    }
}

/// Both URLs parse and share a tuple (scheme, host, port) origin; opaque
/// origins (about:, data:, eclipse:, file:) never match.
fn is_same_tuple_origin(a: &str, b: &str) -> bool {
    let origin = |value: &str| url::Url::parse(value).ok().map(|parsed| parsed.origin());
    match (origin(a), origin(b)) {
        (Some(a), Some(b)) => a.is_tuple() && a == b,
        _ => false,
    }
}

/// A `url_changed` report comes from the committed web document itself:
/// the new URL, the posting frame and the committed document share an
/// origin (pushState cannot leave it), so a late report from a page the tab
/// already left, or from an internal or error page, is ignored.
fn is_plausible_url_report(url: &str, frame_origin: &str, committed_url: &str) -> bool {
    is_http_url(url)
        && is_same_tuple_origin(url, frame_origin)
        && is_same_tuple_origin(frame_origin, committed_url)
}

/// A website's report about the tab's page (its URL, favicon, audio or
/// scroll position) must come from the tab's committed document: a frame of
/// another origin is a page the tab already left. (Little Eclipse popups are
/// told apart by WebKit, not by origin: see `IpcSender::LittleWindow`.)
/// Other commands are left to the sender allowlist.
fn is_plausible_page_report(command: &IpcCommand, frame_origin: &str, committed_url: &str) -> bool {
    match command {
        IpcCommand::UrlChanged { url } => is_plausible_url_report(url, frame_origin, committed_url),
        IpcCommand::UpdateFavicon { .. }
        | IpcCommand::SetAudible { .. }
        | IpcCommand::UpdateScroll { .. } => is_same_tuple_origin(frame_origin, committed_url),
        _ => true,
    }
}

/// The per-sender command allowlist (C1). Websites may only report their own
/// page state and open http(s) links in new tabs; a Little Eclipse popup is
/// not the tab's page, so it may only open http(s) links in new tabs.
pub fn is_command_allowed(sender: IpcSender, command: &IpcCommand) -> bool {
    match sender {
        IpcSender::Sidebar => true,
        IpcSender::Internal => !matches!(
            command,
            IpcCommand::WindowClose
                | IpcCommand::WindowMinimize
                | IpcCommand::WindowZoom
                | IpcCommand::DragWindow
        ),
        IpcSender::DepartingInternal => matches!(
            command,
            IpcCommand::UpdateSettings { .. }
                | IpcCommand::SaveNote { .. }
                | IpcCommand::SaveEasel { .. }
        ),
        IpcSender::Web => match command {
            IpcCommand::UrlChanged { .. }
            | IpcCommand::UpdateFavicon { .. }
            | IpcCommand::SetAudible { .. }
            | IpcCommand::UpdateScroll { .. } => true,
            IpcCommand::NewTab { url: Some(url), .. } => is_http_url(url.trim()),
            _ => false,
        },
        IpcSender::LittleWindow => {
            matches!(command, IpcCommand::NewTab { url: Some(url), .. } if is_http_url(url.trim()))
        }
    }
}

/// The variant name of a command, for logs (never its payload).
fn command_name(command: &IpcCommand) -> String {
    let debug = format!("{command:?}");
    debug
        .split([' ', '{', '('])
        .next()
        .unwrap_or_default()
        .to_string()
}

fn parse_ipc(raw: &str, limit: usize) -> Option<(String, IpcCommand)> {
    if raw.len() > limit {
        return None;
    }
    let mut value: serde_json::Value = serde_json::from_str(raw).ok()?;
    let token = value
        .as_object_mut()?
        .remove("token")?
        .as_str()?
        .to_string();
    if token.len() != 32 || !token.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    let command = serde_json::from_value(value).ok()?;
    Some((token, command))
}

fn is_safe_icon_url(url: &str) -> bool {
    url.len() <= 2048 && (url.starts_with("https://") || url.starts_with("http://"))
}

/// Shortens a message for the log and strips control characters so page
/// text cannot inject terminal escape sequences.
fn truncate_message(message: &str, limit: usize) -> String {
    let mut truncated: String = message
        .chars()
        .filter(|c| !c.is_control() || *c == ' ')
        .take(limit)
        .collect();
    if message.chars().count() > limit {
        truncated.push('…');
    }
    truncated
}

/// What a window.open or target=_blank link that did not open a Little
/// Eclipse window does instead.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NewWindowFallback {
    /// Open the URL in a new tab.
    Tab,
    /// Open the new-tab page.
    NewTabPage,
    /// Hand the URL to the system (mailto:, tel:).
    External,
    /// Show that a popup was blocked.
    Blocked,
    /// Do nothing (internal pages, other schemes).
    Ignore,
}

fn new_window_fallback(url: &str, popup: bool) -> NewWindowFallback {
    if url.is_empty() || url == "about:blank" {
        // A blank popup (no user gesture, or over the popup limit) only
        // makes sense to the script that opened it, and that script got null
        // instead of a window: an empty tab would just be left behind.
        return if popup {
            NewWindowFallback::Blocked
        } else {
            NewWindowFallback::NewTabPage
        };
    }
    if is_http_url(url) {
        NewWindowFallback::Tab
    } else if url.starts_with("mailto:") || url.starts_with("tel:") {
        NewWindowFallback::External
    } else {
        NewWindowFallback::Ignore
    }
}

/// Whether the app can exit now: quitting was requested (`deadline` is set)
/// and nothing is still flushing, or the wait for it timed out.
fn quit_ready(deadline: Option<Instant>, flushes_pending: bool, now: Instant) -> bool {
    deadline.is_some_and(|deadline| !flushes_pending || now >= deadline)
}

/// The "current site" after tabs closed: `current` while a tab still shows
/// it, else the most recently active open web page, else nothing.
fn surviving_web_url(current: &str, open: &[(&str, Instant)]) -> String {
    if current.is_empty() || open.iter().any(|(url, _)| *url == current) {
        return current.to_string();
    }
    open.iter()
        .filter(|(url, _)| is_http_url(url))
        .max_by_key(|(_, last_active)| *last_active)
        .map(|(url, _)| url.to_string())
        .unwrap_or_default()
}

/// What session restore needs to know about a saved tab to pick the active
/// one.
struct RestoreKey<'a> {
    /// Position in session.json as written.
    index: usize,
    workspace: &'a str,
    /// Neither pinned nor a favorite's tab (those show in every space).
    in_space_list: bool,
}

/// Which restored tab (position in `keys`) becomes active: the one saved as
/// active, matched by its position in the file (URLs repeat across tabs and
/// spaces), unless it belongs to another space than the saved one; then the
/// saved space's first tab, then the first tab. None when nothing restores.
fn restored_active_position(
    keys: &[RestoreKey],
    active_index: Option<usize>,
    workspace: &str,
) -> Option<usize> {
    let first_in_space = keys
        .iter()
        .position(|key| key.in_space_list && key.workspace == workspace);
    let saved_active = active_index
        .and_then(|index| keys.iter().position(|key| key.index == index))
        .filter(|&position| {
            let key = &keys[position];
            !key.in_space_list || key.workspace == workspace
        });
    saved_active
        .or(first_in_space)
        .or_else(|| (!keys.is_empty()).then_some(0))
}

/// What the ⌘1-9 / next / prev ordering needs to know about a tab.
struct TabOrderKey<'a> {
    id: usize,
    favorite_index: Option<usize>,
    is_pinned: bool,
    workspace: &'a str,
}

/// Mirrors the sidebar's layout: favorites' tabs by favorite position, then
/// pinned tabs, then the current space's regular tabs, each in list order.
fn display_order(tabs: &[TabOrderKey], workspace: &str) -> Vec<usize> {
    let mut favorites: Vec<(usize, usize)> = tabs
        .iter()
        .filter_map(|tab| tab.favorite_index.map(|index| (index, tab.id)))
        .collect();
    favorites.sort_by_key(|(index, _)| *index);
    favorites
        .into_iter()
        .map(|(_, id)| id)
        .chain(
            tabs.iter()
                .filter(|tab| tab.is_pinned && tab.favorite_index.is_none())
                .map(|tab| tab.id),
        )
        .chain(
            tabs.iter()
                .filter(|tab| !tab.is_pinned && tab.favorite_index.is_none() && tab.workspace == workspace)
                .map(|tab| tab.id),
        )
        .collect()
}

/// Where the favorite at `index` ends up after `Vec::remove(from)` +
/// `Vec::insert(to, _)`.
fn index_after_move(index: usize, from: usize, to: usize) -> usize {
    if index == from {
        to
    } else if from < to && index > from && index <= to {
        index - 1
    } else if to < from && index >= to && index < from {
        index + 1
    } else {
        index
    }
}

/// The download a completion belongs to: the newest one still downloading
/// with the same request URL, else the newest one with that file name.
/// `downloads` is newest first.
fn find_download_for_completion(downloads: &[DownloadItem], url: &str, file_name: &str) -> Option<usize> {
    let downloading = |d: &&DownloadItem| d.status == "downloading";
    downloads
        .iter()
        .position(|d| downloading(&d) && !url.is_empty() && d.url == url)
        .or_else(|| {
            downloads
                .iter()
                .position(|d| downloading(&d) && !file_name.is_empty() && d.file_name == file_name)
        })
}

/// A restored tab without a webview; `select_tab` builds it on demand.
fn hibernated_tab(id: usize, record: SessionTab, favorite_index: Option<usize>) -> Tab {
    let title = if record.title.trim().is_empty() {
        title_for_url(&record.url)
    } else {
        record.title
    };
    Tab::hibernated(
        id,
        &record.url,
        title,
        &record.workspace,
        record.is_pinned || favorite_index.is_some(),
        record.zoom,
        favorite_index,
    )
}

/// Pushes config to one tab (skipped when it already has this revision).
fn sync_tab_config(tab: &mut Tab, config: &BrowserConfig, revision: u64, active_url: &str) {
    tab.sync_config(config, revision, Some(active_url));
}


/// Reads `rel` from an extension directory (`base` must be canonical). The
/// resolved file must stay inside `base`: absolute paths, `..` and symlinks
/// pointing elsewhere are refused.
fn read_extension_file(base: &Path, rel: &str) -> Option<String> {
    let rel = Path::new(rel.trim_start_matches('/'));
    if rel.as_os_str().is_empty()
        || rel
            .components()
            .any(|c| !matches!(c, Component::Normal(_) | Component::CurDir))
    {
        return None;
    }
    let resolved = std::fs::canonicalize(base.join(rel)).ok()?;
    if !resolved.starts_with(base) {
        return None;
    }
    std::fs::read_to_string(resolved).ok()
}

/// A manifest text field, resolving `__MSG_key__` placeholders from
/// `_locales/<default_locale>/messages.json`.
fn manifest_text(base: &Path, manifest: &serde_json::Value, field: &str) -> Option<String> {
    let raw = manifest[field].as_str()?.trim();
    let Some(key) = raw.strip_prefix("__MSG_").and_then(|k| k.strip_suffix("__")) else {
        return (!raw.is_empty()).then(|| raw.to_string());
    };
    let locale = manifest["default_locale"].as_str().unwrap_or("en");
    let messages: serde_json::Value = read_extension_file(base, &format!("_locales/{locale}/messages.json"))
        .and_then(|content| serde_json::from_str(&content).ok())?;
    messages
        .as_object()?
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case(key))
        .and_then(|(_, entry)| entry["message"].as_str())
        .map(|message| message.trim().to_string())
        .filter(|message| !message.is_empty())
}

/// Downloads a Web Store CRX and unpacks it into
/// `~/.config/eclipse-browser/extensions/<id>`. Runs on a worker thread.
fn download_web_store_extension(ext_id: &str) -> Result<PathBuf, String> {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    let extensions_dir = PathBuf::from(home)
        .join(".config")
        .join("eclipse-browser")
        .join("extensions");
    std::fs::create_dir_all(&extensions_dir)
        .map_err(|e| format!("Failed to create the extensions folder: {e}"))?;
    let ext_dir = extensions_dir.join(ext_id);
    let crx_path = extensions_dir.join(format!("{ext_id}.crx.download"));
    let zip_path = extensions_dir.join(format!("{ext_id}.zip"));

    let download_url = format!(
        "https://clients2.google.com/service/update2/crx?response=redirect&prodversion=130.0&acceptformat=crx2,crx3&x=id%3D{ext_id}%26uc"
    );
    let status = std::process::Command::new("/usr/bin/curl")
        .args(["-s", "-L", "--fail", "--connect-timeout", "10", "--max-time", "60"])
        .arg(&download_url)
        .arg("-o")
        .arg(&crx_path)
        .status()
        .map_err(|e| format!("Failed to download extension: {e}"))?;
    if !status.success() {
        let _ = std::fs::remove_file(&crx_path);
        return Err("Failed to download the extension from the Chrome Web Store.".into());
    }

    let bytes = std::fs::read(&crx_path).map_err(|e| format!("Failed to read CRX: {e}"));
    let _ = std::fs::remove_file(&crx_path);
    let zip = crx_zip_payload(&bytes?)?.to_vec();
    std::fs::write(&zip_path, zip).map_err(|e| format!("Failed to write the archive: {e}"))?;

    // Start from an empty folder so files from an older version never linger.
    let _ = std::fs::remove_dir_all(&ext_dir);
    std::fs::create_dir_all(&ext_dir).map_err(|e| format!("Failed to create {ext_dir:?}: {e}"))?;
    let unzip_status = std::process::Command::new("/usr/bin/unzip")
        .arg("-q")
        .arg("-o")
        .arg(&zip_path)
        .arg("-d")
        .arg(&ext_dir)
        .status()
        .map_err(|e| format!("Failed to extract extension: {e}"));
    let _ = std::fs::remove_file(&zip_path);
    // Archives may carry symlinks; none of them may point the installer at
    // files outside the extension.
    remove_symlinks(&ext_dir);
    if !unzip_status?.success() || !ext_dir.join("manifest.json").is_file() {
        return Err("Failed to unpack the extension manifest.".into());
    }
    Ok(ext_dir)
}

/// The ZIP inside a CRX2/CRX3 container (or the bytes themselves if they
/// already are a ZIP), located from the header rather than by searching.
fn crx_zip_payload(bytes: &[u8]) -> Result<&[u8], String> {
    const ZIP_MAGIC: &[u8] = b"PK\x03\x04";
    let invalid = || "Downloaded file is not a valid Chrome extension archive.".to_string();
    if bytes.starts_with(ZIP_MAGIC) {
        return Ok(bytes);
    }
    let le_u32 = |at: usize| -> Option<usize> {
        let field: [u8; 4] = bytes.get(at..at + 4)?.try_into().ok()?;
        Some(u32::from_le_bytes(field) as usize)
    };
    if !bytes.starts_with(b"Cr24") {
        return Err(invalid());
    }
    let offset = match le_u32(4).ok_or_else(invalid)? {
        3 => 12usize.checked_add(le_u32(8).ok_or_else(invalid)?),
        2 => 16usize
            .checked_add(le_u32(8).ok_or_else(invalid)?)
            .and_then(|n| n.checked_add(le_u32(12)?)),
        _ => None,
    }
    .ok_or_else(invalid)?;
    bytes
        .get(offset..)
        .filter(|zip| zip.starts_with(ZIP_MAGIC))
        .ok_or_else(invalid)
}

fn remove_symlinks(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        match std::fs::symlink_metadata(&path) {
            Ok(meta) if meta.file_type().is_symlink() => {
                let _ = std::fs::remove_file(&path);
            }
            Ok(meta) if meta.is_dir() => remove_symlinks(&path),
            _ => {}
        }
    }
}

fn build_sidebar_bridge_script(token: &str) -> String {
    let encoded_token = serde_json::to_string(token).unwrap_or_else(|_| "\"\"".into());
    format!(
        r##"(function () {{
            var raw = window.ipc && typeof window.ipc.postMessage === 'function'
                ? window.ipc.postMessage.bind(window.ipc)
                : null;
            var stringify = JSON.stringify;
            var token = {encoded_token};
            try {{
                Object.defineProperty(window, '__eclipseIpc', {{
                    value: function (payload) {{
                        if (!raw || !payload || typeof payload !== 'object') return;
                        payload.token = token;
                        try {{ raw(stringify(payload)); }} catch (_) {{}}
                    }},
                    writable: false,
                    configurable: false,
                    enumerable: false
                }});
            }} catch (_) {{}}
        }})();"##
    )
}

pub fn title_for_url(url: &str) -> String {
    if url == "eclipse://settings" {
        "Preferences".into()
    } else if url == "eclipse://notes" {
        "Eclipse Notes".into()
    } else if url == "eclipse://easel" {
        "Eclipse Easel".into()
    } else if url.starts_with("eclipse://") {
        "New Tab".into()
    } else {
        LOADING_TITLE.into()
    }
}

pub fn normalize_url(input: &str, config: &BrowserConfig) -> String {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return "eclipse://newtab".to_string();
    }
    if trimmed.starts_with("eclipse://") {
        return canonicalize_eclipse_url(trimmed);
    }

    let lower = trimmed.to_ascii_lowercase();
    if lower == "https://youtube.com" || lower == "http://youtube.com" {
        return "https://www.youtube.com".to_string();
    }
    if lower.starts_with("https://youtube.com/") {
        return format!(
            "https://www.youtube.com{}",
            &trimmed["https://youtube.com".len()..]
        );
    }
    if lower.starts_with("http://youtube.com/") {
        return format!(
            "https://www.youtube.com{}",
            &trimmed["http://youtube.com".len()..]
        );
    }
    if lower.starts_with("http://") || lower.starts_with("https://") {
        return trimmed.to_string();
    }

    if !trimmed.contains(char::is_whitespace) {
        if lower == "localhost"
            || lower.starts_with("localhost:")
            || lower.starts_with("localhost/")
        {
            return format!("http://{trimmed}");
        }
        if lower.starts_with("127.0.0.1:") || lower.starts_with("127.0.0.1/") {
            return format!("http://{trimmed}");
        }
        if lower == "youtube.com" {
            return "https://www.youtube.com".to_string();
        }
        if lower.starts_with("youtube.com/") {
            return format!(
                "https://www.youtube.com{}",
                &trimmed["youtube.com".len()..]
            );
        }
        if trimmed.contains('.')
            && !lower.starts_with("file:")
            && !lower.starts_with("javascript:")
        {
            return format!("https://{trimmed}");
        }
    }

    config.get_search_url(trimmed)
}

/// What a Little Eclipse window's address field loads for `input`: the
/// address bar's rule (`normalize_url`: a web address as typed, a bare word
/// or phrase searched with the configured engine), but only ever an http(s)
/// page. Internal `eclipse://` pages are refused, never searched for.
pub fn little_window_url_from_input(input: &str, config: &BrowserConfig) -> Option<String> {
    let trimmed = input.trim();
    if trimmed.is_empty() || trimmed.len() > 8192 || trimmed.to_ascii_lowercase().starts_with("eclipse:") {
        return None;
    }
    let url = normalize_url(trimmed, config);
    let parsed = url::Url::parse(&url).ok()?;
    let web = matches!(parsed.scheme(), "http" | "https") && parsed.host_str().is_some_and(|h| !h.is_empty());
    web.then_some(url)
}

/// An open tab as the New Little Eclipse Window bar sees it.
pub(crate) struct CommandBarTab<'a> {
    pub title: &'a str,
    pub url: &'a str,
    /// In the Space on screen.
    pub current_space: bool,
    pub last_active: Instant,
}

const COMMAND_BAR_SUGGESTIONS: usize = 64;

/// What the New Little Eclipse Window bar (⌥⌘N) suggests, as (title, URL):
/// the current Space's tabs, most recently used first, then the favourites,
/// then other Spaces' tabs. Web pages only, each address once, titles
/// cleaned of control characters (the URL's host stands in for a missing
/// one).
pub(crate) fn little_command_bar_suggestions(
    tabs: &[CommandBarTab],
    favorites: &[FavoriteShortcut],
) -> Vec<(String, String)> {
    let mut ordered: Vec<&CommandBarTab> = tabs.iter().collect();
    ordered.sort_by_key(|tab| (!tab.current_space, std::cmp::Reverse(tab.last_active)));
    let (current, other): (Vec<&CommandBarTab>, Vec<&CommandBarTab>) =
        ordered.into_iter().partition(|tab| tab.current_space);
    let candidates = current
        .iter()
        .map(|tab| (tab.title, tab.url))
        .chain(favorites.iter().map(|favorite| (favorite.title.as_str(), favorite.url.as_str())))
        .chain(other.iter().map(|tab| (tab.title, tab.url)));
    let mut seen = HashSet::new();
    let mut suggestions = Vec::new();
    for (title, url) in candidates {
        let Ok(parsed) = url::Url::parse(url.trim()) else {
            continue;
        };
        if !matches!(parsed.scheme(), "http" | "https") {
            continue;
        }
        let Some(host) = parsed.host_str().filter(|host| !host.is_empty()).map(str::to_string) else {
            continue;
        };
        let url = String::from(parsed);
        if !seen.insert(url.clone()) {
            continue;
        }
        let title = title
            .chars()
            .take(200)
            .map(|c| if c.is_control() { ' ' } else { c })
            .collect::<String>()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        let title = if title.is_empty() { host } else { title };
        suggestions.push((title, url));
        if suggestions.len() == COMMAND_BAR_SUGGESTIONS {
            break;
        }
    }
    suggestions
}

/// Where a link another app asked Eclipse to open goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ExternalLinkRoute {
    /// A standalone Little Eclipse window, like Little Arc.
    LittleWindow(String),
    /// A new foreground tab in the browser window.
    Tab(String),
    /// Not a web page (file:, eclipse:, javascript: ...): not opened.
    Ignore,
}

pub(crate) fn external_link_route(url: &str, little_windows: bool) -> ExternalLinkRoute {
    let Ok(parsed) = url::Url::parse(url.trim()) else {
        return ExternalLinkRoute::Ignore;
    };
    if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none_or(str::is_empty) {
        return ExternalLinkRoute::Ignore;
    }
    let url = String::from(parsed);
    if little_windows {
        ExternalLinkRoute::LittleWindow(url)
    } else {
        ExternalLinkRoute::Tab(url)
    }
}

/// The colour Little Eclipse windows are framed in for a Space's `theme` and
/// `accent`: the sidebar's colour for that theme as it shows over the
/// browser window, tinted with the accent. sRGB, and whether it is dark.
pub(crate) fn little_window_frame_color(theme: &str, accent: &str) -> ([u8; 3], bool) {
    let (base, dark): ([u8; 3], bool) = match theme {
        "void" => ([9, 10, 14], true),
        "lunar" => ([17, 22, 33], true),
        "nebula" => ([20, 16, 30], true),
        "aurora" => ([14, 22, 23], true),
        "sol" => ([240, 241, 244], false),
        _ => ([15, 17, 23], true),
    };
    let tint = parse_hex_rgb(accent).unwrap_or([0x5E, 0x9E, 0xFF]);
    let amount = if dark { 0.14 } else { 0.10 };
    let mixed = std::array::from_fn(|i| {
        (f64::from(base[i]) * (1.0 - amount) + f64::from(tint[i]) * amount).round() as u8
    });
    (mixed, dark)
}

/// `#rgb`, `#rrggbb` or `#rrggbbaa` (the alpha is ignored).
fn parse_hex_rgb(color: &str) -> Option<[u8; 3]> {
    let hex = color.trim().strip_prefix('#')?;
    if !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let channel = |s: &str| u8::from_str_radix(s, 16).ok();
    match hex.len() {
        3 => {
            let mut out = [0u8; 3];
            for (i, c) in hex.chars().enumerate() {
                let v = channel(&c.to_string())?;
                out[i] = v * 17;
            }
            Some(out)
        }
        6 | 8 => Some([channel(&hex[0..2])?, channel(&hex[2..4])?, channel(&hex[4..6])?]),
        _ => None,
    }
}

pub fn get_process_memory_mb() -> f64 {
    #[cfg(target_os = "macos")]
    unsafe {
        let mut info = std::mem::MaybeUninit::<libc::mach_task_basic_info>::uninit();
        let mut count = (std::mem::size_of::<libc::mach_task_basic_info>()
            / std::mem::size_of::<libc::natural_t>())
            as libc::mach_msg_type_number_t;
        #[allow(deprecated)]
        let kret = libc::task_info(
            libc::mach_task_self(),
            libc::MACH_TASK_BASIC_INFO,
            info.as_mut_ptr() as libc::task_info_t,
            &mut count,
        );
        if kret == libc::KERN_SUCCESS {
            let info = info.assume_init();
            return (info.resident_size as f64) / (1024.0 * 1024.0);
        }
    }
    0.0
}

pub fn extract_chrome_extension_id(input: &str) -> Option<String> {
    let is_ext_id = |s: &str| s.len() == 32 && s.bytes().all(|b| b.is_ascii_lowercase());
    let cleaned = input.trim();
    if is_ext_id(cleaned) {
        return Some(cleaned.to_string());
    }
    for part in cleaned.split(|c: char| !c.is_ascii_alphanumeric()) {
        let p = part.trim();
        if is_ext_id(p) {
            return Some(p.to_string());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> BrowserConfig {
        BrowserConfig::default()
    }

    #[test]
    fn popups_that_open_no_window_fall_back_to_tabs_unless_blank() {
        use NewWindowFallback::*;
        // A blank popup the page got null for (a sized one, or
        // window.open('', name) without a gesture) leaves no empty tab.
        assert_eq!(new_window_fallback("", true), Blocked);
        assert_eq!(new_window_fallback("about:blank", true), Blocked);
        // Web popups (no gesture, over the limit) still open as tabs.
        assert_eq!(new_window_fallback("https://accounts.example/", true), Tab);
        // Links and noopener windows are unchanged.
        assert_eq!(new_window_fallback("", false), NewTabPage);
        assert_eq!(new_window_fallback("about:blank", false), NewTabPage);
        assert_eq!(new_window_fallback("http://a.example/", false), Tab);
        assert_eq!(new_window_fallback("mailto:a@b.example", false), External);
        assert_eq!(new_window_fallback("tel:123", true), External);
        assert_eq!(new_window_fallback("eclipse://localhost/settings", false), Ignore);
        assert_eq!(new_window_fallback("javascript:alert(1)", true), Ignore);
    }

    #[test]
    fn quitting_waits_for_pending_flushes_until_the_deadline() {
        let now = Instant::now();
        let deadline = now + Duration::from_secs(1);
        // Not quitting at all.
        assert!(!quit_ready(None, false, now));
        // A page or the sidebar is still flushing.
        assert!(!quit_ready(Some(deadline), true, now));
        assert!(quit_ready(Some(deadline), false, now));
        // The wait timed out.
        assert!(quit_ready(Some(deadline), true, deadline));
    }

    #[test]
    fn little_window_address_field_loads_web_pages_and_searches_only() {
        let config = config();
        let url = |input: &str| little_window_url_from_input(input, &config);
        assert_eq!(url("https://example.com/a?b=1").as_deref(), Some("https://example.com/a?b=1"));
        assert_eq!(url("  http://127.0.0.1:8831/x.html ").as_deref(), Some("http://127.0.0.1:8831/x.html"));
        assert_eq!(url("example.com").as_deref(), Some("https://example.com"));
        assert_eq!(url("localhost:3000").as_deref(), Some("http://localhost:3000"));
        assert!(url("rust borrow checker").unwrap().starts_with("https://duckduckgo.com/?q=rust+borrow+checker"));
        assert!(url("hello").unwrap().starts_with("https://duckduckgo.com/?q=hello"));
        // The configured engine is used.
        let google = BrowserConfig {
            search_engine: "google".into(),
            ..BrowserConfig::default()
        };
        assert!(
            little_window_url_from_input("hello", &google)
                .unwrap()
                .starts_with("https://www.google.com/search?q=hello")
        );
        // Internal pages are refused outright, never searched for.
        assert_eq!(url("eclipse://settings"), None);
        assert_eq!(url("ECLIPSE://localhost/newtab"), None);
        assert_eq!(url("eclipse:settings"), None);
        // Other schemes become searches, so only http(s) is ever loaded.
        for input in ["javascript:alert(1)", "file:///etc/passwd", "data:text/html,hi", "about:blank"] {
            let resolved = url(input).unwrap();
            assert!(resolved.starts_with("https://duckduckgo.com/?q="), "{input} -> {resolved}");
        }
        assert_eq!(url(""), None);
        assert_eq!(url("   "), None);
        assert_eq!(url(&"a".repeat(9000)), None);
    }

    #[test]
    fn the_little_window_bar_suggests_tabs_then_favourites() {
        let now = Instant::now();
        let earlier = now - Duration::from_secs(60);
        let tab = |title, url, current_space, last_active| CommandBarTab {
            title,
            url,
            current_space,
            last_active,
        };
        let tabs = [
            tab("Old", "https://old.example/", true, earlier),
            tab("Elsewhere", "https://other-space.example/", false, now),
            tab("New Tab", "eclipse://newtab", true, now),
            tab("Recent\n\tpage", "https://recent.example/a?b=1", true, now),
            tab("", "http://untitled.example/", true, earlier),
            tab("Dup", "https://old.example/", false, now),
            tab("File", "file:///etc/passwd", true, now),
        ];
        let favorite = |title: &str, url: &str| FavoriteShortcut {
            title: title.into(),
            url: url.into(),
            icon: String::new(),
        };
        let favorites = [
            favorite("Fav", "https://fav.example/"),
            favorite("Old again", "https://old.example/"),
            favorite("Script", "javascript:alert(1)"),
        ];
        let suggestions = little_command_bar_suggestions(&tabs, &favorites);
        let urls: Vec<&str> = suggestions.iter().map(|(_, url)| url.as_str()).collect();
        assert_eq!(
            urls,
            [
                "https://recent.example/a?b=1",
                "https://old.example/",
                "http://untitled.example/",
                "https://fav.example/",
                "https://other-space.example/",
            ]
        );
        assert_eq!(suggestions[0].0, "Recent page");
        assert_eq!(suggestions[2].0, "untitled.example");
        assert!(little_command_bar_suggestions(&[], &[]).is_empty());
    }

    #[test]
    fn links_from_other_apps_open_little_windows_or_tabs() {
        assert_eq!(
            external_link_route("https://example.com/a", true),
            ExternalLinkRoute::LittleWindow("https://example.com/a".into())
        );
        assert_eq!(
            external_link_route("http://127.0.0.1:8000/x.html", false),
            ExternalLinkRoute::Tab("http://127.0.0.1:8000/x.html".into())
        );
        for url in [
            "eclipse://settings",
            "file:///Users/me/a.html",
            "javascript:alert(1)",
            "mailto:a@b.example",
            "data:text/html,hi",
            "about:blank",
            "not a url",
            "",
        ] {
            assert_eq!(external_link_route(url, true), ExternalLinkRoute::Ignore, "{url}");
            assert_eq!(external_link_route(url, false), ExternalLinkRoute::Ignore, "{url}");
        }
    }

    #[test]
    fn little_windows_are_framed_in_the_space_theme() {
        let (eclipse, dark) = little_window_frame_color("eclipse", "#5E9EFF");
        assert!(dark);
        assert!(eclipse.iter().all(|&c| c < 64), "{eclipse:?}");
        // The accent tints the frame.
        assert_ne!(little_window_frame_color("eclipse", "#FF0000").0, eclipse);
        assert!(little_window_frame_color("eclipse", "#FF0000").0[0] > eclipse[0]);
        let (sol, sol_dark) = little_window_frame_color("sol", "#5E9EFF");
        assert!(!sol_dark);
        assert!(sol.iter().all(|&c| c > 180), "{sol:?}");
        // Unknown themes and bad accents fall back to the defaults.
        assert_eq!(little_window_frame_color("nope", "#5E9EFF"), (eclipse, true));
        assert_eq!(little_window_frame_color("eclipse", "red"), (eclipse, true));
        assert_eq!(parse_hex_rgb("#abc"), Some([0xAA, 0xBB, 0xCC]));
        assert_eq!(parse_hex_rgb("#5E9EFF80"), Some([0x5E, 0x9E, 0xFF]));
        assert_eq!(parse_hex_rgb("#12"), None);
        assert_eq!(parse_hex_rgb("#gggggg"), None);
    }

    #[test]
    fn normalizes_common_inputs() {
        assert_eq!(
            normalize_url("example.com", &config()),
            "https://example.com"
        );
        assert_eq!(
            normalize_url("https://example.com/a", &config()),
            "https://example.com/a"
        );
        assert_eq!(
            normalize_url("localhost:3000", &config()),
            "http://localhost:3000"
        );
        assert_eq!(
            normalize_url("eclipse://localhost/newtab", &config()),
            "eclipse://newtab"
        );
        assert_eq!(
            normalize_url("eclipse://notes", &config()),
            "eclipse://notes"
        );
        assert_eq!(
            normalize_url("eclipse://easel", &config()),
            "eclipse://easel"
        );
        assert!(normalize_url("hello world", &config()).starts_with("https://duckduckgo.com/?q="));
        assert!(
            normalize_url("javascript:alert(1)", &config()).starts_with("https://duckduckgo.com")
        );
        assert!(
            normalize_url("file:///etc/passwd", &config()).starts_with("https://duckduckgo.com")
        );
    }

    #[test]
    fn test_tidy_tabs_categorization() {
        assert_eq!(
            categorize_tab_group("https://github.com/rust-lang/rust", "Rust Repo"),
            "Development"
        );
        assert_eq!(
            categorize_tab_group("https://www.youtube.com/watch?v=123", "Video"),
            "Media & Stream"
        );
        assert_eq!(
            categorize_tab_group("https://mail.google.com/mail/u/0", "Inbox"),
            "Work & Productivity"
        );
        assert_eq!(
            categorize_tab_group("https://discord.com/channels/@me", "Discord"),
            "Social & Community"
        );
        assert_eq!(
            categorize_tab_group("https://duckduckgo.com/?q=test", "Search"),
            "Research & Search"
        );
    }

    #[test]
    fn test_content_bounds_collapsed_and_expanded() {
        let mut cfg = BrowserConfig {
            sidebar_collapsed: true,
            sidebar_width: 240.0,
            ..Default::default()
        };

        let (content_x, content_w) = if cfg.sidebar_collapsed {
            (ARC_BORDER, 1200.0 - (ARC_BORDER * 2.0))
        } else {
            let sidebar_w = cfg.sidebar_width.clamp(200.0, 260.0);
            (
                sidebar_w + ARC_BORDER,
                1200.0 - sidebar_w - (ARC_BORDER * 2.0),
            )
        };
        assert_eq!(content_x, 8.0);
        assert_eq!(content_w, 1184.0);

        cfg.sidebar_collapsed = false;
        let (content_x, content_w) = if cfg.sidebar_collapsed {
            (ARC_BORDER, 1200.0 - (ARC_BORDER * 2.0))
        } else {
            let sidebar_w = cfg.sidebar_width.clamp(200.0, 260.0);
            (
                sidebar_w + ARC_BORDER,
                1200.0 - sidebar_w - (ARC_BORDER * 2.0),
            )
        };
        assert_eq!(content_x, 248.0);
        assert_eq!(content_w, 944.0);
    }

    #[test]
    fn test_extract_chrome_extension_id() {
        assert_eq!(
            extract_chrome_extension_id("cjpalhdlnbpafiamejdnhcphjbkeiagm"),
            Some("cjpalhdlnbpafiamejdnhcphjbkeiagm".to_string())
        );
        assert_eq!(
            extract_chrome_extension_id(
                "https://chromewebstore.google.com/detail/ublock-origin/cjpalhdlnbpafiamejdnhcphjbkeiagm"
            ),
            Some("cjpalhdlnbpafiamejdnhcphjbkeiagm".to_string())
        );
        assert_eq!(
            extract_chrome_extension_id(
                "https://chrome.google.com/webstore/detail/sponsorblock-for-youtube/mnjggcdmjocbbbhaepdhchncahnbgone?hl=en"
            ),
            Some("mnjggcdmjocbbbhaepdhchncahnbgone".to_string())
        );
        assert_eq!(extract_chrome_extension_id("invalid-id"), None);
    }

    const TOKEN: &str = "0123456789abcdef0123456789abcdef";

    fn command(json: &str) -> IpcCommand {
        serde_json::from_str(json).unwrap()
    }

    const INTERNAL_DOC: DocumentTrust = DocumentTrust {
        internal: true,
        left_internal: false,
    };
    const WEB_DOC: DocumentTrust = DocumentTrust {
        internal: false,
        left_internal: false,
    };
    const LEFT_INTERNAL_DOC: DocumentTrust = DocumentTrust {
        internal: false,
        left_internal: true,
    };

    #[test]
    fn classifies_ipc_senders_by_frame_origin_and_committed_document() {
        assert_eq!(
            classify_tab_sender("eclipse://localhost/settings", INTERNAL_DOC),
            IpcSender::Internal
        );
        // The internal page stays trusted while a load that never commits
        // (a download, a refused navigation) is pending: the classification
        // follows the committed document, not the tab's target URL.
        assert_eq!(
            classify_tab_sender("eclipse://localhost/notes", INTERNAL_DOC),
            IpcSender::Internal
        );
        // A website, even one that iframes or links to eclipse:// pages.
        assert_eq!(classify_tab_sender("https://evil.example/", WEB_DOC), IpcSender::Web);
        assert_eq!(classify_tab_sender("eclipse://localhost/settings", WEB_DOC), IpcSender::Web);
        // A web document still current while an eclipse:// load is pending.
        assert_eq!(classify_tab_sender("https://evil.example/", INTERNAL_DOC), IpcSender::Web);
        assert_eq!(classify_tab_sender("https://evil.example/", LEFT_INTERNAL_DOC), IpcSender::Web);
        // The internal page the tab just left, saving as it unloads.
        assert_eq!(
            classify_tab_sender("eclipse://localhost/settings", LEFT_INTERNAL_DOC),
            IpcSender::DepartingInternal
        );
        // The error page (load_html) and unparseable origins.
        assert_eq!(classify_tab_sender("about:blank", WEB_DOC), IpcSender::Web);
        assert_eq!(classify_tab_sender("", INTERNAL_DOC), IpcSender::Web);
    }

    #[test]
    fn departing_internal_pages_may_only_save() {
        for json in [
            r#"{"action":"update_settings","settings":{"custom_css":"a{}"}}"#,
            r#"{"action":"save_note","id":"n1","title":"t","content":"c"}"#,
            r#"{"action":"save_easel","id":"e1","title":"t","elements_json":"[]"}"#,
        ] {
            assert!(is_command_allowed(IpcSender::DepartingInternal, &command(json)), "{json}");
        }
        for json in [
            r#"{"action":"delete_note","id":"n1"}"#,
            r#"{"action":"navigate","url":"https://a.example"}"#,
            r#"{"action":"install_extension","path":"/tmp/x"}"#,
            r#"{"action":"close_tab","id":3}"#,
            r#"{"action":"window_close"}"#,
        ] {
            assert!(!is_command_allowed(IpcSender::DepartingInternal, &command(json)), "{json}");
        }
        assert_eq!(IpcSender::DepartingInternal.message_limit(), TRUSTED_IPC_LIMIT);
    }

    #[test]
    fn url_reports_must_come_from_the_committed_web_document() {
        let page = "https://app.example/feed";
        assert!(is_plausible_url_report("https://app.example/feed?p=2", page, page));
        // A late report from the page the tab already left.
        assert!(!is_plausible_url_report(
            "https://old.example/x",
            "https://old.example/",
            "https://new.example/"
        ));
        // ... or arriving after an internal page committed.
        assert!(!is_plausible_url_report(
            "https://old.example/x",
            "https://old.example/",
            "eclipse://localhost/settings"
        ));
        // A cross-origin URL, an error page and non-web URLs.
        assert!(!is_plausible_url_report("https://evil.example/", page, page));
        assert!(!is_plausible_url_report(
            "https://down.example/",
            "about:blank",
            "https://down.example/"
        ));
        let settings = "eclipse://localhost/settings";
        assert!(!is_plausible_url_report("eclipse://settings", settings, settings));
        assert!(!is_plausible_url_report("https://app.example/", page, ""));
    }

    #[test]
    fn page_reports_must_come_from_the_committed_document() {
        let page = "https://shop.example/checkout";
        let popup = "https://accounts.example/oauth";
        let reports = [
            r#"{"action":"update_favicon","favicon":"https://accounts.example/f.ico"}"#,
            r#"{"action":"set_audible","audible":true}"#,
            r#"{"action":"update_scroll","x":0,"y":10}"#,
        ];
        for json in reports {
            let report = command(json);
            assert!(is_plausible_page_report(&report, "https://shop.example/other", page), "{json}");
            // A frame of another origin (a page the tab already left).
            assert!(!is_plausible_page_report(&report, popup, page), "{json}");
            // A blank popup the opener wrote into, and a late message after
            // the tab moved on to an internal page or has nothing committed.
            assert!(!is_plausible_page_report(&report, "about:blank", page), "{json}");
            assert!(!is_plausible_page_report(&report, page, "eclipse://localhost/newtab"), "{json}");
            assert!(!is_plausible_page_report(&report, page, ""), "{json}");
            // Same host, different scheme or port.
            assert!(!is_plausible_page_report(&report, "http://shop.example/", page), "{json}");
            assert!(!is_plausible_page_report(&report, "https://shop.example:8443/", page), "{json}");
        }
        // url_changed keeps its stricter check.
        let url_report = command(r#"{"action":"url_changed","url":"https://accounts.example/x"}"#);
        assert!(!is_plausible_page_report(&url_report, popup, page));
        let url_report = command(r#"{"action":"url_changed","url":"https://shop.example/done"}"#);
        assert!(is_plausible_page_report(&url_report, page, page));
        // A popup may still open http(s) links in new tabs.
        let new_tab = command(r#"{"action":"new_tab","url":"https://help.example/"}"#);
        assert!(is_plausible_page_report(&new_tab, popup, page));
        assert!(is_command_allowed(classify_tab_sender(popup, DocumentTrust::default()), &new_tab));
    }

    #[test]
    fn little_window_messages_never_count_as_the_tabs_page() {
        const WEB_DOC: DocumentTrust = DocumentTrust {
            internal: false,
            left_internal: false,
        };
        const INTERNAL_DOC: DocumentTrust = DocumentTrust {
            internal: true,
            left_internal: false,
        };
        let page = "http://127.0.0.1:8811/opener.html";
        // An OAuth popup that ends on the opener's own origin passes every
        // origin check, so only WebKit's marking tells it apart.
        let callback = "http://127.0.0.1:8811/popup-callback?state=done";
        let reports = [
            r#"{"action":"url_changed","url":"http://127.0.0.1:8811/popup-callback?state=done"}"#,
            r#"{"action":"update_favicon","favicon":"http://127.0.0.1:8811/f.ico"}"#,
            r#"{"action":"set_audible","audible":true}"#,
            r#"{"action":"update_scroll","x":0,"y":10}"#,
        ];
        let sender = tab_ipc_sender(callback, WEB_DOC, false, true);
        assert_eq!(sender, IpcSender::LittleWindow);
        for json in reports {
            let report = command(json);
            assert!(is_plausible_page_report(&report, callback, page), "{json}");
            assert!(is_command_allowed(IpcSender::Web, &report), "{json}");
            assert!(!is_command_allowed(sender, &report), "{json}");
        }
        // It may still open http(s) links in new tabs, nothing else.
        let new_tab = command(r#"{"action":"new_tab","url":"https://help.example/"}"#);
        assert!(is_command_allowed(sender, &new_tab));
        let eclipse_tab = command(r#"{"action":"new_tab","url":"eclipse://settings"}"#);
        assert!(!is_command_allowed(sender, &eclipse_tab));
        let settings = command(r#"{"action":"update_settings","settings":{}}"#);
        assert!(!is_command_allowed(sender, &settings));
        assert_eq!(sender.message_limit(), WEB_IPC_LIMIT);
        // Never promoted, whatever origin or document it claims.
        assert_eq!(
            tab_ipc_sender("eclipse://localhost/settings", INTERNAL_DOC, false, true),
            IpcSender::LittleWindow
        );
        assert_eq!(
            tab_ipc_sender("eclipse://localhost/settings", INTERNAL_DOC, true, true),
            IpcSender::LittleWindow
        );
        // The tab's own messages are classified as before.
        assert_eq!(tab_ipc_sender(page, WEB_DOC, false, false), IpcSender::Web);
        assert_eq!(
            tab_ipc_sender("eclipse://localhost/settings", INTERNAL_DOC, false, false),
            IpcSender::Internal
        );
        assert_eq!(
            tab_ipc_sender("eclipse://localhost/settings", INTERNAL_DOC, true, false),
            IpcSender::DepartingInternal
        );
    }

    #[test]
    fn current_site_forgets_closed_pages() {
        let now = Instant::now();
        let earlier = now - Duration::from_secs(60);
        let open = [
            ("https://old.example/", earlier),
            ("eclipse://settings", now),
            ("https://recent.example/", now),
        ];
        // Still open: unchanged.
        assert_eq!(surviving_web_url("https://old.example/", &open), "https://old.example/");
        // Closed: the most recently active open web page.
        assert_eq!(surviving_web_url("https://gone.example/", &open), "https://recent.example/");
        // No web page left.
        assert_eq!(surviving_web_url("https://gone.example/", &[("eclipse://settings", now)]), "");
        assert_eq!(surviving_web_url("", &open), "");
    }

    fn restore_key(index: usize, workspace: &str, in_space_list: bool) -> RestoreKey<'_> {
        RestoreKey {
            index,
            workspace,
            in_space_list,
        }
    }

    #[test]
    fn session_restore_picks_the_active_tab_by_position() {
        // Two new-tab pages in two spaces; the second (Media) was active.
        let keys = [restore_key(0, "Default", true), restore_key(1, "Media", true)];
        assert_eq!(restored_active_position(&keys, Some(1), "Media"), Some(1));
        // Positions count the file as written, before unrestorable entries
        // are skipped.
        let keys = [restore_key(0, "Default", true), restore_key(2, "Media", true)];
        assert_eq!(restored_active_position(&keys, Some(2), "Media"), Some(1));
        // A saved index pointing into another space falls back to the saved
        // space's first tab; a pinned tab shows in every space.
        assert_eq!(restored_active_position(&keys, Some(0), "Media"), Some(1));
        let keys = [restore_key(0, "Default", false), restore_key(1, "Media", true)];
        assert_eq!(restored_active_position(&keys, Some(0), "Media"), Some(0));
        // No (valid) index: the saved space's first tab, else the first tab.
        assert_eq!(restored_active_position(&keys, None, "Media"), Some(1));
        assert_eq!(restored_active_position(&keys, Some(9), "Work"), Some(0));
        assert_eq!(restored_active_position(&[], Some(0), "Media"), None);
    }

    #[test]
    fn web_pages_may_only_report_page_state_and_open_http_links() {
        let allowed = [
            r#"{"action":"url_changed","url":"https://a.example/x"}"#,
            r#"{"action":"update_favicon","favicon":"https://a.example/f.ico"}"#,
            r#"{"action":"set_audible","audible":true}"#,
            r#"{"action":"update_scroll","x":0,"y":10}"#,
            r#"{"action":"new_tab","url":"https://a.example/","background":true}"#,
        ];
        for json in allowed {
            assert!(is_command_allowed(IpcSender::Web, &command(json)), "{json}");
        }
        let denied = [
            r#"{"action":"new_tab","url":"eclipse://settings"}"#,
            r#"{"action":"new_tab","url":"file:///etc/passwd"}"#,
            r#"{"action":"new_tab","url":null}"#,
            r#"{"action":"update_settings","settings":{"custom_js":"alert(1)"}}"#,
            r#"{"action":"install_extension","path":"/tmp/x"}"#,
            r#"{"action":"delete_note","id":"n1"}"#,
            r#"{"action":"close_tab","id":3}"#,
            r#"{"action":"select_tab","id":3}"#,
            r#"{"action":"navigate","url":"https://a.example"}"#,
            r#"{"action":"chrome_error","message":"x"}"#,
            r#"{"action":"window_close"}"#,
            r#"{"action":"open_download","id":1}"#,
        ];
        for json in denied {
            assert!(!is_command_allowed(IpcSender::Web, &command(json)), "{json}");
        }
    }

    #[test]
    fn internal_pages_may_not_drive_the_window() {
        assert!(is_command_allowed(
            IpcSender::Internal,
            &command(r#"{"action":"save_note","title":"t","content":"c"}"#)
        ));
        assert!(is_command_allowed(
            IpcSender::Internal,
            &command(r#"{"action":"update_settings","settings":{}}"#)
        ));
        for json in [
            r#"{"action":"window_close"}"#,
            r#"{"action":"window_minimize"}"#,
            r#"{"action":"window_zoom"}"#,
            r#"{"action":"drag_window"}"#,
        ] {
            assert!(!is_command_allowed(IpcSender::Internal, &command(json)), "{json}");
            assert!(is_command_allowed(IpcSender::Sidebar, &command(json)), "{json}");
        }
    }

    #[test]
    fn ipc_messages_need_a_token_and_respect_the_size_cap() {
        let note = format!(
            r#"{{"token":"{TOKEN}","action":"save_note","title":"t","content":"{}"}}"#,
            "x".repeat(100 * 1024)
        );
        assert!(parse_ipc(&note, IpcSender::Web.message_limit()).is_none());
        let (token, parsed) = parse_ipc(&note, IpcSender::Internal.message_limit()).unwrap();
        assert_eq!(token, TOKEN);
        assert!(matches!(parsed, IpcCommand::SaveNote { .. }));

        assert!(parse_ipc(r#"{"action":"reload"}"#, TRUSTED_IPC_LIMIT).is_none());
        assert!(parse_ipc(r#"{"token":"short","action":"reload"}"#, TRUSTED_IPC_LIMIT).is_none());
        // The whole-config command no longer exists.
        let update_config = format!(r#"{{"token":"{TOKEN}","action":"update_config","config":{{}}}}"#);
        assert!(parse_ipc(&update_config, TRUSTED_IPC_LIMIT).is_none());
    }

    #[test]
    fn command_names_never_include_payloads() {
        let cmd = command(r#"{"action":"save_note","title":"secret","content":"secret"}"#);
        assert_eq!(command_name(&cmd), "SaveNote");
        assert_eq!(command_name(&command(r#"{"action":"reload"}"#)), "Reload");
    }

    fn download(id: usize, name: &str, url: &str, status: &str) -> DownloadItem {
        DownloadItem {
            id,
            file_name: name.into(),
            url: url.into(),
            path: format!("/Downloads/{name}"),
            status: status.into(),
            timestamp: 0,
        }
    }

    #[test]
    fn download_completion_matches_by_url_then_file_name() {
        // Newest first, like config.downloads.
        let downloads = vec![
            download(4, "report (1).pdf", "https://host/dl?id=5", "downloading"),
            download(3, "repo-main.zip", "https://github.com/o/repo/archive/main.zip", "downloading"),
            download(2, "report.pdf", "https://host/dl?id=5", "completed"),
            download(1, "notes.txt", "https://host/notes.txt", "downloading"),
        ];
        // Content-Disposition name differs from the URL-derived "main.zip".
        assert_eq!(
            find_download_for_completion(&downloads, "https://github.com/o/repo/archive/main.zip", "main.zip"),
            Some(1)
        );
        // Same URL downloaded twice: the in-flight one wins.
        assert_eq!(find_download_for_completion(&downloads, "https://host/dl?id=5", "dl"), Some(0));
        // Unknown URL falls back to the file name.
        assert_eq!(find_download_for_completion(&downloads, "blob:https://x/1", "notes.txt"), Some(3));
        assert_eq!(find_download_for_completion(&downloads, "https://other", "other.bin"), None);
    }

    fn key(id: usize, favorite_index: Option<usize>, is_pinned: bool, workspace: &str) -> TabOrderKey<'_> {
        TabOrderKey {
            id,
            favorite_index,
            is_pinned,
            workspace,
        }
    }

    #[test]
    fn display_order_matches_the_sidebar() {
        let tabs = [
            key(1, None, false, "Default"),
            key(2, Some(1), true, "Default"),
            key(3, None, true, "Media"),
            key(4, None, false, "Media"),
            key(5, Some(0), true, "Dev"),
            key(6, None, false, "Default"),
        ];
        assert_eq!(display_order(&tabs, "Default"), vec![5, 2, 3, 1, 6]);
        assert_eq!(display_order(&tabs, "Media"), vec![5, 2, 3, 4]);
    }

    #[test]
    fn favorite_bindings_follow_a_move() {
        // [A, B, C, D]: move A (0) to 2 -> [B, C, A, D]
        assert_eq!(index_after_move(0, 0, 2), 2);
        assert_eq!(index_after_move(1, 0, 2), 0);
        assert_eq!(index_after_move(2, 0, 2), 1);
        assert_eq!(index_after_move(3, 0, 2), 3);
        // Move D (3) to 1 -> [A, D, B, C]
        assert_eq!(index_after_move(3, 3, 1), 1);
        assert_eq!(index_after_move(1, 3, 1), 2);
        assert_eq!(index_after_move(2, 3, 1), 3);
        assert_eq!(index_after_move(0, 3, 1), 0);
    }

    #[test]
    fn tidy_groups_match_domains_not_substrings() {
        assert_eq!(categorize_tab_group("https://www.dropbox.com/home", ""), "Dropbox");
        assert_eq!(categorize_tab_group("https://www.xbox.com/", ""), "Xbox");
        assert_eq!(categorize_tab_group("https://x.com/home", ""), "Social & Community");
        assert_eq!(categorize_tab_group("https://site.example/?next=localhost", ""), "Site");
        assert_eq!(categorize_tab_group("http://localhost:3000/", ""), "Development");
        assert_eq!(
            categorize_tab_group("https://www.google.com/search?q=rust", ""),
            "Research & Search"
        );
        assert_eq!(categorize_tab_group("https://outlook.live.com/mail", ""), "Work & Productivity");
    }

    #[test]
    fn restorable_urls_exclude_transient_pages() {
        assert!(is_restorable_url("https://a.example/"));
        assert!(is_restorable_url("eclipse://notes"));
        assert!(!is_restorable_url("about:blank"));
        assert!(!is_restorable_url("eclipse://sidebar"));
        assert!(!is_restorable_url("eclipse://error"));
        assert!(!is_restorable_url("data:text/html,hi"));
        assert!(!is_restorable_url("file:///etc/hosts"));
    }

    #[test]
    fn sidebar_document_never_navigates_to_the_web() {
        assert!(is_sidebar_document_url("about:blank"));
        assert!(is_sidebar_document_url("eclipse://localhost/icon"));
        assert!(!is_sidebar_document_url("https://evil.example/"));
        assert!(!is_sidebar_document_url("file:///Users/me/a.html"));
    }

    #[test]
    fn log_messages_are_truncated_and_stripped_of_control_characters() {
        assert_eq!(truncate_message("a\u{1b}[31mb\nc", 10), "a[31mbc");
        assert_eq!(truncate_message("abcdef", 3), "abc…");
    }

    #[test]
    fn finds_the_zip_inside_crx_containers() {
        let zip = b"PK\x03\x04rest";
        let mut crx3 = b"Cr24".to_vec();
        crx3.extend_from_slice(&3u32.to_le_bytes());
        crx3.extend_from_slice(&5u32.to_le_bytes());
        crx3.extend_from_slice(b"PK\x03\x04"); // a decoy inside the header
        crx3.push(0);
        crx3.extend_from_slice(zip);
        assert_eq!(crx_zip_payload(&crx3).unwrap(), zip);

        let mut crx2 = b"Cr24".to_vec();
        crx2.extend_from_slice(&2u32.to_le_bytes());
        crx2.extend_from_slice(&2u32.to_le_bytes());
        crx2.extend_from_slice(&1u32.to_le_bytes());
        crx2.extend_from_slice(b"kks");
        crx2.extend_from_slice(zip);
        assert_eq!(crx_zip_payload(&crx2).unwrap(), zip);

        assert_eq!(crx_zip_payload(zip).unwrap(), zip);
        assert!(crx_zip_payload(b"<html>not found</html>").is_err());
        let mut truncated = b"Cr24".to_vec();
        truncated.extend_from_slice(&3u32.to_le_bytes());
        truncated.extend_from_slice(&u32::MAX.to_le_bytes());
        assert!(crx_zip_payload(&truncated).is_err());
    }

    #[test]
    fn extension_files_must_stay_inside_the_extension() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("target/test-tmp/ext-containment");
        let _ = std::fs::remove_dir_all(&root);
        let ext = root.join("ext");
        std::fs::create_dir_all(ext.join("_locales/en")).unwrap();
        std::fs::write(root.join("secret.txt"), "secret").unwrap();
        std::fs::write(ext.join("content.js"), "ok()").unwrap();
        std::fs::write(
            ext.join("_locales/en/messages.json"),
            r#"{"extName":{"message":"Real Name"}}"#,
        )
        .unwrap();
        std::os::unix::fs::symlink(root.join("secret.txt"), ext.join("link.css")).unwrap();
        let base = std::fs::canonicalize(&ext).unwrap();

        assert_eq!(read_extension_file(&base, "content.js").as_deref(), Some("ok()"));
        assert_eq!(read_extension_file(&base, "/content.js").as_deref(), Some("ok()"));
        assert_eq!(read_extension_file(&base, "../secret.txt"), None);
        assert_eq!(read_extension_file(&base, "link.css"), None);
        let absolute = root.join("secret.txt");
        assert_eq!(read_extension_file(&base, &absolute.to_string_lossy()), None);

        let manifest = serde_json::json!({"name": "__MSG_EXTNAME__", "default_locale": "en"});
        assert_eq!(manifest_text(&base, &manifest, "name").as_deref(), Some("Real Name"));
        let _ = std::fs::remove_dir_all(&root);
    }
}
