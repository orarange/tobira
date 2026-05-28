#[derive(Debug, Clone, PartialEq)]
pub enum Opcode {
    LoadConst(u16),
    LoadUndefined,
    LoadNull,
    LoadTrue,
    LoadFalse,
    LoadThis,
    Pop,
    Dup,

    GetLocal(u16),
    SetLocal(u16),

    GetUpvalue(u16),
    SetUpvalue(u16),

    GetGlobal(u16),
    SetGlobal(u16),

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
    In,
    Instanceof,

    Jump(i32),
    JumpIfTrue(i32),
    JumpIfFalse(i32),
    JumpIfTruePop(i32),
    JumpIfFalsePop(i32),

    Call(u8),
    Return,
    MakeClosure(u16),

    MakeObject,
    MakeArray(u16),
    GetProp,
    GetPropForCall(u16),
    SetProp,
    GetIndex,
    GetIndexForCall,
    SetIndex,
    New(u8),

    Throw,
    Nop,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Constant {
    Number(f64),
    String(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UpvalueDescriptor {
    pub is_local: bool,
    pub index: u16,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FunctionProto {
    pub name: Option<String>,
    pub arity: u8,
    pub code: Vec<Opcode>,
    pub constants: Vec<Constant>,
    pub upvalue_descriptors: Vec<UpvalueDescriptor>,
    pub nested_functions: Vec<FunctionProto>,
    pub local_count: u16,
    pub is_strict: bool,
}

impl FunctionProto {
    #[must_use]
    pub fn new(name: Option<String>, arity: u8, is_strict: bool) -> Self {
        Self {
            name,
            arity,
            code: Vec::new(),
            constants: Vec::new(),
            upvalue_descriptors: Vec::new(),
            nested_functions: Vec::new(),
            local_count: 0,
            is_strict,
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
