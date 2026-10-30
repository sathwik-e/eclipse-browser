//! "Update available" checks against Eclipse's GitHub releases. Nothing is
//! ever installed: a newer release is announced (sidebar notice, app menu,
//! an alert for a check the user asked for) and Download opens the release's
//! file in a tab, so the browser's own download manager fetches it.
//!
//! This module is the pure part: what the releases API returned, version
//! comparison, which file fits this Mac, which URLs may be opened, when an
//! automatic check is due and what the UI shows. The request itself is made
//! by menu.m (an ephemeral NSURLSession, no cookies or cache) and comes back
//! as `UserEvent::UpdateFeed`.

use std::cmp::Ordering;
use std::time::Duration;

use serde::Serialize;
use serde_json::Value;

/// The version of this build.
pub const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");
/// The only address an update check asks (outside the smoke-test hook).
pub const LATEST_RELEASE_API: &str =
    "https://api.github.com/repos/sathwik-e/eclipse-browser/releases/latest";
/// Every URL an update opens must start with this.
pub const RELEASES_PREFIX: &str = "https://github.com/sathwik-e/eclipse-browser/releases/";
/// Opened whenever the release's own links are missing or not trusted.
pub const LATEST_RELEASE_PAGE: &str = "https://github.com/sathwik-e/eclipse-browser/releases/latest";
/// The first automatic check waits this long after launch, so it never
/// competes with restoring the session.
pub const STARTUP_DELAY: Duration = Duration::from_secs(10);
/// Automatic checks run at most once a day (persisted across launches:
/// GitHub allows 60 unauthenticated requests an hour per address).
pub const CHECK_INTERVAL_SECS: u64 = 24 * 60 * 60;
/// After a check that never reached GitHub (offline, a timeout, a server
/// error), the next automatic one waits this long.
pub const RETRY_SECS: u64 = 60 * 60;
/// Smoke tests point the check at a local feed with this variable (only a
/// loopback http(s) address is taken).
const FEED_URL_OVERRIDE_VAR: &str = "ECLIPSE_SMOKE_UPDATE_FEED_URL";
const MAX_ASSETS: usize = 64;
const MAX_NAME_CHARS: usize = 120;
/// Release notes only ever appear as plain text, this long at most.
pub const MAX_NOTES_CHARS: usize = 600;
const MAX_URL_LEN: usize = 2048;

/// The URL an update check asks: GitHub's latest-release API, or a loopback
/// feed when `ECLIPSE_SMOKE_UPDATE_FEED_URL` is set (inert otherwise).
pub fn feed_url() -> String {
    std::env::var(FEED_URL_OVERRIDE_VAR)
        .ok()
        .filter(|url| is_loopback_feed(url))
        .unwrap_or_else(|| LATEST_RELEASE_API.to_string())
}

fn is_loopback_feed(url: &str) -> bool {
    url::Url::parse(url.trim()).is_ok_and(|parsed| {
        matches!(parsed.scheme(), "http" | "https")
            && matches!(parsed.host_str(), Some("127.0.0.1" | "localhost" | "[::1]"))
    })
}

/// Sent as the request's User-Agent; GitHub requires one. Nothing else about
/// the user or the Mac goes with the request.
pub fn user_agent() -> String {
    format!("Eclipse/{CURRENT_VERSION}")
}

/// "1.0.10", "v2.1" -> numeric components (1 to 4 of them). Anything else
/// (pre-release suffixes, empty parts, letters) is not a version.
pub fn parse_version(text: &str) -> Option<Vec<u64>> {
    let text = text.trim();
    let text = text.strip_prefix(['v', 'V']).unwrap_or(text);
    if text.is_empty() || text.len() > 40 {
        return None;
    }
    let parts = text
        .split('.')
        .map(|part| {
            (!part.is_empty() && part.len() <= 9 && part.bytes().all(|b| b.is_ascii_digit()))
                .then(|| part.parse::<u64>().ok())
                .flatten()
        })
        .collect::<Option<Vec<u64>>>()?;
    (1..=4).contains(&parts.len()).then_some(parts)
}

/// Compares versions component by component; missing components count as
/// zero (1.0 == 1.0.0).
fn compare_versions(a: &[u64], b: &[u64]) -> Ordering {
    let len = a.len().max(b.len());
    (0..len)
        .map(|i| a.get(i).unwrap_or(&0).cmp(b.get(i).unwrap_or(&0)))
        .find(|order| order.is_ne())
        .unwrap_or(Ordering::Equal)
}

/// Whether `candidate` is a strictly newer version than `current`. A version
/// that does not parse is never newer.
pub fn is_newer(candidate: &str, current: &str) -> bool {
    match (parse_version(candidate), parse_version(current)) {
        (Some(candidate), Some(current)) => compare_versions(&candidate, &current) == Ordering::Greater,
        _ => false,
    }
}

/// "Skip This Version" for `skipped` hides `version` and anything older; a
/// newer release is announced again.
pub fn is_skipped(version: &str, skipped: &str) -> bool {
    match (parse_version(version), parse_version(skipped)) {
        (Some(version), Some(skipped)) => compare_versions(&skipped, &version) != Ordering::Less,
        _ => false,
    }
}

