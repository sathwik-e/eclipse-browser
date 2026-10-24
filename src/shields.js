/*
 * Eclipse privacy shields.
 *
 * These are cosmetic/content filters (the WebView does not expose a network
 * content-blocker list through wry), so be honest about what they can do:
 * hide common ad containers, skip YouTube pre-rolls, and advertise DNT/GPC.
 * The module can be switched on and off at runtime from Preferences.
 */
(function () {
  'use strict';
  if (window.__eclipseShieldsReady) return;
  window.__eclipseShieldsReady = true;

  var TOKEN = '__ECLIPSE_TOKEN__';
  var enabled = false;
  var styleEl = null;
  var observer = null;
  var rafPending = false;
  var watcherRetries = 0;

  var COSMETIC_CSS = [
    '.adsbygoogle, [id^="google_ads_"], .ad-banner, .ad-unit,',
    '.advertisement, .ad-container, [data-ad-slot], .ytp-ad-overlay-container,',
    '.ytp-ad-message-container, ytd-promoted-sparkles-web-renderer,',
    'ytd-display-ad-renderer, ytd-banner-promo-renderer,',
    'ytd-ad-slot-renderer, #player-ads, .sparkles-light-cta,',
    'div[id*="ad-slot"], div[class*="ad_unit"], div[class*="Sponsored"],',
    '.trc_related_container, .outbrain, .taboola {',
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

  function skipYouTubeAd() {
    rafPending = false;
    var skipButton = document.querySelector(
      '.ytp-ad-skip-button, .ytp-skip-ad-button, .ytp-ad-skip-button-modern'
    );
    if (skipButton) skipButton.click();

    var closeButton = document.querySelector('.ytp-ad-overlay-close-button');
    if (closeButton) closeButton.click();

    if (!document.querySelector('.ad-showing, .ad-interrupting')) return;
    var video = document.querySelector('video');
    if (video && isFinite(video.duration) && video.duration > 0) {
      try {
        video.currentTime = video.duration;
      } catch (_) {}
    }
  }

  function startYouTubeWatcher() {
    if (observer || !isYouTube()) return;
    if (!document.documentElement) {
      if (watcherRetries++ < 20) {
        setTimeout(startYouTubeWatcher, 300);
      }
      return;
    }
    var target = document.getElementById('movie_player') || document.body;
    if (!target) {
      if (watcherRetries++ < 20) {
        setTimeout(startYouTubeWatcher, 600);
      }
      return;
    }
    watcherRetries = 0;
    observer = new MutationObserver(function () {
      if (rafPending) return;
      rafPending = true;
      requestAnimationFrame(skipYouTubeAd);
    });
    observer.observe(target, {
      childList: true,
      subtree: true,
      attributes: true,
      attributeFilter: ['class']
    });
    skipYouTubeAd();
  }

  function stopYouTubeWatcher() {
    if (!observer) return;
    observer.disconnect();
    observer = null;
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
