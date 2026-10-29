# Changelog

## 1.0.1 — 2026-10-29

### Added
- **Little Eclipse windows, like Arc's Little Arc.** A small window with no
  sidebar and no tabs, framed in the current Space's theme colour (light or
  dark), with the page as a rounded card under a slim top bar.
  - **Links from other apps** open in a Little Eclipse window of their own
    when Eclipse is the default browser (System Settings > Desktop & Dock >
    Default web browser). The app now declares itself a handler for `http`
    and `https` links. A new setting, "Open links
    from other apps in Little Eclipse" (on by default), opens them as a new
    tab instead. Links that arrive while Eclipse is still starting are
    opened once it is up.
  - **Open in Eclipse** (⌘O, File menu, or the button on the right of the
    bar) moves the page into the browser window as a new tab, brings the
    window forward and closes the little window. For a sign-in popup this
    drops its link to the page that opened it, as in Arc.
  - **New Little Eclipse Window** (⌥⌘N) opens an empty one with the address
    field focused.
  - Click the host on the bar (or press ⌘L) to see and edit the full
    address. Return loads a web address as typed, or searches for anything
    else with your search engine; Escape or clicking away puts the host back.
    Internal `eclipse://` pages and other schemes are never loaded.
  - These windows share the tabs' cookies and sign-ins, the Space's user
    agent and its ad blocking, but get no page scripts and no channel to the
    browser. They count toward the limit of ten Little Eclipse windows (past
    it, the link opens as a tab) and are not tied to any tab. A new one opens
    in the middle of the screen with the pointer, at the size you last gave
    one.
  - **Downloads** work in every Little Eclipse window as in a tab: a file the
    window cannot show, or a link marked `download`, goes to `~/Downloads`
    and the Downloads list. A window that was opened only for the file (a
    link to a `.zip` or `.dmg` from Mail or Slack, or a popup that turned out
    to be a file) closes once the download starts, rather than staying empty.
