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
        if range.start > range.end {
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
