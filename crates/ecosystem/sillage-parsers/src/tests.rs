#[cfg(test)]
#[path = "bounded_regressions.rs"]
mod bounded_regressions;

use std::error::Error;
use std::path::PathBuf;

use sillage_domain::ArtifactId;
use sillage_ports::{
    BoundedParseError, FileHandle, FileHandleRef, ParseBudget, ParseBudgetFailure, ParseCheckpoint,
    ParseContext, ParseOutcome, Parser, PortError, RepresentationKind, SourceSpan,
};

use super::*;

fn handle(path: &str, bytes: &[u8]) -> FileHandle {
    FileHandle {
        path: PathBuf::from(path),
        bytes: bytes.to_vec(),
    }
}

fn context(id: u64) -> ParseContext {
    ParseContext {
        artifact_id: ArtifactId::new(id),
    }
}

#[test]
fn markdown_chunks_by_headings_and_creates_summary_card() -> Result<(), Box<dyn Error>> {
    let parsed = MarkdownParser::new().parse(
        handle("guide.md", b"intro\n\n# One\nalpha\n\n## Two\nbeta\n"),
        context(7),
    )?;

    assert_eq!(parsed.chunks.len(), 3);
    assert_eq!(parsed.chunks[0].text, "intro");
    assert_eq!(parsed.chunks[1].text, "# One\nalpha");
    assert_eq!(parsed.chunks[2].text, "## Two\nbeta");
    assert_eq!(
        parsed.chunks[0].chunk_id,
        chunk_id_for(ArtifactId::new(7), 0)?
    );
    assert_eq!(
        parsed.chunks[2].chunk_id,
        chunk_id_for(ArtifactId::new(7), 2)?
    );
    assert_eq!(parsed.cards.len(), 1);
    assert_eq!(
        parsed.cards[0].card.card_id,
        card_id_for(ArtifactId::new(7))
    );
    assert_eq!(parsed.cards[0].card.title, "intro");
    Ok(())
}

#[test]
fn bounded_markdown_preserves_utf8_crlf_and_many_heading_spans() -> Result<(), Box<dyn Error>> {
    let mut source = String::from("intro β\r\n\r\n");
    for _ in 0..128 {
        source.push_str("# H\r\ncafé\r\n");
    }
    let file = FileHandle {
        path: PathBuf::from("bounded.md"),
        bytes: source.into_bytes(),
    };
    let expected = MarkdownParser::new().parse(
        FileHandle {
            path: file.path.clone(),
            bytes: file.bytes.clone(),
        },
        context(31),
    )?;
    let mut continue_parsing = || ParseCheckpoint::Continue;
    let mut budget = ParseBudget::new(250_000, 100_000, &mut continue_parsing);
    let outcome = MarkdownParser::new().parse_outcome_bounded(
        FileHandleRef::new(&file.path, &file.bytes),
        context(31),
        &mut budget,
    )?;
    let actual = match outcome {
        ParseOutcome::Complete(parsed) => parsed,
        ParseOutcome::NeedsOcr { .. } => {
            return Err(Box::new(PortError::internal(
                "bounded Markdown test",
                "Markdown unexpectedly requested OCR",
            )));
        }
    };

    assert_eq!(actual, expected);
    assert_eq!(actual.chunks.len(), 129);
    assert_eq!(
        actual.chunks[0].source_span,
        SourceSpan::TextSpan {
            start_line: 1,
            end_line: 2,
        }
    );
    assert_eq!(actual.chunks[0].text, "intro β");
    assert_eq!(
        actual.chunks[0].representations[0].kind,
        RepresentationKind::Raw
    );
    assert_eq!(
        actual.chunks[0].representations[0].content,
        "intro β\r\n\r\n"
    );
    assert_eq!(
        actual.chunks[1].source_span,
        SourceSpan::TextSpan {
            start_line: 3,
            end_line: 4,
        }
    );
    assert_eq!(actual.chunks[1].text, "# H\r\ncafé");
    assert_eq!(
        actual.chunks[1].representations[0].content,
        "# H\r\ncafé\r\n"
    );
    assert_eq!(
        actual.chunks[128].source_span,
        SourceSpan::TextSpan {
            start_line: 257,
            end_line: 258,
        }
    );
    assert!(budget.work_units_used() > 0);
    assert!(budget.output_bytes_used() > 0);
    Ok(())
}

