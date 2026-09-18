# Which built-ins does a polyfill replace here, and not in Chrome?

A polyfill bundle tests each native before trusting it, and installs its own
where the test fails. Every replacement that happens in tobira **and not in
Chrome** is a native that failed a check Chrome passes -- and the page then
runs the polyfill's code path, which leans on other natives in ways page code
seldom does. react.dev lost 484 elements that way (`Math.min(undefined, 3)`
inside core-js's `startsWith`).

`replaced.html` remembers every built-in function on 27 roots before the
polyfill loads, and lists those that are `!==` afterwards as JSON in
`<pre id="out">`.

```
sed 's#POLYFILL#polyfills.js#' tools/polyfill/replaced.html > pages/replaced.html   # next to the bundle
python -m http.server 8732 --directory pages &
"chrome.exe" --headless --virtual-time-budget=5000 --dump-dom http://127.0.0.1:8732/replaced.html > chrome.html
TOBIRA_DUMP_DOM=tobira.html ./target/release/tobira --cli http://127.0.0.1:8732/replaced.html
```

Read the **difference of the two sets**, never tobira's list alone: Chrome
gets replacements too (proposals it does not ship, the URL classes).

| bundle | tobira-only, first run | after `Object(primitive)` boxes (2026-09-19) |
|--------|------------------------|----------------------------------------------|
| react.dev's polyfills chunk | 45 | 32 |
| core-js-bundle 3.38.1 | 79 | 66 |

A fix must not make either number grow. Adding a half-implemented well-known
symbol does exactly that (see HANDOFF.md, 設計判断).
