use crate::symbols::context::FileContext;
use crate::symbols::markers::{
    marker_from_call_expr, marker_from_macro_path, marker_from_method_call,
};
use crate::symbols::utils::{provenance, record_id, source_range_from_span};
use crate::{SymbolKind, SymbolRecord, Visibility};
use proc_macro2::Span;
use std::collections::BTreeSet;
use syn::spanned::Spanned;
use syn::visit::{self, Visit};
use syn::{ExprCall, ExprMethodCall, ExprPath, ExprUnsafe, PatIdent};

#[derive(Debug)]
pub(super) struct FunctionProbe<'a> {
    context: &'a FileContext<'a>,
    base_name: String,
    pub(super) had_unsafe: bool,
    pub(super) unsafe_records: Vec<SymbolRecord>,
    pub(super) axum_routes: Vec<String>,
    pub(super) sqlx_queries: Vec<String>,
    /// Captured call targets for relation extraction.
    pub(super) call_targets: Vec<(String, bool)>,
    binding_scopes: Vec<BTreeSet<String>>,
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
            call_targets: Vec::new(),
            binding_scopes: vec![BTreeSet::new()],
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

    fn push_binding_scope(&mut self) {
        self.binding_scopes.push(BTreeSet::new());
    }

    fn pop_binding_scope(&mut self) {
        let _ = self.binding_scopes.pop();
    }

    fn bind_local(&mut self, name: String) {
        if let Some(scope) = self.binding_scopes.last_mut() {
            scope.insert(name);
        }
    }

    fn is_locally_bound(&self, name: &str) -> bool {
        self.binding_scopes
            .iter()
            .rev()
            .any(|scope| scope.contains(name))
    }

    fn collect_call_target(&mut self, path: &ExprPath, self_receiver: bool) {
        let target = path
            .path
            .segments
            .iter()
            .map(|segment| segment.ident.to_string())
            .collect::<Vec<_>>()
            .join("::");
        if !target.is_empty() {
            self.call_targets.push((target, self_receiver));
        }
    }
}

impl<'ast, 'a> Visit<'ast> for FunctionProbe<'a> {
    fn visit_expr_unsafe(&mut self, node: &'ast ExprUnsafe) {
        self.add_unsafe(node.block.span());
        visit::visit_expr_unsafe(self, node);
    }

    fn visit_expr_macro(&mut self, node: &'ast syn::ExprMacro) {
        if let Some(marker) = marker_from_macro_path(&node.mac.path) {
            self.sqlx_queries.push(marker);
        }
        visit::visit_expr_macro(self, node);
    }

    fn visit_expr_call(&mut self, node: &'ast ExprCall) {
        if let syn::Expr::Path(path) = &*node.func {
            let is_shadowed = path.path.segments.len() == 1
                && path
                    .path
                    .segments
                    .first()
                    .is_some_and(|segment| self.is_locally_bound(&segment.ident.to_string()));
            if !is_shadowed {
                self.collect_call_target(path, false);
            }
        }
        if let Some(marker) = marker_from_call_expr(&node.func) {
            self.sqlx_queries.push(marker);
        }
        visit::visit_expr_call(self, node);
    }

    fn visit_local(&mut self, node: &'ast syn::Local) {
        for attr in &node.attrs {
            self.visit_attribute(attr);
        }
        if let Some(init) = &node.init {
            self.visit_expr(&init.expr);
            if let Some((_, diverge)) = &init.diverge {
                self.visit_expr(diverge);
            }
        }
        self.visit_pat(&node.pat);
    }

    fn visit_block(&mut self, node: &'ast syn::Block) {
        self.push_binding_scope();
        for statement in &node.stmts {
            self.visit_stmt(statement);
        }
        self.pop_binding_scope();
    }

    fn visit_expr_let(&mut self, node: &'ast syn::ExprLet) {
        for attr in &node.attrs {
            self.visit_attribute(attr);
        }
        self.visit_expr(&node.expr);
        self.visit_pat(&node.pat);
    }

    fn visit_expr_if(&mut self, node: &'ast syn::ExprIf) {
        for attr in &node.attrs {
            self.visit_attribute(attr);
        }
        self.push_binding_scope();
        self.visit_expr(&node.cond);
        self.visit_block(&node.then_branch);
        self.pop_binding_scope();
        if let Some((_, else_branch)) = &node.else_branch {
            self.visit_expr(else_branch);
        }
    }

    fn visit_expr_while(&mut self, node: &'ast syn::ExprWhile) {
        for attr in &node.attrs {
            self.visit_attribute(attr);
        }
        self.push_binding_scope();
        self.visit_expr(&node.cond);
        self.visit_block(&node.body);
        self.pop_binding_scope();
    }

    fn visit_expr_closure(&mut self, node: &'ast syn::ExprClosure) {
        for attr in &node.attrs {
            self.visit_attribute(attr);
        }
        self.push_binding_scope();
        for input in &node.inputs {
            self.visit_pat(input);
        }
        self.visit_expr(&node.body);
        self.pop_binding_scope();
    }

    fn visit_arm(&mut self, node: &'ast syn::Arm) {
        for attr in &node.attrs {
            self.visit_attribute(attr);
        }
        self.push_binding_scope();
        self.visit_pat(&node.pat);
        if let Some((_, guard)) = &node.guard {
            self.visit_expr(guard);
        }
        self.visit_expr(&node.body);
        self.pop_binding_scope();
    }

    fn visit_expr_for_loop(&mut self, node: &'ast syn::ExprForLoop) {
        for attr in &node.attrs {
            self.visit_attribute(attr);
        }
        self.visit_expr(&node.expr);
        self.push_binding_scope();
        self.visit_pat(&node.pat);
        self.visit_block(&node.body);
        self.pop_binding_scope();
    }

    fn visit_pat_ident(&mut self, node: &'ast PatIdent) {
        self.bind_local(node.ident.to_string());
        visit::visit_pat_ident(self, node);
    }

    fn visit_expr_method_call(&mut self, node: &'ast ExprMethodCall) {
        if matches!(
            &*node.receiver,
            syn::Expr::Path(ExprPath { path, .. }) if path.is_ident("self")
        ) {
            self.call_targets.push((node.method.to_string(), true));
        }
        if let Some(marker) = marker_from_method_call(node) {
            self.axum_routes.push(marker);
        }
        visit::visit_expr_method_call(self, node);
    }
}
