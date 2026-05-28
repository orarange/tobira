#![allow(dead_code, unused_imports)]

pub mod ast;
pub mod chunk;
pub mod compiler;
pub mod heap;
pub mod host;
pub mod lexer;
pub mod parser;
pub mod value;
pub mod vm;

pub use ast::{
    ArrowFunctionExpression, ExportAllDeclaration, ExportDefaultDeclaration,
    ExportNamedDeclaration, FunctionDeclaration, FunctionExpression, MetaProperty, Program,
    ProgramKind, SourcePosition, SourceSpan, SourceType, StatementNode, VariableDeclaration,
};
pub use chunk::{Chunk, Constant, FunctionProto, Opcode, UpvalueDescriptor};
pub use compiler::{CompileError, Compiler, compile};
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
    HostDispatch, HostObjectClass, HostObjectSlot, JsObject, JsPropertyDescriptor, JsString,
    ObjectKind, PropertyKey, SymbolId, Value,
};
pub use vm::{CallFrame, Vm, VmError};
