//! Import statement parsing and `Import`-record emission for
//! TypeScript/JavaScript sources.
//!
//! `parse_import` recovers import structure from masked text (local binding
//! names survive masking) with the original line text consulted only for
//! quoted module specifiers, which the masker blanks out. Masking preserves
//! byte length, so offsets into the masked statement are valid offsets into
//! the original. Relative specifiers (`./` or `../`) resolve to a target
//! module path by normalizing the specifier against the importing file's
//! directory; bare package specifiers are not resolvable offline and yield
//! no candidate. Extension probing is intentionally absent: the specifier's
//! own stem is the target module path.

use crate::language::typescript::extract::{FileExtractor, RecordFlags};
use crate::language::typescript::statements::{brace_delta, join_qualified, paren_balance};
use crate::language::typescript::tokens::TS_SOURCE_EXTENSIONS;
use crate::symbols::RelationCandidate;
use crate::types::SourceRange;
use crate::{CodeIntelError, SymbolKind};
use std::path::{Component, Path, PathBuf};

/// One import/export binding with the module specifier it came from.
#[derive(Debug, Clone)]
pub(crate) struct ImportItem {
    pub(crate) local: String,
    pub(crate) module: String,
}

/// Kinds of import statements the extractor turns into `Import` symbols.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ImportKind {
    Default,
    Named,
    Namespace,
    /// `export { a } from "m"` re-export.
    ReExport,
    /// `import "m"` side-effect import: no symbol is emitted.
    SideEffect,
}

/// A complete (possibly multi-line) import statement.
#[derive(Debug, Clone)]
pub(crate) struct ImportStatement {
    pub(crate) kind: ImportKind,
    pub(crate) items: Vec<ImportItem>,
}

/// Parse an import statement from masked text (`masked`) with the original
/// line text (`original`) consulted only to recover quoted module
/// specifiers, which the masker blanks out. Masking preserves byte length,
/// so offsets into `masked` are valid offsets into `original`.
pub(crate) fn parse_import(masked: &str, original: &str) -> Option<ImportStatement> {
    let (rest, is_reexport) = masked
        .strip_prefix("import ")
        .map(|rest| (rest, false))
        .or_else(|| masked.strip_prefix("export ").map(|rest| (rest, true)))?;
    let trimmed_rest = rest.trim_start();
    let leading = rest.len() - trimmed_rest.len();
    if trimmed_rest.starts_with('"') || trimmed_rest.starts_with('\'') {
        // `import "m"` side-effect import.
        let module = quoted_specifier(original, masked.len() - rest.len() + leading)?.to_string();
        return Some(ImportStatement {
            kind: ImportKind::SideEffect,
            items: vec![ImportItem {
                local: String::new(),
                module,
            }],
        });
    }
    if trimmed_rest.starts_with('{') {
        let (names, module) = parse_braced_names(masked, rest, original)?;
        return Some(ImportStatement {
            kind: if is_reexport {
                ImportKind::ReExport
            } else {
                ImportKind::Named
            },
            items: names
                .into_iter()
                .map(|local| ImportItem {
                    local,
                    module: module.clone(),
                })
                .collect(),
        });
    }
    if let Some(after_star) = trimmed_rest.strip_prefix("* as ") {
        let local = identifier_after(after_star)?;
        let module = specifier_after_from(masked, rest, original)?;
        return Some(ImportStatement {
            kind: if is_reexport {
                ImportKind::ReExport
            } else {
                ImportKind::Namespace
            },
            items: vec![ImportItem { local, module }],
        });
    }
    if let Some(after_type) = trimmed_rest.strip_prefix("type ") {
        // `import type { X } from "m"` / `import type X from "m"`.
        let type_rest = after_type.trim_start();
        if type_rest.starts_with('{') {
            let (names, module) = parse_braced_names(masked, after_type, original)?;
            return Some(ImportStatement {
                kind: ImportKind::Named,
                items: names
                    .into_iter()
                    .map(|local| ImportItem {
                        local,
                        module: module.clone(),
                    })
                    .collect(),
            });
        }
        let local = identifier_after(type_rest)?;
        let module = specifier_after_from(masked, after_type, original)?;
        return Some(ImportStatement {
            kind: ImportKind::Default,
            items: vec![ImportItem { local, module }],
        });
    }
    // Default import: `import X from "m"`.
    let local = identifier_after(trimmed_rest)?;
    let module = specifier_after_from(masked, rest, original)?;
    Some(ImportStatement {
        kind: ImportKind::Default,
        items: vec![ImportItem { local, module }],
    })
}

