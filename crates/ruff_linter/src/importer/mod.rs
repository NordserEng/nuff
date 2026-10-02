//! Code modification struct to add and modify import statements.
//!
//! Enables rules to make module members available (that may be not yet be imported) during fix
//! execution.

use std::error::Error;

use anyhow::Result;
use libcst_native as cst;

use ruff_diagnostics::Edit;
use ruff_python_ast::{self as ast, ModModule, Stmt};
use ruff_python_codegen::Stylist;
use ruff_python_importer::Insertion;
use ruff_python_parser::Parsed;
use ruff_python_semantic::{
    ImportedName, MemberNameImport, ModuleNameImport, NameImport, SemanticModel,
};
use ruff_text_size::{Ranged, TextSize};

use crate::cst::matchers::{match_aliases, match_import_from, match_statement};
use crate::fix::codemods::CodegenStylist;

pub(crate) struct Importer<'a> {
    /// The Python AST to which we are adding imports.
    python_ast: &'a [Stmt],
    /// The source code text for `python_ast`.
    source: &'a str,
    /// The [`Stylist`] for the Python AST.
    stylist: &'a Stylist<'a>,
    /// The list of visited, top-level runtime imports in the Python AST.
    runtime_imports: Vec<&'a Stmt>,
    /// The list of visited, top-level `if TYPE_CHECKING:` blocks in the Python AST.
    type_checking_blocks: Vec<&'a Stmt>,
}

impl<'a> Importer<'a> {
    pub(crate) fn new(
        parsed: &'a Parsed<ModModule>,
        source: &'a str,
        stylist: &'a Stylist<'a>,
    ) -> Self {
        Self {
            python_ast: parsed.suite(),
            source,
            stylist,
            runtime_imports: Vec::default(),
            type_checking_blocks: Vec::default(),
        }
    }

    /// Visit a top-level import statement.
    pub(crate) fn visit_import(&mut self, import: &'a Stmt) {
        self.runtime_imports.push(import);
    }

    /// Visit a top-level type-checking block.
    pub(crate) fn visit_type_checking_block(&mut self, type_checking_block: &'a Stmt) {
        self.type_checking_blocks.push(type_checking_block);
    }

    /// Add an import statement to import the given module, with an explicit `lazy` keyword if requested.
    ///
    /// If there are no existing imports, the new import will be added at the top
    /// of the file. If there are future imports, the new import will be added
    /// after the last future import. Otherwise, it will be added after the most
    /// recent top-level import statement.
    pub(crate) fn add_import(&self, import: &NameImport, at: TextSize, is_lazy: bool) -> Edit {
        let required_import = if is_lazy {
            format!("lazy {import}")
        } else {
            import.to_string()
        };
        if let Some(stmt) = self.preceding_import(at) {
            // Insert after the last top-level import.
            Insertion::end_of_statement(stmt, self.source, self.stylist).into_edit(&required_import)
        } else {
            self.add_at_start(&required_import)
        }
    }

    fn add_at_start(&self, text: &str) -> Edit {
        if let Some(last_future_import) = self.find_last_future_import() {
            Insertion::end_of_statement(last_future_import, self.source, self.stylist)
                .into_edit(text)
        } else {
            Insertion::start_of_file(self.python_ast, self.source, self.stylist, None)
                .into_edit(text)
        }
    }

    /// Generate an [`Edit`] to reference the given symbol. Returns the [`Edit`] necessary to make
    /// the symbol available in the current scope along with the bound name of the symbol.
    ///
    /// Attempts to reuse existing imports when possible.
    pub(crate) fn get_or_import_symbol(
        &self,
        symbol: &ImportRequest,
        at: TextSize,
        semantic: &SemanticModel,
    ) -> Result<(Edit, String), ResolutionError> {
        self.get_symbol(symbol, at, semantic)?
            .map_or_else(|| self.import_symbol(symbol, at, None, semantic), Ok)
    }

    /// For a given builtin symbol, determine whether an [`Edit`] is necessary to make the symbol
    /// available in the current scope. For example, if `zip` has been overridden in the relevant
    /// scope, the `builtins` module will need to be imported in order for a `Fix` to reference
    /// `zip`; but otherwise, that won't be necessary.
    ///
    /// Returns a two-item tuple. The first item is either `Some(Edit)` (indicating) that an
    /// edit is necessary to make the symbol available, or `None`, indicating that the symbol has
    /// not been overridden in the current scope. The second item in the tuple is the bound name
    /// of the symbol.
    ///
    /// Attempts to reuse existing imports when possible.
    pub(crate) fn get_or_import_builtin_symbol(
        &self,
        symbol: &str,
        at: TextSize,
        semantic: &SemanticModel,
    ) -> Result<(Option<Edit>, String), ResolutionError> {
        if semantic.has_builtin_binding(symbol) {
            return Ok((None, symbol.to_string()));
        }
        let (import_edit, binding) =
            self.get_or_import_symbol(&ImportRequest::import("builtins", symbol), at, semantic)?;
        Ok((Some(import_edit), binding))
    }

