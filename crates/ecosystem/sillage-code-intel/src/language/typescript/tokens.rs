//! Line-masking and file-shape helpers for TypeScript/JavaScript extraction.
//!
//! A line scanner masks string literals (single/double quotes and backtick
//! templates, `${...}` interpolation included), comments (`//` and `/* */`),
//! and regex literals so declarations inside them are never matched. Regex
//! literals are recognized by a documented heuristic: a `/` whose preceding
//! non-space character is `=`/`(`/`,`/`:`/`[`/`!`/`&`/`|`, or that follows
//! the keyword `return`, starts a regex literal. A mis-detected regex can
//! only hide declaration detection inside it — it never fabricates records
//! (the false positive degrades recall for declarations written inside
//! misclassified division expressions, never record correctness).

use std::path::Path;

/// Source extensions this backend extracts.
pub(crate) const TS_SOURCE_EXTENSIONS: [&str; 6] = ["ts", "tsx", "js", "jsx", "mjs", "cjs"];

/// Whether a repository-relative path names a web test file: `*.test.ts(x)`
/// or `*.spec.ts(x)`, or under `__tests__/`, `tests/`, `test/`, or `e2e/`.
pub(crate) fn is_test_file(rel_path: &str) -> bool {
    let path = Path::new(rel_path);
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .map_or("", |name| name);
    let lower = file_name.to_ascii_lowercase();
    let name_matches = lower.contains(".test.") || lower.contains(".spec.");
    let dir_matches = path.components().any(|component| {
        matches!(
            component.as_os_str().to_str(),
            Some("__tests__" | "tests" | "test" | "e2e")
        )
    });
    name_matches || dir_matches
}

/// Whether a repository-relative path is a benchmark file (under
/// `benchmarks/`).
pub(crate) fn is_bench_file(rel_path: &str) -> bool {
    Path::new(rel_path)
        .components()
        .any(|component| component.as_os_str().to_str() == Some("benchmarks"))
}

/// Module path of a web file: its repository-relative path with the source
/// extension stripped, separators preserved (e.g. `src/components/Button`
/// for `src/components/Button.tsx`). Deterministic per file.
pub(crate) fn module_path_for_file(rel_path: &str) -> String {
    if let Some((dir, file)) = rel_path.rsplit_once('/') {
        if let Some((base, _ext)) = file.rsplit_once('.') {
            let mut out = String::with_capacity(dir.len() + 1 + base.len());
            out.push_str(dir);
            out.push('/');
            out.push_str(base);
            return out;
        }
    } else if let Some((base, _ext)) = rel_path.rsplit_once('.') {
        return base.to_string();
    }
    rel_path.to_string()
}

/// Multi-line masking state machine for TypeScript/JavaScript source.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct WebMasker {
    state: WebMaskState,
    /// `[`/`]` nesting inside a regex literal (a `/` inside a character class
    /// does not terminate the regex).
    regex_brackets: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum WebMaskState {
    #[default]
    Code,
    BlockComment,
    SingleQuote,
    DoubleQuote,
    Template,
    Regex,
}

impl WebMasker {
    /// Mask one line in place, carrying state (block comments, strings,
    /// templates, regex literals) across lines.
    pub(crate) fn mask_line(&mut self, line: &str) -> String {
        let bytes = line.as_bytes();
        let mut out = String::with_capacity(line.len());
        let mut index = 0;
        while index < bytes.len() {
            match self.state {
                WebMaskState::Code => match self.mask_code(bytes, index, &mut out) {
                    Some(next) => index = next,
                    None => break,
                },
                WebMaskState::BlockComment => {
                    index = self.mask_block_comment(bytes, index, &mut out)
                }
                WebMaskState::SingleQuote => {
                    index = self.mask_quoted(bytes, index, b'\'', &mut out)
                }
                WebMaskState::DoubleQuote => index = self.mask_quoted(bytes, index, b'"', &mut out),
                WebMaskState::Template => index = self.mask_quoted(bytes, index, b'`', &mut out),
                WebMaskState::Regex => index = self.mask_regex(bytes, index, &mut out),
            }
        }
        out
    }

    /// One code-state byte; `None` means the rest of the line is masked
    /// (line comment) and the line is done.
    fn mask_code(&mut self, bytes: &[u8], index: usize, out: &mut String) -> Option<usize> {
        if bytes[index] == b'/' && bytes.get(index + 1) == Some(&b'/') {
            mask_rest(out, bytes, index);
            return None;
        }
        if bytes[index] == b'/' && bytes.get(index + 1) == Some(&b'*') {
            self.state = WebMaskState::BlockComment;
            out.push_str("  ");
            return Some(index + 2);
        }
        if bytes[index] == b'\'' {
            self.state = WebMaskState::SingleQuote;
            out.push(' ');
            return Some(index + 1);
        }
        if bytes[index] == b'"' {
            self.state = WebMaskState::DoubleQuote;
            out.push(' ');
            return Some(index + 1);
        }
        if bytes[index] == b'`' {
            self.state = WebMaskState::Template;
            out.push(' ');
            return Some(index + 1);
        }
        if bytes[index] == b'/' && should_start_regex(out) {
            self.state = WebMaskState::Regex;
            self.regex_brackets = 0;
            out.push(' ');
            return Some(index + 1);
        }
        out.push(bytes[index] as char);
        Some(index + 1)
    }

    fn mask_block_comment(&mut self, bytes: &[u8], index: usize, out: &mut String) -> usize {
        if bytes[index..].starts_with(b"*/") {
            self.state = WebMaskState::Code;
            out.push_str("  ");
            index + 2
        } else {
            out.push(' ');
            index + 1
        }
    }

