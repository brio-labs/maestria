use crate::markers::CodeMarker;
use crate::symbols::comments;
use crate::symbols::compound;
use crate::symbols::context::FileContext;
use crate::symbols::markers::{attr_bench, attr_test, declaration_markers};
use crate::symbols::relation::RelationCandidate;
use crate::symbols::trait_methods;
use crate::symbols::utils::{
    dedupe_strings, doc_comment_from_attrs, is_public_api, provenance, qualify, record_id,
    signature_text, simple_record, source_range, to_visibility,
};
use crate::{CodeIntelError, SourceRange, SymbolKind, SymbolRecord, Visibility};
use syn::visit::Visit;
use syn::{Attribute, File as SynFile, Item, ItemFn, ItemMod};

pub(crate) fn extract_file_symbols(
    source: &str,
    context: &FileContext,
    initial_module_stack: &[String],
) -> Result<(Vec<SymbolRecord>, Vec<RelationCandidate>), CodeIntelError> {
    let file: SynFile = syn::parse_file(source).map_err(|error| CodeIntelError::Parse {
        context: format!("parse rust source: {}", context.relative_path),
        details: error.to_string(),
    })?;

    // File-level `//!` inner docs attach to the file's root module symbol.
    let file_doc = doc_comment_from_attrs(&file.attrs);
    let comment_markers = comments::scan_comment_markers(source);

    let mut symbols = Vec::new();
    let mut relation_candidates = Vec::new();
    let mut module_stack = initial_module_stack.to_vec();

    for item in &file.items {
        extract_item(
            item,
            &mut module_stack,
            context,
            &mut symbols,
            &mut relation_candidates,
        )?;
    }

    let orphans = comments::attach_comment_markers(&mut symbols, comment_markers)?;
    if file_doc.is_some() || !orphans.is_empty() {
        let file_module = file_root_module(context, initial_module_stack, file_doc, source)?;
        symbols.insert(0, file_module);
        if let Some(file_module) = symbols.first_mut() {
            for orphan in orphans {
                let code_marker = CodeMarker::new(orphan.kind, orphan.start_line, orphan.end_line)
                    .map_err(|error| CodeIntelError::Integrity {
                        context: "attach orphan comment marker".to_string(),
                        details: error.to_string(),
                    })?;
                file_module.markers.code_markers.push(code_marker);
            }
        }
    }

    Ok((symbols, relation_candidates))
}

/// The synthetic root-module symbol for a file: the crate-root module for
/// target roots (`lib.rs`/`main.rs`) or the module the file implements
/// (`foo.rs`/`foo/mod.rs`). It carries the file's `//!` docs and any comment
/// markers no item's range contains. The qualified name is crate-relative
/// (`crate` / `crate::module`) so it never collides with a `mod` declaration
/// in a parent file during relation resolution.
fn file_root_module(
    context: &FileContext,
    module_stack: &[String],
    doc_comment: Option<String>,
    source: &str,
) -> Result<SymbolRecord, CodeIntelError> {
    let name = match module_stack.last() {
        Some(segment) => segment.to_string(),
        None => file_stem(context),
    };
    let qualified_name = if module_stack.is_empty() {
        "crate".to_string()
    } else {
        format!("crate::{}", module_stack.join("::"))
    };
    let line_count = source.lines().count().max(1);
    let range = SourceRange::new(1, line_count).map_err(|error| CodeIntelError::Integrity {
        context: "derive file root module range".to_string(),
        details: error.to_string(),
    })?;
    Ok(SymbolRecord {
        record_id: record_id(&qualified_name, SymbolKind::Module, &range, context),
        package: context.package.to_string(),
        target: context.target.to_string(),
        kind: SymbolKind::Module,
        name,
        qualified_name: qualified_name.clone(),
        visibility: Visibility::Private,
        is_public_api: false,
        is_async: false,
        is_unsafe: false,
        is_test: context.is_test_target,
        is_bench: context.is_bench_target,
        signature: Some(qualified_name.clone()),
        imports: Vec::new(),
        doc_comment,
        markers: context.file_markers.clone(),
        provenance: provenance(context, range),
    })
}

