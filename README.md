# Tobira

Tobira is a from-scratch browser experiment built without Chromium, WebView, or a browser SDK.

For the most current implementation snapshot and handoff notes, see [HANDOFF.md](HANDOFF.md).
When work switches between Codex, Claude, Gemini, Copilot, or a fresh session, update `HANDOFF.md` so the next person can resume quickly.

Current capabilities:

- Hand-rolled `http://` and `https://` client
- TLS with platform certificate verification
- Response decoding for `gzip`, `deflate`, and `br`
- Hand-rolled HTML tokenizer and DOM-like tree
- CSS parsing for:
  - embedded `<style>` blocks
  - `style=""` inline declarations
  - `link rel="stylesheet"` over `http://` and `https://`
- Selector support for:
  - tag selectors
  - `.class`
  - `#id`
  - descendant selectors
  - child selectors with `>`
- Style support for:
  - `display`
  - `color`
  - `background-color`
  - `margin`
  - `padding`
  - `font-size`
  - `font-weight`
  - `text-align`
  - `text-decoration`
  - `white-space`
- Lightweight GUI window with `winit`
- Software rendering with `softbuffer`
- System font rendering with TrueType / OpenType fonts via `fontdue`
- Plain text CLI renderer with `--cli`
- Basic JavaScript execution with `boa_engine`
- Lightweight mutable DOM support for:
  - `document.querySelector(...)`
  - `document.querySelectorAll(...)`
  - `document.getElementById(...)`
  - `document.createElement(...)`
  - `appendChild(...)`, `insertBefore(...)`, `remove()`
  - `innerHTML`, `textContent`, `classList`, `id`, `className`
  - recursive `document.write(...)`

Still missing:

- full CSS layout coverage
- DOM APIs beyond the basics
- address bar, tabs, history, navigation UI
- images, forms, and modern page features

## Run

GUI mode:

```bash
cargo run -- http://example.com
cargo run -- https://www.google.com
```

CLI mode:

```bash
cargo run -- --cli http://example.com
```

Local CSS demo:

```bash
python -m http.server 8765
cargo run -- http://127.0.0.1:8765/demo/index.html
```

Local JavaScript demo:

```bash
python -m http.server 8765
cargo run -- http://127.0.0.1:8765/demo/js-demo.html
```

Local DOM demo:

```bash
python -m http.server 8765
cargo run -- http://127.0.0.1:8765/demo/dom-demo.html
```

## GUI Controls

- `Up` / `Down`: scroll
- `PageUp` / `PageDown`: page scroll
- `Home` / `End`: jump to top or bottom
- `R`: reload
- `Ctrl+L`: focus the address bar
- `Ctrl+A`: select all text in the address bar
- `Ctrl+C` / `Ctrl+X` / `Ctrl+V`: copy, cut, and paste inside the address bar
- `Esc`: quit

## Project Structure

- `src/url.rs`
  URL parsing and relative URL resolution
- `src/http.rs`
  HTTP fetch, response parsing, chunked decoding, redirect handling
- `src/html.rs`
  HTML tokenization and DOM-like tree building
- `src/css.rs`
  CSS parsing, selector matching, cascade, and computed styles
- `src/font.rs`
  System font loading, glyph rasterization, and text measurement helpers
- `src/layout.rs`
  Styled text layout and block rendering model
- `src/browser.rs`
  Page loading, stylesheet collection, and browser page model
- `src/gui.rs`
  `winit` event loop and software drawing
- `src/render.rs`
  Plain text fallback renderer for CLI mode
- `src/main.rs`
  Application entry point

## Next Steps

1. Expand CSS coverage for more layout properties
2. Add better block layout and inline formatting behavior
3. Add address bar and navigation controls
4. Add image loading and richer page rendering
5. Add JavaScript execution for highly dynamic pages

## JavaScript Scope

Current JS support is intentionally small:

- inline `<script>`
- external `<script src>`
- `document.write()` / `document.writeln()`
- `document.title`
- `document.querySelector(...)` / `querySelectorAll(...)`
- `document.getElementById(...)`
- `document.createElement(...)`
- `appendChild(...)` / `insertBefore(...)` / `remove()`
- `innerHTML`, `textContent`, `classList`, `id`, `className`
- `location.href`
- `console.log()` / `warn()` / `error()`
- immediate `setTimeout(...)` fallback

It still does not implement a full browser DOM, robust events, async networking, or framework-level browser APIs.