#[test]
fn bounded_markdown_enforces_work_and_output_limits() -> Result<(), Box<dyn Error>> {
    let file = handle("limits.md", b"# Title\r\nbody\r\n");
    let (work_used, output_used) = {
        let mut continue_parsing = || ParseCheckpoint::Continue;
        let mut budget = ParseBudget::new(u64::MAX, usize::MAX, &mut continue_parsing);
        let outcome = MarkdownParser::new().parse_outcome_bounded(
            FileHandleRef::new(&file.path, &file.bytes),
            context(32),
            &mut budget,
        )?;
        assert!(matches!(outcome, ParseOutcome::Complete(_)));
        (budget.work_units_used(), budget.output_bytes_used())
    };
    assert!(work_used > 0);
    assert!(output_used > 0);

    {
        let mut continue_parsing = || ParseCheckpoint::Continue;
        let mut work_limited = ParseBudget::new(work_used - 1, usize::MAX, &mut continue_parsing);
        let work_result = MarkdownParser::new().parse_outcome_bounded(
            FileHandleRef::new(&file.path, &file.bytes),
            context(32),
            &mut work_limited,
        );
        assert!(matches!(
            work_result,
            Err(BoundedParseError::Budget(
                ParseBudgetFailure::WorkLimitExceeded { .. }
            ))
        ));
    }

    {
        let mut continue_parsing = || ParseCheckpoint::Continue;
        let mut output_limited =
            ParseBudget::new(work_used, output_used - 1, &mut continue_parsing);
        let output_result = MarkdownParser::new().parse_outcome_bounded(
            FileHandleRef::new(&file.path, &file.bytes),
            context(32),
            &mut output_limited,
        );
        assert!(matches!(
            output_result,
            Err(BoundedParseError::Budget(
                ParseBudgetFailure::OutputLimitExceeded { .. }
            ))
        ));
    }
    Ok(())
}

#[test]
fn bounded_markdown_observes_cancellation_and_deadline_checkpoints() {
    let mut bytes = Vec::new();
    for _ in 0..400 {
        bytes.extend_from_slice(b"# H\nx\n");
    }
    let file = FileHandle {
        path: PathBuf::from("cancel.md"),
        bytes,
    };
    let mut calls = 0;
    {
        let mut cancel_at_checkpoint = || {
            calls += 1;
            if calls == 2 {
                ParseCheckpoint::Cancelled
            } else {
                ParseCheckpoint::Continue
            }
        };
        let mut cancel_budget = ParseBudget::new(u64::MAX, usize::MAX, &mut cancel_at_checkpoint);
        let cancelled = MarkdownParser::new().parse_outcome_bounded(
            FileHandleRef::new(&file.path, &file.bytes),
            context(33),
            &mut cancel_budget,
        );
        assert!(matches!(
            cancelled,
            Err(BoundedParseError::Budget(ParseBudgetFailure::Cancelled))
        ));
    }
    assert_eq!(calls, 2);

    let mut calls = 0;
    {
        let mut expire_at_checkpoint = || {
            calls += 1;
            if calls == 2 {
                ParseCheckpoint::DeadlineExceeded
            } else {
                ParseCheckpoint::Continue
            }
        };
        let mut deadline_budget = ParseBudget::new(u64::MAX, usize::MAX, &mut expire_at_checkpoint);
        let deadline = MarkdownParser::new().parse_outcome_bounded(
            FileHandleRef::new(&file.path, &file.bytes),
            context(33),
            &mut deadline_budget,
        );
        assert!(matches!(
            deadline,
            Err(BoundedParseError::Budget(
                ParseBudgetFailure::DeadlineExceeded
            ))
        ));
    }
    assert_eq!(calls, 2);
}

#[test]
fn parser_registry_only_delegates_to_bounded_capabilities() {
    let registry = ParserRegistry::with_defaults();
    let markdown = handle("guide.md", b"# Title\nbody\n");
    {
        let mut continue_parsing = || ParseCheckpoint::Continue;
        let mut budget = ParseBudget::new(100_000, 100_000, &mut continue_parsing);
        let bounded = registry.parse_outcome_bounded(
            FileHandleRef::new(&markdown.path, &markdown.bytes),
            context(34),
            &mut budget,
        );
        assert!(matches!(bounded, Ok(ParseOutcome::Complete(_))));
    }

    let text = handle("notes.txt", b"ordinary text");
    let mut continue_parsing = || ParseCheckpoint::Continue;
    let mut budget = ParseBudget::new(100_000, 100_000, &mut continue_parsing);
    let declined = registry.parse_outcome_bounded(
        FileHandleRef::new(&text.path, &text.bytes),
        context(35),
        &mut budget,
    );
    assert!(matches!(declined, Err(BoundedParseError::Unsupported)));
}

#[test]
fn bounded_markdown_preserves_final_unterminated_line() -> Result<(), Box<dyn Error>> {
    let file = handle("tail.md", "prefix\r\n\r\n# Tail\r\nlast λ".as_bytes());
    let expected = MarkdownParser::new().parse(
        FileHandle {
            path: file.path.clone(),
            bytes: file.bytes.clone(),
        },
        context(36),
    )?;
    let mut continue_parsing = || ParseCheckpoint::Continue;
    let mut budget = ParseBudget::new(100_000, 100_000, &mut continue_parsing);
    let outcome = MarkdownParser::new().parse_outcome_bounded(
        FileHandleRef::new(&file.path, &file.bytes),
        context(36),
        &mut budget,
    )?;
    let actual = match outcome {
        ParseOutcome::Complete(parsed) => parsed,
        ParseOutcome::NeedsOcr { .. } => {
            return Err(Box::new(PortError::internal(
                "bounded Markdown test",
                "Markdown unexpectedly requested OCR",
            )));
        }
    };
    assert_eq!(actual, expected);
    assert_eq!(
        actual.chunks[1].source_span,
        SourceSpan::TextSpan {
            start_line: 3,
            end_line: 4,
        }
    );
    assert_eq!(actual.chunks[1].text, "# Tail\r\nlast λ");
    assert_eq!(
        actual.chunks[1].representations[0].content,
        "# Tail\r\nlast λ"
    );
    Ok(())
}

