/*
 * Eclipse privacy shields & real content blocker.
 *
 * Real network request blocking is handled natively by Apple WebKit's
 * WKContentRuleList DFA engine at the networking layer. This module provides
 * zero-jank in-page stream and player neutralization:
 *  1. Intercepts YouTube player responses (ytInitialPlayerResponse, fetch, XHR)
 *     to strip adPlacements, playerAds, adSlots, and adConfig at the data layer.
 *     The interceptors are only installed once blocking is enabled for the page.
 *  2. Fast-forwards / auto-dismisses any residual video ad interstitials with
 *     an O(1) class watcher on #movie_player (no full-DOM polling, and no timer
 *     at all while no ad is playing).
 *  3. Injects fast, flat CSS rules (no expensive :has() or universal substring selectors)
 *     so scrolling and layout remain locked at 120Hz.
 *  4. Enforces strict Do Not Track (DNT) and Global Privacy Control (GPC).
 */
(function () {
  'use strict';
  if (window.__eclipseShieldsReady || location.protocol === 'eclipse:') return;
  window.__eclipseShieldsReady = true;

  var CONTROL = '__ECLIPSE_CONTROL__';
  var enabled = false;
  var styleEl = null;
  var playerObserver = null;
  var observedPlayer = null;
  var findPlayerTimer = null;
  var adTimer = null;
  var cachedPlayer = null;
  var cachedVideo = null;
  // The media element changed while an ad played, and what it was before,
  // so it can be put back exactly as the user had it.
  var adVideo = null;
  var savedMuted = false;
  var savedRate = 1;

  var COSMETIC_CSS = [
    '/* Ad-network containers (flat O(1) selectors for zero style-recalc jank).',
    '   Generic names such as .advertisement or .ad-container are left alone:',
    '   too many sites use them for real content. */',
    '.adsbygoogle, [id^="google_ads_"],',
    '.trc_related_container, .outbrain, .taboola,',
    '',
    '/* YouTube Feed, Search & Watch Page Ad Slots */',
    'ytd-ad-slot-renderer,',
    'ytd-in-feed-ad-layout-renderer,',
    'ytd-banner-promo-renderer,',
    'ytd-statement-banner-renderer,',
    'ytd-promoted-sparkles-web-renderer,',
    'ytd-display-ad-renderer,',
    'ytd-promoted-video-renderer,',
    'ytd-compact-promoted-video-renderer,',
    'ytd-player-legacy-desktop-watch-ads-renderer,',
    'ytd-ads-engagement-panel-content-renderer,',
    'ytd-engagement-panel-section-list-renderer[target-id="engagement-panel-ads"],',
    '#masthead-ad,',
    '#player-ads,',
    '.ytd-carousel-ad-renderer,',
    '.sparkles-light-cta,',
    'ytd-search-pyv-renderer,',
    '',
    '/* YouTube Video Player Ad Overlays & Modules */',
    '.ytp-ad-overlay-container,',
    '.ytp-ad-message-container,',
    '.ytp-ad-progress,',
    '.ytp-ad-progress-list,',
    '.ytp-ad-player-overlay,',
    '.ytp-ad-player-overlay-layout,',
    '.ytp-ad-image-overlay,',
    '.ytp-ad-text-overlay,',
    '.ad-showing .ytp-ad-player-overlay,',
    '.ad-showing .ytp-ad-overlay-container,',
    '.ad-showing .ytp-ad-module,',
    'ytd-enforcement-message-view-model {',
    '  display: none !important;',
    '  visibility: hidden !important;',
    '  height: 0 !important;',
    '  max-height: 0 !important;',
    '  overflow: hidden !important;',
    '  opacity: 0 !important;',
    '  pointer-events: none !important;',
    '}'
  ].join('\n');

  function installPrivacyFlags() {
    try {
      Object.defineProperty(navigator, 'doNotTrack', { value: '1', configurable: false });
    } catch (_) {}
    try {
      Object.defineProperty(navigator, 'globalPrivacyControl', { value: true, configurable: false });
    } catch (_) {}
  }

  function ensureStyle() {
    if (styleEl || !document.documentElement) return;
    styleEl = document.createElement('style');
    styleEl.id = '__eclipse_adblock_css';
    styleEl.textContent = COSMETIC_CSS;
    (document.head || document.documentElement).appendChild(styleEl);
  }

  function removeStyle() {
    if (!styleEl) return;
    if (styleEl.parentNode) styleEl.parentNode.removeChild(styleEl);
    styleEl = null;
  }

  function isYouTube() {
    return /(^|\.)youtube(-nocookie)?\.com$/i.test(location.hostname) || /(^|\.)youtu\.be$/i.test(location.hostname);
  }

  /* ------------------------------------------------------------- YouTube Stream Neutralizer */
  function sanitizePlayerData(data) {
    if (!data || typeof data !== 'object') return data;
    try {
      if (data.adPlacements) delete data.adPlacements;
      if (data.playerAds) delete data.playerAds;
      if (data.adSlots) delete data.adSlots;
      if (data.adBreakHeartbeatParams) delete data.adBreakHeartbeatParams;
      if (data.playerConfig && data.playerConfig.adConfig) {
        delete data.playerConfig.adConfig;
      }
    } catch (_) {}
    return data;
  }

  function sanitizePlayerJson(jsonStr) {
    if (typeof jsonStr !== 'string' || !jsonStr) return jsonStr;
    // Fast-path: skip expensive JSON.parse/stringify if no ad keys exist in payload
    if (
      jsonStr.indexOf('"adPlacements"') === -1 &&
      jsonStr.indexOf('"playerAds"') === -1 &&
      jsonStr.indexOf('"adSlots"') === -1 &&
      jsonStr.indexOf('"adBreakHeartbeatParams"') === -1 &&
      jsonStr.indexOf('"adConfig"') === -1
    ) {
      return jsonStr;
    }
    try {
      var data = JSON.parse(jsonStr);
      sanitizePlayerData(data);
      return JSON.stringify(data);
    } catch (_) {
      return jsonStr;
    }
  }

  var ytInterceptorsInstalled = false;
  function installYouTubeInterceptors() {
    if (ytInterceptorsInstalled || !isYouTube()) return;
    ytInterceptorsInstalled = true;

    // 1. Intercept window.ytInitialPlayerResponse
    try {
      var _ytResp = (enabled && window.ytInitialPlayerResponse)
        ? sanitizePlayerData(window.ytInitialPlayerResponse)
        : window.ytInitialPlayerResponse;
      Object.defineProperty(window, 'ytInitialPlayerResponse', {
        get: function () { return _ytResp; },
        set: function (val) {
          _ytResp = enabled ? sanitizePlayerData(val) : val;
        },
        configurable: true,
        enumerable: true
      });
    } catch (_) {}

    // 2. Intercept window.fetch to drop ad-break endpoints and sanitize player responses
    try {
      var originalFetch = window.fetch;
      if (typeof originalFetch === 'function') {
        window.fetch = function (resource, init) {
          if (!enabled) {
            return originalFetch.apply(this, arguments);
          }
          var url = (typeof resource === 'string') ? resource : (resource && resource.url) ? resource.url : '';
          if (url && typeof url === 'string') {
            if (
              url.indexOf('/youtubei/v1/player/ad_break') !== -1 ||
              url.indexOf('/api/stats/ads') !== -1 ||
              url.indexOf('/pagead/') !== -1 ||
              url.indexOf('/ptracking') !== -1
            ) {
              return Promise.resolve(new Response('{}', {
                status: 200,
                headers: { 'Content-Type': 'application/json' }
              }));
            }
          }

          var fetchPromise = originalFetch.apply(this, arguments);

          if (url && typeof url === 'string' && url.indexOf('/youtubei/v1/player') !== -1 && url.indexOf('/ad_break') === -1) {
            return fetchPromise.then(function (response) {
              if (!enabled) return response;
              try {
                return response.text().then(function (text) {
                  var sanitized = sanitizePlayerJson(text);
                  return new Response(sanitized, {
                    status: response.status,
                    statusText: response.statusText,
                    headers: response.headers
                  });
                });
              } catch (_) {
                return response;
              }
            });
          }
          return fetchPromise;
        };
      }
    } catch (_) {}

    // 3. Intercept XMLHttpRequest
    try {
      var originalOpen = XMLHttpRequest.prototype.open;
      var originalSend = XMLHttpRequest.prototype.send;
      XMLHttpRequest.prototype.open = function (method, url) {
        this.__eclipseUrl = (typeof url === 'string') ? url : '';
        return originalOpen.apply(this, arguments);
      };
      XMLHttpRequest.prototype.send = function () {
        if (!enabled) {
          return originalSend.apply(this, arguments);
        }
        var url = this.__eclipseUrl || '';
        if (
          url.indexOf('/youtubei/v1/player/ad_break') !== -1 ||
          url.indexOf('/api/stats/ads') !== -1 ||
          url.indexOf('/pagead/') !== -1 ||
          url.indexOf('/ptracking') !== -1
        ) {
          var self = this;
          setTimeout(function () {
            try {
              Object.defineProperty(self, 'readyState', { value: 4 });
              Object.defineProperty(self, 'status', { value: 200 });
              Object.defineProperty(self, 'responseText', { value: '{}' });
              Object.defineProperty(self, 'response', { value: '{}' });
              if (typeof self.onreadystatechange === 'function') self.onreadystatechange();
              if (typeof self.onload === 'function') self.onload();
            } catch (_) {}
          }, 0);
          return;
        }

        if (url.indexOf('/youtubei/v1/player') !== -1 && url.indexOf('/ad_break') === -1) {
          var xhr = this;
          var origTextGetter = Object.getOwnPropertyDescriptor(XMLHttpRequest.prototype, 'responseText');
          var origRespGetter = Object.getOwnPropertyDescriptor(XMLHttpRequest.prototype, 'response');
          var sanitizedText = null;
          var sanitizedObj = null;

          Object.defineProperty(xhr, 'responseText', {
            get: function () {
              if (!enabled) return origTextGetter ? origTextGetter.get.call(xhr) : '';
              if (sanitizedText !== null) return sanitizedText;
              var raw = origTextGetter ? origTextGetter.get.call(xhr) : '';
              sanitizedText = sanitizePlayerJson(raw);
              return sanitizedText;
            },
            configurable: true
          });

          Object.defineProperty(xhr, 'response', {
            get: function () {
              if (!enabled) return origRespGetter ? origRespGetter.get.call(xhr) : null;
              if (xhr.responseType === '' || xhr.responseType === 'text') {
                return xhr.responseText;
              }
              if (xhr.responseType === 'json') {
                if (sanitizedObj !== null) return sanitizedObj;
                var rawObj = origRespGetter ? origRespGetter.get.call(xhr) : null;
                sanitizedObj = sanitizePlayerData(rawObj);
                return sanitizedObj;
              }
              return origRespGetter ? origRespGetter.get.call(xhr) : null;
            },
            configurable: true
          });
        }
        return originalSend.apply(this, arguments);
      };
    } catch (_) {}
  }

  /* ------------------------------------------------------------- YouTube Player O(1) Auto-Skipper */
  function getPlayer() {
    if (cachedPlayer && cachedPlayer.isConnected) return cachedPlayer;
    cachedPlayer = document.getElementById('movie_player') || document.querySelector('.html5-video-player');
    if (cachedPlayer && cachedPlayer !== observedPlayer && playerObserver) {
      try {
        playerObserver.disconnect();
        playerObserver.observe(cachedPlayer, { attributes: true, attributeFilter: ['class'] });
        observedPlayer = cachedPlayer;
      } catch (_) {}
    }
    return cachedPlayer;
  }

  function getVideo(player) {
    if (cachedVideo && cachedVideo.isConnected) return cachedVideo;
    cachedVideo = (player && player.querySelector('video')) || document.querySelector('video');
    return cachedVideo;
  }

  var SKIP_SELECTORS = '.ytp-ad-skip-button-modern, .ytp-skip-ad-button, .ytp-ad-skip-button, .ytp-ad-skip-button-slot button, .ytp-ad-overlay-close-button';
  var AD_RATE = 16;

  // Puts mute and speed back, but only where they still hold the values we
  // set: YouTube restores the user's own speed after an ad by itself.
  function restoreAdVideo() {
    if (!adVideo) return;
    var video = adVideo;
    adVideo = null;
    try {
      if (video.playbackRate === AD_RATE) video.playbackRate = savedRate;
      if (video.muted && !savedMuted) video.muted = false;
    } catch (_) {}
  }

  function stopAdTimer() {
    if (adTimer) {
      clearInterval(adTimer);
      adTimer = null;
    }
  }

  function skipYouTubeAd() {
    if (!enabled || !isYouTube()) return;

    var player = getPlayer();
    if (!player) return;

    // O(1) class check on #movie_player — zero DOM traversal when no ad is playing
    var classList = player.classList;
    var isAdActive = classList.contains('ad-showing') || classList.contains('ad-interrupting');

    if (!isAdActive) {
      stopAdTimer();
      restoreAdVideo();
      return;
    }

    // Ads can take a few ticks to become skippable; poll only while one shows.
    if (!adTimer) adTimer = setInterval(skipYouTubeAd, 250);

    if (typeof player.skipAd === 'function') {
      try { player.skipAd(); } catch (_) {}
    }

    var btns = player.querySelectorAll(SKIP_SELECTORS);
    for (var i = 0; i < btns.length; i++) {
      try { btns[i].click(); } catch (_) {}
    }

    var video = getVideo(player);
    if (!video) return;
    try {
      // Capture the user's state from the element itself, the first time we
      // actually touch it, never from a default.
      if (adVideo !== video) {
        restoreAdVideo();
        adVideo = video;
        savedMuted = video.muted;
        savedRate = video.playbackRate === AD_RATE ? 1 : video.playbackRate;
      }
      video.muted = true;
      video.playbackRate = AD_RATE;
      if (isFinite(video.duration) && video.duration > 0.1) {
        video.currentTime = video.duration - 0.05;
      }
    } catch (_) {}
  }

  function stopFindPlayer() {
    if (findPlayerTimer) {
      clearInterval(findPlayerTimer);
      findPlayerTimer = null;
    }
  }

  // The player appears after load and is replaced on SPA navigation. Look for
  // it for a while, then rely on the class observer alone.
  function findPlayer() {
    stopFindPlayer();
    if (getPlayer()) {
      skipYouTubeAd();
      return;
    }
    var attempts = 0;
    findPlayerTimer = setInterval(function () {
      attempts += 1;
      if (getPlayer()) {
        stopFindPlayer();
        skipYouTubeAd();
      } else if (attempts >= 60) {
        stopFindPlayer();
      }
    }, 500);
  }

  function startYouTubeWatcher() {
    if (!isYouTube()) return;
    stopYouTubeWatcher();

    // Observe ONLY #movie_player's class attribute instead of the entire document subtree
    playerObserver = new MutationObserver(skipYouTubeAd);
    observedPlayer = null;
    cachedPlayer = null;
    window.addEventListener('yt-navigate-finish', findPlayer, { passive: true });
    findPlayer();
  }

  function stopYouTubeWatcher() {
    if (playerObserver) {
      playerObserver.disconnect();
      playerObserver = null;
    }
    observedPlayer = null;
    window.removeEventListener('yt-navigate-finish', findPlayer, { passive: true });
    stopFindPlayer();
    stopAdTimer();
    restoreAdVideo();
  }

  function setEnabled(next) {
    enabled = !!next;
    if (!enabled) {
      removeStyle();
      stopYouTubeWatcher();
      return;
    }
    installPrivacyFlags();
    if (document.head || document.documentElement) {
      ensureStyle();
    } else {
      document.addEventListener('DOMContentLoaded', ensureStyle, { once: true });
    }
    if (isYouTube()) {
      installYouTubeInterceptors();
      if (document.readyState === 'loading') {
        document.addEventListener('DOMContentLoaded', startYouTubeWatcher, { once: true });
      } else {
        startYouTubeWatcher();
      }
    }
  }

  try {
    Object.defineProperty(window, '__eclipseShields', {
      value: function (secret, next) {
        if (secret !== CONTROL) return;
        setEnabled(next);
      },
      writable: false,
      configurable: false,
      enumerable: false
    });
  } catch (_) {}

  setEnabled(__ECLIPSE_ENABLED__);
})();
