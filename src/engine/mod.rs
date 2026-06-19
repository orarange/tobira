#![allow(dead_code, unused_imports)]

pub mod ast;
pub mod chunk;
pub mod compiler;
pub mod event_loop;
pub mod heap;
pub mod js_regex;
pub mod host;
pub mod lexer;
pub mod parser;
pub mod verifier;
pub mod value;
pub mod vm;

pub use ast::{
    ArrowFunctionExpression, ExportAllDeclaration, ExportDefaultDeclaration,
    ExportNamedDeclaration, FunctionDeclaration, FunctionExpression, MetaProperty, Program,
    ProgramKind, SourcePosition, SourceSpan, SourceType, StatementNode, VariableDeclaration,
};
pub use chunk::{Chunk, Constant, FunctionProto, Opcode, UpvalueDescriptor};
pub use compiler::{CompileError, Compiler, compile};
pub use event_loop::{
    EventLoop, MicrotaskJob, RafEntry, TaskEntry, TaskSource, TickResult, TimerEntry,
};
pub use heap::{
    Arena, ArenaItem, ArenaPage, GcColor, GcRef, Heap, HeapArena, HeapHeader, RawGcRef, RootHandle,
    RootSet,
};
pub use js_regex::{JsCaptures, JsMatch, JsRegex};
pub use host::{
    AdjacentPosition,
    ConsoleLevel, ConsoleMessage, DomEventRequest, DomEventResult, DomMutation, DomMutationResult,
    DomRead, DomReadResult, DomRect, DomStructuralChange, EventTarget, FetchBody, FetchMode, FetchRequest,
    FetchResponse, FrameId,
    HistoryAction, HistoryOutcome, Host, HostData, HostError, HostEvent, HostResult,
    HostTimeSnapshot, HttpMethod, LocationSnapshot, NavigationAction, NavigationOutcome,
    NetworkRequestId, NodeId, NodeKind, NoopHost, ObserverId, ObserverKind, ObserverOp,
    ObserverOptions, ObserverRecord, ObserverResult, ScrollMetrics, SiblingDirection, StorageAreaKind,
    StorageAreaScope, StorageOp, StorageResult, TimerId, TimerKind, TimerRequest, WindowId,
    WindowMetrics,
};
pub use lexer::{LexError, LexGoal, Lexer, SourceLocation, Token, TokenKind};
pub use parser::{ParseError, Parser, ParserOptions};
pub use verifier::{StackVerifyError, verify_stack_balance};
pub use value::{
    AsyncContext, HostDispatch, HostObjectClass, HostObjectSlot, JsObject, JsPropertyDescriptor,
    JsString, ObjectKind, PromiseReaction, PromiseState, PropertyKey, SymbolId, Value,
};
pub use vm::{CallFrame, DomEventInit, Vm, VmError};
// fire_dom_event is a method on Vm — accessible directly via Vm::fire_dom_event
