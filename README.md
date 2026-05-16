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
- Custom title bar and address bar
- Clickable page links plus basic GUI form controls for `GET` submissions
- Basic DOM event plumbing for page controls:
  - bubbling `click`, `input`, `change`, and `submit`
  - target-only `focus` and `blur`
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
- deeper DOM APIs and event coverage
- tabs, history, and richer navigation UI
- POST forms, complex widgets, and modern app-shell browser APIs

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

Local forms demo:

```bash
python -m http.server 8765
cargo run -- http://127.0.0.1:8765/demo/forms-demo.html
```

Local event demo:

```bash
python -m http.server 8765
cargo run -- http://127.0.0.1:8765/demo/events-demo.html
```

## GUI Controls

- `Up` / `Down`: scroll
- `PageUp` / `PageDown`: page scroll
- `Home` / `End`: jump to top or bottom
- `R`: reload
- `Ctrl+L`: focus the address bar
- `Ctrl+A`: select all text in the address bar or a focused page input
- `Ctrl+C` / `Ctrl+X` / `Ctrl+V`: copy, cut, and paste inside the address bar or a focused page input
- `Esc`: blur a focused page input, otherwise quit

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

The living JavaScript roadmap is in [JS_ROADMAP.md](JS_ROADMAP.md).

Short version:

1. Finish richer listener options and capture phase handling
2. Tighten live `input.value` sync and other DOM fidelity gaps
3. Add storage, cookies, and history/navigation behavior
4. Improve networking semantics and reflow after DOM mutation
5. Validate against Google, YouTube, and other app-shell sites

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
- `document.addEventListener(...)` / bubbling for `click`, `input`, `change`, `submit`, `keydown`, and `keyup`
- `focus` / `blur` are currently target-only
- `addEventListener(...)` on page inputs, buttons, links, and forms
- `click`, `focus`, `blur`, `input`, `change`, `submit`, `keydown`, and `keyup` event dispatch
- `location.href`
- `console.log()` / `warn()` / `error()`
- immediate `setTimeout(...)` fallback

It still does not implement a full browser DOM, robust keyboard events, async networking, live `input.value` reflection for every GUI edit path, or framework-level browser APIs.
