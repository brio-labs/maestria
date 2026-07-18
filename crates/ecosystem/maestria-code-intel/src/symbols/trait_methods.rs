use crate::symbols::context::FileContext;
use crate::symbols::markers::{attr_bench, attr_test, declaration_markers};
use crate::symbols::probe::FunctionProbe;
use crate::symbols::relation::RelationCandidate;
use crate::symbols::utils::{
    dedupe_strings, is_public_api, provenance, qualify, record_id, signature_text, source_range,
    to_visibility,
};
use crate::{SymbolKind, SymbolRecord};
use syn::visit::Visit;
use syn::{ItemTrait, TraitItem};

pub(super) fn extract_trait(
    item: &ItemTrait,
    module_stack: &[String],
    context: &FileContext,
) -> (Vec<SymbolRecord>, Vec<RelationCandidate>) {
    let mut records = vec![crate::symbols::utils::simple_record(
        SymbolKind::Trait,
        &item.ident.to_string(),
        module_stack,
        &item.vis,
        &item.attrs,
        context,
        source_range(item),
    )];
    if let Some(record) = records.first_mut() {
        record.is_unsafe = item.unsafety.is_some();
    }
    let mut relation_candidates = Vec::new();
    let method_records = extract_methods(item, module_stack, context);
    for (method_records, calls) in method_records {
        records.extend(method_records);
        relation_candidates.extend(calls);
    }
    (records, relation_candidates)
}

fn extract_methods(
    item: &ItemTrait,
    module_stack: &[String],
    context: &FileContext,
) -> Vec<(Vec<SymbolRecord>, Vec<RelationCandidate>)> {
    let mut stack = module_stack.to_vec();
    stack.push(item.ident.to_string());
    item.items
        .iter()
        .filter_map(|trait_item| {
            let TraitItem::Fn(method) = trait_item else {
                return None;
            };
            let name = method.sig.ident.to_string();
            let qualified_name = qualify(&stack, &name);
            let mut probe = FunctionProbe::new(context, &qualified_name);
            probe.visit_signature(&method.sig);
            if let Some(default) = &method.default {
                probe.visit_block(default);
            }
            let range = source_range(method);
            let mut markers = declaration_markers(&method.attrs, &context.file_markers);
            markers.axum_routes = dedupe_strings(
                markers
                    .axum_routes
                    .into_iter()
                    .chain(probe.axum_routes)
                    .collect(),
            );
            markers.sqlx_queries = dedupe_strings(probe.sqlx_queries);
            let record = SymbolRecord {
                record_id: record_id(&qualified_name, SymbolKind::Method, &range, context),
                package: context.package.to_string(),
                target: context.target.to_string(),
                kind: SymbolKind::Method,
                name,
                qualified_name: qualified_name.clone(),
                visibility: to_visibility(&syn::Visibility::Inherited),
                is_public_api: is_public_api(&item.vis),
                is_async: method.sig.asyncness.is_some(),
                is_unsafe: method.sig.unsafety.is_some() || probe.had_unsafe,
                is_test: attr_test(&method.attrs) || context.is_test_target,
                is_bench: attr_bench(&method.attrs) || context.is_bench_target,
                signature: Some(signature_text(&method.sig)),
                imports: Vec::new(),
                markers,
                provenance: provenance(context, range),
            };
            let source_record_id = record.record_id.clone();
            let source_qualified = record.qualified_name.clone();
            let calls = probe
                .call_targets
                .into_iter()
                .map(|(target_path, self_receiver)| RelationCandidate::Calls {
                    source_record_id: source_record_id.clone(),
                    source_qualified: source_qualified.clone(),
                    target_path,
                    self_receiver,
                })
                .collect();
            let mut records = vec![record];
            records.extend(probe.unsafe_records);
            Some((records, calls))
        })
        .collect()
}
