/**
 * Nyquest Interceptor — MAIN world content script
 * Hooks window.fetch to compress outbound LLM API requests.
 *
 * Two modes:
 *   1. Local rules: compress message content with NyquestCompressor
 *   2. Proxy mode:  rewrite request URL to localhost:5400 (full pipeline)
 *
 * Injected by content.js into the page's MAIN world.
 */

(function () {
  'use strict';

  // Avoid double-injection
  if (window.__nyquest_interceptor__) return;
  window.__nyquest_interceptor__ = true;

  // API endpoint patterns we intercept
  const API_PATTERNS = [
    // Claude.ai — chat completion + append endpoints
    { pattern: /claude\.ai\/api\/.*chat_conversations\/[^/]+\/completion/, provider: 'claude' },
    { pattern: /claude\.ai\/api\/.*\/completion/, provider: 'claude' },
    { pattern: /claude\.ai\/api\/.*\/chat_conversations/, provider: 'claude', method: 'POST' },
    // ChatGPT
    { pattern: /chatgpt\.com\/backend-api\/conversation/, provider: 'chatgpt' },
    // Gemini
    { pattern: /gemini\.google\.com\/_\/BardChatUi/, provider: 'gemini' },
    { pattern: /generativelanguage\.googleapis\.com/, provider: 'gemini' },
    // Direct API calls
    { pattern: /api\.anthropic\.com\/v1\/messages/, provider: 'anthropic' },
    { pattern: /api\.openai\.com\/v1\/chat\/completions/, provider: 'openai' },
  ];

  let config = {
    enabled: true,
    level: 0.7,
    proxyMode: false,
    proxyUrl: 'http://localhost:5400',
  };

  window.addEventListener('message', (e) => {
    if (e.data && e.data.type === '__NYQUEST_CONFIG__') {
      config = { ...config, ...e.data.config };
    }
  });

  const originalFetch = window.fetch;

  window.fetch = async function (input, init) {
    if (!config.enabled) return originalFetch.call(this, input, init);

    const url = (typeof input === 'string') ? input : (input instanceof Request ? input.url : '');
    const method = (init && init.method) || (input instanceof Request ? input.method : 'GET');

    // Debug logging
    if (url && (url.includes('api') || url.includes('conversation') || url.includes('chat') || url.includes('message') || url.includes('completion'))) {
      console.log(`[Nyquest] Fetch: ${method} ${url.substring(0, 150)}`);
    }

    const match = API_PATTERNS.find(p => {
      if (!p.pattern.test(url)) return false;
      if (p.method && p.method !== method.toUpperCase()) return false;
      return true;
    });
    if (!match) return originalFetch.call(this, input, init);

    console.log(`[Nyquest] ✅ Matched ${match.provider}: ${method} ${url.substring(0, 120)}`);

    try {
      if (config.proxyMode) {
        return await handleProxyMode(input, init, url, match);
      }
      return await handleLocalMode(input, init, url, match);
    } catch (err) {
      console.warn('[Nyquest] Error, passing through:', err);
      return originalFetch.call(this, input, init);
    }
  };

  // ── Compress any string field recursively ──
  function compressStringFields(obj, compressor, tracker) {
    if (typeof obj === 'string') {
      if (obj.length < 20) return obj; // skip tiny strings
      const result = compressor.compress(obj);
      tracker.origChars += obj.length;
      tracker.compChars += result.text.length;
      return result.text;
    }
    if (Array.isArray(obj)) {
      return obj.map(item => compressStringFields(item, compressor, tracker));
    }
    if (obj && typeof obj === 'object') {
      const out = {};
      for (const [key, val] of Object.entries(obj)) {
        // Skip fields that shouldn't be compressed
        if (['id', 'uuid', 'conversation_id', 'parent_message_uuid',
             'model', 'timezone', 'attachments', 'files',
             'rendering_mode', 'organization_id', 'type',
             'role', 'name', 'tool_use_id', 'tool_call_id',
             'message_id', 'parent_id', 'action'].includes(key)) {
          out[key] = val;
          continue;
        }
        // Compress text-bearing fields
        if (['prompt', 'content', 'text', 'system', 'instructions',
             'parts', 'messages', 'message'].includes(key)) {
          out[key] = compressStringFields(val, compressor, tracker);
          continue;
        }
        // Recurse into nested objects but not too deep
        if (typeof val === 'object' && val !== null) {
          out[key] = compressStringFields(val, compressor, tracker);
        } else {
          out[key] = val;
        }
      }
      return out;
    }
    return obj;
  }

  async function handleLocalMode(input, init, url, match) {
    if (!init || !init.body) {
      console.log('[Nyquest] No body, passing through');
      return originalFetch.call(window, input, init);
    }

    let bodyStr;
    try {
      if (typeof init.body === 'string') {
        bodyStr = init.body;
      } else if (init.body instanceof ReadableStream) {
        // Clone the stream so we can read it and still send it
        const cloned = init.body;
        bodyStr = await new Response(cloned).text();
      } else {
        bodyStr = await new Response(init.body).text();
      }
    } catch (e) {
      console.log('[Nyquest] Cannot read body:', e.message);
      return originalFetch.call(window, input, init);
    }

    // Log body structure for debugging
    let body;
    try {
      body = JSON.parse(bodyStr);
    } catch (_) {
      console.log('[Nyquest] Body is not JSON, length:', bodyStr.length);
      return originalFetch.call(window, input, init);
    }

    console.log('[Nyquest] Body keys:', Object.keys(body).join(', '));

    const compressor = new NyquestCompressor(config.level);
    const tracker = { origChars: 0, compChars: 0 };

    // Compress the body using recursive field detection
    const compressed = compressStringFields(body, compressor, tracker);

    if (tracker.origChars === 0) {
      console.log('[Nyquest] No compressible text found in body');
      return originalFetch.call(window, input, init);
    }

    const savings = ((1 - tracker.compChars / tracker.origChars) * 100).toFixed(1);
    console.log(`[Nyquest] Compressed: ${tracker.origChars} → ${tracker.compChars} chars (${savings}% savings, ${match.provider})`);

    window.postMessage({
      type: '__NYQUEST_STATS__',
      savings,
      origChars: tracker.origChars,
      compChars: tracker.compChars,
      origTokens: Math.ceil(tracker.origChars / 4),
      compTokens: Math.ceil(tracker.compChars / 4),
      provider: match.provider,
    }, '*');

    const newInit = { ...init, body: JSON.stringify(compressed) };
    return originalFetch.call(window, input, newInit);
  }

  async function handleProxyMode(input, init, url, match) {
    let proxyUrl;
    if (match.provider === 'anthropic' || match.provider === 'claude') {
      proxyUrl = `${config.proxyUrl}/v1/messages`;
    } else {
      proxyUrl = `${config.proxyUrl}/v1/chat/completions`;
    }

    window.postMessage({
      type: '__NYQUEST_PROXY__',
      provider: match.provider,
    }, '*');

    return originalFetch.call(window, proxyUrl, { ...init });
  }
})();