/// The first identifier-like token after an import prefix.
fn identifier_after(text: &str) -> Option<String> {
    let name: String = text
        .chars()
        .take_while(|character| character.is_ascii_alphanumeric() || matches!(character, '_' | '$'))
        .collect();
    let valid = name
        .chars()
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic() || matches!(first, '_' | '$'));
    valid.then_some(name)
}

/// Parse `{ a, b as c }` names and the `from "m"` specifier that follows.
fn parse_braced_names(masked: &str, rest: &str, original: &str) -> Option<(Vec<String>, String)> {
    let (inner, after) = split_braced(rest)?;
    let mut names = Vec::new();
    for entry in inner.split(',') {
        let entry = entry.trim();
        if entry.is_empty() || entry == "*" {
            continue;
        }
        let local = match entry.split_once(" as ") {
            Some((_exported, alias)) => identifier_after(alias.trim())?,
            None => identifier_after(entry)?,
        };
        names.push(local);
    }
    // `after` is a suffix of the full statement; masking preserves length, so
    // its offset within the original is the same as within the masked text.
    let offset = masked.len() - after.len();
    let module = quoted_specifier(original, offset).map_or_else(String::new, str::to_string);
    Some((names, module))
}

/// Split `{ ... }` returning the inner text and the remainder after `}`.
fn split_braced(text: &str) -> Option<(&str, &str)> {
    let mut depth = 0_isize;
    for (index, character) in text.char_indices() {
        match character {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some((&text[1..index], &text[index + 1..]));
                }
            }
            _ => {}
        }
    }
    None
}

/// The module specifier after `from` in a joined statement: the first quoted
/// string in the original text, at the ` from ` position (offsets relative
/// to the full statement, which masking keeps length-equal).
fn specifier_after_from(masked: &str, rest: &str, original: &str) -> Option<String> {
    let offset = masked.len() - rest.len() + rest.find(" from ")? + " from ".len();
    quoted_specifier(original, offset).map(str::to_string)
}

/// The first quoted string in `text` at or after `offset`.
fn quoted_specifier(text: &str, offset: usize) -> Option<&str> {
    let rest = text.get(offset..)?;
    let mut chars = rest.char_indices();
    let start = loop {
        let (index, character) = chars.next()?;
        if character == '"' || character == '\'' {
            break index;
        }
    };
    let delimiter = rest[start..].chars().next()?;
    let body = &rest[start + delimiter.len_utf8()..];
    let end = body.find(delimiter)?;
    Some(&body[..end])
}

/// The import statement text as written, for the symbol signature.
fn import_signature(kind: ImportKind, item: &ImportItem) -> String {
    match kind {
        ImportKind::Default => format!("import {} from \"{}\"", item.local, item.module),
        ImportKind::Named => format!("import {{ {} }} from \"{}\"", item.local, item.module),
        ImportKind::Namespace => {
            format!("import * as {} from \"{}\"", item.local, item.module)
        }
        ImportKind::ReExport => format!("export {{ {} }} from \"{}\"", item.local, item.module),
        ImportKind::SideEffect => format!("import \"{}\"", item.module),
    }
}

/// The module path a relative import specifier resolves to, when the
/// specifier is a relative path (`./` or `../`). Bare package specifiers are
/// not resolvable offline and yield no candidate. Extension probing is
/// intentionally absent: the specifier's own stem is the target module path,
/// and a directory whose index file is imported without its name resolves
/// only when the target file is indexed under that stem.
fn resolve_import_target(rel_path: &str, specifier: &str) -> Option<String> {
    if !(specifier.starts_with("./") || specifier.starts_with("../")) {
        return None;
    }
    let dir = Path::new(rel_path)
        .parent()
        .map_or(Path::new(""), |parent| parent);
    let joined = dir.join(specifier);
    let normalized = normalize_relative(&joined)?;
    let mut target = normalized;
    if target
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| TS_SOURCE_EXTENSIONS.contains(&extension))
    {
        target = target.with_extension("");
    }
    Some(target.to_string_lossy().into_owned())
}

/// Resolve `.`/`..` components of a repository-relative path.
fn normalize_relative(path: &Path) -> Option<PathBuf> {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    return None;
                }
            }
            Component::Normal(part) => out.push(part),
            _ => return None,
        }
    }
    Some(out)
}

