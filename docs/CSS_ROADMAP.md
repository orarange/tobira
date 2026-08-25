# CSS Implementation Roadmap

This file tracks CSS feature implementation across all phases for the Tobira browser engine.

## Status Legend

- ✅ Implemented
- 🔧 Partially implemented (parsed but not fully applied in layout)
- ❌ Not yet implemented

---

## Phase 1 — Core Visual Primitives ✅

Merged into master via PR.

| Feature | Status | Notes |
|---------|--------|-------|
| `border-radius` | ✅ | Rounded rect drawing in GUI with 3-strip optimization |
| `overflow: hidden` | ✅ | Clips child content to element bounds |
| `box-shadow` | ✅ | offset-x, offset-y, blur, color |
| `::before` / `::after` | ✅ | `content`, `display`, `color`, `background-color` |
| `:root` CSS custom properties | ✅ | `--var` declaration and `var(--name, fallback)` |
| `@media` scoped root vars | ✅ | `media_root_vars` applied at compute time |

---

## Phase 2 — Positioning & Flexbox ✅

Branch: `claude/phase2-css` (commit `fe18b04`)

| Feature | Status | Notes |
|---------|--------|-------|
| `position: relative` | ✅ | Offset applied after normal flow |
| `position: absolute` | ✅ | Taken out of flow, placed relative to containing block |
| `position: fixed` | ✅ | Placed relative to viewport, ignores scroll |
| `z-index` | ✅ | Sorted and composited via `positioned_commands` |
| `top` / `right` / `bottom` / `left` | ✅ | Applied to positioned elements |
| `display: flex` | ✅ | Row and column directions |
| `flex-direction` | ✅ | `row`, `row-reverse`, `column`, `column-reverse` |
| `flex-wrap` | ✅ | `nowrap`, `wrap`, `wrap-reverse` |
| `justify-content` | ✅ | `flex-start`, `flex-end`, `center`, `space-between`, `space-around`, `space-evenly` |
| `align-items` | ✅ | `flex-start`, `flex-end`, `center`, `stretch`, `baseline` |
| `align-self` | ✅ | Per-item override of `align-items` |
| `flex-grow` / `flex-shrink` | ✅ | Space distribution. **Correction (2026-08-25)**: `flex-shrink` was marked done here but only genuinely landed on 2026-08-25 (`620811c`). Treat older ✅ marks in this file as claims to spot-check, not facts. |
| `flex-basis` | ✅ | Initial size before grow/shrink |
| `flex` shorthand | ✅ | Expands to grow/shrink/basis |
| `gap` / `row-gap` / `column-gap` | ✅ | Flex and grid gap |
| `order` | ✅ | Reorders flex items |

---

## Phase 3 — Grid & Sticky ✅

Branch: `claude/phase2-css` (commit `28597a7`)

| Feature | Status | Notes |
|---------|--------|-------|
| `display: grid` | ✅ | Full grid container layout |
| `grid-template-columns` | ✅ | px, %, fr units, `repeat()` |
| `grid-template-rows` | ✅ | px, %, fr units, `repeat()` |
| `fr` units | ✅ | Two-pass resolution (fixed first, then fr proportional) |
| `repeat()` | ✅ | Expands repeated track definitions |
| `grid-column` / `grid-row` | ✅ | `start / end` shorthand |
| `grid-column-start/end` | ✅ | Integer line numbers |
| `grid-auto-rows` | ✅ | Default row height for implicit rows |
| Column spanning | ✅ | `span N` syntax |
| `position: sticky` | 🔧 | Parsed; lays out as static (no scroll-based stickiness) |

---

## Phase 4 — Transform & Transition ✅

Branch: `claude/phase2-css` (commit `28597a7`)

| Feature | Status | Notes |
|---------|--------|-------|
| `transform: translate()` | ✅ | Applied in layout (shifts element position) |
| `transform: translateX/Y()` | ✅ | Applied in layout |
| `transform: scale()` | 🔧 | Parsed; not applied in software render |
| `transform: rotate()` | 🔧 | Parsed; not applied in software render |
| `transform: skew()` | 🔧 | Parsed; not applied in software render |
| `transform-origin` | 🔧 | Parsed as f32 %; not applied (scale/rotate not rendered) |
| `transition` | 🔧 | Raw value stored; no animation runtime |
| `animation` | 🔧 | No-op; value stored for compatibility |
| `will-change` | 🔧 | No-op; value stored for compatibility |

---

## Phase 5 — Implemented ✅

