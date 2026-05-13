use std::cell::RefCell;
use std::collections::BTreeMap;
use std::mem;
use std::time::{SystemTime, UNIX_EPOCH};

use boa_engine::object::{ObjectInitializer, builtins::JsFunction};
use boa_engine::property::Attribute;
use boa_engine::{
    Context, Finalize, JsData, JsResult, JsValue, NativeFunction, Source, Trace, js_string,
};

use crate::html::{Node, parse_document};
use crate::http::fetch;
use crate::text::decode_text_response;
use crate::url::Url;

const MAX_SCRIPT_RECURSION: usize = 8;
const MAX_SCRIPT_SOURCE_BYTES: usize = 48 * 1024;
const MAX_TOTAL_SCRIPT_BYTES: usize = 192 * 1024;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProcessedScriptHtml {
    pub html: String,
    pub title_override: Option<String>,
    pub console_logs: Vec<String>,
}

#[derive(Debug, Default)]
struct JavaScriptState {
    current_title: String,
    title_dirty: bool,
    write_buffer: String,
    console_logs: Vec<String>,
    location_href: String,
}

#[derive(Debug, Trace, Finalize, JsData)]
struct JavaScriptHostData {
    #[unsafe_ignore_trace]
    state: RefCell<JavaScriptState>,
}

pub fn process_document_scripts(html: &str, base_url: &Url) -> ProcessedScriptHtml {
    let initial_title = extract_title_text(html).unwrap_or_default();
    let mut runtime = JavaScriptRuntime::new(base_url, &initial_title);
    let expanded_html = expand_scripts_in_html(html, base_url, &mut runtime, 0);
    let title_override = runtime.title_override();
    let html = if let Some(title) = title_override.as_ref() {
        apply_title_override(&expanded_html, title)
    } else {
        expanded_html
    };

    ProcessedScriptHtml {
        html,
        title_override,
        console_logs: runtime.take_logs(),
    }
}

struct JavaScriptRuntime {
    context: Context,
    executed_bytes: usize,
    host: String,
}

impl JavaScriptRuntime {
    fn new(base_url: &Url, initial_title: &str) -> Self {
        let mut context = Context::default();
        context.insert_data(JavaScriptHostData {
            state: RefCell::new(JavaScriptState {
                current_title: initial_title.to_string(),
                title_dirty: false,
                write_buffer: String::new(),
                console_logs: Vec::new(),
                location_href: base_url.to_string(),
            }),
        });

        install_browser_globals(&mut context);

        Self {
            context,
            executed_bytes: 0,
            host: base_url.host.to_ascii_lowercase(),
        }
    }

    fn execute(&mut self, source: &str) {
        if source.trim().is_empty() {
            return;
        }

        if !is_supported_script_source(source, &self.host) {
            self.push_log(format!(
                "js skip: unsupported script pattern ({} bytes)",
                source.len()
            ));
            return;
        }

        if self.executed_bytes.saturating_add(source.len()) > MAX_TOTAL_SCRIPT_BYTES {
            self.push_log(format!(
                "js skip: script budget exceeded at {} bytes",
                self.executed_bytes.saturating_add(source.len())
            ));
            return;
        }

        self.executed_bytes = self.executed_bytes.saturating_add(source.len());

        if let Err(error) = self.context.eval(Source::from_bytes(source)) {
            self.push_log(format!("js error: {error}"));
        }
    }

    fn take_written_html(&self) -> String {
        let Some(host) = self.context.get_data::<JavaScriptHostData>() else {
            return String::new();
        };

        mem::take(&mut host.state.borrow_mut().write_buffer)
    }

    fn title_override(&self) -> Option<String> {
        let host = self.context.get_data::<JavaScriptHostData>()?;
        let state = host.state.borrow();
        state.title_dirty.then(|| state.current_title.clone())
    }

    fn take_logs(&self) -> Vec<String> {
        let Some(host) = self.context.get_data::<JavaScriptHostData>() else {
            return Vec::new();
        };

        mem::take(&mut host.state.borrow_mut().console_logs)
    }

    fn push_log(&self, message: String) {
        if let Some(host) = self.context.get_data::<JavaScriptHostData>() {
            host.state.borrow_mut().console_logs.push(message);
        }
    }
}

