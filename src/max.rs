//! Arc "Max"-style conveniences, all worked out locally: nothing here talks
//! to an AI service. Instant Links hand the query to DuckDuckGo's "!ducky"
//! (its I'm-feeling-lucky), tab and download names are tidied by rules, and
//! a link preview is an ordinary Little Eclipse window.

use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::app::normalize_url;
use crate::config::{BrowserConfig, urlencode};

/// Shift+Return on "a, b; c" opens at most this many tabs.
pub const MAX_INSTANT_QUERIES: usize = 6;
/// The shortest time between two Shift-hover link previews.
pub const PREVIEW_COOLDOWN: Duration = Duration::from_millis(1500);
const MAX_TIDY_NAME_CHARS: usize = 80;
const MAX_URL_LEN: usize = 8192;

/* ------------------------------------------------------------ Instant Links */

/// Whether command bar input is one web address (so commas in it are part of
/// the address, not a list of queries).
fn looks_like_address(input: &str) -> bool {
    let lower = input.to_ascii_lowercase();
    !input.contains(char::is_whitespace)
        && (lower.starts_with("http://")
            || lower.starts_with("https://")
            || lower.starts_with("localhost")
            || (input.contains('.') && !input.contains([',', ';'])))
}

/// The queries in Instant Links input: "a, b; c" is three, a single web
/// address is one. Empty entries are skipped and at most
/// `MAX_INSTANT_QUERIES` are kept.
pub fn split_instant_queries(input: &str) -> Vec<String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }
    if looks_like_address(trimmed) {
        return vec![trimmed.to_string()];
    }
    trimmed
        .split([',', ';'])
        .map(str::trim)
        .filter(|query| !query.is_empty())
        .take(MAX_INSTANT_QUERIES)
        .map(str::to_string)
        .collect()
}

/// Where Shift+Return takes one query: a web address opens as typed; a
/// search goes straight to its top result through DuckDuckGo's "!ducky"
/// bang, whatever the search engine, because it redirects without an
/// interstitial (Google's "I'm Feeling Lucky" often stops on a redirect
/// notice, and the other engines have no such mode).
pub fn instant_link_url(query: &str, config: &BrowserConfig) -> String {
    let query = query.trim();
    if looks_like_address(query) {
        return normalize_url(query, config);
    }
    format!("https://duckduckgo.com/?q=!ducky+{}", urlencode(query))
}

/* -------------------------------------------------------- Tidy Tab Titles */

const TITLE_SEPARATORS: &[&str] = &[" - ", " | ", " — ", " – ", " · ", " • ", " :: ", " » ", " ⋅ "];

/// Host labels that name no site ("www", a language, a generic app host).
const GENERIC_LABELS: &[&str] = &[
    "www", "m", "mobile", "app", "apps", "web", "en", "de", "fr", "es", "api", "home", "my",
    "account", "accounts", "login", "auth", "com", "org", "net", "co", "gov", "edu", "ac",
];