Branch: `claude/phase5-css` (PR #49)

| Feature | Status | Notes |
|---------|--------|-------|
| `clamp(a, b, c)` / `min()` / `max()` | ✅ | Works nested inside `calc()` |
| `aspect-ratio` | ✅ | Stored as milliratio u32 to keep `Eq`; applied in image layout |
| `object-fit` / `object-position` | ✅ | 5 modes: Fill/Contain/Cover/ScaleDown/None |
| `content: attr(name)` | ✅ | Resolved from element attributes in `::before`/`::after` |
| `:hover` / `:focus` / `:active` | ✅ | Real pseudo-classes; `InteractiveState` threaded through cascade; GUI re-layouts on hover change |
| `:checked` / `:disabled` / `:enabled` | ✅ | Matched via element attributes |
| `::placeholder` / `::selection` | ✅ | Parsed; `compute_placeholder_style()` API for GUI integration |
| `display: inline-flex` | ✅ | Inline-level flex container |
| `display: inline-grid` | ✅ | Inline-level grid container |
| `display: grid` | ✅ | Full grid layout with auto-placement engine |
| `grid-template-columns` / `-rows` | ✅ | px, %, fr, auto, min/max-content, `repeat(N, ...)` |
| `fr` units | ✅ | Two-pass distribution (fixed first, then proportional) |
| `grid-column` / `grid-row` | ✅ | Explicit placement + `span N` syntax |
| `grid-auto-rows` / `-columns` | ✅ | Implicit track sizing |
| `flex-flow` shorthand | ✅ | Sets `flex-direction` + `flex-wrap` |
| `align-content` | ✅ | Parsed; applied in multi-line flex cross-axis |
| `min-content` / `max-content` / `fit-content()` | ✅ | `LengthValue` variants; used in width, flex-basis, grid |
| `position: sticky` | 🔧 | Lays out as relative; scroll-offset stickiness deferred |
| `cursor` extended | ✅ | `CursorKind` enum with 14 variants; `cursor_kind` on `ComputedStyle` |
| `pointer-events: none` | ✅ | Gates link + element hitbox emission |
| `filter: blur() / brightness() / opacity()` | ✅ | Parsed into dedicated fields; rendering deferred |
| `@supports` | 🔧 | Treated as always-true (optimistic) |
| `@layer` | 🔧 | Layer name ignored; rules applied as regular rules |
| `backdrop-filter` / `clip-path` | 🔧 | Parsed as no-op |
| `scroll-behavior` / `resize` / `writing-mode` / `user-select` / `appearance` / `contain` | 🔧 | Parsed as no-op (no crash on real-world CSS) |

## Phase 6 — Partially Implemented 🔧

### Phase 6 Batch 1 ✅ (Branch: `claude/phase5-css`)

| Feature | Status | Notes |
|---------|--------|-------|
| `filter: blur()` rendering | ✅ | Separable box blur in `gui.rs`; `LayerCommand.blur_px` field |
| `filter: brightness()` rendering | ✅ | Per-channel scale in `gui.rs`; `LayerCommand.brightness` field |

### Phase 6 Batch 2 ✅ (Branch: `claude/phase5-css`)

| Feature | Status | Notes |
|---------|--------|-------|
| `white-space: nowrap` | ✅ | `WhiteSpaceMode::NoWrap` variant; `layout_nowrap_fragments()` skips line-breaking |
| `text-decoration: line-through` | ✅ | `line_through: bool` on `ComputedStyle` + `TextCommand`; strikethrough rendered in `gui.rs` |
| `font-weight` numeric (100–900) | ✅ | 600–900 → bold, 100–500 → normal |
| `font-family: serif` | ✅ | `FontFamilyKind::Serif`; maps Georgia/Times to serif system font |
| `text-overflow: ellipsis` | ✅ | `text_overflow_ellipsis: bool`; clips inline content with "…" when `overflow: hidden` |
| `text-shadow` | ✅ | `TextShadow` struct (offset-x/y, blur, color); shadow rendered before main text in `gui.rs` |
| `background-image: linear-gradient()` | ✅ | `GradientCommand` draw command; pixel-level angle+stop interpolation in `gui.rs` |
| `background-image: url()` | ✅ | `background_image_url` field; emits `DrawCommand::Image` at element background position |
| `background-size` | ✅ | `Cover`, `Contain`, `Auto` variants |
| `background-repeat` | ✅ | `Repeat`, `NoRepeat`, `RepeatX`, `RepeatY` (single-tile for now) |
| `background-position` | ✅ | x/y as 0–100 percent |

### Phase 6 Batch 3 ✅ (2026-08-26) — grid

| Feature | Status | Notes |
|---------|--------|-------|
| `grid-template-areas` / `grid-area` | ✅ | Named areas parsed into rectangles at parse time, with the spec's validity rules (equal token counts, each area a filled rectangle); an invalid template is dropped whole. A period run is one null cell. |
| Named grid lines | ✅ | `[name]` groups in a track list, and `<custom-ident>` placement (`grid-column: content`). A bare name fills both edges and falls back to `name-start` / `name-end`. |
| `minmax()` | ✅ | Resolves to its maximum. Previously unparseable, which dropped the track and shifted later items a column left. |
| `grid-template` shorthand | ✅ | `<rows> / <columns>`, including the form where the rows are area strings with sizes between them. |
| Content-sized track measurement | ✅ | `min-content` / `max-content` sized from their contents and left there; `auto` measured as a floor then stretched. Measurement is clamped to the container. |

Verified against real pages at viewport 1280: MDN's hero heading went from one CJK character per line
(40px × 7 runs) to a single 280px run, `content_height` 25695 → 4915; Wikipedia's article column moved
from a full-width x=44 block to x=240 / 996px wide, whose inner 948px matches its `minmax(0,59.25rem)`.

Still open: `grid-auto-flow: dense`, `fit-content()`, `repeat()` with line names inside, subgrid, and the
full `grid` shorthand (implicit tracks + auto-flow).

### Phase 6 Remaining ❌

| Feature | Priority | Notes |
|---------|----------|-------|
| `transform: scale/rotate` rendering | Medium | Needs affine transform in software renderer |
| CSS `animation` / `@keyframes` | Low | Requires animation runtime and repaint loop |
| `transition` interpolation | Low | Requires repaint loop and state diffing |
| `position: sticky` scroll tracking | Medium | Requires scroll-offset propagation into layout |
| `grid-auto-flow` | Low | Dense packing auto-placement |
| `counter()` / `counters()` | Low | CSS counters for lists |
| `clip-path` | Low | Shape clipping |
| `writing-mode` | Low | Vertical text layout |
| `direction` / `unicode-bidi` | Low | RTL text support |
| `scroll-behavior: smooth` | Low | Smooth scrolling |
| `::selection` styling | Low | Highlight selected text with custom color |
| `::placeholder` GUI wiring | Low | Apply `::placeholder` style to input placeholder text |
| `background-repeat` tiling | Low | Full tiling (repeat-x/y across element) |
| `text-shadow` with blur | Low | Blur pass for text shadow (offset-only works now) |

### Measured gaps (added 2026-08-25)

`TOBIRA_DEBUG_CSS=1 tobira --dump-styled <url>` ranks the declarations the engine parsed and then discarded (`css::unsupported_property_report()`). This is the empirical worklist - prefer it over guessing. Recurring across Wikipedia / MDN / Google:

| Gap | Why it is cheap | Seen |
|---|---|---|
| Logical properties (`margin-inline*`, `padding-inline`/`block`, `inset`, `inset-block-start`) | For LTR this is a direct map onto the physical properties we already have | All three sites |
| Individual corner radii (`border-top-left-radius` etc.) | Only the `border-radius` shorthand is wired | 388 each on MDN |
| `-webkit-text-decoration` | Pure alias for `text-decoration` | 3431 on MDN |
| `word-wrap` / `overflow-wrap` / `word-break` | Line-breaking already exists; these select the policy | 966 on Wikipedia |
| `counter-increment` / `counter-reset` | Pairs with the existing `list-style-type` work | 167 on Wikipedia |
| `border-collapse` / `border-spacing` / `caption-side` | Table rendering already exists | Wikipedia |
| `mask-image` family, `fill` (painting SVG from CSS), `clip-path` | Larger, mostly cosmetic | MDN / Google |

**Do not rank by raw count.** `transition-*` and `animation-*` alone account for ~44,000 dropped declarations on a single Wikipedia article, but they are the known "no animation runtime" gap, not cheap wins.


---

## Architecture Notes

- CSS parsing lives in `src/css.rs`
- Layout application lives in `src/layout.rs`
- GUI rendering lives in `src/gui.rs`
- Transform-origin f32 fields required removing `Eq` from `ComputedStyle` and related types; `PartialEq` uses `f32::to_bits()`
- Positioned elements (`absolute`/`fixed`) are collected into `positioned_commands: Vec<(i32, Vec<DrawCommand>)>` and composited sorted by z-index after normal flow
- Grid fr units use fixed-point *100 integer arithmetic to avoid f32 in `GridTrackSize`
