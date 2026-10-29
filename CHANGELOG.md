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
  - **New Little Eclipse Window** (⌥⌘N) works like Arc's: a floating
    "Search or Enter URL…" bar near the top right of the screen (also while
    the browser window is hidden or another window is in front) suggests
    your open tabs (this Space's first, most recent first) and favourites,
    each with its site's initial and host. Typing filters them and puts what
    you typed first; ↑/↓ choose (the chosen row in your accent colour) and
    Return opens it in a new Little Eclipse window, loading an address as
    typed or searching for anything else. Escape, the × button, ⌘W or a
    click anywhere else closes the bar.
  - **The top bar looks like Arc's.** Faint diagonal stripes over the
    Space's colour; on the left an "i" button and the full address of the
    page on screen in a small monospaced font, the host brighter than the
    scheme and path; on the right Copy Link (with a brief "Copied") and Open
    in Eclipse. The "i" button shows whether the connection is secure, the
    full host, Copy Link and Open in Eclipse; for plain http it is an orange
    warning sign.
  - Click the address on the bar (or press ⌘L) to edit it. Return loads a
    web address as typed, or searches for anything else with your search
    engine; Escape or clicking away puts the address back. Internal
    `eclipse://` pages and other schemes are never loaded.
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
- **Max**, Arc's Max features done without an AI service. A new Max section
  in Settings switches each one (all on by default):
  - **Tidy Tabs:** the Tidy button, now a broom, shows in the sidebar once
    the Space has more than six tabs. ⇧⌘Y and the command bar tidy at any
    time, and ⇧⌘T still brings back a duplicate it closed.
  - **Instant Links:** Shift+Return on a search in the command bar or the
    address field goes straight to the top result, through DuckDuckGo's
    `!ducky` whatever the search engine (Google's "I'm Feeling Lucky" often
    stops on a redirect notice). Searches separated by commas or semicolons
    open one tab each, up to six. A web address opens as typed.
  - **5-Second Previews:** holding Shift over a link for about 0.6 s opens it
    in a Little Eclipse window. No summary is made.
  - **Tidy Tab Titles:** pinned tabs and new favorites show their title
    without the site's name, a signed-in address or unread counts
    ("(3) Never Gonna Give You Up - YouTube" shows as "Never Gonna Give You
    Up"). **Rename Tab** in the command bar gives any tab a name of its own
    (an empty name gives the page title back); names survive a restart.
  - **Tidy Downloads:** a finished download with a junk name (`download`,
    `file`, numbers, a UUID, a hash or random token) is renamed after the
    page in front when it started, or after the site and the date, keeping
    its extension. It never replaces a file (an existing name gets " (2)"),
    and the Downloads list shows "Renamed from …".
  - **ChatGPT in the Command Bar:** type "chatgpt" or "claude" (or the start
    of it) and press Tab to ask a question; Return opens it on chatgpt.com or
    claude.ai in a new tab. The command bar also offers "Ask ChatGPT" and
    "Ask Claude" rows for a search. Nothing is sent until you choose one.
- **Little Eclipse windows for sign-in popups.** When a web page opens a
  popup window (`window.open` with a size, a position, the `popup` feature
  or turned-off toolbars, as sign-in and payment flows do) in response to a
  click or key press, including shortly after one (a timer or a `fetch`), it
  now opens in a small
  native window instead of a tab. The popup keeps `window.opener`, so it can
  post its result back and close itself, which makes "Sign in with …" popups
  work.
  - The top bar shows the address of the page actually on screen (never a
    user name or password in it), with "Not Secure" in front for plain http.
    When it does not fit, the path is shortened first, then the scheme goes,
    and only then does a long host lose its start (the end, the site's own
    domain, stays); "Not Secure" always stays in view, and a site name in
    the path is never drawn like the host. The page has no control over it:
    a page that is still loading, or that never finishes loading, is not
    shown until it is actually on screen. The window title (Mission Control,
    the Window menu) is the page title.
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
  - As in Arc, a `window.open(url, name)` with no features at all also
    opens in a Little Eclipse window when a click opens it, so it keeps
    `window.opener` too (WebKit can keep the opener only for a window of its
    own, not for a tab).
  - Links that open a new window (`target=_blank`), `window.open(url,
    '_blank', 'noopener')`, popups from internal pages, popups a page opens
    without a click or key press, and popups beyond three per tab (ten in
    all) still open as tabs. Such a popup with no address (the page meant to
    write into it) shows "Pop-up window blocked" instead of leaving an empty
    tab behind.

### Changed
- **The sidebar is laid out like Arc's.**
  - The top row has the window controls, the sidebar toggle right after
    them, and back, forward and reload on the right.
  - The address field is a rounded "Search or Enter URL…" pill. It shows
    only the host of the page on screen (in punycode for look-alike
    domains), with the lock beside it; click it to edit the full address.
    Hovering it shows a Copy Link button.
  - Favourites are a three-column grid of large rounded tiles showing just
    the site's icon; the name is the tooltip. More favourites wrap onto
    more rows, the selected one is highlighted, and the grid disappears
    when there are none.
  - Below the grid: the Space's name and icon (click for the next Space),
    "+ New Tab" (Tidy appears beside it on hover) and the Space's tabs, one
    line each, with the host in the tooltip.
  - The bottom bar has the Library at the left (Downloads with the number
    still downloading, Reopen Closed Tab, Quick Notes, Easel, Preferences
    and memory use), a dot per Space to switch between them, and a "+"
    menu at the right: New Tab, Little Eclipse window, Note, Easel and Tidy
    Tabs. These replace the row of icons and the Space buttons.
  - Right-clicking a tab or a pinned tab's tile opens a menu to pin or unpin
    it, copy its link or close it.
  - Light (Daylight Sol) and dark themes both use the theme colours for the
    new menus and for the site controls popover, which was dark on the
    light theme before.
- The command bar has New Little Eclipse Window and Copy Link.

### Accessibility
- Every sidebar button has a name for VoiceOver, keyboard focus is always
  visible, and tabs, favourite tiles and the new menus work from the
  keyboard (Return or Space to open, arrow keys and Escape in menus).

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
- The one new command websites may send, `preview_link` (5-Second
  Previews), opens nothing on the page's say-so: Eclipse reads the Shift key
  from the system itself, and needs the setting on, the message from the
  active tab's own document in the focused browser window, a plain http(s)
  link without a user name or password, and 1.5 seconds since the last
  preview. Popups and departing internal pages may not send it. Instant
  Links and Rename Tab are the browser's own commands, never a page's.
- A Little Eclipse window opened for a link from another app is built
  without the tab's page bridge or scripts, so its page cannot message the
  browser at all. Its address field loads only http(s) pages.

### Fixed
- Tab hibernation counts a tab's idle time from when you leave it, not from
  when you last selected it, so a tab you read for a long time is not
  discarded seconds after you switch away.
- The site controls popover cut the site's name down to its first letter
  next to "Secure Connection (HTTPS)", and squeezed "Space Profile" into a
  few letters in a narrow sidebar. It now opens right under the address
  pill wherever that is.

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
