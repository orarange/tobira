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

### Phase 6 — Completed in browser-codex merge ✅

The merge of `browser-claude` (feat/outline-text-decoration) plus the
follow-up Batch 1–4 commits brought the following Phase 6 items in:

| Feature | Status | Notes |
|---------|--------|-------|
| `transform: scale/rotate` rendering | ✅ | Affine transform in software renderer |
| `position: sticky` scroll tracking | ✅ | StickyCommand + scroll-aware render in gui.rs |
| `grid-template-areas` | ✅ | Named area placement |
| `grid-auto-flow` (incl. `dense`) | ✅ | Dense packing restarts slot search from (0,0) |
| `counter()` / `counters()` | ✅ | CSS counters in pseudo-element content |
| `background-repeat` tiling | ✅ | Full `repeat-x` / `repeat-y` / `repeat` axes |
| `text-shadow` with blur | ✅ | Offscreen render + box blur + alpha blit |
| `::selection` styling | ✅ | `compute_selection_style` → FormControlCommand.selection_bg/fg |
| `::placeholder` GUI wiring | ✅ | `compute_placeholder_style` → FormControlCommand.placeholder_color/italic |
| `clip-path` (circle/inset/polygon) | ✅ | `ClipPath` enum + `apply_clip_path` in offscreen pass |
| `writing-mode` (parse) | 🔧 | Stored on ComputedStyle; vertical layout not applied yet |
| `direction: rtl` (parse) | 🔧 | Stored on ComputedStyle; full RTL inline reorder not done |
| `scroll-behavior: smooth` (parse) | 🔧 | Stored on ComputedStyle; animated scroll not driven yet |
| CSS `animation` / `@keyframes` | ✅ | `animation:` shorthand + longhands parse into `AnimationSpec`s; `about_to_wait` drives ~16ms frames; opacity / color / background-color / transform interpolated. Demo: `demo/animation-demo.html` |
| `transition` interpolation | 🔧 | `transition:` parses into `TransitionSpec`s and `apply_transitions_to_style` is ready, but not yet driven — needs the previous-style snapshot + `transition_starts` (groundwork fields exist on `BrowserPage`) |

### Phase 6 — Remaining work ❌

| Feature | Priority | Notes |
|---------|----------|-------|
| Transition driving | High | Snapshot previous `ComputedStyle` per element, record `transition_starts` on change, call `apply_transitions_to_style` from the frame loop. Groundwork fields (`previous_styles`, `transition_starts`) already on `BrowserPage` |
| Per-element animation start tracking | Medium | Animations are anchored to one page-level epoch (`animation_epoch`); `animation_starts` map is groundwork for per-element delays/restarts |
| `writing-mode` vertical layout | Low | Rotate text glyphs + flip block/inline axes in layout |
| `direction: rtl` inline reorder | Low | Needs TextAlign::Start/End and reversed line construction |
| `scroll-behavior: smooth` animation | Low | Multi-step scroll reusing the animation frame timer; `smooth_scroll` field is groundwork |

---

## Architecture Notes

- CSS parsing lives in `src/css.rs`
- Layout application lives in `src/layout.rs`
- GUI rendering lives in `src/gui.rs`
- Transform-origin f32 fields required removing `Eq` from `ComputedStyle` and related types; `PartialEq` uses `f32::to_bits()`
- Positioned elements (`absolute`/`fixed`) are collected into `positioned_commands: Vec<(i32, Vec<DrawCommand>)>` and composited sorted by z-index after normal flow
- Grid fr units use fixed-point *100 integer arithmetic to avoid f32 in `GridTrackSize`