#[test]
fn bounded_markdown_rejects_long_invalid_utf8_without_panicking() {
    let file = handle("invalid.md", &[0x80; 130]);
    let mut continue_parsing = || ParseCheckpoint::Continue;
    let mut budget = ParseBudget::new(100_000, 100_000, &mut continue_parsing);
    let result = MarkdownParser::new().parse_outcome_bounded(
        FileHandleRef::new(&file.path, &file.bytes),
        context(37),
        &mut budget,
    );
    assert!(matches!(
        result,
        Err(BoundedParseError::Parse(
            PortError::InvalidInputContext { .. }
        ))
    ));
}

#[test]
fn plain_text_chunks_by_paragraph_groups() -> Result<(), Box<dyn Error>> {
    let parsed = PlainTextParser::new().parse(
        handle("notes.txt", b"alpha\ncontinues\n\n beta \n\n\n gamma"),
        context(3),
    )?;

    assert_eq!(parsed.chunks.len(), 3);
    assert_eq!(parsed.chunks[0].text, "alpha\ncontinues");
    assert_eq!(parsed.chunks[1].text, "beta");
    assert_eq!(parsed.chunks[2].text, "gamma");
    Ok(())
}

#[test]
fn rust_source_chunks_by_structural_starts_and_test_attributes() -> Result<(), Box<dyn Error>> {
    let parsed = RustSourceParser::new().parse(
            handle(
                "lib.rs",
                b"use std::fmt;\n\npub struct Thing;\n\nimpl Thing {\n    pub fn new() -> Self { Self }\n}\n\n#[test]\nfn makes_thing() {}\n",
            ),
            context(11),
        )?;

    assert_eq!(parsed.chunks.len(), 5);
    assert_eq!(parsed.chunks[0].text, "use std::fmt;");
    assert_eq!(parsed.chunks[1].text, "pub struct Thing;");
    assert_eq!(parsed.chunks[2].text, "impl Thing {");
    assert!(parsed.chunks[3].text.starts_with("pub fn new"));
    assert!(parsed.chunks[4].text.starts_with("#[test]\nfn makes_thing"));
    Ok(())
}
#[test]
fn cargo_toml_chunks_by_table_sections() -> Result<(), Box<dyn Error>> {
    let parsed = CargoTomlParser::new().parse(
            handle(
                "Cargo.toml",
                b"license = \"MIT\"\n\n[package]\nname = \"demo\"\n\n[dependencies]\nsillage = \"0.1\"\n",
            ),
            context(5),
        )?;

    assert_eq!(parsed.chunks.len(), 3);
    assert_eq!(parsed.chunks[0].text, "license = \"MIT\"");
    assert_eq!(parsed.chunks[1].text, "[package]\nname = \"demo\"");
    assert_eq!(parsed.chunks[2].text, "[dependencies]\nsillage = \"0.1\"");
    Ok(())
}
#[test]
fn registry_rejects_binary_content_by_sniffing() -> Result<(), Box<dyn Error>> {
    let registry = ParserRegistry::with_defaults();

    // Content decides, not the extension: binary bytes are rejected even
    // under a text-looking name, while ASCII text parses under a binary
    // extension.
    let binary = [0x89, 0x50, 0x4e, 0x47, 0x00, 0x0d, 0x0a, 0x1a];
    let res = registry.parse(handle("image.bin", &binary), context(13));
    assert!(matches!(res, Err(PortError::InvalidInputContext { .. })));
    let parsed = registry.parse(handle("image.bin", b"alpha"), context(14))?;
    assert_eq!(parsed.chunks[0].text, "alpha");
    Ok(())
}

#[test]
fn parser_rejects_invalid_utf8() {
    let res = PlainTextParser::new().parse(handle("notes.txt", &[0xff, 0xfe]), context(17));
    assert!(matches!(res, Err(PortError::InvalidInputContext { .. })));
}

#[test]
fn chunk_id_rejects_orders_outside_artifact_stride() {
    let res1 = chunk_id_for(ArtifactId::new(1), crate::chunking::ID_STRIDE as usize);
    assert!(matches!(res1, Err(PortError::InvalidInputContext { .. })));
    let res2 = chunk_id_for(ArtifactId::new(u64::MAX), 0);
    assert!(matches!(res2, Err(PortError::InvalidInputContext { .. })));
}
