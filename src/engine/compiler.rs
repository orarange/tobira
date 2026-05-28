use std::collections::HashMap;

use super::ast::{
    ArithmeticOpNode, AssignOpNode, AssignTargetNode, BinaryOpNode, BindingNode, BitwiseOpNode,
    ExpressionNode, ForLoopInitializerNode, FormalParameterListNode, FunctionBodyNode,
    FunctionDeclaration, LiteralKindNode, LogicalOpNode, MethodDefinitionKindNode,
    ObjectMethodDefinitionNode, ObjectPropertyDefinition, Program, PropertyAccessFieldNode,
    PropertyNameNode, RelationalOpNode, StatementNode, TemplateElementNode, UnaryOpNode,
    UpdateOpNode, UpdateTargetNode, VariableDeclaration, statement_list_item_to_node,
};
use super::chunk::{Chunk, Constant, FunctionProto, Opcode, UpvalueDescriptor};

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ResolvedBinding {
    Local(u16),
    Upvalue(u16),
    Global,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct LocalBinding {
    slot: u16,
}

#[derive(Debug, Clone, Default)]
struct ScopeFrame {
    bindings: HashMap<String, LocalBinding>,
}

#[derive(Debug, Clone, Default)]
struct OuterBindings {
    scopes: Vec<HashMap<String, u16>>,
    upvalues: HashMap<String, u16>,
}

#[derive(Debug, Clone, Default)]
struct LoopContext {
    break_jumps: Vec<usize>,
    continue_jumps: Vec<usize>,
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

pub struct Compiler<'a> {
    program: &'a Program,
}

impl<'a> Compiler<'a> {
    #[must_use]
    pub const fn new(program: &'a Program) -> Self {
        Self { program }
    }

    pub fn compile(&self) -> Result<Chunk, CompileError> {
        let mut function =
            FunctionCompiler::new(self.program, None, 0, self.program.strict(), true, None);
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
    upvalues: Vec<UpvalueDescriptor>,
    upvalue_names: HashMap<String, u16>,
    nested_functions: Vec<FunctionProto>,
    scopes: Vec<ScopeFrame>,
    next_local: u16,
    outer: Option<OuterBindings>,
    loop_stack: Vec<LoopContext>,
}

impl<'a> FunctionCompiler<'a> {
    fn new(
        program: &'a Program,
        name: Option<String>,
        arity: u8,
        is_strict: bool,
        is_top_level: bool,
        outer: Option<OuterBindings>,
    ) -> Self {
        Self {
            program,
            name,
            arity,
            is_strict,
            is_top_level,
            code: Vec::new(),
            constants: Vec::new(),
            upvalues: Vec::new(),
            upvalue_names: HashMap::new(),
            nested_functions: Vec::new(),
            scopes: vec![ScopeFrame::default()],
            next_local: 0,
            outer,
            loop_stack: Vec::new(),
        }
    }

    fn finish(self) -> FunctionProto {
        FunctionProto {
            name: self.name,
            arity: self.arity,
            code: self.code,
            constants: self.constants,
            upvalue_descriptors: self.upvalues,
            nested_functions: self.nested_functions,
            local_count: self.next_local,
            is_strict: self.is_strict,
        }
    }

    fn emit_implicit_return(&mut self) {
        self.emit(Opcode::LoadUndefined);
        self.emit(Opcode::Return);
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
            | Some(Opcode::JumpIfFalsePop(slot)) => {
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

    fn allocate_hidden_local(&mut self) -> Result<u16, CompileError> {
        let slot = self.next_local;
        self.next_local = self
            .next_local
            .checked_add(1)
            .ok_or_else(|| CompileError::message("local slot count exceeded u16"))?;
        Ok(slot)
    }

    fn push_scope(&mut self) {
        self.scopes.push(ScopeFrame::default());
    }

    fn pop_scope(&mut self) {
        self.scopes.pop();
    }

    fn root_scope_mut(&mut self) -> &mut ScopeFrame {
        self.scopes
            .first_mut()
            .expect("function scope should always exist")
    }

    fn current_scope_mut(&mut self) -> &mut ScopeFrame {
        self.scopes
            .last_mut()
            .expect("current scope should always exist")
    }

    fn declare_function_scoped(&mut self, name: &str) -> Result<u16, CompileError> {
        if let Some(binding) = self.root_scope_mut().bindings.get(name) {
            return Ok(binding.slot);
        }

        let slot = self.allocate_hidden_local()?;
        self.root_scope_mut()
            .bindings
            .insert(name.to_string(), LocalBinding { slot });
        Ok(slot)
    }

    fn declare_block_scoped(&mut self, name: &str) -> Result<u16, CompileError> {
        if let Some(binding) = self.current_scope_mut().bindings.get(name) {
            return Ok(binding.slot);
        }

        let slot = self.allocate_hidden_local()?;
        self.current_scope_mut()
            .bindings
            .insert(name.to_string(), LocalBinding { slot });
        Ok(slot)
    }

    fn resolve_binding(&mut self, name: &str) -> ResolvedBinding {
        for scope in self.scopes.iter().rev() {
            if let Some(binding) = scope.bindings.get(name) {
                return ResolvedBinding::Local(binding.slot);
            }
        }

        if let Some(outer) = &self.outer {
            for scope in outer.scopes.iter().rev() {
                if let Some(slot) = scope.get(name) {
                    let descriptor = UpvalueDescriptor {
                        is_local: true,
                        index: *slot,
                    };
                    let upvalue = self.get_or_create_upvalue(name, descriptor);
                    return ResolvedBinding::Upvalue(upvalue);
                }
            }
            if let Some(upvalue) = outer.upvalues.get(name) {
                let descriptor = UpvalueDescriptor {
                    is_local: false,
                    index: *upvalue,
                };
                let upvalue = self.get_or_create_upvalue(name, descriptor);
                return ResolvedBinding::Upvalue(upvalue);
            }
        }

        ResolvedBinding::Global
    }

    fn get_or_create_upvalue(&mut self, name: &str, descriptor: UpvalueDescriptor) -> u16 {
        if let Some(index) = self.upvalue_names.get(name) {
            return *index;
        }

        let index = self.upvalues.len() as u16;
        self.upvalues.push(descriptor);
        self.upvalue_names.insert(name.to_string(), index);
        index
    }

    fn snapshot_outer_bindings(&self) -> OuterBindings {
        let scopes = self
            .scopes
            .iter()
            .map(|scope| {
                scope
                    .bindings
                    .iter()
                    .map(|(name, binding)| (name.clone(), binding.slot))
                    .collect::<HashMap<_, _>>()
            })
            .collect();

        OuterBindings {
            scopes,
            upvalues: self.upvalue_names.clone(),
        }
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

    fn emit_load_binding(
        &mut self,
        name: &str,
        resolved: ResolvedBinding,
    ) -> Result<(), CompileError> {
        match resolved {
            ResolvedBinding::Local(slot) => {
                self.emit(Opcode::GetLocal(slot));
            }
            ResolvedBinding::Upvalue(slot) => {
                self.emit(Opcode::GetUpvalue(slot));
            }
            ResolvedBinding::Global => {
                let index = self.add_string_constant(name)?;
                self.emit(Opcode::GetGlobal(index));
            }
        }
        Ok(())
    }

    fn emit_store_binding(
        &mut self,
        name: &str,
        resolved: ResolvedBinding,
    ) -> Result<(), CompileError> {
        match resolved {
            ResolvedBinding::Local(slot) => {
                self.emit(Opcode::SetLocal(slot));
            }
            ResolvedBinding::Upvalue(slot) => {
                self.emit(Opcode::SetUpvalue(slot));
            }
            ResolvedBinding::Global => {
                let index = self.add_string_constant(name)?;
                self.emit(Opcode::SetGlobal(index));
            }
        }
        Ok(())
    }

    fn compile_statements(&mut self, statements: &[StatementNode]) -> Result<(), CompileError> {
        for statement in statements {
            self.compile_statement(statement)?;
        }
        Ok(())
    }

    fn compile_function_body(&mut self, body: &FunctionBodyNode) -> Result<(), CompileError> {
        for item in body.statements() {
            let statement = statement_list_item_to_node(item.clone());
            self.compile_statement(&statement)?;
        }
        Ok(())
    }

    fn compile_statement(&mut self, statement: &StatementNode) -> Result<(), CompileError> {
        match statement {
            StatementNode::VariableDeclaration(declaration) => {
                self.compile_variable_declaration(declaration, DeclarationContext::Statement)
            }
            StatementNode::FunctionDeclaration(declaration) => {
                self.compile_function_declaration_statement(declaration)
            }
            StatementNode::ClassDeclaration(_) => {
                Err(CompileError::Unimplemented("class declarations"))
            }
            StatementNode::BlockStatement(block) => self.compile_block_statement(block),
            StatementNode::IfStatement(statement) => self.compile_if_statement(statement),
            StatementNode::SwitchStatement(_) => {
                Err(CompileError::Unimplemented("switch statements"))
            }
            StatementNode::ForStatement(statement) => self.compile_for_statement(statement),
            StatementNode::ForInStatement(_) => {
                Err(CompileError::Unimplemented("for...in statements"))
            }
            StatementNode::ForOfStatement(_) => {
                Err(CompileError::Unimplemented("for...of statements"))
            }
            StatementNode::WhileStatement(statement) => self.compile_while_statement(statement),
            StatementNode::DoWhileStatement(statement) => {
                self.compile_do_while_statement(statement)
            }
            StatementNode::TryStatement(_) => Err(CompileError::Unimplemented("try/catch/finally")),
            StatementNode::ThrowStatement(statement) => {
                self.compile_expression(statement.target())?;
                self.emit(Opcode::Throw);
                Ok(())
            }
            StatementNode::ReturnStatement(statement) => {
                if let Some(target) = statement.target() {
                    self.compile_expression(target)?;
                } else {
                    self.emit(Opcode::LoadUndefined);
                }
                self.emit(Opcode::Return);
                Ok(())
            }
            StatementNode::BreakStatement(_) => {
                let jump = self.emit_jump(Opcode::Jump(0));
                let loop_context = self
                    .loop_stack
                    .last_mut()
                    .ok_or_else(|| CompileError::message("break used outside a loop"))?;
                loop_context.break_jumps.push(jump);
                Ok(())
            }
            StatementNode::ContinueStatement(_) => {
                let jump = self.emit_jump(Opcode::Jump(0));
                let loop_context = self
                    .loop_stack
                    .last_mut()
                    .ok_or_else(|| CompileError::message("continue used outside a loop"))?;
                loop_context.continue_jumps.push(jump);
                Ok(())
            }
            StatementNode::LabeledStatement(_) => {
                Err(CompileError::Unimplemented("labeled statements"))
            }
            StatementNode::ExpressionStatement(expression) => {
                self.compile_expression(expression)?;
                self.emit(Opcode::Pop);
                Ok(())
            }
            StatementNode::EmptyStatement => Ok(()),
            StatementNode::ImportDeclaration(_)
            | StatementNode::ExportNamedDeclaration(_)
            | StatementNode::ExportDefaultDeclaration(_)
            | StatementNode::ExportAllDeclaration(_) => Err(CompileError::Unimplemented(
                "module import/export statements",
            )),
            StatementNode::DebuggerStatement => Ok(()),
            StatementNode::WithStatement(_) => Err(CompileError::Unimplemented("with statements")),
        }
    }

    fn compile_block_statement(
        &mut self,
        block: &super::ast::BlockStatement,
    ) -> Result<(), CompileError> {
        self.push_scope();
        for item in block.statement_list().statements() {
            let statement = statement_list_item_to_node(item.clone());
            self.compile_statement(&statement)?;
        }
        self.pop_scope();
        Ok(())
    }

    fn compile_if_statement(
        &mut self,
        statement: &super::ast::IfStatement,
    ) -> Result<(), CompileError> {
        self.compile_expression(statement.cond())?;
        let else_jump = self.emit_jump(Opcode::JumpIfFalsePop(0));
        let then_statement = super::ast::statement_to_node(statement.body().clone());
        self.compile_statement(&then_statement)?;

        if let Some(else_node) = statement.else_node() {
            let end_jump = self.emit_jump(Opcode::Jump(0));
            let else_start = self.code.len();
            self.patch_jump(else_jump, else_start)?;
            let else_statement = super::ast::statement_to_node(else_node.clone());
            self.compile_statement(&else_statement)?;
            let end = self.code.len();
            self.patch_jump(end_jump, end)?;
        } else {
            let end = self.code.len();
            self.patch_jump(else_jump, end)?;
        }

        Ok(())
    }

    fn compile_while_statement(
        &mut self,
        statement: &super::ast::WhileStatement,
    ) -> Result<(), CompileError> {
        let loop_start = self.code.len();
        self.compile_expression(statement.condition())?;
        let exit_jump = self.emit_jump(Opcode::JumpIfFalsePop(0));
        self.loop_stack.push(LoopContext::default());
        let body = super::ast::statement_to_node(statement.body().clone());
        self.compile_statement(&body)?;
        let loop_context = self.loop_stack.pop().expect("loop context should exist");
        for jump in loop_context.continue_jumps {
            self.patch_jump(jump, loop_start)?;
        }
        self.emit_back_jump(loop_start)?;
        let loop_end = self.code.len();
        self.patch_jump(exit_jump, loop_end)?;
        for jump in loop_context.break_jumps {
            self.patch_jump(jump, loop_end)?;
        }
        Ok(())
    }

    fn compile_do_while_statement(
        &mut self,
        statement: &super::ast::DoWhileStatement,
    ) -> Result<(), CompileError> {
        let loop_start = self.code.len();
        self.loop_stack.push(LoopContext::default());
        let body = super::ast::statement_to_node(statement.body().clone());
        self.compile_statement(&body)?;
        let condition_start = self.code.len();
        let loop_context = self.loop_stack.pop().expect("loop context should exist");
        for jump in loop_context.continue_jumps {
            self.patch_jump(jump, condition_start)?;
        }
        self.compile_expression(statement.cond())?;
        let back_jump = self.emit_jump(Opcode::JumpIfTruePop(0));
        self.patch_jump(back_jump, loop_start)?;
        let loop_end = self.code.len();
        for jump in loop_context.break_jumps {
            self.patch_jump(jump, loop_end)?;
        }
        Ok(())
    }

    fn compile_for_statement(
        &mut self,
        statement: &super::ast::ForStatement,
    ) -> Result<(), CompileError> {
        let uses_lexical_init =
            matches!(statement.init(), Some(ForLoopInitializerNode::Lexical(_)));
        if uses_lexical_init {
            self.push_scope();
        }

        if let Some(initializer) = statement.init() {
            match initializer {
                ForLoopInitializerNode::Expression(expression) => {
                    self.compile_expression(expression)?;
                    self.emit(Opcode::Pop);
                }
                ForLoopInitializerNode::Var(var_decl) => {
                    let declaration = VariableDeclaration::Var(var_decl.clone());
                    self.compile_variable_declaration(
                        &declaration,
                        DeclarationContext::ForInitializer,
                    )?;
                }
                ForLoopInitializerNode::Lexical(lexical) => {
                    let declaration = if lexical.declaration().is_const() {
                        VariableDeclaration::Const(lexical.declaration().clone())
                    } else {
                        VariableDeclaration::Let(lexical.declaration().clone())
                    };
                    self.compile_variable_declaration(
                        &declaration,
                        DeclarationContext::ForInitializer,
                    )?;
                }
            }
        }

        let condition_start = self.code.len();
        let exit_jump = if let Some(condition) = statement.condition() {
            self.compile_expression(condition)?;
            Some(self.emit_jump(Opcode::JumpIfFalsePop(0)))
        } else {
            None
        };

        self.loop_stack.push(LoopContext::default());
        let body = super::ast::statement_to_node(statement.body().clone());
        self.compile_statement(&body)?;

        let increment_start = self.code.len();
        let loop_context = self.loop_stack.pop().expect("loop context should exist");
        for jump in loop_context.continue_jumps {
            self.patch_jump(jump, increment_start)?;
        }

        if let Some(final_expression) = statement.final_expr() {
            self.compile_expression(final_expression)?;
            self.emit(Opcode::Pop);
        }

        self.emit_back_jump(condition_start)?;
        let loop_end = self.code.len();
        if let Some(jump) = exit_jump {
            self.patch_jump(jump, loop_end)?;
        }
        for jump in loop_context.break_jumps {
            self.patch_jump(jump, loop_end)?;
        }

        if uses_lexical_init {
            self.pop_scope();
        }

        Ok(())
    }

    fn compile_variable_declaration(
        &mut self,
        declaration: &VariableDeclaration,
        context: DeclarationContext,
    ) -> Result<(), CompileError> {
        for variable in declaration.variables() {
            let name = self.binding_name(variable.binding())?;
            let resolved = if declaration.is_var() {
                if self.is_top_level {
                    ResolvedBinding::Global
                } else {
                    ResolvedBinding::Local(self.declare_function_scoped(&name)?)
                }
            } else if self.is_top_level
                && context == DeclarationContext::Statement
                && self.scopes.len() == 1
            {
                ResolvedBinding::Global
            } else {
                ResolvedBinding::Local(self.declare_block_scoped(&name)?)
            };

            if let Some(initializer) = variable.init() {
                self.compile_expression(initializer)?;
                self.emit_store_binding(&name, resolved)?;
            } else if declaration.is_const() {
                return Err(CompileError::message(format!(
                    "const declaration '{name}' requires an initializer"
                )));
            } else if matches!(resolved, ResolvedBinding::Global) {
                self.emit(Opcode::LoadUndefined);
                self.emit_store_binding(&name, resolved)?;
            }
        }
        Ok(())
    }

    fn compile_function_declaration_statement(
        &mut self,
        declaration: &FunctionDeclaration,
    ) -> Result<(), CompileError> {
        if declaration.is_async() {
            return Err(CompileError::Unimplemented("async functions"));
        }
        if declaration.is_generator() {
            return Err(CompileError::Unimplemented("generator functions"));
        }

        let name = self.identifier_name(&declaration.name());
        let resolved = if self.is_top_level {
            ResolvedBinding::Global
        } else {
            ResolvedBinding::Local(self.declare_function_scoped(&name)?)
        };

        let nested_index = self.compile_nested_function(
            Some(name.clone()),
            declaration.parameters(),
            declaration.body(),
            declaration.body().strict(),
        )?;
        self.emit(Opcode::MakeClosure(nested_index));
        self.emit_store_binding(&name, resolved)?;
        Ok(())
    }

    fn compile_nested_function(
        &mut self,
        name: Option<String>,
        parameters: &FormalParameterListNode,
        body: &FunctionBodyNode,
        is_strict: bool,
    ) -> Result<u16, CompileError> {
        let arity = u8::try_from(parameters.length())
            .map_err(|_| CompileError::message("function arity exceeded u8"))?;
        let outer = Some(self.snapshot_outer_bindings());
        let mut child = FunctionCompiler::new(self.program, name, arity, is_strict, false, outer);

        for parameter in parameters.as_ref() {
            if parameter.is_rest_param() {
                return Err(CompileError::Unimplemented("rest parameters"));
            }
            let name = child.binding_name(parameter.variable().binding())?;
            let _ = child.declare_function_scoped(&name)?;
            if let Some(initializer) = parameter.init() {
                let _ = initializer;
                return Err(CompileError::message(format!(
                    "default parameter initializers are not supported yet: {name} = ..."
                )));
            }
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
                self.emit(Opcode::LoadThis);
                Ok(())
            }
            ExpressionNode::Identifier(identifier) => {
                let name = self.identifier_name(identifier);
                let resolved = self.resolve_binding(&name);
                self.emit_load_binding(&name, resolved)
            }
            ExpressionNode::Literal(literal) => self.compile_literal(literal.kind()),
            ExpressionNode::RegExpLiteral(_) => {
                Err(CompileError::Unimplemented("regular expressions"))
            }
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
            ),
            ExpressionNode::ArrowFunction(function) => self.compile_nested_function_value(
                function
                    .name()
                    .map(|identifier| self.identifier_name(&identifier)),
                function.parameters(),
                function.body(),
                function.body().strict(),
            ),
            ExpressionNode::AsyncArrowFunction(_) => {
                Err(CompileError::Unimplemented("async arrow functions"))
            }
            ExpressionNode::GeneratorExpression(_) => {
                Err(CompileError::Unimplemented("generator expressions"))
            }
            ExpressionNode::AsyncFunctionExpression(_) => {
                Err(CompileError::Unimplemented("async function expressions"))
            }
            ExpressionNode::AsyncGeneratorExpression(_) => {
                Err(CompileError::Unimplemented("async generator expressions"))
            }
            ExpressionNode::ClassExpression(_) => {
                Err(CompileError::Unimplemented("class expressions"))
            }
            ExpressionNode::TemplateLiteral(template) => self.compile_template_literal(template),
            ExpressionNode::PropertyAccess(access) => {
                self.compile_property_access_expression(access)
            }
            ExpressionNode::New(new_expression) => self.compile_new_expression(new_expression),
            ExpressionNode::Call(call) => self.compile_call_expression(call),
            ExpressionNode::SuperCall(_) => Err(CompileError::Unimplemented("super calls")),
            ExpressionNode::ImportCall(_) => Err(CompileError::Unimplemented("import() calls")),
            ExpressionNode::Optional(_) => Err(CompileError::Unimplemented("optional chaining")),
            ExpressionNode::TaggedTemplate(_) => {
                Err(CompileError::Unimplemented("tagged template literals"))
            }
            ExpressionNode::NewTarget(_) => Err(CompileError::Unimplemented("new.target")),
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
            ExpressionNode::Await(_) => Err(CompileError::Unimplemented("await expressions")),
            ExpressionNode::Yield(_) => Err(CompileError::Unimplemented("yield expressions")),
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
                    self.compile_property_name_value(name)?;
                    self.compile_expression(value)?;
                    self.emit(Opcode::SetProp);
                }
                ObjectPropertyDefinition::MethodDefinition(method) => {
                    self.compile_property_name_value(method.name())?;
                    self.compile_object_method_value(method)?;
                    self.emit(Opcode::SetProp);
                }
                ObjectPropertyDefinition::SpreadObject(_) => {
                    return Err(CompileError::Unimplemented("object spread properties"));
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
        match method.kind() {
            MethodDefinitionKindNode::Ordinary => {}
            MethodDefinitionKindNode::Get | MethodDefinitionKindNode::Set => {
                return Err(CompileError::Unimplemented("object literal accessors"));
            }
            MethodDefinitionKindNode::Generator => {
                return Err(CompileError::Unimplemented("generator object methods"));
            }
            MethodDefinitionKindNode::AsyncGenerator | MethodDefinitionKindNode::Async => {
                return Err(CompileError::Unimplemented("async object methods"));
            }
        }

        self.compile_nested_function_value(
            self.property_name_string(method.name()),
            method.parameters(),
            method.body(),
            method.body().strict(),
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
            super::ast::MemberExpression::Private(_) => {
                Err(CompileError::Unimplemented("private property access"))
            }
            super::ast::MemberExpression::Super(_) => {
                Err(CompileError::Unimplemented("super property access"))
            }
        }
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
    ) -> Result<(), CompileError> {
        let index = self.compile_nested_function(name, parameters, body, is_strict)?;
        self.emit(Opcode::MakeClosure(index));
        Ok(())
    }

    fn compile_call_expression(
        &mut self,
        call: &super::ast::CallExpression,
    ) -> Result<(), CompileError> {
        match call.function() {
            ExpressionNode::PropertyAccess(access) => {
                self.compile_property_access_for_call(access)?
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
            super::ast::MemberExpression::Private(_) => {
                Err(CompileError::Unimplemented("private method calls"))
            }
            super::ast::MemberExpression::Super(_) => {
                Err(CompileError::Unimplemented("super method calls"))
            }
        }
    }

    fn compile_new_expression(
        &mut self,
        new_expression: &super::ast::NewExpression,
    ) -> Result<(), CompileError> {
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
        let (name, resolved) = match assign.lhs() {
            AssignTargetNode::Identifier(identifier) => {
                let name = self.identifier_name(identifier);
                let resolved = self.resolve_binding(&name);
                (name, resolved)
            }
            AssignTargetNode::Access(access) => {
                return self.compile_property_assignment(access, assign.op(), assign.rhs());
            }
            AssignTargetNode::Pattern(_) => {
                return Err(CompileError::Unimplemented("destructuring assignment"));
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
                return Err(CompileError::Unimplemented("logical assignment operators"));
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
            UnaryOpNode::Delete => {
                self.emit(Opcode::Delete);
            }
            UnaryOpNode::Plus => return Err(CompileError::Unimplemented("unary plus")),
        }
        Ok(())
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
            super::ast::MemberExpression::Private(_) => {
                Err(CompileError::Unimplemented("private property access"))
            }
            super::ast::MemberExpression::Super(_) => {
                Err(CompileError::Unimplemented("super property access"))
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
                return Err(CompileError::Unimplemented("nullish coalescing"));
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