fn install_browser_globals(context: &mut Context) {
    let title_getter =
        NativeFunction::from_fn_ptr(js_document_get_title).to_js_function(context.realm());
    let title_setter =
        NativeFunction::from_fn_ptr(js_document_set_title).to_js_function(context.realm());
    let href_getter =
        NativeFunction::from_fn_ptr(js_location_get_href).to_js_function(context.realm());
    let href_setter =
        NativeFunction::from_fn_ptr(js_location_set_href).to_js_function(context.realm());
    let search_getter =
        NativeFunction::from_fn_ptr(js_location_get_search).to_js_function(context.realm());
    let hash_getter =
        NativeFunction::from_fn_ptr(js_location_get_hash).to_js_function(context.realm());
    let pathname_getter =
        NativeFunction::from_fn_ptr(js_location_get_pathname).to_js_function(context.realm());
    let origin_getter =
        NativeFunction::from_fn_ptr(js_location_get_origin).to_js_function(context.realm());
    let host_getter =
        NativeFunction::from_fn_ptr(js_location_get_host).to_js_function(context.realm());
    let hostname_getter =
        NativeFunction::from_fn_ptr(js_location_get_hostname).to_js_function(context.realm());
    let protocol_getter =
        NativeFunction::from_fn_ptr(js_location_get_protocol).to_js_function(context.realm());

    let location = ObjectInitializer::new(context)
        .accessor(
            js_string!("href"),
            Some(href_getter),
            Some(href_setter),
            Attribute::all(),
        )
        .accessor(
            js_string!("search"),
            Some(search_getter),
            None,
            Attribute::all(),
        )
        .accessor(
            js_string!("hash"),
            Some(hash_getter),
            None,
            Attribute::all(),
        )
        .accessor(
            js_string!("pathname"),
            Some(pathname_getter),
            None,
            Attribute::all(),
        )
        .accessor(
            js_string!("origin"),
            Some(origin_getter),
            None,
            Attribute::all(),
        )
        .accessor(
            js_string!("host"),
            Some(host_getter),
            None,
            Attribute::all(),
        )
        .accessor(
            js_string!("hostname"),
            Some(hostname_getter),
            None,
            Attribute::all(),
        )
        .accessor(
            js_string!("protocol"),
            Some(protocol_getter),
            None,
            Attribute::all(),
        )
        .function(
            NativeFunction::from_fn_ptr(js_location_assign),
            js_string!("assign"),
            1,
        )
        .function(
            NativeFunction::from_fn_ptr(js_location_replace),
            js_string!("replace"),
            1,
        )
        .build();

    let dom_stub = build_dom_stub(context);
    let node_list_stub = build_node_list_stub(context);
    let document_fonts = ObjectInitializer::new(context)
        .function(NativeFunction::from_fn_ptr(js_noop), js_string!("load"), 2)
        .build();

    let global_object = context.global_object();
    let document = ObjectInitializer::new(context)
        .function(
            NativeFunction::from_fn_ptr(js_document_write),
            js_string!("write"),
            1,
        )
        .function(
            NativeFunction::from_fn_ptr(js_document_writeln),
            js_string!("writeln"),
            1,
        )
        .accessor(
            js_string!("title"),
            Some(title_getter),
            Some(title_setter),
            Attribute::all(),
        )
        .function(
            NativeFunction::from_fn_ptr(js_return_dom_stub),
            js_string!("querySelector"),
            1,
        )
        .function(
            NativeFunction::from_fn_ptr(js_return_node_list_stub),
            js_string!("querySelectorAll"),
            1,
        )
        .function(
            NativeFunction::from_fn_ptr(js_return_dom_stub),
            js_string!("getElementById"),
            1,
        )
        .function(
            NativeFunction::from_fn_ptr(js_return_dom_stub),
            js_string!("createElement"),
            1,
        )
        .function(
            NativeFunction::from_fn_ptr(js_add_event_listener),
            js_string!("addEventListener"),
            2,
        )
        .function(
            NativeFunction::from_fn_ptr(js_noop),
            js_string!("removeEventListener"),
            2,
        )
        .property(js_string!("location"), location.clone(), Attribute::all())
        .property(js_string!("body"), dom_stub.clone(), Attribute::all())
        .property(js_string!("head"), dom_stub.clone(), Attribute::all())
        .property(
            js_string!("documentElement"),
            dom_stub.clone(),
            Attribute::all(),
        )
        .property(js_string!("fonts"), document_fonts, Attribute::all())
        .property(
            js_string!("readyState"),
            js_string!("complete"),
            Attribute::all(),
        )
        .property(
            js_string!("compatMode"),
            js_string!("CSS1Compat"),
            Attribute::all(),
        )
        .property(js_string!("hidden"), false, Attribute::all())
        .property(
            js_string!("visibilityState"),
            js_string!("visible"),
            Attribute::all(),
        )
        .property(
            js_string!("defaultView"),
            global_object.clone(),
            Attribute::all(),
        )
        .build();

    let console = ObjectInitializer::new(context)
        .function(
            NativeFunction::from_fn_ptr(js_console_log),
            js_string!("log"),
            1,
        )
        .function(
            NativeFunction::from_fn_ptr(js_console_log),
            js_string!("info"),
            1,
        )
        .function(
            NativeFunction::from_fn_ptr(js_console_log),
            js_string!("warn"),
            1,
        )
        .function(
            NativeFunction::from_fn_ptr(js_console_log),
            js_string!("error"),
            1,
        )
        .build();

    context
        .register_global_property(js_string!("document"), document, Attribute::all())
        .expect("document should be installable");
    context
        .register_global_property(js_string!("location"), location, Attribute::all())
        .expect("location should be installable");
    context
        .register_global_property(js_string!("console"), console, Attribute::all())
        .expect("console should be installable");
    let navigator = ObjectInitializer::new(context)
        .property(
            js_string!("userAgent"),
            js_string!(
                "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/136.0.0.0 Safari/537.36"
            ),
            Attribute::all(),
        )
        .property(js_string!("language"), js_string!("ja-JP"), Attribute::all())
        .property(js_string!("languages"), node_list_stub.clone(), Attribute::all())
        .property(js_string!("platform"), js_string!("Win32"), Attribute::all())
        .property(js_string!("vendor"), js_string!("Google Inc."), Attribute::all())
        .build();
    let performance_timing = ObjectInitializer::new(context)
        .property(js_string!("navigationStart"), 0, Attribute::all())
        .build();
    let performance = ObjectInitializer::new(context)
        .function(
            NativeFunction::from_fn_ptr(js_performance_now),
            js_string!("now"),
            0,
        )
        .property(js_string!("timing"), performance_timing, Attribute::all())
        .build();
    let history = ObjectInitializer::new(context)
        .function(
            NativeFunction::from_fn_ptr(js_noop),
            js_string!("pushState"),
            3,
        )
        .function(
            NativeFunction::from_fn_ptr(js_noop),
            js_string!("replaceState"),
            3,
        )
        .function(NativeFunction::from_fn_ptr(js_noop), js_string!("back"), 0)
        .function(
            NativeFunction::from_fn_ptr(js_noop),
            js_string!("forward"),
            0,
        )
        .build();
    let storage = build_storage_stub(context);
    let ytcfg = ObjectInitializer::new(context)
        .function(
            NativeFunction::from_fn_ptr(js_ytcfg_data),
            js_string!("d"),
            0,
        )
        .function(
            NativeFunction::from_fn_ptr(js_ytcfg_get),
            js_string!("get"),
            2,
        )
        .function(NativeFunction::from_fn_ptr(js_noop), js_string!("set"), 2)
        .build();
    let ytcsi = ObjectInitializer::new(context)
        .function(
            NativeFunction::from_fn_ptr(js_return_dom_stub),
            js_string!("gt"),
            1,
        )
        .function(NativeFunction::from_fn_ptr(js_noop), js_string!("tick"), 3)
        .function(NativeFunction::from_fn_ptr(js_noop), js_string!("info"), 3)
        .function(
            NativeFunction::from_fn_ptr(js_noop),
            js_string!("infoGel"),
            2,
        )
        .function(
            NativeFunction::from_fn_ptr(js_noop),
            js_string!("setStart"),
            2,
        )
        .build();
    context
        .register_global_property(js_string!("navigator"), navigator, Attribute::all())
        .expect("navigator should be installable");
    context
        .register_global_property(js_string!("performance"), performance, Attribute::all())
        .expect("performance should be installable");
    context
        .register_global_property(js_string!("history"), history, Attribute::all())
        .expect("history should be installable");
    context
        .register_global_property(
            js_string!("localStorage"),
            storage.clone(),
            Attribute::all(),
        )
        .expect("localStorage should be installable");
    context
        .register_global_property(js_string!("sessionStorage"), storage, Attribute::all())
        .expect("sessionStorage should be installable");
    context
        .register_global_property(js_string!("ytcfg"), ytcfg, Attribute::all())
        .expect("ytcfg should be installable");
    context
        .register_global_property(js_string!("ytcsi"), ytcsi, Attribute::all())
        .expect("ytcsi should be installable");
    context
        .register_global_property(
            js_string!("window"),
            global_object.clone(),
            Attribute::all(),
        )
        .expect("window should be installable");
    context
        .register_global_property(js_string!("self"), global_object, Attribute::all())
        .expect("self should be installable");
    context
        .register_global_builtin_callable(
            js_string!("setTimeout"),
            2,
            NativeFunction::from_fn_ptr(js_set_timeout),
        )
        .expect("setTimeout should be installable");
    context
        .register_global_builtin_callable(
            js_string!("setInterval"),
            2,
            NativeFunction::from_fn_ptr(js_set_timeout),
        )
        .expect("setInterval should be installable");
    context
        .register_global_builtin_callable(
            js_string!("clearTimeout"),
            1,
            NativeFunction::from_fn_ptr(js_clear_timeout),
        )
        .expect("clearTimeout should be installable");
    context
        .register_global_builtin_callable(
            js_string!("clearInterval"),
            1,
            NativeFunction::from_fn_ptr(js_clear_timeout),
        )
        .expect("clearInterval should be installable");
    context
        .register_global_builtin_callable(
            js_string!("requestAnimationFrame"),
            1,
            NativeFunction::from_fn_ptr(js_request_animation_frame),
        )
        .expect("requestAnimationFrame should be installable");
    context
        .register_global_builtin_callable(
            js_string!("cancelAnimationFrame"),
            1,
            NativeFunction::from_fn_ptr(js_clear_timeout),
        )
        .expect("cancelAnimationFrame should be installable");
    context
        .register_global_builtin_callable(
            js_string!("queueMicrotask"),
            1,
            NativeFunction::from_fn_ptr(js_queue_microtask),
        )
        .expect("queueMicrotask should be installable");
    context
        .register_global_builtin_callable(
            js_string!("matchMedia"),
            1,
            NativeFunction::from_fn_ptr(js_match_media),
        )
        .expect("matchMedia should be installable");
    context
        .register_global_builtin_callable(
            js_string!("addEventListener"),
            2,
            NativeFunction::from_fn_ptr(js_add_event_listener),
        )
        .expect("addEventListener should be installable");
    context
        .register_global_builtin_callable(
            js_string!("removeEventListener"),
            2,
            NativeFunction::from_fn_ptr(js_noop),
        )
        .expect("removeEventListener should be installable");
    context
        .register_global_builtin_callable(
            js_string!("alert"),
            1,
            NativeFunction::from_fn_ptr(js_alert),
        )
        .expect("alert should be installable");
    context
        .register_global_builtin_callable(
            js_string!("confirm"),
            1,
            NativeFunction::from_fn_ptr(js_confirm),
        )
        .expect("confirm should be installable");
    context
        .register_global_builtin_callable(
            js_string!("prompt"),
            2,
            NativeFunction::from_fn_ptr(js_prompt),
        )
        .expect("prompt should be installable");
    context
        .register_global_property(js_string!("innerWidth"), 1280, Attribute::all())
        .expect("innerWidth should be installable");
    context
        .register_global_property(js_string!("innerHeight"), 720, Attribute::all())
        .expect("innerHeight should be installable");
}