/// The `url` normalized, when it is an https page or file of Eclipse's own
/// GitHub releases (`RELEASES_PREFIX`); `None` for anything else, including
/// lookalike hosts, credentials, other ports and escaping path segments.
pub fn trusted_release_url(url: &str) -> Option<String> {
    let url = url.trim();
    if url.is_empty() || url.len() > MAX_URL_LEN || url.chars().any(|c| c.is_control() || c == '\\') {
        return None;
    }
    let parsed = url::Url::parse(url).ok()?;
    if parsed.scheme() != "https"
        || parsed.host_str() != Some("github.com")
        || parsed.port().is_some()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
    {
        return None;
    }
    // Encoded dots or slashes could climb out of the releases path once a
    // server decodes them.
    let path = parsed.path().to_ascii_lowercase();
    if path.contains("%2e") || path.contains("%2f") || path.contains("%5c") {
        return None;
    }
    let normalized = parsed.as_str();
    normalized.starts_with(RELEASES_PREFIX).then(|| normalized.to_string())
}

/// Whether `url` is a file of Eclipse's releases (what Download opens), not
/// a page.
pub fn is_release_download_url(url: &str) -> bool {
    trusted_release_url(url).is_some_and(|url| url[RELEASES_PREFIX.len()..].starts_with("download/"))
}

/// One downloadable file of a release.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseAsset {
    pub name: String,
    /// Already checked by `trusted_release_url`.
    pub url: String,
}

/// A published release, as far as the update check needs it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
    /// The tag without its leading "v" ("1.0.2").
    pub version: String,
    /// The release's title (plain text), or "Eclipse <version>".
    pub name: String,
    /// The release page, already checked (`LATEST_RELEASE_PAGE` otherwise).
    pub page_url: String,
    /// The release notes as plain text, at most `MAX_NOTES_CHARS`.
    pub notes: String,
    /// Only files whose URL is trusted.
    pub assets: Vec<ReleaseAsset>,
}

/// Why a check found no release to compare with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FeedError {
    /// No response at all (offline, a timeout, TLS); the system's words.
    Network(String),
    /// GitHub's rate limit (403 or 429).
    RateLimited,
    /// 404: nothing has been published yet.
    NoRelease,
    Http(i32),
    /// Not the JSON of a release, or its tag is not a version.
    Malformed,
    /// A draft or pre-release (the latest-release API normally excludes them).
    NotARelease,
}

impl FeedError {
    /// Whether GitHub answered, so the daily throttle starts over. A check
    /// that never got an answer is retried sooner (`RETRY_SECS`).
    pub fn reached_server(&self) -> bool {
        !matches!(self, FeedError::Network(_) | FeedError::Http(500..=599))
    }

    /// For the alert and the Settings page.
    pub fn message(&self) -> String {
        match self {
            FeedError::Network(description) if !description.trim().is_empty() => {
                format!("{} Check your connection and try again.", plain_text(description, 200))
            }
            FeedError::Network(_) => "GitHub could not be reached. Check your connection and try again.".into(),
            FeedError::RateLimited => "GitHub is limiting requests from this network. Try again in an hour.".into(),
            FeedError::NoRelease => "No release of Eclipse has been published yet.".into(),
            FeedError::Http(status) => format!("GitHub answered with an error (HTTP {status})."),
            FeedError::Malformed => "GitHub's answer could not be read.".into(),
            FeedError::NotARelease => "The newest release on GitHub is a pre-release.".into(),
        }
    }
}

/// What menu.m's request returned: `status` 0 means no response, with the
/// system's `error`.
pub fn evaluate_response(status: i32, body: &[u8], error: Option<&str>) -> Result<Release, FeedError> {
    match status {
        0 => Err(FeedError::Network(error.unwrap_or_default().to_string())),
        200 => parse_release(body),
        403 | 429 => Err(FeedError::RateLimited),
        404 => Err(FeedError::NoRelease),
        other => Err(FeedError::Http(other)),
    }
}

