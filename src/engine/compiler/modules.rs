use boa_ast::declaration::ExportDeclaration as BoaExportDeclaration;

use super::super::ast::{
    BindingNode, ExportAllDeclaration, ExportDefaultDeclaration, ExportNamedDeclaration,
    JSImportDeclaration, StatementNode, VariableDeclaration,
};
use super::{CompileError, DeclarationContext, FunctionDeclaration};
use super::scope::ImportBinding;
use super::Opcode;

impl<'a> super::FunctionCompiler<'a> {
    pub(super) fn compile_export_named_declaration(
        &mut self,
        export: &ExportNamedDeclaration,
    ) -> Result<(), CompileError> {
        match export.0.clone() {
            BoaExportDeclaration::Declaration(declaration) => match declaration {
                boa_ast::declaration::Declaration::FunctionDeclaration(function) => {
                    let name = self.identifier_name(&function.name());
                    self.compile_function_declaration_statement(&super::FunctionDeclaration::Function(function))?;
                    self.emit_module_export_name(&name, &name)
                }
                boa_ast::declaration::Declaration::GeneratorDeclaration(function) => {
                    let name = self.identifier_name(&function.name());
                    self.compile_function_declaration_statement(&super::FunctionDeclaration::Generator(function))?;
                    self.emit_module_export_name(&name, &name)
                }
                boa_ast::declaration::Declaration::AsyncFunctionDeclaration(function) => {
                    let name = self.identifier_name(&function.name());
                    self.compile_function_declaration_statement(&super::FunctionDeclaration::AsyncFunction(function))?;
                    self.emit_module_export_name(&name, &name)
                }
                boa_ast::declaration::Declaration::AsyncGeneratorDeclaration(function) => {
                    let name = self.identifier_name(&function.name());
                    self.compile_function_declaration_statement(&FunctionDeclaration::AsyncGenerator(function))?;
                    self.emit_module_export_name(&name, &name)
                }
                boa_ast::declaration::Declaration::ClassDeclaration(class_decl) => {
                    let name = self.identifier_name(&class_decl.name());
                    self.compile_class_declaration_statement(class_decl.as_ref())?;
                    self.emit_module_export_name(&name, &name)
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
                        self.emit(super::Opcode::LoadConst(export_const));
                        self.emit_module_namespace(&dep_key)?;
                        self.emit(super::Opcode::SetProp);
                        Ok(())
                    }
                    boa_ast::declaration::ReExportKind::Namespaced { name: None } => {
                        let self_key = self.module_self_key()?.to_string();
                        let builtin = self.add_string_constant("\u{0}builtin:moduleReexportAll")?;
                        self.emit(super::Opcode::GetGlobal(builtin));
                        self.emit(super::Opcode::LoadUndefined);
                        self.emit_module_namespace(&self_key)?;
                        self.emit_module_namespace(&dep_key)?;
                        self.emit(super::Opcode::Call(2));
                        self.emit(super::Opcode::Pop);
                        Ok(())
                    }
                    boa_ast::declaration::ReExportKind::Named { names } => {
                        let self_key = self.module_self_key()?.to_string();
                        for spec in names.iter() {
                            self.emit_module_namespace(&self_key)?;
                            let export_const = self.add_string_constant(&self.program.resolve_sym(spec.alias()))?;
                            self.emit(super::Opcode::LoadConst(export_const));
                            self.emit_module_namespace(&dep_key)?;
                            let import_const =
                                self.add_string_constant(&self.program.resolve_sym(spec.private_name()))?;
                            self.emit(super::Opcode::LoadConst(import_const));
                            self.emit(super::Opcode::GetProp);
                            self.emit(super::Opcode::SetProp);
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

    pub(super) fn compile_export_default_declaration(
        &mut self,
        export: &ExportDefaultDeclaration,
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
                self.emit(super::Opcode::SetLocal(slot));
                let self_key = self.module_self_key()?.to_string();
                self.emit_module_namespace(&self_key)?;
                let export_const = self.add_string_constant("default")?;
                self.emit(super::Opcode::LoadConst(export_const));
                self.emit(super::Opcode::GetLocal(slot));
                self.emit(super::Opcode::SetProp);
                self.emit(super::Opcode::Pop);
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

    pub(super) fn compile_import_declaration(
        &mut self,
        import: &JSImportDeclaration,
    ) -> Result<(), CompileError> {
        self.register_import_declaration(import)
    }

    pub(super) fn predeclare_imports(&mut self, statements: &[StatementNode]) -> Result<(), CompileError> {
        for statement in statements {
            if let StatementNode::ImportDeclaration(import) = statement {
                self.register_import_declaration(import)?;
            }
        }
        Ok(())
    }

    pub(super) fn register_import_declaration(
        &mut self,
        import: &JSImportDeclaration,
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
            let name = self.identifier_name(&default);
            self.import_bindings.insert(
                name,
                ImportBinding::Named {
                    dep_key: dep_key.clone(),
                    export_name: "default".to_string(),
                },
            );
        }
        match import.kind() {
            boa_ast::declaration::ImportKind::DefaultOrUnnamed => {}
            boa_ast::declaration::ImportKind::Namespaced { binding } => {
                self.import_bindings.insert(
                    self.identifier_name(&binding),
                    ImportBinding::Namespace {
                        dep_key: dep_key.clone(),
                    },
                );
            }
            boa_ast::declaration::ImportKind::Named { names } => {
                for spec in names.iter().copied() {
                    self.import_bindings.insert(
                        self.identifier_name(&spec.binding()),
                        ImportBinding::Named {
                            dep_key: dep_key.clone(),
                            export_name: self.program.resolve_sym(spec.export_name()),
                        },
                    );
                }
            }
        }
        Ok(())
    }

    pub(super) fn module_self_key(&self) -> Result<&str, CompileError> {
        self.module_context
            .as_ref()
            .map(|ctx| ctx.self_key.as_str())
            .ok_or_else(|| CompileError::Unimplemented("module context"))
    }

    pub(super) fn emit_module_namespace(&mut self, key: &str) -> Result<(), CompileError> {
        let index = self.add_string_constant(key)?;
        self.emit(Opcode::GetGlobal(index));
        Ok(())
    }

    pub(super) fn emit_module_import_name(&mut self, dep_key: &str, export_name: &str) -> Result<(), CompileError> {
        self.emit_module_namespace(dep_key)?;
        let export_const = self.add_string_constant(export_name)?;
        self.emit(Opcode::LoadConst(export_const));
        self.emit(Opcode::GetProp);
        Ok(())
    }

    pub(super) fn emit_module_export_name(&mut self, export_name: &str, local_name: &str) -> Result<(), CompileError> {
        let self_key = self.module_self_key()?.to_string();
        self.emit_module_namespace(&self_key)?;
        let export_const = self.add_string_constant(export_name)?;
        self.emit(Opcode::LoadConst(export_const));
        let resolved = self.resolve_binding(local_name);
        self.emit_load_binding(local_name, resolved)?;
        self.emit(Opcode::SetProp);
        Ok(())
    }

    pub(super) fn emit_module_export_value(&mut self, export_name: &str) -> Result<(), CompileError> {
        let self_key = self.module_self_key()?.to_string();
        self.emit_module_namespace(&self_key)?;
        let export_const = self.add_string_constant(export_name)?;
        self.emit(Opcode::LoadConst(export_const));
        self.emit(Opcode::SetProp);
        Ok(())
    }

    pub(super) fn emit_exported_variable_names(
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

    pub(super) fn compile_export_list(&mut self, list: &[boa_ast::declaration::ExportSpecifier]) -> Result<(), CompileError> {
        for spec in list.iter().copied() {
            let local = self.program.resolve_sym(spec.private_name());
            let export = self.program.resolve_sym(spec.alias());
            self.emit_module_export_name(&export, &local)?;
        }
        Ok(())
    }

    pub(super) fn compile_export_all_declaration(
        &mut self,
        export: &ExportAllDeclaration,
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
}
