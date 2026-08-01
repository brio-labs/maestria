use serde::{Deserialize, Serialize};

use super::SearchCompatibilityError;
use crate::ContentRange;
use crate::ids::StructureNodeId;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum StructureNodeType {
    Document,
    Section,
    Paragraph,
    List,
    ListItem,
    Table,
    Figure,
    TableRow,
    TableCell,
    FigureCaption,
    Formula,
    Code,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StructureNode {
    pub id: StructureNodeId,
    pub parent_id: Option<StructureNodeId>,
    pub sibling_id: Option<StructureNodeId>,
    pub node_type: StructureNodeType,
    pub source_range: ContentRange,
    pub page: Option<u32>,
    pub section_path: Vec<String>,
    pub parser_generation: String,
    pub schema_generation: String,
    pub language: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "WireSourceLocationDto")]
pub enum SourceLocation {
    File {
        path: String,
        start_line: u32,
        end_line: u32,
    },
    Page {
        page_start: u32,
        page_end: u32,
    },
    Region {
        page: u32,
        x: u32,
        y: u32,
        width: u32,
        height: u32,
    },
    Symbol {
        path: String,
        qualified_name: String,
    },
}

impl SourceLocation {
    /// Builds a file line span whose start line does not exceed its end line.
    pub fn file(
        path: String,
        start_line: u32,
        end_line: u32,
    ) -> Result<Self, SearchCompatibilityError> {
        if start_line > end_line {
            return Err(SearchCompatibilityError::InvalidSourceSpan(
                "file start line must not exceed end line",
            ));
        }
        Ok(Self::File {
            path,
            start_line,
            end_line,
        })
    }

    /// Builds a page span whose start page does not exceed its end page.
    pub fn page(page_start: u32, page_end: u32) -> Result<Self, SearchCompatibilityError> {
        if page_start > page_end {
            return Err(SearchCompatibilityError::InvalidSourceSpan(
                "page start must not exceed end page",
            ));
        }
        Ok(Self::Page {
            page_start,
            page_end,
        })
    }

    /// Builds a page region with positive dimensions.
    pub fn region(
        page: u32,
        x: u32,
        y: u32,
        width: u32,
        height: u32,
    ) -> Result<Self, SearchCompatibilityError> {
        if width == 0 || height == 0 {
            return Err(SearchCompatibilityError::InvalidSourceSpan(
                "region width and height must be positive",
            ));
        }
        Ok(Self::Region {
            page,
            x,
            y,
            width,
            height,
        })
    }

    /// Builds a symbol location with a non-empty path and qualified name.
    pub fn symbol(path: String, qualified_name: String) -> Result<Self, SearchCompatibilityError> {
        if path.is_empty() || qualified_name.is_empty() {
            return Err(SearchCompatibilityError::InvalidSourceSpan(
                "symbol path and qualified name must not be empty",
            ));
        }
        Ok(Self::Symbol {
            path,
            qualified_name,
        })
    }
}

#[derive(Deserialize)]
enum WireSourceLocationDto {
    File {
        path: String,
        start_line: u32,
        end_line: u32,
    },
    Page {
        page_start: u32,
        page_end: u32,
    },
    Region {
        page: u32,
        x: u32,
        y: u32,
        width: u32,
        height: u32,
    },
    Symbol {
        path: String,
        qualified_name: String,
    },
}

impl TryFrom<WireSourceLocationDto> for SourceLocation {
    type Error = SearchCompatibilityError;

    fn try_from(dto: WireSourceLocationDto) -> Result<Self, Self::Error> {
        match dto {
            WireSourceLocationDto::File {
                path,
                start_line,
                end_line,
            } => Self::file(path, start_line, end_line),
            WireSourceLocationDto::Page {
                page_start,
                page_end,
            } => Self::page(page_start, page_end),
            WireSourceLocationDto::Region {
                page,
                x,
                y,
                width,
                height,
            } => Self::region(page, x, y, width, height),
            WireSourceLocationDto::Symbol {
                path,
                qualified_name,
            } => Self::symbol(path, qualified_name),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "EvidenceSpanDto")]
pub struct EvidenceSpan {
    node_id: Option<StructureNodeId>,
    location: SourceLocation,
    range: ContentRange,
}

impl EvidenceSpan {
    pub fn new(
        node_id: Option<StructureNodeId>,
        location: SourceLocation,
        range: ContentRange,
    ) -> Result<Self, SearchCompatibilityError> {
        if range.start() > range.end() {
            return Err(SearchCompatibilityError::InvalidSourceSpan(
                "range start must not exceed range end",
            ));
        }
        match &location {
            SourceLocation::File {
                start_line,
                end_line,
                ..
            } if start_line > end_line => {
                return Err(SearchCompatibilityError::InvalidSourceSpan(
                    "file start line must not exceed end line",
                ));
            }
            SourceLocation::Page {
                page_start,
                page_end,
            } if page_start > page_end => {
                return Err(SearchCompatibilityError::InvalidSourceSpan(
                    "page start must not exceed end page",
                ));
            }
            SourceLocation::Region { width, height, .. } if *width == 0 || *height == 0 => {
                return Err(SearchCompatibilityError::InvalidSourceSpan(
                    "region width and height must be positive",
                ));
            }
            SourceLocation::Symbol {
                path,
                qualified_name,
            } if path.is_empty() || qualified_name.is_empty() => {
                return Err(SearchCompatibilityError::InvalidSourceSpan(
                    "symbol path and qualified name must not be empty",
                ));
            }
            _ => {}
        }
        Ok(Self {
            node_id,
            location,
            range,
        })
    }

    pub fn node_id(&self) -> Option<StructureNodeId> {
        self.node_id
    }

    pub fn location(&self) -> &SourceLocation {
        &self.location
    }

    pub fn range(&self) -> ContentRange {
        self.range
    }
}

#[derive(Deserialize)]
struct EvidenceSpanDto {
    node_id: Option<StructureNodeId>,
    location: SourceLocation,
    range: ContentRange,
}

impl TryFrom<EvidenceSpanDto> for EvidenceSpan {
    type Error = SearchCompatibilityError;

    fn try_from(dto: EvidenceSpanDto) -> Result<Self, Self::Error> {
        Self::new(dto.node_id, dto.location, dto.range)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_location_constructors_enforce_invariants() {
        assert!(SourceLocation::file("a.md".to_string(), 1, 1).is_ok());
        assert!(SourceLocation::file("a.md".to_string(), 5, 3).is_err());
        assert!(SourceLocation::page(2, 2).is_ok());
        assert!(SourceLocation::page(3, 2).is_err());
        assert!(SourceLocation::region(1, 1, 2, 3, 4).is_ok());
        assert!(SourceLocation::region(1, 1, 2, 0, 4).is_err());
        assert!(SourceLocation::symbol("p.rs".to_string(), "f".to_string()).is_ok());
        assert!(SourceLocation::symbol(String::new(), "f".to_string()).is_err());
    }

    #[test]
    fn source_location_decode_validates() -> Result<(), Box<dyn std::error::Error>> {
        let valid: SourceLocation =
            serde_json::from_str(r#"{"File":{"path":"a.md","start_line":1,"end_line":4}}"#)?;
        assert_eq!(valid, SourceLocation::file("a.md".to_string(), 1, 4)?);
        assert!(
            serde_json::from_str::<SourceLocation>(
                r#"{"File":{"path":"a.md","start_line":5,"end_line":2}}"#
            )
            .is_err()
        );
        Ok(())
    }
}
