# Script failure probes

Pages whose scripts fail on purpose, to check that one script's failure is
its own. Each page starts a `<pre id="out">` at `start` and every script that
runs appends its name, so the line Chrome prints is the expected one.

`srv.py` serves this directory on port 8733 and answers `/missing.js` with a
**404 whose body is JavaScript** and `/err500.js` with a 500 the same way. A
browser must not run either body.

```
python tools/scripterr/srv.py &
TOBIRA_DEBUG_CONSOLE=1 TOBIRA_DEBUG_SCRIPTS=1 ./target/release/tobira --cli http://127.0.0.1:8733/b.html
```

| page | what fails | Chrome prints |
|------|------------|---------------|
| `b.html` | a parse error, then an uncaught throw, between good scripts | `start s1 s3 s5` |
| `b2.html` | an uncaught throw only | `start s1 s3` |
| `a.html` | `src` answered 404 / 500 / 404 (HTML body) | `start s1 s3 s5 s7` |
| `net.html` | `src` refused / unknown host | `start s1 s3 s5` |
| `events.html` | `load` / `error` on script elements, four ways | see below |
| `onprop.html` | `el.onclick = fn` next to `addEventListener` | `start onclick-prop click-listener onclick-prop click-listener` |
| `style3.html` | `<style>` right after `</p>` with `<font>` still active | `style-parent body child #text ...` |
| `dyn.html` | scripts that script adds: seven ways | see below |
| `repro_classarg.html`, `repro_classarg2.html` | a class expression as a call argument, in every call shape | every line names the right types |
| `repro_new.html` .. `repro_new4.html` | the bisection that led there, from CodeMirror's `ViewPlugin.fromClass` | kept as the record |
| `repro_logical.html` | `a ||= v`, `a &&= v`, `a ??= v`, `import()`, `a++` as call arguments | every line names the right types (`??=` used to lose the callee) |
| `capture.html` | the three phases of propagation, a non-bubbling `error`, `stopPropagation` in capture, an `<img>` failure reaching `window` | see below |
| `fetchprobe.html`, `fetchfail.html` | `fetch()` same-origin / cross-origin / refused / unknown host / 404 / 500, `Promise.all` with a failure, XHR, an `<img>` that fails | every line matches Chrome except `img-onerror` |
| `geomprobe.html` | `offsetHeight` / `getBoundingClientRect` of elements a script just made, before any frame | `fixed 72 / plain 18 (Chrome 24: line height) / inline-fixed 50`, was all 0 |
| `domsurface_names.html` | run in Chrome: prints `Object.getOwnPropertyNames` of 14 interfaces' prototypes (and `window`) as JSON | the reference list, saved as `domsurface_names.json` |
| `domsurface.html` | generated from that list: for every name, on a fresh object and before touching it, `name in obj`, then `typeof obj[name]` | two lists per interface: what `in` denies, what is really absent |
| `numerics.html` | 73 places where Rust's numbers and JavaScript's differ: ToInt32 wrap-around and a string hash, `Math.round` / `sign` / `pow` / `hypot`, `toFixed` / `toPrecision` / `toExponential` / radix strings, number-to-string boundaries, `parseInt` / `parseFloat` / `Number()`, `%`, sorting, index arguments | 72 of 73 match Chrome (the `Date` range and day roll-over line differs); 19 lines differed before |
| `tonumber.html` | ToNumber on dates, arrays, objects with `valueOf`, boxed values, string forms, and through `Math.*` / `isNaN` | 8 of 8 |
| `mathmin.html` | `Math.min` / `max` with NaN, undefined, objects, no arguments and signed zero; `startsWith` / `endsWith` / `includes` given a RegExp, with and without `Symbol.match = false` | every line matches Chrome (`Math.min(undefined, 3)` was 3; the three string methods did not throw) |
| `nativesymbol.html` | the checks core-js uses to decide that Symbol and `Object.assign` are native-grade | three lines still differ: `String(Symbol("x"))`, `Object(Symbol()) instanceof Symbol`, the `assign` getter case. While they do, core-js replaces `startsWith` and `assign` here and not in Chrome |
| `ternary.html`, `destruct.html` | the conditional and the chained destructuring from react.dev's Thumbnail | match Chrome; kept from ruling the language out |
| `batchprobe.html` | the members `TOBIRA_DEBUG_MISSING` ranked: unset `on*` is null, `defaultView`, `namespaceURI`, `frames`, `fonts`, `contentEditable`, `complete`, `checked`, `getElementsByName`, `scripts` | every line matches Chrome |
| `litcheck.html` | Lit's test for adopting stylesheets | `lit-supportsAdopting=false` here, `true` in Chrome: Lit falls back to `<style>`, which is right for a renderer that applies no adopted sheet |
| `dsd.html` | a declarative shadow root: folding, `<slot>` assignment and fallback, `:host` / `::slotted()` / a class the document also uses, `document` not seeing in, `shadowRoot.getElementById` | every line matches Chrome |
| `attrsel.html`, `attrsel2.html`, `attrsel3.html` | the selectors the shadow rewrite produces, in a plain document | all applied; kept from the hunt for why `:host` did not reach (the geometry runs had not read the shadow `<style>`) |
| `supportsprobe.html` | `CSS.supports` (both forms, `not` / `and` / `or`) against `@supports` for the eight properties the renderer says no to | no line says `MISMATCH`; Chrome says yes to seven of the eight, which is a real difference and not a bug |
| `lobtime.html` | lobste.rs's shape: a module script, `DOMContentLoaded`, `innerText = ...` on `<time>` | `innerText-set:y/y ... set:N ago` (was `x/y`: innerText was an expando) |
| `styleprobe.html` | the whole CSSStyleDeclaration and DOMStringMap surface: `length` / `item` / `style[i]`, priorities, `cssFloat`, vendor prefixes, custom properties, `cssText` both ways, `in`, dataset keys / delete | every line matches Chrome except `background` shorthand expansion, `webkitTransform` aliasing to `transform`, and dataset key order (sorted here) |

