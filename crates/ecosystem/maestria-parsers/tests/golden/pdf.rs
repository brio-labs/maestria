use std::error::Error;
use std::path::PathBuf;
use std::sync::Arc;

use maestria_domain::ArtifactId;
use maestria_parsers::*;
use maestria_ports::{
    FileHandle, OcrIdentity, OcrPage, OcrProvider, OcrRequest, OcrResponse, ParseContext, Parser,
    ProviderDisclosure, RetentionPolicy,
};

#[test]
fn pdf_golden_snapshot() -> Result<(), Box<dyn Error>> {
    let pdf_bytes = crate::common::create_minimal_pdf(b"Hello PDF world.")?;
    let parsed = PdfParser::new().parse(
        FileHandle {
            path: PathBuf::from("paper.pdf"),
            bytes: pdf_bytes,
        },
        ParseContext {
            artifact_id: ArtifactId::new(5),
        },
    )?;
    crate::assert_debug_snapshot(
        "pdf_parsed",
        &parsed,
        concat!(module_path!(), "::pdf_golden_snapshot"),
        file!(),
        stringify!(&parsed),
        line!(),
    )?;
    Ok(())
}

#[test]
fn pdf_without_extractable_text_is_explicitly_ocr_pending() -> Result<(), Box<dyn Error>> {
    let parsed = PdfParser::new().parse(
        FileHandle {
            path: PathBuf::from("scan.pdf"),
            bytes: create_no_text_pdf()?,
        },
        ParseContext {
            artifact_id: ArtifactId::new(6),
        },
    )?;
    assert_eq!(parsed.status, maestria_ports::ParseStatus::NeedsOcr);
    assert!(parsed.chunks.is_empty());
    assert_eq!(parsed.tree.nodes.len(), 2);
    Ok(())
}

#[test]
fn configured_ocr_provider_replaces_scanned_page_text() -> Result<(), Box<dyn Error>> {
    struct FixtureOcrProvider;

    impl OcrProvider for FixtureOcrProvider {
        fn recognize(&self, request: OcrRequest) -> Result<OcrResponse, maestria_ports::PortError> {
            Ok(OcrResponse {
                pages: request
                    .pages
                    .into_iter()
                    .map(|page| OcrPage {
                        page,
                        text: "OCR recognized text".to_string(),
                    })
                    .collect(),
                identity: self
                    .identity()
                    .ok_or_else(|| maestria_ports::PortError::Internal {
                        message: "fixture OCR identity missing".to_string(),
                    })?,
                disclosure: ProviderDisclosure {
                    remote: false,
                    retention: RetentionPolicy::NoRetention,
                },
            })
        }

        fn identity(&self) -> Option<OcrIdentity> {
            Some(OcrIdentity {
                provider: "fixture".to_string(),
                model: "fixture-ocr".to_string(),
                revision: "test".to_string(),
                artifact_hash:
                    "sha256:0000000000000000000000000000000000000000000000000000000000000000"
                        .to_string(),
                preprocessing_version: "fixture-v1".to_string(),
            })
        }

        fn disclosure(&self) -> Option<ProviderDisclosure> {
            Some(ProviderDisclosure {
                remote: false,
                retention: RetentionPolicy::NoRetention,
            })
        }
    }

    let parsed = PdfParser::with_ocr_provider(Arc::new(FixtureOcrProvider)).parse(
        FileHandle {
            path: PathBuf::from("scan.pdf"),
            bytes: create_no_text_pdf()?,
        },
        ParseContext {
            artifact_id: ArtifactId::new(8),
        },
    )?;
    assert_eq!(parsed.status, maestria_ports::ParseStatus::Parsed);
    assert_eq!(parsed.chunks[0].text, "OCR recognized text");
    assert!(matches!(
        parsed.chunks[0].source_span,
        maestria_ports::SourceSpan::PdfSpan { page: 1 }
    ));
    Ok(())
}

