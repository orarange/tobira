use super::heap::GcRef;
use indexmap::IndexMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SymbolId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct HostObjectSlot {
    pub class: HostObjectClass,
    pub interface_name: &'static str,
    pub handle: u64,
    pub dispatch: HostDispatch,
    pub supports_indexed_properties: bool,
    pub supports_named_properties: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HostObjectClass {
    Window,
    Document,
    Node,
    EventTarget,
    Observer,
    StorageArea,
    Request,
    Response,
    Other(&'static str),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HostDispatch {
    Ordinary,
    Node,
    EventTarget,
    Collection,
    TokenList,
    Dataset,
    StyleDeclaration,
    CanvasContext2d,
    Other(&'static str),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Undefined,
    Null,
    Bool(bool),
    Number(f64),
    String(GcRef<JsString>),
    Object(GcRef<JsObject>),
    Symbol(SymbolId),
}

impl Value {
    pub fn type_name(&self) -> &'static str {
        match self {
            Self::Undefined => "undefined",
            Self::Null => "object",
            Self::Bool(_) => "boolean",
            Self::Number(_) => "number",
            Self::String(_) => "string",
            Self::Object(_) => "object",
            Self::Symbol(_) => "symbol",
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct JsString {
    pub text: String,
}

impl From<String> for JsString {
    fn from(text: String) -> Self {
        Self { text }
    }
}

impl From<&str> for JsString {
    fn from(text: &str) -> Self {
        Self {
            text: text.to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum PropertyKey {
    String(String),
    Symbol(SymbolId),
    Index(u32),
}

#[derive(Debug, Clone, PartialEq)]
pub enum JsPropertyDescriptor {
    Data {
        value: Value,
        writable: bool,
        enumerable: bool,
        configurable: bool,
    },
    Accessor {
        get: Option<GcRef<JsObject>>,
        set: Option<GcRef<JsObject>>,
        enumerable: bool,
        configurable: bool,
    },
}

impl JsPropertyDescriptor {
    pub fn data(value: Value) -> Self {
        Self::Data {
            value,
            writable: true,
            enumerable: true,
            configurable: true,
        }
    }

    pub fn data_with_flags(
        value: Value,
        writable: bool,
        enumerable: bool,
        configurable: bool,
    ) -> Self {
        Self::Data {
            value,
            writable,
            enumerable,
            configurable,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct PromiseReaction {
    pub handler: Option<GcRef<JsObject>>,
    pub result_promise: Option<GcRef<JsObject>>,
    pub is_reject_handler: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum PromiseState {
    Pending {
        fulfill_reactions: Vec<PromiseReaction>,
        reject_reactions: Vec<PromiseReaction>,
    },
    Fulfilled(Value),
    Rejected(Value),
}

#[derive(Debug, Clone)]
pub struct AsyncContext {
    pub frame: Box<crate::engine::vm::CallFrame>,
    pub stack_snapshot: Vec<Value>,
    pub outer_promise: Option<GcRef<JsObject>>,
    pub async_generator_request: Option<GcRef<JsObject>>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AsyncGeneratorRequest {
    pub sent: Value,
    pub promise: GcRef<JsObject>,
    pub is_return: bool,
}

/// Execution state of a generator object.
#[derive(Debug, Clone, PartialEq)]
pub enum GeneratorState {
    /// Paused, either at the start (ip 0, `started` false) or at a `yield`.
    Suspended {
        frame: Box<crate::engine::vm::CallFrame>,
        stack: Vec<Value>,
        started: bool,
    },
    /// Currently executing (guards against re-entrant `next`).
    Running,
    /// Finished (returned or threw).
    Completed,
}

/// The element type of a typed-array view (and which `TypedArray` subclass it
/// is). Carries the byte width plus the per-element read/coerce-write logic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TypedArrayKind {
    Int8,
    Uint8,
    Uint8Clamped,
    Int16,
    Uint16,
    Int32,
    Uint32,
    Float32,
    Float64,
}

impl TypedArrayKind {
    pub fn bytes_per_element(self) -> usize {
        match self {
            Self::Int8 | Self::Uint8 | Self::Uint8Clamped => 1,
            Self::Int16 | Self::Uint16 => 2,
            Self::Int32 | Self::Uint32 | Self::Float32 => 4,
            Self::Float64 => 8,
        }
    }

    pub fn constructor_name(self) -> &'static str {
        match self {
            Self::Int8 => "Int8Array",
            Self::Uint8 => "Uint8Array",
            Self::Uint8Clamped => "Uint8ClampedArray",
            Self::Int16 => "Int16Array",
            Self::Uint16 => "Uint16Array",
            Self::Int32 => "Int32Array",
            Self::Uint32 => "Uint32Array",
            Self::Float32 => "Float32Array",
            Self::Float64 => "Float64Array",
        }
    }

    /// Read the element starting at `byte_index`, returned as the JS numeric
    /// (f64) value. An out-of-range index reads as 0.
    pub fn read_element(self, bytes: &[u8], byte_index: usize) -> f64 {
        let size = self.bytes_per_element();
        let Some(slice) = bytes.get(byte_index..byte_index + size) else {
            return 0.0;
        };
        match self {
            Self::Int8 => i8::from_le_bytes([slice[0]]) as f64,
            Self::Uint8 | Self::Uint8Clamped => slice[0] as f64,
            Self::Int16 => i16::from_le_bytes([slice[0], slice[1]]) as f64,
            Self::Uint16 => u16::from_le_bytes([slice[0], slice[1]]) as f64,
            Self::Int32 => i32::from_le_bytes([slice[0], slice[1], slice[2], slice[3]]) as f64,
            Self::Uint32 => u32::from_le_bytes([slice[0], slice[1], slice[2], slice[3]]) as f64,
            Self::Float32 => f32::from_le_bytes([slice[0], slice[1], slice[2], slice[3]]) as f64,
            Self::Float64 => f64::from_le_bytes([
                slice[0], slice[1], slice[2], slice[3], slice[4], slice[5], slice[6], slice[7],
            ]),
        }
    }

    /// Coerce `value` per the element type and write it starting at
    /// `byte_index`. An out-of-range index is a no-op.
    pub fn coerce_and_write(self, bytes: &mut [u8], byte_index: usize, value: f64) {
        let size = self.bytes_per_element();
        if byte_index + size > bytes.len() {
            return;
        }
        let target = &mut bytes[byte_index..byte_index + size];
        match self {
            Self::Int8 => target.copy_from_slice(&(to_int32(value) as i8).to_le_bytes()),
            Self::Uint8 => target.copy_from_slice(&(to_int32(value) as u8).to_le_bytes()),
            Self::Uint8Clamped => target.copy_from_slice(&[clamp_to_u8(value)]),
            Self::Int16 => target.copy_from_slice(&(to_int32(value) as i16).to_le_bytes()),
            Self::Uint16 => target.copy_from_slice(&(to_int32(value) as u16).to_le_bytes()),
            Self::Int32 => target.copy_from_slice(&to_int32(value).to_le_bytes()),
            Self::Uint32 => target.copy_from_slice(&(to_int32(value) as u32).to_le_bytes()),
            Self::Float32 => target.copy_from_slice(&(value as f32).to_le_bytes()),
            Self::Float64 => target.copy_from_slice(&value.to_le_bytes()),
        }
    }
}

/// ECMAScript `ToInt32`: truncate toward zero, then reduce modulo 2^32 into the
/// signed 32-bit range. NaN and the infinities map to 0. Narrower integer
/// element types derive from this via Rust's wrapping `as` casts.
fn to_int32(value: f64) -> i32 {
    if !value.is_finite() || value == 0.0 {
        return 0;
    }
    let modulo = value.trunc().rem_euclid(4_294_967_296.0);
    if modulo >= 2_147_483_648.0 {
        (modulo - 4_294_967_296.0) as i32
    } else {
        modulo as i32
    }
}

/// `Uint8ClampedArray` write conversion: clamp to [0, 255] with round-half-to-
/// even, NaN to 0.
fn clamp_to_u8(value: f64) -> u8 {
    if value.is_nan() || value <= 0.0 {
        return 0;
    }
    if value >= 255.0 {
        return 255;
    }
    let floor = value.floor();
    let diff = value - floor;
    let rounded = if diff < 0.5 {
        floor
    } else if diff > 0.5 {
        floor + 1.0
    } else if (floor as i64) % 2 == 0 {
        floor
    } else {
        floor + 1.0
    };
    rounded as u8
}

#[derive(Clone)]
pub enum ObjectKind {
    Ordinary,
    Array,
    Function,
    Error,
    Promise(Box<PromiseState>),
    AsyncResumer(Box<AsyncContext>),
    AsyncGenerator {
        state: Box<GeneratorState>,
        queue: std::collections::VecDeque<AsyncGeneratorRequest>,
    },
    Generator(Box<GeneratorState>),
    Proxy {
        target: GcRef<JsObject>,
        handler: GcRef<JsObject>,
    },
    RegExp {
        source: String,
        flags: String,
        global: bool,
        last_index: u32,
    },
    Map(Vec<(Value, Value)>),
    Set(Vec<Value>),
    /// Ordered (name, value) pairs backing a `URLSearchParams`.
    UrlSearchParams(Vec<(String, String)>),
    /// Raw byte backing store for an `ArrayBuffer`.
    ArrayBuffer(Vec<u8>),
    /// A typed-array view over an `ArrayBuffer` object (`buffer`), interpreting
    /// `length` elements of type `kind` starting at `byte_offset`.
    TypedArray {
        buffer: GcRef<JsObject>,
        kind: TypedArrayKind,
        byte_offset: usize,
        length: usize,
    },
    WeakMap(Vec<(Value, Value)>),
    WeakSet(Vec<Value>),
    ForOfIterator {
        values: Vec<Value>,
        index: usize,
    },
    Host(HostObjectSlot),
    Exotic(&'static str),
}

impl std::fmt::Debug for ObjectKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Ordinary => f.write_str("Ordinary"),
            Self::Array => f.write_str("Array"),
            Self::Function => f.write_str("Function"),
            Self::Error => f.write_str("Error"),
            Self::Promise(state) => f.debug_tuple("Promise").field(state).finish(),
            Self::AsyncResumer(_) => f.write_str("AsyncResumer(..)"),
            Self::AsyncGenerator { state, queue } => f
                .debug_struct("AsyncGenerator")
                .field("state", state)
                .field("queue", queue)
                .finish(),
            Self::Generator(_) => f.write_str("Generator(..)"),
            Self::Proxy { .. } => f.write_str("Proxy(..)"),
            Self::RegExp {
                source,
                flags,
                global,
                last_index,
            } => f
                .debug_struct("RegExp")
                .field("source", source)
                .field("flags", flags)
                .field("global", global)
                .field("last_index", last_index)
                .finish(),
            Self::Map(entries) => f.debug_tuple("Map").field(entries).finish(),
            Self::Set(values) => f.debug_tuple("Set").field(values).finish(),
            Self::UrlSearchParams(pairs) => {
                f.debug_tuple("UrlSearchParams").field(pairs).finish()
            }
            Self::ArrayBuffer(bytes) => {
                f.debug_tuple("ArrayBuffer").field(&bytes.len()).finish()
            }
            Self::TypedArray {
                kind,
                byte_offset,
                length,
                ..
            } => f
                .debug_struct("TypedArray")
                .field("kind", kind)
                .field("byte_offset", byte_offset)
                .field("length", length)
                .finish(),
            Self::WeakMap(entries) => f.debug_tuple("WeakMap").field(entries).finish(),
            Self::WeakSet(values) => f.debug_tuple("WeakSet").field(values).finish(),
            Self::ForOfIterator { values, index } => f
                .debug_struct("ForOfIterator")
                .field("values", values)
                .field("index", index)
                .finish(),
            Self::AsyncGenerator { state, queue } => f
                .debug_struct("AsyncGenerator")
                .field("state", state)
                .field("queue", queue)
                .finish(),
            Self::Host(slot) => f.debug_tuple("Host").field(slot).finish(),
            Self::Exotic(name) => f.debug_tuple("Exotic").field(name).finish(),
        }
    }
}

impl PartialEq for ObjectKind {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Ordinary, Self::Ordinary)
            | (Self::Array, Self::Array)
            | (Self::Function, Self::Function)
            | (Self::Error, Self::Error) => true,
            (Self::Promise(left), Self::Promise(right)) => left == right,
            (Self::AsyncResumer(_), Self::AsyncResumer(_)) => true,
            (
                Self::AsyncGenerator {
                    state: left_state,
                    queue: left_queue,
                },
                Self::AsyncGenerator {
                    state: right_state,
                    queue: right_queue,
                },
            ) => left_state == right_state && left_queue == right_queue,
            (
                Self::RegExp {
                    source: left_source,
                    flags: left_flags,
                    global: left_global,
                    last_index: left_last_index,
                },
                Self::RegExp {
                    source: right_source,
                    flags: right_flags,
                    global: right_global,
                    last_index: right_last_index,
                },
            ) => {
                left_source == right_source
                    && left_flags == right_flags
                    && left_global == right_global
                    && left_last_index == right_last_index
            }
            (Self::Map(left), Self::Map(right)) | (Self::WeakMap(left), Self::WeakMap(right)) => {
                left == right
            }
            (Self::Set(left), Self::Set(right)) | (Self::WeakSet(left), Self::WeakSet(right)) => {
                left == right
            }
            (
                Self::ForOfIterator {
                    values: left_values,
                    index: left_index,
                },
                Self::ForOfIterator {
                    values: right_values,
                    index: right_index,
                },
            ) => left_values == right_values && left_index == right_index,
            (Self::Host(left), Self::Host(right)) => left == right,
            (Self::Exotic(left), Self::Exotic(right)) => left == right,
            _ => false,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct JsObject {
    pub kind: ObjectKind,
    pub prototype: Option<GcRef<JsObject>>,
    pub extensible: bool,
    pub properties: IndexMap<PropertyKey, JsPropertyDescriptor>,
}

impl Default for JsObject {
    fn default() -> Self {
        Self {
            kind: ObjectKind::Ordinary,
            prototype: None,
            extensible: true,
            properties: IndexMap::new(),
        }
    }
}
