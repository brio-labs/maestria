use maestria_domain::StructureNodeType;
use maestria_ports::PortError;

const DEFAULT_PAGE_WIDTH: u32 = 612;
const DEFAULT_PAGE_HEIGHT: u32 = 792;

#[derive(Debug, Clone, Copy)]
struct PageGeometry {
    origin_x: f32,
    origin_y: f32,
    width: u32,
    height: u32,
}

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
        let page_geometry = page_bounds(doc, page_id)?;
        let text_result = doc.extract_text(&[page]);
        let (mut text, text_failed) = match text_result {
            Ok(text) if usable_text(&text) => (text.trim().to_string(), false),
            Ok(_) => (String::new(), true),
            Err(_) => (String::new(), true),
        };
        let (regions, content_failed) = layout_regions(doc, page_id, page, page_geometry);
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

fn page_bounds(doc: &lopdf::Document, page_id: lopdf::ObjectId) -> Result<PageGeometry, PortError> {
    let mut current = page_id;
    let mut visited = std::collections::BTreeSet::new();
    let err = |m: String| PortError::InvalidInput { message: m };
    loop {
        if !visited.insert(current) {
            return Err(err("PDF page parent cycle while resolving MediaBox".into()));
        }
        let dictionary = doc.get_dictionary(current).map_err(|e| err(format!("PDF page dictionary unavailable: {e}")))?;
        match dictionary.get(b"MediaBox") {
            Ok(object) => {
                let (_, object) = doc.dereference(object).map_err(|e| err(format!("PDF MediaBox dereference failed: {e}")))?;
                let values = object.as_array().map_err(|e| err(format!("PDF MediaBox is not an array: {e}")))?;
                return parse_media_box(values.as_slice());
            }
            Err(lopdf::Error::DictKey(_)) => {}
            Err(error) => return Err(err(format!("PDF page dictionary get MediaBox failed: {error}"))),
        }
        current = match dictionary.get(b"Parent") {
            Ok(lopdf::Object::Reference(parent)) => *parent,
            Ok(_) | Err(lopdf::Error::DictKey(_)) => return Ok(PageGeometry {
                origin_x: 0.0, origin_y: 0.0, width: DEFAULT_PAGE_WIDTH, height: DEFAULT_PAGE_HEIGHT,
            }),
            Err(error) => return Err(err(format!("PDF page dictionary get Parent failed: {error}"))),
        };
    }
}

fn parse_media_box(values: &[lopdf::Object]) -> Result<PageGeometry, PortError> {
    if values.len() != 4 {
        return Err(PortError::InvalidInput { message: format!("PDF MediaBox array has {} elements, expected 4", values.len()) });
    }
    let c = |i: usize, label: &str| values[i].as_float().map_err(|e| PortError::InvalidInput {
        message: format!("PDF MediaBox {label} coordinate not a number: {e}"),
    });
    let left = c(0, "left")?;
    let bottom = c(1, "bottom")?;
    let right = c(2, "right")?;
    let top = c(3, "top")?;
    let width = positive_dimension(right - left).ok_or_else(|| PortError::InvalidInput { message: format!("PDF MediaBox non-positive width: {}", right - left) })?;
    let height = positive_dimension(top - bottom).ok_or_else(|| PortError::InvalidInput { message: format!("PDF MediaBox non-positive height: {}", top - bottom) })?;
    Ok(PageGeometry { origin_x: left, origin_y: bottom, width, height })
}
fn positive_dimension(value: f32) -> Option<u32> {
    if !value.is_finite() || value <= 0.0 {
        return None;
    }
    let rounded = value.round();
    if rounded > u32::MAX as f32 {
        return None;
    }
    Some(rounded as u32)
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct PdfTransform {
    a: f32,
    b: f32,
    c: f32,
    d: f32,
    e: f32,
    f: f32,
}

impl PdfTransform {
    const fn identity() -> Self {
        Self {
            a: 1.0,
            b: 0.0,
            c: 0.0,
            d: 1.0,
            e: 0.0,
            f: 0.0,
        }
    }

    fn concat(self, matrix: Self) -> Self {
        Self {
            a: self.a * matrix.a + self.c * matrix.b,
            b: self.b * matrix.a + self.d * matrix.b,
            c: self.a * matrix.c + self.c * matrix.d,
            d: self.b * matrix.c + self.d * matrix.d,
            e: self.a * matrix.e + self.c * matrix.f + self.e,
            f: self.b * matrix.e + self.d * matrix.f + self.f,
        }
    }

    fn apply(self, x: f32, y: f32) -> (f32, f32) {
        (
            self.a * x + self.c * y + self.e,
            self.b * x + self.d * y + self.f,
        )
    }
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
) -> (Vec<PdfRegion>, bool) {
    let content = match doc.get_and_decode_page_content(page_id) {
        Ok(content) => content,
        Err(_) => return (Vec::new(), true),
    };
    let pending = collect_pending_regions(&content.operations, page, geometry);
    let regions = materialize_regions(pending, page);
    (regions, false)
}

fn collect_pending_regions(
    operations: &[lopdf::content::Operation],
    page: u32,
    geometry: PageGeometry,
) -> Vec<PendingRegion> {
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
                if let Some(matrix) = transform_from_operands(&operation.operands) {
                    transform = transform.concat(matrix);
                }
            }
            "re" => {
                if let Some(bounds) = rectangle(&operation.operands, transform, geometry) {
                    pending.push(PendingRegion::Rectangle(bounds));
                }
            }
            "Do" => {
                if let Some(bounds) = unit_region(transform, geometry) {
                    pending.push(PendingRegion::Figure(bounds));
                }
            }
            "BMC" | "BDC" => {
                if let Some(region) = try_tagged_region(operation, transform, geometry, page) {
                    pending.push(region);
                }
            }
            _ => {}
        }
    }
    pending
}

