use crate::symbols::context::FileContext;
use crate::symbols::markers::{
    marker_from_call_expr, marker_from_macro_path, marker_from_method_call,
};
use crate::symbols::utils::{provenance, record_id, source_range_from_span};
use crate::{SymbolKind, SymbolRecord, Visibility};
use proc_macro2::Span;
use syn::spanned::Spanned;
use syn::visit::{self, Visit};
use syn::{ExprCall, ExprMacro, ExprUnsafe};

#[derive(Debug)]
pub(super) struct FunctionProbe<'a> {
    context: &'a FileContext<'a>,
    base_name: String,
    pub(super) had_unsafe: bool,
    pub(super) unsafe_records: Vec<SymbolRecord>,
    pub(super) axum_routes: Vec<String>,
    pub(super) sqlx_queries: Vec<String>,
    unsafe_count: usize,
}

impl<'a> FunctionProbe<'a> {
    pub(super) fn new(context: &'a FileContext<'a>, base_name: &str) -> Self {
        Self {
            context,
            base_name: base_name.to_string(),
            had_unsafe: false,
            unsafe_records: Vec::new(),
            axum_routes: Vec::new(),
            sqlx_queries: Vec::new(),
            unsafe_count: 0,
        }
    }

    fn add_unsafe(&mut self, span: Span) {
        self.had_unsafe = true;
        self.unsafe_count += 1;
        let name = format!("{}_unsafe_{}", self.base_name, self.unsafe_count);
        let range = source_range_from_span(span);

        self.unsafe_records.push(SymbolRecord {
            record_id: record_id(&name, SymbolKind::UnsafeBlock, &range, self.context),
            package: self.context.package.to_string(),
            target: self.context.target.to_string(),
            kind: SymbolKind::UnsafeBlock,
            name: name.clone(),
            qualified_name: self.base_name.clone(),
            visibility: Visibility::Private,
            is_public_api: false,
            is_async: false,
            is_unsafe: true,
            is_test: self.context.is_test_target,
            is_bench: self.context.is_bench_target,
            signature: Some("unsafe".to_string()),
            imports: Vec::new(),
            markers: self.context.file_markers.clone(),
            provenance: provenance(self.context, range),
        });
    }
}

impl<'ast, 'a> Visit<'ast> for FunctionProbe<'a> {
    fn visit_expr_unsafe(&mut self, node: &'ast ExprUnsafe) {
        self.add_unsafe(node.block.span());
        visit::visit_expr_unsafe(self, node);
    }

    fn visit_expr_macro(&mut self, node: &'ast ExprMacro) {
        if let Some(marker) = marker_from_macro_path(&node.mac.path) {
            self.sqlx_queries.push(marker);
        }
        visit::visit_expr_macro(self, node);
    }

    fn visit_expr_call(&mut self, node: &'ast ExprCall) {
        if let Some(marker) = marker_from_call_expr(&node.func) {
            self.sqlx_queries.push(marker);
        }
        visit::visit_expr_call(self, node);
    }

    fn visit_expr_method_call(&mut self, node: &'ast syn::ExprMethodCall) {
        if let Some(marker) = marker_from_method_call(node) {
            self.axum_routes.push(marker);
        }
        visit::visit_expr_method_call(self, node);
    }
}
