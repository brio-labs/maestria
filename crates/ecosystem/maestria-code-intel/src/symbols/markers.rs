use crate::SymbolMarkers;
use syn::parse::Parser;
use syn::{Attribute, Expr, ExprMethodCall, Path};

const AXUM_ROUTE_NAMES: [&str; 13] = [
    "get", "post", "put", "delete", "patch", "head", "options", "trace", "route", "any", "connect",
    "fallback", "router",
];

const SQLX_PREFIXES: [&str; 9] = [
    "query",
    "query_as",
    "query_scalar",
    "query_file",
    "query_one",
    "fetch",
    "fetch_optional",
    "fetch_all",
    "query!",
];

pub(crate) fn declaration_markers(
    attrs: &[Attribute],
    file_markers: &SymbolMarkers,
) -> SymbolMarkers {
    let mut markers = file_markers.clone();
    markers.axum_routes = extract_route_markers(attrs);
    markers
}

pub(crate) fn extract_route_markers(attrs: &[Attribute]) -> Vec<String> {
    let mut routes = Vec::new();
    for attribute in attrs {
        if let Some(name) = attribute
            .path()
            .segments
            .last()
            .map(|segment| segment.ident.to_string())
            && AXUM_ROUTE_NAMES.contains(&name.as_str())
        {
            routes.push(name);
        }
    }
    routes.sort_unstable();
    routes.dedup();
    routes
}

pub(crate) fn file_markers(path: &std::path::Path, source: &str) -> SymbolMarkers {
    let file_name = match path.file_name().and_then(|name| name.to_str()) {
        Some(name) => name.to_ascii_lowercase(),
        None => String::new(),
    };

    let lowered = source.to_ascii_lowercase();

    SymbolMarkers {
        build_script: file_name == "build.rs" || lowered.contains("cargo:rerun-if-changed"),
        generated_code: file_name.ends_with(".generated.rs")
            || file_name.ends_with(".pb.rs")
            || lowered.contains("generated")
            || lowered.contains("do not edit"),
        axum_routes: Vec::new(),
        sqlx_queries: Vec::new(),
    }
}

pub(crate) fn marker_from_macro_path(path: &Path) -> Option<String> {
    let name = path.segments.last()?.ident.to_string();
    sqlx_marker_name(&name)
}

pub(crate) fn marker_from_call_expr(expression: &Expr) -> Option<String> {
    if let Expr::Path(expr_path) = expression {
        let name = expr_path.path.segments.last()?.ident.to_string();
        return sqlx_marker_name(&name);
    }
    None
}

pub(crate) fn marker_from_method_call(call: &ExprMethodCall) -> Option<String> {
    let name = call.method.to_string();
    AXUM_ROUTE_NAMES.contains(&name.as_str()).then_some(name)
}

fn sqlx_marker_name(name: &str) -> Option<String> {
    if SQLX_PREFIXES
        .iter()
        .any(|prefix| name == *prefix || name.starts_with(prefix))
    {
        Some(name.to_string())
    } else {
        None
    }
}

pub(crate) fn attr_test(attrs: &[Attribute]) -> bool {
    attrs.iter().any(|attribute| {
        if attribute
            .path()
            .segments
            .last()
            .is_some_and(|segment| segment.ident == "test")
        {
            true
        } else if attribute
            .path()
            .segments
            .last()
            .is_some_and(|segment| segment.ident == "cfg")
        {
            cfg_meta_contains(attribute, "test")
        } else {
            false
        }
    })
}

pub(crate) fn attr_bench(attrs: &[Attribute]) -> bool {
    attrs.iter().any(|attribute| {
        if attribute
            .path()
            .segments
            .last()
            .is_some_and(|segment| segment.ident == "bench")
        {
            true
        } else if attribute
            .path()
            .segments
            .last()
            .is_some_and(|segment| segment.ident == "cfg")
        {
            cfg_meta_contains(attribute, "bench")
        } else {
            false
        }
    })
}

pub(crate) fn cfg_meta_contains(attribute: &Attribute, marker: &str) -> bool {
    let syn::Meta::List(meta_list) = &attribute.meta else {
        return false;
    };
    match syn::punctuated::Punctuated::<syn::Meta, syn::Token![,]>::parse_terminated
        .parse2(meta_list.tokens.clone())
    {
        Ok(predicates) => predicates
            .iter()
            .any(|predicate| cfg_predicate_contains(predicate, marker)),
        Err(_) => false,
    }
}

fn cfg_predicate_contains(meta: &syn::Meta, marker: &str) -> bool {
    match meta {
        syn::Meta::Path(path) => path.is_ident(marker),
        syn::Meta::List(list) if list.path.is_ident("not") => false,
        syn::Meta::List(list) => {
            match syn::punctuated::Punctuated::<syn::Meta, syn::Token![,]>::parse_terminated
                .parse2(list.tokens.clone())
            {
                Ok(predicates) => predicates
                    .iter()
                    .any(|predicate| cfg_predicate_contains(predicate, marker)),
                Err(_) => false,
            }
        }
        syn::Meta::NameValue(_) => false,
    }
}
