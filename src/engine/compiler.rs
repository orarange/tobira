use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use super::ast::{
    ArithmeticOpNode, ArrayPatternElementNode, AssignOpNode, AssignTargetNode, BinaryOpNode,
    BindingNode, BitwiseOpNode, ClassDeclarationNode, ClassElementNameNode, ClassElementNode,
    ClassExpressionNode, ExpressionNode, ForLoopInitializerNode, FormalParameterListNode,
    FunctionBodyNode, FunctionDeclaration, IterableLoopInitializerNode, LiteralKindNode,
    LogicalOpNode, MethodDefinitionKindNode, ObjectMethodDefinitionNode, ObjectPatternElementNode,
    ObjectPropertyDefinition, OptionalOperationKindNode, Program, PropertyAccessFieldNode,
    PropertyNameNode, RelationalOpNode, StatementNode, SuperCallExpression,
    SuperPropertyAccessNode, TemplateElementNode, UnaryOpNode, UpdateOpNode, UpdateTargetNode,
    VariableDeclaration, statement_list_item_to_node,
};
use super::chunk::{Chunk, Constant, ExceptionHandler, FunctionProto, Opcode, UpvalueDescriptor};

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

/// The upvalue list of a single function, shared (via `Rc`) between the live
/// `FunctionCompiler` and the `OuterBindings` snapshots handed to its nested
/// functions. Sharing is what makes *transitive* upvalue capture possible: a
/// grandchild resolving a name from a grandparent can retroactively add an
/// upvalue to the intermediate parent's real upvalue list.
#[derive(Debug, Clone, Default)]
struct UpvalueState {
    descriptors: Vec<UpvalueDescriptor>,
    names: HashMap<String, u16>,
}

impl UpvalueState {
    fn get_or_create(&mut self, name: &str, descriptor: UpvalueDescriptor) -> u16 {
        if let Some(index) = self.names.get(name) {
            return *index;
        }
        let index = self.descriptors.len() as u16;
        self.descriptors.push(descriptor);
        self.names.insert(name.to_string(), index);
        index
    }
}

/// A view onto an enclosing function used while compiling a nested function.
/// `scopes` is a snapshot of that function's locals at the moment the nested
/// function was created (the enclosing function does not add locals while the
/// nested one compiles). `upvalues` is *shared* with the real enclosing
/// `FunctionCompiler`, and `parent` chains further out.
#[derive(Debug, Clone)]
struct OuterBindings {
    scopes: Vec<HashMap<String, u16>>,
    upvalues: Rc<RefCell<UpvalueState>>,
    parent: Option<Box<OuterBindings>>,
}

impl OuterBindings {
    fn lookup_local(&self, name: &str) -> Option<u16> {
        for scope in self.scopes.iter().rev() {
            if let Some(slot) = scope.get(name) {
                return Some(*slot);
            }
        }
        None
    }

    /// Ensure this enclosing frame has an upvalue capturing `name`, recursing up
    /// the chain and creating any intermediate upvalues. Returns the upvalue
    /// index *within this frame*, or `None` if `name` is not found anywhere up
    /// the chain.
    fn ensure_upvalue(&self, name: &str) -> Option<u16> {
        if let Some(index) = self.upvalues.borrow().names.get(name) {
            return Some(*index);
        }
        let parent = self.parent.as_ref()?;
        if let Some(slot) = parent.lookup_local(name) {
            let descriptor = UpvalueDescriptor {
                is_local: true,
                index: slot,
            };
            return Some(self.upvalues.borrow_mut().get_or_create(name, descriptor));
        }
        let parent_index = parent.ensure_upvalue(name)?;
        let descriptor = UpvalueDescriptor {
            is_local: false,
            index: parent_index,
        };
        Some(self.upvalues.borrow_mut().get_or_create(name, descriptor))
    }
}

#[derive(Debug, Clone, Default)]
struct ControlContext {
    break_jumps: Vec<usize>,
    continue_jumps: Vec<usize>,
    is_loop: bool,
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

#[derive(Debug, Clone)]
struct PendingPatternInit {
    pattern: super::ast::PatternNode,
    slot: u16,
    storage: BindingStorage,
}

#[derive(Debug, Clone, Default)]
struct FunctionCompileOptions {
    super_ctor_binding: Option<String>,
    super_proto_binding: Option<String>,
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
        let mut function = FunctionCompiler::new(
            self.program,
            None,
            0,
            self.program.strict(),
            true,
            None,
            FunctionCompileOptions::default(),
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
    options: FunctionCompileOptions,
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
            options,
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

        if let Some(index) = self.resolve_upvalue(name) {
            return ResolvedBinding::Upvalue(index);
        }

        ResolvedBinding::Global
    }

    /// Resolve `name` as an upvalue of the current function, walking the whole
    /// enclosing chain and lazily creating intermediate upvalues so that
    /// transitive captures (e.g. `a => b => c => a + b + c`) work. Returns the
    /// upvalue index within the current function, or `None` if `name` is global.
    fn resolve_upvalue(&self, name: &str) -> Option<u16> {
        let outer = self.outer.as_ref()?;
        if let Some(slot) = outer.lookup_local(name) {
            let descriptor = UpvalueDescriptor {
                is_local: true,
                index: slot,
            };
            return Some(self.upvalues.borrow_mut().get_or_create(name, descriptor));
        }
        let parent_index = outer.ensure_upvalue(name)?;
        let descriptor = UpvalueDescriptor {
            is_local: false,
            index: parent_index,
        };
        Some(self.upvalues.borrow_mut().get_or_create(name, descriptor))
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
            upvalues: Rc::clone(&self.upvalues),
            parent: self.outer.clone().map(Box::new),
        }
    }

