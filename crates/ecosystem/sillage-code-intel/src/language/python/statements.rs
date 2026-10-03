//! Python statement parsing helpers: declaration/import recognition and
//! masking-adjacent text utilities used by the extractor.

/// Declaration kinds recognized by the line scanner.
#[derive(Debug, Clone, Copy)]
pub(crate) enum DeclarationKind {
    Class,
    Function { is_async: bool },
}

/// One imported name of an import statement.
#[derive(Debug, Clone)]
pub(crate) struct ImportItem {
    pub(crate) path: String,
    pub(crate) alias: Option<String>,
}

/// A complete import statement (possibly spanning continuation lines).
#[derive(Debug, Clone)]
pub(crate) struct ImportStatement {
    pub(crate) kind: ImportKind,
    pub(crate) items: Vec<ImportItem>,
}

#[derive(Debug, Clone)]
pub(crate) enum ImportKind {
    Module,
    From { module: String },
}

/// The import statement text as written, for the symbol signature.
pub(crate) fn import_signature(kind: &ImportKind, item: &ImportItem) -> String {
    match kind {
        ImportKind::Module => match &item.alias {
            Some(alias) => format!("import {} as {}", item.path, alias),
            None => format!("import {}", item.path),
        },
        ImportKind::From { module } => match &item.alias {
            Some(alias) => format!("from {} import {} as {}", module, item.path, alias),
            None => format!("from {} import {}", module, item.path),
        },
    }
}

/// Parse a `class`/`def`/`async def` declaration prefix.
pub(crate) fn parse_declaration(trimmed: &str) -> Option<(DeclarationKind, String)> {
    let mut parts = trimmed.split_whitespace();
    match parts.next() {
        Some("class") => parts
            .next()
            .and_then(identifier_prefix)
            .map(|name| (DeclarationKind::Class, name)),
        Some("def") => parts
            .next()
            .and_then(identifier_prefix)
            .map(|name| (DeclarationKind::Function { is_async: false }, name)),
        Some("async") if parts.next() == Some("def") => parts
            .next()
            .and_then(identifier_prefix)
            .map(|name| (DeclarationKind::Function { is_async: true }, name)),
        _ => None,
    }
}

fn identifier_prefix(raw: &str) -> Option<String> {
    let name: String = raw
        .chars()
        .take_while(|character| character.is_ascii_alphanumeric() || *character == '_')
        .collect();
    let valid = name
        .chars()
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic() || first == '_');
    valid.then_some(name)
}

/// Parse an `import ...` or `from ... import ...` statement.
pub(crate) fn parse_import(text: &str) -> Option<(ImportKind, Vec<ImportItem>)> {
    if let Some(rest) = text.strip_prefix("import ") {
        let items = rest.split(',').filter_map(parse_import_item).collect();
        return Some((ImportKind::Module, items));
    }
    if let Some(rest) = text.strip_prefix("from ") {
        let (module, names) = rest.split_once(" import ")?;
        let names = names.trim();
        let names = match names
            .strip_prefix('(')
            .and_then(|inner| inner.strip_suffix(')'))
        {
            Some(inner) => inner,
            None => names,
        };
        let items = names.split(',').filter_map(parse_import_item).collect();
        return Some((
            ImportKind::From {
                module: module.trim().to_string(),
            },
            items,
        ));
    }
    None
}

fn parse_import_item(item: &str) -> Option<ImportItem> {
    let item = item.trim();
    if item.is_empty() || item == "*" {
        return None;
    }
    let (path, alias) = match item.split_once(" as ") {
        Some((path, alias)) => (path.trim(), Some(alias.trim().to_string())),
        None => (item, None),
    };
    if path.is_empty() {
        return None;
    }
    Some(ImportItem {
        path: path.to_string(),
        alias,
    })
}

pub(crate) fn split_indent(line: &str) -> (usize, &str) {
    let indent = line.bytes().take_while(|byte| *byte == b' ').count();
    (indent, line.trim())
}

pub(crate) fn paren_balance(text: &str) -> isize {
    text.bytes().fold(0_isize, |depth, byte| match byte {
        b'(' => depth + 1,
        b')' => depth - 1,
        _ => depth,
    })
}

pub(crate) fn join_dotted(prefix: &str, name: &str) -> String {
    if prefix.is_empty() {
        name.to_string()
    } else {
        format!("{prefix}.{name}")
    }
}

pub(crate) fn is_python_keyword(name: &str) -> bool {
    matches!(
        name,
        "and"
            | "as"
            | "assert"
            | "async"
            | "await"
            | "break"
            | "case"
            | "class"
            | "continue"
            | "def"
            | "del"
            | "elif"
            | "else"
            | "except"
            | "finally"
            | "for"
            | "from"
            | "global"
            | "if"
            | "import"
            | "in"
            | "is"
            | "lambda"
            | "match"
            | "nonlocal"
            | "not"
            | "or"
            | "pass"
            | "raise"
            | "return"
            | "try"
            | "while"
            | "with"
            | "yield"
    )
}
