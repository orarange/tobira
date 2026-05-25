# Tobira

Tobira is a from-scratch browser experiment built without Chromium, WebView, or a browser SDK.

For the most current implementation snapshot and handoff notes, see [HANDOFF.md](HANDOFF.md).
When work switches between Codex, Claude, Gemini, Copilot, or a fresh session, update `HANDOFF.md` so the next person can resume quickly.

Project north star:

- Chromeと同程度の実用感を目指し、Google/YouTubeなどの複雑なサイトをsynthetic fallbackに頼らず閲覧・操作できるようにする
- priority order: WebComponents / shadow DOM details, DOM mutation to reflow / hit-test synchronization, fetch / XHR / history / storage browser-grade behavior, then real-site stability checks

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
  - pseudo-elements: `::before`, `::after` (with `content`, `display`, `color`, `background-color`)
  - chained and mixed combinator chains: `A + B + C`, `A + B > C`, `A ~ B > C`
- Style support for:
  - box model: `display`, `margin`, `padding`, `width`, `max-width`, `min-width` (block height is derived from content flow; `height` applies to images and `overflow:hidden` clip boxes)
  - color: `color`, `background-color`, `opacity`, `border-color`
  - border: `border`, `border-width`, `border-style`, `border-radius`
  - shadows: `box-shadow` (offset, blur, color)
  - overflow: `overflow: hidden` (clips child content to element bounds)
  - typography: `font-size`, `font-weight`, `font-style`, `font-family`, `text-align`, `text-decoration`, `text-transform`, `text-indent`, `letter-spacing`, `line-height`, `white-space`
  - layout: `list-style-type`, `vertical-align`
  - color values: hex (`#rgb`, `#rrggbb`, `#rgba`, `#rrggbbaa`), `rgb()`, `rgba()` (alpha blended), `hsl()`, `hsla()`, 140+ named colors
  - CSS custom properties (`--var`) and `var(--name, fallback)` with `:root` variable inheritance
  - `calc()` with correct operator precedence (`*`/`/` before `+`/`-`)
  - viewport units: `vw`, `vh` (consistent 1280×800 base)
  - `@media` with `max-width`, `min-width`, `screen`, `print`
  - `getComputedStyle(...)` snapshots for common layout-sensitive values
- Lightweight GUI window with `winit`
- Software rendering with `softbuffer`
- System font rendering with TrueType / OpenType fonts via `fontdue`
- Background page loading and content rendering workers keep the title bar and address bar responsive while navigation is in flight
- Plain text CLI renderer with `--cli`
- JavaScript execution via `boa_engine` & sandboxed DOM/API support:
  - inline `<script>` and external `<script src>`
  - block-list filter (allows most utility scripts; blocks known-dangerous APIs)
  - `document.write()` / `document.writeln()` with recursive expansion
  - `document.title`, `location.href`
  - `window.crypto` stubs (`getRandomValues`, `randomUUID`)
  - `URLSearchParams` stub
  - `console.log()` / `warn()` / `error()`
  - queued `queueMicrotask(...)`, `setTimeout(...)`, `setInterval(...)`, and `requestAnimationFrame(...)` callbacks
  - JS-visible viewport / focus / scroll state:
    - `window.innerWidth` / `window.innerHeight`
    - `window.scrollY` / `window.pageYOffset`
    - `document.activeElement`
    - `window.scrollTo(...)`, `window.scrollBy(...)`, and `scrollTop` setters on DOM nodes
  - Lightweight storage and cookie support:
    - origin-scoped `localStorage` and `sessionStorage`
    - `document.cookie` read/write and HTTP cookie propagation
- Network APIs:
    - Promise-backed `fetch(...)` with response headers iteration
    - minimal `XMLHttpRequest` supporting `getResponseHeader(...)` / `getAllResponseHeaders()`
- WebComponents support:
  - `customElements.define(...)` / `get(...)` / `whenDefined(...)` / `upgrade(...)`
  - `attachShadow(...)` with open / closed roots, `slot.assignedNodes()` / `slot.assignedElements()` with `flatten`
  - `assignedSlot`, `slotchange`, and `Event.composedPath()` plus shadow-boundary retargeting for capture / bubble dispatch
