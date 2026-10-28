use crate::config::{BrowserConfig, DownloadItem, EaselItem, FavoriteShortcut, NoteItem, SpaceProfile};
use serde::{Deserialize, Serialize};

#[derive(Debug)]
pub enum UserEvent {
    Ipc(String),
    /// `origin` is the URL of the frame that posted the message, as reported by
    /// WebKit (not by the page). It decides which commands the sender may run.
    TabIpc { tab_id: usize, msg: String, origin: String },
    PageStarted { tab_id: usize, url: String },
    PageFinished { tab_id: usize, url: String },
    TitleChanged { tab_id: usize, title: String },
    NewWindowRequested { url: String },
    DownloadStarted {
        file_name: String,
        url: String,
        path: String,
    },
    DownloadCompleted {
        url: String,
        file_name: String,
        path: Option<String>,
        success: bool,
    },
    MenuAction(String),
    /// A navigation in a tab failed (provisional or after commit).
    NavigationFailed {
        tab_id: usize,
        code: i64,
        description: String,
        url: String,
    },
    /// The WebContent process of a tab crashed or was killed.
    WebContentTerminated { tab_id: usize },
    TickMemory,
    HoverExpandSidebar,
    HoverCollapseSidebar,
    #[allow(dead_code)]
    FinishSidebarCollapse(u64),
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
    /// Partial settings patch: only the keys the user just changed.
    UpdateSettings {
        settings: serde_json::Map<String, serde_json::Value>,
    },
    /// The sidebar's command bar was dismissed.
    CommandBarClosed,
    /// The sidebar has keyboard focus in a text field or an open popover, so
    /// hover-collapse must wait.
    SidebarInteraction {
        active: bool,
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
    #[serde(alias = "tick_memory")]
    CleanMemory,
    HibernateTab {
        id: usize,
    },
    UpdateConfig {
        config: BrowserConfig,
    },
    ToggleSiteAdblock {
        domain: String,
        enabled: bool,
    },
    InstallExtension {
        path: String,
    },
    InstallFromWebStore {
        input: String,
    },
    ToggleExtension {
        id: String,
        enabled: bool,
    },
    DeleteExtension {
        id: String,
    },
    SaveFavorite {
        index: Option<usize>,
        title: String,
        url: String,
    },
    DeleteFavorite {
        index: usize,
    },
    ReorderTab {
        from_id: usize,
        to_id: usize,
        #[serde(default)]
        before: bool,
    },
    ReorderFavorite {
        from_index: usize,
        to_index: usize,
    },
    TidyTabs,
    OpenDownload {
        id: usize,
    },
    RevealDownload {
        id: usize,
    },
    OpenDownloadsFolder,
    ClearDownloads,
    SaveNote {
        id: Option<String>,
        title: String,
        content: String,
        workspace: Option<String>,
    },
    DeleteNote {
        id: String,
    },
    OpenNotesTab,
    SaveEasel {
        id: Option<String>,
        title: String,
        elements_json: String,
        workspace: Option<String>,
    },
    DeleteEasel {
        id: String,
    },
    OpenEaselTab {
        #[serde(default)]
        #[allow(dead_code)]
        id: Option<String>,
    },
    UpdateSpaceProfile {
        profile: SpaceProfile,
    },
    OpenLittleArc {
        url: String,
    },
    PromoteLittleArc {
        url: String,
    },
    CloseLittleArc,
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
    pub group: Option<String>,
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
    pub downloads: Vec<DownloadItem>,
    pub notes: Vec<NoteItem>,
    pub easels: Vec<EaselItem>,
    pub space_profiles: Vec<SpaceProfile>,
    pub active_little_arc_url: Option<String>,
}
