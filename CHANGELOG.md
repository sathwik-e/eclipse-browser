# Changelog

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
