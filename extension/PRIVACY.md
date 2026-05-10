# Nyquest Chrome Extension — Privacy Policy

**Last updated:** March 3, 2026

## Summary

Nyquest compresses LLM prompts entirely within your browser. We collect no data.

## Data Collection

Nyquest does **not** collect, transmit, store, or share any user data. Specifically:

- **No prompt data** is sent to any external server
- **No browsing history** is tracked or stored
- **No personal information** is collected
- **No analytics or telemetry** of any kind
- **No cookies** are set by the extension

## How It Works

The extension intercepts outbound API requests to supported LLM services (Claude, ChatGPT, Gemini) and compresses the text content using pattern-matching rules that run entirely in your browser's JavaScript engine.

In "Rules Only" mode (the default), all processing happens locally. Your prompts never leave your browser except to the LLM provider you are already using.

In "Proxy" mode, the extension detects a Nyquest engine running on your own machine (localhost:5400) and routes requests through it for deeper compression. This traffic stays on your local machine — it is never sent to any external server operated by Nyquest.

## Permissions Justification

| Permission | Why |
|---|---|
| `activeTab` | To inject the compression script into supported LLM chat pages |
| `storage` | To save your compression level preference locally |
| Host access to claude.ai, chatgpt.com, gemini.google.com | To intercept and compress outbound API requests on these specific sites |
| Host access to localhost:5400 | To check if the local Nyquest proxy is running (optional auto-upgrade) |

## Third-Party Services

Nyquest does not integrate with any third-party analytics, advertising, or data collection services.

## Changes

If this policy changes, the extension version will be updated and the new policy will be published here.

## Contact

For privacy questions: [https://nyquest.ai](https://nyquest.ai)
