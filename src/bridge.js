/*
 * Eclipse content bridge.
 *
 * Runs in the main frame before any page script. The IPC token lives in this
 * closure and is never attached to `window`, so an ordinary website cannot
 * drive browser chrome even though `window.ipc.postMessage` is visible to it.
 * Internal eclipse:// pages get a narrow helper so their own UI can post.
 */
(function () {
  'use strict';
  if (window.__eclipseBridgeActive) return;
  window.__eclipseBridgeActive = true;

  var TOKEN = '__ECLIPSE_TOKEN__';
  var rawPost =
    window.ipc && typeof window.ipc.postMessage === 'function'
      ? window.ipc.postMessage.bind(window.ipc)
      : null;
  var stringify = JSON.stringify;
  var internal = location.protocol === 'eclipse:';

  function post(payload) {
    if (!rawPost || !payload || typeof payload !== 'object') return;
    payload.token = TOKEN;
    try {
      rawPost(stringify(payload));
    } catch (_) {}
  }

  if (internal) {
    try {
      Object.defineProperty(window, '__eclipseIpc', {
        value: function (payload) {
          post(payload);
        },
        writable: false,
        configurable: false,
        enumerable: false
      });
    } catch (_) {}

    function reportChromeError(message) {
      try {
        post({ action: 'chrome_error', message: String(message || '').slice(0, 800) });
      } catch (_) {}
    }
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
    post({ action: 'update_scroll', x: x, y: y });
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
    post({ action: 'set_audible', audible: audible });
  }

  function isPlaying(el) {
    return (
      el &&
      (el.tagName === 'VIDEO' || el.tagName === 'AUDIO') &&
      !el.paused &&
      !el.muted &&
      Number(el.volume) > 0
    );
  }

  document.addEventListener(
    'play',
    function (event) {
      if (isPlaying(event.target)) {
        playing.add(event.target);
        reportAudible();
      }
    },
    true
  );
  document.addEventListener(
    'pause',
    function (event) {
      if (playing.delete(event.target)) reportAudible();
    },
    true
  );
  document.addEventListener(
    'ended',
    function (event) {
      if (playing.delete(event.target)) reportAudible();
    },
    true
  );
  document.addEventListener(
    'volumechange',
    function (event) {
      var el = event.target;
      if (!playing.has(el)) return;
      if (isPlaying(el)) return;
      playing.delete(el);
      reportAudible();
    },
    true
  );
  window.addEventListener(
    'pagehide',
    function () {
      if (!playing.size) return;
      playing.clear();
      lastAudible = false;
      post({ action: 'set_audible', audible: false });
    },
    true
  );

  /* ------------------------------------------------------------- favicons */
  var lastFavicon = '';
  var faviconTimer = null;

  function detectFavicon() {
    if (internal) return;
    var link = document.querySelector(
      'link[rel~="icon"][href], link[rel="apple-touch-icon"][href], link[rel="mask-icon"][href]'
    );
    if (!link) return;
    try {
      var href = new URL(link.getAttribute('href'), location.href).href;
      if (!/^https?:/i.test(href) || href === lastFavicon) return;
      lastFavicon = href;
      post({ action: 'update_favicon', favicon: href });
    } catch (_) {}
  }

  function scheduleFaviconScan() {
    if (faviconTimer) clearTimeout(faviconTimer);
    faviconTimer = setTimeout(detectFavicon, 1200);
  }

  if (document.readyState === 'loading') {
    document.addEventListener(
      'DOMContentLoaded',
      function () {
        detectFavicon();
        scheduleFaviconScan();
      },
      { once: true }
    );
  } else {
    detectFavicon();
    scheduleFaviconScan();
  }

  /* ------------------------------------------------- single-page-app URLs */
  var lastReportedUrl = location.href;

  function reportUrl() {
    if (internal) return;
    var href = location.href;
    if (!href || href === lastReportedUrl) return;
    if (!/^https?:/i.test(href)) return;
    lastReportedUrl = href;
    post({ action: 'url_changed', url: href });
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

  /* ------------------------------------ Celestial Eclipse Loading Overlay */
  var loadingHost = null;
  var loadingHideTimer = null;

  function ensureLoadingUi() {
    if (loadingHost || internal) return;
    if (!document.documentElement) return;
    loadingHost = document.createElement('div');
    loadingHost.id = '__eclipse_loading_host';
    loadingHost.style.cssText =
      'all: initial; position: fixed; top: 14px; right: 16px; z-index: 2147483646; pointer-events: none; display: none;';
    var shadow = loadingHost.attachShadow({ mode: 'closed' });
    shadow.innerHTML = [
      '<style>',
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
      '@keyframes diamondFlash{0%,43%{opacity:0;transform:translate(3.5px,-3.5px) scale(0)}50%{opacity:1;transform:translate(4px,-4px) scale(1.55)}57%,100%{opacity:0;transform:translate(4.5px,-4.5px) scale(0)}}',
      '</style>',
      '<div class="pill">',
      '  <div class="eclipse-loader">',
      '    <div class="eclipse-corona"></div>',
      '    <div class="eclipse-star"></div>',
      '    <div class="eclipse-moon"></div>',
      '    <div class="eclipse-diamond"></div>',
      '  </div>',
      '  <span>Loading Eclipse…</span>',
      '</div>'
    ].join('');
    document.documentElement.appendChild(loadingHost);
  }

  function setLoadingState(isLoading) {
    if (internal) return;
    ensureLoadingUi();
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

  try {
    Object.defineProperty(window, '__eclipseSetLoading', {
      value: function (isLoading) {
        setLoadingState(!!isLoading);
      },
      writable: false,
      configurable: false,
      enumerable: false
    });
  } catch (_) {}

  /* ------------------------------------------------- Little Arc Quick Window */
  var littleArcHost = null;
  var littleArcFrame = null;
  var littleArcTitle = null;
  var littleArcUrlText = null;
  var currentLittleArcUrl = '';

  function ensureLittleArcUi() {
    if (littleArcHost) return;
    if (!document.documentElement) return;
    littleArcHost = document.createElement('div');
    littleArcHost.id = '__eclipse_little_arc_host';
    littleArcHost.style.cssText =
      'all: initial; position: fixed; inset: 0; z-index: 2147483647; display: none; align-items: center; justify-content: center; background: rgba(4, 6, 12, 0.58); backdrop-filter: blur(18px) saturate(160%); -webkit-backdrop-filter: blur(18px) saturate(160%);';
    var shadow = littleArcHost.attachShadow({ mode: 'closed' });
    shadow.innerHTML = [
      '<style>',
      ':host{all:initial}',
      '*{box-sizing:border-box;margin:0;padding:0;font-family:-apple-system,BlinkMacSystemFont,"SF Pro Text",sans-serif}',
      '.window{width:min(760px,90vw);height:min(560px,84vh);background:rgba(14,17,26,.98);border:1px solid rgba(94,158,255,.45);border-radius:16px;box-shadow:0 28px 80px rgba(0,0,0,.82),0 0 36px rgba(94,158,255,.22);display:flex;flex-direction:column;overflow:hidden;animation:laPop .18s cubic-bezier(.16,1,.3,1)}',
      '@keyframes laPop{from{opacity:0;transform:scale(.95) translateY(10px)}to{opacity:1;transform:none}}',
      '.topbar{display:flex;align-items:center;gap:10px;padding:10px 14px;background:rgba(255,255,255,.04);border-bottom:1px solid rgba(255,255,255,.09)}',
      '.badge{font-size:10.5px;font-weight:650;color:#5E9EFF;background:rgba(94,158,255,.14);border:1px solid rgba(94,158,255,.35);padding:3px 8px;border-radius:6px;letter-spacing:.3px;text-transform:uppercase}',
      '.url-bar{flex:1;font-size:12px;color:rgba(255,255,255,.78);white-space:nowrap;overflow:hidden;text-overflow:ellipsis;padding:4px 10px;background:rgba(0,0,0,.28);border-radius:7px;border:1px solid rgba(255,255,255,.07)}',
      '.btn{border:1px solid rgba(255,255,255,.14);background:rgba(255,255,255,.07);color:#fff;font-size:11.5px;font-weight:550;padding:5px 11px;border-radius:8px;cursor:pointer;display:inline-flex;align-items:center;gap:5px}',
      '.btn:hover{background:rgba(94,158,255,.24);border-color:#5E9EFF}',
      '.btn.primary{background:#5E9EFF;color:#080A10;border-color:transparent;font-weight:600}',
      '.btn.primary:hover{filter:brightness(1.08)}',
      '.body{flex:1;position:relative;background:#090A0F;display:flex;flex-direction:column}',
      'iframe{flex:1;width:100%;height:100%;border:0;background:#fff}',
      '.footer-note{padding:6px 14px;font-size:11px;color:rgba(255,255,255,.48);background:rgba(10,12,18,.95);border-top:1px solid rgba(255,255,255,.07);display:flex;justify-content:space-between;align-items:center}',
      '</style>',
      '<div class="window">',
      '  <div class="topbar">',
      '    <span class="badge">Little Arc</span>',
      '    <div class="url-bar" id="la-url">https://</div>',
      '    <button class="btn primary" id="la-promote" title="Open in Main Tab">Expand to Tab ↗</button>',
      '    <button class="btn" id="la-close" title="Close (Esc)">✕</button>',
      '  </div>',
      '  <div class="body">',
      '    <iframe id="la-frame" sandbox="allow-scripts allow-same-origin allow-forms allow-popups"></iframe>',
      '  </div>',
      '  <div class="footer-note">',
      '    <span>Quick preview window • Tip: Shift+Click any link to preview in Little Arc</span>',
      '    <span id="la-title">Ready</span>',
      '  </div>',
      '</div>'
    ].join('');

    littleArcHost.addEventListener('click', function (e) {
      if (e.target === littleArcHost) closeLittleArc();
    });
    shadow.getElementById('la-close').addEventListener('click', closeLittleArc);
    shadow.getElementById('la-promote').addEventListener('click', function () {
      var target = currentLittleArcUrl;
      closeLittleArc();
      if (target) {
        post({ action: 'promote_little_arc', url: target });
      }
    });

    littleArcFrame = shadow.getElementById('la-frame');
    littleArcUrlText = shadow.getElementById('la-url');
    littleArcTitle = shadow.getElementById('la-title');
    document.documentElement.appendChild(littleArcHost);
  }

  function openLittleArc(url) {
    ensureLittleArcUi();
    if (!littleArcHost) return;
    currentLittleArcUrl = url || location.href;
    if (littleArcUrlText) littleArcUrlText.textContent = currentLittleArcUrl;
    if (littleArcTitle) littleArcTitle.textContent = 'Preview Active';
    if (littleArcFrame) littleArcFrame.src = currentLittleArcUrl;
    littleArcHost.style.display = 'flex';
  }

  function closeLittleArc() {
    if (!littleArcHost) return;
    littleArcHost.style.display = 'none';
    if (littleArcFrame) littleArcFrame.src = 'about:blank';
    post({ action: 'close_little_arc' });
  }

  try {
    Object.defineProperty(window, '__eclipseLittleArc', {
      value: function (action, url) {
        if (action === 'open') openLittleArc(url);
        else if (action === 'close') closeLittleArc();
      },
      writable: false,
      configurable: false,
      enumerable: false
    });
  } catch (_) {}

  /* ------------------------------------------------- link click behaviour */
  function linkTarget(event, background) {
    if (!event.isTrusted) return;
    var anchor = event.target && event.target.closest ? event.target.closest('a[href]') : null;
    if (!anchor || anchor.hasAttribute('download')) return;
    if (anchor.target && anchor.target !== '_self') return;
    var href = anchor.href;
    if (!/^https?:/i.test(href)) return;
    event.preventDefault();
    event.stopPropagation();
    post({ action: 'new_tab', url: href, background: !!background });
  }

  document.addEventListener(
    'click',
    function (event) {
      if (!event.isTrusted || event.button !== 0) return;
      var anchor = event.target && event.target.closest ? event.target.closest('a[href]') : null;
      if (anchor && event.shiftKey && !event.metaKey && !event.ctrlKey && /^https?:/i.test(anchor.href)) {
        event.preventDefault();
        event.stopPropagation();
        openLittleArc(anchor.href);
        return;
      }
      if (!event.metaKey || event.ctrlKey || event.altKey) return;
      linkTarget(event, false);
    },
    true
  );

  document.addEventListener(
    'auxclick',
    function (event) {
      if (event.button !== 1) return;
      linkTarget(event, true);
    },
    true
  );

  /* ----------------------------------------------------------- find in page */
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

  function ensureFindUi() {
    if (findHost) return;
    if (!document.documentElement) return;
    findHost = document.createElement('div');
    findHost.id = '__eclipse_find_host';
    findHost.style.cssText =
      'all: initial; position: fixed; top: 14px; right: 18px; z-index: 2147483647; display: none;';
    var shadow = findHost.attachShadow({ mode: 'closed' });
    shadow.innerHTML = [
      '<style>',
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
      '.sep{width:.5px;height:16px;background:rgba(255,255,255,.14);margin:0 2px}',
      '</style>',
      '<div class="bar">',
      '<input type="text" placeholder="Find in page" spellcheck="false" autocomplete="off">',
      '<span class="sep"></span>',
      '<button class="prev" title="Previous match (⇧↩)">&#8249;</button>',
      '<button class="next" title="Next match (↩)">&#8250;</button>',
      '<button class="close" title="Close (esc)">&#215;</button>',
      '</div>'
    ].join('');
    document.documentElement.appendChild(findHost);
    findInput = shadow.querySelector('input');
    var previous = shadow.querySelector('.prev');
    var next = shadow.querySelector('.next');
    var close = shadow.querySelector('.close');
    findInput.addEventListener('input', function () {
      if (findDebounce) clearTimeout(findDebounce);
      findDebounce = setTimeout(function () {
        runFind(false);
      }, 70);
    });
    findInput.addEventListener('keydown', function (event) {
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

  try {
    Object.defineProperty(window, '__eclipseFind', {
      value: function (secret, action) {
        if (secret !== TOKEN) return;
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

  /* ---------------------------------------------------- Arc Floating Command Bar */
  var cmdBarHost = null;
  var cmdBarInput = null;
  var cmdBarResults = null;
  var cmdActiveIdx = 0;
  var cmdItems = [];

  var ICONS = {
    search: '<svg viewBox="0 0 24 24" fill="none" stroke="#5E9EFF" stroke-width="2.2" width="14" height="14"><circle cx="11" cy="11" r="7"/><path d="m20 20-3.5-3.5"/></svg>',
    globe: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" width="14" height="14"><circle cx="12" cy="12" r="9"/><path d="M3 12h18M12 3a14.5 14.5 0 0 1 0 18M12 3a14.5 14.5 0 0 0 0 18"/></svg>',
    settings: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" width="14" height="14"><circle cx="12" cy="12" r="3"/><path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 0 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 0 1-2.83-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 0 1 2.83-2.83l.06.06a1.65 1.65 0 0 0 1.82.33H9a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 0 1 2.83 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82V9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z"/></svg>',
    clean: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" width="14" height="14"><path d="m12 3-1.912 5.813a2 2 0 0 1-1.275 1.275L3 12l5.813 1.912a2 2 0 0 1 1.275 1.275L12 21l1.912-5.813a2 2 0 0 1 1.275-1.275L21 12l-5.813-1.912a2 2 0 0 1-1.275-1.275L12 3Z"/></svg>',
    link: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" width="14" height="14"><path d="M10 13a5 5 0 0 0 7.54.54l3-3a5 5 0 0 0-7.07-7.07l-1.72 1.71"/><path d="M14 11a5 5 0 0 0-7.54-.54l-3 3a5 5 0 0 0 7.07 7.07l1.71-1.71"/></svg>',
    youtube: '<svg viewBox="0 0 24 24" fill="#FF0000" width="14" height="14"><path d="M23.498 6.186a3.016 3.016 0 0 0-2.122-2.136C19.505 3.545 12 3.545 12 3.545s-7.505 0-9.377.505A3.017 3.017 0 0 0 .502 6.186C0 8.07 0 12 0 12s0 3.93.502 5.814a3.016 3.016 0 0 0 2.122 2.136c1.871.505 9.376.505 9.376.505s7.505 0 9.377-.505a3.015 3.015 0 0 0 2.122-2.136C24 15.93 24 12 24 12s0-3.93-.502-5.814zM9.545 15.568V8.432L15.818 12l-6.273 3.568z"/></svg>',
    netflix: '<svg viewBox="0 0 24 24" fill="#E50914" width="14" height="14"><path d="M5.398 0v24L9.89 12.38V0H5.398zm13.204 0L14.11 12.38V24h4.492V0zm-4.492 12.38L9.89 0h4.22z"/></svg>',
    gmail: '<svg viewBox="0 0 24 24" width="14" height="14"><path fill="#EA4335" d="M12 13.5L2.5 6.4V19c0 .8.7 1.5 1.5 1.5h16c.8 0 1.5-.7 1.5-1.5V6.4L12 13.5z"/><path fill="#4285F4" d="M22 6c0-.8-.7-1.5-1.5-1.5H3.5C2.7 4.5 2 5.2 2 6l10 7.5L22 6z"/></svg>',
    github: '<svg viewBox="0 0 24 24" fill="currentColor" width="14" height="14"><path d="M12 0C5.37 0 0 5.37 0 12c0 5.31 3.435 9.795 8.205 11.385.6.105.825-.255.825-.57 0-.285-.015-1.23-.015-2.235-3.015.555-3.795-.735-4.035-1.41-.135-.345-.72-1.41-1.23-1.695-.42-.225-1.02-.78-.015-.795.945-.015 1.62.87 1.845 1.23 1.08 1.815 2.805 1.305 3.495.99.105-.78.42-1.305.765-1.605-2.67-.3-5.46-1.335-5.46-5.925 0-1.305.465-2.385 1.23-3.225-.12-.3-.54-1.53.12-3.18 0 0 1.005-.315 3.3 1.23.96-.27 1.98-.405 3-.405s2.04.135 3 .405c2.295-1.56 3.3-1.23 3.3-1.23.66 1.65.24 2.88.12 3.18.765.84 1.23 1.905 1.23 3.225 0 4.605-2.805 5.625-5.475 5.925.435.375.81 1.095.81 2.22 0 1.605-.015 2.895-.015 3.3 0 .315.225.69.825.57A12.02 12.02 0 0 0 24 12c0-6.63-5.37-12-12-12z"/></svg>',
    discord: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" width="14" height="14"><path d="M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z"/></svg>',
    reddit: '<svg viewBox="0 0 24 24" fill="#FF4500" width="14" height="14"><circle cx="12" cy="12" r="10"/><circle cx="9" cy="11.5" r="1.5" fill="#fff"/><circle cx="15" cy="11.5" r="1.5" fill="#fff"/><path d="M9 15.5c1 1 5 1 6 0" stroke="#fff" stroke-width="1.5" stroke-linecap="round" fill="none"/></svg>',
    instagram: '<svg viewBox="0 0 24 24" width="14" height="14"><defs><linearGradient id="cmd_ig" x1="0%" y1="100%" x2="100%" y2="0%"><stop offset="0%" stop-color="#fdf497"/><stop offset="5%" stop-color="#fdf497"/><stop offset="45%" stop-color="#fd5949"/><stop offset="60%" stop-color="#d6249f"/><stop offset="90%" stop-color="#285AEB"/></linearGradient></defs><rect width="24" height="24" rx="6" fill="url(#cmd_ig)"/><circle cx="12" cy="12" r="4.5" fill="none" stroke="#FFFFFF" stroke-width="2"/><circle cx="17.5" cy="6.5" r="1.2" fill="#FFFFFF"/></svg>',
    apple: '<svg viewBox="0 0 24 24" fill="currentColor" width="14" height="14"><path d="M18.71 19.5c-.83 1.24-1.71 2.45-3.05 2.47-1.34.03-1.77-.79-3.29-.79-1.53 0-2 .77-3.27.82-1.31.05-2.3-1.32-3.14-2.53C4.25 17 2.94 12.45 4.7 9.39c.87-1.52 2.43-2.48 4.12-2.51 1.28-.02 2.5.87 3.29.87.78 0 2.26-1.07 3.81-.91.65.03 2.47.26 3.64 1.98-.09.06-2.17 1.28-2.15 3.81.03 3.02 2.65 4.03 2.68 4.04-.03.07-.42 1.44-1.38 2.83M15.96 6.33c.67-.81 1.12-1.94 1-3.08-.96.04-2.13.64-2.82 1.45-.61.7-.11 1.86-.98 2.97 1.08.08 2.13-.53 2.8-1.34z"/></svg>'
  };

  var DEFAULT_FAVORITES = [
    { title: 'YouTube', url: 'https://www.youtube.com', icon: ICONS.youtube },
    { title: 'Netflix', url: 'https://www.netflix.com', icon: ICONS.netflix },
    { title: 'Gmail', url: 'https://mail.google.com', icon: ICONS.gmail },
    { title: 'GitHub', url: 'https://github.com', icon: ICONS.github },
    { title: 'Discord', url: 'https://discord.com', icon: ICONS.discord },
    { title: 'Reddit', url: 'https://www.reddit.com', icon: ICONS.reddit },
    { title: 'Instagram', url: 'https://www.instagram.com', icon: ICONS.instagram },
    { title: 'Apple', url: 'https://www.apple.com', icon: ICONS.apple }
  ];

  var cachedOpenTabs = [];

  function ensureCommandBarUi() {
    if (cmdBarHost) return;
    if (!document.documentElement) return;
    cmdBarHost = document.createElement('div');
    cmdBarHost.id = '__eclipse_cmd_host';
    cmdBarHost.style.cssText =
      'all: initial; position: fixed; inset: 0; z-index: 2147483647; display: none; align-items: center; justify-content: center; background: rgba(5, 7, 12, 0.65); backdrop-filter: blur(24px) saturate(160%); -webkit-backdrop-filter: blur(24px) saturate(160%);';
    var shadow = cmdBarHost.attachShadow({ mode: 'closed' });
    shadow.innerHTML = [
      '<style>',
      ':host{all:initial}',
      '* { box-sizing: border-box; margin: 0; padding: 0; font-family: -apple-system, BlinkMacSystemFont, "SF Pro Text", "SF Pro Display", sans-serif; }',
      '.cmd-card { width: 580px; max-width: 92vw; background: rgba(18, 22, 34, 0.96); border: 1px solid rgba(94, 158, 255, 0.45); border-radius: 14px; box-shadow: 0 24px 64px rgba(0,0,0,0.8), 0 0 35px rgba(94, 158, 255, 0.25); overflow: hidden; display: flex; flex-direction: column; animation: cmdPop 0.16s cubic-bezier(0.16, 1, 0.3, 1); }',
      '@keyframes cmdPop { from { opacity: 0; transform: scale(0.96) translateY(-8px); } to { opacity: 1; transform: scale(1) translateY(0); } }',
      '.input-row { display: flex; align-items: center; gap: 10px; padding: 12px 16px; background: rgba(255, 255, 255, 0.04); border-bottom: 0.5px solid rgba(255, 255, 255, 0.1); }',
      '.icon { width: 18px; height: 18px; color: #5E9EFF; flex-shrink: 0; }',
      'input { flex: 1; height: 32px; border: 0; outline: 0; background: transparent; color: #fff; font-size: 15px; font-weight: 450; letter-spacing: -0.2px; }',
      'input::placeholder { color: rgba(255, 255, 255, 0.38); }',
      '.close-btn { width: 24px; height: 24px; border: 0; border-radius: 6px; background: transparent; color: rgba(255,255,255,0.45); font-size: 18px; line-height: 1; cursor: pointer; display: flex; align-items: center; justify-content: center; transition: all 0.15s ease; }',
      '.close-btn:hover { background: rgba(255,255,255,0.1); color: #fff; }',
      '.hint-row { display: flex; justify-content: space-between; align-items: center; padding: 6px 16px; background: rgba(0, 0, 0, 0.22); font-size: 11px; color: rgba(255, 255, 255, 0.4); border-bottom: 0.5px solid rgba(255, 255, 255, 0.05); }',
      'kbd { background: rgba(255, 255, 255, 0.1); padding: 1px 5px; border-radius: 4px; font-size: 10.5px; font-family: monospace; color: rgba(255,255,255,0.7); }',
      '.results { max-height: 340px; overflow-y: auto; padding: 6px 8px; display: flex; flex-direction: column; gap: 3px; }',
      '.results::-webkit-scrollbar { width: 4px; }',
      '.results::-webkit-scrollbar-thumb { background: rgba(255,255,255,0.15); border-radius: 2px; }',
      '.section-title { font-size: 11px; text-transform: uppercase; letter-spacing: 0.6px; color: rgba(255, 255, 255, 0.35); font-weight: 600; padding: 6px 10px 4px; }',
      '.item { display: flex; align-items: center; gap: 12px; padding: 9px 12px; border-radius: 8px; cursor: pointer; transition: all 0.12s ease; user-select: none; }',
      '.item:hover, .item.active { background: rgba(94, 158, 255, 0.16); }',
      '.item.active { box-shadow: inset 0 0 0 1px rgba(94, 158, 255, 0.4); }',
      '.item-icon { width: 22px; height: 22px; border-radius: 6px; background: rgba(255,255,255,0.06); display: flex; align-items: center; justify-content: center; flex-shrink: 0; }',
      '.item-icon svg { width: 14px; height: 14px; display: block; }',
      '.item-info { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 1px; }',
      '.item-title { font-size: 13.5px; font-weight: 500; color: #fff; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }',
      '.item-sub { font-size: 11.5px; color: rgba(255, 255, 255, 0.5); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }',
      '.switch-pill { font-size: 11px; font-weight: 550; color: #5E9EFF; background: rgba(94, 158, 255, 0.12); border: 0.5px solid rgba(94, 158, 255, 0.35); padding: 3px 8px; border-radius: 6px; margin-left: auto; flex-shrink: 0; }',
      '</style>',
      '<div class="cmd-card">',
      '  <div class="input-row">',
      '    <svg class="icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><circle cx="11" cy="11" r="7"/><path d="m20 20-3.5-3.5"/></svg>',
      '    <input type="text" placeholder="Search or Enter URL..." spellcheck="false" autocomplete="off">',
      '    <button class="close-btn" title="Close (esc)">&times;</button>',
      '  </div>',
      '  <div class="hint-row">',
      '    <span><kbd>↵</kbd> open &nbsp;&nbsp; <kbd>↑</kbd><kbd>↓</kbd> navigate</span>',
      '    <span><kbd>esc</kbd> cancel</span>',
      '  </div>',
      '  <div class="results"></div>',
      '</div>'
    ].join('');

    cmdBarHost.addEventListener('click', function (e) {
      if (e.target === cmdBarHost) closeCommandBar();
    });

    shadow.querySelector('.close-btn').addEventListener('click', closeCommandBar);
    cmdBarInput = shadow.querySelector('input');
    cmdBarResults = shadow.querySelector('.results');

    cmdBarInput.addEventListener('input', function () {
      cmdActiveIdx = 0;
      renderCommandSuggestions(cmdBarInput.value);
    });

    cmdBarInput.addEventListener('keydown', function (event) {
      if (event.key === 'ArrowDown') {
        event.preventDefault();
        if (cmdItems.length > 0) {
          cmdActiveIdx = (cmdActiveIdx + 1) % cmdItems.length;
          updateActiveItem();
        }
      } else if (event.key === 'ArrowUp') {
        event.preventDefault();
        if (cmdItems.length > 0) {
          cmdActiveIdx = (cmdActiveIdx - 1 + cmdItems.length) % cmdItems.length;
          updateActiveItem();
        }
      } else if (event.key === 'Enter') {
        event.preventDefault();
        if (cmdItems.length > 0 && cmdItems[cmdActiveIdx]) {
          executeCommand(cmdItems[cmdActiveIdx]);
        } else {
          var val = cmdBarInput.value.trim();
          if (val) executeUrl(val);
        }
      } else if (event.key === 'Escape') {
        event.preventDefault();
        closeCommandBar();
      }
      event.stopPropagation();
    });

    document.documentElement.appendChild(cmdBarHost);
  }

  function renderCommandSuggestions(query) {
    if (!cmdBarResults) return;
    cmdBarResults.innerHTML = '';
    cmdItems = [];
    var q = (query || '').trim();

    if (q) {
      var qLower = q.toLowerCase();

      if (cachedOpenTabs && cachedOpenTabs.length > 0) {
        cachedOpenTabs.forEach(function (t) {
          if (t.url !== 'eclipse://newtab' && (
            (t.title && t.title.toLowerCase().indexOf(qLower) !== -1) ||
            (t.url && t.url.toLowerCase().indexOf(qLower) !== -1)
          )) {
            cmdItems.push({
              id: t.id,
              title: t.title || t.url,
              sub: t.url,
              action: 'switch_tab',
              icon: ICONS.globe
            });
          }
        });
      }

      if (q.indexOf('.') !== -1 || q.startsWith('http://') || q.startsWith('https://') || q.startsWith('localhost')) {
        var directUrl = q;
        if (!/^https?:\/\//i.test(directUrl)) directUrl = 'https://' + directUrl;
        cmdItems.push({
          title: 'Open ' + q,
          sub: directUrl,
          url: directUrl,
          icon: ICONS.globe
        });
      }

      cmdItems.push({
        title: 'Search for "' + q + '"',
        sub: 'Web Search',
        url: q,
        icon: ICONS.search
      });

      DEFAULT_FAVORITES.forEach(function (f) {
        if (f.title.toLowerCase().indexOf(qLower) !== -1 || f.url.toLowerCase().indexOf(qLower) !== -1) {
          cmdItems.push({
            title: f.title,
            sub: f.url,
            url: f.url,
            icon: f.icon
          });
        }
      });
    } else {
      if (cachedOpenTabs && cachedOpenTabs.length > 0) {
        var tabsTitle = document.createElement('div');
        tabsTitle.className = 'section-title';
        tabsTitle.textContent = 'Open Tabs';
        cmdBarResults.appendChild(tabsTitle);

        cachedOpenTabs.forEach(function (t) {
          if (t.url !== 'eclipse://newtab') {
            cmdItems.push({
              id: t.id,
              title: t.title || t.url,
              sub: t.url,
              action: 'switch_tab',
              icon: ICONS.globe
            });
          }
        });
      }

      var favTitle = document.createElement('div');
      favTitle.className = 'section-title';
      favTitle.textContent = 'Quick Actions & Links';
      cmdBarResults.appendChild(favTitle);

      cmdItems.push({
        title: 'Tidy Tabs',
        sub: 'Auto-group & deduplicate tabs (⇧⌘Y)',
        action: 'tidy_tabs',
        icon: ICONS.clean
      });
      cmdItems.push({
        title: 'Eclipse Notes',
        sub: 'Open workspace notes (⇧⌘N)',
        url: 'eclipse://notes',
        icon: ICONS.link
      });
      cmdItems.push({
        title: 'Eclipse Easel',
        sub: 'Open visual canvas board (⇧⌘E)',
        url: 'eclipse://easel',
        icon: ICONS.link
      });

      DEFAULT_FAVORITES.forEach(function (f) {
        cmdItems.push({
          title: f.title,
          sub: f.url,
          url: f.url,
          icon: f.icon
        });
      });

      cmdItems.push({
        title: 'Preferences',
        sub: 'Open Settings (⌘,)',
        action: 'open_settings',
        icon: ICONS.settings
      });
      cmdItems.push({
        title: 'Clean Memory Cache',
        sub: 'Free tab memory',
        action: 'clean_memory',
        icon: ICONS.clean
      });
    }

    cmdItems.forEach(function (item, idx) {
      var el = document.createElement('div');
      el.className = 'item' + (idx === cmdActiveIdx ? ' active' : '');
      var actionBadge = item.action === 'switch_tab' ? '<span class="switch-pill">Switch to Tab →</span>' : '';
      el.innerHTML =
        '<div class="item-icon">' + (item.icon || ICONS.link) + '</div>' +
        '<div class="item-info">' +
        '  <div class="item-title">' + escapeHtml(item.title) + '</div>' +
        '  <div class="item-sub">' + escapeHtml(item.sub || '') + '</div>' +
        '</div>' + actionBadge;
      el.addEventListener('click', function () {
        executeCommand(item);
      });
      cmdBarResults.appendChild(el);
    });
  }

  function updateActiveItem() {
    if (!cmdBarResults) return;
    var elements = cmdBarResults.querySelectorAll('.item');
    elements.forEach(function (el, idx) {
      el.classList.toggle('active', idx === cmdActiveIdx);
      if (idx === cmdActiveIdx) {
        el.scrollIntoView({ block: 'nearest' });
      }
    });
  }

  function executeCommand(item) {
    closeCommandBar();
    if (!item) return;
    if (item.action === 'switch_tab' && item.id !== undefined) {
      post({ action: 'select_tab', id: item.id });
    } else if (item.action === 'open_settings') {
      post({ action: 'open_settings' });
    } else if (item.action === 'clean_memory') {
      post({ action: 'clean_memory' });
    } else if (item.action === 'tidy_tabs') {
      post({ action: 'tidy_tabs' });
    } else if (item.url) {
      post({ action: 'navigate_or_new_tab', url: item.url });
    }
  }

  function executeUrl(val) {
    closeCommandBar();
    var trimmed = (val || '').trim();
    if (!trimmed) return;
    post({ action: 'navigate_or_new_tab', url: trimmed });
  }

  function escapeHtml(str) {
    return String(str || '')
      .replace(/&/g, '&amp;')
      .replace(/</g, '&lt;')
      .replace(/>/g, '&gt;')
      .replace(/"/g, '&quot;');
  }

  function openCommandBar(tabs) {
    if (Array.isArray(tabs)) {
      cachedOpenTabs = tabs;
    }
    ensureCommandBarUi();
    if (!cmdBarHost) return;
    cmdBarHost.style.display = 'flex';
    if (cmdBarInput) {
      cmdBarInput.value = '';
      cmdActiveIdx = 0;
      renderCommandSuggestions('');
      setTimeout(function () {
        cmdBarInput.focus();
        cmdBarInput.select();
      }, 25);
    }
  }

  function closeCommandBar() {
    if (!cmdBarHost) return;
    cmdBarHost.style.display = 'none';
    if (cmdBarInput) cmdBarInput.blur();
  }

  try {
    Object.defineProperty(window, '__eclipseCommandBar', {
      value: function (action, data) {
        if (action === 'open') openCommandBar(data);
        else if (action === 'close') closeCommandBar();
      },
      writable: false,
      configurable: false,
      enumerable: false
    });
  } catch (_) {}

  /* ------------------------------------------------ Fullscreen Detection */
  function notifyFullscreenChange() {
    var isFull = !!(document.fullscreenElement || document.webkitFullscreenElement);
    post({ action: 'set_fullscreen', fullscreen: isFull });
  }
  document.addEventListener('fullscreenchange', notifyFullscreenChange);
  document.addEventListener('webkitfullscreenchange', notifyFullscreenChange);

  /* ------------------------------------------------------------ shortcuts */
  window.addEventListener(
    'keydown',
    function (event) {
      if (!event.isTrusted) return;

      var key = (event.key || '').toLowerCase();
      var meta = event.metaKey && !event.ctrlKey;
      var shift = event.shiftKey;

      if (event.key === 'Escape') {
        if (littleArcHost && littleArcHost.style.display !== 'none') {
          closeLittleArc();
          event.preventDefault();
          return;
        }
        if (cmdBarHost && cmdBarHost.style.display !== 'none') {
          closeCommandBar();
          event.preventDefault();
          return;
        }
        if (findHost && findHost.style.display !== 'none') {
          closeFind();
          event.preventDefault();
          return;
        }
        if (!meta && document.readyState !== 'complete') {
          post({ action: 'stop_loading' });
        }
        return;
      }

      if (event.ctrlKey && !event.metaKey && key === 'tab') {
        event.preventDefault();
        post({ action: shift ? 'select_prev_tab' : 'select_next_tab' });
        return;
      }

      if (meta && event.altKey && key === 'n') {
        event.preventDefault();
        event.stopPropagation();
        openLittleArc(location.href);
        return;
      }

      if (!meta || event.altKey) return;

      if (event.repeat && (key === 'w' || key === 'q' || key === 't')) {
        event.preventDefault();
        return;
      }

      var handled = true;
      if (key === 't' && shift) {
        post({ action: 'reopen_closed_tab' });
      } else if (key === 't') {
        post({ action: 'new_tab', url: 'eclipse://newtab' });
      } else if (key === 'w' && shift) {
        post({ action: 'window_close' });
      } else if (key === 'w') {
        post({ action: 'close_tab' });
      } else if (key === 'r') {
        post({ action: 'reload' });
      } else if (key === 'l' || key === 'k') {
        openCommandBar();
      } else if (key === 'j') {
        post({ action: 'command_bar_open_url', command: 'open_downloads' });
      } else if (key === 'y' && shift) {
        post({ action: 'tidy_tabs' });
      } else if (key === 'n' && shift) {
        post({ action: 'open_notes_tab' });
      } else if (key === 'e' && shift) {
        post({ action: 'open_easel_tab' });
      } else if (key === 's' || (key === 'b' && shift)) {
        post({ action: 'toggle_sidebar' });
      } else if (key === 'p') {
        window.print();
      } else if (key === 'd') {
        post({ action: 'favorite_tab' });
      } else if (event.key === 'ArrowLeft' || event.code === 'BracketLeft' || key === '[' || key === '{') {
        post({ action: shift ? 'select_prev_tab' : 'go_back' });
      } else if (event.key === 'ArrowRight' || event.code === 'BracketRight' || key === ']' || key === '}') {
        post({ action: shift ? 'select_next_tab' : 'go_forward' });
      } else if (key === ',') {
        post({ action: 'open_settings' });
      } else if (key === 'q') {
        post({ action: 'window_close' });
      } else if (key === 'm') {
        post({ action: 'window_minimize' });
      } else if (key === '.') {
        post({ action: 'stop_loading' });
      } else if (key === '+' || key === '=') {
        post({ action: 'zoom_in' });
      } else if (key === '-' || key === '_') {
        post({ action: 'zoom_out' });
      } else if (key === '0') {
        post({ action: 'zoom_reset' });
      } else if (key === 'f') {
        openFind();
      } else if (key === 'g') {
        openFind();
        runFind(shift);
      } else if (key >= '1' && key <= '9' && !shift) {
        post({ action: 'select_tab_index', index: parseInt(key, 10) - 1 });
      } else {
        handled = false;
      }

      if (handled) {
        event.preventDefault();
        event.stopPropagation();
      }
    },
    true
  );
})();