- Advanced Chrome & Browser GUI features:
  - Custom title bar and address bar
  - Browser back/forward buttons and `Alt+Left` / `Alt+Right` history navigation
  - `location.reload()` plus `history.go(0)` reload requests and `history.scrollRestoration` auto/manual switching
  - Scroll restoration for both same-document and full-document history entries
  - Page navigation and content rendering complete asynchronously without showing a loading-screen UI
  - Clickable page links plus basic GUI form controls for `GET` submissions, including text inputs, buttons, and checkbox/radio toggles
  - Basic DOM event plumbing (bubbles `click`, `input`, `change`, `submit`; target-only `focus`, `blur`)
  - `MutationObserver` callbacks for `attributes`, `childList`, and `characterData`, plus browser-style event constructors (`Event`, `CustomEvent`, `KeyboardEvent`, `InputEvent`, `MouseEvent`, `FocusEvent`, `SubmitEvent`) and `AbortController` / `AbortSignal`
  - `addEventListener(..., { signal })` listener cancellation support alongside capture / once / passive handling
  - Layout reflow cache keyed by viewport width and page revision
- Lightweight mutable DOM support for:
  - `document.querySelector(...)` / `querySelectorAll(...)`
  - `document.getElementById(...)`
  - `document.createElement(...)`
  - `document.createTextNode(...)`
  - `document.createDocumentFragment(...)`
  - `appendChild(...)`, `insertBefore(...)`, `remove()`
  - `replaceChild(...)`, `removeChild(...)`, `append(...)`, `prepend(...)`, `before(...)`, `after(...)`, `replaceWith(...)`, `replaceChildren(...)`
  - dynamic `document.body`, `document.head`, and `document.documentElement`
  - Node introspection helpers: `nodeType`, `nodeName`, `nodeValue`, `firstChild`, `lastChild`, `previousSibling`, `nextSibling`, `isConnected`
  - text node `CharacterData` support: `data`, `length`, `nodeValue`, `textContent`, and `splitText(...)`
  - `hasAttribute(...)`, `getAttributeNames(...)`
  - `toggleAttribute(...)`
  - `element.attributes` as a live NamedNodeMap-style collection with `length`, `item(...)`, `getNamedItem(...)`, and array-like iteration
  - `matches(...)`, `closest(...)`, `contains(...)`
  - `firstElementChild`, `lastElementChild`, `previousElementSibling`, `nextElementSibling`
  - `innerHTML`, `textContent`, `classList`, `id`, `className`
  - reflected DOM properties such as `src`, `href`, `rel`, `type`, `name`, `value`, `content`
  - `classList.value`, `classList.length`, `classList.item(...)`, `classList.toString()`, `classList.replace(...)`
  - recursive `document.write(...)`
  - GUI-driven DOM attribute changes now refresh the live page snapshot, so reflow follows mutation notifications instead of waiting for a reload
  - inline `element.style` updates through `cssText`, `setProperty(...)`, and common style accessors for text, size, and border properties
- Site-specific rendering paths for YouTube and Google

Still missing:

- Phase 6 CSS visual effects and advanced rendering
- deeper DOM APIs and event coverage
- tabs and richer navigation UI
- session-history replay polish across full document loads (basic scroll restoration is now in place)
- deeper scroll restoration beyond the current full-document / same-document history support
- inline style mutations still need broader coverage across the full CSS property matrix and computed-style parity
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

Debug tracing:

