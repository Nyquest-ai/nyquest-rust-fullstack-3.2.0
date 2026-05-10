/**
 * Nyquest Content Script — ISOLATED world
 * Uses chrome.scripting API to inject into MAIN world (bypasses CSP).
 * Relays config/stats to background, renders floating HUD overlay.
 */

(function () {
  'use strict';

  // ── State ──
  let hudVisible = true;
  let lastSavings = null;
  let requestCount = 0;

  // ── Send config to MAIN world interceptor via window message ──
  function pushConfig(state) {
    window.postMessage({
      type: '__NYQUEST_CONFIG__',
      config: {
        enabled: state.enabled,
        level: state.level,
        proxyMode: state.proxyMode,
        proxyUrl: state.proxyUrl,
      }
    }, '*');
  }

  // Get initial state and push to MAIN world
  chrome.runtime.sendMessage({ type: 'GET_STATE' }, (state) => {
    if (state) {
      pushConfig(state);
      updateHud(state);
    }
  });

  // Listen for settings changes from popup
  chrome.runtime.onMessage.addListener((msg) => {
    if (msg.type === 'CONFIG_UPDATED') {
      chrome.runtime.sendMessage({ type: 'GET_STATE' }, (state) => {
        if (state) {
          pushConfig(state);
          updateHud(state);
        }
      });
    }
  });

  // ── Listen for stats from MAIN world interceptor ──
  window.addEventListener('message', (e) => {
    if (!e.data) return;

    if (e.data.type === '__NYQUEST_STATS__') {
      requestCount++;
      lastSavings = e.data.savings;
      showSavingsToast(e.data.savings, e.data.provider);
      chrome.runtime.sendMessage({
        type: 'LOG_PROXY_RESULT',
        origTokens: e.data.origTokens,
        compTokens: e.data.compTokens,
        savings: e.data.savings,
        provider: e.data.provider,
      });
    }

    if (e.data.type === '__NYQUEST_PROXY__') {
      requestCount++;
      showSavingsToast('PRX', e.data.provider);
    }
  });

  // ── HUD Overlay ──
  let hudEl = null;
  let toastEl = null;

  function createHud() {
    if (hudEl) return;

    hudEl = document.createElement('div');
    hudEl.id = 'nyquest-hud';
    hudEl.innerHTML = `
      <div id="nyquest-hud-inner">
        <div id="nyquest-hud-logo">⚡ NQ</div>
        <div id="nyquest-hud-status">--</div>
      </div>
    `;

    const style = document.createElement('style');
    style.textContent = `
      #nyquest-hud {
        position: fixed; bottom: 16px; right: 16px; z-index: 999999;
        font-family: Consolas, monospace; pointer-events: none; transition: opacity 0.3s;
      }
      #nyquest-hud.hidden { opacity: 0; }
      #nyquest-hud-inner {
        display: flex; align-items: center; gap: 6px;
        background: rgba(10,11,14,0.85); backdrop-filter: blur(8px);
        border: 1px solid rgba(79,209,197,0.3); border-radius: 8px;
        padding: 6px 12px; color: #4fd1c5; font-size: 12px; font-weight: 600;
        box-shadow: 0 2px 12px rgba(0,0,0,0.4);
      }
      #nyquest-hud-logo { font-size: 11px; opacity: 0.7; }
      #nyquest-hud-status { font-variant-numeric: tabular-nums; }
      #nyquest-toast {
        position: fixed; bottom: 52px; right: 16px; z-index: 999998;
        font-family: Consolas, monospace; pointer-events: none;
        opacity: 0; transform: translateY(8px); transition: opacity 0.25s, transform 0.25s;
      }
      #nyquest-toast.show { opacity: 1; transform: translateY(0); }
      #nyquest-toast-inner {
        background: rgba(16,185,129,0.9); backdrop-filter: blur(8px);
        border-radius: 6px; padding: 4px 10px; color: #fff;
        font-size: 11px; font-weight: 700; box-shadow: 0 2px 8px rgba(0,0,0,0.3);
      }
      #nyquest-toast-inner.proxy { background: rgba(79,209,197,0.9); }
    `;

    toastEl = document.createElement('div');
    toastEl.id = 'nyquest-toast';
    toastEl.innerHTML = '<div id="nyquest-toast-inner"></div>';

    document.documentElement.appendChild(style);
    document.documentElement.appendChild(hudEl);
    document.documentElement.appendChild(toastEl);
  }

  function updateHud(state) {
    createHud();
    const statusEl = document.getElementById('nyquest-hud-status');
    if (!statusEl) return;

    if (!state.enabled) {
      statusEl.textContent = 'OFF';
      statusEl.style.color = '#666';
      hudEl.classList.add('hidden');
      return;
    }

    hudEl.classList.remove('hidden');
    if (state.proxyMode) {
      statusEl.textContent = `PROXY v${state.proxyVersion || '?'}`;
      statusEl.style.color = '#10b981';
    } else {
      statusEl.textContent = `L${state.level.toFixed(1)} · ${requestCount} req`;
      statusEl.style.color = '#4fd1c5';
    }
  }

  function showSavingsToast(savings, provider) {
    createHud();
    const inner = document.getElementById('nyquest-toast-inner');
    if (!inner) return;

    if (savings === 'PRX') {
      inner.textContent = `→ Proxy (${provider})`;
      inner.className = 'proxy';
    } else {
      inner.textContent = `−${savings}% (${provider})`;
      inner.className = '';
    }

    toastEl.classList.add('show');
    setTimeout(() => toastEl.classList.remove('show'), 2500);

    const statusEl = document.getElementById('nyquest-hud-status');
    if (statusEl) {
      chrome.runtime.sendMessage({ type: 'GET_STATE' }, (state) => {
        if (state) updateHud(state);
      });
    }
  }

  if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', createHud);
  } else {
    createHud();
  }
})();