fn build_dom_stub(context: &mut Context) -> boa_engine::object::JsObject {
    let node_list = build_node_list_stub(context);
    let style = ObjectInitializer::new(context).build();
    let class_list = ObjectInitializer::new(context)
        .function(NativeFunction::from_fn_ptr(js_noop), js_string!("add"), 1)
        .function(
            NativeFunction::from_fn_ptr(js_noop),
            js_string!("remove"),
            1,
        )
        .function(
            NativeFunction::from_fn_ptr(js_return_false),
            js_string!("contains"),
            1,
        )
        .function(
            NativeFunction::from_fn_ptr(js_return_false),
            js_string!("toggle"),
            1,
        )
        .build();

    ObjectInitializer::new(context)
        .function(
            NativeFunction::from_fn_ptr(js_return_dom_stub),
            js_string!("querySelector"),
            1,
        )
        .function(
            NativeFunction::from_fn_ptr(js_return_node_list_stub),
            js_string!("querySelectorAll"),
            1,
        )
        .function(
            NativeFunction::from_fn_ptr(js_return_dom_stub),
            js_string!("appendChild"),
            1,
        )
        .function(
            NativeFunction::from_fn_ptr(js_return_dom_stub),
            js_string!("insertBefore"),
            2,
        )
        .function(
            NativeFunction::from_fn_ptr(js_return_null),
            js_string!("getAttribute"),
            1,
        )
        .function(
            NativeFunction::from_fn_ptr(js_noop),
            js_string!("setAttribute"),
            2,
        )
        .function(
            NativeFunction::from_fn_ptr(js_noop),
            js_string!("removeAttribute"),
            1,
        )
        .function(
            NativeFunction::from_fn_ptr(js_add_event_listener),
            js_string!("addEventListener"),
            2,
        )
        .function(
            NativeFunction::from_fn_ptr(js_noop),
            js_string!("removeEventListener"),
            2,
        )
        .function(NativeFunction::from_fn_ptr(js_noop), js_string!("focus"), 0)
        .function(NativeFunction::from_fn_ptr(js_noop), js_string!("blur"), 0)
        .function(
            NativeFunction::from_fn_ptr(js_noop),
            js_string!("remove"),
            0,
        )
        .function(
            NativeFunction::from_fn_ptr(js_return_dom_rect_stub),
            js_string!("getBoundingClientRect"),
            0,
        )
        .function(
            NativeFunction::from_fn_ptr(js_get_video_aspect_ratio),
            js_string!("getVideoAspectRatio"),
            0,
        )
        .property(js_string!("style"), style, Attribute::all())
        .property(js_string!("classList"), class_list, Attribute::all())
        .property(js_string!("children"), node_list.clone(), Attribute::all())
        .property(js_string!("childNodes"), node_list, Attribute::all())
        .property(js_string!("textContent"), js_string!(""), Attribute::all())
        .property(js_string!("innerHTML"), js_string!(""), Attribute::all())
        .property(js_string!("value"), js_string!(""), Attribute::all())
        .property(js_string!("checked"), false, Attribute::all())
        .property(js_string!("hidden"), false, Attribute::all())
        .property(js_string!("clientWidth"), 1280, Attribute::all())
        .property(js_string!("clientHeight"), 720, Attribute::all())
        .property(js_string!("scrollWidth"), 1280, Attribute::all())
        .property(js_string!("scrollHeight"), 720, Attribute::all())
        .build()
}