fn alnum_lower(text: &str) -> String {
    text.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn collapse_whitespace(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The labels of `host` that could be a site's name in its titles:
/// "github" for github.com, "wikipedia" for en.wikipedia.org.
fn site_labels(host: &str) -> Vec<String> {
    let mut labels: Vec<&str> = host.trim_start_matches("www.").split('.').collect();
    labels.pop(); // the top-level domain
    labels
        .into_iter()
        .filter(|label| !GENERIC_LABELS.contains(label))
        .map(alnum_lower)
        .filter(|label| label.len() >= 3)
        .collect()
}

/// A count in parentheses or brackets: "(3)", "(99+)", "[2]".
fn strip_count(text: &str, leading: bool) -> &str {
    let is_count = |inner: &str| {
        let digits = inner.strip_suffix('+').unwrap_or(inner);
        !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit())
    };
    for (open, close) in [('(', ')'), ('[', ']')] {
        if leading {
            if let Some(rest) = text.strip_prefix(open)
                && let Some(end) = rest.find(close)
                && is_count(&rest[..end])
            {
                return rest[end + 1..].trim_start();
            }
        } else if let Some(rest) = text.strip_suffix(close)
            && let Some(start) = rest.rfind(open)
            && is_count(&rest[start + 1..])
        {
            return rest[..start].trim_end();
        }
    }
    text
}

/// Notification counts and unread dots around a title.
fn strip_counts(text: &str) -> &str {
    let mut text = text.trim();
    loop {
        let before = text;
        text = text.trim_start_matches(['•', '●', '*']).trim_start();
        text = strip_count(text, true);
        text = strip_count(text, false);
        if text == before {
            return text;
        }
    }
}

/// Whether one separator-delimited part of a title only names the site (or
/// the signed-in account), like "YouTube" on youtube.com or "Gmail" and
/// "me@gmail.com" on mail.google.com. `loose` (the end of a title, where
/// sites put their name) also takes a name that starts or ends with the
/// site's ("Google Docs" on docs.google.com); at the start only the name
/// itself counts, so "Navan Itinerary" stays.
fn is_site_segment(segment: &str, host: &str, labels: &[String], loose: bool) -> bool {
    let segment = strip_counts(segment);
    if segment.contains('/') || segment.split_whitespace().count() > 3 {
        return false;
    }
    if segment.contains('@') && !segment.contains(' ') && segment.contains('.') {
        return true;
    }
    let norm = alnum_lower(segment);
    if norm.len() < 2 {
        return false;
    }
    if norm == alnum_lower(host.trim_start_matches("www.")) {
        return true;
    }
    labels.iter().any(|label| {
        norm == *label
            || (loose
                && ((norm.len() <= label.len() + 10
                    && (norm.starts_with(label.as_str()) || norm.ends_with(label.as_str())))
                    || (norm.len() >= 3 && (label.starts_with(&norm) || label.ends_with(&norm)))))
    })
}

/// Byte ranges of the separator-delimited parts of `text`.
fn title_segments(text: &str) -> Vec<(usize, usize)> {
    let mut segments = Vec::new();
    let mut start = 0;
    loop {
        let next = TITLE_SEPARATORS
            .iter()
            .filter_map(|sep| text[start..].find(sep).map(|at| (start + at, sep.len())))
            .min();
        match next {
            Some((at, len)) => {
                segments.push((start, at));
                start = at + len;
            }
            None => {
                segments.push((start, text.len()));
                return segments;
            }
        }
    }
}

/// A tidier title for a pinned tab or a favorite: the site's own name
/// ("… - YouTube", "GitHub - …", "… | Vercel"), a signed-in address and
/// notification counts ("(3) …") are dropped. What is left is never empty:
/// a title that is only the site's name stays as it is.
pub fn tidy_tab_title(title: &str, url: &str) -> String {
    let original = collapse_whitespace(title);
    let host = url::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(str::to_ascii_lowercase))
        .unwrap_or_default();
    let labels = site_labels(&host);
    let text = strip_counts(&original);
    let segments = title_segments(text);
    let (mut first, mut last) = (0, segments.len() - 1);
    let part = |index: usize| &text[segments[index].0..segments[index].1];
    while first < last && is_site_segment(part(first), &host, &labels, false) {
        first += 1;
    }
    while last > first && is_site_segment(part(last), &host, &labels, true) {
        last -= 1;
    }
    let tidy = strip_counts(&text[segments[first].0..segments[last].1]);
    if tidy.is_empty() {
        original
    } else {
        tidy.to_string()
    }
}

/* --------------------------------------------------------- Tidy Downloads */

const JUNK_STEMS: &[&str] = &[
    "download", "downloads", "file", "document", "untitled", "unknown", "attachment", "export",
    "image", "img", "output", "blob", "temp", "tmp",
];

/// "name (2)" as made for a second download of the same name.
fn strip_copy_suffix(stem: &str) -> &str {
    if let Some(rest) = stem.strip_suffix(')')
        && let Some(open) = rest.rfind(" (")
        && rest[open + 2..].bytes().all(|b| b.is_ascii_digit())
        && open + 2 < rest.len()
    {
        return &rest[..open];
    }
    stem
}

fn is_uuid(text: &str) -> bool {
    let groups: Vec<&str> = text.split('-').collect();
    groups.len() == 5
        && groups.iter().map(|g| g.len()).eq([8, 4, 4, 4, 12])
        && groups.iter().all(|g| g.bytes().all(|b| b.is_ascii_hexdigit()))
}

