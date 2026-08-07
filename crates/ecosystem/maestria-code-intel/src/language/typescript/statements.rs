//! TypeScript/JavaScript statement parsing helpers: declaration and import
//! recognition on masked line text, plus brace/paren balance utilities.

/// Kinds of top-level declarations recognized by the line scanner.
#[derive(Debug, Clone, Copy)]
pub(crate) enum DeclarationKind {
    Function {
        is_async: bool,
    },
    Class,
    Interface,
    TypeAlias,
    /// `const`/`let`/`var` binding; the extractor upgrades arrow bindings
    /// whose body carries JSX to a component `Function`.
    Const {
        is_let: bool,
    },
}

/// A parsed declaration: kind, declared name, and export visibility.
#[derive(Debug, Clone)]
pub(crate) struct ParsedDeclaration {
    pub(crate) kind: DeclarationKind,
    pub(crate) name: String,
    pub(crate) exported: bool,
}

/// Parse a top-level declaration prefix from masked text. Handles the
/// `export` / `export default` prefixes; anonymous default functions and
/// classes get the name `"default"` (documented in `extract`).
pub(crate) fn parse_declaration(trimmed: &str) -> Option<ParsedDeclaration> {
    let mut rest = trimmed;
    let mut exported = false;
    if let Some(after) = rest.strip_prefix("export ") {
        exported = true;
        rest = after;
        if let Some(after) = rest.strip_prefix("default ") {
            rest = after;
        }
    }
    if let Some(after) = rest.strip_prefix("async function ") {
        return identifier_after(after).map(|name| ParsedDeclaration {
            kind: DeclarationKind::Function { is_async: true },
            name,
            exported,
        });
    }
    if let Some(after) = rest.strip_prefix("function ") {
        let name = match identifier_after(after) {
            Some(name) => name,
            None => "default".to_string(),
        };
        return Some(ParsedDeclaration {
            kind: DeclarationKind::Function { is_async: false },
            name,
            exported,
        });
    }
    if let Some(after) = rest.strip_prefix("function* ") {
        return identifier_after(after).map(|name| ParsedDeclaration {
            kind: DeclarationKind::Function { is_async: false },
            name,
            exported,
        });
    }
    if let Some(after) = rest.strip_prefix("class ") {
        let name = match identifier_after(after) {
            Some(name) => name,
            None => "default".to_string(),
        };
        return Some(ParsedDeclaration {
            kind: DeclarationKind::Class,
            name,
            exported,
        });
    }
    if let Some(after) = rest.strip_prefix("interface ") {
        return identifier_after(after).map(|name| ParsedDeclaration {
            kind: DeclarationKind::Interface,
            name,
            exported,
        });
    }
    if let Some(after) = rest.strip_prefix("type ") {
        return identifier_after(after).map(|name| ParsedDeclaration {
            kind: DeclarationKind::TypeAlias,
            name,
            exported,
        });
    }
    if let Some(after) = rest.strip_prefix("const ") {
        return identifier_after(after).map(|name| ParsedDeclaration {
            kind: DeclarationKind::Const { is_let: false },
            name,
            exported,
        });
    }
    if let Some(after) = rest.strip_prefix("let ") {
        return identifier_after(after).map(|name| ParsedDeclaration {
            kind: DeclarationKind::Const { is_let: true },
            name,
            exported,
        });
    }
    if let Some(after) = rest.strip_prefix("var ") {
        return identifier_after(after).map(|name| ParsedDeclaration {
            kind: DeclarationKind::Const { is_let: true },
            name,
            exported,
        });
    }
    // Anonymous `export default <arrow>`: name "default".
    if exported && rest.starts_with('(') {
        return Some(ParsedDeclaration {
            kind: DeclarationKind::Function { is_async: false },
            name: "default".to_string(),
            exported,
        });
    }
    None
}

/// The first identifier-like token after a declaration keyword.
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

/// The declared method name of a class-body line plus whether the method is
/// `async`, or `None` when the line is not a method declaration (modifier
/// prefixes are skipped; control-flow keywords and non-call lines are
/// rejected).
pub(crate) fn parse_method(trimmed: &str) -> Option<(String, bool)> {
    let mut text = trimmed;
    let mut is_async = false;
    loop {
        let mut advanced = false;
        for prefix in [
            "async ",
            "get ",
            "set ",
            "static ",
            "public ",
            "private ",
            "protected ",
            "readonly ",
            "abstract ",
            "override ",
        ] {
            if let Some(rest) = text.strip_prefix(prefix) {
                if prefix == "async " {
                    is_async = true;
                }
                text = rest;
                advanced = true;
                break;
            }
        }
        if !advanced {
            break;
        }
    }
    let name: String = text
        .chars()
        .take_while(|character| character.is_ascii_alphanumeric() || matches!(character, '_' | '$'))
        .collect();
    if name.is_empty() || is_ts_keyword(&name) {
        return None;
    }
    let rest = text[name.len()..].trim_start();
    let rest = if rest.starts_with('<') {
        // Generic method: skip the balanced `<...>` before the parameter list.
        skip_angle_group(rest)?.trim_start()
    } else {
        rest
    };
    rest.starts_with('(').then_some((name, is_async))
}

