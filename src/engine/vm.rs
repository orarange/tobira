use std::{
    cell::RefCell,
    cmp::{Ordering, Reverse},
    collections::{HashMap, VecDeque},
    rc::Rc,
    time::{SystemTime, UNIX_EPOCH},
};

use serde_json::Value as JsonValue;

use super::js_regex::{JsCaptures, JsRegex};
use super::chunk::{Chunk, Constant, FunctionProto, Opcode};
use super::event_loop::{
    EventLoop, MicrotaskJob, RafEntry, TaskEntry, TaskSource, TickResult, TimerEntry,
};
use super::heap::{GcRef, Heap, RawGcRef};
use super::host::{
    AdjacentPosition,
    ConsoleLevel, ConsoleMessage, DomMutation, DomMutationResult, DomRead, DomReadResult, FetchBody,
    FetchMode,
    FetchRequest, FetchResponse, HistoryAction, Host, HostData, HttpMethod, NavigationAction,
    NodeId, NodeKind, NoopHost, ObserverId, ObserverKind, ObserverOp, ObserverOptions,
    ObserverRecord, ObserverResult, SiblingDirection, StorageAreaKind, StorageAreaScope,
    StorageOp, StorageResult, WindowId,
};
use super::value::{
    AsyncContext, AsyncGeneratorRequest, GeneratorState, HostDispatch, HostObjectClass,
    HostObjectSlot, JsObject, JsPropertyDescriptor, JsString, ObjectKind, PromiseReaction,
    PromiseState, PropertyKey, SymbolId, TypedArrayKind, Value,
};

type ValueCell = Rc<RefCell<Value>>;

/// Details for a host-dispatched DOM event (a real user interaction, as opposed
/// to a script's `new Event()`). Lets the host pass keyboard/pointer/input
/// fields so listeners can read `event.key`, `event.data`, modifiers, etc.
#[derive(Debug, Clone, Default)]
pub struct DomEventInit {
    pub bubbles: bool,
    pub cancelable: bool,
    pub key: Option<String>,
    pub code: Option<String>,
    pub data: Option<String>,
    pub input_type: Option<String>,
    pub client_x: Option<i32>,
    pub client_y: Option<i32>,
    pub button: Option<i32>,
    pub buttons: Option<i32>,
    pub alt_key: bool,
    pub ctrl_key: bool,
    pub shift_key: bool,
    pub meta_key: bool,
    pub repeat: bool,
    pub is_composing: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum BuiltinId {
    Assert,
    Noop,
    CallSpread,
    ConstructSpread,
    PromiseConstructor,
    PromiseResolve,
    PromiseReject,
    PromiseAll,
    PromiseRace,
    PromiseAllSettled,
    PromiseAny,
    PromiseProtoThen,
    PromiseProtoCatch,
    PromiseProtoFinally,
    QueueMicrotask,
    SetTimeout,
    ClearTimeout,
    SetInterval,
    ClearInterval,
    RequestAnimationFrame,
    CancelAnimationFrame,
    ObjectConstructor,
    ObjectCreate,
    ObjectDefineProperty,
    ObjectGetOwnPropertyDescriptor,
    ObjectKeys,
    ObjectGetOwnPropertySymbols,
    ObjectValues,
    ObjectEntries,
    ObjectAssign,
    ObjectGetPrototypeOf,
    ObjectSetPrototypeOf,
    ObjectFreeze,
    ObjectIsFrozen,
    ObjectProtoHasOwnProperty,
    ObjectProtoPropertyIsEnumerable,
    ObjectProtoToString,
    ObjectProtoValueOf,
    ObjectProtoIsPrototypeOf,
    FunctionProtoCall,
    FunctionProtoApply,
    FunctionProtoBind,
    FunctionConstructor,
    ErrorConstructor,
    TypeErrorConstructor,
    RangeErrorConstructor,
    ReferenceErrorConstructor,
    SyntaxErrorConstructor,
    UriErrorConstructor,
    EvalErrorConstructor,
    ArrayConstructor,
    ArrayIsArray,
    ArrayFrom,
    GlobalEscape,
    GlobalUnescape,
    ArrayProtoPush,
    ArrayProtoPop,
    ArrayProtoShift,
    ArrayProtoUnshift,
    ArrayProtoMap,
    ArrayProtoFilter,
    ArrayProtoReduce,
    ArrayProtoForEach,
    ArrayProtoFind,
    ArrayProtoFindIndex,
    ArrayProtoIndexOf,
    ArrayProtoLastIndexOf,
    ArrayProtoIncludes,
    ArrayProtoJoin,
    ArrayProtoSlice,
    ArrayProtoConcat,
    ArrayProtoFlat,
    ArrayProtoSome,
    ArrayProtoEvery,
    ArrayProtoSort,
    ArrayProtoReverse,
    StringProtoCharAt,
    StringProtoCharCodeAt,
    StringProtoCodePointAt,
    StringProtoIndexOf,
    StringProtoLastIndexOf,
    StringProtoIncludes,
    StringProtoStartsWith,
    StringProtoEndsWith,
    StringProtoSlice,
    StringProtoSubstring,
    StringProtoSplit,
    StringProtoReplace,
    StringProtoReplaceAll,
    StringProtoTrim,
    StringProtoTrimStart,
    StringProtoTrimEnd,
    StringProtoToUpperCase,
    StringProtoToLowerCase,
    StringProtoPadStart,
    StringProtoPadEnd,
    StringProtoRepeat,
    NumberIsNaN,
    NumberIsFinite,
    NumberIsInteger,
    NumberIsSafeInteger,
    NumberParseInt,
    NumberParseFloat,
    MathFloor,
    MathCeil,
    MathRound,
    MathTrunc,
    MathAbs,
    MathMin,
    MathMax,
    MathPow,
    MathSqrt,
    MathCbrt,
    MathExpm1,
    MathFround,
    MathSin,
    MathCos,
    MathTan,
    MathSinh,
    MathCosh,
    MathTanh,
    MathAsin,
    MathAcos,
    MathAtan,
    MathAsinh,
    MathAcosh,
    MathAtanh,
    MathAtan2,
    MathLog,
    MathLog2,
    MathLog10,
    MathExp,
    MathRandom,
    CryptoGetRandomValues,
    CryptoRandomUUID,
    TextEncoderConstructor,
    TextEncoderEncode,
    TextDecoderConstructor,
    TextDecoderDecode,
    WeakRefConstructor,
    WeakRefDeref,
    JsonStringify,
    JsonParse,
    ConsoleLog,
    ConsoleInfo,
    ConsoleWarn,
    ConsoleError,
    ModuleReexportAll,
    // DOM Document methods
    DomDocQuerySelector,
    DomDocQuerySelectorAll,
    DomDocGetElementById,
    DomDocGetElementsByClassName,
    DomDocGetElementsByTagName,
    DomDocCreateElement,
    DomDocCreateTextNode,
    DomDocCreateFragment,
    DomDocWrite,
    // DOM Node/Element methods
    DomNodeAppendChild,
    DomNodeInsertBefore,
    DomNodePrepend,
    DomNodeHasChildNodes,
    DomCreateElementNs,
    DomNodeRemoveChild,
    DomNodeReplaceChild,
    DomNodeCloneNode,
    DomNodeRemove,
    DomNodeSetAttribute,
    DomNodeGetAttribute,
    DomNodeRemoveAttribute,
    DomNodeHasAttribute,
    ElementStubGetAttribute,
    ElementStubHasAttribute,
    DomNodeToggleAttribute,
    DomNodeGetAttributeNames,
    DomNodeQuerySelector,
    DomNodeQuerySelectorAll,
    DomNodeClosest,
    DomNodeMatches,
    DomNodeContains,
    DomNodeGetBoundingClientRect,
    DomNodeScrollIntoView,
    DomNodeFocus,
    DomNodeBlur,
    DomNodeClick,
    DomNodeAddEventListener,
    DomNodeRemoveEventListener,
    DomNodeDispatchEvent,
    // Event / CustomEvent
    EventConstructor,
    CustomEventConstructor,
    KeyboardEventConstructor,
    MouseEventConstructor,
    // Custom elements
    CustomElementsDefine,
    CustomElementsGet,
    // AbortController / AbortSignal
    AbortControllerConstructor,
    AbortControllerAbort,
    AbortSignalAddEventListener,
    AbortSignalRemoveEventListener,
    AbortSignalThrowIfAborted,
    AbortSignalConstructor,
    AbortSignalAbortStatic,
    AbortSignalTimeoutStatic,
    AbortSignalAnyStatic,
    EventPreventDefault,
    EventStopPropagation,
    EventStopImmediatePropagation,
    // fetch / Response / Headers
    Fetch,
    ResponseText,
    ResponseJson,
    // MutationObserver
    MutationObserverConstructor,
    MutationObserverObserve,
    MutationObserverDisconnect,
    MutationObserverTakeRecords,
    // IntersectionObserver (observe/disconnect/takeRecords reuse the kind-agnostic
    // MutationObserver builtins above)
    IntersectionObserverConstructor,
    IntersectionObserverUnobserve,
    // ResizeObserver (observe/disconnect/takeRecords reuse the kind-agnostic
    // MutationObserver builtins above)
    ResizeObserverConstructor,
    ResizeObserverUnobserve,
    // XMLHttpRequest / Image
    ImageConstructor,
    XhrConstructor,
    XhrOpen,
    XhrSetRequestHeader,
    XhrSend,
    XhrAbort,
    XhrGetAllResponseHeaders,
    XhrGetResponseHeader,
    // classList (TokenList)
    DomClassListAdd,
    DomClassListRemove,
    DomClassListContains,
    DomClassListToggle,
    DomClassListReplace,
    DomClassListItem,
    DomClassListToString,
    DomNodeInsertAdjacentHtml,
    DomNodeReplaceChildren,
    DomNodeSplitText,
    // Shadow DOM
    DomNodeAttachShadow,
    DomNodeGetRootNode,
    DomSlotAssignedNodes,
    DomSlotAssignedElements,
    DomEventComposedPath,
    // attributes (NamedNodeMap)
    DomAttrMapItem,
    DomAttrMapGetNamedItem,
    DomNodeHasAttributes,
    // style (CSSStyleDeclaration)
    DomStyleGetProperty,
    DomStyleSetProperty,
    DomStyleRemoveProperty,
    // getComputedStyle snapshot
    DomComputedStyleGetProperty,
    DomComputedStyleGetPriority,
    // history
    HistoryPushState,
    HistoryReplaceState,
    HistoryBack,
    HistoryForward,
    HistoryGo,
    // performance, idle, encoding
    PerformanceNow,
    PerformanceMark,
    PerformanceMeasure,
    PerformanceClearMarks,
    PerformanceClearMeasures,
    PerformanceGetEntries,
    PerformanceGetEntriesByName,
    PerformanceGetEntriesByType,
    RequestIdleCallback,
    CancelIdleCallback,
    Btoa,
    Atob,
    // localStorage / sessionStorage methods
    StorageGetItem,
    StorageSetItem,
    StorageRemoveItem,
    StorageClear,
    StorageKey,
    // Window
    WindowScrollTo,
    WindowScrollBy,
    WindowGetComputedStyle,
    WindowMatchMedia,
    MapConstructor,
    MapProtoSet,
    MapProtoGet,
    MapProtoHas,
    MapProtoDelete,
    MapProtoClear,
    MapProtoForEach,
    MapProtoEntries,
    MapProtoKeys,
    MapProtoValues,
    SetConstructor,
    SetProtoAdd,
    SetProtoHas,
    SetProtoDelete,
    SetProtoClear,
    SetProtoForEach,
    SetProtoValues,
    // ArrayBuffer + typed arrays.
    ArrayBufferConstructor,
    ArrayBufferProtoSlice,
    TypedArrayConstructor(TypedArrayKind),
    TypedArrayFrom(TypedArrayKind),
    TypedArrayOf(TypedArrayKind),
    TypedArrayProtoSet,
    TypedArrayProtoSubarray,
    TypedArrayProtoSlice,
    TypedArrayProtoFill,
    TypedArrayProtoJoin,
    TypedArrayProtoIndexOf,
    TypedArrayProtoIncludes,
    TypedArrayProtoForEach,
    TypedArrayProtoMap,
    TypedArrayProtoReduce,
    TypedArrayProtoReverse,
    // Added in the feature-completeness pass.
    ArrayProtoSplice,
    ArrayProtoFlatMap,
    ArrayProtoFill,
    ArrayProtoCopyWithin,
    ArrayProtoAt,
    ArrayProtoKeys,
    ArrayProtoValues,
    ArrayProtoEntries,
    ArrayProtoReduceRight,
    ArrayProtoFindLast,
    ArrayProtoFindLastIndex,
    ArrayOf,
    StringProtoAt,
    StringProtoNormalize,
    StringProtoConcat,
    StringConstructor,
    StringFromCharCode,
    StringFromCodePoint,
    StringRaw,
    NumberConstructor,
    NumberProtoToFixed,
    NumberProtoToString,
    NumberProtoToPrecision,
    NumberProtoValueOf,
    BooleanConstructor,
    BooleanProtoToString,
    BooleanProtoValueOf,
    ObjectFromEntries,
    ObjectGetOwnPropertyNames,
    ObjectHasOwn,
    ObjectPreventExtensions,
    ObjectIsExtensible,
    ObjectSeal,
    ObjectIsSealed,
    MathSign,
    MathHypot,
    MathClz32,
    GlobalParseInt,
    GlobalParseFloat,
    GlobalIsNaN,
    GlobalIsFinite,
    EncodeUriComponent,
    DecodeUriComponent,
    EncodeUri,
    DecodeUri,
    RegExpConstructor,
    RegExpProtoTest,
    RegExpProtoExec,
    RegExpProtoToString,
    StringProtoMatch,
    StringProtoMatchAll,
    StringProtoSearch,
    SymbolConstructor,
    SymbolProtoToString,
    DateConstructor,
    DateNow,
    DateUTC,
    DateParse,
    DateProtoGetTime,
    DateProtoGetFullYear,
    DateProtoGetMonth,
    DateProtoGetDate,
    DateProtoGetDay,
    DateProtoGetHours,
    DateProtoGetMinutes,
    DateProtoGetSeconds,
    DateProtoGetMilliseconds,
    DateProtoGetTimezoneOffset,
    DateProtoToISOString,
    DateProtoToString,
    DateProtoValueOf,
    GeneratorProtoNext,
    GeneratorProtoReturn,
    GeneratorProtoIterator,
    ForOfIteratorAdapterNext,
    AsyncGeneratorProtoNext,
    AsyncGeneratorProtoReturn,
    AsyncGeneratorProtoIterator,
    ArrayProtoToSorted,
    ArrayProtoToReversed,
    ArrayProtoWith,
    StringProtoLocaleCompare,
    ObjectGetOwnPropertyDescriptors,
    ObjectDefineProperties,
    ObjectIs,
    NumberProtoToLocaleString,
    SymbolFor,
    SymbolKeyFor,
    WeakMapConstructor,
    WeakSetConstructor,
    ReflectGet,
    ReflectSet,
    ReflectHas,
    ReflectDeleteProperty,
    ReflectOwnKeys,
    ReflectGetPrototypeOf,
    ReflectDefineProperty,
    ReflectApply,
    ReflectConstruct,
    StructuredClone,
    ProxyConstructor,
    UrlSearchParamsConstructor,
    HeadersConstructor,
    HeadersGet,
    HeadersSet,
    HeadersHas,
    HeadersAppend,
    HeadersDelete,
    HeadersForEach,
    HeadersEntries,
    HeadersKeys,
    HeadersValues,
    FormDataConstructor,
    FormDataGet,
    FormDataGetAll,
    FormDataHas,
    FormDataSet,
    FormDataAppend,
    FormDataDelete,
    FormDataForEach,
    FormDataEntries,
    FormDataKeys,
    FormDataValues,
    UrlConstructor,
    UrlToString,
    UrlToPrimitive,
    UspGet,
    UspGetAll,
    UspHas,
    UspSet,
    UspAppend,
    UspDelete,
    UspToString,
    UspForEach,
    UspEntries,
    UspKeys,
    UspValues,
    UspSort,
}

#[derive(Debug, Clone)]
struct RuntimeClosure {
    proto: Rc<FunctionProto>,
    upvalues: Vec<ValueCell>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PromiseCapabilityMode {
    Resolve,
    Reject,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PromiseFinallyMode {
    Fulfill,
    Reject,
}

#[derive(Debug, Clone)]
struct PromiseAllState {
    result_promise: GcRef<JsObject>,
    values: Rc<RefCell<Vec<Option<Value>>>>,
    remaining: Rc<RefCell<usize>>,
}

#[derive(Debug, Clone)]
struct PromiseAllResolveElement {
    state: PromiseAllState,
    index: usize,
}

#[derive(Debug, Clone)]
struct PromiseAllSettledElement {
    state: PromiseAllState,
    index: usize,
    is_reject: bool,
}

#[derive(Debug, Clone)]
struct PromiseAnyRejectElement {
    result_promise: GcRef<JsObject>,
    errors: Rc<RefCell<Vec<Option<Value>>>>,
    remaining: Rc<RefCell<usize>>,
    index: usize,
}

#[derive(Debug, Clone)]
struct BoundFunction {
    target: Value,
    bound_this: Value,
    bound_args: Vec<Value>,
}

#[derive(Debug, Clone)]
enum Callable {
    Builtin(BuiltinId),
    Closure(RuntimeClosure),
    Bound(BoundFunction),
    PromiseCapability {
        promise: GcRef<JsObject>,
        mode: PromiseCapabilityMode,
    },
    PromiseFinally {
        callback: Value,
        mode: PromiseFinallyMode,
    },
    PromiseAllResolveElement(PromiseAllResolveElement),
    PromiseAllReject {
        result_promise: GcRef<JsObject>,
    },
    PromiseRaceResolve {
        result_promise: GcRef<JsObject>,
    },
    PromiseRaceReject {
        result_promise: GcRef<JsObject>,
    },
    PromiseAllSettledElement(PromiseAllSettledElement),
    PromiseAnyResolve {
        result_promise: GcRef<JsObject>,
    },
    PromiseAnyRejectElement(PromiseAnyRejectElement),
}

#[derive(Debug, Clone, PartialEq)]
pub struct CallFrame {
    proto: Rc<FunctionProto>,
    ip: usize,
    stack_base: usize,
    locals: Vec<ValueCell>,
    upvalues: Vec<ValueCell>,
    this_value: Value,
    construct_fallback: Option<Value>,
    pending_exception: Option<Value>,
    async_outer_promise: Option<GcRef<JsObject>>,
    async_gen_request: Option<GcRef<JsObject>>,
    /// The generator object that owns this frame, if it is a generator body.
    generator: Option<GcRef<JsObject>>,
    /// Call arguments retained for the `arguments` object (only when the function
    /// references it).
    arguments: Vec<Value>,
    /// `new.target` — the constructor when invoked via `new`, else undefined.
    new_target: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub enum VmError {
    TypeError(String),
    ReferenceError(String),
    RangeError(String),
    Thrown(Value),
    InfiniteLoop,
    StackOverflow,
    Unimplemented(&'static str),
}

impl std::fmt::Display for VmError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TypeError(message)
            | Self::ReferenceError(message)
            | Self::RangeError(message) => write!(f, "{message}"),
            Self::Thrown(value) => write!(f, "uncaught throw: {value:?}"),
            Self::InfiniteLoop => write!(f, "Maximum loop iteration limit exceeded"),
            Self::StackOverflow => write!(f, "Maximum call stack size exceeded"),
            Self::Unimplemented(feature) => write!(f, "unimplemented in phase 3: {feature}"),
        }
    }
}

impl std::error::Error for VmError {}

/// ECMAScript `parseInt(string, radix)`: skip leading whitespace, accept an
/// optional sign and `0x` prefix, then consume the longest valid digit run.
fn js_parse_int(text: &str, radix: Option<f64>) -> f64 {
    let trimmed = text.trim_start();
    let mut chars = trimmed.chars().peekable();
    let mut sign = 1.0;
    match chars.peek() {
        Some('+') => {
            chars.next();
        }
        Some('-') => {
            sign = -1.0;
            chars.next();
        }
        _ => {}
    }

    let mut radix = match radix {
        Some(r) if r.is_finite() => r as i64,
        _ => 0,
    };
    let rest: String = chars.collect();
    let mut digits = rest.as_str();
    if radix == 16 || radix == 0 {
        if let Some(stripped) = digits.strip_prefix("0x").or_else(|| digits.strip_prefix("0X")) {
            digits = stripped;
            radix = 16;
        }
    }
    if radix == 0 {
        radix = 10;
    }
    if !(2..=36).contains(&radix) {
        return f64::NAN;
    }

    let mut value = 0.0_f64;
    let mut consumed = 0usize;
    for ch in digits.chars() {
        let digit = match ch.to_digit(radix as u32) {
            Some(d) => d,
            None => break,
        };
        value = value * radix as f64 + digit as f64;
        consumed += 1;
    }
    if consumed == 0 {
        return f64::NAN;
    }
    sign * value
}

/// ECMAScript `parseFloat(string)`: consume the longest leading decimal-float
/// prefix (including `Infinity`), ignoring trailing junk.
fn js_parse_float(text: &str) -> f64 {
    let trimmed = text.trim_start();
    if trimmed.starts_with("Infinity") || trimmed.starts_with("+Infinity") {
        return f64::INFINITY;
    }
    if trimmed.starts_with("-Infinity") {
        return f64::NEG_INFINITY;
    }
    let bytes = trimmed.as_bytes();
    let mut end = 0usize;
    let mut seen_dot = false;
    let mut seen_exp = false;
    let mut seen_digit = false;
    while end < bytes.len() {
        let c = bytes[end];
        match c {
            b'+' | b'-' if end == 0 => {}
            b'+' | b'-' if end > 0 && (bytes[end - 1] == b'e' || bytes[end - 1] == b'E') => {}
            b'0'..=b'9' => seen_digit = true,
            b'.' if !seen_dot && !seen_exp => seen_dot = true,
            b'e' | b'E' if !seen_exp && seen_digit => seen_exp = true,
            _ => break,
        }
        end += 1;
    }
    if !seen_digit {
        return f64::NAN;
    }
    trimmed[..end].parse::<f64>().unwrap_or(f64::NAN)
}

/// Clamp a relative array index (negative counts from the end) into `0..=len`.
fn relative_index(value: Option<f64>, len: usize) -> usize {
    match value {
        None => 0,
        Some(n) if n.is_nan() => 0,
        Some(n) if n < 0.0 => (len as f64 + n).max(0.0) as usize,
        Some(n) => (n as usize).min(len),
    }
}

/// Number.prototype.toString(radix) for non-decimal radixes (2..=36).
fn number_to_radix_string(number: f64, radix: u32) -> String {
    if number.is_nan() {
        return "NaN".to_string();
    }
    if !number.is_finite() {
        return if number > 0.0 { "Infinity" } else { "-Infinity" }.to_string();
    }
    if !(2..=36).contains(&radix) {
        return number.to_string();
    }
    let digits = b"0123456789abcdefghijklmnopqrstuvwxyz";
    let negative = number < 0.0;
    let mut int_part = number.abs().trunc() as u64;
    let mut frac = number.abs().fract();
    let mut int_bytes = Vec::new();
    if int_part == 0 {
        int_bytes.push(b'0');
    }
    while int_part > 0 {
        int_bytes.push(digits[(int_part % radix as u64) as usize]);
        int_part /= radix as u64;
    }
    int_bytes.reverse();
    let mut out = String::from_utf8(int_bytes).unwrap_or_default();
    if frac > 0.0 {
        out.push('.');
        let mut count = 0;
        while frac > 0.0 && count < 20 {
            frac *= radix as f64;
            let digit = frac.trunc() as usize;
            out.push(digits[digit] as char);
            frac -= digit as f64;
            count += 1;
        }
    }
    if negative {
        format!("-{out}")
    } else {
        out
    }
}

/// Number.prototype.toPrecision(p): format with `p` significant digits.
fn number_to_precision(number: f64, precision: usize) -> String {
    if number == 0.0 {
        return format!("{:.*}", precision.saturating_sub(1), 0.0);
    }
    if !number.is_finite() {
        return if number.is_nan() {
            "NaN".to_string()
        } else if number > 0.0 {
            "Infinity".to_string()
        } else {
            "-Infinity".to_string()
        };
    }
    let exponent = number.abs().log10().floor() as i32;
    let decimals = (precision as i32 - 1 - exponent).max(0) as usize;
    format!("{number:.decimals$}")
}

/// JSON.stringify pretty-printer with a custom indent string. serde_json's
/// pretty formatter is hard-coded to two spaces, so this renders by hand to
/// honor the third `space` argument.
fn json_to_pretty_string(value: &JsonValue, indent: &str, depth: usize) -> String {
    match value {
        JsonValue::Null => "null".to_string(),
        JsonValue::Bool(boolean) => boolean.to_string(),
        JsonValue::Number(number) => number.to_string(),
        JsonValue::String(_) => {
            serde_json::to_string(value).unwrap_or_else(|_| "\"\"".to_string())
        }
        JsonValue::Array(items) => {
            if items.is_empty() {
                return "[]".to_string();
            }
            let inner = indent.repeat(depth + 1);
            let outer = indent.repeat(depth);
            let parts: Vec<String> = items
                .iter()
                .map(|item| format!("{inner}{}", json_to_pretty_string(item, indent, depth + 1)))
                .collect();
            format!("[\n{}\n{outer}]", parts.join(",\n"))
        }
        JsonValue::Object(map) => {
            if map.is_empty() {
                return "{}".to_string();
            }
            let inner = indent.repeat(depth + 1);
            let outer = indent.repeat(depth);
            let parts: Vec<String> = map
                .iter()
                .map(|(key, value)| {
                    let key_json = serde_json::to_string(&JsonValue::String(key.clone()))
                        .unwrap_or_else(|_| format!("\"{key}\""));
                    format!(
                        "{inner}{key_json}: {}",
                        json_to_pretty_string(value, indent, depth + 1)
                    )
                })
                .collect();
            format!("{{\n{}\n{outer}}}", parts.join(",\n"))
        }
    }
}

/// Translate JS named capture groups `(?<name>...)` into the Rust regex crate's
/// `(?P<name>...)` syntax, leaving lookbehind `(?<=` / `(?<!` untouched.
/// Expand a regex replacement template: `$$`→`$`, `$&`→whole match, `` $` ``→
/// prefix, `$'`→suffix, `$<name>`→named group, `$1`..`$99`→numbered group.
fn expand_replacement(template: &str, caps: &JsCaptures, full_text: &str) -> String {
    let mut out = String::new();
    let mut chars = template.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '$' {
            out.push(c);
            continue;
        }
        match chars.peek().copied() {
            Some('$') => {
                out.push('$');
                chars.next();
            }
            Some('&') => {
                out.push_str(caps.get(0).map(|m| m.as_str()).unwrap_or(""));
                chars.next();
            }
            Some('`') => {
                let start = caps.get(0).map(|m| m.start()).unwrap_or(0);
                out.push_str(&full_text[..start]);
                chars.next();
            }
            Some('\'') => {
                let end = caps.get(0).map(|m| m.end()).unwrap_or(0);
                out.push_str(&full_text[end..]);
                chars.next();
            }
            Some('<') => {
                chars.next();
                let mut name = String::new();
                while let Some(&nc) = chars.peek() {
                    chars.next();
                    if nc == '>' {
                        break;
                    }
                    name.push(nc);
                }
                out.push_str(caps.name(&name).map(|m| m.as_str()).unwrap_or(""));
            }
            Some(d) if d.is_ascii_digit() => {
                chars.next();
                let mut num = d.to_digit(10).unwrap() as usize;
                if let Some(&d2) = chars.peek() {
                    if d2.is_ascii_digit() {
                        let two = num * 10 + d2.to_digit(10).unwrap() as usize;
                        if two < caps.len() {
                            num = two;
                            chars.next();
                        }
                    }
                }
                if num >= 1 && num < caps.len() {
                    out.push_str(caps.get(num).map(|m| m.as_str()).unwrap_or(""));
                } else {
                    out.push('$');
                    out.push(d);
                }
            }
            _ => out.push('$'),
        }
    }
    out
}

/// Expand a plain-string replacement template (only `$&` and `$$` are special).
fn expand_string_replacement(template: &str, matched: &str) -> String {
    let mut out = String::new();
    let mut chars = template.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '$' {
            match chars.peek().copied() {
                Some('$') => {
                    out.push('$');
                    chars.next();
                }
                Some('&') => {
                    out.push_str(matched);
                    chars.next();
                }
                _ => out.push('$'),
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// JS の正規表現 source + flags を JS 互換エンジン(regress)でコンパイルする。
/// `g`/`y` フラグは呼び出し側(グローバル反復/sticky)で処理する。
fn compile_js_regex(source: &str, flags: &str) -> Result<JsRegex, VmError> {
    JsRegex::compile(source, flags)
        .map_err(|error| VmError::TypeError(format!("invalid regular expression: {error}")))
}

// --- Proleptic Gregorian calendar math (Howard Hinnant's algorithms) --------

/// Days since 1970-01-01 for a given civil date. `m` is 1..=12.
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = (if y >= 0 { y } else { y - 399 }) / 400;
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

/// Civil date (year, month 1..=12, day) from days since 1970-01-01.
fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719468;
    let era = (if z >= 0 { z } else { z - 146096 }) / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// Euclidean modulo so day-of-week/time fields are correct for negative epochs.
fn floor_div(a: i64, b: i64) -> i64 {
    (a as f64 / b as f64).floor() as i64
}

fn floor_mod(a: i64, b: i64) -> i64 {
    ((a % b) + b) % b
}

fn normalize_utc_month(year: i64, month0: i64) -> (i64, i64) {
    let year = year + floor_div(month0, 12);
    let month0 = floor_mod(month0, 12);
    (year, month0)
}

/// `application/x-www-form-urlencoded` decode: `+` → space, `%XX` → byte.
fn form_urldecode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b'%' if i + 2 < bytes.len() => {
                let hi = (bytes[i + 1] as char).to_digit(16);
                let lo = (bytes[i + 2] as char).to_digit(16);
                match (hi, lo) {
                    (Some(h), Some(l)) => {
                        out.push((h * 16 + l) as u8);
                        i += 3;
                    }
                    _ => {
                        out.push(bytes[i]);
                        i += 1;
                    }
                }
            }
            other => {
                out.push(other);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// `application/x-www-form-urlencoded` encode: space → `+`, keep `*-._` and
/// alphanumerics, percent-encode the rest.
fn form_urlencode(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for &byte in input.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'*' | b'-' | b'.' | b'_' => {
                out.push(byte as char);
            }
            b' ' => out.push('+'),
            _ => {
                out.push('%');
                out.push_str(&format!("{byte:02X}"));
            }
        }
    }
    out
}

/// Parse a `URLSearchParams` init query string into (name, value) pairs.
fn parse_query_string(input: &str) -> Vec<(String, String)> {
    let input = input.strip_prefix('?').unwrap_or(input);
    let mut pairs = Vec::new();
    for part in input.split('&') {
        if part.is_empty() {
            continue;
        }
        let (name, value) = match part.split_once('=') {
            Some((name, value)) => (name, value),
            None => (part, ""),
        };
        pairs.push((form_urldecode(name), form_urldecode(value)));
    }
    pairs
}

#[derive(Debug, Clone)]
struct UrlComponents {
    href: String,
    protocol: String,
    username: String,
    password: String,
    host: String,
    hostname: String,
    port: String,
    pathname: String,
    search: String,
    hash: String,
    origin: String,
}

fn trim_url_input(input: &str) -> &str {
    input.trim_matches(|c: char| c.is_ascii_whitespace() || c.is_ascii_control())
}

fn is_special_url_scheme(scheme: &str) -> bool {
    matches!(scheme, "http" | "https" | "ws" | "wss" | "ftp" | "file")
}

fn normalize_url_path(path: &str) -> String {
    let absolute = path.starts_with('/');
    let mut segments: Vec<&str> = Vec::new();
    for segment in path.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                segments.pop();
            }
            _ => segments.push(segment),
        }
    }
    let mut out = String::new();
    if absolute {
        out.push('/');
    }
    out.push_str(&segments.join("/"));
    // A trailing "/" — or a trailing "." / ".." segment, which denotes a
    // directory (e.g. `new URL(".", base)`) — keeps the path ending in "/".
    let ends_dir = path.ends_with('/')
        || path.ends_with("/.")
        || path.ends_with("/..")
        || path == "."
        || path == "..";
    if ends_dir && !out.ends_with('/') {
        out.push('/');
    }
    if out.is_empty() && absolute {
        out.push('/');
    }
    out
}

fn parse_url_authority(authority: &str) -> Option<(String, String, String, String, String)> {
    let (userinfo, hostport) = authority.rsplit_once('@').unwrap_or(("", authority));
    let (username, password) = match userinfo.split_once(':') {
        Some((username, password)) => (username.to_string(), password.to_string()),
        None => (userinfo.to_string(), String::new()),
    };
    let (hostname, port) = match hostport.rsplit_once(':') {
        Some((hostname, port)) if !port.is_empty() && port.chars().all(|c| c.is_ascii_digit()) => {
            (hostname.to_string(), port.to_string())
        }
        _ => (hostport.to_string(), String::new()),
    };
    let host = if port.is_empty() { hostname.clone() } else { format!("{hostname}:{port}") };
    Some((username, password, hostname, port, host))
}

fn parse_whatwg_url(input: &str, base: Option<&str>) -> Option<UrlComponents> {
    let input = trim_url_input(input);
    if input.is_empty() {
        return None;
    }

    let is_absolute = input
        .find(':')
        .map(|pos| {
            let scheme = &input[..pos];
            !scheme.is_empty()
                && scheme.chars().enumerate().all(|(i, c)| {
                    if i == 0 {
                        c.is_ascii_alphabetic()
                    } else {
                        c.is_ascii_alphanumeric() || matches!(c, '+' | '.' | '-')
                    }
                })
        })
        .unwrap_or(false);
    let base_components = if is_absolute {
        None
    } else {
        Some(parse_whatwg_url(base?, None)?)
    };

    let (scheme, rest) = if let Some(pos) = input.find(':') {
        (input[..pos].to_ascii_lowercase(), &input[pos + 1..])
    } else {
        (base_components.as_ref()?.protocol.trim_end_matches(':').to_string(), input)
    };
    let protocol = format!("{scheme}:");
    let mut username = String::new();
    let mut password = String::new();
    let mut host = String::new();
    let mut hostname = String::new();
    let mut port = String::new();
    let mut pathname = String::new();
    let mut search = String::new();
    let mut hash = String::new();

    if is_absolute {
        let mut rest = rest;
        let mut authority = "";
        if let Some(after) = rest.strip_prefix("//") {
            let end = after.find(['/', '?', '#']).unwrap_or(after.len());
            authority = &after[..end];
            rest = &after[end..];
        }
        if !authority.is_empty() || is_special_url_scheme(&scheme) {
            let (u, p, hname, pport, h) = parse_url_authority(authority)?;
            username = u;
            password = p;
            hostname = hname;
            port = pport;
            host = h;
            if is_special_url_scheme(&scheme) && scheme != "file" && host.is_empty() {
                return None;
            }
        }
        let path_end = rest.find(['?', '#']).unwrap_or(rest.len());
        pathname = rest[..path_end].to_string();
        rest = &rest[path_end..];
        if pathname.is_empty() && is_special_url_scheme(&scheme) {
            pathname = "/".to_string();
        }
        if let Some(after) = rest.strip_prefix('?') {
            let end = after.find('#').unwrap_or(after.len());
            if end > 0 {
                search = format!("?{}", &after[..end]);
            }
            rest = &after[end..];
        }
        if let Some(after) = rest.strip_prefix('#') {
            if !after.is_empty() {
                hash = format!("#{after}");
            }
        }
    } else {
        let base = base_components.as_ref()?;
        username = base.username.clone();
        password = base.password.clone();
        host = base.host.clone();
        hostname = base.hostname.clone();
        port = base.port.clone();
        let mut rest = rest;
        if rest.starts_with('/') {
            let path_end = rest.find(['?', '#']).unwrap_or(rest.len());
            pathname = normalize_url_path(&rest[..path_end]);
            rest = &rest[path_end..];
        } else if rest.starts_with('?') {
            pathname = base.pathname.clone();
        } else if rest.starts_with('#') {
            pathname = base.pathname.clone();
            search = base.search.clone();
        } else {
            let base_dir = base
                .pathname
                .rsplit_once('/')
                .map(|(dir, _)| format!("{dir}/"))
                .unwrap_or_else(|| "/".to_string());
            let path_end = rest.find(['?', '#']).unwrap_or(rest.len());
            let joined = format!("{base_dir}{}", &rest[..path_end]);
            pathname = normalize_url_path(&joined);
            rest = &rest[path_end..];
        }
        if let Some(after) = rest.strip_prefix('?') {
            let end = after.find('#').unwrap_or(after.len());
            if end > 0 {
                search = format!("?{}", &after[..end]);
            }
            rest = &after[end..];
        }
        if let Some(after) = rest.strip_prefix('#') {
            if !after.is_empty() {
                hash = format!("#{after}");
            }
        }
    }

    if pathname.is_empty() && is_special_url_scheme(&scheme) {
        pathname = "/".to_string();
    }
    let origin = if matches!(scheme.as_str(), "http" | "https" | "ws" | "wss" | "ftp") && !hostname.is_empty() {
        if port.is_empty() {
            format!("{scheme}://{hostname}")
        } else {
            format!("{scheme}://{hostname}:{port}")
        }
    } else {
        "null".to_string()
    };
    let mut href = String::new();
    href.push_str(&protocol);
    if !host.is_empty() || is_special_url_scheme(&scheme) {
        href.push_str("//");
        if !username.is_empty() || !password.is_empty() {
            href.push_str(&username);
            if !password.is_empty() {
                href.push(':');
                href.push_str(&password);
            }
            href.push('@');
        }
        href.push_str(&host);
    }
    href.push_str(&pathname);
    href.push_str(&search);
    href.push_str(&hash);
    Some(UrlComponents { href, protocol, username, password, host, hostname, port, pathname, search, hash, origin })
}

fn is_uri_unreserved(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'!' | b'~' | b'*' | b'\'' | b'(' | b')')
}

/// `encodeURIComponent` (component=false → also keep `;/?:@&=+$,#` for `encodeURI`).
fn encode_uri(text: &str, full_uri: bool) -> String {
    let reserved = b";/?:@&=+$,#";
    let mut out = String::with_capacity(text.len());
    for &byte in text.as_bytes() {
        if is_uri_unreserved(byte) || (full_uri && reserved.contains(&byte)) {
            out.push(byte as char);
        } else {
            out.push('%');
            out.push_str(&format!("{byte:02X}"));
        }
    }
    out
}

/// `decodeURIComponent` / `decodeURI`: undo percent-encoding. Returns `None` on
/// malformed input.
fn decode_uri(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            if i + 2 >= bytes.len() {
                return None;
            }
            let hi = (bytes[i + 1] as char).to_digit(16)?;
            let lo = (bytes[i + 2] as char).to_digit(16)?;
            out.push((hi * 16 + lo) as u8);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// A registered `MutationObserver`: its JS callback and the observer instance
/// object (held so neither is lost and `===` identity is preserved when the
/// instance is handed back to the callback).
#[derive(Clone)]
struct MutationObserverReg {
    callback: Value,
    instance: Value,
}

/// A registered `ResizeObserver`: its JS callback and observer instance.
#[derive(Clone)]
struct ResizeObserverReg {
    callback: Value,
    instance: Value,
}

enum ObjectIntrospectionKind {
    Keys,
    Values,
    Entries,
}

pub struct Vm {
    stack: Vec<Value>,
    frames: Vec<CallFrame>,
    last_backtrace: Option<String>,
    pending_call_name: Option<String>,
    current_script_src: Option<String>,
    current_script_node: Option<NodeId>,
    heap: Heap,
    globals: HashMap<String, Value>,
    callables: HashMap<RawGcRef, Callable>,
    string_cache: HashMap<String, GcRef<JsString>>,
    fuel: u32,
    object_prototype: Option<GcRef<JsObject>>,
    function_prototype: Option<GcRef<JsObject>>,
    array_prototype: Option<GcRef<JsObject>>,
    string_prototype: Option<GcRef<JsObject>>,
    number_prototype: Option<GcRef<JsObject>>,
    boolean_prototype: Option<GcRef<JsObject>>,
    regexp_prototype: Option<GcRef<JsObject>>,
    date_prototype: Option<GcRef<JsObject>>,
    generator_prototype: Option<GcRef<JsObject>>,
    async_generator_prototype: Option<GcRef<JsObject>>,
    url_search_params_prototype: Option<GcRef<JsObject>>,
    headers_prototype: Option<GcRef<JsObject>>,
    form_data_prototype: Option<GcRef<JsObject>>,
    weak_ref_prototype: Option<GcRef<JsObject>>,
    text_encoder_prototype: Option<GcRef<JsObject>>,
    text_decoder_prototype: Option<GcRef<JsObject>>,
    url_prototype: Option<GcRef<JsObject>>,
    error_prototype: Option<GcRef<JsObject>>,
    promise_prototype: Option<GcRef<JsObject>>,
    map_prototype: Option<GcRef<JsObject>>,
    set_prototype: Option<GcRef<JsObject>>,
    array_buffer_prototype: Option<GcRef<JsObject>>,
    typed_array_prototype: Option<GcRef<JsObject>>,
    event_loop: EventLoop,
    random_state: u64,
    host: Box<dyn Host>,
    /// Event listeners stored by (node_handle, event_type) → list of JS function GcRefs.
    /// Lives in the VM (not the Host) so GcRefs remain valid.
    event_listeners: HashMap<u32, HashMap<String, Vec<GcRef<JsObject>>>>,
    /// Live `MutationObserver` instances keyed by `ObserverId.0`: the JS callback
    /// and the observer object itself (passed back to the callback as `this` and
    /// its 2nd argument). Lives in the VM so the callback/instance survive.
    mutation_observers: HashMap<u64, MutationObserverReg>,
    /// Live `ResizeObserver` instances keyed by `ObserverId.0`: the JS callback
    /// and the observer object itself (passed back to the callback as `this` and
    /// its 2nd argument). Lives in the VM so the callback/instance survive.
    resize_observers: HashMap<u64, ResizeObserverReg>,
    /// Re-entrancy guard for the mutation-observer delivery checkpoint, so the
    /// `drain_microtasks` calls made while delivering don't recurse into another
    /// delivery pass.
    delivering_mutations: bool,
    /// Re-entrancy guard for `slotchange` event delivery.
    delivering_slotchange: bool,
    /// Interned DOM node wrappers keyed by node handle, so that accessing the
    /// same node twice (`el.parentNode === parent`, `a.nextSibling === b`)
    /// returns the SAME object — node identity that frameworks rely on. Also
    /// lets expando properties set on a node persist across accesses. Safe
    /// because the heap is append-only within a session and node handles are
    /// stable (removed nodes keep their arena slot).
    node_wrappers: HashMap<u32, GcRef<JsObject>>,
    /// Cache for stateless builtin method values (constructable=false, prototype=None).
    /// Avoids a heap allocation on every DOM property access like element.appendChild.
    builtin_method_cache: HashMap<BuiltinId, Value>,
    /// Next id handed out by `Symbol()`. Ids below `FIRST_USER_SYMBOL` are
    /// reserved for well-known symbols (e.g. Symbol.iterator).
    next_symbol_id: u32,
    /// Optional descriptions for created symbols (for Symbol.prototype.toString).
    symbol_descriptions: HashMap<u32, String>,
    /// Global registry for `Symbol.for(key)` / `Symbol.keyFor(sym)`.
    symbol_registry: HashMap<String, u32>,
    /// Result of the most recent generator step, set by Yield/Return and read by
    /// `Generator.prototype.next`.
    generator_outcome: Option<GeneratorOutcome>,
    /// DOM interface constructor objects (`Node`, `Element`, `HTMLElement`, …)
    /// keyed by the constructor's heap ref, mapped to the interface name. Host DOM
    /// nodes don't have their prototype chains wired to these, so `instanceof`
    /// special-cases them by interface name (see `instanceof_value`). Real DOM
    /// libraries (React) gate on `x instanceof Element` etc.
    dom_interface_ctors: HashMap<RawGcRef, &'static str>,
    /// `customElements.define` registry: lowercase tag name → definition.
    custom_elements: HashMap<String, CustomElementDef>,
}

/// A `customElements.define`d class: the constructor value plus its
/// (lowercased) `observedAttributes` list.
#[derive(Clone)]
struct CustomElementDef {
    class_value: Value,
    observed: Vec<String>,
}

/// DOM interface names exposed as global constructors for `instanceof`. `Event`
/// and `CustomEvent` are intentionally omitted — they already exist as globals.
const DOM_INTERFACE_NAMES: &[&str] = &[
    "EventTarget",
    "Node",
    "Element",
    "HTMLElement",
    "Document",
    "DocumentFragment",
    "CharacterData",
    "Text",
    "Comment",
    "HTMLIFrameElement",
    "HTMLInputElement",
    "HTMLTextAreaElement",
    "HTMLSelectElement",
    "HTMLButtonElement",
    "HTMLAnchorElement",
    "SVGElement",
    "Window",
];

/// How a generator step ended.
enum GeneratorOutcome {
    Yielded(Value),
    Returned(Value),
}

/// Well-known symbol id for `Symbol.iterator`.
const SYMBOL_ITERATOR_ID: u32 = 1;
const SYMBOL_ASYNC_ITERATOR_ID: u32 = 2;
/// Well-known symbol id for `Symbol.toPrimitive` (see the registration table in
/// the global setup: iterator=1, asyncIterator=2, hasInstance=3, toPrimitive=4).
const SYMBOL_TO_PRIMITIVE_ID: u32 = 4;
/// Well-known symbol id for `Symbol.hasInstance` (custom `instanceof`).
const SYMBOL_HAS_INSTANCE_ID: u32 = 3;
/// First id available to user-created `Symbol(...)` values.
const FIRST_USER_SYMBOL: u32 = 16;

/// Maximum JS call-frame depth before a RangeError is thrown. JS→JS calls are
/// iterative (frames live in a heap `Vec`, the interpreter loop drives them — no
/// native Rust recursion per call), so this is an artificial guard against
/// runaway recursion, not a native-stack limit. The JS worker thread has a 32 MB
/// stack, and real engines allow ~10k frames; 1024 was far too low and tripped
/// legitimately deep framework call chains (e.g. Vite's bundle).
const MAX_CALL_FRAMES: usize = 10_000;

impl Vm {
    /// Create a VM with a no-op host (for tests and scripts that don't need DOM/console).
    pub fn new(heap: Heap) -> Self {
        Self::with_host(heap, Box::new(NoopHost))
    }

    /// Create a VM wired to a real host implementation.
    pub fn with_host(heap: Heap, host: Box<dyn Host>) -> Self {
        let random_state = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos() as u64)
            .unwrap_or(0x1234_5678_9abc_def0);
        let mut vm = Self {
            stack: Vec::new(),
            frames: Vec::new(),
            last_backtrace: None,
            pending_call_name: None,
            current_script_src: None,
            current_script_node: None,
            heap,
            globals: HashMap::new(),
            callables: HashMap::new(),
            string_cache: HashMap::new(),
            fuel: 1_000_000,
            object_prototype: None,
            function_prototype: None,
            array_prototype: None,
            string_prototype: None,
            number_prototype: None,
            boolean_prototype: None,
            regexp_prototype: None,
            date_prototype: None,
            generator_prototype: None,
            async_generator_prototype: None,
            url_search_params_prototype: None,
            headers_prototype: None,
            form_data_prototype: None,
            weak_ref_prototype: None,
            text_encoder_prototype: None,
            text_decoder_prototype: None,
            url_prototype: None,
            error_prototype: None,
            promise_prototype: None,
            map_prototype: None,
            set_prototype: None,
            array_buffer_prototype: None,
            typed_array_prototype: None,
            event_loop: EventLoop::new(),
            random_state,
            host,
            event_listeners: HashMap::new(),
            mutation_observers: HashMap::new(),
            resize_observers: HashMap::new(),
            delivering_mutations: false,
            delivering_slotchange: false,
            node_wrappers: HashMap::new(),
            builtin_method_cache: HashMap::new(),
            next_symbol_id: FIRST_USER_SYMBOL,
            symbol_descriptions: HashMap::new(),
            symbol_registry: HashMap::new(),
            generator_outcome: None,
            dom_interface_ctors: HashMap::new(),
            custom_elements: HashMap::new(),
        };
        vm.install_globals();
        vm
    }

    /// Borrow the heap immutably for inspection in tests and diagnostics.
    pub fn heap(&self) -> &Heap {
        &self.heap
    }

    /// Borrow the host mutably (for reading results after execution).
    pub fn host_mut(&mut self) -> &mut dyn Host {
        self.host.as_mut()
    }

    fn capture_backtrace(&self) -> String {
        let mut lines = Vec::with_capacity(self.frames.len());
        for (index, frame) in self.frames.iter().enumerate().rev() {
            let name = if index == 0 {
                "<script>"
            } else {
                match frame.proto.name.as_deref() {
                    Some(name) if !name.is_empty() => name,
                    _ => "<anonymous>",
                }
            };
            lines.push(format!("    at {name}"));
        }
        lines.join("\n")
    }

    pub fn take_last_backtrace(&mut self) -> Option<String> {
        self.last_backtrace.take()
    }

    pub fn set_current_script_src(&mut self, src: Option<String>) {
        self.current_script_src = src;
    }

    pub fn set_current_script_node(&mut self, node: Option<NodeId>) {
        self.current_script_node = node;
    }

    /// Fire a DOM event on a node handle, invoking all registered JS listeners.
    /// `node_handle` is the raw u32 from HostObjectSlot.handle (0 = document/window).
    /// `event_type` is e.g. "DOMContentLoaded", "click", "load".
    /// Whether any listener is registered for `event_type` on the node with the
    /// given handle (handle 0 = window/document). Lets callers skip dispatching
    /// no-op events (e.g. `scroll` when nothing listens for it).
    pub fn has_event_listener(&self, node_handle: u32, event_type: &str) -> bool {
        self.event_listeners
            .get(&node_handle)
            .map(|by_type| by_type.contains_key(event_type))
            .unwrap_or(false)
    }

    pub fn fire_dom_event(&mut self, node_handle: u32, event_type: &str) -> Result<(), VmError> {
        self.fire_dom_event_with(node_handle, event_type, &DomEventInit::default())?;
        Ok(())
    }

    /// Dispatch a host event (a real user interaction) to the node's listeners,
    /// delivering an Event object that carries `init`'s details (key/data/
    /// modifiers, the target, preventDefault/stopPropagation). Returns whether
    /// `preventDefault()` was called, so the host can suppress the default
    /// action (e.g. inserting a typed character).
    pub fn fire_dom_event_with(
        &mut self,
        node_handle: u32,
        event_type: &str,
        init: &DomEventInit,
    ) -> Result<bool, VmError> {
        let target = self.make_dom_node_value(NodeId(node_handle));
        let event_obj = self.build_host_event(event_type, &target, init);
        let event_val = Value::Object(event_obj);
        // Real user events propagate target → ancestors too (so event delegation,
        // e.g. one click listener on a framework's root, fires for child clicks).
        self.propagate_event(node_handle, &event_val, event_type, init.bubbles)?;
        let default_prevented = self
            .get_property_value(&event_val, &PropertyKey::from("defaultPrevented"))
            .unwrap_or(Value::Bool(false));
        Ok(self.is_truthy(&default_prevented))
    }

    /// Dispatch `event_val` of type `event_type` to the listeners along the
    /// propagation path: the target node, then (if `bubbles`) its ancestors,
    /// updating `currentTarget` per node and honoring `stopPropagation`
    /// (cancelBubble) / `stopImmediatePropagation`. Shared by the host-driven
    /// `fire_dom_event_with` and the script-driven `dispatchEvent`.
    fn propagate_event(
        &mut self,
        target_handle: u32,
        event_val: &Value,
        event_type: &str,
        bubbles: bool,
    ) -> Result<(), VmError> {
        let Value::Object(event_ref) = event_val else {
            return Ok(());
        };
        let event_ref = *event_ref;
        // focus/blur reaching a node moves document.activeElement (boa parity).
        if event_type.eq_ignore_ascii_case("focus") || event_type.eq_ignore_ascii_case("blur") {
            let _ = self.host.mutate_dom(DomMutation::NoteFocusChange {
                window: WindowId(0),
                node: NodeId(target_handle),
                focused: event_type.eq_ignore_ascii_case("focus"),
            });
        }
        // The full propagation path (target → root), crossing shadow boundaries
        // when the event is composed. composedPath() returns it; propagation
        // only visits ancestors when the event bubbles.
        let composed = self
            .get_property_value(event_val, &PropertyKey::from("composed"))
            .map(|v| self.is_truthy(&v))
            .unwrap_or(false);
        let full_path: Vec<u32> = match self.host.read_dom(DomRead::EventPath {
            node: NodeId(target_handle),
            composed,
        }) {
            Ok(DomReadResult::Nodes(ids)) => ids.iter().map(|id| id.0).collect(),
            _ => vec![target_handle],
        };
        // Store composedPath (target → root order) on the event.
        let composed_path_values: Vec<Value> = full_path
            .iter()
            .map(|&h| self.make_dom_node_value(NodeId(h)))
            .collect();
        let composed_path_array = self.make_array_from_values(composed_path_values)?;
        self.define_data_property(
            event_ref,
            PropertyKey::from("__composedPath"),
            composed_path_array,
            true,
            false,
            true,
        );
        let path: Vec<u32> = if bubbles {
            full_path
        } else {
            vec![target_handle]
        };
        'propagate: for node_handle in path {
            let current_target = self.make_dom_node_value(NodeId(node_handle));
            self.define_data_property(
                event_ref,
                PropertyKey::from("currentTarget"),
                current_target.clone(),
                true,
                true,
                true,
            );
            // Shadow retargeting: the event's `target` is rewritten relative to
            // the current node's tree root as it crosses shadow boundaries.
            let retargeted = match self.host.read_dom(DomRead::RetargetTarget {
                target: NodeId(target_handle),
                current: NodeId(node_handle),
            }) {
                Ok(DomReadResult::Node(id)) => id,
                _ => NodeId(target_handle),
            };
            let retargeted_value = self.make_dom_node_value(retargeted);
            self.define_data_property(
                event_ref,
                PropertyKey::from("target"),
                retargeted_value,
                true,
                true,
                true,
            );
            let listeners: Vec<GcRef<JsObject>> = self
                .event_listeners
                .get(&node_handle)
                .and_then(|m| m.get(event_type))
                .cloned()
                .unwrap_or_default();
            for listener in listeners {
                // A listener's `this` is the node it is attached to (currentTarget).
                self.call_value_sync(
                    Value::Object(listener),
                    current_target.clone(),
                    vec![event_val.clone()],
                )?;
                self.drain_microtasks();
                let stop_immediate = self
                    .get_property_value(event_val, &PropertyKey::from("__stopImmediate"))
                    .unwrap_or(Value::Undefined);
                if self.is_truthy(&stop_immediate) {
                    break 'propagate;
                }
            }
            let cancel = self
                .get_property_value(event_val, &PropertyKey::from("cancelBubble"))
                .unwrap_or(Value::Undefined);
            if self.is_truthy(&cancel) {
                break;
            }
        }
        Ok(())
    }

    /// Build the Event object delivered to host-event listeners.
    fn build_host_event(
        &mut self,
        event_type: &str,
        target: &Value,
        init: &DomEventInit,
    ) -> GcRef<JsObject> {
        let proto = self.object_prototype_ref();
        let event = self.allocate_ordinary_object(Some(proto));
        let type_val = self.make_string_value(event_type);
        let set = |vm: &mut Self, name: &str, value: Value| {
            vm.define_data_property(event, PropertyKey::from(name), value, true, true, true);
        };
        set(self, "type", type_val);
        set(self, "target", target.clone());
        set(self, "currentTarget", target.clone());
        set(self, "srcElement", target.clone());
        set(self, "bubbles", Value::Bool(init.bubbles));
        set(self, "cancelable", Value::Bool(init.cancelable));
        set(self, "composed", Value::Bool(false));
        set(self, "defaultPrevented", Value::Bool(false));
        set(self, "cancelBubble", Value::Bool(false));
        set(self, "eventPhase", Value::Number(2.0));
        set(self, "timeStamp", Value::Number(0.0));
        set(self, "isTrusted", Value::Bool(true));
        set(self, "altKey", Value::Bool(init.alt_key));
        set(self, "ctrlKey", Value::Bool(init.ctrl_key));
        set(self, "shiftKey", Value::Bool(init.shift_key));
        set(self, "metaKey", Value::Bool(init.meta_key));
        set(self, "repeat", Value::Bool(init.repeat));
        set(self, "isComposing", Value::Bool(init.is_composing));
        for (name, value) in [
            ("key", &init.key),
            ("code", &init.code),
            ("data", &init.data),
            ("inputType", &init.input_type),
        ] {
            if let Some(text) = value {
                let v = self.make_string_value(text);
                self.define_data_property(event, PropertyKey::from(name), v, true, true, true);
            }
        }
        for (name, value) in [
            ("clientX", init.client_x),
            ("clientY", init.client_y),
            ("button", init.button),
            ("buttons", init.buttons),
        ] {
            if let Some(number) = value {
                self.define_data_property(
                    event,
                    PropertyKey::from(name),
                    Value::Number(number as f64),
                    true,
                    true,
                    true,
                );
            }
        }
        let prevent = self.allocate_builtin_method(BuiltinId::EventPreventDefault);
        self.define_data_property(event, PropertyKey::from("preventDefault"), prevent, true, true, true);
        let stop = self.allocate_builtin_method(BuiltinId::EventStopPropagation);
        self.define_data_property(event, PropertyKey::from("stopPropagation"), stop, true, true, true);
        let stop_immediate = self.allocate_builtin_method(BuiltinId::EventStopImmediatePropagation);
        self.define_data_property(
            event,
            PropertyKey::from("stopImmediatePropagation"),
            stop_immediate,
            true,
            true,
            true,
        );
        let composed_path = self.allocate_builtin_method(BuiltinId::DomEventComposedPath);
        self.define_data_property(event, PropertyKey::from("composedPath"), composed_path, true, false, true);
        event
    }

    /// Read a boolean flag from an Event init options object (e.g. `bubbles`).
    fn event_option_flag(&mut self, options: &Option<Value>, name: &str) -> bool {
        match options {
            Some(opts) => {
                let value = self
                    .get_property_value(opts, &PropertyKey::from(name))
                    .unwrap_or(Value::Undefined);
                self.is_truthy(&value)
            }
            None => false,
        }
    }

    /// Read a string field from an event init options object (e.g. `key`).
    fn event_option_string(&mut self, options: &Option<Value>, name: &str) -> String {
        match options {
            Some(opts @ Value::Object(_)) => {
                match self.get_property_value(opts, &PropertyKey::from(name)) {
                    Ok(Value::Undefined) | Err(_) => String::new(),
                    Ok(v) => self.to_string(&v),
                }
            }
            _ => String::new(),
        }
    }

    /// Read a numeric field from an event init options object (e.g. `clientX`).
    fn event_option_number(&mut self, options: &Option<Value>, name: &str) -> f64 {
        match options {
            Some(opts @ Value::Object(_)) => {
                match self.get_property_value(opts, &PropertyKey::from(name)) {
                    Ok(Value::Undefined) | Err(_) => 0.0,
                    Ok(v) => self.to_number(&v),
                }
            }
            _ => 0.0,
        }
    }

    /// Build a JS `Event` (or `CustomEvent`) object: standard data properties
    /// plus the `preventDefault` / `stopPropagation` / `stopImmediatePropagation`
    /// methods (cached builtins). Used by the constructors and could back
    /// synthetic event creation.
    fn make_event_object(
        &mut self,
        event_type: &str,
        options: Option<Value>,
        is_custom: bool,
    ) -> Result<Value, VmError> {
        let bubbles = self.event_option_flag(&options, "bubbles");
        let cancelable = self.event_option_flag(&options, "cancelable");
        let composed = self.event_option_flag(&options, "composed");
        let proto = self.object_prototype_ref();
        let event = self.allocate_ordinary_object(Some(proto));
        let type_val = self.make_string_value(event_type);
        let set = |vm: &mut Self, name: &str, value: Value| {
            vm.define_data_property(event, PropertyKey::from(name), value, true, true, true);
        };
        set(self, "type", type_val);
        set(self, "bubbles", Value::Bool(bubbles));
        set(self, "cancelable", Value::Bool(cancelable));
        set(self, "composed", Value::Bool(composed));
        set(self, "defaultPrevented", Value::Bool(false));
        set(self, "cancelBubble", Value::Bool(false));
        set(self, "target", Value::Null);
        set(self, "currentTarget", Value::Null);
        set(self, "srcElement", Value::Null);
        set(self, "eventPhase", Value::Number(0.0));
        set(self, "timeStamp", Value::Number(0.0));
        set(self, "isTrusted", Value::Bool(false));
        if is_custom {
            let detail = match &options {
                Some(opts) => self
                    .get_property_value(opts, &PropertyKey::from("detail"))
                    .unwrap_or(Value::Null),
                None => Value::Null,
            };
            self.define_data_property(
                event,
                PropertyKey::from("detail"),
                detail,
                true,
                true,
                true,
            );
        }
        let prevent = self.allocate_builtin_method(BuiltinId::EventPreventDefault);
        self.define_data_property(event, PropertyKey::from("preventDefault"), prevent, true, true, true);
        let stop = self.allocate_builtin_method(BuiltinId::EventStopPropagation);
        self.define_data_property(event, PropertyKey::from("stopPropagation"), stop, true, true, true);
        let stop_immediate = self.allocate_builtin_method(BuiltinId::EventStopImmediatePropagation);
        self.define_data_property(
            event,
            PropertyKey::from("stopImmediatePropagation"),
            stop_immediate,
            true,
            true,
            true,
        );
        let composed_path = self.allocate_builtin_method(BuiltinId::DomEventComposedPath);
        self.define_data_property(event, PropertyKey::from("composedPath"), composed_path, true, false, true);
        Ok(Value::Object(event))
    }

    pub fn execute(&mut self, chunk: &Chunk) -> Result<Value, VmError> {
        self.execute_with_this(chunk, self.globals.get("window").cloned().unwrap_or(Value::Undefined))
    }

    pub fn execute_module(&mut self, chunk: &Chunk) -> Result<Value, VmError> {
        self.execute_with_this(chunk, Value::Undefined)
    }

    pub fn set_global(&mut self, name: impl Into<String>, value: Value) {
        self.globals.insert(name.into(), value);
    }

    pub fn set_global_object(&mut self, name: impl Into<String>) {
        let object = self.allocate_ordinary_object(Some(self.object_prototype_ref()));
        self.globals.insert(name.into(), Value::Object(object));
    }

    fn execute_with_this(&mut self, chunk: &Chunk, this_value: Value) -> Result<Value, VmError> {
        self.stack.clear();
        self.frames.clear();
        self.fuel = 1_000_000;

        let closure = RuntimeClosure {
            proto: Rc::new(chunk.top_level.clone()),
            upvalues: Vec::new(),
        };
        self.push_call_frame(closure, Vec::new(), this_value, None)?;
        self.run_until_frame_depth(0)?;
        self.drain_microtasks();
        if self.stack.is_empty() {
            Ok(Value::Undefined)
        } else {
            self.pop_value()
        }
    }

    /// Compile and run a script source on the live VM without resetting the
    /// stack/frames — `document.write`'d <script> elements run nested inside
    /// the writing script's execution.
    pub fn eval_source(&mut self, source: &str) -> Result<Value, VmError> {
        use super::compiler::Compiler;
        use super::parser::Parser;
        let program = Parser::new(source)
            .parse()
            .map_err(|e| VmError::TypeError(format!("parse: {e:?}")))?;
        let chunk = Compiler::new(&program)
            .compile()
            .map_err(|e| VmError::TypeError(format!("compile: {e:?}")))?;
        let closure = RuntimeClosure {
            proto: Rc::new(chunk.top_level.clone()),
            upvalues: Vec::new(),
        };
        let base_depth = self.frames.len();
        let stack_len = self.stack.len();
        let global_this = self.globals.get("window").cloned().unwrap_or(Value::Undefined);
        self.push_call_frame(closure, Vec::new(), global_this, None)?;
        self.run_until_frame_depth(base_depth)?;
        if self.stack.len() > stack_len {
            self.pop_value()
        } else {
            Ok(Value::Undefined)
        }
    }

    pub fn event_loop_tick(&mut self, now_ms: u64, has_render_opportunity: bool) -> TickResult {
        self.event_loop.current_time_ms = now_ms;
        self.enqueue_due_timers(now_ms);

        let mut did_work = false;
        if let Some(task) = self.event_loop.macrotask_queue.pop_front() {
            did_work = true;
            let _ = self.run_task(task);
            self.drain_microtasks();
        }

        let mut needs_render = false;
        if has_render_opportunity && !self.event_loop.raf_callbacks.is_empty() {
            did_work = true;
            needs_render = true;
            let callbacks = self
                .event_loop
                .raf_callbacks
                .drain(..)
                .map(|(_, entry)| entry)
                .collect::<Vec<_>>();
            for entry in callbacks {
                let _ = self.call_value_sync(
                    Value::Object(entry.callback),
                    Value::Undefined,
                    vec![Value::Number(now_ms as f64)],
                );
                self.drain_microtasks();
            }

            self.event_loop.resize_observer_depth = 0;
            while self.event_loop.resize_observer_depth <= 10 {
                break;
            }
            self.event_loop.resize_observer_depth = 0;
        }

        if needs_render {
            TickResult::NeedsRender
        } else if did_work {
            TickResult::DidWork
        } else {
            TickResult::Idle
        }
    }

    /// Run already-due timers (zero-delay `setTimeout`/`setInterval`) and any
    /// queued microtasks to quiescence WITHOUT advancing virtual time, so that
    /// Promise chains and `setTimeout(fn, 0)`-style deferred work settle before
    /// a snapshot. Timers with a real delay stay pending (they only fire once
    /// virtual time is advanced via `event_loop_tick`). `requestAnimationFrame`
    /// callbacks are not run here. Bounded by `max_steps` to guard against
    /// zero-delay timers that reschedule themselves; returns the number of
    /// macrotasks executed.
    pub fn run_due_jobs(&mut self, max_steps: usize) -> usize {
        let now = self.event_loop.current_time_ms;
        self.run_due_jobs_at(now, max_steps)
    }

    /// Like `run_due_jobs`, but first advances the virtual clock to `now_ms`
    /// (never backwards). The initial settle uses a 1ms window so "next turn"
    /// timers fire before the first snapshot (boa parity) while timers they
    /// schedule — and longer delays — stay pending.
    pub fn run_due_jobs_at(&mut self, now_ms: u64, max_steps: usize) -> usize {
        self.drain_microtasks();
        let now = self.event_loop.current_time_ms.max(now_ms);
        let mut steps = 0;
        while steps < max_steps {
            if matches!(self.event_loop_tick(now, false), TickResult::Idle) {
                break;
            }
            steps += 1;
        }
        steps
    }

    /// Whether the event loop has outstanding work (pending timers, RAF
    /// callbacks, or queued tasks/microtasks). Lets a host decide whether to
    /// keep pumping `event_loop_tick` over time.
    pub fn has_pending_event_loop_work(&self) -> bool {
        !self.event_loop.timer_heap.is_empty()
            || !self.event_loop.raf_callbacks.is_empty()
            || !self.event_loop.macrotask_queue.is_empty()
            || !self.event_loop.microtask_queue.is_empty()
    }

    /// The earliest pending timer's due time in ms (heap-min), if any. Lets the
    /// host schedule a wakeup instead of busy-polling.
    pub fn next_timer_due_ms(&self) -> Option<u64> {
        self.event_loop.timer_heap.peek().map(|entry| entry.0.due_ms)
    }

    /// Advance the event loop to `now_ms`: run every timer due by then (plus
    /// their microtasks), then exactly ONE `requestAnimationFrame` pass for this
    /// frame. Returns whether any work ran, so the caller can re-render only on
    /// change.
    ///
    /// The two-phase split matters for animation: a `requestAnimationFrame`
    /// callback that re-registers itself (the normal animation-loop pattern)
    /// would, if we simply looped `event_loop_tick(.., true)` to quiescence,
    /// drain-and-rerun rAF `max_steps` times in a single frame — advancing the
    /// animation thousands of steps at one timestamp. Instead we drain due
    /// timers first (bounded), then run a single rAF pass; callbacks scheduled
    /// during that pass are deferred to the next frame, matching the HTML spec.
    pub fn pump_event_loop(&mut self, now_ms: u64, max_steps: usize) -> bool {
        let mut did_work = false;

        // Phase 1: run all timers/macrotasks due by `now_ms` and their
        // microtasks, without touching rAF. Bounded by `max_steps` to guard
        // against zero-delay timers that reschedule themselves.
        let mut steps = 0;
        while steps < max_steps {
            match self.event_loop_tick(now_ms, false) {
                TickResult::Idle => break,
                _ => {
                    did_work = true;
                    steps += 1;
                }
            }
        }

        // Phase 2: exactly one requestAnimationFrame pass for this frame.
        if !self.event_loop.raf_callbacks.is_empty()
            && !matches!(self.event_loop_tick(now_ms, true), TickResult::Idle)
        {
            did_work = true;
        }

        did_work
    }

    fn run_until_frame_depth(&mut self, target_depth: usize) -> Result<(), VmError> {
        while self.frames.len() > target_depth {
            let opcode = {
                let frame = self
                    .frames
                    .last_mut()
                    .ok_or_else(|| VmError::RangeError("no call frame available".to_string()))?;
                let opcode = frame.proto.code.get(frame.ip).cloned().ok_or_else(|| {
                    VmError::RangeError("instruction pointer ran past bytecode".to_string())
                })?;
                frame.ip += 1;
                opcode
            };
            if let Err(error) = self.execute_opcode(opcode) {
                self.handle_runtime_error(error)?;
            }
        }
        Ok(())
    }

    fn execute_opcode(&mut self, opcode: Opcode) -> Result<(), VmError> {
        match opcode {
            Opcode::LoadConst(index) => {
                let constant = self
                    .current_proto()?
                    .constants
                    .get(index as usize)
                    .cloned()
                    .ok_or_else(|| {
                        VmError::RangeError(format!("constant index {index} out of range"))
                    })?;
                let value = match constant {
                    Constant::Number(number) => Value::Number(number),
                    Constant::String(text) => self.make_string_value(&text),
                    Constant::RegExp { .. } => {
                        return Err(VmError::Unimplemented(
                            "regexp constants must use MakeRegExp",
                        ));
                    }
                };
                self.stack.push(value);
            }
            Opcode::LoadUndefined => self.stack.push(Value::Undefined),
            Opcode::LoadNull => self.stack.push(Value::Null),
            Opcode::LoadTrue => self.stack.push(Value::Bool(true)),
            Opcode::LoadFalse => self.stack.push(Value::Bool(false)),
            Opcode::LoadThis => {
                let value = self.current_this()?.clone();
                self.stack.push(value);
            }
            Opcode::LoadNewTarget => {
                let value = self
                    .frames
                    .last()
                    .map(|frame| frame.new_target.clone())
                    .unwrap_or(Value::Undefined);
                self.stack.push(value);
            }
            Opcode::Pop => {
                self.pop_value()?;
            }
            Opcode::Dup => {
                let value = self.peek_value()?.clone();
                self.stack.push(value);
            }
            Opcode::GetLocal(slot) => {
                let value = self.local_cell(slot)?.borrow().clone();
                self.stack.push(value);
            }
            Opcode::SetLocal(slot) => {
                let value = self.pop_value()?;
                *self.local_cell(slot)?.borrow_mut() = value;
            }
            Opcode::FreshenLocal(slot) => {
                let value = self.local_cell(slot)?.borrow().clone();
                let frame = self
                    .frames
                    .last_mut()
                    .ok_or_else(|| VmError::RangeError("no active frame".to_string()))?;
                if let Some(cell) = frame.locals.get_mut(slot as usize) {
                    *cell = Rc::new(RefCell::new(value));
                }
            }
            Opcode::GetUpvalue(slot) => {
                let value = self.upvalue_cell(slot)?.borrow().clone();
                self.stack.push(value);
            }
            Opcode::SetUpvalue(slot) => {
                let value = self.pop_value()?;
                *self.upvalue_cell(slot)?.borrow_mut() = value;
            }
            Opcode::GetGlobal(index) => {
                let name = self.constant_name(index)?.to_string();
                let value = if let Some(existing) = self.globals.get(&name).cloned() {
                    existing
                } else if Self::is_window_global(&name) {
                    // In a browser the global object IS the window, so a bare
                    // reference like `location` resolves to `window.location`.
                    self.get_window_property(name)?
                } else {
                    return Err(VmError::ReferenceError(format!("{name} is not defined")));
                };
                self.stack.push(value);
            }
            Opcode::SetGlobal(index) => {
                let name = self.constant_name(index)?.to_string();
                let value = self.pop_value()?;
                self.globals.insert(name, value);
            }
            Opcode::GetGlobalOptional(index) => {
                // Used by `typeof name`. Mirror `GetGlobal`'s window-global
                // fallback so `typeof crypto` / `typeof navigator` report the
                // real type instead of "undefined" (the global object IS the
                // window). A name that is neither a real global nor a window
                // global stays `undefined` — `typeof undeclared` must not throw.
                let name = self.constant_name(index)?.to_string();
                let value = if let Some(existing) = self.globals.get(&name).cloned() {
                    existing
                } else if Self::is_window_global(&name) {
                    self.get_window_property(name)?
                } else {
                    Value::Undefined
                };
                self.stack.push(value);
            }
            Opcode::LoadArguments => {
                let args = self
                    .frames
                    .last()
                    .map(|frame| frame.arguments.clone())
                    .unwrap_or_default();
                let array = self.make_array_from_values(args)?;
                self.stack.push(array);
            }
            Opcode::Add => self.binary_add()?,
            Opcode::Sub => self.binary_numeric(|lhs, rhs| lhs - rhs)?,
            Opcode::Mul => self.binary_numeric(|lhs, rhs| lhs * rhs)?,
            Opcode::Div => self.binary_numeric(|lhs, rhs| lhs / rhs)?,
            Opcode::Rem => self.binary_numeric(|lhs, rhs| lhs % rhs)?,
            Opcode::Exp => self.binary_numeric(|lhs, rhs| lhs.powf(rhs))?,
            Opcode::Eq => self.binary_compare(|vm, lhs, rhs| vm.abstract_equal(lhs, rhs))?,
            Opcode::StrictEq => self.binary_compare(|vm, lhs, rhs| vm.strict_equal(lhs, rhs))?,
            Opcode::Ne => self.binary_compare(|vm, lhs, rhs| !vm.abstract_equal(lhs, rhs))?,
            Opcode::StrictNe => self.binary_compare(|vm, lhs, rhs| !vm.strict_equal(lhs, rhs))?,
            Opcode::Lt => self.binary_compare_numeric_or_string(|lhs, rhs| lhs < rhs)?,
            Opcode::Le => self.binary_compare_numeric_or_string(|lhs, rhs| lhs <= rhs)?,
            Opcode::Gt => self.binary_compare_numeric_or_string(|lhs, rhs| lhs > rhs)?,
            Opcode::Ge => self.binary_compare_numeric_or_string(|lhs, rhs| lhs >= rhs)?,
            Opcode::BitAnd => self.binary_bitwise(|lhs, rhs| lhs & rhs)?,
            Opcode::BitOr => self.binary_bitwise(|lhs, rhs| lhs | rhs)?,
            Opcode::BitXor => self.binary_bitwise(|lhs, rhs| lhs ^ rhs)?,
            Opcode::Shl => self.binary_shift(|lhs, rhs| lhs.wrapping_shl(rhs & 0x1f))?,
            Opcode::Shr => self.binary_shift(|lhs, rhs| lhs.wrapping_shr(rhs & 0x1f))?,
            Opcode::UShr => self.binary_unsigned_shift()?,
            Opcode::Neg => {
                let value = self.pop_value()?;
                let n = self.to_number_coerced(&value)?;
                self.stack.push(Value::Number(-n));
            }
            Opcode::Not => {
                let value = self.pop_value()?;
                self.stack.push(Value::Bool(!self.is_truthy(&value)));
            }
            Opcode::BitNot => {
                let value = self.pop_value()?;
                self.stack
                    .push(Value::Number(f64::from(!self.to_int32(&value))));
            }
            Opcode::ToNumber => {
                let value = self.pop_value()?;
                let n = self.to_number_coerced(&value)?;
                self.stack.push(Value::Number(n));
            }
            Opcode::Typeof => {
                let value = self.pop_value()?;
                let type_name = self.typeof_name(&value).to_string();
                let string_value = self.make_string_value(&type_name);
                self.stack.push(string_value);
            }
            Opcode::Void => {
                let _ = self.pop_value()?;
                self.stack.push(Value::Undefined);
            }
            Opcode::Delete => {
                let _ = self.pop_value()?;
                self.stack.push(Value::Bool(true));
            }
            Opcode::DeleteProp => {
                let key = self.pop_value()?;
                let object = self.pop_value()?;
                let key = self.to_property_key(&key)?;
                let result = match object {
                    Value::Object(object) => self.delete_property(object, &key),
                    _ => true,
                };
                self.stack.push(Value::Bool(result));
            }
            Opcode::DefineGetter => {
                let function = self.pop_value()?;
                let key = self.pop_value()?;
                let object = self.pop_value()?;
                let key = self.to_property_key(&key)?;
                if let (Value::Object(object), Value::Object(function)) = (&object, &function) {
                    self.define_accessor(*object, key, *function, true);
                }
            }
            Opcode::DefineSetter => {
                let function = self.pop_value()?;
                let key = self.pop_value()?;
                let object = self.pop_value()?;
                let key = self.to_property_key(&key)?;
                if let (Value::Object(object), Value::Object(function)) = (&object, &function) {
                    self.define_accessor(*object, key, *function, false);
                }
            }
            Opcode::In => {
                let object = self.pop_value()?;
                let key = self.pop_value()?;
                let key = self.to_property_key(&key)?;
                let object = self.require_object_ref(&object, "in operator")?;
                let proxy = self.heap.objects().get(object).and_then(|o| match &o.kind {
                    ObjectKind::Proxy { target, handler } => Some((*target, *handler)),
                    _ => None,
                });
                let present = match proxy {
                    Some((target, handler)) => self.proxy_has(target, handler, &key)?,
                    None => {
                        self.lookup_property_descriptor(object, &key).is_some()
                            || self.host_has_event_handler_property(object, &key)
                    }
                };
                self.stack.push(Value::Bool(present));
            }
            Opcode::Instanceof => {
                let constructor = self.pop_value()?;
                let value = self.pop_value()?;
                // A custom `Symbol.hasInstance` on the RHS overrides the default
                // prototype-chain check (e.g. `n instanceof Even`).
                let has_instance = if matches!(constructor, Value::Object(_)) {
                    self.get_property_value(
                        &constructor,
                        &PropertyKey::Symbol(SymbolId(SYMBOL_HAS_INSTANCE_ID)),
                    )?
                } else {
                    Value::Undefined
                };
                let result = if self.is_callable_value(&has_instance) {
                    let r = self.call_value_sync(has_instance, constructor.clone(), vec![value])?;
                    self.is_truthy(&r)
                } else {
                    self.instanceof_value(&value, &constructor)?
                };
                self.stack.push(Value::Bool(result));
            }
            Opcode::Jump(offset) => {
                self.apply_jump(offset)?;
            }
            Opcode::JumpIfTrue(offset) => {
                if self.is_truthy(self.peek_value()?) {
                    self.apply_jump(offset)?;
                }
            }
            Opcode::JumpIfFalse(offset) => {
                if !self.is_truthy(self.peek_value()?) {
                    self.apply_jump(offset)?;
                }
            }
            Opcode::JumpIfTruePop(offset) => {
                let value = self.pop_value()?;
                if self.is_truthy(&value) {
                    self.apply_jump(offset)?;
                }
            }
            Opcode::JumpIfFalsePop(offset) => {
                let value = self.pop_value()?;
                if !self.is_truthy(&value) {
                    self.apply_jump(offset)?;
                }
            }
            Opcode::JumpIfNullish(offset) => {
                let value = self.peek_value()?;
                if matches!(value, Value::Null | Value::Undefined) {
                    self.apply_jump(offset)?;
                }
            }
            Opcode::Call(argc) => {
                let args = self.pop_args(argc)?;
                let this_value = self.pop_value()?;
                let callee = self.pop_value()?;
                let result = self.invoke_callable_value(callee, this_value, args);
                // Best-effort diagnostic only: nested calls in argument position can
                // overwrite this name before we reach resolve_callable.
                self.pending_call_name = None;
                if let Some(result) = result? {
                    self.stack.push(result);
                }
            }
            Opcode::Await => {
                let awaited = self.pop_value()?;
                self.suspend_current_async_frame(awaited)?;
            }
            Opcode::CallSpread(_) | Opcode::Spread | Opcode::GetSuperCtor => {
                return Err(VmError::Unimplemented("phase 4 opcode"));
            }
            Opcode::MakeRegExp(index) => {
                let (pattern, flags) = self.constant_regexp(index)?;
                let object = self.make_regexp_object(&pattern, &flags);
                self.stack.push(object);
            }
            Opcode::CopyDataProperties => {
                let source = self.pop_value()?;
                let target = self.pop_value()?;
                if !matches!(source, Value::Null | Value::Undefined) {
                    let target_ref = self.require_object_ref(&target, "object spread target")?;
                    if let Value::Object(source_ref) = source {
                        let keys = self.object_own_enumerable_keys(source_ref);
                        for key in keys {
                            let value =
                                self.get_property_value(&Value::Object(source_ref), &key)?;
                            self.set_property_on_object(
                                target_ref,
                                Value::Object(target_ref),
                                key,
                                value,
                            )?;
                        }
                    }
                }
                self.stack.push(target);
            }
            Opcode::GetForInKeys => {
                let value = self.pop_value()?;
                let keys = match value {
                    Value::Object(object) => self.for_in_keys(object),
                    Value::String(string) => self
                        .string_text(string)
                        .chars()
                        .enumerate()
                        .map(|(index, _)| index.to_string())
                        .collect(),
                    Value::Null | Value::Undefined => Vec::new(),
                    _ => Vec::new(),
                };
                let values = keys
                    .into_iter()
                    .map(|key| self.make_string_value(&key))
                    .collect();
                let array = self.make_array_from_values(values)?;
                self.stack.push(array);
            }
            Opcode::GetForOfIterator => {
                let value = self.pop_value()?;
                let values = self.for_of_values(&value)?;
                let iterator = self.heap.allocate_object(JsObject {
                    kind: ObjectKind::ForOfIterator { values, index: 0 },
                    prototype: Some(self.object_prototype_ref()),
                    ..JsObject::default()
                });
                self.stack.push(Value::Object(iterator));
            }
            Opcode::GetForAwaitIterator => {
                let value = self.pop_value()?;
                if let Ok(async_iter) = self.get_property_value(
                    &value,
                    &PropertyKey::Symbol(SymbolId(SYMBOL_ASYNC_ITERATOR_ID)),
                ) {
                    if self.is_callable_value(&async_iter) {
                        let iterator = self.call_value_sync(async_iter, value.clone(), Vec::new())?;
                        self.stack.push(iterator);
                    } else {
                        let iterator = self.allocate_for_of_iterator_adapter(&value)?;
                        self.stack.push(Value::Object(iterator));
                    }
                } else {
                    let iterator = self.allocate_for_of_iterator_adapter(&value)?;
                    self.stack.push(Value::Object(iterator));
                }
            }
            Opcode::ForOfNext => {
                let iterator = self.pop_value()?;
                let iterator_ref = self.require_object_ref(&iterator, "for...of iterator")?;
                let next = self.for_of_next(iterator_ref)?;
                let done = next.is_none();
                self.stack.push(next.unwrap_or(Value::Undefined));
                self.stack.push(Value::Bool(done));
            }
            Opcode::GetProto => {
                let value = self.pop_value()?;
                let object = self.require_object_ref(&value, "prototype lookup")?;
                let proto = self
                    .heap
                    .objects()
                    .get(object)
                    .and_then(|object| object.prototype)
                    .map(Value::Object)
                    .unwrap_or(Value::Null);
                self.stack.push(proto);
            }
            Opcode::SetProtoOf => {
                let proto = self.pop_value()?;
                let value = self.pop_value()?;
                let object = self.require_object_ref(&value, "setPrototypeOf")?;
                let prototype = match proto {
                    Value::Null => None,
                    Value::Object(object) => Some(object),
                    _ => {
                        return Err(VmError::TypeError(
                            "prototype must be an object or null".to_string(),
                        ));
                    }
                };
                if let Some(object_data) = self.heap.objects_mut().get_mut(object) {
                    object_data.prototype = prototype;
                }
                self.stack.push(Value::Object(object));
            }
            Opcode::SetObjectLiteralProto => {
                let value = self.pop_value()?;
                let object = match self.pop_value()? {
                    Value::Object(object) => object,
                    _ => return Ok(()),
                };
                let prototype = match value {
                    Value::Object(object) => Some(object),
                    Value::Null => None,
                    _ => return Ok(()),
                };
                if let Some(object_data) = self.heap.objects_mut().get_mut(object) {
                    object_data.prototype = prototype;
                }
            }
            Opcode::EnterTry(_) | Opcode::LeaveTry => {}
            Opcode::EndFinally => {
                if let Some(frame) = self.frames.last_mut()
                    && let Some(value) = frame.pending_exception.take()
                {
                    return Err(VmError::Thrown(value));
                }
            }
            Opcode::Return => {
                let mut value = self.pop_value()?;
                let frame = self
                    .frames
                    .pop()
                    .ok_or_else(|| VmError::RangeError("return without a frame".to_string()))?;
                if let Some(generator) = frame.generator {
                    // A generator body returning marks the generator complete.
                    self.stack.truncate(frame.stack_base);
                    self.set_generator_state(generator, GeneratorState::Completed);
                    self.generator_outcome = Some(GeneratorOutcome::Returned(value));
                    return Ok(());
                }
                if let Some(outer_promise) = frame.async_outer_promise {
                    self.stack.truncate(frame.stack_base);
                    self.resolve_promise_from_resolution(outer_promise, value)?;
                    return Ok(());
                }
                self.stack.truncate(frame.stack_base);
                if let Some(fallback) = frame.construct_fallback {
                    if !matches!(value, Value::Object(_)) {
                        value = fallback;
                    }
                }
                self.stack.push(value);
            }
            Opcode::AsyncReturn => {
                let result = self.pop_value()?;
                self.finish_async_frame_with_result(result)?;
            }
            Opcode::Yield => {
                let value = self.pop_value()?;
                let frame = self
                    .frames
                    .pop()
                    .ok_or_else(|| VmError::RangeError("yield without a frame".to_string()))?;
                let generator = frame.generator.ok_or_else(|| {
                    VmError::TypeError("yield is only valid inside a generator".to_string())
                })?;
                if frame.async_gen_request.is_some() {
                    let request = frame.async_gen_request.ok_or_else(|| {
                        VmError::TypeError("async generator request missing".to_string())
                    })?;
                    let (_, queue) = self.take_async_generator_state(generator).ok_or_else(|| {
                        VmError::TypeError("yield is only valid inside a generator".to_string())
                    })?;
                    let stack = self.stack.split_off(frame.stack_base);
                    let mut frame = frame;
                    frame.async_gen_request = None;
                    self.set_async_generator_state(
                        generator,
                        GeneratorState::Suspended {
                            frame: Box::new(frame),
                            stack,
                            started: true,
                        },
                        queue,
                    );
                    let result = self.make_iter_result(value, false)?;
                    self.resolve_promise_from_resolution(request, result)?;
                    if let Some((GeneratorState::Suspended { frame, stack, started }, mut queue)) =
                        self.take_async_generator_state(generator)
                    {
                        if let Some(next_request) = queue.pop_front() {
                            let remaining_queue = queue;
                            let _ = self.resume_async_generator_suspended(
                                generator,
                                next_request,
                                frame,
                                stack,
                                started,
                                remaining_queue,
                            )?;
                        } else {
                            self.set_async_generator_state(
                                generator,
                                GeneratorState::Suspended { frame, stack, started },
                                queue,
                            );
                        }
                    }
                } else {
                    let stack = self.stack.split_off(frame.stack_base);
                    self.set_generator_state(
                        generator,
                        GeneratorState::Suspended {
                            frame: Box::new(frame),
                            stack,
                            started: true,
                        },
                    );
                    self.generator_outcome = Some(GeneratorOutcome::Yielded(value));
                }
            }
            Opcode::MakeClosure(index) => {
                let proto = self
                    .current_proto()?
                    .nested_functions
                    .get(index as usize)
                    .cloned()
                    .ok_or_else(|| {
                        VmError::RangeError(format!("function proto index {index} out of range"))
                    })?;
                let mut upvalues = Vec::with_capacity(proto.upvalue_descriptors.len());
                for descriptor in &proto.upvalue_descriptors {
                    let cell = if descriptor.is_local {
                        self.local_cell(descriptor.index)?.clone()
                    } else {
                        self.upvalue_cell(descriptor.index)?.clone()
                    };
                    upvalues.push(cell);
                }
                let value = self.allocate_function_value(RuntimeClosure {
                    proto: Rc::new(proto),
                    upvalues,
                });
                self.stack.push(value);
            }
            Opcode::MakeObject => {
                let object = self.allocate_ordinary_object(Some(self.object_prototype_ref()));
                self.stack.push(Value::Object(object));
            }
            Opcode::MakeArray(count) => {
                let elements = self.pop_args_u16(count)?;
                let array = self.make_array_from_values(elements)?;
                self.stack.push(array);
            }
            Opcode::GetProp => {
                let key = self.pop_value()?;
                let object = self.pop_value()?;
                let value = self.get_property_value(&object, &self.to_property_key(&key)?)?;
                self.stack.push(value);
            }
            Opcode::SetProp => {
                let value = self.pop_value()?;
                let key = self.pop_value()?;
                let object = self.pop_value()?;
                self.set_property_value(&object, self.to_property_key(&key)?, value)?;
            }
            Opcode::GetIndex => {
                let key = self.pop_value()?;
                let object = self.pop_value()?;
                let value = self.get_property_value(&object, &self.to_property_key(&key)?)?;
                self.stack.push(value);
            }
            Opcode::SetIndex => {
                let value = self.pop_value()?;
                let key = self.pop_value()?;
                let object = self.pop_value()?;
                self.set_property_value(&object, self.to_property_key(&key)?, value)?;
            }
            Opcode::GetPropForCall(index) => {
                let object = self.pop_value()?;
                let key = {
                    let name = self.constant_name(index)?.to_string();
                    self.pending_call_name = Some(name.clone());
                    PropertyKey::from(name)
                };
                let callee = self.get_property_value(&object, &key)?;
                self.stack.push(callee);
                self.stack.push(object);
            }
            Opcode::GetIndexForCall => {
                let key = self.pop_value()?;
                let object = self.pop_value()?;
                self.pending_call_name = match &key {
                    Value::String(string) => Some(self.string_text(*string)),
                    Value::Number(number) => Some(self.to_string(&Value::Number(*number))),
                    _ => None,
                };
                let callee = self.get_property_value(&object, &self.to_property_key(&key)?)?;
                self.stack.push(callee);
                self.stack.push(object);
            }
            Opcode::New(argc) => {
                let args = self.pop_args(argc)?;
                let constructor = self.pop_value()?;
                let result = self.construct_value(constructor, args);
                self.pending_call_name = None;
                if let Some(result) = result? {
                    self.stack.push(result);
                }
            }
            Opcode::Throw => {
                let thrown = self.pop_value()?;
                return Err(VmError::Thrown(thrown));
            }
            Opcode::DynamicImport => {
                let value = self.pop_value()?;
                let promise = match value {
                    Value::Undefined => {
                        let err = self.create_error_object(
                            "TypeError",
                            "Failed to resolve dynamically imported module".to_string(),
                        );
                        self.promise_reject_value(err)?
                    }
                    other => self.promise_resolve_value(other)?,
                };
                self.stack.push(Value::Object(promise));
            }
            Opcode::Nop => {}
        }
        Ok(())
    }

    fn install_globals(&mut self) {
        self.globals
            .insert("undefined".to_string(), Value::Undefined);
        self.globals
            .insert("NaN".to_string(), Value::Number(f64::NAN));

        let object_prototype = self.allocate_ordinary_object(None);
        let function_prototype = self.heap.allocate_object(JsObject {
            kind: ObjectKind::Function,
            prototype: Some(object_prototype),
            ..JsObject::default()
        });
        let array_prototype = self.heap.allocate_object(JsObject {
            kind: ObjectKind::Array,
            prototype: Some(object_prototype),
            ..JsObject::default()
        });
        let string_prototype = self.allocate_ordinary_object(Some(object_prototype));
        let number_prototype = self.allocate_ordinary_object(Some(object_prototype));
        let boolean_prototype = self.allocate_ordinary_object(Some(object_prototype));
        let regexp_prototype = self.allocate_ordinary_object(Some(object_prototype));
        let date_prototype = self.allocate_ordinary_object(Some(object_prototype));
        let generator_prototype = self.allocate_ordinary_object(Some(object_prototype));
        let async_generator_prototype = self.allocate_ordinary_object(Some(object_prototype));
        let url_search_params_prototype = self.allocate_ordinary_object(Some(object_prototype));
        let headers_prototype = self.allocate_ordinary_object(Some(object_prototype));
        let form_data_prototype = self.allocate_ordinary_object(Some(object_prototype));
        let weak_ref_prototype = self.allocate_ordinary_object(Some(object_prototype));
        let text_encoder_prototype = self.allocate_ordinary_object(Some(object_prototype));
        let text_decoder_prototype = self.allocate_ordinary_object(Some(object_prototype));
        let url_prototype = self.allocate_ordinary_object(Some(object_prototype));
        let error_prototype = self.heap.allocate_object(JsObject {
            kind: ObjectKind::Error,
            prototype: Some(object_prototype),
            ..JsObject::default()
        });
        let promise_prototype = self.allocate_ordinary_object(Some(object_prototype));
        let map_prototype = self.allocate_ordinary_object(Some(object_prototype));
        let set_prototype = self.allocate_ordinary_object(Some(object_prototype));
        let array_buffer_prototype = self.allocate_ordinary_object(Some(object_prototype));
        let typed_array_prototype = self.allocate_ordinary_object(Some(object_prototype));

        self.object_prototype = Some(object_prototype);
        self.function_prototype = Some(function_prototype);
        self.array_prototype = Some(array_prototype);
        self.string_prototype = Some(string_prototype);
        self.number_prototype = Some(number_prototype);
        self.boolean_prototype = Some(boolean_prototype);
        self.regexp_prototype = Some(regexp_prototype);
        self.date_prototype = Some(date_prototype);
        self.generator_prototype = Some(generator_prototype);
        self.async_generator_prototype = Some(async_generator_prototype);
        self.url_search_params_prototype = Some(url_search_params_prototype);
        self.headers_prototype = Some(headers_prototype);
        self.form_data_prototype = Some(form_data_prototype);
        self.weak_ref_prototype = Some(weak_ref_prototype);
        self.text_encoder_prototype = Some(text_encoder_prototype);
        self.text_decoder_prototype = Some(text_decoder_prototype);
        self.url_prototype = Some(url_prototype);
        self.error_prototype = Some(error_prototype);
        self.promise_prototype = Some(promise_prototype);
        self.map_prototype = Some(map_prototype);
        self.set_prototype = Some(set_prototype);
        self.array_buffer_prototype = Some(array_buffer_prototype);
        self.typed_array_prototype = Some(typed_array_prototype);

        let assert_value = self.allocate_builtin_method(BuiltinId::Assert);
        self.globals.insert("assert".to_string(), assert_value);
        let call_spread = self.allocate_builtin_method(BuiltinId::CallSpread);
        let construct_spread = self.allocate_builtin_method(BuiltinId::ConstructSpread);
        let queue_microtask = self.allocate_builtin_method(BuiltinId::QueueMicrotask);
        let set_timeout = self.allocate_builtin_method(BuiltinId::SetTimeout);
        let clear_timeout = self.allocate_builtin_method(BuiltinId::ClearTimeout);
        let set_interval = self.allocate_builtin_method(BuiltinId::SetInterval);
        let clear_interval = self.allocate_builtin_method(BuiltinId::ClearInterval);
        let request_animation_frame =
            self.allocate_builtin_method(BuiltinId::RequestAnimationFrame);
        let cancel_animation_frame =
            self.allocate_builtin_method(BuiltinId::CancelAnimationFrame);
        self.globals
            .insert("__callSpread".to_string(), call_spread.clone());
        self.globals
            .insert("__constructSpread".to_string(), construct_spread.clone());
        self.globals
            .insert("queueMicrotask".to_string(), queue_microtask);
        self.globals.insert("setTimeout".to_string(), set_timeout);
        self.globals
            .insert("clearTimeout".to_string(), clear_timeout);
        self.globals.insert("setInterval".to_string(), set_interval);
        self.globals
            .insert("clearInterval".to_string(), clear_interval);
        self.globals
            .insert("requestAnimationFrame".to_string(), request_animation_frame);
        self.globals
            .insert("cancelAnimationFrame".to_string(), cancel_animation_frame);

        let object_ctor =
            self.allocate_builtin_value(BuiltinId::ObjectConstructor, true, Some(object_prototype));
        let array_ctor =
            self.allocate_builtin_value(BuiltinId::ArrayConstructor, true, Some(array_prototype));
        let function_ctor = self.allocate_builtin_value(
            BuiltinId::FunctionConstructor,
            true,
            Some(function_prototype),
        );
        let error_ctor =
            self.allocate_builtin_value(BuiltinId::ErrorConstructor, true, Some(error_prototype));
        let type_error_ctor = self.allocate_builtin_value(
            BuiltinId::TypeErrorConstructor,
            true,
            Some(error_prototype),
        );
        let range_error_ctor = self.allocate_builtin_value(
            BuiltinId::RangeErrorConstructor,
            true,
            Some(error_prototype),
        );
        let reference_error_ctor = self.allocate_builtin_value(
            BuiltinId::ReferenceErrorConstructor,
            true,
            Some(error_prototype),
        );
        let syntax_error_ctor = self.allocate_builtin_value(
            BuiltinId::SyntaxErrorConstructor,
            true,
            Some(error_prototype),
        );
        let uri_error_ctor = self.allocate_builtin_value(
            BuiltinId::UriErrorConstructor,
            true,
            Some(error_prototype),
        );
        let eval_error_ctor = self.allocate_builtin_value(
            BuiltinId::EvalErrorConstructor,
            true,
            Some(error_prototype),
        );
        let map_ctor =
            self.allocate_builtin_value(BuiltinId::MapConstructor, true, Some(map_prototype));
        let set_ctor =
            self.allocate_builtin_value(BuiltinId::SetConstructor, true, Some(set_prototype));
        let weak_map_ctor =
            self.allocate_builtin_value(BuiltinId::WeakMapConstructor, true, Some(map_prototype));
        let weak_set_ctor =
            self.allocate_builtin_value(BuiltinId::WeakSetConstructor, true, Some(set_prototype));
        let promise_ctor = self.allocate_callable_value(
            Callable::Builtin(BuiltinId::PromiseConstructor),
            true,
            Some(promise_prototype),
        );
        let number_ctor =
            self.allocate_builtin_value(BuiltinId::NumberConstructor, true, Some(number_prototype));
        let string_ctor =
            self.allocate_builtin_value(BuiltinId::StringConstructor, true, Some(string_prototype));
        let boolean_ctor = self.allocate_builtin_value(
            BuiltinId::BooleanConstructor,
            true,
            Some(boolean_prototype),
        );
        let regexp_ctor =
            self.allocate_builtin_value(BuiltinId::RegExpConstructor, true, Some(regexp_prototype));
        // Symbol is callable but not constructable.
        let symbol_ctor = self.allocate_builtin_value(BuiltinId::SymbolConstructor, false, None);
        let date_ctor =
            self.allocate_builtin_value(BuiltinId::DateConstructor, true, Some(date_prototype));
        let math_object = self.allocate_ordinary_object(Some(object_prototype));
        let json_object = self.allocate_ordinary_object(Some(object_prototype));

        self.globals
            .insert("Object".to_string(), object_ctor.clone());
        self.globals
            .insert("Function".to_string(), function_ctor.clone());
        self.globals.insert("Array".to_string(), array_ctor.clone());
        self.globals.insert("Error".to_string(), error_ctor.clone());
        self.globals
            .insert("TypeError".to_string(), type_error_ctor.clone());
        self.globals
            .insert("RangeError".to_string(), range_error_ctor.clone());
        self.globals
            .insert("ReferenceError".to_string(), reference_error_ctor.clone());
        self.globals
            .insert("SyntaxError".to_string(), syntax_error_ctor.clone());
        self.globals
            .insert("URIError".to_string(), uri_error_ctor.clone());
        self.globals
            .insert("EvalError".to_string(), eval_error_ctor.clone());
        self.globals.insert("Map".to_string(), map_ctor.clone());
        self.globals.insert("Set".to_string(), set_ctor.clone());
        self.globals
            .insert("WeakMap".to_string(), weak_map_ctor.clone());
        self.globals
            .insert("WeakSet".to_string(), weak_set_ctor.clone());

        // ArrayBuffer + typed arrays.
        let array_buffer_ctor = self.allocate_builtin_value(
            BuiltinId::ArrayBufferConstructor,
            true,
            Some(array_buffer_prototype),
        );
        self.globals
            .insert("ArrayBuffer".to_string(), array_buffer_ctor);
        self.define_builtin_method(
            array_buffer_prototype,
            "slice",
            BuiltinId::ArrayBufferProtoSlice,
        );
        // Shared %TypedArray%.prototype methods (each reads its element kind
        // from `this`). length/byteLength/byteOffset/buffer/BYTES_PER_ELEMENT
        // and indexed access are handled directly in the get/set hooks.
        self.define_builtin_method(typed_array_prototype, "set", BuiltinId::TypedArrayProtoSet);
        self.define_builtin_method(
            typed_array_prototype,
            "subarray",
            BuiltinId::TypedArrayProtoSubarray,
        );
        self.define_builtin_method(
            typed_array_prototype,
            "slice",
            BuiltinId::TypedArrayProtoSlice,
        );
        self.define_builtin_method(typed_array_prototype, "fill", BuiltinId::TypedArrayProtoFill);
        self.define_builtin_method(typed_array_prototype, "join", BuiltinId::TypedArrayProtoJoin);
        self.define_builtin_method(
            typed_array_prototype,
            "indexOf",
            BuiltinId::TypedArrayProtoIndexOf,
        );
        self.define_builtin_method(
            typed_array_prototype,
            "includes",
            BuiltinId::TypedArrayProtoIncludes,
        );
        self.define_builtin_method(
            typed_array_prototype,
            "forEach",
            BuiltinId::TypedArrayProtoForEach,
        );
        self.define_builtin_method(typed_array_prototype, "map", BuiltinId::TypedArrayProtoMap);
        self.define_builtin_method(
            typed_array_prototype,
            "reduce",
            BuiltinId::TypedArrayProtoReduce,
        );
        self.define_builtin_method(
            typed_array_prototype,
            "reverse",
            BuiltinId::TypedArrayProtoReverse,
        );

        self.define_builtin_method(weak_ref_prototype, "deref", BuiltinId::WeakRefDeref);
        let weak_ref_ctor =
            self.allocate_builtin_value(BuiltinId::WeakRefConstructor, true, Some(weak_ref_prototype));
        self.globals.insert("WeakRef".to_string(), weak_ref_ctor);

        self.define_builtin_method(text_encoder_prototype, "encode", BuiltinId::TextEncoderEncode);
        let text_encoder_ctor = self.allocate_builtin_value(
            BuiltinId::TextEncoderConstructor,
            true,
            Some(text_encoder_prototype),
        );
        self.globals
            .insert("TextEncoder".to_string(), text_encoder_ctor);

        self.define_builtin_method(text_decoder_prototype, "decode", BuiltinId::TextDecoderDecode);
        let text_decoder_ctor = self.allocate_builtin_value(
            BuiltinId::TextDecoderConstructor,
            true,
            Some(text_decoder_prototype),
        );
        self.globals
            .insert("TextDecoder".to_string(), text_decoder_ctor);

        for kind in [
            TypedArrayKind::Int8,
            TypedArrayKind::Uint8,
            TypedArrayKind::Uint8Clamped,
            TypedArrayKind::Int16,
            TypedArrayKind::Uint16,
            TypedArrayKind::Int32,
            TypedArrayKind::Uint32,
            TypedArrayKind::Float32,
            TypedArrayKind::Float64,
        ] {
            let ctor = self.allocate_builtin_value(
                BuiltinId::TypedArrayConstructor(kind),
                true,
                Some(typed_array_prototype),
            );
            if let Some(ctor_ref) = self.value_object_ref(ctor.clone()) {
                self.define_builtin_method(ctor_ref, "from", BuiltinId::TypedArrayFrom(kind));
                self.define_builtin_method(ctor_ref, "of", BuiltinId::TypedArrayOf(kind));
                self.define_data_property(
                    ctor_ref,
                    PropertyKey::from("BYTES_PER_ELEMENT"),
                    Value::Number(kind.bytes_per_element() as f64),
                    false,
                    false,
                    false,
                );
            }
            self.globals
                .insert(kind.constructor_name().to_string(), ctor);
        }

        // Event / CustomEvent constructors.
        let event_ctor = self.allocate_builtin_value(BuiltinId::EventConstructor, true, None);
        self.globals.insert("Event".to_string(), event_ctor);
        let custom_event_ctor =
            self.allocate_builtin_value(BuiltinId::CustomEventConstructor, true, None);
        self.globals
            .insert("CustomEvent".to_string(), custom_event_ctor);
        // UI event constructors. FocusEvent/InputEvent/UIEvent carry no extra
        // init we model, so the plain Event shape stands in for them.
        let keyboard_event_ctor =
            self.allocate_builtin_value(BuiltinId::KeyboardEventConstructor, true, None);
        self.globals
            .insert("KeyboardEvent".to_string(), keyboard_event_ctor);
        for name in ["MouseEvent", "PointerEvent"] {
            let ctor = self.allocate_builtin_value(BuiltinId::MouseEventConstructor, true, None);
            self.globals.insert(name.to_string(), ctor);
        }
        for name in ["UIEvent", "FocusEvent", "InputEvent"] {
            let ctor = self.allocate_builtin_value(BuiltinId::EventConstructor, true, None);
            self.globals.insert(name.to_string(), ctor);
        }

        // AbortController constructor.
        let abort_controller_ctor =
            self.allocate_builtin_value(BuiltinId::AbortControllerConstructor, true, None);
        self.globals
            .insert("AbortController".to_string(), abort_controller_ctor);

        // AbortSignal global object.
        let abort_signal_ctor = self.allocate_builtin_value(BuiltinId::AbortSignalConstructor, false, None);
        if let Value::Object(abort_signal_ref) = &abort_signal_ctor {
            let abort_method = self.allocate_builtin_method(BuiltinId::AbortSignalAbortStatic);
            self.define_data_property(
                *abort_signal_ref,
                PropertyKey::from("abort"),
                abort_method,
                true,
                false,
                true,
            );
            let timeout_method = self.allocate_builtin_method(BuiltinId::AbortSignalTimeoutStatic);
            self.define_data_property(
                *abort_signal_ref,
                PropertyKey::from("timeout"),
                timeout_method,
                true,
                false,
                true,
            );
            let any_method = self.allocate_builtin_method(BuiltinId::AbortSignalAnyStatic);
            self.define_data_property(
                *abort_signal_ref,
                PropertyKey::from("any"),
                any_method,
                true,
                false,
                true,
            );
        }
        self.globals
            .insert("AbortSignal".to_string(), abort_signal_ctor);

        // MutationObserver constructor.
        let mutation_observer_ctor =
            self.allocate_builtin_value(BuiltinId::MutationObserverConstructor, true, None);
        self.globals
            .insert("MutationObserver".to_string(), mutation_observer_ctor);

        // XMLHttpRequest constructor.
        let xhr_ctor = self.allocate_builtin_value(BuiltinId::XhrConstructor, true, None);
        self.globals
            .insert("XMLHttpRequest".to_string(), xhr_ctor);

        // Image constructor.
        let image_ctor = self.allocate_builtin_value(BuiltinId::ImageConstructor, true, None);
        self.globals.insert("Image".to_string(), image_ctor);

        // IntersectionObserver constructor.
        let intersection_observer_ctor =
            self.allocate_builtin_value(BuiltinId::IntersectionObserverConstructor, true, None);
        self.globals
            .insert("IntersectionObserver".to_string(), intersection_observer_ctor);

        // ResizeObserver constructor.
        let resize_observer_ctor =
            self.allocate_builtin_value(BuiltinId::ResizeObserverConstructor, true, None);
        self.globals
            .insert("ResizeObserver".to_string(), resize_observer_ctor);

        self.globals
            .insert("Promise".to_string(), promise_ctor.clone());
        self.globals.insert("Number".to_string(), number_ctor.clone());
        self.globals.insert("String".to_string(), string_ctor.clone());
        self.globals
            .insert("Boolean".to_string(), boolean_ctor.clone());
        self.globals
            .insert("RegExp".to_string(), regexp_ctor.clone());
        self.globals
            .insert("Symbol".to_string(), symbol_ctor.clone());
        self.globals.insert("Date".to_string(), date_ctor.clone());
        self.globals
            .insert("Math".to_string(), Value::Object(math_object));
        self.globals
            .insert("JSON".to_string(), Value::Object(json_object));

        // document and window host objects
        let document_obj = self.make_host_object(HostObjectSlot {
            class: HostObjectClass::Document,
            interface_name: "HTMLDocument",
            handle: 0,
            dispatch: HostDispatch::Ordinary,
            supports_indexed_properties: false,
            supports_named_properties: false,
        });
        let window_obj = self.make_host_object(HostObjectSlot {
            class: HostObjectClass::Window,
            interface_name: "Window",
            handle: 0,
            dispatch: HostDispatch::Ordinary,
            supports_indexed_properties: false,
            supports_named_properties: false,
        });
        self.globals.insert("document".to_string(), document_obj.clone());
        self.globals.insert("window".to_string(), window_obj.clone());
        self.globals.insert("globalThis".to_string(), window_obj.clone());
        // `self` is an alias for the global object (Window), NOT the document.
        // UMD bundles resolve their global via `global || self` and then attach
        // their exports to it (e.g. `self.React = {}`); pointing `self` at the
        // document meant those exports never landed on the real global.
        self.globals.insert("self".to_string(), window_obj);

        // DOM interface constructors so `node instanceof Element` (and friends)
        // works. Host DOM nodes don't have prototype chains linked to these, so
        // `instanceof_value` recognizes them by interface name; here we just make
        // each constructor exist as an object with a `.prototype`, register it as
        // a global, and remember its ref→interface mapping.
        for &iface in DOM_INTERFACE_NAMES {
            let ctor = self.allocate_ordinary_object(Some(object_prototype));
            let proto = self.allocate_ordinary_object(Some(object_prototype));
            self.define_data_property(
                ctor,
                PropertyKey::from("prototype"),
                Value::Object(proto),
                false,
                false,
                false,
            );
            self.dom_interface_ctors.insert(ctor.raw(), iface);
            self.globals.insert(iface.to_string(), Value::Object(ctor));
        }

        // Encoding globals
        let btoa = self.allocate_builtin_method(BuiltinId::Btoa);
        let atob = self.allocate_builtin_method(BuiltinId::Atob);
        self.globals.insert("btoa".to_string(), btoa);
        self.globals.insert("atob".to_string(), atob);

        // Idle callback globals
        let request_idle = self.allocate_builtin_method(BuiltinId::RequestIdleCallback);
        let cancel_idle = self.allocate_builtin_method(BuiltinId::CancelIdleCallback);
        self.globals.insert("requestIdleCallback".to_string(), request_idle);
        self.globals.insert("cancelIdleCallback".to_string(), cancel_idle);

        // customElements registry
        let custom_elements_object = self.allocate_ordinary_object(Some(object_prototype));
        self.define_builtin_method(custom_elements_object, "define", BuiltinId::CustomElementsDefine);
        self.define_builtin_method(custom_elements_object, "get", BuiltinId::CustomElementsGet);
        self.globals.insert(
            "customElements".to_string(),
            Value::Object(custom_elements_object),
        );

        // console object
        let console_object = self.allocate_ordinary_object(Some(object_prototype));
        self.define_builtin_method(console_object, "log",   BuiltinId::ConsoleLog);
        self.define_builtin_method(console_object, "info",  BuiltinId::ConsoleInfo);
        self.define_builtin_method(console_object, "warn",  BuiltinId::ConsoleWarn);
        self.define_builtin_method(console_object, "error", BuiltinId::ConsoleError);
        self.globals
            .insert("console".to_string(), Value::Object(console_object));

        self.define_builtin_method(
            object_prototype,
            "hasOwnProperty",
            BuiltinId::ObjectProtoHasOwnProperty,
        );
        self.define_builtin_method(
            object_prototype,
            "propertyIsEnumerable",
            BuiltinId::ObjectProtoPropertyIsEnumerable,
        );
        self.define_builtin_method(object_prototype, "toString", BuiltinId::ObjectProtoToString);
        self.define_builtin_method(object_prototype, "valueOf", BuiltinId::ObjectProtoValueOf);
        self.define_builtin_method(
            object_prototype,
            "isPrototypeOf",
            BuiltinId::ObjectProtoIsPrototypeOf,
        );

        self.define_builtin_method(function_prototype, "call", BuiltinId::FunctionProtoCall);
        self.define_builtin_method(function_prototype, "apply", BuiltinId::FunctionProtoApply);
        self.define_builtin_method(function_prototype, "bind", BuiltinId::FunctionProtoBind);

        self.define_builtin_method(promise_prototype, "then", BuiltinId::PromiseProtoThen);
        self.define_builtin_method(promise_prototype, "catch", BuiltinId::PromiseProtoCatch);
        self.define_builtin_method(promise_prototype, "finally", BuiltinId::PromiseProtoFinally);

        self.define_builtin_method(map_prototype, "set", BuiltinId::MapProtoSet);
        self.define_builtin_method(map_prototype, "get", BuiltinId::MapProtoGet);
        self.define_builtin_method(map_prototype, "has", BuiltinId::MapProtoHas);
        self.define_builtin_method(map_prototype, "delete", BuiltinId::MapProtoDelete);
        self.define_builtin_method(map_prototype, "clear", BuiltinId::MapProtoClear);
        self.define_builtin_method(map_prototype, "forEach", BuiltinId::MapProtoForEach);
        self.define_builtin_method(map_prototype, "entries", BuiltinId::MapProtoEntries);
        self.define_builtin_method(map_prototype, "keys", BuiltinId::MapProtoKeys);
        self.define_builtin_method(map_prototype, "values", BuiltinId::MapProtoValues);

        self.define_builtin_method(set_prototype, "add", BuiltinId::SetProtoAdd);
        self.define_builtin_method(set_prototype, "has", BuiltinId::SetProtoHas);
        self.define_builtin_method(set_prototype, "delete", BuiltinId::SetProtoDelete);
        self.define_builtin_method(set_prototype, "clear", BuiltinId::SetProtoClear);
        self.define_builtin_method(set_prototype, "forEach", BuiltinId::SetProtoForEach);
        self.define_builtin_method(set_prototype, "values", BuiltinId::SetProtoValues);

        self.define_builtin_method(array_prototype, "push", BuiltinId::ArrayProtoPush);
        self.define_builtin_method(array_prototype, "pop", BuiltinId::ArrayProtoPop);
        self.define_builtin_method(array_prototype, "shift", BuiltinId::ArrayProtoShift);
        self.define_builtin_method(array_prototype, "unshift", BuiltinId::ArrayProtoUnshift);
        self.define_builtin_method(array_prototype, "map", BuiltinId::ArrayProtoMap);
        self.define_builtin_method(array_prototype, "filter", BuiltinId::ArrayProtoFilter);
        self.define_builtin_method(array_prototype, "reduce", BuiltinId::ArrayProtoReduce);
        self.define_builtin_method(array_prototype, "forEach", BuiltinId::ArrayProtoForEach);
        self.define_builtin_method(array_prototype, "find", BuiltinId::ArrayProtoFind);
        self.define_builtin_method(array_prototype, "findIndex", BuiltinId::ArrayProtoFindIndex);
        self.define_builtin_method(array_prototype, "indexOf", BuiltinId::ArrayProtoIndexOf);
        self.define_builtin_method(
            array_prototype,
            "lastIndexOf",
            BuiltinId::ArrayProtoLastIndexOf,
        );
        self.define_builtin_method(array_prototype, "includes", BuiltinId::ArrayProtoIncludes);
        self.define_builtin_method(array_prototype, "join", BuiltinId::ArrayProtoJoin);
        // Array.prototype.toString delegates to join(',') — so `'' + [1,2,3]`,
        // `\`${[1,2]}\``, and String([1,2]) yield "1,2,3" rather than
        // "[object Object]" (ToPrimitive falls through to toString for arrays).
        self.define_builtin_method(array_prototype, "toString", BuiltinId::ArrayProtoJoin);
        self.define_builtin_method(array_prototype, "slice", BuiltinId::ArrayProtoSlice);
        self.define_builtin_method(array_prototype, "concat", BuiltinId::ArrayProtoConcat);
        self.define_builtin_method(array_prototype, "flat", BuiltinId::ArrayProtoFlat);
        self.define_builtin_method(array_prototype, "some", BuiltinId::ArrayProtoSome);
        self.define_builtin_method(array_prototype, "every", BuiltinId::ArrayProtoEvery);
        self.define_builtin_method(array_prototype, "sort", BuiltinId::ArrayProtoSort);
        self.define_builtin_method(array_prototype, "reverse", BuiltinId::ArrayProtoReverse);
        self.define_builtin_method(array_prototype, "splice", BuiltinId::ArrayProtoSplice);
        self.define_builtin_method(array_prototype, "flatMap", BuiltinId::ArrayProtoFlatMap);
        self.define_builtin_method(array_prototype, "fill", BuiltinId::ArrayProtoFill);
        self.define_builtin_method(array_prototype, "copyWithin", BuiltinId::ArrayProtoCopyWithin);
        self.define_builtin_method(array_prototype, "at", BuiltinId::ArrayProtoAt);
        self.define_builtin_method(array_prototype, "keys", BuiltinId::ArrayProtoKeys);
        self.define_builtin_method(array_prototype, "values", BuiltinId::ArrayProtoValues);
        self.define_builtin_method(array_prototype, "entries", BuiltinId::ArrayProtoEntries);
        self.define_builtin_method(array_prototype, "reduceRight", BuiltinId::ArrayProtoReduceRight);
        self.define_builtin_method(array_prototype, "findLast", BuiltinId::ArrayProtoFindLast);
        self.define_builtin_method(
            array_prototype,
            "findLastIndex",
            BuiltinId::ArrayProtoFindLastIndex,
        );
        self.define_builtin_method(array_prototype, "toSorted", BuiltinId::ArrayProtoToSorted);
        self.define_builtin_method(array_prototype, "toReversed", BuiltinId::ArrayProtoToReversed);
        self.define_builtin_method(array_prototype, "with", BuiltinId::ArrayProtoWith);

        self.define_builtin_method(string_prototype, "charAt", BuiltinId::StringProtoCharAt);
        self.define_builtin_method(
            string_prototype,
            "charCodeAt",
            BuiltinId::StringProtoCharCodeAt,
        );
        self.define_builtin_method(
            string_prototype,
            "codePointAt",
            BuiltinId::StringProtoCodePointAt,
        );
        self.define_builtin_method(string_prototype, "indexOf", BuiltinId::StringProtoIndexOf);
        self.define_builtin_method(
            string_prototype,
            "lastIndexOf",
            BuiltinId::StringProtoLastIndexOf,
        );
        self.define_builtin_method(string_prototype, "includes", BuiltinId::StringProtoIncludes);
        self.define_builtin_method(
            string_prototype,
            "startsWith",
            BuiltinId::StringProtoStartsWith,
        );
        self.define_builtin_method(string_prototype, "endsWith", BuiltinId::StringProtoEndsWith);
        self.define_builtin_method(string_prototype, "slice", BuiltinId::StringProtoSlice);
        self.define_builtin_method(
            string_prototype,
            "substring",
            BuiltinId::StringProtoSubstring,
        );
        self.define_builtin_method(string_prototype, "split", BuiltinId::StringProtoSplit);
        self.define_builtin_method(string_prototype, "replace", BuiltinId::StringProtoReplace);
        self.define_builtin_method(
            string_prototype,
            "replaceAll",
            BuiltinId::StringProtoReplaceAll,
        );
        self.define_builtin_method(string_prototype, "trim", BuiltinId::StringProtoTrim);
        self.define_builtin_method(
            string_prototype,
            "trimStart",
            BuiltinId::StringProtoTrimStart,
        );
        self.define_builtin_method(string_prototype, "trimEnd", BuiltinId::StringProtoTrimEnd);
        self.define_builtin_method(
            string_prototype,
            "toUpperCase",
            BuiltinId::StringProtoToUpperCase,
        );
        self.define_builtin_method(
            string_prototype,
            "toLowerCase",
            BuiltinId::StringProtoToLowerCase,
        );
        self.define_builtin_method(string_prototype, "padStart", BuiltinId::StringProtoPadStart);
        self.define_builtin_method(string_prototype, "padEnd", BuiltinId::StringProtoPadEnd);
        self.define_builtin_method(string_prototype, "repeat", BuiltinId::StringProtoRepeat);
        self.define_builtin_method(string_prototype, "at", BuiltinId::StringProtoAt);
        self.define_builtin_method(string_prototype, "normalize", BuiltinId::StringProtoNormalize);
        self.define_builtin_method(
            string_prototype,
            "localeCompare",
            BuiltinId::StringProtoLocaleCompare,
        );
        self.define_builtin_method(string_prototype, "concat", BuiltinId::StringProtoConcat);
        self.define_builtin_method(string_prototype, "toString", BuiltinId::StringProtoNormalize);
        self.define_builtin_method(string_prototype, "valueOf", BuiltinId::StringProtoNormalize);

        self.define_builtin_method(number_prototype, "toFixed", BuiltinId::NumberProtoToFixed);
        self.define_builtin_method(number_prototype, "toString", BuiltinId::NumberProtoToString);
        self.define_builtin_method(
            number_prototype,
            "toPrecision",
            BuiltinId::NumberProtoToPrecision,
        );
        self.define_builtin_method(number_prototype, "valueOf", BuiltinId::NumberProtoValueOf);
        self.define_builtin_method(
            number_prototype,
            "toLocaleString",
            BuiltinId::NumberProtoToLocaleString,
        );

        self.define_builtin_method(boolean_prototype, "toString", BuiltinId::BooleanProtoToString);
        self.define_builtin_method(boolean_prototype, "valueOf", BuiltinId::BooleanProtoValueOf);

        self.define_builtin_method(regexp_prototype, "test", BuiltinId::RegExpProtoTest);
        self.define_builtin_method(regexp_prototype, "exec", BuiltinId::RegExpProtoExec);
        self.define_builtin_method(regexp_prototype, "toString", BuiltinId::RegExpProtoToString);

        for (name, builtin) in [
            ("getTime", BuiltinId::DateProtoGetTime),
            ("valueOf", BuiltinId::DateProtoValueOf),
            ("getFullYear", BuiltinId::DateProtoGetFullYear),
            ("getMonth", BuiltinId::DateProtoGetMonth),
            ("getDate", BuiltinId::DateProtoGetDate),
            ("getDay", BuiltinId::DateProtoGetDay),
            ("getHours", BuiltinId::DateProtoGetHours),
            ("getMinutes", BuiltinId::DateProtoGetMinutes),
            ("getSeconds", BuiltinId::DateProtoGetSeconds),
            ("getMilliseconds", BuiltinId::DateProtoGetMilliseconds),
            ("getTimezoneOffset", BuiltinId::DateProtoGetTimezoneOffset),
            ("toISOString", BuiltinId::DateProtoToISOString),
            ("toJSON", BuiltinId::DateProtoToISOString),
            ("toString", BuiltinId::DateProtoToString),
        ] {
            self.define_builtin_method(date_prototype, name, builtin);
        }

        self.define_builtin_method(generator_prototype, "next", BuiltinId::GeneratorProtoNext);
        self.define_builtin_method(generator_prototype, "return", BuiltinId::GeneratorProtoReturn);
        // A generator is its own iterator.
        let generator_iterator = self.allocate_builtin_method(BuiltinId::GeneratorProtoIterator);
        self.define_data_property(
            generator_prototype,
            PropertyKey::Symbol(SymbolId(SYMBOL_ITERATOR_ID)),
            generator_iterator,
            true,
            false,
            true,
        );
        self.define_builtin_method(
            async_generator_prototype,
            "next",
            BuiltinId::AsyncGeneratorProtoNext,
        );
        self.define_builtin_method(
            async_generator_prototype,
            "return",
            BuiltinId::AsyncGeneratorProtoReturn,
        );
        let async_generator_iterator =
            self.allocate_builtin_method(BuiltinId::AsyncGeneratorProtoIterator);
        self.define_data_property(
            async_generator_prototype,
            PropertyKey::Symbol(SymbolId(SYMBOL_ASYNC_ITERATOR_ID)),
            async_generator_iterator,
            true,
            false,
            true,
        );

        for (name, builtin) in [
            ("get", BuiltinId::UspGet),
            ("getAll", BuiltinId::UspGetAll),
            ("has", BuiltinId::UspHas),
            ("set", BuiltinId::UspSet),
            ("append", BuiltinId::UspAppend),
            ("delete", BuiltinId::UspDelete),
            ("toString", BuiltinId::UspToString),
            ("forEach", BuiltinId::UspForEach),
            ("entries", BuiltinId::UspEntries),
            ("keys", BuiltinId::UspKeys),
            ("values", BuiltinId::UspValues),
            ("sort", BuiltinId::UspSort),
        ] {
            self.define_builtin_method(url_search_params_prototype, name, builtin);
        }
        for (name, builtin) in [("toString", BuiltinId::UrlToString), ("toJSON", BuiltinId::UrlToString)] {
            self.define_builtin_method(url_prototype, name, builtin);
        }
        self.define_builtin_method(url_prototype, "valueOf", BuiltinId::UrlToPrimitive);
        let url_to_primitive = self.allocate_builtin_method(BuiltinId::UrlToPrimitive);
        self.define_data_property(
            url_prototype,
            PropertyKey::Symbol(SymbolId(SYMBOL_TO_PRIMITIVE_ID)),
            url_to_primitive,
            true,
            false,
            true,
        );
        let usp_iterator = self.allocate_builtin_method(BuiltinId::UspEntries);
        self.define_data_property(
            url_search_params_prototype,
            PropertyKey::Symbol(SymbolId(SYMBOL_ITERATOR_ID)),
            usp_iterator,
            true,
            false,
            true,
        );

        for (name, builtin) in [
            ("get", BuiltinId::HeadersGet),
            ("set", BuiltinId::HeadersSet),
            ("has", BuiltinId::HeadersHas),
            ("append", BuiltinId::HeadersAppend),
            ("delete", BuiltinId::HeadersDelete),
            ("forEach", BuiltinId::HeadersForEach),
            ("entries", BuiltinId::HeadersEntries),
            ("keys", BuiltinId::HeadersKeys),
            ("values", BuiltinId::HeadersValues),
        ] {
            self.define_builtin_method(headers_prototype, name, builtin);
        }
        let headers_iterator = self.allocate_builtin_method(BuiltinId::HeadersEntries);
        self.define_data_property(
            headers_prototype,
            PropertyKey::Symbol(SymbolId(SYMBOL_ITERATOR_ID)),
            headers_iterator,
            true,
            false,
            true,
        );

        for (name, builtin) in [
            ("get", BuiltinId::FormDataGet),
            ("getAll", BuiltinId::FormDataGetAll),
            ("has", BuiltinId::FormDataHas),
            ("set", BuiltinId::FormDataSet),
            ("append", BuiltinId::FormDataAppend),
            ("delete", BuiltinId::FormDataDelete),
            ("forEach", BuiltinId::FormDataForEach),
            ("entries", BuiltinId::FormDataEntries),
            ("keys", BuiltinId::FormDataKeys),
            ("values", BuiltinId::FormDataValues),
        ] {
            self.define_builtin_method(form_data_prototype, name, builtin);
        }
        let form_data_iterator = self.allocate_builtin_method(BuiltinId::FormDataEntries);
        self.define_data_property(
            form_data_prototype,
            PropertyKey::Symbol(SymbolId(SYMBOL_ITERATOR_ID)),
            form_data_iterator,
            true,
            false,
            true,
        );

        self.define_builtin_method(string_prototype, "match", BuiltinId::StringProtoMatch);
        self.define_builtin_method(string_prototype, "matchAll", BuiltinId::StringProtoMatchAll);
        self.define_builtin_method(string_prototype, "search", BuiltinId::StringProtoSearch);

        if let Some(object_ref) = self.value_object_ref(object_ctor) {
            self.define_builtin_method(object_ref, "create", BuiltinId::ObjectCreate);
            self.define_builtin_method(
                object_ref,
                "defineProperty",
                BuiltinId::ObjectDefineProperty,
            );
            self.define_builtin_method(
                object_ref,
                "getOwnPropertyDescriptor",
                BuiltinId::ObjectGetOwnPropertyDescriptor,
            );
            self.define_builtin_method(object_ref, "keys", BuiltinId::ObjectKeys);
            self.define_builtin_method(
                object_ref,
                "getOwnPropertySymbols",
                BuiltinId::ObjectGetOwnPropertySymbols,
            );
            self.define_builtin_method(object_ref, "values", BuiltinId::ObjectValues);
            self.define_builtin_method(object_ref, "entries", BuiltinId::ObjectEntries);
            self.define_builtin_method(object_ref, "assign", BuiltinId::ObjectAssign);
            self.define_builtin_method(
                object_ref,
                "getPrototypeOf",
                BuiltinId::ObjectGetPrototypeOf,
            );
            self.define_builtin_method(
                object_ref,
                "setPrototypeOf",
                BuiltinId::ObjectSetPrototypeOf,
            );
            self.define_builtin_method(object_ref, "freeze", BuiltinId::ObjectFreeze);
            self.define_builtin_method(object_ref, "isFrozen", BuiltinId::ObjectIsFrozen);
            self.define_builtin_method(object_ref, "fromEntries", BuiltinId::ObjectFromEntries);
            self.define_builtin_method(
                object_ref,
                "getOwnPropertyNames",
                BuiltinId::ObjectGetOwnPropertyNames,
            );
            self.define_builtin_method(object_ref, "hasOwn", BuiltinId::ObjectHasOwn);
            self.define_builtin_method(
                object_ref,
                "getOwnPropertyDescriptors",
                BuiltinId::ObjectGetOwnPropertyDescriptors,
            );
            self.define_builtin_method(
                object_ref,
                "defineProperties",
                BuiltinId::ObjectDefineProperties,
            );
            self.define_builtin_method(object_ref, "is", BuiltinId::ObjectIs);
            self.define_builtin_method(
                object_ref,
                "preventExtensions",
                BuiltinId::ObjectPreventExtensions,
            );
            self.define_builtin_method(object_ref, "isExtensible", BuiltinId::ObjectIsExtensible);
            self.define_builtin_method(object_ref, "seal", BuiltinId::ObjectSeal);
            self.define_builtin_method(object_ref, "isSealed", BuiltinId::ObjectIsSealed);
            // Object.prototype as a property of the constructor.
            self.define_data_property(
                object_ref,
                PropertyKey::from("prototype"),
                Value::Object(object_prototype),
                false,
                false,
                false,
            );
        }

        if let Some(array_ref) = self.value_object_ref(array_ctor) {
            self.define_builtin_method(array_ref, "isArray", BuiltinId::ArrayIsArray);
            self.define_builtin_method(array_ref, "from", BuiltinId::ArrayFrom);
            self.define_builtin_method(array_ref, "of", BuiltinId::ArrayOf);
        }

        if let Some(promise_ref) = self.value_object_ref(promise_ctor) {
            self.define_builtin_method(promise_ref, "resolve", BuiltinId::PromiseResolve);
            self.define_builtin_method(promise_ref, "reject", BuiltinId::PromiseReject);
            self.define_builtin_method(promise_ref, "all", BuiltinId::PromiseAll);
            self.define_builtin_method(promise_ref, "race", BuiltinId::PromiseRace);
            self.define_builtin_method(promise_ref, "allSettled", BuiltinId::PromiseAllSettled);
            self.define_builtin_method(promise_ref, "any", BuiltinId::PromiseAny);
        }

        if let Some(number_ref) = self.value_object_ref(self.globals["Number"].clone()) {
            self.define_builtin_method(number_ref, "isNaN", BuiltinId::NumberIsNaN);
            self.define_builtin_method(number_ref, "isFinite", BuiltinId::NumberIsFinite);
            self.define_builtin_method(number_ref, "isInteger", BuiltinId::NumberIsInteger);
            self.define_builtin_method(
                number_ref,
                "isSafeInteger",
                BuiltinId::NumberIsSafeInteger,
            );
            self.define_builtin_method(number_ref, "parseInt", BuiltinId::NumberParseInt);
            self.define_builtin_method(number_ref, "parseFloat", BuiltinId::NumberParseFloat);
            self.define_data_property(
                number_ref,
                PropertyKey::from("MAX_SAFE_INTEGER"),
                Value::Number(9_007_199_254_740_991.0),
                false,
                false,
                false,
            );
            self.define_data_property(
                number_ref,
                PropertyKey::from("MIN_SAFE_INTEGER"),
                Value::Number(-9_007_199_254_740_991.0),
                false,
                false,
                false,
            );
            for (name, value) in [
                ("MAX_VALUE", f64::MAX),
                ("MIN_VALUE", f64::MIN_POSITIVE),
                ("EPSILON", f64::EPSILON),
                ("POSITIVE_INFINITY", f64::INFINITY),
                ("NEGATIVE_INFINITY", f64::NEG_INFINITY),
                ("NaN", f64::NAN),
            ] {
                self.define_data_property(
                    number_ref,
                    PropertyKey::from(name),
                    Value::Number(value),
                    false,
                    false,
                    false,
                );
            }
        }

        if let Some(string_ref) = self.value_object_ref(string_ctor) {
            self.define_builtin_method(
                string_ref,
                "fromCharCode",
                BuiltinId::StringFromCharCode,
            );
            self.define_builtin_method(
                string_ref,
                "fromCodePoint",
                BuiltinId::StringFromCodePoint,
            );
            self.define_builtin_method(string_ref, "raw", BuiltinId::StringRaw);
        }

        if let Some(symbol_ref) = self.value_object_ref(symbol_ctor) {
            // Well-known symbols exposed as static properties of Symbol.
            for (name, id) in [
                ("iterator", SYMBOL_ITERATOR_ID),
                ("asyncIterator", SYMBOL_ASYNC_ITERATOR_ID),
                ("hasInstance", 3),
                ("toPrimitive", 4),
                ("toStringTag", 5),
            ] {
                self.define_data_property(
                    symbol_ref,
                    PropertyKey::from(name),
                    Value::Symbol(SymbolId(id)),
                    false,
                    false,
                    false,
                );
            }
            self.define_builtin_method(symbol_ref, "for", BuiltinId::SymbolFor);
            self.define_builtin_method(symbol_ref, "keyFor", BuiltinId::SymbolKeyFor);
        }

        // Reflect namespace object.
        let reflect_object = self.allocate_ordinary_object(Some(object_prototype));
        for (name, builtin) in [
            ("get", BuiltinId::ReflectGet),
            ("set", BuiltinId::ReflectSet),
            ("has", BuiltinId::ReflectHas),
            ("deleteProperty", BuiltinId::ReflectDeleteProperty),
            ("ownKeys", BuiltinId::ReflectOwnKeys),
            ("getPrototypeOf", BuiltinId::ReflectGetPrototypeOf),
            ("defineProperty", BuiltinId::ReflectDefineProperty),
            ("apply", BuiltinId::ReflectApply),
            ("construct", BuiltinId::ReflectConstruct),
        ] {
            self.define_builtin_method(reflect_object, name, builtin);
        }
        self.globals
            .insert("Reflect".to_string(), Value::Object(reflect_object));

        let module_reexport_all = self.allocate_builtin_method(BuiltinId::ModuleReexportAll);
        self.globals.insert(
            "\u{0}builtin:moduleReexportAll".to_string(),
            module_reexport_all,
        );

        let proxy_ctor = self.allocate_builtin_value(BuiltinId::ProxyConstructor, true, None);
        self.globals.insert("Proxy".to_string(), proxy_ctor);

        let usp_ctor = self.allocate_builtin_value(
            BuiltinId::UrlSearchParamsConstructor,
            true,
            Some(url_search_params_prototype),
        );
        self.globals
            .insert("URLSearchParams".to_string(), usp_ctor);
        let headers_ctor = self.allocate_builtin_value(
            BuiltinId::HeadersConstructor,
            true,
            Some(headers_prototype),
        );
        self.globals.insert("Headers".to_string(), headers_ctor);
        let form_data_ctor = self.allocate_builtin_value(
            BuiltinId::FormDataConstructor,
            true,
            Some(form_data_prototype),
        );
        self.globals.insert("FormData".to_string(), form_data_ctor);
        let url_ctor = self.allocate_builtin_value(BuiltinId::UrlConstructor, true, Some(url_prototype));
        self.globals.insert("URL".to_string(), url_ctor);

        if let Some(date_ref) = self.value_object_ref(date_ctor) {
            self.define_builtin_method(date_ref, "now", BuiltinId::DateNow);
            self.define_builtin_method(date_ref, "UTC", BuiltinId::DateUTC);
            self.define_builtin_method(date_ref, "parse", BuiltinId::DateParse);
        }

        self.define_builtin_method(math_object, "floor", BuiltinId::MathFloor);
        self.define_builtin_method(math_object, "ceil", BuiltinId::MathCeil);
        self.define_builtin_method(math_object, "round", BuiltinId::MathRound);
        self.define_builtin_method(math_object, "trunc", BuiltinId::MathTrunc);
        self.define_builtin_method(math_object, "abs", BuiltinId::MathAbs);
        self.define_builtin_method(math_object, "min", BuiltinId::MathMin);
        self.define_builtin_method(math_object, "max", BuiltinId::MathMax);
        self.define_builtin_method(math_object, "pow", BuiltinId::MathPow);
        self.define_builtin_method(math_object, "sqrt", BuiltinId::MathSqrt);
        self.define_builtin_method(math_object, "cbrt", BuiltinId::MathCbrt);
        self.define_builtin_method(math_object, "expm1", BuiltinId::MathExpm1);
        self.define_builtin_method(math_object, "fround", BuiltinId::MathFround);
        self.define_builtin_method(math_object, "sin", BuiltinId::MathSin);
        self.define_builtin_method(math_object, "cos", BuiltinId::MathCos);
        self.define_builtin_method(math_object, "tan", BuiltinId::MathTan);
        self.define_builtin_method(math_object, "sinh", BuiltinId::MathSinh);
        self.define_builtin_method(math_object, "cosh", BuiltinId::MathCosh);
        self.define_builtin_method(math_object, "tanh", BuiltinId::MathTanh);
        self.define_builtin_method(math_object, "asin", BuiltinId::MathAsin);
        self.define_builtin_method(math_object, "acos", BuiltinId::MathAcos);
        self.define_builtin_method(math_object, "atan", BuiltinId::MathAtan);
        self.define_builtin_method(math_object, "asinh", BuiltinId::MathAsinh);
        self.define_builtin_method(math_object, "acosh", BuiltinId::MathAcosh);
        self.define_builtin_method(math_object, "atanh", BuiltinId::MathAtanh);
        self.define_builtin_method(math_object, "atan2", BuiltinId::MathAtan2);
        self.define_builtin_method(math_object, "log", BuiltinId::MathLog);
        self.define_builtin_method(math_object, "log2", BuiltinId::MathLog2);
        self.define_builtin_method(math_object, "log10", BuiltinId::MathLog10);
        self.define_builtin_method(math_object, "exp", BuiltinId::MathExp);
        self.define_builtin_method(math_object, "random", BuiltinId::MathRandom);
        self.define_builtin_method(math_object, "sign", BuiltinId::MathSign);
        self.define_builtin_method(math_object, "hypot", BuiltinId::MathHypot);
        self.define_builtin_method(math_object, "clz32", BuiltinId::MathClz32);
        self.define_data_property(
            math_object,
            PropertyKey::from("PI"),
            Value::Number(std::f64::consts::PI),
            false,
            false,
            false,
        );
        self.define_data_property(
            math_object,
            PropertyKey::from("E"),
            Value::Number(std::f64::consts::E),
            false,
            false,
            false,
        );

        self.define_builtin_method(json_object, "stringify", BuiltinId::JsonStringify);
        self.define_builtin_method(json_object, "parse", BuiltinId::JsonParse);

        // Global functions.
        for (name, builtin) in [
            ("parseInt", BuiltinId::GlobalParseInt),
            ("parseFloat", BuiltinId::GlobalParseFloat),
            ("isNaN", BuiltinId::GlobalIsNaN),
            ("isFinite", BuiltinId::GlobalIsFinite),
            ("escape", BuiltinId::GlobalEscape),
            ("unescape", BuiltinId::GlobalUnescape),
            ("encodeURIComponent", BuiltinId::EncodeUriComponent),
            ("decodeURIComponent", BuiltinId::DecodeUriComponent),
            ("encodeURI", BuiltinId::EncodeUri),
            ("decodeURI", BuiltinId::DecodeUri),
        ] {
            let value = self.allocate_builtin_method(builtin);
            self.globals.insert(name.to_string(), value);
        }

        let structured_clone = self.allocate_builtin_method(BuiltinId::StructuredClone);
        self.globals
            .insert("structuredClone".to_string(), structured_clone);

        let fetch = self.allocate_builtin_method(BuiltinId::Fetch);
        self.globals.insert("fetch".to_string(), fetch);

        // Global value constants.
        self.globals.insert("NaN".to_string(), Value::Number(f64::NAN));
        self.globals
            .insert("Infinity".to_string(), Value::Number(f64::INFINITY));
        self.globals
            .insert("undefined".to_string(), Value::Undefined);
    }

    fn number_prototype_ref(&self) -> GcRef<JsObject> {
        self.number_prototype
            .expect("number prototype should be installed")
    }

    fn boolean_prototype_ref(&self) -> GcRef<JsObject> {
        self.boolean_prototype
            .expect("boolean prototype should be installed")
    }

    fn regexp_prototype_ref(&self) -> GcRef<JsObject> {
        self.regexp_prototype
            .expect("regexp prototype should be installed")
    }

    fn date_prototype_ref(&self) -> GcRef<JsObject> {
        self.date_prototype
            .expect("date prototype should be installed")
    }

    fn generator_prototype_ref(&self) -> GcRef<JsObject> {
        self.generator_prototype
            .expect("generator prototype should be installed")
    }

    fn async_generator_prototype_ref(&self) -> GcRef<JsObject> {
        self.async_generator_prototype
            .expect("async generator prototype should be installed")
    }

    fn set_generator_state(&mut self, generator: GcRef<JsObject>, state: GeneratorState) {
        if let Some(object) = self.heap.objects_mut().get_mut(generator) {
            object.kind = ObjectKind::Generator(Box::new(state));
        }
    }

    fn set_async_generator_state(
        &mut self,
        generator: GcRef<JsObject>,
        state: GeneratorState,
        queue: VecDeque<AsyncGeneratorRequest>,
    ) {
        if let Some(object) = self.heap.objects_mut().get_mut(generator) {
            object.kind = ObjectKind::AsyncGenerator {
                state: Box::new(state),
                queue,
            };
        }
    }

    /// Remove and return a generator's state (leaving it Ordinary temporarily).
    fn take_generator_state(&mut self, generator: GcRef<JsObject>) -> Option<GeneratorState> {
        match self.heap.objects_mut().get_mut(generator) {
            Some(object) => {
                let kind = std::mem::replace(&mut object.kind, ObjectKind::Ordinary);
                match kind {
                    ObjectKind::Generator(state) => Some(*state),
                    other => {
                        object.kind = other;
                        None
                    }
                }
            }
            None => None,
        }
    }

    fn take_async_generator_state(
        &mut self,
        generator: GcRef<JsObject>,
    ) -> Option<(GeneratorState, VecDeque<AsyncGeneratorRequest>)> {
        match self.heap.objects_mut().get_mut(generator) {
            Some(object) => {
                let kind = std::mem::replace(&mut object.kind, ObjectKind::Ordinary);
                match kind {
                    ObjectKind::AsyncGenerator { state, queue } => Some((*state, queue)),
                    other => {
                        object.kind = other;
                        None
                    }
                }
            }
            None => None,
        }
    }

    /// Build an `{ value, done }` iterator-result object.
    fn make_iter_result(&mut self, value: Value, done: bool) -> Result<Value, VmError> {
        let object = self.allocate_ordinary_object(Some(self.object_prototype_ref()));
        self.define_data_property(object, PropertyKey::from("value"), value, true, true, true);
        self.define_data_property(
            object,
            PropertyKey::from("done"),
            Value::Bool(done),
            true,
            true,
            true,
        );
        Ok(Value::Object(object))
    }

    fn settle_async_generator_request(
        &mut self,
        request: AsyncGeneratorRequest,
        value: Value,
        done: bool,
    ) -> Result<(), VmError> {
        let result = self.make_iter_result(value, done)?;
        let promise = request.promise;
        self.resolve_promise_from_resolution(promise, result)?;
        Ok(())
    }

    fn settle_async_generator_queue_completed(
        &mut self,
        queue: &mut std::collections::VecDeque<AsyncGeneratorRequest>,
    ) -> Result<(), VmError> {
        while let Some(request) = queue.pop_front() {
            self.settle_async_generator_request(request, Value::Undefined, true)?;
        }
        Ok(())
    }

    fn resume_async_generator_suspended(
        &mut self,
        generator: GcRef<JsObject>,
        request: AsyncGeneratorRequest,
        frame: Box<CallFrame>,
        stack: Vec<Value>,
        started: bool,
        queue: std::collections::VecDeque<AsyncGeneratorRequest>,
    ) -> Result<Value, VmError> {
        if request.is_return {
            self.set_async_generator_state(generator, GeneratorState::Completed, queue);
            let result = self.make_iter_result(request.sent, true)?;
            self.resolve_promise_from_resolution(request.promise, result)?;
            let promise = request.promise;
            return Ok(Value::Object(promise));
        }
        let base_depth = self.frames.len();
        let mut frame = *frame;
        frame.stack_base = self.stack.len();
        frame.async_gen_request = Some(request.promise);
        self.frames.push(frame);
        self.stack.extend(stack);
        if started {
            self.stack.push(request.sent);
        }
        self.set_async_generator_state(generator, GeneratorState::Running, queue);
        self.run_until_frame_depth(base_depth)?;
        let promise = request.promise;
        Ok(Value::Object(promise))
    }

    /// Resume (or start) a generator and run until its next yield or completion.
    fn generator_resume(
        &mut self,
        generator: GcRef<JsObject>,
        sent: Value,
    ) -> Result<Value, VmError> {
        match self.take_generator_state(generator) {
            None => Err(VmError::TypeError(
                "next called on a non-generator".to_string(),
            )),
            Some(GeneratorState::Completed) => {
                self.set_generator_state(generator, GeneratorState::Completed);
                self.make_iter_result(Value::Undefined, true)
            }
            Some(GeneratorState::Running) => {
                self.set_generator_state(generator, GeneratorState::Running);
                Err(VmError::TypeError("generator is already running".to_string()))
            }
            Some(GeneratorState::Suspended {
                frame,
                stack,
                started,
            }) => {
                self.set_generator_state(generator, GeneratorState::Running);
                let base_depth = self.frames.len();
                let mut frame = *frame;
                frame.stack_base = self.stack.len();
                self.frames.push(frame);
                self.stack.extend(stack);
                if started {
                    // The sent value becomes the result of the paused `yield`.
                    self.stack.push(sent);
                }
                self.generator_outcome = None;
                self.run_until_frame_depth(base_depth)?;
                match self.generator_outcome.take() {
                    Some(GeneratorOutcome::Yielded(value)) => self.make_iter_result(value, false),
                    Some(GeneratorOutcome::Returned(value)) => self.make_iter_result(value, true),
                    None => self.make_iter_result(Value::Undefined, true),
                }
            }
        }
    }

    fn async_generator_resume(
        &mut self,
        generator: GcRef<JsObject>,
        sent: Value,
        is_return: bool,
    ) -> Result<Value, VmError> {
        let promise = self.allocate_pending_promise_object();
        let request = AsyncGeneratorRequest {
            sent,
            promise,
            is_return,
        };
        match self.take_async_generator_state(generator) {
            None => Err(VmError::TypeError(
                "async generator method called on non-async-generator".to_string(),
            )),
            Some((GeneratorState::Completed, queue)) => {
                self.set_async_generator_state(generator, GeneratorState::Completed, queue);
                let value = if is_return { request.sent } else { Value::Undefined };
                let result = self.make_iter_result(value, true)?;
                self.resolve_promise_from_resolution(request.promise, result)?;
                Ok(Value::Object(promise))
            }
            Some((GeneratorState::Running, mut queue)) => {
                queue.push_back(request);
                self.set_async_generator_state(generator, GeneratorState::Running, queue);
                Ok(Value::Object(promise))
            }
            Some((
                GeneratorState::Suspended {
                    frame,
                    stack,
                    started,
                },
                queue,
            )) => self.resume_async_generator_suspended(
                generator,
                request,
                frame,
                stack,
                started,
                queue,
            ),
        }
    }

    /// Allocate a fresh unique Symbol value, recording its description.
    fn allocate_symbol(&mut self, description: Option<String>) -> Value {
        let id = self.next_symbol_id;
        self.next_symbol_id = self.next_symbol_id.saturating_add(1);
        if let Some(description) = description {
            self.symbol_descriptions.insert(id, description);
        }
        Value::Symbol(SymbolId(id))
    }

    /// Current wall-clock time in milliseconds since the Unix epoch.
    fn current_time_ms(&self) -> f64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_millis() as f64)
            .unwrap_or(0.0)
    }

    /// Allocate a Date object storing its time value in a hidden property.
    fn make_date_object(&mut self, time_ms: f64) -> Value {
        let object = self.allocate_ordinary_object(Some(self.date_prototype_ref()));
        self.define_data_property(
            object,
            PropertyKey::from("__time__"),
            Value::Number(time_ms),
            true,
            false,
            false,
        );
        Value::Object(object)
    }

    /// Read the millisecond time value stored on a Date object.
    fn date_time_value(&mut self, this_value: &Value) -> Result<f64, VmError> {
        let value = self.get_property_value(this_value, &PropertyKey::from("__time__"))?;
        Ok(self.to_number(&value))
    }

    /// Decompose an epoch-ms value into (year, month0, day, hours, minutes,
    /// seconds, millis, weekday). Returns None for NaN.
    fn date_components(time_ms: f64) -> Option<(i64, i64, i64, i64, i64, i64, i64, i64)> {
        if !time_ms.is_finite() {
            return None;
        }
        let ms = time_ms as i64;
        let days = floor_div(ms, 86_400_000);
        let mut rem = floor_mod(ms, 86_400_000);
        let millis = rem % 1000;
        rem /= 1000;
        let seconds = rem % 60;
        rem /= 60;
        let minutes = rem % 60;
        rem /= 60;
        let hours = rem % 24;
        let (year, month, day) = civil_from_days(days);
        let weekday = floor_mod(days + 4, 7); // 1970-01-01 was a Thursday
        Some((year, month - 1, day, hours, minutes, seconds, millis, weekday))
    }

    fn parse_date_utc_ms(&self, text: &str) -> f64 {
        fn parse_i64(text: &str) -> Option<i64> {
            text.parse::<i64>().ok()
        }

        let text = text.strip_suffix('Z').unwrap_or(text);
        let (date_part, time_part) = match text.split_once('T') {
            Some(parts) => parts,
            None => (text, ""),
        };
        let mut date_parts = date_part.split('-');
        let year = match parse_i64(date_parts.next().unwrap_or("")) {
            Some(value) => value,
            None => return f64::NAN,
        };
        let month = match parse_i64(date_parts.next().unwrap_or("")) {
            Some(value) => value,
            None => return f64::NAN,
        };
        let day = match parse_i64(date_parts.next().unwrap_or("")) {
            Some(value) => value,
            None => return f64::NAN,
        };
        if date_parts.next().is_some() {
            return f64::NAN;
        }
        let (hours, minutes, seconds, millis) = if time_part.is_empty() {
            (0, 0, 0, 0)
        } else {
            let mut time_parts = time_part.split(':');
            let hours = match parse_i64(time_parts.next().unwrap_or("")) {
                Some(value) => value,
                None => return f64::NAN,
            };
            let minutes = match parse_i64(time_parts.next().unwrap_or("")) {
                Some(value) => value,
                None => return f64::NAN,
            };
            let seconds_and_ms = time_parts.next();
            if time_parts.next().is_some() {
                return f64::NAN;
            }
            match seconds_and_ms {
                None => (hours, minutes, 0, 0),
                Some(part) => {
                    let (seconds, millis) = match part.split_once('.') {
                        Some((seconds, millis)) => {
                            let seconds = match parse_i64(seconds) {
                                Some(value) => value,
                                None => return f64::NAN,
                            };
                            let millis_text = millis.chars().take(3).collect::<String>();
                            let parsed = parse_i64(&millis_text).unwrap_or(0);
                            let millis = match millis_text.len() {
                                1 => parsed * 100,
                                2 => parsed * 10,
                                _ => parsed,
                            };
                            (seconds, millis)
                        }
                        None => match parse_i64(part) {
                            Some(value) => (value, 0),
                            None => return f64::NAN,
                        },
                    };
                    (hours, minutes, seconds, millis)
                }
            }
        };
        let days = days_from_civil(year, month, day);
        (days * 86_400_000 + hours * 3_600_000 + minutes * 60_000 + seconds * 1000 + millis)
            as f64
    }

    /// Return one decomposed Date field by index (0=year .. 7=weekday).
    fn date_component(&mut self, this_value: &Value, index: usize) -> Result<Value, VmError> {
        let time = self.date_time_value(this_value)?;
        Ok(match Self::date_components(time) {
            Some(c) => {
                let fields = [c.0, c.1, c.2, c.3, c.4, c.5, c.6, c.7];
                Value::Number(fields.get(index).copied().unwrap_or(0) as f64)
            }
            None => Value::Number(f64::NAN),
        })
    }

    /// Allocate a RegExp object with `source`/`flags`/`global`/`lastIndex`
    /// properties and the RegExp prototype.
    fn make_regexp_object(&mut self, pattern: &str, flags: &str) -> Value {
        let source_value = self.make_string_value(pattern);
        let flags_value = self.make_string_value(flags);
        let object = self.heap.allocate_object(JsObject {
            kind: ObjectKind::RegExp {
                source: pattern.to_string(),
                flags: flags.to_string(),
                global: flags.contains('g'),
                last_index: 0,
            },
            prototype: Some(self.regexp_prototype_ref()),
            ..JsObject::default()
        });
        for (name, value) in [("source", source_value), ("flags", flags_value)] {
            self.define_data_property(object, PropertyKey::from(name), value, false, false, false);
        }
        for (name, value) in [
            ("global", Value::Bool(flags.contains('g'))),
            ("ignoreCase", Value::Bool(flags.contains('i'))),
            ("multiline", Value::Bool(flags.contains('m'))),
        ] {
            self.define_data_property(object, PropertyKey::from(name), value, false, false, false);
        }
        self.define_data_property(
            object,
            PropertyKey::from("lastIndex"),
            Value::Number(0.0),
            true,
            false,
            false,
        );
        Value::Object(object)
    }

    /// Extract (source, flags) if `value` is a RegExp object.
    fn regexp_source_flags(&self, value: &Value) -> Option<(String, String)> {
        if let Value::Object(object) = value {
            if let Some(JsObject {
                kind: ObjectKind::RegExp { source, flags, .. },
                ..
            }) = self.heap.objects().get(*object)
            {
                return Some((source.clone(), flags.clone()));
            }
        }
        None
    }

    /// Interpret a String.prototype.{match,replace,split,…} argument as a regex:
    /// a RegExp value keeps its source/flags; any other value is coerced to a
    /// string and used as a literal pattern (matching `new RegExp(str)`).
    fn coerce_regex_arg(&mut self, value: Option<&Value>) -> Result<(String, String), VmError> {
        match value {
            Some(value) if self.regexp_source_flags(value).is_some() => {
                Ok(self.regexp_source_flags(value).unwrap())
            }
            Some(value) => Ok((self.to_string(value), String::new())),
            None => Ok((String::new(), String::new())),
        }
    }

    /// Implements `String.prototype.replace`/`replaceAll` for a RegExp pattern,
    /// supporting both string templates (`$&`, `$1`, `$<name>`, `$$`) and a
    /// replacer function called with (match, ...groups, offset, string).
    fn regex_replace(
        &mut self,
        text: &str,
        regex: &JsRegex,
        replacement: &Value,
        global: bool,
    ) -> Result<Value, VmError> {
        let is_fn = self.is_callable_value(replacement);
        let template = if is_fn {
            String::new()
        } else {
            self.to_string(replacement)
        };
        let caps_list: Vec<JsCaptures> = if global {
            regex.captures_iter(text)
        } else {
            regex.captures(text).into_iter().collect()
        };
        let mut result = String::new();
        let mut last_end = 0;
        for caps in &caps_list {
            let full = match caps.get(0) {
                Some(m) => m,
                None => continue,
            };
            result.push_str(&text[last_end..full.start()]);
            if is_fn {
                let mut call_args = Vec::with_capacity(caps.len() + 2);
                for i in 0..caps.len() {
                    call_args.push(
                        caps.get(i)
                            .map(|g| self.make_string_value(g.as_str()))
                            .unwrap_or(Value::Undefined),
                    );
                }
                call_args.push(Value::Number(text[..full.start()].chars().count() as f64));
                call_args.push(self.make_string_value(text));
                let replaced =
                    self.call_value_sync(replacement.clone(), Value::Undefined, call_args)?;
                let replaced = self.to_string(&replaced);
                result.push_str(&replaced);
            } else {
                result.push_str(&expand_replacement(&template, caps, text));
            }
            last_end = full.end();
        }
        result.push_str(&text[last_end..]);
        Ok(self.make_string_value(&result))
    }

    /// Implements `String.prototype.replace`/`replaceAll` for a plain string
    /// pattern (function replacer or `$&`/`$$` template).
    fn string_replace(
        &mut self,
        text: &str,
        search: &str,
        replacement: &Value,
        all: bool,
    ) -> Result<Value, VmError> {
        let is_fn = self.is_callable_value(replacement);
        if search.is_empty() {
            // Avoid an infinite loop; approximate by replacing once at the front.
            let head = if is_fn {
                let args = vec![
                    self.make_string_value(""),
                    Value::Number(0.0),
                    self.make_string_value(text),
                ];
                let replaced =
                    self.call_value_sync(replacement.clone(), Value::Undefined, args)?;
                self.to_string(&replaced)
            } else {
                expand_string_replacement(&self.to_string(replacement), "")
            };
            return Ok(self.make_string_value(&format!("{head}{text}")));
        }
        let template = if is_fn {
            String::new()
        } else {
            self.to_string(replacement)
        };
        let mut result = String::new();
        let mut cursor = 0;
        while let Some(rel) = text[cursor..].find(search) {
            let start = cursor + rel;
            result.push_str(&text[cursor..start]);
            if is_fn {
                let args = vec![
                    self.make_string_value(search),
                    Value::Number(text[..start].chars().count() as f64),
                    self.make_string_value(text),
                ];
                let replaced =
                    self.call_value_sync(replacement.clone(), Value::Undefined, args)?;
                let replaced = self.to_string(&replaced);
                result.push_str(&replaced);
            } else {
                result.push_str(&expand_string_replacement(&template, search));
            }
            cursor = start + search.len();
            if !all {
                break;
            }
        }
        result.push_str(&text[cursor..]);
        Ok(self.make_string_value(&result))
    }

    fn regexp_last_index(&self, object: GcRef<JsObject>) -> usize {
        match self.heap.objects().get(object).map(|o| &o.kind) {
            Some(ObjectKind::RegExp { last_index, .. }) => *last_index as usize,
            _ => 0,
        }
    }

    fn set_regexp_last_index(&mut self, object: GcRef<JsObject>, value: usize) {
        if let Some(ObjectKind::RegExp { last_index, .. }) =
            self.heap.objects_mut().get_mut(object).map(|o| &mut o.kind)
        {
            *last_index = value as u32;
        }
        self.define_data_property(
            object,
            PropertyKey::from("lastIndex"),
            Value::Number(value as f64),
            true,
            false,
            false,
        );
    }

    /// Build a JS match-result array (`[full, ...groups]` with `index`, `input`,
    /// and `groups`) from a regex capture.
    fn build_match_result(
        &mut self,
        caps: &JsCaptures,
        input: &str,
    ) -> Result<Value, VmError> {
        let mut items = Vec::with_capacity(caps.len());
        for i in 0..caps.len() {
            match caps.get(i) {
                Some(m) => items.push(self.make_string_value(m.as_str())),
                None => items.push(Value::Undefined),
            }
        }
        let array = self.make_array_from_values(items)?;
        let array_ref = self.require_object_ref(&array, "match result")?;
        let match_start = caps.get(0).map(|m| m.start()).unwrap_or(0);
        let char_index = input[..match_start].chars().count();
        self.define_data_property(
            array_ref,
            PropertyKey::from("index"),
            Value::Number(char_index as f64),
            true,
            true,
            true,
        );
        let input_value = self.make_string_value(input);
        self.define_data_property(
            array_ref,
            PropertyKey::from("input"),
            input_value,
            true,
            true,
            true,
        );
        let named: Vec<(String, Option<String>)> = caps
            .named_iter()
            .map(|(name, value)| (name.to_string(), value.map(|m| m.as_str().to_string())))
            .collect();
        let groups = if named.is_empty() {
            Value::Undefined
        } else {
            let groups = self.allocate_ordinary_object(Some(self.object_prototype_ref()));
            for (name, value) in named {
                let value = value
                    .map(|s| self.make_string_value(&s))
                    .unwrap_or(Value::Undefined);
                self.define_data_property(
                    groups,
                    PropertyKey::from(name.as_str()),
                    value,
                    true,
                    true,
                    true,
                );
            }
            Value::Object(groups)
        };
        self.define_data_property(array_ref, PropertyKey::from("groups"), groups, true, true, true);
        Ok(array)
    }

    fn object_prototype_ref(&self) -> GcRef<JsObject> {
        self.object_prototype
            .expect("object prototype should be installed")
    }

    fn function_prototype_ref(&self) -> GcRef<JsObject> {
        self.function_prototype
            .expect("function prototype should be installed")
    }

    fn array_prototype_ref(&self) -> GcRef<JsObject> {
        self.array_prototype
            .expect("array prototype should be installed")
    }

    fn string_prototype_ref(&self) -> GcRef<JsObject> {
        self.string_prototype
            .expect("string prototype should be installed")
    }

    fn error_prototype_ref(&self) -> GcRef<JsObject> {
        self.error_prototype
            .expect("error prototype should be installed")
    }

    fn promise_prototype_ref(&self) -> GcRef<JsObject> {
        self.promise_prototype
            .expect("promise prototype should be installed")
    }

    fn map_prototype_ref(&self) -> GcRef<JsObject> {
        self.map_prototype
            .expect("map prototype should be installed")
    }

    fn set_prototype_ref(&self) -> GcRef<JsObject> {
        self.set_prototype
            .expect("set prototype should be installed")
    }

    fn array_buffer_prototype_ref(&self) -> GcRef<JsObject> {
        self.array_buffer_prototype
            .expect("ArrayBuffer prototype should be installed")
    }

    fn typed_array_prototype_ref(&self) -> GcRef<JsObject> {
        self.typed_array_prototype
            .expect("TypedArray prototype should be installed")
    }

    fn weak_ref_prototype_ref(&self) -> GcRef<JsObject> {
        self.weak_ref_prototype
            .expect("WeakRef prototype should be installed")
    }

    fn text_encoder_prototype_ref(&self) -> GcRef<JsObject> {
        self.text_encoder_prototype
            .expect("TextEncoder prototype should be installed")
    }

    fn text_decoder_prototype_ref(&self) -> GcRef<JsObject> {
        self.text_decoder_prototype
            .expect("TextDecoder prototype should be installed")
    }

    fn allocate_ordinary_object(&mut self, prototype: Option<GcRef<JsObject>>) -> GcRef<JsObject> {
        self.heap.allocate_object(JsObject {
            kind: ObjectKind::Ordinary,
            prototype,
            ..JsObject::default()
        })
    }

    fn allocate_builtin_value(
        &mut self,
        builtin: BuiltinId,
        constructable: bool,
        construct_prototype: Option<GcRef<JsObject>>,
    ) -> Value {
        let object_ref = self.heap.allocate_object(JsObject {
            kind: ObjectKind::Function,
            prototype: Some(self.function_prototype_ref()),
            ..JsObject::default()
        });
        self.callables
            .insert(object_ref.raw(), Callable::Builtin(builtin));
        if constructable {
            let prototype = construct_prototype.unwrap_or_else(|| self.object_prototype_ref());
            self.define_data_property(
                object_ref,
                PropertyKey::from("prototype"),
                Value::Object(prototype),
                true,
                false,
                false,
            );
        }
        Value::Object(object_ref)
    }

    /// Cached version of allocate_builtin_value for stateless methods (constructable=false, prototype=None).
    /// Eliminates the heap allocation on every DOM property access (e.g. element.appendChild).
    #[inline]
    fn allocate_builtin_method(&mut self, builtin: BuiltinId) -> Value {
        if let Some(v) = self.builtin_method_cache.get(&builtin) {
            return v.clone();
        }
        let v = self.allocate_builtin_value(builtin, false, None);
        self.builtin_method_cache.insert(builtin, v.clone());
        v
    }

    fn allocate_callable_object(
        &mut self,
        callable: Callable,
        constructable: bool,
        construct_prototype: Option<GcRef<JsObject>>,
    ) -> GcRef<JsObject> {
        let object_ref = self.heap.allocate_object(JsObject {
            kind: ObjectKind::Function,
            prototype: Some(self.function_prototype_ref()),
            ..JsObject::default()
        });
        self.callables.insert(object_ref.raw(), callable);
        if constructable {
            let prototype = construct_prototype.unwrap_or_else(|| self.object_prototype_ref());
            self.define_data_property(
                object_ref,
                PropertyKey::from("prototype"),
                Value::Object(prototype),
                true,
                false,
                false,
            );
        }
        object_ref
    }

    fn allocate_callable_value(
        &mut self,
        callable: Callable,
        constructable: bool,
        construct_prototype: Option<GcRef<JsObject>>,
    ) -> Value {
        Value::Object(self.allocate_callable_object(callable, constructable, construct_prototype))
    }

    fn allocate_pending_promise_object(&mut self) -> GcRef<JsObject> {
        self.heap.allocate_object(JsObject {
            kind: ObjectKind::Promise(Box::new(PromiseState::Pending {
                fulfill_reactions: Vec::new(),
                reject_reactions: Vec::new(),
            })),
            prototype: Some(self.promise_prototype_ref()),
            ..JsObject::default()
        })
    }

    fn allocate_promise_with_state(&mut self, state: PromiseState) -> GcRef<JsObject> {
        self.heap.allocate_object(JsObject {
            kind: ObjectKind::Promise(Box::new(state)),
            prototype: Some(self.promise_prototype_ref()),
            ..JsObject::default()
        })
    }

    fn allocate_async_resumer(&mut self, context: AsyncContext) -> GcRef<JsObject> {
        self.heap.allocate_object(JsObject {
            kind: ObjectKind::AsyncResumer(Box::new(context)),
            prototype: Some(self.object_prototype_ref()),
            ..JsObject::default()
        })
    }

    fn create_promise_capability_function(
        &mut self,
        promise: GcRef<JsObject>,
        mode: PromiseCapabilityMode,
    ) -> Value {
        self.allocate_callable_value(Callable::PromiseCapability { promise, mode }, false, None)
    }

    fn create_promise_finally_function(
        &mut self,
        callback: Value,
        mode: PromiseFinallyMode,
    ) -> Value {
        self.allocate_callable_value(Callable::PromiseFinally { callback, mode }, false, None)
    }

    fn enqueue_due_timers(&mut self, now_ms: u64) {
        while let Some(Reverse(entry)) = self.event_loop.timer_heap.peek().cloned() {
            if entry.due_ms > now_ms {
                break;
            }
            let Reverse(mut entry) = self
                .event_loop
                .timer_heap
                .pop()
                .expect("peeked timer should still be present");
            if self.event_loop.cancelled_timers.remove(&entry.id) {
                continue;
            }
            self.event_loop.macrotask_queue.push_back(TaskEntry {
                source: TaskSource::Timer,
                callback: entry.callback,
                args: entry.args.clone(),
            });
            if let Some(interval_ms) = entry.interval_ms {
                let mut next_interval: u64 = interval_ms;
                if entry.nesting_level > 5 {
                    next_interval = next_interval.max(4);
                }
                entry.due_ms = entry.due_ms.saturating_add(next_interval);
                entry.nesting_level = entry.nesting_level.saturating_add(1);
                self.event_loop.timer_heap.push(Reverse(entry));
            }
        }
    }

    fn run_task(&mut self, task: TaskEntry) -> Result<(), VmError> {
        let _ = task.source;
        let _ = self.call_value_sync(Value::Object(task.callback), Value::Undefined, task.args)?;
        Ok(())
    }

    fn allocate_function_value(&mut self, closure: RuntimeClosure) -> Value {
        let arity = closure.proto.arity;
        let name = closure.proto.name.clone().unwrap_or_default();
        let object_ref = self.heap.allocate_object(JsObject {
            kind: ObjectKind::Function,
            prototype: Some(self.function_prototype_ref()),
            ..JsObject::default()
        });
        self.callables
            .insert(object_ref.raw(), Callable::Closure(closure));
        let function_prototype = self.allocate_ordinary_object(Some(self.object_prototype_ref()));
        self.define_data_property(
            object_ref,
            PropertyKey::from("prototype"),
            Value::Object(function_prototype),
            true,
            false,
            false,
        );
        let name_value = self.make_string_value(&name);
        self.define_data_property(
            object_ref,
            PropertyKey::from("length"),
            Value::Number(arity as f64),
            false,
            false,
            true,
        );
        self.define_data_property(
            object_ref,
            PropertyKey::from("name"),
            name_value,
            false,
            false,
            true,
        );
        Value::Object(object_ref)
    }

    fn allocate_bound_function_value(&mut self, bound: BoundFunction) -> Value {
        let object_ref = self.heap.allocate_object(JsObject {
            kind: ObjectKind::Function,
            prototype: Some(self.function_prototype_ref()),
            ..JsObject::default()
        });
        self.callables
            .insert(object_ref.raw(), Callable::Bound(bound));
        Value::Object(object_ref)
    }

    fn define_builtin_method(&mut self, object: GcRef<JsObject>, name: &str, builtin: BuiltinId) {
        let value = self.allocate_builtin_value(builtin, false, None);
        self.define_data_property(object, PropertyKey::from(name), value, true, false, true);
    }

    fn define_data_property(
        &mut self,
        object: GcRef<JsObject>,
        key: PropertyKey,
        value: Value,
        writable: bool,
        enumerable: bool,
        configurable: bool,
    ) {
        if let Some(object_data) = self.heap.objects_mut().get_mut(object) {
            object_data.properties.insert(
                key,
                JsPropertyDescriptor::data_with_flags(value, writable, enumerable, configurable),
            );
        }
    }

    fn create_named_error_object(&mut self, name: &str, message: impl Into<String>) -> Value {
        let object = self.heap.allocate_object(JsObject {
            kind: ObjectKind::Error,
            prototype: Some(self.error_prototype_ref()),
            ..JsObject::default()
        });
        let name_value = self.make_string_value(name);
        let message_text = message.into();
        let message_value = self.make_string_value(&message_text);
        self.define_data_property(
            object,
            PropertyKey::from("name"),
            name_value,
            true,
            false,
            true,
        );
        self.define_data_property(
            object,
            PropertyKey::from("message"),
            message_value,
            true,
            false,
            true,
        );
        Value::Object(object)
    }

    fn create_error_object(&mut self, name: &str, message: String) -> Value {
        self.create_named_error_object(name, message)
    }

    fn is_callable_value(&self, value: &Value) -> bool {
        matches!(value, Value::Object(object) if self.callables.contains_key(&object.raw()))
    }

    fn wrap_vm_error_as_value(&mut self, error: &VmError) -> Result<Value, VmError> {
        Ok(match error {
            VmError::TypeError(message) => self.create_error_object("TypeError", message.clone()),
            VmError::ReferenceError(message) => {
                self.create_error_object("ReferenceError", message.clone())
            }
            VmError::RangeError(message) => self.create_error_object("RangeError", message.clone()),
            VmError::Thrown(value) => value.clone(),
            VmError::InfiniteLoop => self.create_error_object(
                "Error",
                "Maximum loop iteration limit exceeded".to_string(),
            ),
            VmError::StackOverflow => self.create_error_object(
                "RangeError",
                "Maximum call stack size exceeded".to_string(),
            ),
            VmError::Unimplemented(feature) => {
                self.create_error_object("Error", format!("unimplemented in phase 5: {feature}"))
            }
        })
    }

    fn promise_state(&self, promise: GcRef<JsObject>) -> Result<&PromiseState, VmError> {
        let object = self
            .heap
            .objects()
            .get(promise)
            .ok_or_else(|| VmError::ReferenceError("missing promise object".to_string()))?;
        match &object.kind {
            ObjectKind::Promise(state) => Ok(state.as_ref()),
            _ => Err(VmError::TypeError("object is not a Promise".to_string())),
        }
    }

    fn promise_state_mut(
        &mut self,
        promise: GcRef<JsObject>,
    ) -> Result<&mut PromiseState, VmError> {
        let object = self
            .heap
            .objects_mut()
            .get_mut(promise)
            .ok_or_else(|| VmError::ReferenceError("missing promise object".to_string()))?;
        match &mut object.kind {
            ObjectKind::Promise(state) => Ok(state.as_mut()),
            _ => Err(VmError::TypeError("object is not a Promise".to_string())),
        }
    }

    fn is_promise_object(&self, object: GcRef<JsObject>) -> bool {
        self.heap
            .objects()
            .get(object)
            .map(|data| matches!(data.kind, ObjectKind::Promise(_)))
            .unwrap_or(false)
    }

    fn normalize_handler_value(&self, value: Option<&Value>) -> Option<GcRef<JsObject>> {
        match value {
            Some(Value::Object(object)) => {
                let is_callable = self.callables.contains_key(&object.raw());
                let is_async_resumer = self
                    .heap
                    .objects()
                    .get(*object)
                    .map(|data| matches!(data.kind, ObjectKind::AsyncResumer(_)))
                    .unwrap_or(false);
                if is_callable || is_async_resumer {
                    Some(*object)
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    fn queue_microtask_job(&mut self, job: MicrotaskJob) {
        self.event_loop.microtask_queue.push_back(job);
    }

    fn enqueue_promise_reaction_job(
        &mut self,
        handler: Option<GcRef<JsObject>>,
        result_promise: Option<GcRef<JsObject>>,
        value: Value,
        is_reject: bool,
    ) {
        self.queue_microtask_job(MicrotaskJob::PromiseReaction {
            handler,
            result_promise,
            value,
            is_reject,
        });
    }

    fn add_promise_reactions(
        &mut self,
        promise: GcRef<JsObject>,
        fulfill_reaction: PromiseReaction,
        reject_reaction: PromiseReaction,
    ) -> Result<(), VmError> {
        enum PromiseSettlement {
            Fulfilled(Value),
            Rejected(Value),
        }

        let settled = match self.promise_state_mut(promise)? {
            PromiseState::Pending {
                fulfill_reactions,
                reject_reactions,
            } => {
                fulfill_reactions.push(fulfill_reaction);
                reject_reactions.push(reject_reaction);
                return Ok(());
            }
            PromiseState::Fulfilled(value) => PromiseSettlement::Fulfilled(value.clone()),
            PromiseState::Rejected(reason) => PromiseSettlement::Rejected(reason.clone()),
        };

        match settled {
            PromiseSettlement::Fulfilled(value) => self.enqueue_promise_reaction_job(
                fulfill_reaction.handler,
                fulfill_reaction.result_promise,
                value,
                false,
            ),
            PromiseSettlement::Rejected(reason) => self.enqueue_promise_reaction_job(
                reject_reaction.handler,
                reject_reaction.result_promise,
                reason,
                true,
            ),
        }
        Ok(())
    }

    fn promise_then_internal(
        &mut self,
        promise: GcRef<JsObject>,
        on_fulfilled: Option<GcRef<JsObject>>,
        on_rejected: Option<GcRef<JsObject>>,
    ) -> Result<GcRef<JsObject>, VmError> {
        let result_promise = self.allocate_pending_promise_object();
        self.add_promise_reactions(
            promise,
            PromiseReaction {
                handler: on_fulfilled,
                result_promise: Some(result_promise),
                is_reject_handler: false,
            },
            PromiseReaction {
                handler: on_rejected,
                result_promise: Some(result_promise),
                is_reject_handler: true,
            },
        )?;
        Ok(result_promise)
    }

    fn fulfill_promise_with_value(
        &mut self,
        promise: GcRef<JsObject>,
        value: Value,
    ) -> Result<(), VmError> {
        let reactions = {
            let state = self.promise_state_mut(promise)?;
            match state {
                PromiseState::Pending {
                    fulfill_reactions,
                    reject_reactions,
                } => {
                    let reactions = std::mem::take(fulfill_reactions);
                    reject_reactions.clear();
                    *state = PromiseState::Fulfilled(value.clone());
                    reactions
                }
                PromiseState::Fulfilled(_) | PromiseState::Rejected(_) => return Ok(()),
            }
        };
        for reaction in reactions {
            self.enqueue_promise_reaction_job(
                reaction.handler,
                reaction.result_promise,
                value.clone(),
                false,
            );
        }
        Ok(())
    }

    fn reject_promise_with_value(
        &mut self,
        promise: GcRef<JsObject>,
        reason: Value,
    ) -> Result<(), VmError> {
        let reactions = {
            let state = self.promise_state_mut(promise)?;
            match state {
                PromiseState::Pending {
                    fulfill_reactions,
                    reject_reactions,
                } => {
                    fulfill_reactions.clear();
                    let reactions = std::mem::take(reject_reactions);
                    *state = PromiseState::Rejected(reason.clone());
                    reactions
                }
                PromiseState::Fulfilled(_) | PromiseState::Rejected(_) => return Ok(()),
            }
        };
        for reaction in reactions {
            self.enqueue_promise_reaction_job(
                reaction.handler,
                reaction.result_promise,
                reason.clone(),
                true,
            );
        }
        Ok(())
    }

    fn resolve_promise_from_resolution(
        &mut self,
        promise: GcRef<JsObject>,
        value: Value,
    ) -> Result<(), VmError> {
        if matches!(value, Value::Object(object) if object.raw() == promise.raw()) {
            let reason = self.create_error_object(
                "TypeError",
                "Promise cannot be resolved with itself".to_string(),
            );
            return self.reject_promise_with_value(promise, reason);
        }

        if let Value::Object(object) = value.clone()
            && self.is_promise_object(object)
        {
            match self.promise_state(object)?.clone() {
                PromiseState::Pending {
                    fulfill_reactions: _,
                    reject_reactions: _,
                } => {
                    return self.add_promise_reactions(
                        object,
                        PromiseReaction {
                            handler: None,
                            result_promise: Some(promise),
                            is_reject_handler: false,
                        },
                        PromiseReaction {
                            handler: None,
                            result_promise: Some(promise),
                            is_reject_handler: true,
                        },
                    );
                }
                PromiseState::Fulfilled(inner) => {
                    return self.fulfill_promise_with_value(promise, inner);
                }
                PromiseState::Rejected(reason) => {
                    return self.reject_promise_with_value(promise, reason);
                }
            }
        }

        self.fulfill_promise_with_value(promise, value)
    }

    fn promise_resolve_value(&mut self, value: Value) -> Result<GcRef<JsObject>, VmError> {
        if let Value::Object(object) = value {
            if self.is_promise_object(object) {
                return Ok(object);
            }
            let promise = self.allocate_pending_promise_object();
            self.fulfill_promise_with_value(promise, Value::Object(object))?;
            return Ok(promise);
        }

        let promise = self.allocate_pending_promise_object();
        self.fulfill_promise_with_value(promise, value)?;
        Ok(promise)
    }

    fn promise_reject_value(&mut self, reason: Value) -> Result<GcRef<JsObject>, VmError> {
        let promise = self.allocate_pending_promise_object();
        self.reject_promise_with_value(promise, reason)?;
        Ok(promise)
    }

    fn drain_microtasks(&mut self) {
        while let Some(job) = self.event_loop.microtask_queue.pop_front() {
            let _ = self.run_microtask_job(job);
        }
        // End of a microtask checkpoint: notify mutation observers (guarded so
        // the drains it performs don't recurse back here).
        self.deliver_mutation_records();
    }

    fn run_microtask_job(&mut self, job: MicrotaskJob) -> Result<(), VmError> {
        match job {
            MicrotaskJob::PromiseReaction {
                handler,
                result_promise,
                value,
                is_reject,
            } => {
                if let Some(handler_object) = handler {
                    if self
                        .heap
                        .objects()
                        .get(handler_object)
                        .map(|data| matches!(data.kind, ObjectKind::AsyncResumer(_)))
                        .unwrap_or(false)
                    {
                        self.resume_async_context(handler_object, value, is_reject)?;
                        return Ok(());
                    }

                    let outcome = self.call_value_sync(
                        Value::Object(handler_object),
                        Value::Undefined,
                        vec![value.clone()],
                    );
                    if let Some(result_promise) = result_promise {
                        match outcome {
                            Ok(result) => {
                                self.resolve_promise_from_resolution(result_promise, result)?
                            }
                            Err(error) => {
                                let reason = self.wrap_vm_error_as_value(&error)?;
                                self.reject_promise_with_value(result_promise, reason)?;
                            }
                        }
                    }
                } else if let Some(result_promise) = result_promise {
                    if is_reject {
                        self.reject_promise_with_value(result_promise, value)?;
                    } else {
                        self.resolve_promise_from_resolution(result_promise, value)?;
                    }
                }
            }
            MicrotaskJob::QueueMicrotask(callback) => {
                let _ = self.call_value_sync(Value::Object(callback), Value::Undefined, Vec::new());
            }
            MicrotaskJob::AsyncResume {
                resumer,
                value,
                is_throw,
            } => {
                self.resume_async_context(resumer, value, is_throw)?;
            }
        }
        Ok(())
    }

    fn resume_async_context(
        &mut self,
        resumer: GcRef<JsObject>,
        value: Value,
        is_throw: bool,
    ) -> Result<(), VmError> {
        // Extract AsyncContext by swapping the kind to Ordinary — avoids cloning the Box
        let context = match self.heap.objects_mut().get_mut(resumer) {
            Some(obj) => {
                let kind = std::mem::replace(&mut obj.kind, ObjectKind::Ordinary);
                match kind {
                    ObjectKind::AsyncResumer(ctx) => *ctx,
                    other => {
                        obj.kind = other; // restore if wrong type
                        return Err(VmError::TypeError(
                            "invalid async resumer continuation".to_string(),
                        ));
                    }
                }
            }
            None => {
                return Err(VmError::TypeError(
                    "invalid async resumer continuation".to_string(),
                ));
            }
        };
        let base_depth = self.frames.len();
        let mut frame = *context.frame;
        frame.stack_base = self.stack.len();
        self.frames.push(frame);
        self.stack.extend(context.stack_snapshot);
        if is_throw {
            self.handle_runtime_error(VmError::Thrown(value))?;
        } else {
            self.stack.push(value);
        }
        self.run_until_frame_depth(base_depth)?;
        Ok(())
    }

    fn suspend_current_async_frame(&mut self, awaited: Value) -> Result<(), VmError> {
        let awaited_promise = self.promise_resolve_value(awaited)?;
        let frame = self
            .frames
            .pop()
            .ok_or_else(|| VmError::RangeError("await without an async frame".to_string()))?;
        let outer_promise = frame.async_outer_promise;
        let async_generator_request = frame.async_gen_request;
        if outer_promise.is_none() && async_generator_request.is_none() {
            return Err(VmError::TypeError(
                "await expressions are only valid in async frames".to_string(),
            ));
        }
        let stack_snapshot = self.stack.split_off(frame.stack_base);
        let context = AsyncContext {
            frame: Box::new(frame),
            stack_snapshot,
            outer_promise,
            async_generator_request,
        };
        let fulfill_resumer = self.allocate_async_resumer(context.clone());
        let reject_resumer = self.allocate_async_resumer(context);
        self.add_promise_reactions(
            awaited_promise,
            PromiseReaction {
                handler: Some(fulfill_resumer),
                result_promise: None,
                is_reject_handler: false,
            },
            PromiseReaction {
                handler: Some(reject_resumer),
                result_promise: None,
                is_reject_handler: true,
            },
        )
    }

    fn finish_async_frame_with_result(&mut self, result: Value) -> Result<(), VmError> {
        let frame = self
            .frames
            .pop()
            .ok_or_else(|| VmError::RangeError("async return without a frame".to_string()))?;
        let outer_promise = frame.async_outer_promise;
        let async_generator_request = frame.async_gen_request;
        let generator = frame.generator;
        self.stack.truncate(frame.stack_base);
        if let Some(promise) = outer_promise {
            self.resolve_promise_from_resolution(promise, result)
        } else if let Some(request) = async_generator_request {
            let generator = generator.ok_or_else(|| {
                VmError::TypeError("async return without a generator".to_string())
            })?;
            let iter_result = self.make_iter_result(result, true)?;
            self.resolve_promise_from_resolution(request, iter_result)
                .and_then(|_| {
                    let (_state, mut queue) = self.take_async_generator_state(generator).ok_or_else(
                        || VmError::TypeError("async return without a generator".to_string()),
                    )?;
                    self.settle_async_generator_queue_completed(&mut queue)?;
                    self.set_async_generator_state(generator, GeneratorState::Completed, queue);
                    Ok(())
                })
        } else {
            let generator = generator.ok_or_else(|| {
                VmError::TypeError("async return without a generator".to_string())
            })?;
            let (_state, mut queue) = self.take_async_generator_state(generator).ok_or_else(|| {
                VmError::TypeError("async return without a generator".to_string())
            })?;
            self.settle_async_generator_queue_completed(&mut queue)?;
            self.set_async_generator_state(generator, GeneratorState::Completed, queue);
            Ok(())
        }
    }

    fn require_promise_this(
        &self,
        this_value: &Value,
        context: &str,
    ) -> Result<GcRef<JsObject>, VmError> {
        let object = self.require_object_ref(this_value, context)?;
        if !self.is_promise_object(object) {
            return Err(VmError::TypeError(format!(
                "{context} called on non-Promise"
            )));
        }
        Ok(object)
    }

    fn require_callable_object(
        &self,
        value: &Value,
        context: &str,
    ) -> Result<GcRef<JsObject>, VmError> {
        match value {
            Value::Object(object) if self.callables.contains_key(&object.raw()) => Ok(*object),
            _ => Err(VmError::TypeError(format!(
                "{context} requires a callable argument"
            ))),
        }
    }

    fn schedule_timer(
        &mut self,
        callback: GcRef<JsObject>,
        delay_ms: i64,
        interval_ms: Option<u64>,
        args: Vec<Value>,
    ) -> u32 {
        let id = self.event_loop.next_timer_id;
        self.event_loop.next_timer_id = self.event_loop.next_timer_id.wrapping_add(1);
        let due_ms = self
            .event_loop
            .current_time_ms
            .saturating_add(delay_ms.max(0) as u64);
        self.event_loop.timer_heap.push(Reverse(TimerEntry {
            id,
            due_ms,
            interval_ms,
            callback,
            args,
            nesting_level: 0,
        }));
        id
    }

    fn promise_all(&mut self, values: Vec<Value>) -> Result<Value, VmError> {
        let result_promise = self.allocate_pending_promise_object();
        if values.is_empty() {
            let empty = self.make_array_from_values(Vec::new())?;
            self.resolve_promise_from_resolution(result_promise, empty)?;
            return Ok(Value::Object(result_promise));
        }

        let state = PromiseAllState {
            result_promise,
            values: Rc::new(RefCell::new(vec![None; values.len()])),
            remaining: Rc::new(RefCell::new(values.len())),
        };
        let reject_handler = self.allocate_callable_object(
            Callable::PromiseAllReject { result_promise },
            false,
            None,
        );
        for (index, value) in values.into_iter().enumerate() {
            let promise = self.promise_resolve_value(value)?;
            let fulfill_handler = self.allocate_callable_object(
                Callable::PromiseAllResolveElement(PromiseAllResolveElement {
                    state: state.clone(),
                    index,
                }),
                false,
                None,
            );
            self.add_promise_reactions(
                promise,
                PromiseReaction {
                    handler: Some(fulfill_handler),
                    result_promise: None,
                    is_reject_handler: false,
                },
                PromiseReaction {
                    handler: Some(reject_handler),
                    result_promise: None,
                    is_reject_handler: true,
                },
            )?;
        }
        Ok(Value::Object(result_promise))
    }

    fn promise_race(&mut self, values: Vec<Value>) -> Result<Value, VmError> {
        let result_promise = self.allocate_pending_promise_object();
        for value in values {
            let promise = self.promise_resolve_value(value)?;
            let fulfill = self.allocate_callable_object(
                Callable::PromiseRaceResolve { result_promise },
                false,
                None,
            );
            let reject = self.allocate_callable_object(
                Callable::PromiseRaceReject { result_promise },
                false,
                None,
            );
            self.add_promise_reactions(
                promise,
                PromiseReaction {
                    handler: Some(fulfill),
                    result_promise: None,
                    is_reject_handler: false,
                },
                PromiseReaction {
                    handler: Some(reject),
                    result_promise: None,
                    is_reject_handler: true,
                },
            )?;
        }
        Ok(Value::Object(result_promise))
    }

    fn promise_all_settled(&mut self, values: Vec<Value>) -> Result<Value, VmError> {
        let result_promise = self.allocate_pending_promise_object();
        if values.is_empty() {
            let empty = self.make_array_from_values(Vec::new())?;
            self.resolve_promise_from_resolution(result_promise, empty)?;
            return Ok(Value::Object(result_promise));
        }

        let state = PromiseAllState {
            result_promise,
            values: Rc::new(RefCell::new(vec![None; values.len()])),
            remaining: Rc::new(RefCell::new(values.len())),
        };
        for (index, value) in values.into_iter().enumerate() {
            let promise = self.promise_resolve_value(value)?;
            let fulfill = self.allocate_callable_object(
                Callable::PromiseAllSettledElement(PromiseAllSettledElement {
                    state: state.clone(),
                    index,
                    is_reject: false,
                }),
                false,
                None,
            );
            let reject = self.allocate_callable_object(
                Callable::PromiseAllSettledElement(PromiseAllSettledElement {
                    state: state.clone(),
                    index,
                    is_reject: true,
                }),
                false,
                None,
            );
            self.add_promise_reactions(
                promise,
                PromiseReaction {
                    handler: Some(fulfill),
                    result_promise: None,
                    is_reject_handler: false,
                },
                PromiseReaction {
                    handler: Some(reject),
                    result_promise: None,
                    is_reject_handler: true,
                },
            )?;
        }
        Ok(Value::Object(result_promise))
    }

    fn promise_any(&mut self, values: Vec<Value>) -> Result<Value, VmError> {
        let result_promise = self.allocate_pending_promise_object();
        if values.is_empty() {
            let error_object =
                self.create_named_error_object("AggregateError", "All promises were rejected");
            if let Value::Object(object) = error_object.clone() {
                let errors_array = self.make_array_from_values(Vec::new())?;
                self.set_property_on_object(
                    object,
                    error_object.clone(),
                    PropertyKey::from("errors"),
                    errors_array,
                )?;
            }
            self.reject_promise_with_value(result_promise, error_object)?;
            return Ok(Value::Object(result_promise));
        }

        let errors = Rc::new(RefCell::new(vec![None; values.len()]));
        let remaining = Rc::new(RefCell::new(values.len()));
        let resolve = self.allocate_callable_object(
            Callable::PromiseAnyResolve { result_promise },
            false,
            None,
        );
        for (index, value) in values.into_iter().enumerate() {
            let promise = self.promise_resolve_value(value)?;
            let reject = self.allocate_callable_object(
                Callable::PromiseAnyRejectElement(PromiseAnyRejectElement {
                    result_promise,
                    errors: errors.clone(),
                    remaining: remaining.clone(),
                    index,
                }),
                false,
                None,
            );
            self.add_promise_reactions(
                promise,
                PromiseReaction {
                    handler: Some(resolve),
                    result_promise: None,
                    is_reject_handler: false,
                },
                PromiseReaction {
                    handler: Some(reject),
                    result_promise: None,
                    is_reject_handler: true,
                },
            )?;
        }
        Ok(Value::Object(result_promise))
    }

    fn push_call_frame(
        &mut self,
        closure: RuntimeClosure,
        args: Vec<Value>,
        this_value: Value,
        construct_fallback: Option<Value>,
    ) -> Result<(), VmError> {
        if self.frames.len() >= MAX_CALL_FRAMES {
            return Err(VmError::StackOverflow);
        }
        let frame = self.make_call_frame(closure, args, this_value, construct_fallback)?;
        self.frames.push(frame);
        Ok(())
    }

    /// Build (but do not push) a call frame with arguments bound. Used both by
    /// `push_call_frame` and by generator construction (which stores the frame).
    fn make_call_frame(
        &mut self,
        closure: RuntimeClosure,
        args: Vec<Value>,
        this_value: Value,
        construct_fallback: Option<Value>,
    ) -> Result<CallFrame, VmError> {
        let mut locals = Vec::with_capacity(closure.proto.local_count as usize);
        for _ in 0..closure.proto.local_count {
            locals.push(Rc::new(RefCell::new(Value::Undefined)));
        }

        let parameter_count = closure.proto.parameter_count as usize;
        let normal_parameter_count = if closure.proto.has_rest_param {
            parameter_count.saturating_sub(1)
        } else {
            parameter_count
        };

        // Retain the full argument list only when the body uses `arguments`.
        let arguments = if closure.proto.uses_arguments {
            args.clone()
        } else {
            Vec::new()
        };

        for (index, value) in args.iter().cloned().enumerate() {
            if index >= normal_parameter_count || index >= locals.len() {
                break;
            }
            *locals[index].borrow_mut() = value;
        }

        if closure.proto.has_rest_param && parameter_count != 0 {
            let rest_values = args.into_iter().skip(normal_parameter_count).collect();
            let rest_value = self.make_array_from_values(rest_values)?;
            if let Some(slot) = locals.get(parameter_count - 1) {
                *slot.borrow_mut() = rest_value;
            }
        }

        Ok(CallFrame {
            proto: closure.proto,
            ip: 0,
            stack_base: self.stack.len(),
            locals,
            upvalues: closure.upvalues,
            this_value,
            construct_fallback,
            pending_exception: None,
            async_outer_promise: None,
            async_gen_request: None,
            generator: None,
            arguments,
            new_target: Value::Undefined,
        })
    }

    fn invoke_callable_value(
        &mut self,
        callee: Value,
        this_value: Value,
        args: Vec<Value>,
    ) -> Result<Option<Value>, VmError> {
        match self.resolve_callable(&callee)? {
            Callable::Builtin(builtin) => Ok(Some(self.invoke_builtin(builtin, this_value, args)?)),
            Callable::Closure(closure) => {
                if closure.proto.is_generator && closure.proto.is_async {
                    let generator =
                        self.allocate_ordinary_object(Some(self.async_generator_prototype_ref()));
                    let mut frame = self.make_call_frame(closure, args, this_value, None)?;
                    frame.generator = Some(generator);
                    frame.async_gen_request = None;
                    self.set_async_generator_state(
                        generator,
                        GeneratorState::Suspended {
                            frame: Box::new(frame),
                            stack: Vec::new(),
                            started: false,
                        },
                        VecDeque::new(),
                    );
                    Ok(Some(Value::Object(generator)))
                } else if closure.proto.is_generator {
                    // Calling a generator function does not run the body; it
                    // returns a generator object suspended at the start.
                    let generator = self.allocate_ordinary_object(Some(self.generator_prototype_ref()));
                    let mut frame =
                        self.make_call_frame(closure, args, this_value, None)?;
                    frame.generator = Some(generator);
                    self.set_generator_state(
                        generator,
                        GeneratorState::Suspended {
                            frame: Box::new(frame),
                            stack: Vec::new(),
                            started: false,
                        },
                    );
                    Ok(Some(Value::Object(generator)))
                } else if closure.proto.is_async {
                    let outer_promise = self.allocate_pending_promise_object();
                    self.stack.push(Value::Object(outer_promise));
                    self.push_call_frame(closure, args, this_value, None)?;
                    if let Some(frame) = self.frames.last_mut() {
                        frame.async_outer_promise = Some(outer_promise);
                    }
                    Ok(None)
                } else {
                    self.push_call_frame(closure, args, this_value, None)?;
                    Ok(None)
                }
            }
            Callable::Bound(bound) => {
                let mut merged_args = bound.bound_args.clone();
                merged_args.extend(args);
                self.invoke_callable_value(bound.target, bound.bound_this, merged_args)
            }
            Callable::PromiseCapability { promise, mode } => {
                let value = args.first().cloned().unwrap_or(Value::Undefined);
                match mode {
                    PromiseCapabilityMode::Resolve => {
                        self.resolve_promise_from_resolution(promise, value)?;
                    }
                    PromiseCapabilityMode::Reject => {
                        self.reject_promise_with_value(promise, value)?;
                    }
                }
                Ok(Some(Value::Undefined))
            }
            Callable::PromiseFinally { callback, mode } => {
                if self.is_callable_value(&callback) {
                    let _ = self.call_value_sync(callback, Value::Undefined, Vec::new())?;
                }
                let original = args.first().cloned().unwrap_or(Value::Undefined);
                match mode {
                    PromiseFinallyMode::Fulfill => Ok(Some(original)),
                    PromiseFinallyMode::Reject => Err(VmError::Thrown(original)),
                }
            }
            Callable::PromiseAllResolveElement(element) => {
                let value = args.first().cloned().unwrap_or(Value::Undefined);
                let mut values = element.state.values.borrow_mut();
                if values
                    .get(element.index)
                    .map(|entry| entry.is_some())
                    .unwrap_or(false)
                {
                    return Ok(Some(Value::Undefined));
                }
                values[element.index] = Some(value);
                drop(values);
                let mut remaining = element.state.remaining.borrow_mut();
                *remaining = remaining.saturating_sub(1);
                if *remaining == 0 {
                    let values = element
                        .state
                        .values
                        .borrow()
                        .iter()
                        .map(|entry| entry.clone().unwrap_or(Value::Undefined))
                        .collect::<Vec<_>>();
                    let array = self.make_array_from_values(values)?;
                    self.resolve_promise_from_resolution(element.state.result_promise, array)?;
                }
                Ok(Some(Value::Undefined))
            }
            Callable::PromiseAllReject { result_promise }
            | Callable::PromiseRaceReject { result_promise } => {
                let reason = args.first().cloned().unwrap_or(Value::Undefined);
                self.reject_promise_with_value(result_promise, reason)?;
                Ok(Some(Value::Undefined))
            }
            Callable::PromiseRaceResolve { result_promise }
            | Callable::PromiseAnyResolve { result_promise } => {
                let value = args.first().cloned().unwrap_or(Value::Undefined);
                self.resolve_promise_from_resolution(result_promise, value)?;
                Ok(Some(Value::Undefined))
            }
            Callable::PromiseAllSettledElement(element) => {
                let settled_value = args.first().cloned().unwrap_or(Value::Undefined);
                let entry_object = self.allocate_ordinary_object(Some(self.object_prototype_ref()));
                let status = if element.is_reject {
                    self.make_string_value("rejected")
                } else {
                    self.make_string_value("fulfilled")
                };
                self.define_data_property(
                    entry_object,
                    PropertyKey::from("status"),
                    status,
                    true,
                    true,
                    true,
                );
                self.define_data_property(
                    entry_object,
                    if element.is_reject {
                        PropertyKey::from("reason")
                    } else {
                        PropertyKey::from("value")
                    },
                    settled_value,
                    true,
                    true,
                    true,
                );
                let mut values = element.state.values.borrow_mut();
                if values
                    .get(element.index)
                    .map(|entry| entry.is_some())
                    .unwrap_or(false)
                {
                    return Ok(Some(Value::Undefined));
                }
                values[element.index] = Some(Value::Object(entry_object));
                drop(values);
                let mut remaining = element.state.remaining.borrow_mut();
                *remaining = remaining.saturating_sub(1);
                if *remaining == 0 {
                    let values = element
                        .state
                        .values
                        .borrow()
                        .iter()
                        .map(|entry| entry.clone().unwrap_or(Value::Undefined))
                        .collect::<Vec<_>>();
                    let array = self.make_array_from_values(values)?;
                    self.resolve_promise_from_resolution(element.state.result_promise, array)?;
                }
                Ok(Some(Value::Undefined))
            }
            Callable::PromiseAnyRejectElement(element) => {
                let reason = args.first().cloned().unwrap_or(Value::Undefined);
                let mut errors = element.errors.borrow_mut();
                if errors
                    .get(element.index)
                    .map(|entry| entry.is_some())
                    .unwrap_or(false)
                {
                    return Ok(Some(Value::Undefined));
                }
                errors[element.index] = Some(reason);
                drop(errors);
                let mut remaining = element.remaining.borrow_mut();
                *remaining = remaining.saturating_sub(1);
                if *remaining == 0 {
                    let errors = element
                        .errors
                        .borrow()
                        .iter()
                        .map(|entry| entry.clone().unwrap_or(Value::Undefined))
                        .collect::<Vec<_>>();
                    let error_object = self
                        .create_named_error_object("AggregateError", "All promises were rejected");
                    if let Value::Object(object) = error_object.clone() {
                        let errors_array = self.make_array_from_values(errors)?;
                        self.set_property_on_object(
                            object,
                            error_object.clone(),
                            PropertyKey::from("errors"),
                            errors_array,
                        )?;
                    }
                    self.reject_promise_with_value(element.result_promise, error_object)?;
                }
                Ok(Some(Value::Undefined))
            }
        }
    }

    fn call_value_sync(
        &mut self,
        callee: Value,
        this_value: Value,
        args: Vec<Value>,
    ) -> Result<Value, VmError> {
        let base_depth = self.frames.len();
        if let Some(value) = self.invoke_callable_value(callee, this_value, args)? {
            return Ok(value);
        }
        self.run_until_frame_depth(base_depth)?;
        self.pop_value()
    }

    fn construct_value_sync(
        &mut self,
        constructor: Value,
        args: Vec<Value>,
    ) -> Result<Value, VmError> {
        let base_depth = self.frames.len();
        if let Some(value) = self.construct_value(constructor, args)? {
            return Ok(value);
        }
        self.run_until_frame_depth(base_depth)?;
        self.pop_value()
    }

    fn handle_runtime_error(&mut self, error: VmError) -> Result<(), VmError> {
        match error {
            VmError::Thrown(value) => self.handle_thrown_value(value),
            // StackOverflow is a catchable `RangeError` in real engines
            // (`try { recurse() } catch (e) { … }`), so route it through the same
            // handler-unwinding path. Unwinding to a `try` also frees the frames
            // that hit the cap. With no handler, it still surfaces as uncaught.
            VmError::TypeError(_)
            | VmError::ReferenceError(_)
            | VmError::RangeError(_)
            | VmError::StackOverflow => {
                let wrapped = self.wrap_vm_error_as_value(&error)?;
                match self.handle_thrown_value(wrapped) {
                    Ok(()) => Ok(()),
                    Err(_) => Err(error),
                }
            }
            other => Err(other),
        }
    }

    fn handle_thrown_value(&mut self, value: Value) -> Result<(), VmError> {
        let mut thrown = value;
        self.last_backtrace = Some(self.capture_backtrace());
        loop {
            let Some(frame_index) = self.frames.len().checked_sub(1) else {
                return Err(VmError::TypeError(format!(
                    "uncaught throw: {}",
                    self.describe_thrown_value(&thrown)
                )));
            };
            let ip = self.frames[frame_index].ip.saturating_sub(1) as u32;
            let handler = self.frames[frame_index]
                .proto
                .handlers
                .iter()
                .rev()
                .find(|handler| handler.try_start <= ip && ip < handler.try_end)
                .cloned();
            if let Some(handler) = handler {
                if let Some(slot) = handler.catch_binding {
                    if let Some(cell) = self.local_cell_in_frame(frame_index, slot) {
                        *cell.borrow_mut() = thrown.clone();
                    }
                }
                if handler.catch_ip != 0 {
                    self.frames[frame_index].pending_exception = None;
                    self.frames[frame_index].ip = handler.catch_ip as usize;
                    return Ok(());
                }
                if handler.finally_ip != 0 {
                    self.frames[frame_index].pending_exception = Some(thrown.clone());
                    self.frames[frame_index].ip = handler.finally_ip as usize;
                    return Ok(());
                }
            }

            if let Some(outer_promise) = self.frames[frame_index].async_outer_promise {
                let frame = self.frames.pop().ok_or_else(|| {
                    VmError::RangeError("async exception propagation without a frame".to_string())
                })?;
                self.stack.truncate(frame.stack_base);
                self.reject_promise_with_value(outer_promise, thrown)?;
                return Ok(());
            } else if let Some(request) = self.frames[frame_index].async_gen_request {
                let frame = self.frames.pop().ok_or_else(|| {
                    VmError::RangeError("async exception propagation without a frame".to_string())
                })?;
                self.stack.truncate(frame.stack_base);
                let generator = frame.generator.ok_or_else(|| {
                    VmError::TypeError("async generator frame must have generator".to_string())
                })?;
                if let Some((_, mut queue)) = self.take_async_generator_state(generator) {
                    self.settle_async_generator_queue_completed(&mut queue)?;
                    self.set_async_generator_state(generator, GeneratorState::Completed, queue);
                }
                self.reject_promise_with_value(request, thrown)?;
                return Ok(());
            }

            let frame = self.frames.pop().ok_or_else(|| {
                VmError::RangeError("exception propagation without a frame".to_string())
            })?;
            self.stack.truncate(frame.stack_base);
            thrown = frame.pending_exception.unwrap_or(thrown);
        }
    }

    fn construct_value(
        &mut self,
        constructor: Value,
        args: Vec<Value>,
    ) -> Result<Option<Value>, VmError> {
        match self.resolve_callable(&constructor)? {
            Callable::Builtin(builtin) => {
                if !self.builtin_constructable(builtin) {
                    return Err(VmError::TypeError(
                        "attempted to construct a non-constructor value".to_string(),
                    ));
                }
                Ok(Some(self.invoke_builtin(
                    builtin,
                    Value::Undefined,
                    args,
                )?))
            }
            Callable::Closure(closure) => {
                if closure.proto.is_async || closure.proto.is_generator {
                    return Err(VmError::TypeError(
                        "attempted to construct a non-constructor value".to_string(),
                    ));
                }
                let this_value = self.construct_this_value(&constructor)?;
                self.push_call_frame(closure, args, this_value.clone(), Some(this_value))?;
                if let Some(frame) = self.frames.last_mut() {
                    frame.new_target = constructor.clone();
                }
                Ok(None)
            }
            Callable::Bound(bound) => {
                let mut merged_args = bound.bound_args.clone();
                merged_args.extend(args);
                self.construct_value(bound.target, merged_args)
            }
            Callable::PromiseCapability { .. }
            | Callable::PromiseFinally { .. }
            | Callable::PromiseAllResolveElement(_)
            | Callable::PromiseAllReject { .. }
            | Callable::PromiseRaceResolve { .. }
            | Callable::PromiseRaceReject { .. }
            | Callable::PromiseAllSettledElement(_)
            | Callable::PromiseAnyResolve { .. }
            | Callable::PromiseAnyRejectElement(_) => Err(VmError::TypeError(
                "attempted to construct a non-constructor value".to_string(),
            )),
        }
    }

    fn builtin_constructable(&self, builtin: BuiltinId) -> bool {
        matches!(
            builtin,
            BuiltinId::ObjectConstructor
                | BuiltinId::ArrayConstructor
                | BuiltinId::FunctionConstructor
                | BuiltinId::PromiseConstructor
                | BuiltinId::ErrorConstructor
                | BuiltinId::TypeErrorConstructor
                | BuiltinId::RangeErrorConstructor
                | BuiltinId::ReferenceErrorConstructor
                | BuiltinId::SyntaxErrorConstructor
                | BuiltinId::UriErrorConstructor
                | BuiltinId::EvalErrorConstructor
                | BuiltinId::MapConstructor
                | BuiltinId::SetConstructor
                | BuiltinId::WeakMapConstructor
                | BuiltinId::WeakSetConstructor
                | BuiltinId::RegExpConstructor
                | BuiltinId::NumberConstructor
                | BuiltinId::StringConstructor
                | BuiltinId::BooleanConstructor
                | BuiltinId::DateConstructor
                | BuiltinId::ProxyConstructor
                | BuiltinId::UrlSearchParamsConstructor
                | BuiltinId::HeadersConstructor
                | BuiltinId::HeadersGet
                | BuiltinId::HeadersSet
                | BuiltinId::HeadersHas
                | BuiltinId::HeadersAppend
                | BuiltinId::HeadersDelete
                | BuiltinId::HeadersForEach
                | BuiltinId::HeadersEntries
                | BuiltinId::HeadersKeys
                | BuiltinId::HeadersValues
                | BuiltinId::FormDataConstructor
                | BuiltinId::FormDataGet
                | BuiltinId::FormDataGetAll
                | BuiltinId::FormDataHas
                | BuiltinId::FormDataSet
                | BuiltinId::FormDataAppend
                | BuiltinId::FormDataDelete
                | BuiltinId::FormDataForEach
                | BuiltinId::FormDataEntries
                | BuiltinId::FormDataKeys
                | BuiltinId::FormDataValues
                | BuiltinId::WeakRefConstructor
                | BuiltinId::TextEncoderConstructor
                | BuiltinId::TextDecoderConstructor
                | BuiltinId::UrlConstructor
                | BuiltinId::ArrayBufferConstructor
                | BuiltinId::TypedArrayConstructor(_)
                | BuiltinId::EventConstructor
                | BuiltinId::CustomEventConstructor
                | BuiltinId::KeyboardEventConstructor
                | BuiltinId::MouseEventConstructor
                | BuiltinId::AbortControllerConstructor
                | BuiltinId::MutationObserverConstructor
                | BuiltinId::ImageConstructor
                | BuiltinId::XhrConstructor
                | BuiltinId::IntersectionObserverConstructor
                | BuiltinId::ResizeObserverConstructor
                | BuiltinId::CryptoGetRandomValues
                | BuiltinId::CryptoRandomUUID
        )
    }

    fn construct_this_value(&mut self, constructor: &Value) -> Result<Value, VmError> {
        let prototype = match constructor {
            Value::Object(object) => {
                match self.get_own_property_descriptor(*object, &PropertyKey::from("prototype")) {
                    Some(JsPropertyDescriptor::Data {
                        value: Value::Object(prototype),
                        ..
                    }) => Some(prototype),
                    _ => Some(self.object_prototype_ref()),
                }
            }
            _ => Some(self.object_prototype_ref()),
        };
        Ok(Value::Object(self.allocate_ordinary_object(prototype)))
    }

    fn resolve_callable(&self, value: &Value) -> Result<Callable, VmError> {
        let object = match value {
            Value::Object(object) => object.raw(),
            _ => {
                let described = match value {
                    Value::Undefined => "undefined".to_string(),
                    Value::Null => "null".to_string(),
                    Value::Bool(_) => "boolean".to_string(),
                    Value::Number(_) => "number".to_string(),
                    Value::String(_) => "string".to_string(),
                    Value::Object(_) => "object".to_string(),
                    Value::Symbol(_) => "symbol".to_string(),
                };
                let message = match &self.pending_call_name {
                    Some(name) => format!("{name} is not a function ({described})"),
                    None => format!("attempted to call a non-function value ({described})"),
                };
                return Err(VmError::TypeError(
                    message,
                ));
            }
        };

        self.callables
            .get(&object)
            .cloned()
            .ok_or_else(|| VmError::TypeError("object is not callable".to_string()))
    }

    fn current_proto(&self) -> Result<&FunctionProto, VmError> {
        self.frames
            .last()
            .map(|frame| frame.proto.as_ref())
            .ok_or_else(|| VmError::RangeError("no current function prototype".to_string()))
    }

    /// Whether the currently executing function is in strict mode. Assignments
    /// that fail (non-writable property, non-extensible object) throw in strict
    /// mode but are silent no-ops in sloppy mode.
    fn in_strict_mode(&self) -> bool {
        self.frames
            .last()
            .map(|frame| frame.proto.is_strict)
            .unwrap_or(false)
    }

    fn current_this(&self) -> Result<&Value, VmError> {
        self.frames
            .last()
            .map(|frame| &frame.this_value)
            .ok_or_else(|| VmError::RangeError("no current this binding".to_string()))
    }

    fn constant_name(&self, index: u16) -> Result<&str, VmError> {
        match self.current_proto()?.constants.get(index as usize) {
            Some(Constant::String(value)) => Ok(value.as_str()),
            Some(Constant::Number(_)) => Err(VmError::TypeError(format!(
                "constant {index} was not a string"
            ))),
            Some(Constant::RegExp { .. }) => Err(VmError::TypeError(format!(
                "constant {index} was not a string"
            ))),
            None => Err(VmError::RangeError(format!(
                "constant index {index} out of range"
            ))),
        }
    }

    fn constant_regexp(&self, index: u16) -> Result<(String, String), VmError> {
        match self.current_proto()?.constants.get(index as usize) {
            Some(Constant::RegExp { pattern, flags }) => Ok((pattern.clone(), flags.clone())),
            _ => Err(VmError::TypeError(format!(
                "constant {index} was not a regular expression"
            ))),
        }
    }

    fn local_cell(&self, slot: u16) -> Result<&ValueCell, VmError> {
        self.frames
            .last()
            .and_then(|frame| frame.locals.get(slot as usize))
            .ok_or_else(|| VmError::RangeError(format!("local slot {slot} out of range")))
    }

    fn local_cell_in_frame(&self, frame_index: usize, slot: u16) -> Option<ValueCell> {
        self.frames
            .get(frame_index)
            .and_then(|frame| frame.locals.get(slot as usize))
            .cloned()
    }

    fn upvalue_cell(&self, slot: u16) -> Result<&ValueCell, VmError> {
        self.frames
            .last()
            .and_then(|frame| frame.upvalues.get(slot as usize))
            .ok_or_else(|| VmError::RangeError(format!("upvalue slot {slot} out of range")))
    }

    fn pop_value(&mut self) -> Result<Value, VmError> {
        self.stack
            .pop()
            .ok_or_else(|| VmError::RangeError("operand stack underflow".to_string()))
    }

    fn peek_value(&self) -> Result<&Value, VmError> {
        self.stack
            .last()
            .ok_or_else(|| VmError::RangeError("operand stack underflow".to_string()))
    }

    fn pop_args(&mut self, argc: u8) -> Result<Vec<Value>, VmError> {
        let mut args = Vec::with_capacity(argc as usize);
        for _ in 0..argc {
            args.push(self.pop_value()?);
        }
        args.reverse();
        Ok(args)
    }

    fn pop_args_u16(&mut self, argc: u16) -> Result<Vec<Value>, VmError> {
        let mut args = Vec::with_capacity(argc as usize);
        for _ in 0..argc {
            args.push(self.pop_value()?);
        }
        args.reverse();
        Ok(args)
    }

    fn apply_jump(&mut self, offset: i32) -> Result<(), VmError> {
        let frame = self
            .frames
            .last_mut()
            .ok_or_else(|| VmError::RangeError("no call frame available".to_string()))?;
        if offset < 0 {
            self.fuel = self.fuel.checked_sub(1).ok_or(VmError::InfiniteLoop)?;
        }
        let target = frame.ip as i64 + i64::from(offset);
        if target < 0 {
            return Err(VmError::RangeError(
                "jump moved before start of bytecode".to_string(),
            ));
        }
        frame.ip = usize::try_from(target)
            .map_err(|_| VmError::RangeError("jump target exceeded usize".to_string()))?;
        Ok(())
    }

    fn make_string_value(&mut self, text: &str) -> Value {
        if let Some(gc_ref) = self.string_cache.get(text) {
            return Value::String(*gc_ref);
        }
        let owned = text.to_string();
        let gc_ref = self.heap.allocate_string(JsString::from(owned.clone()));
        self.string_cache.insert(owned, gc_ref);
        Value::String(gc_ref)
    }

    fn string_text(&self, gc_ref: GcRef<JsString>) -> String {
        self.heap
            .strings()
            .get(gc_ref)
            .map(|string| string.text.clone())
            .unwrap_or_default()
    }

    fn typeof_name(&self, value: &Value) -> &'static str {
        match value {
            Value::Object(object) if self.callables.contains_key(&object.raw()) => "function",
            _ => value.type_name(),
        }
    }

    fn is_truthy(&self, value: &Value) -> bool {
        match value {
            Value::Undefined | Value::Null => false,
            Value::Bool(boolean) => *boolean,
            Value::Number(number) => *number != 0.0 && !number.is_nan(),
            Value::String(string) => !self.string_text(*string).is_empty(),
            Value::Object(_) | Value::Symbol(_) => true,
        }
    }

    fn to_number(&self, value: &Value) -> f64 {
        match value {
            Value::Undefined => f64::NAN,
            Value::Null => 0.0,
            Value::Bool(false) => 0.0,
            Value::Bool(true) => 1.0,
            Value::Number(number) => *number,
            Value::String(string) => {
                let text = self.string_text(*string);
                let trimmed = text.trim();
                if trimmed.is_empty() {
                    0.0
                } else {
                    trimmed.parse::<f64>().unwrap_or(f64::NAN)
                }
            }
            Value::Object(_) | Value::Symbol(_) => f64::NAN,
        }
    }

    fn to_int32(&self, value: &Value) -> i32 {
        self.to_number(value) as i32
    }

    fn to_uint32(&self, value: &Value) -> u32 {
        self.to_number(value) as u32
    }

    /// Read a data property by walking the prototype chain (immutably).
    /// Accessor properties are ignored. Used for best-effort introspection
    /// where running getters is undesirable (e.g. error reporting).
    fn read_data_property_chain(&self, start: GcRef<JsObject>, key: &PropertyKey) -> Option<Value> {
        let mut current = Some(start);
        while let Some(object) = current {
            let obj = self.heap.objects().get(object)?;
            if let Some(JsPropertyDescriptor::Data { value, .. }) = obj.properties.get(key) {
                return Some(value.clone());
            }
            current = obj.prototype;
        }
        None
    }

    /// Describe an uncaught thrown value for diagnostics. For Error-like
    /// objects this yields "Name: message" (matching `Error.prototype.toString`)
    /// instead of the generic "[object Object]" that `to_string` would produce.
    fn describe_thrown_value(&self, value: &Value) -> String {
        if let Value::Object(object) = value
            && !self.callables.contains_key(&object.raw())
        {
            let name = self
                .read_data_property_chain(*object, &PropertyKey::from("name"))
                .map(|value| self.to_string(&value))
                .filter(|text| !text.is_empty());
            let message = self
                .read_data_property_chain(*object, &PropertyKey::from("message"))
                .map(|value| self.to_string(&value))
                .filter(|text| !text.is_empty());
            match (name, message) {
                (Some(name), Some(message)) => return format!("{name}: {message}"),
                (Some(name), None) => return name,
                (None, Some(message)) => return message,
                (None, None) => {}
            }
        }
        self.to_string(value)
    }

    fn to_string(&self, value: &Value) -> String {
        match value {
            Value::Undefined => "undefined".to_string(),
            Value::Null => "null".to_string(),
            Value::Bool(boolean) => boolean.to_string(),
            Value::Number(number) => Self::format_number(*number),
            Value::String(string) => self.string_text(*string),
            Value::Object(object) => {
                if self.callables.contains_key(&object.raw()) {
                    "function() { [native code] }".to_string()
                } else if let Some(href) = self.location_href_for_stringify(*object) {
                    // `String(location)` / ``${location}`` / `new URL(".", location)`
                    // must yield the href, not the generic "[object Object]".
                    href
                } else {
                    "[object Object]".to_string()
                }
            }
            Value::Symbol(_) => "Symbol()".to_string(),
        }
    }

    /// If `object` is the `Location` host object, return its `href` so it
    /// stringifies like a real browser `Location` (whose string value is href).
    fn location_href_for_stringify(&self, object: GcRef<JsObject>) -> Option<String> {
        let is_location = self
            .heap
            .objects()
            .get(object)
            .is_some_and(|o| matches!(&o.kind, ObjectKind::Host(slot) if slot.class == HostObjectClass::Other("Location")));
        if !is_location {
            return None;
        }
        self.host.location(WindowId(0)).ok().map(|l| l.href)
    }

    /// ECMAScript `ToPrimitive`. `prefer`: `Some(true)` = string hint,
    /// `Some(false)` = number hint, `None` = default. Honors `Symbol.toPrimitive`,
    /// then `valueOf`/`toString` in the hint-appropriate order. Non-objects are
    /// returned unchanged. This is what makes `'' + obj`, `+obj`, `obj * 2`,
    /// template `${obj}`, and `'' + [1,2,3]` (Array.prototype.toString) work.
    fn to_primitive(&mut self, value: &Value, prefer: Option<bool>) -> Result<Value, VmError> {
        if !matches!(value, Value::Object(_)) {
            return Ok(value.clone());
        }
        // 1. Exotic @@toPrimitive, if present and callable.
        let exotic = self.get_property_value(
            value,
            &PropertyKey::Symbol(SymbolId(SYMBOL_TO_PRIMITIVE_ID)),
        )?;
        if self.is_callable_value(&exotic) {
            let hint = match prefer {
                Some(true) => "string",
                Some(false) => "number",
                None => "default",
            };
            let hint_val = self.make_string_value(hint);
            let result = self.call_value_sync(exotic, value.clone(), vec![hint_val])?;
            if matches!(result, Value::Object(_)) {
                return Err(VmError::TypeError(
                    "Symbol.toPrimitive must return a primitive value".to_string(),
                ));
            }
            return Ok(result);
        }
        // 2. OrdinaryToPrimitive: try valueOf/toString in hint order.
        let methods: [&str; 2] = if prefer == Some(true) {
            ["toString", "valueOf"]
        } else {
            ["valueOf", "toString"]
        };
        for name in methods {
            let method = self.get_property_value(value, &PropertyKey::from(name))?;
            if self.is_callable_value(&method) {
                let result = self.call_value_sync(method, value.clone(), Vec::new())?;
                if !matches!(result, Value::Object(_)) {
                    return Ok(result);
                }
            }
        }
        Err(VmError::TypeError(
            "cannot convert object to a primitive value".to_string(),
        ))
    }

    /// `ToNumber` that first runs `ToPrimitive` (number hint) so objects with
    /// `valueOf`/`Symbol.toPrimitive` coerce correctly.
    fn to_number_coerced(&mut self, value: &Value) -> Result<f64, VmError> {
        let primitive = self.to_primitive(value, Some(false))?;
        Ok(self.to_number(&primitive))
    }

    /// `ToString` that first runs `ToPrimitive` (string hint) so objects with
    /// `toString`/`Symbol.toPrimitive` (and arrays) coerce correctly.
    fn to_string_coerced(&mut self, value: &Value) -> Result<String, VmError> {
        let primitive = self.to_primitive(value, Some(true))?;
        Ok(self.to_string(&primitive))
    }

    fn format_number(number: f64) -> String {
        if number.is_nan() {
            return "NaN".to_string();
        }
        if number == f64::INFINITY {
            return "Infinity".to_string();
        }
        if number == f64::NEG_INFINITY {
            return "-Infinity".to_string();
        }
        if number == 0.0 {
            return "0".to_string();
        }
        // Fast path: integers below the 1e21 exponential threshold print plainly
        // (covers the overwhelmingly common case without the parsing below).
        if number.fract() == 0.0 && number.abs() < 1e21 {
            return format!("{number:.0}");
        }
        Self::format_number_general(number)
    }

    /// ECMAScript `Number::toString` (base 10) for the cases that need it:
    /// fractional values and magnitudes that switch to exponential notation
    /// (exponent >= 21 or <= -7). Uses Rust's `{:e}` to obtain the shortest
    /// round-tripping mantissa/exponent, then reformats per the spec.
    fn format_number_general(number: f64) -> String {
        let negative = number < 0.0;
        let abs = number.abs();
        // e.g. "1.2345e2", "1e21", "5e-1" — shortest round-trip form.
        let exp_repr = format!("{abs:e}");
        let (mantissa, exp_str) = match exp_repr.split_once('e') {
            Some(parts) => parts,
            None => return number.to_string(),
        };
        let exponent: i32 = match exp_str.parse() {
            Ok(value) => value,
            Err(_) => return number.to_string(),
        };
        // Significant digits with the decimal point removed (no leading or
        // trailing zeros, since this is the shortest representation).
        let digits: String = mantissa.chars().filter(|c| *c != '.').collect();
        let s = if digits.is_empty() { "0" } else { digits.as_str() };
        let k = s.len() as i32;
        // value == s x 10^(n-k); equivalently n == exponent + 1.
        let n = exponent + 1;

        let body = if k <= n && n <= 21 {
            // Integer: digits followed by (n - k) zeros.
            let mut out = String::from(s);
            out.push_str(&"0".repeat((n - k) as usize));
            out
        } else if 0 < n && n <= 21 {
            // Decimal point inside the digit run.
            let (int_part, frac_part) = s.split_at(n as usize);
            format!("{int_part}.{frac_part}")
        } else if -6 < n && n <= 0 {
            // 0.00..digits with (-n) leading zeros after the point.
            format!("0.{}{}", "0".repeat((-n) as usize), s)
        } else if k == 1 {
            // Single-digit mantissa in exponential form.
            let e = n - 1;
            format!("{s}e{}{}", if e >= 0 { "+" } else { "-" }, e.abs())
        } else {
            // Multi-digit mantissa in exponential form.
            let (first, rest) = s.split_at(1);
            let e = n - 1;
            format!(
                "{first}.{rest}e{}{}",
                if e >= 0 { "+" } else { "-" },
                e.abs()
            )
        };

        if negative {
            format!("-{body}")
        } else {
            body
        }
    }

    fn strict_equal(&self, lhs: &Value, rhs: &Value) -> bool {
        match (lhs, rhs) {
            (Value::Undefined, Value::Undefined) | (Value::Null, Value::Null) => true,
            (Value::Bool(left), Value::Bool(right)) => left == right,
            (Value::Number(left), Value::Number(right)) => {
                !left.is_nan() && !right.is_nan() && left == right
            }
            (Value::String(left), Value::String(right)) => {
                self.string_text(*left) == self.string_text(*right)
            }
            (Value::Object(left), Value::Object(right)) => left.raw() == right.raw(),
            (Value::Symbol(left), Value::Symbol(right)) => left == right,
            _ => false,
        }
    }

    fn same_value_zero(&self, lhs: &Value, rhs: &Value) -> bool {
        self.strict_equal(lhs, rhs)
            || matches!((lhs, rhs), (Value::Number(left), Value::Number(right)) if left.is_nan() && right.is_nan())
    }

    fn abstract_equal(&self, lhs: &Value, rhs: &Value) -> bool {
        if std::mem::discriminant(lhs) == std::mem::discriminant(rhs) {
            return self.strict_equal(lhs, rhs);
        }

        match (lhs, rhs) {
            (Value::Null, Value::Undefined) | (Value::Undefined, Value::Null) => true,
            (Value::Number(_), Value::String(_)) => {
                self.abstract_equal(lhs, &Value::Number(self.to_number(rhs)))
            }
            (Value::String(_), Value::Number(_)) => {
                self.abstract_equal(&Value::Number(self.to_number(lhs)), rhs)
            }
            (Value::Bool(_), _) => self.abstract_equal(&Value::Number(self.to_number(lhs)), rhs),
            (_, Value::Bool(_)) => self.abstract_equal(lhs, &Value::Number(self.to_number(rhs))),
            _ => false,
        }
    }

    fn binary_add(&mut self) -> Result<(), VmError> {
        let rhs = self.pop_value()?;
        let lhs = self.pop_value()?;
        // ToPrimitive(default) both operands first (objects/arrays → primitives),
        // then string-concat if either side is now a string, else numeric add.
        let lhs = self.to_primitive(&lhs, None)?;
        let rhs = self.to_primitive(&rhs, None)?;
        if matches!(lhs, Value::String(_)) || matches!(rhs, Value::String(_)) {
            let text = format!("{}{}", self.to_string(&lhs), self.to_string(&rhs));
            let string_value = self.make_string_value(&text);
            self.stack.push(string_value);
        } else {
            self.stack
                .push(Value::Number(self.to_number(&lhs) + self.to_number(&rhs)));
        }
        Ok(())
    }

    fn binary_numeric<F>(&mut self, operator: F) -> Result<(), VmError>
    where
        F: FnOnce(f64, f64) -> f64,
    {
        let rhs = self.pop_value()?;
        let lhs = self.pop_value()?;
        let lhs = self.to_number_coerced(&lhs)?;
        let rhs = self.to_number_coerced(&rhs)?;
        self.stack.push(Value::Number(operator(lhs, rhs)));
        Ok(())
    }

    fn binary_compare<F>(&mut self, operator: F) -> Result<(), VmError>
    where
        F: FnOnce(&Vm, &Value, &Value) -> bool,
    {
        let rhs = self.pop_value()?;
        let lhs = self.pop_value()?;
        self.stack.push(Value::Bool(operator(self, &lhs, &rhs)));
        Ok(())
    }

    fn binary_compare_numeric_or_string<F>(&mut self, operator: F) -> Result<(), VmError>
    where
        F: FnOnce(f64, f64) -> bool + Copy,
    {
        let rhs = self.pop_value()?;
        let lhs = self.pop_value()?;
        // Relational comparison runs ToPrimitive(number hint) on both sides; if
        // both come back as strings, compare lexicographically, else numerically.
        let lhs = self.to_primitive(&lhs, Some(false))?;
        let rhs = self.to_primitive(&rhs, Some(false))?;
        let result = match (&lhs, &rhs) {
            (Value::String(left), Value::String(right)) => {
                self.string_text(*left).cmp(&self.string_text(*right))
            }
            _ => {
                let left = self.to_number(&lhs);
                let right = self.to_number(&rhs);
                self.stack.push(Value::Bool(operator(left, right)));
                return Ok(());
            }
        };
        let ordered = match result {
            Ordering::Less => operator(-1.0, 0.0),
            Ordering::Equal => operator(0.0, 0.0),
            Ordering::Greater => operator(1.0, 0.0),
        };
        self.stack.push(Value::Bool(ordered));
        Ok(())
    }

    fn binary_bitwise<F>(&mut self, operator: F) -> Result<(), VmError>
    where
        F: FnOnce(i32, i32) -> i32,
    {
        let rhs = self.pop_value()?;
        let lhs = self.pop_value()?;
        self.stack.push(Value::Number(f64::from(operator(
            self.to_int32(&lhs),
            self.to_int32(&rhs),
        ))));
        Ok(())
    }

    fn binary_shift<F>(&mut self, operator: F) -> Result<(), VmError>
    where
        F: FnOnce(i32, u32) -> i32,
    {
        let rhs = self.pop_value()?;
        let lhs = self.pop_value()?;
        self.stack.push(Value::Number(f64::from(operator(
            self.to_int32(&lhs),
            self.to_uint32(&rhs),
        ))));
        Ok(())
    }

    fn binary_unsigned_shift(&mut self) -> Result<(), VmError> {
        let rhs = self.pop_value()?;
        let lhs = self.pop_value()?;
        let shifted = self
            .to_uint32(&lhs)
            .wrapping_shr(self.to_uint32(&rhs) & 0x1f);
        self.stack.push(Value::Number(f64::from(shifted)));
        Ok(())
    }

    fn to_property_key(&self, value: &Value) -> Result<PropertyKey, VmError> {
        Ok(match value {
            Value::String(string) => Self::property_key_from_text(&self.string_text(*string)),
            Value::Number(number)
                if number.is_finite() && *number >= 0.0 && number.fract() == 0.0 =>
            {
                PropertyKey::Index(*number as u32)
            }
            Value::Symbol(symbol) => PropertyKey::Symbol(*symbol),
            _ => Self::property_key_from_text(&self.to_string(value)),
        })
    }

    fn property_key_from_text(text: &str) -> PropertyKey {
        if let Ok(index) = text.parse::<u32>() {
            if index.to_string() == text {
                return PropertyKey::Index(index);
            }
        }
        PropertyKey::String(text.to_string())
    }

    fn property_key_to_string(&self, key: &PropertyKey) -> String {
        match key {
            PropertyKey::String(text) => text.clone(),
            PropertyKey::Index(index) => index.to_string(),
            PropertyKey::Symbol(_) => "Symbol()".to_string(),
        }
    }

    /// Convert a property key into the Value a Proxy/Reflect trap receives.
    fn property_key_to_value(&mut self, key: &PropertyKey) -> Value {
        match key {
            PropertyKey::Symbol(symbol) => Value::Symbol(*symbol),
            other => {
                let text = self.property_key_to_string(other);
                self.make_string_value(&text)
            }
        }
    }

    /// Proxy `get`: invoke the handler's get trap, else forward to the target.
    fn proxy_get(
        &mut self,
        target: GcRef<JsObject>,
        handler: GcRef<JsObject>,
        key: &PropertyKey,
        receiver: &Value,
    ) -> Result<Value, VmError> {
        let trap = self.get_property_value(&Value::Object(handler), &PropertyKey::from("get"))?;
        if self.is_callable_value(&trap) {
            let key_value = self.property_key_to_value(key);
            return self.call_value_sync(
                trap,
                Value::Object(handler),
                vec![Value::Object(target), key_value, receiver.clone()],
            );
        }
        self.get_property_value(&Value::Object(target), key)
    }

    /// Proxy `set`: invoke the handler's set trap, else forward to the target.
    fn proxy_set(
        &mut self,
        target: GcRef<JsObject>,
        handler: GcRef<JsObject>,
        key: PropertyKey,
        value: Value,
        receiver: Value,
    ) -> Result<(), VmError> {
        let trap = self.get_property_value(&Value::Object(handler), &PropertyKey::from("set"))?;
        if self.is_callable_value(&trap) {
            let key_value = self.property_key_to_value(&key);
            self.call_value_sync(
                trap,
                Value::Object(handler),
                vec![Value::Object(target), key_value, value, receiver],
            )?;
            return Ok(());
        }
        self.set_property_value(&Value::Object(target), key, value)
    }

    /// Proxy `has` (`in` operator): invoke the handler's has trap, else forward.
    fn proxy_has(
        &mut self,
        target: GcRef<JsObject>,
        handler: GcRef<JsObject>,
        key: &PropertyKey,
    ) -> Result<bool, VmError> {
        let trap = self.get_property_value(&Value::Object(handler), &PropertyKey::from("has"))?;
        if self.is_callable_value(&trap) {
            let key_value = self.property_key_to_value(key);
            let result = self.call_value_sync(
                trap,
                Value::Object(handler),
                vec![Value::Object(target), key_value],
            )?;
            return Ok(self.is_truthy(&result));
        }
        Ok(self.lookup_property_descriptor(target, key).is_some())
    }

    fn value_object_ref(&self, value: Value) -> Option<GcRef<JsObject>> {
        match value {
            Value::Object(object) => Some(object),
            _ => None,
        }
    }

    fn require_object_ref(&self, value: &Value, context: &str) -> Result<GcRef<JsObject>, VmError> {
        match value {
            Value::Object(object) => Ok(*object),
            _ => Err(VmError::TypeError(format!("{context} requires an object"))),
        }
    }

    fn try_require_object_ref(
        &self,
        value: &Value,
        context: &str,
    ) -> Result<Option<GcRef<JsObject>>, VmError> {
        match value {
            Value::Object(object) => Ok(Some(*object)),
            Value::Null | Value::Undefined => {
                Err(VmError::TypeError(format!("{context} requires an object")))
            }
            _ => Ok(None),
        }
    }

    fn object_introspection_primitive_prototype_ref(
        &self,
        value: &Value,
    ) -> GcRef<JsObject> {
        match value {
            Value::String(_) => self.string_prototype_ref(),
            Value::Number(_) => self.number_prototype_ref(),
            Value::Bool(_) => self.boolean_prototype_ref(),
            _ => self.object_prototype_ref(),
        }
    }

    fn get_own_property_descriptor(
        &self,
        object: GcRef<JsObject>,
        key: &PropertyKey,
    ) -> Option<JsPropertyDescriptor> {
        self.heap
            .objects()
            .get(object)
            .and_then(|object| object.properties.get(key).cloned())
    }

    fn lookup_property_descriptor(
        &self,
        object: GcRef<JsObject>,
        key: &PropertyKey,
    ) -> Option<(GcRef<JsObject>, JsPropertyDescriptor)> {
        let mut current = Some(object);
        while let Some(object_ref) = current {
            let object_data = self.heap.objects().get(object_ref)?;
            if let Some(descriptor) = object_data.properties.get(key).cloned() {
                return Some((object_ref, descriptor));
            }
            current = object_data.prototype;
        }
        None
    }

    fn get_property_value(
        &mut self,
        receiver: &Value,
        key: &PropertyKey,
    ) -> Result<Value, VmError> {
        match receiver {
            Value::Object(object) => self.get_property_from_chain(*object, receiver, key),
            Value::String(string) => self.get_property_from_string(*string, receiver, key),
            Value::Number(_) => {
                let proto = self.number_prototype_ref();
                self.get_property_from_chain(proto, receiver, key)
            }
            Value::Bool(_) => {
                let proto = self.boolean_prototype_ref();
                self.get_property_from_chain(proto, receiver, key)
            }
            Value::Symbol(SymbolId(id)) => {
                if let PropertyKey::String(name) = key {
                    if name == "description" {
                        let description = self.symbol_descriptions.get(id).cloned();
                        return Ok(description
                            .map(|d| self.make_string_value(&d))
                            .unwrap_or(Value::Undefined));
                    }
                    if name == "toString" || name == "valueOf" {
                        return Ok(self.allocate_builtin_method(BuiltinId::SymbolProtoToString));
                    }
                }
                Ok(Value::Undefined)
            }
            Value::Null | Value::Undefined => {
                let what = if matches!(receiver, Value::Null) {
                    "null"
                } else {
                    "undefined"
                };
                let key_str = match key {
                    PropertyKey::String(s) => format!(" (reading '{s}')"),
                    PropertyKey::Index(i) => format!(" (reading '{i}')"),
                    PropertyKey::Symbol(_) => String::new(),
                };
                Err(VmError::TypeError(format!(
                    "cannot read properties of {what}{key_str}"
                )))
            }
        }
    }

    fn get_property_from_chain(
        &mut self,
        object: GcRef<JsObject>,
        receiver: &Value,
        key: &PropertyKey,
    ) -> Result<Value, VmError> {
        // Proxy objects route through the handler's `get` trap.
        let proxy = self.heap.objects().get(object).and_then(|o| match &o.kind {
            ObjectKind::Proxy { target, handler } => Some((*target, *handler)),
            _ => None,
        });
        if let Some((target, handler)) = proxy {
            return self.proxy_get(target, handler, key, receiver);
        }

        // Host objects route through the DOM dispatch table first.
        // Copy only HostObjectSlot (Copy type) to avoid expensive ObjectKind::clone()
        // which would clone the Vec contents of Map/Set/Promise objects.
        let host_slot = self.heap.objects().get(object)
            .and_then(|o| if let ObjectKind::Host(slot) = o.kind { Some(slot) } else { None });
        if let Some(slot) = host_slot {
            let value = self.get_host_property(slot, key)?;
            // Fall back to an own expando property for Node names the DOM doesn't
            // expose (mirrors the set path above).
            if matches!(value, Value::Undefined)
                && matches!(
                    slot.class,
                    HostObjectClass::Node | HostObjectClass::EventTarget | HostObjectClass::Document
                )
            {
                if let Some(JsPropertyDescriptor::Data { value, .. }) =
                    self.get_own_property_descriptor(object, key)
                {
                    return Ok(value);
                }
                // Upgraded custom elements get their class prototype linked, so
                // walk it for methods like `connectedCallback` / user methods.
                if let Some(proto) = self.heap.objects().get(object).and_then(|o| o.prototype) {
                    if let Some((_, descriptor)) = self.lookup_property_descriptor(proto, key) {
                        return match descriptor {
                            JsPropertyDescriptor::Data { value, .. } => Ok(value),
                            JsPropertyDescriptor::Accessor { get, .. } => match get {
                                Some(getter) => self.call_value_sync(
                                    Value::Object(getter),
                                    receiver.clone(),
                                    Vec::new(),
                                ),
                                None => Ok(Value::Undefined),
                            },
                        };
                    }
                }
            }
            return Ok(value);
        }

        // ArrayBuffer / typed-array computed properties and indexed element
        // reads (method names fall through to the prototype chain below).
        if let Some(value) = self.typed_array_get_property(object, key) {
            return Ok(value);
        }

        if let Some((_, descriptor)) = self.lookup_property_descriptor(object, key) {
            return match descriptor {
                JsPropertyDescriptor::Data { value, .. } => Ok(value),
                JsPropertyDescriptor::Accessor { get, .. } => match get {
                    Some(getter) => {
                        self.call_value_sync(Value::Object(getter), receiver.clone(), Vec::new())
                    }
                    None => Ok(Value::Undefined),
                },
            };
        }
        Ok(Value::Undefined)
    }

    fn get_property_from_string(
        &mut self,
        string: GcRef<JsString>,
        receiver: &Value,
        key: &PropertyKey,
    ) -> Result<Value, VmError> {
        let text = self.string_text(string);
        match key {
            PropertyKey::Index(index) => {
                let value = text
                    .chars()
                    .nth(*index as usize)
                    .map(|character| self.make_string_value(&character.to_string()))
                    .unwrap_or(Value::Undefined);
                Ok(value)
            }
            PropertyKey::String(name) if name == "length" => {
                Ok(Value::Number(text.chars().count() as f64))
            }
            _ => self.get_property_from_chain(self.string_prototype_ref(), receiver, key),
        }
    }

    fn object_introspection_keys_like(
        &mut self,
        value: &Value,
        kind: ObjectIntrospectionKind,
        context: &str,
    ) -> Result<Value, VmError> {
        if let Some(object) = self.try_require_object_ref(value, context)? {
            match kind {
                ObjectIntrospectionKind::Keys => {
                    let values = self
                        .object_own_enumerable_keys(object)
                        .into_iter()
                        .map(|key| self.make_string_value(&self.property_key_to_string(&key)))
                        .collect();
                    self.make_array_from_values(values)
                }
                ObjectIntrospectionKind::Values => {
                    let mut values = Vec::new();
                    for key in self.object_own_enumerable_keys(object) {
                        values.push(self.get_property_value(&Value::Object(object), &key)?);
                    }
                    self.make_array_from_values(values)
                }
                ObjectIntrospectionKind::Entries => {
                    let mut entries = Vec::new();
                    for key in self.object_own_enumerable_keys(object) {
                        let pair = vec![
                            self.make_string_value(&self.property_key_to_string(&key)),
                            self.get_property_value(&Value::Object(object), &key)?,
                        ];
                        entries.push(self.make_array_from_values(pair)?);
                    }
                    self.make_array_from_values(entries)
                }
            }
        } else {
            match value {
                Value::String(string) => {
                    let text = self.string_text(*string);
                    match kind {
                        ObjectIntrospectionKind::Keys => {
                            let values = text
                                .chars()
                                .enumerate()
                                .map(|(index, _)| self.make_string_value(&index.to_string()))
                                .collect();
                            self.make_array_from_values(values)
                        }
                        ObjectIntrospectionKind::Values => {
                            let values = text
                                .chars()
                                .map(|character| self.make_string_value(&character.to_string()))
                                .collect();
                            self.make_array_from_values(values)
                        }
                        ObjectIntrospectionKind::Entries => {
                            let mut entries = Vec::new();
                            for (index, character) in text.chars().enumerate() {
                                let key = self.make_string_value(&index.to_string());
                                let value = self.make_string_value(&character.to_string());
                                entries.push(self.make_array_from_values(vec![
                                    key,
                                    value,
                                ])?);
                            }
                            self.make_array_from_values(entries)
                        }
                    }
                }
                Value::Number(_) | Value::Bool(_) | Value::Symbol(_) => {
                    self.make_array_from_values(Vec::new())
                }
                Value::Null | Value::Undefined => {
                    Err(VmError::TypeError(format!("{context} requires an object")))
                }
                Value::Object(_) => unreachable!(),
            }
        }
    }

    fn object_introspection_get_own_property_names(
        &mut self,
        value: &Value,
        context: &str,
    ) -> Result<Value, VmError> {
        if let Some(object) = self.try_require_object_ref(value, context)? {
            let mut names = Vec::new();
            if let Some(object_data) = self.heap.objects().get(object) {
                for key in object_data.properties.keys() {
                    if let PropertyKey::Symbol(_) = key {
                        continue;
                    }
                    names.push(self.property_key_to_string(key));
                }
            }
            let values = names.into_iter().map(|name| self.make_string_value(&name)).collect();
            self.make_array_from_values(values)
        } else {
            match value {
                Value::String(string) => {
                    let text = self.string_text(*string);
                    let mut names: Vec<Value> = text
                        .chars()
                        .enumerate()
                        .map(|(index, _)| self.make_string_value(&index.to_string()))
                        .collect();
                    names.push(self.make_string_value("length"));
                    self.make_array_from_values(names)
                }
                Value::Number(_) | Value::Bool(_) | Value::Symbol(_) => {
                    self.make_array_from_values(Vec::new())
                }
                Value::Null | Value::Undefined => Err(VmError::TypeError(format!(
                    "{context} requires an object"
                ))),
                Value::Object(_) => unreachable!(),
            }
        }
    }

    fn object_introspection_get_own_property_symbols(&mut self, value: &Value) -> Result<Value, VmError> {
        if let Value::Object(object) = value {
            let mut symbols = Vec::new();
            if let Some(object_data) = self.heap.objects().get(*object) {
                for key in object_data.properties.keys() {
                    if let PropertyKey::Symbol(symbol) = key {
                        symbols.push(Value::Symbol(*symbol));
                    }
                }
            }
            self.make_array_from_values(symbols)
        } else {
            self.make_array_from_values(Vec::new())
        }
    }

    fn object_introspection_get_own_property_descriptor(
        &mut self,
        value: &Value,
        key: &PropertyKey,
        context: &str,
    ) -> Result<Value, VmError> {
        if let Some(object) = self.try_require_object_ref(value, context)? {
            match self.get_own_property_descriptor(object, key) {
                Some(descriptor) => self.property_descriptor_to_value(descriptor),
                None => Ok(Value::Undefined),
            }
        } else {
            match (value, key) {
                (Value::String(string), PropertyKey::Index(index)) => {
                    let text = self.string_text(*string);
                    let chars: Vec<char> = text.chars().collect();
                    if let Some(character) = chars.get(*index as usize) {
                        let value = self.make_string_value(&character.to_string());
                        self.property_descriptor_to_value(JsPropertyDescriptor::Data {
                            value,
                            writable: false,
                            enumerable: true,
                            configurable: false,
                        })
                    } else {
                        Ok(Value::Undefined)
                    }
                }
                (Value::String(string), PropertyKey::String(name)) if name == "length" => {
                    let text = self.string_text(*string);
                    self.property_descriptor_to_value(JsPropertyDescriptor::Data {
                        value: Value::Number(text.chars().count() as f64),
                        writable: false,
                        enumerable: false,
                        configurable: false,
                    })
                }
                (Value::Number(_) | Value::Bool(_) | Value::Symbol(_), _) => Ok(Value::Undefined),
                (Value::Null | Value::Undefined, _) => Err(VmError::TypeError(format!(
                    "{context} requires an object"
                ))),
                (Value::String(_), PropertyKey::String(_)) => Ok(Value::Undefined),
                (Value::String(_), PropertyKey::Symbol(_)) => Ok(Value::Undefined),
                (Value::Object(_), _) => unreachable!(),
            }
        }
    }

    fn object_introspection_get_prototype_of(
        &self,
        value: &Value,
        context: &str,
    ) -> Result<Value, VmError> {
        if let Some(object) = self.try_require_object_ref(value, context)? {
            let prototype = self
                .heap
                .objects()
                .get(object)
                .and_then(|object_data| object_data.prototype)
                .map(Value::Object)
                .unwrap_or(Value::Null);
            Ok(prototype)
        } else {
            match value {
                Value::String(_)
                | Value::Number(_)
                | Value::Bool(_)
                | Value::Symbol(_) => Ok(Value::Object(self.object_introspection_primitive_prototype_ref(value))),
                Value::Null | Value::Undefined => Err(VmError::TypeError(format!(
                    "{context} requires an object"
                ))),
                Value::Object(_) => unreachable!(),
            }
        }
    }

    fn set_property_value(
        &mut self,
        receiver: &Value,
        key: PropertyKey,
        value: Value,
    ) -> Result<(), VmError> {
        let object = self.require_object_ref(receiver, "property assignment")?;
        self.set_property_on_object(object, receiver.clone(), key, value)
    }

    fn set_property_on_object(
        &mut self,
        object: GcRef<JsObject>,
        receiver: Value,
        key: PropertyKey,
        value: Value,
    ) -> Result<(), VmError> {
        // Proxy objects route writes through the handler's `set` trap.
        let proxy = self.heap.objects().get(object).and_then(|o| match &o.kind {
            ObjectKind::Proxy { target, handler } => Some((*target, *handler)),
            _ => None,
        });
        if let Some((target, handler)) = proxy {
            return self.proxy_set(target, handler, key, value, receiver);
        }

        // Host objects route writes through the DOM dispatch table.
        // Copy only HostObjectSlot (Copy type) to avoid expensive ObjectKind::clone().
        let host_slot = self.heap.objects().get(object)
            .and_then(|o| if let ObjectKind::Host(slot) = o.kind { Some(slot) } else { None });
        if let Some(slot) = host_slot {
            // Node expando: a property name the DOM doesn't manage is stored as an
            // ordinary own property on the (interned) node wrapper, so frameworks
            // can stash data on nodes (React fiber keys, etc.).
            if matches!(slot.class, HostObjectClass::Node | HostObjectClass::EventTarget) {
                if let PropertyKey::String(name) = &key {
                    if !is_dom_managed_node_property(name) {
                        self.define_data_property(object, key, value, true, true, true);
                        return Ok(());
                    }
                }
            }
            if matches!(slot.class, HostObjectClass::Document) {
                if let PropertyKey::String(name) = &key {
                    if !is_dom_managed_document_property(name) {
                        self.define_data_property(object, key, value, true, true, true);
                        return Ok(());
                    }
                }
            }
            let result = self.set_host_property(slot, key, value);
            // boa parity: DOM property writes flush mutation observers
            // synchronously (see invoke_builtin).
            self.deliver_mutation_records();
            self.deliver_slotchange();
            return result;
        }

        // Typed-array indexed writes go straight to the backing buffer (an
        // out-of-range index is silently ignored, per spec).
        if let PropertyKey::Index(index) = &key
            && self.typed_array_info(&Value::Object(object)).is_some()
        {
            let index = *index as usize;
            let number = self.to_number(&value);
            self.typed_array_write_element(&Value::Object(object), index, number)?;
            return Ok(());
        }

        if let Some(descriptor) = self.get_own_property_descriptor(object, &key) {
            return match descriptor {
                JsPropertyDescriptor::Data {
                    writable: false, ..
                } => {
                    if self.in_strict_mode() {
                        Err(VmError::TypeError(format!(
                            "property {} is not writable",
                            self.property_key_to_string(&key)
                        )))
                    } else {
                        // Sloppy mode: assignment to a read-only property is ignored.
                        Ok(())
                    }
                }
                JsPropertyDescriptor::Data {
                    enumerable,
                    configurable,
                    ..
                } => {
                    self.define_data_property(
                        object,
                        key.clone(),
                        value.clone(),
                        true,
                        enumerable,
                        configurable,
                    );
                    self.update_array_length_for_key(object, &key)?;
                    Ok(())
                }
                JsPropertyDescriptor::Accessor { set, .. } => match set {
                    Some(setter) => {
                        let _ =
                            self.call_value_sync(Value::Object(setter), receiver, vec![value])?;
                        Ok(())
                    }
                    None => Err(VmError::TypeError("property has no setter".to_string())),
                },
            };
        }

        if let Some((
            _,
            JsPropertyDescriptor::Accessor {
                set: Some(setter), ..
            },
        )) = self.lookup_property_descriptor(object, &key)
        {
            let _ = self.call_value_sync(Value::Object(setter), receiver, vec![value])?;
            return Ok(());
        }

        let extensible = self
            .heap
            .objects()
            .get(object)
            .map(|object_data| object_data.extensible)
            .unwrap_or(false);
        if !extensible {
            if self.in_strict_mode() {
                return Err(VmError::TypeError("object is not extensible".to_string()));
            }
            // Sloppy mode: adding a property to a non-extensible object is ignored.
            return Ok(());
        }

        self.define_data_property(object, key.clone(), value, true, true, true);
        self.update_array_length_for_key(object, &key)?;
        Ok(())
    }

    fn update_array_length_for_key(
        &mut self,
        object: GcRef<JsObject>,
        key: &PropertyKey,
    ) -> Result<(), VmError> {
        // Cheap discriminant check — no Vec clone needed
        let is_array = matches!(self.heap.objects().get(object).map(|o| &o.kind), Some(ObjectKind::Array));
        if !is_array {
            return Ok(());
        }

        match key {
            PropertyKey::Index(index) => {
                let length = self.array_length(object);
                if *index >= length {
                    self.set_array_length(object, index.saturating_add(1));
                }
            }
            PropertyKey::String(name) if name == "length" => {
                let new_length = self
                    .get_own_property_descriptor(object, key)
                    .and_then(|descriptor| match descriptor {
                        JsPropertyDescriptor::Data {
                            value: Value::Number(number),
                            ..
                        } if number.is_finite() && number >= 0.0 => Some(number as u32),
                        _ => None,
                    })
                    .unwrap_or(0);
                self.truncate_array_to_length(object, new_length);
            }
            _ => {}
        }
        Ok(())
    }

    /// Define (or merge) an accessor property. A getter and setter declared for
    /// the same key combine into a single accessor descriptor.
    fn define_accessor(
        &mut self,
        object: GcRef<JsObject>,
        key: PropertyKey,
        function: GcRef<JsObject>,
        is_getter: bool,
    ) {
        let (mut get, mut set) = match self.get_own_property_descriptor(object, &key) {
            Some(JsPropertyDescriptor::Accessor { get, set, .. }) => (get, set),
            _ => (None, None),
        };
        if is_getter {
            get = Some(function);
        } else {
            set = Some(function);
        }
        if let Some(object_data) = self.heap.objects_mut().get_mut(object) {
            object_data.properties.insert(
                key,
                JsPropertyDescriptor::Accessor {
                    get,
                    set,
                    enumerable: true,
                    configurable: true,
                },
            );
        }
    }

    /// Delete an own property, honoring `configurable`. Returns false only when
    /// the property exists and is non-configurable (matching `delete`).
    fn delete_property(&mut self, object: GcRef<JsObject>, key: &PropertyKey) -> bool {
        let configurable = match self.get_own_property_descriptor(object, key) {
            Some(JsPropertyDescriptor::Data { configurable, .. }) => configurable,
            Some(JsPropertyDescriptor::Accessor { configurable, .. }) => configurable,
            None => return true,
        };
        if !configurable {
            return false;
        }
        if let Some(object_data) = self.heap.objects_mut().get_mut(object) {
            object_data.properties.shift_remove(key);
        }
        true
    }

    fn array_length(&self, object: GcRef<JsObject>) -> u32 {
        if let Some(JsPropertyDescriptor::Data {
            value: Value::Number(number),
            ..
        }) = self.get_own_property_descriptor(object, &PropertyKey::from("length"))
        {
            if number.is_finite() && number >= 0.0 {
                return number as u32;
            }
        }

        self.heap
            .objects()
            .get(object)
            .map(|object_data| {
                object_data
                    .properties
                    .keys()
                    .filter_map(|key| match key {
                        PropertyKey::Index(index) => Some(index.saturating_add(1)),
                        _ => None,
                    })
                    .max()
                    .unwrap_or(0)
            })
            .unwrap_or(0)
    }

    fn set_array_length(&mut self, object: GcRef<JsObject>, length: u32) {
        self.define_data_property(
            object,
            PropertyKey::from("length"),
            Value::Number(length as f64),
            true,
            false,
            false,
        );
    }

    fn truncate_array_to_length(&mut self, object: GcRef<JsObject>, length: u32) {
        if let Some(object_data) = self.heap.objects_mut().get_mut(object) {
            let keys = object_data.properties.keys().cloned().collect::<Vec<_>>();
            for key in keys {
                if matches!(key, PropertyKey::Index(index) if index >= length) {
                    let _ = object_data.properties.shift_remove(&key);
                }
            }
        }
        self.set_array_length(object, length);
    }

    fn make_array_from_values(&mut self, values: Vec<Value>) -> Result<Value, VmError> {
        let array = self.heap.allocate_object(JsObject {
            kind: ObjectKind::Array,
            prototype: Some(self.array_prototype_ref()),
            ..JsObject::default()
        });
        for (index, value) in values.into_iter().enumerate() {
            self.define_data_property(
                array,
                PropertyKey::Index(index as u32),
                value,
                true,
                true,
                true,
            );
        }
        self.set_array_length(array, self.array_length(array));
        Ok(Value::Object(array))
    }

    fn object_own_enumerable_keys(&self, object: GcRef<JsObject>) -> Vec<PropertyKey> {
        self.heap
            .objects()
            .get(object)
            .map(|object_data| {
                object_data
                    .properties
                    .iter()
                    .filter_map(|(key, descriptor)| match descriptor {
                        JsPropertyDescriptor::Data { enumerable, .. }
                        | JsPropertyDescriptor::Accessor { enumerable, .. }
                            if *enumerable =>
                        {
                            Some(key.clone())
                        }
                        _ => None,
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    fn builtin_string_this(&self, this_value: &Value) -> Result<String, VmError> {
        match this_value {
            Value::String(string) => Ok(self.string_text(*string)),
            _ => Err(VmError::TypeError(
                "string method called on non-string".to_string(),
            )),
        }
    }

    fn builtin_object_this(
        &self,
        this_value: &Value,
        context: &str,
    ) -> Result<GcRef<JsObject>, VmError> {
        self.require_object_ref(this_value, context)
    }

    fn array_like_length(&mut self, value: &Value) -> Result<u32, VmError> {
        match value {
            Value::Object(object) => {
                // Host array-likes (NamedNodeMap, …) compute `length` via the
                // host dispatch — it is not an own data property.
                if self
                    .heap
                    .objects()
                    .get(*object)
                    .is_some_and(|o| matches!(o.kind, ObjectKind::Host(_)))
                {
                    let v = self.get_property_value(value, &PropertyKey::from("length"))?;
                    let n = self.to_number(&v);
                    return Ok(if n.is_finite() && n > 0.0 { n as u32 } else { 0 });
                }
                Ok(self.array_length(*object))
            }
            Value::String(string) => Ok(self.string_text(*string).chars().count() as u32),
            _ => Err(VmError::TypeError("value is not array-like".to_string())),
        }
    }

    fn array_like_to_vec(&mut self, value: &Value) -> Result<Vec<Value>, VmError> {
        let length = self.array_like_length(value)?;
        let mut values = Vec::with_capacity(length as usize);
        for index in 0..length {
            values.push(self.get_property_value(value, &PropertyKey::Index(index))?);
        }
        Ok(values)
    }

    fn for_in_keys(&self, object: GcRef<JsObject>) -> Vec<String> {
        let mut keys = Vec::new();
        let mut current = Some(object);
        while let Some(object_ref) = current {
            let Some(object_data) = self.heap.objects().get(object_ref) else {
                break;
            };
            for (key, descriptor) in &object_data.properties {
                let enumerable = matches!(
                    descriptor,
                    JsPropertyDescriptor::Data {
                        enumerable: true,
                        ..
                    } | JsPropertyDescriptor::Accessor {
                        enumerable: true,
                        ..
                    }
                );
                if !enumerable {
                    continue;
                }
                if let PropertyKey::Symbol(_) = key {
                    continue;
                }
                let text = self.property_key_to_string(key);
                if !keys.contains(&text) {
                    keys.push(text);
                }
            }
            current = object_data.prototype;
        }
        keys
    }

    // ---- ArrayBuffer + typed arrays -------------------------------------

    fn make_array_buffer(&mut self, byte_length: usize) -> Value {
        let object = self.heap.allocate_object(JsObject {
            kind: ObjectKind::ArrayBuffer(vec![0u8; byte_length]),
            prototype: Some(self.array_buffer_prototype_ref()),
            ..JsObject::default()
        });
        Value::Object(object)
    }

    fn make_typed_array(
        &mut self,
        kind: TypedArrayKind,
        buffer: Value,
        byte_offset: usize,
        length: usize,
    ) -> Result<Value, VmError> {
        let buffer = self.require_object_ref(&buffer, "typed array buffer")?;
        let object = self.heap.allocate_object(JsObject {
            kind: ObjectKind::TypedArray {
                buffer,
                kind,
                byte_offset,
                length,
            },
            prototype: Some(self.typed_array_prototype_ref()),
            ..JsObject::default()
        });
        Ok(Value::Object(object))
    }

    fn make_uint8_array(&mut self, bytes: Vec<u8>) -> Result<Value, VmError> {
        let length = bytes.len();
        let buffer = self.heap.allocate_object(JsObject {
            kind: ObjectKind::ArrayBuffer(bytes),
            prototype: Some(self.array_buffer_prototype_ref()),
            ..JsObject::default()
        });
        self.make_typed_array(TypedArrayKind::Uint8, Value::Object(buffer), 0, length)
    }

    fn is_array_buffer(&self, object: GcRef<JsObject>) -> bool {
        matches!(
            self.heap.objects().get(object).map(|o| &o.kind),
            Some(ObjectKind::ArrayBuffer(_))
        )
    }

    fn array_buffer_len(&self, buffer: GcRef<JsObject>) -> usize {
        match self.heap.objects().get(buffer).map(|o| &o.kind) {
            Some(ObjectKind::ArrayBuffer(bytes)) => bytes.len(),
            _ => 0,
        }
    }

    /// Read a typed-array view's metadata (buffer, kind, byte offset, element
    /// length). None if `value` is not a typed array.
    fn typed_array_info(
        &self,
        value: &Value,
    ) -> Option<(GcRef<JsObject>, TypedArrayKind, usize, usize)> {
        let Value::Object(object) = value else {
            return None;
        };
        match self.heap.objects().get(*object)?.kind {
            ObjectKind::TypedArray {
                buffer,
                kind,
                byte_offset,
                length,
            } => Some((buffer, kind, byte_offset, length)),
            _ => None,
        }
    }

    /// Read element `index` of a typed array as an f64; None if out of range or
    /// not a typed array.
    fn typed_array_read_element(&self, value: &Value, index: usize) -> Option<f64> {
        let (buffer, kind, byte_offset, length) = self.typed_array_info(value)?;
        if index >= length {
            return None;
        }
        let byte_index = byte_offset + index * kind.bytes_per_element();
        match &self.heap.objects().get(buffer)?.kind {
            ObjectKind::ArrayBuffer(bytes) => Some(kind.read_element(bytes, byte_index)),
            _ => None,
        }
    }

    /// Coerce and write `number` into element `index` (no-op if out of range).
    fn typed_array_write_element(
        &mut self,
        value: &Value,
        index: usize,
        number: f64,
    ) -> Result<(), VmError> {
        let Some((buffer, kind, byte_offset, length)) = self.typed_array_info(value) else {
            return Ok(());
        };
        if index >= length {
            return Ok(());
        }
        let byte_index = byte_offset + index * kind.bytes_per_element();
        if let Some(object) = self.heap.objects_mut().get_mut(buffer)
            && let ObjectKind::ArrayBuffer(bytes) = &mut object.kind
        {
            kind.coerce_and_write(bytes, byte_index, number);
        }
        Ok(())
    }

    /// All elements of a typed array as Number values.
    fn typed_array_to_values(&self, value: &Value) -> Vec<Value> {
        let length = self.typed_array_info(value).map(|info| info.3).unwrap_or(0);
        (0..length)
            .map(|index| Value::Number(self.typed_array_read_element(value, index).unwrap_or(0.0)))
            .collect()
    }

    /// Resolve a `(begin, end)` index pair from up to two relative-index args
    /// against a collection of `length` items (negative counts from the end).
    fn typed_array_range(&self, args: &[Value], length: usize) -> (usize, usize) {
        fn resolve(value: f64, length: usize, default: usize) -> usize {
            if value.is_nan() {
                return default;
            }
            let truncated = value.trunc();
            if truncated < 0.0 {
                (length as f64 + truncated).max(0.0) as usize
            } else {
                (truncated as usize).min(length)
            }
        }
        let begin = match args.first() {
            Some(value) if !matches!(value, Value::Undefined) => {
                resolve(self.to_number(value), length, 0)
            }
            _ => 0,
        };
        let end = match args.get(1) {
            Some(value) if !matches!(value, Value::Undefined) => {
                resolve(self.to_number(value), length, length)
            }
            _ => length,
        };
        (begin, end.max(begin))
    }

    fn construct_array_buffer(&mut self, args: &[Value]) -> Result<Value, VmError> {
        let length = args.first().map(|value| self.to_number(value)).unwrap_or(0.0);
        let length = if length.is_finite() && length >= 0.0 {
            length as usize
        } else {
            0
        };
        Ok(self.make_array_buffer(length))
    }

    fn array_buffer_slice(&mut self, this: &Value, args: &[Value]) -> Result<Value, VmError> {
        let Value::Object(object) = this else {
            return Ok(Value::Undefined);
        };
        let length = self.array_buffer_len(*object);
        let (begin, end) = self.typed_array_range(args, length);
        let source: Vec<u8> = match self.heap.objects().get(*object).map(|o| &o.kind) {
            Some(ObjectKind::ArrayBuffer(bytes)) => {
                bytes.get(begin..end).map(<[u8]>::to_vec).unwrap_or_default()
            }
            _ => Vec::new(),
        };
        let new_buffer = self.make_array_buffer(source.len());
        if let Value::Object(new_ref) = &new_buffer
            && let Some(object) = self.heap.objects_mut().get_mut(*new_ref)
            && let ObjectKind::ArrayBuffer(bytes) = &mut object.kind
        {
            bytes.copy_from_slice(&source);
        }
        Ok(new_buffer)
    }

    fn construct_typed_array(
        &mut self,
        kind: TypedArrayKind,
        args: &[Value],
    ) -> Result<Value, VmError> {
        let bytes_per_element = kind.bytes_per_element();
        match args.first().cloned() {
            // new T(buffer, byteOffset?, length?) — a view over an ArrayBuffer.
            Some(Value::Object(object)) if self.is_array_buffer(object) => {
                let buffer_len = self.array_buffer_len(object);
                let byte_offset = args.get(1).map(|value| self.to_number(value)).unwrap_or(0.0);
                let byte_offset = if byte_offset.is_finite() && byte_offset >= 0.0 {
                    byte_offset as usize
                } else {
                    0
                };
                let length = match args.get(2) {
                    Some(value) if !matches!(value, Value::Undefined) => {
                        let n = self.to_number(value);
                        if n.is_finite() && n >= 0.0 { n as usize } else { 0 }
                    }
                    _ => buffer_len.saturating_sub(byte_offset) / bytes_per_element,
                };
                self.make_typed_array(kind, Value::Object(object), byte_offset, length)
            }
            // new T(typedArray | array | iterable) — copy element values.
            Some(Value::Object(object)) => {
                let source = Value::Object(object);
                let values = if self.typed_array_info(&source).is_some() {
                    self.typed_array_to_values(&source)
                } else {
                    self.for_of_values(&source)?
                };
                let buffer = self.make_array_buffer(values.len() * bytes_per_element);
                let view = self.make_typed_array(kind, buffer, 0, values.len())?;
                for (index, value) in values.into_iter().enumerate() {
                    let number = self.to_number(&value);
                    self.typed_array_write_element(&view, index, number)?;
                }
                Ok(view)
            }
            // new T(length)
            Some(value) => {
                let n = self.to_number(&value);
                let length = if n.is_finite() && n >= 0.0 { n as usize } else { 0 };
                let buffer = self.make_array_buffer(length * bytes_per_element);
                self.make_typed_array(kind, buffer, 0, length)
            }
            // new T()
            None => {
                let buffer = self.make_array_buffer(0);
                self.make_typed_array(kind, buffer, 0, 0)
            }
        }
    }

    fn escape_legacy(&self, input: String) -> String {
        let mut out = String::new();
        for ch in input.chars() {
            if ch.is_ascii_alphanumeric() || matches!(ch, '@' | '*' | '_' | '+' | '-' | '.' | '/') {
                out.push(ch);
            } else {
                let code = ch as u32;
                if code < 256 {
                    out.push_str(&format!("%{code:02X}"));
                } else {
                    out.push_str(&format!("%u{code:04X}"));
                }
            }
        }
        out
    }

    fn unescape_legacy(&self, input: String) -> String {
        let chars: Vec<char> = input.chars().collect();
        let mut out = String::new();
        let mut i = 0;
        while i < chars.len() {
            if chars[i] == '%' && i + 1 < chars.len() {
                if chars[i + 1] == 'u' && i + 5 < chars.len() {
                    let hex: String = chars[i + 2..i + 6].iter().collect();
                    if let Ok(code) = u32::from_str_radix(&hex, 16)
                        && let Some(ch) = char::from_u32(code)
                    {
                        out.push(ch);
                        i += 6;
                        continue;
                    }
                } else if i + 2 < chars.len() {
                    let hex: String = chars[i + 1..i + 3].iter().collect();
                    if let Ok(code) = u8::from_str_radix(&hex, 16) {
                        out.push(code as char);
                        i += 3;
                        continue;
                    }
                }
            }
            out.push(chars[i]);
            i += 1;
        }
        out
    }

    fn typed_array_from(
        &mut self,
        kind: TypedArrayKind,
        args: &[Value],
    ) -> Result<Value, VmError> {
        let source = args.first().cloned().unwrap_or(Value::Undefined);
        let map_fn = args.get(1).cloned();
        let this_arg = args.get(2).cloned().unwrap_or(Value::Undefined);
        let mut values = match &source {
            Value::String(string) => self
                .string_text(*string)
                .chars()
                .map(|character| self.make_string_value(&character.to_string()))
                .collect(),
            Value::Object(_) if self.typed_array_info(&source).is_some() => {
                self.typed_array_to_values(&source)
            }
            Value::Object(_) => self.for_of_values(&source)?,
            Value::Null | Value::Undefined => {
                return Err(VmError::TypeError(
                    "TypedArray.from requires an array-like or iterable object".to_string(),
                ));
            }
            _ => Vec::new(),
        };
        if let Some(callback) = map_fn.filter(|callback| self.is_callable_value(callback)) {
            let mut mapped = Vec::with_capacity(values.len());
            for (index, value) in values.into_iter().enumerate() {
                mapped.push(self.call_value_sync(
                    callback.clone(),
                    this_arg.clone(),
                    vec![value, Value::Number(index as f64)],
                )?);
            }
            values = mapped;
        }
        let buffer = self.make_array_buffer(values.len() * kind.bytes_per_element());
        let view = self.make_typed_array(kind, buffer, 0, values.len())?;
        for (index, value) in values.into_iter().enumerate() {
            let number = self.to_number(&value);
            self.typed_array_write_element(&view, index, number)?;
        }
        Ok(view)
    }

    fn typed_array_proto_set(&mut self, this: &Value, args: &[Value]) -> Result<Value, VmError> {
        let source = args.first().cloned().unwrap_or(Value::Undefined);
        let offset = args.get(1).map(|value| self.to_number(value)).unwrap_or(0.0);
        let offset = if offset.is_finite() && offset >= 0.0 {
            offset as usize
        } else {
            0
        };
        let values = match &source {
            Value::Object(_) if self.typed_array_info(&source).is_some() => {
                self.typed_array_to_values(&source)
            }
            Value::Object(_) => self.array_like_to_vec(&source)?,
            _ => Vec::new(),
        };
        for (index, value) in values.into_iter().enumerate() {
            let number = self.to_number(&value);
            self.typed_array_write_element(this, offset + index, number)?;
        }
        Ok(Value::Undefined)
    }

    fn typed_array_subarray(&mut self, this: &Value, args: &[Value]) -> Result<Value, VmError> {
        let Some((buffer, kind, byte_offset, length)) = self.typed_array_info(this) else {
            return Ok(Value::Undefined);
        };
        let (begin, end) = self.typed_array_range(args, length);
        let new_offset = byte_offset + begin * kind.bytes_per_element();
        self.make_typed_array(kind, Value::Object(buffer), new_offset, end - begin)
    }

    fn typed_array_slice(&mut self, this: &Value, args: &[Value]) -> Result<Value, VmError> {
        let Some((_, kind, _, length)) = self.typed_array_info(this) else {
            return Ok(Value::Undefined);
        };
        let (begin, end) = self.typed_array_range(args, length);
        let new_length = end - begin;
        let buffer = self.make_array_buffer(new_length * kind.bytes_per_element());
        let view = self.make_typed_array(kind, buffer, 0, new_length)?;
        for index in 0..new_length {
            if let Some(number) = self.typed_array_read_element(this, begin + index) {
                self.typed_array_write_element(&view, index, number)?;
            }
        }
        Ok(view)
    }

    fn typed_array_fill(&mut self, this: &Value, args: &[Value]) -> Result<Value, VmError> {
        let Some((_, _, _, length)) = self.typed_array_info(this) else {
            return Ok(this.clone());
        };
        let number = args.first().map(|value| self.to_number(value)).unwrap_or(f64::NAN);
        let rest = if args.len() > 1 { &args[1..] } else { &[] };
        let (start, end) = self.typed_array_range(rest, length);
        for index in start..end {
            self.typed_array_write_element(this, index, number)?;
        }
        Ok(this.clone())
    }

    fn typed_array_join(&mut self, this: &Value, args: &[Value]) -> Result<Value, VmError> {
        let separator = match args.first() {
            Some(value) if !matches!(value, Value::Undefined) => self.to_string(value),
            _ => ",".to_string(),
        };
        let values = self.typed_array_to_values(this);
        let parts: Vec<String> = values.iter().map(|value| self.to_string(value)).collect();
        Ok(self.make_string_value(&parts.join(&separator)))
    }

    fn typed_array_index_of(
        &mut self,
        this: &Value,
        args: &[Value],
        includes: bool,
    ) -> Result<Value, VmError> {
        let not_found = if includes {
            Value::Bool(false)
        } else {
            Value::Number(-1.0)
        };
        let target = args.first().map(|value| self.to_number(value)).unwrap_or(f64::NAN);
        let Some((_, _, _, length)) = self.typed_array_info(this) else {
            return Ok(not_found);
        };
        let from = args.get(1).map(|value| self.to_number(value)).unwrap_or(0.0);
        let start = if from.is_finite() && from < 0.0 {
            (length as f64 + from).max(0.0) as usize
        } else if from.is_finite() {
            (from as usize).min(length)
        } else {
            0
        };
        for index in start..length {
            let Some(element) = self.typed_array_read_element(this, index) else {
                continue;
            };
            let hit = element == target || (includes && element.is_nan() && target.is_nan());
            if hit {
                return Ok(if includes {
                    Value::Bool(true)
                } else {
                    Value::Number(index as f64)
                });
            }
        }
        Ok(not_found)
    }

    fn typed_array_for_each(&mut self, this: &Value, args: &[Value]) -> Result<Value, VmError> {
        let callback = args.first().cloned().unwrap_or(Value::Undefined);
        let this_arg = args.get(1).cloned().unwrap_or(Value::Undefined);
        let Some((_, _, _, length)) = self.typed_array_info(this) else {
            return Ok(Value::Undefined);
        };
        for index in 0..length {
            let element = self.typed_array_read_element(this, index).unwrap_or(0.0);
            self.call_value_sync(
                callback.clone(),
                this_arg.clone(),
                vec![Value::Number(element), Value::Number(index as f64), this.clone()],
            )?;
        }
        Ok(Value::Undefined)
    }

    fn typed_array_map(&mut self, this: &Value, args: &[Value]) -> Result<Value, VmError> {
        let callback = args.first().cloned().unwrap_or(Value::Undefined);
        let this_arg = args.get(1).cloned().unwrap_or(Value::Undefined);
        let Some((_, kind, _, length)) = self.typed_array_info(this) else {
            return Ok(Value::Undefined);
        };
        let buffer = self.make_array_buffer(length * kind.bytes_per_element());
        let view = self.make_typed_array(kind, buffer, 0, length)?;
        for index in 0..length {
            let element = self.typed_array_read_element(this, index).unwrap_or(0.0);
            let mapped = self.call_value_sync(
                callback.clone(),
                this_arg.clone(),
                vec![Value::Number(element), Value::Number(index as f64), this.clone()],
            )?;
            let number = self.to_number(&mapped);
            self.typed_array_write_element(&view, index, number)?;
        }
        Ok(view)
    }

    fn typed_array_reduce(&mut self, this: &Value, args: &[Value]) -> Result<Value, VmError> {
        let callback = args.first().cloned().unwrap_or(Value::Undefined);
        let Some((_, _, _, length)) = self.typed_array_info(this) else {
            return Ok(Value::Undefined);
        };
        let mut index = 0;
        let mut accumulator = if args.len() > 1 {
            args[1].clone()
        } else {
            if length == 0 {
                return Err(VmError::TypeError(
                    "Reduce of empty typed array with no initial value".to_string(),
                ));
            }
            index = 1;
            Value::Number(self.typed_array_read_element(this, 0).unwrap_or(0.0))
        };
        while index < length {
            let element = self.typed_array_read_element(this, index).unwrap_or(0.0);
            accumulator = self.call_value_sync(
                callback.clone(),
                Value::Undefined,
                vec![
                    accumulator,
                    Value::Number(element),
                    Value::Number(index as f64),
                    this.clone(),
                ],
            )?;
            index += 1;
        }
        Ok(accumulator)
    }

    fn typed_array_reverse(&mut self, this: &Value) -> Result<Value, VmError> {
        let Some((_, _, _, length)) = self.typed_array_info(this) else {
            return Ok(this.clone());
        };
        for index in 0..length / 2 {
            let mirror = length - 1 - index;
            let left = self.typed_array_read_element(this, index).unwrap_or(0.0);
            let right = self.typed_array_read_element(this, mirror).unwrap_or(0.0);
            self.typed_array_write_element(this, index, right)?;
            self.typed_array_write_element(this, mirror, left)?;
        }
        Ok(this.clone())
    }

    /// Special-cased property reads for ArrayBuffer / typed-array objects (the
    /// computed `length`/`byteLength`/`buffer` accessors and integer-indexed
    /// element reads). Returns None to fall through to the ordinary chain.
    fn typed_array_get_property(
        &mut self,
        object: GcRef<JsObject>,
        key: &PropertyKey,
    ) -> Option<Value> {
        let value = Value::Object(object);
        // ArrayBuffer.byteLength.
        if self.is_array_buffer(object) {
            if let PropertyKey::String(name) = key
                && name == "byteLength"
            {
                return Some(Value::Number(self.array_buffer_len(object) as f64));
            }
            return None;
        }
        let (buffer, kind, byte_offset, length) = self.typed_array_info(&value)?;
        match key {
            PropertyKey::Index(index) => {
                Some(match self.typed_array_read_element(&value, *index as usize) {
                    Some(number) => Value::Number(number),
                    None => Value::Undefined,
                })
            }
            PropertyKey::String(name) => match name.as_str() {
                "length" => Some(Value::Number(length as f64)),
                "byteLength" => Some(Value::Number((length * kind.bytes_per_element()) as f64)),
                "byteOffset" => Some(Value::Number(byte_offset as f64)),
                "BYTES_PER_ELEMENT" => Some(Value::Number(kind.bytes_per_element() as f64)),
                "buffer" => Some(Value::Object(buffer)),
                _ => None,
            },
            _ => None,
        }
    }

    // ---- fetch / Response / Headers -------------------------------------

    /// `fetch(input, init?)` — performs the request synchronously via the host
    /// and returns an already-resolved `Promise<Response>` (rejected on a
    /// network error). Synchronous for now; the body is materialised eagerly,
    /// so `fetch(u).then(r => r.text())` settles through the microtask queue.
    fn builtin_fetch(&mut self, args: &[Value]) -> Result<Value, VmError> {
        let input = args.first().cloned().unwrap_or(Value::Undefined);
        let url = match &input {
            Value::String(string) => self.string_text(*string),
            Value::Object(_) => {
                let u = self
                    .get_property_value(&input, &PropertyKey::from("url"))
                    .unwrap_or(Value::Undefined);
                self.to_string(&u)
            }
            other => self.to_string(other),
        };
        let init = args.get(1).cloned();
        // An already-aborted AbortSignal rejects without hitting the network.
        if let Some(init_val @ Value::Object(_)) = &init {
            let signal = self
                .get_property_value(init_val, &PropertyKey::from("signal"))
                .unwrap_or(Value::Undefined);
            if matches!(signal, Value::Object(_)) {
                let aborted = self
                    .get_property_value(&signal, &PropertyKey::from("aborted"))
                    .unwrap_or(Value::Undefined);
                if self.is_truthy(&aborted) {
                    let err = self.create_error_object(
                        "AbortError",
                        "The operation was aborted".to_string(),
                    );
                    let promise = self.promise_reject_value(err)?;
                    return Ok(Value::Object(promise));
                }
            }
        }
        let (method, headers, body) = self.read_fetch_init(&init);
        let request = FetchRequest {
            window: WindowId(0),
            url: url.clone(),
            method,
            headers,
            body,
            mode: FetchMode::Cors,
            keepalive: false,
        };
        match self.host.fetch_sync(request) {
            Ok(response) => {
                let response = self.build_response_object(&url, response);
                let promise = self.promise_resolve_value(response)?;
                Ok(Value::Object(promise))
            }
            Err(_) => {
                let err = self.create_error_object("TypeError", format!("Failed to fetch: {url}"));
                let promise = self.promise_reject_value(err)?;
                Ok(Value::Object(promise))
            }
        }
    }

    fn read_fetch_init(
        &mut self,
        init: &Option<Value>,
    ) -> (HttpMethod, Vec<(String, String)>, FetchBody) {
        let mut method = HttpMethod::Get;
        let mut headers = Vec::new();
        let mut body = FetchBody::Empty;
        if let Some(init @ Value::Object(init_ref)) = init {
            let method_val = self
                .get_property_value(init, &PropertyKey::from("method"))
                .unwrap_or(Value::Undefined);
            if !matches!(method_val, Value::Undefined) {
                method = match self.to_string(&method_val).to_uppercase().as_str() {
                    "POST" => HttpMethod::Post,
                    "PUT" => HttpMethod::Put,
                    "PATCH" => HttpMethod::Patch,
                    "DELETE" => HttpMethod::Delete,
                    "HEAD" => HttpMethod::Head,
                    "OPTIONS" => HttpMethod::Options,
                    _ => HttpMethod::Get,
                };
            }
            let body_val = self
                .get_property_value(init, &PropertyKey::from("body"))
                .unwrap_or(Value::Undefined);
            if !matches!(body_val, Value::Undefined | Value::Null) {
                body = FetchBody::Utf8(self.to_string(&body_val));
            }
            let headers_val = self
                .get_property_value(init, &PropertyKey::from("headers"))
                .unwrap_or(Value::Undefined);
            if let Value::Object(headers_ref) = headers_val {
                for key in self.object_own_enumerable_keys(headers_ref) {
                    if let PropertyKey::String(name) = &key {
                        let value = self
                            .get_property_value(&Value::Object(headers_ref), &key)
                            .unwrap_or(Value::Undefined);
                        headers.push((name.clone(), self.to_string(&value)));
                    }
                }
            }
            let _ = init_ref;
        }
        (method, headers, body)
    }

    // ---- XMLHttpRequest (synchronous under the hood via Host::fetch_sync) ----

    fn xhr_construct(&mut self) -> Result<Value, VmError> {
        let proto = self.object_prototype_ref();
        let object = self.allocate_ordinary_object(Some(proto));
        let empty = self.make_string_value("");
        let set = |vm: &mut Self, name: &str, value: Value, enumerable: bool| {
            vm.define_data_property(object, PropertyKey::from(name), value, true, enumerable, true);
        };
        set(self, "readyState", Value::Number(0.0), true);
        set(self, "status", Value::Number(0.0), true);
        set(self, "statusText", empty.clone(), true);
        set(self, "responseText", empty.clone(), true);
        set(self, "response", empty.clone(), true);
        set(self, "responseType", empty.clone(), true);
        set(self, "responseURL", empty.clone(), true);
        set(self, "timeout", Value::Number(0.0), true);
        set(self, "withCredentials", Value::Bool(false), true);
        // Request-header accumulator + parsed response headers (non-enumerable).
        let req_headers = self.allocate_ordinary_object(Some(proto));
        set(self, "__xhrReqHeaders", Value::Object(req_headers), false);
        let empty2 = self.make_string_value("");
        set(self, "__xhrRespHeaders", empty2, false);
        // Methods.
        for (name, builtin) in [
            ("open", BuiltinId::XhrOpen),
            ("send", BuiltinId::XhrSend),
            ("setRequestHeader", BuiltinId::XhrSetRequestHeader),
            ("abort", BuiltinId::XhrAbort),
            ("getAllResponseHeaders", BuiltinId::XhrGetAllResponseHeaders),
            ("getResponseHeader", BuiltinId::XhrGetResponseHeader),
        ] {
            let method = self.allocate_builtin_method(builtin);
            self.define_data_property(object, PropertyKey::from(name), method, true, false, true);
        }
        Ok(Value::Object(object))
    }

    fn image_construct(&mut self, args: &[Value]) -> Result<Value, VmError> {
        let res = self.host.mutate_dom(DomMutation::CreateElement {
            window: WindowId(0),
            local_name: "img".to_string(),
        });
        let node_id = match res {
            Ok(super::host::DomMutationResult::Node(id)) => id,
            _ => return Ok(Value::Undefined),
        };
        if let Some(width) = args.first() {
            let value = self.to_string(width);
            let _ = self.host.mutate_dom(DomMutation::SetAttribute {
                node: node_id,
                name: "width".to_string(),
                value,
            });
        }
        if let Some(height) = args.get(1) {
            let value = self.to_string(height);
            let _ = self.host.mutate_dom(DomMutation::SetAttribute {
                node: node_id,
                name: "height".to_string(),
                value,
            });
        }
        Ok(self.make_dom_node_value(node_id))
    }

    fn xhr_open(&mut self, this: &Value, args: &[Value]) -> Result<Value, VmError> {
        let Value::Object(obj) = this else {
            return Ok(Value::Undefined);
        };
        let obj = *obj;
        let method = args
            .first()
            .map(|v| self.to_string(v))
            .unwrap_or_else(|| "GET".to_string());
        let url = args.get(1).map(|v| self.to_string(v)).unwrap_or_default();
        let method_v = self.make_string_value(&method);
        let url_v = self.make_string_value(&url);
        self.define_data_property(obj, PropertyKey::from("__xhrMethod"), method_v, false, false, true);
        self.define_data_property(obj, PropertyKey::from("__xhrUrl"), url_v, false, false, true);
        self.define_data_property(obj, PropertyKey::from("readyState"), Value::Number(1.0), true, true, true);
        Ok(Value::Undefined)
    }

    fn xhr_set_request_header(&mut self, this: &Value, args: &[Value]) -> Result<Value, VmError> {
        let name = args.first().map(|v| self.to_string(v)).unwrap_or_default();
        let value = args.get(1).map(|v| self.to_string(v)).unwrap_or_default();
        let headers = self.get_property_value(this, &PropertyKey::from("__xhrReqHeaders"))?;
        if let Value::Object(h) = headers {
            let v = self.make_string_value(&value);
            self.define_data_property(h, PropertyKey::from(name.as_str()), v, true, true, true);
        }
        Ok(Value::Undefined)
    }

    fn xhr_send(&mut self, this: &Value, args: &[Value]) -> Result<Value, VmError> {
        let Value::Object(obj) = this else {
            return Ok(Value::Undefined);
        };
        let obj = *obj;
        let method_str = {
            let v = self.get_property_value(this, &PropertyKey::from("__xhrMethod"))?;
            self.to_string(&v)
        };
        let url = {
            let v = self.get_property_value(this, &PropertyKey::from("__xhrUrl"))?;
            self.to_string(&v)
        };
        let method = match method_str.to_uppercase().as_str() {
            "POST" => HttpMethod::Post,
            "PUT" => HttpMethod::Put,
            "PATCH" => HttpMethod::Patch,
            "DELETE" => HttpMethod::Delete,
            "HEAD" => HttpMethod::Head,
            "OPTIONS" => HttpMethod::Options,
            _ => HttpMethod::Get,
        };
        let mut headers = Vec::new();
        let hv = self.get_property_value(this, &PropertyKey::from("__xhrReqHeaders"))?;
        if let Value::Object(h) = hv {
            for key in self.object_own_enumerable_keys(h) {
                if let PropertyKey::String(name) = &key {
                    let val = self.get_property_value(&Value::Object(h), &key)?;
                    headers.push((name.clone(), self.to_string(&val)));
                }
            }
        }
        let body = match args.first() {
            Some(v) if !matches!(v, Value::Undefined | Value::Null) => {
                FetchBody::Utf8(self.to_string(v))
            }
            _ => FetchBody::Empty,
        };
        let request = FetchRequest {
            window: WindowId(0),
            url: url.clone(),
            method,
            headers,
            body,
            mode: FetchMode::Cors,
            keepalive: false,
        };

        match self.host.fetch_sync(request) {
            Ok(response) => {
                let status = response.status;
                let status_text = self.make_string_value(&response.status_text);
                let final_url = if response.final_url.is_empty() {
                    url.clone()
                } else {
                    response.final_url.clone()
                };
                let final_url_v = self.make_string_value(&final_url);
                let text = String::from_utf8_lossy(&response.body).into_owned();
                let text_v = self.make_string_value(&text);
                // Raw response headers string (CRLF-joined "name: value").
                let raw_headers = response
                    .headers
                    .iter()
                    .map(|(k, v)| format!("{}: {}", k.to_lowercase(), v))
                    .collect::<Vec<_>>()
                    .join("\r\n");
                let raw_headers_v = self.make_string_value(&raw_headers);
                self.define_data_property(obj, PropertyKey::from("__xhrRespHeaders"), raw_headers_v, false, false, true);
                self.define_data_property(obj, PropertyKey::from("status"), Value::Number(status as f64), true, true, true);
                self.define_data_property(obj, PropertyKey::from("statusText"), status_text, true, true, true);
                self.define_data_property(obj, PropertyKey::from("responseText"), text_v.clone(), true, true, true);
                self.define_data_property(obj, PropertyKey::from("responseURL"), final_url_v, true, true, true);
                // `response` honors responseType === 'json'.
                let response_type = {
                    let v = self.get_property_value(this, &PropertyKey::from("responseType"))?;
                    self.to_string(&v)
                };
                let response_value = if response_type == "json" {
                    match serde_json::from_str::<JsonValue>(&text) {
                        Ok(json) => self.from_json_value(&json)?,
                        Err(_) => Value::Null,
                    }
                } else {
                    text_v
                };
                self.define_data_property(obj, PropertyKey::from("response"), response_value, true, true, true);
                self.define_data_property(obj, PropertyKey::from("readyState"), Value::Number(4.0), true, true, true);
                self.xhr_fire(this, "onreadystatechange")?;
                self.xhr_fire(this, "onload")?;
                self.xhr_fire(this, "onloadend")?;
            }
            Err(_) => {
                self.define_data_property(obj, PropertyKey::from("status"), Value::Number(0.0), true, true, true);
                self.define_data_property(obj, PropertyKey::from("readyState"), Value::Number(4.0), true, true, true);
                self.xhr_fire(this, "onreadystatechange")?;
                self.xhr_fire(this, "onerror")?;
                self.xhr_fire(this, "onloadend")?;
            }
        }
        Ok(Value::Undefined)
    }

    /// Invoke an XHR `on*` handler property if it is callable.
    fn xhr_fire(&mut self, this: &Value, handler: &str) -> Result<(), VmError> {
        let cb = self.get_property_value(this, &PropertyKey::from(handler))?;
        if self.is_callable_value(&cb) {
            self.call_value_sync(cb, this.clone(), Vec::new())?;
            self.drain_microtasks();
        }
        Ok(())
    }

    fn xhr_get_all_response_headers(&mut self, this: &Value) -> Result<Value, VmError> {
        let v = self.get_property_value(this, &PropertyKey::from("__xhrRespHeaders"))?;
        let text = self.to_string(&v);
        Ok(self.make_string_value(&text))
    }

    fn xhr_get_response_header(&mut self, this: &Value, args: &[Value]) -> Result<Value, VmError> {
        let name = args
            .first()
            .map(|v| self.to_string(v))
            .unwrap_or_default()
            .to_lowercase();
        let v = self.get_property_value(this, &PropertyKey::from("__xhrRespHeaders"))?;
        let raw = self.to_string(&v);
        for line in raw.split("\r\n") {
            if let Some((k, val)) = line.split_once(": ") {
                if k.eq_ignore_ascii_case(&name) {
                    return Ok(self.make_string_value(val));
                }
            }
        }
        Ok(Value::Null)
    }

    fn build_response_object(&mut self, url: &str, response: FetchResponse) -> Value {
        let proto = self.object_prototype_ref();
        let object = self.allocate_ordinary_object(Some(proto));
        let ok = (200..300).contains(&response.status);
        let final_url = if response.final_url.is_empty() {
            url.to_string()
        } else {
            response.final_url.clone()
        };
        let redirected = final_url != url;
        let status_text = self.make_string_value(&response.status_text);
        let url_value = self.make_string_value(&final_url);
        let body_text = String::from_utf8_lossy(&response.body).into_owned();
        let body_value = self.make_string_value(&body_text);
        let headers = self.build_headers_object(&response.headers);
        let text_method = self.allocate_builtin_method(BuiltinId::ResponseText);
        let json_method = self.allocate_builtin_method(BuiltinId::ResponseJson);
        let set = |vm: &mut Self, name: &str, value: Value| {
            vm.define_data_property(object, PropertyKey::from(name), value, true, true, true);
        };
        set(self, "ok", Value::Bool(ok));
        set(self, "status", Value::Number(response.status as f64));
        set(self, "statusText", status_text);
        set(self, "url", url_value);
        set(self, "redirected", Value::Bool(redirected));
        set(self, "bodyUsed", Value::Bool(false));
        set(self, "headers", headers);
        set(self, "text", text_method);
        set(self, "json", json_method);
        // Body kept as a non-enumerable string for text()/json().
        self.define_data_property(
            object,
            PropertyKey::from("__body"),
            body_value,
            false,
            false,
            false,
        );
        Value::Object(object)
    }

    fn build_headers_object(&mut self, headers: &[(String, String)]) -> Value {
        let proto = self.object_prototype_ref();
        let object = self.allocate_ordinary_object(Some(proto));
        for (name, value) in headers {
            let value = self.make_string_value(value);
            self.define_data_property(
                object,
                PropertyKey::from(name.to_lowercase().as_str()),
                value,
                false,
                false,
                true,
            );
        }
        let get_method = self.allocate_builtin_method(BuiltinId::HeadersGet);
        self.define_data_property(object, PropertyKey::from("get"), get_method, true, true, true);
        Value::Object(object)
    }

    fn headers_get(&mut self, this: &Value, args: &[Value]) -> Result<Value, VmError> {
        let name = args
            .first()
            .map(|value| self.to_string(value))
            .unwrap_or_default()
            .to_lowercase();
        if let Value::Object(object) = this {
            if let Some(JsObject {
                kind: ObjectKind::Headers(pairs),
                ..
            }) = self.heap.objects().get(*object)
            {
                let values: Vec<String> = pairs
                    .iter()
                    .filter(|(k, _)| *k == name)
                    .map(|(_, v)| v.clone())
                    .collect();
                return Ok(if values.is_empty() {
                    Value::Null
                } else {
                    self.make_string_value(&values.join(", "))
                });
            }
        }
        let value = self.get_property_value(this, &PropertyKey::from(name.as_str()))?;
        Ok(if matches!(value, Value::Undefined) { Value::Null } else { value })
    }

    fn response_text(&mut self, this: &Value) -> Result<Value, VmError> {
        let body = self
            .get_property_value(this, &PropertyKey::from("__body"))
            .unwrap_or(Value::Undefined);
        let text = self.to_string(&body);
        let value = self.make_string_value(&text);
        Ok(Value::Object(self.promise_resolve_value(value)?))
    }

    fn response_json(&mut self, this: &Value) -> Result<Value, VmError> {
        let body = self
            .get_property_value(this, &PropertyKey::from("__body"))
            .unwrap_or(Value::Undefined);
        let text = self.to_string(&body);
        match serde_json::from_str::<JsonValue>(&text) {
            Ok(json) => {
                let parsed = self.from_json_value(&json)?;
                Ok(Value::Object(self.promise_resolve_value(parsed)?))
            }
            Err(error) => {
                let err = self.create_error_object("SyntaxError", error.to_string());
                Ok(Value::Object(self.promise_reject_value(err)?))
            }
        }
    }

    fn for_of_values(&mut self, value: &Value) -> Result<Vec<Value>, VmError> {
        match value {
            Value::String(string) => Ok(self
                .string_text(*string)
                .chars()
                .map(|character| self.make_string_value(&character.to_string()))
                .collect()),
            Value::Object(object) => {
                let kind = self
                    .heap
                    .objects()
                    .get(*object)
                    .map(|object| object.kind.clone())
                    .unwrap_or(ObjectKind::Ordinary);
                match kind {
                    ObjectKind::Array => self.array_like_to_vec(value),
                    ObjectKind::Map(entries) | ObjectKind::WeakMap(entries) => {
                        let mut pairs = Vec::with_capacity(entries.len());
                        for (key, value) in entries {
                            pairs.push(self.make_array_from_values(vec![key, value])?);
                        }
                        Ok(pairs)
                    }
                    ObjectKind::Set(values) | ObjectKind::WeakSet(values) => Ok(values),
                    ObjectKind::TypedArray { .. } => Ok(self.typed_array_to_values(value)),
                    ObjectKind::UrlSearchParams(pairs) => {
                        let mut entries = Vec::with_capacity(pairs.len());
                        for (name, value) in pairs {
                            let name_value = self.make_string_value(&name);
                            let value_value = self.make_string_value(&value);
                            entries.push(
                                self.make_array_from_values(vec![name_value, value_value])?,
                            );
                        }
                        Ok(entries)
                    }
                    ObjectKind::Headers(pairs) | ObjectKind::FormData(pairs) => {
                        let mut entries = Vec::with_capacity(pairs.len());
                        for (name, value) in pairs {
                            let name_value = self.make_string_value(&name);
                            let value_value = self.make_string_value(&value);
                            entries.push(
                                self.make_array_from_values(vec![name_value, value_value])?,
                            );
                        }
                        Ok(entries)
                    }
                    ObjectKind::ForOfIterator { values, index } => Ok(values[index.min(values.len())..].to_vec()),
                    _ => {
                        // Custom iterable: drain via its Symbol.iterator method.
                        if let Some(values) = self.iterate_via_symbol_iterator(value)? {
                            return Ok(values);
                        }
                        self.array_like_to_vec(value)
                    }
                }
            }
            _ => Err(VmError::TypeError(
                "value is not iterable in phase 4".to_string(),
            )),
        }
    }

    /// If `value` has a callable `Symbol.iterator`, invoke the iteration
    /// protocol (get iterator, repeatedly call `next()` until `done`) and return
    /// the collected values. Returns None if there is no Symbol.iterator method.
    fn iterate_via_symbol_iterator(
        &mut self,
        value: &Value,
    ) -> Result<Option<Vec<Value>>, VmError> {
        let iterator_key = PropertyKey::Symbol(SymbolId(SYMBOL_ITERATOR_ID));
        let iterator_fn = self.get_property_value(value, &iterator_key)?;
        if !self.is_callable_value(&iterator_fn) {
            return Ok(None);
        }
        let iterator = self.call_value_sync(iterator_fn, value.clone(), Vec::new())?;
        let next_fn = self.get_property_value(&iterator, &PropertyKey::from("next"))?;
        if !self.is_callable_value(&next_fn) {
            return Err(VmError::TypeError(
                "iterator.next is not a function".to_string(),
            ));
        }
        let mut values = Vec::new();
        // Cap iterations to avoid an unbounded loop on a misbehaving iterator.
        for _ in 0..1_000_000 {
            let result =
                self.call_value_sync(next_fn.clone(), iterator.clone(), Vec::new())?;
            let done = self.get_property_value(&result, &PropertyKey::from("done"))?;
            if self.is_truthy(&done) {
                break;
            }
            let item = self.get_property_value(&result, &PropertyKey::from("value"))?;
            values.push(item);
        }
        Ok(Some(values))
    }

    fn for_of_next(&mut self, iterator: GcRef<JsObject>) -> Result<Option<Value>, VmError> {
        let Some(object_data) = self.heap.objects_mut().get_mut(iterator) else {
            return Err(VmError::TypeError("invalid iterator object".to_string()));
        };
        match &mut object_data.kind {
            ObjectKind::ForOfIterator { values, index } => {
                if *index >= values.len() {
                    Ok(None)
                } else {
                    let value = values[*index].clone();
                    *index += 1;
                    Ok(Some(value))
                }
            }
            _ => Err(VmError::TypeError(
                "object is not a for...of iterator".to_string(),
            )),
        }
    }

    fn allocate_for_of_iterator_adapter(
        &mut self,
        value: &Value,
    ) -> Result<GcRef<JsObject>, VmError> {
        let values = self.for_of_values(value)?;
        let iterator = self.heap.allocate_object(JsObject {
            kind: ObjectKind::ForOfIterator { values, index: 0 },
            prototype: Some(self.object_prototype_ref()),
            ..JsObject::default()
        });
        let next = self.allocate_builtin_method(BuiltinId::ForOfIteratorAdapterNext);
        self.define_data_property(
            iterator,
            PropertyKey::from("next"),
            next,
            true,
            true,
            true,
        );
        Ok(iterator)
    }

    /// The DOM interfaces a host node value satisfies, for `instanceof`. Host DOM
    /// nodes are not wired into the JS prototype chain of the interface
    /// constructors, so membership is decided structurally by the host class.
    fn host_node_interfaces(&self, object: GcRef<JsObject>) -> Option<&'static [&'static str]> {
        let data = self.heap.objects().get(object)?;
        if let ObjectKind::Host(slot) = &data.kind {
            return Some(match slot.class {
                // Element nodes (our generic node wrapper) — treat as the element
                // interface chain. Text/Comment also use this class today; the
                // distinction rarely matters for the libraries that gate on these.
                HostObjectClass::Node | HostObjectClass::EventTarget => {
                    &["EventTarget", "Node", "Element", "HTMLElement"]
                }
                HostObjectClass::Document => &["EventTarget", "Node", "Document"],
                HostObjectClass::Window => &["EventTarget", "Window"],
                _ => return None,
            });
        }
        None
    }

    /// Whether `key` is an event-handler IDL attribute (`onclick`, `oninput`, …)
    /// on a host DOM object. In a real browser these properties always exist on
    /// Element/Document/Window (defaulting to null), and feature detection relies
    /// on it: `'oninput' in document` is how React decides whether the native
    /// `input` event is supported — if it reads false, React falls back to an IE
    /// polyfill path and `onChange` never fires from input events.
    fn host_has_event_handler_property(&self, object: GcRef<JsObject>, key: &PropertyKey) -> bool {
        let PropertyKey::String(name) = key else {
            return false;
        };
        if !(name.len() > 2 && name.starts_with("on") && name.as_bytes()[2].is_ascii_lowercase()) {
            return false;
        }
        self.heap.objects().get(object).is_some_and(|data| {
            matches!(
                &data.kind,
                ObjectKind::Host(slot)
                    if matches!(
                        slot.class,
                        HostObjectClass::Node
                            | HostObjectClass::EventTarget
                            | HostObjectClass::Document
                            | HostObjectClass::Window
                    )
            )
        })
    }

    fn instanceof_value(&self, value: &Value, constructor: &Value) -> Result<bool, VmError> {
        let Value::Object(object) = value else {
            return Ok(false);
        };
        let ctor = self.require_object_ref(constructor, "instanceof right-hand side")?;
        // DOM interface constructors (`Element`, `Node`, …): host nodes satisfy
        // them by interface name rather than by JS prototype chain.
        if let Some(iface) = self.dom_interface_ctors.get(&ctor.raw()) {
            return Ok(self
                .host_node_interfaces(*object)
                .is_some_and(|set| set.iter().any(|name| name == iface)));
        }
        let prototype =
            match self.get_own_property_descriptor(ctor, &PropertyKey::from("prototype")) {
                Some(JsPropertyDescriptor::Data {
                    value: Value::Object(prototype),
                    ..
                }) => prototype,
                _ => {
                    return Err(VmError::TypeError(
                        "constructor.prototype must be an object".to_string(),
                    ));
                }
            };
        let mut current = self
            .heap
            .objects()
            .get(*object)
            .and_then(|data| data.prototype);
        while let Some(current_object) = current {
            if current_object.raw() == prototype.raw() {
                return Ok(true);
            }
            current = self
                .heap
                .objects()
                .get(current_object)
                .and_then(|data| data.prototype);
        }
        Ok(false)
    }

    fn invoke_builtin(
        &mut self,
        builtin: BuiltinId,
        this_value: Value,
        args: Vec<Value>,
    ) -> Result<Value, VmError> {
        let result = self.invoke_builtin_inner(builtin, this_value, args);
        // boa parity: mutation observers flush synchronously after every DOM
        // operation, so scripts can read their effects mid-turn. Cheap (two
        // field reads) when no observers are registered, and re-entrancy is
        // guarded inside the delivery itself.
        self.deliver_mutation_records();
        self.deliver_slotchange();
        result
    }

    /// Fire `slotchange` on every `<slot>` whose assignment changed since the
    /// last delivery (boa parity: flushed after each DOM operation). Bounded
    /// and re-entrancy guarded.
    fn deliver_slotchange(&mut self) {
        if self.delivering_slotchange {
            return;
        }
        self.delivering_slotchange = true;
        for _ in 0..8 {
            let slots = match self.host.mutate_dom(DomMutation::TakeSlotchangeSlots { window: WindowId(0) }) {
                Ok(DomMutationResult::Nodes(ids)) if !ids.is_empty() => ids,
                _ => break,
            };
            for slot in slots {
                let _ = self.fire_dom_event(slot.0, "slotchange");
            }
        }
        self.delivering_slotchange = false;
    }

    fn invoke_builtin_inner(
        &mut self,
        builtin: BuiltinId,
        this_value: Value,
        args: Vec<Value>,
    ) -> Result<Value, VmError> {
        match builtin {
            BuiltinId::Assert => {
                let condition = args.first().cloned().unwrap_or(Value::Undefined);
                if self.is_truthy(&condition) {
                    Ok(Value::Undefined)
                } else {
                    Err(VmError::TypeError("assertion failed".to_string()))
                }
            }
            BuiltinId::CallSpread => {
                let callee = args.first().cloned().unwrap_or(Value::Undefined);
                let this_arg = args.get(1).cloned().unwrap_or(Value::Undefined);
                let spread_args = match args.get(2) {
                    Some(Value::Null) | Some(Value::Undefined) | None => Vec::new(),
                    Some(value) => self.array_like_to_vec(value)?,
                };
                self.call_value_sync(callee, this_arg, spread_args)
            }
            BuiltinId::ConstructSpread => {
                let constructor = args.first().cloned().unwrap_or(Value::Undefined);
                let spread_args = match args.get(1) {
                    Some(Value::Null) | Some(Value::Undefined) | None => Vec::new(),
                    Some(value) => self.array_like_to_vec(value)?,
                };
                self.construct_value_sync(constructor, spread_args)
            }
            BuiltinId::PromiseConstructor => {
                let executor = args.first().cloned().unwrap_or(Value::Undefined);
                let promise = self.allocate_pending_promise_object();
                let resolve = self
                    .create_promise_capability_function(promise, PromiseCapabilityMode::Resolve);
                let reject =
                    self.create_promise_capability_function(promise, PromiseCapabilityMode::Reject);
                if !self.is_callable_value(&executor) {
                    return Err(VmError::TypeError(
                        "Promise constructor requires a callable executor".to_string(),
                    ));
                }
                match self.call_value_sync(executor, Value::Undefined, vec![resolve, reject]) {
                    Ok(_) => Ok(Value::Object(promise)),
                    Err(error) => {
                        let reason = self.wrap_vm_error_as_value(&error)?;
                        self.reject_promise_with_value(promise, reason)?;
                        Ok(Value::Object(promise))
                    }
                }
            }
            BuiltinId::FunctionConstructor => {
                let body = args
                    .last()
                    .map(|value| self.to_string(value))
                    .unwrap_or_default();
                let params = if args.is_empty() {
                    String::new()
                } else {
                    args[..args.len() - 1]
                        .iter()
                        .map(|value| self.to_string(value))
                        .collect::<Vec<_>>()
                        .join(",")
                };
                let temp_name = "__tobira_function_constructor_result";
                let source = format!(
                    "globalThis.{temp_name} = (function anonymous({params}) {{\n{body}\n}});"
                );
                self.eval_source(&source)?;
                Ok(self
                    .globals
                    .remove(temp_name)
                    .unwrap_or(Value::Undefined))
            }
            BuiltinId::PromiseResolve => {
                let value = args.first().cloned().unwrap_or(Value::Undefined);
                Ok(Value::Object(self.promise_resolve_value(value)?))
            }
            BuiltinId::PromiseReject => {
                let reason = args.first().cloned().unwrap_or(Value::Undefined);
                Ok(Value::Object(self.promise_reject_value(reason)?))
            }
            BuiltinId::PromiseAll => {
                let iterable = args.first().cloned().unwrap_or(Value::Undefined);
                let values = self.for_of_values(&iterable)?;
                self.promise_all(values)
            }
            BuiltinId::PromiseRace => {
                let iterable = args.first().cloned().unwrap_or(Value::Undefined);
                let values = self.for_of_values(&iterable)?;
                self.promise_race(values)
            }
            BuiltinId::PromiseAllSettled => {
                let iterable = args.first().cloned().unwrap_or(Value::Undefined);
                let values = self.for_of_values(&iterable)?;
                self.promise_all_settled(values)
            }
            BuiltinId::PromiseAny => {
                let iterable = args.first().cloned().unwrap_or(Value::Undefined);
                let values = self.for_of_values(&iterable)?;
                self.promise_any(values)
            }
            BuiltinId::PromiseProtoThen | BuiltinId::PromiseProtoCatch => {
                let is_catch = matches!(builtin, BuiltinId::PromiseProtoCatch);
                let context = if is_catch {
                    "Promise.prototype.catch"
                } else {
                    "Promise.prototype.then"
                };
                let promise = self.require_promise_this(&this_value, context)?;
                let (on_fulfilled, on_rejected) = if is_catch {
                    (None, self.normalize_handler_value(args.first()))
                } else {
                    (
                        self.normalize_handler_value(args.first()),
                        self.normalize_handler_value(args.get(1)),
                    )
                };
                Ok(Value::Object(self.promise_then_internal(
                    promise,
                    on_fulfilled,
                    on_rejected,
                )?))
            }
            BuiltinId::PromiseProtoFinally => {
                let promise =
                    self.require_promise_this(&this_value, "Promise.prototype.finally")?;
                let callback = args.first().cloned().unwrap_or(Value::Undefined);
                if !self.is_callable_value(&callback) {
                    return Ok(Value::Object(
                        self.promise_then_internal(promise, None, None)?,
                    ));
                }
                let on_fulfilled = self
                    .create_promise_finally_function(callback.clone(), PromiseFinallyMode::Fulfill);
                let on_rejected =
                    self.create_promise_finally_function(callback, PromiseFinallyMode::Reject);
                Ok(Value::Object(self.promise_then_internal(
                    promise,
                    self.value_object_ref(on_fulfilled),
                    self.value_object_ref(on_rejected),
                )?))
            }
            BuiltinId::QueueMicrotask => {
                let callback = self.require_callable_object(
                    args.first().unwrap_or(&Value::Undefined),
                    "queueMicrotask",
                )?;
                self.queue_microtask_job(MicrotaskJob::QueueMicrotask(callback));
                Ok(Value::Undefined)
            }
            BuiltinId::ClearTimeout | BuiltinId::ClearInterval => {
                if let Some(id_value) = args.first() {
                    let id = self.to_uint32(id_value);
                    self.event_loop.cancelled_timers.insert(id);
                }
                Ok(Value::Undefined)
            }
            BuiltinId::SetTimeout | BuiltinId::SetInterval => {
                let callback_name = match builtin {
                    BuiltinId::SetTimeout => "setTimeout",
                    BuiltinId::SetInterval => "setInterval",
                    _ => unreachable!(),
                };
                let callback = self.require_callable_object(
                    args.first().unwrap_or(&Value::Undefined),
                    callback_name,
                )?;
                let delay = self
                    .to_number(args.get(1).unwrap_or(&Value::Number(0.0)))
                    .max(0.0);
                let (delay_ms, interval) = match builtin {
                    BuiltinId::SetTimeout => (delay as i64, None),
                    BuiltinId::SetInterval => {
                        let delay_ms = delay as u64;
                        (delay_ms as i64, Some(delay_ms))
                    }
                    _ => unreachable!(),
                };
                let id = self.schedule_timer(
                    callback,
                    delay_ms,
                    interval,
                    args.into_iter().skip(2).collect(),
                );
                Ok(Value::Number(id as f64))
            }
            BuiltinId::RequestAnimationFrame => {
                let callback = self.require_callable_object(
                    args.first().unwrap_or(&Value::Undefined),
                    "requestAnimationFrame",
                )?;
                let id = self.event_loop.next_raf_id;
                self.event_loop.next_raf_id = self.event_loop.next_raf_id.wrapping_add(1);
                self.event_loop
                    .raf_callbacks
                    .insert(id, RafEntry { id, callback });
                Ok(Value::Number(id as f64))
            }
            BuiltinId::CancelAnimationFrame => {
                let id = self.to_uint32(args.first().unwrap_or(&Value::Undefined));
                self.event_loop.raf_callbacks.shift_remove(&id);
                Ok(Value::Undefined)
            }
            BuiltinId::ObjectConstructor => Ok(match args.first() {
                Some(Value::Object(_)) => args[0].clone(),
                _ => {
                    Value::Object(self.allocate_ordinary_object(Some(self.object_prototype_ref())))
                }
            }),
            BuiltinId::ObjectCreate => {
                let prototype = match args.first().cloned().unwrap_or(Value::Null) {
                    Value::Null => None,
                    Value::Object(object) => Some(object),
                    _ => {
                        return Err(VmError::TypeError(
                            "Object.create prototype must be an object or null".to_string(),
                        ));
                    }
                };
                Ok(Value::Object(self.allocate_ordinary_object(prototype)))
            }
            BuiltinId::ObjectDefineProperty => {
                let object = self.require_object_ref(
                    args.first().unwrap_or(&Value::Undefined),
                    "Object.defineProperty",
                )?;
                let name = self.to_property_key(args.get(1).unwrap_or(&Value::Undefined))?;
                let descriptor =
                    self.value_to_property_descriptor(args.get(2).unwrap_or(&Value::Undefined))?;
                if let Some(object_data) = self.heap.objects_mut().get_mut(object) {
                    object_data.properties.insert(name.clone(), descriptor);
                }
                self.update_array_length_for_key(object, &name)?;
                Ok(Value::Object(object))
            }
            BuiltinId::ObjectGetOwnPropertyDescriptor => {
                let target = args.first().unwrap_or(&Value::Undefined);
                let key = self.to_property_key(args.get(1).unwrap_or(&Value::Undefined))?;
                self.object_introspection_get_own_property_descriptor(
                    target,
                    &key,
                    "Object.getOwnPropertyDescriptor",
                )
            }
            BuiltinId::ObjectKeys => {
                self.object_introspection_keys_like(
                    args.first().unwrap_or(&Value::Undefined),
                    ObjectIntrospectionKind::Keys,
                    "Object.keys",
                )
            }
            BuiltinId::ObjectGetOwnPropertySymbols => {
                self.object_introspection_get_own_property_symbols(
                    args.first().unwrap_or(&Value::Undefined),
                )
            }
            BuiltinId::ObjectValues => {
                self.object_introspection_keys_like(
                    args.first().unwrap_or(&Value::Undefined),
                    ObjectIntrospectionKind::Values,
                    "Object.values",
                )
            }
            BuiltinId::ObjectEntries => {
                self.object_introspection_keys_like(
                    args.first().unwrap_or(&Value::Undefined),
                    ObjectIntrospectionKind::Entries,
                    "Object.entries",
                )
            }
            BuiltinId::ObjectAssign => {
                let target = self.require_object_ref(
                    args.first().unwrap_or(&Value::Undefined),
                    "Object.assign",
                )?;
                for source in args.iter().skip(1) {
                    if matches!(source, Value::Null | Value::Undefined) {
                        continue;
                    }
                    let source_object = self.require_object_ref(source, "Object.assign source")?;
                    let keys = self.object_own_enumerable_keys(source_object);
                    for key in keys {
                        let value = self.get_property_value(source, &key)?;
                        self.set_property_on_object(target, Value::Object(target), key, value)?;
                    }
                }
                Ok(Value::Object(target))
            }
            BuiltinId::ObjectGetPrototypeOf => {
                self.object_introspection_get_prototype_of(
                    args.first().unwrap_or(&Value::Undefined),
                    "Object.getPrototypeOf",
                )
            }
            BuiltinId::ObjectSetPrototypeOf => {
                let object = self.require_object_ref(
                    args.first().unwrap_or(&Value::Undefined),
                    "Object.setPrototypeOf",
                )?;
                let prototype = match args.get(1).cloned().unwrap_or(Value::Null) {
                    Value::Null => None,
                    Value::Object(object) => Some(object),
                    _ => {
                        return Err(VmError::TypeError(
                            "prototype must be an object or null".to_string(),
                        ));
                    }
                };
                if let Some(object_data) = self.heap.objects_mut().get_mut(object) {
                    object_data.prototype = prototype;
                }
                Ok(Value::Object(object))
            }
            BuiltinId::ObjectFreeze => {
                match args.first().cloned().unwrap_or(Value::Undefined) {
                    Value::Object(object) => {
                        self.freeze_object(object);
                        Ok(Value::Object(object))
                    }
                    value => Ok(value),
                }
            }
            BuiltinId::ObjectIsFrozen => {
                match args.first() {
                    Some(Value::Object(object)) => Ok(Value::Bool(self.is_frozen(*object))),
                    _ => Ok(Value::Bool(true)),
                }
            }
            BuiltinId::ObjectProtoHasOwnProperty => {
                let object = self.builtin_object_this(&this_value, "hasOwnProperty")?;
                let key = self.to_property_key(args.first().unwrap_or(&Value::Undefined))?;
                Ok(Value::Bool(
                    self.get_own_property_descriptor(object, &key).is_some(),
                ))
            }
            BuiltinId::ObjectProtoPropertyIsEnumerable => {
                let object = self.builtin_object_this(&this_value, "propertyIsEnumerable")?;
                let key = self.to_property_key(args.first().unwrap_or(&Value::Undefined))?;
                let enumerable = match self.get_own_property_descriptor(object, &key) {
                    Some(JsPropertyDescriptor::Data { enumerable, .. })
                    | Some(JsPropertyDescriptor::Accessor { enumerable, .. }) => enumerable,
                    None => false,
                };
                Ok(Value::Bool(enumerable))
            }
            BuiltinId::ObjectProtoToString => Ok(self.make_string_value("[object Object]")),
            BuiltinId::ObjectProtoValueOf => Ok(this_value),
            BuiltinId::ObjectProtoIsPrototypeOf => {
                let prototype = self.builtin_object_this(&this_value, "isPrototypeOf")?;
                let mut current = match args.first().cloned().unwrap_or(Value::Undefined) {
                    Value::Object(object) => self
                        .heap
                        .objects()
                        .get(object)
                        .and_then(|data| data.prototype),
                    _ => None,
                };
                while let Some(object) = current {
                    if object.raw() == prototype.raw() {
                        return Ok(Value::Bool(true));
                    }
                    current = self
                        .heap
                        .objects()
                        .get(object)
                        .and_then(|data| data.prototype);
                }
                Ok(Value::Bool(false))
            }
            BuiltinId::FunctionProtoCall => {
                let target = this_value;
                let this_arg = args.first().cloned().unwrap_or(Value::Undefined);
                let call_args = args.into_iter().skip(1).collect();
                self.call_value_sync(target, this_arg, call_args)
            }
            BuiltinId::FunctionProtoApply => {
                let target = this_value;
                let this_arg = args.first().cloned().unwrap_or(Value::Undefined);
                let call_args = match args.get(1) {
                    Some(Value::Null) | Some(Value::Undefined) | None => Vec::new(),
                    Some(value) => self.array_like_to_vec(value)?,
                };
                self.call_value_sync(target, this_arg, call_args)
            }
            BuiltinId::FunctionProtoBind => {
                let target = this_value;
                let this_arg = args.first().cloned().unwrap_or(Value::Undefined);
                let bound_args = args.into_iter().skip(1).collect();
                Ok(self.allocate_bound_function_value(BoundFunction {
                    target,
                    bound_this: this_arg,
                    bound_args,
                }))
            }
            BuiltinId::ErrorConstructor
            | BuiltinId::TypeErrorConstructor
            | BuiltinId::RangeErrorConstructor
            | BuiltinId::ReferenceErrorConstructor
            | BuiltinId::SyntaxErrorConstructor
            | BuiltinId::UriErrorConstructor
            | BuiltinId::EvalErrorConstructor => {
                let name = match builtin {
                    BuiltinId::ErrorConstructor => "Error",
                    BuiltinId::TypeErrorConstructor => "TypeError",
                    BuiltinId::RangeErrorConstructor => "RangeError",
                    BuiltinId::ReferenceErrorConstructor => "ReferenceError",
                    BuiltinId::SyntaxErrorConstructor => "SyntaxError",
                    BuiltinId::UriErrorConstructor => "URIError",
                    BuiltinId::EvalErrorConstructor => "EvalError",
                    _ => unreachable!(),
                };
                Ok(self.create_error_object(
                    name,
                    args.first()
                        .map(|value| self.to_string(value))
                        .unwrap_or_default(),
                ))
            }
            BuiltinId::ArrayConstructor => {
                // `Array(n)` with a single non-negative integer creates an array
                // of that length; otherwise the args become the elements.
                if let [Value::Number(n)] = args.as_slice() {
                    let n = *n;
                    if n >= 0.0 && n.fract() == 0.0 && n <= u32::MAX as f64 {
                        let array = self.make_array_from_values(Vec::new())?;
                        if let Value::Object(object) = &array {
                            self.set_array_length(*object, n as u32);
                        }
                        return Ok(array);
                    }
                }
                self.make_array_from_values(args)
            }
            BuiltinId::MapConstructor | BuiltinId::WeakMapConstructor => {
                let object = self.heap.allocate_object(JsObject {
                    kind: ObjectKind::Map(Vec::new()),
                    prototype: Some(self.map_prototype_ref()),
                    ..JsObject::default()
                });
                self.set_collection_size(object, 0);
                if let Some(iterable) = args.first()
                    && !matches!(iterable, Value::Null | Value::Undefined)
                {
                    for pair in self.for_of_values(iterable)? {
                        let values = self.array_like_to_vec(&pair)?;
                        let key = values.first().cloned().unwrap_or(Value::Undefined);
                        let value = values.get(1).cloned().unwrap_or(Value::Undefined);
                        self.map_set(object, key, value, false)?;
                    }
                }
                Ok(Value::Object(object))
            }
            BuiltinId::SetConstructor | BuiltinId::WeakSetConstructor => {
                let object = self.heap.allocate_object(JsObject {
                    kind: ObjectKind::Set(Vec::new()),
                    prototype: Some(self.set_prototype_ref()),
                    ..JsObject::default()
                });
                self.set_collection_size(object, 0);
                if let Some(iterable) = args.first()
                    && !matches!(iterable, Value::Null | Value::Undefined)
                {
                    for value in self.for_of_values(iterable)? {
                        self.set_add(object, value, false)?;
                    }
                }
                Ok(Value::Object(object))
            }
            BuiltinId::ArrayBufferConstructor => self.construct_array_buffer(&args),
            BuiltinId::ArrayBufferProtoSlice => self.array_buffer_slice(&this_value, &args),
            BuiltinId::TypedArrayConstructor(kind) => self.construct_typed_array(kind, &args),
            BuiltinId::TypedArrayFrom(kind) => self.typed_array_from(kind, &args),
            BuiltinId::TypedArrayOf(kind) => {
                let buffer = self.make_array_buffer(args.len() * kind.bytes_per_element());
                let view = self.make_typed_array(kind, buffer, 0, args.len())?;
                for (index, value) in args.iter().enumerate() {
                    let number = self.to_number(value);
                    self.typed_array_write_element(&view, index, number)?;
                }
                Ok(view)
            }
            BuiltinId::TypedArrayProtoSet => self.typed_array_proto_set(&this_value, &args),
            BuiltinId::TypedArrayProtoSubarray => self.typed_array_subarray(&this_value, &args),
            BuiltinId::TypedArrayProtoSlice => self.typed_array_slice(&this_value, &args),
            BuiltinId::TypedArrayProtoFill => self.typed_array_fill(&this_value, &args),
            BuiltinId::TypedArrayProtoJoin => self.typed_array_join(&this_value, &args),
            BuiltinId::TypedArrayProtoIndexOf => {
                self.typed_array_index_of(&this_value, &args, false)
            }
            BuiltinId::TypedArrayProtoIncludes => {
                self.typed_array_index_of(&this_value, &args, true)
            }
            BuiltinId::TypedArrayProtoForEach => self.typed_array_for_each(&this_value, &args),
            BuiltinId::TypedArrayProtoMap => self.typed_array_map(&this_value, &args),
            BuiltinId::TypedArrayProtoReduce => self.typed_array_reduce(&this_value, &args),
            BuiltinId::TypedArrayProtoReverse => self.typed_array_reverse(&this_value),
            BuiltinId::EventConstructor => {
                let event_type = args.first().map(|v| self.to_string(v)).unwrap_or_default();
                self.make_event_object(&event_type, args.get(1).cloned(), false)
            }
            BuiltinId::CustomEventConstructor => {
                let event_type = args.first().map(|v| self.to_string(v)).unwrap_or_default();
                self.make_event_object(&event_type, args.get(1).cloned(), true)
            }
            BuiltinId::CustomElementsDefine => {
                let tag = args.first().map(|v| self.to_string(v)).unwrap_or_default().to_ascii_lowercase();
                let class_value = args.get(1).cloned().unwrap_or(Value::Undefined);
                if tag.is_empty() || !matches!(class_value, Value::Object(_)) {
                    return Ok(Value::Undefined);
                }
                // Read the static `observedAttributes` getter (lowercased).
                let observed = match self.get_property_value(&class_value, &PropertyKey::from("observedAttributes")) {
                    Ok(list @ Value::Object(_)) => self
                        .array_like_to_vec(&list)
                        .unwrap_or_default()
                        .iter()
                        .map(|v| self.to_string(v).to_ascii_lowercase())
                        .collect(),
                    _ => Vec::new(),
                };
                self.custom_elements.insert(
                    tag.clone(),
                    CustomElementDef { class_value: class_value.clone(), observed },
                );
                // Upgrade existing matching elements in document order.
                let matches = match self.host.read_dom(DomRead::QuerySelectorAll {
                    root: NodeId(0),
                    selectors: tag.clone(),
                }) {
                    Ok(DomReadResult::Nodes(ids)) => ids,
                    _ => Vec::new(),
                };
                for node in matches {
                    self.upgrade_custom_element(node, &tag)?;
                }
                Ok(Value::Undefined)
            }
            BuiltinId::CustomElementsGet => {
                let tag = args.first().map(|v| self.to_string(v)).unwrap_or_default().to_ascii_lowercase();
                Ok(self
                    .custom_elements
                    .get(&tag)
                    .map(|def| def.class_value.clone())
                    .unwrap_or(Value::Undefined))
            }
            BuiltinId::AbortControllerConstructor => {
                let proto = self.object_prototype_ref();
                let signal = self.make_abort_signal(false, Value::Undefined)?;
                let controller = self.allocate_ordinary_object(Some(proto));
                self.define_data_property(controller, PropertyKey::from("signal"), Value::Object(signal), true, true, true);
                let abort_fn = self.allocate_builtin_method(BuiltinId::AbortControllerAbort);
                self.define_data_property(controller, PropertyKey::from("abort"), abort_fn, true, false, true);
                Ok(Value::Object(controller))
            }
            BuiltinId::AbortSignalConstructor => self.make_abort_signal(false, Value::Undefined).map(Value::Object),
            BuiltinId::AbortSignalAbortStatic => {
                let reason = args.first().cloned().unwrap_or(Value::Undefined);
                self.make_abort_signal(true, reason).map(Value::Object)
            }
            BuiltinId::AbortSignalTimeoutStatic => {
                // Timed abort delivery is not wired into the event loop yet.
                self.make_abort_signal(false, Value::Undefined).map(Value::Object)
            }
            BuiltinId::AbortSignalAnyStatic => {
                let values = args.first().map(|v| self.array_like_to_vec(v)).transpose()?.unwrap_or_default();
                for value in values {
                    if let Value::Object(signal_ref) = &value {
                        let signal_val = Value::Object(*signal_ref);
                        let aborted = self.get_property_value(&signal_val, &PropertyKey::from("aborted"))?;
                        if self.is_truthy(&aborted) {
                            let reason = self.get_property_value(&signal_val, &PropertyKey::from("reason"))?;
                            return self.make_abort_signal(true, reason).map(Value::Object);
                        }
                    }
                }
                // Future abort subscriptions are not wired yet.
                self.make_abort_signal(false, Value::Undefined).map(Value::Object)
            }
            BuiltinId::AbortSignalThrowIfAborted => {
                let aborted = self.get_property_value(&this_value, &PropertyKey::from("aborted"))?;
                if self.is_truthy(&aborted) {
                    let reason = self.get_property_value(&this_value, &PropertyKey::from("reason"))?;
                    return Err(VmError::Thrown(reason));
                }
                Ok(Value::Undefined)
            }
            BuiltinId::AbortControllerAbort => {
                let signal = self.get_property_value(&this_value, &PropertyKey::from("signal"))?;
                let Value::Object(signal_ref) = signal else {
                    return Ok(Value::Undefined);
                };
                let signal_val = Value::Object(signal_ref);
                let aborted = self.get_property_value(&signal_val, &PropertyKey::from("aborted"))?;
                if self.is_truthy(&aborted) {
                    return Ok(Value::Undefined);
                }
                self.define_data_property(signal_ref, PropertyKey::from("aborted"), Value::Bool(true), true, true, true);
                let reason = args.first().cloned().unwrap_or(Value::Undefined);
                self.define_data_property(signal_ref, PropertyKey::from("reason"), reason, true, true, true);
                let event = self.make_event_object("abort", None, false)?;
                if let Value::Object(event_ref) = event {
                    self.define_data_property(event_ref, PropertyKey::from("target"), signal_val.clone(), true, true, true);
                    self.define_data_property(event_ref, PropertyKey::from("currentTarget"), signal_val.clone(), true, true, true);
                }
                let listeners_val =
                    self.get_property_value(&signal_val, &PropertyKey::from("__abortListeners"))?;
                let listeners = match &listeners_val {
                    Value::Object(_) => self.array_like_to_vec(&listeners_val)?,
                    _ => Vec::new(),
                };
                for listener in listeners {
                    if self.is_callable_value(&listener) {
                        let _ = self.call_value_sync(listener, signal_val.clone(), vec![event.clone()]);
                    }
                }
                let onabort = self.get_property_value(&signal_val, &PropertyKey::from("onabort"))?;
                if self.is_callable_value(&onabort) {
                    let _ = self.call_value_sync(onabort, signal_val.clone(), vec![event]);
                }
                Ok(Value::Undefined)
            }
            BuiltinId::AbortSignalAddEventListener => {
                let event_type = args.first().map(|v| self.to_string(v)).unwrap_or_default();
                let listener = args.get(1).cloned().unwrap_or(Value::Undefined);
                if event_type == "abort" && self.is_callable_value(&listener) {
                    let listeners_val = self
                        .get_property_value(&this_value, &PropertyKey::from("__abortListeners"))?;
                    let mut listeners = match &listeners_val {
                        Value::Object(_) => self.array_like_to_vec(&listeners_val)?,
                        _ => Vec::new(),
                    };
                    listeners.push(listener);
                    let updated = self.make_array_from_values(listeners)?;
                    if let Value::Object(signal_ref) = &this_value {
                        self.define_data_property(*signal_ref, PropertyKey::from("__abortListeners"), updated, true, false, true);
                    }
                }
                Ok(Value::Undefined)
            }
            BuiltinId::AbortSignalRemoveEventListener => {
                let event_type = args.first().map(|v| self.to_string(v)).unwrap_or_default();
                let listener = args.get(1).cloned().unwrap_or(Value::Undefined);
                if event_type == "abort" {
                    let listeners_val = self
                        .get_property_value(&this_value, &PropertyKey::from("__abortListeners"))?;
                    if matches!(listeners_val, Value::Object(_)) {
                        let all = self.array_like_to_vec(&listeners_val)?;
                        let listeners: Vec<Value> = all
                            .into_iter()
                            .filter(|l| !self.strict_equal(l, &listener))
                            .collect();
                        let updated = self.make_array_from_values(listeners)?;
                        if let Value::Object(signal_ref) = &this_value {
                            self.define_data_property(*signal_ref, PropertyKey::from("__abortListeners"), updated, true, false, true);
                        }
                    }
                }
                Ok(Value::Undefined)
            }
            BuiltinId::KeyboardEventConstructor => {
                let event_type = args.first().map(|v| self.to_string(v)).unwrap_or_default();
                let options = args.get(1).cloned();
                let event = self.make_event_object(&event_type, options.clone(), false)?;
                if let Value::Object(event_ref) = event {
                    for name in ["key", "code"] {
                        let v = self.event_option_string(&options, name);
                        let v = self.make_string_value(&v);
                        self.define_data_property(event_ref, PropertyKey::from(name), v, true, true, true);
                    }
                    for name in ["ctrlKey", "shiftKey", "altKey", "metaKey", "repeat"] {
                        let flag = self.event_option_flag(&options, name);
                        self.define_data_property(event_ref, PropertyKey::from(name), Value::Bool(flag), true, true, true);
                    }
                    let location = self.event_option_number(&options, "location");
                    self.define_data_property(event_ref, PropertyKey::from("location"), Value::Number(location), true, true, true);
                }
                Ok(event)
            }
            BuiltinId::MouseEventConstructor => {
                let event_type = args.first().map(|v| self.to_string(v)).unwrap_or_default();
                let options = args.get(1).cloned();
                let event = self.make_event_object(&event_type, options.clone(), false)?;
                if let Value::Object(event_ref) = event {
                    for name in ["clientX", "clientY", "screenX", "screenY", "pageX", "pageY", "button", "buttons", "detail"] {
                        let n = self.event_option_number(&options, name);
                        self.define_data_property(event_ref, PropertyKey::from(name), Value::Number(n), true, true, true);
                    }
                    for name in ["ctrlKey", "shiftKey", "altKey", "metaKey"] {
                        let flag = self.event_option_flag(&options, name);
                        self.define_data_property(event_ref, PropertyKey::from(name), Value::Bool(flag), true, true, true);
                    }
                    let related = match &options {
                        Some(opts) => self
                            .get_property_value(opts, &PropertyKey::from("relatedTarget"))
                            .unwrap_or(Value::Null),
                        None => Value::Null,
                    };
                    let related = if matches!(related, Value::Undefined) { Value::Null } else { related };
                    self.define_data_property(event_ref, PropertyKey::from("relatedTarget"), related, true, true, true);
                }
                Ok(event)
            }
            BuiltinId::EventPreventDefault => {
                if let Value::Object(event) = &this_value {
                    let cancelable =
                        self.get_property_value(&this_value, &PropertyKey::from("cancelable"))?;
                    if self.is_truthy(&cancelable) {
                        self.define_data_property(
                            *event,
                            PropertyKey::from("defaultPrevented"),
                            Value::Bool(true),
                            true,
                            true,
                            true,
                        );
                    }
                }
                Ok(Value::Undefined)
            }
            BuiltinId::EventStopPropagation => {
                if let Value::Object(event) = &this_value {
                    self.define_data_property(
                        *event,
                        PropertyKey::from("cancelBubble"),
                        Value::Bool(true),
                        true,
                        true,
                        true,
                    );
                }
                Ok(Value::Undefined)
            }
            BuiltinId::EventStopImmediatePropagation => {
                if let Value::Object(event) = &this_value {
                    self.define_data_property(
                        *event,
                        PropertyKey::from("cancelBubble"),
                        Value::Bool(true),
                        true,
                        true,
                        true,
                    );
                    self.define_data_property(
                        *event,
                        PropertyKey::from("__stopImmediate"),
                        Value::Bool(true),
                        false,
                        false,
                        true,
                    );
                }
                Ok(Value::Undefined)
            }
            BuiltinId::Fetch => self.builtin_fetch(&args),
            BuiltinId::ResponseText => self.response_text(&this_value),
            BuiltinId::ResponseJson => self.response_json(&this_value),
            BuiltinId::HeadersGet => self.headers_get(&this_value, &args),
            BuiltinId::MutationObserverConstructor => {
                self.mutation_observer_construct(args.first().cloned())
            }
            BuiltinId::MutationObserverObserve => {
                self.mutation_observer_observe(&this_value, &args)
            }
            BuiltinId::MutationObserverDisconnect => {
                self.mutation_observer_disconnect(&this_value)
            }
            BuiltinId::MutationObserverTakeRecords => {
                self.mutation_observer_take_records(&this_value)
            }
            BuiltinId::IntersectionObserverConstructor => {
                self.intersection_observer_construct(args.first().cloned())
            }
            BuiltinId::IntersectionObserverUnobserve => {
                if let (Some(id), Some(target)) = (
                    self.observer_id_from_this(&this_value),
                    self.node_id_from_host_val(args.first().unwrap_or(&Value::Undefined)),
                ) {
                    let _ = self.host.observer(ObserverOp::Unobserve {
                        observer: ObserverId(id),
                        target,
                    });
                }
                Ok(Value::Undefined)
            }
            BuiltinId::ResizeObserverConstructor => {
                self.resize_observer_construct(args.first().cloned())
            }
            BuiltinId::ResizeObserverUnobserve => {
                if let (Some(id), Some(target)) = (
                    self.observer_id_from_this(&this_value),
                    self.node_id_from_host_val(args.first().unwrap_or(&Value::Undefined)),
                ) {
                    let _ = self.host.observer(ObserverOp::Unobserve {
                        observer: ObserverId(id),
                        target,
                    });
                }
                Ok(Value::Undefined)
            }
            BuiltinId::ImageConstructor => self.image_construct(&args),
            BuiltinId::XhrConstructor => self.xhr_construct(),
            BuiltinId::XhrOpen => self.xhr_open(&this_value, &args),
            BuiltinId::XhrSetRequestHeader => self.xhr_set_request_header(&this_value, &args),
            BuiltinId::XhrSend => self.xhr_send(&this_value, &args),
            BuiltinId::XhrAbort => {
                if let Value::Object(o) = &this_value {
                    self.define_data_property(*o, PropertyKey::from("readyState"), Value::Number(0.0), true, true, true);
                }
                Ok(Value::Undefined)
            }
            BuiltinId::XhrGetAllResponseHeaders => self.xhr_get_all_response_headers(&this_value),
            BuiltinId::XhrGetResponseHeader => self.xhr_get_response_header(&this_value, &args),
            BuiltinId::ArrayIsArray => Ok(Value::Bool(matches!(
                args.first(),
                Some(Value::Object(object))
                    if self
                        .heap
                        .objects()
                        .get(*object)
                        .map(|object| object.kind == ObjectKind::Array)
                        .unwrap_or(false)
            ))),
            BuiltinId::ArrayFrom => {
                let source = args.first().cloned().unwrap_or(Value::Undefined);
                let map_fn = args.get(1).cloned();
                let this_arg = args.get(2).cloned().unwrap_or(Value::Undefined);
                let values = match &source {
                    Value::String(string) => self
                        .string_text(*string)
                        .chars()
                        .map(|character| self.make_string_value(&character.to_string()))
                        .collect(),
                    Value::Object(_) => self.for_of_values(&source)?,
                    Value::Null | Value::Undefined => {
                        return Err(VmError::TypeError(
                            "Array.from requires an array-like or iterable object".to_string(),
                        ));
                    }
                    _ => Vec::new(),
                };
                let values = match map_fn {
                    Some(callback) if self.is_callable_value(&callback) => {
                        let mut mapped = Vec::with_capacity(values.len());
                        for (index, value) in values.into_iter().enumerate() {
                            mapped.push(self.call_value_sync(
                                callback.clone(),
                                this_arg.clone(),
                                vec![value, Value::Number(index as f64)],
                            )?);
                        }
                        mapped
                    }
                    _ => values,
                };
                self.make_array_from_values(values)
            }
            BuiltinId::ArrayOf => self.make_array_from_values(args),
            BuiltinId::ArrayProtoPush => {
                let object = self.builtin_object_this(&this_value, "Array.prototype.push")?;
                let mut length = self.array_length(object);
                for value in args {
                    self.set_property_on_object(
                        object,
                        Value::Object(object),
                        PropertyKey::Index(length),
                        value,
                    )?;
                    length = length.saturating_add(1);
                }
                Ok(Value::Number(self.array_length(object) as f64))
            }
            BuiltinId::ArrayProtoPop => {
                let object = self.builtin_object_this(&this_value, "Array.prototype.pop")?;
                let length = self.array_length(object);
                if length == 0 {
                    return Ok(Value::Undefined);
                }
                let key = PropertyKey::Index(length - 1);
                let value = self.get_property_value(&Value::Object(object), &key)?;
                if let Some(object_data) = self.heap.objects_mut().get_mut(object) {
                    let _ = object_data.properties.shift_remove(&key);
                }
                self.set_array_length(object, length - 1);
                Ok(value)
            }
            BuiltinId::ArrayProtoShift => {
                let object = self.builtin_object_this(&this_value, "Array.prototype.shift")?;
                let length = self.array_length(object);
                if length == 0 {
                    return Ok(Value::Undefined);
                }
                let first =
                    self.get_property_value(&Value::Object(object), &PropertyKey::Index(0))?;
                for index in 1..length {
                    let value = self
                        .get_property_value(&Value::Object(object), &PropertyKey::Index(index))?;
                    self.set_property_on_object(
                        object,
                        Value::Object(object),
                        PropertyKey::Index(index - 1),
                        value,
                    )?;
                }
                if let Some(object_data) = self.heap.objects_mut().get_mut(object) {
                    let _ = object_data
                        .properties
                        .shift_remove(&PropertyKey::Index(length - 1));
                }
                self.set_array_length(object, length - 1);
                Ok(first)
            }
            BuiltinId::ArrayProtoUnshift => {
                let object = self.builtin_object_this(&this_value, "Array.prototype.unshift")?;
                let original = self.array_like_to_vec(&Value::Object(object))?;
                let mut values = args;
                values.extend(original);
                self.replace_array_contents(object, values)?;
                Ok(Value::Number(self.array_length(object) as f64))
            }
            BuiltinId::ArrayProtoMap => self.array_callback_map(&this_value, args),
            BuiltinId::ArrayProtoFilter => self.array_callback_filter(&this_value, args),
            BuiltinId::ArrayProtoReduce => self.array_callback_reduce(&this_value, args),
            BuiltinId::ArrayProtoForEach => self.array_callback_for_each(&this_value, args),
            BuiltinId::ArrayProtoFind => self.array_callback_find(&this_value, args, false),
            BuiltinId::ArrayProtoFindIndex => self.array_callback_find(&this_value, args, true),
            BuiltinId::ArrayProtoIndexOf => {
                let values = self.array_like_to_vec(&this_value)?;
                let needle = args.first().cloned().unwrap_or(Value::Undefined);
                let from = args
                    .get(1)
                    .map(|value| self.to_number(value) as isize)
                    .unwrap_or(0);
                let start = if from < 0 { 0 } else { from as usize };
                let index = values
                    .iter()
                    .enumerate()
                    .skip(start)
                    .find_map(|(index, value)| self.strict_equal(value, &needle).then_some(index))
                    .map(|index| index as f64)
                    .unwrap_or(-1.0);
                Ok(Value::Number(index))
            }
            BuiltinId::ArrayProtoLastIndexOf => {
                let values = self.array_like_to_vec(&this_value)?;
                let needle = args.first().cloned().unwrap_or(Value::Undefined);
                let index = values
                    .iter()
                    .enumerate()
                    .rev()
                    .find_map(|(index, value)| {
                        self.strict_equal(value, &needle).then_some(index)
                    })
                    .map(|index| index as f64)
                    .unwrap_or(-1.0);
                Ok(Value::Number(index))
            }
            BuiltinId::ArrayProtoIncludes => {
                let values = self.array_like_to_vec(&this_value)?;
                let needle = args.first().cloned().unwrap_or(Value::Undefined);
                Ok(Value::Bool(
                    values
                        .iter()
                        .any(|value| self.same_value_zero(value, &needle)),
                ))
            }
            BuiltinId::ArrayProtoJoin => {
                let values = self.array_like_to_vec(&this_value)?;
                let separator = args
                    .first()
                    .map(|value| self.to_string(value))
                    .unwrap_or_else(|| ",".to_string());
                let joined = values
                    .iter()
                    .map(|value| self.to_string(value))
                    .collect::<Vec<_>>()
                    .join(&separator);
                Ok(self.make_string_value(&joined))
            }
            BuiltinId::ArrayProtoSlice => {
                let values = self.array_like_to_vec(&this_value)?;
                let (start, end) =
                    self.normalize_slice_bounds(values.len(), args.first(), args.get(1));
                self.make_array_from_values(values[start..end].to_vec())
            }
            BuiltinId::ArrayProtoConcat => {
                let mut values = self.array_like_to_vec(&this_value)?;
                for argument in args {
                    match argument {
                        Value::Object(object)
                            if self
                                .heap
                                .objects()
                                .get(object)
                                .map(|object| object.kind == ObjectKind::Array)
                                .unwrap_or(false) =>
                        {
                            values.extend(self.array_like_to_vec(&Value::Object(object))?);
                        }
                        other => values.push(other),
                    }
                }
                self.make_array_from_values(values)
            }
            BuiltinId::ArrayProtoFlat => {
                let values = self.array_like_to_vec(&this_value)?;
                let depth = match args.first() {
                    Some(Value::Undefined) | None => 1,
                    Some(value) => {
                        let n = self.to_number(value);
                        if n.is_nan() || n < 0.0 { 0 } else { n as usize }
                    }
                };
                let flattened = self.flatten_values(values, depth)?;
                self.make_array_from_values(flattened)
            }
            BuiltinId::ArrayProtoSome => self.array_callback_predicate(&this_value, args, true),
            BuiltinId::ArrayProtoEvery => self.array_callback_predicate(&this_value, args, false),
            BuiltinId::ArrayProtoSort => {
                let object = self.builtin_object_this(&this_value, "Array.prototype.sort")?;
                let mut values = self.array_like_to_vec(&this_value)?;
                self.sort_values(&mut values, args.first())?;
                self.replace_array_contents(object, values)?;
                Ok(Value::Object(object))
            }
            BuiltinId::ArrayProtoReverse => {
                let object = self.builtin_object_this(&this_value, "Array.prototype.reverse")?;
                let mut values = self.array_like_to_vec(&this_value)?;
                values.reverse();
                self.replace_array_contents(object, values)?;
                Ok(Value::Object(object))
            }
            BuiltinId::ArrayProtoSplice => {
                let object = self.builtin_object_this(&this_value, "Array.prototype.splice")?;
                let mut values = self.array_like_to_vec(&this_value)?;
                let len = values.len();
                let start = relative_index(args.first().map(|v| self.to_number(v)), len);
                let delete_count = match args.get(1) {
                    None => len - start,
                    Some(value) => {
                        let n = self.to_number(value);
                        if n.is_nan() || n < 0.0 { 0 } else { (n as usize).min(len - start) }
                    }
                };
                let removed: Vec<Value> = values.splice(
                    start..start + delete_count,
                    args.iter().skip(2).cloned(),
                ).collect();
                self.replace_array_contents(object, values)?;
                self.make_array_from_values(removed)
            }
            BuiltinId::ArrayProtoFlatMap => {
                let mapped = self.array_callback_map(&this_value, args)?;
                let values = self.array_like_to_vec(&mapped)?;
                let flattened = self.flatten_values(values, 1)?;
                self.make_array_from_values(flattened)
            }
            BuiltinId::ArrayProtoFill => {
                let object = self.builtin_object_this(&this_value, "Array.prototype.fill")?;
                let mut values = self.array_like_to_vec(&this_value)?;
                let len = values.len();
                let fill_value = args.first().cloned().unwrap_or(Value::Undefined);
                let start = relative_index(args.get(1).map(|v| self.to_number(v)), len);
                let end = match args.get(2) {
                    None | Some(Value::Undefined) => len,
                    Some(value) => relative_index(Some(self.to_number(value)), len),
                };
                for slot in values.iter_mut().take(end).skip(start) {
                    *slot = fill_value.clone();
                }
                self.replace_array_contents(object, values)?;
                Ok(Value::Object(object))
            }
            BuiltinId::ArrayProtoCopyWithin => {
                let object = self.builtin_object_this(&this_value, "Array.prototype.copyWithin")?;
                let mut values = self.array_like_to_vec(&this_value)?;
                let len = values.len();
                let target = relative_index(args.first().map(|v| self.to_number(v)), len);
                let start = relative_index(args.get(1).map(|v| self.to_number(v)), len);
                let end = match args.get(2) {
                    None | Some(Value::Undefined) => len,
                    Some(value) => relative_index(Some(self.to_number(value)), len),
                };
                let slice: Vec<Value> = values[start.min(len)..end.min(len).max(start.min(len))].to_vec();
                for (offset, value) in slice.into_iter().enumerate() {
                    if target + offset >= len {
                        break;
                    }
                    values[target + offset] = value;
                }
                self.replace_array_contents(object, values)?;
                Ok(Value::Object(object))
            }
            BuiltinId::ArrayProtoAt => {
                let values = self.array_like_to_vec(&this_value)?;
                let len = values.len() as i64;
                let mut index = self.number_arg(&args, 0) as i64;
                if index < 0 {
                    index += len;
                }
                if index < 0 || index >= len {
                    Ok(Value::Undefined)
                } else {
                    Ok(values[index as usize].clone())
                }
            }
            BuiltinId::ArrayProtoKeys => {
                let values = self.array_like_to_vec(&this_value)?;
                let keys = (0..values.len()).map(|i| Value::Number(i as f64)).collect();
                Ok(self.make_for_of_iterator(keys))
            }
            BuiltinId::ArrayProtoValues => {
                let values = self.array_like_to_vec(&this_value)?;
                Ok(self.make_for_of_iterator(values))
            }
            BuiltinId::ArrayProtoEntries => {
                let values = self.array_like_to_vec(&this_value)?;
                let mut entries = Vec::with_capacity(values.len());
                for (index, value) in values.into_iter().enumerate() {
                    entries.push(self.make_array_from_values(vec![Value::Number(index as f64), value])?);
                }
                Ok(self.make_for_of_iterator(entries))
            }
            BuiltinId::ArrayProtoReduceRight => {
                let mut values = self.array_like_to_vec(&this_value)?;
                values.reverse();
                let callback = args.first().cloned().unwrap_or(Value::Undefined);
                if !self.is_callable_value(&callback) {
                    return Err(VmError::TypeError(
                        "Array.prototype.reduceRight requires a callback".to_string(),
                    ));
                }
                let len = values.len();
                let (mut acc, start) = match args.get(1) {
                    Some(initial) => (initial.clone(), 0),
                    None => {
                        if values.is_empty() {
                            return Err(VmError::TypeError(
                                "Reduce of empty array with no initial value".to_string(),
                            ));
                        }
                        (values[0].clone(), 1)
                    }
                };
                for i in start..len {
                    let index_from_right = len - 1 - i;
                    acc = self.call_value_sync(
                        callback.clone(),
                        Value::Undefined,
                        vec![
                            acc,
                            values[i].clone(),
                            Value::Number(index_from_right as f64),
                            this_value.clone(),
                        ],
                    )?;
                }
                Ok(acc)
            }
            BuiltinId::ArrayProtoFindLast | BuiltinId::ArrayProtoFindLastIndex => {
                let return_index = matches!(builtin, BuiltinId::ArrayProtoFindLastIndex);
                let values = self.array_like_to_vec(&this_value)?;
                let callback = args.first().cloned().unwrap_or(Value::Undefined);
                for i in (0..values.len()).rev() {
                    let matched = self.call_value_sync(
                        callback.clone(),
                        Value::Undefined,
                        vec![values[i].clone(), Value::Number(i as f64), this_value.clone()],
                    )?;
                    if self.is_truthy(&matched) {
                        return Ok(if return_index {
                            Value::Number(i as f64)
                        } else {
                            values[i].clone()
                        });
                    }
                }
                Ok(if return_index { Value::Number(-1.0) } else { Value::Undefined })
            }
            BuiltinId::StringProtoAt => {
                let text = self.builtin_string_this(&this_value)?;
                let chars: Vec<char> = text.chars().collect();
                let len = chars.len() as i64;
                let mut index = self.number_arg(&args, 0) as i64;
                if index < 0 {
                    index += len;
                }
                if index < 0 || index >= len {
                    Ok(Value::Undefined)
                } else {
                    Ok(self.make_string_value(&chars[index as usize].to_string()))
                }
            }
            BuiltinId::StringProtoNormalize => {
                let text = self.builtin_string_this(&this_value)?;
                Ok(self.make_string_value(&text))
            }
            BuiltinId::StringProtoConcat => {
                let mut text = self.builtin_string_this(&this_value)?;
                for value in &args {
                    text.push_str(&self.to_string(value));
                }
                Ok(self.make_string_value(&text))
            }
            BuiltinId::StringFromCharCode => {
                let mut text = String::with_capacity(args.len());
                for value in &args {
                    let code = self.to_number(value) as u32 & 0xFFFF;
                    if let Some(ch) = char::from_u32(code) {
                        text.push(ch);
                    }
                }
                Ok(self.make_string_value(&text))
            }
            BuiltinId::StringFromCodePoint => {
                let mut text = String::with_capacity(args.len());
                for value in &args {
                    let code = self.to_number(value) as u32;
                    if let Some(ch) = char::from_u32(code) {
                        text.push(ch);
                    }
                }
                Ok(self.make_string_value(&text))
            }
            BuiltinId::StringRaw => {
                let strings = args.first().cloned().unwrap_or(Value::Undefined);
                let raw = self.get_property_value(&strings, &PropertyKey::from("raw"))?;
                let raw_parts = self.array_like_to_vec(&raw)?;
                let mut out = String::new();
                for (index, part) in raw_parts.iter().enumerate() {
                    out.push_str(&self.to_string(part));
                    // Interleave the substitution that follows this part, if any.
                    if let Some(substitution) = args.get(index + 1) {
                        out.push_str(&self.to_string(substitution));
                    }
                }
                Ok(self.make_string_value(&out))
            }
            BuiltinId::NumberConstructor => {
                let number = match args.first() {
                    None => 0.0,
                    Some(value) => self.to_number(value),
                };
                Ok(Value::Number(number))
            }
            BuiltinId::StringConstructor => {
                let text = match args.first() {
                    None => String::new(),
                    Some(value) => self.to_string(value),
                };
                Ok(self.make_string_value(&text))
            }
            BuiltinId::BooleanConstructor => {
                let value = args.first().cloned().unwrap_or(Value::Undefined);
                Ok(Value::Bool(self.is_truthy(&value)))
            }
            BuiltinId::NumberProtoValueOf => Ok(Value::Number(self.to_number(&this_value))),
            BuiltinId::NumberProtoToFixed => {
                let number = self.to_number(&this_value);
                let digits = self.number_arg(&args, 0);
                let digits = if digits.is_nan() { 0 } else { (digits as usize).min(100) };
                Ok(self.make_string_value(&format!("{number:.digits$}")))
            }
            BuiltinId::NumberProtoToPrecision => {
                let number = self.to_number(&this_value);
                match args.first() {
                    None | Some(Value::Undefined) => {
                        Ok(self.make_string_value(&Self::format_number(number)))
                    }
                    Some(value) => {
                        let precision = (self.to_number(value) as usize).clamp(1, 100);
                        Ok(self.make_string_value(&number_to_precision(number, precision)))
                    }
                }
            }
            BuiltinId::NumberProtoToString => {
                let number = self.to_number(&this_value);
                let radix = match args.first() {
                    None | Some(Value::Undefined) => 10,
                    Some(value) => self.to_number(value) as u32,
                };
                if radix == 10 {
                    Ok(self.make_string_value(&Self::format_number(number)))
                } else {
                    Ok(self.make_string_value(&number_to_radix_string(number, radix)))
                }
            }
            BuiltinId::BooleanProtoValueOf => Ok(Value::Bool(self.is_truthy(&this_value))),
            BuiltinId::BooleanProtoToString => {
                let text = if self.is_truthy(&this_value) { "true" } else { "false" };
                Ok(self.make_string_value(text))
            }
            BuiltinId::ObjectFromEntries => {
                let iterable = args.first().cloned().unwrap_or(Value::Undefined);
                let entries = self.for_of_values(&iterable)?;
                let object = self.allocate_ordinary_object(Some(self.object_prototype_ref()));
                for entry in entries {
                    let key = self.get_property_value(&entry, &PropertyKey::Index(0))?;
                    let value = self.get_property_value(&entry, &PropertyKey::Index(1))?;
                    let key = self.to_property_key(&key)?;
                    self.set_property_on_object(object, Value::Object(object), key, value)?;
                }
                Ok(Value::Object(object))
            }
            BuiltinId::ObjectGetOwnPropertyNames => {
                self.object_introspection_get_own_property_names(
                    args.first().unwrap_or(&Value::Undefined),
                    "Object.getOwnPropertyNames",
                )
            }
            BuiltinId::ObjectHasOwn => {
                let target = args.first().cloned().unwrap_or(Value::Undefined);
                let object = self.require_object_ref(&target, "Object.hasOwn")?;
                let key = self.to_property_key(args.get(1).unwrap_or(&Value::Undefined))?;
                Ok(Value::Bool(
                    self.get_own_property_descriptor(object, &key).is_some(),
                ))
            }
            BuiltinId::ObjectPreventExtensions => {
                let target = args.first().cloned().unwrap_or(Value::Undefined);
                if let Value::Object(object) = target {
                    if let Some(object_data) = self.heap.objects_mut().get_mut(object) {
                        object_data.extensible = false;
                    }
                }
                Ok(target)
            }
            BuiltinId::ObjectIsExtensible => {
                match args.first() {
                    Some(Value::Object(object)) => Ok(Value::Bool(
                        self.heap
                            .objects()
                            .get(*object)
                            .map(|o| o.extensible)
                            .unwrap_or(false),
                    )),
                    _ => Ok(Value::Bool(false)),
                }
            }
            BuiltinId::ObjectSeal => {
                let target = args.first().cloned().unwrap_or(Value::Undefined);
                if let Value::Object(object) = target {
                    if let Some(object_data) = self.heap.objects_mut().get_mut(object) {
                        object_data.extensible = false;
                    }
                }
                Ok(target)
            }
            BuiltinId::ObjectIsSealed => {
                match args.first() {
                    Some(Value::Object(object)) => Ok(Value::Bool(
                        self.heap
                            .objects()
                            .get(*object)
                            .map(|o| !o.extensible)
                            .unwrap_or(true),
                    )),
                    _ => Ok(Value::Bool(true)),
                }
            }
            BuiltinId::MathSign => {
                let number = self.number_arg(&args, 0);
                let result = if number.is_nan() {
                    f64::NAN
                } else if number > 0.0 {
                    1.0
                } else if number < 0.0 {
                    -1.0
                } else {
                    number // preserves +0 / -0
                };
                Ok(Value::Number(result))
            }
            BuiltinId::MathHypot => {
                let sum: f64 = args.iter().map(|value| {
                    let n = self.to_number(value);
                    n * n
                }).sum();
                Ok(Value::Number(sum.sqrt()))
            }
            BuiltinId::MathClz32 => {
                let number = self.number_arg(&args, 0);
                let int = if number.is_finite() { number as i64 as u32 } else { 0 };
                Ok(Value::Number(int.leading_zeros() as f64))
            }
            BuiltinId::RegExpConstructor => {
                let (source, flags) = match args.first() {
                    Some(value) if self.regexp_source_flags(value).is_some() => {
                        let (source, existing_flags) = self.regexp_source_flags(value).unwrap();
                        let flags = match args.get(1) {
                            Some(Value::Undefined) | None => existing_flags,
                            Some(flag_value) => self.to_string(flag_value),
                        };
                        (source, flags)
                    }
                    Some(value) => {
                        let source = self.to_string(value);
                        let flags = args.get(1).map(|v| self.to_string(v)).unwrap_or_default();
                        (source, flags)
                    }
                    None => (String::new(), String::new()),
                };
                // Validate the pattern eagerly so a bad regex throws at construction.
                compile_js_regex(&source, &flags)?;
                Ok(self.make_regexp_object(&source, &flags))
            }
            BuiltinId::RegExpProtoToString => {
                let (source, flags) = self
                    .regexp_source_flags(&this_value)
                    .ok_or_else(|| VmError::TypeError("Method called on non-RegExp".to_string()))?;
                Ok(self.make_string_value(&format!("/{source}/{flags}")))
            }
            BuiltinId::RegExpProtoTest => {
                let (source, flags) = self
                    .regexp_source_flags(&this_value)
                    .ok_or_else(|| VmError::TypeError("Method called on non-RegExp".to_string()))?;
                let regex = compile_js_regex(&source, &flags)?;
                let text = self.string_arg(&args, 0);
                let sticky_or_global = flags.contains('g') || flags.contains('y');
                if sticky_or_global {
                    let object = self.require_object_ref(&this_value, "RegExp.prototype.test")?;
                    let start = self.regexp_last_index(object).min(text.len());
                    match regex.find_at(&text, start) {
                        Some(m) => {
                            self.set_regexp_last_index(object, m.end());
                            Ok(Value::Bool(true))
                        }
                        None => {
                            self.set_regexp_last_index(object, 0);
                            Ok(Value::Bool(false))
                        }
                    }
                } else {
                    Ok(Value::Bool(regex.is_match(&text)))
                }
            }
            BuiltinId::RegExpProtoExec => {
                let (source, flags) = self
                    .regexp_source_flags(&this_value)
                    .ok_or_else(|| VmError::TypeError("Method called on non-RegExp".to_string()))?;
                let regex = compile_js_regex(&source, &flags)?;
                let text = self.string_arg(&args, 0);
                let sticky_or_global = flags.contains('g') || flags.contains('y');
                let start = if sticky_or_global {
                    let object = self.require_object_ref(&this_value, "RegExp.prototype.exec")?;
                    self.regexp_last_index(object).min(text.len())
                } else {
                    0
                };
                match regex.captures_at(&text, start) {
                    Some(caps) => {
                        if sticky_or_global {
                            let end = caps.get(0).map(|m| m.end()).unwrap_or(start);
                            let object =
                                self.require_object_ref(&this_value, "RegExp.prototype.exec")?;
                            self.set_regexp_last_index(object, end);
                        }
                        self.build_match_result(&caps, &text)
                    }
                    None => {
                        if sticky_or_global {
                            let object =
                                self.require_object_ref(&this_value, "RegExp.prototype.exec")?;
                            self.set_regexp_last_index(object, 0);
                        }
                        Ok(Value::Null)
                    }
                }
            }
            BuiltinId::StringProtoSearch => {
                let text = self.builtin_string_this(&this_value)?;
                let (source, flags) = self.coerce_regex_arg(args.first())?;
                let regex = compile_js_regex(&source, &flags)?;
                match regex.find(&text) {
                    Some(m) => Ok(Value::Number(text[..m.start()].chars().count() as f64)),
                    None => Ok(Value::Number(-1.0)),
                }
            }
            BuiltinId::StringProtoMatch => {
                let text = self.builtin_string_this(&this_value)?;
                let (source, flags) = self.coerce_regex_arg(args.first())?;
                let regex = compile_js_regex(&source, &flags)?;
                if flags.contains('g') {
                    let matches: Vec<Value> = regex
                        .find_iter(&text)
                        .into_iter()
                        .map(|m| m.as_str().to_string())
                        .collect::<Vec<_>>()
                        .into_iter()
                        .map(|s| self.make_string_value(&s))
                        .collect();
                    if matches.is_empty() {
                        Ok(Value::Null)
                    } else {
                        self.make_array_from_values(matches)
                    }
                } else {
                    match regex.captures(&text) {
                        Some(caps) => self.build_match_result(&caps, &text),
                        None => Ok(Value::Null),
                    }
                }
            }
            BuiltinId::StringProtoMatchAll => {
                let text = self.builtin_string_this(&this_value)?;
                let (source, flags) = self.coerce_regex_arg(args.first())?;
                let regex = compile_js_regex(&source, &flags)?;
                let captures: Vec<JsCaptures> = regex.captures_iter(&text);
                let mut results = Vec::with_capacity(captures.len());
                for caps in &captures {
                    results.push(self.build_match_result(caps, &text)?);
                }
                Ok(self.make_for_of_iterator(results))
            }
            BuiltinId::SymbolConstructor => {
                let description = match args.first() {
                    Some(Value::Undefined) | None => None,
                    Some(value) => Some(self.to_string(value)),
                };
                Ok(self.allocate_symbol(description))
            }
            BuiltinId::SymbolProtoToString => {
                let description = match &this_value {
                    Value::Symbol(SymbolId(id)) => {
                        self.symbol_descriptions.get(id).cloned().unwrap_or_default()
                    }
                    _ => String::new(),
                };
                Ok(self.make_string_value(&format!("Symbol({description})")))
            }
            BuiltinId::DateNow => Ok(Value::Number(self.current_time_ms())),
            BuiltinId::DateUTC => {
                if args.len() < 2 {
                    return Ok(Value::Number(f64::NAN));
                }
                let year_number = self.to_number(args.first().unwrap_or(&Value::Undefined));
                let month_number = self.to_number(args.get(1).unwrap_or(&Value::Undefined));
                if !year_number.is_finite() || !month_number.is_finite() {
                    return Ok(Value::Number(f64::NAN));
                }
                let mut year = year_number as i64;
                let month = month_number as i64;
                if (0..=99).contains(&year) {
                    year += 1900;
                }
                let (year, month0) = normalize_utc_month(year, month);
                let day = if args.len() > 2 {
                    let value = self.to_number(args.get(2).unwrap_or(&Value::Undefined));
                    if !value.is_finite() {
                        return Ok(Value::Number(f64::NAN));
                    }
                    value as i64
                } else {
                    1
                };
                let hours = if args.len() > 3 {
                    let value = self.to_number(args.get(3).unwrap_or(&Value::Undefined));
                    if !value.is_finite() {
                        return Ok(Value::Number(f64::NAN));
                    }
                    value as i64
                } else {
                    0
                };
                let minutes = if args.len() > 4 {
                    let value = self.to_number(args.get(4).unwrap_or(&Value::Undefined));
                    if !value.is_finite() {
                        return Ok(Value::Number(f64::NAN));
                    }
                    value as i64
                } else {
                    0
                };
                let seconds = if args.len() > 5 {
                    let value = self.to_number(args.get(5).unwrap_or(&Value::Undefined));
                    if !value.is_finite() {
                        return Ok(Value::Number(f64::NAN));
                    }
                    value as i64
                } else {
                    0
                };
                let millis = if args.len() > 6 {
                    let value = self.to_number(args.get(6).unwrap_or(&Value::Undefined));
                    if !value.is_finite() {
                        return Ok(Value::Number(f64::NAN));
                    }
                    value as i64
                } else {
                    0
                };
                let days = days_from_civil(year, month0 + 1, day);
                Ok(Value::Number(
                    (days * 86_400_000
                        + hours * 3_600_000
                        + minutes * 60_000
                        + seconds * 1000
                        + millis) as f64,
                ))
            }
            BuiltinId::DateParse => {
                let text = args.first().map(|value| self.to_string(value)).unwrap_or_default();
                Ok(Value::Number(self.parse_date_utc_ms(&text)))
            }
            BuiltinId::DateConstructor => {
                let time = match args.len() {
                    0 => self.current_time_ms(),
                    1 => match &args[0] {
                        // String date parsing is not supported yet.
                        Value::String(_) => f64::NAN,
                        other => self.to_number(other),
                    },
                    _ => {
                        let year = self.number_arg(&args, 0) as i64;
                        let month = self.number_arg(&args, 1) as i64;
                        let day = if args.len() > 2 { self.number_arg(&args, 2) as i64 } else { 1 };
                        let hours = if args.len() > 3 { self.number_arg(&args, 3) as i64 } else { 0 };
                        let minutes =
                            if args.len() > 4 { self.number_arg(&args, 4) as i64 } else { 0 };
                        let seconds =
                            if args.len() > 5 { self.number_arg(&args, 5) as i64 } else { 0 };
                        let millis =
                            if args.len() > 6 { self.number_arg(&args, 6) as i64 } else { 0 };
                        let days = days_from_civil(year, month + 1, day);
                        (days * 86_400_000
                            + hours * 3_600_000
                            + minutes * 60_000
                            + seconds * 1000
                            + millis) as f64
                    }
                };
                Ok(self.make_date_object(time))
            }
            BuiltinId::DateProtoGetTime | BuiltinId::DateProtoValueOf => {
                Ok(Value::Number(self.date_time_value(&this_value)?))
            }
            BuiltinId::DateProtoGetFullYear => self.date_component(&this_value, 0),
            BuiltinId::DateProtoGetMonth => self.date_component(&this_value, 1),
            BuiltinId::DateProtoGetDate => self.date_component(&this_value, 2),
            BuiltinId::DateProtoGetHours => self.date_component(&this_value, 3),
            BuiltinId::DateProtoGetMinutes => self.date_component(&this_value, 4),
            BuiltinId::DateProtoGetSeconds => self.date_component(&this_value, 5),
            BuiltinId::DateProtoGetMilliseconds => self.date_component(&this_value, 6),
            BuiltinId::DateProtoGetDay => self.date_component(&this_value, 7),
            BuiltinId::DateProtoGetTimezoneOffset => Ok(Value::Number(0.0)),
            BuiltinId::DateProtoToISOString | BuiltinId::DateProtoToString => {
                let time = self.date_time_value(&this_value)?;
                match Self::date_components(time) {
                    Some((year, month0, day, hours, minutes, seconds, millis, _)) => {
                        let iso = format!(
                            "{year:04}-{:02}-{day:02}T{hours:02}:{minutes:02}:{seconds:02}.{millis:03}Z",
                            month0 + 1
                        );
                        Ok(self.make_string_value(&iso))
                    }
                    None => Ok(self.make_string_value("Invalid Date")),
                }
            }
            BuiltinId::GeneratorProtoNext => {
                let generator = self.require_object_ref(&this_value, "Generator.prototype.next")?;
                let sent = args.first().cloned().unwrap_or(Value::Undefined);
                self.generator_resume(generator, sent)
            }
            BuiltinId::GeneratorProtoReturn => {
                let generator =
                    self.require_object_ref(&this_value, "Generator.prototype.return")?;
                let value = args.first().cloned().unwrap_or(Value::Undefined);
                self.set_generator_state(generator, GeneratorState::Completed);
                self.make_iter_result(value, true)
            }
            BuiltinId::GeneratorProtoIterator => Ok(this_value),
            BuiltinId::AsyncGeneratorProtoNext => {
                let generator =
                    self.require_object_ref(&this_value, "AsyncGenerator.prototype.next")?;
                self.async_generator_resume(
                    generator,
                    args.first().cloned().unwrap_or(Value::Undefined),
                    false,
                )
            }
            BuiltinId::AsyncGeneratorProtoReturn => {
                let generator =
                    self.require_object_ref(&this_value, "AsyncGenerator.prototype.return")?;
                self.async_generator_resume(
                    generator,
                    args.first().cloned().unwrap_or(Value::Undefined),
                    true,
                )
            }
            BuiltinId::AsyncGeneratorProtoIterator => Ok(this_value),
            BuiltinId::ForOfIteratorAdapterNext => {
                let iterator =
                    self.require_object_ref(&this_value, "ForOfIteratorAdapter.next")?;
                match self.for_of_next(iterator)? {
                    Some(value) => self.make_iter_result(value, false),
                    None => self.make_iter_result(Value::Undefined, true),
                }
            }
            BuiltinId::ArrayProtoToSorted => {
                let mut values = self.array_like_to_vec(&this_value)?;
                self.sort_values(&mut values, args.first())?;
                self.make_array_from_values(values)
            }
            BuiltinId::ArrayProtoToReversed => {
                let mut values = self.array_like_to_vec(&this_value)?;
                values.reverse();
                self.make_array_from_values(values)
            }
            BuiltinId::ArrayProtoWith => {
                let mut values = self.array_like_to_vec(&this_value)?;
                let len = values.len() as i64;
                let mut index = self.number_arg(&args, 0) as i64;
                if index < 0 {
                    index += len;
                }
                if index < 0 || index >= len {
                    return Err(VmError::RangeError("Invalid index".to_string()));
                }
                values[index as usize] = args.get(1).cloned().unwrap_or(Value::Undefined);
                self.make_array_from_values(values)
            }
            BuiltinId::StringProtoLocaleCompare => {
                let text = self.builtin_string_this(&this_value)?;
                let other = self.string_arg(&args, 0);
                let result = match text.cmp(&other) {
                    std::cmp::Ordering::Less => -1.0,
                    std::cmp::Ordering::Equal => 0.0,
                    std::cmp::Ordering::Greater => 1.0,
                };
                Ok(Value::Number(result))
            }
            BuiltinId::ObjectIs => {
                let a = args.first().cloned().unwrap_or(Value::Undefined);
                let b = args.get(1).cloned().unwrap_or(Value::Undefined);
                Ok(Value::Bool(self.same_value(&a, &b)))
            }
            BuiltinId::ObjectGetOwnPropertyDescriptors => {
                let object = self.require_object_ref(
                    args.first().unwrap_or(&Value::Undefined),
                    "Object.getOwnPropertyDescriptors",
                )?;
                let result = self.allocate_ordinary_object(Some(self.object_prototype_ref()));
                let keys: Vec<PropertyKey> = self
                    .heap
                    .objects()
                    .get(object)
                    .map(|o| o.properties.keys().cloned().collect())
                    .unwrap_or_default();
                for key in keys {
                    if let Some(descriptor) = self.get_own_property_descriptor(object, &key) {
                        let descriptor_value = self.property_descriptor_to_value(descriptor)?;
                        self.set_property_on_object(
                            result,
                            Value::Object(result),
                            key,
                            descriptor_value,
                        )?;
                    }
                }
                Ok(Value::Object(result))
            }
            BuiltinId::ObjectDefineProperties => {
                let object = self.require_object_ref(
                    args.first().unwrap_or(&Value::Undefined),
                    "Object.defineProperties",
                )?;
                if let Some(Value::Object(props)) = args.get(1) {
                    let props = *props;
                    for key in self.object_own_enumerable_keys(props) {
                        let descriptor_value =
                            self.get_property_value(&Value::Object(props), &key)?;
                        let descriptor = self.value_to_property_descriptor(&descriptor_value)?;
                        if let Some(object_data) = self.heap.objects_mut().get_mut(object) {
                            object_data.properties.insert(key.clone(), descriptor);
                        }
                        self.update_array_length_for_key(object, &key)?;
                    }
                }
                Ok(Value::Object(object))
            }
            BuiltinId::NumberProtoToLocaleString => {
                let number = self.to_number(&this_value);
                Ok(self.make_string_value(&Self::format_number(number)))
            }
            BuiltinId::SymbolFor => {
                let key = self.string_arg(&args, 0);
                let id = match self.symbol_registry.get(&key) {
                    Some(id) => *id,
                    None => {
                        let id = self.next_symbol_id;
                        self.next_symbol_id = self.next_symbol_id.saturating_add(1);
                        self.symbol_registry.insert(key.clone(), id);
                        self.symbol_descriptions.insert(id, key);
                        id
                    }
                };
                Ok(Value::Symbol(SymbolId(id)))
            }
            BuiltinId::SymbolKeyFor => {
                let found = match args.first() {
                    Some(Value::Symbol(SymbolId(id))) => self
                        .symbol_registry
                        .iter()
                        .find(|(_, registered)| *registered == id)
                        .map(|(key, _)| key.clone()),
                    _ => None,
                };
                match found {
                    Some(key) => Ok(self.make_string_value(&key)),
                    None => Ok(Value::Undefined),
                }
            }
            BuiltinId::ReflectGet => {
                let target = args.first().cloned().unwrap_or(Value::Undefined);
                let key = self.to_property_key(args.get(1).unwrap_or(&Value::Undefined))?;
                self.get_property_value(&target, &key)
            }
            BuiltinId::ReflectSet => {
                let target = args.first().cloned().unwrap_or(Value::Undefined);
                let key = self.to_property_key(args.get(1).unwrap_or(&Value::Undefined))?;
                let value = args.get(2).cloned().unwrap_or(Value::Undefined);
                self.set_property_value(&target, key, value)?;
                Ok(Value::Bool(true))
            }
            BuiltinId::ReflectHas => {
                let target =
                    self.require_object_ref(args.first().unwrap_or(&Value::Undefined), "Reflect.has")?;
                let key = self.to_property_key(args.get(1).unwrap_or(&Value::Undefined))?;
                Ok(Value::Bool(
                    self.lookup_property_descriptor(target, &key).is_some(),
                ))
            }
            BuiltinId::ReflectDeleteProperty => {
                let target = self.require_object_ref(
                    args.first().unwrap_or(&Value::Undefined),
                    "Reflect.deleteProperty",
                )?;
                let key = self.to_property_key(args.get(1).unwrap_or(&Value::Undefined))?;
                Ok(Value::Bool(self.delete_property(target, &key)))
            }
            BuiltinId::ReflectOwnKeys => {
                let target = self.require_object_ref(
                    args.first().unwrap_or(&Value::Undefined),
                    "Reflect.ownKeys",
                )?;
                let keys: Vec<PropertyKey> = self
                    .heap
                    .objects()
                    .get(target)
                    .map(|o| o.properties.keys().cloned().collect())
                    .unwrap_or_default();
                let values = keys
                    .into_iter()
                    .filter(|key| !matches!(key, PropertyKey::Symbol(_)))
                    .map(|key| self.make_string_value(&self.property_key_to_string(&key)))
                    .collect();
                self.make_array_from_values(values)
            }
            BuiltinId::ReflectGetPrototypeOf => {
                let target = self.require_object_ref(
                    args.first().unwrap_or(&Value::Undefined),
                    "Reflect.getPrototypeOf",
                )?;
                match self.heap.objects().get(target).and_then(|o| o.prototype) {
                    Some(prototype) => Ok(Value::Object(prototype)),
                    None => Ok(Value::Null),
                }
            }
            BuiltinId::ReflectDefineProperty => {
                let object = self.require_object_ref(
                    args.first().unwrap_or(&Value::Undefined),
                    "Reflect.defineProperty",
                )?;
                let name = self.to_property_key(args.get(1).unwrap_or(&Value::Undefined))?;
                let descriptor =
                    self.value_to_property_descriptor(args.get(2).unwrap_or(&Value::Undefined))?;
                if let Some(object_data) = self.heap.objects_mut().get_mut(object) {
                    object_data.properties.insert(name.clone(), descriptor);
                }
                self.update_array_length_for_key(object, &name)?;
                Ok(Value::Bool(true))
            }
            BuiltinId::ReflectApply => {
                let target = args.first().cloned().unwrap_or(Value::Undefined);
                let this_arg = args.get(1).cloned().unwrap_or(Value::Undefined);
                let arg_list = match args.get(2) {
                    Some(value) if !matches!(value, Value::Null | Value::Undefined) => {
                        self.array_like_to_vec(value)?
                    }
                    _ => Vec::new(),
                };
                self.call_value_sync(target, this_arg, arg_list)
            }
            BuiltinId::ReflectConstruct => {
                let target = args.first().cloned().unwrap_or(Value::Undefined);
                let arg_list = match args.get(1) {
                    Some(value) if !matches!(value, Value::Null | Value::Undefined) => {
                        self.array_like_to_vec(value)?
                    }
                    _ => Vec::new(),
                };
                self.construct_value_sync(target, arg_list)
            }
            BuiltinId::StructuredClone => {
                let value = args.first().cloned().unwrap_or(Value::Undefined);
                self.structured_clone(&value, 0)
            }
            BuiltinId::UrlSearchParamsConstructor => {
                let pairs = match args.first() {
                    None | Some(Value::Undefined) | Some(Value::Null) => Vec::new(),
                    Some(Value::String(string)) => parse_query_string(&self.string_text(*string)),
                    Some(value @ Value::Object(object)) => {
                        let object = *object;
                        let kind_is_usp = matches!(
                            self.heap.objects().get(object).map(|o| &o.kind),
                            Some(ObjectKind::UrlSearchParams(_))
                        );
                        let is_array = matches!(
                            self.heap.objects().get(object).map(|o| &o.kind),
                            Some(ObjectKind::Array)
                        );
                        if kind_is_usp {
                            self.usp_pairs(value)?
                        } else if is_array {
                            // Sequence of [name, value] pairs.
                            let entries = self.for_of_values(value)?;
                            let mut pairs = Vec::with_capacity(entries.len());
                            for entry in entries {
                                let name =
                                    self.get_property_value(&entry, &PropertyKey::Index(0))?;
                                let val =
                                    self.get_property_value(&entry, &PropertyKey::Index(1))?;
                                pairs.push((self.to_string(&name), self.to_string(&val)));
                            }
                            pairs
                        } else {
                            // Record of name -> value.
                            let mut pairs = Vec::new();
                            for key in self.object_own_enumerable_keys(object) {
                                let val = self.get_property_value(value, &key)?;
                                pairs.push((
                                    self.property_key_to_string(&key),
                                    self.to_string(&val),
                                ));
                            }
                            pairs
                        }
                    }
                    Some(other) => parse_query_string(&self.to_string(other)),
                };
                let object = self.heap.allocate_object(JsObject {
                    kind: ObjectKind::UrlSearchParams(pairs),
                    prototype: Some(self.url_search_params_prototype_ref()),
                    ..JsObject::default()
                });
                Ok(Value::Object(object))
            }
            BuiltinId::HeadersConstructor => {
                let pairs = self.headers_init_pairs(args.first())?;
                let object = self.heap.allocate_object(JsObject {
                    kind: ObjectKind::Headers(pairs),
                    prototype: Some(self.headers_prototype_ref()),
                    ..JsObject::default()
                });
                Ok(Value::Object(object))
            }
            BuiltinId::FormDataConstructor => {
                let object = self.heap.allocate_object(JsObject {
                    kind: ObjectKind::FormData(Vec::new()),
                    prototype: Some(self.form_data_prototype_ref()),
                    ..JsObject::default()
                });
                Ok(Value::Object(object))
            }
            BuiltinId::UrlConstructor => {
                let input = args
                    .first()
                    .map(|value| self.to_string(value))
                    .unwrap_or_default();
                let base = args.get(1).map(|value| self.to_string(value));
                let components = parse_whatwg_url(&input, base.as_deref())
                    .ok_or_else(|| VmError::TypeError("Invalid URL".to_string()))?;
                let object = self.allocate_ordinary_object(Some(self.url_prototype_ref()));
                let UrlComponents {
                    href,
                    protocol,
                    username,
                    password,
                    host,
                    hostname,
                    port,
                    pathname,
                    search,
                    hash,
                    origin,
                } = components;
                for (name, value) in [
                    ("href", href),
                    ("protocol", protocol),
                    ("username", username),
                    ("password", password),
                    ("host", host),
                    ("hostname", hostname),
                    ("port", port),
                    ("pathname", pathname),
                    ("search", search),
                    ("hash", hash),
                    ("origin", origin),
                ] {
                    let value = self.make_string_value(&value);
                    self.define_data_property(
                        object,
                        PropertyKey::from(name),
                        value,
                        true,
                        true,
                        true,
                    );
                }
                let search_text = self
                    .get_property_value(&Value::Object(object), &PropertyKey::from("search"))?;
                let search_pairs = parse_query_string(&self.to_string(&search_text));
                let search_params = self.heap.allocate_object(JsObject {
                    kind: ObjectKind::UrlSearchParams(search_pairs),
                    prototype: Some(self.url_search_params_prototype_ref()),
                    ..JsObject::default()
                });
                self.define_data_property(
                    object,
                    PropertyKey::from("searchParams"),
                    Value::Object(search_params),
                    true,
                    true,
                    true,
                );
                Ok(Value::Object(object))
            }
            BuiltinId::UrlToString => {
                let href = self
                    .get_property_value(&this_value, &PropertyKey::from("href"))
                    .unwrap_or(Value::Undefined);
                Ok(match href {
                    Value::Undefined => self.make_string_value(""),
                    other => self.make_string_value(&self.to_string(&other)),
                })
            }
            BuiltinId::UrlToPrimitive => {
                let href = self
                    .get_property_value(&this_value, &PropertyKey::from("href"))
                    .unwrap_or(Value::Undefined);
                Ok(match href {
                    Value::Undefined => Value::Undefined,
                    other => self.make_string_value(&self.to_string(&other)),
                })
            }
            BuiltinId::UspGet => {
                let pairs = self.usp_pairs(&this_value)?;
                let name = self.string_arg(&args, 0);
                match pairs.iter().find(|(k, _)| *k == name) {
                    Some((_, value)) => Ok(self.make_string_value(value)),
                    None => Ok(Value::Null),
                }
            }
            BuiltinId::UspGetAll => {
                let pairs = self.usp_pairs(&this_value)?;
                let name = self.string_arg(&args, 0);
                let values: Vec<String> = pairs
                    .into_iter()
                    .filter(|(k, _)| *k == name)
                    .map(|(_, v)| v)
                    .collect();
                let values = values
                    .into_iter()
                    .map(|v| self.make_string_value(&v))
                    .collect();
                self.make_array_from_values(values)
            }
            BuiltinId::UspHas => {
                let pairs = self.usp_pairs(&this_value)?;
                let name = self.string_arg(&args, 0);
                Ok(Value::Bool(pairs.iter().any(|(k, _)| *k == name)))
            }
            BuiltinId::UspSet => {
                let mut pairs = self.usp_pairs(&this_value)?;
                let name = self.string_arg(&args, 0);
                let value = self.string_arg(&args, 1);
                if pairs.iter().any(|(k, _)| *k == name) {
                    let mut replaced = false;
                    pairs.retain_mut(|(k, v)| {
                        if *k == name {
                            if replaced {
                                false
                            } else {
                                *v = value.clone();
                                replaced = true;
                                true
                            }
                        } else {
                            true
                        }
                    });
                } else {
                    pairs.push((name, value));
                }
                self.usp_set_pairs(&this_value, pairs);
                Ok(Value::Undefined)
            }
            BuiltinId::UspAppend => {
                let mut pairs = self.usp_pairs(&this_value)?;
                let name = self.string_arg(&args, 0);
                let value = self.string_arg(&args, 1);
                pairs.push((name, value));
                self.usp_set_pairs(&this_value, pairs);
                Ok(Value::Undefined)
            }
            BuiltinId::UspDelete => {
                let mut pairs = self.usp_pairs(&this_value)?;
                let name = self.string_arg(&args, 0);
                pairs.retain(|(k, _)| *k != name);
                self.usp_set_pairs(&this_value, pairs);
                Ok(Value::Undefined)
            }
            BuiltinId::UspToString => {
                let pairs = self.usp_pairs(&this_value)?;
                let encoded = pairs
                    .iter()
                    .map(|(k, v)| format!("{}={}", form_urlencode(k), form_urlencode(v)))
                    .collect::<Vec<_>>()
                    .join("&");
                Ok(self.make_string_value(&encoded))
            }
            BuiltinId::UspForEach => {
                let pairs = self.usp_pairs(&this_value)?;
                let callback = args.first().cloned().unwrap_or(Value::Undefined);
                let this_arg = args.get(1).cloned().unwrap_or(Value::Undefined);
                for (name, value) in pairs {
                    let name_value = self.make_string_value(&name);
                    let value_value = self.make_string_value(&value);
                    self.call_value_sync(
                        callback.clone(),
                        this_arg.clone(),
                        vec![value_value, name_value, this_value.clone()],
                    )?;
                }
                Ok(Value::Undefined)
            }
            BuiltinId::UspEntries => {
                let pairs = self.usp_pairs(&this_value)?;
                let mut entries = Vec::with_capacity(pairs.len());
                for (name, value) in pairs {
                    let name_value = self.make_string_value(&name);
                    let value_value = self.make_string_value(&value);
                    entries.push(self.make_array_from_values(vec![name_value, value_value])?);
                }
                Ok(self.make_for_of_iterator(entries))
            }
            BuiltinId::UspKeys => {
                let pairs = self.usp_pairs(&this_value)?;
                let keys = pairs
                    .into_iter()
                    .map(|(k, _)| self.make_string_value(&k))
                    .collect();
                Ok(self.make_for_of_iterator(keys))
            }
            BuiltinId::UspValues => {
                let pairs = self.usp_pairs(&this_value)?;
                let values = pairs
                    .into_iter()
                    .map(|(_, v)| self.make_string_value(&v))
                    .collect();
                Ok(self.make_for_of_iterator(values))
            }
            BuiltinId::UspSort => {
                let mut pairs = self.usp_pairs(&this_value)?;
                pairs.sort_by(|a, b| a.0.cmp(&b.0));
                self.usp_set_pairs(&this_value, pairs);
                Ok(Value::Undefined)
            }
            BuiltinId::HeadersSet => {
                let mut pairs = self.headers_pairs(&this_value)?;
                let name = self.headers_name_arg(&args, 0);
                let value = self.string_arg(&args, 1);
                pairs.retain(|(k, _)| *k != name);
                pairs.push((name, value));
                self.headers_set_pairs(&this_value, pairs);
                Ok(Value::Undefined)
            }
            BuiltinId::HeadersHas => {
                let pairs = self.headers_pairs(&this_value)?;
                let name = self.headers_name_arg(&args, 0);
                Ok(Value::Bool(pairs.iter().any(|(k, _)| *k == name)))
            }
            BuiltinId::HeadersAppend => {
                let mut pairs = self.headers_pairs(&this_value)?;
                let name = self.headers_name_arg(&args, 0);
                let value = self.string_arg(&args, 1);
                pairs.push((name, value));
                self.headers_set_pairs(&this_value, pairs);
                Ok(Value::Undefined)
            }
            BuiltinId::HeadersDelete => {
                let mut pairs = self.headers_pairs(&this_value)?;
                let name = self.headers_name_arg(&args, 0);
                pairs.retain(|(k, _)| *k != name);
                self.headers_set_pairs(&this_value, pairs);
                Ok(Value::Undefined)
            }
            BuiltinId::HeadersForEach => {
                let pairs = self.headers_pairs(&this_value)?;
                let callback = args.first().cloned().unwrap_or(Value::Undefined);
                let this_arg = args.get(1).cloned().unwrap_or(Value::Undefined);
                for (name, value) in pairs {
                    let name_value = self.make_string_value(&name);
                    let value_value = self.make_string_value(&value);
                    self.call_value_sync(
                        callback.clone(),
                        this_arg.clone(),
                        vec![value_value, name_value, this_value.clone()],
                    )?;
                }
                Ok(Value::Undefined)
            }
            BuiltinId::HeadersEntries => {
                let pairs = self.headers_pairs(&this_value)?;
                let mut entries = Vec::with_capacity(pairs.len());
                for (name, value) in pairs {
                    let name_value = self.make_string_value(&name);
                    let value_value = self.make_string_value(&value);
                    entries.push(self.make_array_from_values(vec![name_value, value_value])?);
                }
                Ok(self.make_for_of_iterator(entries))
            }
            BuiltinId::HeadersKeys => {
                let pairs = self.headers_pairs(&this_value)?;
                let keys = pairs
                    .into_iter()
                    .map(|(k, _)| self.make_string_value(&k))
                    .collect();
                Ok(self.make_for_of_iterator(keys))
            }
            BuiltinId::HeadersValues => {
                let pairs = self.headers_pairs(&this_value)?;
                let values = pairs
                    .into_iter()
                    .map(|(_, v)| self.make_string_value(&v))
                    .collect();
                Ok(self.make_for_of_iterator(values))
            }
            BuiltinId::FormDataGet => {
                let pairs = self.form_data_pairs(&this_value)?;
                let name = self.string_arg(&args, 0);
                match pairs.iter().find(|(k, _)| *k == name) {
                    Some((_, value)) => Ok(self.make_string_value(value)),
                    None => Ok(Value::Null),
                }
            }
            BuiltinId::FormDataGetAll => {
                let pairs = self.form_data_pairs(&this_value)?;
                let name = self.string_arg(&args, 0);
                let values: Vec<Value> = pairs
                    .into_iter()
                    .filter(|(k, _)| *k == name)
                    .map(|(_, v)| self.make_string_value(&v))
                    .collect();
                self.make_array_from_values(values)
            }
            BuiltinId::FormDataHas => {
                let pairs = self.form_data_pairs(&this_value)?;
                let name = self.string_arg(&args, 0);
                Ok(Value::Bool(pairs.iter().any(|(k, _)| *k == name)))
            }
            BuiltinId::FormDataSet => {
                let mut pairs = self.form_data_pairs(&this_value)?;
                let name = self.string_arg(&args, 0);
                let value = self.string_arg(&args, 1);
                pairs.retain(|(k, _)| *k != name);
                pairs.push((name, value));
                self.form_data_set_pairs(&this_value, pairs);
                Ok(Value::Undefined)
            }
            BuiltinId::FormDataAppend => {
                let mut pairs = self.form_data_pairs(&this_value)?;
                let name = self.string_arg(&args, 0);
                let value = self.string_arg(&args, 1);
                pairs.push((name, value));
                self.form_data_set_pairs(&this_value, pairs);
                Ok(Value::Undefined)
            }
            BuiltinId::FormDataDelete => {
                let mut pairs = self.form_data_pairs(&this_value)?;
                let name = self.string_arg(&args, 0);
                pairs.retain(|(k, _)| *k != name);
                self.form_data_set_pairs(&this_value, pairs);
                Ok(Value::Undefined)
            }
            BuiltinId::FormDataForEach => {
                let pairs = self.form_data_pairs(&this_value)?;
                let callback = args.first().cloned().unwrap_or(Value::Undefined);
                let this_arg = args.get(1).cloned().unwrap_or(Value::Undefined);
                for (name, value) in pairs {
                    let name_value = self.make_string_value(&name);
                    let value_value = self.make_string_value(&value);
                    self.call_value_sync(
                        callback.clone(),
                        this_arg.clone(),
                        vec![value_value, name_value, this_value.clone()],
                    )?;
                }
                Ok(Value::Undefined)
            }
            BuiltinId::FormDataEntries => {
                let pairs = self.form_data_pairs(&this_value)?;
                let mut entries = Vec::with_capacity(pairs.len());
                for (name, value) in pairs {
                    let name_value = self.make_string_value(&name);
                    let value_value = self.make_string_value(&value);
                    entries.push(self.make_array_from_values(vec![name_value, value_value])?);
                }
                Ok(self.make_for_of_iterator(entries))
            }
            BuiltinId::FormDataKeys => {
                let pairs = self.form_data_pairs(&this_value)?;
                let keys = pairs
                    .into_iter()
                    .map(|(k, _)| self.make_string_value(&k))
                    .collect();
                Ok(self.make_for_of_iterator(keys))
            }
            BuiltinId::FormDataValues => {
                let pairs = self.form_data_pairs(&this_value)?;
                let values = pairs
                    .into_iter()
                    .map(|(_, v)| self.make_string_value(&v))
                    .collect();
                Ok(self.make_for_of_iterator(values))
            }
            BuiltinId::ProxyConstructor => {
                let target = self
                    .require_object_ref(args.first().unwrap_or(&Value::Undefined), "Proxy target")?;
                let handler = self.require_object_ref(
                    args.get(1).unwrap_or(&Value::Undefined),
                    "Proxy handler",
                )?;
                let proxy = self.heap.allocate_object(JsObject {
                    kind: ObjectKind::Proxy { target, handler },
                    prototype: self.heap.objects().get(target).and_then(|o| o.prototype),
                    ..JsObject::default()
                });
                Ok(Value::Object(proxy))
            }
            BuiltinId::StringProtoCharAt => {
                let text = self.builtin_string_this(&this_value)?;
                let index = args
                    .first()
                    .map(|value| self.to_number(value) as usize)
                    .unwrap_or(0);
                Ok(text
                    .chars()
                    .nth(index)
                    .map(|character| self.make_string_value(&character.to_string()))
                    .unwrap_or_else(|| self.make_string_value("")))
            }
            BuiltinId::StringProtoCharCodeAt | BuiltinId::StringProtoCodePointAt => {
                let text = self.builtin_string_this(&this_value)?;
                let index = args
                    .first()
                    .map(|value| self.to_number(value) as usize)
                    .unwrap_or(0);
                let code = text.chars().nth(index).map(|character| character as u32);
                Ok(code
                    .map(|value| Value::Number(value as f64))
                    .unwrap_or(Value::Number(f64::NAN)))
            }
            BuiltinId::StringProtoIndexOf
            | BuiltinId::StringProtoLastIndexOf
            | BuiltinId::StringProtoIncludes
            | BuiltinId::StringProtoStartsWith
            | BuiltinId::StringProtoEndsWith => {
                let text = self.builtin_string_this(&this_value)?;
                let needle = args
                    .first()
                    .map(|value| self.to_string(value))
                    .unwrap_or_default();
                Ok(match builtin {
                    BuiltinId::StringProtoIndexOf => {
                        let from = args
                            .get(1)
                            .map(|value| self.to_number(value) as usize)
                            .unwrap_or(0);
                        let index = text[from.min(text.len())..]
                            .find(&needle)
                            .map(|value| value + from)
                            .map(|value| value as f64)
                            .unwrap_or(-1.0);
                        Value::Number(index)
                    }
                    BuiltinId::StringProtoLastIndexOf => {
                        let index = text
                            .rfind(&needle)
                            .map(|value| value as f64)
                            .unwrap_or(-1.0);
                        Value::Number(index)
                    }
                    BuiltinId::StringProtoIncludes => Value::Bool(text.contains(&needle)),
                    BuiltinId::StringProtoStartsWith => Value::Bool(text.starts_with(&needle)),
                    BuiltinId::StringProtoEndsWith => Value::Bool(text.ends_with(&needle)),
                    _ => unreachable!(),
                })
            }
            BuiltinId::StringProtoSlice => {
                let text = self.builtin_string_this(&this_value)?;
                let chars = text.chars().collect::<Vec<_>>();
                let (start, end) =
                    self.normalize_slice_bounds(chars.len(), args.first(), args.get(1));
                let slice = chars[start..end].iter().collect::<String>();
                Ok(self.make_string_value(&slice))
            }
            BuiltinId::StringProtoSubstring => {
                let text = self.builtin_string_this(&this_value)?;
                let chars = text.chars().collect::<Vec<_>>();
                let start = args
                    .first()
                    .map(|value| self.to_number(value).max(0.0) as usize)
                    .unwrap_or(0)
                    .min(chars.len());
                let end = args
                    .get(1)
                    .map(|value| self.to_number(value).max(0.0) as usize)
                    .unwrap_or(chars.len())
                    .min(chars.len());
                let (start, end) = if start <= end {
                    (start, end)
                } else {
                    (end, start)
                };
                Ok(self.make_string_value(&chars[start..end].iter().collect::<String>()))
            }
            BuiltinId::StringProtoSplit => {
                let text = self.builtin_string_this(&this_value)?;
                // RegExp separator splits via the regex engine.
                if let Some(value) = args.first() {
                    if let Some((source, flags)) = self.regexp_source_flags(value) {
                        let regex = compile_js_regex(&source, &flags)?;
                        let segments: Vec<String> = regex.split(&text);
                        let values = segments
                            .into_iter()
                            .map(|s| self.make_string_value(&s))
                            .collect();
                        return self.make_array_from_values(values);
                    }
                }
                let separator = args.first().map(|value| self.to_string(value));
                let values = match separator {
                    Some(separator) if separator.is_empty() => text
                        .chars()
                        .map(|character| self.make_string_value(&character.to_string()))
                        .collect(),
                    Some(separator) => text
                        .split(&separator)
                        .map(|segment| self.make_string_value(segment))
                        .collect(),
                    None => vec![self.make_string_value(&text)],
                };
                self.make_array_from_values(values)
            }
            BuiltinId::StringProtoReplace | BuiltinId::StringProtoReplaceAll => {
                let text = self.builtin_string_this(&this_value)?;
                let replace_all = builtin == BuiltinId::StringProtoReplaceAll;
                let pattern = args.first().cloned().unwrap_or(Value::Undefined);
                let replacement = args.get(1).cloned().unwrap_or(Value::Undefined);

                // RegExp pattern: replace all when the regex is global (or replaceAll).
                if let Some((source, flags)) = self.regexp_source_flags(&pattern) {
                    let regex = compile_js_regex(&source, &flags)?;
                    let global = replace_all || flags.contains('g');
                    return self.regex_replace(&text, &regex, &replacement, global);
                }

                // String pattern.
                let search = self.to_string(&pattern);
                self.string_replace(&text, &search, &replacement, replace_all)
            }
            BuiltinId::StringProtoTrim => {
                let text = self.builtin_string_this(&this_value)?;
                Ok(self.make_string_value(text.trim()))
            }
            BuiltinId::StringProtoTrimStart => {
                let text = self.builtin_string_this(&this_value)?;
                Ok(self.make_string_value(text.trim_start()))
            }
            BuiltinId::StringProtoTrimEnd => {
                let text = self.builtin_string_this(&this_value)?;
                Ok(self.make_string_value(text.trim_end()))
            }
            BuiltinId::StringProtoToUpperCase => {
                let text = self.builtin_string_this(&this_value)?;
                Ok(self.make_string_value(&text.to_uppercase()))
            }
            BuiltinId::StringProtoToLowerCase => {
                let text = self.builtin_string_this(&this_value)?;
                Ok(self.make_string_value(&text.to_lowercase()))
            }
            BuiltinId::StringProtoPadStart | BuiltinId::StringProtoPadEnd => {
                let text = self.builtin_string_this(&this_value)?;
                let target_len = args
                    .first()
                    .map(|value| self.to_number(value).max(0.0) as usize)
                    .unwrap_or(0);
                let pad = args
                    .get(1)
                    .map(|value| self.to_string(value))
                    .filter(|pad| !pad.is_empty())
                    .unwrap_or_else(|| " ".to_string());
                let mut result = text.clone();
                while result.chars().count() < target_len {
                    if builtin == BuiltinId::StringProtoPadStart {
                        result = format!("{pad}{result}");
                    } else {
                        result.push_str(&pad);
                    }
                }
                let trimmed = result.chars().take(target_len).collect::<String>();
                Ok(self.make_string_value(&trimmed))
            }
            BuiltinId::StringProtoRepeat => {
                let text = self.builtin_string_this(&this_value)?;
                let count = args
                    .first()
                    .map(|value| self.to_number(value) as isize)
                    .unwrap_or(0);
                if count < 0 {
                    return Err(VmError::RangeError(
                        "repeat count must be non-negative".to_string(),
                    ));
                }
                Ok(self.make_string_value(&text.repeat(count as usize)))
            }
            BuiltinId::NumberIsNaN => Ok(Value::Bool(matches!(
                args.first(),
                Some(Value::Number(number)) if number.is_nan()
            ))),
            BuiltinId::NumberIsFinite => Ok(Value::Bool(matches!(
                args.first(),
                Some(Value::Number(number)) if number.is_finite()
            ))),
            BuiltinId::NumberIsInteger => Ok(Value::Bool(matches!(
                args.first(),
                Some(Value::Number(number)) if number.is_finite() && number.fract() == 0.0
            ))),
            BuiltinId::NumberIsSafeInteger => Ok(Value::Bool(matches!(
                args.first(),
                Some(Value::Number(number))
                    if number.is_finite()
                        && number.fract() == 0.0
                        && number.abs() <= 9_007_199_254_740_991.0
            ))),
            BuiltinId::NumberParseInt | BuiltinId::GlobalParseInt => {
                let text = args
                    .first()
                    .map(|value| self.to_string(value))
                    .unwrap_or_default();
                let radix = args.get(1).map(|value| self.to_number(value));
                Ok(Value::Number(js_parse_int(&text, radix)))
            }
            BuiltinId::NumberParseFloat | BuiltinId::GlobalParseFloat => {
                let text = args
                    .first()
                    .map(|value| self.to_string(value))
                    .unwrap_or_default();
                Ok(Value::Number(js_parse_float(&text)))
            }
            BuiltinId::GlobalIsNaN => {
                let number = self.number_arg(&args, 0);
                Ok(Value::Bool(number.is_nan()))
            }
            BuiltinId::GlobalIsFinite => {
                let number = self.number_arg(&args, 0);
                Ok(Value::Bool(number.is_finite()))
            }
            BuiltinId::EncodeUriComponent => {
                let text = self.string_arg(&args, 0);
                Ok(self.make_string_value(&encode_uri(&text, false)))
            }
            BuiltinId::EncodeUri => {
                let text = self.string_arg(&args, 0);
                Ok(self.make_string_value(&encode_uri(&text, true)))
            }
            BuiltinId::DecodeUriComponent | BuiltinId::DecodeUri => {
                let text = self.string_arg(&args, 0);
                let decoded = decode_uri(&text)
                    .ok_or_else(|| VmError::TypeError("URI malformed".to_string()))?;
                Ok(self.make_string_value(&decoded))
            }
            BuiltinId::MathFloor => Ok(Value::Number(self.number_arg(&args, 0).floor())),
            BuiltinId::MathCeil => Ok(Value::Number(self.number_arg(&args, 0).ceil())),
            BuiltinId::MathRound => Ok(Value::Number(self.number_arg(&args, 0).round())),
            BuiltinId::MathTrunc => Ok(Value::Number(self.number_arg(&args, 0).trunc())),
            BuiltinId::MathAbs => Ok(Value::Number(self.number_arg(&args, 0).abs())),
            BuiltinId::MathMin => Ok(Value::Number(
                args.iter()
                    .map(|value| self.to_number(value))
                    .fold(f64::INFINITY, f64::min),
            )),
            BuiltinId::MathMax => Ok(Value::Number(
                args.iter()
                    .map(|value| self.to_number(value))
                    .fold(f64::NEG_INFINITY, f64::max),
            )),
            BuiltinId::MathPow => Ok(Value::Number(
                self.number_arg(&args, 0).powf(self.number_arg(&args, 1)),
            )),
            BuiltinId::MathSqrt => Ok(Value::Number(self.number_arg(&args, 0).sqrt())),
            BuiltinId::MathCbrt => Ok(Value::Number(self.number_arg(&args, 0).cbrt())),
            BuiltinId::MathExpm1 => Ok(Value::Number(self.number_arg(&args, 0).exp_m1())),
            BuiltinId::MathFround => Ok(Value::Number((self.number_arg(&args, 0) as f32) as f64)),
            BuiltinId::MathSin => Ok(Value::Number(self.number_arg(&args, 0).sin())),
            BuiltinId::MathCos => Ok(Value::Number(self.number_arg(&args, 0).cos())),
            BuiltinId::MathTan => Ok(Value::Number(self.number_arg(&args, 0).tan())),
            BuiltinId::MathSinh => Ok(Value::Number(self.number_arg(&args, 0).sinh())),
            BuiltinId::MathCosh => Ok(Value::Number(self.number_arg(&args, 0).cosh())),
            BuiltinId::MathTanh => Ok(Value::Number(self.number_arg(&args, 0).tanh())),
            BuiltinId::MathAsin => Ok(Value::Number(self.number_arg(&args, 0).asin())),
            BuiltinId::MathAcos => Ok(Value::Number(self.number_arg(&args, 0).acos())),
            BuiltinId::MathAtan => Ok(Value::Number(self.number_arg(&args, 0).atan())),
            BuiltinId::MathAsinh => Ok(Value::Number(self.number_arg(&args, 0).asinh())),
            BuiltinId::MathAcosh => Ok(Value::Number(self.number_arg(&args, 0).acosh())),
            BuiltinId::MathAtanh => Ok(Value::Number(self.number_arg(&args, 0).atanh())),
            BuiltinId::MathAtan2 => Ok(Value::Number(
                self.number_arg(&args, 0).atan2(self.number_arg(&args, 1)),
            )),
            BuiltinId::MathLog => Ok(Value::Number(self.number_arg(&args, 0).ln())),
            BuiltinId::MathLog2 => Ok(Value::Number(self.number_arg(&args, 0).log2())),
            BuiltinId::MathLog10 => Ok(Value::Number(self.number_arg(&args, 0).log10())),
            BuiltinId::MathExp => Ok(Value::Number(self.number_arg(&args, 0).exp())),
            BuiltinId::MathRandom => Ok(Value::Number(self.next_random())),
            BuiltinId::CryptoGetRandomValues => {
                let target = args.first().cloned().unwrap_or(Value::Undefined);
                let Some((_, _kind, _, length)) = self.typed_array_info(&target) else {
                    return Err(VmError::TypeError(
                        "crypto.getRandomValues: argument must be an integer-typed TypedArray"
                            .to_string(),
                    ));
                };
                for index in 0..length {
                    let r = (self.next_random() * 4294967296.0).floor();
                    self.typed_array_write_element(&target, index, r)?;
                }
                Ok(target)
            }
            BuiltinId::CryptoRandomUUID => {
                let mut bytes = [0u8; 16];
                for b in bytes.iter_mut() {
                    *b = (self.next_random() * 256.0).floor() as u8;
                }
                bytes[6] = (bytes[6] & 0x0f) | 0x40;
                bytes[8] = (bytes[8] & 0x3f) | 0x80;
                let hex: String = bytes.iter().map(|b| format!("{:02x}", b)).collect();
                let uuid = format!(
                    "{}-{}-{}-{}-{}",
                    &hex[0..8],
                    &hex[8..12],
                    &hex[12..16],
                    &hex[16..20],
                    &hex[20..32]
                );
                Ok(self.make_string_value(&uuid))
            }
            BuiltinId::JsonStringify => {
                let value = args.first().cloned().unwrap_or(Value::Undefined);
                let replacer = args
                    .get(1)
                    .filter(|value| self.is_callable_value(value))
                    .cloned();
                let json = match self.to_json_value("", &value, replacer.as_ref())? {
                    Some(json) => json,
                    None => return Ok(Value::Undefined),
                };
                // Third argument controls indentation (number of spaces or a string).
                let indent = match args.get(2) {
                    Some(Value::Number(n)) if *n >= 1.0 => {
                        Some(" ".repeat((*n as usize).min(10)))
                    }
                    Some(Value::String(s)) => {
                        let text = self.string_text(*s);
                        if text.is_empty() {
                            None
                        } else {
                            Some(text.chars().take(10).collect::<String>())
                        }
                    }
                    _ => None,
                };
                let output = match indent {
                    Some(indent) => json_to_pretty_string(&json, &indent, 0),
                    None => serde_json::to_string(&json).unwrap_or_else(|_| "null".to_string()),
                };
                Ok(self.make_string_value(&output))
            }
            BuiltinId::JsonParse => {
                let text = args
                    .first()
                    .map(|value| self.to_string(value))
                    .unwrap_or_default();
                let json = serde_json::from_str::<JsonValue>(&text)
                    .map_err(|error| VmError::TypeError(error.to_string()))?;
                let parsed = self.from_json_value(&json)?;
                // Optional reviver: walk the result bottom-up transforming values.
                match args.get(1).filter(|value| self.is_callable_value(value)) {
                    Some(reviver) => {
                        let reviver = reviver.clone();
                        let holder =
                            self.allocate_ordinary_object(Some(self.object_prototype_ref()));
                        self.set_property_on_object(
                            holder,
                            Value::Object(holder),
                            PropertyKey::from(""),
                            parsed,
                        )?;
                        self.internalize_json_property(holder, "", &reviver)
                    }
                    None => Ok(parsed),
                }
            }
            BuiltinId::ConsoleLog | BuiltinId::ConsoleInfo | BuiltinId::ConsoleWarn | BuiltinId::ConsoleError => {
                let level = match builtin {
                    BuiltinId::ConsoleInfo  => ConsoleLevel::Info,
                    BuiltinId::ConsoleWarn  => ConsoleLevel::Warn,
                    BuiltinId::ConsoleError => ConsoleLevel::Error,
                    _                       => ConsoleLevel::Log,
                };
                let parts: Vec<String> = args.iter().map(|v| self.to_string(v)).collect();
                let _ = self.host.console(ConsoleMessage { level, parts });
                Ok(Value::Undefined)
            }
            BuiltinId::ModuleReexportAll => {
                let Some(self_namespace) = args.first().and_then(|value| match value {
                    Value::Object(object) => Some(*object),
                    _ => None,
                }) else {
                    return Err(VmError::TypeError("module namespace expected".to_string()));
                };
                let Some(dep_namespace) = args.get(1).and_then(|value| match value {
                    Value::Object(object) => Some(*object),
                    _ => None,
                }) else {
                    return Err(VmError::TypeError("module namespace expected".to_string()));
                };
                for key in self.object_own_enumerable_keys(dep_namespace) {
                    if key == PropertyKey::from("default") {
                        continue;
                    }
                    let value = self.get_property_value(&Value::Object(dep_namespace), &key)?;
                    self.define_data_property(self_namespace, key, value, true, true, true);
                }
                Ok(Value::Undefined)
            }
            BuiltinId::MapProtoSet => {
                let object = self.builtin_object_this(&this_value, "Map.prototype.set")?;
                let key = args.first().cloned().unwrap_or(Value::Undefined);
                let value = args.get(1).cloned().unwrap_or(Value::Undefined);
                self.map_set(object, key, value, false)?;
                Ok(Value::Object(object))
            }
            BuiltinId::MapProtoGet => {
                let object = self.builtin_object_this(&this_value, "Map.prototype.get")?;
                Ok(self
                    .map_get(object, args.first().unwrap_or(&Value::Undefined))?
                    .unwrap_or(Value::Undefined))
            }
            BuiltinId::MapProtoHas => {
                let object = self.builtin_object_this(&this_value, "Map.prototype.has")?;
                Ok(Value::Bool(
                    self.map_get(object, args.first().unwrap_or(&Value::Undefined))?
                        .is_some(),
                ))
            }
            BuiltinId::MapProtoDelete => {
                let object = self.builtin_object_this(&this_value, "Map.prototype.delete")?;
                Ok(Value::Bool(self.map_delete(
                    object,
                    args.first().unwrap_or(&Value::Undefined),
                )?))
            }
            BuiltinId::MapProtoClear => {
                let object = self.builtin_object_this(&this_value, "Map.prototype.clear")?;
                self.map_clear(object)?;
                Ok(Value::Undefined)
            }
            BuiltinId::MapProtoForEach => {
                let object = self.builtin_object_this(&this_value, "Map.prototype.forEach")?;
                let callback = args.first().cloned().unwrap_or(Value::Undefined);
                let entries: Vec<(Value, Value)> = match self.heap.objects().get(object) {
                    Some(data) => match &data.kind {
                        ObjectKind::Map(entries) | ObjectKind::WeakMap(entries) => entries.clone(),
                        _ => Vec::new(),
                    },
                    None => Vec::new(),
                };
                for (key, value) in entries {
                    let _ = self.call_value_sync(
                        callback.clone(),
                        Value::Undefined,
                        vec![value, key, Value::Object(object)],
                    )?;
                }
                Ok(Value::Undefined)
            }
            BuiltinId::MapProtoEntries => {
                let object = self.builtin_object_this(&this_value, "Map.prototype.entries")?;
                let entries: Vec<(Value, Value)> = match self.heap.objects().get(object) {
                    Some(data) => match &data.kind {
                        ObjectKind::Map(entries) | ObjectKind::WeakMap(entries) => entries.clone(),
                        _ => Vec::new(),
                    },
                    None => Vec::new(),
                };
                let mut pairs = Vec::with_capacity(entries.len());
                for (key, value) in entries {
                    pairs.push(self.make_array_from_values(vec![key, value])?);
                }
                self.make_array_from_values(pairs)
            }
            BuiltinId::MapProtoKeys => {
                let object = self.builtin_object_this(&this_value, "Map.prototype.keys")?;
                let keys: Vec<Value> = match self.heap.objects().get(object) {
                    Some(data) => match &data.kind {
                        ObjectKind::Map(entries) | ObjectKind::WeakMap(entries) => {
                            entries.iter().map(|(k, _)| k.clone()).collect()
                        }
                        _ => Vec::new(),
                    },
                    None => Vec::new(),
                };
                self.make_array_from_values(keys)
            }
            BuiltinId::MapProtoValues => {
                let object = self.builtin_object_this(&this_value, "Map.prototype.values")?;
                let values: Vec<Value> = match self.heap.objects().get(object) {
                    Some(data) => match &data.kind {
                        ObjectKind::Map(entries) | ObjectKind::WeakMap(entries) => {
                            entries.iter().map(|(_, v)| v.clone()).collect()
                        }
                        _ => Vec::new(),
                    },
                    None => Vec::new(),
                };
                self.make_array_from_values(values)
            }
            BuiltinId::SetProtoAdd => {
                let object = self.builtin_object_this(&this_value, "Set.prototype.add")?;
                let value = args.first().cloned().unwrap_or(Value::Undefined);
                self.set_add(object, value, false)?;
                Ok(Value::Object(object))
            }
            BuiltinId::SetProtoHas => {
                let object = self.builtin_object_this(&this_value, "Set.prototype.has")?;
                Ok(Value::Bool(self.set_has(
                    object,
                    args.first().unwrap_or(&Value::Undefined),
                )?))
            }
            BuiltinId::SetProtoDelete => {
                let object = self.builtin_object_this(&this_value, "Set.prototype.delete")?;
                Ok(Value::Bool(self.set_delete(
                    object,
                    args.first().unwrap_or(&Value::Undefined),
                )?))
            }
            BuiltinId::SetProtoClear => {
                let object = self.builtin_object_this(&this_value, "Set.prototype.clear")?;
                self.set_clear(object)?;
                Ok(Value::Undefined)
            }
            BuiltinId::SetProtoForEach => {
                let object = self.builtin_object_this(&this_value, "Set.prototype.forEach")?;
                let callback = args.first().cloned().unwrap_or(Value::Undefined);
                let values: Vec<Value> = match self.heap.objects().get(object) {
                    Some(data) => match &data.kind {
                        ObjectKind::Set(values) | ObjectKind::WeakSet(values) => values.clone(),
                        _ => Vec::new(),
                    },
                    None => Vec::new(),
                };
                for value in values {
                    let _ = self.call_value_sync(
                        callback.clone(),
                        Value::Undefined,
                        vec![value.clone(), value, Value::Object(object)],
                    )?;
                }
                Ok(Value::Undefined)
            }
            BuiltinId::SetProtoValues => {
                let object = self.builtin_object_this(&this_value, "Set.prototype.values")?;
                let values: Vec<Value> = match self.heap.objects().get(object) {
                    Some(data) => match &data.kind {
                        ObjectKind::Set(values) | ObjectKind::WeakSet(values) => values.clone(),
                        _ => Vec::new(),
                    },
                    None => Vec::new(),
                };
                self.make_array_from_values(values)
            }
            // ----------------------------------------------------------------
            // DOM — document-level methods (this = Document host object)
            // ----------------------------------------------------------------
            BuiltinId::DomDocQuerySelector | BuiltinId::DomNodeQuerySelector => {
                let sel = args.first().map(|v| self.to_string(v)).unwrap_or_default();
                // Root at the document node itself (boa parity): documentElement
                // can miss parser-rescued content outside <html>.
                let root = self.this_node_id(&this_value);
                let res = self.host.read_dom(DomRead::QuerySelector { root, selectors: sel });
                Ok(match res { Ok(DomReadResult::Node(id)) => self.make_dom_node_value(id), _ => Value::Null })
            }
            BuiltinId::DomDocQuerySelectorAll | BuiltinId::DomNodeQuerySelectorAll => {
                let sel = args.first().map(|v| self.to_string(v)).unwrap_or_default();
                self.query_all_to_array(self.this_node_id(&this_value), sel)
            }
            BuiltinId::DomDocGetElementById => {
                let id_str = args.first().map(|v| self.to_string(v)).unwrap_or_default();
                let sel = format!("#{id_str}");
                // Root at the document node itself (boa parity): documentElement
                // can miss parser-rescued content outside <html>.
                let root = self.this_node_id(&this_value);
                let res = self.host.read_dom(DomRead::QuerySelector { root, selectors: sel });
                Ok(match res { Ok(DomReadResult::Node(id)) => self.make_dom_node_value(id), _ => Value::Null })
            }
            BuiltinId::DomDocGetElementsByClassName => {
                let cls = args.first().map(|v| self.to_string(v)).unwrap_or_default();
                let sel = cls.split_whitespace().map(|c| format!(".{c}")).collect::<Vec<_>>().join("");
                self.query_all_to_array(self.this_node_id(&this_value), sel)
            }
            BuiltinId::DomDocGetElementsByTagName => {
                let tag = args.first().map(|v| self.to_string(v)).unwrap_or_default();
                self.query_all_to_array(self.this_node_id(&this_value), tag)
            }
            BuiltinId::DomDocCreateElement => {
                let tag = args.first().map(|v| self.to_string(v)).unwrap_or_default();
                let res = self.host.mutate_dom(DomMutation::CreateElement { window: WindowId(0), local_name: tag });
                Ok(match res { Ok(super::host::DomMutationResult::Node(id)) => self.make_dom_node_value(id), _ => Value::Undefined })
            }
            BuiltinId::DomCreateElementNs => {
                // createElementNS(namespace, qualifiedName): we don't track
                // namespaces, so create a plain element from the qualified name
                // (enough for SVG/MathML JS that frameworks emit).
                let tag = args.get(1).map(|v| self.to_string(v)).unwrap_or_default();
                let res = self.host.mutate_dom(DomMutation::CreateElement { window: WindowId(0), local_name: tag });
                Ok(match res { Ok(super::host::DomMutationResult::Node(id)) => self.make_dom_node_value(id), _ => Value::Undefined })
            }
            BuiltinId::DomDocCreateTextNode => {
                let text = args.first().map(|v| self.to_string(v)).unwrap_or_default();
                let res = self.host.mutate_dom(DomMutation::CreateTextNode { window: WindowId(0), data: text });
                Ok(match res { Ok(super::host::DomMutationResult::Node(id)) => self.make_dom_node_value(id), _ => Value::Undefined })
            }
            BuiltinId::DomDocCreateFragment => {
                let res = self.host.mutate_dom(DomMutation::CreateDocumentFragment { window: WindowId(0) });
                Ok(match res { Ok(super::host::DomMutationResult::Node(id)) => self.make_dom_node_value(id), _ => Value::Undefined })
            }
            BuiltinId::DomDocWrite => {
                let html = args.iter().map(|v| self.to_string(v)).collect::<Vec<_>>().join("");
                // document.write is recursive: any <script> the write adds
                // executes immediately. Diff the document's scripts around the
                // mutation to find what it added.
                let scripts_of = |vm: &mut Self| -> Vec<NodeId> {
                    match vm.host.read_dom(DomRead::QuerySelectorAll {
                        root: NodeId(0),
                        selectors: "script".to_string(),
                    }) {
                        Ok(DomReadResult::Nodes(ids)) => ids,
                        _ => Vec::new(),
                    }
                };
                let before = scripts_of(self);
                let _ = self.host.mutate_dom(DomMutation::WriteHtml { window: WindowId(0), html });
                let after = scripts_of(self);
                for id in after.into_iter().filter(|id| !before.contains(id)) {
                    let source = match self.host.read_dom(DomRead::TextContent { node: id }) {
                        Ok(DomReadResult::String(s)) => s,
                        _ => continue,
                    };
                    if !source.trim().is_empty() {
                        let _ = self.eval_source(&source);
                    }
                }
                Ok(Value::Undefined)
            }
            // ----------------------------------------------------------------
            // DOM — node/element methods (this = Node host object)
            // ----------------------------------------------------------------
            BuiltinId::DomNodeAppendChild => {
                let parent_id = self.this_node_id(&this_value);
                let child_ids = self.node_ids_from_node_or_string_args(&args);
                let _ = self.host.mutate_dom(DomMutation::Append { parent: parent_id, children: child_ids });
                Ok(args.first().cloned().unwrap_or(Value::Undefined))
            }
            BuiltinId::DomNodeInsertBefore => {
                let parent_id = self.this_node_id(&this_value);
                let child_id = self.node_id_from_host_val(args.first().unwrap_or(&Value::Undefined)).unwrap_or(NodeId(0));
                let ref_id = args.get(1).and_then(|v| self.node_id_from_host_val(v));
                let _ = self.host.mutate_dom(DomMutation::InsertBefore { parent: parent_id, child: child_id, reference: ref_id });
                Ok(args.first().cloned().unwrap_or(Value::Undefined))
            }
            BuiltinId::DomNodePrepend => {
                let parent_id = self.this_node_id(&this_value);
                let child_ids = self.node_ids_from_node_or_string_args(&args);
                let _ = self
                    .host
                    .mutate_dom(DomMutation::Prepend { parent: parent_id, children: child_ids });
                Ok(Value::Undefined)
            }
            BuiltinId::DomNodeReplaceChildren => {
                let parent_id = self.this_node_id(&this_value);
                let child_ids = self.node_ids_from_node_or_string_args(&args);
                let existing = match self
                    .host
                    .read_dom(DomRead::Children { node: parent_id, elements_only: false })
                {
                    Ok(DomReadResult::Nodes(ids)) => ids,
                    _ => Vec::new(),
                };
                for child in existing {
                    let _ = self.host.mutate_dom(DomMutation::Remove { node: child });
                }
                let _ = self.host.mutate_dom(DomMutation::Append { parent: parent_id, children: child_ids });
                Ok(Value::Undefined)
            }
            BuiltinId::DomNodeHasChildNodes => {
                let node_id = self.this_node_id(&this_value);
                let res = self
                    .host
                    .read_dom(DomRead::Children { node: node_id, elements_only: false });
                Ok(Value::Bool(matches!(res, Ok(DomReadResult::Nodes(ids)) if !ids.is_empty())))
            }
            BuiltinId::DomNodeRemoveChild => {
                // Only detach when the node really is a child of `this` (per
                // spec removeChild on a non-child throws; we no-op instead).
                let parent_id = self.this_node_id(&this_value);
                let child_id = self.node_id_from_host_val(args.first().unwrap_or(&Value::Undefined)).unwrap_or(NodeId(0));
                let is_child = matches!(
                    self.host.read_dom(DomRead::Parent { node: child_id }),
                    Ok(DomReadResult::Node(p)) if p == parent_id
                );
                if is_child {
                    let _ = self.host.mutate_dom(DomMutation::Remove { node: child_id });
                }
                Ok(args.first().cloned().unwrap_or(Value::Undefined))
            }
            BuiltinId::DomNodeReplaceChild => {
                let parent_id = self.this_node_id(&this_value);
                let new_id = self.node_id_from_host_val(args.first().unwrap_or(&Value::Undefined)).unwrap_or(NodeId(0));
                let old_id = self.node_id_from_host_val(args.get(1).unwrap_or(&Value::Undefined)).unwrap_or(NodeId(0));
                let _ = self.host.mutate_dom(DomMutation::ReplaceChild { parent: parent_id, new_child: new_id, old_child: old_id });
                Ok(args.first().cloned().unwrap_or(Value::Undefined))
            }
            BuiltinId::DomNodeCloneNode => {
                let node_id = self.this_node_id(&this_value);
                let deep = args.first().map(|v| self.is_truthy(v)).unwrap_or(false);
                let res = self.host.mutate_dom(DomMutation::CloneNode { node: node_id, deep });
                Ok(match res { Ok(super::host::DomMutationResult::Node(id)) => self.make_dom_node_value(id), _ => Value::Undefined })
            }
            BuiltinId::DomNodeRemove => {
                let node_id = self.this_node_id(&this_value);
                let _ = self.host.mutate_dom(DomMutation::Remove { node: node_id });
                Ok(Value::Undefined)
            }
            BuiltinId::DomNodeSetAttribute => {
                let node_id = self.this_node_id(&this_value);
                let name = args.first().map(|v| self.to_string(v)).unwrap_or_default();
                let value = args.get(1).map(|v| self.to_string(v)).unwrap_or_default();
                let old = match self.host.read_dom(DomRead::Attribute { node: node_id, name: name.clone() }) {
                    Ok(DomReadResult::String(s)) => Some(s),
                    _ => None,
                };
                let _ = self.host.mutate_dom(DomMutation::SetAttribute { node: node_id, name: name.clone(), value: value.clone() });
                self.fire_attribute_changed_callback(
                    node_id,
                    &this_value,
                    &name.to_ascii_lowercase(),
                    old.as_deref(),
                    Some(&value),
                )?;
                Ok(Value::Undefined)
            }
            BuiltinId::DomNodeGetAttribute => {
                let node_id = self.this_node_id(&this_value);
                let name = args.first().map(|v| self.to_string(v)).unwrap_or_default();
                let res = self.host.read_dom(DomRead::Attribute { node: node_id, name });
                Ok(match res { Ok(DomReadResult::String(s)) => self.make_string_value(&s), _ => Value::Null })
            }
            BuiltinId::ElementStubGetAttribute => {
                let name = args.first().map(|v| self.to_string(v)).unwrap_or_default();
                let key = PropertyKey::from(name);
                let value = match this_value {
                    Value::Object(object) => self.get_own_property_descriptor(object, &key),
                    _ => None,
                };
                Ok(match value {
                    Some(JsPropertyDescriptor::Data { value, .. }) if matches!(value, Value::String(_)) => value,
                    None => Value::Null,
                    _ => Value::Null,
                })
            }
            BuiltinId::ElementStubHasAttribute => {
                let name = args.first().map(|v| self.to_string(v)).unwrap_or_default();
                let key = PropertyKey::from(name);
                let value = match this_value {
                    Value::Object(object) => self.get_own_property_descriptor(object, &key),
                    _ => None,
                };
                Ok(Value::Bool(matches!(value, Some(JsPropertyDescriptor::Data { value: Value::String(_), .. }))))
            }
            BuiltinId::DomNodeRemoveAttribute => {
                let node_id = self.this_node_id(&this_value);
                let name = args.first().map(|v| self.to_string(v)).unwrap_or_default();
                let _ = self.host.mutate_dom(DomMutation::RemoveAttribute { node: node_id, name });
                Ok(Value::Undefined)
            }
            BuiltinId::DomNodeHasAttribute => {
                let node_id = self.this_node_id(&this_value);
                let name = args.first().map(|v| self.to_string(v)).unwrap_or_default();
                let res = self.host.read_dom(DomRead::Attribute { node: node_id, name });
                Ok(Value::Bool(matches!(res, Ok(DomReadResult::String(_)))))
            }
            BuiltinId::DomNodeToggleAttribute => {
                let node_id = self.this_node_id(&this_value);
                let name = args.first().map(|v| self.to_string(v)).unwrap_or_default();
                let force = args.get(1).map(|v| self.is_truthy(v));
                let res = self.host.mutate_dom(DomMutation::ToggleAttribute { node: node_id, name, force });
                Ok(match res { Ok(super::host::DomMutationResult::Bool(b)) => Value::Bool(b), _ => Value::Bool(false) })
            }
            BuiltinId::DomNodeGetAttributeNames => {
                let node_id = self.this_node_id(&this_value);
                let res = self.host.read_dom(DomRead::AttributeNames { node: node_id });
                match res {
                    Ok(DomReadResult::StringList(names)) => {
                        let items: Vec<Value> = names.iter().map(|s| self.make_string_value(s)).collect();
                        self.make_array_from_values(items)
                    }
                    _ => self.make_array_from_values(vec![]),
                }
            }
            BuiltinId::DomNodeClosest => {
                let node_id = self.this_node_id(&this_value);
                let sel = args.first().map(|v| self.to_string(v)).unwrap_or_default();
                let res = self.host.read_dom(DomRead::Closest { node: node_id, selectors: sel });
                Ok(match res { Ok(DomReadResult::Node(id)) => self.make_dom_node_value(id), _ => Value::Null })
            }
            BuiltinId::DomNodeMatches => {
                let node_id = self.this_node_id(&this_value);
                let sel = args.first().map(|v| self.to_string(v)).unwrap_or_default();
                let res = self.host.read_dom(DomRead::Matches { node: node_id, selectors: sel });
                Ok(match res { Ok(DomReadResult::Bool(b)) => Value::Bool(b), _ => Value::Bool(false) })
            }
            BuiltinId::DomNodeContains => {
                let ancestor_id = self.this_node_id(&this_value);
                let descendant_id = self.node_id_from_host_val(args.first().unwrap_or(&Value::Undefined)).unwrap_or(NodeId(0));
                let res = self.host.read_dom(DomRead::Contains { ancestor: ancestor_id, descendant: descendant_id });
                Ok(match res { Ok(DomReadResult::Bool(b)) => Value::Bool(b), _ => Value::Bool(false) })
            }
            BuiltinId::DomNodeGetBoundingClientRect => {
                let node_id = self.this_node_id(&this_value);
                let (x, y, w, h) = match self
                    .host
                    .read_dom(DomRead::BoundingClientRect { node: node_id })
                {
                    Ok(DomReadResult::Rect(r)) => (r.x, r.y, r.width, r.height),
                    _ => (0.0, 0.0, 0.0, 0.0),
                };
                let rect_obj = self.allocate_ordinary_object(None);
                let set = |vm: &mut Self, name: &str, value: f64| {
                    vm.define_data_property(rect_obj, PropertyKey::from(name), Value::Number(value), true, true, true);
                };
                set(self, "x", x);
                set(self, "y", y);
                set(self, "width", w);
                set(self, "height", h);
                set(self, "top", y);
                set(self, "left", x);
                set(self, "right", x + w);
                set(self, "bottom", y + h);
                Ok(Value::Object(rect_obj))
            }
            BuiltinId::DomNodeScrollIntoView => Ok(Value::Undefined),
            BuiltinId::Noop => Ok(Value::Undefined),
            BuiltinId::DomNodeFocus | BuiltinId::DomNodeBlur => {
                // focus()/blur() dispatch the corresponding (non-bubbling)
                // event; propagate_event moves document.activeElement.
                let target_handle = self
                    .node_id_from_host_val(&this_value)
                    .map(|id| id.0)
                    .unwrap_or(0);
                let event_type = if matches!(builtin, BuiltinId::DomNodeFocus) {
                    "focus"
                } else {
                    "blur"
                };
                let init = DomEventInit::default();
                let event_ref = self.build_host_event(event_type, &this_value, &init);
                let event_val = Value::Object(event_ref);
                self.propagate_event(target_handle, &event_val, event_type, false)?;
                Ok(Value::Undefined)
            }
            BuiltinId::DomNodeClick => {
                // `el.click()` must actually dispatch a trusted, bubbling,
                // cancelable click — otherwise delegated handlers never fire. This
                // is exactly how React responds to clicks: it registers ONE listener
                // on the root container and relies on the event bubbling up from the
                // real target, so a no-op `click()` left React UIs inert.
                let target_handle = self
                    .node_id_from_host_val(&this_value)
                    .map(|id| id.0)
                    .unwrap_or(0);
                let init = DomEventInit {
                    bubbles: true,
                    cancelable: true,
                    button: Some(0),
                    buttons: Some(1),
                    ..DomEventInit::default()
                };
                let event_ref = self.build_host_event("click", &this_value, &init);
                let event_val = Value::Object(event_ref);
                self.propagate_event(target_handle, &event_val, "click", true)?;
                Ok(Value::Undefined)
            }
            BuiltinId::DomNodeAddEventListener => {
                let node_handle = self.node_id_from_host_val(&this_value)
                    .map(|id| id.0)
                    .unwrap_or(0); // 0 = document/window
                let event_type = args.first().map(|v| self.to_string(v)).unwrap_or_default();
                let listener = args.get(1).cloned().unwrap_or(Value::Undefined);
                if let Value::Object(fn_ref) = listener {
                    self.event_listeners
                        .entry(node_handle)
                        .or_default()
                        .entry(event_type)
                        .or_default()
                        .push(fn_ref);
                }
                Ok(Value::Undefined)
            }
            BuiltinId::DomNodeRemoveEventListener => {
                let node_handle = self
                    .node_id_from_host_val(&this_value)
                    .map(|id| id.0)
                    .unwrap_or(0);
                let event_type = args.first().map(|v| self.to_string(v)).unwrap_or_default();
                let listener = args.get(1).cloned().unwrap_or(Value::Undefined);
                if let Value::Object(fn_ref) = listener {
                    if let Some(types) = self.event_listeners.get_mut(&node_handle) {
                        if let Some(list) = types.get_mut(&event_type) {
                            list.retain(|f| f.raw() != fn_ref.raw());
                        }
                    }
                }
                Ok(Value::Undefined)
            }
            BuiltinId::DomNodeDispatchEvent => {
                let target_handle = self
                    .node_id_from_host_val(&this_value)
                    .map(|id| id.0)
                    .unwrap_or(0); // 0 = document/window
                let event = args.first().cloned().unwrap_or(Value::Undefined);
                let Value::Object(event_ref) = event else {
                    return Ok(Value::Bool(true));
                };
                // `target`/`srcElement` stay the dispatch node for the whole
                // propagation; `currentTarget` updates per node below.
                self.define_data_property(event_ref, PropertyKey::from("target"), this_value.clone(), true, true, true);
                self.define_data_property(event_ref, PropertyKey::from("srcElement"), this_value.clone(), true, true, true);
                let event = Value::Object(event_ref);
                let type_value = self.get_property_value(&event, &PropertyKey::from("type"))?;
                let event_type = self.to_string(&type_value);
                let bubbles = self
                    .get_property_value(&event, &PropertyKey::from("bubbles"))
                    .map(|v| self.is_truthy(&v))
                    .unwrap_or(false);

                self.propagate_event(target_handle, &event, &event_type, bubbles)?;

                // dispatchEvent returns false iff a cancelable event had
                // preventDefault() called, true otherwise.
                let default_prevented = self
                    .get_property_value(&event, &PropertyKey::from("defaultPrevented"))
                    .unwrap_or(Value::Bool(false));
                Ok(Value::Bool(!self.is_truthy(&default_prevented)))
            }
            // ----------------------------------------------------------------
            // classList (TokenList) — this = TokenList host object with handle = element NodeId
            // ----------------------------------------------------------------
            BuiltinId::DomClassListAdd => {
                let node_id = self.this_node_id(&this_value);
                for arg in args {
                    let class_to_add = self.to_string(&arg);
                    let existing = self.get_dom_attribute(node_id, "class");
                    let mut classes: Vec<String> = existing.split_whitespace().map(|s| s.to_string()).collect();
                    if !classes.iter().any(|c| c == &class_to_add) {
                        classes.push(class_to_add);
                        let _ = self.host.mutate_dom(DomMutation::SetAttribute { node: node_id, name: "class".to_string(), value: classes.join(" ") });
                    }
                }
                Ok(Value::Undefined)
            }
            BuiltinId::DomClassListRemove => {
                let node_id = self.this_node_id(&this_value);
                let names_to_remove: Vec<String> = args.iter().map(|v| self.to_string(v)).collect();
                let existing = self.get_dom_attribute(node_id, "class");
                let filtered: Vec<String> = existing.split_whitespace()
                    .filter(|c| !names_to_remove.iter().any(|r| r == c))
                    .map(|c| c.to_string())
                    .collect();
                let _ = self.host.mutate_dom(DomMutation::SetAttribute { node: node_id, name: "class".to_string(), value: filtered.join(" ") });
                Ok(Value::Undefined)
            }
            BuiltinId::DomClassListContains => {
                let node_id = self.this_node_id(&this_value);
                let class_name = args.first().map(|v| self.to_string(v)).unwrap_or_default();
                let existing = self.get_dom_attribute(node_id, "class");
                Ok(Value::Bool(existing.split_whitespace().any(|c| c == class_name)))
            }
            BuiltinId::DomClassListToggle => {
                let node_id = self.this_node_id(&this_value);
                let class_name = args.first().map(|v| self.to_string(v)).unwrap_or_default();
                let force = args.get(1).map(|v| self.is_truthy(v));
                let existing = self.get_dom_attribute(node_id, "class");
                let has = existing.split_whitespace().any(|c| c == class_name);
                let should_add = force.unwrap_or(!has);
                if should_add {
                    if !has {
                        let new_class = if existing.is_empty() { class_name.clone() } else { format!("{existing} {class_name}") };
                        let _ = self.host.mutate_dom(DomMutation::SetAttribute { node: node_id, name: "class".to_string(), value: new_class });
                    }
                } else {
                    let filtered: String = existing.split_whitespace().filter(|c| *c != class_name).collect::<Vec<_>>().join(" ");
                    let _ = self.host.mutate_dom(DomMutation::SetAttribute { node: node_id, name: "class".to_string(), value: filtered });
                }
                Ok(Value::Bool(should_add))
            }
            BuiltinId::DomClassListReplace => {
                let node_id = self.this_node_id(&this_value);
                let old_cls = args.first().map(|v| self.to_string(v)).unwrap_or_default();
                let new_cls = args.get(1).map(|v| self.to_string(v)).unwrap_or_default();
                let existing = self.get_dom_attribute(node_id, "class");
                if existing.split_whitespace().any(|c| c == old_cls) {
                    let updated: String = existing.split_whitespace()
                        .map(|c| if c == old_cls { new_cls.as_str() } else { c })
                        .collect::<Vec<_>>().join(" ");
                    let _ = self.host.mutate_dom(DomMutation::SetAttribute { node: node_id, name: "class".to_string(), value: updated });
                    Ok(Value::Bool(true))
                } else {
                    Ok(Value::Bool(false))
                }
            }
            BuiltinId::DomClassListItem => {
                let node_id = self.this_node_id(&this_value);
                let index = args.first().map(|v| self.to_number(v) as usize).unwrap_or(0);
                let existing = self.get_dom_attribute(node_id, "class");
                let item = existing.split_whitespace().nth(index).map(|s| self.make_string_value(s));
                Ok(item.unwrap_or(Value::Null))
            }
            BuiltinId::DomClassListToString => {
                let node_id = self.this_node_id(&this_value);
                let existing = self.get_dom_attribute(node_id, "class");
                Ok(self.make_string_value(&existing))
            }
            BuiltinId::DomNodeAttachShadow => {
                let node_id = self.this_node_id(&this_value);
                let mode = match args.first() {
                    Some(opts @ Value::Object(_)) => self
                        .get_property_value(opts, &PropertyKey::from("mode"))
                        .map(|v| self.to_string(&v))
                        .unwrap_or_default(),
                    _ => String::new(),
                };
                let open = !mode.eq_ignore_ascii_case("closed");
                match self.host.mutate_dom(DomMutation::AttachShadow { host: node_id, open }) {
                    Ok(DomMutationResult::Node(shadow)) => Ok(self.make_dom_node_value(shadow)),
                    _ => Err(VmError::TypeError("attachShadow failed".to_string())),
                }
            }
            BuiltinId::DomNodeGetRootNode => {
                let node_id = self.this_node_id(&this_value);
                let composed = match args.first() {
                    Some(opts @ Value::Object(_)) => {
                        let v = self.get_property_value(opts, &PropertyKey::from("composed")).unwrap_or(Value::Undefined);
                        self.is_truthy(&v)
                    }
                    _ => false,
                };
                let res = self.host.read_dom(DomRead::RootNode { node: node_id, composed });
                Ok(match res {
                    Ok(DomReadResult::Node(id)) => self.root_node_value(id),
                    _ => Value::Null,
                })
            }
            BuiltinId::DomSlotAssignedNodes | BuiltinId::DomSlotAssignedElements => {
                let node_id = self.this_node_id(&this_value);
                let flatten = match args.first() {
                    Some(opts @ Value::Object(_)) => {
                        let v = self.get_property_value(opts, &PropertyKey::from("flatten")).unwrap_or(Value::Undefined);
                        self.is_truthy(&v)
                    }
                    _ => false,
                };
                let nodes = match self.host.read_dom(DomRead::AssignedNodes { slot: node_id, flatten }) {
                    Ok(DomReadResult::Nodes(ids)) => ids,
                    _ => Vec::new(),
                };
                let elements_only = matches!(builtin, BuiltinId::DomSlotAssignedElements);
                let mut items = Vec::new();
                for id in nodes {
                    if elements_only
                        && !matches!(
                            self.host.read_dom(DomRead::NodeKind { node: id }),
                            Ok(DomReadResult::Kind(NodeKind::Element))
                        )
                    {
                        continue;
                    }
                    items.push(self.make_dom_node_value(id));
                }
                self.make_array_from_values(items)
            }
            BuiltinId::DomEventComposedPath => {
                let stored = self.get_property_value(&this_value, &PropertyKey::from("__composedPath"))?;
                match stored {
                    Value::Object(_) => Ok(stored),
                    _ => self.make_array_from_values(Vec::new()),
                }
            }
            BuiltinId::DomNodeSplitText => {
                let node_id = self.this_node_id(&this_value);
                let offset = args.first().map(|v| self.to_number(v).max(0.0) as usize).unwrap_or(0);
                match self.host.mutate_dom(DomMutation::SplitText { node: node_id, offset }) {
                    Ok(DomMutationResult::Node(tail)) => Ok(self.make_dom_node_value(tail)),
                    _ => Err(VmError::TypeError("splitText: not a Text node".to_string())),
                }
            }
            BuiltinId::DomNodeHasAttributes => {
                let node_id = self.this_node_id(&this_value);
                let names = match self.host.read_dom(DomRead::AttributeNames { node: node_id }) {
                    Ok(DomReadResult::StringList(names)) => names,
                    _ => Vec::new(),
                };
                Ok(Value::Bool(!names.is_empty()))
            }
            BuiltinId::DomAttrMapItem | BuiltinId::DomAttrMapGetNamedItem => {
                let node_id = self.this_node_id(&this_value);
                let names = match self.host.read_dom(DomRead::AttributeNames { node: node_id }) {
                    Ok(DomReadResult::StringList(names)) => names,
                    _ => Vec::new(),
                };
                let found = if matches!(builtin, BuiltinId::DomAttrMapItem) {
                    let index = args.first().map(|v| self.to_number(v) as usize).unwrap_or(0);
                    names.get(index).cloned()
                } else {
                    let wanted = args.first().map(|v| self.to_string(v)).unwrap_or_default();
                    names.iter().find(|n| **n == wanted).cloned()
                };
                Ok(match found {
                    Some(name) => self.make_attr_value(node_id, &name),
                    None => Value::Null,
                })
            }
            BuiltinId::DomNodeInsertAdjacentHtml => {
                let node_id = self.this_node_id(&this_value);
                let position_str = args.first().map(|v| self.to_string(v)).unwrap_or_default();
                let html = args.get(1).map(|v| self.to_string(v)).unwrap_or_default();
                let Some(position) = AdjacentPosition::parse(&position_str) else {
                    return Err(VmError::TypeError(format!(
                        "insertAdjacentHTML: invalid position '{position_str}'"
                    )));
                };
                let _ = self.host.mutate_dom(DomMutation::InsertAdjacentHtml {
                    node: node_id,
                    position,
                    html,
                });
                Ok(Value::Undefined)
            }
            // style
            BuiltinId::DomStyleGetProperty => {
                let node_id = self.this_node_id(&this_value);
                let prop = args.first().map(|v| self.to_string(v)).unwrap_or_default();
                let existing = self.get_dom_attribute(node_id, "style");
                let value = get_inline_style_prop(&existing, &prop);
                Ok(self.make_string_value(&value))
            }
            BuiltinId::DomStyleSetProperty => {
                let node_id = self.this_node_id(&this_value);
                let prop = args.first().map(|v| self.to_string(v)).unwrap_or_default();
                let val = args.get(1).map(|v| self.to_string(v)).unwrap_or_default();
                let existing = self.get_dom_attribute(node_id, "style");
                let updated = set_inline_style_prop(&existing, &prop, &val);
                let _ = self.host.mutate_dom(DomMutation::SetAttribute { node: node_id, name: "style".to_string(), value: updated });
                Ok(Value::Undefined)
            }
            BuiltinId::DomComputedStyleGetProperty => {
                let node_id = self.this_node_id(&this_value);
                let raw = args.first().map(|v| self.to_string(v)).unwrap_or_default();
                let prop = camel_to_css_prop(raw.trim());
                let value = self.computed_style_value(node_id, &prop);
                Ok(self.make_string_value(&value))
            }
            BuiltinId::DomComputedStyleGetPriority => Ok(self.make_string_value("")),
            BuiltinId::DomStyleRemoveProperty => {
                let node_id = self.this_node_id(&this_value);
                let prop = args.first().map(|v| self.to_string(v)).unwrap_or_default();
                let existing = self.get_dom_attribute(node_id, "style");
                let (updated, removed) = remove_inline_style_prop(&existing, &prop);
                let _ = self.host.mutate_dom(DomMutation::SetAttribute { node: node_id, name: "style".to_string(), value: updated });
                Ok(self.make_string_value(&removed))
            }
            // performance.now()
            BuiltinId::HistoryPushState | BuiltinId::HistoryReplaceState => {
                let state = self.value_to_host_data(args.first().unwrap_or(&Value::Undefined));
                let title = args.get(1).map(|v| self.to_string(v)).unwrap_or_default();
                let url = match args.get(2) {
                    Some(Value::Undefined) | Some(Value::Null) | None => None,
                    Some(v) => Some(self.to_string(v)),
                };
                let action = if matches!(builtin, BuiltinId::HistoryPushState) {
                    HistoryAction::PushState { window: WindowId(0), url, title, state }
                } else {
                    HistoryAction::ReplaceState { window: WindowId(0), url, title, state }
                };
                let _ = self.host.history(action);
                Ok(Value::Undefined)
            }
            BuiltinId::HistoryBack | BuiltinId::HistoryForward | BuiltinId::HistoryGo => {
                let action = match builtin {
                    BuiltinId::HistoryBack => HistoryAction::Back { window: WindowId(0) },
                    BuiltinId::HistoryForward => HistoryAction::Forward { window: WindowId(0) },
                    _ => {
                        let delta = args.first().map(|v| self.to_number(v) as i32).unwrap_or(0);
                        HistoryAction::Go { window: WindowId(0), delta }
                    }
                };
                let outcome = self.host.history(action);
                // A move (restored_scroll_y is Some) fires `popstate` after the
                // location + history.state have been committed by the host.
                if matches!(&outcome, Ok(o) if o.restored_scroll_y.is_some()) {
                    self.fire_dom_event(0, "popstate")?;
                }
                Ok(Value::Undefined)
            }
            BuiltinId::PerformanceNow => {
                let ms = self.host.now().monotonic_ms as f64;
                Ok(Value::Number(ms))
            }
            BuiltinId::PerformanceMark => {
                let name = args.first().map(|v| self.to_string(v)).unwrap_or_default();
                let mark = self.allocate_ordinary_object(None);
                let mark_name = self.make_string_value(&name);
                let mark_entry_type = self.make_string_value("mark");
                self.define_data_property(mark, PropertyKey::from("name"), mark_name, true, true, true);
                self.define_data_property(mark, PropertyKey::from("entryType"), mark_entry_type, true, true, true);
                self.define_data_property(mark, PropertyKey::from("startTime"), Value::Number(self.host.now().monotonic_ms as f64), true, true, true);
                self.define_data_property(mark, PropertyKey::from("duration"), Value::Number(0.0), true, true, true);
                Ok(Value::Object(mark))
            }
            // requestIdleCallback — run callback synchronously
            BuiltinId::PerformanceMeasure => {
                let name = args.first().map(|v| self.to_string(v)).unwrap_or_default();
                let measure = self.allocate_ordinary_object(None);
                let measure_name = self.make_string_value(&name);
                let measure_entry_type = self.make_string_value("measure");
                self.define_data_property(measure, PropertyKey::from("name"), measure_name, true, true, true);
                self.define_data_property(measure, PropertyKey::from("entryType"), measure_entry_type, true, true, true);
                self.define_data_property(measure, PropertyKey::from("startTime"), Value::Number(0.0), true, true, true);
                self.define_data_property(measure, PropertyKey::from("duration"), Value::Number(0.0), true, true, true);
                Ok(Value::Object(measure))
            }
            BuiltinId::PerformanceClearMarks | BuiltinId::PerformanceClearMeasures => Ok(Value::Undefined),
            BuiltinId::PerformanceGetEntries | BuiltinId::PerformanceGetEntriesByName | BuiltinId::PerformanceGetEntriesByType => {
                self.make_array_from_values(Vec::new())
            }
            BuiltinId::RequestIdleCallback => {
                let cb = args.first().cloned().unwrap_or(Value::Undefined);
                if self.is_callable_value(&cb) {
                    let deadline = self.allocate_ordinary_object(None);
                    self.define_data_property(deadline, PropertyKey::from("didTimeout"), Value::Bool(false), true, true, true);
                    let _ = self.call_value_sync(cb, Value::Undefined, vec![Value::Object(deadline)]);
                }
                Ok(Value::Number(0.0))
            }
            BuiltinId::CancelIdleCallback => Ok(Value::Undefined),
            BuiltinId::GlobalEscape => {
                let s = args.first().map(|v| self.to_string(v)).unwrap_or_default();
                Ok(self.make_string_value(&self.escape_legacy(s)))
            }
            BuiltinId::GlobalUnescape => {
                let s = args.first().map(|v| self.to_string(v)).unwrap_or_default();
                Ok(self.make_string_value(&self.unescape_legacy(s)))
            }
            // btoa / atob
            BuiltinId::Btoa => {
                let s = args.first().map(|v| self.to_string(v)).unwrap_or_default();
                Ok(self.make_string_value(&base64_encode(s.as_bytes())))
            }
            BuiltinId::Atob => {
                let s = args.first().map(|v| self.to_string(v)).unwrap_or_default();
                let decoded = base64_decode(&s).unwrap_or_default();
                Ok(self.make_string_value(&String::from_utf8_lossy(&decoded)))
            }
            BuiltinId::WeakRefConstructor => {
                let target = args.first().cloned().unwrap_or(Value::Undefined);
                let target_obj = self.require_object_ref(&target, "WeakRef: target must be an object")?;
                let weak_ref = self.allocate_ordinary_object(Some(self.weak_ref_prototype_ref()));
                self.define_data_property(
                    weak_ref,
                    PropertyKey::from("\u{0}weakref:target"),
                    Value::Object(target_obj),
                    false,
                    false,
                    false,
                );
                Ok(Value::Object(weak_ref))
            }
            BuiltinId::WeakRefDeref => {
                let target = self
                    .get_property_value(&this_value, &PropertyKey::from("\u{0}weakref:target"))
                    .unwrap_or(Value::Undefined);
                Ok(target)
            }
            BuiltinId::TextEncoderConstructor => {
                let encoder = self.allocate_ordinary_object(Some(self.text_encoder_prototype_ref()));
                let encoding = self.make_string_value("utf-8");
                self.define_data_property(
                    encoder,
                    PropertyKey::from("encoding"),
                    encoding,
                    true,
                    true,
                    true,
                );
                Ok(Value::Object(encoder))
            }
            BuiltinId::TextEncoderEncode => {
                let s = args.first().map(|v| self.to_string(v)).unwrap_or_default();
                self.make_uint8_array(s.into_bytes())
            }
            BuiltinId::TextDecoderConstructor => {
                let label = args.first().map(|v| self.to_string(v)).unwrap_or_default();
                let encoding = if label.is_empty() || label.eq_ignore_ascii_case("utf-8") || label.eq_ignore_ascii_case("utf8") {
                    "utf-8"
                } else {
                    "utf-8"
                };
                let decoder = self.allocate_ordinary_object(Some(self.text_decoder_prototype_ref()));
                let encoding_value = self.make_string_value(encoding);
                self.define_data_property(
                    decoder,
                    PropertyKey::from("encoding"),
                    encoding_value,
                    true,
                    true,
                    true,
                );
                Ok(Value::Object(decoder))
            }
            BuiltinId::TextDecoderDecode => {
                let input = args.first().cloned().unwrap_or(Value::Undefined);
                if matches!(input, Value::Undefined) {
                    return Ok(self.make_string_value(""));
                }
                let bytes = if let Some((buffer, kind, byte_offset, length)) = self.typed_array_info(&input) {
                    let byte_len = kind.bytes_per_element() * length;
                    match self.heap.objects().get(buffer).map(|o| &o.kind) {
                        Some(ObjectKind::ArrayBuffer(buf)) => buf
                            .get(byte_offset..byte_offset + byte_len)
                            .map(<[u8]>::to_vec)
                            .unwrap_or_default(),
                        _ => Vec::new(),
                    }
                } else if let Value::Object(object) = &input {
                    match self.heap.objects().get(*object).map(|o| &o.kind) {
                        Some(ObjectKind::ArrayBuffer(buf)) => buf.clone(),
                        _ => Vec::new(),
                    }
                } else {
                    Vec::new()
                };
                Ok(self.make_string_value(&String::from_utf8_lossy(&bytes)))
            }
            // Storage item ops (this = Storage host object with kind encoded in handle)
            BuiltinId::StorageGetItem => {
                let key = args.first().map(|v| self.to_string(v)).unwrap_or_default();
                let (kind, scope) = self.storage_context(&this_value);
                let res = self.host.storage(StorageOp::Get {
                    kind,
                    scope,
                    key,
                });
                Ok(match res { Ok(StorageResult::Value(Some(v))) => self.make_string_value(&v), _ => Value::Null })
            }
            BuiltinId::StorageSetItem => {
                let key = args.first().map(|v| self.to_string(v)).unwrap_or_default();
                let val = args.get(1).map(|v| self.to_string(v)).unwrap_or_default();
                let (kind, scope) = self.storage_context(&this_value);
                let _ = self.host.storage(StorageOp::Set {
                    kind,
                    scope,
                    key,
                    value: val,
                });
                Ok(Value::Undefined)
            }
            BuiltinId::StorageRemoveItem => {
                let key = args.first().map(|v| self.to_string(v)).unwrap_or_default();
                let (kind, scope) = self.storage_context(&this_value);
                let _ = self.host.storage(StorageOp::Remove {
                    kind,
                    scope,
                    key,
                });
                Ok(Value::Undefined)
            }
            BuiltinId::StorageClear => {
                let (kind, scope) = self.storage_context(&this_value);
                let _ = self.host.storage(StorageOp::Clear {
                    kind,
                    scope,
                });
                Ok(Value::Undefined)
            }
            BuiltinId::StorageKey => {
                let index = args.first().map(|v| self.to_number(v) as usize).unwrap_or(0);
                let (kind, scope) = self.storage_context(&this_value);
                let res = self.host.storage(StorageOp::Keys {
                    kind,
                    scope,
                });
                Ok(match res {
                    Ok(StorageResult::Keys(keys)) => keys.get(index).map(|k| self.make_string_value(k)).unwrap_or(Value::Null),
                    _ => Value::Null,
                })
            }
            // window
            BuiltinId::WindowScrollTo | BuiltinId::WindowScrollBy => {
                // scrollTo(x, y) / scrollTo({left, top}); scrollBy offsets the
                // current position, and scrollTo keeps it for omitted axes.
                let is_by = matches!(builtin, BuiltinId::WindowScrollBy);
                let (cur_x, cur_y) = self
                    .host
                    .window_metrics(WindowId(0))
                    .map(|m| (m.scroll_x, m.scroll_y))
                    .unwrap_or((0.0, 0.0));
                let (opt_x, opt_y) = match args.first() {
                    Some(options @ Value::Object(_)) => {
                        let axis = |vm: &mut Self, key: &str| {
                            match vm.get_property_value(options, &PropertyKey::from(key)) {
                                Ok(Value::Undefined) | Err(_) => None,
                                Ok(v) => Some(vm.to_number(&v)),
                            }
                        };
                        (axis(self, "left"), axis(self, "top"))
                    }
                    _ => (
                        args.first().map(|v| self.to_number(v)),
                        args.get(1).map(|v| self.to_number(v)),
                    ),
                };
                let (x, y) = if is_by {
                    (cur_x + opt_x.unwrap_or(0.0), cur_y + opt_y.unwrap_or(0.0))
                } else {
                    (opt_x.unwrap_or(cur_x), opt_y.unwrap_or(cur_y))
                };
                let _ = self.host.mutate_dom(DomMutation::SetWindowScroll { window: WindowId(0), x, y });
                Ok(Value::Undefined)
            }
            BuiltinId::WindowGetComputedStyle => {
                let Some(node_id) = args.first().and_then(|v| self.node_id_from_host_val(v)) else {
                    return Err(VmError::TypeError(
                        "getComputedStyle requires an Element".to_string(),
                    ));
                };
                Ok(self.make_host_object(HostObjectSlot {
                    class: HostObjectClass::Other("ComputedStyle"),
                    interface_name: "CSSStyleDeclaration",
                    handle: node_id.0 as u64,
                    dispatch: HostDispatch::Ordinary,
                    supports_indexed_properties: false,
                    supports_named_properties: true,
                }))
            }
            BuiltinId::WindowMatchMedia => {
                let result_obj = self.allocate_ordinary_object(None);
                self.define_data_property(result_obj, PropertyKey::from("matches"), Value::Bool(false), true, true, true);
                let media = args.first().map(|v| self.to_string(v)).unwrap_or_default();
                let media_value = self.make_string_value(&media);
                self.define_data_property(result_obj, PropertyKey::from("media"), media_value, true, true, true);
                self.define_data_property(result_obj, PropertyKey::from("onchange"), Value::Null, true, true, true);
                self.define_builtin_method(result_obj, "addEventListener", BuiltinId::Noop);
                self.define_builtin_method(result_obj, "removeEventListener", BuiltinId::Noop);
                self.define_builtin_method(result_obj, "addListener", BuiltinId::Noop);
                self.define_builtin_method(result_obj, "removeListener", BuiltinId::Noop);
                Ok(Value::Object(result_obj))
            }
        }
    }

    fn number_arg(&self, args: &[Value], index: usize) -> f64 {
        args.get(index)
            .map(|value| self.to_number(value))
            .unwrap_or(f64::NAN)
    }

    fn string_arg(&self, args: &[Value], index: usize) -> String {
        args.get(index)
            .map(|value| self.to_string(value))
            .unwrap_or_default()
    }

    fn url_search_params_prototype_ref(&self) -> GcRef<JsObject> {
        self.url_search_params_prototype
            .expect("URLSearchParams prototype should be installed")
    }

    fn headers_prototype_ref(&self) -> GcRef<JsObject> {
        self.headers_prototype
            .expect("Headers prototype should be installed")
    }

    fn form_data_prototype_ref(&self) -> GcRef<JsObject> {
        self.form_data_prototype
            .expect("FormData prototype should be installed")
    }

    fn url_prototype_ref(&self) -> GcRef<JsObject> {
        self.url_prototype.expect("URL prototype should be installed")
    }

    /// Read the (name, value) pairs of a URLSearchParams `this`.
    fn usp_pairs(&self, this_value: &Value) -> Result<Vec<(String, String)>, VmError> {
        if let Value::Object(object) = this_value {
            if let Some(JsObject {
                kind: ObjectKind::UrlSearchParams(pairs),
                ..
            }) = self.heap.objects().get(*object)
            {
                return Ok(pairs.clone());
            }
        }
        Err(VmError::TypeError(
            "method called on a non-URLSearchParams object".to_string(),
        ))
    }

    fn usp_set_pairs(&mut self, this_value: &Value, pairs: Vec<(String, String)>) {
        if let Value::Object(object) = this_value {
            if let Some(data) = self.heap.objects_mut().get_mut(*object) {
                data.kind = ObjectKind::UrlSearchParams(pairs);
            }
        }
    }

    fn headers_name_arg(&self, args: &[Value], index: usize) -> String {
        self.string_arg(args, index).to_lowercase()
    }

    fn headers_pairs(&self, this_value: &Value) -> Result<Vec<(String, String)>, VmError> {
        if let Value::Object(object) = this_value {
            if let Some(JsObject { kind: ObjectKind::Headers(pairs), .. }) =
                self.heap.objects().get(*object)
            {
                return Ok(pairs.clone());
            }
        }
        Err(VmError::TypeError(
            "method called on a non-Headers object".to_string(),
        ))
    }

    fn headers_set_pairs(&mut self, this_value: &Value, pairs: Vec<(String, String)>) {
        if let Value::Object(object) = this_value {
            if let Some(data) = self.heap.objects_mut().get_mut(*object) {
                data.kind = ObjectKind::Headers(pairs);
            }
        }
    }

    fn headers_init_pairs(&mut self, init: Option<&Value>) -> Result<Vec<(String, String)>, VmError> {
        match init {
            None | Some(Value::Undefined) | Some(Value::Null) => Ok(Vec::new()),
            Some(Value::Object(object)) => {
                let object = *object;
                let kind = self.heap.objects().get(object).map(|o| &o.kind);
                let kind_is_headers = matches!(kind, Some(ObjectKind::Headers(_)));
                let kind_is_usp = matches!(kind, Some(ObjectKind::UrlSearchParams(_)));
                let is_array = matches!(kind, Some(ObjectKind::Array));
                if kind_is_headers || kind_is_usp {
                    let mut pairs = Vec::new();
                    for (name, value) in self.headers_like_pairs(Value::Object(object))? {
                        pairs.push((name.to_lowercase(), value));
                    }
                    Ok(pairs)
                } else if is_array {
                    let entries = self.for_of_values(&Value::Object(object))?;
                    let mut pairs = Vec::with_capacity(entries.len());
                    for entry in entries {
                        let name = self.get_property_value(&entry, &PropertyKey::Index(0))?;
                        let val = self.get_property_value(&entry, &PropertyKey::Index(1))?;
                        pairs.push((self.to_lowercase_string(&name), self.to_string(&val)));
                    }
                    Ok(pairs)
                } else {
                    let mut pairs = Vec::new();
                    for key in self.object_own_enumerable_keys(object) {
                        let val = self.get_property_value(&Value::Object(object), &key)?;
                        pairs.push((
                            self.property_key_to_string(&key).to_lowercase(),
                            self.to_string(&val),
                        ));
                    }
                    Ok(pairs)
                }
            }
            Some(other) => {
                let text = self.to_string(other);
                let mut pairs = Vec::new();
                for (name, value) in parse_query_string(&text) {
                    pairs.push((name.to_lowercase(), value));
                }
                Ok(pairs)
            }
        }
    }

    fn form_data_pairs(&self, this_value: &Value) -> Result<Vec<(String, String)>, VmError> {
        if let Value::Object(object) = this_value {
            if let Some(JsObject {
                kind: ObjectKind::FormData(pairs),
                ..
            }) = self.heap.objects().get(*object)
            {
                return Ok(pairs.clone());
            }
        }
        Err(VmError::TypeError(
            "method called on a non-FormData object".to_string(),
        ))
    }

    fn form_data_set_pairs(&mut self, this_value: &Value, pairs: Vec<(String, String)>) {
        if let Value::Object(object) = this_value {
            if let Some(data) = self.heap.objects_mut().get_mut(*object) {
                data.kind = ObjectKind::FormData(pairs);
            }
        }
    }

    fn headers_like_pairs(&self, this_value: Value) -> Result<Vec<(String, String)>, VmError> {
        match this_value {
            Value::Object(object) => {
                if let Some(JsObject { kind: ObjectKind::Headers(pairs), .. }) =
                    self.heap.objects().get(object)
                {
                    return Ok(pairs.clone());
                }
                if let Some(JsObject {
                    kind: ObjectKind::UrlSearchParams(pairs),
                    ..
                }) = self.heap.objects().get(object)
                {
                    return Ok(pairs.clone());
                }
                Err(VmError::TypeError(
                    "method called on an unsupported headers init object".to_string(),
                ))
            }
            _ => Err(VmError::TypeError(
                "method called on an unsupported headers init value".to_string(),
            )),
        }
    }

    fn to_lowercase_string(&self, value: &Value) -> String {
        self.to_string(value).to_lowercase()
    }

    /// Allocate a `ForOfIterator` object wrapping the given values. Used by
    /// Array.prototype.keys/values/entries; iterable via for-of and spread.
    fn make_for_of_iterator(&mut self, values: Vec<Value>) -> Value {
        let iterator = self.heap.allocate_object(JsObject {
            kind: ObjectKind::ForOfIterator { values, index: 0 },
            prototype: Some(self.object_prototype_ref()),
            ..JsObject::default()
        });
        Value::Object(iterator)
    }

    /// Deep clone for `structuredClone` (acyclic data: objects, arrays, Map,
    /// Set, Date, RegExp; primitives are copied directly).
    fn structured_clone(&mut self, value: &Value, depth: usize) -> Result<Value, VmError> {
        if depth > 1000 {
            return Err(VmError::RangeError(
                "structuredClone: structure too deep (or cyclic)".to_string(),
            ));
        }
        let Value::Object(object) = value else {
            return Ok(value.clone());
        };
        let object = *object;
        // Callables cannot be cloned.
        if self.callables.contains_key(&object.raw()) {
            return Err(VmError::TypeError(
                "structuredClone: cannot clone a function".to_string(),
            ));
        }
        let kind = self
            .heap
            .objects()
            .get(object)
            .map(|o| o.kind.clone())
            .unwrap_or(ObjectKind::Ordinary);
        match kind {
            ObjectKind::Array => {
                let elements = self.array_like_to_vec(value)?;
                let mut cloned = Vec::with_capacity(elements.len());
                for element in &elements {
                    cloned.push(self.structured_clone(element, depth + 1)?);
                }
                self.make_array_from_values(cloned)
            }
            ObjectKind::Map(entries) => {
                let new_map = self.heap.allocate_object(JsObject {
                    kind: ObjectKind::Map(Vec::new()),
                    prototype: Some(self.map_prototype_ref()),
                    ..JsObject::default()
                });
                self.set_collection_size(new_map, 0);
                for (key, value) in entries {
                    let key = self.structured_clone(&key, depth + 1)?;
                    let value = self.structured_clone(&value, depth + 1)?;
                    self.map_set(new_map, key, value, false)?;
                }
                Ok(Value::Object(new_map))
            }
            ObjectKind::Set(values) => {
                let new_set = self.heap.allocate_object(JsObject {
                    kind: ObjectKind::Set(Vec::new()),
                    prototype: Some(self.set_prototype_ref()),
                    ..JsObject::default()
                });
                self.set_collection_size(new_set, 0);
                for value in values {
                    let value = self.structured_clone(&value, depth + 1)?;
                    self.set_add(new_set, value, false)?;
                }
                Ok(Value::Object(new_set))
            }
            ObjectKind::RegExp { source, flags, .. } => Ok(self.make_regexp_object(&source, &flags)),
            _ => {
                // Plain object: clone own enumerable string/index properties.
                let new_object = self.allocate_ordinary_object(Some(self.object_prototype_ref()));
                for key in self.object_own_enumerable_keys(object) {
                    let property = self.get_property_value(&Value::Object(object), &key)?;
                    let cloned = self.structured_clone(&property, depth + 1)?;
                    self.set_property_on_object(new_object, Value::Object(new_object), key, cloned)?;
                }
                Ok(Value::Object(new_object))
            }
        }
    }

    /// SameValue: like strict equality but NaN equals NaN and +0 differs from -0.
    fn same_value(&self, a: &Value, b: &Value) -> bool {
        match (a, b) {
            (Value::Number(x), Value::Number(y)) => {
                if x.is_nan() && y.is_nan() {
                    true
                } else if *x == 0.0 && *y == 0.0 {
                    x.is_sign_negative() == y.is_sign_negative()
                } else {
                    x == y
                }
            }
            _ => self.strict_equal(a, b),
        }
    }

    /// Sort `values` in place (Array.prototype.sort / toSorted). Uses the given
    /// comparator, or default lexicographic-by-string order.
    fn sort_values(
        &mut self,
        values: &mut [Value],
        compare_fn: Option<&Value>,
    ) -> Result<(), VmError> {
        let comparator = compare_fn
            .filter(|f| !matches!(f, Value::Undefined))
            .cloned();
        if let Some(compare_fn) = comparator {
            // Stable insertion sort: equal elements (comparator <= 0) never swap,
            // so their relative order is preserved (required since ES2019).
            let len = values.len();
            for i in 1..len {
                let mut j = i;
                while j > 0 {
                    let result = self.call_value_sync(
                        compare_fn.clone(),
                        Value::Undefined,
                        vec![values[j - 1].clone(), values[j].clone()],
                    )?;
                    if self.to_number(&result) > 0.0 {
                        values.swap(j - 1, j);
                        j -= 1;
                    } else {
                        break;
                    }
                }
            }
        } else {
            // Default order compares the string form of each element.
            let mut keyed: Vec<(String, Value)> = values
                .iter()
                .map(|value| (self.to_string(value), value.clone()))
                .collect();
            keyed.sort_by(|a, b| a.0.cmp(&b.0));
            for (slot, (_, value)) in values.iter_mut().zip(keyed.into_iter()) {
                *slot = value;
            }
        }
        Ok(())
    }

    /// Recursively flatten array values up to `depth` levels (Array.prototype.flat).
    fn flatten_values(&mut self, values: Vec<Value>, depth: usize) -> Result<Vec<Value>, VmError> {
        let mut out = Vec::new();
        for value in values {
            let is_array = matches!(
                value,
                Value::Object(object)
                    if self.heap.objects().get(object).map(|o| o.kind == ObjectKind::Array).unwrap_or(false)
            );
            if depth > 0 && is_array {
                let inner = self.array_like_to_vec(&value)?;
                let flattened = self.flatten_values(inner, depth - 1)?;
                out.extend(flattened);
            } else {
                out.push(value);
            }
        }
        Ok(out)
    }

    fn next_random(&mut self) -> f64 {
        self.random_state = self
            .random_state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1);
        ((self.random_state >> 11) as f64) / ((1u64 << 53) as f64)
    }

    fn freeze_object(&mut self, object: GcRef<JsObject>) {
        if let Some(object_data) = self.heap.objects_mut().get_mut(object) {
            object_data.extensible = false;
            for descriptor in object_data.properties.values_mut() {
                match descriptor {
                    JsPropertyDescriptor::Data {
                        writable,
                        configurable,
                        ..
                    } => {
                        *writable = false;
                        *configurable = false;
                    }
                    JsPropertyDescriptor::Accessor { configurable, .. } => {
                        *configurable = false;
                    }
                }
            }
        }
    }

    fn is_frozen(&self, object: GcRef<JsObject>) -> bool {
        let Some(object_data) = self.heap.objects().get(object) else {
            return false;
        };
        !object_data.extensible
            && object_data
                .properties
                .values()
                .all(|descriptor| match descriptor {
                    JsPropertyDescriptor::Data {
                        writable,
                        configurable,
                        ..
                    } => !*writable && !*configurable,
                    JsPropertyDescriptor::Accessor { configurable, .. } => !*configurable,
                })
    }

    fn value_to_property_descriptor(
        &mut self,
        value: &Value,
    ) -> Result<JsPropertyDescriptor, VmError> {
        let object = self.require_object_ref(value, "property descriptor")?;
        let get = self.get_property_value(&Value::Object(object), &PropertyKey::from("get"))?;
        let set = self.get_property_value(&Value::Object(object), &PropertyKey::from("set"))?;
        if !matches!(get, Value::Undefined) || !matches!(set, Value::Undefined) {
            let get = match get {
                Value::Object(object) => Some(object),
                Value::Undefined => None,
                _ => {
                    return Err(VmError::TypeError(
                        "descriptor getter must be a function".to_string(),
                    ));
                }
            };
            let set = match set {
                Value::Object(object) => Some(object),
                Value::Undefined => None,
                _ => {
                    return Err(VmError::TypeError(
                        "descriptor setter must be a function".to_string(),
                    ));
                }
            };
            let enumerable_value =
                self.get_property_value(&Value::Object(object), &PropertyKey::from("enumerable"))?;
            let configurable_value = self
                .get_property_value(&Value::Object(object), &PropertyKey::from("configurable"))?;
            let enumerable = self.is_truthy(&enumerable_value);
            let configurable = self.is_truthy(&configurable_value);
            return Ok(JsPropertyDescriptor::Accessor {
                get,
                set,
                enumerable,
                configurable,
            });
        }

        let value = self.get_property_value(&Value::Object(object), &PropertyKey::from("value"))?;
        let writable_value =
            self.get_property_value(&Value::Object(object), &PropertyKey::from("writable"))?;
        let enumerable_value =
            self.get_property_value(&Value::Object(object), &PropertyKey::from("enumerable"))?;
        let configurable_value =
            self.get_property_value(&Value::Object(object), &PropertyKey::from("configurable"))?;
        let writable = self.is_truthy(&writable_value);
        let enumerable = self.is_truthy(&enumerable_value);
        let configurable = self.is_truthy(&configurable_value);
        Ok(JsPropertyDescriptor::Data {
            value,
            writable,
            enumerable,
            configurable,
        })
    }

    fn property_descriptor_to_value(
        &mut self,
        descriptor: JsPropertyDescriptor,
    ) -> Result<Value, VmError> {
        let object = self.allocate_ordinary_object(Some(self.object_prototype_ref()));
        match descriptor {
            JsPropertyDescriptor::Data {
                value,
                writable,
                enumerable,
                configurable,
            } => {
                self.define_data_property(
                    object,
                    PropertyKey::from("value"),
                    value,
                    true,
                    true,
                    true,
                );
                self.define_data_property(
                    object,
                    PropertyKey::from("writable"),
                    Value::Bool(writable),
                    true,
                    true,
                    true,
                );
                self.define_data_property(
                    object,
                    PropertyKey::from("enumerable"),
                    Value::Bool(enumerable),
                    true,
                    true,
                    true,
                );
                self.define_data_property(
                    object,
                    PropertyKey::from("configurable"),
                    Value::Bool(configurable),
                    true,
                    true,
                    true,
                );
            }
            JsPropertyDescriptor::Accessor {
                get,
                set,
                enumerable,
                configurable,
            } => {
                self.define_data_property(
                    object,
                    PropertyKey::from("get"),
                    get.map(Value::Object).unwrap_or(Value::Undefined),
                    true,
                    true,
                    true,
                );
                self.define_data_property(
                    object,
                    PropertyKey::from("set"),
                    set.map(Value::Object).unwrap_or(Value::Undefined),
                    true,
                    true,
                    true,
                );
                self.define_data_property(
                    object,
                    PropertyKey::from("enumerable"),
                    Value::Bool(enumerable),
                    true,
                    true,
                    true,
                );
                self.define_data_property(
                    object,
                    PropertyKey::from("configurable"),
                    Value::Bool(configurable),
                    true,
                    true,
                    true,
                );
            }
        }
        Ok(Value::Object(object))
    }

    fn replace_array_contents(
        &mut self,
        object: GcRef<JsObject>,
        values: Vec<Value>,
    ) -> Result<(), VmError> {
        let new_length = values.len() as u32;
        if let Some(object_data) = self.heap.objects_mut().get_mut(object) {
            let keys = object_data.properties.keys().cloned().collect::<Vec<_>>();
            for key in keys {
                if matches!(key, PropertyKey::Index(_)) {
                    let _ = object_data.properties.shift_remove(&key);
                }
            }
        }
        for (index, value) in values.into_iter().enumerate() {
            self.set_property_on_object(
                object,
                Value::Object(object),
                PropertyKey::Index(index as u32),
                value,
            )?;
        }
        // Set the length explicitly: index assignments above only ever grow the
        // length, so a shrink (e.g. splice removing elements) must be applied here.
        self.set_array_length(object, new_length);
        Ok(())
    }

    fn set_collection_size(&mut self, object: GcRef<JsObject>, size: usize) {
        self.define_data_property(
            object,
            PropertyKey::from("size"),
            Value::Number(size as f64),
            false,
            false,
            true,
        );
    }

    fn map_set(
        &mut self,
        object: GcRef<JsObject>,
        key: Value,
        value: Value,
        weak: bool,
    ) -> Result<(), VmError> {
        let existing_index =
            self.heap
                .objects()
                .get(object)
                .and_then(|object_data| match &object_data.kind {
                    ObjectKind::Map(entries) => entries
                        .iter()
                        .position(|(existing_key, _)| self.same_value_zero(existing_key, &key)),
                    ObjectKind::WeakMap(entries) if weak => entries
                        .iter()
                        .position(|(existing_key, _)| self.same_value_zero(existing_key, &key)),
                    _ => None,
                });
        let Some(object_data) = self.heap.objects_mut().get_mut(object) else {
            return Err(VmError::TypeError("invalid Map object".to_string()));
        };
        let entries = match &mut object_data.kind {
            ObjectKind::Map(entries) => entries,
            ObjectKind::WeakMap(entries) if weak => entries,
            _ => return Err(VmError::TypeError("object is not a Map".to_string())),
        };
        if let Some(index) = existing_index {
            entries[index].1 = value;
        } else {
            entries.push((key, value));
        }
        let size = entries.len();
        let _ = object_data;
        self.set_collection_size(object, size);
        Ok(())
    }

    fn map_get(&self, object: GcRef<JsObject>, key: &Value) -> Result<Option<Value>, VmError> {
        let Some(object_data) = self.heap.objects().get(object) else {
            return Err(VmError::TypeError("invalid Map object".to_string()));
        };
        let entries = match &object_data.kind {
            ObjectKind::Map(entries) | ObjectKind::WeakMap(entries) => entries,
            _ => return Err(VmError::TypeError("object is not a Map".to_string())),
        };
        Ok(entries
            .iter()
            .find(|(existing_key, _)| self.same_value_zero(existing_key, key))
            .map(|(_, value)| value.clone()))
    }

    fn map_delete(&mut self, object: GcRef<JsObject>, key: &Value) -> Result<bool, VmError> {
        let delete_index =
            self.heap
                .objects()
                .get(object)
                .and_then(|object_data| match &object_data.kind {
                    ObjectKind::Map(entries) | ObjectKind::WeakMap(entries) => entries
                        .iter()
                        .position(|(existing_key, _)| self.same_value_zero(existing_key, key)),
                    _ => None,
                });
        let Some(object_data) = self.heap.objects_mut().get_mut(object) else {
            return Err(VmError::TypeError("invalid Map object".to_string()));
        };
        let entries = match &mut object_data.kind {
            ObjectKind::Map(entries) | ObjectKind::WeakMap(entries) => entries,
            _ => return Err(VmError::TypeError("object is not a Map".to_string())),
        };
        let deleted = if let Some(index) = delete_index {
            entries.remove(index);
            true
        } else {
            false
        };
        let size = entries.len();
        let _ = object_data;
        self.set_collection_size(object, size);
        Ok(deleted)
    }

    fn map_clear(&mut self, object: GcRef<JsObject>) -> Result<(), VmError> {
        let Some(object_data) = self.heap.objects_mut().get_mut(object) else {
            return Err(VmError::TypeError("invalid Map object".to_string()));
        };
        match &mut object_data.kind {
            ObjectKind::Map(entries) | ObjectKind::WeakMap(entries) => entries.clear(),
            _ => return Err(VmError::TypeError("object is not a Map".to_string())),
        }
        let _ = object_data;
        self.set_collection_size(object, 0);
        Ok(())
    }

    fn set_add(
        &mut self,
        object: GcRef<JsObject>,
        value: Value,
        weak: bool,
    ) -> Result<(), VmError> {
        let exists = self
            .heap
            .objects()
            .get(object)
            .and_then(|object_data| match &object_data.kind {
                ObjectKind::Set(values) => Some(
                    values
                        .iter()
                        .any(|existing| self.same_value_zero(existing, &value)),
                ),
                ObjectKind::WeakSet(values) if weak => Some(
                    values
                        .iter()
                        .any(|existing| self.same_value_zero(existing, &value)),
                ),
                _ => None,
            })
            .unwrap_or(false);
        let Some(object_data) = self.heap.objects_mut().get_mut(object) else {
            return Err(VmError::TypeError("invalid Set object".to_string()));
        };
        let values = match &mut object_data.kind {
            ObjectKind::Set(values) => values,
            ObjectKind::WeakSet(values) if weak => values,
            _ => return Err(VmError::TypeError("object is not a Set".to_string())),
        };
        if !exists {
            values.push(value);
        }
        let size = values.len();
        let _ = object_data;
        self.set_collection_size(object, size);
        Ok(())
    }

    fn set_has(&self, object: GcRef<JsObject>, value: &Value) -> Result<bool, VmError> {
        let Some(object_data) = self.heap.objects().get(object) else {
            return Err(VmError::TypeError("invalid Set object".to_string()));
        };
        let values = match &object_data.kind {
            ObjectKind::Set(values) | ObjectKind::WeakSet(values) => values,
            _ => return Err(VmError::TypeError("object is not a Set".to_string())),
        };
        Ok(values
            .iter()
            .any(|existing| self.same_value_zero(existing, value)))
    }

    fn set_delete(&mut self, object: GcRef<JsObject>, value: &Value) -> Result<bool, VmError> {
        let delete_index =
            self.heap
                .objects()
                .get(object)
                .and_then(|object_data| match &object_data.kind {
                    ObjectKind::Set(values) | ObjectKind::WeakSet(values) => values
                        .iter()
                        .position(|existing| self.same_value_zero(existing, value)),
                    _ => None,
                });
        let Some(object_data) = self.heap.objects_mut().get_mut(object) else {
            return Err(VmError::TypeError("invalid Set object".to_string()));
        };
        let values = match &mut object_data.kind {
            ObjectKind::Set(values) | ObjectKind::WeakSet(values) => values,
            _ => return Err(VmError::TypeError("object is not a Set".to_string())),
        };
        let deleted = if let Some(index) = delete_index {
            values.remove(index);
            true
        } else {
            false
        };
        let size = values.len();
        let _ = object_data;
        self.set_collection_size(object, size);
        Ok(deleted)
    }

    fn set_clear(&mut self, object: GcRef<JsObject>) -> Result<(), VmError> {
        let Some(object_data) = self.heap.objects_mut().get_mut(object) else {
            return Err(VmError::TypeError("invalid Set object".to_string()));
        };
        match &mut object_data.kind {
            ObjectKind::Set(values) | ObjectKind::WeakSet(values) => values.clear(),
            _ => return Err(VmError::TypeError("object is not a Set".to_string())),
        }
        let _ = object_data;
        self.set_collection_size(object, 0);
        Ok(())
    }

    fn normalize_slice_bounds(
        &self,
        length: usize,
        start: Option<&Value>,
        end: Option<&Value>,
    ) -> (usize, usize) {
        let start = Self::normalize_index(
            length,
            start.map(|value| self.to_number(value)).unwrap_or(0.0),
        );
        let end = Self::normalize_index(
            length,
            end.map(|value| self.to_number(value))
                .unwrap_or(length as f64),
        );
        (start.min(length), end.min(length).max(start.min(length)))
    }

    fn normalize_index(length: usize, value: f64) -> usize {
        if value.is_nan() {
            return 0;
        }
        if value < 0.0 {
            length.saturating_sub((-value) as usize)
        } else {
            value as usize
        }
    }

    fn array_callback_map(
        &mut self,
        this_value: &Value,
        args: Vec<Value>,
    ) -> Result<Value, VmError> {
        let callback = args.first().cloned().ok_or_else(|| {
            VmError::TypeError("Array.prototype.map requires a callback".to_string())
        })?;
        let values = self.array_like_to_vec(this_value)?;
        let mut mapped = Vec::with_capacity(values.len());
        for (index, value) in values.iter().cloned().enumerate() {
            mapped.push(self.call_value_sync(
                callback.clone(),
                Value::Undefined,
                vec![value, Value::Number(index as f64), this_value.clone()],
            )?);
        }
        self.make_array_from_values(mapped)
    }

    fn array_callback_filter(
        &mut self,
        this_value: &Value,
        args: Vec<Value>,
    ) -> Result<Value, VmError> {
        let callback = args.first().cloned().ok_or_else(|| {
            VmError::TypeError("Array.prototype.filter requires a callback".to_string())
        })?;
        let values = self.array_like_to_vec(this_value)?;
        let mut filtered = Vec::new();
        for (index, value) in values.iter().cloned().enumerate() {
            let keep = self.call_value_sync(
                callback.clone(),
                Value::Undefined,
                vec![
                    value.clone(),
                    Value::Number(index as f64),
                    this_value.clone(),
                ],
            )?;
            if self.is_truthy(&keep) {
                filtered.push(value);
            }
        }
        self.make_array_from_values(filtered)
    }

    fn array_callback_reduce(
        &mut self,
        this_value: &Value,
        args: Vec<Value>,
    ) -> Result<Value, VmError> {
        let callback = args.first().cloned().ok_or_else(|| {
            VmError::TypeError("Array.prototype.reduce requires a callback".to_string())
        })?;
        let values = self.array_like_to_vec(this_value)?;
        let mut iter = values.into_iter().enumerate();
        let mut accumulator = if let Some(initial) = args.get(1).cloned() {
            initial
        } else if let Some((_, first)) = iter.next() {
            first
        } else {
            return Err(VmError::TypeError(
                "reduce of empty array with no initial value".to_string(),
            ));
        };
        for (index, value) in iter {
            accumulator = self.call_value_sync(
                callback.clone(),
                Value::Undefined,
                vec![
                    accumulator,
                    value,
                    Value::Number(index as f64),
                    this_value.clone(),
                ],
            )?;
        }
        Ok(accumulator)
    }

    fn array_callback_for_each(
        &mut self,
        this_value: &Value,
        args: Vec<Value>,
    ) -> Result<Value, VmError> {
        let callback = args.first().cloned().ok_or_else(|| {
            VmError::TypeError("Array.prototype.forEach requires a callback".to_string())
        })?;
        let values = self.array_like_to_vec(this_value)?;
        for (index, value) in values.iter().cloned().enumerate() {
            let _ = self.call_value_sync(
                callback.clone(),
                Value::Undefined,
                vec![value, Value::Number(index as f64), this_value.clone()],
            )?;
        }
        Ok(Value::Undefined)
    }

    fn array_callback_find(
        &mut self,
        this_value: &Value,
        args: Vec<Value>,
        index_only: bool,
    ) -> Result<Value, VmError> {
        let callback = args
            .first()
            .cloned()
            .ok_or_else(|| VmError::TypeError("array search requires a callback".to_string()))?;
        let values = self.array_like_to_vec(this_value)?;
        for (index, value) in values.iter().cloned().enumerate() {
            let matched = self.call_value_sync(
                callback.clone(),
                Value::Undefined,
                vec![
                    value.clone(),
                    Value::Number(index as f64),
                    this_value.clone(),
                ],
            )?;
            if self.is_truthy(&matched) {
                return Ok(if index_only {
                    Value::Number(index as f64)
                } else {
                    value
                });
            }
        }
        Ok(if index_only {
            Value::Number(-1.0)
        } else {
            Value::Undefined
        })
    }

    fn array_callback_predicate(
        &mut self,
        this_value: &Value,
        args: Vec<Value>,
        any: bool,
    ) -> Result<Value, VmError> {
        let callback = args
            .first()
            .cloned()
            .ok_or_else(|| VmError::TypeError("array predicate requires a callback".to_string()))?;
        let values = self.array_like_to_vec(this_value)?;
        for (index, value) in values.iter().cloned().enumerate() {
            let matched = self.call_value_sync(
                callback.clone(),
                Value::Undefined,
                vec![value, Value::Number(index as f64), this_value.clone()],
            )?;
            let truthy = self.is_truthy(&matched);
            if any && truthy {
                return Ok(Value::Bool(true));
            }
            if !any && !truthy {
                return Ok(Value::Bool(false));
            }
        }
        Ok(Value::Bool(!any))
    }

    fn to_json_value(
        &mut self,
        key: &str,
        value: &Value,
        replacer: Option<&Value>,
    ) -> Result<Option<JsonValue>, VmError> {
        // Apply a `toJSON` method, then a replacer function, before serializing.
        let mut value = value.clone();
        if let Value::Object(_) = &value {
            let to_json = self.get_property_value(&value, &PropertyKey::from("toJSON"))?;
            if self.is_callable_value(&to_json) {
                let key_value = self.make_string_value(key);
                value = self.call_value_sync(to_json, value.clone(), vec![key_value])?;
            }
        }
        if let Some(replacer) = replacer {
            if self.is_callable_value(replacer) {
                let key_value = self.make_string_value(key);
                value = self.call_value_sync(
                    replacer.clone(),
                    Value::Undefined,
                    vec![key_value, value],
                )?;
            }
        }

        Ok(match &value {
            Value::Undefined | Value::Symbol(_) => None,
            Value::Null => Some(JsonValue::Null),
            Value::Bool(boolean) => Some(JsonValue::Bool(*boolean)),
            Value::Number(number) => {
                let number = *number;
                if !number.is_finite() {
                    // NaN and ±Infinity stringify to null per the JSON grammar.
                    Some(JsonValue::Null)
                } else if number.fract() == 0.0
                    && number >= i64::MIN as f64
                    && number <= i64::MAX as f64
                {
                    // Preserve integers as integers so they don't serialize as "1.0".
                    Some(JsonValue::Number(serde_json::Number::from(number as i64)))
                } else {
                    serde_json::Number::from_f64(number).map(JsonValue::Number)
                }
            }
            Value::String(string) => Some(JsonValue::String(self.string_text(*string))),
            Value::Object(object) => {
                let object = *object;
                if self.callables.contains_key(&object.raw()) {
                    return Ok(None);
                }
                let is_array = matches!(
                    self.heap.objects().get(object).map(|o| &o.kind),
                    Some(ObjectKind::Array)
                );
                if is_array {
                    let elements = self.array_like_to_vec(&Value::Object(object))?;
                    let mut items = Vec::with_capacity(elements.len());
                    for (index, element) in elements.iter().enumerate() {
                        items.push(
                            self.to_json_value(&index.to_string(), element, replacer)?
                                .unwrap_or(JsonValue::Null),
                        );
                    }
                    Some(JsonValue::Array(items))
                } else {
                    let mut map = serde_json::Map::new();
                    for property_key in self.object_own_enumerable_keys(object) {
                        let property_value =
                            self.get_property_value(&Value::Object(object), &property_key)?;
                        let key_string = self.property_key_to_string(&property_key);
                        if let Some(json) =
                            self.to_json_value(&key_string, &property_value, replacer)?
                        {
                            map.insert(key_string, json);
                        }
                    }
                    Some(JsonValue::Object(map))
                }
            }
        })
    }

    fn from_json_value(&mut self, value: &JsonValue) -> Result<Value, VmError> {
        match value {
            JsonValue::Null => Ok(Value::Null),
            JsonValue::Bool(boolean) => Ok(Value::Bool(*boolean)),
            JsonValue::Number(number) => Ok(Value::Number(number.as_f64().unwrap_or(f64::NAN))),
            JsonValue::String(text) => Ok(self.make_string_value(text)),
            JsonValue::Array(values) => {
                let mut converted = Vec::with_capacity(values.len());
                for value in values {
                    converted.push(self.from_json_value(value)?);
                }
                self.make_array_from_values(converted)
            }
            JsonValue::Object(entries) => {
                let object = self.allocate_ordinary_object(Some(self.object_prototype_ref()));
                for (key, value) in entries {
                    let converted = self.from_json_value(value)?;
                    self.set_property_on_object(
                        object,
                        Value::Object(object),
                        PropertyKey::from(key.as_str()),
                        converted,
                    )?;
                }
                Ok(Value::Object(object))
            }
        }
    }

    /// JSON.parse reviver walk (InternalizeJSONProperty): recurse into children
    /// first, then call `reviver(key, value)`; an `undefined` result deletes the
    /// property.
    fn internalize_json_property(
        &mut self,
        holder: GcRef<JsObject>,
        key: &str,
        reviver: &Value,
    ) -> Result<Value, VmError> {
        let value = self.get_property_value(&Value::Object(holder), &PropertyKey::from(key))?;
        if let Value::Object(object) = &value {
            let object = *object;
            let is_array = matches!(
                self.heap.objects().get(object).map(|o| &o.kind),
                Some(ObjectKind::Array)
            );
            if is_array {
                let length = self.array_length(object);
                for index in 0..length {
                    let element =
                        self.internalize_json_property(object, &index.to_string(), reviver)?;
                    if matches!(element, Value::Undefined) {
                        self.delete_property(object, &PropertyKey::Index(index));
                    } else {
                        self.set_property_on_object(
                            object,
                            Value::Object(object),
                            PropertyKey::Index(index),
                            element,
                        )?;
                    }
                }
            } else {
                for property_key in self.object_own_enumerable_keys(object) {
                    let key_string = self.property_key_to_string(&property_key);
                    let new_value =
                        self.internalize_json_property(object, &key_string, reviver)?;
                    if matches!(new_value, Value::Undefined) {
                        self.delete_property(object, &property_key);
                    } else {
                        self.set_property_on_object(
                            object,
                            Value::Object(object),
                            property_key,
                            new_value,
                        )?;
                    }
                }
            }
        }
        let key_value = self.make_string_value(key);
        self.call_value_sync(reviver.clone(), Value::Object(holder), vec![key_value, value])
    }

    // ========================================================================
    // DOM / Host dispatch helpers
    // ========================================================================

    fn make_host_object(&mut self, slot: HostObjectSlot) -> Value {
        let obj = self.heap.allocate_object(JsObject {
            kind: ObjectKind::Host(slot),
            ..JsObject::default()
        });
        Value::Object(obj)
    }

    fn make_dom_node_value(&mut self, node_id: NodeId) -> Value {
        // Return the interned wrapper so the same node compares `===` equal and
        // keeps any expando properties across accesses.
        if let Some(existing) = self.node_wrappers.get(&node_id.0) {
            return Value::Object(*existing);
        }
        let value = self.make_host_object(HostObjectSlot {
            class: HostObjectClass::Node,
            interface_name: "Element",
            handle: node_id.0 as u64,
            dispatch: HostDispatch::Node,
            supports_indexed_properties: false,
            supports_named_properties: false,
        });
        if let Value::Object(object) = value {
            self.node_wrappers.insert(node_id.0, object);
        }
        value
    }

    /// Upgrade one element to its registered custom-element class: link the
    /// node wrapper's prototype to the class prototype (so `instanceof` and
    /// method lookup resolve) and fire `connectedCallback`.
    fn upgrade_custom_element(&mut self, node: NodeId, tag: &str) -> Result<(), VmError> {
        let Some(def) = self.custom_elements.get(tag).cloned() else {
            return Ok(());
        };
        let class_proto = match self.get_property_value(&def.class_value, &PropertyKey::from("prototype")) {
            Ok(Value::Object(proto)) => Some(proto),
            _ => None,
        };
        let wrapper = self.make_dom_node_value(node);
        if let (Value::Object(wrapper_ref), Some(proto)) = (&wrapper, class_proto) {
            if let Some(data) = self.heap.objects_mut().get_mut(*wrapper_ref) {
                data.prototype = Some(proto);
            }
        }
        // connectedCallback (looked up on the class prototype).
        let cb = self.get_property_value(&wrapper, &PropertyKey::from("connectedCallback"))?;
        if self.is_callable_value(&cb) {
            self.call_value_sync(cb, wrapper.clone(), Vec::new())?;
        }
        // Deliver attributeChangedCallback for already-present observed attrs.
        for attr in &def.observed {
            if let Ok(DomReadResult::String(value)) = self.host.read_dom(DomRead::Attribute {
                node,
                name: attr.clone(),
            }) {
                self.fire_attribute_changed_callback(node, &wrapper, attr, None, Some(&value))?;
            }
        }
        Ok(())
    }

    /// Fire a custom element's `attributeChangedCallback(name, old, new)` when
    /// the attribute is observed. `wrapper` is the node's JS object.
    fn fire_attribute_changed_callback(
        &mut self,
        node: NodeId,
        wrapper: &Value,
        attr_name: &str,
        old_value: Option<&str>,
        new_value: Option<&str>,
    ) -> Result<(), VmError> {
        let tag = self.get_node_name(node).to_ascii_lowercase();
        if tag.is_empty() {
            return Ok(());
        }
        let Some(def) = self.custom_elements.get(&tag).cloned() else {
            return Ok(());
        };
        if !def.observed.iter().any(|a| a == attr_name) {
            return Ok(());
        }
        let cb = self.get_property_value(wrapper, &PropertyKey::from("attributeChangedCallback"))?;
        if !self.is_callable_value(&cb) {
            return Ok(());
        }
        let name_v = self.make_string_value(attr_name);
        let old_v = old_value.map(|v| self.make_string_value(v)).unwrap_or(Value::Null);
        let new_v = new_value.map(|v| self.make_string_value(v)).unwrap_or(Value::Null);
        self.call_value_sync(cb, wrapper.clone(), vec![name_v, old_v, new_v])?;
        Ok(())
    }

    /// Wrap a node, but return the global `document` host object when the node
    /// is the document (so `getRootNode() === document` holds).
    fn root_node_value(&mut self, id: NodeId) -> Value {
        if matches!(
            self.host.read_dom(DomRead::NodeKind { node: id }),
            Ok(DomReadResult::Kind(NodeKind::Document))
        ) {
            if let Some(doc) = self.globals.get("document").cloned() {
                return doc;
            }
        }
        self.make_dom_node_value(id)
    }

    fn node_id_from_host_val(&self, value: &Value) -> Option<NodeId> {
        if let Value::Object(obj_ref) = value {
            if let Some(obj) = self.heap.objects().get(*obj_ref) {
                if let ObjectKind::Host(slot) = &obj.kind {
                    return Some(NodeId(slot.handle as u32));
                }
            }
        }
        None
    }

    fn this_node_id(&self, this_value: &Value) -> NodeId {
        self.node_id_from_host_val(this_value).unwrap_or(NodeId(0))
    }

    fn dom_child(&mut self, node_id: NodeId, elements_only: bool, last: bool) -> Value {
        match self.host.read_dom(DomRead::Children {
            node: node_id,
            elements_only,
        }) {
            Ok(DomReadResult::Nodes(ids)) => {
                let target = if last { ids.last() } else { ids.first() };
                target
                    .map(|&id| self.make_dom_node_value(id))
                    .unwrap_or(Value::Null)
            }
            _ => Value::Null,
        }
    }

    fn dom_sibling(
        &mut self,
        node_id: NodeId,
        direction: SiblingDirection,
        elements_only: bool,
    ) -> Value {
        match self.host.read_dom(DomRead::Sibling {
            node: node_id,
            direction,
            elements_only,
        }) {
            Ok(DomReadResult::Node(id)) => self.make_dom_node_value(id),
            _ => Value::Null,
        }
    }

    fn get_dom_attribute(&self, node_id: NodeId, name: &str) -> String {
        match self.host.read_dom(DomRead::Attribute {
            node: node_id,
            name: name.to_string(),
        }) {
            Ok(DomReadResult::String(s)) => s,
            _ => String::new(),
        }
    }

    fn get_node_name(&self, node_id: NodeId) -> String {
        match self.host.read_dom(DomRead::NodeName { node: node_id }) {
            Ok(DomReadResult::String(s)) => s,
            _ => String::new(),
        }
    }

    fn query_all_to_array(&mut self, root: NodeId, selectors: String) -> Result<Value, VmError> {
        match self.host.read_dom(DomRead::QuerySelectorAll { root, selectors }) {
            Ok(DomReadResult::Nodes(ids)) => {
                let items: Vec<Value> = ids.iter().map(|&id| self.make_dom_node_value(id)).collect();
                self.make_array_from_values(items)
            }
            _ => self.make_array_from_values(vec![]),
        }
    }

    /// Map `append`/`prepend`/`replaceChildren` arguments to node ids: DOM
    /// nodes pass through, anything else becomes a new text node (per spec).
    fn node_ids_from_node_or_string_args(&mut self, args: &[Value]) -> Vec<NodeId> {
        args.iter()
            .filter_map(|v| {
                if let Some(id) = self.node_id_from_host_val(v) {
                    return Some(id);
                }
                let data = self.to_string(v);
                match self.host.mutate_dom(DomMutation::CreateTextNode {
                    window: WindowId(0),
                    data,
                }) {
                    Ok(DomMutationResult::Node(id)) => Some(id),
                    _ => None,
                }
            })
            .collect()
    }

    fn get_host_property(
        &mut self,
        slot: HostObjectSlot,
        key: &PropertyKey,
    ) -> Result<Value, VmError> {
        // NamedNodeMap supports indexed access (`attrs[0]`), so it gets the
        // raw key before the string-only fast path below.
        if matches!(slot.class, HostObjectClass::Other("NamedNodeMap")) {
            return self.get_attrmap_property(slot, key);
        }
        let name = match key {
            PropertyKey::String(s) => s.clone(),
            _ => return Ok(Value::Undefined),
        };
        match slot.class {
            HostObjectClass::Window => self.get_window_property(name),
            HostObjectClass::Document => self.get_document_property(name),
            HostObjectClass::Node | HostObjectClass::EventTarget => {
                self.get_node_property(slot, name)
            }
            HostObjectClass::Other("TokenList") => self.get_classlist_property(slot, name),
            HostObjectClass::Other("CSSStyleDeclaration") => self.get_style_property(slot, name),
            HostObjectClass::Other("ComputedStyle") => {
                self.get_computed_style_object_property(slot, name)
            }
            HostObjectClass::Other("Dataset") => self.get_dataset_property(slot, name),
            HostObjectClass::Other("Location") => self.get_location_property(name),
            HostObjectClass::Other("History") => self.get_history_property(name),
            HostObjectClass::StorageArea => self.get_storage_property(slot, name),
            HostObjectClass::Observer => self.get_observer_property(name),
            _ => Ok(Value::Undefined),
        }
    }

    /// The attribute names of the node backing a NamedNodeMap host object.
    fn attrmap_names(&mut self, slot: &HostObjectSlot) -> Vec<String> {
        let node_id = NodeId(slot.handle as u32);
        match self.host.read_dom(DomRead::AttributeNames { node: node_id }) {
            Ok(DomReadResult::StringList(names)) => names,
            _ => Vec::new(),
        }
    }

    /// An `Attr`-like snapshot object (`{name, value}`) for one attribute.
    fn make_attr_value(&mut self, node: NodeId, name: &str) -> Value {
        let value = match self.host.read_dom(DomRead::Attribute {
            node,
            name: name.to_string(),
        }) {
            Ok(DomReadResult::String(s)) => s,
            _ => String::new(),
        };
        let obj = self.allocate_ordinary_object(None);
        let name_v = self.make_string_value(name);
        let value_v = self.make_string_value(&value);
        self.define_data_property(obj, PropertyKey::from("name"), name_v.clone(), true, true, true);
        self.define_data_property(obj, PropertyKey::from("localName"), name_v.clone(), true, true, true);
        self.define_data_property(obj, PropertyKey::from("nodeName"), name_v, true, true, true);
        self.define_data_property(obj, PropertyKey::from("value"), value_v, true, true, true);
        Value::Object(obj)
    }

    /// `element.attributes` (NamedNodeMap): length / item() / getNamedItem() /
    /// indexed and named access. Attr objects are read-only snapshots.
    fn get_attrmap_property(
        &mut self,
        slot: HostObjectSlot,
        key: &PropertyKey,
    ) -> Result<Value, VmError> {
        let node_id = NodeId(slot.handle as u32);
        match key {
            PropertyKey::Index(i) => {
                let names = self.attrmap_names(&slot);
                Ok(match names.get(*i as usize) {
                    Some(name) => {
                        let name = name.clone();
                        self.make_attr_value(node_id, &name)
                    }
                    None => Value::Undefined,
                })
            }
            PropertyKey::String(s) => match s.as_str() {
                "length" => Ok(Value::Number(self.attrmap_names(&slot).len() as f64)),
                "item" => Ok(self.allocate_builtin_method(BuiltinId::DomAttrMapItem)),
                "getNamedItem" => {
                    Ok(self.allocate_builtin_method(BuiltinId::DomAttrMapGetNamedItem))
                }
                name => {
                    let names = self.attrmap_names(&slot);
                    Ok(if names.iter().any(|n| n == name) {
                        self.make_attr_value(node_id, name)
                    } else {
                        Value::Undefined
                    })
                }
            },
            _ => Ok(Value::Undefined),
        }
    }

    /// Method lookup on a `MutationObserver` instance (`observe` / `disconnect`
    /// / `takeRecords`). Each returns a cached builtin method that reads the
    /// observer id from `this`'s host handle when invoked.
    fn get_observer_property(&mut self, name: String) -> Result<Value, VmError> {
        match name.as_str() {
            "observe" => Ok(self.allocate_builtin_method(BuiltinId::MutationObserverObserve)),
            "disconnect" => {
                Ok(self.allocate_builtin_method(BuiltinId::MutationObserverDisconnect))
            }
            "takeRecords" => {
                Ok(self.allocate_builtin_method(BuiltinId::MutationObserverTakeRecords))
            }
            // IntersectionObserver / ResizeObserver-only; observe/disconnect/takeRecords
            // above are kind-agnostic and shared with MutationObserver.
            "unobserve" => {
                Ok(self.allocate_builtin_method(BuiltinId::IntersectionObserverUnobserve))
            }
            _ => Ok(Value::Undefined),
        }
    }

    /// The `ObserverId` backing a `MutationObserver` JS instance (its host
    /// handle), if `this` is one.
    fn observer_id_from_this(&self, this_value: &Value) -> Option<u64> {
        if let Value::Object(obj_ref) = this_value {
            if let Some(obj) = self.heap.objects().get(*obj_ref) {
                if let ObjectKind::Host(slot) = &obj.kind {
                    if matches!(slot.class, HostObjectClass::Observer) {
                        return Some(slot.handle);
                    }
                }
            }
        }
        None
    }

    /// `new MutationObserver(callback)` — create the host-side observer, remember
    /// the callback + instance, and return the instance host object.
    fn mutation_observer_construct(&mut self, callback: Option<Value>) -> Result<Value, VmError> {
        let callback = callback.unwrap_or(Value::Undefined);
        if !self.is_callable_value(&callback) {
            return Err(VmError::TypeError(
                "MutationObserver constructor argument is not callable".to_string(),
            ));
        }
        let id = match self.host.observer(ObserverOp::Create {
            kind: ObserverKind::Mutation,
        }) {
            Ok(ObserverResult::Created(ObserverId(id))) => id,
            _ => {
                return Err(VmError::TypeError(
                    "host does not support MutationObserver".to_string(),
                ));
            }
        };
        let instance = self.make_host_object(HostObjectSlot {
            class: HostObjectClass::Observer,
            interface_name: "MutationObserver",
            handle: id,
            dispatch: HostDispatch::Ordinary,
            supports_indexed_properties: false,
            supports_named_properties: false,
        });
        self.mutation_observers.insert(
            id,
            MutationObserverReg {
                callback,
                instance: instance.clone(),
            },
        );
        Ok(instance)
    }

    fn make_abort_signal(&mut self, aborted: bool, reason: Value) -> Result<GcRef<JsObject>, VmError> {
        let proto = self.object_prototype_ref();
        let signal = self.allocate_ordinary_object(Some(proto));
        self.define_data_property(signal, PropertyKey::from("aborted"), Value::Bool(aborted), true, true, true);
        self.define_data_property(signal, PropertyKey::from("reason"), reason, true, true, true);
        self.define_data_property(signal, PropertyKey::from("onabort"), Value::Null, true, true, true);
        let add = self.allocate_builtin_method(BuiltinId::AbortSignalAddEventListener);
        self.define_data_property(signal, PropertyKey::from("addEventListener"), add, true, false, true);
        let remove = self.allocate_builtin_method(BuiltinId::AbortSignalRemoveEventListener);
        self.define_data_property(signal, PropertyKey::from("removeEventListener"), remove, true, false, true);
        let throw_if_aborted = self.allocate_builtin_method(BuiltinId::AbortSignalThrowIfAborted);
        self.define_data_property(signal, PropertyKey::from("throwIfAborted"), throw_if_aborted, true, false, true);
        let listeners = self.make_array_from_values(Vec::new())?;
        self.define_data_property(signal, PropertyKey::from("__abortListeners"), listeners, true, false, true);
        Ok(signal)
    }

    /// `new ResizeObserver(callback)` — create the host observer, remember
    /// callback + instance, return the instance host object.
    fn resize_observer_construct(&mut self, callback: Option<Value>) -> Result<Value, VmError> {
        let callback = callback.unwrap_or(Value::Undefined);
        if !self.is_callable_value(&callback) {
            return Err(VmError::TypeError(
                "ResizeObserver constructor argument is not callable".to_string(),
            ));
        }
        let id = match self.host.observer(ObserverOp::Create {
            kind: ObserverKind::Resize,
        }) {
            Ok(ObserverResult::Created(ObserverId(id))) => id,
            _ => {
                return Err(VmError::TypeError(
                    "host does not support ResizeObserver".to_string(),
                ));
            }
        };
        let instance = self.make_host_object(HostObjectSlot {
            class: HostObjectClass::Observer,
            interface_name: "ResizeObserver",
            handle: id,
            dispatch: HostDispatch::Ordinary,
            supports_indexed_properties: false,
            supports_named_properties: false,
        });
        self.resize_observers.insert(
            id,
            ResizeObserverReg {
                callback,
                instance: instance.clone(),
            },
        );
        Ok(instance)
    }

    /// `new IntersectionObserver(callback, options)` — create the host observer,
    /// remember callback + instance, return the instance host object. Options
    /// (root/rootMargin/threshold) are accepted but the current implementation
    /// reports against the viewport with a >0 ratio threshold.
    fn intersection_observer_construct(
        &mut self,
        callback: Option<Value>,
    ) -> Result<Value, VmError> {
        let callback = callback.unwrap_or(Value::Undefined);
        if !self.is_callable_value(&callback) {
            return Err(VmError::TypeError(
                "IntersectionObserver constructor argument is not callable".to_string(),
            ));
        }
        let id = match self.host.observer(ObserverOp::Create {
            kind: ObserverKind::Intersection,
        }) {
            Ok(ObserverResult::Created(ObserverId(id))) => id,
            _ => {
                return Err(VmError::TypeError(
                    "host does not support IntersectionObserver".to_string(),
                ));
            }
        };
        let instance = self.make_host_object(HostObjectSlot {
            class: HostObjectClass::Observer,
            interface_name: "IntersectionObserver",
            handle: id,
            dispatch: HostDispatch::Ordinary,
            supports_indexed_properties: false,
            supports_named_properties: false,
        });
        self.mutation_observers.insert(
            id,
            MutationObserverReg {
                callback,
                instance: instance.clone(),
            },
        );
        Ok(instance)
    }

    /// `observer.observe(target, options)`.
    fn mutation_observer_observe(
        &mut self,
        this_value: &Value,
        args: &[Value],
    ) -> Result<Value, VmError> {
        let Some(id) = self.observer_id_from_this(this_value) else {
            return Err(VmError::TypeError(
                "observe called on a non-MutationObserver".to_string(),
            ));
        };
        let Some(target) = self.node_id_from_host_val(args.first().unwrap_or(&Value::Undefined))
        else {
            return Err(VmError::TypeError(
                "MutationObserver.observe requires a node target".to_string(),
            ));
        };
        let options = self.read_observer_options(args.get(1));
        let _ = self.host.observer(ObserverOp::Observe {
            observer: ObserverId(id),
            target,
            options,
        });
        Ok(Value::Undefined)
    }

    /// `observer.disconnect()`.
    fn mutation_observer_disconnect(&mut self, this_value: &Value) -> Result<Value, VmError> {
        if let Some(id) = self.observer_id_from_this(this_value) {
            let _ = self.host.observer(ObserverOp::Disconnect {
                observer: ObserverId(id),
            });
            self.mutation_observers.remove(&id);
        }
        Ok(Value::Undefined)
    }

    /// `observer.takeRecords()` — drain and return pending records as an array.
    fn mutation_observer_take_records(&mut self, this_value: &Value) -> Result<Value, VmError> {
        let Some(id) = self.observer_id_from_this(this_value) else {
            return self.make_array_from_values(Vec::new());
        };
        let records = match self.host.observer(ObserverOp::TakeRecords {
            observer: ObserverId(id),
        }) {
            Ok(ObserverResult::Records(records)) => records,
            _ => Vec::new(),
        };
        self.build_mutation_record_array(records)
    }

    /// Read a `MutationObserverInit` dictionary into `ObserverOptions`.
    fn read_observer_options(&mut self, value: Option<&Value>) -> ObserverOptions {
        let mut opts = ObserverOptions {
            subtree: false,
            child_list: false,
            attributes: false,
            character_data: false,
            attribute_old_value: false,
            character_data_old_value: false,
            threshold: None,
            root: None,
            root_margin: None,
        };
        let Some(value) = value else {
            return opts;
        };
        if !matches!(value, Value::Object(_)) {
            return opts;
        }
        opts.subtree = self.observer_init_flag(value, "subtree");
        opts.child_list = self.observer_init_flag(value, "childList");
        opts.attributes = self.observer_init_flag(value, "attributes");
        opts.character_data = self.observer_init_flag(value, "characterData");
        opts.attribute_old_value = self.observer_init_flag(value, "attributeOldValue");
        opts.character_data_old_value = self.observer_init_flag(value, "characterDataOldValue");
        // Per spec, *OldValue implies the corresponding observation is on.
        if opts.attribute_old_value {
            opts.attributes = true;
        }
        if opts.character_data_old_value {
            opts.character_data = true;
        }
        opts
    }

    fn observer_init_flag(&mut self, obj: &Value, key: &str) -> bool {
        match self.get_property_value(obj, &PropertyKey::from(key)) {
            Ok(v) => self.is_truthy(&v),
            Err(_) => false,
        }
    }

    fn build_mutation_record_array(
        &mut self,
        records: Vec<ObserverRecord>,
    ) -> Result<Value, VmError> {
        let mut values = Vec::with_capacity(records.len());
        for record in records {
            values.push(self.build_mutation_record(record)?);
        }
        self.make_array_from_values(values)
    }

    /// Build a `MutationRecord` JS object from a host record, filling the spec's
    /// always-present fields (`target`, `type`, `addedNodes`, `removedNodes`,
    /// `attributeName`, `oldValue`) with sensible defaults when absent.
    fn build_mutation_record(&mut self, record: ObserverRecord) -> Result<Value, VmError> {
        let kind = record.kind;
        let target_id = record.target;
        let value = self.host_data_to_value(record.payload);
        let Value::Object(object) = value else {
            return Ok(value);
        };
        let target = self.make_dom_node_value(target_id);
        self.define_data_property(object, PropertyKey::from("target"), target, true, true, true);
        if kind == ObserverKind::Mutation {
            // Defaults for fields the payload may omit (e.g. addedNodes on an
            // attributes record), so reads never yield `undefined`.
            let empty_added = self.make_array_from_values(Vec::new())?;
            let empty_removed = self.make_array_from_values(Vec::new())?;
            self.ensure_default_property(object, "addedNodes", empty_added);
            self.ensure_default_property(object, "removedNodes", empty_removed);
            self.ensure_default_property(object, "attributeName", Value::Null);
            self.ensure_default_property(object, "oldValue", Value::Null);
            self.ensure_default_property(object, "attributeNamespace", Value::Null);
            self.ensure_default_property(object, "previousSibling", Value::Null);
            self.ensure_default_property(object, "nextSibling", Value::Null);
        }
        // Intersection entries carry their full payload (isIntersecting, ratio,
        // boundingClientRect, …) from the host already.
        Ok(Value::Object(object))
    }

    /// Build a `ResizeObserverEntry` JS object from a host record.
    /// `devicePixelContentBoxSize` and the depth-gated multi-pass loop are
    /// intentionally omitted for now.
    fn build_resize_entry(&mut self, record: ObserverRecord) -> Result<Value, VmError> {
        let target = self.make_dom_node_value(record.target);
        let mut width = 0.0;
        let mut height = 0.0;
        if let HostData::Object(pairs) = record.payload {
            for (key, value) in pairs {
                match (key.as_str(), value) {
                    ("width", HostData::Number(n)) => width = n,
                    ("height", HostData::Number(n)) => height = n,
                    _ => {}
                }
            }
        }
        let rect = self.allocate_ordinary_object(Some(self.object_prototype_ref()));
        for (key, val) in [
            ("width", Value::Number(width)),
            ("height", Value::Number(height)),
            ("top", Value::Number(0.0)),
            ("left", Value::Number(0.0)),
            ("bottom", Value::Number(height)),
            ("right", Value::Number(width)),
        ] {
            self.define_data_property(rect, PropertyKey::from(key), val, true, true, true);
        }
        let size = self.allocate_ordinary_object(Some(self.object_prototype_ref()));
        self.define_data_property(
            size,
            PropertyKey::from("inlineSize"),
            Value::Number(width),
            true,
            true,
            true,
        );
        self.define_data_property(
            size,
            PropertyKey::from("blockSize"),
            Value::Number(height),
            true,
            true,
            true,
        );
        let content_box = self.make_array_from_values(vec![Value::Object(size)])?;
        let border_box = self.make_array_from_values(vec![Value::Object(size)])?;
        let entry = self.allocate_ordinary_object(Some(self.object_prototype_ref()));
        self.define_data_property(entry, PropertyKey::from("target"), target, true, true, true);
        self.define_data_property(
            entry,
            PropertyKey::from("contentRect"),
            Value::Object(rect),
            true,
            true,
            true,
        );
        self.define_data_property(
            entry,
            PropertyKey::from("contentBoxSize"),
            content_box,
            true,
            true,
            true,
        );
        self.define_data_property(
            entry,
            PropertyKey::from("borderBoxSize"),
            border_box,
            true,
            true,
            true,
        );
        Ok(Value::Object(entry))
    }

    fn ensure_default_property(&mut self, object: GcRef<JsObject>, key: &str, default: Value) {
        let pk = PropertyKey::from(key);
        if self.get_own_property_descriptor(object, &pk).is_none() {
            self.define_data_property(object, pk, default, true, true, true);
        }
    }

    /// Convert a host `HostData` tree into a JS `Value` (recursively).
    fn host_data_to_value(&mut self, data: HostData) -> Value {
        match data {
            HostData::Null => Value::Null,
            HostData::Bool(b) => Value::Bool(b),
            HostData::Number(n) => Value::Number(n),
            HostData::String(s) => self.make_string_value(&s),
            HostData::Node(id) => self.make_dom_node_value(id),
            HostData::Array(items) => {
                let mut values = Vec::with_capacity(items.len());
                for item in items {
                    values.push(self.host_data_to_value(item));
                }
                self.make_array_from_values(values)
                    .unwrap_or(Value::Undefined)
            }
            HostData::Object(pairs) => {
                let object = self.allocate_ordinary_object(Some(self.object_prototype_ref()));
                for (key, item) in pairs {
                    let val = self.host_data_to_value(item);
                    self.define_data_property(
                        object,
                        PropertyKey::from(key.as_str()),
                        val,
                        true,
                        true,
                        true,
                    );
                }
                Value::Object(object)
            }
        }
    }

    /// Convert a JS `Value` into a host `HostData` tree (inverse of
    /// `host_data_to_value`). Used to ferry `history.state` to the host. Plain
    /// objects become `HostData::Object` (own enumerable string keys); functions
    /// and symbols become `Null`. Depth-bounded against cyclic state.
    fn value_to_host_data(&mut self, value: &Value) -> HostData {
        self.value_to_host_data_depth(value, 0)
    }

    fn value_to_host_data_depth(&mut self, value: &Value, depth: u32) -> HostData {
        if depth > 64 {
            return HostData::Null;
        }
        match value {
            Value::Undefined | Value::Null => HostData::Null,
            Value::Bool(b) => HostData::Bool(*b),
            Value::Number(n) => HostData::Number(*n),
            Value::String(_) => HostData::String(self.to_string(value)),
            Value::Symbol(_) => HostData::Null,
            Value::Object(obj) => {
                if self.is_callable_value(value) {
                    return HostData::Null;
                }
                let object = *obj;
                let keys = self.object_own_enumerable_keys(object);
                let mut pairs = Vec::with_capacity(keys.len());
                for key in keys {
                    let name = self.property_key_to_string(&key);
                    let item = self
                        .get_property_value(value, &key)
                        .unwrap_or(Value::Undefined);
                    pairs.push((name, self.value_to_host_data_depth(&item, depth + 1)));
                }
                HostData::Object(pairs)
            }
        }
    }

    /// Public entry point to flush queued observer records (Mutation +
    /// Intersection) to their callbacks. Called by the host after feeding new
    /// geometry (IntersectionObserver) and at microtask checkpoints (Mutation).
    pub fn deliver_observer_records(&mut self) {
        self.deliver_mutation_records();
        self.deliver_resize_records();
    }

    /// Deliver pending observer records (Mutation + Intersection) to each
    /// observer's callback. Run at the end of a microtask checkpoint (the spec's
    /// "notify mutation observers") and after geometry feeds. Loops until no
    /// records remain (callbacks may mutate the DOM and enqueue more), bounded to
    /// avoid a pathological infinite loop.
    fn deliver_mutation_records(&mut self) {
        if self.delivering_mutations || self.mutation_observers.is_empty() {
            return;
        }
        self.delivering_mutations = true;
        let mut rounds = 0;
        loop {
            rounds += 1;
            if rounds > 1000 {
                break;
            }
            let regs: Vec<(u64, MutationObserverReg)> = self
                .mutation_observers
                .iter()
                .map(|(id, reg)| (*id, reg.clone()))
                .collect();
            let mut delivered_any = false;
            for (id, reg) in regs {
                // Skip observers disconnected during this checkpoint.
                if !self.mutation_observers.contains_key(&id) {
                    continue;
                }
                let records = match self.host.observer(ObserverOp::TakeRecords {
                    observer: ObserverId(id),
                }) {
                    Ok(ObserverResult::Records(records)) if !records.is_empty() => records,
                    _ => continue,
                };
                delivered_any = true;
                // One record per callback invocation: the boa backend flushes
                // after every DOM mutation, so observers see each change as
                // its own delivery.
                for record in records {
                    let records_array = match self.build_mutation_record_array(vec![record]) {
                        Ok(value) => value,
                        Err(_) => continue,
                    };
                    let _ = self.call_value_sync(
                        reg.callback.clone(),
                        reg.instance.clone(),
                        vec![records_array, reg.instance.clone()],
                    );
                    // Drain microtasks the callback queued (delivery is guarded,
                    // so this won't recurse into another delivery pass).
                    self.drain_microtasks();
                }
            }
            if !delivered_any {
                break;
            }
        }
        self.delivering_mutations = false;
    }

    fn deliver_resize_records(&mut self) {
        if self.resize_observers.is_empty() {
            return;
        }
        let regs: Vec<(u64, ResizeObserverReg)> = self
            .resize_observers
            .iter()
            .map(|(id, reg)| (*id, reg.clone()))
            .collect();
        for (id, reg) in regs {
            if !self.resize_observers.contains_key(&id) {
                continue;
            }
            let records = match self.host.observer(ObserverOp::TakeRecords {
                observer: ObserverId(id),
            }) {
                Ok(ObserverResult::Records(records)) if !records.is_empty() => records,
                _ => continue,
            };
            let mut entries = Vec::with_capacity(records.len());
            for record in records {
                match self.build_resize_entry(record) {
                    Ok(entry) => entries.push(entry),
                    Err(_) => continue,
                }
            }
            let Ok(entries_array) = self.make_array_from_values(entries) else {
                continue;
            };
            let _ = self.call_value_sync(
                reg.callback.clone(),
                Value::Undefined,
                vec![entries_array, reg.instance],
            );
        }
    }

    /// Names that resolve as bare globals by virtue of the window being the
    /// global object (e.g. `location`, `navigator`, `localStorage`). A bare
    /// identifier not in `self.globals` falls back to `get_window_property`
    /// only for these; anything else is a genuine ReferenceError.
    fn is_window_global(name: &str) -> bool {
        matches!(
            name,
            "location"
                | "navigator"
                | "screen"
                | "history"
                | "performance"
                | "localStorage"
                | "sessionStorage"
                | "innerWidth"
                | "innerHeight"
                | "scrollX"
                | "scrollY"
                | "pageXOffset"
                | "pageYOffset"
                | "devicePixelRatio"
                | "btoa"
                | "atob"
                | "requestIdleCallback"
                | "cancelIdleCallback"
                | "getComputedStyle"
                | "matchMedia"
                | "scrollTo"
                | "scroll"
                | "scrollBy"
                | "getSelection"
                | "crypto"
                | "isSecureContext"
                | "crossOriginIsolated"
                | "addEventListener"
                | "removeEventListener"
        )
    }

    fn get_window_property(&mut self, name: String) -> Result<Value, VmError> {
        match name.as_str() {
            // Return the same GcRef as the document global so `window.document === document`
            "document" => Ok(self.globals.get("document").cloned().unwrap_or(Value::Undefined)),
            "window" | "self" | "globalThis" => Ok(self.globals.get("window").cloned().unwrap_or(Value::Undefined)),
            "innerWidth" => {
                let v = self.host.window_metrics(WindowId(0)).map(|m| m.inner_width).unwrap_or(0.0);
                Ok(Value::Number(v))
            }
            "innerHeight" => {
                let v = self.host.window_metrics(WindowId(0)).map(|m| m.inner_height).unwrap_or(0.0);
                Ok(Value::Number(v))
            }
            "scrollX" | "pageXOffset" => {
                let v = self.host.window_metrics(WindowId(0)).map(|m| m.scroll_x).unwrap_or(0.0);
                Ok(Value::Number(v))
            }
            "scrollY" | "pageYOffset" => {
                let v = self.host.window_metrics(WindowId(0)).map(|m| m.scroll_y).unwrap_or(0.0);
                Ok(Value::Number(v))
            }
            "devicePixelRatio" => {
                let v = self.host.window_metrics(WindowId(0)).map(|m| m.device_pixel_ratio).unwrap_or(1.0);
                Ok(Value::Number(v))
            }
            "location" => self.make_location_object(),
            "navigator" => {
                let nav = self.allocate_ordinary_object(None);
                let ua = self.make_string_value("Tobira/0.1");
                self.define_data_property(nav, PropertyKey::from("userAgent"), ua, true, true, true);
                let lang = self.make_string_value("en");
                self.define_data_property(nav, PropertyKey::from("language"), lang, true, true, true);
                Ok(Value::Object(nav))
            }
            "screen" => {
                let scr = self.allocate_ordinary_object(None);
                self.define_data_property(scr, PropertyKey::from("width"), Value::Number(1920.0), true, true, true);
                self.define_data_property(scr, PropertyKey::from("height"), Value::Number(1080.0), true, true, true);
                Ok(Value::Object(scr))
            }
            "history" => Ok(self.make_host_object(HostObjectSlot {
                class: HostObjectClass::Other("History"),
                interface_name: "History",
                handle: 0,
                dispatch: HostDispatch::Ordinary,
                supports_indexed_properties: false,
                supports_named_properties: false,
            })),
            "scrollTo" | "scroll" => Ok(self.allocate_builtin_method(BuiltinId::WindowScrollTo)),
            "scrollBy" => Ok(self.allocate_builtin_method(BuiltinId::WindowScrollBy)),
            "getComputedStyle" => Ok(self.allocate_builtin_method(BuiltinId::WindowGetComputedStyle)),
            "matchMedia" => Ok(self.allocate_builtin_method(BuiltinId::WindowMatchMedia)),
            "addEventListener" | "removeEventListener" => {
                Ok(self.allocate_builtin_method(BuiltinId::DomNodeAddEventListener))
            }
            "performance" => {
                let perf = self.allocate_ordinary_object(None);
                let now_fn = self.allocate_builtin_method(BuiltinId::PerformanceNow);
                self.define_data_property(perf, PropertyKey::from("now"), now_fn, true, true, true);
                self.define_data_property(perf, PropertyKey::from("timeOrigin"), Value::Number(0.0), true, true, true);
                let mark_fn = self.allocate_builtin_method(BuiltinId::PerformanceMark);
                self.define_data_property(perf, PropertyKey::from("mark"), mark_fn, true, true, true);
                let measure_fn = self.allocate_builtin_method(BuiltinId::PerformanceMeasure);
                self.define_data_property(perf, PropertyKey::from("measure"), measure_fn, true, true, true);
                let clear_marks_fn = self.allocate_builtin_method(BuiltinId::PerformanceClearMarks);
                self.define_data_property(perf, PropertyKey::from("clearMarks"), clear_marks_fn, true, true, true);
                let clear_measures_fn = self.allocate_builtin_method(BuiltinId::PerformanceClearMeasures);
                self.define_data_property(perf, PropertyKey::from("clearMeasures"), clear_measures_fn, true, true, true);
                let get_entries_fn = self.allocate_builtin_method(BuiltinId::PerformanceGetEntries);
                self.define_data_property(perf, PropertyKey::from("getEntries"), get_entries_fn, true, true, true);
                let get_entries_by_name_fn = self.allocate_builtin_method(BuiltinId::PerformanceGetEntriesByName);
                self.define_data_property(perf, PropertyKey::from("getEntriesByName"), get_entries_by_name_fn, true, true, true);
                let get_entries_by_type_fn = self.allocate_builtin_method(BuiltinId::PerformanceGetEntriesByType);
                self.define_data_property(perf, PropertyKey::from("getEntriesByType"), get_entries_by_type_fn, true, true, true);
                let timing = self.allocate_ordinary_object(None);
                self.define_data_property(timing, PropertyKey::from("navigationStart"), Value::Number(0.0), true, true, true);
                self.define_data_property(perf, PropertyKey::from("timing"), Value::Object(timing), true, true, true);
                Ok(Value::Object(perf))
            }
            "requestIdleCallback" => Ok(self.allocate_builtin_method(BuiltinId::RequestIdleCallback)),
            "cancelIdleCallback" => Ok(self.allocate_builtin_method(BuiltinId::CancelIdleCallback)),
            "btoa" => Ok(self.allocate_builtin_method(BuiltinId::Btoa)),
            "atob" => Ok(self.allocate_builtin_method(BuiltinId::Atob)),
            "localStorage" => Ok(self.make_storage_object(super::host::StorageAreaKind::Local)),
            "sessionStorage" => Ok(self.make_storage_object(super::host::StorageAreaKind::Session)),
            "getSelection" => {
                // Return a function that returns null (selection not implemented)
                let null_fn = self.allocate_ordinary_object(None);
                Ok(Value::Object(null_fn))
            }
            "open" | "close" | "focus" | "blur" | "resizeTo" | "resizeBy" | "moveTo" | "moveBy" => {
                Ok(self.allocate_builtin_method(BuiltinId::DomNodeAddEventListener)) // no-op stub
            }
            "crypto" => {
                let crypto = self.allocate_ordinary_object(Some(self.object_prototype_ref()));
                self.define_builtin_method(crypto, "getRandomValues", BuiltinId::CryptoGetRandomValues);
                self.define_builtin_method(crypto, "randomUUID", BuiltinId::CryptoRandomUUID);
                Ok(Value::Object(crypto))
            }
            "isSecureContext" => Ok(Value::Bool(false)),
            "crossOriginIsolated" => Ok(Value::Bool(false)),
            _ => Ok(self.globals.get(&name).cloned().unwrap_or(Value::Undefined)),
        }
    }

    fn make_location_object(&mut self) -> Result<Value, VmError> {
        // A host object so that property *writes* (`location.href = …`,
        // `location.hash = …`) route through `set_host_property` into the host's
        // navigation, instead of silently setting an inert data property.
        Ok(self.make_host_object(HostObjectSlot {
            class: HostObjectClass::Other("Location"),
            interface_name: "Location",
            handle: 0,
            dispatch: HostDispatch::Ordinary,
            supports_indexed_properties: false,
            supports_named_properties: false,
        }))
    }

    /// Read a `location` property live from the host (so it reflects hash /
    /// pushState changes made during the same turn).
    fn get_location_property(&mut self, name: String) -> Result<Value, VmError> {
        let l = match self.host.location(WindowId(0)) {
            Ok(l) => l,
            Err(_) => return Ok(Value::Undefined),
        };
        let value = match name.as_str() {
            "href" => Some(l.href.clone()),
            "origin" => Some(l.origin.clone()),
            "protocol" => Some(l.protocol.clone()),
            "host" => Some(l.host.clone()),
            "hostname" => Some(l.hostname.clone()),
            "port" => Some(l.port.clone()),
            "pathname" => Some(l.pathname.clone()),
            "search" => Some(l.search.clone()),
            "hash" => Some(l.hash.clone()),
            _ => None,
        };
        Ok(value
            .map(|s| self.make_string_value(&s))
            .unwrap_or(Value::Undefined))
    }

    /// `window.history`: `length` / `state` query the host (a zero-delta `Go` is
    /// a side-effect-free read), and the methods are cached builtins.
    fn get_history_property(&mut self, name: String) -> Result<Value, VmError> {
        match name.as_str() {
            "pushState" => Ok(self.allocate_builtin_method(BuiltinId::HistoryPushState)),
            "replaceState" => Ok(self.allocate_builtin_method(BuiltinId::HistoryReplaceState)),
            "back" => Ok(self.allocate_builtin_method(BuiltinId::HistoryBack)),
            "forward" => Ok(self.allocate_builtin_method(BuiltinId::HistoryForward)),
            "go" => Ok(self.allocate_builtin_method(BuiltinId::HistoryGo)),
            "scrollRestoration" => Ok(self.make_string_value("auto")),
            "length" => {
                let len = self
                    .host
                    .history(HistoryAction::Go { window: WindowId(0), delta: 0 })
                    .map(|o| o.length)
                    .unwrap_or(1);
                Ok(Value::Number(len as f64))
            }
            "state" => {
                let state = self
                    .host
                    .history(HistoryAction::Go { window: WindowId(0), delta: 0 })
                    .ok()
                    .and_then(|o| o.state);
                Ok(match state {
                    Some(data) => self.host_data_to_value(data),
                    None => Value::Null,
                })
            }
            _ => Ok(Value::Undefined),
        }
    }

    fn get_document_property(&mut self, name: String) -> Result<Value, VmError> {
        match name.as_str() {
            // The document is its own root; per spec its ownerDocument is null.
            "ownerDocument" => Ok(Value::Null),
            "body" => {
                let res = self.host.read_dom(DomRead::DocumentBody { window: WindowId(0) });
                Ok(match res { Ok(DomReadResult::Node(id)) => self.make_dom_node_value(id), _ => Value::Null })
            }
            "head" => {
                let res = self.host.read_dom(DomRead::DocumentHead { window: WindowId(0) });
                Ok(match res { Ok(DomReadResult::Node(id)) => self.make_dom_node_value(id), _ => Value::Null })
            }
            "documentElement" => {
                let res = self.host.read_dom(DomRead::DocumentRoot { window: WindowId(0) });
                Ok(match res { Ok(DomReadResult::Node(id)) => self.make_dom_node_value(id), _ => Value::Null })
            }
            "title" => {
                let head_res = self.host.read_dom(DomRead::DocumentHead { window: WindowId(0) });
                match head_res {
                    Ok(DomReadResult::Node(head_id)) => {
                        let title_res = self.host.read_dom(DomRead::QuerySelector { root: head_id, selectors: "title".to_string() });
                        match title_res {
                            Ok(DomReadResult::Node(title_id)) => {
                                let text_res = self.host.read_dom(DomRead::TextContent { node: title_id });
                                Ok(match text_res { Ok(DomReadResult::String(s)) => self.make_string_value(&s), _ => self.make_string_value("") })
                            }
                            _ => Ok(self.make_string_value("")),
                        }
                    }
                    _ => Ok(self.make_string_value("")),
                }
            }
            "nodeType" => Ok(Value::Number(9.0)),
            "nodeName" => Ok(self.make_string_value("#document")),
            "readyState" => Ok(self.make_string_value("complete")),
            "compatMode" => Ok(self.make_string_value("CSS1Compat")),
            "charset" | "characterSet" => Ok(self.make_string_value("UTF-8")),
            "location" => self.make_location_object(),
            "currentScript" => {
                if let Some(node) = self.current_script_node {
                    return Ok(self.make_dom_node_value(node));
                }
                if let Some(src) = self.current_script_src.clone() {
                    let object = self.allocate_ordinary_object(Some(self.object_prototype_ref()));
                    let src_value = self.make_string_value(&src);
                    let tag_name = self.make_string_value("SCRIPT");
                    self.define_data_property(
                        object,
                        PropertyKey::from("src"),
                        src_value,
                        true,
                        true,
                        true,
                    );
                    self.define_data_property(
                        object,
                        PropertyKey::from("tagName"),
                        tag_name.clone(),
                        true,
                        true,
                        true,
                    );
                    self.define_data_property(
                        object,
                        PropertyKey::from("nodeName"),
                        tag_name,
                        true,
                        true,
                        true,
                    );
                    self.define_builtin_method(object, "getAttribute", BuiltinId::ElementStubGetAttribute);
                    self.define_builtin_method(object, "hasAttribute", BuiltinId::ElementStubHasAttribute);
                    Ok(Value::Object(object))
                } else {
                    Ok(Value::Null)
                }
            }
            "URL" | "documentURI" => {
                let res = self.host.location(WindowId(0));
                Ok(match res { Ok(l) => self.make_string_value(&l.href), _ => self.make_string_value("") })
            }
            "domain" => {
                let res = self.host.location(WindowId(0));
                Ok(match res { Ok(l) => self.make_string_value(&l.hostname), _ => self.make_string_value("") })
            }
            "querySelector" => Ok(self.allocate_builtin_method(BuiltinId::DomDocQuerySelector)),
            "querySelectorAll" => Ok(self.allocate_builtin_method(BuiltinId::DomDocQuerySelectorAll)),
            "getElementById" => Ok(self.allocate_builtin_method(BuiltinId::DomDocGetElementById)),
            "getElementsByClassName" => Ok(self.allocate_builtin_method(BuiltinId::DomDocGetElementsByClassName)),
            "getElementsByTagName" => Ok(self.allocate_builtin_method(BuiltinId::DomDocGetElementsByTagName)),
            "createElement" => Ok(self.allocate_builtin_method(BuiltinId::DomDocCreateElement)),
            "createElementNS" => Ok(self.allocate_builtin_method(BuiltinId::DomCreateElementNs)),
            "createTextNode" => Ok(self.allocate_builtin_method(BuiltinId::DomDocCreateTextNode)),
            "createDocumentFragment" => Ok(self.allocate_builtin_method(BuiltinId::DomDocCreateFragment)),
            "write" | "writeln" => Ok(self.allocate_builtin_method(BuiltinId::DomDocWrite)),
            "addEventListener" | "removeEventListener" => {
                Ok(self.allocate_builtin_method(BuiltinId::DomNodeAddEventListener))
            }
            // Common document properties
            "cookie" => Ok(self.make_string_value("")),
            "referrer" => Ok(self.make_string_value("")),
            "hidden" => Ok(Value::Bool(false)),
            "visibilityState" => Ok(self.make_string_value("visible")),
            "activeElement" => {
                let res = self.host.read_dom(DomRead::ActiveElement { window: WindowId(0) });
                Ok(match res { Ok(DomReadResult::Node(id)) => self.make_dom_node_value(id), _ => Value::Null })
            }
            "createEvent" | "createComment" => {
                // Return a stub event/comment object
                Ok(Value::Object(self.allocate_ordinary_object(None)))
            }
            "implementation" => {
                let impl_obj = self.allocate_ordinary_object(None);
                Ok(Value::Object(impl_obj))
            }
            _ => Ok(Value::Undefined),
        }
    }

    fn get_node_property(&mut self, slot: HostObjectSlot, name: String) -> Result<Value, VmError> {
        let node_id = NodeId(slot.handle as u32);
        match name.as_str() {
            "nodeType" => {
                let res = self.host.read_dom(DomRead::NodeKind { node: node_id });
                Ok(match res {
                    Ok(DomReadResult::Kind(k)) => Value::Number(match k {
                        NodeKind::Element => 1.0,
                        NodeKind::Text => 3.0,
                        NodeKind::Document => 9.0,
                        _ => 11.0,
                    }),
                    _ => Value::Number(1.0),
                })
            }
            "nodeName" | "tagName" => {
                let tag = self.get_node_name(node_id);
                Ok(self.make_string_value(&tag))
            }
            "constructor" => {
                // Return the node's DOM interface constructor so libraries can read
                // `node.constructor.prototype` (React's input value-tracker does this
                // — if it throws on `undefined.prototype`, mounting ANY <input>
                // fails). The constructor has a `.prototype` but no `value`/`checked`
                // accessor, so React's tracker bails gracefully and falls back to
                // firing change on every input event.
                let tag = self.get_node_name(node_id).to_ascii_uppercase();
                let iface = match tag.as_str() {
                    "INPUT" => "HTMLInputElement",
                    "TEXTAREA" => "HTMLTextAreaElement",
                    "SELECT" => "HTMLSelectElement",
                    "BUTTON" => "HTMLButtonElement",
                    "A" => "HTMLAnchorElement",
                    "#TEXT" => "Text",
                    "#COMMENT" => "Comment",
                    _ => "HTMLElement",
                };
                Ok(self
                    .globals
                    .get(iface)
                    .cloned()
                    .or_else(|| self.globals.get("HTMLElement").cloned())
                    .unwrap_or(Value::Undefined))
            }
            "nodeValue" | "data" => {
                let res = self.host.read_dom(DomRead::NodeValue { node: node_id });
                Ok(match res { Ok(DomReadResult::String(s)) => self.make_string_value(&s), _ => Value::Null })
            }
            "splitText" => Ok(self.allocate_builtin_method(BuiltinId::DomNodeSplitText)),
            "textContent" => {
                let res = self.host.read_dom(DomRead::TextContent { node: node_id });
                Ok(match res { Ok(DomReadResult::String(s)) => self.make_string_value(&s), _ => Value::Null })
            }
            "innerHTML" => {
                let res = self.host.read_dom(DomRead::InnerHtml { node: node_id });
                Ok(match res { Ok(DomReadResult::String(s)) => self.make_string_value(&s), _ => self.make_string_value("") })
            }
            "outerHTML" => {
                let res = self.host.read_dom(DomRead::OuterHtml { node: node_id });
                Ok(match res { Ok(DomReadResult::String(s)) => self.make_string_value(&s), _ => self.make_string_value("") })
            }
            "insertAdjacentHTML" => Ok(self.allocate_builtin_method(BuiltinId::DomNodeInsertAdjacentHtml)),
            "id" => {
                let res = self.host.read_dom(DomRead::Attribute { node: node_id, name: "id".to_string() });
                Ok(match res { Ok(DomReadResult::String(s)) => self.make_string_value(&s), _ => self.make_string_value("") })
            }
            "className" => {
                let value = self.get_dom_attribute(node_id, "class");
                Ok(self.make_string_value(&value))
            }
            "type" => {
                // `input.type` reflects the attribute but DEFAULTS to "text" when
                // absent. React's isTextInputElement does `supportedInputTypes[el.type]`
                // — returning "" there makes React treat a plain <input> as a
                // non-text element and skip its onChange handling entirely.
                let attr = match self.host.read_dom(DomRead::Attribute { node: node_id, name: "type".to_string() }) {
                    Ok(DomReadResult::String(s)) => s,
                    _ => String::new(),
                };
                if !attr.is_empty() {
                    return Ok(self.make_string_value(&attr));
                }
                let tag = self.get_node_name(node_id).to_ascii_uppercase();
                Ok(self.make_string_value(if tag == "INPUT" { "text" } else { "" }))
            }
            "width" | "height" => {
                let attr = match self.host.read_dom(DomRead::Attribute {
                    node: node_id,
                    name: name.clone(),
                }) {
                    Ok(DomReadResult::String(s)) => s,
                    _ => String::new(),
                };
                Ok(self.make_string_value(&attr))
            }
            "classList" => Ok(self.make_host_object(HostObjectSlot {
                class: HostObjectClass::Other("TokenList"),
                interface_name: "DOMTokenList",
                handle: slot.handle,
                dispatch: HostDispatch::TokenList,
                supports_indexed_properties: false,
                supports_named_properties: false,
            })),
            "style" => Ok(self.make_host_object(HostObjectSlot {
                class: HostObjectClass::Other("CSSStyleDeclaration"),
                interface_name: "CSSStyleDeclaration",
                handle: slot.handle,
                dispatch: HostDispatch::StyleDeclaration,
                supports_indexed_properties: false,
                supports_named_properties: false,
            })),
            "dataset" => Ok(self.make_host_object(HostObjectSlot {
                class: HostObjectClass::Other("Dataset"),
                interface_name: "DOMStringMap",
                handle: slot.handle,
                dispatch: HostDispatch::Dataset,
                supports_indexed_properties: false,
                supports_named_properties: true,
            })),
            "parentNode" | "parentElement" => {
                let res = self.host.read_dom(DomRead::Parent { node: node_id });
                Ok(match res { Ok(DomReadResult::Node(id)) => self.make_dom_node_value(id), _ => Value::Null })
            }
            "children" => {
                let res = self.host.read_dom(DomRead::Children { node: node_id, elements_only: true });
                match res {
                    Ok(DomReadResult::Nodes(ids)) => {
                        let items: Vec<Value> = ids.iter().map(|&id| self.make_dom_node_value(id)).collect();
                        self.make_array_from_values(items)
                    }
                    _ => self.make_array_from_values(vec![]),
                }
            }
            "childNodes" => {
                let res = self.host.read_dom(DomRead::Children { node: node_id, elements_only: false });
                match res {
                    Ok(DomReadResult::Nodes(ids)) => {
                        let items: Vec<Value> = ids.iter().map(|&id| self.make_dom_node_value(id)).collect();
                        self.make_array_from_values(items)
                    }
                    _ => self.make_array_from_values(vec![]),
                }
            }
            "childElementCount" => {
                let res = self.host.read_dom(DomRead::Children { node: node_id, elements_only: true });
                Ok(Value::Number(match res { Ok(DomReadResult::Nodes(ids)) => ids.len() as f64, _ => 0.0 }))
            }
            "firstChild" => Ok(self.dom_child(node_id, false, false)),
            "lastChild" => Ok(self.dom_child(node_id, false, true)),
            "firstElementChild" => Ok(self.dom_child(node_id, true, false)),
            "lastElementChild" => Ok(self.dom_child(node_id, true, true)),
            "nextSibling" => Ok(self.dom_sibling(node_id, SiblingDirection::Next, false)),
            "previousSibling" => Ok(self.dom_sibling(node_id, SiblingDirection::Previous, false)),
            "nextElementSibling" => Ok(self.dom_sibling(node_id, SiblingDirection::Next, true)),
            "previousElementSibling" => Ok(self.dom_sibling(node_id, SiblingDirection::Previous, true)),
            "value" => {
                let res = self.host.read_dom(DomRead::Attribute { node: node_id, name: "value".to_string() });
                Ok(match res { Ok(DomReadResult::String(s)) => self.make_string_value(&s), _ => self.make_string_value("") })
            }
            "hidden" => {
                let res = self.host.read_dom(DomRead::Attribute { node: node_id, name: "hidden".to_string() });
                Ok(Value::Bool(matches!(res, Ok(DomReadResult::String(_)))))
            }
            "disabled" => {
                let res = self.host.read_dom(DomRead::Attribute { node: node_id, name: "disabled".to_string() });
                Ok(Value::Bool(matches!(res, Ok(DomReadResult::String(_)))))
            }
            "length" => {
                // CharacterData length (text nodes) vs child count elsewhere.
                if matches!(
                    self.host.read_dom(DomRead::NodeKind { node: node_id }),
                    Ok(DomReadResult::Kind(NodeKind::Text))
                ) {
                    let text = match self.host.read_dom(DomRead::NodeValue { node: node_id }) {
                        Ok(DomReadResult::String(s)) => s,
                        _ => String::new(),
                    };
                    return Ok(Value::Number(text.chars().count() as f64));
                }
                let res = self.host.read_dom(DomRead::Children { node: node_id, elements_only: false });
                Ok(Value::Number(match res { Ok(DomReadResult::Nodes(ids)) => ids.len() as f64, _ => 0.0 }))
            }
            // Box dimensions derived from the laid-out bounding rect.
            "offsetWidth" | "offsetHeight" | "offsetLeft" | "offsetTop"
            | "clientWidth" | "clientHeight"
            | "scrollWidth" | "scrollHeight" => {
                // Derive box metrics from the laid-out bounding rect. offset/scroll
                // dimensions ≈ the border-box size; offsetLeft/Top ≈ document
                // position (we don't track offsetParent-relative coords yet).
                let (x, y, w, h) = match self
                    .host
                    .read_dom(DomRead::BoundingClientRect { node: node_id })
                {
                    Ok(DomReadResult::Rect(r)) => (r.x, r.y, r.width, r.height),
                    _ => (0.0, 0.0, 0.0, 0.0),
                };
                let scroll_y = self
                    .host
                    .window_metrics(WindowId(0))
                    .map(|m| m.scroll_y)
                    .unwrap_or(0.0);
                let scroll_x = self
                    .host
                    .window_metrics(WindowId(0))
                    .map(|m| m.scroll_x)
                    .unwrap_or(0.0);
                let value = match name.as_str() {
                    "offsetWidth" | "clientWidth" | "scrollWidth" => w,
                    "offsetHeight" | "clientHeight" | "scrollHeight" => h,
                    // bounding rect is viewport-relative; add scroll back for the
                    // document-relative offsetLeft/offsetTop approximation.
                    "offsetLeft" => x + scroll_x,
                    "offsetTop" => y + scroll_y,
                    _ => 0.0,
                };
                Ok(Value::Number(value))
            }
            "clientLeft" | "clientTop" => Ok(Value::Number(0.0)),
            "scrollLeft" | "scrollTop" => {
                // The root element's scrollLeft/scrollTop mirror the window
                // scroll; other elements don't track their own overflow yet.
                let is_root = matches!(
                    self.host.read_dom(DomRead::DocumentRoot { window: WindowId(0) }),
                    Ok(DomReadResult::Node(root)) if root == node_id
                );
                if is_root {
                    let v = self
                        .host
                        .window_metrics(WindowId(0))
                        .map(|m| if name == "scrollLeft" { m.scroll_x } else { m.scroll_y })
                        .unwrap_or(0.0);
                    Ok(Value::Number(v))
                } else {
                    Ok(Value::Number(0.0))
                }
            }
            "offsetParent" => Ok(Value::Null),
            "isConnected" => Ok(Value::Bool(true)),
            "ownerDocument" => Ok(self.globals.get("document").cloned().unwrap_or(Value::Undefined)),
            "baseURI" => {
                let res = self.host.location(WindowId(0));
                Ok(match res { Ok(l) => self.make_string_value(&l.href), _ => self.make_string_value("") })
            }
            "getAttribute" => Ok(self.allocate_builtin_method(BuiltinId::DomNodeGetAttribute)),
            "setAttribute" => Ok(self.allocate_builtin_method(BuiltinId::DomNodeSetAttribute)),
            "removeAttribute" => Ok(self.allocate_builtin_method(BuiltinId::DomNodeRemoveAttribute)),
            "hasAttribute" => Ok(self.allocate_builtin_method(BuiltinId::DomNodeHasAttribute)),
            "hasAttributes" => Ok(self.allocate_builtin_method(BuiltinId::DomNodeHasAttributes)),
            "toggleAttribute" => Ok(self.allocate_builtin_method(BuiltinId::DomNodeToggleAttribute)),
            "getAttributeNames" => Ok(self.allocate_builtin_method(BuiltinId::DomNodeGetAttributeNames)),
            "attributes" => Ok(self.make_host_object(HostObjectSlot {
                class: HostObjectClass::Other("NamedNodeMap"),
                interface_name: "NamedNodeMap",
                handle: slot.handle,
                dispatch: HostDispatch::Ordinary,
                supports_indexed_properties: true,
                supports_named_properties: true,
            })),
            "appendChild" | "append" => Ok(self.allocate_builtin_method(BuiltinId::DomNodeAppendChild)),
            "prepend" => Ok(self.allocate_builtin_method(BuiltinId::DomNodePrepend)),
            "insertBefore" => Ok(self.allocate_builtin_method(BuiltinId::DomNodeInsertBefore)),
            "hasChildNodes" => Ok(self.allocate_builtin_method(BuiltinId::DomNodeHasChildNodes)),
            "removeChild" => Ok(self.allocate_builtin_method(BuiltinId::DomNodeRemoveChild)),
            "replaceChild" => Ok(self.allocate_builtin_method(BuiltinId::DomNodeReplaceChild)),
            "replaceChildren" => Ok(self.allocate_builtin_method(BuiltinId::DomNodeReplaceChildren)),
            "cloneNode" => Ok(self.allocate_builtin_method(BuiltinId::DomNodeCloneNode)),
            "remove" => Ok(self.allocate_builtin_method(BuiltinId::DomNodeRemove)),
            "querySelector" => Ok(self.allocate_builtin_method(BuiltinId::DomNodeQuerySelector)),
            "querySelectorAll" => Ok(self.allocate_builtin_method(BuiltinId::DomNodeQuerySelectorAll)),
            "closest" => Ok(self.allocate_builtin_method(BuiltinId::DomNodeClosest)),
            "matches" | "webkitMatchesSelector" => Ok(self.allocate_builtin_method(BuiltinId::DomNodeMatches)),
            "contains" => Ok(self.allocate_builtin_method(BuiltinId::DomNodeContains)),
            "getBoundingClientRect" => Ok(self.allocate_builtin_method(BuiltinId::DomNodeGetBoundingClientRect)),
            "scrollIntoView" => Ok(self.allocate_builtin_method(BuiltinId::DomNodeScrollIntoView)),
            "focus" => Ok(self.allocate_builtin_method(BuiltinId::DomNodeFocus)),
            "blur" => Ok(self.allocate_builtin_method(BuiltinId::DomNodeBlur)),
            "attachShadow" => Ok(self.allocate_builtin_method(BuiltinId::DomNodeAttachShadow)),
            "getRootNode" => Ok(self.allocate_builtin_method(BuiltinId::DomNodeGetRootNode)),
            "assignedNodes" => Ok(self.allocate_builtin_method(BuiltinId::DomSlotAssignedNodes)),
            "assignedElements" => Ok(self.allocate_builtin_method(BuiltinId::DomSlotAssignedElements)),
            "shadowRoot" => {
                let res = self.host.read_dom(DomRead::ShadowRoot { host: node_id });
                Ok(match res { Ok(DomReadResult::Node(id)) => self.make_dom_node_value(id), _ => Value::Null })
            }
            // ShadowRoot-node accessors (the wrapper is a generic Node).
            "host" => {
                let res = self.host.read_dom(DomRead::ShadowRootHost { node: node_id });
                Ok(match res { Ok(DomReadResult::Node(id)) => self.make_dom_node_value(id), _ => Value::Undefined })
            }
            "mode" => {
                let res = self.host.read_dom(DomRead::ShadowRootMode { node: node_id });
                match res {
                    Ok(DomReadResult::String(s)) if !s.is_empty() => Ok(self.make_string_value(&s)),
                    _ => Ok(Value::Undefined),
                }
            }
            "assignedSlot" => {
                let res = self.host.read_dom(DomRead::AssignedSlot { node: node_id });
                Ok(match res { Ok(DomReadResult::Node(id)) => self.make_dom_node_value(id), _ => Value::Null })
            }
            "click" => Ok(self.allocate_builtin_method(BuiltinId::DomNodeClick)),
            "addEventListener" => Ok(self.allocate_builtin_method(BuiltinId::DomNodeAddEventListener)),
            "removeEventListener" => Ok(self.allocate_builtin_method(BuiltinId::DomNodeRemoveEventListener)),
            "dispatchEvent" => Ok(self.allocate_builtin_method(BuiltinId::DomNodeDispatchEvent)),
            // Object.prototype methods that libraries call on DOM nodes. Host nodes
            // aren't wired into the JS prototype chain, so expose the common ones
            // directly. React's input value-tracker calls `node.hasOwnProperty(...)`
            // during mount — without this it hit "attempted to call a non-function
            // value" and every <input> failed to render.
            "hasOwnProperty" => Ok(self.allocate_builtin_method(BuiltinId::ObjectProtoHasOwnProperty)),
            "propertyIsEnumerable" => {
                Ok(self.allocate_builtin_method(BuiltinId::ObjectProtoPropertyIsEnumerable))
            }
            "isPrototypeOf" => Ok(self.allocate_builtin_method(BuiltinId::ObjectProtoIsPrototypeOf)),
            "valueOf" => Ok(self.allocate_builtin_method(BuiltinId::ObjectProtoValueOf)),
            "toString" => Ok(self.allocate_builtin_method(BuiltinId::ObjectProtoToString)),
            _ => Ok(Value::Undefined),
        }
    }

    fn get_classlist_property(&mut self, slot: HostObjectSlot, name: String) -> Result<Value, VmError> {
        let node_id = NodeId(slot.handle as u32);
        match name.as_str() {
            "length" => {
                let res = self.host.read_dom(DomRead::Attribute { node: node_id, name: "class".to_string() });
                let cls = match res { Ok(DomReadResult::String(s)) => s, _ => String::new() };
                Ok(Value::Number(if cls.trim().is_empty() { 0.0 } else { cls.split_whitespace().count() as f64 }))
            }
            "value" => {
                let res = self.host.read_dom(DomRead::Attribute { node: node_id, name: "class".to_string() });
                Ok(match res { Ok(DomReadResult::String(s)) => self.make_string_value(&s), _ => self.make_string_value("") })
            }
            "add" => Ok(self.allocate_builtin_method(BuiltinId::DomClassListAdd)),
            "remove" => Ok(self.allocate_builtin_method(BuiltinId::DomClassListRemove)),
            "contains" => Ok(self.allocate_builtin_method(BuiltinId::DomClassListContains)),
            "toggle" => Ok(self.allocate_builtin_method(BuiltinId::DomClassListToggle)),
            "replace" => Ok(self.allocate_builtin_method(BuiltinId::DomClassListReplace)),
            "item" => Ok(self.allocate_builtin_method(BuiltinId::DomClassListItem)),
            "toString" => Ok(self.allocate_builtin_method(BuiltinId::DomClassListToString)),
            _ => Ok(Value::Undefined),
        }
    }

    fn get_style_property(
        &mut self,
        slot: HostObjectSlot,
        name: String,
    ) -> Result<Value, VmError> {
        match name.as_str() {
            "getPropertyValue" => Ok(self.allocate_builtin_method(BuiltinId::DomStyleGetProperty)),
            "setProperty" => Ok(self.allocate_builtin_method(BuiltinId::DomStyleSetProperty)),
            "removeProperty" => Ok(self.allocate_builtin_method(BuiltinId::DomStyleRemoveProperty)),
            _ => {
                // Read a camelCase CSS property back from the inline style attr
                // (`el.style.color`), or the whole declaration via `cssText`.
                let node_id = NodeId(slot.handle as u32);
                let existing = match self
                    .host
                    .read_dom(DomRead::Attribute { node: node_id, name: "style".to_string() })
                {
                    Ok(DomReadResult::String(s)) => s,
                    _ => String::new(),
                };
                if name == "cssText" {
                    return Ok(self.make_string_value(&existing));
                }
                let css_prop = camel_to_css_prop(&name);
                let value = get_inline_style_prop(&existing, &css_prop);
                Ok(self.make_string_value(&value))
            }
        }
    }

    /// `el.dataset.fooBar` reads the `data-foo-bar` attribute.
    fn get_dataset_property(
        &mut self,
        slot: HostObjectSlot,
        name: String,
    ) -> Result<Value, VmError> {
        let node_id = NodeId(slot.handle as u32);
        let attr = format!("data-{}", camel_to_css_prop(&name));
        let res = self
            .host
            .read_dom(DomRead::Attribute { node: node_id, name: attr });
        Ok(match res {
            Ok(DomReadResult::String(s)) => self.make_string_value(&s),
            // Absent data attribute: fall back to Object.prototype so plain
            // members (`dataset.toString`, `hasOwnProperty`, …) still resolve.
            _ => {
                let proto = self.object_prototype_ref();
                match self.lookup_property_descriptor(proto, &PropertyKey::from(name.as_str())) {
                    Some((_, JsPropertyDescriptor::Data { value, .. })) => value,
                    _ => Value::Undefined,
                }
            }
        })
    }

    /// Property access on a `getComputedStyle(el)` snapshot object.
    fn get_computed_style_object_property(
        &mut self,
        slot: HostObjectSlot,
        name: String,
    ) -> Result<Value, VmError> {
        match name.as_str() {
            "getPropertyValue" => {
                Ok(self.allocate_builtin_method(BuiltinId::DomComputedStyleGetProperty))
            }
            "getPropertyPriority" => {
                Ok(self.allocate_builtin_method(BuiltinId::DomComputedStyleGetPriority))
            }
            _ => {
                let prop = camel_to_css_prop(&name);
                let value = self.computed_style_value(NodeId(slot.handle as u32), &prop);
                Ok(self.make_string_value(&value))
            }
        }
    }

    /// The computed value of one CSS property — a port of the boa backend's
    /// `computed_style_property_value`: inline declarations win (with
    /// `inherit` walking up), inheritable properties fall back to the parent,
    /// and everything else gets a per-tag UA default.
    fn computed_style_value(&mut self, node: NodeId, prop: &str) -> String {
        let inline = {
            let style = match self.host.read_dom(DomRead::Attribute {
                node,
                name: "style".to_string(),
            }) {
                Ok(DomReadResult::String(s)) => s,
                _ => String::new(),
            };
            get_inline_style_prop(&style, prop)
        };
        if !inline.is_empty() {
            if inline.eq_ignore_ascii_case("inherit") {
                return self.computed_style_parent_value(node, prop).unwrap_or_default();
            }
            return inline;
        }

        let tag = self.get_node_name(node).to_ascii_lowercase();

        match prop {
            "display" => {
                let hidden = matches!(
                    self.host.read_dom(DomRead::Attribute { node, name: "hidden".to_string() }),
                    Ok(DomReadResult::String(_))
                );
                if hidden {
                    "none".to_string()
                } else {
                    default_display_for_tag(&tag).to_string()
                }
            }
            "position" => "static".to_string(),
            "visibility" => self
                .computed_style_parent_value(node, "visibility")
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| "visible".to_string()),
            "color" => self
                .computed_style_parent_value(node, "color")
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| "rgb(0, 0, 0)".to_string()),
            "background-color" => default_background_color_for_tag(&tag).to_string(),
            "font-size" => match tag.as_str() {
                "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                    default_font_size_for_tag(&tag).to_string()
                }
                _ => self
                    .computed_style_parent_value(node, "font-size")
                    .filter(|value| !value.is_empty())
                    .unwrap_or_else(|| "16px".to_string()),
            },
            "font-weight" => match tag.as_str() {
                "strong" | "b" | "th" => default_font_weight_for_tag(&tag).to_string(),
                _ => self
                    .computed_style_parent_value(node, "font-weight")
                    .filter(|value| !value.is_empty())
                    .unwrap_or_else(|| "400".to_string()),
            },
            "font-family" => self
                .computed_style_parent_value(node, "font-family")
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| "sans-serif".to_string()),
            "font-style" => "normal".to_string(),
            "line-height" => self
                .computed_style_parent_value(node, "line-height")
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| "normal".to_string()),
            "text-align" => self
                .computed_style_parent_value(node, "text-align")
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| "left".to_string()),
            "white-space" => self
                .computed_style_parent_value(node, "white-space")
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| "normal".to_string()),
            "text-decoration" => "none".to_string(),
            "text-transform" => "none".to_string(),
            "text-indent" => "0px".to_string(),
            "letter-spacing" => "normal".to_string(),
            "pointer-events" => "auto".to_string(),
            "opacity" => "1".to_string(),
            "overflow" => "visible".to_string(),
            "width" | "height" => "auto".to_string(),
            "max-width" | "min-width" | "max-height" | "min-height" => "none".to_string(),
            "margin-top" | "margin-right" | "margin-bottom" | "margin-left" | "padding-top"
            | "padding-right" | "padding-bottom" | "padding-left" => "0px".to_string(),
            "margin" | "padding" | "border-width" | "border-style" | "border-color" => {
                let suffix = prop.strip_prefix("border-").unwrap_or("");
                let (t, r, b, l) = if prop == "margin" || prop == "padding" {
                    (
                        format!("{prop}-top"),
                        format!("{prop}-right"),
                        format!("{prop}-bottom"),
                        format!("{prop}-left"),
                    )
                } else {
                    (
                        format!("border-top-{suffix}"),
                        format!("border-right-{suffix}"),
                        format!("border-bottom-{suffix}"),
                        format!("border-left-{suffix}"),
                    )
                };
                let top = self.computed_style_value(node, &t);
                let right = self.computed_style_value(node, &r);
                let bottom = self.computed_style_value(node, &b);
                let left = self.computed_style_value(node, &l);
                box_shorthand_value(&top, &right, &bottom, &left)
            }
            "border-top-width" | "border-right-width" | "border-bottom-width"
            | "border-left-width" => "0px".to_string(),
            "border-top-style" | "border-right-style" | "border-bottom-style"
            | "border-left-style" => "none".to_string(),
            "border-top-color" | "border-right-color" | "border-bottom-color"
            | "border-left-color" => "currentcolor".to_string(),
            "vertical-align" => "baseline".to_string(),
            "cursor" => "auto".to_string(),
            _ => String::new(),
        }
    }

    fn computed_style_parent_value(&mut self, node: NodeId, prop: &str) -> Option<String> {
        match self.host.read_dom(DomRead::Parent { node }) {
            Ok(DomReadResult::Node(parent)) => Some(self.computed_style_value(parent, prop)),
            _ => None,
        }
    }

    fn set_host_property(
        &mut self,
        slot: HostObjectSlot,
        key: PropertyKey,
        value: Value,
    ) -> Result<(), VmError> {
        let name = match &key { PropertyKey::String(s) => s.clone(), _ => return Ok(()) };
        match slot.class {
            HostObjectClass::Node | HostObjectClass::EventTarget => {
                let node_id = NodeId(slot.handle as u32);
                match name.as_str() {
                    "innerHTML" => {
                        let html = self.to_string(&value);
                        let _ = self.host.mutate_dom(DomMutation::SetInnerHtml { node: node_id, html });
                    }
                    "outerHTML" => {
                        let html = self.to_string(&value);
                        let _ = self.host.mutate_dom(DomMutation::SetOuterHtml { node: node_id, html });
                    }
                    "textContent" | "nodeValue" | "data" => {
                        let text = self.to_string(&value);
                        let _ = self.host.mutate_dom(DomMutation::SetTextContent { node: node_id, value: text });
                    }
                    "id" => { let v = self.to_string(&value); let _ = self.host.mutate_dom(DomMutation::SetAttribute { node: node_id, name: "id".to_string(), value: v }); }
                    "className" => { let v = self.to_string(&value); let _ = self.host.mutate_dom(DomMutation::SetAttribute { node: node_id, name: "class".to_string(), value: v }); }
                    "value" => { let v = self.to_string(&value); let _ = self.host.mutate_dom(DomMutation::SetAttribute { node: node_id, name: "value".to_string(), value: v }); }
                    "href" => { let v = self.to_string(&value); let _ = self.host.mutate_dom(DomMutation::SetAttribute { node: node_id, name: "href".to_string(), value: v }); }
                    "src" => { let v = self.to_string(&value); let _ = self.host.mutate_dom(DomMutation::SetAttribute { node: node_id, name: "src".to_string(), value: v }); }
                    "hidden" => {
                        let truthy = self.is_truthy(&value);
                        if truthy { let _ = self.host.mutate_dom(DomMutation::SetAttribute { node: node_id, name: "hidden".to_string(), value: String::new() }); }
                        else { let _ = self.host.mutate_dom(DomMutation::RemoveAttribute { node: node_id, name: "hidden".to_string() }); }
                    }
                    "disabled" => {
                        let truthy = self.is_truthy(&value);
                        if truthy { let _ = self.host.mutate_dom(DomMutation::SetAttribute { node: node_id, name: "disabled".to_string(), value: String::new() }); }
                        else { let _ = self.host.mutate_dom(DomMutation::RemoveAttribute { node: node_id, name: "disabled".to_string() }); }
                    }
                    "scrollTop" | "scrollLeft" => {
                        // Only the root element scrolls the window; other
                        // elements don't track their own overflow yet.
                        let is_root = matches!(
                            self.host.read_dom(DomRead::DocumentRoot { window: WindowId(0) }),
                            Ok(DomReadResult::Node(root)) if root == node_id
                        );
                        if is_root {
                            let n = self.to_number(&value);
                            let (cur_x, cur_y) = self
                                .host
                                .window_metrics(WindowId(0))
                                .map(|m| (m.scroll_x, m.scroll_y))
                                .unwrap_or((0.0, 0.0));
                            let (x, y) = if name == "scrollLeft" { (n, cur_y) } else { (cur_x, n) };
                            let _ = self.host.mutate_dom(DomMutation::SetWindowScroll { window: WindowId(0), x, y });
                        }
                    }
                    _ => {}
                }
            }
            HostObjectClass::Other("CSSStyleDeclaration") => {
                let node_id = NodeId(slot.handle as u32);
                let css_prop = camel_to_css_prop(&name);
                let new_val = self.to_string(&value);
                let existing = match self.host.read_dom(DomRead::Attribute { node: node_id, name: "style".to_string() }) {
                    Ok(DomReadResult::String(s)) => s,
                    _ => String::new(),
                };
                let updated = set_inline_style_prop(&existing, &css_prop, &new_val);
                let _ = self.host.mutate_dom(DomMutation::SetAttribute { node: node_id, name: "style".to_string(), value: updated });
            }
            HostObjectClass::Other("Dataset") => {
                // `el.dataset.fooBar = v` writes the `data-foo-bar` attribute.
                let node_id = NodeId(slot.handle as u32);
                let attr = format!("data-{}", camel_to_css_prop(&name));
                let v = self.to_string(&value);
                let _ = self.host.mutate_dom(DomMutation::SetAttribute {
                    node: node_id,
                    name: attr,
                    value: v,
                });
            }
            HostObjectClass::Document => {
                // `document.title = …` writes the <head><title> text, creating
                // the element if the document has none.
                if name == "title" {
                    let text = self.to_string(&value);
                    let head = match self.host.read_dom(DomRead::DocumentHead { window: WindowId(0) }) {
                        Ok(DomReadResult::Node(id)) => Some(id),
                        // No <head> yet: create one under documentElement (or
                        // the document itself) — boa creates it on demand too.
                        _ => {
                            let parent = match self.host.read_dom(DomRead::DocumentRoot { window: WindowId(0) }) {
                                Ok(DomReadResult::Node(root)) => root,
                                _ => NodeId(0),
                            };
                            match self.host.mutate_dom(DomMutation::CreateElement { window: WindowId(0), local_name: "head".to_string() }) {
                                Ok(DomMutationResult::Node(h)) => {
                                    let _ = self.host.mutate_dom(DomMutation::Append { parent, children: vec![h] });
                                    Some(h)
                                }
                                _ => None,
                            }
                        }
                    };
                    let existing = head.and_then(|h| {
                        match self.host.read_dom(DomRead::QuerySelector { root: h, selectors: "title".to_string() }) {
                            Ok(DomReadResult::Node(id)) => Some(id),
                            _ => None,
                        }
                    });
                    match existing {
                        Some(title) => {
                            let _ = self.host.mutate_dom(DomMutation::SetTextContent { node: title, value: text });
                        }
                        None => {
                            if let Some(head) = head {
                                if let Ok(DomMutationResult::Node(title)) = self.host.mutate_dom(DomMutation::CreateElement { window: WindowId(0), local_name: "title".to_string() }) {
                                    let _ = self.host.mutate_dom(DomMutation::Append { parent: head, children: vec![title] });
                                    let _ = self.host.mutate_dom(DomMutation::SetTextContent { node: title, value: text });
                                }
                            }
                        }
                    }
                }
            }
            HostObjectClass::Other("Location") => {
                let v = self.to_string(&value);
                match name.as_str() {
                    "hash" => {
                        let _ = self.host.navigate(NavigationAction::SetHash {
                            window: WindowId(0),
                            hash: v,
                        });
                        // A hash change fires `hashchange` on the window.
                        let _ = self.fire_dom_event(0, "hashchange");
                    }
                    "href" => {
                        // Same-document if only the fragment differs from the
                        // current URL: treat as a hash change (+ hashchange);
                        // otherwise a full navigation (reload).
                        let current = self
                            .host
                            .location(WindowId(0))
                            .map(|l| l.href)
                            .unwrap_or_default();
                        let cur_base = current.split('#').next().unwrap_or("");
                        let (new_base, new_hash) = match v.split_once('#') {
                            Some((b, h)) => (b, Some(h.to_string())),
                            None => (v.as_str(), None),
                        };
                        let same_doc = new_hash.is_some()
                            && (new_base.is_empty() || new_base == cur_base);
                        if same_doc {
                            let _ = self.host.navigate(NavigationAction::SetHash {
                                window: WindowId(0),
                                hash: new_hash.unwrap_or_default(),
                            });
                            let _ = self.fire_dom_event(0, "hashchange");
                        } else {
                            let _ = self.host.navigate(NavigationAction::Navigate {
                                window: WindowId(0),
                                url: v,
                                replace: false,
                            });
                        }
                    }
                    // pathname/search/etc. assignment: a full navigation to the
                    // rebuilt URL. Kept minimal; the tests exercise href/hash.
                    _ => {}
                }
            }
            HostObjectClass::Window => {
                // `globalThis`/`window`/`self` IS the global object, so an expando
                // assignment (`window.X = …`, `globalThis.X = …`) creates/updates
                // a global binding. Frameworks rely on this (`window.React`,
                // feature-detection flags, UMD globals, …).
                self.globals.insert(name, value);
            }
            _ => {}
        }
        Ok(())
    }

    // ----------------------------------------------------------------
    // Storage helpers
    // ----------------------------------------------------------------

    fn make_storage_object(&mut self, kind: super::host::StorageAreaKind) -> Value {
        let handle: u64 = match kind {
            super::host::StorageAreaKind::Local => 0,
            super::host::StorageAreaKind::Session => 1,
            super::host::StorageAreaKind::Cookie => 2,
        };
        let obj = self.heap.allocate_object(JsObject {
            kind: ObjectKind::Host(HostObjectSlot {
                class: HostObjectClass::StorageArea,
                interface_name: "Storage",
                handle,
                dispatch: HostDispatch::Ordinary,
                supports_indexed_properties: false,
                supports_named_properties: false,
            }),
            ..JsObject::default()
        });
        Value::Object(obj)
    }

    fn storage_kind_from_host_val(&self, value: &Value) -> super::host::StorageAreaKind {
        if let Value::Object(obj_ref) = value {
            if let Some(obj) = self.heap.objects().get(*obj_ref) {
                if let ObjectKind::Host(slot) = &obj.kind {
                    return match slot.handle {
                        1 => super::host::StorageAreaKind::Session,
                        2 => super::host::StorageAreaKind::Cookie,
                        _ => super::host::StorageAreaKind::Local,
                    };
                }
            }
        }
        super::host::StorageAreaKind::Local
    }

    fn storage_context(&self, this_value: &Value) -> (StorageAreaKind, StorageAreaScope) {
        (
            self.storage_kind_from_host_val(this_value),
            StorageAreaScope::Window(WindowId(0)),
        )
    }

    fn get_storage_property(&mut self, slot: HostObjectSlot, name: String) -> Result<Value, VmError> {
        use super::host::{StorageAreaScope, StorageOp, StorageResult};
        let kind = match slot.handle {
            1 => super::host::StorageAreaKind::Session,
            2 => super::host::StorageAreaKind::Cookie,
            _ => super::host::StorageAreaKind::Local,
        };
        match name.as_str() {
            "length" => {
                let res = self.host.storage(StorageOp::Len {
                    kind,
                    scope: StorageAreaScope::Window(WindowId(0)),
                });
                Ok(match res { Ok(StorageResult::Len(n)) => Value::Number(n as f64), _ => Value::Number(0.0) })
            }
            "getItem" => Ok(self.allocate_builtin_method(BuiltinId::StorageGetItem)),
            "setItem" => Ok(self.allocate_builtin_method(BuiltinId::StorageSetItem)),
            "removeItem" => Ok(self.allocate_builtin_method(BuiltinId::StorageRemoveItem)),
            "clear" => Ok(self.allocate_builtin_method(BuiltinId::StorageClear)),
            "key" => Ok(self.allocate_builtin_method(BuiltinId::StorageKey)),
            _ => {
                // Named property access: storage[key]
                let res = self.host.storage(StorageOp::Get {
                    kind,
                    scope: StorageAreaScope::Window(WindowId(0)),
                    key: name,
                });
                Ok(match res { Ok(StorageResult::Value(Some(v))) => self.make_string_value(&v), _ => Value::Null })
            }
        }
    }
}

impl From<&str> for PropertyKey {
    fn from(value: &str) -> Self {
        Vm::property_key_from_text(value)
    }
}

impl From<String> for PropertyKey {
    fn from(value: String) -> Self {
        Vm::property_key_from_text(&value)
    }
}

#[cfg(test)]
mod tests {
    use super::Vm;
    use crate::engine::ast::SourceType;
    use crate::engine::{Compiler, Heap, JsPropertyDescriptor, Parser, PropertyKey, Value};
    use crate::engine::compiler::ModuleContext;

    fn run_script(source: &str) {
        let program = Parser::new(source).parse().expect("script should parse");
        let chunk = Compiler::new(&program)
            .compile()
            .expect("script should compile");
        let mut vm = Vm::new(Heap::new());
        vm.execute(&chunk).expect("script should execute");
    }

    fn execute_script(source: &str) -> (Vm, Result<(), crate::engine::vm::VmError>) {
        let program = Parser::new(source).parse().expect("script should parse");
        let chunk = Compiler::new(&program)
            .compile()
            .expect("script should compile");
        let mut vm = Vm::new(Heap::new());
        let result = vm.execute(&chunk).map(|_| ());
        (vm, result)
    }

    #[test]
    fn phase_2_arithmetic_and_coercion_corpus() {
        run_script(
            r#"
            assert(1 + 2 === 3);
            assert("a" + "b" === "ab");
            assert(1 + "2" === "12");
            assert(typeof undefined === "undefined");
            assert(typeof null === "object");
            assert(null == undefined);
            assert(null !== undefined);
            assert(NaN !== NaN);
            "#,
        );
    }

    #[test]
    fn phase_2_control_flow_corpus() {
        run_script(
            r#"
            let x = 0;
            for (let i = 0; i < 5; i++) { x += i; }
            assert(x === 10);

            let s = "";
            let i = 0;
            while (i < 3) { s += i; i++; }
            assert(s === "012");

            function fact(n) { if (n <= 1) return 1; return n * fact(n - 1); }
            assert(fact(5) === 120);
            "#,
        );
    }

    #[test]
    fn phase_2_closure_corpus() {
        run_script(
            r#"
            function makeCounter() { let n = 0; return function() { return ++n; }; }
            const c = makeCounter();
            assert(c() === 1);
            assert(c() === 2);
            assert(c() === 3);

            function adder(x) { return function(y) { return x + y; }; }
            const add5 = adder(5);
            assert(add5(3) === 8);
            "#,
        );
    }

    #[test]
    fn phase_2_scope_corpus() {
        run_script(
            r#"
            var a = 1;
            { var a = 2; }
            assert(a === 2);

            let b = 1;
            { let b = 2; }
            assert(b === 1);
            "#,
        );
    }

    #[test]
    fn phase_3_object_property_access_corpus() {
        run_script(
            r#"
            const obj = { x: 1, y: 2 };
            assert(obj.x === 1);
            obj.z = 3;
            assert(obj.z === 3);
            assert(obj["y"] === 2);
            assert(typeof {} === "object");
            "#,
        );
    }

    #[test]
    fn object_introspection_to_object_semantics() {
        run_script(
            r#"
            assert(JSON.stringify(Object.getOwnPropertyNames(42)) === "[]");
            assert(JSON.stringify(Object.keys(42)) === "[]");

            assert(JSON.stringify(Object.keys("ab")) === "[\"0\",\"1\"]");
            assert(JSON.stringify(Object.getOwnPropertyNames("ab")) === "[\"0\",\"1\",\"length\"]");
            assert(JSON.stringify(Object.values("ab")) === "[\"a\",\"b\"]");

            assert(Object.getOwnPropertyDescriptor("ab", "0").value === "a");
            assert(Object.getOwnPropertyDescriptor("ab", "length").value === 2);
            assert(Object.getPrototypeOf("x") === String.prototype);

            let threw = false;
            try { Object.keys(null); } catch (e) { threw = true; }
            assert(threw === true);

            threw = false;
            try { Object.getOwnPropertyNames(undefined); } catch (e) { threw = true; }
            assert(threw === true);

            assert(Object.keys({ a: 1, b: 2 }).length === 2);
            "#,
        );
    }

    #[test]
    fn module_top_level_this_is_undefined() {
        let program = Parser::new("globalThis.__t = (typeof this);")
            .with_source_type(SourceType::Module)
            .parse()
            .expect("module should parse");
        let chunk = Compiler::new(&program)
            .compile()
            .expect("module should compile");
        let mut vm = Vm::new(Heap::new());
        vm.execute_module(&chunk).expect("module should execute");
        assert_eq!(
            vm.globals.get("__t").map(|value| vm.to_string(value)),
            Some("undefined".to_string())
        );
    }

    #[test]
    fn classic_top_level_this_stays_global_object() {
        let program = Parser::new("globalThis.__t = (typeof this);")
            .parse()
            .expect("script should parse");
        let chunk = Compiler::new(&program)
            .compile()
            .expect("script should compile");
        let mut vm = Vm::new(Heap::new());
        vm.execute(&chunk).expect("script should execute");
        assert_eq!(
            vm.globals.get("__t").map(|value| vm.to_string(value)),
            Some("object".to_string())
        );
    }

    #[test]
    fn function_constructor_is_global_and_callable() {
        run_script(
            r#"
            assert(typeof Function === "function");
            assert(new Function("a","b","return a+b")(2,3) === 5);
            assert(Function("return 42")() === 42);
            assert(typeof Function.prototype !== "undefined");
            assert((function(){}) instanceof Function === true);
            assert(typeof Function.prototype.call === "function");
            assert(typeof Function.prototype.apply === "function");
            "#,
        );
    }

    #[test]
    fn module_exports_compile_and_run() {
        let self_key = "\u{0}module:test".to_string();
        let program = Parser::new(
            r#"
            export const x = 5;
            globalThis.__r = x * 2;
            export function f(){ return 7; }
            globalThis.__r2 = f();
            export default function(){ }
            "#,
        )
        .with_source_type(SourceType::Module)
        .parse()
        .expect("module should parse");
        let chunk = Compiler::new(&program)
            .with_module_context(ModuleContext {
                self_key: self_key.clone(),
                imports: Default::default(),
                dynamic_imports: Default::default(),
            })
            .compile()
            .expect("module should compile");
        let mut vm = Vm::new(Heap::new());
        vm.set_global_object(self_key);
        vm.execute_module(&chunk).expect("module should execute");
        assert_eq!(vm.globals.get("__r").cloned(), Some(Value::Number(10.0)));
        assert_eq!(vm.globals.get("__r2").cloned(), Some(Value::Number(7.0)));
    }

    #[test]
    fn module_named_function_and_class_exports_use_their_own_names() {
        let self_key = "\u{0}module:self".to_string();
        let program = Parser::new(
            r#"
            export function f(){ return 7; }
            export class C {}
            "#,
        )
        .with_source_type(SourceType::Module)
        .parse()
        .expect("module should parse");
        let chunk = Compiler::new(&program)
            .with_module_context(ModuleContext {
                self_key: self_key.clone(),
                imports: Default::default(),
                dynamic_imports: Default::default(),
            })
            .compile()
            .expect("module should compile");
        let mut vm = Vm::new(Heap::new());
        vm.set_global_object(self_key.clone());
        vm.execute_module(&chunk).expect("module should execute");

        let self_ns = match vm.globals.get(&self_key) {
            Some(Value::Object(object)) => *object,
            _ => panic!("self namespace should exist"),
        };
        let f = vm
            .get_own_property_descriptor(self_ns, &PropertyKey::from("f"))
            .expect("f should be exported");
        let c = vm
            .get_own_property_descriptor(self_ns, &PropertyKey::from("C"))
            .expect("C should be exported");
        assert!(matches!(f, JsPropertyDescriptor::Data { value: Value::Object(_), .. }));
        assert!(matches!(c, JsPropertyDescriptor::Data { value: Value::Object(_), .. }));
        assert!(
            vm.get_own_property_descriptor(self_ns, &PropertyKey::from("default"))
                .is_none()
        );
        assert!(matches!(
            vm.get_property_value(&Value::Object(self_ns), &PropertyKey::from("f"))
                .expect("f should read"),
            Value::Object(_)
        ));
        let script = Parser::new(
            r#"
            globalThis.__call = globalThis["\u{0}module:self"].f();
            "#,
        )
        .parse()
        .expect("script should parse");
        let chunk = Compiler::new(&script)
            .compile()
            .expect("script should compile");
        vm.execute(&chunk).expect("script should execute");
        assert_eq!(vm.globals.get("__call").cloned(), Some(Value::Number(7.0)));
    }

    #[test]
    fn module_named_export_list_compiles_without_error() {
        let self_key = "\u{0}module:test".to_string();
        let program = Parser::new(
            r#"
            const x = 1;
            export { x };
            globalThis.__r3 = x;
            "#,
        )
        .with_source_type(SourceType::Module)
        .parse()
        .expect("module should parse");
        let chunk = Compiler::new(&program)
            .with_module_context(ModuleContext {
                self_key: self_key.clone(),
                imports: Default::default(),
                dynamic_imports: Default::default(),
            })
            .compile()
            .expect("module should compile");
        let mut vm = Vm::new(Heap::new());
        vm.set_global_object(self_key);
        vm.execute_module(&chunk).expect("module should execute");
        assert_eq!(vm.globals.get("__r3").cloned(), Some(Value::Number(1.0)));
    }

    #[test]
    fn module_imports_are_live_bindings_not_snapshots() {
        let dep_key = "\u{0}module:dep".to_string();
        let self_key = "\u{0}module:self".to_string();
        let program = Parser::new(
            r#"
            import { get } from "./dep";
            globalThis.__call = () => get();
            "#,
        )
        .with_source_type(SourceType::Module)
        .parse()
        .expect("module should parse");
        let chunk = Compiler::new(&program)
            .with_module_context(ModuleContext {
                self_key: self_key.clone(),
                imports: std::iter::once(("./dep".to_string(), dep_key.clone())).collect(),
                dynamic_imports: Default::default(),
            })
            .compile()
            .expect("module should compile");
        let mut vm = Vm::new(Heap::new());
        vm.set_global_object(self_key.clone());
        let dep_ns = vm.allocate_ordinary_object(None);
        vm.globals.insert(dep_key.clone(), Value::Object(dep_ns));
        vm.execute_module(&chunk).expect("module should execute");

        let program = Parser::new(
            r#"
            globalThis["\u{0}module:dep"].get = () => 42;
            globalThis.__result = globalThis.__call();
            "#,
        )
        .parse()
        .expect("script should parse");
        let chunk = Compiler::new(&program)
            .compile()
            .expect("script should compile");
        vm.execute(&chunk).expect("script should execute");
        assert_eq!(vm.globals.get("__result").cloned(), Some(Value::Number(42.0)));
    }

    #[test]
    fn module_import_is_shadowed_by_local_binding() {
        let dep_key = "\u{0}module:dep".to_string();
        let self_key = "\u{0}module:self".to_string();
        let program = Parser::new(
            r#"
            import { get } from "./dep";
            {
                let get = () => 7;
                globalThis.__shadow = get();
            }
            "#,
        )
        .with_source_type(SourceType::Module)
        .parse()
        .expect("module should parse");
        let chunk = Compiler::new(&program)
            .with_module_context(ModuleContext {
                self_key: self_key.clone(),
                imports: std::iter::once(("./dep".to_string(), dep_key.clone())).collect(),
                dynamic_imports: Default::default(),
            })
            .compile()
            .expect("module should compile");
        let mut vm = Vm::new(Heap::new());
        vm.set_global_object(self_key);
        let dep_ns = vm.allocate_ordinary_object(None);
        vm.globals.insert(dep_key, Value::Object(dep_ns));
        vm.execute_module(&chunk).expect("module should execute");
        assert_eq!(vm.globals.get("__shadow").cloned(), Some(Value::Number(7.0)));
    }

    #[test]
    fn module_reexport_all_copies_enumerable_exports_except_default() {
        let dep_key = "\u{0}module:dep".to_string();
        let self_key = "\u{0}module:self".to_string();
        let mut vm = Vm::new(Heap::new());

        let dep_ns = vm.allocate_ordinary_object(None);
        vm.define_data_property(dep_ns, PropertyKey::from("a"), Value::Number(1.0), true, true, true);
        vm.define_data_property(dep_ns, PropertyKey::from("b"), Value::Number(2.0), true, true, true);
        vm.define_data_property(dep_ns, PropertyKey::from("default"), Value::Number(99.0), true, true, true);
        vm.globals.insert(dep_key.clone(), Value::Object(dep_ns));
        vm.set_global_object(self_key.clone());

        let program = Parser::new(
            r#"
            export * from "./dep";
            "#,
        )
        .with_source_type(SourceType::Module)
        .parse()
        .expect("module should parse");
        let chunk = Compiler::new(&program)
            .with_module_context(ModuleContext {
                self_key: self_key.clone(),
                imports: std::iter::once(("./dep".to_string(), dep_key.clone())).collect(),
                dynamic_imports: Default::default(),
            })
            .compile()
            .expect("module should compile");
        vm.execute_module(&chunk).expect("module should execute");

        let self_ns = match vm.globals.get(&self_key) {
            Some(Value::Object(object)) => *object,
            _ => panic!("self namespace should exist"),
        };
        assert_eq!(
            vm.get_property_value(&Value::Object(self_ns), &PropertyKey::from("a"))
                .expect("a should read"),
            Value::Number(1.0)
        );
        assert_eq!(
            vm.get_property_value(&Value::Object(self_ns), &PropertyKey::from("b"))
                .expect("b should read"),
            Value::Number(2.0)
        );
        assert_eq!(
            vm.get_property_value(&Value::Object(self_ns), &PropertyKey::from("default"))
                .expect("default should read"),
            Value::Undefined
        );
    }

    #[test]
    fn object_integrity_methods_accept_primitives_and_still_affect_objects() {
        run_script(
            r#"
            assert(Object.freeze(42) === 42);
            assert(Object.freeze("s") === "s");
            assert(Object.freeze(null) === null);
            assert(Object.freeze(undefined) === undefined);

            assert(Object.isFrozen(42) === true);
            assert(Object.isSealed("x") === true);
            assert(Object.isExtensible(42) === false);

            var o = { a: 1 };
            Object.freeze(o);
            o.a = 2;
            assert(o.a === 1);
            assert(Object.isFrozen(o) === true);
            "#,
        );
    }

    #[test]
    fn phase_3_prototype_new_and_this_corpus() {
        run_script(
            r#"
            function Animal(name) { this.name = name; }
            Animal.prototype.speak = function() { return this.name + " speaks"; };
            const dog = new Animal("Dog");
            assert(dog.name === "Dog");
            assert(dog.speak() === "Dog speaks");
            assert(dog.hasOwnProperty("name") === true);
            assert(dog.hasOwnProperty("speak") === false);

            const proto = { greet() { return "hello " + this.name; } };
            const obj2 = Object.create(proto);
            obj2.name = "world";
            assert(obj2.greet() === "hello world");

            Object.defineProperty(obj2, "id", { value: 42, writable: false, enumerable: true, configurable: false });
            assert(obj2.id === 42);
            "#,
        );
    }

    #[test]
    fn phase_3_array_corpus() {
        run_script(
            r#"
            const arr = [1, 2, 3];
            assert(arr.length === 3);
            assert(arr[0] === 1);
            arr.push(4);
            assert(arr.length === 4);
            const mapped = arr.map(x => x * 2);
            assert(mapped[0] === 2);
            assert(mapped[3] === 8);
            assert(arr.includes(2) === true);
            assert(arr.indexOf(3) === 2);
            "#,
        );
    }

    #[test]
    fn phase_3_string_and_math_corpus() {
        run_script(
            r#"
            assert("hello world".includes("world") === true);
            assert("hello".toUpperCase() === "HELLO");
            assert("  hi  ".trim() === "hi");
            assert("a,b,c".split(",").length === 3);
            assert(Math.floor(3.7) === 3);
            assert(Math.max(1, 2, 3) === 3);
            assert(Math.abs(-5) === 5);
            assert(typeof Math.random() === "number");
            "#,
        );
    }

    #[test]
    fn phase_4_try_catch_finally_corpus() {
        run_script(
            r#"
            let result = "";
            try {
              result += "try";
              throw new Error("test");
              result += "never";
            } catch (e) {
              result += " catch:" + e.message;
            } finally {
              result += " finally";
            }
            assert(result === "try catch:test finally");
            "#,
        );
    }

    #[test]
    fn uncaught_throw_captures_backtrace_in_call_order() {
        let (mut vm, result) = execute_script(
            r#"
            function outer() { inner(); }
            function inner() { null.x; }
            outer();
            "#,
        );
        let error = result.expect_err("script should throw");
        let backtrace = vm
            .take_last_backtrace()
            .expect("backtrace should be captured");
        assert!(!format!("{error}").is_empty());
        let inner = backtrace.find("    at inner").expect("inner frame");
        let outer = backtrace.find("    at outer").expect("outer frame");
        let script = backtrace.find("    at <script>").expect("script frame");
        assert!(inner < outer, "backtrace order was not innermost-first: {backtrace}");
        assert!(outer < script, "backtrace order was not innermost-first: {backtrace}");
    }

    #[test]
    fn non_function_call_mentions_value_type() {
        let (mut vm, result) = execute_script(
            r#"
            undefined();
            "#,
        );
        let error = result.expect_err("script should fail");
        let message = format!("{error}");
        assert!(message.contains("non-function value"), "{message}");
        assert!(message.contains("undefined"), "{message}");
        assert!(vm.take_last_backtrace().is_some(), "backtrace should be captured");
    }

    #[test]
    fn method_call_diagnostics_include_missing_property_name() {
        let (_, result) = execute_script(
            r#"
            var o = {};
            o.missing();
            "#,
        );
        let error = result.expect_err("script should fail");
        let message = format!("{error}");
        assert!(message.contains("missing is not a function"), "{message}");
        assert!(message.contains("undefined"), "{message}");
    }

    #[test]
    fn indexed_method_call_diagnostics_include_string_key_name() {
        let (_, result) = execute_script(
            r#"
            var o = { a: 1 };
            o["b"]();
            "#,
        );
        let error = result.expect_err("script should fail");
        let message = format!("{error}");
        assert!(message.contains("b is not a function"), "{message}");
    }

    #[test]
    fn plain_non_method_calls_and_successful_method_calls_still_work() {
        let (mut vm, result) = execute_script(
            r#"
            undefined();
            "#,
        );
        let error = result.expect_err("script should fail");
        let message = format!("{error}");
        assert!(message.contains("non-function value"), "{message}");
        assert!(message.contains("undefined"), "{message}");
        assert!(vm.take_last_backtrace().is_some(), "backtrace should be captured");

        run_script(
            r#"
            var o = { f() { return 5; } };
            assert(o.f() === 5);
            "#,
        );
    }

    #[test]
    fn phase_4_destructuring_corpus() {
        run_script(
            r#"
            const { a, b: renamed, c = 99 } = { a: 1, b: 2 };
            assert(a === 1);
            assert(renamed === 2);
            assert(c === 99);

            const [x, , z, ...rest] = [10, 20, 30, 40, 50];
            assert(x === 10);
            assert(z === 30);
            assert(rest.length === 2);
            assert(rest[0] === 40);
            "#,
        );
    }

    #[test]
    fn phase_4_class_and_super_corpus() {
        run_script(
            r#"
            class Animal {
              constructor(name) { this.name = name; }
              speak() { return this.name + " makes a noise."; }
            }
            class Dog extends Animal {
              constructor(name) { super(name); }
              speak() { return super.speak() + " Woof!"; }
            }
            const d = new Dog("Rex");
            assert(d.speak() === "Rex makes a noise. Woof!");
            assert(d instanceof Dog);
            assert(d instanceof Animal);
            "#,
        );
    }

    #[test]
    fn phase_4_map_and_set_corpus() {
        run_script(
            r#"
            const m = new Map();
            m.set("a", 1);
            m.set("b", 2);
            assert(m.get("a") === 1);
            assert(m.has("b") === true);
            assert(m.size === 2);

            const s = new Set([1, 2, 3, 2, 1]);
            assert(s.size === 3);
            assert(s.has(2) === true);
            "#,
        );
    }

    #[test]
    fn phase_4_spread_and_rest_corpus() {
        run_script(
            r#"
            function sum(...nums) { return nums.reduce((a, b) => a + b, 0); }
            assert(sum(1, 2, 3) === 6);
            "#,
        );
        run_script(
            r#"
            function sum(...nums) { return nums.reduce((a, b) => a + b, 0); }
            const parts = [3, 4];
            assert(sum(1, 2, ...parts) === 10);
            "#,
        );
    }

    #[test]
    fn phase_4_nullish_optional_and_switch_corpus() {
        run_script(
            r#"
            const x2 = null ?? "default";
            assert(x2 === "default");

            const obj = { nested: { value: 42 } };
            assert(obj?.nested?.value === 42);
            assert(obj?.missing?.value === undefined);

            let sw = "";
            switch (2) {
              case 1: sw = "one"; break;
              case 2: sw = "two"; break;
              default: sw = "other";
            }
            assert(sw === "two");
            "#,
        );
    }

    #[test]
    fn performance_api_minimal_surface() {
        run_script(
            r#"
            assert(typeof performance.mark === "function");
            const mark = performance.mark("x");
            assert(mark && typeof mark === "object");
            assert(mark.entryType === "mark");
            assert(mark.name === "x");

            const measure = performance.measure("m");
            assert(measure && typeof measure === "object");
            assert(measure.entryType === "measure");

            const entries = performance.getEntriesByName("x");
            assert(Array.isArray(entries) === true);
            assert(entries.length === 0);

            assert(performance.clearMarks("x") === undefined);
            assert(typeof performance.now() === "number");
            "#,
        );
    }

    #[test]
    fn abort_signal_global_and_controller_surface() {
        run_script(
            r#"
            assert(typeof AbortSignal !== "undefined");
            assert(typeof AbortSignal === "function" || typeof AbortSignal === "object");
            assert(typeof AbortSignal.abort === "function");
            assert(typeof AbortSignal.timeout === "function");
            assert(typeof AbortSignal.any === "function");

            const aborted = AbortSignal.abort();
            assert(aborted.aborted === true);

            const timeoutSignal = AbortSignal.timeout(50);
            assert(timeoutSignal.aborted === false);

            const controller = new AbortController();
            assert(controller.signal.aborted === false);
            controller.abort("x");
            assert(controller.signal.aborted === true);
            assert(controller.signal.reason === "x");

            const s = AbortSignal.abort("boom");
            let threw = false;
            try {
              s.throwIfAborted();
            } catch (e) {
              threw = (e === "boom");
            }
            assert(threw === true);
            "#,
        );
    }
}

    // ----------------------------------------------------------------
    // Storage helpers
    // ----------------------------------------------------------------

// ---------------------------------------------------------------------------
// CSS inline-style helpers (free functions, no VM state needed)
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Base64 helpers (no external dependency)
// ---------------------------------------------------------------------------

const BASE64_CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn base64_encode(input: &[u8]) -> String {
    let mut out = String::with_capacity((input.len() + 2) / 3 * 4);
    for chunk in input.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = if chunk.len() > 1 { chunk[1] as u32 } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] as u32 } else { 0 };
        let val = (b0 << 16) | (b1 << 8) | b2;
        out.push(BASE64_CHARS[((val >> 18) & 63) as usize] as char);
        out.push(BASE64_CHARS[((val >> 12) & 63) as usize] as char);
        out.push(if chunk.len() > 1 { BASE64_CHARS[((val >> 6) & 63) as usize] as char } else { '=' });
        out.push(if chunk.len() > 2 { BASE64_CHARS[(val & 63) as usize] as char } else { '=' });
    }
    out
}

fn base64_decode(input: &str) -> Option<Vec<u8>> {
    let mut table = [255u8; 256];
    for (i, &c) in BASE64_CHARS.iter().enumerate() {
        table[c as usize] = i as u8;
    }
    let input: Vec<u8> = input.bytes().filter(|&b| b != b'=').collect();
    let mut out = Vec::with_capacity(input.len() * 3 / 4);
    for chunk in input.chunks(4) {
        let get = |i: usize| -> Option<u8> {
            chunk.get(i).and_then(|&b| {
                let v = table[b as usize];
                if v == 255 { None } else { Some(v) }
            })
        };
        let a = get(0)?;
        let b = get(1)?;
        out.push((a << 2) | (b >> 4));
        if let Some(c) = get(2) {
            out.push((b << 4) | (c >> 2));
            if let Some(d) = get(3) {
                out.push((c << 6) | d);
            }
        }
    }
    Some(out)
}

fn camel_to_css_prop(camel: &str) -> String {
    let mut out = String::new();
    for ch in camel.chars() {
        if ch.is_uppercase() {
            out.push('-');
            out.push(ch.to_ascii_lowercase());
        } else {
            out.push(ch);
        }
    }
    out
}

/// Property names that `set_host_property` reflects to the DOM for an element
/// (so they must NOT be treated as plain expando own-properties). Keep in sync
/// with the `Node | EventTarget` arm of `set_host_property`.
fn is_dom_managed_node_property(name: &str) -> bool {
    matches!(
        name,
        "innerHTML"
            | "outerHTML"
            | "textContent"
            | "nodeValue"
            | "data"
            | "id"
            | "className"
            | "value"
            | "href"
            | "src"
            | "hidden"
            | "disabled"
            | "scrollTop"
            | "scrollLeft"
    )
}

/// Property names that `get_document_property` manages directly.
/// Keep this list in sync with `get_document_property`.
fn is_dom_managed_document_property(name: &str) -> bool {
    matches!(
        name,
        "ownerDocument"
            | "body"
            | "head"
            | "documentElement"
            | "title"
            | "nodeType"
            | "nodeName"
            | "readyState"
            | "compatMode"
            | "charset"
            | "characterSet"
            | "location"
            | "URL"
            | "documentURI"
            | "domain"
            | "querySelector"
            | "querySelectorAll"
            | "getElementById"
            | "getElementsByClassName"
            | "getElementsByTagName"
            | "createElement"
            | "createElementNS"
            | "createTextNode"
            | "createDocumentFragment"
            | "write"
            | "writeln"
            | "addEventListener"
            | "removeEventListener"
            | "cookie"
            | "referrer"
            | "hidden"
            | "visibilityState"
            | "activeElement"
            | "createEvent"
            | "createComment"
            | "implementation"
    )
}

/// Read one declaration value out of an inline `style` attribute string.
fn get_inline_style_prop(existing: &str, prop: &str) -> String {
    existing
        .split(';')
        .find_map(|part| {
            let mut iter = part.splitn(2, ':');
            let k = iter.next()?.trim();
            if k.eq_ignore_ascii_case(prop) {
                Some(iter.next().unwrap_or("").trim().to_string())
            } else {
                None
            }
        })
        .unwrap_or_default()
}

// ── getComputedStyle UA defaults (ported from the boa backend) ──────────────

fn default_display_for_tag(tag_name: &str) -> &'static str {
    match tag_name {
        "html" | "body" | "div" | "section" | "article" | "aside" | "main" | "header"
        | "footer" | "nav" | "p" | "ul" | "ol" | "form" | "fieldset" | "legend" | "pre"
        | "blockquote" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => "block",
        "table" => "table",
        "tr" => "table-row",
        "td" | "th" => "table-cell",
        "thead" | "tbody" | "tfoot" => "table-row-group",
        "li" => "list-item",
        "img" | "button" | "input" | "select" | "textarea" => "inline-block",
        "span" | "a" | "b" | "i" | "u" | "strong" | "em" | "small" | "code" | "abbr" | "label"
        | "sup" | "sub" | "mark" => "inline",
        "script" | "style" | "head" | "meta" | "link" | "title" | "template" => "none",
        _ => "inline",
    }
}

fn default_font_size_for_tag(tag_name: &str) -> &'static str {
    match tag_name {
        "h1" => "2em",
        "h2" => "1.5em",
        "h3" => "1.17em",
        "h4" => "1em",
        "h5" => "0.83em",
        "h6" => "0.67em",
        _ => "16px",
    }
}

fn default_font_weight_for_tag(tag_name: &str) -> &'static str {
    match tag_name {
        "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "strong" | "b" | "th" => "700",
        _ => "400",
    }
}

fn default_background_color_for_tag(tag_name: &str) -> &'static str {
    match tag_name {
        "html" | "body" => "rgb(255, 255, 255)",
        _ => "rgba(0, 0, 0, 0)",
    }
}

fn box_shorthand_value(top: &str, right: &str, bottom: &str, left: &str) -> String {
    if top == right && right == bottom && bottom == left {
        top.to_string()
    } else if top == bottom && right == left {
        format!("{top} {right}")
    } else if right == left {
        format!("{top} {right} {bottom}")
    } else {
        format!("{top} {right} {bottom} {left}")
    }
}

/// Remove one declaration from an inline `style` attribute string. Returns the
/// updated declaration string and the removed value (empty if absent).
fn remove_inline_style_prop(existing: &str, prop: &str) -> (String, String) {
    let mut removed = String::new();
    let kept: Vec<String> = existing
        .split(';')
        .filter_map(|part| {
            let part = part.trim();
            if part.is_empty() {
                return None;
            }
            let mut iter = part.splitn(2, ':');
            let k = iter.next()?.trim();
            let v = iter.next().unwrap_or("").trim();
            if k.eq_ignore_ascii_case(prop) {
                removed = v.to_string();
                None
            } else {
                Some(format!("{k}: {v}"))
            }
        })
        .collect();
    (kept.join("; "), removed)
}

fn set_inline_style_prop(existing: &str, prop: &str, value: &str) -> String {
    // Parse existing "prop: val; prop2: val2" and replace or append
    let mut props: Vec<(String, String)> = existing
        .split(';')
        .filter_map(|part| {
            let part = part.trim();
            if part.is_empty() { return None; }
            let mut iter = part.splitn(2, ':');
            let k = iter.next()?.trim().to_string();
            let v = iter.next().unwrap_or("").trim().to_string();
            Some((k, v))
        })
        .collect();

    let found = props.iter_mut().find(|(k, _)| k == prop);
    if let Some(entry) = found {
        entry.1 = value.to_string();
    } else if !value.is_empty() {
        props.push((prop.to_string(), value.to_string()));
    }

    props.iter().map(|(k, v)| format!("{k}: {v}")).collect::<Vec<_>>().join("; ")
}
