//! Browsing history, saved to `history.json` next to the config. One entry
//! per address (http and https only) with its title, the site's icon, the
//! times of its most recent visits and how many visits it had in all.
//! Visits older than `RETENTION_DAYS` are dropped and at most `MAX_ENTRIES`
//! addresses are kept, so the file and memory stay bounded.

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use crate::config::{BrowserConfig, bad_file_path, unix_now, write_atomic};

pub const RETENTION_DAYS: u64 = 90;
pub const MAX_ENTRIES: usize = 10_000;
/// Once history is full, a site holding more entries than this gives up its
/// own oldest first, so one site (a page rewriting its address in a loop)
/// cannot push every other site out.
const MAX_ENTRIES_PER_ORIGIN: usize = MAX_ENTRIES / 10;
/// Visit times kept per address (the newest). Enough to rank by recency and
/// to clear a time range; older visits only count in `visit_count`.
pub const MAX_VISITS_PER_ENTRY: usize = 20;
/// A page of eclipse://history never holds more than this many entries.
pub const MAX_PAGE: usize = 200;
const DEFAULT_PAGE: usize = 100;
const MAX_URL_LEN: usize = 8 * 1024;
const MAX_TITLE_CHARS: usize = 300;
const MAX_ICON_LEN: usize = 2048;
const MAX_QUERY_CHARS: usize = 200;
const DAY: u64 = 24 * 60 * 60;
/// Frecency looks at this many of the newest visits (as Firefox does).
const FRECENCY_SAMPLE: usize = 10;
const FILE_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Entry {
    #[serde(default)]
    title: String,
    /// The icon the page reported for itself; none: the site's /favicon.ico.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    icon: Option<String>,
    /// Unix seconds of the newest visits, oldest first, at most
    /// `MAX_VISITS_PER_ENTRY` (never empty).
    visits: Vec<u64>,
    /// All visits, including those no longer in `visits` (always at least
    /// `visits.len()`). The ones not listed are older than `visits[0]`.
    #[serde(default)]
    visit_count: u64,
}

impl Entry {
    fn last_visit(&self) -> u64 {
        self.visits.last().copied().unwrap_or(0)
    }

    /// Recency and frequency in one number: the average of a weight per
    /// sampled recent visit (100 within 4 days, 70 within 2 weeks, 50 within
    /// a month, 30 within 3 months, 10 older) times the number of visits.
    fn frecency(&self, now: u64) -> u64 {
        let sampled: Vec<u64> = self.visits.iter().rev().take(FRECENCY_SAMPLE).copied().collect();
        if sampled.is_empty() {
            return 0;
        }
        let weight = |at: u64| match now.saturating_sub(at) / DAY {
            0..4 => 100,
            4..14 => 70,
            14..31 => 50,
            31..90 => 30,
            _ => 10,
        };
        let total: u64 = sampled.iter().map(|&at| weight(at)).sum();
        self.visit_count.max(sampled.len() as u64) * total / sampled.len() as u64
    }
}

/// One stored entry as history.json holds it (newest visit first in the file).
#[derive(Serialize, Deserialize)]
struct StoredEntry {
    url: String,
    #[serde(flatten)]
    entry: Entry,
}

#[derive(Serialize, Deserialize)]
struct HistoryFile {
    #[serde(default)]
    version: u32,
    #[serde(default)]
    entries: Vec<StoredEntry>,
}

/// A visit `record_visit` stored: the address and the time it was filed at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Visit {
    pub url: String,
    pub at: u64,
}

/// Where the previous page of eclipse://history ended: the next page holds
/// the entries listed after this one, so visits made meanwhile (which move
/// entries to the top) never show a row twice.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct HistoryCursor {
    pub last_visit: u64,
    pub url: String,
}

/// One row of eclipse://history.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct HistoryItem {
    pub url: String,
    pub title: String,
    pub host: String,
    /// An http(s) icon to show; the page falls back to a globe.
    pub icon: Option<String>,
    pub last_visit: u64,
    pub visit_count: u64,
}

/// A page of entries for eclipse://history, newest first.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct HistoryPage {
    pub entries: Vec<HistoryItem>,
    /// How many entries match the query in all.
    pub total: usize,
    pub offset: usize,
    /// More matching entries are listed after this page.
    pub more: bool,
}

/// The time ranges "Clear browsing history…" offers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClearRange {
    LastHour,
    Today,
    TodayAndYesterday,
    All,
}

impl ClearRange {
    /// The first second the range covers (local time for days), or None for
    /// all history.
    pub fn start(self, now: u64) -> Option<u64> {
        match self {
            ClearRange::LastHour => Some(now.saturating_sub(60 * 60)),
            ClearRange::Today => Some(local_midnight(now, 0)),
            ClearRange::TodayAndYesterday => Some(local_midnight(now, 1)),
            ClearRange::All => None,
        }
    }

    /// For a toast: "Cleared history from the last hour".
    pub fn description(self) -> &'static str {
        match self {
            ClearRange::LastHour => "the last hour",
            ClearRange::Today => "today",
            ClearRange::TodayAndYesterday => "today and yesterday",
            ClearRange::All => "all time",
        }
    }
}

/// Unix time of local midnight `days_back` days before the day of `now`.
fn local_midnight(now: u64, days_back: i32) -> u64 {
    let time = now as libc::time_t;
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    if unsafe { libc::localtime_r(&time, &mut tm) }.is_null() {
        // No time zone information: UTC days.
        return (now / DAY).saturating_sub(days_back as u64) * DAY;
    }
    tm.tm_hour = 0;
    tm.tm_min = 0;
    tm.tm_sec = 0;
    tm.tm_mday -= days_back;
    tm.tm_isdst = -1;
    let midnight = unsafe { libc::mktime(&mut tm) };
    if midnight < 0 {
        (now / DAY).saturating_sub(days_back as u64) * DAY
    } else {
        (midnight as u64).min(now)
    }
}

/// The address history keeps for a committed main-frame URL: http(s) with a
/// host, without a user name or password (never written to disk). None for
/// everything else (eclipse:, about:, data:, blob:, file:, ...).
pub fn history_url(url: &str) -> Option<String> {
    let url = url.trim();
    if url.is_empty() || url.len() > MAX_URL_LEN {
        return None;
    }
    let mut parsed = url::Url::parse(url).ok()?;
    if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none_or(str::is_empty) {
        return None;
    }
    let _ = parsed.set_username("");
    let _ = parsed.set_password(None);
    let url = String::from(parsed);
    (url.len() <= MAX_URL_LEN).then_some(url)
}

