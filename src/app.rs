use std::sync::Arc;
use std::time::{Duration, Instant};
use tao::dpi::{LogicalPosition, LogicalSize, Position, Size};
use tao::event_loop::EventLoopProxy;
use tao::window::Window;
use wry::{Rect, WebView, WebViewBuilder};

use crate::config::BrowserConfig;
use crate::events::{BrowserUiState, IpcCommand, UserEvent};
use crate::tab::{get_default_favicon, Tab};
use crate::ui_assets::SIDEBAR_HTML;

pub struct BrowserApp {
    pub window: Arc<Window>,
    pub proxy: EventLoopProxy<UserEvent>,
    pub sidebar_webview: WebView,
    pub tabs: Vec<Tab>,
    pub active_tab_id: usize,
    pub next_tab_id: usize,
    pub config: BrowserConfig,
    pub current_workspace: String,
    pub window_size: (f64, f64),
}

impl BrowserApp {
    pub fn new(
        window: Arc<Window>,
        proxy: EventLoopProxy<UserEvent>,
        initial_size: (f64, f64),
    ) -> Self {
        let config = BrowserConfig::load();
        let sidebar_w = if config.sidebar_collapsed {
            68.0
        } else {
            config.sidebar_width
        };

        let sidebar_bounds = Rect {
            position: Position::Logical(LogicalPosition::new(0.0, 0.0)),
            size: Size::Logical(LogicalSize::new(sidebar_w, initial_size.1)),
        };

        let proxy_clone = proxy.clone();
        let sidebar_webview = WebViewBuilder::new()
            .with_bounds(sidebar_bounds)
            .with_transparent(true)
            .with_devtools(true)
            .with_html(SIDEBAR_HTML)
            .with_ipc_handler(move |req| {
                let body = req.body().clone();
                let _ = proxy_clone.send_event(UserEvent::Ipc(body));
            })
            .build(&window)
            .expect("Failed to initialize sidebar webview");

        let mut app = Self {
            window: window.clone(),
            proxy: proxy.clone(),
            sidebar_webview,
            tabs: Vec::new(),
            active_tab_id: 1,
            next_tab_id: 1,
            config,
            current_workspace: "Default".into(),
            window_size: initial_size,
        };

        // Create first tab
        app.create_tab("eclipse://newtab", true);
        app.broadcast_state();
        app
    }

    pub fn content_bounds(&self) -> Rect {
        let sidebar_w = if self.config.sidebar_collapsed {
            68.0
        } else {
            self.config.sidebar_width
        };

        let content_w = (self.window_size.0 - sidebar_w).max(100.0);
        let content_h = self.window_size.1.max(100.0);

        Rect {
            position: Position::Logical(LogicalPosition::new(sidebar_w, 0.0)),
            size: Size::Logical(LogicalSize::new(content_w, content_h)),
        }
    }

    pub fn sidebar_bounds(&self) -> Rect {
        let sidebar_w = if self.config.sidebar_collapsed {
            68.0
        } else {
            self.config.sidebar_width
        };

        Rect {
            position: Position::Logical(LogicalPosition::new(0.0, 0.0)),
            size: Size::Logical(LogicalSize::new(sidebar_w, self.window_size.1)),
        }
    }

    pub fn create_tab(&mut self, url: &str, select: bool) -> usize {
        let id = self.next_tab_id;
        self.next_tab_id += 1;

        let bounds = self.content_bounds();
        let tab = Tab::new(
            id,
            url,
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
            if let Some(tab) = self.tabs.iter().find(|t| t.id == id) {
                tab.set_visible(false);
            }
        }

        id
    }

    pub fn select_tab(&mut self, id: usize) {
        let bounds = self.content_bounds();
        for tab in &mut self.tabs {
            if tab.id == id {
                tab.restore_if_needed(&self.window, self.proxy.clone(), bounds, &self.config);
                tab.set_bounds(bounds);
                tab.set_visible(true);
            } else {
                tab.set_visible(false);
            }
        }
        self.active_tab_id = id;
        self.broadcast_state();
    }

    pub fn close_tab(&mut self, id: usize) {
        let idx = self.tabs.iter().position(|t| t.id == id);
        if let Some(pos) = idx {
            // Remove tab and drop its webview
            self.tabs.remove(pos);

            if self.tabs.is_empty() {
                self.create_tab("eclipse://newtab", true);
            } else if self.active_tab_id == id {
                let new_active_id = if pos < self.tabs.len() {
                    self.tabs[pos].id
                } else {
                    self.tabs[pos - 1].id
                };
                self.select_tab(new_active_id);
            } else {
                self.broadcast_state();
            }
        }
    }

    pub fn navigate_active(&mut self, input: &str) {
        let url = normalize_url(input, &self.config);
        if let Some(tab) = self.tabs.iter_mut().find(|t| t.id == self.active_tab_id) {
            tab.load_url(&url);
            tab.title = "Loading...".into();
            tab.favicon = get_default_favicon(&url);
            tab.is_loading = true;
        }
        self.broadcast_state();
    }

