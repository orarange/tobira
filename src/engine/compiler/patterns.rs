use super::super::ast::{
    ArrayPatternElementNode, ArrayPatternNode, AssignOpNode, BindingNode, ExpressionNode,
    ObjectPatternElementNode, ObjectPatternNode, PatternNode, PropertyNameNode,
};
use super::super::chunk::Opcode;
use super::{BindingStorage, CompileError, DeclarationContext};

#[derive(Debug, Clone)]
pub(super) struct PendingPatternInit {
    pub(super) pattern: PatternNode,
    pub(super) slot: u16,
    pub(super) storage: BindingStorage,
}

impl<'a> super::FunctionCompiler<'a> {
    pub(super) fn compile_binding_store(
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

    pub(super) fn compile_pattern_store(
        &mut self,
        pattern: &PatternNode,
        source_slot: u16,
        storage: BindingStorage,
        context: DeclarationContext,
    ) -> Result<(), CompileError> {
        match pattern {
            PatternNode::Object(pattern) => {
                self.compile_object_pattern_store(pattern, source_slot, storage, context)
            }
            PatternNode::Array(pattern) => {
                self.compile_array_pattern_store(pattern, source_slot, storage, context)
            }
        }
    }

    pub(super) fn compile_object_pattern_store(
        &mut self,
        pattern: &ObjectPatternNode,
        source_slot: u16,
        storage: BindingStorage,
        context: DeclarationContext,
    ) -> Result<(), CompileError> {
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

    pub(super) fn compile_array_pattern_store(
        &mut self,
        pattern: &ArrayPatternNode,
        source_slot: u16,
        storage: BindingStorage,
        context: DeclarationContext,
    ) -> Result<(), CompileError> {
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

    pub(super) fn extract_object_property_to_slot(
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

    pub(super) fn extract_array_index_to_slot(
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

    pub(super) fn apply_default_initializer_slot(
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

    pub(super) fn assign_member_from_slot(
        &mut self,
        access: &super::super::ast::MemberExpression,
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

    pub(super) fn copy_slot_to_object(
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
        self.emit(Opcode::Pop);
        for key in exclude {
            let constant = self.add_string_constant(key.clone())?;
            self.emit(Opcode::GetLocal(slot));
            self.emit(Opcode::LoadConst(constant));
            self.emit(Opcode::DeleteProp);
            self.emit(Opcode::Pop);
        }
        Ok(slot)
    }

    pub(super) fn slice_array_slot(&mut self, source_slot: u16, start: u32) -> Result<u16, CompileError> {
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

    pub(super) fn create_accumulator_array_slot(&mut self) -> Result<u16, CompileError> {
        let slot = self.allocate_hidden_local()?;
        self.emit(Opcode::MakeArray(0));
        self.emit(Opcode::SetLocal(slot));
        Ok(slot)
    }

    pub(super) fn push_value_into_array_slot(
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

    pub(super) fn push_undefined_into_array_slot(&mut self, array_slot: u16) -> Result<(), CompileError> {
        let push_name = self.add_string_constant("push")?;
        self.emit(Opcode::GetLocal(array_slot));
        self.emit(Opcode::GetPropForCall(push_name));
        self.emit(Opcode::LoadUndefined);
        self.emit(Opcode::Call(1));
        self.emit(Opcode::Pop);
        Ok(())
    }

    pub(super) fn concat_spread_expression_into_array_slot(
        &mut self,
        array_slot: u16,
        spread: &super::super::ast::SpreadElement,
    ) -> Result<(), CompileError> {
        let concat_name = self.add_string_constant("concat")?;
        self.emit(Opcode::GetLocal(array_slot));
        self.emit(Opcode::GetPropForCall(concat_name));
        self.emit_array_from_expression(spread.target())?;
        self.emit(Opcode::Call(1));
        self.emit(Opcode::SetLocal(array_slot));
        Ok(())
    }

    pub(super) fn compile_pattern_assignment_expression(
        &mut self,
        pattern: &PatternNode,
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
}
