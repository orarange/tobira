// WPT's own report script talks to a test runner over postMessage. This one
// writes the results into the page instead, where `TOBIRA_DUMP_DOM` can read
// them: one line per test, `PASS`/`FAIL`/`TIMEOUT`/`NOTRUN` and its name, and
// a final `HARNESS <status>` line so a page that died before reporting is
// told apart from one with no tests in it.
//
// It replaces the file of the same name in WPT; everything else under
// `resources/` is theirs, unchanged.
add_completion_callback(function (tests, status) {
    var lines = tests.map(function (test) {
        var names = ["PASS", "FAIL", "TIMEOUT", "NOTRUN", "PRECONDITION_FAILED"];
        var name = names[test.status] || ("STATUS" + test.status);
        var line = name + " " + test.name;
        if (test.status !== 0 && test.message) {
            line += " -- " + String(test.message).split("\n")[0];
        }
        return line;
    });
    var harness = ["OK", "ERROR", "TIMEOUT", "PRECONDITION_FAILED"][status.status];
    lines.push("HARNESS " + (harness || status.status) +
               (status.message ? " -- " + String(status.message).split("\n")[0] : ""));
    var out = document.getElementById("__wpt_out");
    if (!out) {
        out = document.createElement("pre");
        out.id = "__wpt_out";
        (document.body || document.documentElement).appendChild(out);
    }
    out.textContent = lines.join("\n");
});