    /// Return the [`ImportedName`] to for existing symbol, if it's present in the given [`SemanticModel`].
    fn find_symbol(
        symbol: &ImportRequest,
        at: TextSize,
        semantic: &SemanticModel,
    ) -> Result<Option<ImportedName>, ResolutionError> {
        // If the symbol is already available in the current scope, use it.
        let Some(imported_name) =
            semantic.resolve_qualified_import_name(symbol.module, symbol.member)
        else {
            return Ok(None);
        };

        // If the symbol source (i.e., the import statement) comes after the current location,
        // abort. For example, we could be generating an edit within a function, and the import
        // could be defined in the module scope, but after the function definition. In this case,
        // it's unclear whether we can use the symbol (the function could be called between the
        // import and the current location, and thus the symbol would not be available). It's also
        // unclear whether should add an import statement at the start of the file, since it could
        // be shadowed between the import and the current location.
        if imported_name.start() > at {
            return Err(ResolutionError::ImportAfterUsage);
        }

        // If the symbol source (i.e., the import statement) is in a typing-only context, but we're
        // in a runtime context, abort.
        if imported_name.context().is_typing() && semantic.execution_context().is_runtime() {
            return Err(ResolutionError::IncompatibleContext);
        }

        Ok(Some(imported_name))
    }

    /// Return an [`Edit`] to reference an existing symbol, if it's present in the given [`SemanticModel`].
    fn get_symbol(
        &self,
        symbol: &ImportRequest,
        at: TextSize,
        semantic: &SemanticModel,
    ) -> Result<Option<(Edit, String)>, ResolutionError> {
        // Find the symbol in the current scope.
        let Some(imported_name) = Self::find_symbol(symbol, at, semantic)? else {
            return Ok(None);
        };

        // We also add a no-op edit to force conflicts with any other fixes that might try to
        // remove the import. Consider:
        //
        // ```python
        // import sys
        //
        // quit()
        // ```
        //
        // Assume you omit this no-op edit. If you run Ruff with `unused-imports` and
        // `sys-exit-alias` over this snippet, it will generate two fixes: (1) remove the unused
        // `sys` import; and (2) replace `quit()` with `sys.exit()`, under the assumption that `sys`
        // is already imported and available.
        //
        // By adding this no-op edit, we force the `unused-imports` fix to conflict with the
        // `sys-exit-alias` fix, and thus will avoid applying both fixes in the same pass.
        let import_edit = Edit::range_replacement(
            self.source[imported_name.range()].to_string(),
            imported_name.range(),
        );
        Ok(Some((import_edit, imported_name.into_name())))
    }

    /// Generate an [`Edit`] to reference the given symbol. Returns the [`Edit`] necessary to make
    /// the symbol available in the current scope along with the bound name of the symbol.
    ///
    /// For example, assuming `module` is `"functools"` and `member` is `"lru_cache"`, this function
    /// could return an [`Edit`] to add `import functools` to the start of the file, alongside with
    /// the name on which the `lru_cache` symbol would be made available (`"functools.lru_cache"`).
    fn import_symbol(
        &self,
        symbol: &ImportRequest,
        at: TextSize,
        except: Option<&Stmt>,
        semantic: &SemanticModel,
    ) -> Result<(Edit, String), ResolutionError> {
        if let Some(stmt) = self
            .find_import_from(symbol.module, at)
            .filter(|stmt| except != Some(stmt))
        {
            // Case 1: `from functools import lru_cache` is in scope, and we're trying to reference
            // `functools.cache`; thus, we add `cache` to the import, and return `"cache"` as the
            // bound name.
            if semantic.is_available(symbol.member) {
                let Ok(import_edit) = self.add_member(stmt, symbol.member) else {
                    return Err(ResolutionError::InvalidEdit);
                };
                Ok((import_edit, symbol.member.to_string()))
            } else {
                Err(ResolutionError::ConflictingName(symbol.member.to_string()))
            }
        } else {
            match symbol.style {
                ImportStyle::Import => {
                    // Case 2a: No `functools` import is in scope; thus, we add `import functools`,
                    // and return `"functools.cache"` as the bound name.
                    if semantic.is_available(symbol.module) {
                        let import_edit = self.add_import(
                            &NameImport::Import(ModuleNameImport::module(
                                symbol.module.to_string(),
                            )),
                            at,
                            false,
                        );
                        Ok((
                            import_edit,
                            format!(
                                "{module}.{member}",
                                module = symbol.module,
                                member = symbol.member
                            ),
                        ))
                    } else {
                        Err(ResolutionError::ConflictingName(symbol.module.to_string()))
                    }
                }
                ImportStyle::ImportFrom => {
                    // Case 2b: No `functools` import is in scope; thus, we add
                    // `from functools import cache`, and return `"cache"` as the bound name.
                    if semantic.is_available(symbol.member) {
                        let import_edit = self.add_import(
                            &NameImport::ImportFrom(MemberNameImport::member(
                                symbol.module.to_string(),
                                symbol.member.to_string(),
                            )),
                            at,
                            false,
                        );
                        Ok((import_edit, symbol.member.to_string()))
                    } else {
                        Err(ResolutionError::ConflictingName(symbol.member.to_string()))
                    }
                }
            }
        }
    }

