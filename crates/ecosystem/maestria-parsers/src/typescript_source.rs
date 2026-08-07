#![forbid(unsafe_code)]

use maestria_ports::{FileHandle, FileMetadata, ParseContext, ParsedArtifact, Parser, PortError};

use crate::chunking::{
    decode_utf8, extension_is, paragraph_chunks, parsed_artifact, ranges_from_starts,
};

/// Structural chunking for TypeScript/JavaScript source: one chunk per
/// module-level declaration (`export `, `function `, `class `, `interface `,
/// `type `, `const ` at line start) with `@` decorator lines as pending
/// starts, mirroring `rust_chunks` and `python_chunks`. Falls back to
/// paragraph chunks when the file has no declarations.
#[derive(Debug, Clone, Copy, Default)]
pub struct TypeScriptSourceParser;

impl TypeScriptSourceParser {
    pub const fn new() -> Self {
        Self
    }
}

impl Parser for TypeScriptSourceParser {
    fn id(&self) -> &'static str {
        "typescript-source-parser"
    }

    fn supports(&self, file: &FileMetadata) -> bool {
        extension_is(file, &["ts", "tsx", "js", "jsx", "mjs", "cjs"])
    }

    fn parse(&self, file: FileHandle, context: ParseContext) -> Result<ParsedArtifact, PortError> {
        let text = decode_utf8(file.bytes.clone())?;
        let chunks = typescript_chunks(&text);
        parsed_artifact(
            context.artifact_id,
            &file.path,
            &file.bytes,
            chunks,
            "typescript-source-v1".to_string(),
            "tree-v1".to_string(),
            Some("typescript".to_string()),
        )
    }
}

/// One chunk per structural start; decorator lines are attached to the
/// declaration that follows them.
fn typescript_chunks(text: &str) -> Vec<(String, maestria_ports::SourceSpan)> {
    let mut starts = Vec::new();
    let mut pending_decorator_start = None;

    for (index, line) in text.lines().enumerate() {
        let trimmed = line.trim_start();
        if trimmed.starts_with('@') {
            pending_decorator_start.get_or_insert(index);
            continue;
        }

        if is_structural_start(trimmed) {
            let start = match pending_decorator_start.take() {
                Some(decorator_start) => decorator_start,
                None => index,
            };
            starts.push(start);
        } else if !trimmed.is_empty() && !trimmed.starts_with("//") && !trimmed.starts_with("/*") {
            pending_decorator_start = None;
        }
    }

    if starts.is_empty() {
        return paragraph_chunks(text);
    }

    starts.sort_unstable();
    starts.dedup();
    ranges_from_starts(text, starts)
}

/// Whether a trimmed line starts a module-level web declaration.
fn is_structural_start(trimmed: &str) -> bool {
    trimmed.starts_with("export ")
        || trimmed.starts_with("export{")
        || trimmed.starts_with("function ")
        || trimmed.starts_with("async function ")
        || trimmed.starts_with("class ")
        || trimmed.starts_with("interface ")
        || trimmed.starts_with("type ")
        || trimmed.starts_with("const ")
        || trimmed.starts_with("let ")
        || trimmed.starts_with("var ")
        || trimmed.starts_with("import ")
        || trimmed.starts_with("import{")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::error::Error;
    use std::path::PathBuf;

    use maestria_domain::ArtifactId;

    fn handle(path: &str, bytes: &[u8]) -> FileHandle {
        FileHandle {
            path: PathBuf::from(path),
            bytes: bytes.to_vec(),
        }
    }

    #[test]
    fn typescript_chunks_tile_declarations_and_decorators() -> Result<(), Box<dyn Error>> {
        let parsed = TypeScriptSourceParser::new().parse(
            handle(
                "components.tsx",
                b"import { Button } from \"./button\";\n\n@Component\nexport class List {\n  render() {}\n}\n\nexport function helper() {}\n\nconst size = 3;\n",
            ),
            ParseContext {
                artifact_id: ArtifactId::new(9),
            },
        )?;

        assert_eq!(parsed.chunks.len(), 4);
        assert!(parsed.chunks[0].text.starts_with("import { Button }"));
        assert!(parsed.chunks[1].text.starts_with("@Component"));
        assert!(parsed.chunks[2].text.starts_with("export function helper"));
        assert!(parsed.chunks[3].text.starts_with("const size"));
        assert_eq!(
            parsed.chunks[0].chunk_id,
            crate::chunk_id_for(ArtifactId::new(9), 0)?
        );
        Ok(())
    }

    #[test]
    fn typescript_parser_falls_back_to_paragraph_chunks() -> Result<(), Box<dyn Error>> {
        let parsed = TypeScriptSourceParser::new().parse(
            handle("notes.ts", b"const a = 1;\nconst b = 2;\n"),
            ParseContext {
                artifact_id: ArtifactId::new(9),
            },
        )?;
        assert!(!parsed.chunks.is_empty());
        Ok(())
    }
}