/// A page title as history stores it: control characters and runs of white
/// space collapsed, at most `MAX_TITLE_CHARS` characters.
fn clean_title(title: &str) -> String {
    title
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(MAX_TITLE_CHARS)
        .collect()
}

/// Icons history keeps and shows: https only, so looking at history never
/// sends a plaintext request that tells the network which sites were visited.
fn is_safe_icon(icon: &str) -> bool {
    icon.len() <= MAX_ICON_LEN && icon.starts_with("https://")
}

/// The site a host belongs to, roughly its registrable domain: the last two
/// labels, or three under a short second-level label of a country code
/// ("example.co.uk"). An IP address is its own site.
fn site_of(host: &str) -> &str {
    if host.parse::<std::net::IpAddr>().is_ok() || host.starts_with('[') {
        return host;
    }
    let labels: Vec<&str> = host.rsplit('.').collect();
    let keep = match labels.as_slice() {
        [tld, second, _, ..] if tld.len() == 2 && second.len() <= 3 => 3,
        _ => 2,
    };
    if labels.len() <= keep {
        return host;
    }
    let cut: usize = labels[..keep].iter().map(|label| label.len() + 1).sum();
    &host[host.len() + 1 - cut..]
}

/// Whether `icon` is served by the site of the page at `url`. An icon on
/// another site would be fetched each time history is looked at, telling
/// that site (a tracker the page chose) when.
fn is_same_site_icon(url: &str, icon: &str) -> bool {
    let host = |value: &str| {
        url::Url::parse(value)
            .ok()
            .and_then(|u| u.host_str().map(str::to_ascii_lowercase))
    };
    match (host(url), host(icon)) {
        (Some(page), Some(icon)) => site_of(&page) == site_of(&icon),
        _ => false,
    }
}

fn host_of(url: &str) -> String {
    url::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(str::to_string))
        .unwrap_or_default()
}

/// The scheme, host and port of a stored address (all of `history_url`'s
/// addresses have them), without parsing it again.
fn origin_of(url: &str) -> &str {
    let start = url.find("://").map_or(0, |i| i + 3);
    let end = url[start..]
        .find(['/', '?', '#'])
        .map_or(url.len(), |i| start + i);
    &url[..end]
}

/// The address without its scheme and a leading "www.", for prefix matches.
fn bare_address(url: &str) -> &str {
    let rest = url.split_once("://").map_or(url, |(_, rest)| rest);
    rest.strip_prefix("www.").unwrap_or(rest)
}

/// The words of a search, lowercased (all of them must match).
fn search_terms(query: &str) -> Vec<String> {
    query
        .chars()
        .take(MAX_QUERY_CHARS)
        .collect::<String>()
        .to_lowercase()
        .split_whitespace()
        .map(str::to_string)
        .collect()
}

fn matches_terms(url: &str, entry: &Entry, terms: &[String]) -> bool {
    if terms.is_empty() {
        return true;
    }
    let url = url.to_lowercase();
    let title = entry.title.to_lowercase();
    terms.iter().all(|term| url.contains(term.as_str()) || title.contains(term.as_str()))
}

pub struct History {
    entries: HashMap<String, Entry>,
    /// None: kept in memory only (ECLIPSE_TEST_URL runs, tests).
    path: Option<PathBuf>,
}

impl History {
    pub fn path() -> PathBuf {
        BrowserConfig::config_dir().join("history.json")
    }

    /// The saved history. A file that cannot be read or parsed is kept as
    /// `history.json.bad-<timestamp>` and history starts empty.
    pub fn load() -> Self {
        Self::load_from(&Self::path(), unix_now())
    }

    pub fn in_memory() -> Self {
        Self {
            entries: HashMap::new(),
            path: None,
        }
    }

    fn load_from(path: &Path, now: u64) -> Self {
        let mut history = Self {
            entries: HashMap::new(),
            path: Some(path.to_path_buf()),
        };
        let bytes = match fs::read(path) {
            Ok(bytes) => bytes,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return history,
            Err(err) => {
                let backup = bad_file_path(path);
                match fs::rename(path, &backup) {
                    Ok(()) => eprintln!(
                        "[eclipse] history at {path:?} could not be read ({err}); moved to {backup:?}, starting empty"
                    ),
                    Err(move_err) => eprintln!(
                        "[eclipse] history at {path:?} could not be read ({err}) or moved aside ({move_err})"
                    ),
                }
                return history;
            }
        };
        let file: HistoryFile = match serde_json::from_slice(&bytes) {
            Ok(file) => file,
            Err(err) => {
                // Moved aside, so the next save cannot replace it before the
                // user had a chance to look.
                let backup = bad_file_path(path);
                match fs::rename(path, &backup) {
                    Ok(()) => eprintln!(
                        "[eclipse] history at {path:?} is unreadable ({err}); kept at {backup:?}, starting empty"
                    ),
                    Err(move_err) => eprintln!(
                        "[eclipse] history at {path:?} is unreadable ({err}) and could not be moved aside ({move_err})"
                    ),
                }
                return history;
            }
        };
        for StoredEntry { url, mut entry } in file.entries {
            let Some(url) = history_url(&url) else {
                continue;
            };
            entry.visits.sort_unstable();
            entry.visits.dedup();
            if entry.visits.is_empty() {
                continue;
            }
            let excess = entry.visits.len().saturating_sub(MAX_VISITS_PER_ENTRY);
            entry.visits.drain(..excess);
            entry.visit_count = entry.visit_count.max(entry.visits.len() as u64);
            entry.title = clean_title(&entry.title);
            entry.icon = entry.icon.filter(|icon| is_safe_icon(icon));
            // Two spellings of one address: keep the newer.
            if history
                .entries
                .get(&url)
                .is_none_or(|existing| existing.last_visit() < entry.last_visit())
            {
                history.entries.insert(url, entry);
            }
        }
        history.prune(now);
        history
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.entries.len()
    }

