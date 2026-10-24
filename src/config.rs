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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FavoriteShortcut {
    pub title: String,
    pub url: String,
    #[serde(default)]
    pub icon: String,
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
    vec![
        ExtensionConfig {
            id: "sponsorblock".into(),
            name: "SponsorBlock for YouTube".into(),
            version: "5.5.0".into(),
            description: "Automatically skip sponsor segments, intros, and end cards on YouTube.".into(),
            enabled: true,
            path: None,
            matches: vec!["*://*.youtube.com/*".into(), "*://youtu.be/*".into()],
            content_scripts: vec![
                r#"(function() {
                    if (window.__sponsorBlockReady) return;
                    window.__sponsorBlockReady = true;
                    function checkSponsor() {
                        var skipBtn = document.querySelector('.ytp-ad-skip-button, .ytp-skip-ad-button');
                        if (skipBtn) { skipBtn.click(); }
                    }
                    setInterval(checkSponsor, 500);
                })();"#.into()
            ],
            content_css: vec![],
        },
        ExtensionConfig {
            id: "return-dislike".into(),
            name: "Return YouTube Dislike".into(),
            version: "3.0.0".into(),
            description: "Restores dislike count on YouTube videos using returnyoutubedislikeapi.com.".into(),
            enabled: true,
            path: None,
            matches: vec!["*://*.youtube.com/*".into()],
            content_scripts: vec![
                r#"(function() {
                    if (window.__rydReady) return;
                    window.__rydReady = true;
                    function fetchDislikes() {
                        var urlParams = new URLSearchParams(window.location.search);
                        var videoId = urlParams.get('v');
                        if (!videoId) return;
                        fetch('https://returnyoutubedislikeapi.com/votes?videoId=' + videoId)
                            .then(function(r) { return r.json(); })
                            .then(function(data) {
                                if (data && data.dislikes != null) {
                                    var btn = document.querySelector('dislike-button-view-model button, #segmented-dislike-button button');
                                    if (btn) {
                                        var countText = data.dislikes >= 1000 ? (data.dislikes/1000).toFixed(1) + 'K' : data.dislikes;
                                        btn.setAttribute('title', countText + ' dislikes');
                                    }
                                }
                            }).catch(function() {});
                    }
                    setTimeout(fetchDislikes, 1200);
                    window.addEventListener('yt-navigate-finish', function() { setTimeout(fetchDislikes, 1000); });
                })();"#.into()
            ],
            content_css: vec![],
        },
        ExtensionConfig {
            id: "dark-reader".into(),
            name: "Dark Reader".into(),
            version: "4.9.80".into(),
            description: "Inverts colors and applies a soothing dark theme to light web pages.".into(),
            enabled: false,
            path: None,
            matches: vec!["<all_urls>".into()],
            content_scripts: vec![],
            content_css: vec![
                r#"html { filter: invert(0.9) hue-rotate(180deg) !important; background: #111 !important; }
                   img, video, iframe, canvas, picture { filter: invert(1.11) hue-rotate(180deg) !important; }"#.into()
            ],
        },
        ExtensionConfig {
            id: "volume-booster".into(),
            name: "Volume Booster 600%".into(),
            version: "2.1.0".into(),
            description: "Boost audio volume on HTML5 videos and audio players up to 600%.".into(),
            enabled: false,
            path: None,
            matches: vec!["<all_urls>".into()],
            content_scripts: vec![
                r#"(function() {
                    if (window.__volBoostReady) return;
                    window.__volBoostReady = true;
                    try {
                        var AudioCtx = window.AudioContext || window.webkitAudioContext;
                        if (!AudioCtx) return;
                        var ctx = null;
                        document.addEventListener('play', function(e) {
                            var el = e.target;
                            if (el && (el.tagName === 'VIDEO' || el.tagName === 'AUDIO') && !el.__boosted) {
                                el.__boosted = true;
                                if (!ctx) ctx = new AudioCtx();
                                var source = ctx.createMediaElementSource(el);
                                var gainNode = ctx.createGain();
                                gainNode.gain.value = 2.0;
                                source.connect(gainNode);
                                gainNode.connect(ctx.destination);
                            }
                        }, true);
                    } catch (_) {}
                })();"#.into()
            ],
            content_css: vec![],
        }
    ]
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
                    // The old default was 264 px, which reads as a thick rail
                    // on modern displays. There is no width editor, so migrate
                    // anything in the old wide range to the compact default.
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
    }

    #[test]
    fn missing_fields_are_filled_in() {
        let config: BrowserConfig = serde_json::from_str(r#"{"custom_css":"body{}"}"#).unwrap();
        assert_eq!(config.theme, "eclipse");
        assert_eq!(config.tab_hibernation_mins, 15);
        assert!(!config.favorites.is_empty());
    }

    #[test]
    fn an_explicitly_empty_favorites_list_is_kept() {
        let config: BrowserConfig = serde_json::from_str(r#"{"favorites":[]}"#).unwrap();
        assert!(config.favorites.is_empty());
    }
}
