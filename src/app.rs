use std::sync::Arc;
use std::time::{Duration, Instant};
use tao::dpi::{LogicalPosition, LogicalSize, Position, Size};
use tao::event_loop::EventLoopProxy;
use tao::window::Window;
use wry::{Rect, WebView, WebViewBuilder};

use crate::config::{BrowserConfig, FavoriteShortcut};
use crate::events::{BrowserUiState, IpcCommand, UserEvent};
use crate::tab::{
    Tab, canonicalize_eclipse_url, generate_ipc_token, get_default_favicon, get_favicon_url,
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
            -sidebar_w - 50.0
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
            .with_visible(!config.sidebar_collapsed)
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

        let mut app = Self {
            window: window.clone(),
            proxy: proxy.clone(),
            sidebar_webview,
            sidebar_token,
            tabs: Vec::new(),
            active_tab_id: 1,
            next_tab_id: 1,
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
                -sidebar_w - 50.0
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
            let next_id = self
                .tabs
                .get(position)
                .map(|tab| tab.id)
                .or_else(|| {
                    position
                        .checked_sub(1)
                        .and_then(|index| self.tabs.get(index))
                        .map(|tab| tab.id)
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
        self.current_workspace = workspace.clone();

        let active_in_ws = self
            .tabs
            .iter()
            .find(|tab| tab.id == self.active_tab_id)
            .map(|tab| tab.workspace == workspace)
            .unwrap_or(false);

        if active_in_ws {
            self.broadcast_state();
            return;
        }

        let candidate_id = self
            .tabs
            .iter()
            .filter(|tab| tab.workspace == workspace)
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

        // 1. Look for an existing tab explicitly linked to this favorite index
        let mut target_tab_id = self
            .tabs
            .iter()
            .find(|t| t.favorite_index == Some(index))
            .map(|t| t.id);

        // 2. Fallback: look for any pinned tab matching the favorite URL/host
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
            // Check if current active tab is an unpinned blank newtab that can be converted
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

    pub fn toggle_sidebar(&mut self) {
        self.sidebar_hover_expanded = false;
        #[cfg(target_os = "macos")]
        crate::macos_menu::set_hover_expanded(false, self.sidebar_width());
        self.config.sidebar_collapsed = !self.config.sidebar_collapsed;
        self.config.save();
        if self.config.sidebar_collapsed {
            let _ = self
                .sidebar_webview
                .evaluate_script("window.popoutSidebar && window.popoutSidebar(false);");
            self.update_layout_animated(0.28);
            let proxy = self.proxy.clone();
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(300));
                let _ = proxy.send_event(UserEvent::FinishSidebarCollapse);
            });
            if let Some(tab) = self.tabs.iter().find(|tab| tab.id == self.active_tab_id) {
                tab.focus();
            }
        } else {
            let _ = self.sidebar_webview.set_visible(true);
            let _ = self.sidebar_webview.evaluate_script(
                "document.body.classList.remove('floating-popout', 'sliding-out');",
            );
            self.update_layout_animated(0.28);
        }
    }

    pub fn hover_expand_sidebar(&mut self) {
        if self.config.sidebar_collapsed && !self.sidebar_hover_expanded {
            self.sidebar_hover_expanded = true;
            #[cfg(target_os = "macos")]
            crate::macos_menu::set_hover_expanded(true, self.sidebar_width());
            let sidebar_bounds = self.sidebar_bounds();
            let _ = self.sidebar_webview.set_bounds(sidebar_bounds);
            let _ = self.sidebar_webview.set_visible(true);
            self.bring_sidebar_to_front();
            let _ = self
                .sidebar_webview
                .evaluate_script("window.popoutSidebar && window.popoutSidebar(true);");
            self.flush_state();
        }
    }

    pub fn hover_collapse_sidebar(&mut self) {
        if self.sidebar_hover_expanded {
            self.sidebar_hover_expanded = false;
            #[cfg(target_os = "macos")]
            crate::macos_menu::set_hover_expanded(false, self.sidebar_width());
            let _ = self
                .sidebar_webview
                .evaluate_script("window.popoutSidebar && window.popoutSidebar(false);");
            self.flush_state();

            let proxy = self.proxy.clone();
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(215));
                let _ = proxy.send_event(UserEvent::FinishSidebarCollapse);
            });
        }
    }

    pub fn finish_sidebar_collapse(&mut self) {
        if self.config.sidebar_collapsed && !self.sidebar_hover_expanded {
            let _ = self.sidebar_webview.set_bounds(self.sidebar_bounds());
            let _ = self.sidebar_webview.set_visible(false);
            let _ = self.sidebar_webview.evaluate_script(
                "document.body.classList.remove('floating-popout', 'sliding-out');",
            );
        }
    }

    pub fn set_fullscreen(&mut self, fullscreen: bool) {
        if self.is_fullscreen == fullscreen {
            return;
        }
        self.is_fullscreen = fullscreen;
        let _ = self.sidebar_webview.set_bounds(self.sidebar_bounds());
        let _ = self
            .sidebar_webview
            .set_visible(!fullscreen && !self.config.sidebar_collapsed);
        let bounds = self.content_bounds();
        for tab in &mut self.tabs {
            if tab.id == self.active_tab_id {
                tab.set_bounds(bounds);
            }
        }
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
                    "window.__eclipseCommandBar && window.__eclipseCommandBar('open', {tabs_json});"
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
            IpcCommand::UpdateConfig { config } => {
                let custom_css_changed = self.config.custom_css != config.custom_css;
                let layout_changed = self.config.sidebar_collapsed != config.sidebar_collapsed
                    || (self.config.sidebar_width - config.sidebar_width).abs() > f64::EPSILON;
                self.config = config;
                self.config.save();

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
                    self.config.save();
                    let config_snapshot = self.config.clone();
                    let active_url = self.active_url();
                    for tab in &self.tabs {
                        tab.sync_config(&config_snapshot, Some(&active_url));
                    }
                    self.broadcast_state();
                }
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

    pub fn toggle_site_adblock(&mut self, domain: &str, enabled: bool) {
        let domain_clean = domain.trim().to_lowercase();
        if domain_clean.is_empty() {
            return;
        }
        if enabled {
            self.config
                .adblock_disabled_domains
                .retain(|d| d.to_lowercase() != domain_clean);
        } else {
            if !self
                .config
                .adblock_disabled_domains
                .iter()
                .any(|d| d.to_lowercase() == domain_clean)
            {
                self.config.adblock_disabled_domains.push(domain_clean);
            }
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
        self.config
            .installed_extensions
            .retain(|e| e.id != id);
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
                let name = json["name"].as_str().unwrap_or("Custom Extension").to_string();
                let version = json["version"].as_str().unwrap_or("1.0.0").to_string();
                let description = json["description"].as_str().unwrap_or("Loaded unpacked Chrome extension").to_string();
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

                let id = format!("ext_{}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0));
                let ext = crate::config::ExtensionConfig {
                    id,
                    name,
                    version,
                    description,
                    enabled: true,
                    path: Some(path_str.to_string()),
                    matches,
                    content_scripts: scripts,
                    content_css: styles,
                };
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
            // A new document gets a new site icon; don't carry the previous
            // site's favicon until the bridge finds a declared one.
            tab.favicon_url = get_favicon_url(&canonical);
            tab.favicon = get_default_favicon(&canonical);
            if canonical.starts_with("eclipse://") {
                tab.title = title_for_url(&canonical);
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

    pub fn handle_download_started(&mut self, file_name: String) {
        eprintln!("[DEBUG_DOWNLOAD] started: {file_name}");
        self.show_toast(&format!("Downloading {file_name}…"));
    }

    pub fn handle_download_completed(&mut self, file_name: String, success: bool) {
        if success {
            self.show_toast(&format!("Downloaded {file_name}"));
        } else {
            self.show_toast(&format!("Download failed: {file_name}"));
        }
    }

    pub fn handle_menu_action(&mut self, action: &str) {
        match action {
            "new_tab" => {
                self.open_command_bar();
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
        let sidebar_bounds = self.sidebar_bounds();
        let _ = self.sidebar_webview.set_bounds(sidebar_bounds);

        if self.config.sidebar_collapsed {
            if self.sidebar_hover_expanded {
                let _ = self.sidebar_webview.set_visible(true);
                self.bring_sidebar_to_front();
            }
        } else {
            let _ = self.sidebar_webview.set_visible(true);
        }

        let content_bounds = self.content_bounds();
        if let Some(tab) = self.tabs.iter().find(|tab| tab.id == self.active_tab_id) {
            tab.animate_bounds(content_bounds, duration);
        }
        self.broadcast_state();
    }

    pub fn resize(&mut self, width: f64, height: f64) {
        self.window_size = (width, height);
        self.update_layout();
    }

    /// Mark the chrome as needing a repaint. Actual IPC is coalesced to one
    /// update per display frame in `pump_state`, so a chatty page cannot flood
    /// the sidebar with JSON serialisation work.
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
        };

        if let Ok(json) = serde_json::to_string(&ui_state) {
            let script = format!("window.updateBrowserState && window.updateBrowserState({json});");
            let _ = self.sidebar_webview.evaluate_script(&script);

            if let Some(tab) = active_tab
                && tab.url == "eclipse://newtab"
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

    /// Smoke-test probe: proves the sidebar bootstrap and the first state push
    /// actually reached the UI. Only runs when ECLIPSE_PERF is set.
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
        return format!("https://www.youtube.com{}", &trimmed["https://youtube.com".len()..]);
    }
    if lower.starts_with("http://youtube.com/") {
        return format!("https://www.youtube.com{}", &trimmed["http://youtube.com".len()..]);
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
            return format!("https://www.youtube.com{}", &trimmed["youtube.com".len()..]);
        }
        if trimmed.contains('.') && !lower.starts_with("file:") && !lower.starts_with("javascript:")
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
        assert!(normalize_url("hello world", &config()).starts_with("https://duckduckgo.com/?q="));
        assert!(
            normalize_url("javascript:alert(1)", &config()).starts_with("https://duckduckgo.com")
        );
        assert!(
            normalize_url("file:///etc/passwd", &config()).starts_with("https://duckduckgo.com")
        );
    }

    #[test]
    fn test_content_bounds_collapsed_and_expanded() {
        let mut cfg = BrowserConfig {
            sidebar_collapsed: true,
            sidebar_width: 240.0,
            ..Default::default()
        };

        // When collapsed, content stays full width regardless of hover overlay
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

        // When pinned/docked (not collapsed), content starts after sidebar
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
}