fn file_stem(context: &FileContext) -> String {
    std::path::Path::new(&context.relative_path)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .map_or_else(|| "root".to_string(), str::to_string)
}

fn extract_item(
    item: &Item,
    module_stack: &mut Vec<String>,
    context: &FileContext,
    symbols: &mut Vec<SymbolRecord>,
    relation_candidates: &mut Vec<RelationCandidate>,
) -> Result<(), CodeIntelError> {
    if let Some(simple) = simple_item(item)? {
        let record = simple_record_for(
            simple.kind,
            simple.name,
            simple.visibility,
            simple.attrs,
            module_stack,
            context,
            simple.range,
        );
        maybe_emit_define_candidate(module_stack, &record, relation_candidates);
        symbols.push(record);
        return Ok(());
    }

    match item {
        Item::Mod(item_mod) => extract_module(
            item_mod,
            module_stack,
            context,
            symbols,
            relation_candidates,
        ),
        Item::Trait(item_trait) => {
            let (records, calls) = trait_methods::extract_trait(item_trait, module_stack, context)?;
            for record in records {
                maybe_emit_define_candidate(module_stack, &record, relation_candidates);
                symbols.push(record);
            }
            relation_candidates.extend(calls);
            Ok(())
        }
        Item::Fn(item_fn) => {
            let (record, unsafe_records, calls) = extract_function(item_fn, module_stack, context)?;
            maybe_emit_define_candidate(module_stack, &record, relation_candidates);
            symbols.push(record);
            symbols.extend(unsafe_records);
            relation_candidates.extend(calls);
            Ok(())
        }
        Item::Use(item_use) => {
            let (records, calls) = compound::extract_imports(item_use, module_stack, context)?;
            for record in records {
                maybe_emit_define_candidate(module_stack, &record, relation_candidates);
                symbols.push(record);
            }
            relation_candidates.extend(calls);
            Ok(())
        }
        Item::Impl(item_impl) => compound::extract_impl(
            item_impl,
            module_stack,
            context,
            symbols,
            relation_candidates,
        ),
        _ => Ok(()),
    }
}

struct SimpleItem<'a> {
    kind: SymbolKind,
    name: &'a syn::Ident,
    visibility: &'a syn::Visibility,
    attrs: &'a [Attribute],
    range: SourceRange,
}

fn simple_item(item: &Item) -> Result<Option<SimpleItem<'_>>, CodeIntelError> {
    Ok(match item {
        Item::Struct(item) => Some(SimpleItem {
            kind: SymbolKind::Struct,
            name: &item.ident,
            visibility: &item.vis,
            attrs: &item.attrs,
            range: source_range(item)?,
        }),
        Item::Enum(item) => Some(SimpleItem {
            kind: SymbolKind::Enum,
            name: &item.ident,
            visibility: &item.vis,
            attrs: &item.attrs,
            range: source_range(item)?,
        }),
        Item::Union(item) => Some(SimpleItem {
            kind: SymbolKind::Union,
            name: &item.ident,
            visibility: &item.vis,
            attrs: &item.attrs,
            range: source_range(item)?,
        }),
        Item::Type(item) => Some(SimpleItem {
            kind: SymbolKind::TypeAlias,
            name: &item.ident,
            visibility: &item.vis,
            attrs: &item.attrs,
            range: source_range(item)?,
        }),
        Item::Const(item) => Some(SimpleItem {
            kind: SymbolKind::Const,
            name: &item.ident,
            visibility: &item.vis,
            attrs: &item.attrs,
            range: source_range(item)?,
        }),
        Item::Static(item) => Some(SimpleItem {
            kind: SymbolKind::Static,
            name: &item.ident,
            visibility: &item.vis,
            attrs: &item.attrs,
            range: source_range(item)?,
        }),
        _ => None,
    })
}

