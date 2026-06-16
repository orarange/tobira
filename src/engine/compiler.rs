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
        // Emit innermost finally first. While emitting finally[i], only the OUTER
        // finallys (0..i) stay active, so a `return`/`break` inside a finally does
        // not re-enter that same finally (which would recurse forever) — it only
        // runs the remaining outer ones.
        let blocks = self.active_finally_blocks.clone();
        let saved = std::mem::take(&mut self.active_finally_blocks);
        let mut result = Ok(());
        for index in (0..blocks.len()).rev() {
            self.active_finally_blocks = blocks[0..index].to_vec();
            if let Err(error) = self.compile_inline_block(&blocks[index]) {
                result = Err(error);
                break;
            }
        }
        self.active_finally_blocks = saved;
        result
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
                    let binding_slot = match resolved {
                        ResolvedBinding::Local(slot) if slot == raw_slot => Some(slot),
                        ResolvedBinding::Local(slot) => {
                            self.emit(Opcode::GetLocal(raw_slot));
                            self.emit(Opcode::SetLocal(slot));
                            Some(slot)
                        }
                        _ => {
                            self.emit(Opcode::GetLocal(raw_slot));
                            self.emit_store_binding(&name, resolved)?;
                            None
                        }
                    };
                    // Default value: applied left-to-right after binding so a
                    // later default can reference an earlier parameter.
                    if let Some(initializer) = parameter.init() {
                        let slot = binding_slot.unwrap_or(raw_slot);
                        self.apply_param_default(slot, initializer)?;
                    }
                }
                BindingNode::Pattern(pattern) => {
                    // Apply the default to the raw argument slot before destructuring.
                    if let Some(initializer) = parameter.init() {
                        self.apply_param_default(raw_slot, initializer)?;
                    }
                    pending.push(PendingPatternInit {
                        pattern: pattern.clone(),
                        slot: raw_slot,
                        storage: BindingStorage::Let,
                    });
                }
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

    /// Emit `if (slot === undefined) slot = <initializer>` for a default
    /// parameter value.
    fn apply_param_default(
        &mut self,
        slot: u16,
        initializer: &ExpressionNode,
    ) -> Result<(), CompileError> {
        self.emit(Opcode::GetLocal(slot));
        self.emit(Opcode::LoadUndefined);
        self.emit(Opcode::StrictEq);
        let skip = self.emit_jump(Opcode::JumpIfFalsePop(0));
        self.compile_expression(initializer)?;
        self.emit(Opcode::SetLocal(slot));
        let target = self.code.len();
        self.patch_jump(skip, target)?;
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
        // Static names destructured by non-rest elements; a trailing `...rest`
        // must exclude these. Computed keys can't be excluded statically (rare).
        let rest_exclude: Vec<String> = pattern
            .bindings()
            .iter()
            .filter_map(|property| match property {
                ObjectPatternElementNode::SingleName { name, .. }
                | ObjectPatternElementNode::Pattern { name, .. }
                | ObjectPatternElementNode::AssignmentPropertyAccess { name, .. } => {
                    self.property_name_string(name)
                }
                _ => None,
            })
            .collect();
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
                    let rest_slot = self.copy_slot_to_object(source_slot, &rest_exclude)?;
                    let temp_binding = BindingNode::Identifier(*ident);
                    self.compile_binding_store(&temp_binding, rest_slot, storage, context)?;
                }
                ObjectPatternElementNode::AssignmentRestPropertyAccess { access } => {
                    let rest_slot = self.copy_slot_to_object(source_slot, &rest_exclude)?;
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
        // Normalize the source to a real array via `Array.from` so that array
        // destructuring follows the iterator protocol and works on any iterable
        // (Set, Map, string, generators, custom iterables), not just arrays.
        // The existing index-based extraction below then operates on the
        // materialized array. `Array.from` is lenient on non-iterables (it
        // yields an empty array), which preserves the prior behavior for
        // array-likes and primitives.
        let array_slot = self.allocate_hidden_local()?;
        let array_name = self.add_string_constant("Array")?;
        let from_name = self.add_string_constant("from")?;
        self.emit(Opcode::GetGlobal(array_name));
        self.emit(Opcode::GetPropForCall(from_name));
        self.emit(Opcode::GetLocal(source_slot));
        self.emit(Opcode::Call(1));
        self.emit(Opcode::SetLocal(array_slot));
        for (index, element) in pattern.bindings().iter().enumerate() {
            match element {
                ArrayPatternElementNode::Elision => {}
                ArrayPatternElementNode::SingleName {
                    ident,
                    default_init,
                } => {
                    let value_slot = self.extract_array_index_to_slot(array_slot, index as u32)?;
                    let value_slot =
                        self.apply_default_initializer_slot(value_slot, default_init.as_ref())?;
                    let temp_binding = BindingNode::Identifier(*ident);
                    self.compile_binding_store(&temp_binding, value_slot, storage, context)?;
                }
                ArrayPatternElementNode::PropertyAccess {
                    access,
                    default_init,
                } => {
                    let value_slot = self.extract_array_index_to_slot(array_slot, index as u32)?;
                    let value_slot =
                        self.apply_default_initializer_slot(value_slot, default_init.as_ref())?;
                    self.assign_member_from_slot(access, value_slot)?;
                }
                ArrayPatternElementNode::Pattern {
                    pattern,
                    default_init,
                } => {
                    let value_slot = self.extract_array_index_to_slot(array_slot, index as u32)?;
                    let value_slot =
                        self.apply_default_initializer_slot(value_slot, default_init.as_ref())?;
                    self.compile_pattern_store(pattern, value_slot, storage, context)?;
                }
                ArrayPatternElementNode::SingleNameRest { ident } => {
                    let rest_slot = self.slice_array_slot(array_slot, index as u32)?;
                    let temp_binding = BindingNode::Identifier(*ident);
                    self.compile_binding_store(&temp_binding, rest_slot, storage, context)?;
                }
                ArrayPatternElementNode::PropertyAccessRest { access } => {
                    let rest_slot = self.slice_array_slot(array_slot, index as u32)?;
                    self.assign_member_from_slot(access, rest_slot)?;
                }
                ArrayPatternElementNode::PatternRest { pattern } => {
                    let rest_slot = self.slice_array_slot(array_slot, index as u32)?;
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

    /// Build the object for an object-rest pattern (`const { a, ...rest } = src`):
    /// shallow-copy `src`'s own enumerable properties into a fresh object, then
    /// delete the keys that were already destructured before the rest (`exclude`),
    /// so `rest` holds only the remaining properties.
    fn copy_slot_to_object(
        &mut self,
        source_slot: u16,
        exclude: &[String],
    ) -> Result<u16, CompileError> {
        let slot = self.allocate_hidden_local()?;
        self.emit(Opcode::MakeObject);
        self.emit(Opcode::SetLocal(slot));
        self.emit(Opcode::GetLocal(slot));
        self.emit(Opcode::GetLocal(source_slot));
        self.emit(Opcode::CopyDataProperties);
        // CopyDataProperties pushes the target back; we keep it in `slot`, so drop
        // the stack copy to stay balanced.
        self.emit(Opcode::Pop);
        for key in exclude {
            let constant = self.add_string_constant(key.clone())?;
            self.emit(Opcode::GetLocal(slot));
            self.emit(Opcode::LoadConst(constant));
            self.emit(Opcode::DeleteProp);
            self.emit(Opcode::Pop); // discard the delete result boolean
        }
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
        // JumpIfNullish *peeks* (does not pop), so the duplicated lhs is still on
        // the stack in both branches.
        let use_rhs = self.emit_jump(Opcode::JumpIfNullish(0));
        // Not nullish: drop the duplicate, keep the original lhs as the result.
        self.emit(Opcode::Pop);
        let end = self.emit_jump(Opcode::Jump(0));
        let rhs_start = self.code.len();
        self.patch_jump(use_rhs, rhs_start)?;
        // Nullish: drop BOTH the duplicate and the original lhs, then evaluate rhs.
        self.emit(Opcode::Pop);
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

        // A switch is a single block. Per spec every function declaration inside
        // its cases is instantiated at block entry, BEFORE the discriminant
        // dispatch picks a case — otherwise a `function g(){}` declared in one
        // case is undefined when control jumps directly to another case (or when
        // a case references a helper declared in a later case). Minified code
        // (Terser) emits exactly this shape in reducers / dispatch tables, so the
        // gap surfaced as "X is not defined". Hoist them here, mirroring
        // `compile_statements`: predeclare all the names first (so sibling helpers
        // cross-reference), then build the closures.
        let case_statements: Vec<StatementNode> = statement
            .cases()
            .iter()
            .flat_map(|case| {
                case.body()
                    .statements()
                    .iter()
                    .map(|item| statement_list_item_to_node(item.clone()))
            })
            .collect();
        if !self.is_top_level {
            for stmt in &case_statements {
                if let StatementNode::FunctionDeclaration(declaration) = stmt {
                    let name = self.identifier_name(&declaration.name());
                    self.declare_function_scoped(&name)?;
                }
            }
        }
        for stmt in &case_statements {
            if let StatementNode::FunctionDeclaration(declaration) = stmt {
                self.compile_function_declaration_statement(declaration)?;
            }
        }

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
        self.push_control_context(false);

        for (index, case) in statement.cases().iter().enumerate() {
            case_starts[index] = self.code.len();
            for item in case.body().statements() {
                let statement = statement_list_item_to_node(item.clone());
                // Function declarations were already hoisted/instantiated at switch
                // entry above; don't re-compile them inline.
                if matches!(statement, StatementNode::FunctionDeclaration(_)) {
                    continue;
                }
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
        self.push_control_context(true);
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
        let is_await = statement.r#await();
        if is_await && !self.is_async {
            return Err(CompileError::message(
                "for await...of is only valid inside async functions",
            ));
        }
        self.push_scope();
        self.compile_expression(statement.iterable())?;
        self.emit(if is_await {
            Opcode::GetForAwaitIterator
        } else {
            Opcode::GetForOfIterator
        });
        let iter_slot = self.allocate_hidden_local()?;
        let value_slot = self.allocate_hidden_local()?;
        let done_slot = self.allocate_hidden_local()?;
        self.emit(Opcode::SetLocal(iter_slot));

        let loop_start = self.code.len();
        if is_await {
            self.emit(Opcode::GetLocal(iter_slot));
            let next_name = self.add_string_constant("next".to_string())?;
            self.emit(Opcode::GetPropForCall(next_name));
            self.emit(Opcode::Call(0));
            self.emit(Opcode::Await);
            self.emit(Opcode::SetLocal(done_slot));
            self.emit(Opcode::GetLocal(done_slot));
            let done_name = self.add_string_constant("done".to_string())?;
            self.emit(Opcode::LoadConst(done_name));
            self.emit(Opcode::GetProp);
            let exit_jump = self.emit_jump(Opcode::JumpIfTruePop(0));
            self.emit(Opcode::GetLocal(done_slot));
            let value_name = self.add_string_constant("value".to_string())?;
            self.emit(Opcode::LoadConst(value_name));
            self.emit(Opcode::GetProp);
            self.emit(Opcode::Await);
            self.emit(Opcode::SetLocal(value_slot));
            self.compile_iterable_initializer_store(statement.initializer(), value_slot)?;
            self.push_control_context(true);
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
        } else {
            self.emit(Opcode::GetLocal(iter_slot));
            self.emit(Opcode::ForOfNext);
            self.emit(Opcode::SetLocal(done_slot));
            self.emit(Opcode::SetLocal(value_slot));
            self.emit(Opcode::GetLocal(done_slot));
            let exit_jump = self.emit_jump(Opcode::JumpIfTruePop(0));
            self.compile_iterable_initializer_store(statement.initializer(), value_slot)?;
            self.push_control_context(true);
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
        let private_instance_fields = elements
            .iter()
            .filter_map(|element| match element {
                ClassElementNode::PrivateFieldDefinition(field) => Some(field),
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
                &private_instance_fields,
            )?
        } else {
            self.compile_synthetic_class_constructor(
                name.clone(),
                &options,
                &instance_fields,
                &private_instance_fields,
            )?
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

        // Establish the class's inner name binding (an immutable binding scoped
        // to the class body) so static blocks and static field initializers can
        // refer to the class by name (e.g. `A`) while it is being defined,
        // before the outer declaration binding is stored.
        let class_body_scope = name.is_some();
        if let Some(class_name) = &name {
            self.push_scope();
            let name_slot = self.declare_named_hidden_local(class_name.clone())?;
            self.emit(Opcode::GetLocal(class_slot));
            self.emit(Opcode::SetLocal(name_slot));
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
                // Instance private fields are initialized inside the constructor.
                ClassElementNode::PrivateFieldDefinition(_) => {}
                ClassElementNode::PrivateStaticFieldDefinition(field) => {
                    self.emit(Opcode::GetLocal(class_slot));
                    let constant = self.add_string_constant(self.private_field_key(field.name()))?;
                    self.emit(Opcode::LoadConst(constant));
                    if let Some(initializer) = field.initializer() {
                        self.compile_expression(initializer)?;
                    } else {
                        self.emit(Opcode::LoadUndefined);
                    }
                    self.emit(Opcode::SetProp);
                }
                ClassElementNode::StaticBlock(block) => {
                    // A static block runs once at class-definition time, in
                    // source order with the static fields. Compile its body
                    // inline in a fresh lexical scope. The class is in scope by
                    // name (e.g. `A`) via the inner name binding established
                    // above. (Note: `this` resolves to the enclosing `this`
                    // rather than the class, matching the static-field
                    // initializers above; rebinding `this` to the class is a
                    // future refinement.)
                    self.push_scope();
                    self.compile_function_body(block.statements())?;
                    self.pop_scope();
                }
            }
        }

        if class_body_scope {
            self.pop_scope();
        }

        self.emit(Opcode::GetLocal(class_slot));
        Ok(())
    }

    fn compile_synthetic_class_constructor(
        &mut self,
        name: Option<String>,
        options: &FunctionCompileOptions,
        field_initializers: &[&super::ast::ClassFieldDefinitionNode],
        private_field_initializers: &[&boa_ast::function::PrivateFieldDefinition],
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
            None,
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
        for field in private_field_initializers {
            child.compile_private_field_initializer(field)?;
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
        // accessor: None = ordinary/generator/async method (SetProp);
        // Some(true/false) = getter/setter.
        let (accessor, is_async, is_generator) = match method.kind() {
            MethodDefinitionKindNode::Ordinary => (None, false, false),
            MethodDefinitionKindNode::Generator => (None, false, true),
            MethodDefinitionKindNode::Async => (None, true, false),
            MethodDefinitionKindNode::Get => (Some(true), false, false),
            MethodDefinitionKindNode::Set => (Some(false), false, false),
            MethodDefinitionKindNode::AsyncGenerator => (None, true, true),
        };
        let nested_index = self.compile_nested_function_with_options(
            Some(self.class_element_name_string(method.name())),
            method.parameters(),
            method.body(),
            method.body().strict(),
            is_async,
            is_generator,
            false,
            options.clone(),
            &[],
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
        match accessor {
            None => self.emit(Opcode::SetProp),
            Some(true) => self.emit(Opcode::DefineGetter),
            Some(false) => self.emit(Opcode::DefineSetter),
        };
        Ok(())
    }

    fn class_element_name_string(&self, name: &ClassElementNameNode) -> String {
        match name {
            ClassElementNameNode::PropertyName(property_name) => self
                .property_name_string(property_name)
                .unwrap_or_else(|| "<computed>".to_string()),
            ClassElementNameNode::PrivateName(private) => self.private_field_key(private),
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
            ClassElementNameNode::PrivateName(private) => {
                let constant = self.add_string_constant(self.private_field_key(private))?;
                self.emit(Opcode::LoadConst(constant));
                Ok(())
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

    fn compile_private_field_initializer(
        &mut self,
        field: &boa_ast::function::PrivateFieldDefinition,
    ) -> Result<(), CompileError> {
        self.emit(Opcode::LoadThis);
        let constant = self.add_string_constant(self.private_field_key(field.name()))?;
        self.emit(Opcode::LoadConst(constant));
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
        // Pre-declare hoistable bindings so that function declarations compiled
        // ahead of their textual position still resolve the variables they
        // capture (and so calling a function before its definition works).
        self.predeclare_hoisted(statements)?;
        // Hoist function declarations (create the closures up front).
        for statement in statements {
            if let StatementNode::FunctionDeclaration(declaration) = statement {
                self.compile_function_declaration_statement(declaration)?;
            }
        }
        for statement in statements {
            if matches!(statement, StatementNode::FunctionDeclaration(_)) {
                continue;
            }
            self.compile_statement(statement)?;
        }
        Ok(())
    }

    /// Allocate slots for the function/var/let/const names declared directly in
    /// this statement list, so hoisted function bodies see them in scope.
    fn predeclare_hoisted(&mut self, statements: &[StatementNode]) -> Result<(), CompileError> {
        for statement in statements {
            match statement {
                StatementNode::FunctionDeclaration(declaration) => {
                    if !self.is_top_level {
                        let name = self.identifier_name(&declaration.name());
                        self.declare_function_scoped(&name)?;
                    }
                }
                StatementNode::VariableDeclaration(declaration) => {
                    let storage = if declaration.is_var() {
                        BindingStorage::Var
                    } else if declaration.is_const() {
                        BindingStorage::Const
                    } else {
                        BindingStorage::Let
                    };
                    for variable in declaration.variables() {
                        if let BindingNode::Identifier(identifier) = variable.binding() {
                            let name = self.identifier_name(identifier);
                            self.resolve_declaration_binding(
                                &name,
                                storage,
                                DeclarationContext::Statement,
                            )?;
                        }
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn compile_function_body(&mut self, body: &FunctionBodyNode) -> Result<(), CompileError> {
        let statements: Vec<StatementNode> = body
            .statements()
            .iter()
            .map(|item| statement_list_item_to_node(item.clone()))
            .collect();
        // `var` is function-scoped, so a `var` nested anywhere in this body (inside
        // blocks, if/for/while, try/catch, switch cases, labels — but NOT inside
        // nested functions) must be allocated a function slot BEFORE we compile the
        // body. Otherwise a closure created earlier than the textual `var` (e.g. a
        // hoisted function declaration, or a function expression assigned before
        // the `var`'s block runs) snapshots the outer scope without that name and
        // resolves it as a global → "X is not defined" at call time. Minified code
        // (Terser) routinely places `var`s inside blocks while closures above them
        // capture them, which is exactly how real React tripped this.
        if !self.is_top_level {
            let mut var_names = Vec::new();
            Self::collect_var_names(self.program, &statements, &mut var_names);
            for name in var_names {
                self.declare_function_scoped(&name)?;
            }
        }
        self.compile_statements(&statements)
    }

    /// Recursively collect the names introduced by `var` declarations within these
    /// statements, descending into nested control-flow blocks but NOT into nested
    /// function/class bodies (which have their own scope). Destructuring patterns
    /// are skipped here (handled when the declaration itself is compiled).
    fn collect_var_names(program: &Program, statements: &[StatementNode], out: &mut Vec<String>) {
        fn push_decl_names(program: &Program, decl: &VariableDeclaration, out: &mut Vec<String>) {
            if !decl.is_var() {
                return;
            }
            for variable in decl.variables() {
                if let BindingNode::Identifier(identifier) = variable.binding() {
                    out.push(program.resolve_sym(identifier.sym()));
                }
            }
        }
        fn visit(program: &Program, statement: &StatementNode, out: &mut Vec<String>) {
            match statement {
                StatementNode::VariableDeclaration(decl) => push_decl_names(program, decl, out),
                StatementNode::BlockStatement(block) => {
                    for item in block.statement_list().statements() {
                        visit(program, &statement_list_item_to_node(item.clone()), out);
                    }
                }
                StatementNode::IfStatement(stmt) => {
                    visit(program, &super::ast::statement_to_node(stmt.body().clone()), out);
                    if let Some(else_node) = stmt.else_node() {
                        visit(program, &super::ast::statement_to_node(else_node.clone()), out);
                    }
                }
                StatementNode::ForStatement(stmt) => {
                    if let Some(ForLoopInitializerNode::Var(var_decl)) = stmt.init() {
                        push_decl_names(program, &VariableDeclaration::Var(var_decl.clone()), out);
                    }
                    visit(program, &super::ast::statement_to_node(stmt.body().clone()), out);
                }
                StatementNode::ForInStatement(stmt) => {
                    if let IterableLoopInitializerNode::Var(variable) = stmt.initializer() {
                        if let BindingNode::Identifier(identifier) = variable.binding() {
                            out.push(program.resolve_sym(identifier.sym()));
                        }
                    }
                    visit(program, &super::ast::statement_to_node(stmt.body().clone()), out);
                }
                StatementNode::ForOfStatement(stmt) => {
                    if let IterableLoopInitializerNode::Var(variable) = stmt.initializer() {
                        if let BindingNode::Identifier(identifier) = variable.binding() {
                            out.push(program.resolve_sym(identifier.sym()));
                        }
                    }
                    visit(program, &super::ast::statement_to_node(stmt.body().clone()), out);
                }
                StatementNode::WhileStatement(stmt) => {
                    visit(program, &super::ast::statement_to_node(stmt.body().clone()), out);
                }
                StatementNode::DoWhileStatement(stmt) => {
                    visit(program, &super::ast::statement_to_node(stmt.body().clone()), out);
                }
                StatementNode::SwitchStatement(stmt) => {
                    for case in stmt.cases() {
                        for item in case.body().statements() {
                            visit(program, &statement_list_item_to_node(item.clone()), out);
                        }
                    }
                }
                StatementNode::TryStatement(stmt) => {
                    for item in stmt.block().statement_list().statements() {
                        visit(program, &statement_list_item_to_node(item.clone()), out);
                    }
                    if let Some(catch) = stmt.catch() {
                        for item in catch.block().statement_list().statements() {
                            visit(program, &statement_list_item_to_node(item.clone()), out);
                        }
                    }
                    if let Some(finally) = stmt.finally() {
                        for item in finally.block().statement_list().statements() {
                            visit(program, &statement_list_item_to_node(item.clone()), out);
                        }
                    }
                }
                StatementNode::LabeledStatement(labeled) => {
                    if let boa_ast::statement::LabelledItem::Statement(inner) = labeled.item() {
                        visit(program, &super::ast::statement_to_node(inner.clone()), out);
                    }
                }
                // Function/class declarations introduce their own scope; their inner
                // `var`s do not belong to this function. All other statements carry
                // no nested `var` bindings.
                _ => {}
            }
        }
        for statement in statements {
            visit(program, statement, out);
        }
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
            StatementNode::BreakStatement(break_statement) => {
                self.emit_active_finally_blocks()?;
                let jump = self.emit_jump(Opcode::Jump(0));
                let label = break_statement
                    .label()
                    .map(|sym| self.program.resolve_sym(sym));
                let context = match &label {
                    Some(name) => self
                        .control_stack
                        .iter_mut()
                        .rev()
                        .find(|context| context.label.as_deref() == Some(name.as_str())),
                    None => self.control_stack.iter_mut().next_back(),
                }
                .ok_or_else(|| CompileError::message("break used outside a loop or switch"))?;
                context.break_jumps.push(jump);
                Ok(())
            }
            StatementNode::ContinueStatement(continue_statement) => {
                self.emit_active_finally_blocks()?;
                let jump = self.emit_jump(Opcode::Jump(0));
                let label = continue_statement
                    .label()
                    .map(|sym| self.program.resolve_sym(sym));
                let loop_context = match &label {
                    Some(name) => self.control_stack.iter_mut().rev().find(|context| {
                        context.is_loop && context.label.as_deref() == Some(name.as_str())
                    }),
                    None => self.control_stack.iter_mut().rev().find(|context| context.is_loop),
                }
                .ok_or_else(|| CompileError::message("continue used outside a loop"))?;
                loop_context.continue_jumps.push(jump);
                Ok(())
            }
            StatementNode::LabeledStatement(labeled) => self.compile_labeled_statement(labeled),
            StatementNode::ExpressionStatement(expression) => {
                self.compile_expression(expression)?;
                self.emit(Opcode::Pop);
                Ok(())
            }
            StatementNode::EmptyStatement => Ok(()),
            StatementNode::ExportNamedDeclaration(export) => {
                self.compile_export_named_declaration(export)
            }
            StatementNode::ExportDefaultDeclaration(export) => {
                self.compile_export_default_declaration(export)
            }
            StatementNode::ExportAllDeclaration(export) => self.compile_export_all_declaration(export),
            StatementNode::ImportDeclaration(import) => self.compile_import_declaration(import),
            StatementNode::DebuggerStatement => Ok(()),
            StatementNode::WithStatement(_) => Err(CompileError::Unimplemented("with statements")),
        }
    }

    fn compile_export_named_declaration(
        &mut self,
        export: &super::ast::ExportNamedDeclaration,
    ) -> Result<(), CompileError> {
        match export.0.clone() {
            BoaExportDeclaration::Declaration(declaration) => match declaration {
                boa_ast::declaration::Declaration::FunctionDeclaration(function) => {
                    let name = self.identifier_name(&function.name());
                    self.compile_function_declaration_statement(&FunctionDeclaration::Function(function))?;
                    self.emit_module_export_name("default", &name)
                }
                boa_ast::declaration::Declaration::GeneratorDeclaration(function) => self
                    .compile_function_declaration_statement(&FunctionDeclaration::Generator(function))
                    .and_then(|_| self.emit_module_export_name("default", "default")),
                boa_ast::declaration::Declaration::AsyncFunctionDeclaration(function) => self
                    .compile_function_declaration_statement(&FunctionDeclaration::AsyncFunction(function))
                    .and_then(|_| self.emit_module_export_name("default", "default")),
                boa_ast::declaration::Declaration::AsyncGeneratorDeclaration(function) => self
                    .compile_function_declaration_statement(&FunctionDeclaration::AsyncGenerator(function))
                    .and_then(|_| self.emit_module_export_name("default", "default")),
                boa_ast::declaration::Declaration::ClassDeclaration(class_decl) => {
                    let name = self.identifier_name(&class_decl.name());
                    self.compile_class_declaration_statement(class_decl.as_ref())?;
                    self.emit_module_export_name("default", &name)
                }
                boa_ast::declaration::Declaration::Lexical(lexical) => {
                    let declaration = match lexical {
                        boa_ast::declaration::LexicalDeclaration::Let(_) => {
                            VariableDeclaration::Let(lexical)
                        }
                        boa_ast::declaration::LexicalDeclaration::Const(_) => {
                            VariableDeclaration::Const(lexical)
                        }
                    };
                    self.compile_variable_declaration(&declaration, DeclarationContext::Statement)?;
                    self.emit_exported_variable_names(&declaration)?;
                    Ok(())
                }
            },
            BoaExportDeclaration::VarStatement(var) => {
                let declaration = VariableDeclaration::Var(var);
                self.compile_variable_declaration(&declaration, DeclarationContext::Statement)?;
                self.emit_exported_variable_names(&declaration)
            }
            BoaExportDeclaration::List(list) => {
                self.compile_export_list(list.as_ref())?;
                Ok(())
            }
            BoaExportDeclaration::ReExport { kind, specifier } => {
                let Some(module_context) = self.module_context.clone() else {
                    return Err(CompileError::Unimplemented("export * (phase 2)"));
                };
                let specifier = self.program.resolve_sym(specifier.sym());
                let dep_key = module_context
                    .imports
                    .get(&specifier)
                    .cloned()
                    .ok_or_else(|| CompileError::message(format!("missing module context for re-export '{specifier}'")))?;
                match kind {
                    boa_ast::declaration::ReExportKind::Namespaced { name: Some(alias) } => {
                        let self_key = self.module_self_key()?.to_string();
                        self.emit_module_namespace(&self_key)?;
                        let export_const = self.add_string_constant(&self.program.resolve_sym(alias))?;
                        self.emit(Opcode::LoadConst(export_const));
                        self.emit_module_namespace(&dep_key)?;
                        self.emit(Opcode::SetProp);
                        Ok(())
                    }
                    boa_ast::declaration::ReExportKind::Namespaced { name: None } => {
                        let self_key = self.module_self_key()?.to_string();
                        let builtin = self.add_string_constant("\u{0}builtin:moduleReexportAll")?;
                        self.emit(Opcode::GetGlobal(builtin));
                        self.emit(Opcode::LoadUndefined);
                        self.emit_module_namespace(&self_key)?;
                        self.emit_module_namespace(&dep_key)?;
                        self.emit(Opcode::Call(2));
                        self.emit(Opcode::Pop);
                        Ok(())
                    }
                    boa_ast::declaration::ReExportKind::Named { names } => {
                        let self_key = self.module_self_key()?.to_string();
                        for spec in names.iter() {
                            self.emit_module_namespace(&self_key)?;
                            let export_const = self.add_string_constant(&self.program.resolve_sym(spec.alias()))?;
                            self.emit(Opcode::LoadConst(export_const));
                            self.emit_module_namespace(&dep_key)?;
                            let import_const =
                                self.add_string_constant(&self.program.resolve_sym(spec.private_name()))?;
                            self.emit(Opcode::LoadConst(import_const));
                            self.emit(Opcode::GetProp);
                            self.emit(Opcode::SetProp);
                        }
                        Ok(())
                    }
                }
            }
            BoaExportDeclaration::DefaultFunctionDeclaration(_)
            | BoaExportDeclaration::DefaultGeneratorDeclaration(_)
            | BoaExportDeclaration::DefaultAsyncFunctionDeclaration(_)
            | BoaExportDeclaration::DefaultAsyncGeneratorDeclaration(_)
            | BoaExportDeclaration::DefaultClassDeclaration(_)
            | BoaExportDeclaration::DefaultAssignmentExpression(_) => unreachable!(
                "default export declarations are routed through compile_export_default_declaration"
            ),
        }
    }

    fn compile_export_default_declaration(
        &mut self,
        export: &super::ast::ExportDefaultDeclaration,
    ) -> Result<(), CompileError> {
        match export.0.clone() {
            BoaExportDeclaration::DefaultFunctionDeclaration(function) => {
                self.compile_function_declaration_statement(&FunctionDeclaration::Function(function))?;
                self.emit_module_export_name("default", "default")
            }
            BoaExportDeclaration::DefaultGeneratorDeclaration(function) => self
                .compile_function_declaration_statement(&FunctionDeclaration::Generator(function))
                .and_then(|_| self.emit_module_export_name("default", "default")),
            BoaExportDeclaration::DefaultAsyncFunctionDeclaration(function) => self
                .compile_function_declaration_statement(&FunctionDeclaration::AsyncFunction(function))
                .and_then(|_| self.emit_module_export_name("default", "default")),
            BoaExportDeclaration::DefaultAsyncGeneratorDeclaration(function) => self.compile_function_declaration_statement(&FunctionDeclaration::AsyncGenerator(function)),
            BoaExportDeclaration::DefaultClassDeclaration(class_decl) => {
                self.compile_class_declaration_statement(class_decl.as_ref())?;
                self.emit_module_export_name("default", "default")
            }
            BoaExportDeclaration::DefaultAssignmentExpression(expr) => {
                self.compile_expression(&expr)?;
                let slot = self.allocate_hidden_local()?;
                self.emit(Opcode::SetLocal(slot));
                let self_key = self.module_self_key()?.to_string();
                self.emit_module_namespace(&self_key)?;
                let export_const = self.add_string_constant("default")?;
                self.emit(Opcode::LoadConst(export_const));
                self.emit(Opcode::GetLocal(slot));
                self.emit(Opcode::SetProp);
                self.emit(Opcode::Pop);
                Ok(())
            }
            BoaExportDeclaration::ReExport { .. }
            | BoaExportDeclaration::List(_)
            | BoaExportDeclaration::VarStatement(_)
            | BoaExportDeclaration::Declaration(_) => Err(CompileError::Unimplemented(
                "invalid default export declaration",
            )),
        }
    }

    fn compile_import_declaration(
        &mut self,
        import: &super::ast::JSImportDeclaration,
    ) -> Result<(), CompileError> {
        let Some(module_context) = self.module_context.clone() else {
            return Err(CompileError::Unimplemented("import (phase 2)"));
        };
        let specifier = self.program.resolve_sym(import.specifier().sym());
        let dep_key = module_context
            .imports
            .get(&specifier)
            .cloned()
            .ok_or_else(|| CompileError::message(format!("missing module context for import '{specifier}'")))?;
        if let Some(default) = import.default() {
            self.emit_module_import_name(&dep_key, "default")?;
            let name = self.identifier_name(&default);
            let slot = self.declare_block_scoped(&name)?;
            self.emit(Opcode::SetLocal(slot));
        }
        match import.kind() {
            boa_ast::declaration::ImportKind::DefaultOrUnnamed => {}
            boa_ast::declaration::ImportKind::Namespaced { binding } => {
                self.emit_module_namespace(&dep_key)?;
                let slot = self.declare_block_scoped(&self.identifier_name(binding))?;
                self.emit(Opcode::SetLocal(slot));
            }
            boa_ast::declaration::ImportKind::Named { names } => {
                for spec in names.iter().copied() {
                    self.emit_module_import_name(&dep_key, &self.program.resolve_sym(spec.export_name()))?;
                    let slot = self.declare_block_scoped(&self.identifier_name(&spec.binding()))?;
                    self.emit(Opcode::SetLocal(slot));
                }
            }
        }
        Ok(())
    }

    fn module_self_key(&self) -> Result<&str, CompileError> {
        self.module_context
            .as_ref()
            .map(|ctx| ctx.self_key.as_str())
            .ok_or_else(|| CompileError::Unimplemented("module context"))
    }

    fn emit_module_namespace(&mut self, key: &str) -> Result<(), CompileError> {
        let index = self.add_string_constant(key)?;
        self.emit(Opcode::GetGlobal(index));
        Ok(())
    }

    fn emit_module_import_name(&mut self, dep_key: &str, export_name: &str) -> Result<(), CompileError> {
        self.emit_module_namespace(dep_key)?;
        let export_const = self.add_string_constant(export_name)?;
        self.emit(Opcode::LoadConst(export_const));
        self.emit(Opcode::GetProp);
        Ok(())
    }

    fn emit_module_export_name(&mut self, export_name: &str, local_name: &str) -> Result<(), CompileError> {
        let self_key = self.module_self_key()?.to_string();
        self.emit_module_namespace(&self_key)?;
        let export_const = self.add_string_constant(export_name)?;
        self.emit(Opcode::LoadConst(export_const));
        let resolved = self.resolve_binding(local_name);
        self.emit_load_binding(local_name, resolved)?;
        self.emit(Opcode::SetProp);
        Ok(())
    }

    fn emit_module_export_value(&mut self, export_name: &str) -> Result<(), CompileError> {
        let self_key = self.module_self_key()?.to_string();
        self.emit_module_namespace(&self_key)?;
        let export_const = self.add_string_constant(export_name)?;
        self.emit(Opcode::LoadConst(export_const));
        self.emit(Opcode::SetProp);
        Ok(())
    }

    fn emit_exported_variable_names(
        &mut self,
        declaration: &VariableDeclaration,
    ) -> Result<(), CompileError> {
        for variable in declaration.variables() {
            if let BindingNode::Identifier(identifier) = variable.binding() {
                let name = self.identifier_name(identifier);
                self.emit_module_export_name(&name, &name)?;
            }
        }
        Ok(())
    }

    fn compile_export_list(&mut self, list: &[boa_ast::declaration::ExportSpecifier]) -> Result<(), CompileError> {
        for spec in list.iter().copied() {
            let local = self.program.resolve_sym(spec.private_name());
            let export = self.program.resolve_sym(spec.alias());
            self.emit_module_export_name(&export, &local)?;
        }
        Ok(())
    }

    fn compile_export_all_declaration(
        &mut self,
        export: &super::ast::ExportAllDeclaration,
    ) -> Result<(), CompileError> {
        match export.0.clone() {
            BoaExportDeclaration::ReExport { kind, specifier } => {
                let Some(module_context) = self.module_context.clone() else {
                    return Err(CompileError::Unimplemented("export * (phase 2)"));
                };
                let specifier = self.program.resolve_sym(specifier.sym());
                let dep_key = module_context
                    .imports
                    .get(&specifier)
                    .cloned()
                    .ok_or_else(|| CompileError::message(format!("missing module context for re-export '{specifier}'")))?;
                match kind {
                    boa_ast::declaration::ReExportKind::Namespaced { name: None } => {
                        let self_key = self.module_self_key()?.to_string();
                        let builtin = self.add_string_constant("\u{0}builtin:moduleReexportAll")?;
                        self.emit(Opcode::GetGlobal(builtin));
                        self.emit(Opcode::LoadUndefined);
                        self.emit_module_namespace(&self_key)?;
                        self.emit_module_namespace(&dep_key)?;
                        self.emit(Opcode::Call(2));
                        self.emit(Opcode::Pop);
                        Ok(())
                    }
                    _ => Err(CompileError::Unimplemented("export * as ns")),
                }
            }
            _ => Ok(()),
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

    fn compile_labeled_statement(
        &mut self,
        labeled: &super::ast::LabeledStatement,
    ) -> Result<(), CompileError> {
        let name = self.program.resolve_sym(labeled.label());
        let item = match labeled.item() {
            boa_ast::statement::LabelledItem::Statement(statement) => {
                super::ast::statement_to_node(statement.clone())
            }
            boa_ast::statement::LabelledItem::FunctionDeclaration(_) => {
                // Labeled function declarations are vanishingly rare in practice.
                return Err(CompileError::Unimplemented("labeled function declaration"));
            }
        };
        let is_loop = matches!(
            item,
            StatementNode::WhileStatement(_)
                | StatementNode::DoWhileStatement(_)
                | StatementNode::ForStatement(_)
                | StatementNode::ForInStatement(_)
                | StatementNode::ForOfStatement(_)
        );
        if is_loop {
            // The loop's control context picks up this label so that
            // `break label` / `continue label` resolve to it.
            self.pending_label = Some(name);
            self.compile_statement(&item)?;
            self.pending_label = None;
        } else {
            // Labeled non-loop statement: only `break label` is valid.
            self.control_stack.push(ControlContext {
                break_jumps: Vec::new(),
                continue_jumps: Vec::new(),
                is_loop: false,
                label: Some(name),
            });
            self.compile_statement(&item)?;
            let context = self
                .control_stack
                .pop()
                .expect("labeled control context should exist");
            let end = self.code.len();
            for jump in context.break_jumps {
                self.patch_jump(jump, end)?;
            }
        }
        Ok(())
    }

    /// Push a control context for a loop or switch, attaching any pending label
    /// from an enclosing labeled statement.
    fn push_control_context(&mut self, is_loop: bool) {
        let label = self.pending_label.take();
        self.control_stack.push(ControlContext {
            break_jumps: Vec::new(),
            continue_jumps: Vec::new(),
            is_loop,
            label,
        });
    }

    fn compile_while_statement(
        &mut self,
        statement: &super::ast::WhileStatement,
    ) -> Result<(), CompileError> {
        let loop_start = self.code.len();
        self.compile_expression(statement.condition())?;
        let exit_jump = self.emit_jump(Opcode::JumpIfFalsePop(0));
        self.push_control_context(true);
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
        self.push_control_context(true);
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

        // For `for (let …)` loops, capture the slots of the loop variables so
        // each iteration can be given a fresh binding (per-iteration semantics).
        let loop_slots: Vec<u16> = if uses_lexical_init {
            self.scopes
                .last()
                .map(|scope| scope.bindings.values().map(|binding| binding.slot).collect())
                .unwrap_or_default()
        } else {
            Vec::new()
        };

        let condition_start = self.code.len();
        let exit_jump = if let Some(condition) = statement.condition() {
            self.compile_expression(condition)?;
            Some(self.emit_jump(Opcode::JumpIfFalsePop(0)))
        } else {
            None
        };

        self.push_control_context(true);
        let body = super::ast::statement_to_node(statement.body().clone());
        self.compile_statement(&body)?;

        let increment_start = self.code.len();
        let loop_context = self.control_stack.pop().expect("loop context should exist");
        for jump in loop_context.continue_jumps {
            self.patch_jump(jump, increment_start)?;
        }

        // Per-iteration binding: copy each loop variable into a fresh cell before
        // running the increment, so closures captured in the just-finished body
        // keep the value they saw.
        for &slot in &loop_slots {
            self.emit(Opcode::FreshenLocal(slot));
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
                    ResolvedBinding::Global => self.emit(Opcode::LoadThis),
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
                    self.compile_property_name_value(name)?;
                    self.compile_expression(value)?;
                    self.emit(Opcode::SetProp);
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
