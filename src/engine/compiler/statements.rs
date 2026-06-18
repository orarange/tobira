use crate::engine::ast::*;
use crate::engine::chunk::{Constant, ExceptionHandler, Opcode};
use super::{
    BindingNode, BindingStorage, CompileError, ControlContext, DeclarationContext, ExpressionNode,
    FormalParameterListNode, FunctionBodyNode, ImportBinding, IterableLoopInitializerNode,
    PendingPatternInit, ResolvedBinding,
};

impl<'a> super::FunctionCompiler<'a> {
    pub(super) fn emit_active_finally_blocks(&mut self) -> Result<(), CompileError> {
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

    pub(super) fn compile_inline_block(
        &mut self,
        block: &BlockStatement,
    ) -> Result<(), CompileError> {
        self.push_scope();
        for item in block.statement_list().statements() {
            let statement = statement_list_item_to_node(item.clone());
            self.compile_statement(&statement)?;
        }
        self.pop_scope();
        Ok(())
    }

    pub(super) fn compile_function_parameters(
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
                    let name = self.identifier_name(&identifier);
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

    pub(super) fn emit_array_from_expression(
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

    pub(super) fn compile_array_literal_with_spread(
        &mut self,
        array: &ArrayExpression,
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

    pub(super) fn compile_argument_array(
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

    pub(super) fn compile_call_expression_with_spread(
        &mut self,
        call: &CallExpression,
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

    pub(super) fn compile_new_expression_with_spread(
        &mut self,
        new_expression: &NewExpression,
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

    pub(super) fn compile_logical_assignment_expression(
        &mut self,
        assign: &AssignmentExpression,
    ) -> Result<(), CompileError> {
        match assign.lhs() {
            AssignTargetNode::Identifier(identifier) => {
                let name = self.identifier_name(&identifier);
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

    pub(super) fn compile_nullish_expression(
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

    pub(super) fn compile_regexp_literal(
        &mut self,
        regexp: &RegexLiteral,
    ) -> Result<(), CompileError> {
        let index = self.add_constant(Constant::RegExp {
            pattern: self.program.resolve_sym(regexp.pattern()),
            flags: self.program.resolve_sym(regexp.flags()),
        })?;
        self.emit(Opcode::MakeRegExp(index));
        Ok(())
    }

    pub(super) fn compile_switch_statement(
        &mut self,
        statement: &SwitchStatement,
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

    pub(super) fn compile_for_in_statement(
        &mut self,
        statement: &ForInStatement,
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
        let body = statement_to_node(statement.body().clone());
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

    pub(super) fn compile_for_of_statement(
        &mut self,
        statement: &ForOfStatement,
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
            let body = statement_to_node(statement.body().clone());
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
            let body = statement_to_node(statement.body().clone());
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

    pub(super) fn compile_iterable_initializer_store(
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

    pub(super) fn compile_try_statement(
        &mut self,
        statement: &TryStatement,
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
                        let name = self.identifier_name(&identifier);
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

    pub(super) fn compile_optional_expression(
        &mut self,
        optional: &OptionalExpression,
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

    pub(super) fn compile_optional_call(
        &mut self,
        optional: &OptionalExpression,
        call: &CallExpression,
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

    pub(super) fn emit_load_binding(
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
            ResolvedBinding::ModuleImport => {
                match self.import_bindings.get(name) {
                    Some(ImportBinding::Named { dep_key, export_name }) => {
                        let dep_key = dep_key.clone();
                        let export_name = export_name.clone();
                        self.emit_module_import_name(&dep_key, &export_name)?;
                    }
                    Some(ImportBinding::Namespace { dep_key }) => {
                        let dep_key = dep_key.clone();
                        self.emit_module_namespace(&dep_key)?;
                    }
                    None => {
                        return Err(CompileError::message(format!(
                            "missing import binding for '{name}'"
                        )));
                    }
                }
            }
            ResolvedBinding::Global => {
                let index = self.add_string_constant(name)?;
                self.emit(Opcode::GetGlobal(index));
            }
        }
        Ok(())
    }

    pub(super) fn emit_store_binding(
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
            ResolvedBinding::ModuleImport => {
                return Err(CompileError::message(format!(
                    "cannot assign to imported binding '{name}'"
                )));
            }
            ResolvedBinding::Global => {
                let index = self.add_string_constant(name)?;
                self.emit(Opcode::SetGlobal(index));
            }
        }
        Ok(())
    }

    pub(super) fn compile_statements(&mut self, statements: &[StatementNode]) -> Result<(), CompileError> {
        // Pre-declare hoistable bindings so that function declarations compiled
        // ahead of their textual position still resolve the variables they
        // capture (and so calling a function before its definition works).
        self.predeclare_hoisted(statements)?;
        if self.is_top_level {
            self.predeclare_imports(statements)?;
        }
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
    pub(super) fn predeclare_hoisted(&mut self, statements: &[StatementNode]) -> Result<(), CompileError> {
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
                            let name = self.identifier_name(&identifier);
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

    pub(super) fn compile_function_body(&mut self, body: &FunctionBodyNode) -> Result<(), CompileError> {
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
    pub(super) fn collect_var_names(program: &Program, statements: &[StatementNode], out: &mut Vec<String>) {
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
                    visit(program, &statement_to_node(stmt.body().clone()), out);
                    if let Some(else_node) = stmt.else_node() {
                        visit(program, &statement_to_node(else_node.clone()), out);
                    }
                }
                StatementNode::ForStatement(stmt) => {
                    if let Some(ForLoopInitializerNode::Var(var_decl)) = stmt.init() {
                        push_decl_names(program, &VariableDeclaration::Var(var_decl.clone()), out);
                    }
                    visit(program, &statement_to_node(stmt.body().clone()), out);
                }
                StatementNode::ForInStatement(stmt) => {
                    if let IterableLoopInitializerNode::Var(variable) = stmt.initializer() {
                        if let BindingNode::Identifier(identifier) = variable.binding() {
                            out.push(program.resolve_sym(identifier.sym()));
                        }
                    }
                    visit(program, &statement_to_node(stmt.body().clone()), out);
                }
                StatementNode::ForOfStatement(stmt) => {
                    if let IterableLoopInitializerNode::Var(variable) = stmt.initializer() {
                        if let BindingNode::Identifier(identifier) = variable.binding() {
                            out.push(program.resolve_sym(identifier.sym()));
                        }
                    }
                    visit(program, &statement_to_node(stmt.body().clone()), out);
                }
                StatementNode::WhileStatement(stmt) => {
                    visit(program, &statement_to_node(stmt.body().clone()), out);
                }
                StatementNode::DoWhileStatement(stmt) => {
                    visit(program, &statement_to_node(stmt.body().clone()), out);
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
                        visit(program, &statement_to_node(inner.clone()), out);
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

    pub(super) fn compile_statement(&mut self, statement: &StatementNode) -> Result<(), CompileError> {
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

    pub(super) fn compile_block_statement(
        &mut self,
        block: &BlockStatement,
    ) -> Result<(), CompileError> {
        self.push_scope();
        for item in block.statement_list().statements() {
            let statement = statement_list_item_to_node(item.clone());
            self.compile_statement(&statement)?;
        }
        self.pop_scope();
        Ok(())
    }

    pub(super) fn compile_if_statement(
        &mut self,
        statement: &IfStatement,
    ) -> Result<(), CompileError> {
        self.compile_expression(statement.cond())?;
        let else_jump = self.emit_jump(Opcode::JumpIfFalsePop(0));
        let then_statement = statement_to_node(statement.body().clone());
        self.compile_statement(&then_statement)?;

        if let Some(else_node) = statement.else_node() {
            let end_jump = self.emit_jump(Opcode::Jump(0));
            let else_start = self.code.len();
            self.patch_jump(else_jump, else_start)?;
            let else_statement = statement_to_node(else_node.clone());
            self.compile_statement(&else_statement)?;
            let end = self.code.len();
            self.patch_jump(end_jump, end)?;
        } else {
            let end = self.code.len();
            self.patch_jump(else_jump, end)?;
        }

        Ok(())
    }

    pub(super) fn compile_labeled_statement(
        &mut self,
        labeled: &LabeledStatement,
    ) -> Result<(), CompileError> {
        let name = self.program.resolve_sym(labeled.label());
        let item = match labeled.item() {
            boa_ast::statement::LabelledItem::Statement(statement) => {
                statement_to_node(statement.clone())
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
    pub(super) fn push_control_context(&mut self, is_loop: bool) {
        let label = self.pending_label.take();
        self.control_stack.push(ControlContext {
            break_jumps: Vec::new(),
            continue_jumps: Vec::new(),
            is_loop,
            label,
        });
    }

    pub(super) fn compile_while_statement(
        &mut self,
        statement: &WhileStatement,
    ) -> Result<(), CompileError> {
        let loop_start = self.code.len();
        self.compile_expression(statement.condition())?;
        let exit_jump = self.emit_jump(Opcode::JumpIfFalsePop(0));
        self.push_control_context(true);
        let body = statement_to_node(statement.body().clone());
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

    pub(super) fn compile_do_while_statement(
        &mut self,
        statement: &DoWhileStatement,
    ) -> Result<(), CompileError> {
        let loop_start = self.code.len();
        self.push_control_context(true);
        let body = statement_to_node(statement.body().clone());
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

    pub(super) fn compile_for_statement(
        &mut self,
        statement: &ForStatement,
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
        let body = statement_to_node(statement.body().clone());
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

    pub(super) fn compile_variable_declaration(
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
                        let name = self.identifier_name(&identifier);
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

    pub(super) fn compile_function_declaration_statement(
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


}