- **Little Eclipse windows for sign-in popups.** When a web page opens a
  popup window (`window.open` with a size, a position, the `popup` feature
  or turned-off toolbars, as sign-in and payment flows do) in response to a
  click or key press, including shortly after one (a timer or a `fetch`), it
  now opens in a small
  native window instead of a tab. The popup keeps `window.opener`, so it can
  post its result back and close itself, which makes "Sign in with …" popups
  work.
  - The top bar shows a lock and the host of the page actually on screen,
    or "Not Secure" for plain http. When the host is too long for the bar it
    loses its start (the end, the site's own domain, stays) and "Not Secure"
    always stays in view. The page has no control over it: a page
    that is still loading, or that never finishes loading, is not shown as
    the host until it is actually on screen. The window title (Mission
    Control, the Window menu) is the page title.
  - JavaScript dialogs appear as sheets on the popup. `window.close()` closes
    it, and ⌘W closes it while it is in front, also while it shows a dialog.
  - While a popup is in front, Reload, Stop, Back, Forward, Print, Zoom and
    the Web Inspector act on the popup. Bookmark, Home and Find do nothing
    there rather than act on the hidden tab behind it. Other shortcuts (New
    Tab, Open Location, …) bring the browser window to the front first.
  - `popup.focus()` and reopening a popup by name bring it to the front.
    `resizeTo`/`moveTo` work within the popup's size limits, and the page
    sees the window's real size and position.
  - A link clicked in a popup opens in a new tab and brings the browser
    window forward. A tab a popup's script opens stays behind it.
  - A popup can only show web pages, never internal `eclipse://` pages, and it
    closes when the tab that opened it is closed or hibernated. A tab with a
    popup open is never hibernated automatically or by "Clean memory".
  - A popup's size is that of its page; popups that give no size get a
    500×640 page, or keep the one side they gave.
  - Ordinary links that open a new window (`target=_blank`, or
    `window.open(url)` with no features), popups from internal pages, popups a page opens
    without a click or key press, and popups beyond three per tab (ten in
    all) still open as tabs. Such a popup with no address (the page meant to
    write into it) shows "Pop-up window blocked" instead of leaving an empty
    tab behind.

### Security
- A page in a popup window is never taken for the tab that opened it. The
  popup posts through its opener tab's channel, but Eclipse marks its messages,
  so even a popup on the tab's own site (an OAuth callback page, say) cannot
  change the tab's address, icon, scroll position or audio state, and it
  cannot load internal `eclipse://` resources. It can still open http(s)
  links in new tabs.
- A popup cannot use `window.opener` to send its opener tab to internal
  `eclipse://` pages, not even the new-tab page (from there it could have
  walked the tab on to Settings, Notes or Easel).
- Websites' reports about a tab's page (favicon, audio, scroll position) are
  now accepted only from a frame on the same origin as the tab's document.
- A Little Eclipse window opened for a link from another app is built
  without the tab's page bridge or scripts, so its page cannot message the
  browser at all. Its address field loads only http(s) pages.

### Fixed
- Tab hibernation counts a tab's idle time from when you leave it, not from
  when you last selected it, so a tab you read for a long time is not
  discarded seconds after you switch away.

## 1.0.0 — 2026-10-28

First public release. This version is the result of a full audit of the
0.2 codebase, which found 137 issues across the core, page scripts, native
layer, UI and packaging.

### Security
- Websites can no longer drive the browser. Each message is authorised by its
  sender (sidebar, internal page or website), using the frame URL that WebKit
  reports. Websites get only a small allowlist of commands.
- The page bridge no longer leaks its IPC token. Earlier versions let a page
  steal it through `Object.prototype` setters, `toJSON` hooks or wrapped
  functions.
- The bridge no longer aborts on Trusted Types sites such as YouTube.
- Settings, notes and extension source code are no longer injected into web
  pages. Neither is the Space name.
- Websites can't navigate a tab to internal `eclipse://` pages.
- The sidebar can't be navigated away from its own document. Dropped links
  open in new tabs.
- Internal pages are served with a restrictive CSP and no wildcard CORS.
- HTML injection in the sidebar and Settings is fixed: all untrusted strings
  are rendered as text.
- Extension files must stay inside the extension folder.
- Hardened-runtime entitlements are cut down to camera and microphone.

### Fixed
- The network ad blocker never compiled, because WebKit rejects `|` in
  content-blocker regexes. Rules are now one per domain, limited to third-party
  loads, and cached between launches.
- Failed loads spun forever. They now show an error page with Try Again, and
  crashed pages reload automatically.
- Pages without a background colour of their own rendered as black text on a
  dark background.
- Downloads often never finished in the UI, and some legitimate files were
  silently cancelled.
- Saving Settings sent a stale copy of the whole config, which reverted
  changes made elsewhere and failed silently once the config passed 64 KB.
  Settings now send small patches.
- Sidebar updates could stall until an unrelated event arrived, because the
  event loop's wake-up deadline was overwritten.
- The hover sidebar collapsed while you were typing in it.
- ⌘←/⌘→ were hijacked in text fields, and page-level shortcut capture fought
  with websites. The native menu now owns all shortcuts.
- ⌘1–⌘9 and next/previous tab now follow the order shown in the sidebar.
- The bottom toolbar overflowed and hid the New Tab button.
- The space selector got stuck. Pinned tabs on subdomains disappeared. The
  extension Remove button crashed.
- The Easel's Sketch mode never drew. Notes and Easel lost edits when you
  switched documents or closed the tab. Easel cards could be dragged off
  screen.
- A config file with one bad field reset everything. Deleted favourites,
  notes and boards came back on restart.
- "Clean memory" did nothing. Tidy Tabs could not be undone.
- `alert()`, `confirm()`, `prompt()` and Print did nothing.
- Fullscreen video was treated as window fullscreen. The sidebar was
  unreachable in full screen.

### Added
- Session restore.
- The command bar now lives in the browser sidebar instead of the web page.
- Error pages and web-process crash recovery.
- Native menu items for every shortcut.
- A `--universal` packaging flag, versioned `dist/` artifacts, and CI.

### Performance
- Config is only pushed when it changes, and large collections are no longer
  re-sent on every UI update.
- The idle New Tab page no longer runs paint-heavy animations.
- The edge-hover poller no longer wakes the CPU 31 times a second.
- Resizing only re-lays out visible tabs.
- Extension installs no longer freeze the UI.

### Removed
- The in-page "Little Arc" iframe preview and its Shift-click hijack.
- Unused Helium/ungoogled-chromium resources.
- The legacy Objective-C fallback build.
- The packaging step that silently replaced `/Applications/Eclipse.app`.
