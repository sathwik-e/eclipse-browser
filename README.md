# Eclipse

A small, fast macOS web browser written in **Rust**. Pages are rendered by the
system **WebKit** (`WKWebView`) through [`tao`](https://github.com/tauri-apps/tao)
and [`wry`](https://github.com/tauri-apps/wry), so the whole app is a few
megabytes and starts in under a tenth of a second.

Eclipse has an Arc-style vertical sidebar that hides itself, Spaces, a
command bar, built-in ad and tracker blocking, Notes and an Easel canvas.

> **Status:** 1.0.1, macOS 11+ only. The app is ad-hoc signed, not notarized
> (see [Installing](#installing)).

---

## Features

- **Vertical tabs in a hide-away sidebar.** Move the pointer to the left edge
  to reveal it. It stays open while you type in it or use a menu, then slides
  away again. Pin it open with ⌘S.
- **Spaces.** Separate tab sets (Work, Personal, …), each with its own theme,
  accent colour and ad-blocking choice. Favourites and pinned tabs show in
  every Space.
- **Command bar (⌘K).** Jump to an open tab, a favourite, a command, or search.
  It runs in the browser's own sidebar, never inside the web page.
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
  - **New Little Eclipse Window (⌥⌘N)** opens an empty one with the address
    field ready: type an address or a search.
  - The bar always shows the real host of the page on screen, with a lock or
    "Not Secure" for plain http; the page has no say in it. Click it (or press
    ⌘L) to see and edit the full address; Escape puts the host back. Only web
    pages load there, never internal `eclipse://` pages.
  - **Sign-in popups** (window.open with popup features, opened with a click)
    get one too, so they keep `window.opener` and can hand their result back
    to the page. Shortcuts such as Reload, Print and Zoom act on the little
    window while it is in front. Ordinary links that open new windows, and
    popups a page opens on its own, still open as tabs.
  - They share the tabs' cookies and sign-ins, user agent and ad blocking.
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
    position, and open http(s) links in new tabs.
  - A page in a Little Eclipse popup window can only open http(s) links in
    new tabs. WebKit tells its messages apart from the tab's own, even when
    it is on the same site. It cannot load internal `eclipse://` resources
    or send its opener tab to internal pages.
  - A Little Eclipse window opened for a link from another app (or with
    ⌥⌘N) has no channel to the browser at all: no page bridge and no
    injected scripts. It only shares the tabs' website data (cookies and
    sign-ins), user agent and content blocker.
  - Only the sidebar and internal pages can change settings, install
    extensions or manage tabs.
- The page bridge never exposes its token to page scripts.
- Settings, notes and extension code are never injected into websites.
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
| ⌥⌘N | New Little Eclipse window |
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
| ⇧⌘Y | Tidy tabs |
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
src/tab.rs         one WKWebView per tab, injected scripts, navigation policy, downloads
src/config.rs      settings model, load/repair/save, settings patches
src/events.rs      event-loop events, IPC commands, UI state types
src/bridge.js      page bridge: find in page, SPA URL, favicon, audio, links
src/shields.js     cosmetic filtering and YouTube ad skipping
src/menu.m         AppKit menu, window styling, edge hover, content blocker,
                   navigation-failure hooks, JavaScript dialogs, Little
                   Eclipse windows
src/ui_assets.rs   eclipse:// protocol serving the embedded pages below
assets/            sidebar, new tab, settings, notes and easel pages; app icon
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
(user gesture, limits, the host on the top bar for loads that never commit,
popup messages, `eclipse://` requests and opener navigation, shortcuts with
and without a dialog up, focus, resizing, links that open tabs, Open in
Eclipse, standalone windows without an opener, and the address field,
including its refusal of `eclipse://`). It builds
`src/menu.m` into a small harness, serves local test pages on ports 8831 and
8832, and opens windows for about two minutes; the harness makes itself the
active app, so leave the keyboard alone while it runs. `GUARD_MALLOC=1` runs
it under `libgmalloc`.

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