fn build_node_list_stub(context: &mut Context) -> boa_engine::object::JsObject {
    ObjectInitializer::new(context)
        .function(
            NativeFunction::from_fn_ptr(js_noop),
            js_string!("forEach"),
            1,
        )
        .function(
            NativeFunction::from_fn_ptr(js_return_undefined),
            js_string!("item"),
            1,
        )
        .property(js_string!("length"), 0, Attribute::all())
        .build()
}

fn build_storage_stub(context: &mut Context) -> boa_engine::object::JsObject {
    ObjectInitializer::new(context)
        .function(
            NativeFunction::from_fn_ptr(js_return_null),
            js_string!("getItem"),
            1,
        )
        .function(
            NativeFunction::from_fn_ptr(js_noop),
            js_string!("setItem"),
            2,
        )
        .function(
            NativeFunction::from_fn_ptr(js_noop),
            js_string!("removeItem"),
            1,
        )
        .function(NativeFunction::from_fn_ptr(js_noop), js_string!("clear"), 0)
        .property(js_string!("length"), 0, Attribute::all())
        .build()
}

fn build_match_media_stub(context: &mut Context, media: String) -> boa_engine::object::JsObject {
    ObjectInitializer::new(context)
        .function(
            NativeFunction::from_fn_ptr(js_add_event_listener),
            js_string!("addEventListener"),
            2,
        )
        .function(
            NativeFunction::from_fn_ptr(js_noop),
            js_string!("removeEventListener"),
            2,
        )
        .function(
            NativeFunction::from_fn_ptr(js_noop),
            js_string!("addListener"),
            1,
        )
        .function(
            NativeFunction::from_fn_ptr(js_noop),
            js_string!("removeListener"),
            1,
        )
        .property(js_string!("matches"), false, Attribute::all())
        .property(js_string!("media"), js_string!(media), Attribute::all())
        .build()
}

