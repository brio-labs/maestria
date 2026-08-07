//! Line-masking and file-shape helpers for Python extraction.
//!
//! A line scanner masks string literals and comments (triple-quoted
//! docstrings included) so declarations inside strings are never matched.
//! No Python execution and no parser dependency: every rule is
//! line/indentation based and deterministic.

use std::path::Path;

/// Whether a repository-relative path names a test file: `test_*.py`,
/// `*_test.py`, or under a `tests/`/`test/` directory.
pub(crate) fn is_test_file(rel_path: &str) -> bool {
    let path = Path::new(rel_path);
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .map_or("", |v| v);
    let name_matches = file_name.starts_with("test_") || file_name.ends_with("_test.py");
    let dir_matches = path
        .components()
        .any(|component| matches!(component.as_os_str().to_str(), Some("tests" | "test")));
    name_matches || dir_matches
}

/// Whether a repository-relative path is a benchmark file (under
/// `benchmarks/`/`bench/`).
pub(crate) fn is_bench_file(rel_path: &str) -> bool {
    Path::new(rel_path)
        .components()
        .any(|component| matches!(component.as_os_str().to_str(), Some("benchmarks" | "bench")))
}

/// Dotted module path of a `.py` file relative to the top-level package
/// root: the outermost ancestor directory holding `__init__.py` whose parent
/// does not, plus the path from there to the file. Files outside any package
/// use their full relative path (dirs joined with `.`).
pub(crate) fn module_path_for_file(root: &Path, rel_path: &str) -> String {
    let path = Path::new(rel_path);
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .map_or("", |v| v);
    let is_init = file_name == "__init__.py";
    let parent = match path.parent() {
        Some(parent) => parent,
        None => Path::new(""),
    };
    let mut ancestors: Vec<&Path> = Vec::new();
    let mut current = parent;
    while !current.as_os_str().is_empty() {
        ancestors.push(current);
        match current.parent() {
            Some(parent_dir) => current = parent_dir,
            None => break,
        }
    }
    let mut package_root: Option<&Path> = None;
    for dir in &ancestors {
        let has_init = root.join(dir).join("__init__.py").is_file();
        let parent_has_init = match dir.parent() {
            Some(parent_dir) if !parent_dir.as_os_str().is_empty() => {
                root.join(parent_dir).join("__init__.py").is_file()
            }
            _ => false,
        };
        if has_init && !parent_has_init {
            package_root = Some(dir);
            break;
        }
    }
    let stem = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .map_or("", |v| v);
    let mut parts: Vec<String> = Vec::new();
    match package_root {
        Some(package_dir) => {
            if let Some(name) = package_dir.file_name().and_then(|name| name.to_str()) {
                parts.push(name.to_string());
            }
            let mut rest: Vec<String> = Vec::new();
            let mut current = parent;
            while current != package_dir && !current.as_os_str().is_empty() {
                if let Some(name) = current.file_name().and_then(|name| name.to_str()) {
                    rest.push(name.to_string());
                }
                match current.parent() {
                    Some(parent_dir) => current = parent_dir,
                    None => break,
                }
            }
            rest.reverse();
            if !is_init {
                rest.push(stem.to_string());
            }
            parts.extend(rest);
        }
        None => {
            let mut dirs: Vec<String> = Vec::new();
            let mut current = parent;
            while !current.as_os_str().is_empty() {
                if let Some(name) = current.file_name().and_then(|name| name.to_str()) {
                    dirs.push(name.to_string());
                }
                match current.parent() {
                    Some(parent_dir) => current = parent_dir,
                    None => break,
                }
            }
            dirs.reverse();
            parts.extend(dirs);
            if !is_init {
                parts.push(stem.to_string());
            }
        }
    }
    parts.join(".")
}

/// Line-masking state machine: replaces string literals and comments with
/// spaces so declarations inside them are never matched.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct StringMasker {
    state: StringState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum StringState {
    #[default]
    None,
    Single(char),
    Triple(&'static str),
}

impl StringMasker {
    pub(crate) fn mask_line(&mut self, line: &str) -> String {
        let bytes = line.as_bytes();
        let mut out = String::with_capacity(line.len());
        let mut index = 0;
        while index < bytes.len() {
            match self.state {
                StringState::None => {
                    let byte = bytes[index];
                    if byte == b'#' {
                        for _ in index..bytes.len() {
                            out.push(' ');
                        }
                        break;
                    }
                    if bytes[index..].starts_with(b"\"\"\"") {
                        self.state = StringState::Triple("\"\"\"");
                        out.push_str("   ");
                        index += 3;
                        continue;
                    }
                    if bytes[index..].starts_with(b"'''") {
                        self.state = StringState::Triple("'''");
                        out.push_str("   ");
                        index += 3;
                        continue;
                    }
                    if byte == b'\'' || byte == b'"' {
                        self.state = StringState::Single(byte as char);
                        out.push(' ');
                        index += 1;
                        continue;
                    }
                    out.push(byte as char);
                    index += 1;
                }
                StringState::Single(delimiter) => {
                    if bytes[index] == delimiter as u8 && !is_escaped(bytes, index) {
                        self.state = StringState::None;
                        out.push(' ');
                        index += 1;
                        continue;
                    }
                    out.push(' ');
                    index += 1;
                }
                StringState::Triple(delimiter) => {
                    let delimiter = delimiter.as_bytes();
                    if bytes[index..].starts_with(delimiter) && !is_escaped(bytes, index) {
                        self.state = StringState::None;
                        for _ in 0..delimiter.len() {
                            out.push(' ');
                        }
                        index += delimiter.len();
                        continue;
                    }
                    out.push(' ');
                    index += 1;
                }
            }
        }
        out
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
