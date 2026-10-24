/*
 * Eclipse privacy shields & real content blocker.
 *
 * Real network request blocking is handled natively by Apple WebKit's
 * WKContentRuleList DFA engine at the networking layer. This module provides
 * the in-page stream and player neutralization:
 *  1. Intercepts YouTube player responses (ytInitialPlayerResponse, fetch, XHR)
 *     to strip adPlacements, playerAds, and adSlots at the data layer.
 *  2. Fast-forwards / auto-dismisses video ad interstitials.
 *  3. Injects zero-layout CSS rules to ensure no ad placeholders flash.
 *  4. Enforces strict Do Not Track (DNT) and Global Privacy Control (GPC).
 */
(function () {
  'use strict';
  if (window.__eclipseShieldsReady) return;
  window.__eclipseShieldsReady = true;

  var TOKEN = '__ECLIPSE_TOKEN__';
  var enabled = false;
  var styleEl = null;
  var observer = null;
  var adSkipTimer = null;
  var rafPending = false;

  var COSMETIC_CSS = [
    '.adsbygoogle, [id^="google_ads_"], .ad-banner, .ad-unit,',
    '.advertisement, .ad-container, [data-ad-slot],',
    '.ytp-ad-overlay-container, .ytp-ad-message-container,',
    'ytd-ad-slot-renderer, #player-ads, .sparkles-light-cta,',
    'ytd-promoted-sparkles-web-renderer, ytd-display-ad-renderer,',
    'ytd-banner-promo-renderer, ytd-statement-banner-renderer,',
    '.trc_related_container, .outbrain, .taboola,',
    '[aria-label*="Sponsored" i], [aria-label*="advertisement" i] {',
    'display: none !important; visibility: hidden !important;',
    'height: 0 !important; max-height: 0 !important; overflow: hidden !important;',
    'opacity: 0 !important; pointer-events: none !important;',
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
    return /(^|\.)youtube\.com$/i.test(location.hostname) || /(^|\.)youtu\.be$/i.test(location.hostname);
  }

  /* ------------------------------------------------------------- YouTube Stream Neutralizer */
  function sanitizePlayerData(data) {
    if (!data || typeof data !== 'object') return data;
    try {
      if (data.adPlacements) delete data.adPlacements;
      if (data.playerAds) delete data.playerAds;
      if (data.adSlots) delete data.adSlots;
      if (data.adBreakHeartbeatParams) delete data.adBreakHeartbeatParams;
    } catch (_) {}
    return data;
  }

  function sanitizePlayerJson(jsonStr) {
    if (typeof jsonStr !== 'string' || !jsonStr) return jsonStr;
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
      if (window.ytInitialPlayerResponse) {
        sanitizePlayerData(window.ytInitialPlayerResponse);
      }
      var _ytResp = window.ytInitialPlayerResponse;
      Object.defineProperty(window, 'ytInitialPlayerResponse', {
        get: function () { return _ytResp; },
        set: function (val) { _ytResp = sanitizePlayerData(val); },
        configurable: true
      });
    } catch (_) {}

    // 2. Intercept window.fetch
    try {
      var originalFetch = window.fetch;
      if (typeof originalFetch === 'function') {
        window.fetch = function (resource, init) {
          var url = (typeof resource === 'string') ? resource : (resource && resource.url) ? resource.url : '';
          if (url && typeof url === 'string') {
            if (url.indexOf('/youtubei/v1/player/ad_break') !== -1 || url.indexOf('/api/stats/ads') !== -1) {
              return Promise.resolve(new Response('{}', {
                status: 200,
                headers: { 'Content-Type': 'application/json' }
              }));
            }
          }

          var promise = originalFetch.apply(this, arguments);
          if (url && typeof url === 'string' && url.indexOf('/youtubei/v1/player') !== -1 && url.indexOf('/ad_break') === -1) {
            return promise.then(function (response) {
              try {
                var clone = response.clone();
                var origJson = clone.json.bind(clone);
                var origText = clone.text.bind(clone);

                response.json = function () {
                  return origJson().then(function (data) {
                    return sanitizePlayerData(data);
                  });
                };
                response.text = function () {
                  return origText().then(function (text) {
                    return sanitizePlayerJson(text);
                  });
                };
              } catch (_) {}
              return response;
            });
          }
          return promise;
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
        var url = this.__eclipseUrl || '';
        if (url.indexOf('/youtubei/v1/player/ad_break') !== -1 || url.indexOf('/api/stats/ads') !== -1) {
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
          var origGetter = Object.getOwnPropertyDescriptor(XMLHttpRequest.prototype, 'responseText');
          var sanitized = null;
          Object.defineProperty(xhr, 'responseText', {
            get: function () {
              if (sanitized !== null) return sanitized;
              var raw = origGetter ? origGetter.get.call(xhr) : '';
              sanitized = sanitizePlayerJson(raw);
              return sanitized;
            },
            configurable: true
          });
        }
        return originalSend.apply(this, arguments);
      };
    } catch (_) {}
  }

  /* ------------------------------------------------------------- YouTube Player Auto-Skipper */
  function skipYouTubeAd() {
    rafPending = false;
    var player = document.getElementById('movie_player') || document.querySelector('.html5-video-player');
    if (!player) return;

    var skipButton = player.querySelector(
      '.ytp-ad-skip-button, .ytp-skip-ad-button, .ytp-ad-skip-button-modern, .ytp-ad-skip-button-slot, button.ytp-ad-skip-button-text'
    );
    if (skipButton) {
      skipButton.click();
      return;
    }

    var closeButton = player.querySelector('.ytp-ad-overlay-close-button');
    if (closeButton) closeButton.click();

    var ad = player.querySelector('.ad-showing, .ad-interrupting');
    if (ad) {
      var video = player.querySelector('video') || document.querySelector('video');
      if (video && isFinite(video.duration) && video.duration > 0) {
        try {
          video.muted = true;
          video.currentTime = video.duration;
        } catch (_) {}
      }
    }
  }

  function startYouTubeWatcher() {
    if (observer || !isYouTube()) return;
    var player = document.getElementById('movie_player') || document.querySelector('.html5-video-player');
    if (!player) {
      setTimeout(startYouTubeWatcher, 200);
      return;
    }
    observer = new MutationObserver(function () {
      if (rafPending) return;
      rafPending = true;
      requestAnimationFrame(skipYouTubeAd);
    });
    observer.observe(player, {
      childList: true,
      subtree: true,
      attributes: true,
      attributeFilter: ['class']
    });
    skipYouTubeAd();

    if (!adSkipTimer) {
      adSkipTimer = setInterval(skipYouTubeAd, 500);
    }
  }

  function stopYouTubeWatcher() {
    if (observer) {
      observer.disconnect();
      observer = null;
    }
    if (adSkipTimer) {
      clearInterval(adSkipTimer);
      adSkipTimer = null;
    }
    rafPending = false;
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

  // Preemptively install YouTube data interceptors immediately upon script injection
  if (isYouTube()) {
    installYouTubeInterceptors();
  }

  try {
    Object.defineProperty(window, '__eclipseShields', {
      value: function (secret, next) {
        if (secret !== TOKEN) return;
        setEnabled(next);
      },
      writable: false,
      configurable: false,
      enumerable: false
    });
  } catch (_) {}

  setEnabled(__ECLIPSE_ENABLED__);
})();
