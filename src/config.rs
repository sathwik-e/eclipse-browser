use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowserConfig {
    pub theme: String,
    pub accent_color: String,
    pub sidebar_width: f64,
    pub sidebar_collapsed: bool,
    pub search_engine: String,
    pub user_agent: String,
    pub drm_fairplay_enabled: bool,
    pub tab_hibernation_mins: u32,
    pub adblock_enabled: bool,
    pub custom_css: String,
    pub custom_user_agent: String,
}

impl Default for BrowserConfig {
    fn default() -> Self {
        Self {
            theme: "eclipse".into(),
            accent_color: "#F59E0B".into(), // Eclipse Corona Amber / Gold
            sidebar_width: 275.0,
            sidebar_collapsed: false,
            search_engine: "duckduckgo".into(),
            // Modern macOS Safari User Agent - enables native Apple FairPlay DRM streaming for Netflix, Apple TV, Prime
            user_agent: "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.6 Safari/605.1.15".into(),
            drm_fairplay_enabled: true,
            tab_hibernation_mins: 15,
            adblock_enabled: true,
            custom_css: "".into(),
            custom_user_agent: "".into(),
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
            if let Ok(config) = serde_json::from_str::<Self>(&content) {
                return config;
            }
        }
        Self::default()
    }

    pub fn save(&self) {
        let path = Self::config_path();
        if let Ok(json) = serde_json::to_string_pretty(self) {
            let _ = fs::write(path, json);
        }
    }

    pub fn get_search_url(&self, query: &str) -> String {
        let encoded = urlencoding(query);
        match self.search_engine.as_str() {
            "google" => format!("https://www.google.com/search?q={}", encoded),
            "brave" => format!("https://search.brave.com/search?q={}", encoded),
            "ecosia" => format!("https://www.ecosia.org/search?q={}", encoded),
            "kagi" => format!("https://kagi.com/search?q={}", encoded),
            _ => format!("https://duckduckgo.com/?q={}", encoded),
        }
    }

    pub fn active_user_agent(&self) -> &str {
        if !self.custom_user_agent.trim().is_empty() {
            &self.custom_user_agent
        } else {
            &self.user_agent
        }
    }
}

fn urlencoding(input: &str) -> String {
    let mut encoded = String::with_capacity(input.len());
    for byte in input.bytes() {
        match byte {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                encoded.push(byte as char);
            }
            b' ' => encoded.push('+'),
            _ => {
                encoded.push_str(&format!("%{:02X}", byte));
            }
        }
    }
    encoded
}