`capture.html`, Chrome: `start win-cap:1:w doc-cap:1:#document a-cap:1:a
b-cap:1:b c-cap:2:c c-bub:2:c b-bub:3:b b-onprop:3 a-bub:3:a doc-bub:3:#document
win-bub:3:w | ew-cap:1 ed-cap:1 eb-cap:1 ec-cap:2 ec-bub:2 | kd-a-cap |
ew-cap:1 img-err-at-window:1 ed-cap:1 img-onerror`. tobira matches the
phases, the order and the non-bubbling `error`, with three known differences:
`window` and `document` are one handle (0), so both print `#document` and
their listeners come out in one registration order; the `on<type>` property
runs before that node's listeners rather than at the position it was first
set; and an `<img>` that fails to load fires no `error` at all yet (the last
four entries are missing).

`domstat.py` counts elements, text and tags in a serialized DOM and diffs two
of them. With `TOBIRA_DUMP_DOM=<path>` (the document as the scripts left it)
and `TOBIRA_DYNAMIC_SCRIPTS=0` (no scripts that script added), a page can be
read three ways -- Chrome, tobira without, tobira with -- and compared:

```
"chrome.exe" --headless --dump-dom URL > chrome.html
TOBIRA_DYNAMIC_SCRIPTS=0 TOBIRA_DUMP_DOM=off.html tobira --cli URL
TOBIRA_DUMP_DOM=on.html tobira --cli URL
python tools/scripterr/domstat.py off.html on.html chrome.html
```

That is how react.dev's blank page was found: `on` had 62 elements against
`off`'s 995, and Chrome's 1846.

`dyn.html`: Chrome prints `start inline-dyn sync-end next-parser-script
err-missing3 ran-d3 ran-d2 ran-d1 load-d1 ran-d4 ran-d5-chained`; tobira
prints `start sync-end ran-d1 load-d1 ran-d2 inline-dyn ran-d3 err-missing3
ran-d5-chained next-parser-script ran-d4`. Same set, different order, and the
order is the known approximation: Chrome runs a dynamic inline script the
moment it is inserted and fetches external ones off the main thread, running
each as it lands, after the parser's own scripts; tobira runs everything a
script added, in arrival order, as soon as that script is done. Until
2026-09-18 tobira printed `start sync-end next-parser-script` -- none of the
seven ran.

`events.html` is the one page where Chrome and tobira legitimately differ.
Chrome prints `start cap-err:script err-attr ran-ok cap-load:script load-attr
end cap-err:script err-dyn ran-ok2 cap-load:script load-dyn`; tobira prints
`start el-onerror:/missing.js el-error:/missing.js ran-ok el-onload:/ok.js
el-load:/ok.js end cap-load:#document`. The `el-*` lines are listeners an
earlier inline script attached straight to the later script elements, which
only works in tobira because it parses the whole document before any script
runs. The three gaps the page shows are real: `onerror="..."` **attributes**
are not handlers at all, a script element appended by script (`ran-ok2`)
**never runs**, and non-bubbling events skip the capture phase (`cap-err`).

Until 2026-09-18 tobira printed `start s1` for all four: the first failure
broke out of the script loop, and a non-2xx body was executed as script
(`a.html` also printed `BAD404 BAD500`). `TOBIRA_DEBUG_SCRIPTS=1` prints one
line per script with its outcome, which is how to count how many of a real
page's scripts actually ran.
