use crate::symbols::context::FileContext;
use crate::symbols::markers::{attr_bench, attr_test, declaration_markers};
use crate::symbols::probe::FunctionProbe;
use crate::symbols::relation::RelationCandidate;
use crate::symbols::utils::{
    dedupe_strings, flatten_use_tree, is_public_api, provenance, qualify, record_id,
    resolve_type_name, signature_text, source_range, to_visibility,
};
use crate::{SymbolKind, SymbolRecord, Visibility};
use syn::visit::Visit;
use syn::{ImplItem, ItemImpl, ItemUse};

pub(crate) fn emit_call_candidates(
    source_record_id: String,
    source_qualified: String,
    call_targets: Vec<(String, bool)>,
    relation_candidates: &mut Vec<RelationCandidate>,
) {
    relation_candidates.extend(
        call_targets
            .into_iter()
            .map(|(target_path, self_receiver)| RelationCandidate::Calls {
                source_record_id: source_record_id.clone(),
                source_qualified: source_qualified.clone(),
                target_path,
                self_receiver,
            }),
    );
}

pub(crate) fn resolve_path_for_scope(raw_path: &str, module_stack: &[String]) -> String {
    let trimmed = raw_path.trim();
    if let Some(trimmed) = trimmed.strip_prefix("crate::") {
        return trimmed.to_string();
    }
    if let Some(trimmed) = trimmed.strip_prefix("self::") {
        if module_stack.is_empty() {
            return trimmed.to_string();
        }
        return format!("{}::{trimmed}", module_stack.join("::"));
    }
    if let Some(trimmed) = trimmed.strip_prefix("super::") {
        if module_stack.is_empty() {
            return trimmed.to_string();
        }
        if module_stack.len() == 1 {
            return trimmed.to_string();
        }
        let parent = module_stack[..module_stack.len() - 1].join("::");
        return format!("{}::{trimmed}", parent);
    }
    if !trimmed.contains("::") && !module_stack.is_empty() {
        return format!("{}::{trimmed}", module_stack.join("::"));
    }
    trimmed.to_string()
}

pub(crate) fn extract_impl(
    item: &ItemImpl,
    module_stack: &[String],
    context: &FileContext,
    symbols: &mut Vec<SymbolRecord>,
    relation_candidates: &mut Vec<RelationCandidate>,
) -> Result<(), crate::CodeIntelError> {
    let (impl_record, trait_name) = build_impl_record(item, module_stack, context);
    if let Some(trait_name) = trait_name {
        relation_candidates.push(RelationCandidate::Implements {
            source_record_id: impl_record.record_id.clone(),
            target_qualified: resolve_path_for_scope(&trait_name, module_stack),
        });
    }
    symbols.push(impl_record.clone());

    for impl_item in &item.items {
        let ImplItem::Fn(method) = impl_item else {
            continue;
        };
        extract_impl_method(
            method,
            module_stack,
            context,
            &impl_record.name,
            &impl_record.qualified_name,
            symbols,
            relation_candidates,
        )?;
    }

    Ok(())
}

fn build_impl_record(
    item: &ItemImpl,
    module_stack: &[String],
    context: &FileContext,
) -> (SymbolRecord, Option<String>) {
    let impl_type = resolve_type_name(&item.self_ty);
    let impl_range = source_range(item);
    let impl_name = qualify(module_stack, &impl_type);
    let trait_name = item.trait_.as_ref().map(|(_, path, _)| {
        path.segments
            .iter()
            .map(|segment| segment.ident.to_string())
            .collect::<Vec<_>>()
            .join("::")
    });
    let signature = match &trait_name {
        Some(trait_name) => format!("impl {trait_name} for {impl_type}"),
        None => format!("impl {impl_type}"),
    };

    let impl_record = SymbolRecord {
        record_id: record_id(&impl_name, SymbolKind::Impl, &impl_range, context),
        package: context.package.to_string(),
        target: context.target.to_string(),
        kind: SymbolKind::Impl,
        name: impl_type,
        qualified_name: impl_name,
        visibility: Visibility::Private,
        is_public_api: false,
        is_async: false,
        is_unsafe: item.unsafety.is_some(),
        is_test: context.is_test_target,
        is_bench: context.is_bench_target,
        signature: Some(signature),
        imports: Vec::new(),
        markers: context.file_markers.clone(),
        provenance: provenance(context, impl_range),
    };
    (impl_record, trait_name)
}

