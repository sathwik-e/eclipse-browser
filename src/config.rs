use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub const MIN_SIDEBAR_WIDTH: f64 = 200.0;
pub const MAX_SIDEBAR_WIDTH: f64 = 248.0;
const MAX_ROUNDED_CORNERS_RADIUS: f64 = 24.0;
const MAX_HIBERNATION_MINS: u64 = 24 * 60;
const MAX_USER_AGENT_LEN: usize = 512;
const MAX_CUSTOM_CODE_LEN: usize = 1024 * 1024;
const MAX_DISABLED_DOMAINS: usize = 1000;
const SEARCH_ENGINES: &[&str] = &["duckduckgo", "google", "brave", "ecosia", "kagi"];

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

fn default_zoom() -> f64 {
    1.0
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
    fn config_dir() -> PathBuf {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
        let mut path = PathBuf::from(home);
        path.push(".config");
        path.push("eclipse-browser");
        let _ = fs::create_dir_all(&path);
        path
    }

    pub fn config_path() -> PathBuf {
        Self::config_dir().join("config.json")
    }

    /// Loads the config. Defaults are only used for fields that are missing
    /// (first run) or unreadable; a single bad field never discards the rest.
    /// When anything had to be dropped, the original file is copied to
    /// `config.json.bad-<timestamp>` first and the repaired config is saved.
    pub fn load() -> Self {
        let path = Self::config_path();
        let Ok(content) = fs::read_to_string(&path) else {
            return Self::default();
        };
        let (mut config, problems) = Self::parse_lenient(&content);
        config.sanitize();
        if !problems.is_empty() {
            let backup = path.with_extension(format!("json.bad-{}", unix_now()));
            match fs::copy(&path, &backup) {
                Ok(_) => eprintln!(
                    "[eclipse] config at {path:?} had unreadable data ({}); original kept at {backup:?}",
                    problems.join("; ")
                ),
                Err(err) => eprintln!(
                    "[eclipse] config at {path:?} had unreadable data ({}); backup failed: {err}",
                    problems.join("; ")
                ),
            }
            config.save();
        }
        config
    }

    /// Parses config JSON field by field. A field (or a list entry) with the
    /// wrong type falls back to its default instead of failing the whole
    /// file. Returns the config and a description of everything dropped.
    pub fn parse_lenient(content: &str) -> (Self, Vec<String>) {
        let fields = match serde_json::from_str::<Value>(content) {
            Ok(Value::Object(fields)) => fields,
            Ok(_) => return (Self::default(), vec!["config is not a JSON object".into()]),
            Err(err) => return (Self::default(), vec![format!("invalid JSON: {err}")]),
        };
        if let Ok(config) = serde_json::from_value::<Self>(Value::Object(fields.clone())) {
            return (config, Vec::new());
        }

        // Every field has a serde default, so a one-key object tells whether
        // that key's value deserializes on its own.
        let accepts = |key: &str, value: &Value| {
            let mut probe = Map::new();
            probe.insert(key.to_string(), value.clone());
            serde_json::from_value::<Self>(Value::Object(probe)).is_ok()
        };
        let mut accepted = Map::new();
        let mut problems = Vec::new();
        for (key, value) in fields {
            if accepts(&key, &value) {
                accepted.insert(key, value);
            } else if let Value::Array(items) = value {
                let total = items.len();
                let kept: Vec<Value> = items
                    .into_iter()
                    .filter(|item| accepts(&key, &Value::Array(vec![item.clone()])))
                    .collect();
                problems.push(format!("{key}: dropped {} of {total} entries", total - kept.len()));
                accepted.insert(key, Value::Array(kept));
            } else {
                problems.push(format!("{key}: invalid value replaced by the default"));
            }
        }
        let config = serde_json::from_value(Value::Object(accepted)).unwrap_or_default();
        (config, problems)
    }

    /// Repairs out-of-range values. User-deleted favorites, notes and easels
    /// are deliberately not re-seeded.
    fn sanitize(&mut self) {
        if !(MIN_SIDEBAR_WIDTH..=MAX_SIDEBAR_WIDTH).contains(&self.sidebar_width) {
            self.sidebar_width = default_sidebar_width();
        }
        if self.user_agent.trim().is_empty() {
            self.user_agent = default_user_agent();
        }
        if self.space_profiles.is_empty() {
            self.space_profiles = default_space_profiles();
        }
        if !(0.0..=MAX_ROUNDED_CORNERS_RADIUS).contains(&self.rounded_corners_radius) {
            self.rounded_corners_radius = default_rounded_corners_radius();
        }
        self.installed_extensions.retain(|e| {
            e.id != "sponsorblock"
                && e.id != "return-dislike"
                && e.id != "dark-reader"
                && e.id != "volume-booster"
        });
        // A download that was in flight when the app quit can never complete.
        for download in &mut self.downloads {
            if download.status == "downloading" {
                download.status = "failed".into();
            }
        }
    }

    /// Persist config atomically so a crash during save can never truncate the
    /// user's settings file.
    pub fn save(&self) {
        if let Ok(json) = serde_json::to_string_pretty(self) {
            write_atomic(&Self::config_path(), &json);
        }
    }

    /// The config as internal eclipse:// pages and the sidebar may see it:
    /// extension source code is stripped (pages only need names, ids and
    /// switches).
    pub fn page_safe(&self) -> Self {
        let mut page = self.clone();
        for ext in &mut page.installed_extensions {
            ext.content_scripts.clear();
            ext.content_css.clear();
        }
        page
    }

    /// Serialized `page_safe()` config for tab.rs's internal-page injection.
    /// (Unused in this crate until tab.rs adopts it; see contract notes.)
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn to_page_json(&self) -> String {
        serde_json::to_string(&self.page_safe()).unwrap_or_else(|_| "{}".into())
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

    /// Applies an `update_settings` patch. Only keys in `PATCHABLE_SETTINGS`
    /// with a valid type and range are applied; everything else is ignored.
    /// Returns the keys whose value actually changed.
    pub fn apply_settings_patch(&mut self, patch: &Map<String, Value>) -> Vec<&'static str> {
        Self::PATCHABLE_SETTINGS
            .iter()
            .copied()
            .filter(|key| patch.get(*key).is_some_and(|value| self.apply_setting(key, value)))
            .collect()
    }

    fn apply_setting(&mut self, key: &str, value: &Value) -> bool {
        fn set<T: PartialEq>(slot: &mut T, new: T) -> bool {
            if *slot == new {
                false
            } else {
                *slot = new;
                true
            }
        }
        let text = |max: usize| value.as_str().filter(|s| s.len() <= max);
        match key {
            "theme" => text(32)
                .filter(|s| is_identifier(s))
                .is_some_and(|s| set(&mut self.theme, s.to_string())),
            "accent_color" => text(9)
                .filter(|s| is_hex_color(s))
                .is_some_and(|s| set(&mut self.accent_color, s.to_string())),
            "sidebar_width" => value
                .as_f64()
                .filter(|w| (MIN_SIDEBAR_WIDTH..=MAX_SIDEBAR_WIDTH).contains(w))
                .is_some_and(|w| set(&mut self.sidebar_width, w)),
            "sidebar_collapsed" => value.as_bool().is_some_and(|b| set(&mut self.sidebar_collapsed, b)),
            "search_engine" => text(16)
                .filter(|s| SEARCH_ENGINES.contains(s))
                .is_some_and(|s| set(&mut self.search_engine, s.to_string())),
            "user_agent" => text(MAX_USER_AGENT_LEN)
                .map(str::trim)
                .filter(|s| !s.is_empty() && is_header_safe(s))
                .is_some_and(|s| set(&mut self.user_agent, s.to_string())),
            "custom_user_agent" => text(MAX_USER_AGENT_LEN)
                .map(str::trim)
                .filter(|s| is_header_safe(s))
                .is_some_and(|s| set(&mut self.custom_user_agent, s.to_string())),
            "drm_fairplay_enabled" => value.as_bool().is_some_and(|b| set(&mut self.drm_fairplay_enabled, b)),
            "autoplay_enabled" => value.as_bool().is_some_and(|b| set(&mut self.autoplay_enabled, b)),
            "adblock_enabled" => value.as_bool().is_some_and(|b| set(&mut self.adblock_enabled, b)),
            "webextensions_enabled" => value.as_bool().is_some_and(|b| set(&mut self.webextensions_enabled, b)),
            "tab_hibernation_mins" => value
                .as_u64()
                .filter(|m| *m <= MAX_HIBERNATION_MINS)
                .is_some_and(|m| set(&mut self.tab_hibernation_mins, m as u32)),
            "adblock_disabled_domains" => {
                let Some(items) = value.as_array() else {
                    return false;
                };
                if items.len() > MAX_DISABLED_DOMAINS || !items.iter().all(Value::is_string) {
                    return false;
                }
                let mut domains: Vec<String> = Vec::new();
                for domain in items.iter().filter_map(Value::as_str).filter_map(normalize_site_domain) {
                    if !domains.contains(&domain) {
                        domains.push(domain);
                    }
                }
                set(&mut self.adblock_disabled_domains, domains)
            }
            "custom_js" => text(MAX_CUSTOM_CODE_LEN).is_some_and(|s| set(&mut self.custom_js, s.to_string())),
            "custom_css" => text(MAX_CUSTOM_CODE_LEN).is_some_and(|s| set(&mut self.custom_css, s.to_string())),
            "rounded_corners_radius" => value
                .as_f64()
                .filter(|r| (0.0..=MAX_ROUNDED_CORNERS_RADIUS).contains(r))
                .is_some_and(|r| set(&mut self.rounded_corners_radius, r)),
            _ => false,
        }
    }

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

/// One tab of the saved browsing session.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SessionTab {
    pub url: String,
    #[serde(default)]
    pub title: String,
    #[serde(default = "default_workspace_name")]
    pub workspace: String,
    #[serde(default)]
    pub is_pinned: bool,
    /// URL of the favorite this tab belongs to, so the binding survives a
    /// restart even if indices shift.
    #[serde(default)]
    pub favorite_url: Option<String>,
    #[serde(default = "default_zoom")]
    pub zoom: f64,
}

