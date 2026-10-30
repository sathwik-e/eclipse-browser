# Eclipse

A small, fast macOS web browser written in **Rust**. Pages are rendered by the
system **WebKit** (`WKWebView`) through [`tao`](https://github.com/tauri-apps/tao)
and [`wry`](https://github.com/tauri-apps/wry), so the whole app is a few
megabytes and starts in under a tenth of a second.

Eclipse has an Arc-style vertical sidebar that hides itself, Spaces, a
command bar, built-in ad and tracker blocking, Notes and an Easel canvas.

> **Status:** 1.0.2, macOS 11+ only. The app is ad-hoc signed, not notarized
> (see [Installing](#installing)).

---

## Features

- **Vertical tabs in a hide-away sidebar.** Move the pointer to the left edge
  to reveal it. It stays open while you type in it or use a menu, then slides
  away again. Pin it open with ⌘S.
  - Laid out like Arc's: window controls, the sidebar toggle and back /
    forward / reload on top; an address pill that shows the site's host
    (click it to edit the full address, hover it to copy the link);
    favourites as a grid of icon tiles; the Space's name, "+ New Tab" and
    its tabs; and a bottom bar with the Library (Downloads, Reopen Closed Tab,
    Notes, Easel, Preferences, memory use), dots to switch Space and a "+"
    menu (new tab, the New Little Eclipse Window bar, note, easel, Tidy Tabs).
  - Right-click a tab to pin, copy its link or close it. Tabs, tiles and
    menus work from the keyboard.
- **Spaces.** Separate tab sets (Work, Personal, …), each with its own theme,
  accent colour and ad-blocking choice. Favourites and pinned tabs show in
  every Space.
- **Command bar (⌘K).** Jump to an open tab, a favourite, a page from your
  history, a command (copy the page's link, rename a tab, open a Little
  Eclipse window, …), or search. It runs in the browser's own sidebar, never
  inside the web page.
- **Browsing history (⌘Y).** History > Show All History, or History in the
  sidebar's Library menu, opens `eclipse://history`: pages grouped by day,
  with search, remove-one (the × on hover) and Clear browsing history (last
  hour, today, today and yesterday, or all). Click a page to open it here,
  ⌘-click or middle-click for a new tab. Pages visited in tabs and Little
  Eclipse windows are kept for 90 days (up to 10,000 pages) and suggested in
  the command bar, the new tab page and the ⌥⌘N bar, ranked by how often and
  how recently you visited them. "Save browsing history" in Settings > History
  turns recording off.
- **Ad and tracker blocking.**
  - Network-level blocking with a native `WKContentRuleList`.
  - Cosmetic filtering and YouTube ad skipping on top of that.
  - Can be switched off per site from the address bar's site controls.
- **Session restore.** Open tabs and Spaces come back on the next launch.
  Restored tabs stay asleep until you open them.
- **Tab hibernation.** Background tabs that are quiet are discarded after a
  configurable time away from them and restored on demand. Tabs that are
  loading, playing audio or have a sign-in popup open are never discarded.
- **Little Eclipse windows.** Like Arc's Little Arc: a small window with no
  sidebar and no tabs, framed in the current Space's theme colour, with the
  page as a rounded card under a slim top bar.
  - **Links from other apps** open in one, so a link from Mail or Slack
    doesn't pile up tabs. Turn this off in Settings ("Open links from other
    apps in Little Eclipse") to get a new tab instead.
  - **Open in Eclipse (⌘O)**, or the button on the right of the bar, moves the
    page into the browser window as a new tab and closes the little window.
  - **New Little Eclipse Window (⌥⌘N)** shows a floating "Search or Enter
    URL…" bar at the top right of the screen, as in Arc, also while the
    browser window is hidden. It suggests your open tabs and favourites,
    each with the icon the sidebar shows for it; type to filter or to enter
    an address or search, ↑/↓ to choose and Return to open it in a new
    Little Eclipse window. The sidebar's "+" menu
    and the command bar open the same bar. Escape, the × or a
    click elsewhere closes the bar.
  - The top bar looks like Arc's: faint diagonal stripes over the Space's
    colour, an "i" button for site information (whether the connection is
    secure, the host, Copy Link, Open in Eclipse), and the full address of
    the page on screen in a small monospaced font with the host brightest.
    When space runs short the path is shortened first and the host last, and
    plain http always shows "Not Secure". The page has no say in what the bar
    shows. On the right, in Arc's groups: Copy Link | Copy Screenshot (a
    picture of the page to the clipboard) | Web Inspector | Open in Eclipse.
    A window too narrow to keep about forty characters of address (a
    480-point sign-in popup) shows only Copy Link and Open in Eclipse; ⌥⌘I
    still opens the inspector there. Arc's translate, extensions and other
    buttons have no Eclipse counterpart and are left out. Click the
    address (or press ⌘L) to edit it; Escape puts it back. Only web pages
    load there, never internal `eclipse://` pages.
  - **Popups** get one too, so they keep `window.opener` and can hand their
    result back to the page: sign-in popups (window.open with popup
    features) and, as in Arc, any `window.open(url, name)` a click opens.
    Shortcuts such as Reload, Print and Zoom act on the little window while
    it is in front. Links with `target=_blank`, `noopener` windows and
    popups a page opens on its own still open as tabs.
  - They share the tabs' cookies and sign-ins, user agent and ad blocking.
  - Files download as they would from a tab, into `~/Downloads` and the
    Downloads list. A window opened only for a file (a link to a `.zip` from
    Mail, say) closes once the download starts.
- **Max**, Arc's conveniences without an AI service (Settings > Max, each
  on by default and switchable):
  - **Tidy Tabs.** A broom beside New Tab once a Space has more than six tabs
    groups them into categories (Development, Media, Work, …) or by site and
    closes duplicates and blank new tabs (⇧⌘T reopens them one at a time;
    ⇧⌘Y tidies any time).
  - **Instant Links.** Shift+Return on a search in the command bar or the
    address field opens the top result directly; "rust book, tokio docs"
    opens one tab per search (up to six). This uses DuckDuckGo's `!ducky`
    whatever your search engine, as it is the only one that redirects
    straight to the result.
  - **5-Second Previews.** Hold Shift over a link for a moment to open it
    in a Little Eclipse window. The next preview loads into the same window
    while it is open, so there is one at a time.
  - **Tidy Tab Titles.** Pinned tabs and new favorites lose the site's name
    and unread counts ("(3) Inbox - Gmail" shows as "Inbox"). Any tab can be
    renamed with "Rename Tab" in the command bar.
  - **Tidy Downloads.** Files with junk names (`download.pdf`, numbers,
    UUIDs, hashes) are renamed after the tab page that started them (never
    simply the tab in front), or else the site and date (always for a file
    from a Little Eclipse window), without ever replacing a file. The Downloads list shows the
    old name.
  - **ChatGPT in the Command Bar.** Type "chatgpt" or "claude" and press Tab
    to ask; Return opens the question on chatgpt.com or claude.ai in a new
    tab. Nothing is sent before that.
- **Notes and Easel.** A notes page and a sketch-and-cards canvas, stored
  locally.
- **Real browser plumbing:**
  - Native macOS menus and shortcuts, find in page and per-tab zoom.
  - Downloads to `~/Downloads`, printing and JavaScript dialogs.
  - Error pages for failed loads, and automatic recovery when a page's web
    process crashes.
- **Experimental WebExtensions support.** Content scripts and CSS from unpacked
  Chrome extensions, or extensions installed from the Chrome Web Store by ID.

## Privacy and security model

- The browser chrome (sidebar and internal `eclipse://` pages) and ordinary
  websites never share privileges.
- Every web view gets its own random token. Rust also checks **who sent each
  message**, using the frame URL reported by WebKit rather than by the page:
  - A normal website can only report its URL, title icon, audio and scroll
    position, open http(s) links in new tabs, and ask for a Shift-hover link
    preview. Eclipse opens the preview only when Shift really is held (read
    from the system, not the page), for the active tab of the focused
    window, for a plain http(s) link, and at most once every 1.5 seconds.
  - A page in a Little Eclipse popup window can only open http(s) links in
    new tabs. WebKit tells its messages apart from the tab's own, even when
    it is on the same site. It cannot load internal `eclipse://` resources
    or send its opener tab to internal pages.
  - A Little Eclipse window opened for a link from another app (or with
    ⌥⌘N, or for a link preview) has no channel to the browser at all: no page bridge and no
    injected scripts. It only shares the tabs' website data (cookies and
    sign-ins), user agent and content blocker.
  - Only the sidebar and internal pages can change settings, install
    extensions or manage tabs.
- The page bridge never exposes its token to page scripts.
- Settings, notes and extension code are never injected into websites.
- Browsing history can only be read or changed by the `eclipse://history`
  page itself (the Settings page may also clear it, and the new tab page and
  sidebar only get suggestions for what is typed). Websites, Little Eclipse
  windows and other internal pages are refused. It is stored in
  `~/.config/eclipse-browser/history.json`; clearing or removing entries is
  written to disk straight away.
- Favicons come from the site itself, not a third-party favicon service.
- Settings are stored in `~/.config/eclipse-browser/config.json`. An
  unreadable file is backed up instead of being overwritten.

## Installing

Download the `.dmg` from the
[latest release](https://github.com/sathwik-e/eclipse-browser/releases/latest),
open it and drag **Eclipse** into Applications.

Release builds are ad-hoc signed, not notarized. The first time, right-click
Eclipse in Applications and choose **Open**, or run:

```bash
xattr -dr com.apple.quarantine /Applications/Eclipse.app
```

To have links you click in other apps open in Eclipse (in a Little Eclipse
window), make it your default browser: **System Settings > Desktop & Dock >
Default web browser**, then choose Eclipse.

## Building from source

Requirements:

- macOS 11 or later
- Rust 1.88 or later (edition 2024)
- the Xcode Command Line Tools (`clang`, `codesign`)

```bash
# run a development build
cargo run

# optimised release binary
cargo build --release
./target/release/eclipse

# Eclipse.app plus a versioned .dmg and .zip in dist/
./scripts/package_macos.sh
```

`package_macos.sh` options:

| Option | Effect |
| :--- | :--- |
| `--universal` | Build `aarch64` + `x86_64` and merge them with `lipo` (needs both rustup targets). |
| `--install` | Also copy the app to `/Applications`. An existing `Eclipse.app` with a different bundle id (for example the Eclipse IDE) is never replaced. |
| `ECLIPSE_SIGN_IDENTITY=…` | Codesign identity; defaults to ad-hoc (`-`). |

Checks used by CI (`.github/workflows/ci.yml`):

```bash
cargo test
cargo clippy --all-targets -- -D warnings
```

## Keyboard shortcuts

| Shortcut | Action |
| :--- | :--- |
| ⌘T / ⇧⌘T | New tab / reopen closed tab |
| ⌥⌘N | New Little Eclipse window (search or enter an address) |
| ⌘O | Open in Eclipse (move the Little Eclipse window in front into a tab) |
| ⌘W / ⇧⌘W | Close tab / close window |
| ⌘K | Command bar |
| ⌘L | Open location (focus the address bar) |
| ⌘R / ⌘. | Reload / stop |
| ⌘[ / ⌘] | Back / forward |
| ⇧⌘[ / ⇧⌘] or ⌃⇧Tab / ⌃Tab | Previous / next tab |
| ⌘1 … ⌘8 / ⌘9 | Select tab 1 … 8 / last tab (in sidebar order) |
| ⌘F / ⌘G / ⇧⌘G | Find / next / previous |
| ⌘+ / ⌘- / ⌘0 | Zoom in / out / actual size |
| ⌘S or ⇧⌘B | Toggle (pin) the sidebar |
| ⌘D | Bookmark tab as a favourite |
| ⌘J | Downloads |
| ⇧⌘N / ⇧⌘E | New note / new easel |
| ⌘Y | Show all history |
| ⇧⌘Y | Tidy tabs |
| ⇧↩ (command bar, address field) | Open the top result (Instant Links) |
| "chatgpt" or "claude", then Tab (command bar) | Ask ChatGPT or Claude |
| Shift + point at a link | Preview it in a Little Eclipse window |
| ⌘P | Print |
| ⌘, | Settings |
| ⌥⌘I | Web Inspector |
| ⌃⌘F | Enter full screen |

Websites see a key first. If a site handles a shortcut itself (for example ⌘K
in a web app), the site wins.

## Architecture

```text
src/main.rs        tao event loop, window, native menu, smoke-test hooks
src/app.rs         tabs, Spaces, sessions, IPC trust model, state push to the sidebar
src/max.rs         Max: title and download-name tidying, Instant Links, previews
src/history.rs     browsing history: store, retention, search, frecency suggestions
src/tab.rs         one WKWebView per tab, injected scripts, navigation policy, downloads
src/config.rs      settings model, load/repair/save, settings patches
src/events.rs      event-loop events, IPC commands, UI state types
src/bridge.js      page bridge: find in page, SPA URL, favicon, audio, links,
                   Shift-hover link previews
src/shields.js     cosmetic filtering and YouTube ad skipping
src/menu.m         AppKit menu, window styling, edge hover, content blocker,
                   navigation-failure hooks, JavaScript dialogs, Little
                   Eclipse windows
src/ui_assets.rs   eclipse:// protocol serving the embedded pages below
assets/            sidebar, new tab, settings, notes, easel and history pages; app icon
scripts/           packaging (app bundle, dmg, zip), icon generation and the
                   Little Eclipse end-to-end checks
```

`BrowserApp` runs on the main thread. WebKit callbacks reach it through a `tao`
event-loop proxy. Updates to the sidebar are coalesced to one per display
frame, and large collections (notes, easels, downloads) are only sent when
they change.

### Smoke-test hooks

These do nothing unless the environment variable is set:

```bash
ECLIPSE_PERF=1                # print startup timing and UI probes
ECLIPSE_SMOKE_TEST_MS=8000    # quit automatically after N ms
ECLIPSE_SMOKE_URL=https://example.com   # open a URL through the normal tab path
ECLIPSE_SMOKE_LITTLE_URL=https://example.com   # open a URL as a link from another app
```

`scripts/little-eclipse-e2e/run.sh` checks Little Eclipse windows end to end
(user gesture, which window.open calls get a window and which links and
`noopener` windows get tabs, limits, the address on the top bar for loads
that never commit, popup messages, `eclipse://` requests and opener
navigation, shortcuts with and without a dialog up, focus, resizing, links
that open tabs, Open in Eclipse, Copy Link, Copy Screenshot, the Web
Inspector button and the site-information popover,
standalone windows without an opener, the address field, including its
refusal of `eclipse://`, "Not Secure" and the host kept on long addresses,
the ⌥⌘N bar and its site icons, and downloads from standalone windows, popups,
links and the tab itself, marked as the window's or the tab's). It builds
`src/menu.m` into a small harness, serves local test pages on ports 8831 and
8832, and opens windows for about two minutes; the harness makes itself the
active app, so leave the keyboard alone while it runs. `GUARD_MALLOC=1` runs
it under `libgmalloc`; `SNAPSHOT_DIR=<dir>` saves a picture of each window at
the end of a scenario (`HARNESS_LIGHT=1` for the light theme).

## Known limitations

- **macOS only.**
- **Hibernation drops a tab's back/forward list.** The URL, title, scroll
  position and zoom are kept.
- **One browser window.** There are no private windows yet.
- **WebExtensions support is partial.** Only content scripts and styles are
  supported, not background pages, popups or the full `chrome.*` API.
- **Not notarized.** See [Installing](#installing).

## License

[MIT](LICENSE) © 2026 Sathwik Elaprolu
