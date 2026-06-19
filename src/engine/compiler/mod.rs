use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use boa_ast::declaration::ExportDeclaration as BoaExportDeclaration;

use super::ast::{
    ArithmeticOpNode, ArrayPatternElementNode, AssignOpNode, AssignTargetNode, BinaryOpNode,
    BindingNode, BitwiseOpNode, ClassDeclarationNode, ClassElementNameNode, ClassElementNode,
    ClassExpressionNode, ExpressionNode, ForLoopInitializerNode, FormalParameterListNode,
    FunctionBodyNode, FunctionDeclaration, IterableLoopInitializerNode, LiteralKindNode,
    LogicalOpNode, MethodDefinitionKindNode, ObjectMethodDefinitionNode, ObjectPatternElementNode,
    ObjectPropertyDefinition, OptionalOperationKindNode, Program, PropertyAccessFieldNode,
    PropertyNameNode, RelationalOpNode, StatementNode, SuperCallExpression,
    SuperPropertyAccessNode, TemplateElementNode, UnaryOpNode, UpdateOpNode, UpdateTargetNode,
    VariableDeclaration, SourceType, statement_list_item_to_node,
};
use super::chunk::{Chunk, Constant, ExceptionHandler, FunctionProto, Opcode, UpvalueDescriptor};
mod scope;
mod classes;
mod patterns;
mod expressions;
mod modules;
mod statements;
use patterns::PendingPatternInit;
use scope::{ImportBinding, OuterBindings, ResolvedBinding, ScopeFrame, UpvalueState};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompileError {
    Message(String),
    Unimplemented(&'static str),
}

impl CompileError {
    fn message(message: impl Into<String>) -> Self {
        Self::Message(message.into())
    }
}

impl std::fmt::Display for CompileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Message(message) => write!(f, "{message}"),
            Self::Unimplemented(feature) => write!(f, "unimplemented in phase 3: {feature}"),
        }
    }
}

impl std::error::Error for CompileError {}