/// Whether a run of letters reads as a word ("final", "Report", "PDF")
/// rather than part of a random token ("hbGciOiJ").
fn is_word_like(run: &str) -> bool {
    let mut chars = run.chars();
    let first_ok = chars.next().is_some_and(char::is_alphabetic);
    let rest: Vec<char> = chars.collect();
    let cased = rest.iter().all(|c| c.is_lowercase()) || run.chars().all(char::is_uppercase);
    run.chars().count() >= 4
        && first_ok
        && cased
        && run.chars().any(|c| "aeiouyAEIOUY".contains(c))
}

/// Splits "MyFinalReport" into "My", "Final", "Report".
fn camel_words(run: &str) -> Vec<&str> {
    let mut words = Vec::new();
    let mut start = 0;
    let mut previous_lower = false;
    for (at, c) in run.char_indices() {
        if previous_lower && c.is_uppercase() {
            words.push(&run[start..at]);
            start = at;
        }
        previous_lower = c.is_lowercase();
    }
    words.push(&run[start..]);
    words
}

/// Whether a download's name (without its extension) says nothing about the
/// file: "download", "file (2)", numbers ("387494876"), a UUID, a hex hash or
/// a random token.
pub fn is_junk_download_stem(stem: &str) -> bool {
    let stem = strip_copy_suffix(stem.trim());
    let lower = stem.to_lowercase();
    if JUNK_STEMS.contains(&lower.as_str()) || is_uuid(&lower) {
        return true;
    }
    if !stem.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | ' ' | '.')) {
        return false;
    }
    let core: String = stem.chars().filter(char::is_ascii_alphanumeric).collect();
    if core.is_empty() || core.bytes().all(|b| b.is_ascii_digit()) {
        return true;
    }
    let digits = core.bytes().filter(u8::is_ascii_digit).count();
    if core.len() >= 12 && digits > 0 && core.bytes().all(|b| b.is_ascii_hexdigit()) {
        return true;
    }
    let has_word = stem
        .split(|c: char| !c.is_ascii_alphabetic())
        .flat_map(camel_words)
        .any(is_word_like);
    core.len() >= 16 && digits >= 2 && core.len() - digits >= 3 && !has_word
}

/// The name, cleaned for a file name: no path separators, colons, control
/// characters or leading dots (a hidden file), whitespace collapsed, at most
/// `MAX_TIDY_NAME_CHARS` characters.
fn clean_name_part(text: &str) -> String {
    let replaced: String = text
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '|' => '-',
            '"' | '*' | '?' | '<' | '>' => ' ',
            c if c.is_control() => ' ',
            c => c,
        })
        .collect();
    let collapsed = collapse_whitespace(&replaced);
    let trimmed = collapsed.trim_start_matches(['.', '-', ' ']).trim_end_matches([' ', '.']);
    let short: String = trimmed.chars().take(MAX_TIDY_NAME_CHARS).collect();
    short.trim_end_matches([' ', '.', '-']).to_string()
}

/// Splits "a.tar.gz" style names at the last dot when what follows looks
/// like an extension (1-10 letters or digits).
fn split_extension(file_name: &str) -> (&str, Option<&str>) {
    match file_name.rfind('.') {
        Some(dot)
            if dot > 0
                && (1..=10).contains(&(file_name.len() - dot - 1))
                && file_name[dot + 1..].bytes().all(|b| b.is_ascii_alphanumeric()) =>
        {
            (&file_name[..dot], Some(&file_name[dot + 1..]))
        }
        _ => (file_name, None),
    }
}

