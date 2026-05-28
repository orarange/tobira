use std::{cell::RefCell, collections::HashMap, rc::Rc};

use super::chunk::{Chunk, Constant, FunctionProto, Opcode};
use super::heap::{GcRef, Heap, RawGcRef};
use super::value::{JsObject, JsString, ObjectKind, Value};

type ValueCell = Rc<RefCell<Value>>;
type BuiltinFunction = fn(&mut Vm, Value, Vec<Value>) -> Result<Value, VmError>;

#[derive(Debug, Clone)]
struct RuntimeClosure {
    proto: Rc<FunctionProto>,
    upvalues: Vec<ValueCell>,
}

#[derive(Debug, Clone)]
enum Callable {
    Builtin(BuiltinFunction),
    Closure(RuntimeClosure),
}

#[derive(Debug)]
pub struct CallFrame {
    proto: Rc<FunctionProto>,
    ip: usize,
    stack_base: usize,
    locals: Vec<ValueCell>,
    upvalues: Vec<ValueCell>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum VmError {
    TypeError(String),
    ReferenceError(String),
    RangeError(String),
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
            Self::InfiniteLoop => write!(f, "execution exceeded the per-call loop budget"),
            Self::StackOverflow => write!(f, "call stack exceeded the phase 2 limit"),
            Self::Unimplemented(feature) => write!(f, "unimplemented in phase 2: {feature}"),
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
}

impl Vm {
    pub fn new(heap: Heap) -> Self {
        let mut vm = Self {
            stack: Vec::new(),
            frames: Vec::new(),
            heap,
            globals: HashMap::new(),
            callables: HashMap::new(),
            string_cache: HashMap::new(),
            fuel: 1_000_000,
        };
        vm.install_globals();
        vm
    }

