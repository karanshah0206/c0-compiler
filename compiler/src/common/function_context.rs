use std::collections::{HashMap, HashSet};

use crate::front::ast::{Ident, Typ, Variable};

/// Metadata and semantic info for a function.
pub struct FunctionContext {
  /// Sequence of parameter types taken by this function.
  params: Vec<Typ>,
  /// Scope context for semantic analysis within this function.
  pub scope_context: ScopeContext,
  /// Variables within this function.
  variables: HashMap<Ident, VarContext>,
  /// Function calls made from within this function.
  function_calls: HashSet<Ident>,
}

impl FunctionContext {
  /// Generate a new context for a function, treating parameters as defined variables.
  pub fn new(params: &[Variable]) -> Self {
    let mut function_context = FunctionContext {
      params: params.iter().map(|(t, _)| t.clone()).collect(),
      scope_context: ScopeContext::new(),
      variables: HashMap::new(),
      function_calls: HashSet::new(),
    };
    function_context.reset_params(params);
    function_context
  }

  /// Clear previous param names and initialize new param list as defined
  pub fn reset_params(&mut self, params: &[Variable]) {
    self.variables.clear();
    for param in params {
      self.define_var(param.clone());
    }
  }

  /// Get a function's sequence of parameter types.
  pub fn get_params(&self) -> &Vec<Typ> {
    &self.params
  }

  /// Get the type of a variable.
  pub fn get_var_type(&self, id: &Ident) -> Typ {
    self
      .variables
      .get(id)
      .map(|ctx| ctx.typ.clone())
      .expect(&format!("Unknown variable {id}."))
  }

  /// Check if a variable is declared in the currently active scopes.
  pub fn is_var_declared(&self, var: &Ident) -> bool {
    match self.variables.get(var) {
      Some(var_ctx) => self.scope_context.is_active(var_ctx.decl_scope),
      None => false,
    }
  }

  /// Check if a variable is defined in the currently active scopes.
  pub fn is_var_defined(&self, var: &Ident) -> bool {
    if let Some(var_ctx) = self.variables.get(var)
      && let Some(scope_id) = var_ctx.defn_scope
    {
      self.scope_context.is_active(scope_id)
    } else {
      false
    }
  }

  /// Declare a variable in the current scope.
  pub fn declare_var(&mut self, var: Variable) {
    assert!(var.0 != Typ::Void, "Variable {} cannot be void.", var.1);

    assert!(
      !self.is_var_declared(&var.1),
      "Variable {} is already declared in this scope.",
      var.1
    );

    self.variables.insert(
      var.1,
      VarContext {
        typ: var.0,
        decl_scope: self.scope_context.current_id,
        defn_scope: None,
      },
    );
  }

  /// Define a variable in the current scope (and declare if fresh).
  pub fn define_var(&mut self, var: Variable) {
    assert!(var.0 != Typ::Void, "Variable {} cannot be void.", var.1);

    if self.is_var_declared(&var.1) {
      let existing = self.variables.get_mut(&var.1).unwrap();
      existing.typ = var.0;

      // Keep the declaration scope and preserve an already-active definition.
      if !existing
        .defn_scope
        .is_some_and(|scope_id| self.scope_context.is_active(scope_id))
      {
        existing.defn_scope = Some(self.scope_context.current_id);
      }
    } else {
      self.variables.insert(
        var.1,
        VarContext {
          typ: var.0,
          decl_scope: self.scope_context.current_id,
          defn_scope: Some(self.scope_context.current_id),
        },
      );
    }
  }

  /// Define all variables in the current local scope.
  pub fn define_all_vars(&mut self) -> HashSet<Ident> {
    let current_scope_id = self.scope_context.current_id;

    let mut defines = HashSet::new();

    for (id, var_ctx) in self.variables.iter_mut() {
      if self.scope_context.is_active(var_ctx.decl_scope) {
        // Only promote declarations that are currently undefined in active scopes.
        if !var_ctx
          .defn_scope
          .is_some_and(|scope_id| self.scope_context.is_active(scope_id))
        {
          defines.insert(id.to_string());
          var_ctx.defn_scope = Some(current_scope_id)
        }
      }
    }

    defines
  }

  /// Get a copy of functions called by this function.
  pub fn get_function_calls(&self) -> HashSet<Ident> {
    self.function_calls.clone()
  }

  /// Add a function call made by this function.
  pub fn insert_function_call(&mut self, id: Ident) {
    self.function_calls.insert(id);
  }
}

/// Structure and state of scopes within the type context.
pub struct ScopeContext {
  /// Identifier for the currently evaluated scope in context.
  current_id: usize,
  /// Total number of scopes within context.
  count: usize,
  /// Scopes that are currently active.
  active_scopes: HashSet<usize>,
  /// Path to current scope from root scope.
  scope_stack: Vec<usize>,
}

impl ScopeContext {
  /// Generate a new, empty scope context.
  fn new() -> Self {
    ScopeContext {
      current_id: 0,
      count: 0,
      active_scopes: HashSet::new(),
      scope_stack: Vec::new(),
    }
  }

  /// Create and enter a nested scope from the current scope.
  pub fn enter_scope(&mut self) {
    self.current_id = self.count;
    self.count += 1;
    self.active_scopes.insert(self.current_id);
    self.scope_stack.push(self.current_id);
  }

  /// Leave current scope and go back to parent scope.
  pub fn exit_scope(&mut self) {
    self.active_scopes.remove(&self.current_id);
    self.scope_stack.pop().expect("Scope stack is empty.");
    self.current_id = *self.scope_stack.last().unwrap_or(&0);
  }

  /// Check that a scope id is currently active.
  pub fn is_active(&self, id: usize) -> bool {
    self.active_scopes.contains(&id)
  }
}

/// Metadata and semantic info of a variable.
struct VarContext {
  /// Type of the variable.
  typ: Typ,
  /// Id of the scope within which the variable is declared.
  decl_scope: usize,
  /// Id of the scope within which the variable is defined.
  defn_scope: Option<usize>,
}
