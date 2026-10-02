use ruff_python_ast as ast;
use ruff_python_semantic::{Definition, DefinitionId, Definitions, Member, MemberKind};

#[derive(Copy, Clone)]
pub(super) enum ExtractionTarget<'a> {
    Class(&'a ast::StmtClassDef),
    Function(&'a ast::StmtFunctionDef),
}

/// Extract a `Definition` from the AST node defined by a `Stmt`.
pub(super) fn extract_definition<'a>(
    target: ExtractionTarget<'a>,
    parent: DefinitionId,
    definitions: &Definitions<'a>,
) -> Member<'a> {
    match target {
        ExtractionTarget::Function(function) => match &definitions[parent] {
            Definition::Module(..) => Member {
                parent,
                kind: MemberKind::Function(function),
            },
            Definition::Member(Member {
                kind: MemberKind::Class(_) | MemberKind::NestedClass(_),
                ..
            }) => Member {
                parent,
                kind: MemberKind::Method(function),
            },
            Definition::Member(_) => Member {
                parent,
                kind: MemberKind::NestedFunction(function),
            },
        },
        ExtractionTarget::Class(class) => match &definitions[parent] {
            Definition::Module(_) => Member {
                parent,
                kind: MemberKind::Class(class),
            },
            Definition::Member(_) => Member {
                parent,
                kind: MemberKind::NestedClass(class),
            },
        },
    }
}