/// Open tabs and spaces, saved to `session.json` next to the config.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Session {
    #[serde(default)]
    pub tabs: Vec<SessionTab>,
    #[serde(default)]
    pub active_index: Option<usize>,
    #[serde(default = "default_workspace_name")]
    pub current_workspace: String,
}

impl Session {
    pub fn path() -> PathBuf {
        BrowserConfig::config_dir().join("session.json")
    }

    pub fn load() -> Option<Self> {
        let content = fs::read_to_string(Self::path()).ok()?;
        match serde_json::from_str(&content) {
            Ok(session) => Some(session),
            Err(err) => {
                eprintln!("[eclipse] ignoring unreadable session: {err}");
                None
            }
        }
    }
}

/// Writes through a temp file and rename so a crash mid-write can never
/// truncate the destination.
pub fn write_atomic(path: &Path, contents: &str) {
    let tmp = path.with_extension("json.tmp");
    if fs::write(&tmp, contents).is_ok() && fs::rename(&tmp, path).is_err() {
        let _ = fs::remove_file(&tmp);
    }
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Reduces user input ("https://www.Example.com:8080/a", "example.com/")
/// to a bare lowercase hostname, or `None` if it isn't one.
pub fn normalize_site_domain(input: &str) -> Option<String> {
    let raw = input.trim().to_ascii_lowercase();
    let host = if raw.contains("://") {
        url::Url::parse(&raw).ok()?.host_str()?.to_string()
    } else {
        raw.split(['/', '?', '#']).next()?.split(':').next()?.trim().to_string()
    };
    let valid = !host.is_empty()
        && host.len() <= 253
        && !host.starts_with('.')
        && !host.ends_with('.')
        && host
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'.' || b == b'-');
    valid.then_some(host)
}