fn maybe_emit_define_candidate(
    module_stack: &[String],
    record: &SymbolRecord,
    relation_candidates: &mut Vec<RelationCandidate>,
) {
    let source_module_qualified = if matches!(&record.kind, SymbolKind::Method) {
        record
            .qualified_name
            .rsplit_once("::")
            .map(|(parent, _)| parent.to_string())
    } else if module_stack.is_empty() {
        None
    } else {
        Some(module_stack.join("::"))
    };
    if let Some(source_module_qualified) = source_module_qualified {
        relation_candidates.push(RelationCandidate::Defines {
            source_module_qualified,
            target_record_id: record.record_id.clone(),
        });
    }
}

fn simple_record_for(
    kind: SymbolKind,
    name: &syn::Ident,
    visibility: &syn::Visibility,
    attrs: &[Attribute],
    module_stack: &[String],
    context: &FileContext,
    range: SourceRange,
) -> SymbolRecord {
    simple_record(
        kind,
        &name.to_string(),
        module_stack,
        visibility,
        attrs,
        context,
        range,
    )
}

fn extract_module(
    item: &ItemMod,
    module_stack: &mut Vec<String>,
    context: &FileContext,
    symbols: &mut Vec<SymbolRecord>,
    relation_candidates: &mut Vec<RelationCandidate>,
) -> Result<(), CodeIntelError> {
    let record = simple_record(
        SymbolKind::Module,
        &item.ident.to_string(),
        module_stack,
        &item.vis,
        &item.attrs,
        context,
        source_range(item)?,
    );
    maybe_emit_define_candidate(module_stack, &record, relation_candidates);
    symbols.push(record);

    if let Some((_, items)) = &item.content {
        let nested_context = context.nested(&item.attrs);
        module_stack.push(item.ident.to_string());
        for nested in items {
            extract_item(
                nested,
                module_stack,
                &nested_context,
                symbols,
                relation_candidates,
            )?;
        }
        module_stack.pop();
    }

    Ok(())
}

fn extract_function(
    function: &ItemFn,
    module_stack: &[String],
    context: &FileContext,
) -> Result<(SymbolRecord, Vec<SymbolRecord>, Vec<RelationCandidate>), CodeIntelError> {
    let name = function.sig.ident.to_string();
    let qualified = qualify(module_stack, &name);
    let mut probe = crate::symbols::probe::FunctionProbe::new(context, &qualified);
    probe.visit_signature(&function.sig);
    probe.visit_block(&function.block);

    if let Some(error) = probe.take_error() {
        return Err(error);
    }
    let mut markers = declaration_markers(&function.attrs, &context.file_markers);
    markers.axum_routes = dedupe_strings(
        markers
            .axum_routes
            .into_iter()
            .chain(probe.axum_routes)
            .collect(),
    );
    markers.sqlx_queries = dedupe_strings(probe.sqlx_queries);
    let range = source_range(function)?;
    let record = SymbolRecord {
        record_id: record_id(&qualified, SymbolKind::Function, &range, context),
        package: context.package.to_string(),
        target: context.target.to_string(),
        kind: SymbolKind::Function,
        name,
        qualified_name: qualified.clone(),
        visibility: to_visibility(&function.vis),
        is_public_api: is_public_api(&function.vis),
        is_async: function.sig.asyncness.is_some(),
        is_unsafe: function.sig.unsafety.is_some() || probe.had_unsafe,
        is_test: attr_test(&function.attrs) || context.is_test_target,
        is_bench: attr_bench(&function.attrs) || context.is_bench_target,
        signature: Some(signature_text(&function.sig)),
        imports: Vec::new(),
        doc_comment: doc_comment_from_attrs(&function.attrs),
        markers,
        provenance: provenance(context, range),
    };
    let mut calls = Vec::new();
    compound::emit_call_candidates(
        record.record_id.clone(),
        qualified,
        module_stack.join("::"),
        probe.call_targets,
        &mut calls,
    );
    Ok((record, probe.unsafe_records, calls))
}
