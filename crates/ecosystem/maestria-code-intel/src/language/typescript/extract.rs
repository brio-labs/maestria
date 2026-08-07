//! Deterministic tokenizer-based TypeScript/JavaScript symbol extraction.
//!
//! Masks strings, comments, templates, and regex literals (see `tokens`),
//! then detects module-level declarations (`function`/`class`/`interface`/
//! `type`/`const`/`let`/`var`, with `export`/`export default` prefixes),
//! class methods, imports, and call expressions from the masked text with
//! pure line/brace/paren rules. Arrow bindings whose body carries a JSX
//! marker (`<` followed by an ASCII letter, or `<>`) become component
//! `Function` symbols; the heuristic is deterministic and documented, and
//! false positives degrade only the kind of a `const` binding, never record
//! correctness.
//!
//! Anonymous default exports (`export default function`/`class`/arrow with
//! no name) get the name `"default"` and the qualified name
//! `{module}::default`; non-declaration `export default <expression>;`
//! statements produce no symbol.

use crate::language::typescript::calls::cut_signature;
use crate::language::typescript::statements::{
    DeclarationKind, ParsedDeclaration, brace_delta, join_qualified, paren_balance,
    parse_declaration, parse_method,
};
use crate::language::typescript::tokens::{WebMasker, module_path_for_file};
use crate::symbols::RelationCandidate;
use crate::symbols::context::FileContext;
use crate::symbols::utils::{provenance, record_id};
use crate::types::{SourceRange, SymbolRecord};
use crate::{CodeIntelError, SymbolKind, Visibility};

/// Everything extracted from one web source file.
pub(crate) struct WebFileExtraction {
    pub(crate) symbols: Vec<SymbolRecord>,
    pub(crate) candidates: Vec<RelationCandidate>,
}

/// Extract symbols and relation candidates from one `.ts`/`.tsx`/`.js` file.
pub(crate) fn extract_web_file(
    source: &str,
    rel_path: &str,
    context: &FileContext,
) -> Result<WebFileExtraction, CodeIntelError> {
    let lines: Vec<&str> = source.split('\n').collect();
    let line_count = lines.len().max(1);
    let mut masker = WebMasker::default();
    let mut original = Vec::with_capacity(lines.len());
    let mut masked = Vec::with_capacity(lines.len());
    for line in &lines {
        original.push((*line).to_string());
        masked.push(masker.mask_line(line));
    }
    let module_path = module_path_for_file(rel_path);
    let mut extractor = FileExtractor {
        masked: &masked,
        original: &original,
        module_path,
        rel_path,
        context,
        line_count,
        brace_depth: 0,
        paren_depth: 0,
        symbols: Vec::new(),
        candidates: Vec::new(),
    };
    extractor.run()?;
    Ok(WebFileExtraction {
        symbols: extractor.symbols,
        candidates: extractor.candidates,
    })
}

/// Kind-independent record flags shared by every declaration record.
#[derive(Debug, Clone, Copy)]
pub(crate) struct RecordFlags {
    pub(crate) is_async: bool,
    pub(crate) exported: bool,
}

/// An open declaration scope (class or function) with its block bounds.
struct Scope {
    record_id: String,
    qualified_name: String,
    kind: ScopeKind,
    body_depth: isize,
    end_line: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ScopeKind {
    Class,
    Function,
}

/// Mutable per-file extraction state; `impl` blocks live in the sibling
/// `imports`/`calls`/`jsx` modules, so the fields are crate-visible.
pub(crate) struct FileExtractor<'a> {
    pub(crate) masked: &'a [String],
    pub(crate) original: &'a [String],
    pub(crate) module_path: String,
    pub(crate) rel_path: &'a str,
    pub(crate) context: &'a FileContext<'a>,
    pub(crate) line_count: usize,
    pub(crate) brace_depth: isize,
    pub(crate) paren_depth: isize,
    pub(crate) symbols: Vec<SymbolRecord>,
    pub(crate) candidates: Vec<RelationCandidate>,
}

