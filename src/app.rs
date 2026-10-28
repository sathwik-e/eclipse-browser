use std::collections::HashMap;
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
use crate::events::{BrowserUiState, IpcCommand, UserEvent};
use crate::tab::{
    Tab, canonicalize_eclipse_url, downloads_dir, generate_ipc_token, get_default_favicon,
    get_favicon_url,
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
    /// An eclipse:// page in a tab whose current URL is eclipse://.
    Internal,
    /// Any other page.
    Web,
}

impl IpcSender {
    fn message_limit(self) -> usize {
        match self {
            IpcSender::Web => WEB_IPC_LIMIT,
            IpcSender::Sidebar | IpcSender::Internal => TRUSTED_IPC_LIMIT,
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
        let sidebar_webview = WebViewBuilder::new()
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
            })
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
        self.broadcast_state();
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

        for tab in &mut self.tabs {
            if tab.id == id {
                tab.restore_if_needed(&window, proxy.clone(), bounds, &self.config);
                // Background tabs are not re-laid out on resize; catch up here.
                tab.animate_bounds(bounds, 0.0);
                tab.set_visible(true);
                tab.last_active = Instant::now();
                if is_http_url(&tab.url) {
                    self.last_web_url = tab.url.clone();
                }
                if !tab.is_pinned {
                    target_workspace = Some(tab.workspace.clone());
                }
            } else {
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
        drop(closed);

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
            }
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
    /// WebKit, which together with the tab's current URL decides whether the
    /// sender is an internal page or an ordinary website.
    pub fn handle_tab_ipc(&mut self, tab_id: usize, raw: &str, origin: &str) {
        let Some(tab) = self.tabs.iter().find(|tab| tab.id == tab_id) else {
            return;
        };
        let sender = classify_tab_sender(origin, &tab.url);
        let Some((token, command)) = parse_ipc(raw, sender.message_limit()) else {
            return;
        };
        if token != tab.token {
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
            IpcCommand::StopLoading => {
                if let Some(tab) = self
                    .tabs
                    .iter_mut()
                    .find(|tab| tab.id == self.active_tab_id)
                {
                    tab.stop_loading();
                }
                self.broadcast_state();
            }
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
                    None => self.sidebar_pushed_revision = None,
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

    /// window.open / target=_blank from a page. Pages may not open internal
    /// eclipse:// pages.
    pub fn handle_new_window(&mut self, url: String) {
        if url.is_empty() || url == "about:blank" {
            self.create_tab("eclipse://newtab", true);
            return;
        }
        if is_http_url(&url) {
            self.create_tab(&url, true);
        } else if url.starts_with("mailto:") || url.starts_with("tel:") {
            let _ = std::process::Command::new("open").arg(&url).spawn();
        }
    }

    pub fn handle_download_started(&mut self, file_name: String, url: String, path: String) {
        let id = self.next_download_id;
        self.next_download_id += 1;
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

    /// Completion events carry the original request URL (the name WebKit
    /// finally used may differ from what the URL suggests), so match by URL
    /// first and only fall back to the file name.
    pub fn handle_download_completed(
        &mut self,
        url: String,
        file_name: String,
        path: Option<String>,
        success: bool,
    ) {
        let status = if success { "completed" } else { "failed" };
        let display_name = match find_download_for_completion(&self.config.downloads, &url, &file_name) {
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

    /// A navigation failed (DNS, offline, TLS, …): stop the spinner and show
    /// an error page with a retry button instead of a blank "Loading…" tab.
    pub fn handle_navigation_failed(
        &mut self,
        tab_id: usize,
        code: i64,
        description: String,
        url: String,
    ) {
        let Some(tab) = self.tabs.iter_mut().find(|tab| tab.id == tab_id) else {
            return;
        };
        if self.perf {
            eprintln!("[nav] tab {tab_id} failed ({code}) {url}: {description}");
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
            "command_bar" => {
                self.open_command_bar();
            }
            // Little Arc (an in-page preview) is gone; its menu item opens a
            // fresh tab.
            "little_arc" => {
                self.create_tab("eclipse://newtab", true);
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
            "focus_omnibar" => self.focus_omnibar(),
            "open_settings" => self.open_settings(),
            "open_settings_tab" => self.open_settings_tab(),
            "reload" => {
                if let Some(tab) = self.tabs.iter().find(|tab| tab.id == self.active_tab_id) {
                    tab.reload();
                }
            }
            "stop_loading" => {
                if let Some(tab) = self
                    .tabs
                    .iter_mut()
                    .find(|tab| tab.id == self.active_tab_id)
                {
                    tab.stop_loading();
                }
                self.broadcast_state();
            }
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

    /// Hibernates a background tab if it can safely lose its page state.
    fn can_hibernate(tab: &Tab, active_id: usize) -> bool {
        tab.id != active_id
            && !tab.is_hibernated
            && tab.webview.is_some()
            && !tab.is_audible
            && !tab.is_loading
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

        [
            self.state_dirty.then_some(self.next_state_flush),
            self.config_save_due,
            self.session_save_due,
            self.hover_hold_until,
            self.sidebar_hide_due.map(|(due, _)| due),
        ]
        .into_iter()
        .flatten()
        .min()
    }

    /// Writes everything that is pending; called when the app quits.
    pub fn shutdown(&mut self) {
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
        let active_url = session
            .active_index
            .and_then(|index| session.tabs.get(index))
            .map(|tab| tab.url.clone());
        let mut active_id = None;
        for record in session
            .tabs
            .into_iter()
            .filter(|record| is_restorable_url(&record.url))
            .take(MAX_SESSION_TABS)
        {
            let id = self.next_tab_id;
            self.next_tab_id += 1;
            let favorite_index = record
                .favorite_url
                .as_ref()
                .and_then(|url| self.config.favorites.iter().position(|f| &f.url == url));
            if active_id.is_none() && active_url.as_deref() == Some(record.url.as_str()) {
                active_id = Some(id);
            }
            self.tabs.push(hibernated_tab(id, record, favorite_index));
        }
        let Some(first) = self.tabs.first().map(|tab| tab.id) else {
            return false;
        };
        let workspace = session.current_workspace;
        self.current_workspace = workspace.clone();
        let profile = self.config.profile_for_workspace(&workspace);
        self.config.theme = profile.theme;
        self.config.accent_color = profile.accent_color;
        self.select_tab(active_id.unwrap_or(first));
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
        let zoom = tab.set_zoom(next);
        self.show_toast(&format!("Zoom {:.0}%", zoom * 100.0));
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

/// Internal = an eclipse:// frame in a tab that is currently on an eclipse://
/// page. Everything else (including the error page, a subframe of a website,
/// or a web document still current while an eclipse:// load is pending) is a
/// website.
pub fn classify_tab_sender(origin: &str, tab_url: &str) -> IpcSender {
    let origin_is_eclipse = url::Url::parse(origin).is_ok_and(|u| u.scheme() == "eclipse");
    if origin_is_eclipse && tab_url.starts_with("eclipse://") {
        IpcSender::Internal
    } else {
        IpcSender::Web
    }
}

/// The per-sender command allowlist (C1). Websites may only report their own
/// page state and open http(s) links in new tabs.
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
        IpcSender::Web => match command {
            IpcCommand::UrlChanged { .. }
            | IpcCommand::UpdateFavicon { .. }
            | IpcCommand::SetAudible { .. }
            | IpcCommand::UpdateScroll { .. } => true,
            IpcCommand::NewTab { url: Some(url), .. } => is_http_url(url.trim()),
            _ => false,
        },
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
        "Loading…".into()
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

    #[test]
    fn classifies_ipc_senders_by_frame_origin_and_tab_url() {
        assert_eq!(
            classify_tab_sender("eclipse://localhost/settings", "eclipse://settings"),
            IpcSender::Internal
        );
        // A website, even one that iframes or links to eclipse:// pages.
        assert_eq!(classify_tab_sender("https://evil.example/", "https://evil.example/"), IpcSender::Web);
        assert_eq!(classify_tab_sender("eclipse://localhost/settings", "https://evil.example/"), IpcSender::Web);
        // A web document still current while an eclipse:// load is pending.
        assert_eq!(classify_tab_sender("https://evil.example/", "eclipse://notes"), IpcSender::Web);
        // The error page (load_html) and unparseable origins.
        assert_eq!(classify_tab_sender("about:blank", "https://down.example/"), IpcSender::Web);
        assert_eq!(classify_tab_sender("", "eclipse://newtab"), IpcSender::Web);
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
