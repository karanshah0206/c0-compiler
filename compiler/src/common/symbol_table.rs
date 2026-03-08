use std::collections::{HashMap, HashSet};

use crate::common::function_context::FunctionContext;
use crate::front::ast::{Ident, Typ, Variable};

/// Store and query metadata about symbols (identifiers) in the source code.
pub struct SymbolTable {
  /// Concrete type for typedefs, return type for functions.
  typ: HashMap<Ident, Typ>,
  /// Sequence of parameter types of a declared function.
  function_context: HashMap<Ident, FunctionContext>,
  /// Set of functions that have been defined.
  defined_functions: HashSet<Ident>,
  /// Set of functions declared in header.
  header_functions: HashSet<Ident>,
}

impl SymbolTable {
  /// Create a new, empty symbol table.
  pub fn new() -> Self {
    SymbolTable {
      typ: HashMap::new(),
      function_context: HashMap::new(),
      defined_functions: HashSet::new(),
      header_functions: HashSet::new(),
    }
  }

  /// Add a new type definition.
  pub fn add_typedef(&mut self, id: &Ident, typ: &Typ) {
    use Typ::*;

    if let Typedef(id) = typ
      && self.is_function(id)
    {
      panic!("Cannot typedef with {id} as it is a function identifier.");
    }

    let typ = self.resolve_type(typ.clone());
    assert!(typ != Void, "Illegal typedef void {id}");
    assert!(
      self.typ.insert(id.clone(), typ).is_none(),
      "Cannot typedef with non-unique identifier {id}"
    );
  }

  /// Add a function declaration.
  pub fn declare_function(
    &mut self,
    id: &Ident,
    typ: &mut Typ,
    params: &mut [Variable],
    is_header: bool,
  ) {
    use Typ::*;

    assert!(
      !is_header || *id != "main",
      "Cannot declare `main` in the header file.",
    );

    assert!(
      !self.is_typedef(id),
      "Cannot declare function {id} because the identifier is a type alias."
    );

    *typ = self.resolve_type(typ.clone());

    // validate function parameters in declaration
    let mut param_ids: HashSet<Ident> = HashSet::new();
    for (typ, id) in params.iter_mut() {
      *typ = self.resolve_type(typ.clone());
      assert!(*typ != Void, "Parameter {id} cannot be void.");

      assert!(
        !self.is_typedef(id),
        "Parameter {id} conflicts with a typedef."
      );

      assert!(
        param_ids.insert(id.to_string()),
        "Identifier {id} cannot be used for multiple parameters."
      );
    }

    if self.is_function(id) {
      // a function with this identifier was already declared

      assert!(
        self.typ.get(id).unwrap() == typ,
        "Mismatching return type in redeclaration of function {id}."
      );

      // check parameter types match
      let function_ctx = self.function_context.get_mut(id).unwrap();
      let decl_params = function_ctx.get_params();
      assert!(
        decl_params.len() == params.len() && params.iter().map(|(t, _)| t).eq(decl_params.iter()),
        "Mismatching parameter list in redeclaration of function {id}."
      );
      function_ctx.reset_params(params);
    } else {
      // new function declaration for this identifier

      self.typ.insert(id.clone(), typ.clone());
      self
        .function_context
        .insert(id.clone(), FunctionContext::new(params));

      if is_header {
        self.defined_functions.insert(id.clone()); // header functions are treated as defined
        self.header_functions.insert(id.clone());
      }
    }
  }

  /// Add a function definition.
  pub fn define_function(&mut self, id: &Ident, typ: &mut Typ, params: &mut [Variable]) {
    assert!(!self.is_defined(id), "Function {id} cannot be redefined.");

    self.declare_function(id, typ, params, false);
    self.defined_functions.insert(id.clone());
  }

  /// Get the concrete type of a type alias.
  pub fn resolve_type(&mut self, typ: Typ) -> Typ {
    use Typ::*;

    match typ {
      Typedef(id) => {
        if let Some(typ) = self.typ.get(&id).cloned() {
          assert!(!self.is_function(&id), "{id} is not a type.");

          let underlying = self.resolve_type(typ);
          self.typ.insert(id, underlying.clone());
          underlying
        } else {
          panic!("Unknown identifier {id}");
        }
      }
      _ => typ,
    }
  }

  /// Get the return type and parameter list of a declared function.
  pub fn get_function_signature(&self, id: &Ident) -> Option<(Typ, Vec<Typ>)> {
    if let Some(typ) = self.typ.get(id)
      && let Some(params) = self.function_context.get(id)
    {
      Some((typ.clone(), params.get_params().clone()))
    } else {
      None
    }
  }

  pub fn get_function_context(&mut self, id: &Ident) -> &mut FunctionContext {
    self
      .function_context
      .get_mut(id)
      .expect(&format!("Unknown function {id}."))
  }

  /// Check whether an identifier is a type alias.
  pub fn is_typedef(&self, id: &Ident) -> bool {
    self.typ.contains_key(id) && !self.function_context.contains_key(id)
  }

  /// Check whether an identifier belongs to a declared function.
  pub fn is_function(&self, id: &Ident) -> bool {
    self.function_context.contains_key(id)
  }

  /// Check whether a function has been defined.
  pub fn is_defined(&self, id: &Ident) -> bool {
    self.defined_functions.contains(id)
  }
}
