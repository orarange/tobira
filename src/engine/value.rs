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
    pub outer_promise: GcRef<JsObject>,
}

/// Execution state of a generator object.
#[derive(Debug, Clone)]
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

#[derive(Clone)]
pub enum ObjectKind {
    Ordinary,
    Array,
    Function,
    Error,
    Promise(Box<PromiseState>),
    AsyncResumer(Box<AsyncContext>),
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
            Self::WeakMap(entries) => f.debug_tuple("WeakMap").field(entries).finish(),
            Self::WeakSet(values) => f.debug_tuple("WeakSet").field(values).finish(),
            Self::ForOfIterator { values, index } => f
                .debug_struct("ForOfIterator")
                .field("values", values)
                .field("index", index)
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