fn build_dom_rect_stub(context: &mut Context) -> boa_engine::object::JsObject {
    ObjectInitializer::new(context)
        .property(js_string!("x"), 0, Attribute::all())
        .property(js_string!("y"), 0, Attribute::all())
        .property(js_string!("top"), 0, Attribute::all())
        .property(js_string!("left"), 0, Attribute::all())
        .property(js_string!("right"), 1280, Attribute::all())
        .property(js_string!("bottom"), 720, Attribute::all())
        .property(js_string!("width"), 1280, Attribute::all())
        .property(js_string!("height"), 720, Attribute::all())
        .build()
}

fn expand_scripts_in_html(
    html: &str,
    base_url: &Url,
    runtime: &mut JavaScriptRuntime,
    depth: usize,
) -> String {
    if depth >= MAX_SCRIPT_RECURSION {
        return html.to_string();
    }

    let mut output = String::new();
    let mut cursor = 0;

    while let Some(script_offset) = find_case_insensitive(&html[cursor..], "<script") {
        let start = cursor + script_offset;
        output.push_str(&html[cursor..start]);

        let Some(open_end) = find_tag_end(&html[start..]) else {
            output.push_str(&html[start..]);
            return output;
        };
        let open_end = start + open_end;
        let open_tag = &html[start..=open_end];
        let attributes = parse_tag_attributes(open_tag);
        let self_closing = open_tag.trim_end_matches('>').trim_end().ends_with('/');

        let (script_body, next_cursor) = if self_closing {
            ("", open_end + 1)
        } else if let Some(close_offset) = find_case_insensitive(&html[open_end + 1..], "</script>")
        {
            let close_start = open_end + 1 + close_offset;
            (
                &html[open_end + 1..close_start],
                close_start + "</script>".len(),
            )
        } else {
            (&html[open_end + 1..], html.len())
        };

        if should_execute_script(&attributes) {
            if let Some(source) = load_script_source(script_body, &attributes, base_url) {
                runtime.execute(&source);
                let written = runtime.take_written_html();
                if !written.is_empty() {
                    output.push_str(&expand_scripts_in_html(
                        &written,
                        base_url,
                        runtime,
                        depth + 1,
                    ));
                }
            }
        } else {
            output.push_str(&html[start..next_cursor]);
        }

        cursor = next_cursor;
    }

    output.push_str(&html[cursor..]);
    output
}

fn load_script_source(
    inline_script: &str,
    attributes: &BTreeMap<String, String>,
    base_url: &Url,
) -> Option<String> {
    if let Some(src) = attributes.get("src") {
        let url = base_url.resolve(src).ok()?;
        let response = fetch(&url).ok()?;
        return Some(decode_text_response(
            &response.body,
            response.header("content-type"),
        ));
    }

    Some(inline_script.to_string())
}

fn should_execute_script(attributes: &BTreeMap<String, String>) -> bool {
    let language = attributes
        .get("language")
        .map(|value| value.trim().to_ascii_lowercase());
    if let Some(language) = language {
        let language = language.trim_start_matches("text/");
        if !matches!(language, "" | "javascript" | "jscript" | "ecmascript") {
            return false;
        }
    }

    let script_type = attributes
        .get("type")
        .map(|value| value.trim().to_ascii_lowercase());
    match script_type.as_deref() {
        None
        | Some("")
        | Some("text/javascript")
        | Some("application/javascript")
        | Some("application/ecmascript")
        | Some("text/ecmascript") => true,
        Some("module") | Some("text/module") => false,
        Some(other) if other.ends_with("javascript") || other.ends_with("ecmascript") => true,
        Some(_) => false,
    }
}

fn is_supported_script_source(source: &str, host: &str) -> bool {
    if source.len() > MAX_SCRIPT_SOURCE_BYTES {
        return false;
    }

    let lowered = source.to_ascii_lowercase();
    if contains_any(&lowered, BLOCKED_SCRIPT_PATTERNS) {
        return false;
    }

    if is_youtube_host_name(host) {
        return true;
    }

    contains_any(&lowered, CONSERVATIVE_SCRIPT_PATTERNS)
}

const CONSERVATIVE_SCRIPT_PATTERNS: &[&str] = &[
    "document.write",
    "document.writeln",
    "document.title",
    "settimeout",
    "alert(",
    "confirm(",
    "prompt(",
    "location.href",
    "location.assign",
    "location.replace",
    "console.log",
    "console.info",
    "console.warn",
    "console.error",
];

const BLOCKED_SCRIPT_PATTERNS: &[&str] = &[
    "document.cookie",
    "xmlhttprequest",
    "fetch(",
    "websocket(",
    "eventsource(",
    "sharedworker(",
    "serviceworker",
    "indexeddb",
    "navigator.serviceworker",
    "new image",
    "eval(",
    "new function",
    "import(",
];

fn contains_any(haystack: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| haystack.contains(needle))
}

fn is_youtube_host_name(host: &str) -> bool {
    host == "youtube.com" || host.ends_with(".youtube.com")
}