    /// Writes the history (pruned first) through a temp file and a rename.
    pub fn save(&mut self, now: u64) {
        self.prune(now);
        let Some(path) = self.path.clone() else {
            return;
        };
        let mut entries: Vec<StoredEntry> = self
            .entries
            .iter()
            .map(|(url, entry)| StoredEntry {
                url: url.clone(),
                entry: entry.clone(),
            })
            .collect();
        entries.sort_by(|a, b| {
            b.entry
                .last_visit()
                .cmp(&a.entry.last_visit())
                .then_with(|| a.url.cmp(&b.url))
        });
        let file = HistoryFile {
            version: FILE_VERSION,
            entries,
        };
        if let Ok(json) = serde_json::to_string(&file) {
            write_atomic(&path, &json);
        }
    }

    /// A visit to `url` (see `history_url`) committed at `now`. `title` is
    /// only taken for an entry that has none yet; the page's own title
    /// arrives with `set_title`. Returns the visit recorded.
    pub fn record_visit(&mut self, url: &str, title: Option<&str>, now: u64) -> Option<Visit> {
        let url = history_url(url)?;
        let at = self.add_visit(&url, title.map(clean_title).unwrap_or_default(), now);
        Some(Visit { url, at })
    }

    /// Adds a visit at `at` (or just after the newest, should the clock have
    /// gone backwards) to the entry for `url`, an address as `history_url`
    /// spells it, creating it when there is none. Returns the time filed.
    fn add_visit(&mut self, url: &str, title: String, at: u64) -> u64 {
        if let Some(entry) = self.entries.get_mut(url) {
            // The clock can go backwards; visits stay in order.
            let at = at.max(entry.last_visit());
            if entry.visits.len() >= MAX_VISITS_PER_ENTRY {
                entry.visits.remove(0);
            }
            entry.visits.push(at);
            entry.visit_count = entry.visit_count.saturating_add(1);
            if entry.title.is_empty() {
                entry.title = title;
            }
            return at;
        }
        if self.entries.len() >= MAX_ENTRIES {
            self.make_room();
        }
        self.entries.insert(
            url.to_string(),
            Entry {
                title,
                icon: None,
                visits: vec![at],
                visit_count: 1,
            },
        );
        at
    }

    /// Takes back the visit to `url` filed at `at` (a page that only
    /// redirected). An entry left without visits goes. Returns whether
    /// there was such a visit.
    pub fn remove_visit(&mut self, url: &str, at: u64) -> bool {
        let Some(entry) = self.entries.get_mut(url) else {
            return false;
        };
        let Some(index) = entry.visits.iter().rposition(|&visit| visit == at) else {
            return false;
        };
        entry.visits.remove(index);
        if entry.visits.is_empty() {
            self.entries.remove(url);
        } else {
            entry.visit_count = entry.visit_count.saturating_sub(1).max(entry.visits.len() as u64);
        }
        true
    }

    /// The page at `from` replaced its own address with `to`
    /// (history.replaceState): the visit filed at `at` moves there instead
    /// of `to` counting as another visit. `title` is the page's, for an
    /// entry `to` that has none. Returns the visit as now filed, or None
    /// (nothing changed) when that visit is gone (deleted or cleared) or
    /// `to` is not an address history keeps.
    pub fn move_visit(&mut self, from: &str, at: u64, to: &str, title: Option<&str>) -> Option<Visit> {
        let to = history_url(to)?;
        if from == to {
            return self.entries.get(&to)?.visits.contains(&at).then_some(Visit { url: to, at });
        }
        let icon = self.entries.get(from).and_then(|entry| entry.icon.clone());
        if !self.remove_visit(from, at) {
            return None;
        }
        let title = title.map(clean_title).unwrap_or_default();
        let at = match self.entries.get_mut(&to) {
            Some(entry) => {
                // Older than its newest visit: filed in order, never added
                // past the visits kept.
                let index = entry.visits.partition_point(|&visit| visit <= at);
                if index == 0 && entry.visits.len() >= MAX_VISITS_PER_ENTRY {
                    entry.visit_count = entry.visit_count.saturating_add(1);
                } else {
                    entry.visits.insert(index, at);
                    if entry.visits.len() > MAX_VISITS_PER_ENTRY {
                        entry.visits.remove(0);
                    }
                    entry.visit_count = entry.visit_count.saturating_add(1);
                }
                if entry.title.is_empty() {
                    entry.title = title;
                }
                at
            }
            None => {
                let at = self.add_visit(&to, title, at);
                // Same document, same site: its icon still applies.
                if let (Some(entry), Some(icon)) = (self.entries.get_mut(&to), icon) {
                    entry.icon = Some(icon);
                }
                at
            }
        };
        Some(Visit { url: to, at })
    }

    /// The title of the entry for exactly `url`. Returns whether it changed.
    pub fn set_title(&mut self, url: &str, title: &str) -> bool {
        let title = clean_title(title);
        match self.entries.get_mut(url) {
            Some(entry) if !title.is_empty() && entry.title != title => {
                entry.title = title;
                true
            }
            _ => false,
        }
    }

    /// The title of the entry for exactly `url`, if it has one.
    pub fn title_of(&self, url: &str) -> Option<&str> {
        self.entries.get(url).map(|entry| entry.title.as_str())
    }

    /// Puts back a title `set_title` replaced (it may be empty). Returns
    /// whether it changed.
    pub fn restore_title(&mut self, url: &str, title: &str) -> bool {
        let title = clean_title(title);
        match self.entries.get_mut(url) {
            Some(entry) if entry.title != title => {
                entry.title = title;
                true
            }
            _ => false,
        }
    }

    /// The icon the page at exactly `url` reported, when it is an https
    /// image on the page's own site. Returns whether it changed.
    pub fn set_icon(&mut self, url: &str, icon: &str) -> bool {
        if !is_safe_icon(icon) || !is_same_site_icon(url, icon) {
            return false;
        }
        match self.entries.get_mut(url) {
            Some(entry) if entry.icon.as_deref() != Some(icon) => {
                entry.icon = Some(icon.to_string());
                true
            }
            _ => false,
        }
    }

    /// Whether `url` (as `history_url` spells it) has an entry without a title.
    pub fn is_untitled(&self, url: &str) -> bool {
        self.entries.get(url).is_some_and(|entry| entry.title.is_empty())
    }

    /// Deletes the entry for `url`. Returns whether there was one.
    pub fn remove(&mut self, url: &str) -> bool {
        let key = history_url(url).unwrap_or_else(|| url.to_string());
        self.entries.remove(&key).is_some()
    }

