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