impl<'a> FileExtractor<'a> {
    fn run(&mut self) -> Result<(), CodeIntelError> {
        self.emit_module_symbol()?;
        let mut scopes: Vec<Scope> = Vec::new();
        let mut decorator_start: Option<usize> = None;
        for index in 0..self.masked.len() {
            let depth_before = self.brace_depth;
            let paren_before = self.paren_depth;
            let trimmed = self.masked[index].trim();
            while scopes
                .last()
                .is_some_and(|scope| scope.end_line < index || scope.body_depth > depth_before)
            {
                scopes.pop();
            }
            if trimmed.is_empty() {
                self.update_depths(index);
                continue;
            }
            if depth_before == 0 && paren_before == 0 {
                if trimmed.starts_with('@') {
                    if decorator_start.is_none() {
                        decorator_start = Some(index);
                    }
                    self.update_depths(index);
                    continue;
                }
                if let Some(start) = decorator_start.take()
                    && let Some(declaration) = parse_declaration(trimmed)
                {
                    self.handle_declaration(
                        index,
                        depth_before,
                        declaration,
                        Some(start),
                        &mut scopes,
                    )?;
                    self.update_depths(index);
                    continue;
                }
                if (trimmed.starts_with("import ")
                    || trimmed.starts_with("export {")
                    || trimmed.starts_with("export *"))
                    && let Some(statement) = self.collect_import_statement(index)
                {
                    self.emit_imports(&statement, index)?;
                    self.update_depths(index);
                    continue;
                }
                if let Some(declaration) = parse_declaration(trimmed) {
                    self.handle_declaration(index, depth_before, declaration, None, &mut scopes)?;
                    self.update_depths(index);
                    continue;
                }
            } else if let Some((kind, body_depth, record_id, qualified_name)) =
                scopes.last().map(|scope| {
                    (
                        scope.kind,
                        scope.body_depth,
                        scope.record_id.clone(),
                        scope.qualified_name.clone(),
                    )
                })
            {
                if kind == ScopeKind::Class
                    && depth_before == body_depth
                    && let Some((method, is_async)) = parse_method(trimmed)
                {
                    self.handle_method(
                        index,
                        depth_before,
                        &method,
                        is_async,
                        &qualified_name,
                        &mut scopes,
                    )?;
                    self.update_depths(index);
                    continue;
                }
                if kind == ScopeKind::Function {
                    self.emit_body_calls(index, &record_id);
                }
            }
            self.update_depths(index);
        }
        Ok(())
    }

    fn update_depths(&mut self, index: usize) {
        self.brace_depth += brace_delta(&self.masked[index]);
        self.paren_depth += paren_balance(&self.masked[index]);
        if self.brace_depth < 0 {
            self.brace_depth = 0;
        }
        if self.paren_depth < 0 {
            self.paren_depth = 0;
        }
    }

    fn emit_module_symbol(&mut self) -> Result<(), CodeIntelError> {
        let range = SourceRange::new(1, self.line_count)?;
        let record = SymbolRecord {
            record_id: record_id(&self.module_path, SymbolKind::Module, &range, self.context),
            package: self.context.package.to_string(),
            target: self.context.target.to_string(),
            kind: SymbolKind::Module,
            name: self.module_path.clone(),
            qualified_name: self.module_path.clone(),
            visibility: Visibility::Public,
            is_public_api: true,
            is_async: false,
            is_unsafe: false,
            is_test: self.context.is_test_target,
            is_bench: self.context.is_bench_target,
            signature: Some(self.module_path.clone()),
            imports: Vec::new(),
            doc_comment: None,
            markers: Default::default(),
            provenance: provenance(self.context, range),
        };
        self.symbols.push(record);
        Ok(())
    }

