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
| `flex-grow` / `flex-shrink` | ✅ | Space distribution |
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
| `filter: blur()` rendering | ✅ | Separable box blur in `gui.rs`; `LayerCommand.blur_px` field; triggered when `filter_blur_px > 0` |
| `filter: brightness()` rendering | ✅ | Per-channel scale in `gui.rs`; `LayerCommand.brightness` field; triggered when `filter_brightness != 10000` |

### Phase 6 Remaining ❌

| Feature | Priority | Notes |
|---------|----------|-------|
| `transform: scale/rotate` rendering | Medium | Needs affine transform in software renderer |
| CSS `animation` / `@keyframes` | Low | Requires animation runtime and repaint loop |
| `transition` interpolation | Low | Requires repaint loop and state diffing |
| `position: sticky` scroll tracking | Medium | Requires scroll-offset propagation into layout |
| `grid-template-areas` | Low | Named area placement |
| `grid-auto-flow` | Low | Dense packing auto-placement |
| `counter()` / `counters()` | Low | CSS counters for lists |
| `clip-path` | Low | Shape clipping |
| `writing-mode` | Low | Vertical text layout |
| `direction` / `unicode-bidi` | Low | RTL text support |
| `scroll-behavior: smooth` | Low | Smooth scrolling |
| `::selection` styling | Low | Highlight selected text with custom color |
| `::placeholder` GUI wiring | Low | Apply `::placeholder` style to input placeholder text |

---

## Architecture Notes

- CSS parsing lives in `src/css.rs`
- Layout application lives in `src/layout.rs`
- GUI rendering lives in `src/gui.rs`
- Transform-origin f32 fields required removing `Eq` from `ComputedStyle` and related types; `PartialEq` uses `f32::to_bits()`
- Positioned elements (`absolute`/`fixed`) are collected into `positioned_commands: Vec<(i32, Vec<DrawCommand>)>` and composited sorted by z-index after normal flow
- Grid fr units use fixed-point *100 integer arithmetic to avoid f32 in `GridTrackSize`
