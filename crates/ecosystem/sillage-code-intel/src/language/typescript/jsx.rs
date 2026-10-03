//! Arrow-component detection: the JSX heuristic for `const`/`let` bindings.
//!
//! A module-level arrow binding is a component `Function` when its body
//! contains a JSX marker: after masking, `<` followed by an ASCII letter
//! (fragment opener `<>` included) anywhere between the `=>` and the end of
//! the arrow expression. `</`, `<!`, `<=`, and `<<` are not markers. The
//! heuristic is deterministic and documented; false positives degrade only
//! the kind of a `const` binding (a comparison like `a < b` at arrow depth
//! can be misread as JSX), never record correctness.

use crate::language::typescript::extract::FileExtractor;
use crate::language::typescript::statements::paren_balance;

impl<'a> FileExtractor<'a> {
    /// Whether the `const`/`let` binding at `index` is an arrow whose body
    /// contains a JSX marker.
    pub(crate) fn is_arrow_component(&self, index: usize) -> bool {
        let Some((arrow_line, end)) = self.arrow_extent(index) else {
            return false;
        };
        (arrow_line..=end).any(|j| contains_jsx_marker(&self.masked[j]))
    }

    pub(crate) fn arrow_is_async(&self, index: usize) -> bool {
        let Some((arrow_line, _)) = self.arrow_extent(index) else {
            return false;
        };
        let text = &self.masked[arrow_line];
        let arrow_offset = match text.find("=>") {
            Some(position) => position,
            None => text.len(),
        };
        let before = &text[..arrow_offset];
        before.split_whitespace().any(|token| token == "async")
    }

    /// `(arrow line, end line)` of the arrow expression starting at `index`,
    /// or `None` when the statement has no arrow. The end is the first line
    /// whose parentheses balance and whose text is terminated.
    pub(crate) fn arrow_extent(&self, index: usize) -> Option<(usize, usize)> {
        let mut paren = 0_isize;
        let mut arrow_line = None;
        for j in index..self.masked.len() {
            let text = &self.masked[j];
            if let Some(position) = text.find("=>") {
                paren += paren_balance(&text[..position]);
                arrow_line = Some(j);
                break;
            }
            paren += paren_balance(text);
            if paren <= 0 && j > index {
                return None;
            }
        }
        let arrow_line = arrow_line?;
        let mut depth = paren;
        let mut end = arrow_line;
        for j in arrow_line..self.masked.len() {
            let text = &self.masked[j];
            depth += paren_balance(text);
            if depth <= 0 && is_terminated_line(text) {
                end = j;
                break;
            }
            end = j;
        }
        Some((arrow_line, end))
    }
}

/// Whether masked text carries a JSX element marker: `<` followed by an
/// ASCII letter, or the fragment opener `<>`.
fn contains_jsx_marker(text: &str) -> bool {
    let bytes = text.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'<' {
            match bytes.get(index + 1) {
                Some(b'>') => return true,
                Some(next) if next.is_ascii_alphabetic() => return true,
                _ => {}
            }
        }
        index += 1;
    }
    false
}

/// Whether a masked line ends with a statement terminator.
fn is_terminated_line(text: &str) -> bool {
    let trimmed = text.trim_end();
    trimmed.ends_with(';')
        || trimmed.ends_with('}')
        || trimmed.ends_with(')')
        || trimmed.ends_with('>')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jsx_markers_are_recognized() {
        assert!(contains_jsx_marker("<div>"));
        assert!(contains_jsx_marker("<>"));
        assert!(contains_jsx_marker("return <button>x</button>;"));
        assert!(!contains_jsx_marker("const x = a < b ? c : d;"));
        assert!(!contains_jsx_marker("if (a <= b) {}"));
        assert!(!contains_jsx_marker("x << 1;"));
        assert!(!contains_jsx_marker("<="));
    }

    #[test]
    fn terminated_lines_are_recognized() {
        assert!(is_terminated_line("  </div>;"));
        assert!(is_terminated_line("}"));
        assert!(is_terminated_line(");"));
        assert!(is_terminated_line("  <div>"));
        assert!(!is_terminated_line("const x = 1"));
    }
}
