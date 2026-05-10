/**
 * Nyquest Popup Controller
 * Reads state from background, updates UI, sends changes back.
 * Includes live feed, test compression, and animated stats.
 */

(function () {
  'use strict';

  const $ = (sel) => document.querySelector(sel);
  const $$ = (sel) => document.querySelectorAll(sel);

  const toggleEl = $('#toggle-enabled');
  const sliderEl = $('#level-slider');
  const levelValEl = $('#level-value');
  const modeDot = $('#mode-dot');
  const modeLabel = $('#mode-label');
  const resetBtn = $('#reset-btn');
  const feedEl = $('#feed');
  const testInput = $('#test-input');
  const testBtn = $('#test-btn');
  const testResult = $('#test-result');
  const barFill = $('#bar-fill');
  const barPct = $('#bar-pct');

  let lastRequestCount = 0;
  let pollInterval = null;

  function formatNum(n) {
    if (n >= 1000000) return (n / 1000000).toFixed(1) + 'M';
    if (n >= 1000) return (n / 1000).toFixed(1) + 'k';
    return String(n);
  }

  function flashCard(id) {
    const card = $(`#${id}`);
    if (!card) return;
    card.classList.add('flash');
    setTimeout(() => card.classList.remove('flash'), 600);
  }

  function render(state) {
    // Toggle
    toggleEl.checked = state.enabled;

    // Level
    sliderEl.value = state.level;
    levelValEl.textContent = state.level.toFixed(1);

    // Mode indicator
    if (!state.enabled) {
      modeDot.className = 'mode-dot off';
      modeLabel.innerHTML = '<strong>Disabled</strong>';
    } else if (state.proxyMode) {
      modeDot.className = 'mode-dot proxy';
      modeLabel.innerHTML = `<strong>Full Pipeline</strong> · Proxy v${state.proxyVersion || '?'}`;
    } else {
      modeDot.className = 'mode-dot';
      modeLabel.innerHTML = '<strong>Rules Only</strong> · 350+ patterns';
    }

    // Stats
    const s = state.stats || {};
    const requests = s.totalRequests || 0;
    const origTokens = s.totalOriginalTokens || 0;
    const compTokens = s.totalCompressedTokens || 0;
    const savedTokens = origTokens - compTokens;
    const avgSavings = origTokens > 0
      ? ((1 - compTokens / origTokens) * 100).toFixed(1)
      : '0.0';

    // Flash on new request
    if (requests > lastRequestCount && lastRequestCount > 0) {
      flashCard('card-requests');
      flashCard('card-savings');
      flashCard('card-saved');
    }
    lastRequestCount = requests;

    $('#stat-requests').textContent = formatNum(requests);
    $('#stat-savings').textContent = avgSavings + '%';
    $('#stat-orig-tokens').textContent = formatNum(origTokens);
    $('#stat-saved-tokens').textContent = formatNum(savedTokens);

    // Savings bar
    const pct = parseFloat(avgSavings);
    barFill.style.width = Math.min(pct, 100) + '%';
    barPct.textContent = avgSavings + '%';

    // Live feed from recent events
    if (state.recentEvents && state.recentEvents.length > 0) {
      renderFeed(state.recentEvents);
    }
  }

  function renderFeed(events) {
    if (!events || events.length === 0) return;

    // Clear empty state
    const empty = feedEl.querySelector('.feed-empty');
    if (empty) empty.remove();

    // Only add new events
    const existingCount = feedEl.querySelectorAll('.feed-item').length;
    const newEvents = events.slice(existingCount);

    for (const evt of newEvents) {
      const item = document.createElement('div');
      item.className = 'feed-item';

      const savingsVal = parseFloat(evt.savings || 0);
      const savingsClass = savingsVal >= 15 ? '' : savingsVal >= 5 ? 'low' : 'none';
      const providerClass = (evt.provider || 'unknown').toLowerCase();

      item.innerHTML = `
        <span class="feed-provider ${providerClass}">${evt.provider || '?'}</span>
        <span class="feed-detail"><strong>${formatNum(evt.origTokens || 0)}</strong> → <strong>${formatNum(evt.compTokens || 0)}</strong> tok</span>
        <span class="feed-savings ${savingsClass}">−${evt.savings || '0'}%</span>
      `;

      feedEl.insertBefore(item, feedEl.firstChild);
    }

    // Keep max 20 items
    while (feedEl.querySelectorAll('.feed-item').length > 20) {
      feedEl.removeChild(feedEl.lastElementChild);
    }
  }

  // ── Load state + start polling ──
  function refresh() {
    chrome.runtime.sendMessage({ type: 'GET_STATE' }, (state) => {
      if (state) render(state);
    });
  }

  refresh();
  // Poll every 1s for live updates
  pollInterval = setInterval(refresh, 1000);

  // ── Toggle ──
  toggleEl.addEventListener('change', () => {
    chrome.runtime.sendMessage({ type: 'SET_ENABLED', value: toggleEl.checked }, () => {
      refresh();
      chrome.tabs.query({ active: true, currentWindow: true }, (tabs) => {
        if (tabs[0]) chrome.tabs.sendMessage(tabs[0].id, { type: 'CONFIG_UPDATED' });
      });
    });
  });

  // ── Slider ──
  sliderEl.addEventListener('input', () => {
    levelValEl.textContent = parseFloat(sliderEl.value).toFixed(1);
  });
  sliderEl.addEventListener('change', () => {
    const level = parseFloat(sliderEl.value);
    chrome.runtime.sendMessage({ type: 'SET_LEVEL', value: level }, () => {
      refresh();
      chrome.tabs.query({ active: true, currentWindow: true }, (tabs) => {
        if (tabs[0]) chrome.tabs.sendMessage(tabs[0].id, { type: 'CONFIG_UPDATED' });
      });
    });
  });

  // ── Test Compression ──
  testBtn.addEventListener('click', () => {
    const text = testInput.value.trim();
    if (!text) {
      testResult.innerHTML = '<span style="color:var(--text-dim)">Enter some text first</span>';
      return;
    }
    chrome.runtime.sendMessage({ type: 'COMPRESS', text, isTest: true }, (res) => {
      if (res && res.compressed) {
        testResult.innerHTML = `<span class="saved">−${res.savings}%</span> · ${res.origTokens} → ${res.compTokens} tok · ${res.ruleHits} rules hit`;
        // Show compressed output in the textarea briefly
        testInput.value = res.text;
        testInput.style.borderColor = 'var(--green)';
        setTimeout(() => {
          testInput.style.borderColor = '';
        }, 1500);
      } else {
        testResult.innerHTML = '<span style="color:var(--text-dim)">No compression (too short or error)</span>';
      }
    });
  });

  // ── Reset ──
  resetBtn.addEventListener('click', () => {
    chrome.runtime.sendMessage({ type: 'RESET_STATS' }, () => {
      feedEl.innerHTML = '<div class="feed-empty">No requests yet<div class="hint">Send a message on Claude, ChatGPT, or Gemini</div></div>';
      refresh();
    });
  });

  // Cleanup on close
  window.addEventListener('unload', () => {
    if (pollInterval) clearInterval(pollInterval);
  });
})();