fn parse_tag_attributes(tag: &str) -> BTreeMap<String, String> {
    let mut attributes = BTreeMap::new();
    let bytes = tag.as_bytes();
    let mut index = 0;

    if bytes.first() == Some(&b'<') {
        index += 1;
    }
    while index < bytes.len() && is_tag_name_char(bytes[index]) {
        index += 1;
    }

    while index < bytes.len() {
        skip_whitespace(tag, &mut index);
        if index >= bytes.len() || matches!(bytes[index], b'>' | b'/') {
            index += 1;
            continue;
        }

        let start = index;
        while index < bytes.len()
            && !matches!(
                bytes[index],
                b'=' | b'>' | b'/' | b' ' | b'\n' | b'\r' | b'\t'
            )
        {
            index += 1;
        }

        let name = tag[start..index].trim().to_ascii_lowercase();
        skip_whitespace(tag, &mut index);
        let value = if index < bytes.len() && bytes[index] == b'=' {
            index += 1;
            skip_whitespace(tag, &mut index);
            parse_attribute_value(tag, &mut index)
        } else {
            String::new()
        };

        if !name.is_empty() {
            attributes.insert(name, value);
        }
    }

    attributes
}

fn parse_attribute_value(input: &str, index: &mut usize) -> String {
    let bytes = input.as_bytes();
    if *index >= bytes.len() {
        return String::new();
    }

    let quote = bytes[*index];
    if quote == b'"' || quote == b'\'' {
        *index += 1;
        let start = *index;
        while *index < bytes.len() && bytes[*index] != quote {
            *index += 1;
        }
        let value = input[start..*index].to_string();
        if *index < bytes.len() {
            *index += 1;
        }
        value
    } else {
        let start = *index;
        while *index < bytes.len()
            && !matches!(bytes[*index], b'>' | b'/' | b' ' | b'\n' | b'\r' | b'\t')
        {
            *index += 1;
        }
        input[start..*index].to_string()
    }
}

fn skip_whitespace(input: &str, index: &mut usize) {
    let bytes = input.as_bytes();
    while *index < bytes.len() && bytes[*index].is_ascii_whitespace() {
        *index += 1;
    }
}

fn is_tag_name_char(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b':' | b'_')
}

fn find_tag_end(input: &str) -> Option<usize> {
    let mut quote = None;
    for (index, character) in input.char_indices() {
        match character {
            '"' | '\'' if quote.is_none() => quote = Some(character),
            '"' | '\'' if quote == Some(character) => quote = None,
            '>' if quote.is_none() => return Some(index),
            _ => {}
        }
    }
    None
}

fn find_case_insensitive(haystack: &str, needle: &str) -> Option<usize> {
    haystack
        .to_ascii_lowercase()
        .find(&needle.to_ascii_lowercase())
}

fn apply_title_override(html: &str, title: &str) -> String {
    let escaped_title = escape_html_text(title);
    let Some(open_offset) = find_case_insensitive(html, "<title") else {
        if let Some(head_end) = find_case_insensitive(html, "</head>") {
            let mut updated = String::new();
            updated.push_str(&html[..head_end]);
            updated.push_str("<title>");
            updated.push_str(&escaped_title);
            updated.push_str("</title>");
            updated.push_str(&html[head_end..]);
            return updated;
        }
        return html.to_string();
    };

    let Some(open_end) = find_tag_end(&html[open_offset..]) else {
        return html.to_string();
    };
    let content_start = open_offset + open_end + 1;
    let Some(close_offset) = find_case_insensitive(&html[content_start..], "</title>") else {
        return html.to_string();
    };
    let content_end = content_start + close_offset;

    let mut updated = String::new();
    updated.push_str(&html[..content_start]);
    updated.push_str(&escaped_title);
    updated.push_str(&html[content_end..]);
    updated
}

