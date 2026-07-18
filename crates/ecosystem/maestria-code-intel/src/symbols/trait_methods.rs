use crate::symbols::context::FileContext;
use crate::symbols::markers::{attr_bench, attr_test, declaration_markers};
use crate::symbols::probe::FunctionProbe;
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
) -> Vec<SymbolRecord> {
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
    records.extend(extract_methods(item, module_stack, context));
    records
}

fn extract_methods(
    item: &ItemTrait,
    module_stack: &[String],
    context: &FileContext,
) -> Vec<SymbolRecord> {
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
                qualified_name,
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
            let mut records = vec![record];
            records.extend(probe.unsafe_records);
            Some(records)
        })
        .flatten()
        .collect()
}
