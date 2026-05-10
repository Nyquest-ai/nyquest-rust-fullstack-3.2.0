/**
 * Nyquest Chrome Extension — Background Service Worker
 * Manages proxy detection, settings, badge updates, and stats.
 */

import './compressor.js';

// ── Defaults ──
const DEFAULTS = {
  enabled: true,
  level: 0.7,
  proxyUrl: 'http://localhost:5400',
  proxyMode: false,   // auto-detected
  perSite: {},        // domain → boolean overrides
  recentEvents: [],   // last 20 compression events for live feed
  stats: {
    totalRequests: 0,
    totalOriginalTokens: 0,
    totalCompressedTokens: 0,
    sessionStart: Date.now(),
  }
};

function addEvent(evt) {
  if (!state.recentEvents) state.recentEvents = [];
  state.recentEvents.push({
    ...evt,
    timestamp: Date.now(),
  });
  // Keep last 20
  if (state.recentEvents.length > 20) {
    state.recentEvents = state.recentEvents.slice(-20);
  }
}

let state = { ...DEFAULTS };

// ── Init ──
chrome.runtime.onInstalled.addListener(() => {
  chrome.storage.local.get(null, (stored) => {
    state = { ...DEFAULTS, ...stored };
    checkProxy();
  });
});

chrome.runtime.onStartup.addListener(() => {
  chrome.storage.local.get(null, (stored) => {
    state = { ...DEFAULTS, ...stored };
    checkProxy();
  });
});

// ── Proxy health check ──
async function checkProxy() {
  try {
    const controller = new AbortController();
    const timeout = setTimeout(() => controller.abort(), 2000);
    const res = await fetch(`${state.proxyUrl}/health`, {
      signal: controller.signal
    });
    clearTimeout(timeout);
    if (res.ok) {
      const data = await res.json();
      state.proxyMode = true;
      state.proxyVersion = data.version || 'unknown';
      updateBadge();
      save();
      return;
    }
  } catch (_) { /* proxy not running */ }
  state.proxyMode = false;
  state.proxyVersion = null;
  updateBadge();
  save();
}

// Re-check proxy every 30s
setInterval(checkProxy, 30000);

// ── Badge ──
function updateBadge() {
  if (!state.enabled) {
    chrome.action.setBadgeText({ text: 'OFF' });
    chrome.action.setBadgeBackgroundColor({ color: '#666' });
    return;
  }
  if (state.proxyMode) {
    chrome.action.setBadgeText({ text: 'PRX' });
    chrome.action.setBadgeBackgroundColor({ color: '#10b981' });
  } else {
    const pct = Math.round(state.level * 100);
    chrome.action.setBadgeText({ text: `${pct}%` });
    chrome.action.setBadgeBackgroundColor({ color: '#4fd1c5' });
  }
}

// ── Persistence ──
function save() {
  chrome.storage.local.set(state);
}

// ── Message handler (from content/popup) ──
chrome.runtime.onMessage.addListener((msg, sender, sendResponse) => {
  switch (msg.type) {
    case 'GET_STATE':
      sendResponse({ ...state });
      return true;

    case 'SET_ENABLED':
      state.enabled = !!msg.value;
      updateBadge();
      save();
      sendResponse({ ok: true });
      return true;

    case 'SET_LEVEL':
      state.level = Math.max(0, Math.min(1, msg.value));
      updateBadge();
      save();
      sendResponse({ ok: true });
      return true;

    case 'SET_SITE_OVERRIDE':
      if (!state.perSite) state.perSite = {};
      if (msg.value === null) {
        delete state.perSite[msg.domain];
      } else {
        state.perSite[msg.domain] = !!msg.value;
      }
      save();
      sendResponse({ ok: true });
      return true;

    case 'CHECK_PROXY':
      checkProxy().then(() => sendResponse({ proxyMode: state.proxyMode, proxyVersion: state.proxyVersion }));
      return true;

    case 'RESET_STATS':
      state.stats = { ...DEFAULTS.stats, sessionStart: Date.now() };
      state.recentEvents = [];
      save();
      sendResponse({ ok: true });
      return true;

    case 'COMPRESS':
      // Content script or popup asks us to compress text
      if (!state.enabled) {
        sendResponse({ text: msg.text, compressed: false });
        return true;
      }
      try {
        const engine = new NyquestCompressor(state.level);
        const result = engine.compress(msg.text);
        const origLen = msg.text.length;
        const compLen = result.text.length;

        // Approximate token count (chars / 4)
        const origTokens = Math.ceil(origLen / 4);
        const compTokens = Math.ceil(compLen / 4);
        const savings = origLen > 0 ? ((1 - compLen / origLen) * 100).toFixed(1) : '0.0';

        // Only count toward stats if from content script (not test panel)
        if (!msg.isTest) {
          state.stats.totalRequests++;
          state.stats.totalOriginalTokens += origTokens;
          state.stats.totalCompressedTokens += compTokens;
          addEvent({
            provider: msg.provider || 'test',
            origTokens,
            compTokens,
            savings,
            ruleHits: result.stats.totalHits,
          });
        }
        save();

        sendResponse({
          text: result.text,
          compressed: true,
          savings,
          ruleHits: result.stats.totalHits,
          origTokens,
          compTokens,
        });
      } catch (e) {
        sendResponse({ text: msg.text, compressed: false, error: e.message });
      }
      return true;

    case 'LOG_PROXY_RESULT':
      // Content script reports compression stats (from interceptor)
      state.stats.totalRequests++;
      state.stats.totalOriginalTokens += msg.origTokens || 0;
      state.stats.totalCompressedTokens += msg.compTokens || 0;
      addEvent({
        provider: msg.provider || 'unknown',
        origTokens: msg.origTokens || 0,
        compTokens: msg.compTokens || 0,
        savings: msg.savings || '0.0',
        ruleHits: msg.ruleHits || 0,
      });
      save();
      sendResponse({ ok: true });
      return true;

    default:
      sendResponse({ error: 'unknown message type' });
      return true;
  }
});
