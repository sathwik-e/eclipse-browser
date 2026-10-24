use crate::config::{BrowserConfig, FavoriteShortcut};
use serde::{Deserialize, Serialize};

#[derive(Debug)]
pub enum UserEvent {
    Ipc(String),
    TabIpc { tab_id: usize, msg: String },
    PageStarted { tab_id: usize, url: String },
    PageFinished { tab_id: usize, url: String },
    TitleChanged { tab_id: usize, title: String },
    NewWindowRequested { url: String },
    DownloadStarted { file_name: String },
    DownloadCompleted { file_name: String, success: bool },
    MenuAction(String),
    TickMemory,
    HoverExpandSidebar,
    HoverCollapseSidebar,
    FinishSidebarCollapse,
    CloseWindow,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum IpcCommand {
    Navigate {
        url: String,
    },
    NewTab {
        url: Option<String>,
        #[serde(default)]
        background: Option<bool>,
    },
    SelectTab {
        id: usize,
    },
    SelectTabIndex {
        index: usize,
    },
    SelectNextTab,
    SelectPrevTab,
    CloseTab {
        id: Option<usize>,
    },
    ReopenClosedTab,
    GoBack,
    GoForward,
    Reload,
    StopLoading,
    ZoomIn,
    ZoomOut,
    ZoomReset,
    Print,
    FavoriteTab,
    ToggleSidebar,
    HoverExpandSidebar,
    HoverCollapseSidebar,
    FocusOmnibar,
    OpenCommandBar,
    SetFullscreen {
        fullscreen: bool,
    },
    NavigateOrNewTab {
        url: String,
    },
    CommandBarOpenUrl {
        url: Option<String>,
        tab_id: Option<usize>,
        command: Option<String>,
    },
    OpenSettings,
    OpenSettingsTab,
    TogglePin {
        id: usize,
    },
    #[serde(rename = "open_favorite")]
    OpenFavorite {
        index: usize,
    },
    #[serde(alias = "switch_workspace")]
    SetWorkspace {
        workspace: String,
    },
    CleanMemory,
    HibernateTab {
        id: usize,
    },
    UpdateConfig {
        config: BrowserConfig,
    },
    SaveFavorite {
        index: Option<usize>,
        title: String,
        url: String,
    },
    DeleteFavorite {
        index: usize,
    },
    UpdateScroll {
        x: f64,
        y: f64,
    },
    UpdateFavicon {
        favicon: String,
    },
    SetAudible {
        audible: bool,
    },
    ChromeError {
        message: String,
    },
    UrlChanged {
        url: String,
    },
    WindowClose,
    WindowMinimize,
    WindowZoom,
    DragWindow,
    OpenDevtools,
    Ready,
}

#[derive(Debug, Clone, Serialize)]
pub struct TabState {
    pub id: usize,
    pub title: String,
    pub url: String,
    pub favicon: String,
    pub favicon_url: Option<String>,
    pub is_loading: bool,
    pub is_pinned: bool,
    pub is_hibernated: bool,
    pub is_audible: bool,
    pub zoom: f64,
    pub workspace: String,
    pub favorite_index: Option<usize>,
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
    pub sidebar_hover_expanded: bool,
    pub current_workspace: String,
    pub memory_mb: f64,
    pub config: BrowserConfig,
    pub favorites: Vec<FavoriteShortcut>,
}
