use maestria_domain::StructureNodeType;
use maestria_ports::PortError;

use crate::pdf_geometry::{
    PageGeometry, PdfTransform, page_bounds, rectangle, transform_from_operands, unit_region,
};

#[derive(Debug, Clone)]
pub(super) struct PdfPageLayout {
    pub(super) page: u32,
    pub(super) text: String,
    pub(super) regions: Vec<PdfRegion>,
    pub(super) needs_ocr: bool,
}

#[derive(Debug, Clone)]
pub(super) struct PdfRegion {
    pub(super) node_type: StructureNodeType,
    pub(super) label: String,
    pub(super) x: u32,
    pub(super) y: u32,
    pub(super) width: u32,
    pub(super) height: u32,
}

pub(super) fn extract_page_layouts(doc: &lopdf::Document) -> Result<Vec<PdfPageLayout>, PortError> {
    let mut pages = Vec::new();
    for (page, page_id) in doc.get_pages() {
        let page_geometry = match page_bounds(doc, page_id) {
            Ok(geometry) => geometry,
            // R53: without authoritative page geometry no coordinate evidence
            // can be exact; the page degrades to metadata-only instead of
            // fabricating coordinates against invented dimensions.
            Err(_) => {
                pages.push(PdfPageLayout {
                    page,
                    text: String::new(),
                    regions: Vec::new(),
                    needs_ocr: true,
                });
                continue;
            }
        };
        let text_result = doc.extract_text(&[page]);
        let (mut text, text_failed) = match text_result {
            Ok(text) if usable_text(&text) => (text.trim().to_string(), false),
            Ok(_) => (String::new(), true),
            Err(_) => (String::new(), true),
        };
        let (regions, content_failed) = match layout_regions(doc, page_id, page, page_geometry) {
            Ok(result) => result,
            Err(_) => (Vec::new(), true),
        };
        let image_dominated = regions
            .iter()
            .any(|region| matches!(&region.node_type, StructureNodeType::Figure))
            && text.chars().count() < 64;
        if image_dominated {
            text.clear();
        }
        pages.push(PdfPageLayout {
            page,
            text,
            regions,
            needs_ocr: text_failed || content_failed || image_dominated,
        });
    }
    Ok(pages)
}

fn usable_text(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.is_empty() || trimmed.contains('\u{fffd}') {
        return false;
    }
    let characters: Vec<_> = trimmed.chars().collect();
    let printable = characters
        .iter()
        .filter(|character| !character.is_control())
        .count();
    let alphanumeric = characters
        .iter()
        .filter(|character| character.is_alphanumeric())
        .count();
    printable * 4 >= characters.len() * 3 && alphanumeric >= 2
}

enum PendingRegion {
    Rectangle((u32, u32, u32, u32)),
    Figure((u32, u32, u32, u32)),
    Tagged(StructureNodeType, String, (u32, u32, u32, u32)),
}

fn layout_regions(
    doc: &lopdf::Document,
    page_id: lopdf::ObjectId,
    page: u32,
    geometry: PageGeometry,
) -> Result<(Vec<PdfRegion>, bool), PortError> {
    let content = doc
        .get_and_decode_page_content(page_id)
        .map_err(|e| PortError::invalid_input("PDF page content decode failed", e.to_string()))?;
    let pending = collect_pending_regions(&content.operations, page, geometry)?;
    let regions = materialize_regions(pending, page);
    Ok((regions, false))
}

fn collect_pending_regions(
    operations: &[lopdf::content::Operation],
    page: u32,
    geometry: PageGeometry,
) -> Result<Vec<PendingRegion>, PortError> {
    let mut pending = Vec::new();
    let mut transform = PdfTransform::identity();
    let mut stack = Vec::new();
    for operation in operations {
        match operation.operator.as_str() {
            "q" => stack.push(transform),
            "Q" => {
                transform = match stack.pop() {
                    Some(saved) => saved,
                    None => PdfTransform::identity(),
                };
            }
            "cm" => {
                let matrix = transform_from_operands(&operation.operands)?;
                transform = transform.concat(matrix);
            }
            "re" => {
                let bounds = rectangle(&operation.operands, transform, geometry)?;
                pending.push(PendingRegion::Rectangle(bounds));
            }
            "Do" => {
                // An identity transform carries no XObject placement
                // information; a unit square at the origin would be a
                // fabricated region, not evidence (R53).
                if transform != PdfTransform::identity()
                    && let Some(bounds) = unit_region(transform, geometry)
                {
                    pending.push(PendingRegion::Figure(bounds));
                }
            }
            "BMC" | "BDC" => {
                if let Some(region) = try_tagged_region(operation, transform, geometry, page)? {
                    pending.push(region);
                }
            }
            _ => {}
        }
    }
    Ok(pending)
}

fn try_tagged_region(
    operation: &lopdf::content::Operation,
    transform: PdfTransform,
    geometry: PageGeometry,
    page: u32,
) -> Result<Option<PendingRegion>, PortError> {
    let first_operand = operation.operands.first().ok_or_else(|| {
        PortError::invalid_input(
            "PDF tagged region operand is missing",
            "tagged region operation requires an operand",
        )
    })?;
    let name_bytes = first_operand.as_name().map_err(|e| {
        PortError::invalid_input("PDF tagged region operand is not a name", e.to_string())
    })?;
    let name = std::str::from_utf8(name_bytes).map_err(|e| {
        PortError::invalid_input("PDF tagged region name is not valid UTF-8", e.to_string())
    })?;
    let node_type = if name.eq_ignore_ascii_case("figure") {
        Some(StructureNodeType::Figure)
    } else if name.eq_ignore_ascii_case("table") || name.eq_ignore_ascii_case("table-cell") {
        Some(StructureNodeType::Table)
    } else {
        None
    };
    let Some(node_type) = node_type else {
        return Ok(None);
    };
    if transform == PdfTransform::identity() {
        return Ok(None);
    }
    let Some(bounds) = unit_region(transform, geometry) else {
        return Ok(None);
    };
    Ok(Some(PendingRegion::Tagged(
        node_type,
        format!("{name} region on page {page}"),
        bounds,
    )))
}

fn materialize_regions(pending: Vec<PendingRegion>, page: u32) -> Vec<PdfRegion> {
    let rectangle_count = pending
        .iter()
        .filter(|region| matches!(region, PendingRegion::Rectangle(_)))
        .count();
    pending
        .into_iter()
        .filter_map(|region| match region {
            PendingRegion::Rectangle(bounds) if rectangle_count >= 2 => Some(PdfRegion {
                node_type: StructureNodeType::Table,
                label: format!("Table region on page {page}"),
                x: bounds.0,
                y: bounds.1,
                width: bounds.2,
                height: bounds.3,
            }),
            PendingRegion::Figure(bounds) => Some(PdfRegion {
                node_type: StructureNodeType::Figure,
                label: format!("Figure region on page {page}"),
                x: bounds.0,
                y: bounds.1,
                width: bounds.2,
                height: bounds.3,
            }),
            PendingRegion::Tagged(node_type, label, bounds) => Some(PdfRegion {
                node_type,
                label,
                x: bounds.0,
                y: bounds.1,
                width: bounds.2,
                height: bounds.3,
            }),
            PendingRegion::Rectangle(_) => None,
        })
        .collect()
}