fn escape_html_text(input: &str) -> String {
    input
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn extract_title_text(html: &str) -> Option<String> {
    fn first_text_by_tag(node: &Node, tag_name: &str) -> Option<String> {
        match node {
            Node::Text(_) => None,
            Node::Element(element) => {
                if element.tag_name == tag_name {
                    let mut text = String::new();
                    collect_text(node, &mut text);
                    let normalized = text.split_whitespace().collect::<Vec<_>>().join(" ");
                    if !normalized.trim().is_empty() {
                        return Some(normalized);
                    }
                }

                for child in &element.children {
                    if let Some(found) = first_text_by_tag(child, tag_name) {
                        return Some(found);
                    }
                }

                None
            }
        }
    }

    fn collect_text(node: &Node, output: &mut String) {
        match node {
            Node::Text(text) => {
                output.push_str(text);
                output.push(' ');
            }
            Node::Element(element) => {
                for child in &element.children {
                    collect_text(child, output);
                }
            }
        }
    }

    first_text_by_tag(&parse_document(html), "title")
}

fn js_document_write(_: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    append_written_html(args, "", context)?;
    Ok(JsValue::undefined())
}

fn js_document_writeln(_: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    append_written_html(args, "\n", context)?;
    Ok(JsValue::undefined())
}

fn append_written_html(args: &[JsValue], suffix: &str, context: &mut Context) -> JsResult<()> {
    let mut written = String::new();
    for value in args {
        written.push_str(&js_value_to_string(value, context)?);
    }
    written.push_str(suffix);

    if let Some(host) = context.get_data::<JavaScriptHostData>() {
        host.state.borrow_mut().write_buffer.push_str(&written);
    }

    Ok(())
}

fn js_document_get_title(_: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let title = context
        .get_data::<JavaScriptHostData>()
        .map(|host| host.state.borrow().current_title.clone())
        .unwrap_or_default();
    Ok(JsValue::from(js_string!(title)))
}

fn js_document_set_title(
    _: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let title = js_value_to_string(args.first().unwrap_or(&JsValue::undefined()), context)?;
    if let Some(host) = context.get_data::<JavaScriptHostData>() {
        let mut state = host.state.borrow_mut();
        state.current_title = title;
        state.title_dirty = true;
    }
    Ok(JsValue::undefined())
}

fn js_location_get_href(_: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let href = context
        .get_data::<JavaScriptHostData>()
        .map(|host| host.state.borrow().location_href.clone())
        .unwrap_or_default();
    Ok(JsValue::from(js_string!(href)))
}

fn js_location_set_href(_: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let href = js_value_to_string(args.first().unwrap_or(&JsValue::undefined()), context)?;
    set_location_href(&href, context);
    Ok(JsValue::undefined())
}

fn js_location_get_search(_: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let search = current_location_url(context)
        .map(|url| {
            url.path
                .split_once('?')
                .map(|(_, query)| format!("?{query}"))
                .unwrap_or_default()
        })
        .unwrap_or_default();
    Ok(JsValue::from(js_string!(search)))
}

fn js_location_get_hash(_: &JsValue, _: &[JsValue], _: &mut Context) -> JsResult<JsValue> {
    Ok(JsValue::from(js_string!("")))
}

fn js_location_get_pathname(
    _: &JsValue,
    _: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let pathname = current_location_url(context)
        .map(|url| {
            url.path
                .split(['?', '#'])
                .next()
                .unwrap_or(url.path.as_str())
                .to_string()
        })
        .unwrap_or_else(|| "/".to_string());
    Ok(JsValue::from(js_string!(pathname)))
}

fn js_location_get_origin(_: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let origin = current_location_url(context)
        .map(|url| format!("{}://{}", url.scheme, url.host_header()))
        .unwrap_or_default();
    Ok(JsValue::from(js_string!(origin)))
}

fn js_location_get_host(_: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let host = current_location_url(context)
        .map(|url| url.host_header())
        .unwrap_or_default();
    Ok(JsValue::from(js_string!(host)))
}

fn js_location_get_hostname(
    _: &JsValue,
    _: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let hostname = current_location_url(context)
        .map(|url| url.host)
        .unwrap_or_default();
    Ok(JsValue::from(js_string!(hostname)))
}

fn js_location_get_protocol(
    _: &JsValue,
    _: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let protocol = current_location_url(context)
        .map(|url| format!("{}:", url.scheme))
        .unwrap_or_default();
    Ok(JsValue::from(js_string!(protocol)))
}

fn js_location_assign(_: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let href = js_value_to_string(args.first().unwrap_or(&JsValue::undefined()), context)?;
    set_location_href(&href, context);
    Ok(JsValue::undefined())
}

fn js_location_replace(_: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let href = js_value_to_string(args.first().unwrap_or(&JsValue::undefined()), context)?;
    set_location_href(&href, context);
    Ok(JsValue::undefined())
}

fn set_location_href(href: &str, context: &mut Context) {
    if let Some(host) = context.get_data::<JavaScriptHostData>() {
        host.state.borrow_mut().location_href = href.to_string();
    }
}

fn current_location_url(context: &mut Context) -> Option<Url> {
    let href = context
        .get_data::<JavaScriptHostData>()
        .map(|host| host.state.borrow().location_href.clone())?;
    Url::parse(&href).ok()
}

fn js_console_log(_: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let mut parts = Vec::new();
    for value in args {
        parts.push(js_value_to_string(value, context)?);
    }

    if let Some(host) = context.get_data::<JavaScriptHostData>() {
        host.state.borrow_mut().console_logs.push(parts.join(" "));
    }

    Ok(JsValue::undefined())
}

fn js_request_animation_frame(
    _: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    if let Some(callback) = args.first() {
        call_js_callback(callback, &[JsValue::new(performance_now_ms())], context)?;
    }
    Ok(JsValue::new(1))
}

fn js_queue_microtask(_: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    if let Some(callback) = args.first() {
        call_js_callback(callback, &[], context)?;
    }
    Ok(JsValue::undefined())
}

fn js_set_timeout(_: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    if let Some(callback) = args.first() {
        if callback.as_object().is_some() {
            call_js_callback(callback, &[], context)?;
        } else if callback.is_string() {
            let script = js_value_to_string(callback, context)?;
            let _ = context.eval(Source::from_bytes(script.as_str()));
        }
    }

    Ok(JsValue::new(0))
}

fn js_clear_timeout(_: &JsValue, _: &[JsValue], _: &mut Context) -> JsResult<JsValue> {
    Ok(JsValue::undefined())
}

fn js_add_event_listener(
    _: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let event_name = args
        .first()
        .map(|value| js_value_to_string(value, context))
        .transpose()?
        .unwrap_or_default()
        .to_ascii_lowercase();
    if matches!(
        event_name.as_str(),
        "load" | "domcontentloaded" | "readystatechange" | "script-load-dpj"
    ) && let Some(callback) = args.get(1)
    {
        call_js_callback(callback, &[], context)?;
    }

    Ok(JsValue::undefined())
}

fn js_alert(_: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    js_console_log(&JsValue::undefined(), args, context)?;
    Ok(JsValue::undefined())
}

fn js_confirm(_: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    js_console_log(&JsValue::undefined(), args, context)?;
    Ok(JsValue::new(false))
}

fn js_prompt(_: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    js_console_log(&JsValue::undefined(), args, context)?;
    Ok(JsValue::null())
}

fn js_match_media(_: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let media = js_value_to_string(args.first().unwrap_or(&JsValue::undefined()), context)?;
    Ok(JsValue::from(build_match_media_stub(context, media)))
}

fn js_performance_now(_: &JsValue, _: &[JsValue], _: &mut Context) -> JsResult<JsValue> {
    Ok(JsValue::new(performance_now_ms()))
}

fn js_return_dom_stub(_: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    Ok(JsValue::from(build_dom_stub(context)))
}

fn js_return_node_list_stub(
    _: &JsValue,
    _: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    Ok(JsValue::from(build_node_list_stub(context)))
}

fn js_return_dom_rect_stub(_: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    Ok(JsValue::from(build_dom_rect_stub(context)))
}

fn js_get_video_aspect_ratio(_: &JsValue, _: &[JsValue], _: &mut Context) -> JsResult<JsValue> {
    Ok(JsValue::new(16.0 / 9.0))
}

fn js_return_false(_: &JsValue, _: &[JsValue], _: &mut Context) -> JsResult<JsValue> {
    Ok(JsValue::new(false))
}

fn js_return_null(_: &JsValue, _: &[JsValue], _: &mut Context) -> JsResult<JsValue> {
    Ok(JsValue::null())
}

fn js_return_undefined(_: &JsValue, _: &[JsValue], _: &mut Context) -> JsResult<JsValue> {
    Ok(JsValue::undefined())
}

fn js_noop(_: &JsValue, _: &[JsValue], _: &mut Context) -> JsResult<JsValue> {
    Ok(JsValue::undefined())
}

fn js_ytcfg_data(_: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    Ok(JsValue::from(build_dom_stub(context)))
}

fn js_ytcfg_get(_: &JsValue, args: &[JsValue], _: &mut Context) -> JsResult<JsValue> {
    Ok(args.get(1).cloned().unwrap_or_else(JsValue::undefined))
}

fn call_js_callback(
    callback: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    if let Some(object) = callback.as_object()
        && let Some(function) = JsFunction::from_object(object.clone())
    {
        return function.call(&JsValue::undefined(), args, context);
    }

    Ok(JsValue::undefined())
}

fn performance_now_ms() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs_f64() * 1000.0)
        .unwrap_or(0.0)
}

