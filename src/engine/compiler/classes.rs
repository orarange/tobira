use boa_ast::function::PrivateFieldDefinition;

use super::super::ast::{
    ClassDeclarationNode, ClassElementNameNode, ClassElementNode, ClassExpressionNode,
    ClassFieldDefinitionNode, ClassMethodDefinitionNode, ExpressionNode,
    MethodDefinitionKindNode, SuperCallExpression, SuperPropertyAccessNode,
};
use super::super::chunk::Opcode;
use super::{
    BindingStorage, CompileError, DeclarationContext, FunctionCompileOptions, FunctionCompiler,
    PropertyAccessFieldNode, PropertyOpKind,
};

impl<'a> super::FunctionCompiler<'a> {
    pub(super) fn compile_class_declaration_statement(
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

    pub(super) fn compile_class_expression(
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

    pub(super) fn compile_class_value(
        &mut self,
        name: Option<String>,
        super_ref: Option<&ExpressionNode>,
        constructor: Option<&super::super::ast::ClassConstructorExpressionNode>,
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

    pub(super) fn compile_synthetic_class_constructor(
        &mut self,
        name: Option<String>,
        options: &FunctionCompileOptions,
        field_initializers: &[&ClassFieldDefinitionNode],
        private_field_initializers: &[&PrivateFieldDefinition],
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
            self.import_bindings.clone(),
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

    pub(super) fn compile_class_method_definition(
        &mut self,
        class_slot: u16,
        method: &ClassMethodDefinitionNode,
        options: &FunctionCompileOptions,
    ) -> Result<(), CompileError> {
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

    pub(super) fn class_element_name_string(&self, name: &ClassElementNameNode) -> String {
        match name {
            ClassElementNameNode::PropertyName(property_name) => self
                .property_name_string(property_name)
                .unwrap_or_else(|| "<computed>".to_string()),
            ClassElementNameNode::PrivateName(private) => self.private_field_key(private),
        }
    }

    pub(super) fn compile_class_element_name_value(
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

    pub(super) fn compile_class_field_initializer(
        &mut self,
        field: &ClassFieldDefinitionNode,
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

    pub(super) fn compile_private_field_initializer(
        &mut self,
        field: &PrivateFieldDefinition,
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

    pub(super) fn compile_static_class_field_initializer(
        &mut self,
        class_slot: u16,
        field: &ClassFieldDefinitionNode,
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

    pub(super) fn compile_super_call(&mut self, call: &SuperCallExpression) -> Result<(), CompileError> {
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

    pub(super) fn compile_super_property_access(
        &mut self,
        access: &SuperPropertyAccessNode,
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
                let constant = self.add_string_constant(self.identifier_name(&identifier))?;
                self.emit(Opcode::LoadConst(constant));
                self.emit(Opcode::GetProp);
            }
            PropertyAccessFieldNode::Expr(expression) => {
                self.compile_expression(&expression)?;
                self.emit(Opcode::GetIndex);
            }
        }
        Ok(())
    }

    pub(super) fn compile_super_property_for_call(
        &mut self,
        access: &SuperPropertyAccessNode,
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
                let constant = self.add_string_constant(self.identifier_name(&identifier))?;
                self.emit(Opcode::LoadConst(constant));
                self.emit(Opcode::GetProp);
            }
            PropertyAccessFieldNode::Expr(expression) => {
                self.compile_expression(&expression)?;
                self.emit(Opcode::GetIndex);
            }
        }
        self.emit(Opcode::LoadThis);
        Ok(())
    }

    pub(super) fn compile_super_property_access_temps(
        &mut self,
        access: &SuperPropertyAccessNode,
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
                let constant = self.add_string_constant(self.identifier_name(&identifier))?;
                self.emit(Opcode::LoadConst(constant));
                self.emit(Opcode::SetLocal(key_temp));
                Ok(PropertyOpKind::Named)
            }
            PropertyAccessFieldNode::Expr(expression) => {
                self.compile_expression(&expression)?;
                self.emit(Opcode::SetLocal(key_temp));
                Ok(PropertyOpKind::Computed)
            }
        }
    }
}