    /// Install a hidden local named `this` for non-arrow functions and seed it
    /// from the call frame's receiver. Arrow functions deliberately skip this so
    /// that `this` resolves up the scope chain to the nearest enclosing
    /// non-arrow function (lexical `this`).
    fn install_this_binding(&mut self) -> Result<(), CompileError> {
        let slot = self.allocate_hidden_local()?;
        self.root_scope_mut()
            .bindings
            .insert("this".to_string(), LocalBinding { slot });
        self.emit(Opcode::LoadThis);
        self.emit(Opcode::SetLocal(slot));
        Ok(())
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

    fn declare_named_hidden_local(&mut self, name: impl Into<String>) -> Result<u16, CompileError> {
        let slot = self.allocate_hidden_local()?;
        self.current_scope_mut()
            .bindings
            .insert(name.into(), LocalBinding { slot });
        Ok(slot)
    }

    fn resolve_declaration_binding(
        &mut self,
        name: &str,
        storage: BindingStorage,
        context: DeclarationContext,
    ) -> Result<ResolvedBinding, CompileError> {
        Ok(match storage {
            BindingStorage::Assignment => self.resolve_binding(name),
            BindingStorage::Var => {
                if self.is_top_level {
                    ResolvedBinding::Global
                } else {
                    ResolvedBinding::Local(self.declare_function_scoped(name)?)
                }
            }
            BindingStorage::Let | BindingStorage::Const => {
                if self.is_top_level
                    && context == DeclarationContext::Statement
                    && self.scopes.len() == 1
                {
                    ResolvedBinding::Global
                } else {
                    ResolvedBinding::Local(self.declare_block_scoped(name)?)
                }
            }
        })
    }

    fn emit_active_finally_blocks(&mut self) -> Result<(), CompileError> {
        let blocks = self.active_finally_blocks.clone();
        for block in blocks.iter().rev() {
            self.compile_inline_block(block)?;
        }
        Ok(())
    }

    fn compile_inline_block(
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

    fn compile_function_parameters(
        &mut self,
        parameters: &FormalParameterListNode,
    ) -> Result<(), CompileError> {
        let parameter_entries: Vec<_> = parameters.as_ref().iter().cloned().collect();
        let mut raw_slots = Vec::with_capacity(parameter_entries.len());
        for parameter in &parameter_entries {
            let raw_slot = self.allocate_hidden_local()?;
            raw_slots.push(raw_slot);
            self.parameter_count = self
                .parameter_count
                .checked_add(1)
                .ok_or_else(|| CompileError::message("parameter slot count exceeded u16"))?;
            if parameter.is_rest_param() {
                self.has_rest_param = true;
            }
            if let Some(_initializer) = parameter.init() {
                let name = match parameter.variable().binding() {
                    BindingNode::Identifier(identifier) => self.identifier_name(identifier),
                    BindingNode::Pattern(_) => "<pattern>".to_string(),
                };
                return Err(CompileError::message(format!(
                    "default parameter initializers are not supported yet: {name} = ..."
                )));
            }
        }

        let mut pending = Vec::new();
        for (parameter, raw_slot) in parameter_entries.iter().zip(raw_slots.iter().copied()) {
            match parameter.variable().binding() {
                BindingNode::Identifier(identifier) => {
                    let name = self.identifier_name(identifier);
                    let resolved = self.resolve_declaration_binding(
                        &name,
                        if parameter.is_rest_param() {
                            BindingStorage::Let
                        } else {
                            BindingStorage::Var
                        },
                        DeclarationContext::Statement,
                    )?;
                    match resolved {
                        ResolvedBinding::Local(slot) if slot == raw_slot => {}
                        ResolvedBinding::Local(slot) => {
                            self.emit(Opcode::GetLocal(raw_slot));
                            self.emit(Opcode::SetLocal(slot));
                        }
                        _ => {
                            self.emit(Opcode::GetLocal(raw_slot));
                            self.emit_store_binding(&name, resolved)?;
                        }
                    }
                }
                BindingNode::Pattern(pattern) => pending.push(PendingPatternInit {
                    pattern: pattern.clone(),
                    slot: raw_slot,
                    storage: BindingStorage::Let,
                }),
            }
        }

        for init in pending {
            self.compile_pattern_store(
                &init.pattern,
                init.slot,
                init.storage,
                DeclarationContext::Statement,
            )?;
        }

        Ok(())
    }

    fn compile_binding_store(
        &mut self,
        binding: &BindingNode,
        source_slot: u16,
        storage: BindingStorage,
        context: DeclarationContext,
    ) -> Result<(), CompileError> {
        match binding {
            BindingNode::Identifier(identifier) => {
                let name = self.identifier_name(identifier);
                let resolved = self.resolve_declaration_binding(&name, storage, context)?;
                self.emit(Opcode::GetLocal(source_slot));
                self.emit_store_binding(&name, resolved)
            }
            BindingNode::Pattern(pattern) => {
                self.compile_pattern_store(pattern, source_slot, storage, context)
            }
        }
    }

    fn compile_pattern_store(
        &mut self,
        pattern: &super::ast::PatternNode,
        source_slot: u16,
        storage: BindingStorage,
        context: DeclarationContext,
    ) -> Result<(), CompileError> {
        match pattern {
            super::ast::PatternNode::Object(pattern) => {
                self.compile_object_pattern_store(pattern, source_slot, storage, context)
            }
            super::ast::PatternNode::Array(pattern) => {
                self.compile_array_pattern_store(pattern, source_slot, storage, context)
            }
        }
    }

    fn compile_object_pattern_store(
        &mut self,
        pattern: &super::ast::ObjectPatternNode,
        source_slot: u16,
        storage: BindingStorage,
        context: DeclarationContext,
    ) -> Result<(), CompileError> {
        for property in pattern.bindings() {
            match property {
                ObjectPatternElementNode::SingleName {
                    name,
                    ident,
                    default_init,
                } => {
                    let value_slot = self.extract_object_property_to_slot(source_slot, name)?;
                    let value_slot =
                        self.apply_default_initializer_slot(value_slot, default_init.as_ref())?;
                    let temp_binding = BindingNode::Identifier(*ident);
                    self.compile_binding_store(&temp_binding, value_slot, storage, context)?;
                }
                ObjectPatternElementNode::Pattern {
                    name,
                    pattern,
                    default_init,
                } => {
                    let value_slot = self.extract_object_property_to_slot(source_slot, name)?;
                    let value_slot =
                        self.apply_default_initializer_slot(value_slot, default_init.as_ref())?;
                    self.compile_pattern_store(pattern, value_slot, storage, context)?;
                }
                ObjectPatternElementNode::AssignmentPropertyAccess {
                    name,
                    access,
                    default_init,
                } => {
                    let value_slot = self.extract_object_property_to_slot(source_slot, name)?;
                    let value_slot =
                        self.apply_default_initializer_slot(value_slot, default_init.as_ref())?;
                    self.assign_member_from_slot(access, value_slot)?;
                }
                ObjectPatternElementNode::RestProperty { ident } => {
                    let rest_slot = self.copy_slot_to_object(source_slot)?;
                    let temp_binding = BindingNode::Identifier(*ident);
                    self.compile_binding_store(&temp_binding, rest_slot, storage, context)?;
                }
                ObjectPatternElementNode::AssignmentRestPropertyAccess { access } => {
                    let rest_slot = self.copy_slot_to_object(source_slot)?;
                    self.assign_member_from_slot(access, rest_slot)?;
                }
            }
        }
        Ok(())
    }

    fn compile_array_pattern_store(
        &mut self,
        pattern: &super::ast::ArrayPatternNode,
        source_slot: u16,
        storage: BindingStorage,
        context: DeclarationContext,
    ) -> Result<(), CompileError> {
        for (index, element) in pattern.bindings().iter().enumerate() {
            match element {
                ArrayPatternElementNode::Elision => {}
                ArrayPatternElementNode::SingleName {
                    ident,
                    default_init,
                } => {
                    let value_slot = self.extract_array_index_to_slot(source_slot, index as u32)?;
                    let value_slot =
                        self.apply_default_initializer_slot(value_slot, default_init.as_ref())?;
                    let temp_binding = BindingNode::Identifier(*ident);
                    self.compile_binding_store(&temp_binding, value_slot, storage, context)?;
                }
                ArrayPatternElementNode::PropertyAccess {
                    access,
                    default_init,
                } => {
                    let value_slot = self.extract_array_index_to_slot(source_slot, index as u32)?;
                    let value_slot =
                        self.apply_default_initializer_slot(value_slot, default_init.as_ref())?;
                    self.assign_member_from_slot(access, value_slot)?;
                }
                ArrayPatternElementNode::Pattern {
                    pattern,
                    default_init,
                } => {
                    let value_slot = self.extract_array_index_to_slot(source_slot, index as u32)?;
                    let value_slot =
                        self.apply_default_initializer_slot(value_slot, default_init.as_ref())?;
                    self.compile_pattern_store(pattern, value_slot, storage, context)?;
                }
                ArrayPatternElementNode::SingleNameRest { ident } => {
                    let rest_slot = self.slice_array_slot(source_slot, index as u32)?;
                    let temp_binding = BindingNode::Identifier(*ident);
                    self.compile_binding_store(&temp_binding, rest_slot, storage, context)?;
                }
                ArrayPatternElementNode::PropertyAccessRest { access } => {
                    let rest_slot = self.slice_array_slot(source_slot, index as u32)?;
                    self.assign_member_from_slot(access, rest_slot)?;
                }
                ArrayPatternElementNode::PatternRest { pattern } => {
                    let rest_slot = self.slice_array_slot(source_slot, index as u32)?;
                    self.compile_pattern_store(pattern, rest_slot, storage, context)?;
                }
            }
        }
        Ok(())
    }

    fn extract_object_property_to_slot(
        &mut self,
        source_slot: u16,
        name: &PropertyNameNode,
    ) -> Result<u16, CompileError> {
        let slot = self.allocate_hidden_local()?;
        self.emit(Opcode::GetLocal(source_slot));
        match name {
            PropertyNameNode::Literal(identifier) => {
                let constant = self.add_string_constant(self.identifier_name(identifier))?;
                self.emit(Opcode::LoadConst(constant));
                self.emit(Opcode::GetProp);
            }
            PropertyNameNode::Computed(expression) => {
                self.compile_expression(expression)?;
                self.emit(Opcode::GetIndex);
            }
        }
        self.emit(Opcode::SetLocal(slot));
        Ok(slot)
    }

    fn extract_array_index_to_slot(
        &mut self,
        source_slot: u16,
        index: u32,
    ) -> Result<u16, CompileError> {
        let slot = self.allocate_hidden_local()?;
        let constant = self.add_number_constant(index as f64)?;
        self.emit(Opcode::GetLocal(source_slot));
        self.emit(Opcode::LoadConst(constant));
        self.emit(Opcode::GetIndex);
        self.emit(Opcode::SetLocal(slot));
        Ok(slot)
    }

    fn apply_default_initializer_slot(
        &mut self,
        value_slot: u16,
        default_init: Option<&ExpressionNode>,
    ) -> Result<u16, CompileError> {
        let Some(default_init) = default_init else {
            return Ok(value_slot);
        };

        let result_slot = self.allocate_hidden_local()?;
        self.emit(Opcode::GetLocal(value_slot));
        self.emit(Opcode::Dup);
        self.emit(Opcode::LoadUndefined);
        self.emit(Opcode::StrictEq);
        let use_original = self.emit_jump(Opcode::JumpIfFalsePop(0));
        self.emit(Opcode::Pop);
        self.compile_expression(default_init)?;
        self.emit(Opcode::SetLocal(result_slot));
        let end = self.emit_jump(Opcode::Jump(0));
        let original_branch = self.code.len();
        self.patch_jump(use_original, original_branch)?;
        self.emit(Opcode::SetLocal(result_slot));
        let end_index = self.code.len();
        self.patch_jump(end, end_index)?;
        Ok(result_slot)
    }

    fn assign_member_from_slot(
        &mut self,
        access: &super::ast::MemberExpression,
        value_slot: u16,
    ) -> Result<(), CompileError> {
        let obj_temp = self.allocate_hidden_local()?;
        let key_temp = self.allocate_hidden_local()?;
        let kind = self.compile_property_access_temps(access, obj_temp, key_temp)?;
        self.emit(Opcode::GetLocal(obj_temp));
        self.emit(Opcode::GetLocal(key_temp));
        self.emit(Opcode::GetLocal(value_slot));
        self.emit_property_set(kind);
        Ok(())
    }

    fn copy_slot_to_object(&mut self, source_slot: u16) -> Result<u16, CompileError> {
        let slot = self.allocate_hidden_local()?;
        self.emit(Opcode::MakeObject);
        self.emit(Opcode::SetLocal(slot));
        self.emit(Opcode::GetLocal(slot));
        self.emit(Opcode::GetLocal(source_slot));
        self.emit(Opcode::CopyDataProperties);
        self.emit(Opcode::GetLocal(slot));
        self.emit(Opcode::SetLocal(slot));
        Ok(slot)
    }

    fn slice_array_slot(&mut self, source_slot: u16, start: u32) -> Result<u16, CompileError> {
        let slot = self.allocate_hidden_local()?;
        let property = self.add_string_constant("slice")?;
        let start_constant = self.add_number_constant(start as f64)?;
        self.emit(Opcode::GetLocal(source_slot));
        self.emit(Opcode::GetPropForCall(property));
        self.emit(Opcode::LoadConst(start_constant));
        self.emit(Opcode::Call(1));
        self.emit(Opcode::SetLocal(slot));
        Ok(slot)
    }

    fn create_accumulator_array_slot(&mut self) -> Result<u16, CompileError> {
        let slot = self.allocate_hidden_local()?;
        self.emit(Opcode::MakeArray(0));
        self.emit(Opcode::SetLocal(slot));
        Ok(slot)
    }

    fn push_value_into_array_slot(
        &mut self,
        array_slot: u16,
        value: &ExpressionNode,
    ) -> Result<(), CompileError> {
        let push_name = self.add_string_constant("push")?;
        self.emit(Opcode::GetLocal(array_slot));
        self.emit(Opcode::GetPropForCall(push_name));
        self.compile_expression(value)?;
        self.emit(Opcode::Call(1));
        self.emit(Opcode::Pop);
        Ok(())
    }

    fn push_undefined_into_array_slot(&mut self, array_slot: u16) -> Result<(), CompileError> {
        let push_name = self.add_string_constant("push")?;
        self.emit(Opcode::GetLocal(array_slot));
        self.emit(Opcode::GetPropForCall(push_name));
        self.emit(Opcode::LoadUndefined);
        self.emit(Opcode::Call(1));
        self.emit(Opcode::Pop);
        Ok(())
    }

    fn concat_spread_expression_into_array_slot(
        &mut self,
        array_slot: u16,
        spread: &super::ast::SpreadElement,
    ) -> Result<(), CompileError> {
        let concat_name = self.add_string_constant("concat")?;
        self.emit(Opcode::GetLocal(array_slot));
        self.emit(Opcode::GetPropForCall(concat_name));
        self.emit_array_from_expression(spread.target())?;
        self.emit(Opcode::Call(1));
        self.emit(Opcode::SetLocal(array_slot));
        Ok(())
    }

    fn emit_array_from_expression(
        &mut self,
        expression: &ExpressionNode,
    ) -> Result<(), CompileError> {
        let array_name = self.add_string_constant("Array")?;
        let from_name = self.add_string_constant("from")?;
        self.emit(Opcode::GetGlobal(array_name));
        self.emit(Opcode::GetPropForCall(from_name));
        self.compile_expression(expression)?;
        self.emit(Opcode::Call(1));
        Ok(())
    }

    fn compile_array_literal_with_spread(
        &mut self,
        array: &super::ast::ArrayExpression,
    ) -> Result<(), CompileError> {
        let array_slot = self.create_accumulator_array_slot()?;
        for element in array.as_ref() {
            match element {
                Some(ExpressionNode::Spread(spread)) => {
                    self.concat_spread_expression_into_array_slot(array_slot, spread)?;
                }
                Some(expression) => {
                    self.push_value_into_array_slot(array_slot, expression)?;
                }
                None => {
                    self.push_undefined_into_array_slot(array_slot)?;
                }
            }
        }
        self.emit(Opcode::GetLocal(array_slot));
        Ok(())
    }

    fn compile_argument_array(
        &mut self,
        arguments: &[ExpressionNode],
    ) -> Result<u16, CompileError> {
        let array_slot = self.create_accumulator_array_slot()?;
        for argument in arguments {
            match argument {
                ExpressionNode::Spread(spread) => {
                    self.concat_spread_expression_into_array_slot(array_slot, spread)?;
                }
                other => self.push_value_into_array_slot(array_slot, other)?,
            }
        }
        Ok(array_slot)
    }

    fn compile_call_expression_with_spread(
        &mut self,
        call: &super::ast::CallExpression,
    ) -> Result<(), CompileError> {
        let helper = self.add_string_constant("__callSpread")?;
        self.emit(Opcode::GetGlobal(helper));
        self.emit(Opcode::LoadUndefined);
        match call.function() {
            ExpressionNode::PropertyAccess(access) => {
                self.compile_property_access_for_call(access)?
            }
            other => {
                self.compile_expression(other)?;
                self.emit(Opcode::LoadUndefined);
            }
        }
        let args_slot = self.compile_argument_array(call.args())?;
        self.emit(Opcode::GetLocal(args_slot));
        self.emit(Opcode::Call(3));
        Ok(())
    }

    fn compile_new_expression_with_spread(
        &mut self,
        new_expression: &super::ast::NewExpression,
    ) -> Result<(), CompileError> {
        let helper = self.add_string_constant("__constructSpread")?;
        self.emit(Opcode::GetGlobal(helper));
        self.emit(Opcode::LoadUndefined);
        self.compile_expression(new_expression.constructor())?;
        let args_slot = self.compile_argument_array(new_expression.arguments())?;
        self.emit(Opcode::GetLocal(args_slot));
        self.emit(Opcode::Call(2));
        Ok(())
    }

    fn compile_logical_assignment_expression(
        &mut self,
        assign: &super::ast::AssignmentExpression,
    ) -> Result<(), CompileError> {
        match assign.lhs() {
            AssignTargetNode::Identifier(identifier) => {
                let name = self.identifier_name(identifier);
                let resolved = self.resolve_binding(&name);
                self.emit_load_binding(&name, resolved)?;
                self.emit(Opcode::Dup);
                let jump = match assign.op() {
                    AssignOpNode::BoolAnd => self.emit_jump(Opcode::JumpIfFalsePop(0)),
                    AssignOpNode::BoolOr => self.emit_jump(Opcode::JumpIfTruePop(0)),
                    AssignOpNode::Coalesce => self.emit_jump(Opcode::JumpIfNullish(0)),
                    _ => unreachable!(),
                };
                self.emit(Opcode::Pop);
                if matches!(assign.op(), AssignOpNode::Coalesce) {
                    self.emit_load_binding(&name, resolved)?;
                    let skip = self.emit_jump(Opcode::Jump(0));
                    let assign_start = self.code.len();
                    self.patch_jump(jump, assign_start)?;
                    self.emit(Opcode::Pop);
                    self.compile_expression(assign.rhs())?;
                    self.emit(Opcode::Dup);
                    self.emit_store_binding(&name, resolved)?;
                    let end = self.code.len();
                    self.patch_jump(skip, end)?;
                    return Ok(());
                }
                self.compile_expression(assign.rhs())?;
                self.emit(Opcode::Dup);
                self.emit_store_binding(&name, resolved)?;
                let end = self.code.len();
                self.patch_jump(jump, end)?;
                Ok(())
            }
            AssignTargetNode::Access(access) => {
                let obj_temp = self.allocate_hidden_local()?;
                let key_temp = self.allocate_hidden_local()?;
                let kind = self.compile_property_access_temps(access, obj_temp, key_temp)?;
                self.emit(Opcode::GetLocal(obj_temp));
                self.emit(Opcode::GetLocal(key_temp));
                self.emit_property_get(kind);
                self.emit(Opcode::Dup);
                let jump = match assign.op() {
                    AssignOpNode::BoolAnd => self.emit_jump(Opcode::JumpIfFalsePop(0)),
                    AssignOpNode::BoolOr => self.emit_jump(Opcode::JumpIfTruePop(0)),
                    AssignOpNode::Coalesce => self.emit_jump(Opcode::JumpIfNullish(0)),
                    _ => unreachable!(),
                };
                self.emit(Opcode::Pop);
                if matches!(assign.op(), AssignOpNode::Coalesce) {
                    self.emit(Opcode::GetLocal(obj_temp));
                    self.emit(Opcode::GetLocal(key_temp));
                    self.emit_property_get(kind);
                    let skip = self.emit_jump(Opcode::Jump(0));
                    let assign_start = self.code.len();
                    self.patch_jump(jump, assign_start)?;
                    self.emit(Opcode::Pop);
                    self.compile_expression(assign.rhs())?;
                    self.emit(Opcode::Dup);
                    let value_slot = self.allocate_hidden_local()?;
                    self.emit(Opcode::SetLocal(value_slot));
                    self.emit(Opcode::GetLocal(obj_temp));
                    self.emit(Opcode::GetLocal(key_temp));
                    self.emit(Opcode::GetLocal(value_slot));
                    self.emit_property_set(kind);
                    let end = self.code.len();
                    self.patch_jump(skip, end)?;
                    return Ok(());
                }
                self.compile_expression(assign.rhs())?;
                self.emit(Opcode::Dup);
                let value_slot = self.allocate_hidden_local()?;
                self.emit(Opcode::SetLocal(value_slot));
                self.emit(Opcode::GetLocal(obj_temp));
                self.emit(Opcode::GetLocal(key_temp));
                self.emit(Opcode::GetLocal(value_slot));
                self.emit_property_set(kind);
                let end = self.code.len();
                self.patch_jump(jump, end)?;
                Ok(())
            }
            AssignTargetNode::Pattern(_) => Err(CompileError::message(
                "logical assignment does not support pattern targets",
            )),
        }
    }

    fn compile_nullish_expression(
        &mut self,
        lhs: &ExpressionNode,
        rhs: &ExpressionNode,
    ) -> Result<(), CompileError> {
        self.compile_expression(lhs)?;
        self.emit(Opcode::Dup);
        let use_rhs = self.emit_jump(Opcode::JumpIfNullish(0));
        self.emit(Opcode::Pop);
        let end = self.emit_jump(Opcode::Jump(0));
        let rhs_start = self.code.len();
        self.patch_jump(use_rhs, rhs_start)?;
        self.emit(Opcode::Pop);
        self.compile_expression(rhs)?;
        let end_index = self.code.len();
        self.patch_jump(end, end_index)?;
        Ok(())
    }

    fn compile_regexp_literal(
        &mut self,
        regexp: &super::ast::RegexLiteral,
    ) -> Result<(), CompileError> {
        let index = self.add_constant(Constant::RegExp {
            pattern: self.program.resolve_sym(regexp.pattern()),
            flags: self.program.resolve_sym(regexp.flags()),
        })?;
        self.emit(Opcode::MakeRegExp(index));
        Ok(())
    }

    fn compile_pattern_assignment_expression(
        &mut self,
        pattern: &super::ast::PatternNode,
        operator: AssignOpNode,
        rhs: &ExpressionNode,
    ) -> Result<(), CompileError> {
        if operator != AssignOpNode::Assign {
            return Err(CompileError::message(
                "destructuring assignment only supports '='",
            ));
        }
        self.compile_expression(rhs)?;
        let temp = self.allocate_hidden_local()?;
        self.emit(Opcode::Dup);
        self.emit(Opcode::SetLocal(temp));
        self.compile_pattern_store(
            pattern,
            temp,
            BindingStorage::Assignment,
            DeclarationContext::Statement,
        )?;
        Ok(())
    }

    fn compile_switch_statement(
        &mut self,
        statement: &super::ast::SwitchStatement,
    ) -> Result<(), CompileError> {
        let discriminant_slot = self.allocate_hidden_local()?;
        self.compile_expression(statement.val())?;
        self.emit(Opcode::SetLocal(discriminant_slot));

        let mut case_jump_indices = Vec::new();
        let mut default_jump = None;
        let mut case_starts = vec![0usize; statement.cases().len()];

        for (index, case) in statement.cases().iter().enumerate() {
            if let Some(test) = case.condition() {
                self.emit(Opcode::GetLocal(discriminant_slot));
                self.compile_expression(test)?;
                self.emit(Opcode::StrictEq);
                case_jump_indices.push((index, self.emit_jump(Opcode::JumpIfTruePop(0))));
            } else {
                default_jump = Some(self.emit_jump(Opcode::Jump(0)));
            }
        }

        let to_end = self.emit_jump(Opcode::Jump(0));
        self.control_stack.push(ControlContext {
            break_jumps: Vec::new(),
            continue_jumps: Vec::new(),
            is_loop: false,
        });

        for (index, case) in statement.cases().iter().enumerate() {
            case_starts[index] = self.code.len();
            for item in case.body().statements() {
                let statement = statement_list_item_to_node(item.clone());
                self.compile_statement(&statement)?;
            }
        }

        let switch_end = self.code.len();
        let default_target = statement
            .cases()
            .iter()
            .position(|case| case.condition().is_none())
            .and_then(|index| case_starts.get(index).copied())
            .unwrap_or(switch_end);
        self.patch_jump(to_end, default_target)?;
        for (index, jump) in case_jump_indices {
            self.patch_jump(jump, case_starts[index])?;
        }
        if let Some(default_jump) = default_jump {
            self.patch_jump(default_jump, default_target)?;
        }
        let context = self
            .control_stack
            .pop()
            .expect("switch context should exist");
        for jump in context.break_jumps {
            self.patch_jump(jump, switch_end)?;
        }
        Ok(())
    }

    fn compile_for_in_statement(
        &mut self,
        statement: &super::ast::ForInStatement,
    ) -> Result<(), CompileError> {
        self.push_scope();
        self.compile_expression(statement.target())?;
        self.emit(Opcode::GetForInKeys);
        let keys_slot = self.allocate_hidden_local()?;
        let index_slot = self.allocate_hidden_local()?;
        let value_slot = self.allocate_hidden_local()?;
        let zero = self.add_number_constant(0.0)?;
        self.emit(Opcode::SetLocal(keys_slot));
        self.emit(Opcode::LoadConst(zero));
        self.emit(Opcode::SetLocal(index_slot));

        let loop_start = self.code.len();
        self.emit(Opcode::GetLocal(index_slot));
        self.emit(Opcode::GetLocal(keys_slot));
        let length_name = self.add_string_constant("length")?;
        self.emit(Opcode::LoadConst(length_name));
        self.emit(Opcode::GetProp);
        self.emit(Opcode::Lt);
        let exit_jump = self.emit_jump(Opcode::JumpIfFalsePop(0));
        self.emit(Opcode::GetLocal(keys_slot));
        self.emit(Opcode::GetLocal(index_slot));
        self.emit(Opcode::GetIndex);
        self.emit(Opcode::SetLocal(value_slot));
        self.compile_iterable_initializer_store(statement.initializer(), value_slot)?;
        self.control_stack.push(ControlContext {
            break_jumps: Vec::new(),
            continue_jumps: Vec::new(),
            is_loop: true,
        });
        let body = super::ast::statement_to_node(statement.body().clone());
        self.compile_statement(&body)?;
        let increment_start = self.code.len();
        let context = self
            .control_stack
            .pop()
            .expect("for-in control context should exist");
        for jump in context.continue_jumps {
            self.patch_jump(jump, increment_start)?;
        }
        let one = self.add_number_constant(1.0)?;
        self.emit(Opcode::GetLocal(index_slot));
        self.emit(Opcode::LoadConst(one));
        self.emit(Opcode::Add);
        self.emit(Opcode::SetLocal(index_slot));
        self.emit_back_jump(loop_start)?;
        let loop_end = self.code.len();
        self.patch_jump(exit_jump, loop_end)?;
        for jump in context.break_jumps {
            self.patch_jump(jump, loop_end)?;
        }
        self.pop_scope();
        Ok(())
    }

    fn compile_for_of_statement(
        &mut self,
        statement: &super::ast::ForOfStatement,
    ) -> Result<(), CompileError> {
        if statement.r#await() {
            return Err(CompileError::Unimplemented("for await...of statements"));
        }
        self.push_scope();
        self.compile_expression(statement.iterable())?;
        self.emit(Opcode::GetForOfIterator);
        let iter_slot = self.allocate_hidden_local()?;
        let value_slot = self.allocate_hidden_local()?;
        let done_slot = self.allocate_hidden_local()?;
        self.emit(Opcode::SetLocal(iter_slot));

        let loop_start = self.code.len();
        self.emit(Opcode::GetLocal(iter_slot));
        self.emit(Opcode::ForOfNext);
        self.emit(Opcode::SetLocal(done_slot));
        self.emit(Opcode::SetLocal(value_slot));
        self.emit(Opcode::GetLocal(done_slot));
        let exit_jump = self.emit_jump(Opcode::JumpIfTruePop(0));
        self.compile_iterable_initializer_store(statement.initializer(), value_slot)?;
        self.control_stack.push(ControlContext {
            break_jumps: Vec::new(),
            continue_jumps: Vec::new(),
            is_loop: true,
        });
        let body = super::ast::statement_to_node(statement.body().clone());
        self.compile_statement(&body)?;
        let increment_start = self.code.len();
        let context = self
            .control_stack
            .pop()
            .expect("for-of control context should exist");
        for jump in context.continue_jumps {
            self.patch_jump(jump, increment_start)?;
        }
        self.emit_back_jump(loop_start)?;
        let loop_end = self.code.len();
        self.patch_jump(exit_jump, loop_end)?;
        for jump in context.break_jumps {
            self.patch_jump(jump, loop_end)?;
        }
        self.pop_scope();
        Ok(())
    }

    fn compile_iterable_initializer_store(
        &mut self,
        initializer: &IterableLoopInitializerNode,
        value_slot: u16,
    ) -> Result<(), CompileError> {
        match initializer {
            IterableLoopInitializerNode::Identifier(identifier) => {
                let temp_binding = BindingNode::Identifier(*identifier);
                self.compile_binding_store(
                    &temp_binding,
                    value_slot,
                    BindingStorage::Assignment,
                    DeclarationContext::Statement,
                )
            }
            IterableLoopInitializerNode::Access(access) => {
                self.assign_member_from_slot(access, value_slot)
            }
            IterableLoopInitializerNode::Var(variable) => self.compile_binding_store(
                variable.binding(),
                value_slot,
                BindingStorage::Var,
                DeclarationContext::Statement,
            ),
            IterableLoopInitializerNode::Let(binding) => self.compile_binding_store(
                binding,
                value_slot,
                BindingStorage::Let,
                DeclarationContext::Statement,
            ),
            IterableLoopInitializerNode::Const(binding) => self.compile_binding_store(
                binding,
                value_slot,
                BindingStorage::Const,
                DeclarationContext::Statement,
            ),
            IterableLoopInitializerNode::Pattern(pattern) => self.compile_pattern_store(
                pattern,
                value_slot,
                BindingStorage::Assignment,
                DeclarationContext::Statement,
            ),
        }
    }

    fn compile_try_statement(
        &mut self,
        statement: &super::ast::TryStatement,
    ) -> Result<(), CompileError> {
        let handler_index = self.handlers.len();
        self.handlers.push(ExceptionHandler {
            try_start: 0,
            try_end: 0,
            catch_ip: 0,
            catch_binding: None,
            finally_ip: 0,
        });

        let catch_clause = statement.catch();
        let finally_block = statement.finally().map(|finally| finally.block());

        self.emit(Opcode::EnterTry(u16::try_from(handler_index).map_err(
            |_| CompileError::message("exception handler count exceeded u16"),
        )?));
        let try_start = self.code.len();
        if let Some(finally_block) = finally_block {
            self.active_finally_blocks.push(finally_block.clone());
        }
        self.compile_inline_block(statement.block())?;
        if finally_block.is_some() {
            self.active_finally_blocks.pop();
        }
        let try_end = self.code.len();
        self.emit(Opcode::LeaveTry);
        let try_success_jump = self.emit_jump(Opcode::Jump(0));

        let mut catch_ip = 0u32;
        let mut catch_binding = None;
        let mut catch_handler_range = None;

        if let Some(catch_clause) = catch_clause {
            self.push_scope();
            let catch_start = self.code.len();
            catch_ip = u32::try_from(catch_start)
                .map_err(|_| CompileError::message("bytecode offset exceeded u32"))?;
            if let Some(binding) = catch_clause.parameter() {
                let slot = match binding {
                    BindingNode::Identifier(identifier) => {
                        let name = self.identifier_name(identifier);
                        self.declare_block_scoped(&name)?
                    }
                    BindingNode::Pattern(_) => self.allocate_hidden_local()?,
                };
                catch_binding = Some(slot);
                if let BindingNode::Pattern(pattern) = binding {
                    self.compile_pattern_store(
                        pattern,
                        slot,
                        BindingStorage::Let,
                        DeclarationContext::Statement,
                    )?;
                }
            }
            if let Some(finally_block) = finally_block {
                self.active_finally_blocks.push(finally_block.clone());
            }
            for item in catch_clause.block().statement_list().statements() {
                let statement = statement_list_item_to_node(item.clone());
                self.compile_statement(&statement)?;
            }
            if finally_block.is_some() {
                self.active_finally_blocks.pop();
            }
            let catch_end = self.code.len();
            catch_handler_range = Some((catch_start, catch_end));
            self.pop_scope();
        }

        let finally_start = if let Some(finally_block) = finally_block {
            let start = self.code.len();
            self.compile_inline_block(finally_block)?;
            self.emit(Opcode::EndFinally);
            u32::try_from(start)
                .map_err(|_| CompileError::message("bytecode offset exceeded u32"))?
        } else {
            0
        };

        let end = self.code.len();
        self.patch_jump(
            try_success_jump,
            usize::try_from(if finally_start != 0 {
                finally_start
            } else {
                u32::try_from(end)
                    .map_err(|_| CompileError::message("bytecode offset exceeded u32"))?
            })
            .map_err(|_| CompileError::message("bytecode offset exceeded usize"))?,
        )?;

        self.handlers[handler_index] = ExceptionHandler {
            try_start: u32::try_from(try_start)
                .map_err(|_| CompileError::message("bytecode offset exceeded u32"))?,
            try_end: u32::try_from(try_end)
                .map_err(|_| CompileError::message("bytecode offset exceeded u32"))?,
            catch_ip,
            catch_binding,
            finally_ip: finally_start,
        };

        if let (Some((catch_start, catch_end)), true) = (catch_handler_range, finally_start != 0) {
            self.handlers.push(ExceptionHandler {
                try_start: u32::try_from(catch_start)
                    .map_err(|_| CompileError::message("bytecode offset exceeded u32"))?,
                try_end: u32::try_from(catch_end)
                    .map_err(|_| CompileError::message("bytecode offset exceeded u32"))?,
                catch_ip: 0,
                catch_binding: None,
                finally_ip: finally_start,
            });
        }

        Ok(())
    }

    fn compile_class_declaration_statement(
        &mut self,
        class_decl: &ClassDeclarationNode,
    ) -> Result<(), CompileError> {
        let name = self.identifier_name(&class_decl.name());
        self.compile_class_value(
            Some(name.clone()),
            class_decl.super_ref(),
            class_decl.constructor(),
            class_decl.elements(),
        )?;
        let resolved = self.resolve_declaration_binding(
            &name,
            BindingStorage::Let,
            DeclarationContext::Statement,
        )?;
        self.emit_store_binding(&name, resolved)
    }

    fn compile_class_expression(
        &mut self,
        class_expression: &ClassExpressionNode,
    ) -> Result<(), CompileError> {
        self.compile_class_value(
            class_expression
                .name()
                .map(|identifier| self.identifier_name(&identifier)),
            class_expression.super_ref(),
            class_expression.constructor(),
            class_expression.elements(),
        )
    }

    fn compile_class_value(
        &mut self,
        name: Option<String>,
        super_ref: Option<&ExpressionNode>,
        constructor: Option<&super::ast::ClassConstructorExpressionNode>,
        elements: &[ClassElementNode],
    ) -> Result<(), CompileError> {
        let class_slot = self.allocate_hidden_local()?;
        let instance_fields = elements
            .iter()
            .filter_map(|element| match element {
                ClassElementNode::FieldDefinition(field) => Some(field),
                _ => None,
            })
            .collect::<Vec<_>>();

        let mut options = FunctionCompileOptions::default();
        let mut super_ctor_slot = None;
        let mut super_proto_slot = None;
        if let Some(super_ref) = super_ref {
            let ctor_binding_name = format!("__class_super_ctor_{}", self.next_local);
            let proto_binding_name = format!("__class_super_proto_{}", self.next_local + 1);
            let ctor_slot = self.declare_named_hidden_local(ctor_binding_name.clone())?;
            self.compile_expression(super_ref)?;
            self.emit(Opcode::SetLocal(ctor_slot));
            let proto_slot = self.declare_named_hidden_local(proto_binding_name.clone())?;
            let prototype_name = self.add_string_constant("prototype")?;
            self.emit(Opcode::GetLocal(ctor_slot));
            self.emit(Opcode::LoadConst(prototype_name));
            self.emit(Opcode::GetProp);
            self.emit(Opcode::SetLocal(proto_slot));
            options.super_ctor_binding = Some(ctor_binding_name);
            options.super_proto_binding = Some(proto_binding_name);
            super_ctor_slot = Some(ctor_slot);
            super_proto_slot = Some(proto_slot);
        }

        let constructor_index = if let Some(constructor) = constructor {
            self.compile_nested_function_with_options(
                name.clone(),
                constructor.parameters(),
                constructor.body(),
                constructor.body().strict(),
                false,
                false,
                false,
                options.clone(),
                &instance_fields,
            )?
        } else {
            self.compile_synthetic_class_constructor(name.clone(), &options, &instance_fields)?
        };

        self.emit(Opcode::MakeClosure(constructor_index));
        self.emit(Opcode::Dup);
        self.emit(Opcode::SetLocal(class_slot));

        if let (Some(super_ctor_slot), Some(super_proto_slot)) = (super_ctor_slot, super_proto_slot)
        {
            let prototype_name = self.add_string_constant("prototype")?;
            self.emit(Opcode::GetLocal(class_slot));
            self.emit(Opcode::LoadConst(prototype_name));
            self.emit(Opcode::GetProp);
            self.emit(Opcode::GetLocal(super_proto_slot));
            self.emit(Opcode::SetProtoOf);
            self.emit(Opcode::Pop);
            self.emit(Opcode::GetLocal(class_slot));
            self.emit(Opcode::GetLocal(super_ctor_slot));
            self.emit(Opcode::SetProtoOf);
            self.emit(Opcode::Pop);
        }

        for element in elements {
            match element {
                ClassElementNode::MethodDefinition(method) => {
                    self.compile_class_method_definition(class_slot, method, &options)?;
                }
                ClassElementNode::StaticFieldDefinition(field) => {
                    self.compile_static_class_field_initializer(class_slot, field)?;
                }
                ClassElementNode::FieldDefinition(_) => {}
                ClassElementNode::PrivateFieldDefinition(_)
                | ClassElementNode::PrivateStaticFieldDefinition(_) => {
                    return Err(CompileError::Unimplemented("private class fields"));
                }
                ClassElementNode::StaticBlock(_) => {
                    return Err(CompileError::Unimplemented("class static blocks"));
                }
            }
        }

        self.emit(Opcode::GetLocal(class_slot));
        Ok(())
    }

    fn compile_synthetic_class_constructor(
        &mut self,
        name: Option<String>,
        options: &FunctionCompileOptions,
        field_initializers: &[&super::ast::ClassFieldDefinitionNode],
    ) -> Result<u16, CompileError> {
        let outer = Some(self.snapshot_outer_bindings());
        let mut child = FunctionCompiler::new(
            self.program,
            name,
            0,
            self.is_strict,
            false,
            outer,
            options.clone(),
        );
        if options.super_ctor_binding.is_some() {
            let rest_slot = child.allocate_hidden_local()?;
            child.parameter_count = 1;
            child.has_rest_param = true;
            let helper = child.add_string_constant("__callSpread")?;
            let super_name = options
                .super_ctor_binding
                .as_deref()
                .ok_or(CompileError::message("missing super constructor binding"))?;
            let resolved = child.resolve_binding(super_name);
            child.emit(Opcode::GetGlobal(helper));
            child.emit(Opcode::LoadUndefined);
            child.emit_load_binding(super_name, resolved)?;
            child.emit(Opcode::LoadThis);
            child.emit(Opcode::GetLocal(rest_slot));
            child.emit(Opcode::Call(3));
            child.emit(Opcode::Pop);
        }
        child.install_this_binding()?;
        for field in field_initializers {
            child.compile_class_field_initializer(field)?;
        }
        child.emit_implicit_return();
        let index = self.nested_functions.len();
        self.nested_functions.push(child.finish());
        u16::try_from(index)
            .map_err(|_| CompileError::message("nested function count exceeded u16"))
    }

    fn compile_class_method_definition(
        &mut self,
        class_slot: u16,
        method: &super::ast::ClassMethodDefinitionNode,
        options: &FunctionCompileOptions,
    ) -> Result<(), CompileError> {
        if method.is_private() {
            return Err(CompileError::Unimplemented("private class methods"));
        }
        match method.kind() {
            MethodDefinitionKindNode::Ordinary => {}
            MethodDefinitionKindNode::Get | MethodDefinitionKindNode::Set => {
                return Err(CompileError::Unimplemented("class accessors"));
            }
            MethodDefinitionKindNode::Generator => {
                return Err(CompileError::Unimplemented("generator class methods"));
            }
            MethodDefinitionKindNode::Async | MethodDefinitionKindNode::AsyncGenerator => {
                return Err(CompileError::Unimplemented("async class methods"));
            }
        }
        let nested_index = self.compile_nested_function_with_options(
            Some(self.class_element_name_string(method.name())),
            method.parameters(),
            method.body(),
            method.body().strict(),
            false,
            false,
            false,
            options.clone(),
            &[],
        )?;

        if method.is_static() {
            self.emit(Opcode::GetLocal(class_slot));
        } else {
            let prototype_name = self.add_string_constant("prototype")?;
            self.emit(Opcode::GetLocal(class_slot));
            self.emit(Opcode::LoadConst(prototype_name));
            self.emit(Opcode::GetProp);
        }
        self.compile_class_element_name_value(method.name())?;
        self.emit(Opcode::MakeClosure(nested_index));
        self.emit(Opcode::SetProp);
        Ok(())
    }

    fn class_element_name_string(&self, name: &ClassElementNameNode) -> String {
        match name {
            ClassElementNameNode::PropertyName(property_name) => self
                .property_name_string(property_name)
                .unwrap_or_else(|| "<computed>".to_string()),
            ClassElementNameNode::PrivateName(_) => "#private".to_string(),
        }
    }

    fn compile_class_element_name_value(
        &mut self,
        name: &ClassElementNameNode,
    ) -> Result<(), CompileError> {
        match name {
            ClassElementNameNode::PropertyName(property_name) => {
                self.compile_property_name_value(property_name)
            }
            ClassElementNameNode::PrivateName(_) => {
                Err(CompileError::Unimplemented("private class elements"))
            }
        }
    }

    fn compile_class_field_initializer(
        &mut self,
        field: &super::ast::ClassFieldDefinitionNode,
    ) -> Result<(), CompileError> {
        self.emit(Opcode::LoadThis);
        self.compile_property_name_value(field.name())?;
        if let Some(initializer) = field.initializer() {
            self.compile_expression(initializer)?;
        } else {
            self.emit(Opcode::LoadUndefined);
        }
        self.emit(Opcode::SetProp);
        Ok(())
    }

    fn compile_static_class_field_initializer(
        &mut self,
        class_slot: u16,
        field: &super::ast::ClassFieldDefinitionNode,
    ) -> Result<(), CompileError> {
        self.emit(Opcode::GetLocal(class_slot));
        self.compile_property_name_value(field.name())?;
        if let Some(initializer) = field.initializer() {
            self.compile_expression(initializer)?;
        } else {
            self.emit(Opcode::LoadUndefined);
        }
        self.emit(Opcode::SetProp);
        Ok(())
    }

    fn compile_super_call(&mut self, call: &SuperCallExpression) -> Result<(), CompileError> {
        if call
            .arguments()
            .iter()
            .any(|argument| matches!(argument, ExpressionNode::Spread(_)))
        {
            let helper = self.add_string_constant("__callSpread")?;
            self.emit(Opcode::GetGlobal(helper));
            self.emit(Opcode::LoadUndefined);
            let super_name = self
                .options
                .super_ctor_binding
                .clone()
                .ok_or_else(|| CompileError::message("super() used outside a derived class"))?;
            let resolved = self.resolve_binding(&super_name);
            self.emit_load_binding(&super_name, resolved)?;
            self.emit(Opcode::LoadThis);
            let args_slot = self.compile_argument_array(call.arguments())?;
            self.emit(Opcode::GetLocal(args_slot));
            self.emit(Opcode::Call(3));
            return Ok(());
        }

        let super_name = self
            .options
            .super_ctor_binding
            .clone()
            .ok_or_else(|| CompileError::message("super() used outside a derived class"))?;
        let resolved = self.resolve_binding(&super_name);
        self.emit_load_binding(&super_name, resolved)?;
        self.emit(Opcode::LoadThis);
        let argc = u8::try_from(call.arguments().len())
            .map_err(|_| CompileError::message("super call argument count exceeded u8"))?;
        for argument in call.arguments() {
            self.compile_expression(argument)?;
        }
        self.emit(Opcode::Call(argc));
        Ok(())
    }

    fn compile_super_property_access(
        &mut self,
        access: &super::ast::SuperPropertyAccessNode,
    ) -> Result<(), CompileError> {
        let super_name = self
            .options
            .super_proto_binding
            .clone()
            .ok_or_else(|| CompileError::message("super property used outside a method"))?;
        let resolved = self.resolve_binding(&super_name);
        self.emit_load_binding(&super_name, resolved)?;
        match access.field() {
            PropertyAccessFieldNode::Const(identifier) => {
                let constant = self.add_string_constant(self.identifier_name(identifier))?;
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

    fn compile_super_property_for_call(
        &mut self,
        access: &super::ast::SuperPropertyAccessNode,
    ) -> Result<(), CompileError> {
        let super_name = self
            .options
            .super_proto_binding
            .clone()
            .ok_or_else(|| CompileError::message("super property used outside a method"))?;
        let resolved = self.resolve_binding(&super_name);
        self.emit_load_binding(&super_name, resolved)?;
        match access.field() {
            PropertyAccessFieldNode::Const(identifier) => {
                let constant = self.add_string_constant(self.identifier_name(identifier))?;
                self.emit(Opcode::LoadConst(constant));
                self.emit(Opcode::GetProp);
            }
            PropertyAccessFieldNode::Expr(expression) => {
                self.compile_expression(expression)?;
                self.emit(Opcode::GetIndex);
            }
        }
        self.emit(Opcode::LoadThis);
        Ok(())
    }

    fn compile_super_property_access_temps(
        &mut self,
        access: &super::ast::SuperPropertyAccessNode,
        obj_temp: u16,
        key_temp: u16,
    ) -> Result<PropertyOpKind, CompileError> {
        let super_name = self
            .options
            .super_proto_binding
            .clone()
            .ok_or_else(|| CompileError::message("super property used outside a method"))?;
        let resolved = self.resolve_binding(&super_name);
        self.emit_load_binding(&super_name, resolved)?;
        self.emit(Opcode::SetLocal(obj_temp));
        match access.field() {
            PropertyAccessFieldNode::Const(identifier) => {
                let constant = self.add_string_constant(self.identifier_name(identifier))?;
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

    fn compile_optional_expression(
        &mut self,
        optional: &super::ast::OptionalExpression,
    ) -> Result<(), CompileError> {
        let current_slot = self.allocate_hidden_local()?;
        let receiver_slot = self.allocate_hidden_local()?;
        let mut short_jumps = Vec::new();
        let mut last_was_property = false;

        self.compile_expression(optional.target())?;
        self.emit(Opcode::SetLocal(current_slot));
        self.emit(Opcode::LoadUndefined);
        self.emit(Opcode::SetLocal(receiver_slot));

        for operation in optional.chain() {
            if operation.shorted() {
                self.emit(Opcode::GetLocal(current_slot));
                short_jumps.push(self.emit_jump(Opcode::JumpIfNullish(0)));
                self.emit(Opcode::Pop);
            }

            match operation.kind() {
                OptionalOperationKindNode::SimplePropertyAccess { field } => {
                    self.emit(Opcode::GetLocal(current_slot));
                    self.emit(Opcode::SetLocal(receiver_slot));
                    self.emit(Opcode::GetLocal(current_slot));
                    match field {
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
                    self.emit(Opcode::SetLocal(current_slot));
                    last_was_property = true;
                }
                OptionalOperationKindNode::PrivatePropertyAccess { .. } => {
                    return Err(CompileError::Unimplemented("private optional chaining"));
                }
                OptionalOperationKindNode::Call { args } => {
                    if args
                        .iter()
                        .any(|argument| matches!(argument, ExpressionNode::Spread(_)))
                    {
                        let helper = self.add_string_constant("__callSpread")?;
                        self.emit(Opcode::GetGlobal(helper));
                        self.emit(Opcode::LoadUndefined);
                        self.emit(Opcode::GetLocal(current_slot));
                        if last_was_property {
                            self.emit(Opcode::GetLocal(receiver_slot));
                        } else {
                            self.emit(Opcode::LoadUndefined);
                        }
                        let args_slot = self.compile_argument_array(args)?;
                        self.emit(Opcode::GetLocal(args_slot));
                        self.emit(Opcode::Call(3));
                    } else {
                        self.emit(Opcode::GetLocal(current_slot));
                        if last_was_property {
                            self.emit(Opcode::GetLocal(receiver_slot));
                        } else {
                            self.emit(Opcode::LoadUndefined);
                        }
                        let argc = u8::try_from(args.len()).map_err(|_| {
                            CompileError::message("optional call arity exceeded u8")
                        })?;
                        for argument in args.iter() {
                            self.compile_expression(argument)?;
                        }
                        self.emit(Opcode::Call(argc));
                    }
                    self.emit(Opcode::SetLocal(current_slot));
                    last_was_property = false;
                }
            }
        }

        self.emit(Opcode::GetLocal(current_slot));
        let end_jump = self.emit_jump(Opcode::Jump(0));
        let short_target = self.code.len();
        for jump in short_jumps {
            self.patch_jump(jump, short_target)?;
        }
        self.emit(Opcode::Pop);
        self.emit(Opcode::LoadUndefined);
        let end = self.code.len();
        self.patch_jump(end_jump, end)?;
        Ok(())
    }

    fn compile_optional_call(
        &mut self,
        optional: &super::ast::OptionalExpression,
        call: &super::ast::CallExpression,
    ) -> Result<(), CompileError> {
        self.compile_optional_expression(optional)?;
        self.emit(Opcode::LoadUndefined);
        if call
            .args()
            .iter()
            .any(|argument| matches!(argument, ExpressionNode::Spread(_)))
        {
            let helper = self.add_string_constant("__callSpread")?;
            let callee_slot = self.allocate_hidden_local()?;
            self.emit(Opcode::SetLocal(callee_slot));
            self.emit(Opcode::GetGlobal(helper));
            self.emit(Opcode::LoadUndefined);
            self.emit(Opcode::GetLocal(callee_slot));
            self.emit(Opcode::LoadUndefined);
            let args_slot = self.compile_argument_array(call.args())?;
            self.emit(Opcode::GetLocal(args_slot));
            self.emit(Opcode::Call(3));
            return Ok(());
        }
        let argc = u8::try_from(call.args().len())
            .map_err(|_| CompileError::message("call argument count exceeded u8"))?;
        for argument in call.args() {
            self.compile_expression(argument)?;
        }
        self.emit(Opcode::Call(argc));
        Ok(())
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
            StatementNode::ClassDeclaration(class_decl) => {
                self.compile_class_declaration_statement(class_decl)
            }
            StatementNode::BlockStatement(block) => self.compile_block_statement(block),
            StatementNode::IfStatement(statement) => self.compile_if_statement(statement),
            StatementNode::SwitchStatement(statement) => self.compile_switch_statement(statement),
            StatementNode::ForStatement(statement) => self.compile_for_statement(statement),
            StatementNode::ForInStatement(statement) => self.compile_for_in_statement(statement),
            StatementNode::ForOfStatement(statement) => self.compile_for_of_statement(statement),
            StatementNode::WhileStatement(statement) => self.compile_while_statement(statement),
            StatementNode::DoWhileStatement(statement) => {
                self.compile_do_while_statement(statement)
            }
            StatementNode::TryStatement(statement) => self.compile_try_statement(statement),
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
                let return_slot = self.allocate_hidden_local()?;
                self.emit(Opcode::SetLocal(return_slot));
                self.emit_active_finally_blocks()?;
                self.emit(Opcode::GetLocal(return_slot));
                self.emit(if self.is_async {
                    Opcode::AsyncReturn
                } else {
                    Opcode::Return
                });
                Ok(())
            }
            StatementNode::BreakStatement(_) => {
                self.emit_active_finally_blocks()?;
                let jump = self.emit_jump(Opcode::Jump(0));
                let context =
                    self.control_stack.iter_mut().rev().next().ok_or_else(|| {
                        CompileError::message("break used outside a loop or switch")
                    })?;
                context.break_jumps.push(jump);
                Ok(())
            }
            StatementNode::ContinueStatement(_) => {
                self.emit_active_finally_blocks()?;
                let jump = self.emit_jump(Opcode::Jump(0));
                let loop_context = self
                    .control_stack
                    .iter_mut()
                    .rev()
                    .find(|context| context.is_loop)
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
        self.control_stack.push(ControlContext {
            break_jumps: Vec::new(),
            continue_jumps: Vec::new(),
            is_loop: true,
        });
        let body = super::ast::statement_to_node(statement.body().clone());
        self.compile_statement(&body)?;
        let loop_context = self.control_stack.pop().expect("loop context should exist");
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
        self.control_stack.push(ControlContext {
            break_jumps: Vec::new(),
            continue_jumps: Vec::new(),
            is_loop: true,
        });
        let body = super::ast::statement_to_node(statement.body().clone());
        self.compile_statement(&body)?;
        let condition_start = self.code.len();
        let loop_context = self.control_stack.pop().expect("loop context should exist");
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

        self.control_stack.push(ControlContext {
            break_jumps: Vec::new(),
            continue_jumps: Vec::new(),
            is_loop: true,
        });
        let body = super::ast::statement_to_node(statement.body().clone());
        self.compile_statement(&body)?;

        let increment_start = self.code.len();
        let loop_context = self.control_stack.pop().expect("loop context should exist");
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
        let storage = if declaration.is_var() {
            BindingStorage::Var
        } else if declaration.is_const() {
            BindingStorage::Const
        } else {
            BindingStorage::Let
        };

        for variable in declaration.variables() {
            if let Some(initializer) = variable.init() {
                self.compile_expression(initializer)?;
                let value_slot = self.allocate_hidden_local()?;
                self.emit(Opcode::SetLocal(value_slot));
                self.compile_binding_store(variable.binding(), value_slot, storage, context)?;
            } else {
                match variable.binding() {
                    BindingNode::Identifier(identifier) => {
                        let name = self.identifier_name(identifier);
                        let resolved = self.resolve_declaration_binding(&name, storage, context)?;
                        if declaration.is_const() {
                            return Err(CompileError::message(format!(
                                "const declaration '{name}' requires an initializer"
                            )));
                        }
                        if matches!(resolved, ResolvedBinding::Global) {
                            self.emit(Opcode::LoadUndefined);
                            self.emit_store_binding(&name, resolved)?;
                        }
                    }
                    BindingNode::Pattern(_) => {
                        return Err(CompileError::message(format!(
                            "{} destructuring declarations require an initializer",
                            declaration.keyword()
                        )));
                    }
                }
            }
        }
        Ok(())
    }

    fn compile_function_declaration_statement(
        &mut self,
        declaration: &FunctionDeclaration,
    ) -> Result<(), CompileError> {
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
            declaration.is_async(),
            declaration.is_generator(),
            false,
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
            FunctionCompileOptions::default(),
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
    ) -> Result<u16, CompileError> {
        if is_generator {
            return Err(CompileError::Unimplemented("generator functions"));
        }
        let arity = u8::try_from(parameters.length())
            .map_err(|_| CompileError::message("function arity exceeded u8"))?;
        let outer = Some(self.snapshot_outer_bindings());
        let mut child =
            FunctionCompiler::new(self.program, name, arity, is_strict, false, outer, options);
        child.is_async = is_async;
        child.is_generator = is_generator;
        child.compile_function_parameters(parameters)?;
        if !is_arrow {
            child.install_this_binding()?;
        }
        for field in field_initializers {
            child.compile_class_field_initializer(field)?;
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
                    ResolvedBinding::Global => self.emit(Opcode::LoadThis),
                };
                Ok(())
            }
            ExpressionNode::Identifier(identifier) => {
                let name = self.identifier_name(identifier);
                let resolved = self.resolve_binding(&name);
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
            ExpressionNode::GeneratorExpression(_) => {
                Err(CompileError::Unimplemented("generator expressions"))
            }
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
            ExpressionNode::AsyncGeneratorExpression(_) => {
                Err(CompileError::Unimplemented("async generator expressions"))
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
                    self.compile_property_name_value(name)?;
                    self.compile_expression(value)?;
                    self.emit(Opcode::SetProp);
                }
                ObjectPropertyDefinition::MethodDefinition(method) => {
                    self.compile_property_name_value(method.name())?;
                    self.compile_object_method_value(method)?;
                    self.emit(Opcode::SetProp);
                }
                ObjectPropertyDefinition::SpreadObject(expression) => {
                    self.compile_expression(expression)?;
                    self.emit(Opcode::CopyDataProperties);
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
            super::ast::MemberExpression::Private(_) => {
                Err(CompileError::Unimplemented("private property access"))
            }
            super::ast::MemberExpression::Super(access) => {
                self.compile_super_property_access(access)
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
        is_async: bool,
        is_generator: bool,
        is_arrow: bool,
    ) -> Result<(), CompileError> {
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
            super::ast::MemberExpression::Private(_) => {
                Err(CompileError::Unimplemented("private method calls"))
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
            UnaryOpNode::Plus => {
                self.emit(Opcode::ToNumber);
            }
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
