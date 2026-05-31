/// Integration tests for the new JS engine in browser-codex.
/// These tests exercise the full pipeline: HTML → DOM → inline scripts → snapshot.

use tobira::js::start_document_script_session;
use tobira::url::Url;

fn base_url() -> Url {
    Url::parse("http://example.com/").unwrap()
}

fn run(html: &str) -> (tobira::js_common::ProcessedScriptHtml, Option<tobira::js::JavaScriptSession>) {
    start_document_script_session(html, &base_url())
}

// ---------------------------------------------------------------------------
// Basic DOM manipulation
// ---------------------------------------------------------------------------

#[test]
fn console_log_captured() {
    let (snap, _) = run(r#"<html><body><script>console.log("hello from engine");</script></body></html>"#);
    assert!(snap.console_logs.iter().any(|l| l.contains("hello from engine")),
        "console_logs = {:?}", snap.console_logs);
}

#[test]
fn dom_content_loaded_fires() {
    let (snap, _) = run(r#"<html><body>
        <script>
        document.addEventListener('DOMContentLoaded', function() {
            document.body.innerHTML = '<p id="result">loaded</p>';
        });
        </script>
        </body></html>"#);
    assert!(snap.html.contains("loaded"), "html = {}", &snap.html[..snap.html.len().min(500)]);
}

#[test]
fn create_element_and_append() {
    let (snap, _) = run(r#"<html><body><div id="app"></div><script>
        var app = document.getElementById('app');
        var p = document.createElement('p');
        p.textContent = 'dynamic content';
        app.appendChild(p);
    </script></body></html>"#);
    assert!(snap.html.contains("dynamic content"), "html = {}", &snap.html[..snap.html.len().min(500)]);
}

#[test]
fn classlist_operations() {
    let (snap, _) = run(r#"<html><body><div id="box" class="a"></div><script>
        var box = document.getElementById('box');
        box.classList.add('b');
        box.classList.remove('a');
        var result = box.className;
        console.log('classname:' + result);
    </script></body></html>"#);
    assert!(snap.console_logs.iter().any(|l| l.contains("classname:b")),
        "logs = {:?}", snap.console_logs);
}

#[test]
fn set_attribute_and_get_attribute() {
    let (snap, _) = run(r#"<html><body><input id="inp"><script>
        var inp = document.getElementById('inp');
        inp.setAttribute('value', 'hello');
        var v = inp.getAttribute('value');
        console.log('value:' + v);
    </script></body></html>"#);
    assert!(snap.console_logs.iter().any(|l| l.contains("value:hello")),
        "logs = {:?}", snap.console_logs);
}

#[test]
fn window_document_identity() {
    let (snap, _) = run(r#"<html><body><script>
        var same = (window.document === document);
        console.log('same:' + same);
    </script></body></html>"#);
    assert!(snap.console_logs.iter().any(|l| l.contains("same:true")),
        "logs = {:?}", snap.console_logs);
}

#[test]
fn btoa_atob_roundtrip() {
    let (snap, _) = run(r#"<html><body><script>
        var encoded = btoa('Hello, World!');
        var decoded = atob(encoded);
        console.log('rt:' + decoded);
    </script></body></html>"#);
    assert!(snap.console_logs.iter().any(|l| l.contains("rt:Hello, World!")),
        "logs = {:?}", snap.console_logs);
}

#[test]
fn promise_in_domcontentloaded() {
    let (snap, _) = run(r#"<html><body><script>
        document.addEventListener('DOMContentLoaded', function() {
            Promise.resolve(42).then(function(v) {
                console.log('promise:' + v);
            });
        });
    </script></body></html>"#);
    assert!(snap.console_logs.iter().any(|l| l.contains("promise:42")),
        "logs = {:?}", snap.console_logs);
}

#[test]
fn async_domcontentloaded() {
    let (snap, _) = run(r#"<html><body><script>
        document.addEventListener('DOMContentLoaded', async function() {
            var v = await Promise.resolve(99);
            console.log('async:' + v);
        });
    </script></body></html>"#);
    assert!(snap.console_logs.iter().any(|l| l.contains("async:99")),
        "logs = {:?}", snap.console_logs);
}

#[test]
fn query_selector_all() {
    let (snap, _) = run(r#"<html><body>
        <ul><li class="item">a</li><li class="item">b</li><li class="item">c</li></ul>
        <script>
        var items = document.querySelectorAll('.item');
        console.log('count:' + items.length);
        </script>
    </body></html>"#);
    assert!(snap.console_logs.iter().any(|l| l.contains("count:3")),
        "logs = {:?}", snap.console_logs);
}

#[test]
fn innerhtml_mutation() {
    let (snap, _) = run(r#"<html><body><div id="root"></div><script>
        document.getElementById('root').innerHTML = '<span>injected</span>';
    </script></body></html>"#);
    assert!(snap.html.contains("injected"), "html = {}", &snap.html[..snap.html.len().min(500)]);
}

#[test]
fn live_session_event_dispatch() {
    // The session should stay alive so events can be dispatched after load
    let url = base_url();
    let (snap, session) = start_document_script_session(
        r#"<html><body><div id="out"></div><script>
            document.addEventListener('click', function(e) {
                document.getElementById('out').textContent = 'clicked';
            });
        </script></body></html>"#,
        &url,
    );
    assert!(session.is_some(), "session should be Some");
    let session = session.unwrap();
    // Dispatch a click on the document (node 0)
    let result = session.dispatch_event(tobira::js::DomEventRequest {
        target_node_id: 0,
        event_type: "click".to_string(),
        ..Default::default()
    });
    // Should get a snapshot back
    assert!(result.is_some(), "dispatch_event should return Some");
    let result = result.unwrap();
    assert!(result.snapshot.html.contains("clicked"),
        "snapshot html = {}", &result.snapshot.html[..result.snapshot.html.len().min(500)]);
}
