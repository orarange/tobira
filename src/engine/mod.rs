#![allow(dead_code, unused_imports)]

pub mod ast;
pub mod heap;
pub mod host;
pub mod lexer;
pub mod parser;
pub mod value;

pub use ast::{
    ArrowFunctionExpression, ExportAllDeclaration, ExportDefaultDeclaration,
    ExportNamedDeclaration, FunctionDeclaration, FunctionExpression, MetaProperty, Program,
    ProgramKind, SourcePosition, SourceSpan, SourceType, StatementNode, VariableDeclaration,
};
pub use heap::{
    Arena, ArenaItem, ArenaPage, GcColor, GcRef, Heap, HeapArena, HeapHeader, RawGcRef, RootHandle,
    RootSet,
};
pub use host::{
    ConsoleLevel, ConsoleMessage, DomEventRequest, DomEventResult, DomMutation, DomMutationResult,
    DomRead, DomReadResult, EventTarget, FetchBody, FetchMode, FetchRequest, FrameId,
    HistoryAction, HistoryOutcome, Host, HostData, HostError, HostEvent, HostResult,
    HostTimeSnapshot, HttpMethod, LocationSnapshot, NavigationAction, NavigationOutcome,
    NetworkRequestId, NodeId, NodeKind, ObserverId, ObserverKind, ObserverOp, ObserverOptions,
    ObserverResult, StorageAreaKind, StorageAreaScope, StorageOp, StorageResult, TimerId,
    TimerKind, TimerRequest, WindowId, WindowMetrics,
};
pub use lexer::{LexError, LexGoal, Lexer, SourceLocation, Token, TokenKind};
pub use parser::{ParseError, Parser, ParserOptions};
pub use value::{
    HostDispatch, HostObjectClass, HostObjectSlot, JsObject, JsProperty, JsPropertyDescriptor,
    JsString, ObjectKind, PropertyKey, SymbolId, Value,
};
