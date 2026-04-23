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
  /// Mapping from struct identifier to fields in declaration order.
  structs: HashMap<String, Vec<(Typ, Ident)>>,
}

impl SymbolTable {
  /// Create a new, empty symbol table.
  pub fn new() -> Self {
    SymbolTable {
      typ: HashMap::new(),
      function_context: HashMap::new(),
      defined_functions: HashSet::new(),
      header_functions: HashSet::new(),
      structs: HashMap::new(),
    }
  }

  /// Add a new type definition.
  pub fn add_typedef(&mut self, id: &Ident, typ: &Typ) {
    use Typ::*;

    if let Typ::Typedef(id) = typ
      && self.is_function(id)
    {
      panic!("Cannot typedef with {id} as it is a function identifier.");
    }

    let typ = self.resolve_type(typ.clone());
    assert!(typ != Void && typ != Null, "Illegal typedef void {id}.");
    assert!(
      Self::is_pointer_legal(&typ),
      "Illegal typedef of unsupported pointer type {id}."
    );

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
    assert!(
      !matches!(typ, Typ::Struct(_)),
      "Function {id} cannot return a struct by value."
    );
    assert!(
      Self::is_pointer_legal(typ),
      "Function {id} returns an illegal type."
    );

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
        !matches!(typ, Typ::Struct(_)),
        "Parameter {id} cannot be a struct value type."
      );
      assert!(
        Self::is_pointer_legal(typ),
        "Parameter {id} is of illegal type {typ}."
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
    assert!(
      !self.is_function_defined(id),
      "Function {id} cannot be redefined."
    );

    self.declare_function(id, typ, params, false);
    self.defined_functions.insert(id.clone());
  }

  /// Add a struct definition.
  pub fn define_struct(&mut self, id: &String, fields: &mut [(Typ, Ident)]) {
    assert!(
      !self.structs.contains_key(id),
      "Struct {id} was already defined."
    );

    let mut field_names = HashSet::new();
    for (field_type, field_id) in fields.iter_mut() {
      *field_type = self.resolve_type(field_type.clone());

      assert!(
        !matches!(field_type, Typ::Void | Typ::Null),
        "Struct field {field_id} in {id} has invalid type {field_type}."
      );

      if let Typ::Struct(struct_id) = field_type {
        assert!(
          self.is_struct_defined(struct_id),
          "Struct field {field_id} in {id} uses incomplete struct type {struct_id}."
        );
      }

      assert!(
        Self::is_pointer_legal(field_type),
        "Struct field {field_id} in {id} has illegal type {field_type}."
      );

      assert!(
        field_names.insert(field_id.to_owned()),
        "Field {field_id} is defined multiple times in struct {id}."
      );
    }

    self.structs.insert(id.to_owned(), fields.to_owned());
  }

  /// Get the concrete type of a type alias.
  pub fn resolve_type(&mut self, typ: Typ) -> Typ {
    use Typ::*;

    match typ {
      Typ::Array(typ, dimensions) => {
        let typ = self.resolve_type(*typ);
        if let Typ::Array(inner_type, inner_dims) = typ {
          Typ::Array(inner_type, inner_dims + dimensions)
        } else {
          Typ::Array(Box::new(typ), dimensions)
        }
      }
      Typ::Pointer(typ, dimensions) => {
        let typ = self.resolve_type(*typ);
        if let Typ::Pointer(inner_typ, inner_dims) = typ {
          Typ::Pointer(inner_typ, inner_dims + dimensions)
        } else {
          Typ::Pointer(Box::new(typ), dimensions)
        }
      }
      Typedef(id) => {
        if let Some(typ) = self.typ.get(&id).cloned() {
          assert!(!self.is_function(&id), "{id} is not a type.");

          let underlying = self.resolve_type(typ);
          self.typ.insert(id, underlying.clone());
          underlying
        } else {
          unreachable!("Unknown identifier {id}");
        }
      }
      Typ::Struct(_) | Typ::Bool | Typ::Int | Typ::Null | Typ::Void => typ,
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

  /// Get function context by function name.
  pub fn get_function_context(&self, id: &Ident) -> &FunctionContext {
    self
      .function_context
      .get(id)
      .unwrap_or_else(|| unreachable!("Unknown function {id}."))
  }

  /// Get a mutable function context by function name.
  pub fn get_mut_function_context(&mut self, id: &Ident) -> &mut FunctionContext {
    self
      .function_context
      .get_mut(id)
      .unwrap_or_else(|| unreachable!("Unknown function {id}."))
  }

  /// Get the type of a struct's field.
  pub fn get_struct_field_type(&self, struct_id: &Ident, field_id: &Ident) -> Option<Typ> {
    self.structs.get(struct_id).and_then(|fields| {
      fields
        .iter()
        .find(|(_, id)| id == field_id)
        .map(|(typ, _)| typ.clone())
    })
  }

  /// Get ordered field list of a struct definition.
  pub fn get_struct_fields(&self, struct_id: &Ident) -> Option<&Vec<(Typ, Ident)>> {
    self.structs.get(struct_id)
  }

  /// Check whether an identifier is a type alias.
  pub fn is_typedef(&self, id: &Ident) -> bool {
    self.typ.contains_key(id) && !self.function_context.contains_key(id)
  }

  /// Check whether an identifier belongs to a declared function.
  pub fn is_function(&self, id: &Ident) -> bool {
    self.function_context.contains_key(id)
  }

  /// Check whether an identifier belongs to a function declared in the header file.
  pub fn is_header_function(&self, id: &Ident) -> bool {
    self.header_functions.contains(id)
  }

  /// Check whether a function has been defined.
  pub fn is_function_defined(&self, id: &Ident) -> bool {
    self.defined_functions.contains(id)
  }

  /// Check whether a struct has been defined.
  pub fn is_struct_defined(&self, id: &Ident) -> bool {
    self.structs.contains_key(id)
  }

  /// Checks that the shape of a pointer type is valid.
  fn is_pointer_legal(typ: &Typ) -> bool {
    match typ {
      Typ::Pointer(inner, _) | Typ::Array(inner, _) => {
        !matches!(inner.as_ref(), Typ::Void | Typ::Null | Typ::Typedef(_))
          && Self::is_pointer_legal(inner)
      }
      Typ::Bool | Typ::Int | Typ::Null | Typ::Void | Typ::Struct(_) | Typ::Typedef(_) => true,
    }
  }
}
