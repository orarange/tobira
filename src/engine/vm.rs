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
use super::value::{
    AsyncContext, JsObject, JsPropertyDescriptor, JsString, ObjectKind, PromiseReaction,
    PromiseState, PropertyKey, Value,
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
    error_prototype: Option<GcRef<JsObject>>,
    promise_prototype: Option<GcRef<JsObject>>,
    map_prototype: Option<GcRef<JsObject>>,
    set_prototype: Option<GcRef<JsObject>>,
    event_loop: EventLoop,
    random_state: u64,
}

impl Vm {
    pub fn new(heap: Heap) -> Self {
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
            error_prototype: None,
            promise_prototype: None,
            map_prototype: None,
            set_prototype: None,
            event_loop: EventLoop::new(),
            random_state,
        };
        vm.install_globals();
        vm
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
            Opcode::GetUpvalue(slot) => {
                let value = self.upvalue_cell(slot)?.borrow().clone();
                self.stack.push(value);
            }
            Opcode::SetUpvalue(slot) => {
                let value = self.pop_value()?;
                *self.upvalue_cell(slot)?.borrow_mut() = value;
            }
            Opcode::GetGlobal(index) => {
                let name = self.constant_name(index)?;
                let value = self
                    .globals
                    .get(&name)
                    .cloned()
                    .ok_or_else(|| VmError::ReferenceError(format!("{name} is not defined")))?;
                self.stack.push(value);
            }
            Opcode::SetGlobal(index) => {
                let name = self.constant_name(index)?;
                let value = self.pop_value()?;
                self.globals.insert(name, value);
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
            Opcode::In => {
                let object = self.pop_value()?;
                let key = self.pop_value()?;
                let key = self.to_property_key(&key)?;
                let object = self.require_object_ref(&object, "in operator")?;
                self.stack.push(Value::Bool(
                    self.lookup_property_descriptor(object, &key).is_some(),
                ));
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
                let source_value = self.make_string_value(&pattern);
                let flags_value = self.make_string_value(&flags);
                let object = self.heap.allocate_object(JsObject {
                    kind: ObjectKind::RegExp {
                        source: pattern.clone(),
                        flags: flags.clone(),
                        global: flags.contains('g'),
                        last_index: 0,
                    },
                    prototype: Some(self.object_prototype_ref()),
                    ..JsObject::default()
                });
                self.define_data_property(
                    object,
                    PropertyKey::from("source"),
                    source_value,
                    false,
                    false,
                    false,
                );
                self.define_data_property(
                    object,
                    PropertyKey::from("flags"),
                    flags_value,
                    false,
                    false,
                    false,
                );
                self.stack.push(Value::Object(object));
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
                let key = PropertyKey::from(self.constant_name(index)?);
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
        self.error_prototype = Some(error_prototype);
        self.promise_prototype = Some(promise_prototype);
        self.map_prototype = Some(map_prototype);
        self.set_prototype = Some(set_prototype);

        let assert_value = self.allocate_builtin_value(BuiltinId::Assert, false, None);
        self.globals.insert("assert".to_string(), assert_value);
        let call_spread = self.allocate_builtin_value(BuiltinId::CallSpread, false, None);
        let construct_spread = self.allocate_builtin_value(BuiltinId::ConstructSpread, false, None);
        let queue_microtask = self.allocate_builtin_value(BuiltinId::QueueMicrotask, false, None);
        let set_timeout = self.allocate_builtin_value(BuiltinId::SetTimeout, false, None);
        let clear_timeout = self.allocate_builtin_value(BuiltinId::ClearTimeout, false, None);
        let set_interval = self.allocate_builtin_value(BuiltinId::SetInterval, false, None);
        let clear_interval = self.allocate_builtin_value(BuiltinId::ClearInterval, false, None);
        let request_animation_frame =
            self.allocate_builtin_value(BuiltinId::RequestAnimationFrame, false, None);
        let cancel_animation_frame =
            self.allocate_builtin_value(BuiltinId::CancelAnimationFrame, false, None);
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
        let promise_ctor = self.allocate_callable_value(
            Callable::Builtin(BuiltinId::PromiseConstructor),
            true,
            Some(promise_prototype),
        );
        let number_ctor = self.allocate_builtin_value(BuiltinId::NumberParseInt, false, None);
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
            .insert("Promise".to_string(), promise_ctor.clone());
        let number_object = self.create_number_object();
        self.globals.insert("Number".to_string(), number_object);
        self.globals
            .insert("Math".to_string(), Value::Object(math_object));
        self.globals
            .insert("JSON".to_string(), Value::Object(json_object));

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
        }

        if let Some(array_ref) = self.value_object_ref(array_ctor) {
            self.define_builtin_method(array_ref, "isArray", BuiltinId::ArrayIsArray);
            self.define_builtin_method(array_ref, "from", BuiltinId::ArrayFrom);
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
        let _ = number_ctor;
    }

    fn create_number_object(&mut self) -> Value {
        let object_ref = self.heap.allocate_object(JsObject {
            kind: ObjectKind::Function,
            prototype: Some(self.function_prototype_ref()),
            ..JsObject::default()
        });
        Value::Object(object_ref)
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
        let context = match self
            .heap
            .objects()
            .get(resumer)
            .map(|object| object.kind.clone())
        {
            Some(ObjectKind::AsyncResumer(context)) => *context,
            _ => {
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

        self.frames.push(CallFrame {
            proto: closure.proto,
            ip: 0,
            stack_base: self.stack.len(),
            locals,
            upvalues: closure.upvalues,
            this_value,
            construct_fallback,
            pending_exception: None,
            async_outer_promise: None,
        });
        Ok(())
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
                if closure.proto.is_async {
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

    fn current_this(&self) -> Result<&Value, VmError> {
        self.frames
            .last()
            .map(|frame| &frame.this_value)
            .ok_or_else(|| VmError::RangeError("no current this binding".to_string()))
    }

    fn constant_name(&self, index: u16) -> Result<String, VmError> {
        match self.current_proto()?.constants.get(index as usize) {
            Some(Constant::String(value)) => Ok(value.clone()),
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
        if let Some(descriptor) = self.get_own_property_descriptor(object, &key) {
            return match descriptor {
                JsPropertyDescriptor::Data {
                    writable: false, ..
                } => Err(VmError::TypeError(format!(
                    "property {} is not writable",
                    self.property_key_to_string(&key)
                ))),
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
            return Err(VmError::TypeError("object is not extensible".to_string()));
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
        let kind = self
            .heap
            .objects()
            .get(object)
            .map(|object| object.kind.clone())
            .unwrap_or(ObjectKind::Ordinary);
        if kind != ObjectKind::Array {
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
                    _ => self.array_like_to_vec(value),
                }
            }
            _ => Err(VmError::TypeError(
                "value is not iterable in phase 4".to_string(),
            )),
        }
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
            BuiltinId::ArrayConstructor => self.make_array_from_values(args),
            BuiltinId::MapConstructor => {
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
            BuiltinId::SetConstructor => {
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
                let values = match &source {
                    Value::String(string) => self
                        .string_text(*string)
                        .chars()
                        .map(|character| self.make_string_value(&character.to_string()))
                        .collect(),
                    _ => self.array_like_to_vec(&source)?,
                };
                self.make_array_from_values(values)
            }
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
                let mut flattened = Vec::new();
                for value in values {
                    match value {
                        Value::Object(object)
                            if self
                                .heap
                                .objects()
                                .get(object)
                                .map(|object| object.kind == ObjectKind::Array)
                                .unwrap_or(false) =>
                        {
                            flattened.extend(self.array_like_to_vec(&Value::Object(object))?);
                        }
                        other => flattened.push(other),
                    }
                }
                self.make_array_from_values(flattened)
            }
            BuiltinId::ArrayProtoSome => self.array_callback_predicate(&this_value, args, true),
            BuiltinId::ArrayProtoEvery => self.array_callback_predicate(&this_value, args, false),
            BuiltinId::ArrayProtoSort => {
                let object = self.builtin_object_this(&this_value, "Array.prototype.sort")?;
                let mut values = self.array_like_to_vec(&this_value)?;
                if let Some(compare_fn) = args.first() {
                    if !matches!(compare_fn, Value::Undefined) {
                        let compare_fn = compare_fn.clone();
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
                        values.sort_by_key(|value| self.to_string(value));
                    }
                } else {
                    values.sort_by_key(|value| self.to_string(value));
                }
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
                let search = args
                    .first()
                    .map(|value| self.to_string(value))
                    .unwrap_or_default();
                let replacement = args
                    .get(1)
                    .map(|value| self.to_string(value))
                    .unwrap_or_default();
                let replaced = if builtin == BuiltinId::StringProtoReplace {
                    text.replacen(&search, &replacement, 1)
                } else {
                    text.replace(&search, &replacement)
                };
                Ok(self.make_string_value(&replaced))
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
            BuiltinId::NumberParseInt => {
                let text = args
                    .first()
                    .map(|value| self.to_string(value))
                    .unwrap_or_default();
                let radix = args
                    .get(1)
                    .map(|value| self.to_number(value) as u32)
                    .unwrap_or(10);
                let parsed = i64::from_str_radix(text.trim(), radix)
                    .map(|value| Value::Number(value as f64))
                    .unwrap_or(Value::Number(f64::NAN));
                Ok(parsed)
            }
            BuiltinId::NumberParseFloat => {
                let text = args
                    .first()
                    .map(|value| self.to_string(value))
                    .unwrap_or_default();
                Ok(Value::Number(
                    text.trim().parse::<f64>().unwrap_or(f64::NAN),
                ))
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
                match self.to_json_value(&value)? {
                    Some(json) => Ok(self.make_string_value(
                        &serde_json::to_string(&json).unwrap_or_else(|_| "null".to_string()),
                    )),
                    None => Ok(Value::Undefined),
                }
            }
            BuiltinId::JsonParse => {
                let text = args
                    .first()
                    .map(|value| self.to_string(value))
                    .unwrap_or_default();
                let json = serde_json::from_str::<JsonValue>(&text)
                    .map_err(|error| VmError::TypeError(error.to_string()))?;
                self.from_json_value(&json)
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
                let entries = match self
                    .heap
                    .objects()
                    .get(object)
                    .map(|data| data.kind.clone())
                {
                    Some(ObjectKind::Map(entries)) | Some(ObjectKind::WeakMap(entries)) => entries,
                    _ => Vec::new(),
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
                let entries = match self
                    .heap
                    .objects()
                    .get(object)
                    .map(|data| data.kind.clone())
                {
                    Some(ObjectKind::Map(entries)) | Some(ObjectKind::WeakMap(entries)) => entries,
                    _ => Vec::new(),
                };
                let mut pairs = Vec::with_capacity(entries.len());
                for (key, value) in entries {
                    pairs.push(self.make_array_from_values(vec![key, value])?);
                }
                self.make_array_from_values(pairs)
            }
            BuiltinId::MapProtoKeys => {
                let object = self.builtin_object_this(&this_value, "Map.prototype.keys")?;
                let entries = match self
                    .heap
                    .objects()
                    .get(object)
                    .map(|data| data.kind.clone())
                {
                    Some(ObjectKind::Map(entries)) | Some(ObjectKind::WeakMap(entries)) => entries,
                    _ => Vec::new(),
                };
                self.make_array_from_values(entries.into_iter().map(|(key, _)| key).collect())
            }
            BuiltinId::MapProtoValues => {
                let object = self.builtin_object_this(&this_value, "Map.prototype.values")?;
                let entries = match self
                    .heap
                    .objects()
                    .get(object)
                    .map(|data| data.kind.clone())
                {
                    Some(ObjectKind::Map(entries)) | Some(ObjectKind::WeakMap(entries)) => entries,
                    _ => Vec::new(),
                };
                self.make_array_from_values(entries.into_iter().map(|(_, value)| value).collect())
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
                let values = match self
                    .heap
                    .objects()
                    .get(object)
                    .map(|data| data.kind.clone())
                {
                    Some(ObjectKind::Set(values)) | Some(ObjectKind::WeakSet(values)) => values,
                    _ => Vec::new(),
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
                let values = match self
                    .heap
                    .objects()
                    .get(object)
                    .map(|data| data.kind.clone())
                {
                    Some(ObjectKind::Set(values)) | Some(ObjectKind::WeakSet(values)) => values,
                    _ => Vec::new(),
                };
                self.make_array_from_values(values)
            }
        }
    }

    fn number_arg(&self, args: &[Value], index: usize) -> f64 {
        args.get(index)
            .map(|value| self.to_number(value))
            .unwrap_or(f64::NAN)
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
        self.set_array_length(object, self.array_length(object));
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

    fn to_json_value(&mut self, value: &Value) -> Result<Option<JsonValue>, VmError> {
        Ok(match value {
            Value::Undefined | Value::Symbol(_) => None,
            Value::Null => Some(JsonValue::Null),
            Value::Bool(boolean) => Some(JsonValue::Bool(*boolean)),
            Value::Number(number) => serde_json::Number::from_f64(*number).map(JsonValue::Number),
            Value::String(string) => Some(JsonValue::String(self.string_text(*string))),
            Value::Object(object) => {
                if self.callables.contains_key(&object.raw()) {
                    return Ok(None);
                }
                let kind = self
                    .heap
                    .objects()
                    .get(*object)
                    .map(|object_data| object_data.kind.clone())
                    .unwrap_or(ObjectKind::Ordinary);
                if kind == ObjectKind::Array {
                    let mut items = Vec::new();
                    for value in self.array_like_to_vec(&Value::Object(*object))? {
                        items.push(self.to_json_value(&value)?.unwrap_or(JsonValue::Null));
                    }
                    Some(JsonValue::Array(items))
                } else {
                    let mut map = serde_json::Map::new();
                    for key in self.object_own_enumerable_keys(*object) {
                        let value = self.get_property_value(&Value::Object(*object), &key)?;
                        if let Some(json) = self.to_json_value(&value)? {
                            map.insert(self.property_key_to_string(&key), json);
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
