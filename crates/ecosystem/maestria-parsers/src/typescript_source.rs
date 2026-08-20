#![forbid(unsafe_code)]

use crate::chunking::{extension_is, structural_chunks};
use crate::text_parser;

fn is_typescript_pending(line: &str) -> bool {
    line.starts_with('@')
}

fn is_typescript_comment(line: &str) -> bool {
    line.starts_with("//") || line.starts_with("/*")
}

fn is_typescript_structural_start(trimmed: &str) -> bool {
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

fn typescript_chunks(text: &str) -> Vec<(String, maestria_ports::SourceSpan)> {
    structural_chunks(
        text,
        is_typescript_pending,
        is_typescript_structural_start,
        is_typescript_comment,
    )
}

text_parser!(
    TypeScriptSourceParser,
    "typescript-source-parser",
    |file: &maestria_ports::FileMetadata| {
        extension_is(file, &crate::chunking::CODE_EXTENSIONS[2..])
    },
    "typescript-source-v1",
    "tree-v1",
    "typescript",
    typescript_chunks
);

#[cfg(test)]
mod tests {
    use super::*;
    use std::error::Error;
    use std::path::PathBuf;

    use maestria_domain::ArtifactId;
    use maestria_ports::Parser;

    fn handle(path: &str, bytes: &[u8]) -> maestria_ports::FileHandle {
        maestria_ports::FileHandle {
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
            maestria_ports::ParseContext {
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
            maestria_ports::ParseContext {
                artifact_id: ArtifactId::new(9),
            },
        )?;
        assert!(!parsed.chunks.is_empty());
        Ok(())
    }
}
