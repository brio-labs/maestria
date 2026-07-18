use crate::symbols::context::FileContext;
use crate::symbols::markers::{attr_bench, attr_test, declaration_markers};
use crate::symbols::probe::FunctionProbe;
use crate::symbols::utils::{
    dedupe_strings, flatten_use_tree, is_public_api, provenance, qualify, record_id,
    resolve_type_name, signature_text, simple_record, source_range, to_visibility,
};
use crate::{CodeIntelError, SourceRange, SymbolKind, SymbolRecord, Visibility};
use syn::visit::Visit;
use syn::{Attribute, File as SynFile, ImplItem, Item, ItemFn, ItemImpl, ItemMod, ItemUse};
pub(crate) fn extract_file_symbols(
    source: &str,
    context: &FileContext,
    initial_module_stack: &[String],
) -> Result<Vec<SymbolRecord>, CodeIntelError> {
    let file: SynFile = syn::parse_file(source).map_err(|error| CodeIntelError::Parse {
        context: format!("parse rust source: {}", context.relative_path),
        details: error.to_string(),
    })?;

    let mut symbols = Vec::new();
    let mut module_stack = initial_module_stack.to_vec();

    for item in &file.items {
        extract_item(item, &mut module_stack, context, &mut symbols)?;
    }

    Ok(symbols)
}

