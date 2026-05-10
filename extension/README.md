# Nyquest Chrome Extension

**Compress LLM prompts by 15–37% before they leave your browser.**

350+ regex rules across 18 categories. Zero server required. Zero data leaves your machine.

## How It Works

Nyquest intercepts outbound API requests to Claude, ChatGPT, and Gemini. Before the request leaves your browser, it compresses the prompt text using 350+ pattern-matching rules that remove filler, simplify verbose phrasing, collapse redundant clauses, and strip boilerplate — without changing the semantic meaning.

### Two Modes

| Mode | How | Savings | Latency |
|---|---|---|---|
| **Rules Only** | All compression runs in the browser | 15–37% | <1ms |
| **Full Pipeline** | Auto-detects Nyquest proxy on localhost:5400 | 15–75% | 200–350ms |

If you have the Nyquest engine running locally, the extension automatically upgrades to proxy mode — routing requests through the full 6-stage pipeline including the semantic LLM stage. No configuration needed.

## Supported Sites

- **Claude.ai** (claude.ai)
- **ChatGPT** (chatgpt.com)
- **Gemini** (gemini.google.com)

## Privacy

- All compression runs locally in your browser (Rules Only mode)
- Prompt content is never sent to any third-party server
- No telemetry, no analytics, no tracking
- In proxy mode, requests route to your own localhost — never to external servers

## Installation

### Chrome Web Store
*(Coming soon)*

### Developer Mode
1. Clone or download this `extension/` directory
2. Open `chrome://extensions/`
3. Enable "Developer mode" (top right)
4. Click "Load unpacked" → select the `extension/` folder
5. Pin the Nyquest icon in your toolbar

## Compression Levels

| Level | Strategy | Typical Savings |
|---|---|---|
| 0.2 | Filler removal (~50 rules) | 5–10% |
| 0.5 | + Structural compression (~120 rules) | 15–25% |
| 0.7 | Default — balanced | 18–30% |
| 1.0 | + Aggressive + format + minify (350+ rules) | 25–37% |

## Building Icons

```bash
cd extension/icons
python3 gen_icons.py
```

Requires one of: `rsvg-convert`, ImageMagick `convert`, or Python PIL/Pillow.

## License

AGPL-3.0 — same as the Nyquest engine.