fn extract_impl_method(
    method: &syn::ImplItemFn,
    module_stack: &[String],
    context: &FileContext,
    impl_type: &str,
    impl_qualified: &str,
    symbols: &mut Vec<SymbolRecord>,
    relation_candidates: &mut Vec<RelationCandidate>,
) -> Result<(), crate::CodeIntelError> {
    let mut method_stack = module_stack.to_vec();
    method_stack.push(impl_type.to_string());
    let method_name = method.sig.ident.to_string();
    let method_qualified = qualify(&method_stack, &method_name);
    let mut probe = FunctionProbe::new(context, &method_qualified);
    probe.visit_signature(&method.sig);
    probe.visit_block(&method.block);
    let mut markers = declaration_markers(&method.attrs, &context.file_markers);
    markers.axum_routes = dedupe_strings(
        markers
            .axum_routes
            .into_iter()
            .chain(probe.axum_routes)
            .collect(),
    );
    markers.sqlx_queries = dedupe_strings(probe.sqlx_queries);
    let method_range = source_range(method);
    let record = SymbolRecord {
        record_id: record_id(
            &method_qualified,
            SymbolKind::Method,
            &method_range,
            context,
        ),
        package: context.package.to_string(),
        target: context.target.to_string(),
        kind: SymbolKind::Method,
        name: method_name,
        qualified_name: method_qualified.clone(),
        visibility: to_visibility(&method.vis),
        is_public_api: is_public_api(&method.vis),
        is_async: method.sig.asyncness.is_some(),
        is_unsafe: method.sig.unsafety.is_some() || probe.had_unsafe,
        is_test: attr_test(&method.attrs) || context.is_test_target,
        is_bench: attr_bench(&method.attrs) || context.is_bench_target,
        signature: Some(signature_text(&method.sig)),
        imports: Vec::new(),
        markers,
        provenance: provenance(context, method_range),
    };
    relation_candidates.push(RelationCandidate::Defines {
        source_module_qualified: impl_qualified.to_string(),
        target_record_id: record.record_id.clone(),
    });
    let record_id = record.record_id.clone();
    symbols.push(record);
    symbols.extend(probe.unsafe_records);
    emit_call_candidates(
        record_id,
        method_qualified,
        probe.call_targets,
        relation_candidates,
    );
    Ok(())
}

pub(crate) fn extract_imports(
    item: &ItemUse,
    module_stack: &[String],
    context: &FileContext,
) -> (Vec<SymbolRecord>, Vec<RelationCandidate>) {
    let mut symbols = Vec::new();
    let mut relation_candidates = Vec::new();
    let mut names = Vec::new();
    flatten_use_tree(&item.tree, &mut Vec::new(), &mut names);
    let markers = declaration_markers(&item.attrs, &context.file_markers);
    let range = source_range(item);

    for name in names {
        let local_name = import_local_name(&name);
        let qualified_name = qualify(module_stack, &local_name);
        let record_id = record_id(&qualified_name, SymbolKind::Import, &range, context);
        let import_target = resolve_import_target(&name, module_stack);
        symbols.push(SymbolRecord {
            record_id: record_id.clone(),
            package: context.package.to_string(),
            target: context.target.to_string(),
            kind: SymbolKind::Import,
            name: local_name,
            qualified_name,
            visibility: to_visibility(&item.vis),
            is_public_api: is_public_api(&item.vis),
            is_async: false,
            is_unsafe: false,
            is_test: attr_test(&item.attrs) || context.is_test_target,
            is_bench: attr_bench(&item.attrs) || context.is_bench_target,
            signature: Some(format!("use {name}")),
            imports: vec![name],
            markers: markers.clone(),
            provenance: provenance(context, range.clone()),
        });
        if let Some(target_qualified) = import_target {
            relation_candidates.push(RelationCandidate::Imports {
                source_record_id: record_id,
                target_qualified,
            });
        }
    }
    (symbols, relation_candidates)
}
fn import_local_name(raw_name: &str) -> String {
    if let Some((_, alias)) = raw_name.split_once(" as ") {
        return alias.trim().to_string();
    }
    if let Some(name) = raw_name.rsplit("::").next() {
        return name.to_string();
    }
    raw_name.trim().to_string()
}

fn resolve_import_target(raw_name: &str, module_stack: &[String]) -> Option<String> {
    let mut target = raw_name.trim().to_string();
    if target.ends_with("::*") {
        return None;
    }
    if let Some((left, _)) = target.split_once(" as ") {
        target = left.trim().to_string();
    }
    if target.is_empty() {
        return None;
    }
    Some(resolve_path_for_scope(&target, module_stack))
}
