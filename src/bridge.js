/*
 * Eclipse content bridge.
 *
 * Runs in the main frame before any page script, but in the page's own JS
 * world: anything reachable from `window` (prototypes, JSON, window.ipc) can
 * be hooked by the page later on. So every primitive used at post time is
 * captured here, at document start, payloads are null-prototype objects, and
 * the token is spliced into the JSON text instead of being put on an object.
 * Rust still decides what each message may do from the posting frame's URL
 * (reported by WebKit, not by the page), so a website can only ever report
 * its own scroll, favicon, audio and URL, and open http(s) links in tabs.
 * Internal eclipse:// pages get a narrow helper so their own UI can post.
 */
(function () {
  'use strict';
  if (window.__eclipseBridgeActive) return;
  try {
    Object.defineProperty(window, '__eclipseBridgeActive', { value: true });
  } catch (_) {
    return;
  }

  var TOKEN = '__ECLIPSE_TOKEN__';
  var CONTROL = '__ECLIPSE_CONTROL__';
  var internal = location.protocol === 'eclipse:';

  /* ------------------------------------------------------------ primordials */
  var stringify = JSON.stringify;
  var createObject = Object.create;
  var objectKeys = Object.keys;
  var defineProperty = Object.defineProperty;
  var bind = Function.prototype.bind;
  var strSlice = bind.call(Function.prototype.call, String.prototype.slice);
  // Captured so a page cannot hook it to grab our closed shadow roots.
  var nativeAttachShadow =
    window.Element && typeof Element.prototype.attachShadow === 'function'
      ? bind.call(Function.prototype.call, Element.prototype.attachShadow)
      : null;
  function attachShadow(host, init) {
    return nativeAttachShadow ? nativeAttachShadow(host, init) : host.attachShadow(init);
  }
  var rawPost = null;
  try {
    var handler = window.webkit && window.webkit.messageHandlers && window.webkit.messageHandlers.ipc;
    if (handler && typeof handler.postMessage === 'function') {
      rawPost = bind.call(handler.postMessage, handler);
    }
  } catch (_) {}
  if (!rawPost && window.ipc && typeof window.ipc.postMessage === 'function') {
    rawPost = bind.call(window.ipc.postMessage, window.ipc);
  }

  function message(action) {
    var payload = createObject(null);
    payload.action = action;
    return payload;
  }

  // `payload` must be a null-prototype object built by this closure, so
  // stringify() never consults a page-defined toJSON or accessor.
  function post(payload) {
    if (!rawPost) return;
    var body;
    try {
      body = stringify(payload);
    } catch (_) {
      return;
    }
    if (typeof body !== 'string' || body[0] !== '{') return;
    var text =
      body === '{}'
        ? '{"token":"' + TOKEN + '"}'
        : '{"token":"' + TOKEN + '",' + strSlice(body, 1);
    try {
      rawPost(text);
    } catch (_) {}
  }

  /* -------------------------------------------------- in-page control hooks */
  // Defined before anything else can fail. Rust calls these with CONTROL,
  // which is not the IPC token, so a page that defines a look-alike first
  // learns nothing it could post with.
  try {
    defineProperty(window, '__eclipseFind', {
      value: function (secret, action) {
        if (secret !== CONTROL) return;
        if (action === 'open') openFind();
        else if (action === 'next') {
          openFind();
          runFind(false);
        } else if (action === 'prev') {
          openFind();
          runFind(true);
        } else if (action === 'close') closeFind();
      },
      writable: false,
      configurable: false,
      enumerable: false
    });
  } catch (_) {}

  try {
    defineProperty(window, '__eclipseSetLoading', {
      value: function (isLoading) {
        setLoadingState(!!isLoading);
      },
      writable: false,
      configurable: false,
      enumerable: false
    });
  } catch (_) {}

  if (internal) {
    try {
      defineProperty(window, '__eclipseIpc', {
        value: function (payload) {
          if (!payload || typeof payload !== 'object') return;
          var copy = createObject(null);
          var keys = objectKeys(payload);
          for (var i = 0; i < keys.length; i++) {
            if (keys[i] !== 'token') copy[keys[i]] = payload[keys[i]];
          }
          post(copy);
        },
        writable: false,
        configurable: false,
        enumerable: false
      });
    } catch (_) {}

    var reportChromeError = function (text) {
      var payload = message('chrome_error');
      payload.message = strSlice(String(text || ''), 0, 800);
      post(payload);
    };
    window.addEventListener(
      'error',
      function (event) {
        if (event && event.message) {
          reportChromeError(event.message + ' (' + (event.filename || '') + ':' + (event.lineno || 0) + ')');
        }
      },
      true
    );
    window.addEventListener('unhandledrejection', function (event) {
      var reason = event && event.reason;
      reportChromeError(reason && reason.message ? reason.message : String(reason || 'Unhandled rejection'));
    });
    // Internal pages are served with a strict CSP (ui_assets.rs); surface
    // anything it blocks instead of failing silently.
    document.addEventListener('securitypolicyviolation', function (event) {
      reportChromeError('CSP blocked ' + (event.blockedURI || 'inline') + ' (' + event.violatedDirective + ')');
    });
  }

  function isHttpUrl(value) {
    return typeof value === 'string' && /^https?:/i.test(value);
  }

  /* ---------------------------------------------------------------- scroll */
  var scrollTimer = null;
  var lastScrollX = window.scrollX || 0;
  var lastScrollY = window.scrollY || 0;

  function reportScroll() {
    var x = window.scrollX || 0;
    var y = window.scrollY || 0;
    if (Math.abs(x - lastScrollX) < 1 && Math.abs(y - lastScrollY) < 1) return;
    lastScrollX = x;
    lastScrollY = y;
    var payload = message('update_scroll');
    payload.x = x;
    payload.y = y;
    post(payload);
  }

  window.addEventListener(
    'scroll',
    function () {
      if (scrollTimer) clearTimeout(scrollTimer);
      scrollTimer = setTimeout(reportScroll, 300);
    },
    { passive: true }
  );
  window.addEventListener('pagehide', reportScroll, true);

  /* ----------------------------------------------------------------- audio */
  var playing = new Set();
  var lastAudible = false;

  function reportAudible() {
    var audible = playing.size > 0;
    if (audible === lastAudible) return;
    lastAudible = audible;
    var payload = message('set_audible');
    payload.audible = audible;
    post(payload);
  }

  function isMedia(el) {
    return el && (el.tagName === 'VIDEO' || el.tagName === 'AUDIO');
  }

  function isPlaying(el) {
    return isMedia(el) && !el.paused && !el.ended && !el.muted && Number(el.volume) > 0;
  }

  // Re-evaluates one element. Covers unmuting an already playing (autoplay
  // muted) video as well as muting or pausing an audible one.
  function updatePlaying(el) {
    if (!isMedia(el)) return;
    if (isPlaying(el)) playing.add(el);
    else playing.delete(el);
    reportAudible();
  }

  ['play', 'playing', 'pause', 'ended', 'volumechange', 'emptied'].forEach(function (type) {
    document.addEventListener(
      type,
      function (event) {
        updatePlaying(event.target);
      },
      true
    );
  });
  window.addEventListener(
    'pagehide',
    function () {
      if (!playing.size) return;
      playing.clear();
      reportAudible();
    },
    true
  );

  /* ------------------------------------------------------------- favicons */
  var lastFavicon = '';
  var faviconTimer = null;

  // Prefers real favicons (rel "icon" / "shortcut icon") over touch icons,
  // and Safari's monochrome mask-icon only as a last resort.
  function faviconRank(link) {
    var rel = ' ' + String(link.getAttribute('rel') || '').toLowerCase().replace(/\s+/g, ' ') + ' ';
    if (rel.indexOf(' icon ') !== -1) return 0;
    if (rel.indexOf(' apple-touch-icon ') !== -1 || rel.indexOf(' apple-touch-icon-precomposed ') !== -1) return 1;
    if (rel.indexOf(' mask-icon ') !== -1) return 2;
    return -1;
  }

  function detectFavicon() {
    if (internal || !document.head) return;
    var links = document.head.querySelectorAll('link[rel][href]');
    var best = null;
    var bestRank = 99;
    for (var i = 0; i < links.length; i++) {
      var rank = faviconRank(links[i]);
      if (rank >= 0 && rank < bestRank) {
        best = links[i];
        bestRank = rank;
      }
    }
    if (!best) return;
    try {
      var href = new URL(best.getAttribute('href'), location.href).href;
      if (!isHttpUrl(href) || href === lastFavicon) return;
      lastFavicon = href;
      var payload = message('update_favicon');
      payload.favicon = href;
      post(payload);
    } catch (_) {}
  }

  function scheduleFaviconScan(delay) {
    if (internal) return;
    if (faviconTimer) clearTimeout(faviconTimer);
    faviconTimer = setTimeout(detectFavicon, delay);
  }

  // Single-page apps swap their favicon after load (unread badges, routing).
  function observeFavicon() {
    if (internal || !document.head || typeof MutationObserver !== 'function') return;
    try {
      new MutationObserver(function (records) {
        for (var i = 0; i < records.length; i++) {
          var record = records[i];
          if (record.type === 'attributes' || record.addedNodes.length || record.removedNodes.length) {
            scheduleFaviconScan(250);
            return;
          }
        }
      }).observe(document.head, {
        childList: true,
        subtree: true,
        attributes: true,
        attributeFilter: ['href', 'rel']
      });
    } catch (_) {}
  }

  function onDomReady() {
    detectFavicon();
    scheduleFaviconScan(1200);
    observeFavicon();
  }

  if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', onDomReady, { once: true });
  } else {
    onDomReady();
  }

  /* ------------------------------------------------- single-page-app URLs */
  var lastReportedUrl = location.href;

  function reportUrl() {
    if (internal) return;
    var href = location.href;
    if (!href || href === lastReportedUrl || !isHttpUrl(href)) return;
    lastReportedUrl = href;
    var payload = message('url_changed');
    payload.url = href;
    post(payload);
    scheduleFaviconScan(400);
  }

  try {
    var originalPushState = history.pushState;
    var originalReplaceState = history.replaceState;
    history.pushState = function () {
      var result = originalPushState.apply(this, arguments);
      reportUrl();
      return result;
    };
    history.replaceState = function () {
      var result = originalReplaceState.apply(this, arguments);
      reportUrl();
      return result;
    };
  } catch (_) {}
  window.addEventListener('popstate', reportUrl, true);
  window.addEventListener('hashchange', reportUrl, true);
  window.addEventListener('pageshow', reportUrl, true);

  /* ---------------------------------------------------------- DOM helpers */
  // DOM is built node by node: Trusted Types sites (YouTube) throw on any
  // HTML string sink, which would abort the rest of this script.
  function el(tag, className, text) {
    var node = document.createElement(tag);
    if (className) node.className = className;
    if (text) node.textContent = text;
    return node;
  }

  function styleSheet(cssText) {
    var style = document.createElement('style');
    style.textContent = cssText;
    return style;
  }

  /* ------------------------------------ Celestial Eclipse Loading Overlay */
  var LOADING_CSS = [
    ':host{all:initial}',
    '.pill{display:inline-flex;align-items:center;gap:10px;padding:7px 13px;border-radius:999px;',
    'background:rgba(11,14,23,.92);backdrop-filter:blur(20px);-webkit-backdrop-filter:blur(20px);',
    'border:1px solid rgba(94,158,255,.4);box-shadow:0 10px 28px rgba(0,0,0,.55),0 0 18px rgba(56,189,248,.28);',
    'font:500 11.5px -apple-system,BlinkMacSystemFont,"SF Pro Text",sans-serif;color:rgba(255,255,255,.92);',
    'animation:pillIn .18s cubic-bezier(.16,1,.3,1)}',
    '@keyframes pillIn{from{opacity:0;transform:translateY(-6px) scale(.95)}to{opacity:1;transform:none}}',
    '.eclipse-loader{width:20px;height:20px;position:relative;display:inline-flex;align-items:center;justify-content:center;flex-shrink:0}',
    '.eclipse-star{position:absolute;width:10px;height:10px;border-radius:50%;',
    'background:radial-gradient(circle at 38% 38%,#fff 0%,#93c5fd 25%,#38bdf8 55%,#1d4ed8 100%);',
    'box-shadow:0 0 6px #38bdf8,0 0 12px rgba(56,189,248,.75);animation:starLuminescence 2.2s ease-in-out infinite;z-index:1}',
    '.eclipse-corona{position:absolute;width:15px;height:15px;border-radius:50%;border:1px solid rgba(147,197,253,.9);',
    'box-shadow:0 0 10px rgba(56,189,248,.8),inset 0 0 6px rgba(37,99,235,.6);animation:coronaFlare 2.2s ease-in-out infinite;z-index:0}',
    '.eclipse-moon{position:absolute;width:10.6px;height:10.6px;border-radius:50%;',
    'background:radial-gradient(circle at 35% 35%,#0f172a 0%,#030712 100%);',
    'box-shadow:inset -1px -1px 2px rgba(147,197,253,.5);animation:moonTransit 2.2s cubic-bezier(.42,0,.58,1) infinite;z-index:2}',
    '.eclipse-diamond{position:absolute;width:3px;height:3px;border-radius:50%;background:#fff;',
    'box-shadow:0 0 5px #fff,0 0 9px #93c5fd;animation:diamondFlash 2.2s ease-in-out infinite;z-index:3}',
    '@keyframes moonTransit{0%{transform:translateX(-14px) scale(.9);opacity:0}15%{opacity:1}50%{transform:translateX(0) scale(1.02);opacity:1}85%{opacity:1}100%{transform:translateX(14px) scale(.9);opacity:0}}',
    '@keyframes starLuminescence{0%,100%{transform:scale(.95);opacity:.85}50%{transform:scale(1.08);opacity:1}}',
    '@keyframes coronaFlare{0%,100%{transform:scale(.86);opacity:.35}46%,54%{transform:scale(1.34);opacity:1}}',
    '@keyframes diamondFlash{0%,43%{opacity:0;transform:translate(3.5px,-3.5px) scale(0)}50%{opacity:1;transform:translate(4px,-4px) scale(1.55)}57%,100%{opacity:0;transform:translate(4.5px,-4.5px) scale(0)}}'
  ].join('');

  var loadingHost = null;
  var loadingHideTimer = null;

  function ensureLoadingUi() {
    if (loadingHost || internal) return;
    if (!document.documentElement) return;
    var host = document.createElement('div');
    host.id = '__eclipse_loading_host';
    host.style.cssText =
      'all: initial; position: fixed; top: 14px; right: 16px; z-index: 2147483646; pointer-events: none; display: none;';
    var shadow = attachShadow(host, { mode: 'closed' });
    shadow.appendChild(styleSheet(LOADING_CSS));
    var pill = el('div', 'pill');
    var loader = el('div', 'eclipse-loader');
    loader.appendChild(el('div', 'eclipse-corona'));
    loader.appendChild(el('div', 'eclipse-star'));
    loader.appendChild(el('div', 'eclipse-moon'));
    loader.appendChild(el('div', 'eclipse-diamond'));
    pill.appendChild(loader);
    pill.appendChild(el('span', '', 'Loading Eclipse…'));
    shadow.appendChild(pill);
    document.documentElement.appendChild(host);
    loadingHost = host;
  }

  function setLoadingState(isLoading) {
    if (internal) return;
    try {
      ensureLoadingUi();
    } catch (_) {}
    if (!loadingHost) return;
    if (loadingHideTimer) {
      clearTimeout(loadingHideTimer);
      loadingHideTimer = null;
    }
    if (isLoading) {
      loadingHost.style.display = 'block';
      loadingHideTimer = setTimeout(function () {
        if (loadingHost) loadingHost.style.display = 'none';
      }, 8000);
    } else {
      loadingHost.style.display = 'none';
    }
  }

  if (!internal && document.readyState === 'loading') {
    setLoadingState(true);
    window.addEventListener(
      'DOMContentLoaded',
      function () {
        setTimeout(function () {
          setLoadingState(false);
        }, 280);
      },
      { once: true }
    );
  }

  window.addEventListener('beforeunload', function () {
    setLoadingState(true);
  });

  /* ------------------------------------------------- link click behaviour */
  // Cmd+click and middle-click open http(s) links in a background tab
  // (Shift brings it to the front), like Safari and Chrome.
  function openLinkInTab(event) {
    var anchor = event.target && event.target.closest ? event.target.closest('a[href]') : null;
    if (!anchor || anchor.hasAttribute('download')) return;
    if (anchor.target && anchor.target !== '_self') return;
    var href = anchor.href;
    if (!isHttpUrl(href)) return;
    event.preventDefault();
    event.stopPropagation();
    var payload = message('new_tab');
    payload.url = href;
    payload.background = !event.shiftKey;
    post(payload);
  }

  document.addEventListener(
    'click',
    function (event) {
      if (!event.isTrusted || event.button !== 0) return;
      if (!event.metaKey || event.ctrlKey || event.altKey) return;
      openLinkInTab(event);
    },
    true
  );

  document.addEventListener(
    'auxclick',
    function (event) {
      if (!event.isTrusted || event.button !== 1) return;
      openLinkInTab(event);
    },
    true
  );

  /* ----------------------------------------------------------- find in page */
  var FIND_CSS = [
    ':host{all:initial}',
    '.bar{display:flex;align-items:center;gap:5px;padding:6px 7px;border-radius:9px;',
    'background:rgba(18,21,30,.97);border:.5px solid rgba(255,255,255,.18);',
    'box-shadow:0 12px 34px rgba(0,0,0,.5);font:12px -apple-system,BlinkMacSystemFont,"SF Pro Text",sans-serif;',
    'color:#f5f7fb}',
    'input{width:210px;height:24px;border:0;outline:0;background:transparent;color:#f5f7fb;',
    'font:12.5px -apple-system,BlinkMacSystemFont,"SF Pro Text",sans-serif;padding:0 4px}',
    'input::placeholder{color:rgba(255,255,255,.38)}',
    'button{width:24px;height:24px;border:0;border-radius:6px;background:transparent;color:rgba(255,255,255,.66);',
    'font:15px -apple-system,sans-serif;line-height:1;cursor:pointer;display:flex;align-items:center;justify-content:center}',
    'button:hover{background:rgba(255,255,255,.1);color:#fff}',
    '.close{margin-left:1px;font-size:17px}',
    '.sep{width:.5px;height:16px;background:rgba(255,255,255,.14);margin:0 2px}'
  ].join('');

  var findHost = null;
  var findInput = null;
  var nativeFind = typeof window.find === 'function' ? window.find.bind(window) : null;
  var findDebounce = null;

  function runFind(backwards) {
    if (!nativeFind || !findInput) return;
    var query = findInput.value;
    if (!query) return;
    try {
      nativeFind(query, false, !!backwards, true, false, false, false);
    } catch (_) {}
  }

  function findButton(className, label, title) {
    var button = el('button', className, label);
    button.type = 'button';
    button.title = title;
    return button;
  }

  function ensureFindUi() {
    if (findHost) return;
    if (!document.documentElement) return;
    var host = document.createElement('div');
    host.id = '__eclipse_find_host';
    host.style.cssText =
      'all: initial; position: fixed; top: 14px; right: 18px; z-index: 2147483647; display: none;';
    var shadow = attachShadow(host, { mode: 'closed' });
    shadow.appendChild(styleSheet(FIND_CSS));
    var bar = el('div', 'bar');
    var input = document.createElement('input');
    input.type = 'text';
    input.placeholder = 'Find in page';
    input.spellcheck = false;
    input.autocomplete = 'off';
    var previous = findButton('prev', '‹', 'Previous match (⇧↩)');
    var next = findButton('next', '›', 'Next match (↩)');
    var close = findButton('close', '×', 'Close (esc)');
    bar.appendChild(input);
    bar.appendChild(el('span', 'sep'));
    bar.appendChild(previous);
    bar.appendChild(next);
    bar.appendChild(close);
    shadow.appendChild(bar);
    document.documentElement.appendChild(host);
    findHost = host;
    findInput = input;
    input.addEventListener('input', function () {
      if (findDebounce) clearTimeout(findDebounce);
      findDebounce = setTimeout(function () {
        runFind(false);
      }, 70);
    });
    input.addEventListener('keydown', function (event) {
      if (event.key === 'Enter') {
        event.preventDefault();
        runFind(event.shiftKey);
      } else if (event.key === 'Escape') {
        event.preventDefault();
        closeFind();
      }
      event.stopPropagation();
    });
    previous.addEventListener('click', function () {
      runFind(true);
    });
    next.addEventListener('click', function () {
      runFind(false);
    });
    close.addEventListener('click', closeFind);
  }

  function openFind() {
    ensureFindUi();
    if (!findHost) return;
    findHost.style.display = 'block';
    if (findInput) {
      findInput.focus();
      findInput.select();
      if (findInput.value) runFind(false);
    }
  }

  function closeFind() {
    if (!findHost) return;
    findHost.style.display = 'none';
    if (findInput) findInput.blur();
  }
})();
