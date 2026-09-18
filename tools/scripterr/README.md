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

Until 2026-09-18 tobira printed `start s1` for all four: the first failure
broke out of the script loop, and a non-2xx body was executed as script
(`a.html` also printed `BAD404 BAD500`). `TOBIRA_DEBUG_SCRIPTS=1` prints one
line per script with its outcome, which is how to count how many of a real
page's scripts actually ran.
