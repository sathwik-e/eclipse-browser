# Eclipse Browser — optimised build

A native macOS browser written in **Rust**, rendered by Apple's **system WebKit**
(`WKWebView`) through `tao` + `wry`. This copy of the project has been audited
and optimised for real browsing rather than demo browsing: native macOS menus,
authenticated chrome IPC, frame-coalesced UI updates, real WebKit history,
per-tab zoom, find-in-page, live shields, and a leaner new-tab/settings UI.

> This is the `Deepseek/` working copy. The original project folder was left
> untouched; see [COPY-NOTES.md](COPY-NOTES.md) for exactly what was copied and
> what was intentionally excluded (build cache / git metadata).

---

## What was improved

### Performance

- **Coalesced chrome updates.** Page events and IPC no longer serialise the
  whole browser state for every single message. Updates are capped at one per
  display frame, and the sidebar skips config/theme/favourites work unless the
  relevant data actually changed.
- **Real WebKit history.** Back/forward now use `WKWebView`'s native
  `goBack:`/`goForward:` and `canGoBack`/`canGoForward` instead of a parallel URL
  list that could drift out of sync on redirects and single-page apps.
- **Background tab throttling.** Child web views use WebKit's inactive
  scheduling throttle, and hidden tabs stop compositing.
- **Smarter hibernation.** Tabs that are loading or audibly playing media are
  never suspended. Background tabs that are quiet are still discarded after the
  configured interval, keeping URL/title/scroll/zoom and restoring instantly.
- **Cheaper UI.** Removed unnecessary 24–40 px backdrop blurs from the
  new-tab surface, reduced the sidebar blur, cut repeated `innerHTML` rebuilds
  of the favourites drawer and settings list, replaced the 10-second clock
  polling loop with a minute-aligned timer, and added
  `prefers-reduced-motion` / `prefers-reduced-transparency` support.
- **No third-party favicon service.** Icons are loaded from the site's own
  `/favicon.ico` (or its declared `<link rel="icon">`), so the browser no longer
  leaks every visited hostname to `icons.duckduckgo.com`.
- **No white flash.** Pages get a dark under-page background while they load.
- **Idle-friendly event loop.** The loop blocks on events; the only periodic
  wake-up is a 10-second maintenance/memory tick.

### Look and feel

- **The supplied Eclipse artwork is now the identity everywhere.** It is the
  macOS app icon, the sidebar brand mark, the New Tab hero, the Preferences
  header, and the favicon for internal pages. One embedded 256 px image is
  served through the internal `eclipse://…/icon` route; the `.icns` is generated
  from a 1024 px source with `sips` + `iconutil`.
- **Slimmer sidebar.** The default width is now **232 px** (was 264) and the
  collapsed rail is **64 px** (was 68), with tighter header, nav, tab and footer
  padding. A stored 264 px width is migrated to the compact default on load, so
  the change is visible without editing the config file.

### Browser behaviour

- **Native macOS main menu** with App/File/Edit/History/View/Window menus. This
  is what makes standard editing shortcuts (`⌘C`, `⌘V`, `⌘X`, `⌘Z`, `⌘A`),
  Quit, Hide, Minimize, and Services actually work like a macOS app.
- **Find in page** (`⌘F`) with a self-contained, shadow-DOM search bar; `⌘G` /
  `⇧⌘G` cycle matches.
- **Per-tab zoom** (`⌘+`, `⌘-`, `⌘0`) using WebKit's native page zoom, retained
  across hibernation.
- **Reopen closed tab** (`⇧⌘T`) with a 25-tab undo stack.
- **Command-click / middle-click links** open in a new (foreground/background)
  tab; links using `target="_blank"` keep WebKit's native new-window flow.
- **Single-page-app URL sync** so the omnibar follows `pushState`,
  `replaceState`, `popstate`, and hash changes without a reload.
- **Audible-tab indicator** and an audio-aware hibernation rule.
- **Downloads** are saved under `~/Downloads` with collision-safe names, and
  show transient success/failure toasts.
- **Safe address handling**: `javascript:`, `file:` and other non-web input goes
  through the search engine instead of the web view.
- **Live shields** can be toggled without reloading the page.
- **Privacy hardening**: chrome IPC is authenticated with a per-web-view random
  128-bit token kept in a closure, so an ordinary website can see
  `window.ipc.postMessage` but cannot drive tabs, settings, downloads, or the
  window. Keyboard shortcuts also require trusted (real user) events.
- **Correct close/select focus**: closing the active tab selects its neighbour,
  selecting a tab focuses the page, and focusing the omnibar focuses the sidebar.

---

## Keyboard shortcuts

| Shortcut | Action |
| :--- | :--- |
| `⌘T` | New tab |
| `⇧⌘T` | Reopen closed tab |
| `⌘W` | Close active tab |
| `⇧⌘W` | Close window |
| `⌘R` | Reload |
| `⌘.` | Stop loading |
| `⌘L` | Focus omnibar |
| `⌘F` | Find in page |
| `⌘G` / `⇧⌘G` | Find next / previous |
| `⌘+` / `⌘-` / `⌘0` | Zoom in / out / reset |
| `⌘[` / `⌘]` | Back / forward |
| `⇧⌘[` / `⇧⌘]` or `⌃Tab` | Previous / next tab |
| `⌘1` … `⌘8`, `⌘9` | Select tab 1…8 / last tab |
| `⇧⌘B` | Toggle sidebar |
| `⌘,` | Preferences drawer |
| `⌥⌘I` | Web Inspector |
| `⌃⌘F` | Enter full screen |
| `⌘Q` | Quit |