    /// Deletes every visit at or after `since` (None: everything). An entry
    /// keeps its older visits, so a page visited today and last week is
    /// still listed (under last week) after clearing today; one with no
    /// listed visit left before `since` goes entirely, since its unlisted
    /// older visits have no known time. Returns the entries removed.
    pub fn clear_since(&mut self, since: Option<u64>) -> usize {
        let Some(since) = since else {
            let count = self.entries.len();
            self.entries.clear();
            self.remove_leftover_files();
            return count;
        };
        let before = self.entries.len();
        self.entries.retain(|_, entry| {
            let kept = entry.visits.partition_point(|&at| at < since);
            if kept == 0 {
                return false;
            }
            let removed = (entry.visits.len() - kept) as u64;
            entry.visits.truncate(kept);
            entry.visit_count = entry.visit_count.saturating_sub(removed).max(kept as u64);
            true
        });
        before - self.entries.len()
    }

    /// Drops visits older than `RETENTION_DAYS` (and entries left without
    /// any), then the least recently visited entries past `MAX_ENTRIES`.
    pub fn prune(&mut self, now: u64) {
        let cutoff = now.saturating_sub(RETENTION_DAYS * DAY);
        self.entries.retain(|_, entry| {
            let expired = entry.visits.partition_point(|&at| at < cutoff);
            if expired == 0 {
                return true;
            }
            entry.visits.drain(..expired);
            // The visits no longer listed were older still.
            entry.visit_count = entry.visits.len() as u64;
            !entry.visits.is_empty()
        });
        if self.entries.len() > MAX_ENTRIES {
            self.evict_oldest(self.entries.len() - MAX_ENTRIES);
        }
    }

    /// All history was cleared: copies of it outside history.json go too (a
    /// file kept aside as unreadable, a temp file a crash left behind).
    fn remove_leftover_files(&self) {
        let Some(path) = self.path.as_deref() else {
            return;
        };
        let (Some(dir), Some(name)) = (path.parent(), path.file_name().and_then(|n| n.to_str())) else {
            return;
        };
        let Ok(files) = fs::read_dir(dir) else {
            return;
        };
        let backup_prefix = format!("{name}.bad-");
        let temp_name = format!("{name}.tmp");
        for file in files.flatten() {
            let file_name = file.file_name();
            let Some(file_name) = file_name.to_str() else {
                continue;
            };
            if (file_name.starts_with(&backup_prefix) || file_name == temp_name)
                && file.file_type().is_ok_and(|t| t.is_file())
            {
                let _ = fs::remove_file(file.path());
            }
        }
    }

    /// Makes room for one new entry: the least recently visited one goes,
    /// taken from the site with the most entries when that site holds more
    /// than `MAX_ENTRIES_PER_ORIGIN`.
    fn make_room(&mut self) {
        let mut per_origin: HashMap<&str, usize> = HashMap::new();
        for url in self.entries.keys() {
            *per_origin.entry(origin_of(url)).or_default() += 1;
        }
        let crowded = per_origin
            .into_iter()
            .filter(|&(_, count)| count > MAX_ENTRIES_PER_ORIGIN)
            .max_by(|a, b| a.1.cmp(&b.1).then_with(|| b.0.cmp(a.0)))
            .map(|(origin, _)| origin.to_string());
        let victim = self
            .entries
            .iter()
            .filter(|(url, _)| crowded.as_deref().is_none_or(|origin| origin_of(url) == origin))
            .min_by(|a, b| a.1.last_visit().cmp(&b.1.last_visit()).then_with(|| a.0.cmp(b.0)))
            .map(|(url, _)| url.clone());
        if let Some(url) = victim {
            self.entries.remove(&url);
        }
    }

    fn evict_oldest(&mut self, count: usize) {
        if count == 0 {
            return;
        }
        let mut by_age: Vec<(u64, String)> = self
            .entries
            .iter()
            .map(|(url, entry)| (entry.last_visit(), url.clone()))
            .collect();
        by_age.sort_unstable();
        for (_, url) in by_age.into_iter().take(count) {
            self.entries.remove(&url);
        }
    }

    fn item(url: &str, entry: &Entry) -> HistoryItem {
        HistoryItem {
            url: url.to_string(),
            title: entry.title.clone(),
            host: host_of(url),
            // No plaintext /favicon.ico for an http page: a network
            // observer would learn the sites visited from looking at history.
            icon: entry
                .icon
                .clone()
                .or_else(|| url.starts_with("https://").then(|| crate::tab::get_favicon_url(url)).flatten()),
            last_visit: entry.last_visit(),
            visit_count: entry.visit_count,
        }
    }

    #[cfg(test)]
    fn query(&self, query: &str, offset: usize, limit: Option<usize>) -> HistoryPage {
        self.query_after(query, None, offset, limit)
    }

    /// The entries whose title or address contains every word of `query`,
    /// most recently visited first, `limit` (at most `MAX_PAGE`) from
    /// `offset` among those listed after `after` (the last row of the page
    /// before; None: from the top).
    pub fn query_after(
        &self,
        query: &str,
        after: Option<&HistoryCursor>,
        offset: usize,
        limit: Option<usize>,
    ) -> HistoryPage {
        let terms = search_terms(query);
        let listed_after = |url: &String, entry: &Entry| {
            after.is_none_or(|cursor| {
                let last = entry.last_visit();
                last < cursor.last_visit || (last == cursor.last_visit && url.as_str() > cursor.url.as_str())
            })
        };
        let mut total = 0;
        let mut matching: Vec<(&String, &Entry)> = self
            .entries
            .iter()
            .filter(|(url, entry)| matches_terms(url, entry, &terms))
            .inspect(|_| total += 1)
            .filter(|(url, entry)| listed_after(url, entry))
            .collect();
        matching.sort_by(|a, b| b.1.last_visit().cmp(&a.1.last_visit()).then_with(|| a.0.cmp(b.0)));
        let limit = limit.unwrap_or(DEFAULT_PAGE).clamp(1, MAX_PAGE);
        HistoryPage {
            entries: matching
                .iter()
                .skip(offset)
                .take(limit)
                .map(|(url, entry)| Self::item(url, entry))
                .collect(),
            total,
            offset,
            more: matching.len() > offset.saturating_add(limit),
        }
    }