fn js_value_to_string(value: &JsValue, context: &mut Context) -> JsResult<String> {
    Ok(value.to_string(context)?.to_std_string_escaped())
}

#[cfg(test)]
mod tests {
    use super::process_document_scripts;
    use crate::url::Url;

    #[test]
    fn expands_document_write_output() {
        let processed = process_document_scripts(
            "<html><body><script>document.write('<p>Hello</p>')</script></body></html>",
            &Url::parse("https://example.com").unwrap(),
        );

        assert!(processed.html.contains("<p>Hello</p>"));
        assert!(!processed.html.contains("document.write"));
    }

    #[test]
    fn updates_document_title_from_script() {
        let processed = process_document_scripts(
            "<html><head><title>Demo</title><script>document.title = 'Changed'</script></head><body></body></html>",
            &Url::parse("https://example.com").unwrap(),
        );

        assert_eq!(processed.title_override.as_deref(), Some("Changed"));
        assert!(processed.html.contains("<title>Changed</title>"));
    }

    #[test]
    fn executes_script_written_scripts_recursively() {
        let processed = process_document_scripts(
            "<script>document.write('<script>document.write(\"<p>Nested</p>\")</' + 'script>')</script>",
            &Url::parse("https://example.com").unwrap(),
        );

        assert!(processed.html.contains("<p>Nested</p>"));
    }

    #[test]
    fn skips_non_javascript_script_types() {
        let processed = process_document_scripts(
            "<script type=\"application/ld+json\">{\"name\":\"demo\"}</script>",
            &Url::parse("https://example.com").unwrap(),
        );

        assert!(processed.html.contains("application/ld+json"));
    }

    #[test]
    fn runs_set_timeout_callbacks_immediately() {
        let processed = process_document_scripts(
            "<script>setTimeout(function () { document.write('<p>Later</p>'); }, 1);</script>",
            &Url::parse("https://example.com").unwrap(),
        );

        assert!(processed.html.contains("<p>Later</p>"));
    }

    #[test]
    fn skips_large_scripts_even_if_they_reference_supported_apis() {
        let large_script = format!(
            "<script>{}document.write('<p>Nope</p>')</script>",
            "x".repeat(super::MAX_SCRIPT_SOURCE_BYTES)
        );
        let processed =
            process_document_scripts(&large_script, &Url::parse("https://example.com").unwrap());

        assert!(!processed.html.contains("<p>Nope</p>"));
        assert!(
            processed
                .console_logs
                .iter()
                .any(|entry| entry.contains("unsupported script pattern"))
        );
    }

    #[test]
    fn skips_scripts_that_touch_unsupported_dom_apis() {
        let processed = process_document_scripts(
            "<script>fetch('/api/demo').then(function(){ document.write('<p>Nope</p>'); });</script>",
            &Url::parse("https://example.com").unwrap(),
        );

        assert!(!processed.html.contains("<p>Nope</p>"));
        assert!(
            processed
                .console_logs
                .iter()
                .any(|entry| entry.contains("unsupported script pattern"))
        );
    }

    #[test]
    fn supports_lightweight_dom_like_scripts() {
        let processed = process_document_scripts(
            "<script>document.addEventListener('DOMContentLoaded', function () { var el = document.querySelector('#demo'); if (el) { document.write('<p>Ready</p>'); } });</script>",
            &Url::parse("https://example.com").unwrap(),
        );

        assert!(processed.html.contains("<p>Ready</p>"));
    }
}