/// Parses GitHub's release JSON. Only `tag_name` is required; the title,
/// page, notes and files are optional, and links that are not Eclipse's own
/// releases are dropped.
pub fn parse_release(body: &[u8]) -> Result<Release, FeedError> {
    let value: Value = serde_json::from_slice(body).map_err(|_| FeedError::Malformed)?;
    let object = value.as_object().ok_or(FeedError::Malformed)?;
    let flag = |key: &str| object.get(key).and_then(Value::as_bool).unwrap_or(false);
    if flag("draft") || flag("prerelease") {
        return Err(FeedError::NotARelease);
    }
    let tag = object.get("tag_name").and_then(Value::as_str).ok_or(FeedError::Malformed)?;
    let tag = tag.trim();
    if parse_version(tag).is_none() {
        return Err(FeedError::Malformed);
    }
    let version = tag.strip_prefix(['v', 'V']).unwrap_or(tag).to_string();
    let text = |key: &str, limit: usize| {
        object
            .get(key)
            .and_then(Value::as_str)
            .map(|s| plain_text(s, limit))
            .unwrap_or_default()
    };
    let name = text("name", MAX_NAME_CHARS).replace('\n', " ");
    let name = if name.trim().is_empty() {
        format!("Eclipse {version}")
    } else {
        name.trim().to_string()
    };
    let page_url = object
        .get("html_url")
        .and_then(Value::as_str)
        .and_then(trusted_release_url)
        .unwrap_or_else(|| LATEST_RELEASE_PAGE.to_string());
    let assets = object
        .get("assets")
        .and_then(Value::as_array)
        .map(|assets| {
            assets
                .iter()
                .take(MAX_ASSETS)
                .filter_map(|asset| {
                    let name = asset.get("name")?.as_str()?;
                    let url = trusted_release_url(asset.get("browser_download_url")?.as_str()?)?;
                    Some(ReleaseAsset {
                        name: plain_text(name, MAX_NAME_CHARS).replace('\n', " "),
                        url,
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(Release {
        version,
        name,
        page_url,
        notes: text("body", MAX_NOTES_CHARS),
        assets,
    })
}

/// Text for a native alert or a page's textContent, never markup: control
/// characters dropped (newlines kept), runs of blank lines squeezed, at most
/// `limit` characters.
fn plain_text(text: &str, limit: usize) -> String {
    let mut out = String::new();
    let mut newlines = 0;
    for c in text.replace("\r\n", "\n").chars() {
        match c {
            '\n' => {
                newlines += 1;
                if newlines <= 2 {
                    out.push('\n');
                }
            }
            '\t' => {
                newlines = 0;
                out.push(' ');
            }
            c if c.is_control() => {}
            c => {
                newlines = 0;
                out.push(c);
            }
        }
    }
    let trimmed = out.trim();
    if trimmed.chars().count() > limit {
        let mut cut: String = trimmed.chars().take(limit).collect();
        cut.truncate(cut.trim_end().len());
        cut.push('…');
        cut
    } else {
        trimmed.to_string()
    }
}

/// The CPU this build runs on, which picks the release file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Arch {
    Arm64,
    X86_64,
}

impl Arch {
    pub fn current() -> Self {
        if cfg!(target_arch = "aarch64") {
            Arch::Arm64
        } else {
            Arch::X86_64
        }
    }

    fn tokens(self) -> &'static [&'static str] {
        match self {
            Arch::Arm64 => &["arm64", "aarch64"],
            Arch::X86_64 => &["x86_64", "x64", "amd64", "intel"],
        }
    }
}

/// The release file for this Mac: a macOS .dmg or .zip ("Eclipse-1.0.1-
/// macos-arm64.dmg") built for `arch`, else a universal one; a .dmg before a
/// .zip of the same kind.
pub fn pick_asset(assets: &[ReleaseAsset], arch: Arch) -> Option<&ReleaseAsset> {
    assets
        .iter()
        .filter_map(|asset| {
            let name = asset.name.to_ascii_lowercase();
            let dmg = name.ends_with(".dmg");
            if !dmg && !name.ends_with(".zip") {
                return None;
            }
            let tokens: Vec<&str> = name.split(['-', '.', ' ', '+']).collect();
            let has = |wanted: &[&str]| tokens.iter().any(|token| wanted.contains(token));
            if !has(&["macos", "mac", "osx", "darwin"]) {
                return None;
            }
            let fit = if has(arch.tokens()) {
                2
            } else if has(&["universal", "universal2"]) {
                1
            } else {
                return None;
            };
            Some(((fit, dmg), asset))
        })
        .max_by_key(|(rank, _)| *rank)
        .map(|(_, asset)| asset)
}

/// What Download opens: the file for this Mac, or the release page when
/// there is none. Re-checked here, so nothing but Eclipse's releases is ever
/// opened.
pub fn download_url(release: &Release, arch: Arch) -> String {
    pick_asset(&release.assets, arch)
        .and_then(|asset| trusted_release_url(&asset.url))
        .unwrap_or_else(|| release_page_url(release))
}

/// What Release Notes opens.
pub fn release_page_url(release: &Release) -> String {
    trusted_release_url(&release.page_url).unwrap_or_else(|| LATEST_RELEASE_PAGE.to_string())
}

/// Whether a day has passed since the last check that reached GitHub (or
/// there was none, or the clock went back).
pub fn throttle_elapsed(last_checked: u64, now: u64) -> bool {
    last_checked == 0 || now < last_checked || now - last_checked >= CHECK_INTERVAL_SECS
}

/// Who asked for a check, which decides how its result is shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckTrigger {
    /// Daily, in the background: only a newer version is shown (the notice
    /// and the menu); failures are silent.
    Automatic,
    /// Eclipse > Check for Updates…: always ends in an alert.
    Menu,
    /// The Settings page's Check Now: the page shows the result.
    Settings,
}

/// The result of one check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    UpToDate,
    Available(Release),
    Failed(FeedError),
}

/// A finished check: what it found and whether an alert reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finished {
    pub outcome: Outcome,
    pub alert: bool,
    /// GitHub answered: the daily throttle starts over.
    pub reached_server: bool,
}

/// The update state of this run (the persisted parts, last check time and
/// skipped version, live in the config).
#[derive(Debug, Default)]
pub struct UpdateChecker {
    /// The request in flight, and whether an alert reports it.
    in_flight: Option<(u64, bool)>,
    next_request_id: u64,
    /// The newest release known to be newer than this build.
    latest: Option<Release>,
    /// The sidebar notice was closed (or answered) this run.
    dismissed: bool,
    last_outcome: Option<Outcome>,
    /// No automatic check before this (unix seconds), after one that never
    /// reached GitHub.
    retry_not_before: u64,
}

impl UpdateChecker {
    /// Starts a check: the id of the request to make, or `None` when one is
    /// already in flight (it then also reports to an alert if this check
    /// would have).
    pub fn start(&mut self, trigger: CheckTrigger) -> Option<u64> {
        let alert = trigger == CheckTrigger::Menu;
        if let Some((_, in_flight_alert)) = &mut self.in_flight {
            *in_flight_alert |= alert;
            return None;
        }
        self.next_request_id += 1;
        self.in_flight = Some((self.next_request_id, alert));
        Some(self.next_request_id)
    }

    pub fn is_checking(&self) -> bool {
        self.in_flight.is_some()
    }

    /// Whether the automatic check should run now: turned on, the app has
    /// been up for `STARTUP_DELAY`, nothing in flight, a day since the last
    /// check that reached GitHub and past any retry pause.
    pub fn auto_check_due(&self, enabled: bool, last_checked: u64, now: u64, since_launch: Duration) -> bool {
        enabled
            && since_launch >= STARTUP_DELAY
            && self.in_flight.is_none()
            && self.retry_pause_over(now)
            && throttle_elapsed(last_checked, now)
    }

    /// Whether the retry pause has run out, or the clock went back past its
    /// start (like the daily throttle, do not wait for it to catch up).
    fn retry_pause_over(&self, now: u64) -> bool {
        now >= self.retry_not_before || now.saturating_add(RETRY_SECS) < self.retry_not_before
    }

    /// Records the result of request `id` (`None` for a stale or unknown id).
    pub fn finish(&mut self, id: u64, result: Result<Release, FeedError>, now: u64) -> Option<Finished> {
        let (in_flight_id, alert) = self.in_flight?;
        if in_flight_id != id {
            return None;
        }
        self.in_flight = None;
        let outcome = match result {
            Ok(release) if is_newer(&release.version, CURRENT_VERSION) => {
                // A different version than the one dismissed is announced again.
                if self.latest.as_ref().is_none_or(|known| known.version != release.version) {
                    self.dismissed = false;
                }
                self.latest = Some(release.clone());
                Outcome::Available(release)
            }
            Ok(_) => {
                self.latest = None;
                Outcome::UpToDate
            }
            Err(error) => Outcome::Failed(error),
        };
        let reached_server = match &outcome {
            Outcome::Failed(error) => error.reached_server(),
            _ => true,
        };
        if !reached_server {
            self.retry_not_before = now.saturating_add(RETRY_SECS);
        }
        self.last_outcome = Some(outcome.clone());
        Some(Finished {
            outcome,
            alert,
            reached_server,
        })
    }

    /// The newest release known to be newer than this build, skipped or not.
    pub fn latest(&self) -> Option<&Release> {
        self.latest.as_ref()
    }

    /// The newer version waiting for the user, unless they skipped it.
    pub fn pending_version(&self, skipped: &str) -> Option<&str> {
        self.latest
            .as_ref()
            .map(|release| release.version.as_str())
            .filter(|version| !is_skipped(version, skipped))
    }

    /// The version the sidebar notice announces, if it shows.
    pub fn notice_version(&self, skipped: &str) -> Option<&str> {
        self.pending_version(skipped).filter(|_| !self.dismissed)
    }

    /// Hides the sidebar notice for the rest of this run.
    pub fn dismiss(&mut self) {
        self.dismissed = true;
    }

    pub fn ui_state(&self, skipped: &str) -> UpdateUiState {
        let (status, error) = match &self.last_outcome {
            None => ("", String::new()),
            Some(Outcome::UpToDate) => ("up_to_date", String::new()),
            Some(Outcome::Available(_)) => ("available", String::new()),
            Some(Outcome::Failed(error)) => ("failed", error.message()),
        };
        UpdateUiState {
            current_version: CURRENT_VERSION,
            checking: self.is_checking(),
            status,
            error,
            available_version: self.latest.as_ref().map(|release| release.version.clone()),
            skipped: self.latest.is_some() && self.pending_version(skipped).is_none(),
            notice: self.notice_version(skipped).is_some(),
        }
    }
}

/// Pushed to the sidebar (the notice) and the Settings page (status, Check
/// Now, Download, Skip). Release notes and URLs never leave Rust.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UpdateUiState {
    pub current_version: &'static str,
    pub checking: bool,
    /// The last check this run: "" (none yet), "up_to_date", "available" or
    /// "failed" (with `error`).
    pub status: &'static str,
    pub error: String,
    /// The newest version known to be newer than this build.
    pub available_version: Option<String>,
    /// `available_version` was skipped.
    pub skipped: bool,
    /// The sidebar shows its notice.
    pub notice: bool,
}

