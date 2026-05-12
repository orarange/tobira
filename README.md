# Scratch Browser

Scratch Browser is a from-scratch browser experiment built without Chromium, WebView, or a browser SDK.

Current capabilities:

- Hand-rolled `http://` client
- Hand-rolled HTML tokenizer and DOM-like tree
- CSS parsing for:
  - embedded `<style>` blocks
  - `style=""` inline declarations
  - `link rel="stylesheet"` over `http://`
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
- Plain text CLI renderer with `--cli`

Still missing:

- `https://`
- full CSS layout coverage
- JavaScript execution
- address bar, tabs, history, navigation UI
- images, forms, and modern page features

## Run

GUI mode:

```bash
cargo run -- http://example.com
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

## GUI Controls

- `Up` / `Down`: scroll
- `PageUp` / `PageDown`: page scroll
- `Home` / `End`: jump to top or bottom
- `R`: reload
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

1. Add `https://` support with `rustls`
2. Expand CSS coverage for more layout properties
3. Add better block layout and inline formatting behavior
4. Add address bar and navigation controls
5. Add image loading and richer page rendering