#[derive(Debug, Clone, Default)]
struct ControlContext {
    break_jumps: Vec<usize>,
    continue_jumps: Vec<usize>,
    is_loop: bool,
    label: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DeclarationContext {
    Statement,
    ForInitializer,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PropertyOpKind {
    Named,
    Computed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BindingStorage {
    Var,
    Let,
    Const,
    Assignment,
}

#[derive(Debug, Clone, Default)]
struct FunctionCompileOptions {
    super_ctor_binding: Option<String>,
    super_proto_binding: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ModuleContext {
    pub self_key: String,
    pub imports: HashMap<String, String>,
    pub dynamic_imports: HashMap<String, String>,
}

pub struct Compiler<'a> {
    program: &'a Program,
    module_context: Option<ModuleContext>,
}

impl<'a> Compiler<'a> {
    #[must_use]
    pub const fn new(program: &'a Program) -> Self {
        Self {
            program,
            module_context: None,
        }
    }

    #[must_use]
    pub fn with_module_context(mut self, module_context: ModuleContext) -> Self {
        self.module_context = Some(module_context);
        self
    }

    pub fn compile(&self) -> Result<Chunk, CompileError> {
        let module_context = if self.program.source_type() == SourceType::Module {
            self.module_context.clone().or_else(|| {
                Some(ModuleContext {
                    self_key: "\u{0}module:test".to_string(),
                    imports: HashMap::new(),
                    dynamic_imports: HashMap::new(),
                })
            })
        } else {
            self.module_context.clone()
        };
        let mut function = FunctionCompiler::new(
            self.program,
            None,
            0,
            self.program.strict(),
            true,
            None,
            FunctionCompileOptions::default(),
            module_context,
            HashMap::new(),
        );
        function.install_this_binding()?;
        function.compile_statements(self.program.body())?;
        function.emit_implicit_return();
        Ok(Chunk::new(function.finish()))
    }
}

pub fn compile(program: &Program) -> Result<Chunk, CompileError> {
    Compiler::new(program).compile()
}

struct FunctionCompiler<'a> {
    program: &'a Program,
    name: Option<String>,
    arity: u8,
    is_strict: bool,
    is_top_level: bool,
    code: Vec<Opcode>,
    constants: Vec<Constant>,
    upvalues: Rc<RefCell<UpvalueState>>,
    nested_functions: Vec<FunctionProto>,
    handlers: Vec<ExceptionHandler>,
    scopes: Vec<ScopeFrame>,
    next_local: u16,
    parameter_count: u16,
    has_rest_param: bool,
    is_async: bool,
    is_generator: bool,
    outer: Option<OuterBindings>,
    control_stack: Vec<ControlContext>,
    active_finally_blocks: Vec<super::ast::BlockStatement>,
    /// Label attached to the next loop to be compiled (from a labeled statement).
    pending_label: Option<String>,
    /// Whether this is an arrow function (no own `this`/`arguments`).
    is_arrow: bool,
    /// Set when the body references the `arguments` object.
    uses_arguments: bool,
    options: FunctionCompileOptions,
    module_context: Option<ModuleContext>,
    import_bindings: HashMap<String, ImportBinding>,
}

impl<'a> FunctionCompiler<'a> {
    fn new(
        program: &'a Program,
        name: Option<String>,
        arity: u8,
        is_strict: bool,
        is_top_level: bool,
        outer: Option<OuterBindings>,
        options: FunctionCompileOptions,
        module_context: Option<ModuleContext>,
        import_bindings: HashMap<String, ImportBinding>,
    ) -> Self {
        Self {
            program,
            name,
            arity,
            is_strict,
            is_top_level,
            code: Vec::new(),
            constants: Vec::new(),
            upvalues: Rc::new(RefCell::new(UpvalueState::default())),
            nested_functions: Vec::new(),
            handlers: Vec::new(),
            scopes: vec![ScopeFrame::default()],
            next_local: 0,
            parameter_count: 0,
            has_rest_param: false,
            is_async: false,
            is_generator: false,
            outer,
            control_stack: Vec::new(),
            active_finally_blocks: Vec::new(),
            pending_label: None,
            is_arrow: false,
            uses_arguments: false,
            options,
            module_context,
            import_bindings,
        }
    }

    fn finish(self) -> FunctionProto {
        FunctionProto {
            name: self.name,
            arity: self.arity,
            parameter_count: self.parameter_count,
            has_rest_param: self.has_rest_param,
            is_async: self.is_async,
            is_generator: self.is_generator,
            code: self.code,
            constants: self.constants,
            upvalue_descriptors: self.upvalues.borrow().descriptors.clone(),
            nested_functions: self.nested_functions,
            handlers: self.handlers,
            local_count: self.next_local,
            is_strict: self.is_strict,
            uses_arguments: self.uses_arguments,
        }
    }

    fn emit_implicit_return(&mut self) {
        self.emit(Opcode::LoadUndefined);
        self.emit(if self.is_async {
            Opcode::AsyncReturn
        } else {
            Opcode::Return
        });
    }

    fn emit(&mut self, opcode: Opcode) -> usize {
        let index = self.code.len();
        self.code.push(opcode);
        index
    }

    fn emit_jump(&mut self, opcode: Opcode) -> usize {
        self.emit(opcode)
    }

    fn patch_jump(&mut self, jump_index: usize, target_index: usize) -> Result<(), CompileError> {
        let offset = target_index as i64 - (jump_index as i64 + 1);
        let offset = i32::try_from(offset)
            .map_err(|_| CompileError::message("jump offset overflowed i32"))?;

        match self.code.get_mut(jump_index) {
            Some(Opcode::Jump(slot))
            | Some(Opcode::JumpIfTrue(slot))
            | Some(Opcode::JumpIfFalse(slot))
            | Some(Opcode::JumpIfTruePop(slot))
            | Some(Opcode::JumpIfFalsePop(slot))
            | Some(Opcode::JumpIfNullish(slot)) => {
                *slot = offset;
                Ok(())
            }
            _ => Err(CompileError::message("patch target was not a jump")),
        }
    }

    fn emit_back_jump(&mut self, target_index: usize) -> Result<(), CompileError> {
        let jump_index = self.code.len();
        let offset = target_index as i64 - (jump_index as i64 + 1);
        let offset = i32::try_from(offset)
            .map_err(|_| CompileError::message("back jump offset overflowed i32"))?;
        self.emit(Opcode::Jump(offset));
        Ok(())
    }

    fn add_constant(&mut self, constant: Constant) -> Result<u16, CompileError> {
        if let Some(index) = self.constants.iter().position(|entry| entry == &constant) {
            return u16::try_from(index)
                .map_err(|_| CompileError::message("constant table exceeded u16"));
        }

        let index = self.constants.len();
        self.constants.push(constant);
        u16::try_from(index).map_err(|_| CompileError::message("constant table exceeded u16"))
    }

    fn add_number_constant(&mut self, value: f64) -> Result<u16, CompileError> {
        self.add_constant(Constant::Number(value))
    }

    fn add_string_constant(&mut self, value: impl Into<String>) -> Result<u16, CompileError> {
        self.add_constant(Constant::String(value.into()))
    }

    fn identifier_name(&self, identifier: &super::ast::IdentifierNode) -> String {
        self.program.resolve_sym(identifier.sym())
    }

    fn binding_name(&self, binding: &BindingNode) -> Result<String, CompileError> {
        match binding {
            BindingNode::Identifier(identifier) => Ok(self.identifier_name(identifier)),
            BindingNode::Pattern(_) => Err(CompileError::Unimplemented(
                "destructuring bindings and patterns",
            )),
        }
    }

    fn compile_nested_function(
        &mut self,
        name: Option<String>,
        parameters: &FormalParameterListNode,
        body: &FunctionBodyNode,
        is_strict: bool,
        is_async: bool,
        is_generator: bool,
        is_arrow: bool,
    ) -> Result<u16, CompileError> {
        self.compile_nested_function_with_options(
            name,
            parameters,
            body,
            is_strict,
            is_async,
            is_generator,
            is_arrow,
            if is_arrow {
                self.options.clone()
            } else {
                FunctionCompileOptions::default()
            },
            &[],
            &[],
        )
    }

    fn compile_nested_function_with_options(
        &mut self,
        name: Option<String>,
        parameters: &FormalParameterListNode,
        body: &FunctionBodyNode,
        is_strict: bool,
        is_async: bool,
        is_generator: bool,
        is_arrow: bool,
        options: FunctionCompileOptions,
        field_initializers: &[&super::ast::ClassFieldDefinitionNode],
        private_field_initializers: &[&boa_ast::function::PrivateFieldDefinition],
    ) -> Result<u16, CompileError> {
        let arity = u8::try_from(parameters.length())
            .map_err(|_| CompileError::message("function arity exceeded u8"))?;
        let outer = Some(self.snapshot_outer_bindings());
        let mut child = FunctionCompiler::new(
            self.program,
            name,
            arity,
            is_strict,
            false,
            outer,
            options,
            None,
            self.import_bindings.clone(),
        );
        child.is_async = is_async;
        child.is_generator = is_generator;
        child.is_arrow = is_arrow;
        child.compile_function_parameters(parameters)?;
        if !is_arrow {
            child.install_this_binding()?;
        }
        for field in field_initializers {
            child.compile_class_field_initializer(field)?;
        }
        for field in private_field_initializers {
            child.compile_private_field_initializer(field)?;
        }
        child.compile_function_body(body)?;
        child.emit_implicit_return();

        let index = self.nested_functions.len();
        self.nested_functions.push(child.finish());
        u16::try_from(index)
            .map_err(|_| CompileError::message("nested function count exceeded u16"))
    }

}