fn is_identifier(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

fn is_hex_color(s: &str) -> bool {
    s.strip_prefix('#').is_some_and(|hex| {
        matches!(hex.len(), 3 | 4 | 6 | 8) && hex.bytes().all(|b| b.is_ascii_hexdigit())
    })
}

/// Safe to use as an HTTP header value (no control characters).
fn is_header_safe(s: &str) -> bool {
    !s.chars().any(char::is_control)
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
        assert_eq!(config.favorites.len(), 8);
        assert!(!config.favorites.iter().any(|f| f.url.contains("remotasks")));
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

    #[test]
    fn deleted_defaults_stay_deleted_after_load() {
        let (mut config, problems) = BrowserConfig::parse_lenient(
            r#"{"favorites":[{"title":"A","url":"https://a.example"}],"notes":[],"easels":[]}"#,
        );
        config.sanitize();
        assert!(problems.is_empty());
        assert_eq!(config.favorites.len(), 1);
        assert!(config.notes.is_empty());
        assert!(config.easels.is_empty());
    }

    #[test]
    fn a_corrupt_field_keeps_the_rest_of_the_config() {
        let (config, problems) = BrowserConfig::parse_lenient(
            r##"{
                "theme": 5,
                "accent_color": "#123456",
                "notes": [
                    {"id":"n1","title":"Keep me","content":"My important note"},
                    {"id":"n2","title":7,"content":"broken"}
                ],
                "favorites": [{"title":"Mine","url":"https://mine.example"}]
            }"##,
        );
        assert_eq!(problems.len(), 2, "{problems:?}");
        assert_eq!(config.theme, "eclipse");
        assert_eq!(config.accent_color, "#123456");
        assert_eq!(config.notes.len(), 1);
        assert_eq!(config.notes[0].content, "My important note");
        assert_eq!(config.favorites.len(), 1);
    }

    #[test]
    fn unparseable_json_reports_a_problem() {
        let (config, problems) = BrowserConfig::parse_lenient(r#"{"theme": "#);
        assert_eq!(problems.len(), 1);
        assert_eq!(config, BrowserConfig::default());
    }

    #[test]
    fn in_flight_downloads_are_marked_failed_on_load() {
        let (mut config, _) = BrowserConfig::parse_lenient(
            r#"{"downloads":[{"id":1,"file_name":"a.zip","status":"downloading"}]}"#,
        );
        config.sanitize();
        assert_eq!(config.downloads[0].status, "failed");
    }

    fn patch(json: &str) -> Map<String, Value> {
        serde_json::from_str(json).unwrap()
    }

    #[test]
    fn settings_patch_applies_only_whitelisted_valid_keys() {
        let mut config = BrowserConfig::default();
        let changed = config.apply_settings_patch(&patch(
            r##"{
                "theme": "void",
                "accent_color": "#F472B6",
                "adblock_enabled": false,
                "tab_hibernation_mins": 30,
                "notes": [],
                "favorites": [],
                "installed_extensions": [],
                "downloads": []
            }"##,
        ));
        assert_eq!(
            changed,
            vec!["theme", "accent_color", "tab_hibernation_mins", "adblock_enabled"]
        );
        assert_eq!(config.theme, "void");
        assert!(!config.adblock_enabled);
        assert_eq!(config.tab_hibernation_mins, 30);
        assert!(!config.notes.is_empty());
        assert!(!config.favorites.is_empty());
    }

    #[test]
    fn settings_patch_rejects_wrong_types_and_ranges() {
        let mut config = BrowserConfig::default();
        let before = config.clone();
        let changed = config.apply_settings_patch(&patch(
            r#"{
                "theme": 5,
                "accent_color": "red; background:url(x)",
                "sidebar_width": 9000,
                "sidebar_collapsed": "yes",
                "search_engine": "evil",
                "user_agent": "",
                "custom_user_agent": "a\nInjected: header",
                "tab_hibernation_mins": -1,
                "rounded_corners_radius": 99,
                "adblock_disabled_domains": "example.com",
                "custom_css": 1
            }"#,
        ));
        assert!(changed.is_empty(), "{changed:?}");
        assert_eq!(config, before);
    }

    #[test]
    fn settings_patch_normalizes_domains_and_skips_unchanged_values() {
        let mut config = BrowserConfig::default();
        let changed = config.apply_settings_patch(&patch(
            r#"{"adblock_disabled_domains": ["https://WWW.Example.com/path", "x');alert(1);//", "example.com/", "www.example.com"], "search_engine": "duckduckgo"}"#,
        ));
        assert_eq!(changed, vec!["adblock_disabled_domains"]);
        assert_eq!(
            config.adblock_disabled_domains,
            vec!["www.example.com", "example.com"]
        );
    }

    #[test]
    fn every_patchable_key_is_handled() {
        let mut config = BrowserConfig::default();
        let samples = patch(
            r##"{
                "theme": "lunar", "accent_color": "#abc", "sidebar_width": 210,
                "sidebar_collapsed": false, "search_engine": "kagi", "user_agent": "UA/1",
                "drm_fairplay_enabled": false, "autoplay_enabled": false,
                "tab_hibernation_mins": 0, "adblock_enabled": false,
                "adblock_disabled_domains": ["a.example"], "webextensions_enabled": false,
                "custom_js": "1", "custom_css": "a{}", "custom_user_agent": "UA/2",
                "rounded_corners_radius": 4
            }"##,
        );
        let changed = config.apply_settings_patch(&samples);
        assert_eq!(changed, BrowserConfig::PATCHABLE_SETTINGS.to_vec());
    }

    #[test]
    fn normalizes_site_domains() {
        assert_eq!(normalize_site_domain("Example.COM"), Some("example.com".into()));
        assert_eq!(
            normalize_site_domain("https://a.b.example:8443/x?y"),
            Some("a.b.example".into())
        );
        assert_eq!(normalize_site_domain("x');alert(1)"), None);
        assert_eq!(normalize_site_domain(""), None);
        assert_eq!(normalize_site_domain("a b.com"), None);
    }

    #[test]
    fn page_json_strips_extension_source() {
        let mut config = BrowserConfig::default();
        config.installed_extensions.push(ExtensionConfig {
            id: "ext_1".into(),
            name: "Ext".into(),
            version: "1".into(),
            description: String::new(),
            enabled: true,
            path: None,
            matches: vec!["<all_urls>".into()],
            content_scripts: vec!["secretSource()".into()],
            content_css: vec!["body{}".into()],
        });
        let json = config.to_page_json();
        assert!(json.contains("\"ext_1\""));
        assert!(!json.contains("secretSource"));
    }

    #[test]
    fn session_round_trips_with_defaults() {
        let session: Session =
            serde_json::from_str(r#"{"tabs":[{"url":"https://a.example"}]}"#).unwrap();
        assert_eq!(session.current_workspace, "Default");
        assert_eq!(session.tabs[0].workspace, "Default");
        assert_eq!(session.tabs[0].zoom, 1.0);
        assert!(!session.tabs[0].is_pinned);
    }
}
