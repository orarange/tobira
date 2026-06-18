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

    fn compile_expression(&mut self, expression: &ExpressionNode) -> Result<(), CompileError> {
        match expression {
            ExpressionNode::This(_) => {
                match self.resolve_binding("this") {
                    ResolvedBinding::Local(slot) => self.emit(Opcode::GetLocal(slot)),
                    ResolvedBinding::Upvalue(slot) => self.emit(Opcode::GetUpvalue(slot)),
                    ResolvedBinding::ModuleImport | ResolvedBinding::Global => {
                        self.emit(Opcode::LoadThis)
                    }
                };
                Ok(())
            }
            ExpressionNode::Identifier(identifier) => {
                let name = self.identifier_name(identifier);
                let resolved = self.resolve_binding(&name);
                // `arguments` in a non-arrow function builds the arguments array.
                if name == "arguments"
                    && matches!(resolved, ResolvedBinding::Global)
                    && !self.is_arrow
                    && !self.is_top_level
                {
                    self.uses_arguments = true;
                    self.emit(Opcode::LoadArguments);
                    return Ok(());
                }
                self.emit_load_binding(&name, resolved)
            }
            ExpressionNode::Literal(literal) => self.compile_literal(literal.kind()),
            ExpressionNode::RegExpLiteral(regexp) => self.compile_regexp_literal(regexp),
            ExpressionNode::ArrayLiteral(array) => self.compile_array_literal(array),
            ExpressionNode::ObjectLiteral(object) => self.compile_object_literal(object),
            ExpressionNode::Spread(_) => Err(CompileError::Unimplemented("spread expressions")),
            ExpressionNode::FunctionExpression(function) => self.compile_nested_function_value(
                function
                    .name()
                    .map(|identifier| self.identifier_name(&identifier)),
                function.parameters(),
                function.body(),
                function.body().strict(),
                false,
                false,
                false,
            ),
            ExpressionNode::ArrowFunction(function) => self.compile_nested_function_value(
                function
                    .name()
                    .map(|identifier| self.identifier_name(&identifier)),
                function.parameters(),
                function.body(),
                function.body().strict(),
                false,
                false,
                true,
            ),
            ExpressionNode::AsyncArrowFunction(function) => self.compile_nested_function_value(
                function
                    .name()
                    .map(|identifier| self.identifier_name(&identifier)),
                function.parameters(),
                function.body(),
                function.body().strict(),
                true,
                false,
                true,
            ),
            ExpressionNode::GeneratorExpression(function) => self.compile_nested_function_value(
                function
                    .name()
                    .map(|identifier| self.identifier_name(&identifier)),
                function.parameters(),
                function.body(),
                function.body().strict(),
                false,
                true,
                false,
            ),
            ExpressionNode::AsyncFunctionExpression(function) => self
                .compile_nested_function_value(
                    function
                        .name()
                        .map(|identifier| self.identifier_name(&identifier)),
                    function.parameters(),
                    function.body(),
                    function.body().strict(),
                    true,
                    false,
                    false,
                ),
            ExpressionNode::AsyncGeneratorExpression(function) => {
                self.compile_nested_function_value(
                    function
                        .name()
                        .map(|identifier| self.identifier_name(&identifier)),
                    function.parameters(),
                    function.body(),
                    function.body().strict(),
                    true,
                    true,
                    false,
                )
            }
            ExpressionNode::ClassExpression(class_expression) => {
                self.compile_class_expression(class_expression)
            }
            ExpressionNode::TemplateLiteral(template) => self.compile_template_literal(template),
            ExpressionNode::PropertyAccess(access) => {
                self.compile_property_access_expression(access)
            }
            ExpressionNode::New(new_expression) => self.compile_new_expression(new_expression),
            ExpressionNode::Call(call) => self.compile_call_expression(call),
            ExpressionNode::SuperCall(call) => self.compile_super_call(call),
            ExpressionNode::ImportCall(_) => Err(CompileError::Unimplemented("import() calls")),
            ExpressionNode::Optional(optional) => self.compile_optional_expression(optional),
            ExpressionNode::TaggedTemplate(template) => self.compile_tagged_template(template),
            ExpressionNode::NewTarget(_) => {
                self.emit(Opcode::LoadNewTarget);
                Ok(())
            }
            ExpressionNode::ImportMeta(_) => Err(CompileError::Unimplemented("import.meta")),
            ExpressionNode::Assign(assign) => self.compile_assignment_expression(assign),
            ExpressionNode::Unary(unary) => self.compile_unary_expression(unary),
            ExpressionNode::Update(update) => self.compile_update_expression(update),
            ExpressionNode::Binary(binary) => self.compile_binary_expression(binary),
            ExpressionNode::BinaryInPrivate(_) => {
                Err(CompileError::Unimplemented("private in expressions"))
            }
            ExpressionNode::Conditional(conditional) => {
                self.compile_conditional_expression(conditional)
            }
            ExpressionNode::Await(await_expression) => {
                if !self.is_async {
                    return Err(CompileError::message(
                        "await expressions are only valid inside async functions",
                    ));
                }
                self.compile_expression(await_expression.target())?;
                self.emit(Opcode::Await);
                Ok(())
            }
            ExpressionNode::Yield(yield_expression) => self.compile_yield(yield_expression),
            ExpressionNode::Parenthesized(expression) => {
                self.compile_expression(expression.expression())
            }
            ExpressionNode::FormalParameterList(_) | ExpressionNode::Debugger => {
                Err(CompileError::message("invalid expression node"))
            }
        }
    }

    fn compile_array_literal(
        &mut self,
        array: &super::ast::ArrayExpression,
    ) -> Result<(), CompileError> {
        if array
            .as_ref()
            .iter()
            .flatten()
            .any(|expression| matches!(expression, ExpressionNode::Spread(_)))
        {
            return self.compile_array_literal_with_spread(array);
        }
        let mut count = 0usize;
        for element in array.as_ref() {
            match element {
                Some(expression) => self.compile_expression(expression)?,
                None => {
                    self.emit(Opcode::LoadUndefined);
                }
            }
            count += 1;
        }
        let count = u16::try_from(count)
            .map_err(|_| CompileError::message("array literal length exceeded u16"))?;
        self.emit(Opcode::MakeArray(count));
        Ok(())
    }

    fn compile_object_literal(
        &mut self,
        object: &super::ast::ObjectExpression,
    ) -> Result<(), CompileError> {
        self.emit(Opcode::MakeObject);
        for property in object.properties() {
            self.emit(Opcode::Dup);
            match property {
                ObjectPropertyDefinition::IdentifierReference(identifier) => {
                    let name = self.identifier_name(identifier);
                    let constant = self.add_string_constant(name.clone())?;
                    self.emit(Opcode::LoadConst(constant));
                    let resolved = self.resolve_binding(&name);
                    self.emit_load_binding(&name, resolved)?;
                    self.emit(Opcode::SetProp);
                }
                ObjectPropertyDefinition::Property(name, value) => {
                    match name {
                        PropertyNameNode::Literal(identifier)
                            if self.identifier_name(identifier) == "__proto__" =>
                        {
                            self.compile_expression(value)?;
                            self.emit(Opcode::SetObjectLiteralProto);
                        }
                        _ => {
                            self.compile_property_name_value(name)?;
                            self.compile_expression(value)?;
                            self.emit(Opcode::SetProp);
                        }
                    }
                }
                ObjectPropertyDefinition::MethodDefinition(method) => {
                    match method.kind() {
                        MethodDefinitionKindNode::Get => {
                            self.compile_property_name_value(method.name())?;
                            self.compile_object_accessor_value(method)?;
                            self.emit(Opcode::DefineGetter);
                        }
                        MethodDefinitionKindNode::Set => {
                            self.compile_property_name_value(method.name())?;
                            self.compile_object_accessor_value(method)?;
                            self.emit(Opcode::DefineSetter);
                        }
                        _ => {
                            self.compile_property_name_value(method.name())?;
                            self.compile_object_method_value(method)?;
                            self.emit(Opcode::SetProp);
                        }
                    }
                }
                ObjectPropertyDefinition::SpreadObject(expression) => {
                    self.compile_expression(expression)?;
                    self.emit(Opcode::CopyDataProperties);
                    // Each property arm is entered with a `Dup` of the object on
                    // the stack and is expected to consume it. `SetProp` does, but
                    // `CopyDataProperties` pops the target and pushes it back, so
                    // the dup survives — drop it, or it leaks and corrupts the
                    // stack for later properties (esp. nested object spreads).
                    self.emit(Opcode::Pop);
                }
                ObjectPropertyDefinition::CoverInitializedName(identifier, expression) => {
                    let constant = self.add_string_constant(self.identifier_name(identifier))?;
                    self.emit(Opcode::LoadConst(constant));
                    self.compile_expression(expression)?;
                    self.emit(Opcode::SetProp);
                }
            }
        }
        Ok(())
    }

    fn compile_property_name_value(&mut self, name: &PropertyNameNode) -> Result<(), CompileError> {
        match name {
            PropertyNameNode::Literal(identifier) => {
                let constant = self.add_string_constant(self.identifier_name(identifier))?;
                self.emit(Opcode::LoadConst(constant));
            }
            PropertyNameNode::Computed(expression) => {
                self.compile_expression(expression)?;
            }
        }
        Ok(())
    }

    fn property_name_string(&self, name: &PropertyNameNode) -> Option<String> {
        match name {
            PropertyNameNode::Literal(identifier) => Some(self.identifier_name(identifier)),
            PropertyNameNode::Computed(ExpressionNode::Literal(literal)) => match literal.kind() {
                LiteralKindNode::String(sym) => Some(self.program.resolve_sym(*sym)),
                LiteralKindNode::Int(value) if *value >= 0 => Some(value.to_string()),
                LiteralKindNode::Num(value)
                    if value.is_finite() && *value >= 0.0 && value.fract() == 0.0 =>
                {
                    Some((*value as u64).to_string())
                }
                _ => None,
            },
            _ => None,
        }
    }

    fn compile_object_method_value(
        &mut self,
        method: &ObjectMethodDefinitionNode,
    ) -> Result<(), CompileError> {
        let (is_async, is_generator) = match method.kind() {
            MethodDefinitionKindNode::Ordinary => (false, false),
            MethodDefinitionKindNode::Generator => (false, true),
            MethodDefinitionKindNode::Get | MethodDefinitionKindNode::Set => {
                return Err(CompileError::Unimplemented("object literal accessors"));
            }
            MethodDefinitionKindNode::Async => (true, false),
            MethodDefinitionKindNode::AsyncGenerator => (true, true),
        };

        self.compile_nested_function_value(
            self.property_name_string(method.name()),
            method.parameters(),
            method.body(),
            method.body().strict(),
            is_async,
            is_generator,
            false,
        )
    }

    /// Compile a getter/setter function for an object-literal accessor. Unlike
    /// `compile_object_method_value`, it does not reject the Get/Set kinds.
    fn compile_object_accessor_value(
        &mut self,
        method: &ObjectMethodDefinitionNode,
    ) -> Result<(), CompileError> {
        self.compile_nested_function_value(
            self.property_name_string(method.name()),
            method.parameters(),
            method.body(),
            method.body().strict(),
            false,
            false,
            false,
        )
    }

    fn compile_property_access_expression(
        &mut self,
        access: &super::ast::MemberExpression,
    ) -> Result<(), CompileError> {
        match access {
            super::ast::MemberExpression::Simple(access) => {
                self.compile_expression(access.target())?;
                match access.field() {
                    PropertyAccessFieldNode::Const(identifier) => {
                        let constant =
                            self.add_string_constant(self.identifier_name(identifier))?;
                        self.emit(Opcode::LoadConst(constant));
                        self.emit(Opcode::GetProp);
                    }
                    PropertyAccessFieldNode::Expr(expression) => {
                        self.compile_expression(expression)?;
                        self.emit(Opcode::GetIndex);
                    }
                }
                Ok(())
            }
            super::ast::MemberExpression::Private(access) => {
                self.compile_expression(access.target())?;
                let constant = self.add_string_constant(self.private_field_key(&access.field()))?;
                self.emit(Opcode::LoadConst(constant));
                self.emit(Opcode::GetProp);
                Ok(())
            }
            super::ast::MemberExpression::Super(access) => {
                self.compile_super_property_access(access)
            }
        }
    }

    /// Mangle a private name `#x` into the property key string used to store it.
    fn private_field_key(&self, name: &boa_ast::function::PrivateName) -> String {
        format!("#{}", self.program.resolve_sym(name.description()))
    }

    fn compile_literal(&mut self, literal: &LiteralKindNode) -> Result<(), CompileError> {
        match literal {
            LiteralKindNode::String(sym) => {
                let index = self.add_string_constant(self.program.resolve_sym(*sym))?;
                self.emit(Opcode::LoadConst(index));
            }
            LiteralKindNode::Num(number) => {
                let index = self.add_number_constant(*number)?;
                self.emit(Opcode::LoadConst(index));
            }
            LiteralKindNode::Int(number) => {
                let index = self.add_number_constant(f64::from(*number))?;
                self.emit(Opcode::LoadConst(index));
            }
            LiteralKindNode::BigInt(_) => {
                return Err(CompileError::Unimplemented("bigint literals"));
            }
            LiteralKindNode::Bool(true) => {
                self.emit(Opcode::LoadTrue);
            }
            LiteralKindNode::Bool(false) => {
                self.emit(Opcode::LoadFalse);
            }
            LiteralKindNode::Null => {
                self.emit(Opcode::LoadNull);
            }
            LiteralKindNode::Undefined => {
                self.emit(Opcode::LoadUndefined);
            }
        }
        Ok(())
    }

    fn compile_yield(
        &mut self,
        yield_expression: &boa_ast::expression::Yield,
    ) -> Result<(), CompileError> {
        if yield_expression.delegate() {
            return self.compile_yield_delegate(yield_expression);
        }
        match yield_expression.target() {
            Some(target) => self.compile_expression(target)?,
            None => {
                self.emit(Opcode::LoadUndefined);
            }
        };
        if self.is_async && self.is_generator {
            self.emit(Opcode::Await);
        }
        self.emit(Opcode::Yield);
        Ok(())
    }

    /// `yield* iterable`: drive the iterable, yielding each value in turn.
    fn compile_yield_delegate(
        &mut self,
        yield_expression: &boa_ast::expression::Yield,
    ) -> Result<(), CompileError> {
        let target = yield_expression
            .target()
            .ok_or_else(|| CompileError::message("yield* requires an operand"))?;
        let iter_slot = self.allocate_hidden_local()?;
        self.compile_expression(target)?;
        self.emit(Opcode::GetForOfIterator);
        self.emit(Opcode::SetLocal(iter_slot));
        let loop_start = self.code.len();
        self.emit(Opcode::GetLocal(iter_slot));
        self.emit(Opcode::ForOfNext); // -> [value, done]
        let exit = self.emit_jump(Opcode::JumpIfTruePop(0)); // pop done; if done jump out
        if self.is_async && self.is_generator {
            self.emit(Opcode::Await);
        }
        self.emit(Opcode::Yield); // yields value; on resume leaves the sent value
        self.emit(Opcode::Pop); // discard the sent value
        self.emit_back_jump(loop_start)?;
        let end = self.code.len();
        self.patch_jump(exit, end)?;
        self.emit(Opcode::Pop); // drop the leftover (undefined) value
        self.emit(Opcode::LoadUndefined); // value of the yield* expression
        Ok(())
    }

    fn compile_tagged_template(
        &mut self,
        template: &super::ast::TaggedTemplateExpression,
    ) -> Result<(), CompileError> {
        // tag(strings, ...substitutions) where `strings` is the cooked array
        // carrying a `.raw` array of the raw segments.
        self.compile_expression(template.tag())?;
        self.emit(Opcode::LoadUndefined); // `this` for the tag call

        // Build the cooked strings array (None = invalid escape → undefined).
        let cookeds = template.cookeds();
        for cooked in cookeds {
            match cooked {
                Some(sym) => {
                    let index = self.add_string_constant(self.program.resolve_sym(*sym))?;
                    self.emit(Opcode::LoadConst(index));
                }
                None => {
                    self.emit(Opcode::LoadUndefined);
                }
            }
        }
        let cooked_count = u16::try_from(cookeds.len())
            .map_err(|_| CompileError::message("template segment count exceeded u16"))?;
        self.emit(Opcode::MakeArray(cooked_count));

        // strings.raw = [ ...raw segments ]
        self.emit(Opcode::Dup);
        let raw_key = self.add_string_constant("raw")?;
        self.emit(Opcode::LoadConst(raw_key));
        let raws = template.raws();
        for raw in raws {
            let index = self.add_string_constant(self.program.resolve_sym(*raw))?;
            self.emit(Opcode::LoadConst(index));
        }
        let raw_count = u16::try_from(raws.len())
            .map_err(|_| CompileError::message("template segment count exceeded u16"))?;
        self.emit(Opcode::MakeArray(raw_count));
        self.emit(Opcode::SetProp);

        // Substitution expressions.
        let exprs = template.exprs();
        for expr in exprs {
            self.compile_expression(expr)?;
        }
        let argc = u8::try_from(1 + exprs.len())
            .map_err(|_| CompileError::message("tagged template argument count exceeded u8"))?;
        self.emit(Opcode::Call(argc));
        Ok(())
    }

    fn compile_template_literal(
        &mut self,
        template: &super::ast::TemplateLiteralExpression,
    ) -> Result<(), CompileError> {
        let mut first = true;
        for element in template.elements() {
            match element {
                TemplateElementNode::String(sym) => {
                    let index = self.add_string_constant(self.program.resolve_sym(*sym))?;
                    self.emit(Opcode::LoadConst(index));
                }
                TemplateElementNode::Expr(expression) => {
                    self.compile_expression(expression)?;
                }
            }

            if first {
                first = false;
            } else {
                self.emit(Opcode::Add);
            }
        }

        if first {
            let index = self.add_string_constant(String::new())?;
            self.emit(Opcode::LoadConst(index));
        }

        Ok(())
    }

    fn compile_nested_function_value(
        &mut self,
        name: Option<String>,
        parameters: &FormalParameterListNode,
        body: &FunctionBodyNode,
        is_strict: bool,
        is_async: bool,
        is_generator: bool,
        is_arrow: bool,
    ) -> Result<(), CompileError> {
        // Named function EXPRESSIONS bind their own name inside the function body
        // (and only there) to the function itself, so recursive self-reference
        // works: `var f = function I(n){ return n<=1?1:n*I(n-1); }`. Minifiers
        // (Terser) rely on this heavily — every self-referential helper becomes a
        // short-named function expression like `function I(){…I…}`. We desugar it
        // the spec way: introduce a fresh block scope in THIS (outer) compiler
        // holding the name, compile the body so it captures that binding as an
        // upvalue, then store the freshly made closure into the binding. Arrows
        // have no such self-binding, and declarations bind in the enclosing scope
        // already, so this only applies to non-arrow expressions with a name.
        let self_binding_slot = match (&name, is_arrow) {
            (Some(fn_name), false) => {
                self.push_scope();
                Some((self.declare_block_scoped(fn_name)?, ()))
            }
            _ => None,
        };

        let index = self.compile_nested_function(
            name,
            parameters,
            body,
            is_strict,
            is_async,
            is_generator,
            is_arrow,
        )?;
        self.emit(Opcode::MakeClosure(index));

        if let Some((slot, ())) = self_binding_slot {
            // Closure is on the stack; copy it into the self-binding cell that the
            // body captured as an upvalue, leaving the closure as the expression's
            // value. Then drop the scope so the name isn't visible to outer code.
            self.emit(Opcode::Dup);
            self.emit(Opcode::SetLocal(slot));
            self.pop_scope();
        }
        Ok(())
    }

    fn compile_call_expression(
        &mut self,
        call: &super::ast::CallExpression,
    ) -> Result<(), CompileError> {
        if call
            .args()
            .iter()
            .any(|argument| matches!(argument, ExpressionNode::Spread(_)))
        {
            return self.compile_call_expression_with_spread(call);
        }
        match call.function() {
            ExpressionNode::PropertyAccess(access) => {
                self.compile_property_access_for_call(access)?
            }
            ExpressionNode::Optional(optional) => {
                return self.compile_optional_call(optional, call);
            }
            other => {
                self.compile_expression(other)?;
                self.emit(Opcode::LoadUndefined);
            }
        }
        let argc = u8::try_from(call.args().len())
            .map_err(|_| CompileError::message("call argument count exceeded u8"))?;
        for argument in call.args() {
            self.compile_expression(argument)?;
        }
        self.emit(Opcode::Call(argc));
        Ok(())
    }

    fn compile_property_access_for_call(
        &mut self,
        access: &super::ast::MemberExpression,
    ) -> Result<(), CompileError> {
        match access {
            super::ast::MemberExpression::Simple(access) => {
                self.compile_expression(access.target())?;
                match access.field() {
                    PropertyAccessFieldNode::Const(identifier) => {
                        let constant =
                            self.add_string_constant(self.identifier_name(identifier))?;
                        self.emit(Opcode::GetPropForCall(constant));
                    }
                    PropertyAccessFieldNode::Expr(expression) => {
                        self.compile_expression(expression)?;
                        self.emit(Opcode::GetIndexForCall);
                    }
                }
                Ok(())
            }
            super::ast::MemberExpression::Private(access) => {
                self.compile_expression(access.target())?;
                let constant = self.add_string_constant(self.private_field_key(&access.field()))?;
                self.emit(Opcode::GetPropForCall(constant));
                Ok(())
            }
            super::ast::MemberExpression::Super(access) => {
                self.compile_super_property_for_call(access)
            }
        }
    }

    fn compile_new_expression(
        &mut self,
        new_expression: &super::ast::NewExpression,
    ) -> Result<(), CompileError> {
        if new_expression
            .arguments()
            .iter()
            .any(|argument| matches!(argument, ExpressionNode::Spread(_)))
        {
            return self.compile_new_expression_with_spread(new_expression);
        }
        self.compile_expression(new_expression.constructor())?;
        let argc = u8::try_from(new_expression.arguments().len())
            .map_err(|_| CompileError::message("new expression arity exceeded u8"))?;
        for argument in new_expression.arguments() {
            self.compile_expression(argument)?;
        }
        self.emit(Opcode::New(argc));
        Ok(())
    }

    fn compile_assignment_expression(
        &mut self,
        assign: &super::ast::AssignmentExpression,
    ) -> Result<(), CompileError> {
        if matches!(
            assign.op(),
            AssignOpNode::BoolAnd | AssignOpNode::BoolOr | AssignOpNode::Coalesce
        ) {
            return self.compile_logical_assignment_expression(assign);
        }

        let (name, resolved) = match assign.lhs() {
            AssignTargetNode::Identifier(identifier) => {
                let name = self.identifier_name(identifier);
                let resolved = self.resolve_binding(&name);
                (name, resolved)
            }
            AssignTargetNode::Access(access) => {
                return self.compile_property_assignment(access, assign.op(), assign.rhs());
            }
            AssignTargetNode::Pattern(pattern) => {
                return self.compile_pattern_assignment_expression(
                    pattern,
                    assign.op(),
                    assign.rhs(),
                );
            }
        };

        match assign.op() {
            AssignOpNode::Assign => {
                self.compile_expression(assign.rhs())?;
                self.emit(Opcode::Dup);
                self.emit_store_binding(&name, resolved)?;
            }
            operator => {
                self.emit_load_binding(&name, resolved)?;
                self.compile_expression(assign.rhs())?;
                self.emit_assignment_operator(operator)?;
                self.emit(Opcode::Dup);
                self.emit_store_binding(&name, resolved)?;
            }
        }

        Ok(())
    }

    fn emit_assignment_operator(&mut self, operator: AssignOpNode) -> Result<(), CompileError> {
        match operator {
            AssignOpNode::Add => {
                self.emit(Opcode::Add);
            }
            AssignOpNode::Sub => {
                self.emit(Opcode::Sub);
            }
            AssignOpNode::Mul => {
                self.emit(Opcode::Mul);
            }
            AssignOpNode::Div => {
                self.emit(Opcode::Div);
            }
            AssignOpNode::Mod => {
                self.emit(Opcode::Rem);
            }
            AssignOpNode::Exp => {
                self.emit(Opcode::Exp);
            }
            AssignOpNode::And => {
                self.emit(Opcode::BitAnd);
            }
            AssignOpNode::Or => {
                self.emit(Opcode::BitOr);
            }
            AssignOpNode::Xor => {
                self.emit(Opcode::BitXor);
            }
            AssignOpNode::Shl => {
                self.emit(Opcode::Shl);
            }
            AssignOpNode::Shr => {
                self.emit(Opcode::Shr);
            }
            AssignOpNode::Ushr => {
                self.emit(Opcode::UShr);
            }
            AssignOpNode::Assign => {}
            AssignOpNode::BoolAnd | AssignOpNode::BoolOr | AssignOpNode::Coalesce => {
                return Ok(());
            }
        }
        Ok(())
    }

    fn compile_property_assignment(
        &mut self,
        access: &super::ast::MemberExpression,
        operator: AssignOpNode,
        rhs: &ExpressionNode,
    ) -> Result<(), CompileError> {
        let obj_temp = self.allocate_hidden_local()?;
        let key_temp = self.allocate_hidden_local()?;
        let value_temp = self.allocate_hidden_local()?;
        let kind = self.compile_property_access_temps(access, obj_temp, key_temp)?;

        if operator == AssignOpNode::Assign {
            self.compile_expression(rhs)?;
            self.emit(Opcode::Dup);
            self.emit(Opcode::SetLocal(value_temp));
            self.emit(Opcode::GetLocal(obj_temp));
            self.emit(Opcode::GetLocal(key_temp));
            self.emit(Opcode::GetLocal(value_temp));
            self.emit_property_set(kind);
            return Ok(());
        }

        self.emit(Opcode::GetLocal(obj_temp));
        self.emit(Opcode::GetLocal(key_temp));
        self.emit_property_get(kind);
        self.compile_expression(rhs)?;
        self.emit_assignment_operator(operator)?;
        self.emit(Opcode::Dup);
        self.emit(Opcode::SetLocal(value_temp));
        self.emit(Opcode::GetLocal(obj_temp));
        self.emit(Opcode::GetLocal(key_temp));
        self.emit(Opcode::GetLocal(value_temp));
        self.emit_property_set(kind);
        Ok(())
    }

    fn compile_unary_expression(
        &mut self,
        unary: &super::ast::UnaryExpression,
    ) -> Result<(), CompileError> {
        // `delete` must NOT evaluate its operand as a value when it is a property
        // reference — it needs the object/key in order to remove the property.
        if matches!(unary.op(), UnaryOpNode::Delete) {
            return self.compile_delete_expression(unary.target());
        }
        // `typeof undeclaredName` must yield "undefined" rather than throwing a
        // ReferenceError (common feature-detection idiom).
        if matches!(unary.op(), UnaryOpNode::TypeOf) {
            if let ExpressionNode::Identifier(identifier) = unary.target() {
                let name = self.identifier_name(identifier);
                let resolved = self.resolve_binding(&name);
                if matches!(resolved, ResolvedBinding::Global) && name != "arguments" {
                    let index = self.add_string_constant(name)?;
                    self.emit(Opcode::GetGlobalOptional(index));
                    self.emit(Opcode::Typeof);
                    return Ok(());
                }
            }
        }
        self.compile_expression(unary.target())?;
        match unary.op() {
            UnaryOpNode::Minus => {
                self.emit(Opcode::Neg);
            }
            UnaryOpNode::Not => {
                self.emit(Opcode::Not);
            }
            UnaryOpNode::Tilde => {
                self.emit(Opcode::BitNot);
            }
            UnaryOpNode::TypeOf => {
                self.emit(Opcode::Typeof);
            }
            UnaryOpNode::Void => {
                self.emit(Opcode::Void);
            }
            UnaryOpNode::Delete => unreachable!("delete handled above"),
            UnaryOpNode::Plus => {
                self.emit(Opcode::ToNumber);
            }
        }
        Ok(())
    }

    fn compile_delete_expression(
        &mut self,
        target: &ExpressionNode,
    ) -> Result<(), CompileError> {
        if let ExpressionNode::PropertyAccess(super::ast::MemberExpression::Simple(access)) = target
        {
            self.compile_expression(access.target())?;
            match access.field() {
                PropertyAccessFieldNode::Const(identifier) => {
                    let constant = self.add_string_constant(self.identifier_name(identifier))?;
                    self.emit(Opcode::LoadConst(constant));
                }
                PropertyAccessFieldNode::Expr(expression) => {
                    self.compile_expression(expression)?;
                }
            }
            self.emit(Opcode::DeleteProp);
            Ok(())
        } else {
            // `delete <non-reference>`: evaluate for side effects, discard, yield true.
            self.compile_expression(target)?;
            self.emit(Opcode::Delete);
            Ok(())
        }
    }

    fn compile_update_expression(
        &mut self,
        update: &super::ast::UpdateExpression,
    ) -> Result<(), CompileError> {
        let (name, resolved) = match update.target() {
            UpdateTargetNode::Identifier(identifier) => {
                let name = self.identifier_name(identifier);
                let resolved = self.resolve_binding(&name);
                (name, resolved)
            }
            UpdateTargetNode::PropertyAccess(access) => {
                return self.compile_property_update_expression(access, update.op());
            }
        };

        let one = self.add_number_constant(1.0)?;
        match update.op() {
            UpdateOpNode::IncrementPre | UpdateOpNode::DecrementPre => {
                self.emit_load_binding(&name, resolved)?;
                self.emit(Opcode::LoadConst(one));
                match update.op() {
                    UpdateOpNode::IncrementPre => self.emit(Opcode::Add),
                    UpdateOpNode::DecrementPre => self.emit(Opcode::Sub),
                    _ => unreachable!(),
                };
                self.emit(Opcode::Dup);
                self.emit_store_binding(&name, resolved)?;
            }
            UpdateOpNode::IncrementPost | UpdateOpNode::DecrementPost => {
                let temp = self.allocate_hidden_local()?;
                self.emit_load_binding(&name, resolved)?;
                self.emit(Opcode::Dup);
                self.emit(Opcode::SetLocal(temp));
                self.emit(Opcode::LoadConst(one));
                match update.op() {
                    UpdateOpNode::IncrementPost => self.emit(Opcode::Add),
                    UpdateOpNode::DecrementPost => self.emit(Opcode::Sub),
                    _ => unreachable!(),
                };
                self.emit_store_binding(&name, resolved)?;
                self.emit(Opcode::GetLocal(temp));
            }
        }
        Ok(())
    }

    fn compile_property_update_expression(
        &mut self,
        access: &super::ast::MemberExpression,
        operator: UpdateOpNode,
    ) -> Result<(), CompileError> {
        let obj_temp = self.allocate_hidden_local()?;
        let key_temp = self.allocate_hidden_local()?;
        let old_temp = self.allocate_hidden_local()?;
        let new_temp = self.allocate_hidden_local()?;
        let kind = self.compile_property_access_temps(access, obj_temp, key_temp)?;
        let one = self.add_number_constant(1.0)?;

        self.emit(Opcode::GetLocal(obj_temp));
        self.emit(Opcode::GetLocal(key_temp));
        self.emit_property_get(kind);
        self.emit(Opcode::Dup);
        self.emit(Opcode::SetLocal(old_temp));
        self.emit(Opcode::LoadConst(one));
        match operator {
            UpdateOpNode::IncrementPre | UpdateOpNode::IncrementPost => {
                self.emit(Opcode::Add);
            }
            UpdateOpNode::DecrementPre | UpdateOpNode::DecrementPost => {
                self.emit(Opcode::Sub);
            }
        }
        self.emit(Opcode::Dup);
        self.emit(Opcode::SetLocal(new_temp));
        self.emit(Opcode::GetLocal(obj_temp));
        self.emit(Opcode::GetLocal(key_temp));
        self.emit(Opcode::GetLocal(new_temp));
        self.emit_property_set(kind);

        match operator {
            UpdateOpNode::IncrementPre | UpdateOpNode::DecrementPre => {}
            UpdateOpNode::IncrementPost | UpdateOpNode::DecrementPost => {
                self.emit(Opcode::Pop);
                self.emit(Opcode::GetLocal(old_temp));
            }
        }
        Ok(())
    }

    fn compile_property_access_temps(
        &mut self,
        access: &super::ast::MemberExpression,
        obj_temp: u16,
        key_temp: u16,
    ) -> Result<PropertyOpKind, CompileError> {
        match access {
            super::ast::MemberExpression::Simple(access) => {
                self.compile_expression(access.target())?;
                self.emit(Opcode::SetLocal(obj_temp));
                match access.field() {
                    PropertyAccessFieldNode::Const(identifier) => {
                        let constant =
                            self.add_string_constant(self.identifier_name(identifier))?;
                        self.emit(Opcode::LoadConst(constant));
                        self.emit(Opcode::SetLocal(key_temp));
                        Ok(PropertyOpKind::Named)
                    }
                    PropertyAccessFieldNode::Expr(expression) => {
                        self.compile_expression(expression)?;
                        self.emit(Opcode::SetLocal(key_temp));
                        Ok(PropertyOpKind::Computed)
                    }
                }
            }
            super::ast::MemberExpression::Private(access) => {
                self.compile_expression(access.target())?;
                self.emit(Opcode::SetLocal(obj_temp));
                let constant = self.add_string_constant(self.private_field_key(&access.field()))?;
                self.emit(Opcode::LoadConst(constant));
                self.emit(Opcode::SetLocal(key_temp));
                Ok(PropertyOpKind::Named)
            }
            super::ast::MemberExpression::Super(access) => {
                self.compile_super_property_access_temps(access, obj_temp, key_temp)
            }
        }
    }

    fn emit_property_get(&mut self, kind: PropertyOpKind) {
        match kind {
            PropertyOpKind::Named => self.emit(Opcode::GetProp),
            PropertyOpKind::Computed => self.emit(Opcode::GetIndex),
        };
    }

    fn emit_property_set(&mut self, kind: PropertyOpKind) {
        match kind {
            PropertyOpKind::Named => self.emit(Opcode::SetProp),
            PropertyOpKind::Computed => self.emit(Opcode::SetIndex),
        };
    }

    fn compile_binary_expression(
        &mut self,
        binary: &super::ast::BinaryExpression,
    ) -> Result<(), CompileError> {
        match binary.op() {
            BinaryOpNode::Arithmetic(operator) => {
                self.compile_expression(binary.lhs())?;
                self.compile_expression(binary.rhs())?;
                match operator {
                    ArithmeticOpNode::Add => self.emit(Opcode::Add),
                    ArithmeticOpNode::Sub => self.emit(Opcode::Sub),
                    ArithmeticOpNode::Div => self.emit(Opcode::Div),
                    ArithmeticOpNode::Mul => self.emit(Opcode::Mul),
                    ArithmeticOpNode::Exp => self.emit(Opcode::Exp),
                    ArithmeticOpNode::Mod => self.emit(Opcode::Rem),
                };
            }
            BinaryOpNode::Bitwise(operator) => {
                self.compile_expression(binary.lhs())?;
                self.compile_expression(binary.rhs())?;
                match operator {
                    BitwiseOpNode::And => self.emit(Opcode::BitAnd),
                    BitwiseOpNode::Or => self.emit(Opcode::BitOr),
                    BitwiseOpNode::Xor => self.emit(Opcode::BitXor),
                    BitwiseOpNode::Shl => self.emit(Opcode::Shl),
                    BitwiseOpNode::Shr => self.emit(Opcode::Shr),
                    BitwiseOpNode::UShr => self.emit(Opcode::UShr),
                };
            }
            BinaryOpNode::Relational(operator) => {
                self.compile_expression(binary.lhs())?;
                self.compile_expression(binary.rhs())?;
                match operator {
                    RelationalOpNode::Equal => self.emit(Opcode::Eq),
                    RelationalOpNode::NotEqual => self.emit(Opcode::Ne),
                    RelationalOpNode::StrictEqual => self.emit(Opcode::StrictEq),
                    RelationalOpNode::StrictNotEqual => self.emit(Opcode::StrictNe),
                    RelationalOpNode::GreaterThan => self.emit(Opcode::Gt),
                    RelationalOpNode::GreaterThanOrEqual => self.emit(Opcode::Ge),
                    RelationalOpNode::LessThan => self.emit(Opcode::Lt),
                    RelationalOpNode::LessThanOrEqual => self.emit(Opcode::Le),
                    RelationalOpNode::In => self.emit(Opcode::In),
                    RelationalOpNode::InstanceOf => self.emit(Opcode::Instanceof),
                };
            }
            BinaryOpNode::Logical(operator) => {
                return self.compile_logical_expression(binary.lhs(), binary.rhs(), operator);
            }
            BinaryOpNode::Comma => {
                self.compile_expression(binary.lhs())?;
                self.emit(Opcode::Pop);
                self.compile_expression(binary.rhs())?;
            }
        }
        Ok(())
    }

    fn compile_logical_expression(
        &mut self,
        lhs: &ExpressionNode,
        rhs: &ExpressionNode,
        operator: LogicalOpNode,
    ) -> Result<(), CompileError> {
        match operator {
            LogicalOpNode::And => {
                self.compile_expression(lhs)?;
                self.emit(Opcode::Dup);
                let jump = self.emit_jump(Opcode::JumpIfFalsePop(0));
                self.emit(Opcode::Pop);
                self.compile_expression(rhs)?;
                let end = self.code.len();
                self.patch_jump(jump, end)?;
            }
            LogicalOpNode::Or => {
                self.compile_expression(lhs)?;
                self.emit(Opcode::Dup);
                let jump = self.emit_jump(Opcode::JumpIfTruePop(0));
                self.emit(Opcode::Pop);
                self.compile_expression(rhs)?;
                let end = self.code.len();
                self.patch_jump(jump, end)?;
            }
            LogicalOpNode::Coalesce => {
                return self.compile_nullish_expression(lhs, rhs);
            }
        }
        Ok(())
    }

    fn compile_conditional_expression(
        &mut self,
        conditional: &super::ast::ConditionalExpression,
    ) -> Result<(), CompileError> {
        self.compile_expression(conditional.condition())?;
        let false_jump = self.emit_jump(Opcode::JumpIfFalsePop(0));
        self.compile_expression(conditional.if_true())?;
        let end_jump = self.emit_jump(Opcode::Jump(0));
        let false_branch = self.code.len();
        self.patch_jump(false_jump, false_branch)?;
        self.compile_expression(conditional.if_false())?;
        let end = self.code.len();
        self.patch_jump(end_jump, end)?;
        Ok(())
    }
}
