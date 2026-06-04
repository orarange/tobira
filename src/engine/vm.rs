use std::{
    cell::RefCell,
    cmp::{Ordering, Reverse},
    collections::HashMap,
    rc::Rc,
    time::{SystemTime, UNIX_EPOCH},
};

use serde_json::Value as JsonValue;

use super::chunk::{Chunk, Constant, FunctionProto, Opcode};
use super::event_loop::{
    EventLoop, MicrotaskJob, RafEntry, TaskEntry, TaskSource, TickResult, TimerEntry,
};
use super::heap::{GcRef, Heap, RawGcRef};
use super::host::{
    ConsoleLevel, ConsoleMessage, DomMutation, DomRead, DomReadResult, Host, NodeId, NodeKind,
    NoopHost, SiblingDirection, WindowId,
};
use super::value::{
    AsyncContext, GeneratorState, HostDispatch, HostObjectClass, HostObjectSlot, JsObject,
    JsPropertyDescriptor, JsString, ObjectKind, PromiseReaction, PromiseState, PropertyKey,
    SymbolId, Value,
};

type ValueCell = Rc<RefCell<Value>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum BuiltinId {
    Assert,
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
    ObjectValues,
    ObjectEntries,
    ObjectAssign,
    ObjectGetPrototypeOf,
    ObjectSetPrototypeOf,
    ObjectFreeze,
    ObjectIsFrozen,
    ObjectProtoHasOwnProperty,
    ObjectProtoToString,
    ObjectProtoValueOf,
    ObjectProtoIsPrototypeOf,
    FunctionProtoCall,
    FunctionProtoApply,
    FunctionProtoBind,
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
    MathSin,
    MathCos,
    MathTan,
    MathAsin,
    MathAcos,
    MathAtan,
    MathAtan2,
    MathLog,
    MathLog2,
    MathLog10,
    MathExp,
    MathRandom,
    JsonStringify,
    JsonParse,
    ConsoleLog,
    ConsoleInfo,
    ConsoleWarn,
    ConsoleError,
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
    DomNodeRemoveChild,
    DomNodeReplaceChild,
    DomNodeCloneNode,
    DomNodeRemove,
    DomNodeSetAttribute,
    DomNodeGetAttribute,
    DomNodeRemoveAttribute,
    DomNodeHasAttribute,
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
    // classList (TokenList)
    DomClassListAdd,
    DomClassListRemove,
    DomClassListContains,
    DomClassListToggle,
    DomClassListReplace,
    DomClassListItem,
    DomClassListToString,
    // style (CSSStyleDeclaration)
    DomStyleGetProperty,
    DomStyleSetProperty,
    DomStyleRemoveProperty,
    // performance, idle, encoding
    PerformanceNow,
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
    DateProtoGetTime,
    DateProtoGetFullYear,
    DateProtoGetMonth,
    DateProtoGetDate,
    DateProtoGetDay,
    DateProtoGetHours,
    DateProtoGetMinutes,
    DateProtoGetSeconds,
    DateProtoGetMilliseconds,
    DateProtoToISOString,
    DateProtoToString,
    DateProtoValueOf,
    GeneratorProtoNext,
    GeneratorProtoReturn,
    GeneratorProtoIterator,
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

#[derive(Debug, Clone)]
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
            Self::InfiniteLoop => write!(f, "execution exceeded the per-call loop budget"),
            Self::StackOverflow => write!(f, "call stack exceeded the phase 3 limit"),
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
fn translate_regex_named_groups(source: &str) -> String {
    let mut out = String::with_capacity(source.len() + 8);
    let mut chars = source.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' {
            out.push(c);
            if let Some(next) = chars.next() {
                out.push(next);
            }
            continue;
        }
        if c == '(' && chars.peek() == Some(&'?') {
            let mut lookahead = chars.clone();
            lookahead.next(); // '?'
            if lookahead.peek() == Some(&'<') {
                lookahead.next(); // '<'
                let after = lookahead.peek().copied();
                if after != Some('=') && after != Some('!') {
                    out.push_str("(?P<");
                    chars.next(); // '?'
                    chars.next(); // '<'
                    continue;
                }
            }
        }
        out.push(c);
    }
    out
}

