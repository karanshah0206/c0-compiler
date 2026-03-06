use std::collections::{HashMap, HashSet};

use crate::front::ast::{Ident, Typ, Variable};

/// Metadata and semantic info for a function.
pub struct FunctionContext {
  /// Sequence of parameter types taken by this function.
  params: Vec<Typ>,
  /// Scope context for semantic analysis within this function.
  scope_context: ScopeContext,
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

    // Function parameters are treated as defined
    for param in params {
      function_context.define_var(param.clone());
    }

    function_context
  }

  /// Get a function's sequence of parameter types.
  pub fn get_params(&self) -> &Vec<Typ> {
    &self.params
  }

  /// Try getting a variable's context.
  pub fn get_var_ctx(&self, id: &Ident) -> Option<&VarContext> {
    self.variables.get(id)
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

    let mut var_context = VarContext {
      typ: var.0,
      decl_scope: self.scope_context.current_id,
      defn_scope: Some(self.scope_context.current_id),
    };

    // in case it was previously declared
    if let Some(ctx) = self.variables.get(&var.1) {
      var_context.decl_scope = ctx.decl_scope;
    }

    self.variables.insert(var.1, var_context);
  }

  /// Define all variables in the current local scope.
  pub fn define_all_vars(&mut self) -> HashSet<&Ident> {
    let current_scope_id = self.scope_context.current_id;

    let mut defines = HashSet::new();

    for (id, ctx) in self.variables.iter_mut() {
      if self.scope_context.is_active(ctx.decl_scope) {
        if ctx.defn_scope != Some(current_scope_id) {
          defines.insert(id);
          ctx.defn_scope = Some(current_scope_id)
        }
      }
    }

    defines
  }
}

/// Structure and state of scopes within the type context.
struct ScopeContext {
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
  /// Generate a new scope context with a root scope indexed at `0`.
  fn new() -> Self {
    ScopeContext {
      current_id: 0,
      count: 1,
      active_scopes: HashSet::new(),
      scope_stack: vec![0],
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
    self.scope_stack.pop().expect("Cannot exit from root scope");
    self.current_id = *self.scope_stack.last().unwrap_or(&0);
  }

  /// Check that a scope id is currently active.
  pub fn is_active(&self, id: usize) -> bool {
    self.active_scopes.contains(&id)
  }
}

/// Metadata and semantic info of a variable.
pub struct VarContext {
  /// Type of the variable.
  typ: Typ,
  /// Id of the scope within which the variable is declared.
  decl_scope: usize,
  /// Id of the scope within which the variable is defined.
  defn_scope: Option<usize>,
}
