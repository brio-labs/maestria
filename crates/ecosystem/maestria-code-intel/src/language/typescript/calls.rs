//! Call-expression candidate emission for TypeScript/JavaScript bodies.
//!
//! Scans masked body lines for `name(` expressions and emits one
//! `TypeScriptCall` candidate per callee, using the innermost function's
//! record id. The binding name of a nested `const`/`function` declaration is
//! skipped (it is being declared, not called); reserved words never become
//! candidates. Dotted chains (`this.render`) are emitted whole and resolved
//! by the shared short-name machinery.

use crate::language::typescript::extract::FileExtractor;
use crate::language::typescript::statements::is_ts_keyword;
use crate::symbols::RelationCandidate;

impl<'a> FileExtractor<'a> {
    /// Emit call candidates for `name(` expressions on one body line, using
    /// the innermost function's record id.
    pub(crate) fn emit_body_calls(&mut self, index: usize, source_record_id: &str) {
        let line = &self.masked[index];
        let declared = declared_binding_name(line);
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
                    || matches!(bytes[start - 1], b'_' | b'.' | b'$'))
            {
                start -= 1;
            }
            if start < cursor
                && (bytes[start].is_ascii_alphabetic() || matches!(bytes[start], b'_' | b'$'))
            {
                let chain = &line[start..cursor];
                let bare = !chain.contains('.');
                if !bare || (!is_ts_keyword(chain) && declared.as_deref() != Some(chain)) {
                    self.candidates.push(RelationCandidate::TypeScriptCall {
                        source_record_id: source_record_id.to_string(),
                        target_hint: chain.to_string(),
                    });
                }
            }
            cursor += 1;
        }
    }
}

/// Cut a signature at the first body-opening brace or statement terminator at
/// parenthesis depth zero.
pub(crate) fn cut_signature(text: &str) -> &str {
    let mut depth = 0_isize;
    for (index, character) in text.char_indices() {
        match character {
            '(' => depth += 1,
            ')' => depth -= 1,
            '{' if depth == 0 => return &text[..index],
            ';' => return &text[..index],
            _ => {}
        }
    }
    text
}

/// The binding name declared at the start of a line (`const X = ...`), used
/// to skip the declaration's own call-like parenthesis.
fn declared_binding_name(line: &str) -> Option<String> {
    for prefix in [
        "const ",
        "let ",
        "var ",
        "function ",
        "function* ",
        "class ",
    ] {
        if let Some(rest) = line.trim_start().strip_prefix(prefix) {
            let name: String = rest
                .chars()
                .take_while(|character| {
                    character.is_ascii_alphanumeric() || matches!(character, '_' | '$')
                })
                .collect();
            if !name.is_empty() {
                return Some(name);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn declared_binding_names_are_recognized() {
        assert_eq!(
            declared_binding_name("const helper = () => {").as_deref(),
            Some("helper")
        );
        assert_eq!(declared_binding_name("let x = 1;").as_deref(), Some("x"));
        assert_eq!(
            declared_binding_name("function fn() {}").as_deref(),
            Some("fn")
        );
        assert_eq!(declared_binding_name("return helper();"), None);
        assert_eq!(
            declared_binding_name("  const y = 2;").as_deref(),
            Some("y")
        );
    }
}