    /// What the command bars suggest for `query` (nothing for an empty
    /// one): matching entries not in `exclude` (open tabs, favourites; as
    /// `history_url` spells them), addresses that start with what was typed
    /// first, then titles with a word that does, then by frecency.
    pub fn suggestions(&self, query: &str, limit: usize, now: u64, exclude: &HashSet<String>) -> Vec<HistoryItem> {
        let terms = search_terms(query);
        let Some(first) = terms.first() else {
            return Vec::new();
        };
        let mut ranked: Vec<(u8, u64, u64, &String, &Entry)> = self
            .entries
            .iter()
            .filter(|(url, entry)| !exclude.contains(*url) && matches_terms(url, entry, &terms))
            .map(|(url, entry)| {
                let address = bare_address(url).to_lowercase();
                let quality = if address.starts_with(first.as_str()) {
                    2
                } else if entry
                    .title
                    .to_lowercase()
                    .split(|c: char| !c.is_alphanumeric())
                    .any(|word| word.starts_with(first.as_str()))
                {
                    1
                } else {
                    0
                };
                (quality, entry.frecency(now), entry.last_visit(), url, entry)
            })
            .collect();
        ranked.sort_by(|a, b| {
            b.0.cmp(&a.0)
                .then_with(|| b.1.cmp(&a.1))
                .then_with(|| b.2.cmp(&a.2))
                .then_with(|| a.3.cmp(b.3))
        });
        ranked
            .into_iter()
            .take(limit)
            .map(|(_, _, _, url, entry)| Self::item(url, entry))
            .collect()
    }

