use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

fn default_true() -> bool {
    true
}

fn default_theme() -> String {
    "eclipse".into()
}

fn default_accent() -> String {
    "#5E9EFF".into()
}

fn default_sidebar_width() -> f64 {
    232.0
}

fn default_search_engine() -> String {
    "duckduckgo".into()
}

fn default_user_agent() -> String {
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.0 Safari/605.1.15".into()
}

fn default_hibernation_mins() -> u32 {
    15
}

fn default_workspace_name() -> String {
    "Default".into()
}

fn default_rounded_corners_radius() -> f64 {
    12.0
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FavoriteShortcut {
    pub title: String,
    pub url: String,
    #[serde(default)]
    pub icon: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DownloadItem {
    pub id: usize,
    pub file_name: String,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub timestamp: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NoteItem {
    pub id: String,
    pub title: String,
    pub content: String,
    #[serde(default = "default_workspace_name")]
    pub workspace: String,
    #[serde(default)]
    pub updated_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EaselItem {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub elements_json: String,
    #[serde(default = "default_workspace_name")]
    pub workspace: String,
    #[serde(default)]
    pub updated_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SpaceProfile {
    pub workspace: String,
    pub name: String,
    #[serde(default)]
    pub profile_name: String,
    #[serde(default)]
    pub icon: String,
    #[serde(default = "default_theme")]
    pub theme: String,
    #[serde(default = "default_accent")]
    pub accent_color: String,
    #[serde(default = "default_true")]
    pub adblock_enabled: bool,
    #[serde(default)]
    pub custom_user_agent: String,
}

fn default_notes() -> Vec<NoteItem> {
    vec![NoteItem {
        id: "note_welcome".into(),
        title: "Welcome to Eclipse Notes".into(),
        content: "Capture quick thoughts, code snippets, or research links right from your sidebar or inside eclipse://notes.\n\n• Notes automatically sync with your active Space\n• Use Cmd+Shift+N or the sidebar Notes icon anytime".into(),
        workspace: "Default".into(),
        updated_at: 1700000000,
    }]
}

fn default_easels() -> Vec<EaselItem> {
    vec![EaselItem {
        id: "easel_welcome".into(),
        title: "Project Moodboard & Scratchpad".into(),
        elements_json: r##"[{"id":"card_1","type":"sticky","x":48,"y":52,"w":230,"h":150,"color":"#1E293B","accent":"#5E9EFF","text":"✨ Eclipse Easel\nDrag cards, sketch ideas, or drop links onto your infinite canvas."},{"id":"card_2","type":"sticky","x":310,"y":64,"w":220,"h":150,"color":"#1E1B4B","accent":"#A78BFA","text":"🚀 Shortcuts\n• Add Sticky Note\n• Add Link Card\n• Freehand Draw Mode"}]"##.into(),
        workspace: "Default".into(),
        updated_at: 1700000000,
    }]
}

pub fn default_space_profiles() -> Vec<SpaceProfile> {
    vec![
        SpaceProfile {
            workspace: "Default".into(),
            name: "Work".into(),
            profile_name: "Work Profile".into(),
            icon: "briefcase".into(),
            theme: "eclipse".into(),
            accent_color: "#5E9EFF".into(),
            adblock_enabled: true,
            custom_user_agent: String::new(),
        },
        SpaceProfile {
            workspace: "Media".into(),
            name: "Media".into(),
            profile_name: "Cinema & Streaming".into(),
            icon: "play".into(),
            theme: "void".into(),
            accent_color: "#F472B6".into(),
            adblock_enabled: true,
            custom_user_agent: String::new(),
        },
        SpaceProfile {
            workspace: "Dev".into(),
            name: "Dev".into(),
            profile_name: "Engineering Sandbox".into(),
            icon: "code".into(),
            theme: "lunar".into(),
            accent_color: "#34D399".into(),
            adblock_enabled: true,
            custom_user_agent: String::new(),
        },
        SpaceProfile {
            workspace: "Personal".into(),
            name: "Personal".into(),
            profile_name: "Personal & Social".into(),
            icon: "sparkles".into(),
            theme: "nebula".into(),
            accent_color: "#A78BFA".into(),
            adblock_enabled: true,
            custom_user_agent: String::new(),
        },
    ]
}

fn default_favorites() -> Vec<FavoriteShortcut> {
    vec![
        FavoriteShortcut {
            title: "Gmail".into(),
            url: "https://mail.google.com".into(),
            icon: "gmail".into(),
        },
        FavoriteShortcut {
            title: "Netflix".into(),
            url: "https://www.netflix.com".into(),
            icon: "netflix".into(),
        },
        FavoriteShortcut {
            title: "YouTube".into(),
            url: "https://www.youtube.com".into(),
            icon: "youtube".into(),
        },
        FavoriteShortcut {
            title: "Remotasks".into(),
            url: "https://www.remotasks.com".into(),
            icon: "remotasks".into(),
        },
        FavoriteShortcut {
            title: "Outlook".into(),
            url: "https://outlook.live.com".into(),
            icon: "outlook".into(),
        },
        FavoriteShortcut {
            title: "Instagram".into(),
            url: "https://www.instagram.com".into(),
            icon: "instagram".into(),
        },
        FavoriteShortcut {
            title: "GitHub".into(),
            url: "https://github.com".into(),
            icon: "github".into(),
        },
        FavoriteShortcut {
            title: "Apple".into(),
            url: "https://www.apple.com".into(),
            icon: "apple".into(),
        },
        FavoriteShortcut {
            title: "Discord".into(),
            url: "https://discord.com".into(),
            icon: "chat".into(),
        },
    ]
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ExtensionConfig {
    pub id: String,
    pub name: String,
    pub version: String,
    pub description: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub matches: Vec<String>,
    #[serde(default)]
    pub content_scripts: Vec<String>,
    #[serde(default)]
    pub content_css: Vec<String>,
}

fn default_builtin_extensions() -> Vec<ExtensionConfig> {
    Vec::new()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BrowserConfig {
    #[serde(default = "default_theme")]
    pub theme: String,
    #[serde(default = "default_accent")]
    pub accent_color: String,
    #[serde(default = "default_sidebar_width")]
    pub sidebar_width: f64,
    #[serde(default = "default_true")]
    pub sidebar_collapsed: bool,
    #[serde(default = "default_search_engine")]
    pub search_engine: String,
    #[serde(default = "default_user_agent")]
    pub user_agent: String,
    #[serde(default = "default_true")]
    pub drm_fairplay_enabled: bool,
    #[serde(default = "default_true")]
    pub autoplay_enabled: bool,
    #[serde(default = "default_hibernation_mins")]
    pub tab_hibernation_mins: u32,
    #[serde(default = "default_true")]
    pub adblock_enabled: bool,
    #[serde(default)]
    pub adblock_disabled_domains: Vec<String>,
    #[serde(default = "default_true")]
    pub webextensions_enabled: bool,
    #[serde(default = "default_builtin_extensions")]
    pub installed_extensions: Vec<ExtensionConfig>,
    #[serde(default)]
    pub custom_js: String,
    #[serde(default)]
    pub custom_css: String,
    #[serde(default)]
    pub custom_user_agent: String,
    #[serde(default = "default_favorites")]
    pub favorites: Vec<FavoriteShortcut>,
    #[serde(default = "default_notes")]
    pub notes: Vec<NoteItem>,
    #[serde(default = "default_easels")]
    pub easels: Vec<EaselItem>,
    #[serde(default = "default_space_profiles")]
    pub space_profiles: Vec<SpaceProfile>,
    #[serde(default)]
    pub downloads: Vec<DownloadItem>,
    #[serde(default = "default_rounded_corners_radius")]
    pub rounded_corners_radius: f64,
}

impl Default for BrowserConfig {
    fn default() -> Self {
        Self {
            theme: default_theme(),
            accent_color: default_accent(),
            sidebar_width: default_sidebar_width(),
            sidebar_collapsed: true,
            search_engine: default_search_engine(),
            user_agent: default_user_agent(),
            drm_fairplay_enabled: true,
            autoplay_enabled: true,
            tab_hibernation_mins: default_hibernation_mins(),
            adblock_enabled: true,
            adblock_disabled_domains: Vec::new(),
            webextensions_enabled: true,
            installed_extensions: default_builtin_extensions(),
            custom_js: String::new(),
            custom_css: String::new(),
            custom_user_agent: String::new(),
            favorites: default_favorites(),
            notes: default_notes(),
            easels: default_easels(),
            space_profiles: default_space_profiles(),
            downloads: Vec::new(),
            rounded_corners_radius: default_rounded_corners_radius(),
        }
    }
}

impl BrowserConfig {
    pub fn config_path() -> PathBuf {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
        let mut path = PathBuf::from(home);
        path.push(".config");
        path.push("eclipse-browser");
        let _ = fs::create_dir_all(&path);
        path.push("config.json");
        path
    }

    pub fn load() -> Self {
        let path = Self::config_path();
        if let Ok(content) = fs::read_to_string(&path) {
            match serde_json::from_str::<Self>(&content) {
                Ok(mut config) => {
                    if config.sidebar_width < 200.0 || config.sidebar_width > 248.0 {
                        config.sidebar_width = default_sidebar_width();
                    }
                    if config.user_agent.trim().is_empty() {
                        config.user_agent = default_user_agent();
                    }
                    if config.favorites.len() < 3 {
                        for d in default_favorites() {
                            if !config.favorites.iter().any(|f| f.url == d.url) {
                                config.favorites.push(d);
                                if config.favorites.len() >= 6 {
                                    break;
                                }
                            }
                        }
                    }
                    if config.space_profiles.is_empty() {
                        config.space_profiles = default_space_profiles();
                    }
                    if config.notes.is_empty() {
                        config.notes = default_notes();
                    }
                    if config.easels.is_empty() {
                        config.easels = default_easels();
                    }
                    if config.rounded_corners_radius < 0.0 || config.rounded_corners_radius > 24.0 {
                        config.rounded_corners_radius = default_rounded_corners_radius();
                    }
                    config.installed_extensions.retain(|e| {
                        e.id != "sponsorblock"
                            && e.id != "return-dislike"
                            && e.id != "dark-reader"
                            && e.id != "volume-booster"
                    });
                    return config;
                }
                Err(err) => {
                    eprintln!("[eclipse] ignoring unreadable config at {path:?}: {err}");
                }
            }
        }
        Self::default()
    }

    /// Persist config atomically so a crash during save can never truncate the
    /// user's settings file.
    pub fn save(&self) {
        let path = Self::config_path();
        let Ok(json) = serde_json::to_string_pretty(self) else {
            return;
        };
        let tmp = path.with_extension("json.tmp");
        if fs::write(&tmp, json).is_ok() && fs::rename(&tmp, &path).is_err() {
            let _ = fs::remove_file(&tmp);
        }
    }

    /// The config as internal eclipse:// pages may see it: extension source
    /// code is stripped (pages only need names, ids and switches).
    pub fn to_page_json(&self) -> String {
        let mut page = self.clone();
        for ext in &mut page.installed_extensions {
            ext.content_scripts.clear();
            ext.content_css.clear();
        }
        serde_json::to_string(&page).unwrap_or_else(|_| "{}".into())
    }

    /// Keys of `BrowserConfig` that `update_settings` patches may change.
    /// Collections (favorites, notes, easels, downloads, extensions, space
    /// profiles) have dedicated commands and are never patched wholesale.
    pub const PATCHABLE_SETTINGS: &'static [&'static str] = &[
        "theme",
        "accent_color",
        "sidebar_width",
        "sidebar_collapsed",
        "search_engine",
        "user_agent",
        "drm_fairplay_enabled",
        "autoplay_enabled",
        "tab_hibernation_mins",
        "adblock_enabled",
        "adblock_disabled_domains",
        "webextensions_enabled",
        "custom_js",
        "custom_css",
        "custom_user_agent",
        "rounded_corners_radius",
    ];

    pub fn get_search_url(&self, query: &str) -> String {
        let encoded = urlencode(query);
        match self.search_engine.as_str() {
            "google" => format!("https://www.google.com/search?q={encoded}"),
            "brave" => format!("https://search.brave.com/search?q={encoded}"),
            "ecosia" => format!("https://www.ecosia.org/search?q={encoded}"),
            "kagi" => format!("https://kagi.com/search?q={encoded}"),
            _ => format!("https://duckduckgo.com/?q={encoded}"),
        }
    }

    pub fn active_user_agent(&self) -> &str {
        let custom = self.custom_user_agent.trim();
        if custom.is_empty() {
            &self.user_agent
        } else {
            custom
        }
    }

    pub fn profile_for_workspace(&self, workspace: &str) -> SpaceProfile {
        let normalized = if workspace == "Work" { "Default" } else { workspace };
        self.space_profiles
            .iter()
            .find(|p| p.workspace.eq_ignore_ascii_case(normalized) || p.name.eq_ignore_ascii_case(workspace))
            .cloned()
            .unwrap_or_else(|| SpaceProfile {
                workspace: normalized.to_string(),
                name: if normalized == "Default" { "Work".into() } else { normalized.to_string() },
                profile_name: format!("{normalized} Profile"),
                icon: "briefcase".into(),
                theme: self.theme.clone(),
                accent_color: self.accent_color.clone(),
                adblock_enabled: self.adblock_enabled,
                custom_user_agent: self.custom_user_agent.clone(),
            })
    }
}

fn urlencode(input: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut encoded = String::with_capacity(input.len());
    for byte in input.bytes() {
        match byte {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                encoded.push(byte as char);
            }
            b' ' => encoded.push('+'),
            _ => {
                encoded.push('%');
                encoded.push(HEX[(byte >> 4) as usize] as char);
                encoded.push(HEX[(byte & 0x0F) as usize] as char);
            }
        }
    }
    encoded
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_search_queries() {
        assert_eq!(urlencode("hello world"), "hello+world");
        assert_eq!(urlencode("a&b=c/d"), "a%26b%3Dc%2Fd");
        assert_eq!(urlencode("ümlaut"), "%C3%BCmlaut");
    }

    #[test]
    fn default_config_is_sane() {
        let config = BrowserConfig::default();
        assert_eq!(config.search_engine, "duckduckgo");
        assert!(config.adblock_enabled);
        assert_eq!(config.favorites.len(), 9);
        assert_eq!(config.sidebar_width, 232.0);
        assert!(config.active_user_agent().contains("Safari"));
        assert_eq!(config.space_profiles.len(), 4);
        assert!(!config.notes.is_empty());
        assert!(!config.easels.is_empty());
    }

    #[test]
    fn missing_fields_are_filled_in() {
        let config: BrowserConfig = serde_json::from_str(r#"{"custom_css":"body{}"}"#).unwrap();
        assert_eq!(config.theme, "eclipse");
        assert_eq!(config.tab_hibernation_mins, 15);
        assert!(!config.favorites.is_empty());
        assert_eq!(config.space_profiles.len(), 4);
    }

    #[test]
    fn an_explicitly_empty_favorites_list_is_kept() {
        let config: BrowserConfig = serde_json::from_str(r#"{"favorites":[]}"#).unwrap();
        assert!(config.favorites.is_empty());
    }
}