/// The app menu item's title.
pub fn menu_title(pending: Option<&str>) -> String {
    match pending {
        Some(version) => format!("Update Available: {version}…"),
        None => "Check for Updates…".to_string(),
    }
}

/// Menu-action names an alert button sends back (menu.m calls the menu
/// callback with them).
pub const ACTION_DOWNLOAD: &str = "update_download";
pub const ACTION_RELEASE_NOTES: &str = "update_release_notes";
pub const ACTION_LATER: &str = "update_later";
/// Followed by ":" and the version the alert showed.
pub const ACTION_SKIP_PREFIX: &str = "update_skip:";
pub const ACTION_RELEASES_PAGE: &str = "update_releases_page";

/// A native alert reporting a check the user asked for. The first button is
/// the default; `cancel` is the one Escape presses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AlertSpec {
    pub message: String,
    pub info: String,
    /// (title, menu action; empty: only closes the alert).
    pub buttons: Vec<(&'static str, String)>,
    pub cancel: Option<usize>,
    pub warning: bool,
}

pub fn alert_for(outcome: &Outcome) -> AlertSpec {
    match outcome {
        Outcome::UpToDate => AlertSpec {
            message: "You’re up to date".into(),
            info: format!("Eclipse {CURRENT_VERSION} is the newest version available."),
            buttons: vec![("OK", String::new())],
            cancel: Some(0),
            warning: false,
        },
        Outcome::Available(release) => {
            let mut info = format!("You have Eclipse {CURRENT_VERSION}.");
            let default_name = format!("Eclipse {}", release.version);
            if release.name != default_name && release.name != release.version {
                info.push_str(&format!(" This release: {}.", release.name));
            }
            if !release.notes.is_empty() {
                info.push_str("\n\n");
                info.push_str(&release.notes);
            }
            info.push_str(
                "\n\nDownload opens the release in a new tab. When it has downloaded, open it and drag Eclipse into Applications.",
            );
            AlertSpec {
                message: format!("Eclipse {} is available", release.version),
                info,
                buttons: vec![
                    ("Download", ACTION_DOWNLOAD.into()),
                    ("Later", ACTION_LATER.into()),
                    ("Release Notes", ACTION_RELEASE_NOTES.into()),
                    ("Skip This Version", format!("{ACTION_SKIP_PREFIX}{}", release.version)),
                ],
                cancel: Some(1),
                warning: false,
            }
        }
        Outcome::Failed(error) => AlertSpec {
            message: "Couldn’t check for updates".into(),
            info: error.message(),
            buttons: vec![("OK", String::new()), ("Open Releases Page", ACTION_RELEASES_PAGE.into())],
            cancel: Some(0),
            warning: true,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn release_json(tag: &str) -> String {
        format!(
            r#"{{
                "tag_name": "{tag}",
                "name": "Eclipse {tag}",
                "html_url": "https://github.com/sathwik-e/eclipse-browser/releases/tag/{tag}",
                "body": "Fixes things.",
                "draft": false,
                "prerelease": false,
                "assets": [
                    {{"name": "Eclipse-1.0.2-macos-arm64.zip", "browser_download_url": "https://github.com/sathwik-e/eclipse-browser/releases/download/{tag}/Eclipse-1.0.2-macos-arm64.zip"}},
                    {{"name": "Eclipse-1.0.2-macos-arm64.dmg", "browser_download_url": "https://github.com/sathwik-e/eclipse-browser/releases/download/{tag}/Eclipse-1.0.2-macos-arm64.dmg"}},
                    {{"name": "Eclipse-1.0.2-macos-x86_64.dmg", "browser_download_url": "https://github.com/sathwik-e/eclipse-browser/releases/download/{tag}/Eclipse-1.0.2-macos-x86_64.dmg"}}
                ]
            }}"#
        )
    }

    fn asset(name: &str) -> ReleaseAsset {
        ReleaseAsset {
            name: name.into(),
            url: format!("{RELEASES_PREFIX}download/v9/{name}"),
        }
    }

    fn release(version: &str) -> Release {
        Release {
            version: version.into(),
            name: format!("Eclipse {version}"),
            page_url: format!("{RELEASES_PREFIX}tag/v{version}"),
            notes: String::new(),
            assets: Vec::new(),
        }
    }

    #[test]
    fn versions_compare_numerically() {
        assert!(is_newer("1.0.10", "1.0.9"));
        assert!(is_newer("v1.0.2", "1.0.1"));
        assert!(is_newer("2", "1.9.9"));
        assert!(is_newer("1.1", "1.0.99"));
        assert!(!is_newer("1.0.2", "1.0.2"));
        assert!(!is_newer("v1.0.2", "1.0.2"));
        assert!(!is_newer("1.0", "1.0.0"));
        assert!(!is_newer("1.0.1", "1.0.2"));
        assert!(!is_newer("1.0.9", "1.0.10"));
    }

    #[test]
    fn malformed_versions_are_never_newer() {
        for tag in ["", "v", "vv2", "latest", "1..2", "2.", "1.0.2-beta", "1.0.x", " .1", "1.2.3.4.5", "١.٢", "99999999999.0"] {
            assert!(!is_newer(tag, "1.0.1"), "{tag:?}");
        }
        assert_eq!(parse_version(" v1.0.2 "), Some(vec![1, 0, 2]));
        assert_eq!(parse_version("1.0.2-beta"), None);
        assert_eq!(parse_version("1.2.3.4.5"), None);
        assert_eq!(parse_version("+1"), None);
        assert!(!is_newer("1.0.2", "not a version"));
    }

    #[test]
    fn parses_a_release() {
        let release = parse_release(release_json("v1.0.2").as_bytes()).unwrap();
        assert_eq!(release.version, "1.0.2");
        assert_eq!(release.name, "Eclipse v1.0.2");
        assert_eq!(release.page_url, format!("{RELEASES_PREFIX}tag/v1.0.2"));
        assert_eq!(release.notes, "Fixes things.");
        assert_eq!(release.assets.len(), 3);
    }

    #[test]
    fn missing_optional_fields_get_safe_defaults() {
        let release = parse_release(br#"{"tag_name": "1.0.3"}"#).unwrap();
        assert_eq!(release.version, "1.0.3");
        assert_eq!(release.name, "Eclipse 1.0.3");
        assert_eq!(release.page_url, LATEST_RELEASE_PAGE);
        assert!(release.notes.is_empty());
        assert!(release.assets.is_empty());

        // Wrong types are ignored, not fatal.
        let release = parse_release(
            br#"{"tag_name": "1.0.3", "name": 5, "html_url": null, "body": [], "assets": [{"name": 1}, "x", {"name": "a.dmg"}]}"#,
        )
        .unwrap();
        assert_eq!(release.name, "Eclipse 1.0.3");
        assert!(release.assets.is_empty());
    }

    #[test]
    fn drafts_prereleases_and_garbage_are_not_releases() {
        assert_eq!(
            parse_release(br#"{"tag_name": "v9.0.0", "prerelease": true}"#),
            Err(FeedError::NotARelease)
        );
        assert_eq!(
            parse_release(br#"{"tag_name": "v9.0.0", "draft": true}"#),
            Err(FeedError::NotARelease)
        );
        for body in [&b""[..], b"[]", b"null", b"{", b"<html>", br#"{"name": "x"}"#, br#"{"tag_name": 9}"#, br#"{"tag_name": "nightly"}"#] {
            assert_eq!(parse_release(body), Err(FeedError::Malformed), "{}", String::from_utf8_lossy(body));
        }
    }

    #[test]
    fn untrusted_release_links_are_dropped() {
        let body = br#"{
            "tag_name": "v1.0.2",
            "html_url": "https://evil.example/sathwik-e/eclipse-browser/releases/tag/v1.0.2",
            "assets": [
                {"name": "Eclipse-1.0.2-macos-arm64.dmg", "browser_download_url": "https://github.com.evil.example/x.dmg"},
                {"name": "Eclipse-1.0.2-macos-arm64.zip", "browser_download_url": "http://github.com/sathwik-e/eclipse-browser/releases/download/v1.0.2/a.zip"}
            ]
        }"#;
        let release = parse_release(body).unwrap();
        assert_eq!(release.page_url, LATEST_RELEASE_PAGE);
        assert!(release.assets.is_empty());
        assert_eq!(download_url(&release, Arch::Arm64), LATEST_RELEASE_PAGE);
    }

    #[test]
    fn release_notes_are_plain_limited_text() {
        let long = "a".repeat(5000);
        let body = format!(r#"{{"tag_name": "1.0.2", "body": "<script>x</script>\r\n\r\n\r\n\r\nline\u0007 two\t{long}", "name": "Big\nRelease"}}"#);
        let release = parse_release(body.as_bytes()).unwrap();
        // Text is kept as text (never interpreted); control characters go.
        assert!(release.notes.starts_with("<script>x</script>\n\nline two "));
        assert!(!release.notes.contains('\u{7}'));
        assert!(release.notes.ends_with('…'));
        assert!(release.notes.chars().count() <= MAX_NOTES_CHARS + 1);
        assert_eq!(release.name, "Big Release");
    }

    #[test]
    fn responses_map_to_errors() {
        assert_eq!(
            evaluate_response(0, b"", Some("The Internet connection appears to be offline.")),
            Err(FeedError::Network("The Internet connection appears to be offline.".into()))
        );
        assert_eq!(evaluate_response(403, b"{}", None), Err(FeedError::RateLimited));
        assert_eq!(evaluate_response(429, b"{}", None), Err(FeedError::RateLimited));
        assert_eq!(evaluate_response(404, b"{}", None), Err(FeedError::NoRelease));
        assert_eq!(evaluate_response(502, b"{}", None), Err(FeedError::Http(502)));
        assert_eq!(evaluate_response(200, b"{oops", None), Err(FeedError::Malformed));
        assert!(evaluate_response(200, release_json("v1.0.2").as_bytes(), None).is_ok());
        // Offline and server errors are retried sooner than the daily check.
        assert!(!FeedError::Network(String::new()).reached_server());
        assert!(!FeedError::Http(503).reached_server());
        assert!(FeedError::RateLimited.reached_server());
        assert!(FeedError::Malformed.reached_server());
        for error in [FeedError::Network(String::new()), FeedError::RateLimited, FeedError::Http(500)] {
            assert!(!error.message().is_empty());
        }
    }

    #[test]
    fn picks_the_file_for_this_mac() {
        let assets = vec![
            asset("Eclipse-1.0.2-macos-arm64.zip"),
            asset("Eclipse-1.0.2-macos-arm64.dmg"),
            asset("Eclipse-1.0.2-macos-x86_64.zip"),
            asset("Eclipse-1.0.2-linux-x86_64.dmg"),
            asset("checksums.txt"),
        ];
        assert_eq!(pick_asset(&assets, Arch::Arm64).unwrap().name, "Eclipse-1.0.2-macos-arm64.dmg");
        assert_eq!(pick_asset(&assets, Arch::X86_64).unwrap().name, "Eclipse-1.0.2-macos-x86_64.zip");

        // A universal build serves both, after a file for the exact CPU.
        let universal = vec![asset("Eclipse-1.0.2-macos-universal.zip"), asset("Eclipse-1.0.2-macos-arm64.zip")];
        assert_eq!(pick_asset(&universal, Arch::Arm64).unwrap().name, "Eclipse-1.0.2-macos-arm64.zip");
        assert_eq!(pick_asset(&universal, Arch::X86_64).unwrap().name, "Eclipse-1.0.2-macos-universal.zip");

        // No file for this Mac: Download opens the release page.
        let other = vec![asset("Eclipse-1.0.2-macos-arm64.dmg")];
        assert!(pick_asset(&other, Arch::X86_64).is_none());
        let mut rel = release("1.0.2");
        rel.assets = other;
        assert_eq!(download_url(&rel, Arch::X86_64), rel.page_url);
        assert_eq!(download_url(&rel, Arch::Arm64), format!("{RELEASES_PREFIX}download/v9/Eclipse-1.0.2-macos-arm64.dmg"));
        let _ = Arch::current();
    }

    #[test]
    fn only_eclipse_release_urls_are_trusted() {
        for good in [
            "https://github.com/sathwik-e/eclipse-browser/releases/latest",
            "https://github.com/sathwik-e/eclipse-browser/releases/tag/v1.0.2",
            "https://github.com/sathwik-e/eclipse-browser/releases/download/v1.0.2/Eclipse-1.0.2-macos-arm64.dmg",
            "https://GitHub.com:443/sathwik-e/eclipse-browser/releases/tag/v1.0.2",
        ] {
            assert!(trusted_release_url(good).is_some(), "{good}");
        }
        for bad in [
            "",
            "http://github.com/sathwik-e/eclipse-browser/releases/latest",
            "https://github.com/sathwik-e/eclipse-browser/releases",
            "https://github.com/sathwik-e/eclipse-browser/archive/main.zip",
            "https://github.com/someone-else/eclipse-browser/releases/latest",
            "https://github.com.evil.example/sathwik-e/eclipse-browser/releases/latest",
            "https://evil.example/https://github.com/sathwik-e/eclipse-browser/releases/",
            "https://user:pw@github.com/sathwik-e/eclipse-browser/releases/latest",
            "https://github.com:8443/sathwik-e/eclipse-browser/releases/latest",
            "https://github.com/sathwik-e/eclipse-browser/releases/../../evil/repo",
            "https://github.com/sathwik-e/eclipse-browser/releases/%2e%2e/%2E%2E/evil",
            "https://github.com/sathwik-e/eclipse-browser/releases/x%2F..%2F..%2Fevil",
            "https://github.com/sathwik-e/eclipse-browser/releases/\\..\\evil",
            "https://github.com/sathwik-e/eclipse-browser/releases/\nx",
            "javascript:alert(1)//https://github.com/sathwik-e/eclipse-browser/releases/",
            "eclipse://settings",
        ] {
            assert_eq!(trusted_release_url(bad), None, "{bad:?}");
        }
        assert_eq!(
            trusted_release_url("https://GitHub.com:443/sathwik-e/eclipse-browser/releases/tag/v1").as_deref(),
            Some("https://github.com/sathwik-e/eclipse-browser/releases/tag/v1")
        );
        let mut rel = release("1.0.2");
        rel.page_url = "https://evil.example/".into();
        assert_eq!(release_page_url(&rel), LATEST_RELEASE_PAGE);
    }

    #[test]
    fn release_files_are_told_apart_from_release_pages() {
        assert!(is_release_download_url(
            "https://github.com/sathwik-e/eclipse-browser/releases/download/v1.0.2/Eclipse-1.0.2-macos-arm64.dmg"
        ));
        assert!(!is_release_download_url(LATEST_RELEASE_PAGE));
        assert!(!is_release_download_url(
            "https://github.com/sathwik-e/eclipse-browser/releases/tag/v1.0.2"
        ));
        assert!(!is_release_download_url(
            "https://github.com/someone-else/eclipse-browser/releases/download/v1/x.dmg"
        ));
    }

    #[test]
    fn the_smoke_feed_must_be_local() {
        assert!(is_loopback_feed("http://127.0.0.1:8899/latest.json"));
        assert!(is_loopback_feed("http://localhost:8899/latest.json"));
        assert!(!is_loopback_feed("https://evil.example/latest.json"));
        assert!(!is_loopback_feed("file:///tmp/latest.json"));
        assert!(!is_loopback_feed("http://127.0.0.1.evil.example/"));
    }

    #[test]
    fn automatic_checks_run_once_a_day_after_startup() {
        let day = CHECK_INTERVAL_SECS;
        let now = 1_800_000_000;
        assert!(throttle_elapsed(0, now));
        assert!(!throttle_elapsed(now - 60, now));
        assert!(!throttle_elapsed(now - day + 1, now));
        assert!(throttle_elapsed(now - day, now));
        // The clock went back: do not wait for it to catch up.
        assert!(throttle_elapsed(now + 3600, now));

        let mut checker = UpdateChecker::default();
        let up = STARTUP_DELAY;
        assert!(checker.auto_check_due(true, 0, now, up));
        assert!(!checker.auto_check_due(false, 0, now, up));
        assert!(!checker.auto_check_due(true, 0, now, Duration::from_secs(2)));
        assert!(!checker.auto_check_due(true, now - 60, now, up));

        // In flight: no second request.
        let id = checker.start(CheckTrigger::Automatic).unwrap();
        assert!(!checker.auto_check_due(true, 0, now, up));
        assert_eq!(checker.start(CheckTrigger::Automatic), None);

        // Offline: retried in an hour, not every tick.
        let finished = checker.finish(id, Err(FeedError::Network("offline".into())), now).unwrap();
        assert!(!finished.reached_server && !finished.alert);
        assert!(!checker.auto_check_due(true, 0, now + 10, up));
        assert!(checker.auto_check_due(true, 0, now + RETRY_SECS, up));
        // The clock went back past the failure: the pause is over, not
        // stretched until the clock catches up.
        assert!(checker.auto_check_due(true, 0, now - 1, up));
        assert!(checker.auto_check_due(true, 0, now - 2 * RETRY_SECS, up));
    }

    #[test]
    fn a_menu_check_joins_one_in_flight_and_gets_an_alert() {
        let mut checker = UpdateChecker::default();
        let id = checker.start(CheckTrigger::Automatic).unwrap();
        assert!(checker.is_checking());
        assert_eq!(checker.start(CheckTrigger::Menu), None);
        assert_eq!(checker.finish(id + 1, Ok(release("99.0.0")), 0), None, "stale id");
        let finished = checker.finish(id, Ok(release("99.0.0")), 0).unwrap();
        assert!(finished.alert && finished.reached_server);
        assert_eq!(finished.outcome, Outcome::Available(release("99.0.0")));
        assert!(!checker.is_checking());
        // The next one starts fresh.
        let id = checker.start(CheckTrigger::Settings).unwrap();
        let finished = checker.finish(id, Ok(release(CURRENT_VERSION)), 0).unwrap();
        assert!(!finished.alert);
        assert_eq!(finished.outcome, Outcome::UpToDate);
        assert!(checker.latest().is_none());
    }

    #[test]
    fn skipped_and_dismissed_versions_hide_the_notice() {
        assert!(is_skipped("1.0.2", "1.0.2"));
        assert!(is_skipped("1.0.2", "v1.0.3"));
        assert!(!is_skipped("1.0.3", "1.0.2"));
        assert!(!is_skipped("1.0.2", ""));
        assert!(!is_skipped("1.0.2", "garbage"));

        let mut checker = UpdateChecker::default();
        let id = checker.start(CheckTrigger::Automatic).unwrap();
        checker.finish(id, Ok(release("99.0.0")), 0);
        assert_eq!(checker.notice_version(""), Some("99.0.0"));
        assert_eq!(checker.pending_version(""), Some("99.0.0"));
        let state = checker.ui_state("");
        assert!(state.notice && !state.skipped);
        assert_eq!(state.status, "available");
        assert_eq!(state.available_version.as_deref(), Some("99.0.0"));

        // Skipped: no notice and no menu badge, until a newer one appears.
        assert_eq!(checker.notice_version("99.0.0"), None);
        assert_eq!(checker.pending_version("99.0.0"), None);
        assert!(checker.ui_state("99.0.0").skipped);
        let id = checker.start(CheckTrigger::Automatic).unwrap();
        checker.finish(id, Ok(release("99.0.1")), 0);
        assert_eq!(checker.notice_version("99.0.0"), Some("99.0.1"));

        // Dismissed: hidden for this run, the menu still says so.
        checker.dismiss();
        assert_eq!(checker.notice_version(""), None);
        assert_eq!(checker.pending_version(""), Some("99.0.1"));
        // The same version found again stays dismissed; a newer one shows.
        let id = checker.start(CheckTrigger::Automatic).unwrap();
        checker.finish(id, Ok(release("99.0.1")), 0);
        assert_eq!(checker.notice_version(""), None);
        let id = checker.start(CheckTrigger::Automatic).unwrap();
        checker.finish(id, Ok(release("99.1.0")), 0);
        assert_eq!(checker.notice_version(""), Some("99.1.0"));

        // A failed check keeps what was known.
        let id = checker.start(CheckTrigger::Automatic).unwrap();
        checker.finish(id, Err(FeedError::RateLimited), 0);
        assert_eq!(checker.notice_version(""), Some("99.1.0"));
        // Settings shows the failure next to the known release (both sent).
        let state = checker.ui_state("");
        assert_eq!(state.status, "failed");
        assert!(!state.error.is_empty());
        assert_eq!(state.available_version.as_deref(), Some("99.1.0"));
    }

    #[test]
    fn alerts_and_the_menu_describe_the_result() {
        assert_eq!(menu_title(None), "Check for Updates…");
        assert_eq!(menu_title(Some("1.0.2")), "Update Available: 1.0.2…");

        let up = alert_for(&Outcome::UpToDate);
        assert!(up.info.contains(CURRENT_VERSION));
        assert_eq!(up.buttons.len(), 1);

        let mut rel = release("99.0.0");
        rel.notes = "Faster tabs.".into();
        let available = alert_for(&Outcome::Available(rel));
        assert_eq!(available.message, "Eclipse 99.0.0 is available");
        assert!(available.info.contains("Faster tabs."));
        let actions: Vec<&str> = available.buttons.iter().map(|(_, action)| action.as_str()).collect();
        assert_eq!(
            actions,
            vec![ACTION_DOWNLOAD, ACTION_LATER, ACTION_RELEASE_NOTES, "update_skip:99.0.0"]
        );
        assert_eq!(available.buttons[available.cancel.unwrap()].0, "Later");

        let failed = alert_for(&Outcome::Failed(FeedError::RateLimited));
        assert!(failed.warning);
        assert!(failed.message.starts_with("Couldn’t check for updates"));
    }
}