    /// The `limit` entries with the highest frecency, for a bar that filters
    /// its suggestions itself (the New Little Eclipse Window bar).
    pub fn top_sites(&self, limit: usize, now: u64) -> Vec<HistoryItem> {
        let mut ranked: Vec<(u64, u64, &String, &Entry)> = self
            .entries
            .iter()
            .map(|(url, entry)| (entry.frecency(now), entry.last_visit(), url, entry))
            .collect();
        ranked.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| b.1.cmp(&a.1)).then_with(|| a.2.cmp(b.2)));
        ranked
            .into_iter()
            .take(limit)
            .map(|(_, _, url, entry)| Self::item(url, entry))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    const NOW: u64 = 1_800_000_000;
    const HOUR: u64 = 60 * 60;

    fn scratch_dir(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "eclipse-history-{label}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or_default()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn urls(page: &HistoryPage) -> Vec<&str> {
        page.entries.iter().map(|e| e.url.as_str()).collect()
    }

    #[test]
    fn only_web_addresses_are_recorded() {
        assert_eq!(history_url("https://example.com").as_deref(), Some("https://example.com/"));
        assert_eq!(history_url(" http://a.example/x?y#z ").as_deref(), Some("http://a.example/x?y#z"));
        // Credentials never reach the file.
        assert_eq!(
            history_url("https://user:secret@a.example/p").as_deref(),
            Some("https://a.example/p")
        );
        for url in [
            "eclipse://history",
            "eclipse://localhost/settings",
            "about:blank",
            "data:text/html,hi",
            "blob:https://a.example/1",
            "file:///etc/hosts",
            "javascript:alert(1)",
            "ftp://a.example/",
            "https://",
            "not a url",
            "",
        ] {
            assert_eq!(history_url(url), None, "{url}");
        }
        assert_eq!(history_url(&format!("https://a.example/{}", "x".repeat(9000))), None);

        let mut history = History::in_memory();
        assert_eq!(history.record_visit("eclipse://notes", None, NOW), None);
        assert_eq!(history.record_visit("about:blank", None, NOW), None);
        assert_eq!(history.len(), 0);
    }

    #[test]
    fn visits_to_one_address_are_one_entry() {
        let mut history = History::in_memory();
        history.record_visit("https://a.example/", Some("A"), NOW - 2 * HOUR);
        history.record_visit("https://a.example", Some("Ignored"), NOW - HOUR);
        history.record_visit("https://b.example/", None, NOW);
        assert_eq!(history.len(), 2);
        let page = history.query("", 0, None);
        assert_eq!(urls(&page), ["https://b.example/", "https://a.example/"]);
        let a = &page.entries[1];
        assert_eq!(a.visit_count, 2);
        assert_eq!(a.last_visit, NOW - HOUR);
        // The first title stays until the page reports its own.
        assert_eq!(a.title, "A");
        assert_eq!(a.host, "a.example");
        assert_eq!(a.icon.as_deref(), Some("https://a.example/favicon.ico"));
        // Only the newest visit times are kept, all are counted.
        for i in 0..(MAX_VISITS_PER_ENTRY as u64 + 5) {
            history.record_visit("https://c.example/", None, NOW + i);
        }
        let c = &history.entries["https://c.example/"];
        assert_eq!(c.visits.len(), MAX_VISITS_PER_ENTRY);
        assert_eq!(c.visit_count, MAX_VISITS_PER_ENTRY as u64 + 5);
        assert_eq!(c.last_visit(), NOW + MAX_VISITS_PER_ENTRY as u64 + 4);
    }

    #[test]
    fn titles_change_only_for_their_own_address() {
        let mut history = History::in_memory();
        history.record_visit("https://a.example/", None, NOW);
        history.record_visit("https://b.example/", Some("B"), NOW);
        assert!(history.is_untitled("https://a.example/"));
        assert!(history.set_title("https://a.example/", "  Page\n\tA  "));
        assert!(!history.set_title("https://a.example/", "Page A"));
        // An empty title, or one for an address never visited, changes nothing.
        assert!(!history.set_title("https://a.example/", "   "));
        assert!(!history.set_title("https://c.example/", "C"));
        assert_eq!(history.len(), 2);
        assert_eq!(history.entries["https://a.example/"].title, "Page A");
        assert_eq!(history.entries["https://b.example/"].title, "B");
        history.set_title("https://b.example/", &"x".repeat(1000));
        assert_eq!(history.entries["https://b.example/"].title.chars().count(), MAX_TITLE_CHARS);
        // Icons: web images only, and only for the address given.
        assert!(history.set_icon("https://a.example/", "https://static.a.example/a.png"));
        assert!(!history.set_icon("https://a.example/", "data:image/png;base64,AAAA"));
        assert!(!history.set_icon("https://c.example/", "https://c.example/c.png"));
        assert_eq!(
            history.query("page a", 0, None).entries[0].icon.as_deref(),
            Some("https://static.a.example/a.png")
        );
    }

    #[test]
    fn only_https_icons_of_the_page_s_own_site_are_kept() {
        let mut history = History::in_memory();
        history.record_visit("https://www.shop.example/p", None, NOW);
        history.record_visit("http://plain.example/", None, NOW);
        // Another site's image would be fetched each time history is shown.
        assert!(!history.set_icon("https://www.shop.example/p", "https://tracker.example/p?uid=1"));
        assert!(!history.set_icon("https://www.shop.example/p", "http://www.shop.example/i.png"));
        assert!(history.set_icon("https://www.shop.example/p", "https://cdn.shop.example/i.png"));
        assert!(!history.set_icon("http://plain.example/", "http://plain.example/i.png"));
        let page = history.query("", 0, None);
        let icon = |url: &str| page.entries.iter().find(|e| e.url == url).unwrap().icon.clone();
        assert_eq!(icon("https://www.shop.example/p").as_deref(), Some("https://cdn.shop.example/i.png"));
        // No plaintext /favicon.ico for an http page either.
        assert_eq!(icon("http://plain.example/"), None);

        assert_eq!(site_of("a.b.example.com"), "example.com");
        assert_eq!(site_of("www.example.co.uk"), "example.co.uk");
        assert_eq!(site_of("example.com"), "example.com");
        assert_eq!(site_of("localhost"), "localhost");
        assert_eq!(site_of("192.168.1.10"), "192.168.1.10");
        assert!(!is_same_site_icon("https://a.example.co.uk/", "https://b.example2.co.uk/i.png"));
        assert!(is_same_site_icon("https://a.example.co.uk/", "https://b.example.co.uk/i.png"));
    }

    #[test]
    fn a_replaced_address_moves_the_visit_instead_of_adding_one() {
        let mut history = History::in_memory();
        let first = history.record_visit("https://maps.example/@1,1", Some("Maps"), NOW).unwrap();
        history.set_icon(&first.url, "https://maps.example/icon.png");
        // A page rewriting its address as it is used: still one visit.
        let mut visit = first;
        for i in 2..50 {
            visit = history
                .move_visit(&visit.url, visit.at, &format!("https://maps.example/@{i},{i}"), Some("Maps"))
                .unwrap();
        }
        assert_eq!(history.len(), 1);
        let page = history.query("", 0, None);
        assert_eq!(urls(&page), ["https://maps.example/@49,49"]);
        assert_eq!(page.entries[0].visit_count, 1);
        assert_eq!(page.entries[0].title, "Maps");
        assert_eq!(page.entries[0].icon.as_deref(), Some("https://maps.example/icon.png"));

        // Onto an address visited before: one more visit there, one fewer
        // on the address left (which keeps its earlier visit).
        let mut history = History::in_memory();
        history.record_visit("https://a.example/?utm_source=x", None, NOW - HOUR);
        history.record_visit("https://a.example/", Some("A"), NOW - 2 * HOUR);
        let visit = history.record_visit("https://a.example/?utm_source=x", None, NOW).unwrap();
        let moved = history.move_visit(&visit.url, visit.at, "https://a.example/", None).unwrap();
        assert_eq!(moved, Visit { url: "https://a.example/".into(), at: NOW });
        assert_eq!(history.entries["https://a.example/"].visits, vec![NOW - 2 * HOUR, NOW]);
        assert_eq!(history.entries["https://a.example/?utm_source=x"].visits, vec![NOW - HOUR]);
        assert_eq!(history.entries["https://a.example/?utm_source=x"].visit_count, 1);

        // A visit that is gone (deleted, cleared) is not brought back.
        assert_eq!(history.move_visit(&visit.url, visit.at, "https://b.example/", None), None);
        history.clear_since(None);
        assert_eq!(history.move_visit("https://a.example/", NOW, "https://b.example/", None), None);
        assert_eq!(history.len(), 0);
    }

    #[test]
    fn taking_back_a_redirect_visit() {
        let mut history = History::in_memory();
        history.record_visit("https://r.example/url?q=x", None, NOW - DAY);
        let visit = history.record_visit("https://r.example/url?q=x", None, NOW).unwrap();
        assert!(history.remove_visit(&visit.url, visit.at));
        assert_eq!(history.entries[&visit.url].visits, vec![NOW - DAY]);
        assert_eq!(history.entries[&visit.url].visit_count, 1);
        assert!(!history.remove_visit(&visit.url, visit.at));
        // Its only visit: the entry goes.
        assert!(history.remove_visit(&visit.url, NOW - DAY));
        assert_eq!(history.len(), 0);
    }

    #[test]
    fn one_site_cannot_push_every_other_site_out() {
        let mut history = History::in_memory();
        let others = MAX_ENTRIES / 2;
        for i in 0..others as u64 {
            history.record_visit(&format!("https://site{i}.example/"), None, NOW - DAY + i);
        }
        // A page flooding history with addresses of its own.
        for i in 0..(MAX_ENTRIES as u64 + 200) {
            history.record_visit(&format!("https://flood.example/?{i}"), None, NOW + i);
        }
        assert_eq!(history.len(), MAX_ENTRIES);
        let kept_others = history.entries.keys().filter(|url| !url.starts_with("https://flood.")).count();
        assert_eq!(kept_others, others);
        // The flood gives up its own oldest entries.
        assert!(!history.entries.contains_key("https://flood.example/?0"));
        // So does a new site's visit.
        history.record_visit("https://new.example/", None, NOW + 2 * MAX_ENTRIES as u64);
        assert!(history.entries.contains_key("https://site0.example/"));
        assert!(history.entries.contains_key("https://new.example/"));
        assert_eq!(history.len(), MAX_ENTRIES);
        assert_eq!(origin_of("https://a.example:8080/x?y#z"), "https://a.example:8080");
        assert_eq!(origin_of("https://a.example?y"), "https://a.example");
    }

    #[test]
    fn next_pages_continue_after_the_last_row_shown() {
        let mut history = History::in_memory();
        for i in 0..10u64 {
            history.record_visit(&format!("https://p{i}.example/"), None, NOW + i);
        }
        let first = history.query_after("", None, 0, Some(4));
        assert_eq!(first.total, 10);
        assert!(first.more);
        let last = first.entries.last().unwrap();
        let cursor = HistoryCursor {
            last_visit: last.last_visit,
            url: last.url.clone(),
        };
        // Visits made meanwhile: one new address, one listed further down
        // moving to the top. Neither shifts the next page.
        history.record_visit("https://new.example/", None, NOW + 20);
        history.record_visit("https://p1.example/", None, NOW + 21);
        let next = history.query_after("", Some(&cursor), 0, Some(4));
        assert_eq!(
            urls(&next),
            ["https://p5.example/", "https://p4.example/", "https://p3.example/", "https://p2.example/"]
        );
        assert_eq!(next.total, 11);
        assert!(next.more);
        let end = HistoryCursor {
            last_visit: NOW + 2,
            url: "https://p2.example/".into(),
        };
        let rest = history.query_after("", Some(&end), 0, Some(4));
        assert_eq!(urls(&rest), ["https://p0.example/"]);
        assert!(!rest.more);
    }

    #[test]
    fn old_visits_expire_and_the_entry_count_is_capped() {
        let mut history = History::in_memory();
        let day = DAY;
        history.record_visit("https://old.example/", None, NOW - 100 * day);
        history.record_visit("https://mixed.example/", None, NOW - 95 * day);
        history.record_visit("https://mixed.example/", None, NOW - 10 * day);
        history.record_visit("https://new.example/", None, NOW);
        history.prune(NOW);
        assert!(!history.entries.contains_key("https://old.example/"));
        let mixed = &history.entries["https://mixed.example/"];
        assert_eq!(mixed.visits, vec![NOW - 10 * day]);
        assert_eq!(mixed.visit_count, 1);
        assert_eq!(history.len(), 2);

        let mut history = History::in_memory();
        for i in 0..MAX_ENTRIES as u64 {
            history.record_visit(&format!("https://site{i}.example/"), None, NOW + i);
        }
        assert_eq!(history.len(), MAX_ENTRIES);
        // One more address pushes out the least recently visited one.
        history.record_visit("https://latest.example/", None, NOW + MAX_ENTRIES as u64);
        assert_eq!(history.len(), MAX_ENTRIES);
        assert!(!history.entries.contains_key("https://site0.example/"));
        assert!(history.entries.contains_key("https://site1.example/"));
        assert!(history.entries.contains_key("https://latest.example/"));
    }

    /// An entry visited `at` each of these times.
    fn history_with(visits: &[(&str, &[u64])]) -> History {
        let mut history = History::in_memory();
        for (url, times) in visits {
            for &at in *times {
                history.record_visit(url, Some(url), at);
            }
        }
        history
    }

    #[test]
    fn clearing_a_range_keeps_older_visits() {
        let week_ago = NOW - 7 * DAY;
        let visits: &[(&str, &[u64])] = &[
            ("https://recent.example/", &[NOW - 10 * 60]),
            ("https://both.example/", &[week_ago, NOW - 5 * 60]),
            ("https://old.example/", &[week_ago]),
        ];
        // Last hour: the page visited today and last week stays, now listed
        // under last week with one visit.
        let mut history = history_with(visits);
        assert_eq!(history.clear_since(ClearRange::LastHour.start(NOW)), 1);
        let page = history.query("", 0, None);
        assert_eq!(urls(&page), ["https://both.example/", "https://old.example/"]);
        assert_eq!(page.entries[0].last_visit, week_ago);
        assert_eq!(page.entries[0].visit_count, 1);

        // All history.
        let mut history = history_with(visits);
        assert_eq!(history.clear_since(ClearRange::All.start(NOW)), 3);
        assert_eq!(history.len(), 0);

        // An entry whose listed visits all fall in the range goes, even
        // though it had older visits that are no longer listed.
        let mut history = History::in_memory();
        for i in 0..(MAX_VISITS_PER_ENTRY as u64 + 3) {
            history.record_visit("https://busy.example/", None, NOW - 30 * 60 + i);
        }
        assert_eq!(history.clear_since(ClearRange::LastHour.start(NOW)), 1);
        assert_eq!(history.len(), 0);
    }

    #[test]
    fn day_ranges_start_at_local_midnight() {
        let now = unix_now();
        let today = ClearRange::Today.start(now).unwrap();
        let yesterday = ClearRange::TodayAndYesterday.start(now).unwrap();
        assert!(today <= now && now - today < DAY + HOUR, "{now} {today}");
        // A day is 23 to 25 hours around daylight saving changes.
        assert!(today - yesterday >= 23 * HOUR && today - yesterday <= 25 * HOUR);
        let hour = ClearRange::LastHour.start(now).unwrap();
        assert_eq!(now - hour, HOUR);
        assert_eq!(ClearRange::All.start(now), None);

        // Today and yesterday removes yesterday's visits but not older ones.
        let mut history = history_with(&[
            ("https://today.example/", &[now]),
            ("https://yesterday.example/", &[yesterday + 60]),
            ("https://before.example/", &[yesterday - 60]),
        ]);
        assert_eq!(history.clear_since(ClearRange::Today.start(now)), 1);
        assert_eq!(history.clear_since(ClearRange::TodayAndYesterday.start(now)), 1);
        assert_eq!(urls(&history.query("", 0, None)), ["https://before.example/"]);

        let range: ClearRange = serde_json::from_str(r#""today_and_yesterday""#).unwrap();
        assert_eq!(range, ClearRange::TodayAndYesterday);
        assert!(serde_json::from_str::<ClearRange>(r#""forever""#).is_err());
    }

    #[test]
    fn deleting_an_entry() {
        let mut history = history_with(&[("https://a.example/", &[NOW]), ("https://b.example/", &[NOW])]);
        assert!(history.remove("https://a.example"));
        assert!(!history.remove("https://a.example/"));
        assert!(!history.remove("https://never.example/"));
        assert_eq!(urls(&history.query("", 0, None)), ["https://b.example/"]);
    }

    #[test]
    fn search_matches_every_word_in_the_title_or_address() {
        let mut history = History::in_memory();
        history.record_visit("https://docs.rs/serde", Some("serde - Rust"), NOW - 3);
        history.record_visit("https://github.com/serde-rs/json", Some("JSON for Rust"), NOW - 2);
        history.record_visit("https://news.example/", Some("Daily News"), NOW - 1);
        assert_eq!(
            urls(&history.query("SERDE", 0, None)),
            ["https://github.com/serde-rs/json", "https://docs.rs/serde"]
        );
        assert_eq!(urls(&history.query("rust json", 0, None)), ["https://github.com/serde-rs/json"]);
        assert_eq!(urls(&history.query("daily", 0, None)), ["https://news.example/"]);
        assert!(history.query("nothing here", 0, None).entries.is_empty());
        assert_eq!(history.query("  ", 0, None).total, 3);
    }

    #[test]
    fn pages_of_results() {
        let mut history = History::in_memory();
        for i in 0..250u64 {
            history.record_visit(&format!("https://p{i}.example/"), None, NOW + i);
        }
        let first = history.query("", 0, Some(100));
        assert_eq!(first.total, 250);
        assert_eq!(first.entries.len(), 100);
        assert_eq!(first.entries[0].url, "https://p249.example/");
        let last = history.query("", 200, Some(100));
        assert_eq!(last.entries.len(), 50);
        assert_eq!(last.offset, 200);
        assert_eq!(last.entries[49].url, "https://p0.example/");
        // The page size is capped, and past the end is empty.
        assert_eq!(history.query("", 0, Some(100_000)).entries.len(), MAX_PAGE);
        assert_eq!(history.query("", 0, Some(0)).entries.len(), 1);
        assert!(history.query("", 1000, None).entries.is_empty());
        assert_eq!(history.query("", 0, None).entries.len(), DEFAULT_PAGE);
    }

    #[test]
    fn suggestions_prefer_typed_prefixes_then_frecency() {
        let mut history = History::in_memory();
        // Visited often, but long ago.
        for i in 0..5 {
            history.record_visit("https://github.com/old", Some("Old repo"), NOW - 60 * DAY + i);
        }
        // Visited often and recently.
        for i in 0..5 {
            history.record_visit("https://github.com/", Some("GitHub"), NOW - i);
        }
        history.record_visit("https://www.gitlab.com/", Some("GitLab"), NOW - 10);
        history.record_visit("https://blog.example/git-tips", Some("Git tips"), NOW);
        history.record_visit("https://other.example/", Some("Something"), NOW);

        let suggested = |query: &str, exclude: &HashSet<String>| -> Vec<String> {
            history.suggestions(query, 10, NOW, exclude).into_iter().map(|s| s.url).collect()
        };
        let none = HashSet::new();
        assert_eq!(
            suggested("git", &none),
            [
                "https://github.com/",
                "https://github.com/old",
                "https://www.gitlab.com/",
                "https://blog.example/git-tips",
            ]
        );
        // Open tabs and favourites are not suggested again.
        let open: HashSet<String> = ["https://github.com/".to_string()].into();
        assert_eq!(suggested("git", &open)[0], "https://github.com/old");
        assert!(suggested("", &none).is_empty());
        assert_eq!(history.suggestions("git", 2, NOW, &none).len(), 2);
        // Frecency: recent visits weigh more than old ones.
        let recent = &history.entries["https://github.com/"];
        let old = &history.entries["https://github.com/old"];
        assert!(recent.frecency(NOW) > old.frecency(NOW));
        assert_eq!(history.top_sites(1, NOW)[0].url, "https://github.com/");
    }

    #[test]
    fn history_round_trips_through_the_file() {
        let dir = scratch_dir("roundtrip");
        let path = dir.join("history.json");
        let mut history = History::load_from(&path, NOW);
        assert_eq!(history.len(), 0);
        history.record_visit("https://a.example/", Some("A"), NOW - 10);
        history.record_visit("https://a.example/", None, NOW - 5);
        history.record_visit("https://b.example/", Some("B"), NOW);
        history.set_icon("https://b.example/", "https://b.example/icon.png");
        history.save(NOW);
        assert!(!dir.join("history.json.tmp").exists());

        let loaded = History::load_from(&path, NOW);
        assert_eq!(loaded.entries, history.entries);
        assert_eq!(urls(&loaded.query("", 0, None)), ["https://b.example/", "https://a.example/"]);
        // The file lists the newest first.
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.find("b.example").unwrap() < text.find("a.example").unwrap());
        assert!(text.contains("\"version\":1"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_corrupt_file_is_kept_aside_and_history_starts_empty() {
        let dir = scratch_dir("corrupt");
        let path = dir.join("history.json");
        fs::write(&path, b"{\"entries\": [ {\"url\": ").unwrap();
        let mut history = History::load_from(&path, NOW);
        assert_eq!(history.len(), 0);
        assert!(!path.exists());
        let backups: Vec<String> = fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok().map(|e| e.file_name().to_string_lossy().into_owned()))
            .filter(|name| name.starts_with("history.json.bad-"))
            .collect();
        assert_eq!(backups.len(), 1, "{backups:?}");
        // New history is written next to the backup, which is left alone.
        history.record_visit("https://a.example/", None, NOW);
        history.save(NOW);
        assert_eq!(History::load_from(&path, NOW).len(), 1);
        assert!(dir.join(&backups[0]).exists());
        // Clearing all history removes the copy kept aside too, and a temp
        // file a crash left behind, but nothing else.
        fs::write(dir.join("history.json.tmp"), b"{}").unwrap();
        fs::write(dir.join("config.json.bad-1"), b"{}").unwrap();
        history.clear_since(ClearRange::LastHour.start(NOW));
        assert!(dir.join(&backups[0]).exists());
        history.clear_since(None);
        history.save(NOW);
        assert!(!dir.join(&backups[0]).exists());
        assert!(!dir.join("history.json.tmp").exists());
        assert!(dir.join("config.json.bad-1").exists());
        assert!(path.exists());
        history.record_visit("https://a.example/", None, NOW);
        history.save(NOW);

        // Bad entries in a readable file are skipped, not fatal; expired
        // visits are dropped on load.
        fs::write(
            &path,
            format!(
                r#"{{"version":1,"entries":[
                    {{"url":"eclipse://settings","visits":[{NOW}]}},
                    {{"url":"https://ok.example/","title":"OK","visits":[{NOW}],"visit_count":3}},
                    {{"url":"https://empty.example/","visits":[]}},
                    {{"url":"https://expired.example/","visits":[1]}}
                ]}}"#
            ),
        )
        .unwrap();
        let history = History::load_from(&path, NOW);
        assert_eq!(urls(&history.query("", 0, None)), ["https://ok.example/"]);
        assert_eq!(history.entries["https://ok.example/"].visit_count, 3);

        // A missing file is a first run and creates nothing.
        let missing = dir.join("missing.json");
        assert_eq!(History::load_from(&missing, NOW).len(), 0);
        assert!(!missing.exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn in_memory_history_never_writes() {
        let mut history = History::in_memory();
        history.record_visit("https://a.example/", None, NOW);
        history.save(NOW);
        assert_eq!(history.len(), 1);
        assert!(history.path.is_none());
    }
}