fn skip_angle_group(text: &str) -> Option<&str> {
    let mut depth = 0_isize;
    for (index, character) in text.char_indices() {
        match character {
            '<' => depth += 1,
            '>' => {
                depth -= 1;
                if depth == 0 {
                    return Some(&text[index + 1..]);
                }
            }
            _ => {}
        }
    }
    None
}

/// Brace balance delta of a masked line (`{` minus `}`).
pub(crate) fn brace_delta(text: &str) -> isize {
    text.bytes().fold(0_isize, |depth, byte| match byte {
        b'{' => depth + 1,
        b'}' => depth - 1,
        _ => depth,
    })
}

/// Parenthesis balance of a masked line.
pub(crate) fn paren_balance(text: &str) -> isize {
    text.bytes().fold(0_isize, |depth, byte| match byte {
        b'(' => depth + 1,
        b')' => depth - 1,
        _ => depth,
    })
}

/// Join a module path and a leaf name with the `::` separator used by every
/// backend's qualified names.
pub(crate) fn join_qualified(module_path: &str, name: &str) -> String {
    format!("{module_path}::{name}")
}

/// Whether a name is a reserved word that can never be a declared binding or
/// a callee target worth an edge.
pub(crate) fn is_ts_keyword(name: &str) -> bool {
    matches!(
        name,
        "break"
            | "case"
            | "catch"
            | "class"
            | "const"
            | "continue"
            | "debugger"
            | "default"
            | "delete"
            | "do"
            | "else"
            | "enum"
            | "export"
            | "extends"
            | "finally"
            | "for"
            | "function"
            | "if"
            | "import"
            | "in"
            | "instanceof"
            | "let"
            | "new"
            | "of"
            | "return"
            | "static"
            | "super"
            | "switch"
            | "this"
            | "throw"
            | "try"
            | "typeof"
            | "var"
            | "void"
            | "while"
            | "with"
            | "yield"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn declaration_parsing_covers_export_forms() -> Result<(), Box<dyn std::error::Error>> {
        let parsed = parse_declaration("export function add(a: number): number {")
            .ok_or("expected parse")?;
        assert!(parsed.exported);
        assert!(matches!(
            parsed.kind,
            DeclarationKind::Function { is_async: false }
        ));
        assert_eq!(parsed.name, "add");
        let parsed = parse_declaration("export default class Button {").ok_or("expected parse")?;
        assert!(parsed.exported);
        assert_eq!(parsed.name, "Button");
        let parsed = parse_declaration("export default function () {").ok_or("expected parse")?;
        assert_eq!(parsed.name, "default");
        let parsed = parse_declaration("export default class {").ok_or("expected parse")?;
        assert_eq!(parsed.name, "default");
        let parsed = parse_declaration("interface Named {").ok_or("expected parse")?;
        assert!(!parsed.exported);
        assert!(matches!(parsed.kind, DeclarationKind::Interface));
        let parsed = parse_declaration("export type Options = {").ok_or("expected parse")?;
        assert!(parsed.exported);
        assert!(matches!(parsed.kind, DeclarationKind::TypeAlias));
        let parsed = parse_declaration("const config = {").ok_or("expected parse")?;
        assert!(matches!(
            parsed.kind,
            DeclarationKind::Const { is_let: false }
        ));
        assert_eq!(parsed.name, "config");
        let parsed = parse_declaration("export let count = 0;").ok_or("expected parse")?;
        assert!(matches!(
            parsed.kind,
            DeclarationKind::Const { is_let: true }
        ));
        assert_eq!(parsed.name, "count");
        assert!(parse_declaration("if (x) {").is_none());
        assert!(parse_declaration("return 1;").is_none());
        assert!(parse_declaration("import { a } from \"m\";").is_none());
        Ok(())
    }

    #[test]
    fn method_parsing_rejects_control_flow() {
        assert_eq!(
            parse_method("render() {"),
            Some(("render".to_string(), false))
        );
        assert_eq!(
            parse_method("async load() {"),
            Some(("load".to_string(), true))
        );
        assert_eq!(
            parse_method("private helper(): void {"),
            Some(("helper".to_string(), false))
        );
        assert_eq!(
            parse_method("get value() {"),
            Some(("value".to_string(), false))
        );
        assert_eq!(
            parse_method("static create() {"),
            Some(("create".to_string(), false))
        );
        assert_eq!(
            parse_method("map<T>(fn: (x: T) => T) {"),
            Some(("map".to_string(), false))
        );
        assert_eq!(
            parse_method("constructor() {"),
            Some(("constructor".to_string(), false))
        );
        assert!(parse_method("if (x) {").is_none());
        assert!(parse_method("for (let i = 0; i < 1; i++) {").is_none());
        assert!(parse_method("this.render();").is_none());
        assert!(parse_method("const x = 1;").is_none());
        assert!(parse_method("return 1;").is_none());
    }
}