    pub fn execute(&mut self, chunk: &Chunk) -> Result<Value, VmError> {
        self.stack.clear();
        self.frames.clear();

        let closure = RuntimeClosure {
            proto: Rc::new(chunk.top_level.clone()),
            upvalues: Vec::new(),
        };
        self.push_call_frame(closure, Vec::new())?;

        loop {
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
                    };
                    self.stack.push(value);
                }
                Opcode::LoadUndefined => self.stack.push(Value::Undefined),
                Opcode::LoadNull => self.stack.push(Value::Null),
                Opcode::LoadTrue => self.stack.push(Value::Bool(true)),
                Opcode::LoadFalse => self.stack.push(Value::Bool(false)),
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
                    let value = self.globals.get(&name).cloned().ok_or_else(|| {
                        VmError::ReferenceError(format!("{name} is not defined"))
                    })?;
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
                Opcode::StrictEq => {
                    self.binary_compare(|vm, lhs, rhs| vm.strict_equal(lhs, rhs))?
                }
                Opcode::Ne => self.binary_compare(|vm, lhs, rhs| !vm.abstract_equal(lhs, rhs))?,
                Opcode::StrictNe => {
                    self.binary_compare(|vm, lhs, rhs| !vm.strict_equal(lhs, rhs))?
                }
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
                Opcode::In => return Err(VmError::Unimplemented("in operator")),
                Opcode::Instanceof => return Err(VmError::Unimplemented("instanceof operator")),
                Opcode::Jump(offset) => {
                    self.apply_jump(offset)?;
                }
                Opcode::JumpIfTrue(offset) => {
                    let take_jump = self.is_truthy(self.peek_value()?);
                    if take_jump {
                        self.apply_jump(offset)?;
                    }
                }
                Opcode::JumpIfFalse(offset) => {
                    let take_jump = !self.is_truthy(self.peek_value()?);
                    if take_jump {
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
                Opcode::Call(argc) => {
                    let mut args = Vec::with_capacity(argc as usize);
                    for _ in 0..argc {
                        args.push(self.pop_value()?);
                    }
                    args.reverse();
                    let this_value = self.pop_value()?;
                    let callee = self.pop_value()?;
                    match self.resolve_callable(&callee)? {
                        Callable::Builtin(function) => {
                            let result = function(self, this_value, args)?;
                            self.stack.push(result);
                        }
                        Callable::Closure(closure) => {
                            self.push_call_frame(closure, args)?;
                        }
                    }
                }
                Opcode::Return => {
                    let value = self.pop_value()?;
                    let frame = self
                        .frames
                        .pop()
                        .ok_or_else(|| VmError::RangeError("return without a frame".to_string()))?;
                    self.stack.truncate(frame.stack_base);
                    if self.frames.is_empty() {
                        return Ok(value);
                    }
                    self.stack.push(value);
                }
                Opcode::MakeClosure(index) => {
                    let proto = self
                        .current_proto()?
                        .nested_functions
                        .get(index as usize)
                        .cloned()
                        .ok_or_else(|| {
                            VmError::RangeError(format!(
                                "function proto index {index} out of range"
                            ))
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
                Opcode::MakeObject => return Err(VmError::Unimplemented("object literals")),
                Opcode::MakeArray(_) => return Err(VmError::Unimplemented("array literals")),
                Opcode::GetProp | Opcode::SetProp | Opcode::GetIndex | Opcode::SetIndex => {
                    return Err(VmError::Unimplemented("property access"));
                }
                Opcode::Throw => {
                    let thrown = self.pop_value()?;
                    return Err(VmError::TypeError(format!(
                        "uncaught throw: {}",
                        self.to_string(&thrown)
                    )));
                }
                Opcode::Nop => {}
            }
        }
    }

    fn install_globals(&mut self) {
        self.globals
            .insert("undefined".to_string(), Value::Undefined);
        self.globals
            .insert("NaN".to_string(), Value::Number(f64::NAN));
        let assert_value = self.allocate_builtin(Self::builtin_assert);
        self.globals.insert("assert".to_string(), assert_value);
    }

    fn builtin_assert(&mut self, _this: Value, args: Vec<Value>) -> Result<Value, VmError> {
        let condition = args.first().cloned().unwrap_or(Value::Undefined);
        if self.is_truthy(&condition) {
            Ok(Value::Undefined)
        } else {
            Err(VmError::TypeError("assertion failed".to_string()))
        }
    }

    fn allocate_builtin(&mut self, function: BuiltinFunction) -> Value {
        let object_ref = self.heap.allocate_object(JsObject {
            kind: ObjectKind::Function,
            ..JsObject::default()
        });
        self.callables
            .insert(object_ref.raw(), Callable::Builtin(function));
        Value::Object(object_ref)
    }

    fn allocate_function_value(&mut self, closure: RuntimeClosure) -> Value {
        let object_ref = self.heap.allocate_object(JsObject {
            kind: ObjectKind::Function,
            ..JsObject::default()
        });
        self.callables
            .insert(object_ref.raw(), Callable::Closure(closure));
        Value::Object(object_ref)
    }

    fn push_call_frame(
        &mut self,
        closure: RuntimeClosure,
        args: Vec<Value>,
    ) -> Result<(), VmError> {
        if self.frames.len() >= 1024 {
            return Err(VmError::StackOverflow);
        }

        let mut locals = Vec::with_capacity(closure.proto.local_count as usize);
        for _ in 0..closure.proto.local_count {
            locals.push(Rc::new(RefCell::new(Value::Undefined)));
        }

        for (index, value) in args.into_iter().enumerate() {
            if index >= locals.len() {
                break;
            }
            *locals[index].borrow_mut() = value;
        }

        let frame = CallFrame {
            proto: closure.proto,
            ip: 0,
            stack_base: self.stack.len(),
            locals,
            upvalues: closure.upvalues,
        };
        self.frames.push(frame);
        Ok(())
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

    fn constant_name(&self, index: u16) -> Result<String, VmError> {
        match self.current_proto()?.constants.get(index as usize) {
            Some(Constant::String(value)) => Ok(value.clone()),
            Some(Constant::Number(_)) => Err(VmError::TypeError(format!(
                "constant {index} was not a string"
            ))),
            None => Err(VmError::RangeError(format!(
                "constant index {index} out of range"
            ))),
        }
    }

    fn local_cell(&self, slot: u16) -> Result<&ValueCell, VmError> {
        self.frames
            .last()
            .and_then(|frame| frame.locals.get(slot as usize))
            .ok_or_else(|| VmError::RangeError(format!("local slot {slot} out of range")))
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
            Value::Object(_) => "[object Object]".to_string(),
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
            std::cmp::Ordering::Less => operator(-1.0, 0.0),
            std::cmp::Ordering::Equal => operator(0.0, 0.0),
            std::cmp::Ordering::Greater => operator(1.0, 0.0),
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
}