#[test]
fn pdf_layout_regions_preserve_geometry_and_structure() -> Result<(), Box<dyn Error>> {
    let parsed = PdfParser::new().parse(
        FileHandle {
            path: PathBuf::from("layout.pdf"),
            bytes: create_layout_pdf()?,
        },
        ParseContext {
            artifact_id: ArtifactId::new(7),
        },
    )?;
    assert_eq!(parsed.status, maestria_ports::ParseStatus::NeedsOcr);
    let chunk = parsed
        .chunks
        .first()
        .ok_or("layout PDF did not produce a region chunk")?;
    assert!(matches!(
        chunk.source_span,
        maestria_ports::SourceSpan::PdfRegion {
            page: 1,
            x: 72,
            y: 100,
            width: 200,
            height: 100,
        }
    ));
    assert!(chunk.text.is_empty());
    assert!(
        !chunk.representations.iter().any(|representation| {
            representation.kind == maestria_ports::RepresentationKind::Raw
        })
    );
    assert!(parsed.tree.nodes.iter().any(|node| {
        node.id == chunk.node_id
            && node.node_type == maestria_domain::StructureNodeType::Table
            && node.page == Some(1)
    }));
    assert_eq!(
        chunk
            .representations
            .iter()
            .filter(|representation| {
                representation.kind == maestria_ports::RepresentationKind::Visual
            })
            .count(),
        1
    );
    Ok(())
}

// ── PDF helpers ──

use lopdf::{Dictionary, Object, dictionary};

fn create_no_text_pdf() -> Result<Vec<u8>, Box<dyn Error>> {
    let mut doc = lopdf::Document::with_version("1.4");
    let pages_id = doc.new_object_id();
    let page_id = doc.new_object_id();
    let catalog_id = doc.new_object_id();
    doc.objects.insert(
        page_id,
        Object::Dictionary(dictionary! {
            "Type" => "Page",
            "Parent" => pages_id,
            "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
        }),
    );
    doc.objects.insert(
        pages_id,
        Object::Dictionary(dictionary! {
            "Type" => "Pages",
            "Kids" => vec![page_id.into()],
            "Count" => 1,
        }),
    );
    doc.objects.insert(
        catalog_id,
        Object::Dictionary(dictionary! {
            "Type" => "Catalog",
            "Pages" => pages_id,
        }),
    );
    doc.trailer.set("Root", catalog_id);
    let mut output = Vec::new();
    doc.save_to(&mut output)?;
    Ok(output)
}

fn create_layout_pdf() -> Result<Vec<u8>, Box<dyn Error>> {
    let mut doc = lopdf::Document::with_version("1.4");
    let pages_id = doc.new_object_id();
    let page_id = doc.new_object_id();
    let content_id = doc.new_object_id();
    let catalog_id = doc.new_object_id();
    let content = lopdf::content::Content {
        operations: vec![
            lopdf::content::Operation::new(
                "re",
                vec![
                    Object::Integer(72),
                    Object::Integer(100),
                    Object::Integer(200),
                    Object::Integer(100),
                ],
            ),
            lopdf::content::Operation::new(
                "re",
                vec![
                    Object::Integer(300),
                    Object::Integer(100),
                    Object::Integer(200),
                    Object::Integer(100),
                ],
            ),
            lopdf::content::Operation::new("S", vec![]),
        ],
    };
    doc.objects.insert(
        content_id,
        Object::Stream(lopdf::Stream::new(Dictionary::new(), content.encode()?)),
    );
    doc.objects.insert(
        page_id,
        Object::Dictionary(dictionary! {
            "Type" => "Page",
            "Parent" => pages_id,
            "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
            "Contents" => content_id,
        }),
    );
    doc.objects.insert(
        pages_id,
        Object::Dictionary(dictionary! {
            "Type" => "Pages",
            "Kids" => vec![page_id.into()],
            "Count" => 1,
        }),
    );
    doc.objects.insert(
        catalog_id,
        Object::Dictionary(dictionary! {
            "Type" => "Catalog",
            "Pages" => pages_id,
        }),
    );
    doc.trailer.set("Root", catalog_id);
    let mut output = Vec::new();
    doc.save_to(&mut output)?;
    Ok(output)
}
