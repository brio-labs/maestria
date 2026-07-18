use crate::symbols::context::FileContext;
use crate::{RecordProvenance, SourceRange, SymbolKind, SymbolRecord, Visibility};
use proc_macro2::Span;
use syn::Type;
use syn::spanned::Spanned;

pub(crate) fn simple_record(
    kind: SymbolKind,
    name: &str,
    module_stack: &[String],
    visibility: &syn::Visibility,
    attrs: &[syn::Attribute],
    context: &FileContext,
    range: SourceRange,
) -> SymbolRecord {
    SymbolRecord {
        record_id: record_id(&qualify(module_stack, name), kind.clone(), &range, context),
        package: context.package.to_string(),
        target: context.target.to_string(),
        kind,
        name: name.to_string(),
        qualified_name: qualify(module_stack, name),
        visibility: to_visibility(visibility),
        is_public_api: is_public_api(visibility),
        is_async: false,
        is_unsafe: false,
        is_test: context.is_test_target || crate::symbols::markers::attr_test(attrs),
        is_bench: context.is_bench_target || crate::symbols::markers::attr_bench(attrs),
        signature: Some(name.to_string()),
        imports: Vec::new(),
        markers: crate::symbols::markers::declaration_markers(attrs, &context.file_markers),
        provenance: provenance(context, range),
    }
}

pub(crate) fn dedupe_strings(mut values: Vec<String>) -> Vec<String> {
    values.sort_unstable();
    values.dedup();
    values
}

pub(crate) fn to_visibility(visibility: &syn::Visibility) -> Visibility {
    match visibility {
        syn::Visibility::Public(_) => Visibility::Public,
        syn::Visibility::Restricted(restricted) => {
            let segments = restricted.path.segments.iter().collect::<Vec<_>>();
            match segments.as_slice() {
                [segment] if segment.ident == "crate" => Visibility::Crate,
                [segment] if segment.ident == "super" => Visibility::Super,
                [segment] if segment.ident == "self" => Visibility::Inherited,
                _ => Visibility::Restricted,
            }
        }
        syn::Visibility::Inherited => Visibility::Inherited,
    }
}

pub(crate) fn is_public_api(visibility: &syn::Visibility) -> bool {
    matches!(visibility, syn::Visibility::Public(_))
}

pub(crate) fn signature_text(sig: &syn::Signature) -> String {
    let count = sig.inputs.len();
    if count == 0 {
        format!("{}()", sig.ident)
    } else {
        format!("{}({count} args)", sig.ident)
    }
}

pub(crate) fn resolve_type_name(ty: &Type) -> String {
    match ty {
        Type::Path(path) => path
            .path
            .segments
            .iter()
            .map(|segment| segment.ident.to_string())
            .collect::<Vec<_>>()
            .join("::"),
        _ => "impl".to_string(),
    }
}

pub(crate) fn qualify(stack: &[String], leaf: &str) -> String {
    if stack.is_empty() {
        leaf.to_string()
    } else {
        let mut names = stack.to_vec();
        names.push(leaf.to_string());
        names.join("::")
    }
}

pub(crate) fn record_id(
    name: &str,
    kind: SymbolKind,
    range: &SourceRange,
    context: &FileContext,
) -> String {
    format!(
        "{}:{}:{}:{}-{}",
        context.relative_path,
        symbol_kind_id(&kind),
        name,
        range.start_line,
        range.end_line,
    )
}

pub(crate) fn symbol_kind_id(kind: &SymbolKind) -> &'static str {
    match kind {
        SymbolKind::Module => "module",
        SymbolKind::Struct => "struct",
        SymbolKind::Enum => "enum",
        SymbolKind::Union => "union",
        SymbolKind::Trait => "trait",
        SymbolKind::TypeAlias => "type_alias",
        SymbolKind::Function => "function",
        SymbolKind::Method => "method",
        SymbolKind::Impl => "impl",
        SymbolKind::Const => "const",
        SymbolKind::Static => "static",
        SymbolKind::Import => "import",
        SymbolKind::Field => "field",
        SymbolKind::UnsafeBlock => "unsafe_block",
        SymbolKind::Other => "other",
    }
}

pub(crate) fn source_range<T: Spanned>(value: &T) -> SourceRange {
    source_range_from_span(value.span())
}

pub(crate) fn source_range_from_span(span: Span) -> SourceRange {
    let start = span.start();
    let end = span.end();
    SourceRange {
        start_line: start.line,
        end_line: end.line,
    }
}

pub(crate) fn provenance(context: &FileContext, range: SourceRange) -> RecordProvenance {
    RecordProvenance {
        repository_root: context.identity.root.clone(),
        commit_sha: context.identity.commit.clone(),
        worktree_identity: context.identity.worktree_identity.clone(),
        file_path: context.relative_path.clone(),
        source_range: range,
        parser_generation: context.parser_generation.to_string(),
    }
}

pub(crate) fn flatten_use_tree(
    tree: &syn::UseTree,
    prefix: &mut Vec<String>,
    out: &mut Vec<String>,
) {
    match tree {
        syn::UseTree::Path(path) => {
            prefix.push(path.ident.to_string());
            flatten_use_tree(&path.tree, prefix, out);
            prefix.pop();
        }
        syn::UseTree::Name(name) => {
            let mut full = prefix.clone();
            full.push(name.ident.to_string());
            out.push(full.join("::"));
        }
        syn::UseTree::Rename(rename) => {
            let mut full = prefix.clone();
            full.push(format!("{} as {}", rename.ident, rename.rename));
            out.push(full.join("::"));
        }
        syn::UseTree::Glob(_) => {
            let mut full = prefix.clone();
            full.push("*".to_string());
            out.push(full.join("::"));
        }
        syn::UseTree::Group(group) => {
            for item in &group.items {
                flatten_use_tree(item, prefix, out);
            }
        }
    }
}
