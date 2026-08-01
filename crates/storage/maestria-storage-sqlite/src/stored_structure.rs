use maestria_domain::{ContentRange, StructureNode, StructureNodeId, StructureNodeType};
use serde::{Deserialize, Serialize};

/// Wire mirror of `maestria_domain::ContentRange`. The domain struct owns
/// its `start <= end` invariant, so decode validates through the fallible
/// constructor instead of reconstructing the struct directly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StoredContentRange {
    pub(crate) start: usize,
    pub(crate) end: usize,
}

impl StoredContentRange {
    pub(crate) fn from_domain(range: ContentRange) -> Self {
        Self {
            start: range.start(),
            end: range.end(),
        }
    }

    pub(crate) fn try_into_domain(self) -> Result<ContentRange, maestria_ports::PortError> {
        ContentRange::new(self.start, self.end).map_err(|error| {
            maestria_ports::PortError::InvalidInputContext {
                context: "decode stored content range",
                source: error.to_string(),
            }
        })
    }
}

/// Wire mirror of `maestria_domain::StructureNodeType`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum StoredStructureNodeType {
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

impl StoredStructureNodeType {
    pub(crate) fn from_domain(node_type: &StructureNodeType) -> Self {
        match node_type {
            StructureNodeType::Document => Self::Document,
            StructureNodeType::Section => Self::Section,
            StructureNodeType::Paragraph => Self::Paragraph,
            StructureNodeType::List => Self::List,
            StructureNodeType::ListItem => Self::ListItem,
            StructureNodeType::Table => Self::Table,
            StructureNodeType::Figure => Self::Figure,
            StructureNodeType::TableRow => Self::TableRow,
            StructureNodeType::TableCell => Self::TableCell,
            StructureNodeType::FigureCaption => Self::FigureCaption,
            StructureNodeType::Formula => Self::Formula,
            StructureNodeType::Code => Self::Code,
        }
    }

    pub(crate) fn try_into_domain(self) -> Result<StructureNodeType, maestria_ports::PortError> {
        Ok(match self {
            Self::Document => StructureNodeType::Document,
            Self::Section => StructureNodeType::Section,
            Self::Paragraph => StructureNodeType::Paragraph,
            Self::List => StructureNodeType::List,
            Self::ListItem => StructureNodeType::ListItem,
            Self::Table => StructureNodeType::Table,
            Self::Figure => StructureNodeType::Figure,
            Self::TableRow => StructureNodeType::TableRow,
            Self::TableCell => StructureNodeType::TableCell,
            Self::FigureCaption => StructureNodeType::FigureCaption,
            Self::Formula => StructureNodeType::Formula,
            Self::Code => StructureNodeType::Code,
        })
    }
}

/// Wire mirror of `maestria_domain::StructureNode`. Identifier fields are
/// flattened to their raw `u64` values and rebuilt via the id newtype
/// constructors on decode.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StoredStructureNode {
    pub(crate) id: u64,
    pub(crate) parent_id: Option<u64>,
    pub(crate) sibling_id: Option<u64>,
    pub(crate) node_type: StoredStructureNodeType,
    pub(crate) source_range: StoredContentRange,
    pub(crate) page: Option<u32>,
    pub(crate) section_path: Vec<String>,
    pub(crate) parser_generation: String,
    pub(crate) schema_generation: String,
    pub(crate) language: Option<String>,
}

impl StoredStructureNode {
    pub(crate) fn from_domain(node: &StructureNode) -> Self {
        Self {
            id: node.id.value(),
            parent_id: node.parent_id.map(|id| id.value()),
            sibling_id: node.sibling_id.map(|id| id.value()),
            node_type: StoredStructureNodeType::from_domain(&node.node_type),
            source_range: StoredContentRange::from_domain(node.source_range),
            page: node.page,
            section_path: node.section_path.clone(),
            parser_generation: node.parser_generation.clone(),
            schema_generation: node.schema_generation.clone(),
            language: node.language.clone(),
        }
    }

    pub(crate) fn try_into_domain(self) -> Result<StructureNode, maestria_ports::PortError> {
        Ok(StructureNode {
            id: StructureNodeId::new(self.id),
            parent_id: self.parent_id.map(StructureNodeId::new),
            sibling_id: self.sibling_id.map(StructureNodeId::new),
            node_type: self.node_type.try_into_domain()?,
            source_range: self.source_range.try_into_domain()?,
            page: self.page,
            section_path: self.section_path,
            parser_generation: self.parser_generation,
            schema_generation: self.schema_generation,
            language: self.language,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node() -> Result<StructureNode, Box<dyn std::error::Error>> {
        Ok(StructureNode {
            id: StructureNodeId::new(42),
            parent_id: Some(StructureNodeId::new(7)),
            sibling_id: None,
            node_type: StructureNodeType::Section,
            source_range: ContentRange::new(10, 20)?,
            page: Some(3),
            section_path: vec!["intro".to_string(), "background".to_string()],
            parser_generation: "parser-v1".to_string(),
            schema_generation: "schema-v1".to_string(),
            language: Some("en".to_string()),
        })
    }

    #[test]
    fn structure_node_round_trip() -> Result<(), Box<dyn std::error::Error>> {
        let original = node()?;
        let stored = StoredStructureNode::from_domain(&original);
        let json = serde_json::to_string(&stored)?;
        let decoded = serde_json::from_str::<StoredStructureNode>(&json)?;
        let restored = decoded.try_into_domain()?;
        assert_eq!(restored, original);
        Ok(())
    }

    #[test]
    fn content_range_round_trip() -> Result<(), Box<dyn std::error::Error>> {
        let original = ContentRange::new(3, 9)?;
        let stored = StoredContentRange::from_domain(original);
        let restored = stored.try_into_domain()?;
        assert_eq!(restored, original);
        Ok(())
    }

    #[test]
    fn every_node_type_round_trips() -> Result<(), Box<dyn std::error::Error>> {
        for node_type in [
            StructureNodeType::Document,
            StructureNodeType::Section,
            StructureNodeType::Paragraph,
            StructureNodeType::List,
            StructureNodeType::ListItem,
            StructureNodeType::Table,
            StructureNodeType::Figure,
            StructureNodeType::TableRow,
            StructureNodeType::TableCell,
            StructureNodeType::FigureCaption,
            StructureNodeType::Formula,
            StructureNodeType::Code,
        ] {
            let stored = StoredStructureNodeType::from_domain(&node_type);
            assert_eq!(stored.try_into_domain()?, node_type);
        }
        Ok(())
    }

    #[test]
    fn missing_node_field_is_rejected_during_deserialization()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut value = serde_json::from_str::<serde_json::Value>(
            r#"{"id":1,"parent_id":null,"sibling_id":null,"node_type":"section","source_range":{"start":1,"end":2},"page":null,"section_path":[],"parser_generation":"p","schema_generation":"s","language":null}"#,
        )?;
        value
            .as_object_mut()
            .ok_or_else(|| "expected JSON object".to_string())?
            .remove("parser_generation");
        assert!(serde_json::from_value::<StoredStructureNode>(value).is_err());
        Ok(())
    }

    #[test]
    fn unknown_node_field_is_rejected_during_deserialization()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut value = serde_json::to_value(StoredStructureNode::from_domain(&node()?))?;
        value
            .as_object_mut()
            .ok_or_else(|| "expected JSON object".to_string())?
            .insert("extra".to_string(), serde_json::Value::from("x"));
        assert!(serde_json::from_value::<StoredStructureNode>(value).is_err());
        Ok(())
    }
}