    /// Return the top-level [`Stmt`] that imports the given module using `Stmt::ImportFrom`
    /// preceding the given position, if any.
    fn find_import_from(&self, module: &str, at: TextSize) -> Option<&Stmt> {
        let mut import_from = None;
        for stmt in &self.runtime_imports {
            if stmt.start() >= at {
                break;
            }
            if let Stmt::ImportFrom(ast::StmtImportFrom {
                module: name,
                names,
                level,
                is_lazy: _,
                range: _,
                node_index: _,
            }) = stmt
            {
                if *level == 0
                    && name.as_ref().is_some_and(|name| name == module)
                    && names.iter().all(|alias| alias.name.as_str() != "*")
                {
                    import_from = Some(*stmt);
                }
            }
        }
        import_from
    }

    /// Add the given member to an existing `Stmt::ImportFrom` statement.
    fn add_member(&self, stmt: &Stmt, member: &str) -> Result<Edit> {
        let mut statement = match_statement(&self.source[stmt.range()])?;
        let (_, names, _) = match_import_from(&mut statement)?;
        let aliases = match_aliases(names)?;
        aliases.push(cst::ImportAlias {
            name: cst::NameOrAttribute::N(Box::new(cst::Name {
                value: member,
                lpar: vec![],
                rpar: vec![],
            })),
            asname: None,
            comma: aliases.last().and_then(|alias| alias.comma.clone()),
        });
        Ok(Edit::range_replacement(
            statement.codegen_stylist(self.stylist),
            stmt.range(),
        ))
    }

    /// Return the import statement that precedes the given position, if any.
    fn preceding_import(&self, at: TextSize) -> Option<&'a Stmt> {
        self.runtime_imports
            .partition_point(|stmt| stmt.start() < at)
            .checked_sub(1)
            .map(|idx| self.runtime_imports[idx])
    }

    /// Find the last `from __future__` import statement in the AST.
    fn find_last_future_import(&self) -> Option<&'a Stmt> {
        let mut body = self.python_ast.iter().peekable();
        let _docstring = body.next_if(|stmt| ast::helpers::is_docstring_stmt(stmt));

        body.take_while(|stmt| {
            stmt.as_import_from_stmt().is_some_and(|import_from| {
                !import_from.is_lazy && import_from.module.as_deref() == Some("__future__")
            })
        })
        .last()
    }
}

#[derive(Debug)]
enum ImportStyle {
    /// Import the symbol using the `import` statement (e.g. `import foo; foo.bar`).
    Import,
    /// Import the symbol using the `from` statement (e.g. `from foo import bar; bar`).
    ImportFrom,
}

#[derive(Debug)]
pub(crate) struct ImportRequest<'a> {
    /// The module from which the symbol can be imported (e.g., `foo`, in `from foo import bar`).
    module: &'a str,
    /// The member to import (e.g., `bar`, in `from foo import bar`).
    member: &'a str,
    /// The preferred style to use when importing the symbol (e.g., `import foo` or
    /// `from foo import bar`), if it's not already in scope.
    style: ImportStyle,
}

impl<'a> ImportRequest<'a> {
    /// Create a new `ImportRequest` from a module and member. If not present in the scope,
    /// the symbol should be imported using the "import" statement.
    pub(crate) fn import(module: &'a str, member: &'a str) -> Self {
        Self {
            module,
            member,
            style: ImportStyle::Import,
        }
    }

    /// Create a new `ImportRequest` from a module and member. If not present in the scope,
    /// the symbol should be imported using the "import from" statement.
    pub(crate) fn import_from(module: &'a str, member: &'a str) -> Self {
        Self {
            module,
            member,
            style: ImportStyle::ImportFrom,
        }
    }
}

/// The result of an [`Importer::get_or_import_symbol`] call.
#[derive(Debug)]
pub(crate) enum ResolutionError {
    /// The symbol is imported, but the import came after the current location.
    ImportAfterUsage,
    /// The symbol is imported, but in an incompatible context (e.g., in typing-only context, while
    /// we're in a runtime context).
    IncompatibleContext,
    /// The symbol can't be imported, because another symbol is bound to the same name.
    ConflictingName(String),
    /// The symbol can't be imported due to an error in editing an existing import statement.
    InvalidEdit,
}

impl std::fmt::Display for ResolutionError {
    fn fmt(&self, fmt: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ResolutionError::ImportAfterUsage => {
                fmt.write_str("Unable to use existing symbol due to late binding")
            }
            ResolutionError::IncompatibleContext => {
                fmt.write_str("Unable to use existing symbol due to incompatible context")
            }
            ResolutionError::ConflictingName(binding) => std::write!(
                fmt,
                "Unable to insert `{binding}` into scope due to name conflict"
            ),
            ResolutionError::InvalidEdit => {
                fmt.write_str("Unable to modify existing import statement")
            }
        }
    }
}

impl Error for ResolutionError {}