/// A readable name for a download with a junk name (see
/// `is_junk_download_stem`): the title of the page it came from (tidied as
/// for a pinned tab) or, without a usable title, the site and `date`, with
/// the original extension. None when the name is fine as it is.
pub fn tidy_download_name(file_name: &str, source_title: &str, source_url: &str, date: &str) -> Option<String> {
    let (stem, extension) = split_extension(file_name);
    if !is_junk_download_stem(stem) {
        return None;
    }
    let title = clean_name_part(&tidy_tab_title(source_title, source_url));
    let usable_title = !title.is_empty()
        && !title.to_ascii_lowercase().starts_with("http")
        && !is_junk_download_stem(&title);
    let base = if usable_title {
        title
    } else {
        let host = url::Url::parse(source_url)
            .ok()
            .and_then(|u| u.host_str().map(|h| h.trim_start_matches("www.").to_string()))
            .unwrap_or_default();
        let site = clean_name_part(&host);
        if site.is_empty() {
            return None;
        }
        format!("{site} {date}")
    };
    let name = match extension {
        Some(ext) => format!("{base}.{ext}"),
        None => base,
    };
    (name != file_name).then_some(name)
}

/// Renames the downloaded file at `path` to `new_name` in the same folder,
/// never replacing a file: an existing name gets " (2)", " (3)", … The new
/// path is made with a hard link (which fails rather than overwrite) before
/// the old name is removed; volumes without hard links fall back to a rename
/// after an existence check. None when the file was left as it was.
pub fn rename_download(path: &Path, new_name: &str) -> Option<PathBuf> {
    let dir = path.parent()?;
    if new_name.is_empty() || new_name.contains('/') || new_name.starts_with('.') {
        return None;
    }
    let (stem, extension) = split_extension(new_name);
    for index in 1..100 {
        let name = match (index, extension) {
            (1, _) => new_name.to_string(),
            (_, Some(ext)) => format!("{stem} ({index}).{ext}"),
            (_, None) => format!("{stem} ({index})"),
        };
        let candidate = dir.join(name);
        if candidate == path {
            return None;
        }
        match fs::hard_link(path, &candidate) {
            Ok(()) => {
                if fs::remove_file(path).is_err() {
                    let _ = fs::remove_file(&candidate);
                    return None;
                }
                return Some(candidate);
            }
            Err(err) if err.kind() == ErrorKind::AlreadyExists => continue,
            Err(_) => {
                if candidate.exists() {
                    continue;
                }
                return fs::rename(path, &candidate).ok().map(|()| candidate);
            }
        }
    }
    None
}

/// Today's local date as "2026-10-29", for download names.
pub fn local_date() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as libc::time_t)
        .unwrap_or(0);
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    if unsafe { libc::localtime_r(&now, &mut tm) }.is_null() {
        return String::new();
    }
    format!("{:04}-{:02}-{:02}", tm.tm_year + 1900, tm.tm_mon + 1, tm.tm_mday)
}

/* ------------------------------------------------------- 5-Second Previews */

/// The address a Shift-hover preview may open: an http(s) URL with a host
/// and no user name or password in it (those disguise where a link goes).
pub fn preview_target(url: &str) -> Option<String> {
    let url = url.trim();
    if url.is_empty() || url.len() > MAX_URL_LEN {
        return None;
    }
    let parsed = url::Url::parse(url).ok()?;
    let web = matches!(parsed.scheme(), "http" | "https")
        && parsed.host_str().is_some_and(|host| !host.is_empty())
        && parsed.username().is_empty()
        && parsed.password().is_none();
    web.then(|| parsed.to_string())
}

/// Whether enough time passed since the last preview for another one.
pub fn preview_cooldown_over(last: Option<Instant>, now: Instant) -> bool {
    last.is_none_or(|last| now.saturating_duration_since(last) >= PREVIEW_COOLDOWN)
}

/// Whether Shift is held down right now (and Command is not), read from the
/// window server, so a page cannot claim it. Shift-hover previews need it.
#[cfg(target_os = "macos")]
pub fn shift_key_held() -> bool {
    #[link(name = "CoreGraphics", kind = "framework")]
    unsafe extern "C" {
        fn CGEventSourceFlagsState(state_id: i32) -> u64;
    }
    const COMBINED_SESSION_STATE: i32 = 0;
    const SHIFT: u64 = 0x0002_0000;
    const COMMAND: u64 = 0x0010_0000;
    let flags = unsafe { CGEventSourceFlagsState(COMBINED_SESSION_STATE) };
    flags & SHIFT != 0 && flags & COMMAND == 0
}