impl<'a> FileExtractor<'a> {
    /// A complete import statement starting at `index`, consuming
    /// continuation lines while braces or parentheses are unbalanced.
    pub(crate) fn collect_import_statement(&self, index: usize) -> Option<ImportStatement> {
        let mut masked = self.masked[index].clone();
        let mut original = self.original[index].clone();
        let mut balance = paren_balance(&masked) + brace_delta(&masked);
        let mut end = index;
        while balance > 0 && end + 1 < self.masked.len() {
            end += 1;
            balance += paren_balance(&self.masked[end]) + brace_delta(&self.masked[end]);
            masked.push(' ');
            masked.push_str(&self.masked[end]);
            original.push(' ');
            original.push_str(&self.original[end]);
        }
        parse_import(&masked, &original)
    }

    pub(crate) fn emit_imports(
        &mut self,
        statement: &ImportStatement,
        index: usize,
    ) -> Result<(), CodeIntelError> {
        if statement.kind == ImportKind::SideEffect {
            return Ok(());
        }
        let end = self.import_statement_end(index);
        let range = SourceRange::new(index + 1, end + 1)?;
        for item in &statement.items {
            if item.module.is_empty() {
                // Plain `export { a }` lists re-export local declarations;
                // they add no import record.
                continue;
            }
            let qualified = join_qualified(&self.module_path, &item.local);
            let record = self.symbol_record(
                SymbolKind::Import,
                &item.local,
                &qualified,
                &range,
                Some(import_signature(statement.kind, item)),
                RecordFlags {
                    is_async: false,
                    exported: statement.kind == ImportKind::ReExport,
                },
            );
            let mut record = record;
            record.imports = vec![item.module.clone()];
            self.symbols.push(record.clone());
            if let Some(target) = resolve_import_target(self.rel_path, &item.module) {
                // The module candidate always resolves when the target file
                // is indexed; the `::local` candidate resolves when the
                // target module declares the imported name.
                self.candidates.push(RelationCandidate::Imports {
                    source_record_id: record.record_id.clone(),
                    target_qualified: target.clone(),
                });
                self.candidates.push(RelationCandidate::Imports {
                    source_record_id: record.record_id.clone(),
                    target_qualified: format!("{target}::{}", item.local),
                });
            }
        }
        Ok(())
    }

    pub(crate) fn import_statement_end(&self, index: usize) -> usize {
        let mut end = index;
        let mut balance = paren_balance(&self.masked[index]) + brace_delta(&self.masked[index]);
        while balance > 0 && end + 1 < self.masked.len() {
            end += 1;
            balance += paren_balance(&self.masked[end]) + brace_delta(&self.masked[end]);
        }
        end
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn import_parsing_recovers_specifiers_from_original_text()
    -> Result<(), Box<dyn std::error::Error>> {
        let statement = parse_import(
            "import { a, b as c } from          ;",
            "import { a, b as c } from \"./items\";",
        )
        .ok_or("expected parse")?;
        assert_eq!(statement.kind, ImportKind::Named);
        assert_eq!(statement.items.len(), 2);
        assert_eq!(statement.items[0].local, "a");
        assert_eq!(statement.items[0].module, "./items");
        assert_eq!(statement.items[1].local, "c");

        let statement = parse_import(
            "import x from        ",
            "import x from \"../utils/helper\";",
        )
        .ok_or("expected parse")?;
        assert_eq!(statement.kind, ImportKind::Default);
        assert_eq!(statement.items[0].local, "x");
        assert_eq!(statement.items[0].module, "../utils/helper");

        let statement = parse_import("import * as ns from ", "import * as ns from \"react\";")
            .ok_or("expected parse")?;
        assert_eq!(statement.kind, ImportKind::Namespace);
        assert_eq!(statement.items[0].local, "ns");
        assert_eq!(statement.items[0].module, "react");

        let statement =
            parse_import("import \"m\";", "import \"./polyfill\";").ok_or("expected parse")?;
        assert_eq!(statement.kind, ImportKind::SideEffect);

        let statement = parse_import("export { a } from        ", "export { a } from \"./mod\";")
            .ok_or("expected parse")?;
        assert_eq!(statement.kind, ImportKind::ReExport);

        assert!(parse_import("const x = 1;", "const x = 1;").is_none());
        Ok(())
    }

    #[test]
    fn relative_specifiers_resolve_to_module_paths() {
        assert_eq!(
            resolve_import_target("src/App.ts", "./components/Button"),
            Some("src/components/Button".to_string())
        );
        assert_eq!(
            resolve_import_target("src/App.ts", "./components/Button.tsx"),
            Some("src/components/Button".to_string())
        );
        assert_eq!(
            resolve_import_target("tests/button.test.ts", "../src/components/Button"),
            Some("src/components/Button".to_string())
        );
        assert_eq!(resolve_import_target("src/App.ts", "react"), None);
        assert_eq!(
            resolve_import_target("src/App.ts", "./Button"),
            Some("src/Button".to_string())
        );
    }
}
