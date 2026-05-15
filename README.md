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
  - tag, `.class`, `#id`, `*` universal
  - descendant (` `), child (`>`), adjacent sibling (`+`), general sibling (`~`)
  - attribute selectors: `[attr]`, `[attr=val]`, `[attr*=val]`, `[attr^=val]`, `[attr$=val]`
  - pseudo-classes: `:first-child`, `:last-child`, `:nth-child(an+b)`, `:not(...)`
  - chained and mixed combinator chains: `A + B + C`, `A + B > C`, `A ~ B > C`
- Style support for:
  - box model: `display`, `margin`, `padding`, `width`, `height`, `max-width`, `min-width`
  - color: `color`, `background-color`, `opacity`, `border-color`
  - border: `border`, `border-width`, `border-style`
  - typography: `font-size`, `font-weight`, `font-style`, `font-family`, `text-align`, `text-decoration`, `text-transform`, `text-indent`, `letter-spacing`, `line-height`, `white-space`
  - layout: `list-style-type`, `vertical-align`
  - color values: hex (`#rgb`, `#rrggbb`, `#rgba`, `#rrggbbaa`), `rgb()`, `rgba()` (alpha blended), `hsl()`, `hsla()`, 140+ named colors
  - CSS custom properties (`--var`) and `var(--name, fallback)` with `:root` variable inheritance
  - `calc()` with correct operator precedence (`*`/`/` before `+`/`-`)
  - viewport units: `vw`, `vh` (consistent 1280×800 base)
  - `@media` with `max-width`, `min-width`, `screen`, `print`
- Lightweight GUI window with `winit`
- Software rendering with `softbuffer`
- System font rendering with TrueType / OpenType fonts via `fontdue`
- Plain text CLI renderer with `--cli`
- JavaScript execution via `boa_engine`:
  - inline `<script>` and external `<script src>`
  - block-list filter (allows most utility scripts; blocks known-dangerous APIs)
  - `document.write()` / `document.writeln()` with recursive expansion
  - `document.title`, `location.href`
  - `window.crypto` stubs (`getRandomValues`, `randomUUID`)
  - `URLSearchParams` stub
  - `document.cookie` read stub
  - `console.log()` / `warn()` / `error()`
  - immediate `setTimeout(...)` fallback
- Lightweight mutable DOM support for:
  - `document.querySelector(...)` / `querySelectorAll(...)`
  - `document.getElementById(...)`
  - `document.createElement(...)`
  - `document.createTextNode(...)`
  - `appendChild(...)`, `insertBefore(...)`, `remove()`
  - `innerHTML`, `textContent`, `classList`, `id`, `className`
  - reflected DOM properties such as `src`, `href`, `type`, and `value`
  - recursive `document.write(...)`
  - Promise-backed `fetch(...)`
  - minimal `XMLHttpRequest`
- Site-specific rendering paths for YouTube and Google

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

- `src/url.rs` — URL parsing and relative URL resolution
- `src/http.rs` — HTTP fetch, response parsing, chunked decoding, redirect handling
- `src/html.rs` — HTML tokenization and DOM-like tree building
- `src/css.rs` — CSS parsing, selector matching, cascade, computed styles, `@media`, `calc()`, color parsing
- `src/layout.rs` — Layout pipeline, text flow, tables, image placement, background/border drawing, link hitboxes
- `src/font.rs` — System font loading, glyph rasterization, text measurement
- `src/browser.rs` — Page loading pipeline, site-specific rewrites, YouTube/Google synthetic documents
- `src/gui.rs` — `winit` event loop, address bar, input handling, software rendering
- `src/js.rs` — Sandboxed JS execution, block-list filter, mutable DOM bridge, browser-ish stubs
- `src/render.rs` — Plain text fallback renderer for CLI mode
- `src/main.rs` — Application entry point
