use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tao::dpi::{LogicalPosition, LogicalSize, Position, Size};
use tao::event_loop::EventLoopProxy;
use tao::window::Window;
use wry::{Rect, WebView, WebViewBuilder};

use crate::config::{
    BrowserConfig, DownloadItem, EaselItem, FavoriteShortcut, NoteItem,
};
use crate::events::{BrowserUiState, IpcCommand, UserEvent};
use crate::tab::{
    Tab, canonicalize_eclipse_url, downloads_dir, generate_ipc_token, get_default_favicon,
    get_favicon_url,
};

pub const ARC_BORDER: f64 = 8.0;
const STATE_FRAME_INTERVAL: Duration = Duration::from_millis(16);
const CLOSED_TAB_LIMIT: usize = 25;

struct ClosedTab {
    url: String,
    workspace: String,
    is_pinned: bool,
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
    pub started_at: Instant,
    pub perf: bool,
    pub first_page_logged: bool,
    pub sidebar_hover_expanded: bool,
    pub is_fullscreen: bool,
    pub collapse_generation: u64,
    pub active_little_arc_url: Option<String>,
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
            last_broadcast_memory_mb: get_process_memory_mb(),
            started_at,
            perf,
            first_page_logged: false,
            sidebar_hover_expanded: false,
            is_fullscreen: false,
            collapse_generation: 0,
            active_little_arc_url: None,
        };

        #[cfg(target_os = "macos")]
        {
            use tao::platform::macos::WindowExtMacOS;
            let ns_win = window.ns_window();
            crate::macos_menu::configure_window(ns_win);
        }

        let initial_url =
            std::env::var("ECLIPSE_TEST_URL").unwrap_or_else(|_| "eclipse://newtab".into());
        app.create_tab(&initial_url, true);

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

    pub fn content_bounds(&self) -> Rect {
        if self.is_fullscreen {
            return Rect {
                position: Position::Logical(LogicalPosition::new(0.0, 0.0)),
                size: Size::Logical(LogicalSize::new(self.window_size.0, self.window_size.1)),
            };
        }
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
        if self.is_fullscreen {
            return Rect {
                position: Position::Logical(LogicalPosition::new(-600.0, 0.0)),
                size: Size::Logical(LogicalSize::new(0.0, 0.0)),
            };
        }
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
            if normalized == "eclipse://newtab" {
                if let Some(tab) = self.tabs.iter().find(|t| t.id == id) {
                    tab.evaluate_script(
                        "setTimeout(function(){ var s = document.getElementById('search-box'); if(s){ s.focus(); s.select(); } }, 30);",
                    );
                }
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
        let config = self.config.clone();
        let mut target_workspace = None;

        for tab in &mut self.tabs {
            if tab.id == id {
                tab.restore_if_needed(&window, proxy.clone(), bounds, &config);
                tab.set_bounds(bounds);
                tab.set_visible(true);
                tab.last_active = Instant::now();
                let act_url = tab.url.clone();
                tab.sync_config(&config, Some(&act_url));
                tab.focus();
                if !tab.is_pinned {
                    target_workspace = Some(tab.workspace.clone());
                }
            } else {
                tab.set_visible(false);
            }
        }

        if let Some(workspace) = target_workspace {
            self.current_workspace = workspace;
        }
        self.active_tab_id = id;
        self.bring_sidebar_to_front();
        if self.config.sidebar_collapsed && self.sidebar_hover_expanded {
            self.hover_collapse_sidebar();
        }
        self.broadcast_state();
    }

    pub fn select_tab_by_workspace_index(&mut self, index: usize) {
        let current_ws = self.current_workspace.clone();
        let visible_ids: Vec<usize> = self
            .tabs
            .iter()
            .filter(|tab| tab.is_pinned || tab.workspace == current_ws)
            .map(|tab| tab.id)
            .collect();

        if visible_ids.is_empty() {
            return;
        }
        if index == 8 {
            if let Some(&last_id) = visible_ids.last() {
                self.select_tab(last_id);
            }
        } else if let Some(&id) = visible_ids.get(index) {
            self.select_tab(id);
        }
    }

    pub fn select_relative_tab(&mut self, delta: isize) {
        let current_ws = self.current_workspace.clone();
        let visible_ids: Vec<usize> = self
            .tabs
            .iter()
            .filter(|tab| tab.is_pinned || tab.workspace == current_ws)
            .map(|tab| tab.id)
            .collect();
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
        let closed_workspace = self.tabs[position].workspace.clone();

        self.closed_tabs.push(ClosedTab {
            url: self.tabs[position].url.clone(),
            workspace: closed_workspace.clone(),
            is_pinned: self.tabs[position].is_pinned,
        });
        if self.closed_tabs.len() > CLOSED_TAB_LIMIT {
            self.closed_tabs.remove(0);
        }

        self.tabs.remove(position);

        if self.tabs.is_empty() {
            self.current_workspace = closed_workspace;
            self.create_tab("eclipse://newtab", true);
            return;
        }

        if was_active {
            let next_id = self.tabs[position..]
                .iter()
                .find(|tab| !tab.is_pinned && tab.workspace == closed_workspace)
                .map(|tab| tab.id)
                .or_else(|| {
                    self.tabs[..position]
                        .iter()
                        .rev()
                        .find(|tab| !tab.is_pinned && tab.workspace == closed_workspace)
                        .map(|tab| tab.id)
                })
                .or_else(|| {
                    self.tabs
                        .get(position)
                        .map(|tab| tab.id)
                        .or_else(|| {
                            position
                                .checked_sub(1)
                                .and_then(|index| self.tabs.get(index))
                                .map(|tab| tab.id)
                        })
                })
                .unwrap_or(self.tabs[0].id);
            self.select_tab(next_id);
        } else {
            self.broadcast_state();
        }
    }

    pub fn reopen_closed_tab(&mut self) {
        let Some(record) = self.closed_tabs.pop() else {
            return;
        };
        self.current_workspace = record.workspace.clone();
        let id = self.create_tab(&record.url, true);
        if let Some(tab) = self.tabs.iter_mut().find(|tab| tab.id == id) {
            tab.is_pinned = record.is_pinned;
            tab.workspace = record.workspace;
        }
        self.broadcast_state();
    }

    pub fn switch_workspace(&mut self, workspace: String) {
        let normalized_ws = if workspace == "Work" {
            "Default".to_string()
        } else {
            workspace
        };
        self.current_workspace = normalized_ws.clone();

        // Apply per-space profile settings (theme, accent, adblock, custom UA)
        let profile = self.config.profile_for_workspace(&normalized_ws);
        self.config.theme = profile.theme.clone();
        self.config.accent_color = profile.accent_color.clone();
        self.config.adblock_enabled = profile.adblock_enabled;
        if !profile.custom_user_agent.trim().is_empty() {
            self.config.custom_user_agent = profile.custom_user_agent.clone();
        }
        self.config.save();

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
                    if !t.is_pinned {
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
        self.config.save();
        self.reconcile_favorite_tabs();
        self.broadcast_state();
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
        {
            if active_tab.url != "eclipse://newtab" {
                seen_urls.insert(active_tab.url.clone());
            }
        }

        for tab in &self.tabs {
            if tab.is_pinned || tab.favorite_index.is_some() || tab.workspace != ws {
                continue;
            }
            if tab.id == active_id {
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

        let removed_count = duplicate_ids.len();
        if !duplicate_ids.is_empty() {
            self.tabs.retain(|t| !duplicate_ids.contains(&t.id));
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
                "Tidied tabs into {group_count} group(s) • Closed {removed_count} duplicate(s)"
            ));
        } else {
            self.show_toast(&format!("Tidied tabs into {group_count} group(s)"));
        }
        self.broadcast_state();
    }

    pub fn focus_omnibar(&mut self) {
        if self.config.sidebar_collapsed {
            self.config.sidebar_collapsed = false;
            self.config.save();
            self.update_layout();
        }
        let _ = self.sidebar_webview.focus();
        let _ = self
            .sidebar_webview
            .evaluate_script("window.focusOmnibar && window.focusOmnibar();");
    }

    pub fn open_settings(&mut self) {
        if self.config.sidebar_collapsed {
            self.config.sidebar_collapsed = false;
            self.config.save();
            self.update_layout();
            self.flush_state();
        }
        let _ = self.sidebar_webview.focus();
        let _ = self
            .sidebar_webview
            .evaluate_script("window.openPreferencesDrawer && window.openPreferencesDrawer();");
    }

    pub fn open_downloads_panel(&mut self) {
        if self.config.sidebar_collapsed {
            self.config.sidebar_collapsed = false;
            self.config.save();
            self.update_layout();
            self.flush_state();
        }
        let _ = self.sidebar_webview.focus();
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

    pub fn open_little_arc(&mut self, url: &str) {
        let normalized = if url.trim().is_empty() {
            "https://duckduckgo.com".to_string()
        } else {
            normalize_url(url, &self.config)
        };
        self.active_little_arc_url = Some(normalized.clone());
        let encoded = serde_json::to_string(&normalized).unwrap_or_else(|_| "\"\"".into());
        if let Some(tab) = self.tabs.iter().find(|t| t.id == self.active_tab_id) {
            tab.evaluate_script(&format!(
                "window.__eclipseLittleArc && window.__eclipseLittleArc('open', {encoded});"
            ));
        }
        let _ = self.sidebar_webview.evaluate_script(&format!(
            "window.onLittleArcUpdated && window.onLittleArcUpdated({encoded});"
        ));
        self.broadcast_state();
    }

    pub fn promote_little_arc(&mut self, url: &str) {
        self.active_little_arc_url = None;
        if let Some(tab) = self.tabs.iter().find(|t| t.id == self.active_tab_id) {
            tab.evaluate_script(
                "window.__eclipseLittleArc && window.__eclipseLittleArc('close');",
            );
        }
        let target = if url.trim().is_empty() {
            "eclipse://newtab".to_string()
        } else {
            normalize_url(url, &self.config)
        };
        self.create_tab(&target, true);
    }

    pub fn close_little_arc(&mut self) {
        self.active_little_arc_url = None;
        if let Some(tab) = self.tabs.iter().find(|t| t.id == self.active_tab_id) {
            tab.evaluate_script(
                "window.__eclipseLittleArc && window.__eclipseLittleArc('close');",
            );
        }
        self.broadcast_state();
    }

    pub fn toggle_sidebar(&mut self) {
        self.sidebar_hover_expanded = false;
        self.collapse_generation += 1;
        #[cfg(target_os = "macos")]
        crate::macos_menu::set_hover_expanded(false, self.sidebar_width());
        self.config.sidebar_collapsed = !self.config.sidebar_collapsed;
        self.config.save();
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
        if self.config.sidebar_collapsed && !self.sidebar_hover_expanded {
            self.collapse_generation += 1;
            self.sidebar_hover_expanded = true;
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

    pub fn hover_collapse_sidebar(&mut self) {
        if self.sidebar_hover_expanded {
            self.collapse_generation += 1;
            self.sidebar_hover_expanded = false;
            #[cfg(target_os = "macos")]
            crate::macos_menu::set_hover_expanded(false, self.sidebar_width());
            let _ = self
                .sidebar_webview
                .evaluate_script("window.popoutSidebar && window.popoutSidebar(false);");
            self.animate_sidebar_bounds(self.sidebar_bounds(), 0.20);
            self.flush_state();
        }
    }

    pub fn finish_sidebar_collapse(&mut self, _generation: u64) {}

    pub fn set_fullscreen(&mut self, fullscreen: bool) {
        if self.is_fullscreen == fullscreen {
            return;
        }
        self.is_fullscreen = fullscreen;
        if fullscreen {
            self.sidebar_hover_expanded = false;
            #[cfg(target_os = "macos")]
            crate::macos_menu::set_hover_expanded(false, self.sidebar_width());
        }
        let _ = self.sidebar_webview.set_bounds(self.sidebar_bounds());
        let _ = self.sidebar_webview.set_visible(!fullscreen);
        if !fullscreen {
            self.bring_sidebar_to_front();
        }
        let bounds = self.content_bounds();
        for tab in &mut self.tabs {
            tab.set_bounds(bounds);
        }
        self.broadcast_state();
    }

    pub fn open_command_bar(&mut self) {
        if let Some(tab) = self.tabs.iter().find(|tab| tab.id == self.active_tab_id) {
            if tab.url.starts_with("eclipse://newtab") {
                tab.evaluate_script(
                    "const s = document.getElementById('search-box'); if (s) { s.focus(); s.select(); }",
                );
            } else {
                let open_tabs: Vec<serde_json::Value> = self
                    .tabs
                    .iter()
                    .map(|t| {
                        serde_json::json!({
                            "id": t.id,
                            "title": t.title,
                            "url": t.url,
                            "is_pinned": t.is_pinned,
                        })
                    })
                    .collect();
                let tabs_json = serde_json::to_string(&open_tabs).unwrap_or_else(|_| "[]".into());
                tab.evaluate_script(&format!(
                    "if (window.__eclipseCommandBar) {{ window.__eclipseCommandBar('open', {tabs_json}); }}"
                ));
            }
            tab.focus();
        }
    }

    pub fn handle_ipc(&mut self, raw: &str) {
        if let Some((token, command)) = parse_ipc(raw) {
            if token == self.sidebar_token {
                self.handle_ipc_command(None, command);
                return;
            }
        }
        if let Some(command) = parse_sidebar_ipc(raw) {
            self.handle_ipc_command(None, command);
        }
    }

    pub fn handle_tab_ipc(&mut self, tab_id: usize, raw: &str) {
        let Some((token, command)) = parse_ipc(raw) else {
            return;
        };
        let Some(expected) = self
            .tabs
            .iter()
            .find(|tab| tab.id == tab_id)
            .map(|tab| tab.token.as_str())
        else {
            return;
        };
        if token != expected {
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
            IpcCommand::CleanMemory => self.handle_maintenance_tick(),
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
            IpcCommand::Print => {
                if let Some(tab) = self.tabs.iter().find(|tab| tab.id == self.active_tab_id) {
                    tab.evaluate_script("window.print();");
                }
            }
            IpcCommand::FavoriteTab => self.favorite_active_tab(),
            IpcCommand::ToggleSidebar => self.toggle_sidebar(),
            IpcCommand::HoverExpandSidebar => self.hover_expand_sidebar(),
            IpcCommand::HoverCollapseSidebar => self.hover_collapse_sidebar(),
            IpcCommand::FocusOmnibar => self.focus_omnibar(),
            IpcCommand::OpenCommandBar => self.open_command_bar(),
            IpcCommand::SetFullscreen { fullscreen } => self.set_fullscreen(fullscreen),
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
                        "clean_memory" => self.handle_maintenance_tick(),
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
            IpcCommand::UpdateConfig { mut config } => {
                let custom_css_changed = self.config.custom_css != config.custom_css;
                let favorites_changed = self.config.favorites != config.favorites;
                let layout_changed = self.config.sidebar_collapsed != config.sidebar_collapsed
                    || (self.config.sidebar_width - config.sidebar_width).abs() > f64::EPSILON;

                // Also update the active space profile's theme/accent/adblock so per-space profiles stay synced
                let ws = self.current_workspace.clone();
                if let Some(sp) = config
                    .space_profiles
                    .iter_mut()
                    .find(|p| p.workspace.eq_ignore_ascii_case(&ws))
                {
                    sp.theme = config.theme.clone();
                    sp.accent_color = config.accent_color.clone();
                    sp.adblock_enabled = config.adblock_enabled;
                }

                self.config = config;
                self.config.save();
                if favorites_changed {
                    self.reconcile_favorite_tabs();
                }

                let config_snapshot = self.config.clone();
                let active_url = self.active_url();
                for tab in &mut self.tabs {
                    if custom_css_changed {
                        tab.apply_custom_css(&config_snapshot.custom_css);
                    }
                    tab.sync_config(&config_snapshot, Some(&active_url));
                }

                if layout_changed {
                    self.update_layout();
                } else {
                    self.broadcast_state();
                }
            }
            IpcCommand::ToggleSiteAdblock { domain, enabled } => {
                self.toggle_site_adblock(&domain, enabled);
            }
            IpcCommand::InstallExtension { path } => {
                self.install_extension_from_path(&path);
            }
            IpcCommand::InstallFromWebStore { input } => {
                match self.install_from_web_store(&input) {
                    Ok(_) => self.show_toast("Extension installed"),
                    Err(err) => self.show_toast(&err),
                }
            }
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
                if let Some(index) = index {
                    if index < self.config.favorites.len() {
                        self.config.favorites[index] = favorite;
                    } else {
                        self.config.favorites.push(favorite);
                    }
                } else {
                    self.config.favorites.push(favorite);
                }
                self.config.save();
                let config_snapshot = self.config.clone();
                let active_url = self.active_url();
                for tab in &self.tabs {
                    tab.sync_config(&config_snapshot, Some(&active_url));
                }
                self.broadcast_state();
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
                    self.config.save();
                    let config_snapshot = self.config.clone();
                    let active_url = self.active_url();
                    for tab in &self.tabs {
                        tab.sync_config(&config_snapshot, Some(&active_url));
                    }
                    self.broadcast_state();
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
                self.config.save();
                self.broadcast_state();
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
                self.config.save();
                let cfg = self.config.clone();
                let active_url = self.active_url();
                for tab in &self.tabs {
                    tab.sync_config(&cfg, Some(&active_url));
                }
                self.broadcast_state();
            }
            IpcCommand::DeleteNote { id } => {
                self.config.notes.retain(|n| n.id != id);
                self.config.save();
                let cfg = self.config.clone();
                let active_url = self.active_url();
                for tab in &self.tabs {
                    tab.sync_config(&cfg, Some(&active_url));
                }
                self.broadcast_state();
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
                self.config.save();
                let cfg = self.config.clone();
                let active_url = self.active_url();
                for tab in &self.tabs {
                    tab.sync_config(&cfg, Some(&active_url));
                }
                self.broadcast_state();
            }
            IpcCommand::DeleteEasel { id } => {
                self.config.easels.retain(|e| e.id != id);
                self.config.save();
                let cfg = self.config.clone();
                let active_url = self.active_url();
                for tab in &self.tabs {
                    tab.sync_config(&cfg, Some(&active_url));
                }
                self.broadcast_state();
            }
            IpcCommand::OpenEaselTab { id: _ } => {
                self.open_easel_tab();
            }
            IpcCommand::UpdateSpaceProfile { profile } => {
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
                if self.current_workspace.eq_ignore_ascii_case(&profile.workspace) {
                    self.config.theme = profile.theme;
                    self.config.accent_color = profile.accent_color;
                    self.config.adblock_enabled = profile.adblock_enabled;
                    self.config.custom_user_agent = profile.custom_user_agent;
                }
                self.config.save();
                let cfg = self.config.clone();
                let active_url = self.active_url();
                for tab in &self.tabs {
                    tab.sync_config(&cfg, Some(&active_url));
                }
                self.broadcast_state();
            }
            IpcCommand::OpenLittleArc { url } => {
                self.open_little_arc(&url);
            }
            IpcCommand::PromoteLittleArc { url } => {
                self.promote_little_arc(&url);
            }
            IpcCommand::CloseLittleArc => {
                self.close_little_arc();
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
                    eprintln!("[chrome-js] {}", truncate_message(&message, 500));
                }
            }
            IpcCommand::UrlChanged { url } => {
                if !url.starts_with("http://") && !url.starts_with("https://") {
                    return;
                }
                let target_id = source_tab_id.unwrap_or(self.active_tab_id);
                if let Some(tab) = self.tabs.iter_mut().find(|tab| tab.id == target_id)
                    && tab.url != url
                {
                    tab.url = url;
                    self.broadcast_state();
                }
            }
            IpcCommand::WindowClose => {
                let _ = self.proxy.send_event(UserEvent::CloseWindow);
            }
            IpcCommand::WindowMinimize => {
                self.window.set_minimized(true);
            }
            IpcCommand::WindowZoom => {
                #[cfg(target_os = "macos")]
                crate::macos_menu::toggle_fullscreen();
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
                self.flush_state();
                if self.perf {
                    self.probe_sidebar();
                }
            }
        }
    }

    pub fn reconcile_favorite_tabs(&mut self) {
        let fav_hosts: Vec<Option<String>> = self
            .config
            .favorites
            .iter()
            .map(|f| {
                url::Url::parse(&f.url).ok().and_then(|u| {
                    u.host_str()
                        .map(|h| h.trim_start_matches("www.").to_ascii_lowercase())
                })
            })
            .collect();

        for tab in &mut self.tabs {
            if let Some(idx) = tab.favorite_index {
                let tab_host = url::Url::parse(&tab.url).ok().and_then(|u| {
                    u.host_str()
                        .map(|h| h.trim_start_matches("www.").to_ascii_lowercase())
                });
                if idx < fav_hosts.len() && fav_hosts[idx].is_some() && fav_hosts[idx] == tab_host {
                    continue;
                }
                if let Some(ref th) = tab_host
                    && let Some(new_idx) = fav_hosts.iter().position(|fh| fh.as_ref() == Some(th))
                {
                    tab.favorite_index = Some(new_idx);
                } else {
                    tab.favorite_index = None;
                    tab.is_pinned = false;
                }
            }
        }
    }

    pub fn toggle_site_adblock(&mut self, domain: &str, enabled: bool) {
        let raw = domain.trim().to_lowercase();
        if raw.is_empty() {
            return;
        }
        let domain_clean = if raw.contains("://") {
            url::Url::parse(&raw)
                .ok()
                .and_then(|u| u.host_str().map(|h| h.to_lowercase()))
                .unwrap_or(raw)
        } else {
            raw.split('/')
                .next()
                .unwrap_or("")
                .split(':')
                .next()
                .unwrap_or("")
                .trim()
                .to_string()
        };
        if domain_clean.is_empty() || domain_clean == "localhost" {
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
        self.config.save();

        let cfg = self.config.clone();
        let active_url = self.active_url();
        for tab in &self.tabs {
            tab.sync_config(&cfg, Some(&active_url));
        }

        if let Some(tab) = self.tabs.iter_mut().find(|t| t.id == self.active_tab_id) {
            tab.reload();
        }
        self.broadcast_state();
    }

    pub fn toggle_extension(&mut self, id: &str, enabled: bool) {
        if let Some(ext) = self
            .config
            .installed_extensions
            .iter_mut()
            .find(|e| e.id == id)
        {
            ext.enabled = enabled;
            self.config.save();
            let cfg = self.config.clone();
            let active_url = self.active_url();
            for tab in &self.tabs {
                tab.sync_config(&cfg, Some(&active_url));
            }
            self.broadcast_state();
        }
    }

    pub fn delete_extension(&mut self, id: &str) {
        self.config.installed_extensions.retain(|e| e.id != id);
        self.config.save();
        let cfg = self.config.clone();
        let active_url = self.active_url();
        for tab in &self.tabs {
            tab.sync_config(&cfg, Some(&active_url));
        }
        self.broadcast_state();
    }

    pub fn install_extension_from_path(&mut self, path_str: &str) {
        use std::path::Path;
        let p = Path::new(path_str.trim());
        let manifest_path = if p.is_dir() {
            p.join("manifest.json")
        } else {
            p.to_path_buf()
        };

        if let Ok(content) = std::fs::read_to_string(&manifest_path) {
            if let Ok(json) = serde_json::from_str::<serde_json::Value>(&content) {
                let name = json["name"]
                    .as_str()
                    .unwrap_or("Custom Extension")
                    .to_string();
                let version = json["version"].as_str().unwrap_or("1.0.0").to_string();
                let description = json["description"]
                    .as_str()
                    .unwrap_or("Loaded unpacked Chrome extension")
                    .to_string();
                let base_dir = manifest_path.parent().unwrap_or(p);

                let mut matches = Vec::new();
                let mut scripts = Vec::new();
                let mut styles = Vec::new();

                if let Some(cs_arr) = json["content_scripts"].as_array() {
                    for cs in cs_arr {
                        if let Some(m_arr) = cs["matches"].as_array() {
                            for m in m_arr {
                                if let Some(m_str) = m.as_str() {
                                    matches.push(m_str.to_string());
                                }
                            }
                        }
                        if let Some(js_arr) = cs["js"].as_array() {
                            for js in js_arr {
                                if let Some(js_file) = js.as_str() {
                                    let script_file = base_dir.join(js_file);
                                    if let Ok(code) = std::fs::read_to_string(&script_file) {
                                        scripts.push(code);
                                    }
                                }
                            }
                        }
                        if let Some(css_arr) = cs["css"].as_array() {
                            for css in css_arr {
                                if let Some(css_file) = css.as_str() {
                                    let style_file = base_dir.join(css_file);
                                    if let Ok(code) = std::fs::read_to_string(&style_file) {
                                        styles.push(code);
                                    }
                                }
                            }
                        }
                    }
                }

                if matches.is_empty() {
                    matches.push("<all_urls>".into());
                }

                let id = format!(
                    "ext_{}",
                    SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .map(|d| d.as_millis())
                        .unwrap_or(0)
                );
                let ext = crate::config::ExtensionConfig {
                    id,
                    name: name.clone(),
                    version,
                    description,
                    enabled: true,
                    path: Some(path_str.to_string()),
                    matches,
                    content_scripts: scripts,
                    content_css: styles,
                };
                self.config
                    .installed_extensions
                    .retain(|e| e.name != name && e.path.as_deref() != Some(path_str));
                self.config.installed_extensions.push(ext);
                self.config.save();
                let cfg = self.config.clone();
                let active_url = self.active_url();
                for tab in &self.tabs {
                    tab.sync_config(&cfg, Some(&active_url));
                }
                self.broadcast_state();
            }
        }
    }

    pub fn install_from_web_store(&mut self, input: &str) -> Result<String, String> {
        let ext_id = extract_chrome_extension_id(input).ok_or_else(|| {
            "Invalid Chrome Web Store URL or ID. Expected 32-character extension ID.".to_string()
        })?;

        let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
        let ext_dir = std::path::PathBuf::from(home)
            .join(".config")
            .join("eclipse-browser")
            .join("extensions")
            .join(&ext_id);
        let _ = std::fs::create_dir_all(&ext_dir);

        let crx_path = ext_dir.join("download.crx");
        let zip_path = ext_dir.join("download.zip");

        let download_url = format!(
            "https://clients2.google.com/service/update2/crx?response=redirect&prodversion=130.0&acceptformat=crx2,crx3&x=id%3D{}%26uc",
            ext_id
        );

        let status = std::process::Command::new("curl")
            .arg("-s")
            .arg("-L")
            .arg("--connect-timeout")
            .arg("10")
            .arg("--max-time")
            .arg("30")
            .arg(&download_url)
            .arg("-o")
            .arg(&crx_path)
            .status()
            .map_err(|e| format!("Failed to download extension: {e}"))?;

        if !status.success() || !crx_path.exists() {
            return Err("Failed to download CRX from Chrome Web Store.".into());
        }

        let bytes = std::fs::read(&crx_path).map_err(|e| format!("Failed to read CRX: {e}"))?;
        let zip_magic = [0x50, 0x4B, 0x03, 0x04];
        let zip_offset = bytes
            .windows(4)
            .position(|w| w == zip_magic)
            .ok_or_else(|| {
                "Downloaded file is not a valid Chrome Web Extension archive.".to_string()
            })?;

        std::fs::write(&zip_path, &bytes[zip_offset..])
            .map_err(|e| format!("Failed to create zip archive: {e}"))?;

        let _ = std::fs::remove_file(&crx_path);

        let unzip_status = std::process::Command::new("/usr/bin/unzip")
            .arg("-q")
            .arg("-o")
            .arg(&zip_path)
            .arg("-d")
            .arg(&ext_dir)
            .status()
            .map_err(|e| format!("Failed to extract extension: {e}"))?;

        let _ = std::fs::remove_file(&zip_path);

        if !unzip_status.success() || !ext_dir.join("manifest.json").exists() {
            return Err("Failed to unpack extension manifest.".into());
        }

        self.install_extension_from_path(&ext_dir.to_string_lossy());
        Ok(ext_id)
    }

    pub fn handle_page_started(&mut self, tab_id: usize, url: String) {
        let canonical = canonicalize_eclipse_url(&url);
        let active_url = if tab_id == self.active_tab_id {
            canonical.clone()
        } else {
            self.active_url()
        };
        if self.perf {
            eprintln!(
                "[perf] tab {tab_id} started {canonical} at {:.1} ms",
                self.started_at.elapsed().as_secs_f64() * 1000.0
            );
        }
        if let Some(tab) = self.tabs.iter_mut().find(|tab| tab.id == tab_id) {
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
            tab.sync_config(&self.config, Some(&active_url));
        }
        self.broadcast_state();
    }

    pub fn handle_page_finished(&mut self, tab_id: usize, url: String) {
        let canonical = canonicalize_eclipse_url(&url);
        let active_url = if tab_id == self.active_tab_id {
            canonical.clone()
        } else {
            self.active_url()
        };
        if self.perf {
            eprintln!(
                "[perf] tab {tab_id} finished {canonical} at {:.1} ms",
                self.started_at.elapsed().as_secs_f64() * 1000.0
            );
        }
        let config = self.config.clone();
        if let Some(tab) = self.tabs.iter_mut().find(|tab| tab.id == tab_id) {
            tab.url = canonical.clone();
            tab.is_loading = false;
            tab.set_loading_indicator(false);
            tab.favicon = get_default_favicon(&canonical);
            if tab.favicon_url.is_none() {
                tab.favicon_url = get_favicon_url(&canonical);
            }
            tab.restore_scroll_if_pending();
            tab.sync_config(&config, Some(&active_url));

            if self.perf && tab.id == self.active_tab_id {
                tab.evaluate_with_callback(
                    r#"JSON.stringify({
                        bridge: window.__eclipseBridgeActive === true,
                        find: typeof window.__eclipseFind,
                        shields: typeof window.__eclipseShields,
                        config: location.protocol === 'eclipse:' ? !!window.__ECLIPSE_CONFIG__ : 'hidden-from-web',
                        brandIcon: (function () {
                            var el = document.querySelector('img[src="eclipse://localhost/icon"]');
                            return el ? { complete: el.complete, w: el.naturalWidth, h: el.naturalHeight } : null;
                        })()
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

    pub fn handle_new_window(&mut self, url: String) {
        if url.is_empty() || url == "about:blank" {
            self.create_tab("eclipse://newtab", true);
            return;
        }
        if url.starts_with("http://")
            || url.starts_with("https://")
            || url.starts_with("eclipse://")
        {
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
        self.config.save();
        self.show_toast(&format!("Downloading {file_name}…"));
        self.broadcast_state();
    }

    pub fn handle_download_completed(
        &mut self,
        file_name: String,
        path: Option<String>,
        success: bool,
    ) {
        if let Some(item) = self
            .config
            .downloads
            .iter_mut()
            .find(|d| d.file_name == file_name && d.status == "downloading")
        {
            item.status = if success {
                "completed".into()
            } else {
                "failed".into()
            };
            if let Some(p) = path {
                item.path = p;
            }
        } else {
            let id = self.next_download_id;
            self.next_download_id += 1;
            self.config.downloads.insert(
                0,
                DownloadItem {
                    id,
                    file_name: file_name.clone(),
                    url: String::new(),
                    path: path.unwrap_or_default(),
                    status: if success {
                        "completed".into()
                    } else {
                        "failed".into()
                    },
                    timestamp: now_unix_secs(),
                },
            );
        }
        self.config.save();
        if success {
            self.show_toast(&format!("Downloaded {file_name}"));
        } else {
            self.show_toast(&format!("Download failed: {file_name}"));
        }
        self.broadcast_state();
    }

    pub fn handle_menu_action(&mut self, action: &str) {
        match action {
            "new_tab" => {
                // Cmd+T ALWAYS creates and switches to a fresh tab reliably!
                self.create_tab("eclipse://newtab", true);
            }
            "command_bar" => {
                self.open_command_bar();
            }
            "little_arc" => {
                let url = self.active_url();
                let target = if url.is_empty() || url.starts_with("eclipse://") {
                    "https://duckduckgo.com"
                } else {
                    &url
                };
                self.open_little_arc(target);
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
            "toggle_sidebar" => self.toggle_sidebar(),
            "hover_expand_sidebar" => self.hover_expand_sidebar(),
            "hover_collapse_sidebar" => self.hover_collapse_sidebar(),
            "open_devtools" => {
                if let Some(tab) = self.tabs.iter().find(|tab| tab.id == self.active_tab_id) {
                    tab.open_devtools();
                }
            }
            "find" => self.find_active("open"),
            "find_next" => self.find_active("next"),
            "find_prev" => self.find_active("prev"),
            "print" => {
                if let Some(tab) = self.tabs.iter().find(|tab| tab.id == self.active_tab_id) {
                    tab.evaluate_script("window.print();");
                }
            }
            "favorite_tab" => self.favorite_active_tab(),
            "duplicate_tab" => {
                if let Some(tab) = self.tabs.iter().find(|tab| tab.id == self.active_tab_id) {
                    let url = tab.url.clone();
                    self.create_tab(&url, true);
                }
            }
            _ => {}
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
                self.config.favorites.push(crate::config::FavoriteShortcut {
                    title,
                    url,
                    icon: String::new(),
                });
                self.config.save();
                self.broadcast_state();
            }
        }
    }

    pub fn handle_maintenance_tick(&mut self) {
        let mut hibernated_any = false;
        let minutes = self.config.tab_hibernation_mins;

        if minutes > 0 {
            let threshold = Duration::from_secs((minutes as u64) * 60);
            let now = Instant::now();
            let active_id = self.active_tab_id;

            for tab in &mut self.tabs {
                if tab.id == active_id
                    || tab.is_hibernated
                    || tab.webview.is_none()
                    || tab.is_audible
                    || tab.is_loading
                {
                    continue;
                }
                if now.duration_since(tab.last_active) > threshold {
                    tab.hibernate();
                    hibernated_any = true;
                }
            }
        }

        if hibernated_any {
            self.broadcast_state();
            return;
        }

        let current_mb = get_process_memory_mb();
        if (current_mb - self.last_broadcast_memory_mb).abs() >= 1.0 {
            self.last_broadcast_memory_mb = current_mb;
            let script =
                format!("window.updateMemoryUsage && window.updateMemoryUsage({current_mb:.1});");
            let _ = self.sidebar_webview.evaluate_script(&script);
        }
    }

    pub fn update_layout(&mut self) {
        self.update_layout_animated(0.0);
    }

    pub fn update_layout_animated(&mut self, duration: f64) {
        if !self.is_fullscreen {
            let _ = self.sidebar_webview.set_visible(true);
            self.bring_sidebar_to_front();
        }
        let sidebar_bounds = self.sidebar_bounds();
        if duration > 0.001 {
            self.animate_sidebar_bounds(sidebar_bounds, duration);
        } else {
            let _ = self.sidebar_webview.set_bounds(sidebar_bounds);
        }

        let content_bounds = self.content_bounds();
        for tab in &self.tabs {
            if tab.id == self.active_tab_id {
                tab.animate_bounds(content_bounds, duration);
            } else if duration <= 0.001 {
                tab.set_bounds(content_bounds);
            }
        }
        self.broadcast_state();
    }

    pub fn resize(&mut self, width: f64, height: f64) {
        self.window_size = (width, height);
        self.update_layout();
    }

    pub fn broadcast_state(&mut self) {
        self.state_dirty = true;
    }

    pub fn pump_state(&mut self) -> Option<Instant> {
        if !self.state_dirty {
            return None;
        }
        let now = Instant::now();
        if now >= self.next_state_flush {
            self.flush_state();
            None
        } else {
            Some(self.next_state_flush)
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
        let memory_mb = get_process_memory_mb();
        self.last_broadcast_memory_mb = memory_mb;

        let ui_state = BrowserUiState {
            tabs: self.tabs.iter().map(Tab::to_state).collect(),
            active_tab_id: self.active_tab_id,
            active_url,
            active_title: active_title.clone(),
            can_go_back,
            can_go_forward,
            is_loading,
            sidebar_collapsed: self.config.sidebar_collapsed,
            sidebar_hover_expanded: self.sidebar_hover_expanded,
            current_workspace: self.current_workspace.clone(),
            memory_mb,
            config: self.config.clone(),
            favorites: self.config.favorites.clone(),
            downloads: self.config.downloads.clone(),
            notes: self.config.notes.clone(),
            easels: self.config.easels.clone(),
            space_profiles: self.config.space_profiles.clone(),
            active_little_arc_url: self.active_little_arc_url.clone(),
        };

        if let Ok(json) = serde_json::to_string(&ui_state) {
            let script = format!("window.updateBrowserState && window.updateBrowserState({json});");
            let _ = self.sidebar_webview.evaluate_script(&script);

            if let Some(tab) = active_tab
                && tab.url.starts_with("eclipse://")
            {
                tab.evaluate_script(&format!(
                    "window.onEclipseStateUpdated && window.onEclipseStateUpdated({json});"
                ));
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
        let script = r#"JSON.stringify({
            tokenBridge: !!window.__eclipseIpc,
            stateHandler: typeof window.updateBrowserState,
            tabCards: document.querySelectorAll('#tab-container .tab-card').length,
            pinnedItems: document.querySelectorAll('#pinned-container .pinned-item').length,
            omnibar: document.getElementById('omnibar').value,
            ram: document.getElementById('ram-text').textContent,
            workspace: document.getElementById('workspace-label').textContent,
            drawer: getComputedStyle(document.getElementById('settings-drawer')).display,
            sidebarWidth: window.innerWidth,
            overflowX: document.documentElement.scrollWidth > document.documentElement.clientWidth + 1,
            brandIcon: (function () {
                var el = document.querySelector('.brand-icon');
                return el ? { complete: el.complete, w: el.naturalWidth, h: el.naturalHeight } : null;
            })()
        })"#;
        let _ = self
            .sidebar_webview
            .evaluate_script_with_callback(script, |result| {
                eprintln!("[perf] sidebar probe: {result}");
            });
    }
}

pub fn categorize_tab_group(url: &str, title: &str) -> String {
    if url.starts_with("eclipse://") {
        return "Eclipse Workspace".into();
    }
    let lower_url = url.to_lowercase();
    let lower_title = title.to_lowercase();

    if lower_url.contains("github.com")
        || lower_url.contains("gitlab.com")
        || lower_url.contains("stackoverflow.com")
        || lower_url.contains("crates.io")
        || lower_url.contains("docs.rs")
        || lower_url.contains("localhost")
        || lower_url.contains("127.0.0.1")
        || lower_url.contains("vercel.com")
        || lower_title.contains("pull request")
    {
        return "Development".into();
    }
    if lower_url.contains("youtube.com")
        || lower_url.contains("youtu.be")
        || lower_url.contains("netflix.com")
        || lower_url.contains("spotify.com")
        || lower_url.contains("twitch.tv")
        || lower_url.contains("vimeo.com")
    {
        return "Media & Stream".into();
    }
    if lower_url.contains("mail.google.com")
        || lower_url.contains("outlook.")
        || lower_url.contains("docs.google.com")
        || lower_url.contains("notion.so")
        || lower_url.contains("linear.app")
        || lower_url.contains("figma.com")
        || lower_url.contains("remotasks.com")
    {
        return "Work & Productivity".into();
    }
    if lower_url.contains("discord.com")
        || lower_url.contains("reddit.com")
        || lower_url.contains("instagram.com")
        || lower_url.contains("x.com")
        || lower_url.contains("twitter.com")
        || lower_url.contains("linkedin.com")
    {
        return "Social & Community".into();
    }
    if lower_url.contains("duckduckgo.com")
        || lower_url.contains("google.com/search")
        || lower_url.contains("wikipedia.org")
        || lower_url.contains("arxiv.org")
        || lower_url.contains("chatgpt.com")
        || lower_url.contains("claude.ai")
    {
        return "Research & Search".into();
    }
    if let Ok(parsed) = url::Url::parse(url)
        && let Some(host) = parsed.host_str()
    {
        let clean = host.trim_start_matches("www.");
        if let Some(first) = clean.split('.').next()
            && !first.is_empty()
        {
            let mut chars = first.chars();
            if let Some(c) = chars.next() {
                return format!("{}{}", c.to_uppercase(), chars.as_str());
            }
        }
    }
    "General".into()
}

fn now_unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn parse_ipc(raw: &str) -> Option<(String, IpcCommand)> {
    if raw.len() > 64 * 1024 {
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

fn parse_sidebar_ipc(raw: &str) -> Option<IpcCommand> {
    if raw.len() > 64 * 1024 {
        return None;
    }
    let value: serde_json::Value = serde_json::from_str(raw).ok()?;
    serde_json::from_value(value).ok()
}

fn is_safe_icon_url(url: &str) -> bool {
    url.len() <= 2048 && (url.starts_with("https://") || url.starts_with("http://"))
}

fn truncate_message(message: &str, limit: usize) -> String {
    if message.chars().count() <= limit {
        message.to_string()
    } else {
        let mut truncated: String = message.chars().take(limit).collect();
        truncated.push('…');
        truncated
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
    45.0
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
}