```powershell
$env:TOBIRA_TRACE_JS=1
$env:TOBIRA_TRACE_LOAD=1
cargo run -- https://www.youtube.com/
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

Local scroll demo:

```bash
python -m http.server 8765
cargo run -- http://127.0.0.1:8765/demo/scroll-demo.html
```

Local storage / cookie demo:

```bash
python -m http.server 8765
cargo run -- http://127.0.0.1:8765/demo/storage-demo.html
```
## GUI Controls

- `Up` / `Down`: scroll
- `PageUp` / `PageDown`: page scroll
- `Home` / `End`: jump to top or bottom
- `R`: reload
- `Ctrl+L`: focus the address bar
- `Ctrl+A`: select all text in the address bar or a focused page input
- `Ctrl+C` / `Ctrl+X` / `Ctrl+V`: copy, cut, and paste inside the address bar or a focused page input
- `Alt+Left` / `Alt+Right`: browser back and forward
- `Esc`: blur a focused page input, otherwise quit

## Project Structure

- `src/url.rs` — URL parsing and relative URL resolution
- `src/site_state.rs` — Origin-scoped storage and cookie state shared across HTTP and JS
- `src/http.rs` — HTTP fetch, response parsing, chunked decoding, redirect handling
- `src/html.rs` — HTML tokenization and DOM-like tree building
- `src/css.rs` — CSS parsing, selector matching, cascade, computed styles, `@media`, `calc()`, color parsing
- `src/layout.rs` — Styled text layout, block rendering, tables, image placement, background/border drawing, link hitboxes
- `src/font.rs` — System font loading, glyph rasterization, and text measurement helpers
- `src/browser.rs` — Page loading pipeline, stylesheet collection, site-specific rewrites, YouTube/Google synthetic documents
- `src/gui.rs` — `winit` event loop, address bar, input handling, software rendering
- `src/js.rs` — Sandboxed JS execution, block-list filter, mutable DOM bridge, browser-ish stubs
- `src/render.rs` — Plain text fallback renderer for CLI mode
- `src/main.rs` — Application entry point

## Next Steps

The living JavaScript roadmap is in [JS_ROADMAP.md](JS_ROADMAP.md).

Short version:

1. Finish richer history/back-forward behavior and document-load replay
2. Improve networking semantics and incremental reflow after DOM mutation
3. Validate against Google, YouTube, and other app-shell sites

## JavaScript Scope

Current JS support is intentionally small:

- inline `<script>`
- external `<script src>`
- `document.write()` / `document.writeln()`
- `localStorage`, `sessionStorage`, and `document.cookie`
- `document.title`
- `document.querySelector(...)` / `querySelectorAll(...)`
- `document.getElementById(...)`
- `document.createElement(...)`
- `appendChild(...)` / `insertBefore(...)` / `remove()`
- `hasAttribute(...)` / `getAttributeNames(...)`
- `toggleAttribute(...)`
- `matches(...)`, `closest(...)`, `contains(...)`
- `firstElementChild`, `lastElementChild`, `previousElementSibling`, `nextElementSibling`
- `innerHTML`, `textContent`, `classList`, `id`, `className`
- `classList.value`, `classList.length`, `classList.item(...)`, `classList.toString()`, `classList.replace(...)`
- `element.attributes` as a live NamedNodeMap-style collection with `length`, `item(...)`, `getNamedItem(...)`, and array-like iteration
- `document.addEventListener(...)` / capture + bubbling for `click`, `input`, `change`, `submit`, `keydown`, and `keyup`
- `focus` / `blur` are currently target-only
- `addEventListener(...)` / `removeEventListener(...)` on page inputs, buttons, links, forms, and document nodes
- `click`, `focus`, `blur`, `input`, `change`, `submit`, `keydown`, and `keyup` event dispatch, including `once`, capture-phase, and passive listeners
- native GUI typing stays in sync with DOM `input.value`
- text inputs and textareas expose browser-like `selectionStart` / `selectionEnd` / `selectionDirection` plus `setSelectionRange(...)` / `select()`
- `location.hash` plus `history.pushState(...)` / `replaceState(...)` soft navigation
- `history.state`
- `popstate` / `hashchange`
- `location.href`
- lightweight response header iteration plus XHR `getResponseHeader(...)` / `getAllResponseHeaders()`
- `console.log()` / `warn()` / `error()`
- queued `queueMicrotask(...)`, `setTimeout(...)`, `setInterval(...)`, and `requestAnimationFrame(...)` callbacks
- YouTube generic home pages now take a synthetic fast path before the heavy JS session, which keeps the app responsive on startup

It still does not implement a full browser DOM, async networking, or framework-level browser APIs.