    fn handle_declaration(
        &mut self,
        index: usize,
        depth_before: isize,
        declaration: ParsedDeclaration,
        decorator_start: Option<usize>,
        scopes: &mut Vec<Scope>,
    ) -> Result<(), CodeIntelError> {
        let start_line = match decorator_start {
            Some(start) => start,
            None => index,
        };
        let block_end = self.compute_block_end(index, depth_before);
        let range = SourceRange::new(start_line + 1, block_end + 1)?;
        let qualified = join_qualified(&self.module_path, &declaration.name);
        match declaration.kind {
            DeclarationKind::Function { .. } | DeclarationKind::Class => {
                let (kind, scope_kind, is_async) = match declaration.kind {
                    DeclarationKind::Function { is_async } => {
                        (SymbolKind::Function, ScopeKind::Function, is_async)
                    }
                    _ => (SymbolKind::Class, ScopeKind::Class, false),
                };
                let signature = self.gather_signature(index);
                let record = self.symbol_record(
                    kind,
                    &declaration.name,
                    &qualified,
                    &range,
                    Some(signature),
                    RecordFlags {
                        is_async,
                        exported: declaration.exported,
                    },
                );
                self.symbols.push(record.clone());
                scopes.push(Scope {
                    record_id: record.record_id,
                    qualified_name: qualified,
                    kind: scope_kind,
                    body_depth: depth_before + 1,
                    end_line: block_end,
                });
            }
            DeclarationKind::Interface | DeclarationKind::TypeAlias => {
                let signature = self.gather_signature(index);
                let record = self.symbol_record(
                    SymbolKind::TypeAlias,
                    &declaration.name,
                    &qualified,
                    &range,
                    Some(signature),
                    RecordFlags {
                        is_async: false,
                        exported: declaration.exported,
                    },
                );
                self.symbols.push(record);
            }
            DeclarationKind::Const { is_let } => {
                let keyword = if is_let { "let" } else { "const" };
                let signature = Some(format!("{keyword} {} = …", declaration.name));
                let (kind, is_async) = if self.is_arrow_component(index) {
                    (SymbolKind::Function, self.arrow_is_async(index))
                } else if is_let {
                    (SymbolKind::Static, false)
                } else {
                    (SymbolKind::Const, false)
                };
                let record = self.symbol_record(
                    kind,
                    &declaration.name,
                    &qualified,
                    &range,
                    signature,
                    RecordFlags {
                        is_async,
                        exported: declaration.exported,
                    },
                );
                self.symbols.push(record.clone());
                if block_end > index {
                    // A block-bodied binding is a call scope for its interior
                    // lines (arrow bodies and multi-line object literals).
                    scopes.push(Scope {
                        record_id: record.record_id,
                        qualified_name: qualified,
                        kind: ScopeKind::Function,
                        body_depth: depth_before + 1,
                        end_line: block_end,
                    });
                }
            }
        }
        Ok(())
    }

    fn handle_method(
        &mut self,
        index: usize,
        depth_before: isize,
        name: &str,
        is_async: bool,
        class_qualified: &str,
        scopes: &mut Vec<Scope>,
    ) -> Result<(), CodeIntelError> {
        let block_end = self.compute_block_end(index, depth_before);
        let range = SourceRange::new(index + 1, block_end + 1)?;
        let qualified = format!("{class_qualified}::{name}");
        let signature = self.gather_signature(index);
        let record = self.symbol_record(
            SymbolKind::Method,
            name,
            &qualified,
            &range,
            Some(signature),
            RecordFlags {
                is_async,
                exported: false,
            },
        );
        self.symbols.push(record.clone());
        scopes.push(Scope {
            record_id: record.record_id,
            qualified_name: qualified,
            kind: ScopeKind::Function,
            body_depth: depth_before + 1,
            end_line: block_end,
        });
        Ok(())
    }

    pub(crate) fn symbol_record(
        &self,
        kind: SymbolKind,
        name: &str,
        qualified: &str,
        range: &SourceRange,
        signature: Option<String>,
        flags: RecordFlags,
    ) -> SymbolRecord {
        SymbolRecord {
            record_id: record_id(qualified, kind.clone(), range, self.context),
            package: self.context.package.to_string(),
            target: self.context.target.to_string(),
            kind,
            name: name.to_string(),
            qualified_name: qualified.to_string(),
            visibility: Visibility::Public,
            is_public_api: flags.exported,
            is_async: flags.is_async,
            is_unsafe: false,
            is_test: self.context.is_test_target,
            is_bench: self.context.is_bench_target,
            signature,
            imports: Vec::new(),
            doc_comment: None,
            markers: Default::default(),
            provenance: provenance(self.context, range.clone()),
        }
    }

    /// End line of the block opened by the declaration at `index`: the first
    /// line (starting at `index`) where the brace depth returns to
    /// `start_depth`; balanced one-liners end on their own line.
    fn compute_block_end(&self, index: usize, start_depth: isize) -> usize {
        let mut depth = start_depth;
        let mut end = index;
        for j in index..self.masked.len() {
            depth += brace_delta(&self.masked[j]);
            if depth <= start_depth {
                return j;
            }
            end = j;
        }
        end
    }

    /// The declaration header (declaration line plus continuation lines while
    /// parameter parentheses are unbalanced), stripped of its body and
    /// collapsed to single spaces.
    fn gather_signature(&self, index: usize) -> String {
        let mut text = String::new();
        let mut depth = 0_isize;
        for line_index in index..self.masked.len() {
            let line = self.masked[line_index].trim();
            text.push_str(line);
            depth += paren_balance(line);
            if depth <= 0 {
                break;
            }
            text.push(' ');
        }
        let cut = cut_signature(&text);
        cut.split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .trim()
            .to_string()
    }
}
