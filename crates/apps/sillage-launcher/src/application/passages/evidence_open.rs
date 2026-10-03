use serde::Deserialize;

use super::{Passage, PassageLocation, ReopenError, ReopenedPassage, number_range};
pub(super) fn parse_reopened(
    passage: &Passage,
    output: &[u8],
) -> Result<ReopenedPassage, ReopenError> {
    let response: EvidenceEnvelope =
        serde_json::from_slice(output).map_err(|_| ReopenError::Unavailable)?;
    if response.kind != "evidence" || response.data.evidence_id != passage.evidence_id {
        return Err(ReopenError::Changed);
    }
    verify_opened_location(
        &passage.location,
        response.data.source,
        response.data.excerpt,
    )
}

fn verify_opened_location(
    preview: &PassageLocation,
    opened: EvidenceSource,
    excerpt: String,
) -> Result<ReopenedPassage, ReopenError> {
    match opened {
        EvidenceSource::File {
            path,
            start_line,
            end_line,
            content_hash,
        } => verify_file(preview, path, start_line, end_line, content_hash, excerpt),
        EvidenceSource::DocxParagraph {
            path,
            start_paragraph,
            end_paragraph,
            content_hash,
        } => verify_docx(
            preview,
            path,
            start_paragraph,
            end_paragraph,
            content_hash,
            excerpt,
        ),
        EvidenceSource::Pdf {
            snapshot_id,
            page_start,
            page_end,
            path,
        } => verify_pdf(preview, snapshot_id, page_start, page_end, path, excerpt),
        EvidenceSource::PdfRegion {
            snapshot_id,
            page,
            x,
            y,
            width,
            height,
            path,
        } => match preview {
            PassageLocation::PdfRegion {
                snapshot_id: preview_snapshot,
                page: preview_page,
                x: preview_x,
                y: preview_y,
                width: preview_width,
                height: preview_height,
            } if preview_snapshot == &snapshot_id
                && preview_page == &page
                && preview_x == &x
                && preview_y == &y
                && preview_width == &width
                && preview_height == &height =>
            {
                let citation_path = path.as_deref().map_or_else(
                    || format!("PDF snapshot {snapshot_id}"),
                    |path| path.to_owned(),
                );
                Ok(ReopenedPassage {
                    excerpt,
                    citation: format!(
                        "{} · page {page} · region {x},{y} {width}×{height}",
                        citation_path
                    ),
                    path: path.map(std::path::PathBuf::from),
                    pdf_page: Some(page),
                    fallback: format!(
                        "Default PDF viewer may not jump to page {page} or region; passage shown here."
                    ),
                })
            }
            _ => Err(ReopenError::Changed),
        },
        EvidenceSource::Unsupported => Err(ReopenError::Changed),
    }
}

fn verify_file(
    preview: &PassageLocation,
    path: String,
    start_line: u32,
    end_line: u32,
    content_hash: String,
    excerpt: String,
) -> Result<ReopenedPassage, ReopenError> {
    match preview {
        PassageLocation::File {
            path: preview_path,
            start_line: preview_start,
            end_line: preview_end,
            content_hash: preview_hash,
        } if preview_path == &path
            && preview_start == &start_line
            && preview_end == &end_line
            && preview_hash == &content_hash =>
        {
            Ok(ReopenedPassage {
                excerpt,
                citation: format!("{}:{}", path, number_range(start_line, end_line)),
                path: Some(path.into()),
                pdf_page: None,
                fallback: format!(
                    "Default viewer may not jump to line {start_line}; passage shown here."
                ),
            })
        }
        _ => Err(ReopenError::Changed),
    }
}

fn verify_docx(
    preview: &PassageLocation,
    path: String,
    start_paragraph: u32,
    end_paragraph: u32,
    content_hash: String,
    excerpt: String,
) -> Result<ReopenedPassage, ReopenError> {
    match preview {
        PassageLocation::DocxParagraph {
            path: preview_path,
            start_paragraph: preview_start,
            end_paragraph: preview_end,
            content_hash: preview_hash,
        } if preview_path == &path
            && preview_start == &start_paragraph
            && preview_end == &end_paragraph
            && preview_hash == &content_hash =>
        {
            Ok(ReopenedPassage {
                excerpt,
                citation: format!(
                    "{} · paragraph {}",
                    path,
                    number_range(start_paragraph, end_paragraph)
                ),
                path: Some(path.into()),
                pdf_page: None,
                fallback: format!(
                    "Default viewer may not jump to paragraph {start_paragraph}; passage shown here."
                ),
            })
        }
        _ => Err(ReopenError::Changed),
    }
}

