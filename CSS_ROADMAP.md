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

## Phase 5 — Future Work ❌

Not yet planned or implemented.

| Feature | Priority | Notes |
|---------|----------|-------|
| `position: sticky` scroll tracking | Medium | Requires scroll-offset propagation into layout |
| `transform: scale/rotate` rendering | Medium | Needs affine transform in software renderer |
| CSS `animation` / `@keyframes` | Low | Requires animation runtime and repaint loop |
| `transition` interpolation | Low | Requires repaint loop and state diffing |
| `display: inline-flex` | Medium | Inline-level flex container |
| `display: inline-grid` | Low | Inline-level grid container |
| `flex-flow` shorthand | Low | Shorthand for `flex-direction` + `flex-wrap` |
| `align-content` | Low | Multi-line flex alignment |
| `grid-template-areas` | Low | Named area placement |
| `grid-auto-flow` | Low | Auto-placement algorithm |
| `min-content` / `max-content` | Medium | Intrinsic sizing keywords |
| `fit-content()` | Low | Clamped intrinsic size |
| `clamp()` | Medium | `clamp(min, val, max)` in property values |
| `min()` / `max()` | Medium | CSS math functions |
| `filter` | Low | `blur()`, `brightness()`, etc. |
| `clip-path` | Low | Shape clipping |
| `backdrop-filter` | Low | Behind-element blur |
| `counter()` / `counters()` | Low | CSS counters for lists |
| `content: attr(...)` | Medium | Attribute value in pseudo-elements |
| `:hover` / `:focus` / `:active` | Medium | Interactive pseudo-classes |
| `:checked` / `:disabled` | Low | Form state pseudo-classes |
| `::placeholder` | Low | Input placeholder styling |
| `::selection` | Low | Selected text styling |
| CSS `@supports` | Low | Feature queries |
| CSS `@layer` | Low | Cascade layers |
| `writing-mode` | Low | Vertical text layout |
| `direction` / `unicode-bidi` | Low | RTL text support |
| `scroll-behavior: smooth` | Low | Smooth scrolling |
| `aspect-ratio` | Medium | Intrinsic aspect ratio |
| `object-fit` / `object-position` | Medium | Image fitting inside box |
| `resize` | Low | User-resizable elements |
| `cursor` | Low | Mouse cursor styling |
| `pointer-events` | Low | Hit-testing control |

---

## Architecture Notes

- CSS parsing lives in `src/css.rs`
- Layout application lives in `src/layout.rs`
- GUI rendering lives in `src/gui.rs`
- Transform-origin f32 fields required removing `Eq` from `ComputedStyle` and related types; `PartialEq` uses `f32::to_bits()`
- Positioned elements (`absolute`/`fixed`) are collected into `positioned_commands: Vec<(i32, Vec<DrawCommand>)>` and composited sorted by z-index after normal flow
- Grid fr units use fixed-point *100 integer arithmetic to avoid f32 in `GridTrackSize`
