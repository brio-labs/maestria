//! Deterministic tokenizer-based Python symbol extraction.
//!
//! Masks strings and comments (see `tokens`), then detects
//! `class`/`def`/`async def` declarations, imports, and call expressions
//! from the masked text with pure line/indentation rules.

use crate::language::python::statements::{
    DeclarationKind, ImportKind, ImportStatement, import_signature, is_python_keyword, join_dotted,
    paren_balance, parse_declaration, parse_import, split_indent,
};
use crate::language::python::tokens::StringMasker;
use crate::symbols::RelationCandidate;
use crate::symbols::context::FileContext;
use crate::symbols::relation::RelationCandidate as Candidate;
use crate::symbols::utils::{provenance, record_id};
use crate::types::{SourceRange, SymbolRecord};
use crate::{CodeIntelError, SymbolKind, Visibility};

/// Everything extracted from one Python source file.
pub(crate) struct PythonFileExtraction {
    pub(crate) symbols: Vec<SymbolRecord>,
    pub(crate) candidates: Vec<RelationCandidate>,
}

/// Extract symbols and relation candidates from one `.py` file.
pub(crate) fn extract_python_file(
    source: &str,
    _rel_path: &str,
    module_path: &str,
    context: &FileContext,
) -> Result<PythonFileExtraction, CodeIntelError> {
    let lines: Vec<&str> = source.split('\n').collect();
    let line_count = lines.len().max(1);
    let mut masker = StringMasker::default();
    let masked: Vec<String> = lines.iter().map(|line| masker.mask_line(line)).collect();
    let mut extractor = FileExtractor {
        masked: &masked,
        module_path,
        context,
        line_count,
        symbols: Vec::new(),
        candidates: Vec::new(),
    };
    extractor.run()?;
    Ok(PythonFileExtraction {
        symbols: extractor.symbols,
        candidates: extractor.candidates,
    })
}

/// An open declaration scope (class or function) with its block bounds.
struct Scope {
    qualified: String,
    record_id: String,
    indent: usize,
    end_line: usize,
    is_class: bool,
}

struct FileExtractor<'a> {
    masked: &'a [String],
    module_path: &'a str,
    context: &'a FileContext<'a>,
    line_count: usize,
    symbols: Vec<SymbolRecord>,
    candidates: Vec<RelationCandidate>,
}

impl<'a> FileExtractor<'a> {
    fn run(&mut self) -> Result<(), CodeIntelError> {
        self.emit_module_symbol()?;
        let mut scopes: Vec<Scope> = Vec::new();
        let mut decorator_start: Option<usize> = None;
        let mut decorator_depth: isize = 0;
        for index in 0..self.masked.len() {
            let (indent, trimmed) = split_indent(&self.masked[index]);
            if trimmed.is_empty() {
                continue;
            }
            while scopes.last().is_some_and(|scope| index > scope.end_line) {
                scopes.pop();
            }
            if trimmed.starts_with('@') {
                if decorator_start.is_none() {
                    decorator_start = Some(index);
                }
                decorator_depth = paren_balance(trimmed);
                continue;
            }
            if let Some(start) = decorator_start {
                if decorator_depth > 0 {
                    decorator_depth += paren_balance(trimmed);
                    continue;
                }
                decorator_start = None;
                if let Some((kind, name)) = parse_declaration(trimmed) {
                    self.handle_declaration(index, indent, kind, &name, Some(start), &mut scopes)?;
                    continue;
                }
            } else if let Some((kind, name)) = parse_declaration(trimmed) {
                self.handle_declaration(index, indent, kind, &name, None, &mut scopes)?;
                continue;
            }
            if let Some(statement) = self.collect_import_statement(index) {
                self.emit_imports(&statement, index)?;
                continue;
            }
            if let Some(scope) = scopes.last() {
                self.emit_body_calls(index, &scope.record_id);
            }
        }
        Ok(())
    }