fn verify_pdf(
    preview: &PassageLocation,
    snapshot_id: u64,
    page_start: u32,
    page_end: u32,
    path: Option<String>,
    excerpt: String,
) -> Result<ReopenedPassage, ReopenError> {
    match preview {
        PassageLocation::Pdf {
            snapshot_id: preview_snapshot,
            page_start: preview_start,
            page_end: preview_end,
        } if preview_snapshot == &snapshot_id
            && preview_start == &page_start
            && preview_end == &page_end =>
        {
            let citation_path = path.as_deref().map_or_else(
                || format!("PDF snapshot {snapshot_id}"),
                |path| path.to_owned(),
            );
            Ok(ReopenedPassage {
                excerpt,
                citation: format!(
                    "{} · page {}",
                    citation_path,
                    number_range(page_start, page_end)
                ),
                path: path.map(std::path::PathBuf::from),
                pdf_page: Some(page_start),
                fallback: format!(
                    "Default PDF viewer may not jump to page {page_start}; passage shown here."
                ),
            })
        }
        _ => Err(ReopenError::Changed),
    }
}

#[derive(Deserialize)]
struct EvidenceEnvelope {
    #[serde(rename = "type")]
    kind: String,
    data: EvidenceResponse,
}

#[derive(Deserialize)]
struct EvidenceResponse {
    evidence_id: u64,
    source: EvidenceSource,
    excerpt: String,
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum EvidenceSource {
    File {
        path: String,
        start_line: u32,
        end_line: u32,
        content_hash: String,
    },
    DocxParagraph {
        path: String,
        start_paragraph: u32,
        end_paragraph: u32,
        content_hash: String,
    },
    Pdf {
        snapshot_id: u64,
        page_start: u32,
        page_end: u32,
        #[serde(default)]
        path: Option<String>,
    },
    PdfRegion {
        snapshot_id: u64,
        page: u32,
        x: u32,
        y: u32,
        width: u32,
        height: u32,
        #[serde(default)]
        path: Option<String>,
    },
    #[serde(other)]
    Unsupported,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pdf_region_action_requires_the_reopened_snapshot_and_exact_rectangle()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut passage = Passage {
            evidence_id: 41,
            artifact_version: 3,
            excerpt: "Old preview".to_owned(),
            truncated: false,
            location: PassageLocation::PdfRegion {
                snapshot_id: 12,
                page: 2,
                x: 10,
                y: 20,
                width: 30,
                height: 40,
            },
        };
        let output = br#"{
            "type": "evidence",
            "data": {
                "evidence_id": 41,
                "source": {
                    "type": "pdf_region",
                    "snapshot_id": 12,
                    "page": 2,
                    "x": 10,
                    "y": 20,
                    "width": 30,
                    "height": 40,
                    "path": "/approved/atlas.pdf"
                },
                "excerpt": "Freshly reopened text"
            }
        }"#;
        let opened =
            parse_reopened(&passage, output).map_err(|_| "current PDF region was rejected")?;
        assert_eq!(opened.excerpt, "Freshly reopened text");
        assert_eq!(
            opened.path.as_deref(),
            Some(std::path::Path::new("/approved/atlas.pdf"))
        );
        assert_eq!(opened.pdf_page, Some(2));
        assert!(opened.citation.contains("region 10,20 30×40"));

        passage.location = PassageLocation::PdfRegion {
            snapshot_id: 12,
            page: 2,
            x: 10,
            y: 20,
            width: 30,
            height: 41,
        };
        assert!(matches!(
            parse_reopened(&passage, output),
            Err(ReopenError::Changed)
        ));
        Ok(())
    }
}
