use std::error::Error;
use std::path::PathBuf;

use maestria_domain::ArtifactId;
use maestria_parsers::*;
use maestria_ports::{FileHandle, ParseContext, Parser};

#[test]
fn markdown_golden_snapshot() -> Result<(), Box<dyn Error>> {
    let input =
        b"# Title\n\nIntro paragraph.\n\n## Section 1\nContent here.\n\n### Subsection\nMore content.\n";
    let parsed = MarkdownParser::new().parse(
        FileHandle {
            path: PathBuf::from("guide.md"),
            bytes: input.to_vec(),
        },
        ParseContext {
            artifact_id: ArtifactId::new(1),
        },
    )?;
    insta::assert_debug_snapshot!("markdown_parsed", &parsed);
    Ok(())
}

#[test]
fn plain_text_golden_snapshot() -> Result<(), Box<dyn Error>> {
    let input = b"First paragraph.\nStill first.\n\nSecond paragraph.\n";
    let parsed = PlainTextParser::new().parse(
        FileHandle {
            path: PathBuf::from("notes.txt"),
            bytes: input.to_vec(),
        },
        ParseContext {
            artifact_id: ArtifactId::new(2),
        },
    )?;
    insta::assert_debug_snapshot!("plain_text_parsed", &parsed);
    Ok(())
}

#[test]
fn rust_source_golden_snapshot() -> Result<(), Box<dyn Error>> {
    let input = b"use std::fmt;\n\npub struct Thing;\n\nimpl Thing {\n    pub fn new() -> Self { Self }\n}\n\n#[test]\nfn makes_thing() {}\n";
    let parsed = RustSourceParser::new().parse(
        FileHandle {
            path: PathBuf::from("lib.rs"),
            bytes: input.to_vec(),
        },
        ParseContext {
            artifact_id: ArtifactId::new(3),
        },
    )?;
    insta::assert_debug_snapshot!("rust_source_parsed", &parsed);
    Ok(())
}

#[test]
fn cargo_toml_golden_snapshot() -> Result<(), Box<dyn Error>> {
    let input =
        b"[package]\nname = \"demo\"\nversion = \"0.1.0\"\n\n[dependencies]\nserde = \"1\"\n";
    let parsed = CargoTomlParser::new().parse(
        FileHandle {
            path: PathBuf::from("Cargo.toml"),
            bytes: input.to_vec(),
        },
        ParseContext {
            artifact_id: ArtifactId::new(4),
        },
    )?;
    insta::assert_debug_snapshot!("cargo_toml_parsed", &parsed);
    Ok(())
}
#[test]
fn pdf_golden_snapshot() -> Result<(), Box<dyn Error>> {
    let pdf_bytes = create_minimal_pdf(b"Hello PDF world.")?;
    let parsed = PdfParser::new().parse(
        FileHandle {
            path: PathBuf::from("paper.pdf"),
            bytes: pdf_bytes,
        },
        ParseContext {
            artifact_id: ArtifactId::new(5),
        },
    )?;
    insta::assert_debug_snapshot!("pdf_parsed", &parsed);
    Ok(())
}

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
        Err(maestria_ports::PortError::InvalidInput { .. })
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
        Err(maestria_ports::PortError::InvalidInput { .. })
    ));
}

/// Build a minimal valid PDF containing the given text.
fn create_minimal_pdf(text: &[u8]) -> Result<Vec<u8>, Box<dyn Error>> {
    use lopdf::content::{Content, Operation};
    use lopdf::{Dictionary, Object, Stream};

    let mut doc = lopdf::Document::with_version("1.4");
    let pages_id = doc.new_object_id();
    let page_id = doc.new_object_id();
    let content_id = doc.new_object_id();
    let font_id = doc.new_object_id();

    // Font dictionary
    let mut font_dict = Dictionary::new();
    font_dict.set("Type", Object::Name("Font".into()));
    font_dict.set("Subtype", Object::Name("Type1".into()));
    font_dict.set("BaseFont", Object::Name("Courier".into()));
    doc.objects.insert(font_id, Object::Dictionary(font_dict));

    // Content stream with text
    let content = Content {
        operations: vec![
            Operation::new("BT", vec![]),
            Operation::new("Tf", vec![Object::Name("F1".into()), Object::Integer(12)]),
            Operation::new("Td", vec![Object::Integer(72), Object::Integer(700)]),
            Operation::new(
                "Tj",
                vec![Object::String(text.to_vec(), lopdf::StringFormat::Literal)],
            ),
            Operation::new("ET", vec![]),
        ],
    };
    doc.objects.insert(
        content_id,
        Object::Stream(Stream::new(Dictionary::new(), content.encode()?)),
    );

    // Resources dictionary
    let mut resources = Dictionary::new();
    let mut fonts = Dictionary::new();
    fonts.set("F1", Object::Reference(font_id));
    resources.set("Font", Object::Dictionary(fonts));

    // Page object
    let mut page = Dictionary::new();
    page.set("Type", Object::Name("Page".into()));
    page.set("Parent", Object::Reference(pages_id));
    page.set(
        "MediaBox",
        Object::Array(vec![
            Object::Integer(0),
            Object::Integer(0),
            Object::Integer(612),
            Object::Integer(792),
        ]),
    );
    page.set("Contents", Object::Reference(content_id));
    page.set("Resources", Object::Dictionary(resources));
    doc.objects.insert(page_id, Object::Dictionary(page));

    // Pages object
    let mut pages = Dictionary::new();
    pages.set("Type", Object::Name("Pages".into()));
    pages.set("Kids", Object::Array(vec![Object::Reference(page_id)]));
    pages.set("Count", Object::Integer(1));
    doc.objects.insert(pages_id, Object::Dictionary(pages));

    // Catalog
    let catalog_id = doc.new_object_id();
    let mut catalog = Dictionary::new();
    catalog.set("Type", Object::Name("Catalog".into()));
    catalog.set("Pages", Object::Reference(pages_id));
    doc.objects.insert(catalog_id, Object::Dictionary(catalog));
    doc.trailer.set("Root", Object::Reference(catalog_id));

    let mut output = Vec::new();
    doc.save_to(&mut output)?;
    Ok(output)
}