    /// One byte of a single/double-quoted string or backtick template
    /// (`${...}` interpolation stays inside the masked region).
    fn mask_quoted(
        &mut self,
        bytes: &[u8],
        index: usize,
        delimiter: u8,
        out: &mut String,
    ) -> usize {
        if bytes[index] == delimiter && !is_escaped(bytes, index) {
            self.state = WebMaskState::Code;
        }
        out.push(' ');
        index + 1
    }

    /// One byte of a regex literal; the literal ends at the first unescaped
    /// `/` outside a character class.
    fn mask_regex(&mut self, bytes: &[u8], index: usize, out: &mut String) -> usize {
        match bytes[index] {
            b'\\' => {
                out.push(' ');
                if index + 1 < bytes.len() {
                    out.push(' ');
                    index + 2
                } else {
                    index + 1
                }
            }
            b'[' => {
                self.regex_brackets += 1;
                out.push(' ');
                index + 1
            }
            b']' if self.regex_brackets > 0 => {
                self.regex_brackets -= 1;
                out.push(' ');
                index + 1
            }
            b'/' if self.regex_brackets == 0 && !is_escaped(bytes, index) => {
                self.state = WebMaskState::Code;
                out.push(' ');
                index + 1
            }
            _ => {
                out.push(' ');
                index + 1
            }
        }
    }
}

fn mask_rest(out: &mut String, bytes: &[u8], index: usize) {
    for _ in index..bytes.len() {
        out.push(' ');
    }
}

/// Whether a `/` at the current position starts a regex literal: the last
/// non-space character already emitted is one of `=(,:![&|`, or the emitted
/// text ends with the keyword `return`.
fn should_start_regex(out: &str) -> bool {
    let trimmed = out.trim_end();
    match trimmed.chars().next_back() {
        Some(character) => {
            matches!(character, '=' | '(' | ',' | ':' | '[' | '!' | '&' | '|')
                || trimmed.ends_with("return")
        }
        None => false,
    }
}

/// Whether the byte before `index` is an unescaped backslash.
fn is_escaped(bytes: &[u8], index: usize) -> bool {
    let mut backslashes = 0;
    let mut cursor = index;
    while cursor > 0 && bytes[cursor - 1] == b'\\' {
        backslashes += 1;
        cursor -= 1;
    }
    backslashes % 2 == 1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn masker_hides_strings_comments_and_templates() {
        let mut masker = WebMasker::default();
        let masked = masker.mask_line("const s = \"function fake() {\"; // class Real {}");
        assert!(masked.contains("const s"));
        assert!(!masked.contains("fake()"));
        assert!(!masked.contains("Real"));
        let masked = masker.mask_line("const t = `template ${function hidden() {}}`;");
        assert!(!masked.contains("hidden"));
        let masked = masker.mask_line("const x = 'str'; /* comment */ const y = 1;");
        assert!(!masked.contains("comment"));
        assert!(masked.contains("y = 1"));
    }

    #[test]
    fn masker_spans_block_comments_across_lines() {
        let mut masker = WebMasker::default();
        let first = masker.mask_line("/* start");
        assert!(!first.contains("start"));
        let second = masker.mask_line("function hidden() {}");
        assert!(!second.contains("hidden"), "block comment must continue");
        let third = masker.mask_line("end */ function visible() {}");
        assert!(
            third.contains("visible"),
            "code after the closing comment must unmask"
        );
        let fourth = masker.mask_line("function now_visible() {}");
        assert!(fourth.contains("now_visible"));
    }

    #[test]
    fn masker_tracks_templates_across_lines() {
        let mut masker = WebMasker::default();
        let first = masker.mask_line("const t = `line one");
        assert!(!first.contains("line one"));
        let second = masker.mask_line("function hidden() {} and more");
        assert!(!second.contains("hidden"));
        let third = masker.mask_line("closing`; function visible() {}");
        assert!(third.contains("visible"));
    }

    #[test]
    fn masker_masks_regex_literals() {
        let mut masker = WebMasker::default();
        let masked = masker.mask_line("const re = /function hidden() {}/g;");
        assert!(masked.contains("re ="));
        assert!(!masked.contains("hidden"));
        let masked = masker.mask_line("return /class Fake {}/.test(x);");
        assert!(!masked.contains("Fake"));
    }

    #[test]
    fn masker_keeps_division_operators() {
        let mut masker = WebMasker::default();
        let masked = masker.mask_line("const half = total / 2;");
        assert!(masked.contains("total"));
        assert!(masked.contains("/ 2"));
    }

    #[test]
    fn test_and_bench_paths() {
        assert!(is_test_file("src/button.test.ts"));
        assert!(is_test_file("src/button.spec.tsx"));
        assert!(is_test_file("src/__tests__/button.ts"));
        assert!(is_test_file("tests/button.ts"));
        assert!(is_test_file("e2e/flow.ts"));
        assert!(!is_test_file("src/button.ts"));
        assert!(!is_test_file("src/components/contest.ts"));
        assert!(is_bench_file("benchmarks/perf.ts"));
        assert!(!is_bench_file("src/benchmarks_helper.ts"));
    }

    #[test]
    fn module_path_strips_extension() {
        assert_eq!(
            module_path_for_file("src/components/Button.tsx"),
            "src/components/Button"
        );
        assert_eq!(module_path_for_file("index.ts"), "index");
        assert_eq!(module_path_for_file("lib/util.js"), "lib/util");
    }
}
