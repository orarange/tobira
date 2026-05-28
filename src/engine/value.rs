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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ObjectKind {
    Ordinary,
    Array,
    Function,
    Error,
    Host(HostObjectSlot),
    Exotic(&'static str),
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