    pub fn handle_ipc(&mut self, raw: &str) {
        if let Ok(cmd) = serde_json::from_str::<IpcCommand>(raw) {
            match cmd {
                IpcCommand::Navigate { url } => self.navigate_active(&url),
                IpcCommand::NewTab { url } => {
                    let target = url.unwrap_or_else(|| "eclipse://newtab".into());
                    self.create_tab(&target, true);
                }
                IpcCommand::SelectTab { id } => self.select_tab(id),
                IpcCommand::CloseTab { id } => self.close_tab(id),
                IpcCommand::GoBack => {
                    if let Some(tab) = self.tabs.iter().find(|t| t.id == self.active_tab_id) {
                        tab.go_back();
                    }
                }
                IpcCommand::GoForward => {
                    if let Some(tab) = self.tabs.iter().find(|t| t.id == self.active_tab_id) {
                        tab.go_forward();
                    }
                }
                IpcCommand::Reload => {
                    if let Some(tab) = self.tabs.iter().find(|t| t.id == self.active_tab_id) {
                        tab.reload();
                    }
                }
                IpcCommand::ToggleSidebar => {
                    self.config.sidebar_collapsed = !self.config.sidebar_collapsed;
                    self.config.save();
                    self.update_layout();
                }
                IpcCommand::TogglePin { id } => {
                    if let Some(tab) = self.tabs.iter_mut().find(|t| t.id == id) {
                        tab.is_pinned = !tab.is_pinned;
                    }
                    self.broadcast_state();
                }
                IpcCommand::SetWorkspace { workspace } => {
                    self.current_workspace = workspace;
                    self.broadcast_state();
                }
                IpcCommand::HibernateTab { id } => {
                    if id != self.active_tab_id {
                        if let Some(tab) = self.tabs.iter_mut().find(|t| t.id == id) {
                            tab.hibernate();
                        }
                        self.broadcast_state();
                    }
                }
                IpcCommand::UpdateConfig { config } => {
                    self.config = config;
                    self.config.save();
                    self.update_layout();
                }
                IpcCommand::OpenDevtools => {
                    if let Some(tab) = self.tabs.iter().find(|t| t.id == self.active_tab_id) {
                        if let Some(ref wv) = tab.webview {
                            let _ = wv.evaluate_script("console.log('Zenith DevTools');");
                        }
                    }
                }
            }
        }
    }

    pub fn handle_page_started(&mut self, tab_id: usize, url: String) {
        if let Some(tab) = self.tabs.iter_mut().find(|t| t.id == tab_id) {
            tab.url = url.clone();
            tab.is_loading = true;
            tab.favicon = get_default_favicon(&url);
        }
        self.broadcast_state();
    }

    pub fn handle_page_finished(&mut self, tab_id: usize, url: String) {
        if let Some(tab) = self.tabs.iter_mut().find(|t| t.id == tab_id) {
            tab.url = url.clone();
            tab.is_loading = false;
            tab.favicon = get_default_favicon(&url);
        }
        self.broadcast_state();
    }

    pub fn handle_title_changed(&mut self, tab_id: usize, title: String) {
        if let Some(tab) = self.tabs.iter_mut().find(|t| t.id == tab_id) {
            if !title.is_empty() {
                tab.title = title;
            }
        }
        self.broadcast_state();
    }

    pub fn update_layout(&mut self) {
        let s_bounds = self.sidebar_bounds();
        let _ = self.sidebar_webview.set_bounds(s_bounds);

        let c_bounds = self.content_bounds();
        if let Some(tab) = self.tabs.iter().find(|t| t.id == self.active_tab_id) {
            tab.set_bounds(c_bounds);
        }
        self.broadcast_state();
    }

    pub fn resize(&mut self, width: f64, height: f64) {
        self.window_size = (width, height);
        self.update_layout();
    }

    pub fn check_hibernation(&mut self) {
        let mins = self.config.tab_hibernation_mins;
        if mins == 0 {
            return;
        }

        let threshold = Duration::from_secs((mins as u64) * 60);
        let now = Instant::now();
        let active_id = self.active_tab_id;

        let mut hibernated_any = false;
        for tab in &mut self.tabs {
            if tab.id != active_id && !tab.is_hibernated && tab.webview.is_some() {
                if now.duration_since(tab.last_active) > threshold {
                    tab.hibernate();
                    hibernated_any = true;
                }
            }
        }

        if hibernated_any {
            self.broadcast_state();
        }
    }

    pub fn broadcast_state(&self) {
        let active_tab = self.tabs.iter().find(|t| t.id == self.active_tab_id);
        let active_url = active_tab.map(|t| t.url.clone()).unwrap_or_default();
        let active_title = active_tab.map(|t| t.title.clone()).unwrap_or_default();
        let is_loading = active_tab.map(|t| t.is_loading).unwrap_or(false);

        let ui_state = BrowserUiState {
            tabs: self.tabs.iter().map(|t| t.to_state()).collect(),
            active_tab_id: self.active_tab_id,
            active_url,
            active_title,
            can_go_back: true,
            can_go_forward: true,
            is_loading,
            sidebar_collapsed: self.config.sidebar_collapsed,
            current_workspace: self.current_workspace.clone(),
            memory_mb: get_process_memory_mb(),
            config: self.config.clone(),
        };

        if let Ok(json) = serde_json::to_string(&ui_state) {
            let script = format!("window.updateBrowserState && window.updateBrowserState({});", json);
            let _ = self.sidebar_webview.evaluate_script(&script);
        }
    }
}

pub fn normalize_url(input: &str, config: &BrowserConfig) -> String {
    let trimmed = input.trim();
    if trimmed.starts_with("eclipse://") {
        return trimmed.to_string();
    }
    if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        return trimmed.to_string();
    }
    // Check if input looks like a domain name
    if !trimmed.contains(' ') && (trimmed.contains('.') || trimmed.starts_with("localhost")) {
        return format!("https://{}", trimmed);
    }
    // Fall back to search engine
    config.get_search_url(trimmed)
}

pub fn get_process_memory_mb() -> f64 {
    #[cfg(target_os = "macos")]
    unsafe {
        let mut info = std::mem::MaybeUninit::<libc::mach_task_basic_info>::uninit();
        let mut count = (std::mem::size_of::<libc::mach_task_basic_info>() / std::mem::size_of::<libc::natural_t>()) as libc::mach_msg_type_number_t;
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