    fn emit_module_symbol(&mut self) -> Result<(), CodeIntelError> {
        let range = SourceRange::new(1, self.line_count)?;
        let record = SymbolRecord {
            record_id: record_id(self.module_path, SymbolKind::Module, &range, self.context),
            package: self.context.package.to_string(),
            target: self.context.target.to_string(),
            kind: SymbolKind::Module,
            name: self.module_path.to_string(),
            qualified_name: self.module_path.to_string(),
            visibility: Visibility::Public,
            is_public_api: true,
            is_async: false,
            is_unsafe: false,
            is_test: self.context.is_test_target,
            is_bench: self.context.is_bench_target,
            signature: Some(self.module_path.to_string()),
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
        indent: usize,
        kind: DeclarationKind,
        name: &str,
        decorator_start: Option<usize>,
        scopes: &mut Vec<Scope>,
    ) -> Result<(), CodeIntelError> {
        while scopes.last().is_some_and(|scope| scope.indent >= indent) {
            scopes.pop();
        }
        let block_end = self.compute_block_end(index, indent);
        let start_line = match decorator_start {
            Some(start) => start,
            None => index,
        };
        let range = SourceRange::new(start_line + 1, block_end + 1)?;
        let enclosing_class = scopes.iter().rev().find(|scope| scope.is_class);
        match kind {
            DeclarationKind::Class => {
                let qualified = join_dotted(
                    enclosing_class.map_or(self.module_path, |scope| scope.qualified.as_str()),
                    name,
                );
                let record = self.symbol_record(
                    SymbolKind::Class,
                    name,
                    &qualified,
                    &range,
                    Some(format!("class {name}")),
                    false,
                );
                self.symbols.push(record.clone());
                scopes.push(Scope {
                    qualified,
                    record_id: record.record_id,
                    indent,
                    end_line: block_end,
                    is_class: true,
                });
            }
            DeclarationKind::Function { is_async } => {
                let nested = scopes.iter().any(|scope| !scope.is_class);
                let is_method = enclosing_class.is_some();
                let qualified = match enclosing_class {
                    Some(class_scope) => join_dotted(class_scope.qualified.as_str(), name),
                    None => join_dotted(self.module_path, name),
                };
                let kind = if is_method {
                    SymbolKind::Method
                } else {
                    SymbolKind::Function
                };
                if !nested {
                    let signature = self.gather_signature(index);
                    let record = self.symbol_record(
                        kind,
                        name,
                        &qualified,
                        &range,
                        Some(signature),
                        is_async,
                    );
                    self.symbols.push(record.clone());
                    scopes.push(Scope {
                        qualified,
                        record_id: record.record_id,
                        indent,
                        end_line: block_end,
                        is_class: false,
                    });
                }
            }
        }
        Ok(())
    }

    fn symbol_record(
        &self,
        kind: SymbolKind,
        name: &str,
        qualified: &str,
        range: &SourceRange,
        signature: Option<String>,
        is_async: bool,
    ) -> SymbolRecord {
        SymbolRecord {
            record_id: record_id(qualified, kind.clone(), range, self.context),
            package: self.context.package.to_string(),
            target: self.context.target.to_string(),
            kind,
            name: name.to_string(),
            qualified_name: qualified.to_string(),
            visibility: Visibility::Public,
            is_public_api: true,
            is_async,
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

    /// End line index of the block starting at `index`: the last line whose
    /// indentation is deeper than the declaration (the line before the next
    /// sibling at the same or shallower level, skipping blank lines).
    fn compute_block_end(&self, index: usize, indent: usize) -> usize {
        let mut end = index;
        for next in index + 1..self.masked.len() {
            let (next_indent, trimmed) = split_indent(&self.masked[next]);
            if trimmed.is_empty() {
                continue;
            }
            if next_indent <= indent {
                break;
            }
            end = next;
        }
        end
    }

    /// The declaration line plus any continuation lines up to the final
    /// colon, joined with spaces and stripped of the trailing colon.
    fn gather_signature(&self, index: usize) -> String {
        let mut text = String::new();
        let mut depth = 0_isize;
        for line_index in index..self.masked.len() {
            let line = self.masked[line_index].trim();
            text.push_str(line);
            depth += paren_balance(line);
            if depth <= 0 && line.ends_with(':') {
                break;
            }
            text.push(' ');
        }
        text.trim_end_matches(':').trim().to_string()
    }

    /// An import statement starting at `index`, consuming continuation lines
    /// while parentheses are unbalanced or the line ends with a backslash.
    fn collect_import_statement(&self, index: usize) -> Option<ImportStatement> {
        let mut text = self.masked[index].trim().to_string();
        let mut depth = paren_balance(&text);
        let mut end = index;
        while (depth > 0 || text.ends_with('\\')) && end + 1 < self.masked.len() {
            end += 1;
            let next = self.masked[end].trim();
            depth += paren_balance(next);
            text.push(' ');
            text.push_str(next);
        }
        parse_import(&text).map(|(kind, items)| ImportStatement { kind, items })
    }

    fn emit_imports(
        &mut self,
        statement: &ImportStatement,
        index: usize,
    ) -> Result<(), CodeIntelError> {
        let end = self.import_statement_end(index);
        let range = SourceRange::new(index + 1, end + 1)?;
        for item in &statement.items {
            let local = match &item.alias {
                Some(alias) => alias.clone(),
                None => item.path.clone(),
            };
            let qualified = join_dotted(self.module_path, &local);
            let mut record = self.symbol_record(
                SymbolKind::Import,
                &local,
                &qualified,
                &range,
                Some(import_signature(&statement.kind, item)),
                false,
            );
            record.imports = vec![item.path.clone()];
            self.symbols.push(record.clone());
            let target = match (&statement.kind, item) {
                (ImportKind::Module, item) => item.path.clone(),
                (ImportKind::From { module }, item) => {
                    if module.starts_with('.') {
                        continue;
                    }
                    join_dotted(module, &item.path)
                }
            };
            self.candidates.push(Candidate::Imports {
                source_record_id: record.record_id,
                target_qualified: target,
            });
        }
        Ok(())
    }

    fn import_statement_end(&self, index: usize) -> usize {
        let mut end = index;
        let mut depth = paren_balance(self.masked[index].trim());
        while (depth > 0 || self.masked[index].trim().ends_with('\\'))
            && end + 1 < self.masked.len()
        {
            end += 1;
            depth += paren_balance(self.masked[end].trim());
        }
        end
    }

    /// Emit call candidates for `name(` expressions on one body line.
    fn emit_body_calls(&mut self, index: usize, source_record_id: &str) {
        let line = &self.masked[index];
        let bytes = line.as_bytes();
        let mut cursor = 0;
        while cursor < bytes.len() {
            if bytes[cursor] != b'(' {
                cursor += 1;
                continue;
            }
            let mut start = cursor;
            while start > 0
                && (bytes[start - 1].is_ascii_alphanumeric()
                    || bytes[start - 1] == b'_'
                    || bytes[start - 1] == b'.')
            {
                start -= 1;
            }
            if start < cursor && (bytes[start].is_ascii_alphabetic() || bytes[start] == b'_') {
                let chain = &line[start..cursor];
                let bare = !chain.contains('.');
                if !bare || !is_python_keyword(chain) {
                    self.candidates.push(Candidate::PythonCall {
                        source_record_id: source_record_id.to_string(),
                        target_hint: chain.to_string(),
                    });
                }
            }
            cursor += 1;
        }
    }
}
