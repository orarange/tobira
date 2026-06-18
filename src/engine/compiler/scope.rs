use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use super::super::chunk::UpvalueDescriptor;
use super::{BindingStorage, CompileError, DeclarationContext, FunctionCompiler};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ResolvedBinding {
    Local(u16),
    Upvalue(u16),
    ModuleImport,
    Global,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum ImportBinding {
    Named { dep_key: String, export_name: String },
    Namespace { dep_key: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct LocalBinding {
    pub(super) slot: u16,
}

#[derive(Debug, Clone, Default)]
pub(super) struct ScopeFrame {
    pub(super) bindings: HashMap<String, LocalBinding>,
}

/// The upvalue list of a single function, shared (via `Rc`) between the live
/// `FunctionCompiler` and the `OuterBindings` snapshots handed to its nested
/// functions. Sharing is what makes *transitive* upvalue capture possible: a
/// grandchild resolving a name from a grandparent can retroactively add an
/// upvalue to the intermediate parent's real upvalue list.
#[derive(Debug, Clone, Default)]
pub(super) struct UpvalueState {
    pub(super) descriptors: Vec<UpvalueDescriptor>,
    pub(super) names: HashMap<String, u16>,
}

impl UpvalueState {
    pub(super) fn get_or_create(&mut self, name: &str, descriptor: UpvalueDescriptor) -> u16 {
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
pub(super) struct OuterBindings {
    pub(super) scopes: Vec<HashMap<String, u16>>,
    pub(super) upvalues: Rc<RefCell<UpvalueState>>,
    pub(super) parent: Option<Box<OuterBindings>>,
}

impl OuterBindings {
    pub(super) fn lookup_local(&self, name: &str) -> Option<u16> {
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
    pub(super) fn ensure_upvalue(&self, name: &str) -> Option<u16> {
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

impl<'a> FunctionCompiler<'a> {
    pub(super) fn allocate_hidden_local(&mut self) -> Result<u16, CompileError> {
        let slot = self.next_local;
        self.next_local = self
            .next_local
            .checked_add(1)
            .ok_or_else(|| CompileError::message("local slot count exceeded u16"))?;
        Ok(slot)
    }

    pub(super) fn push_scope(&mut self) {
        self.scopes.push(ScopeFrame::default());
    }

    pub(super) fn pop_scope(&mut self) {
        self.scopes.pop();
    }

    pub(super) fn root_scope_mut(&mut self) -> &mut ScopeFrame {
        self.scopes
            .first_mut()
            .expect("function scope should always exist")
    }

    pub(super) fn current_scope_mut(&mut self) -> &mut ScopeFrame {
        self.scopes
            .last_mut()
            .expect("current scope should always exist")
    }

    pub(super) fn declare_function_scoped(&mut self, name: &str) -> Result<u16, CompileError> {
        if let Some(binding) = self.root_scope_mut().bindings.get(name) {
            return Ok(binding.slot);
        }

        let slot = self.allocate_hidden_local()?;
        self.root_scope_mut()
            .bindings
            .insert(name.to_string(), LocalBinding { slot });
        Ok(slot)
    }

    pub(super) fn declare_block_scoped(&mut self, name: &str) -> Result<u16, CompileError> {
        if let Some(binding) = self.current_scope_mut().bindings.get(name) {
            return Ok(binding.slot);
        }

        let slot = self.allocate_hidden_local()?;
        self.current_scope_mut()
            .bindings
            .insert(name.to_string(), LocalBinding { slot });
        Ok(slot)
    }

    pub(super) fn resolve_binding(&mut self, name: &str) -> ResolvedBinding {
        for scope in self.scopes.iter().rev() {
            if let Some(binding) = scope.bindings.get(name) {
                return ResolvedBinding::Local(binding.slot);
            }
        }

        if let Some(index) = self.resolve_upvalue(name) {
            return ResolvedBinding::Upvalue(index);
        }

        if self.import_bindings.contains_key(name) {
            return ResolvedBinding::ModuleImport;
        }

        ResolvedBinding::Global
    }

    /// Resolve `name` as an upvalue of the current function, walking the whole
    /// enclosing chain and lazily creating intermediate upvalues so that
    /// transitive captures (e.g. `a => b => c => a + b + c`) work. Returns the
    /// upvalue index within the current function, or `None` if `name` is global.
    pub(super) fn resolve_upvalue(&self, name: &str) -> Option<u16> {
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

    pub(super) fn snapshot_outer_bindings(&self) -> OuterBindings {
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
    pub(super) fn install_this_binding(&mut self) -> Result<(), CompileError> {
        let slot = self.allocate_hidden_local()?;
        self.root_scope_mut()
            .bindings
            .insert("this".to_string(), LocalBinding { slot });
        self.emit(super::Opcode::LoadThis);
        self.emit(super::Opcode::SetLocal(slot));
        Ok(())
    }

    pub(super) fn declare_named_hidden_local(
        &mut self,
        name: impl Into<String>,
    ) -> Result<u16, CompileError> {
        let slot = self.allocate_hidden_local()?;
        self.current_scope_mut()
            .bindings
            .insert(name.into(), LocalBinding { slot });
        Ok(slot)
    }

    pub(super) fn resolve_declaration_binding(
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
}