`⌘B` and `⌘K` are deliberately **not** captured: websites and web apps use
them for bold and search/command palettes.

---

## Build and run

Requirements: macOS 11+, Rust 1.88+ (edition 2024 with let-chains; tested
with 1.99.1), Xcode Command Line Tools (`clang`, `codesign`).

```bash
# development build
cargo run

# optimised, stripped release build
cargo build --release
./target/release/eclipse

# app bundle + zip + (when permitted) dmg
./scripts/package_macos.sh
open Eclipse.app
```

The app writes settings to `~/.config/eclipse-browser/config.json`. The path is
kept from the original build so existing preferences carry over.

### Smoke-test hooks

These are inert unless the environment variables are set:

```bash
# report startup timing, memory, menu install, and a sidebar UI probe
ECLIPSE_PERF=1 ./Eclipse.app/Contents/MacOS/eclipse

# automatically quit after N milliseconds
ECLIPSE_SMOKE_TEST_MS=5000 ./Eclipse.app/Contents/MacOS/eclipse

# also open a URL through the normal tab-creation path
ECLIPSE_SMOKE_URL=https://example.com ECLIPSE_PERF=1 \
  ECLIPSE_SMOKE_TEST_MS=8000 ./Eclipse.app/Contents/MacOS/eclipse
```

---

## Verified results (this machine)

Measured on macOS 27.2 (arm64), Rust 1.99.1, release profile, while running
inside the harness file sandbox. The sandbox prevents WebKit from writing its
cache directory and screen capture, so first-load numbers are conservative.

| Metric | Result |
| :--- | :--- |
| Release binary | **1.39 MB** (1,389,392 bytes unsigned; 1,381,456 signed in bundle) |
| App bundle | **2.1 MB** |
| Distribution zip | **~1.4 MB** |
| Window + sidebar + first tab created | **67–100 ms** |
| First page finished (new-tab page) | **231–286 ms** |
| `example.com` finished, including startup | **~249–442 ms** |
| App RSS after startup | **~87 MB** |
| App RSS with two tabs / example.com | **~104–107 MB** |
| CPU for a 10 s session including launch | **0.34 s user + 0.10 s sys** |
| Unit tests | **10 passed** |
| `cargo clippy --all-targets -- -D warnings` | **clean** |

The RAM pill in the sidebar reports the **app process** RSS. WebKit's
WebContent processes are separate and are not included in that number, so it is
not a total browser-footprint measurement.

A UI probe also verifies the compact chrome at runtime: sidebar width **232 px**,
no horizontal overflow, and the embedded brand icon loaded at **256×256** in both
the sidebar and New Tab.

---

## Architecture

```text
src/main.rs        tao event loop, window, native menu install, smoke hooks
src/app.rs         tabs, workspaces, IPC validation, state coalescing, commands
src/tab.rs         WKWebView construction, native history, zoom, URL/icon helpers
src/bridge.js      per-page trusted-event bridge, find UI, SPA/media/scroll hooks
src/shields.js     runtime-toggleable cosmetic shields / YouTube ad skipper
src/menu.m         native AppKit menu + full-screen helper
build.rs           compiles menu.m and links Cocoa
assets/            sidebar, new-tab, settings, and brand-icon sources
scripts/           icon generation (sips/iconutil) and app-bundle packaging
```

Each tab is a native child `WKWebView` with its own IPC token. The sidebar is a
separate WebView with a different token. `BrowserApp` runs on the main thread;
WebKit callbacks are forwarded through a `tao` event-loop proxy and applied to
the tab model there.

---

## Known limitations

- **Shields are cosmetic.** They hide ad containers and skip YouTube ads inside
  the page. `wry` does not expose `WKContentRuleList` / network content blockers,
  so this is not a network-level ad blocker.
- **Hibernation drops WebKit's in-memory back/forward stack.** The URL, title,
  scroll position, zoom, pin state, and workspace are preserved; the site's own
  navigation history restarts from the restored URL. This is the cost of
  discarding the `WKWebView` to reclaim memory.
- **One window.** `window.open()` targets become normal tabs; multi-window
  browsing, private windows, and session restore are not implemented.
- **DRM/autoplay preference changes** apply to tabs opened afterwards. Full
  FairPlay support depends on the system WebKit and the streaming site.
- **Ad-hoc signed, not notarised.** If Gatekeeper complains, right-click
  `Eclipse.app` → Open, or remove the quarantine attribute from a locally built
  copy.
- **DMG creation is environment-dependent.** `scripts/package_macos.sh` creates
  an `.dmg` when the OS allows disk-image creation and always produces
  `Eclipse.zip` as a fallback. This harness sandbox blocks image creation.
- The `scripts/standalone_macos.m` fallback is a legacy Objective-C build path
  and does not include the native menu/token-bridge changes or the shared icon
  route; brand images hide themselves gracefully there.

See [OPTIMIZATIONS.md](OPTIMIZATIONS.md) for the detailed audit, changes, and
verification log.