/// Expand a regex replacement template: `$$`→`$`, `$&`→whole match, `` $` ``→
/// prefix, `$'`→suffix, `$<name>`→named group, `$1`..`$99`→numbered group.
fn expand_replacement(template: &str, caps: &regex::Captures, full_text: &str) -> String {
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

/// Compile a JS regex source + flag string into a Rust `regex::Regex`. Flags
/// `g`/`y` are handled at the call site (global iteration / sticky), not here.
fn compile_js_regex(source: &str, flags: &str) -> Result<regex::Regex, VmError> {
    let translated = translate_regex_named_groups(source);
    let mut builder = regex::RegexBuilder::new(&translated);
    builder.case_insensitive(flags.contains('i'));
    builder.multi_line(flags.contains('m'));
    builder.dot_matches_new_line(flags.contains('s'));
    builder.swap_greed(false);
    builder
        .build()
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

pub struct Vm {
    stack: Vec<Value>,
    frames: Vec<CallFrame>,
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
    url_search_params_prototype: Option<GcRef<JsObject>>,
    error_prototype: Option<GcRef<JsObject>>,
    promise_prototype: Option<GcRef<JsObject>>,
    map_prototype: Option<GcRef<JsObject>>,
    set_prototype: Option<GcRef<JsObject>>,
    event_loop: EventLoop,
    random_state: u64,
    host: Box<dyn Host>,
    /// Event listeners stored by (node_handle, event_type) → list of JS function GcRefs.
    /// Lives in the VM (not the Host) so GcRefs remain valid.
    event_listeners: HashMap<u32, HashMap<String, Vec<GcRef<JsObject>>>>,
    /// Cache for stateless builtin method values (constructable=false, prototype=None).
    /// Avoids a heap allocation on every DOM property access like element.appendChild.
    builtin_method_cache: HashMap<u32, Value>,
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
}

/// How a generator step ended.
enum GeneratorOutcome {
    Yielded(Value),
    Returned(Value),
}

/// Well-known symbol id for `Symbol.iterator`.
const SYMBOL_ITERATOR_ID: u32 = 1;
/// First id available to user-created `Symbol(...)` values.
const FIRST_USER_SYMBOL: u32 = 16;

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
            url_search_params_prototype: None,
            error_prototype: None,
            promise_prototype: None,
            map_prototype: None,
            set_prototype: None,
            event_loop: EventLoop::new(),
            random_state,
            host,
            event_listeners: HashMap::new(),
            builtin_method_cache: HashMap::new(),
            next_symbol_id: FIRST_USER_SYMBOL,
            symbol_descriptions: HashMap::new(),
            symbol_registry: HashMap::new(),
            generator_outcome: None,
        };
        vm.install_globals();
        vm
    }

    /// Borrow the host mutably (for reading results after execution).
    pub fn host_mut(&mut self) -> &mut dyn Host {
        self.host.as_mut()
    }

    /// Fire a DOM event on a node handle, invoking all registered JS listeners.
    /// `node_handle` is the raw u32 from HostObjectSlot.handle (0 = document/window).
    /// `event_type` is e.g. "DOMContentLoaded", "click", "load".
    pub fn fire_dom_event(&mut self, node_handle: u32, event_type: &str) -> Result<(), VmError> {
        // Snapshot listener list to avoid borrow issues during call
        let listeners: Vec<GcRef<JsObject>> = self
            .event_listeners
            .get(&node_handle)
            .and_then(|m| m.get(event_type))
            .cloned()
            .unwrap_or_default();

        if listeners.is_empty() {
            return Ok(());
        }

        // Build a minimal event object: { type, target: null, bubbles: false }
        let event_obj = self.allocate_ordinary_object(None);
        let type_val = self.make_string_value(event_type);
        self.define_data_property(event_obj, PropertyKey::from("type"), type_val, true, true, true);
        self.define_data_property(event_obj, PropertyKey::from("target"), Value::Null, true, true, true);
        self.define_data_property(event_obj, PropertyKey::from("bubbles"), Value::Bool(false), true, true, true);
        self.define_data_property(event_obj, PropertyKey::from("cancelable"), Value::Bool(false), true, true, true);
        let event_val = Value::Object(event_obj);

        for fn_ref in listeners {
            let _ = self.call_value_sync(Value::Object(fn_ref), Value::Undefined, vec![event_val.clone()]);
            self.drain_microtasks();
        }
        Ok(())
    }

    pub fn execute(&mut self, chunk: &Chunk) -> Result<Value, VmError> {
        self.stack.clear();
        self.frames.clear();
        self.fuel = 1_000_000;

        let closure = RuntimeClosure {
            proto: Rc::new(chunk.top_level.clone()),
            upvalues: Vec::new(),
        };
        self.push_call_frame(closure, Vec::new(), Value::Undefined, None)?;
        self.run_until_frame_depth(0)?;
        self.drain_microtasks();
        if self.stack.is_empty() {
            Ok(Value::Undefined)
        } else {
            self.pop_value()
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
                let value = {
                    let name = self.constant_name(index)?;
                    self.globals
                        .get(name)
                        .cloned()
                        .ok_or_else(|| VmError::ReferenceError(format!("{name} is not defined")))?
                };
                self.stack.push(value);
            }
            Opcode::SetGlobal(index) => {
                let name = self.constant_name(index)?.to_string();
                let value = self.pop_value()?;
                self.globals.insert(name, value);
            }
            Opcode::GetGlobalOptional(index) => {
                let value = {
                    let name = self.constant_name(index)?;
                    self.globals.get(name).cloned().unwrap_or(Value::Undefined)
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
                self.stack.push(Value::Number(-self.to_number(&value)));
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
                self.stack.push(Value::Number(self.to_number(&value)));
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
                    None => self.lookup_property_descriptor(object, &key).is_some(),
                };
                self.stack.push(Value::Bool(present));
            }
            Opcode::Instanceof => {
                let constructor = self.pop_value()?;
                let value = self.pop_value()?;
                self.stack
                    .push(Value::Bool(self.instanceof_value(&value, &constructor)?));
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
                if let Some(result) = self.invoke_callable_value(callee, this_value, args)? {
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
                let object = self.require_object_ref(&value, "for...in target")?;
                let keys = self.for_in_keys(object);
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
                    let name = self.constant_name(index)?;
                    PropertyKey::from(name)
                };
                let callee = self.get_property_value(&object, &key)?;
                self.stack.push(callee);
                self.stack.push(object);
            }
            Opcode::GetIndexForCall => {
                let key = self.pop_value()?;
                let object = self.pop_value()?;
                let callee = self.get_property_value(&object, &self.to_property_key(&key)?)?;
                self.stack.push(callee);
                self.stack.push(object);
            }
            Opcode::New(argc) => {
                let args = self.pop_args(argc)?;
                let constructor = self.pop_value()?;
                if let Some(result) = self.construct_value(constructor, args)? {
                    self.stack.push(result);
                }
            }
            Opcode::Throw => {
                let thrown = self.pop_value()?;
                return Err(VmError::Thrown(thrown));
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
        let url_search_params_prototype = self.allocate_ordinary_object(Some(object_prototype));
        let error_prototype = self.heap.allocate_object(JsObject {
            kind: ObjectKind::Error,
            prototype: Some(object_prototype),
            ..JsObject::default()
        });
        let promise_prototype = self.allocate_ordinary_object(Some(object_prototype));
        let map_prototype = self.allocate_ordinary_object(Some(object_prototype));
        let set_prototype = self.allocate_ordinary_object(Some(object_prototype));

        self.object_prototype = Some(object_prototype);
        self.function_prototype = Some(function_prototype);
        self.array_prototype = Some(array_prototype);
        self.string_prototype = Some(string_prototype);
        self.number_prototype = Some(number_prototype);
        self.boolean_prototype = Some(boolean_prototype);
        self.regexp_prototype = Some(regexp_prototype);
        self.date_prototype = Some(date_prototype);
        self.generator_prototype = Some(generator_prototype);
        self.url_search_params_prototype = Some(url_search_params_prototype);
        self.error_prototype = Some(error_prototype);
        self.promise_prototype = Some(promise_prototype);
        self.map_prototype = Some(map_prototype);
        self.set_prototype = Some(set_prototype);

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
        self.globals.insert("globalThis".to_string(), window_obj);
        self.globals.insert("self".to_string(), document_obj); // some libraries use self

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
        self.define_builtin_method(array_prototype, "includes", BuiltinId::ArrayProtoIncludes);
        self.define_builtin_method(array_prototype, "join", BuiltinId::ArrayProtoJoin);
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
        let usp_iterator = self.allocate_builtin_method(BuiltinId::UspEntries);
        self.define_data_property(
            url_search_params_prototype,
            PropertyKey::Symbol(SymbolId(SYMBOL_ITERATOR_ID)),
            usp_iterator,
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
                ("asyncIterator", 2),
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

        let proxy_ctor = self.allocate_builtin_value(BuiltinId::ProxyConstructor, true, None);
        self.globals.insert("Proxy".to_string(), proxy_ctor);

        let usp_ctor = self.allocate_builtin_value(
            BuiltinId::UrlSearchParamsConstructor,
            true,
            Some(url_search_params_prototype),
        );
        self.globals
            .insert("URLSearchParams".to_string(), usp_ctor);

        if let Some(date_ref) = self.value_object_ref(date_ctor) {
            self.define_builtin_method(date_ref, "now", BuiltinId::DateNow);
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
        self.define_builtin_method(math_object, "sin", BuiltinId::MathSin);
        self.define_builtin_method(math_object, "cos", BuiltinId::MathCos);
        self.define_builtin_method(math_object, "tan", BuiltinId::MathTan);
        self.define_builtin_method(math_object, "asin", BuiltinId::MathAsin);
        self.define_builtin_method(math_object, "acos", BuiltinId::MathAcos);
        self.define_builtin_method(math_object, "atan", BuiltinId::MathAtan);
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

    fn set_generator_state(&mut self, generator: GcRef<JsObject>, state: GeneratorState) {
        if let Some(object) = self.heap.objects_mut().get_mut(generator) {
            object.kind = ObjectKind::Generator(Box::new(state));
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
        regex: &regex::Regex,
        replacement: &Value,
        global: bool,
    ) -> Result<Value, VmError> {
        let is_fn = self.is_callable_value(replacement);
        let template = if is_fn {
            String::new()
        } else {
            self.to_string(replacement)
        };
        let caps_list: Vec<regex::Captures> = if global {
            regex.captures_iter(text).collect()
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
        regex: &regex::Regex,
        caps: &regex::Captures,
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
        let names: Vec<String> = regex
            .capture_names()
            .flatten()
            .map(|name| name.to_string())
            .collect();
        let groups = if names.is_empty() {
            Value::Undefined
        } else {
            let groups = self.allocate_ordinary_object(Some(self.object_prototype_ref()));
            for name in names {
                let value = caps
                    .name(&name)
                    .map(|m| self.make_string_value(m.as_str()))
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
        let key = builtin as u32;
        if let Some(v) = self.builtin_method_cache.get(&key) {
            return v.clone();
        }
        let v = self.allocate_builtin_value(builtin, false, None);
        self.builtin_method_cache.insert(key, v.clone());
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
                "execution exceeded the per-call loop budget".to_string(),
            ),
            VmError::StackOverflow => self.create_error_object(
                "RangeError",
                "call stack exceeded the phase 5 limit".to_string(),
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
        let outer_promise = frame.async_outer_promise.ok_or_else(|| {
            VmError::TypeError("await expressions are only valid in async frames".to_string())
        })?;
        let stack_snapshot = self.stack.split_off(frame.stack_base);
        let context = AsyncContext {
            frame: Box::new(frame),
            stack_snapshot,
            outer_promise,
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
        let outer_promise = frame.async_outer_promise.ok_or_else(|| {
            VmError::TypeError("async return without an outer promise".to_string())
        })?;
        self.stack.truncate(frame.stack_base);
        self.resolve_promise_from_resolution(outer_promise, result)
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
        if self.frames.len() >= 1024 {
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
                if closure.proto.is_generator {
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
            VmError::TypeError(_) | VmError::ReferenceError(_) | VmError::RangeError(_) => {
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
        loop {
            let Some(frame_index) = self.frames.len().checked_sub(1) else {
                return Err(VmError::TypeError(format!(
                    "uncaught throw: {}",
                    self.to_string(&thrown)
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
                return Err(VmError::TypeError(
                    "attempted to call a non-function value".to_string(),
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
                } else {
                    "[object Object]".to_string()
                }
            }
            Value::Symbol(_) => "Symbol()".to_string(),
        }
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
        if number.fract() == 0.0 {
            return format!("{number:.0}");
        }
        number.to_string()
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
        self.stack.push(Value::Number(operator(
            self.to_number(&lhs),
            self.to_number(&rhs),
        )));
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
            Value::Null | Value::Undefined => Err(VmError::TypeError(
                "cannot read properties of null or undefined".to_string(),
            )),
            _ => Ok(Value::Undefined),
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
            return self.get_host_property(slot, key);
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
            return self.set_host_property(slot, key, value);
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
            Value::Object(object) => Ok(self.array_length(*object)),
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

    fn instanceof_value(&self, value: &Value, constructor: &Value) -> Result<bool, VmError> {
        let Value::Object(object) = value else {
            return Ok(false);
        };
        let ctor = self.require_object_ref(constructor, "instanceof right-hand side")?;
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
            BuiltinId::PromiseProtoThen => {
                let promise = self.require_promise_this(&this_value, "Promise.prototype.then")?;
                let on_fulfilled = self.normalize_handler_value(args.first());
                let on_rejected = self.normalize_handler_value(args.get(1));
                Ok(Value::Object(self.promise_then_internal(
                    promise,
                    on_fulfilled,
                    on_rejected,
                )?))
            }
            BuiltinId::PromiseProtoCatch => {
                let promise = self.require_promise_this(&this_value, "Promise.prototype.catch")?;
                let on_rejected = self.normalize_handler_value(args.first());
                Ok(Value::Object(self.promise_then_internal(
                    promise,
                    None,
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
            BuiltinId::SetTimeout => {
                let callback = self.require_callable_object(
                    args.first().unwrap_or(&Value::Undefined),
                    "setTimeout",
                )?;
                let delay_ms = self
                    .to_number(args.get(1).unwrap_or(&Value::Number(0.0)))
                    .max(0.0) as i64;
                let id = self.schedule_timer(
                    callback,
                    delay_ms,
                    None,
                    args.into_iter().skip(2).collect(),
                );
                Ok(Value::Number(id as f64))
            }
            BuiltinId::ClearTimeout | BuiltinId::ClearInterval => {
                if let Some(id_value) = args.first() {
                    let id = self.to_uint32(id_value);
                    self.event_loop.cancelled_timers.insert(id);
                }
                Ok(Value::Undefined)
            }
            BuiltinId::SetInterval => {
                let callback = self.require_callable_object(
                    args.first().unwrap_or(&Value::Undefined),
                    "setInterval",
                )?;
                let delay_ms = self
                    .to_number(args.get(1).unwrap_or(&Value::Number(0.0)))
                    .max(0.0) as u64;
                let id = self.schedule_timer(
                    callback,
                    delay_ms as i64,
                    Some(delay_ms),
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
                let object = self.require_object_ref(
                    args.first().unwrap_or(&Value::Undefined),
                    "Object.getOwnPropertyDescriptor",
                )?;
                let key = self.to_property_key(args.get(1).unwrap_or(&Value::Undefined))?;
                match self.get_own_property_descriptor(object, &key) {
                    Some(descriptor) => self.property_descriptor_to_value(descriptor),
                    None => Ok(Value::Undefined),
                }
            }
            BuiltinId::ObjectKeys => {
                let object = self
                    .require_object_ref(args.first().unwrap_or(&Value::Undefined), "Object.keys")?;
                let values = self
                    .object_own_enumerable_keys(object)
                    .into_iter()
                    .map(|key| self.make_string_value(&self.property_key_to_string(&key)))
                    .collect();
                self.make_array_from_values(values)
            }
            BuiltinId::ObjectValues => {
                let object = self.require_object_ref(
                    args.first().unwrap_or(&Value::Undefined),
                    "Object.values",
                )?;
                let mut values = Vec::new();
                for key in self.object_own_enumerable_keys(object) {
                    values.push(self.get_property_value(&Value::Object(object), &key)?);
                }
                self.make_array_from_values(values)
            }
            BuiltinId::ObjectEntries => {
                let object = self.require_object_ref(
                    args.first().unwrap_or(&Value::Undefined),
                    "Object.entries",
                )?;
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
                let object = self.require_object_ref(
                    args.first().unwrap_or(&Value::Undefined),
                    "Object.getPrototypeOf",
                )?;
                let prototype = self
                    .heap
                    .objects()
                    .get(object)
                    .and_then(|object_data| object_data.prototype)
                    .map(Value::Object)
                    .unwrap_or(Value::Null);
                Ok(prototype)
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
                let object = self.require_object_ref(
                    args.first().unwrap_or(&Value::Undefined),
                    "Object.freeze",
                )?;
                self.freeze_object(object);
                Ok(Value::Object(object))
            }
            BuiltinId::ObjectIsFrozen => {
                let object = self.require_object_ref(
                    args.first().unwrap_or(&Value::Undefined),
                    "Object.isFrozen",
                )?;
                Ok(Value::Bool(self.is_frozen(object)))
            }
            BuiltinId::ObjectProtoHasOwnProperty => {
                let object = self.builtin_object_this(&this_value, "hasOwnProperty")?;
                let key = self.to_property_key(args.first().unwrap_or(&Value::Undefined))?;
                Ok(Value::Bool(
                    self.get_own_property_descriptor(object, &key).is_some(),
                ))
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
            BuiltinId::ErrorConstructor => Ok(self.create_error_object(
                "Error",
                args.first()
                    .map(|value| self.to_string(value))
                    .unwrap_or_default(),
            )),
            BuiltinId::TypeErrorConstructor => Ok(self.create_error_object(
                "TypeError",
                args.first()
                    .map(|value| self.to_string(value))
                    .unwrap_or_default(),
            )),
            BuiltinId::RangeErrorConstructor => Ok(self.create_error_object(
                "RangeError",
                args.first()
                    .map(|value| self.to_string(value))
                    .unwrap_or_default(),
            )),
            BuiltinId::ReferenceErrorConstructor => Ok(self.create_error_object(
                "ReferenceError",
                args.first()
                    .map(|value| self.to_string(value))
                    .unwrap_or_default(),
            )),
            BuiltinId::SyntaxErrorConstructor => Ok(self.create_error_object(
                "SyntaxError",
                args.first()
                    .map(|value| self.to_string(value))
                    .unwrap_or_default(),
            )),
            BuiltinId::UriErrorConstructor => Ok(self.create_error_object(
                "URIError",
                args.first()
                    .map(|value| self.to_string(value))
                    .unwrap_or_default(),
            )),
            BuiltinId::EvalErrorConstructor => Ok(self.create_error_object(
                "EvalError",
                args.first()
                    .map(|value| self.to_string(value))
                    .unwrap_or_default(),
            )),
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
                let object = self.require_object_ref(
                    args.first().unwrap_or(&Value::Undefined),
                    "Object.getOwnPropertyNames",
                )?;
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
                if let Value::Object(object) = &target {
                    if let Some(object_data) = self.heap.objects_mut().get_mut(*object) {
                        object_data.extensible = false;
                    }
                }
                Ok(target)
            }
            BuiltinId::ObjectIsExtensible => {
                let extensible = matches!(args.first(), Some(Value::Object(object))
                    if self.heap.objects().get(*object).map(|o| o.extensible).unwrap_or(false));
                Ok(Value::Bool(extensible))
            }
            BuiltinId::ObjectSeal => {
                let target = args.first().cloned().unwrap_or(Value::Undefined);
                if let Value::Object(object) = &target {
                    if let Some(object_data) = self.heap.objects_mut().get_mut(*object) {
                        object_data.extensible = false;
                    }
                }
                Ok(target)
            }
            BuiltinId::ObjectIsSealed => {
                let sealed = match args.first() {
                    Some(Value::Object(object)) => self
                        .heap
                        .objects()
                        .get(*object)
                        .map(|o| !o.extensible)
                        .unwrap_or(true),
                    _ => true,
                };
                Ok(Value::Bool(sealed))
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
                        self.build_match_result(&regex, &caps, &text)
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
                        Some(caps) => self.build_match_result(&regex, &caps, &text),
                        None => Ok(Value::Null),
                    }
                }
            }
            BuiltinId::StringProtoMatchAll => {
                let text = self.builtin_string_this(&this_value)?;
                let (source, flags) = self.coerce_regex_arg(args.first())?;
                let regex = compile_js_regex(&source, &flags)?;
                let captures: Vec<regex::Captures> = regex.captures_iter(&text).collect();
                let mut results = Vec::with_capacity(captures.len());
                for caps in &captures {
                    results.push(self.build_match_result(&regex, caps, &text)?);
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
            BuiltinId::StringProtoIndexOf => {
                let text = self.builtin_string_this(&this_value)?;
                let needle = args
                    .first()
                    .map(|value| self.to_string(value))
                    .unwrap_or_default();
                let from = args
                    .get(1)
                    .map(|value| self.to_number(value) as usize)
                    .unwrap_or(0);
                let index = text[from.min(text.len())..]
                    .find(&needle)
                    .map(|value| value + from)
                    .map(|value| value as f64)
                    .unwrap_or(-1.0);
                Ok(Value::Number(index))
            }
            BuiltinId::StringProtoLastIndexOf => {
                let text = self.builtin_string_this(&this_value)?;
                let needle = args
                    .first()
                    .map(|value| self.to_string(value))
                    .unwrap_or_default();
                let index = text
                    .rfind(&needle)
                    .map(|value| value as f64)
                    .unwrap_or(-1.0);
                Ok(Value::Number(index))
            }
            BuiltinId::StringProtoIncludes => {
                let text = self.builtin_string_this(&this_value)?;
                let needle = args
                    .first()
                    .map(|value| self.to_string(value))
                    .unwrap_or_default();
                Ok(Value::Bool(text.contains(&needle)))
            }
            BuiltinId::StringProtoStartsWith => {
                let text = self.builtin_string_this(&this_value)?;
                let needle = args
                    .first()
                    .map(|value| self.to_string(value))
                    .unwrap_or_default();
                Ok(Value::Bool(text.starts_with(&needle)))
            }
            BuiltinId::StringProtoEndsWith => {
                let text = self.builtin_string_this(&this_value)?;
                let needle = args
                    .first()
                    .map(|value| self.to_string(value))
                    .unwrap_or_default();
                Ok(Value::Bool(text.ends_with(&needle)))
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
                        let segments: Vec<String> =
                            regex.split(&text).map(|s| s.to_string()).collect();
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
            BuiltinId::MathSin => Ok(Value::Number(self.number_arg(&args, 0).sin())),
            BuiltinId::MathCos => Ok(Value::Number(self.number_arg(&args, 0).cos())),
            BuiltinId::MathTan => Ok(Value::Number(self.number_arg(&args, 0).tan())),
            BuiltinId::MathAsin => Ok(Value::Number(self.number_arg(&args, 0).asin())),
            BuiltinId::MathAcos => Ok(Value::Number(self.number_arg(&args, 0).acos())),
            BuiltinId::MathAtan => Ok(Value::Number(self.number_arg(&args, 0).atan())),
            BuiltinId::MathAtan2 => Ok(Value::Number(
                self.number_arg(&args, 0).atan2(self.number_arg(&args, 1)),
            )),
            BuiltinId::MathLog => Ok(Value::Number(self.number_arg(&args, 0).ln())),
            BuiltinId::MathLog2 => Ok(Value::Number(self.number_arg(&args, 0).log2())),
            BuiltinId::MathLog10 => Ok(Value::Number(self.number_arg(&args, 0).log10())),
            BuiltinId::MathExp => Ok(Value::Number(self.number_arg(&args, 0).exp())),
            BuiltinId::MathRandom => Ok(Value::Number(self.next_random())),
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
            BuiltinId::DomDocQuerySelector => {
                let sel = args.first().map(|v| self.to_string(v)).unwrap_or_default();
                let root_res = self.host.read_dom(DomRead::DocumentRoot { window: WindowId(0) });
                let root = match root_res { Ok(DomReadResult::Node(id)) => id, _ => return Ok(Value::Null) };
                let res = self.host.read_dom(DomRead::QuerySelector { root, selectors: sel });
                Ok(match res { Ok(DomReadResult::Node(id)) => self.make_dom_node_value(id), _ => Value::Null })
            }
            BuiltinId::DomDocQuerySelectorAll => {
                let sel = args.first().map(|v| self.to_string(v)).unwrap_or_default();
                let root_res = self.host.read_dom(DomRead::DocumentRoot { window: WindowId(0) });
                let root = match root_res { Ok(DomReadResult::Node(id)) => id, _ => return self.make_array_from_values(vec![]) };
                let res = self.host.read_dom(DomRead::QuerySelectorAll { root, selectors: sel });
                match res {
                    Ok(DomReadResult::Nodes(ids)) => {
                        let items: Vec<Value> = ids.iter().map(|&id| self.make_dom_node_value(id)).collect();
                        self.make_array_from_values(items)
                    }
                    _ => self.make_array_from_values(vec![]),
                }
            }
            BuiltinId::DomDocGetElementById => {
                let id_str = args.first().map(|v| self.to_string(v)).unwrap_or_default();
                let sel = format!("#{id_str}");
                let root_res = self.host.read_dom(DomRead::DocumentRoot { window: WindowId(0) });
                let root = match root_res { Ok(DomReadResult::Node(id)) => id, _ => return Ok(Value::Null) };
                let res = self.host.read_dom(DomRead::QuerySelector { root, selectors: sel });
                Ok(match res { Ok(DomReadResult::Node(id)) => self.make_dom_node_value(id), _ => Value::Null })
            }
            BuiltinId::DomDocGetElementsByClassName => {
                let cls = args.first().map(|v| self.to_string(v)).unwrap_or_default();
                let sel = cls.split_whitespace().map(|c| format!(".{c}")).collect::<Vec<_>>().join("");
                let root_res = self.host.read_dom(DomRead::DocumentRoot { window: WindowId(0) });
                let root = match root_res { Ok(DomReadResult::Node(id)) => id, _ => return self.make_array_from_values(vec![]) };
                let res = self.host.read_dom(DomRead::QuerySelectorAll { root, selectors: sel });
                match res {
                    Ok(DomReadResult::Nodes(ids)) => {
                        let items: Vec<Value> = ids.iter().map(|&id| self.make_dom_node_value(id)).collect();
                        self.make_array_from_values(items)
                    }
                    _ => self.make_array_from_values(vec![]),
                }
            }
            BuiltinId::DomDocGetElementsByTagName => {
                let tag = args.first().map(|v| self.to_string(v)).unwrap_or_default();
                let root_res = self.host.read_dom(DomRead::DocumentRoot { window: WindowId(0) });
                let root = match root_res { Ok(DomReadResult::Node(id)) => id, _ => return self.make_array_from_values(vec![]) };
                let res = self.host.read_dom(DomRead::QuerySelectorAll { root, selectors: tag });
                match res {
                    Ok(DomReadResult::Nodes(ids)) => {
                        let items: Vec<Value> = ids.iter().map(|&id| self.make_dom_node_value(id)).collect();
                        self.make_array_from_values(items)
                    }
                    _ => self.make_array_from_values(vec![]),
                }
            }
            BuiltinId::DomDocCreateElement => {
                let tag = args.first().map(|v| self.to_string(v)).unwrap_or_default();
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
                let _ = self.host.mutate_dom(DomMutation::WriteHtml { window: WindowId(0), html });
                Ok(Value::Undefined)
            }
            // ----------------------------------------------------------------
            // DOM — node/element methods (this = Node host object)
            // ----------------------------------------------------------------
            BuiltinId::DomNodeQuerySelector => {
                let sel = args.first().map(|v| self.to_string(v)).unwrap_or_default();
                let node_id = self.node_id_from_host_val(&this_value).unwrap_or(NodeId(0));
                let res = self.host.read_dom(DomRead::QuerySelector { root: node_id, selectors: sel });
                Ok(match res { Ok(DomReadResult::Node(id)) => self.make_dom_node_value(id), _ => Value::Null })
            }
            BuiltinId::DomNodeQuerySelectorAll => {
                let sel = args.first().map(|v| self.to_string(v)).unwrap_or_default();
                let node_id = self.node_id_from_host_val(&this_value).unwrap_or(NodeId(0));
                let res = self.host.read_dom(DomRead::QuerySelectorAll { root: node_id, selectors: sel });
                match res {
                    Ok(DomReadResult::Nodes(ids)) => {
                        let items: Vec<Value> = ids.iter().map(|&id| self.make_dom_node_value(id)).collect();
                        self.make_array_from_values(items)
                    }
                    _ => self.make_array_from_values(vec![]),
                }
            }
            BuiltinId::DomNodeAppendChild => {
                let parent_id = self.node_id_from_host_val(&this_value).unwrap_or(NodeId(0));
                let child_ids: Vec<NodeId> = args.iter().filter_map(|v| self.node_id_from_host_val(v)).collect();
                let _ = self.host.mutate_dom(DomMutation::Append { parent: parent_id, children: child_ids });
                Ok(args.first().cloned().unwrap_or(Value::Undefined))
            }
            BuiltinId::DomNodeInsertBefore => {
                let parent_id = self.node_id_from_host_val(&this_value).unwrap_or(NodeId(0));
                let child_id = self.node_id_from_host_val(args.first().unwrap_or(&Value::Undefined)).unwrap_or(NodeId(0));
                let ref_id = args.get(1).and_then(|v| self.node_id_from_host_val(v));
                let _ = self.host.mutate_dom(DomMutation::InsertBefore { parent: parent_id, child: child_id, reference: ref_id });
                Ok(args.first().cloned().unwrap_or(Value::Undefined))
            }
            BuiltinId::DomNodeRemoveChild => {
                let parent_id = self.node_id_from_host_val(&this_value).unwrap_or(NodeId(0));
                let child_id = self.node_id_from_host_val(args.first().unwrap_or(&Value::Undefined)).unwrap_or(NodeId(0));
                let _ = self.host.mutate_dom(DomMutation::ReplaceChild { parent: parent_id, new_child: child_id, old_child: child_id });
                Ok(args.first().cloned().unwrap_or(Value::Undefined))
            }
            BuiltinId::DomNodeReplaceChild => {
                let parent_id = self.node_id_from_host_val(&this_value).unwrap_or(NodeId(0));
                let new_id = self.node_id_from_host_val(args.first().unwrap_or(&Value::Undefined)).unwrap_or(NodeId(0));
                let old_id = self.node_id_from_host_val(args.get(1).unwrap_or(&Value::Undefined)).unwrap_or(NodeId(0));
                let _ = self.host.mutate_dom(DomMutation::ReplaceChild { parent: parent_id, new_child: new_id, old_child: old_id });
                Ok(args.first().cloned().unwrap_or(Value::Undefined))
            }
            BuiltinId::DomNodeCloneNode => {
                let node_id = self.node_id_from_host_val(&this_value).unwrap_or(NodeId(0));
                let deep = args.first().map(|v| self.is_truthy(v)).unwrap_or(false);
                let res = self.host.mutate_dom(DomMutation::CloneNode { node: node_id, deep });
                Ok(match res { Ok(super::host::DomMutationResult::Node(id)) => self.make_dom_node_value(id), _ => Value::Undefined })
            }
            BuiltinId::DomNodeRemove => {
                let node_id = self.node_id_from_host_val(&this_value).unwrap_or(NodeId(0));
                let _ = self.host.mutate_dom(DomMutation::Remove { node: node_id });
                Ok(Value::Undefined)
            }
            BuiltinId::DomNodeSetAttribute => {
                let node_id = self.node_id_from_host_val(&this_value).unwrap_or(NodeId(0));
                let name = args.first().map(|v| self.to_string(v)).unwrap_or_default();
                let value = args.get(1).map(|v| self.to_string(v)).unwrap_or_default();
                let _ = self.host.mutate_dom(DomMutation::SetAttribute { node: node_id, name, value });
                Ok(Value::Undefined)
            }
            BuiltinId::DomNodeGetAttribute => {
                let node_id = self.node_id_from_host_val(&this_value).unwrap_or(NodeId(0));
                let name = args.first().map(|v| self.to_string(v)).unwrap_or_default();
                let res = self.host.read_dom(DomRead::Attribute { node: node_id, name });
                Ok(match res { Ok(DomReadResult::String(s)) => self.make_string_value(&s), _ => Value::Null })
            }
            BuiltinId::DomNodeRemoveAttribute => {
                let node_id = self.node_id_from_host_val(&this_value).unwrap_or(NodeId(0));
                let name = args.first().map(|v| self.to_string(v)).unwrap_or_default();
                let _ = self.host.mutate_dom(DomMutation::RemoveAttribute { node: node_id, name });
                Ok(Value::Undefined)
            }
            BuiltinId::DomNodeHasAttribute => {
                let node_id = self.node_id_from_host_val(&this_value).unwrap_or(NodeId(0));
                let name = args.first().map(|v| self.to_string(v)).unwrap_or_default();
                let res = self.host.read_dom(DomRead::Attribute { node: node_id, name });
                Ok(Value::Bool(matches!(res, Ok(DomReadResult::String(_)))))
            }
            BuiltinId::DomNodeToggleAttribute => {
                let node_id = self.node_id_from_host_val(&this_value).unwrap_or(NodeId(0));
                let name = args.first().map(|v| self.to_string(v)).unwrap_or_default();
                let force = args.get(1).map(|v| self.is_truthy(v));
                let res = self.host.mutate_dom(DomMutation::ToggleAttribute { node: node_id, name, force });
                Ok(match res { Ok(super::host::DomMutationResult::Bool(b)) => Value::Bool(b), _ => Value::Bool(false) })
            }
            BuiltinId::DomNodeGetAttributeNames => {
                let node_id = self.node_id_from_host_val(&this_value).unwrap_or(NodeId(0));
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
                let node_id = self.node_id_from_host_val(&this_value).unwrap_or(NodeId(0));
                let sel = args.first().map(|v| self.to_string(v)).unwrap_or_default();
                let res = self.host.read_dom(DomRead::Closest { node: node_id, selectors: sel });
                Ok(match res { Ok(DomReadResult::Node(id)) => self.make_dom_node_value(id), _ => Value::Null })
            }
            BuiltinId::DomNodeMatches => {
                let node_id = self.node_id_from_host_val(&this_value).unwrap_or(NodeId(0));
                let sel = args.first().map(|v| self.to_string(v)).unwrap_or_default();
                let res = self.host.read_dom(DomRead::Matches { node: node_id, selectors: sel });
                Ok(match res { Ok(DomReadResult::Bool(b)) => Value::Bool(b), _ => Value::Bool(false) })
            }
            BuiltinId::DomNodeContains => {
                let ancestor_id = self.node_id_from_host_val(&this_value).unwrap_or(NodeId(0));
                let descendant_id = self.node_id_from_host_val(args.first().unwrap_or(&Value::Undefined)).unwrap_or(NodeId(0));
                let res = self.host.read_dom(DomRead::Contains { ancestor: ancestor_id, descendant: descendant_id });
                Ok(match res { Ok(DomReadResult::Bool(b)) => Value::Bool(b), _ => Value::Bool(false) })
            }
            BuiltinId::DomNodeGetBoundingClientRect => {
                let rect_obj = self.allocate_ordinary_object(None);
                self.define_data_property(rect_obj, PropertyKey::from("x"), Value::Number(0.0), true, true, true);
                self.define_data_property(rect_obj, PropertyKey::from("y"), Value::Number(0.0), true, true, true);
                self.define_data_property(rect_obj, PropertyKey::from("width"), Value::Number(0.0), true, true, true);
                self.define_data_property(rect_obj, PropertyKey::from("height"), Value::Number(0.0), true, true, true);
                self.define_data_property(rect_obj, PropertyKey::from("top"), Value::Number(0.0), true, true, true);
                self.define_data_property(rect_obj, PropertyKey::from("right"), Value::Number(0.0), true, true, true);
                self.define_data_property(rect_obj, PropertyKey::from("bottom"), Value::Number(0.0), true, true, true);
                self.define_data_property(rect_obj, PropertyKey::from("left"), Value::Number(0.0), true, true, true);
                Ok(Value::Object(rect_obj))
            }
            BuiltinId::DomNodeScrollIntoView | BuiltinId::DomNodeFocus | BuiltinId::DomNodeBlur | BuiltinId::DomNodeClick => {
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
                // Stub: silently succeed (GcRef equality would be needed for correct removal)
                Ok(Value::Undefined)
            }
            BuiltinId::DomNodeDispatchEvent => {
                Ok(Value::Undefined)
            }
            // ----------------------------------------------------------------
            // classList (TokenList) — this = TokenList host object with handle = element NodeId
            // ----------------------------------------------------------------
            BuiltinId::DomClassListAdd => {
                let node_id = self.node_id_from_host_val(&this_value).unwrap_or(NodeId(0));
                for arg in args {
                    let class_to_add = self.to_string(&arg);
                    let existing = match self.host.read_dom(DomRead::Attribute { node: node_id, name: "class".to_string() }) {
                        Ok(DomReadResult::String(s)) => s,
                        _ => String::new(),
                    };
                    let mut classes: Vec<String> = existing.split_whitespace().map(|s| s.to_string()).collect();
                    if !classes.iter().any(|c| c == &class_to_add) {
                        classes.push(class_to_add);
                        let _ = self.host.mutate_dom(DomMutation::SetAttribute { node: node_id, name: "class".to_string(), value: classes.join(" ") });
                    }
                }
                Ok(Value::Undefined)
            }
            BuiltinId::DomClassListRemove => {
                let node_id = self.node_id_from_host_val(&this_value).unwrap_or(NodeId(0));
                let names_to_remove: Vec<String> = args.iter().map(|v| self.to_string(v)).collect();
                let existing = match self.host.read_dom(DomRead::Attribute { node: node_id, name: "class".to_string() }) {
                    Ok(DomReadResult::String(s)) => s,
                    _ => String::new(),
                };
                let filtered: Vec<String> = existing.split_whitespace()
                    .filter(|c| !names_to_remove.iter().any(|r| r == c))
                    .map(|c| c.to_string())
                    .collect();
                let _ = self.host.mutate_dom(DomMutation::SetAttribute { node: node_id, name: "class".to_string(), value: filtered.join(" ") });
                Ok(Value::Undefined)
            }
            BuiltinId::DomClassListContains => {
                let node_id = self.node_id_from_host_val(&this_value).unwrap_or(NodeId(0));
                let class_name = args.first().map(|v| self.to_string(v)).unwrap_or_default();
                let existing = match self.host.read_dom(DomRead::Attribute { node: node_id, name: "class".to_string() }) {
                    Ok(DomReadResult::String(s)) => s,
                    _ => String::new(),
                };
                Ok(Value::Bool(existing.split_whitespace().any(|c| c == class_name)))
            }
            BuiltinId::DomClassListToggle => {
                let node_id = self.node_id_from_host_val(&this_value).unwrap_or(NodeId(0));
                let class_name = args.first().map(|v| self.to_string(v)).unwrap_or_default();
                let force = args.get(1).map(|v| self.is_truthy(v));
                let existing = match self.host.read_dom(DomRead::Attribute { node: node_id, name: "class".to_string() }) {
                    Ok(DomReadResult::String(s)) => s,
                    _ => String::new(),
                };
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
                let node_id = self.node_id_from_host_val(&this_value).unwrap_or(NodeId(0));
                let old_cls = args.first().map(|v| self.to_string(v)).unwrap_or_default();
                let new_cls = args.get(1).map(|v| self.to_string(v)).unwrap_or_default();
                let existing = match self.host.read_dom(DomRead::Attribute { node: node_id, name: "class".to_string() }) {
                    Ok(DomReadResult::String(s)) => s,
                    _ => String::new(),
                };
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
                let node_id = self.node_id_from_host_val(&this_value).unwrap_or(NodeId(0));
                let index = args.first().map(|v| self.to_number(v) as usize).unwrap_or(0);
                let existing = match self.host.read_dom(DomRead::Attribute { node: node_id, name: "class".to_string() }) {
                    Ok(DomReadResult::String(s)) => s,
                    _ => String::new(),
                };
                let item = existing.split_whitespace().nth(index).map(|s| self.make_string_value(s));
                Ok(item.unwrap_or(Value::Null))
            }
            BuiltinId::DomClassListToString => {
                let node_id = self.node_id_from_host_val(&this_value).unwrap_or(NodeId(0));
                let existing = match self.host.read_dom(DomRead::Attribute { node: node_id, name: "class".to_string() }) {
                    Ok(DomReadResult::String(s)) => s,
                    _ => String::new(),
                };
                Ok(self.make_string_value(&existing))
            }
            // style
            BuiltinId::DomStyleGetProperty => {
                Ok(self.make_string_value(""))
            }
            BuiltinId::DomStyleSetProperty => {
                let node_id = self.node_id_from_host_val(&this_value).unwrap_or(NodeId(0));
                let prop = args.first().map(|v| self.to_string(v)).unwrap_or_default();
                let val = args.get(1).map(|v| self.to_string(v)).unwrap_or_default();
                let existing = match self.host.read_dom(DomRead::Attribute { node: node_id, name: "style".to_string() }) {
                    Ok(DomReadResult::String(s)) => s,
                    _ => String::new(),
                };
                let updated = set_inline_style_prop(&existing, &prop, &val);
                let _ = self.host.mutate_dom(DomMutation::SetAttribute { node: node_id, name: "style".to_string(), value: updated });
                Ok(Value::Undefined)
            }
            BuiltinId::DomStyleRemoveProperty => {
                Ok(self.make_string_value(""))
            }
            // performance.now()
            BuiltinId::PerformanceNow => {
                let ms = self.host.now().monotonic_ms as f64;
                Ok(Value::Number(ms))
            }
            // requestIdleCallback — run callback synchronously
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
            // Storage item ops (this = Storage host object with kind encoded in handle)
            BuiltinId::StorageGetItem => {
                use super::host::{StorageAreaKind, StorageAreaScope, StorageOp, StorageResult};
                let key = args.first().map(|v| self.to_string(v)).unwrap_or_default();
                let kind = self.storage_kind_from_host_val(&this_value);
                let res = self.host.storage(StorageOp::Get {
                    kind,
                    scope: StorageAreaScope::Window(WindowId(0)),
                    key,
                });
                Ok(match res { Ok(StorageResult::Value(Some(v))) => self.make_string_value(&v), _ => Value::Null })
            }
            BuiltinId::StorageSetItem => {
                use super::host::{StorageAreaScope, StorageOp};
                let key = args.first().map(|v| self.to_string(v)).unwrap_or_default();
                let val = args.get(1).map(|v| self.to_string(v)).unwrap_or_default();
                let kind = self.storage_kind_from_host_val(&this_value);
                let _ = self.host.storage(StorageOp::Set {
                    kind,
                    scope: StorageAreaScope::Window(WindowId(0)),
                    key,
                    value: val,
                });
                Ok(Value::Undefined)
            }
            BuiltinId::StorageRemoveItem => {
                use super::host::{StorageAreaScope, StorageOp};
                let key = args.first().map(|v| self.to_string(v)).unwrap_or_default();
                let kind = self.storage_kind_from_host_val(&this_value);
                let _ = self.host.storage(StorageOp::Remove {
                    kind,
                    scope: StorageAreaScope::Window(WindowId(0)),
                    key,
                });
                Ok(Value::Undefined)
            }
            BuiltinId::StorageClear => {
                use super::host::{StorageAreaScope, StorageOp};
                let kind = self.storage_kind_from_host_val(&this_value);
                let _ = self.host.storage(StorageOp::Clear {
                    kind,
                    scope: StorageAreaScope::Window(WindowId(0)),
                });
                Ok(Value::Undefined)
            }
            BuiltinId::StorageKey => {
                use super::host::{StorageAreaScope, StorageOp, StorageResult};
                let index = args.first().map(|v| self.to_number(v) as usize).unwrap_or(0);
                let kind = self.storage_kind_from_host_val(&this_value);
                let res = self.host.storage(StorageOp::Keys {
                    kind,
                    scope: StorageAreaScope::Window(WindowId(0)),
                });
                Ok(match res {
                    Ok(StorageResult::Keys(keys)) => keys.get(index).map(|k| self.make_string_value(k)).unwrap_or(Value::Null),
                    _ => Value::Null,
                })
            }
            // window
            BuiltinId::WindowScrollTo | BuiltinId::WindowScrollBy => {
                let y = args.get(1).map(|v| self.to_number(v)).unwrap_or(0.0);
                let _ = self.host.mutate_dom(DomMutation::SetWindowScroll { window: WindowId(0), x: 0.0, y });
                Ok(Value::Undefined)
            }
            BuiltinId::WindowGetComputedStyle => {
                Ok(Value::Object(self.allocate_ordinary_object(None)))
            }
            BuiltinId::WindowMatchMedia => {
                let result_obj = self.allocate_ordinary_object(None);
                self.define_data_property(result_obj, PropertyKey::from("matches"), Value::Bool(false), true, true, true);
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
            let len = values.len();
            for i in 0..len {
                for j in i + 1..len {
                    let result = self.call_value_sync(
                        compare_fn.clone(),
                        Value::Undefined,
                        vec![values[i].clone(), values[j].clone()],
                    )?;
                    if self.to_number(&result) > 0.0 {
                        values.swap(i, j);
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
        self.make_host_object(HostObjectSlot {
            class: HostObjectClass::Node,
            interface_name: "Element",
            handle: node_id.0 as u64,
            dispatch: HostDispatch::Node,
            supports_indexed_properties: false,
            supports_named_properties: false,
        })
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

    fn get_host_property(
        &mut self,
        slot: HostObjectSlot,
        key: &PropertyKey,
    ) -> Result<Value, VmError> {
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
            HostObjectClass::Other("CSSStyleDeclaration") => self.get_style_property(name),
            HostObjectClass::StorageArea => self.get_storage_property(slot, name),
            _ => Ok(Value::Undefined),
        }
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
            "history" => {
                let hist = self.allocate_ordinary_object(None);
                self.define_data_property(hist, PropertyKey::from("length"), Value::Number(1.0), true, true, true);
                Ok(Value::Object(hist))
            }
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
                let crypto = self.allocate_ordinary_object(None);
                Ok(Value::Object(crypto))
            }
            "isSecureContext" => Ok(Value::Bool(false)),
            "crossOriginIsolated" => Ok(Value::Bool(false)),
            _ => Ok(self.globals.get(&name).cloned().unwrap_or(Value::Undefined)),
        }
    }

    fn make_location_object(&mut self) -> Result<Value, VmError> {
        let loc = self.host.location(WindowId(0));
        let obj = self.allocate_ordinary_object(None);
        if let Ok(l) = loc {
            let href = self.make_string_value(&l.href);
            self.define_data_property(obj, PropertyKey::from("href"), href, true, true, true);
            let origin = self.make_string_value(&l.origin);
            self.define_data_property(obj, PropertyKey::from("origin"), origin, true, true, true);
            let proto = self.make_string_value(&l.protocol);
            self.define_data_property(obj, PropertyKey::from("protocol"), proto, true, true, true);
            let host_v = self.make_string_value(&l.host);
            self.define_data_property(obj, PropertyKey::from("host"), host_v, true, true, true);
            let hostname = self.make_string_value(&l.hostname);
            self.define_data_property(obj, PropertyKey::from("hostname"), hostname, true, true, true);
            let port = self.make_string_value(&l.port);
            self.define_data_property(obj, PropertyKey::from("port"), port, true, true, true);
            let pathname = self.make_string_value(&l.pathname);
            self.define_data_property(obj, PropertyKey::from("pathname"), pathname, true, true, true);
            let search = self.make_string_value(&l.search);
            self.define_data_property(obj, PropertyKey::from("search"), search, true, true, true);
            let hash = self.make_string_value(&l.hash);
            self.define_data_property(obj, PropertyKey::from("hash"), hash, true, true, true);
        }
        Ok(Value::Object(obj))
    }

    fn get_document_property(&mut self, name: String) -> Result<Value, VmError> {
        match name.as_str() {
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
                let res = self.host.read_dom(DomRead::NodeName { node: node_id });
                Ok(match res { Ok(DomReadResult::String(s)) => self.make_string_value(&s), _ => Value::Undefined })
            }
            "nodeValue" => {
                let res = self.host.read_dom(DomRead::NodeValue { node: node_id });
                Ok(match res { Ok(DomReadResult::String(s)) => self.make_string_value(&s), _ => Value::Null })
            }
            "textContent" => {
                let res = self.host.read_dom(DomRead::TextContent { node: node_id });
                Ok(match res { Ok(DomReadResult::String(s)) => self.make_string_value(&s), _ => Value::Null })
            }
            "innerHTML" => {
                let res = self.host.read_dom(DomRead::InnerHtml { node: node_id });
                Ok(match res { Ok(DomReadResult::String(s)) => self.make_string_value(&s), _ => self.make_string_value("") })
            }
            "id" => {
                let res = self.host.read_dom(DomRead::Attribute { node: node_id, name: "id".to_string() });
                Ok(match res { Ok(DomReadResult::String(s)) => self.make_string_value(&s), _ => self.make_string_value("") })
            }
            "className" => {
                let res = self.host.read_dom(DomRead::Attribute { node: node_id, name: "class".to_string() });
                Ok(match res { Ok(DomReadResult::String(s)) => self.make_string_value(&s), _ => self.make_string_value("") })
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
            "firstChild" => {
                let res = self.host.read_dom(DomRead::Children { node: node_id, elements_only: false });
                Ok(match res { Ok(DomReadResult::Nodes(ids)) => ids.first().map(|&id| self.make_dom_node_value(id)).unwrap_or(Value::Null), _ => Value::Null })
            }
            "lastChild" => {
                let res = self.host.read_dom(DomRead::Children { node: node_id, elements_only: false });
                Ok(match res { Ok(DomReadResult::Nodes(ids)) => ids.last().map(|&id| self.make_dom_node_value(id)).unwrap_or(Value::Null), _ => Value::Null })
            }
            "firstElementChild" => {
                let res = self.host.read_dom(DomRead::Children { node: node_id, elements_only: true });
                Ok(match res { Ok(DomReadResult::Nodes(ids)) => ids.first().map(|&id| self.make_dom_node_value(id)).unwrap_or(Value::Null), _ => Value::Null })
            }
            "lastElementChild" => {
                let res = self.host.read_dom(DomRead::Children { node: node_id, elements_only: true });
                Ok(match res { Ok(DomReadResult::Nodes(ids)) => ids.last().map(|&id| self.make_dom_node_value(id)).unwrap_or(Value::Null), _ => Value::Null })
            }
            "nextSibling" => {
                let res = self.host.read_dom(DomRead::Sibling { node: node_id, direction: SiblingDirection::Next, elements_only: false });
                Ok(match res { Ok(DomReadResult::Node(id)) => self.make_dom_node_value(id), _ => Value::Null })
            }
            "previousSibling" => {
                let res = self.host.read_dom(DomRead::Sibling { node: node_id, direction: SiblingDirection::Previous, elements_only: false });
                Ok(match res { Ok(DomReadResult::Node(id)) => self.make_dom_node_value(id), _ => Value::Null })
            }
            "nextElementSibling" => {
                let res = self.host.read_dom(DomRead::Sibling { node: node_id, direction: SiblingDirection::Next, elements_only: true });
                Ok(match res { Ok(DomReadResult::Node(id)) => self.make_dom_node_value(id), _ => Value::Null })
            }
            "previousElementSibling" => {
                let res = self.host.read_dom(DomRead::Sibling { node: node_id, direction: SiblingDirection::Previous, elements_only: true });
                Ok(match res { Ok(DomReadResult::Node(id)) => self.make_dom_node_value(id), _ => Value::Null })
            }
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
                let res = self.host.read_dom(DomRead::Children { node: node_id, elements_only: false });
                Ok(Value::Number(match res { Ok(DomReadResult::Nodes(ids)) => ids.len() as f64, _ => 0.0 }))
            }
            // Geometry / layout properties (all return 0 — layout not wired yet)
            "offsetWidth" | "offsetHeight" | "offsetLeft" | "offsetTop"
            | "clientWidth" | "clientHeight" | "clientLeft" | "clientTop"
            | "scrollWidth" | "scrollHeight" | "scrollLeft" | "scrollTop" => Ok(Value::Number(0.0)),
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
            "toggleAttribute" => Ok(self.allocate_builtin_method(BuiltinId::DomNodeToggleAttribute)),
            "getAttributeNames" => Ok(self.allocate_builtin_method(BuiltinId::DomNodeGetAttributeNames)),
            "appendChild" | "append" => Ok(self.allocate_builtin_method(BuiltinId::DomNodeAppendChild)),
            "prepend" => Ok(self.allocate_builtin_method(BuiltinId::DomNodeInsertBefore)),
            "insertBefore" => Ok(self.allocate_builtin_method(BuiltinId::DomNodeInsertBefore)),
            "removeChild" => Ok(self.allocate_builtin_method(BuiltinId::DomNodeRemoveChild)),
            "replaceChild" => Ok(self.allocate_builtin_method(BuiltinId::DomNodeReplaceChild)),
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
            "click" => Ok(self.allocate_builtin_method(BuiltinId::DomNodeClick)),
            "addEventListener" | "removeEventListener" => Ok(self.allocate_builtin_method(BuiltinId::DomNodeAddEventListener)),
            "dispatchEvent" => Ok(self.allocate_builtin_method(BuiltinId::DomNodeDispatchEvent)),
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

    fn get_style_property(&mut self, name: String) -> Result<Value, VmError> {
        match name.as_str() {
            "getPropertyValue" => Ok(self.allocate_builtin_method(BuiltinId::DomStyleGetProperty)),
            "setProperty" => Ok(self.allocate_builtin_method(BuiltinId::DomStyleSetProperty)),
            "removeProperty" => Ok(self.allocate_builtin_method(BuiltinId::DomStyleRemoveProperty)),
            _ => Ok(self.make_string_value("")),
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
                    "textContent" | "nodeValue" => {
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
        if let Ok(index) = value.parse::<u32>() {
            if index.to_string() == value {
                return PropertyKey::Index(index);
            }
        }
        PropertyKey::String(value)
    }
}

#[cfg(test)]
mod tests {
    use super::Vm;
    use crate::engine::{Compiler, Heap, Parser};

    fn run_script(source: &str) {
        let program = Parser::new(source).parse().expect("script should parse");
        let chunk = Compiler::new(&program)
            .compile()
            .expect("script should compile");
        let mut vm = Vm::new(Heap::new());
        vm.execute(&chunk).expect("script should execute");
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
