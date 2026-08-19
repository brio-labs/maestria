use maestria_ports::PortError;

#[derive(Debug, Clone, Copy)]
pub(super) struct PageGeometry {
    pub(super) origin_x: f32,
    pub(super) origin_y: f32,
    pub(super) width: u32,
    pub(super) height: u32,
}

pub(super) fn page_bounds(
    doc: &lopdf::Document,
    page_id: lopdf::ObjectId,
) -> Result<PageGeometry, PortError> {
    let mut current = page_id;
    let mut visited = std::collections::BTreeSet::new();
    let err = |m: String| PortError::InvalidInputContext {
        context: "validate PDF page geometry",
        source: m,
    };
    loop {
        if !visited.insert(current) {
            return Err(err("PDF page parent cycle while resolving MediaBox".into()));
        }
        let dictionary = doc
            .get_dictionary(current)
            .map_err(|e| err(format!("PDF page dictionary unavailable: {e}")))?;
        match dictionary.get(b"MediaBox") {
            Ok(object) => {
                let (_, object) = doc
                    .dereference(object)
                    .map_err(|e| err(format!("PDF MediaBox dereference failed: {e}")))?;
                let values = object
                    .as_array()
                    .map_err(|e| err(format!("PDF MediaBox is not an array: {e}")))?;
                return parse_media_box(values.as_slice());
            }
            Err(lopdf::Error::DictKey(_)) => {}
            Err(error) => {
                return Err(err(format!(
                    "PDF page dictionary get MediaBox failed: {error}"
                )));
            }
        }
        current = match dictionary.get(b"Parent") {
            Ok(lopdf::Object::Reference(parent)) => *parent,
            Ok(_) | Err(lopdf::Error::DictKey(_)) => {
                return Err(err(
                    "PDF page has no MediaBox in its page/parent chain".into()
                ));
            }
            Err(error) => {
                return Err(err(format!(
                    "PDF page dictionary get Parent failed: {error}"
                )));
            }
        };
    }
}

fn parse_media_box(values: &[lopdf::Object]) -> Result<PageGeometry, PortError> {
    if values.len() != 4 {
        return Err(PortError::InvalidInputContext {
            context: "PDF MediaBox element count expected 4",
            source: values.len().to_string(),
        });
    }
    let c = |i: usize, label: &'static str| {
        values[i]
            .as_float()
            .map_err(|e| PortError::invalid_input(label, e.to_string()))
    };
    let left = c(0, "left")?;
    let bottom = c(1, "bottom")?;
    let right = c(2, "right")?;
    let top = c(3, "top")?;
    let width = positive_dimension(right - left).ok_or_else(|| {
        PortError::invalid_input(
            "PDF MediaBox non-positive width",
            (right - left).to_string(),
        )
    })?;
    let height = positive_dimension(top - bottom).ok_or_else(|| {
        PortError::invalid_input(
            "PDF MediaBox non-positive height",
            (top - bottom).to_string(),
        )
    })?;
    Ok(PageGeometry {
        origin_x: left,
        origin_y: bottom,
        width,
        height,
    })
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
pub(super) struct PdfTransform {
    a: f32,
    b: f32,
    c: f32,
    d: f32,
    e: f32,
    f: f32,
}

impl PdfTransform {
    pub(super) const fn identity() -> Self {
        Self {
            a: 1.0,
            b: 0.0,
            c: 0.0,
            d: 1.0,
            e: 0.0,
            f: 0.0,
        }
    }

    pub(super) fn concat(self, matrix: Self) -> Self {
        Self {
            a: self.a * matrix.a + self.c * matrix.b,
            b: self.b * matrix.a + self.d * matrix.b,
            c: self.a * matrix.c + self.c * matrix.d,
            d: self.b * matrix.c + self.d * matrix.d,
            e: self.a * matrix.e + self.c * matrix.f + self.e,
            f: self.b * matrix.e + self.d * matrix.f + self.f,
        }
    }

    pub(super) fn apply(self, x: f32, y: f32) -> (f32, f32) {
        (
            self.a * x + self.c * y + self.e,
            self.b * x + self.d * y + self.f,
        )
    }
}

pub(super) fn as_float_result(object: &lopdf::Object) -> Result<f32, PortError> {
    object
        .as_float()
        .map_err(|e| PortError::invalid_input("PDF operand is not a number", e.to_string()))
}

pub(super) fn transform_from_operands(values: &[lopdf::Object]) -> Result<PdfTransform, PortError> {
    if values.len() != 6 {
        return Err(PortError::InvalidInputContext {
            context: "PDF transform operand count expected 6",
            source: values.len().to_string(),
        });
    }
    let a = as_float_result(&values[0])?;
    let b = as_float_result(&values[1])?;
    let c = as_float_result(&values[2])?;
    let d = as_float_result(&values[3])?;
    let e = as_float_result(&values[4])?;
    let f = as_float_result(&values[5])?;
    if [a, b, c, d, e, f].iter().all(|v| v.is_finite()) {
        Ok(PdfTransform { a, b, c, d, e, f })
    } else {
        Err(PortError::InvalidInputContext {
            context: "PDF transform contains non-finite values",
            source: "transform values must be finite".to_string(),
        })
    }
}

pub(super) fn rectangle(
    values: &[lopdf::Object],
    transform: PdfTransform,
    geometry: PageGeometry,
) -> Result<(u32, u32, u32, u32), PortError> {
    if values.len() != 4 {
        return Err(PortError::InvalidInputContext {
            context: "PDF rectangle operand count expected 4",
            source: values.len().to_string(),
        });
    }
    let x = as_float_result(&values[0])?;
    let y = as_float_result(&values[1])?;
    let width = as_float_result(&values[2])?;
    let height = as_float_result(&values[3])?;
    if ![x, y, width, height].iter().all(|v| v.is_finite()) || width == 0.0 || height == 0.0 {
        return Err(PortError::InvalidInputContext {
            context: "PDF rectangle dimensions are invalid",
            source: "rectangle dimensions must be finite and non-zero".to_string(),
        });
    }
    bounds(
        [
            transform.apply(x, y),
            transform.apply(x + width, y),
            transform.apply(x, y + height),
            transform.apply(x + width, y + height),
        ],
        geometry,
    )
    .ok_or_else(|| {
        PortError::invalid_input(
            "PDF rectangle bounds are invalid",
            "rectangle falls outside page geometry",
        )
    })
}

pub(super) fn unit_region(
    transform: PdfTransform,
    geometry: PageGeometry,
) -> Option<(u32, u32, u32, u32)> {
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
