use std::path::PathBuf;

use maestria_domain::ArtifactId;
use maestria_parsers::*;
use maestria_ports::{FileHandle, ParseContext, Parser};

#[test]
fn parsers_reject_empty_input() {
    assert!(matches!(
        MarkdownParser::new().parse(
            FileHandle {
                path: PathBuf::from("empty.md"),
                bytes: vec![],
            },
            ParseContext {
                artifact_id: ArtifactId::new(99),
            },
        ),
        Err(maestria_ports::PortError::InvalidInput { .. }
            | maestria_ports::PortError::InvalidInputContext { .. })
    ));
    assert!(matches!(
        PlainTextParser::new().parse(
            FileHandle {
                path: PathBuf::from("empty.txt"),
                bytes: vec![],
            },
            ParseContext {
                artifact_id: ArtifactId::new(99),
            },
        ),
        Err(maestria_ports::PortError::InvalidInput { .. }
            | maestria_ports::PortError::InvalidInputContext { .. })
    ));
    assert!(matches!(
        RustSourceParser::new().parse(
            FileHandle {
                path: PathBuf::from("empty.rs"),
                bytes: vec![],
            },
            ParseContext {
                artifact_id: ArtifactId::new(99),
            },
        ),
        Err(maestria_ports::PortError::InvalidInput { .. }
            | maestria_ports::PortError::InvalidInputContext { .. })
    ));
    assert!(matches!(
        CargoTomlParser::new().parse(
            FileHandle {
                path: PathBuf::from("empty.toml"),
                bytes: vec![],
            },
            ParseContext {
                artifact_id: ArtifactId::new(99),
            },
        ),
        Err(maestria_ports::PortError::InvalidInput { .. }
            | maestria_ports::PortError::InvalidInputContext { .. })
    ));
    assert!(matches!(
        PdfParser::new().parse(
            FileHandle {
                path: PathBuf::from("empty.pdf"),
                bytes: vec![],
            },
            ParseContext {
                artifact_id: ArtifactId::new(99),
            },
        ),
        Err(maestria_ports::PortError::InvalidInput { .. }
            | maestria_ports::PortError::InvalidInputContext { .. })
    ));
}