#[cfg(not(target_os = "macos"))]
pub fn shift_key_held() -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_instant_queries_on_commas_and_semicolons() {
        assert_eq!(split_instant_queries("rust book"), vec!["rust book"]);
        assert_eq!(
            split_instant_queries(" rust book, tokio docs ;serde  "),
            vec!["rust book", "tokio docs", "serde"]
        );
        assert_eq!(split_instant_queries("a,,b; ;"), vec!["a", "b"]);
        assert!(split_instant_queries("  , ; ").is_empty());
        assert!(split_instant_queries("").is_empty());
        // At most six tabs.
        assert_eq!(split_instant_queries("1,2,3,4,5,6,7,8").len(), MAX_INSTANT_QUERIES);
        // A web address keeps its commas.
        assert_eq!(
            split_instant_queries("https://example.com/a,b;c"),
            vec!["https://example.com/a,b;c"]
        );
    }

    #[test]
    fn builds_instant_link_urls() {
        let config = BrowserConfig::default();
        assert_eq!(
            instant_link_url("rust book", &config),
            "https://duckduckgo.com/?q=!ducky+rust+book"
        );
        assert_eq!(
            instant_link_url("c++ & you?", &config),
            "https://duckduckgo.com/?q=!ducky+c%2B%2B+%26+you%3F"
        );
        // Whatever the search engine: only DuckDuckGo redirects reliably.
        let google = BrowserConfig {
            search_engine: "google".into(),
            ..BrowserConfig::default()
        };
        assert!(instant_link_url("weather", &google).starts_with("https://duckduckgo.com/?q=!ducky+"));
        // Addresses open as typed.
        assert_eq!(instant_link_url("example.com", &config), "https://example.com");
        assert_eq!(instant_link_url("https://a.example/x", &config), "https://a.example/x");
    }

    #[test]
    fn tidies_tab_titles() {
        let cases = [
            ("(3) Never Gonna Give You Up - YouTube", "https://www.youtube.com/watch?v=x", "Never Gonna Give You Up"),
            ("Pull requests · rust-lang/rust · GitHub", "https://github.com/pulls", "Pull requests · rust-lang/rust"),
            ("GitHub - rust-lang/rust: Empowering everyone", "https://github.com/rust-lang/rust", "rust-lang/rust: Empowering everyone"),
            ("Inbox (12) - me@gmail.com - Gmail", "https://mail.google.com/mail/u/0/", "Inbox"),
            ("Rust (programming language) - Wikipedia", "https://en.wikipedia.org/wiki/Rust", "Rust (programming language)"),
            ("Spider-Man - Wikipedia", "https://en.wikipedia.org/wiki/Spider-Man", "Spider-Man"),
            ("Dashboard | Vercel", "https://vercel.com/dashboard", "Dashboard"),
            ("Stack Overflow - Where Developers Learn", "https://stackoverflow.com/", "Where Developers Learn"),
            ("Untitled document - Google Docs", "https://docs.google.com/document/d/1", "Untitled document"),
            ("[2] Slack | general | Acme", "https://app.slack.com/client/T1", "general | Acme"),
            ("news.ycombinator.com | Hacker News", "https://news.ycombinator.com/", "Hacker News"),
            // At the start only the site's exact name goes.
            ("Navan Itinerary | Navan", "https://app.navan.com/trips/1", "Navan Itinerary"),
        ];
        for (title, url, expected) in cases {
            assert_eq!(tidy_tab_title(title, url), expected, "{title}");
        }
    }

    #[test]
    fn tidy_titles_never_lose_the_whole_title() {
        assert_eq!(tidy_tab_title("YouTube", "https://www.youtube.com/"), "YouTube");
        assert_eq!(tidy_tab_title("Hacker News", "https://news.ycombinator.com/"), "Hacker News");
        assert_eq!(tidy_tab_title("(3)", "https://example.com/"), "(3)");
        assert_eq!(tidy_tab_title("  Two   spaces ", "not a url"), "Two spaces");
        assert_eq!(tidy_tab_title("", "https://example.com/"), "");
        // A dash inside a word is not a separator.
        assert_eq!(tidy_tab_title("Wi-Fi settings", "https://example.com/"), "Wi-Fi settings");
    }

    #[test]
    fn recognizes_junk_download_names() {
        for junk in [
            "download",
            "Download (2)",
            "file",
            "387494876",
            "2024-10-29_1530",
            "3f2504e0-4f89-11d3-9a0c-0305e82c3301",
            "a3f9c2e1b7d4e5f6",
            "a8Fk2LmQ9xZ0pR7tQ1",
            "eyJhbGciOiJIUzI1NiJ9",
        ] {
            assert!(is_junk_download_stem(junk), "{junk}");
        }
        for fine in [
            "Navan Itinerary",
            "installer_2024_10_29_final",
            "Screenshot 2024-10-29 at 15.30",
            "report-q3",
            "IMG_1234",
            "rustup-init",
            "MyDocumentFinalVersion22",
            "résumé",
        ] {
            assert!(!is_junk_download_stem(fine), "{fine}");
        }
    }

    #[test]
    fn names_junk_downloads_after_their_page() {
        let date = "2026-10-29";
        assert_eq!(
            tidy_download_name("387494876.pdf", "Navan Itinerary | Navan", "https://app.navan.com/trips/1", date),
            Some("Navan Itinerary.pdf".into())
        );
        // Characters a file name cannot hold, and no leading dot.
        assert_eq!(
            tidy_download_name("download.zip", ".hidden: a/b", "https://example.com/", date),
            Some("hidden- a-b.zip".into())
        );
        // No usable title: the site and the date.
        assert_eq!(
            tidy_download_name("3f2504e0-4f89-11d3-9a0c-0305e82c3301.csv", "", "https://www.example.com/x", date),
            Some("example.com 2026-10-29.csv".into())
        );
        assert_eq!(
            tidy_download_name("file", "https://example.com/page", "https://example.com/page", date),
            Some("example.com 2026-10-29".into())
        );
        // Good names stay.
        assert_eq!(tidy_download_name("Quarterly Report.pdf", "Page", "https://example.com/", date), None);
        // Nothing to name it after.
        assert_eq!(tidy_download_name("download.bin", "", "", date), None);
        // Long titles are shortened.
        let long = "x".repeat(300);
        let name = tidy_download_name("file.txt", &long, "https://example.com/", date).unwrap();
        assert_eq!(name.chars().count(), MAX_TIDY_NAME_CHARS + 4);
    }

    #[test]
    fn renaming_a_download_never_replaces_a_file() {
        let dir = std::env::temp_dir().join(format!("eclipse-max-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("Report.pdf"), "existing").unwrap();
        let download = dir.join("387494876.pdf");
        fs::write(&download, "new").unwrap();
        let renamed = rename_download(&download, "Report.pdf").unwrap();
        assert_eq!(renamed, dir.join("Report (2).pdf"));
        assert_eq!(fs::read_to_string(dir.join("Report.pdf")).unwrap(), "existing");
        assert_eq!(fs::read_to_string(&renamed).unwrap(), "new");
        assert!(!download.exists());
        // Names that would leave the folder or hide the file are refused.
        assert!(rename_download(&renamed, "../escape.pdf").is_none());
        assert!(rename_download(&renamed, ".hidden").is_none());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn previews_only_open_plain_web_links() {
        assert_eq!(preview_target(" https://example.com/a "), Some("https://example.com/a".into()));
        assert_eq!(preview_target("http://example.com"), Some("http://example.com/".into()));
        for refused in [
            "eclipse://settings",
            "javascript:alert(1)",
            "file:///etc/passwd",
            "data:text/html,hi",
            "https://user:pass@example.com/",
            "https://bank.example@evil.example/",
            "",
        ] {
            assert_eq!(preview_target(refused), None, "{refused}");
        }
        assert_eq!(preview_target(&format!("https://example.com/{}", "a".repeat(9000))), None);
    }

    #[test]
    fn previews_are_rate_limited() {
        let now = Instant::now();
        assert!(preview_cooldown_over(None, now));
        assert!(!preview_cooldown_over(Some(now), now + Duration::from_millis(200)));
        assert!(preview_cooldown_over(Some(now), now + PREVIEW_COOLDOWN));
    }
}
