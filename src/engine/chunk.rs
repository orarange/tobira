#[derive(Debug, Clone, PartialEq)]
pub enum Opcode {
    LoadConst(u16),
    LoadUndefined,
    LoadNull,
    LoadTrue,
    LoadFalse,
    LoadThis,
    /// Push `new.target` for the current frame (the constructor, or undefined).
    LoadNewTarget,
    Pop,
    Dup,

    GetLocal(u16),
    SetLocal(u16),
    /// Replace a local's storage cell with a fresh one holding a copy of the
    /// current value. Used to give `for (let …)` loops a per-iteration binding so
    /// closures created in different iterations capture distinct variables.
    FreshenLocal(u16),

    GetUpvalue(u16),
    SetUpvalue(u16),

    GetGlobal(u16),
    SetGlobal(u16),
    /// Like GetGlobal but pushes `undefined` instead of throwing when the global
    /// is absent. Used for `typeof undeclaredName`.
    GetGlobalOptional(u16),
    DynamicImport,
    /// Build an `arguments` array from the current frame's call arguments.
    LoadArguments,

    Add,
    Sub,
    Mul,
    Div,
    Rem,
    Exp,
    Eq,
    StrictEq,
    Ne,
    StrictNe,
    Lt,
    Le,
    Gt,
    Ge,
    BitAnd,
    BitOr,
    BitXor,
    Shl,
    Shr,
    UShr,
    Neg,
    Not,
    BitNot,
    Typeof,
    Void,
    Delete,
    /// Delete an object property: pops [object, key], pushes a boolean result.
    DeleteProp,
    /// Define an accessor getter: pops [object, key, fn]; merges into any
    /// existing accessor descriptor. Leaves the stack otherwise unchanged.
    DefineGetter,
    /// Define an accessor setter: pops [object, key, fn].
    DefineSetter,
    In,
    Instanceof,
    ToNumber,
    JumpIfNullish(i32),

    Jump(i32),
    JumpIfTrue(i32),
    JumpIfFalse(i32),
    JumpIfTruePop(i32),
    JumpIfFalsePop(i32),

    Call(u8),
    CallSpread(u8),
    Return,
    /// Suspend the current generator: pop the yielded value, save the frame, and
    /// return control to the `.next()` caller. On resume the sent value is pushed.
    Yield,
    Await,
    AsyncReturn,
    MakeClosure(u16),

    MakeObject,
    MakeArray(u16),
    MakeRegExp(u16),
    GetProp,
    GetPropForCall(u16),
    SetProp,
    GetIndex,
    GetIndexForCall,
    SetIndex,
    CopyDataProperties,
    New(u8),
    Spread,
    GetForInKeys,
    GetForOfIterator,
    GetForAwaitIterator,
    ForOfNext,
    GetProto,
    SetProtoOf,
    SetObjectLiteralProto,
    GetSuperCtor,

    EnterTry(u16),
    LeaveTry,
    EndFinally,
    Throw,
    Nop,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Constant {
    Number(f64),
    String(String),
    RegExp { pattern: String, flags: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UpvalueDescriptor {
    pub is_local: bool,
    pub index: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExceptionHandler {
    pub try_start: u32,
    pub try_end: u32,
    pub catch_ip: u32,
    pub catch_binding: Option<u16>,
    pub finally_ip: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FunctionProto {
    pub name: Option<String>,
    pub arity: u8,
    pub parameter_count: u16,
    pub has_rest_param: bool,
    pub is_async: bool,
    pub is_generator: bool,
    pub code: Vec<Opcode>,
    pub constants: Vec<Constant>,
    pub upvalue_descriptors: Vec<UpvalueDescriptor>,
    pub nested_functions: Vec<FunctionProto>,
    pub handlers: Vec<ExceptionHandler>,
    pub local_count: u16,
    pub is_strict: bool,
    /// Whether the function body references the `arguments` object (so the VM
    /// retains the call arguments for it).
    pub uses_arguments: bool,
}

impl FunctionProto {
    #[must_use]
    pub fn new(name: Option<String>, arity: u8, is_strict: bool) -> Self {
        Self {
            name,
            arity,
            parameter_count: arity as u16,
            has_rest_param: false,
            is_async: false,
            is_generator: false,
            code: Vec::new(),
            constants: Vec::new(),
            upvalue_descriptors: Vec::new(),
            nested_functions: Vec::new(),
            handlers: Vec::new(),
            local_count: 0,
            is_strict,
            uses_arguments: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Chunk {
    pub top_level: FunctionProto,
}

impl Chunk {
    #[must_use]
    pub const fn new(top_level: FunctionProto) -> Self {
        Self { top_level }
    }
}