fn try_tagged_region(
    operation: &lopdf::content::Operation,
    transform: PdfTransform,
    geometry: PageGeometry,
    page: u32,
) -> Option<PendingRegion> {
    let name = match operation.operands.first() {
        Some(operand) => match operand.as_name() {
            Ok(name) => match std::str::from_utf8(name) { Ok(s) => s, Err(_) => return None },
            Err(_) => return None,
        },
        None => return None,
    };
    let node_type = if name.eq_ignore_ascii_case("figure") { Some(StructureNodeType::Figure)
    } else if name.eq_ignore_ascii_case("table") || name.eq_ignore_ascii_case("table-cell") { Some(StructureNodeType::Table)
    } else { None }?;
    if transform == PdfTransform::identity() { return None; }
    let bounds = unit_region(transform, geometry)?;
    Some(PendingRegion::Tagged(node_type, format!("{name} region on page {page}"), bounds))
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

fn as_float_opt(object: &lopdf::Object) -> Option<f32> {
    match object.as_float() {
        Ok(f) => Some(f),
        Err(_) => None,
    }
}

fn transform_from_operands(values: &[lopdf::Object]) -> Option<PdfTransform> {
    if values.len() != 6 { return None; }
    let mut numbers = Vec::with_capacity(6);
    for value in values { numbers.push(as_float_opt(value)?); }
    let [a, b, c, d, e, f] = [numbers[0], numbers[1], numbers[2], numbers[3], numbers[4], numbers[5]];
    if [a, b, c, d, e, f].iter().all(|v| v.is_finite()) {
        Some(PdfTransform { a, b, c, d, e, f })
    } else {
        None
    }
}

fn rectangle(
    values: &[lopdf::Object],
    transform: PdfTransform,
    geometry: PageGeometry,
) -> Option<(u32, u32, u32, u32)> {
    if values.len() != 4 { return None; }
    let x = as_float_opt(values.first()?)?;
    let y = as_float_opt(values.get(1)?)?;
    let width = as_float_opt(values.get(2)?)?;
    let height = as_float_opt(values.get(3)?)?;
    if ![x, y, width, height].iter().all(|v| v.is_finite()) || width == 0.0 || height == 0.0 { return None; }
    bounds([transform.apply(x, y), transform.apply(x + width, y), transform.apply(x, y + height), transform.apply(x + width, y + height)], geometry)
}

fn unit_region(transform: PdfTransform, geometry: PageGeometry) -> Option<(u32, u32, u32, u32)> {
    bounds(
        [
            transform.apply(0.0, 0.0),
            transform.apply(1.0, 0.0),
            transform.apply(0.0, 1.0),
            transform.apply(1.0, 1.0),
        ],
        geometry,
    )
}

fn bounds(points: [(f32, f32); 4], geometry: PageGeometry) -> Option<(u32, u32, u32, u32)> {
    let raw_left = points.iter().map(|(x, _)| *x).fold(f32::INFINITY, f32::min);
    let raw_bottom = points.iter().map(|(_, y)| *y).fold(f32::INFINITY, f32::min);
    let raw_right = points
        .iter()
        .map(|(x, _)| *x)
        .fold(f32::NEG_INFINITY, f32::max);
    let raw_top = points
        .iter()
        .map(|(_, y)| *y)
        .fold(f32::NEG_INFINITY, f32::max);
    let left = (raw_left - geometry.origin_x)
        .max(0.0)
        .min(geometry.width as f32);
    let bottom = (raw_bottom - geometry.origin_y)
        .max(0.0)
        .min(geometry.height as f32);
    let right = (raw_right - geometry.origin_x)
        .max(0.0)
        .min(geometry.width as f32);
    let top = (raw_top - geometry.origin_y)
        .max(0.0)
        .min(geometry.height as f32);
    let width = right - left;
    let height = top - bottom;
    if ![left, bottom, width, height]
        .iter()
        .all(|value| value.is_finite())
        || width <= 0.0
        || height <= 0.0
    {
        return None;
    }
    Some((
        left.round() as u32,
        bottom.round() as u32,
        width.round() as u32,
        height.round() as u32,
    ))
}
