use crate::config::BrowserConfig;
use serde::{Deserialize, Serialize};

#[derive(Debug)]
pub enum UserEvent {
    Ipc(String),
    PageStarted { tab_id: usize, url: String },
    PageFinished { tab_id: usize, url: String },
    TitleChanged { tab_id: usize, title: String },
    NewWindowRequested { url: String },
    TickMemory,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum IpcCommand {
    Navigate { url: String },
    NewTab { url: Option<String> },
    SelectTab { id: usize },
    CloseTab { id: usize },
    GoBack,
    GoForward,
    Reload,
    ToggleSidebar,
    TogglePin { id: usize },
    SetWorkspace { workspace: String },
    HibernateTab { id: usize },
    UpdateConfig { config: BrowserConfig },
    OpenDevtools,
    Ready,
}

#[derive(Debug, Clone, Serialize)]
pub struct TabState {
    pub id: usize,
    pub title: String,
    pub url: String,
    pub favicon: String,
    pub is_loading: bool,
    pub is_pinned: bool,
    pub is_hibernated: bool,
    pub workspace: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct BrowserUiState {
    pub tabs: Vec<TabState>,
    pub active_tab_id: usize,
    pub active_url: String,
    pub active_title: String,
    pub can_go_back: bool,
    pub can_go_forward: bool,
    pub is_loading: bool,
    pub sidebar_collapsed: bool,
    pub current_workspace: String,
    pub memory_mb: f64,
    pub config: BrowserConfig,
}