fn extract_item(
    item: &Item,
    module_stack: &mut Vec<String>,
    context: &FileContext,
    out: &mut Vec<SymbolRecord>,
) -> Result<(), CodeIntelError> {
    match item {
        Item::Mod(item_mod) => extract_module(item_mod, module_stack, context, out),
        Item::Struct(item_struct) => {
            out.push(simple_record_for(
                SymbolKind::Struct,
                &item_struct.ident,
                &item_struct.vis,
                &item_struct.attrs,
                module_stack,
                context,
                source_range(item_struct),
            ));
            Ok(())
        }
        Item::Enum(item_enum) => {
            out.push(simple_record_for(
                SymbolKind::Enum,
                &item_enum.ident,
                &item_enum.vis,
                &item_enum.attrs,
                module_stack,
                context,
                source_range(item_enum),
            ));
            Ok(())
        }
        Item::Union(item_union) => {
            out.push(simple_record_for(
                SymbolKind::Union,
                &item_union.ident,
                &item_union.vis,
                &item_union.attrs,
                module_stack,
                context,
                source_range(item_union),
            ));
            Ok(())
        }
        Item::Trait(item_trait) => {
            out.extend(crate::symbols::trait_methods::extract_trait(
                item_trait,
                module_stack,
                context,
            ));
            Ok(())
        }
        Item::Type(item_type) => {
            out.push(simple_record_for(
                SymbolKind::TypeAlias,
                &item_type.ident,
                &item_type.vis,
                &item_type.attrs,
                module_stack,
                context,
                source_range(item_type),
            ));
            Ok(())
        }
        Item::Fn(item_fn) => {
            let (record, unsafe_records) = extract_function(item_fn, module_stack, context)?;
            out.push(record);
            out.extend(unsafe_records);
            Ok(())
        }
        Item::Const(item_const) => {
            out.push(simple_record_for(
                SymbolKind::Const,
                &item_const.ident,
                &item_const.vis,
                &item_const.attrs,
                module_stack,
                context,
                source_range(item_const),
            ));
            Ok(())
        }
        Item::Static(item_static) => {
            out.push(simple_record_for(
                SymbolKind::Static,
                &item_static.ident,
                &item_static.vis,
                &item_static.attrs,
                module_stack,
                context,
                source_range(item_static),
            ));
            Ok(())
        }
        Item::Use(item_use) => {
            out.extend(extract_imports(item_use, module_stack, context));
            Ok(())
        }
        Item::Impl(item_impl) => extract_impl(item_impl, module_stack, context, out),
        _ => Ok(()),
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
    out: &mut Vec<SymbolRecord>,
) -> Result<(), CodeIntelError> {
    out.push(simple_record(
        SymbolKind::Module,
        &item.ident.to_string(),
        module_stack,
        &item.vis,
        &item.attrs,
        context,
        source_range(item),
    ));

    if let Some((_, items)) = &item.content {
        let nested_context = context.nested(&item.attrs);
        module_stack.push(item.ident.to_string());
        for nested in items {
            extract_item(nested, module_stack, &nested_context, out)?;
        }
        module_stack.pop();
    }

    Ok(())
}

fn extract_function(
    function: &ItemFn,
    module_stack: &[String],
    context: &FileContext,
) -> Result<(SymbolRecord, Vec<SymbolRecord>), CodeIntelError> {
    let name = function.sig.ident.to_string();
    let qualified = qualify(module_stack, &name);
    let mut probe = FunctionProbe::new(context, &qualified);
    probe.visit_signature(&function.sig);
    probe.visit_block(&function.block);

    let mut markers = declaration_markers(&function.attrs, &context.file_markers);
    markers.axum_routes = dedupe_strings(
        markers
            .axum_routes
            .into_iter()
            .chain(probe.axum_routes)
            .collect(),
    );
    markers.sqlx_queries = dedupe_strings(probe.sqlx_queries);
    let range = source_range(function);

    let record = SymbolRecord {
        record_id: record_id(&qualified, SymbolKind::Function, &range, context),
        package: context.package.to_string(),
        target: context.target.to_string(),
        kind: SymbolKind::Function,
        name,
        qualified_name: qualified,
        visibility: to_visibility(&function.vis),
        is_public_api: is_public_api(&function.vis),
        is_async: function.sig.asyncness.is_some(),
        is_unsafe: function.sig.unsafety.is_some() || probe.had_unsafe,
        is_test: attr_test(&function.attrs) || context.is_test_target,
        is_bench: attr_bench(&function.attrs) || context.is_bench_target,
        signature: Some(signature_text(&function.sig)),
        imports: Vec::new(),
        markers,
        provenance: provenance(context, range.clone()),
    };

    Ok((record, probe.unsafe_records))
}

fn extract_impl(
    item: &ItemImpl,
    module_stack: &[String],
    context: &FileContext,
    out: &mut Vec<SymbolRecord>,
) -> Result<(), CodeIntelError> {
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
    let signature = match trait_name {
        Some(trait_name) => format!("impl {trait_name} for {impl_type}"),
        None => format!("impl {impl_type}"),
    };

    out.push(SymbolRecord {
        record_id: record_id(&impl_name, SymbolKind::Impl, &impl_range, context),
        package: context.package.to_string(),
        target: context.target.to_string(),
        kind: SymbolKind::Impl,
        name: impl_type.clone(),
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
    });

    for impl_item in &item.items {
        if let ImplItem::Fn(method) = impl_item {
            let method_name = method.sig.ident.to_string();
            let mut stack = module_stack.to_vec();
            stack.push(impl_type.clone());
            let qualified = qualify(&stack, &method_name);

            let mut probe = FunctionProbe::new(context, &qualified);
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

            let method_record = SymbolRecord {
                record_id: record_id(&qualified, SymbolKind::Method, &method_range, context),
                package: context.package.to_string(),
                target: context.target.to_string(),
                kind: SymbolKind::Method,
                name: method_name,
                qualified_name: qualified,
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

            out.push(method_record);
            out.extend(probe.unsafe_records);
        }
    }

    Ok(())
}

fn extract_imports(
    item: &ItemUse,
    module_stack: &[String],
    context: &FileContext,
) -> Vec<SymbolRecord> {
    let mut names = Vec::new();
    flatten_use_tree(&item.tree, &mut Vec::new(), &mut names);
    let markers = declaration_markers(&item.attrs, &context.file_markers);

    names
        .into_iter()
        .map(|name| {
            let range = source_range(item);
            SymbolRecord {
                record_id: record_id(
                    &qualify(module_stack, &name),
                    SymbolKind::Import,
                    &range,
                    context,
                ),
                package: context.package.to_string(),
                target: context.target.to_string(),
                kind: SymbolKind::Import,
                name: name.clone(),
                qualified_name: qualify(module_stack, &name),
                visibility: to_visibility(&item.vis),
                is_public_api: is_public_api(&item.vis),
                is_async: false,
                is_unsafe: false,
                is_test: attr_test(&item.attrs) || context.is_test_target,
                is_bench: attr_bench(&item.attrs) || context.is_bench_target,
                signature: Some(format!("use {name}")),
                imports: vec![name],
                markers: markers.clone(),
                provenance: provenance(context, range),
            }
        })
        .collect()
}
