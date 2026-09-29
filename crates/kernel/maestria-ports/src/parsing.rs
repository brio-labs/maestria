use maestria_domain::{
    ArtifactId, ArtifactVersionId, ChunkId, ContentHash, CreateCardInput, StructureNode,
    StructureNodeId, StructureTreeError, validate_structure_tree,
};

use super::traits::{FileHandle, FileMetadata, PortError};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseContext {
    pub artifact_id: ArtifactId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseStatus {
    Parsed,
    Unsupported,
    Failed,
    MetadataOnly,
    NeedsOcr,
    Quarantined,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OcrPageSet(Vec<u32>);

impl OcrPageSet {
    pub fn try_new(pages: impl IntoIterator<Item = u32>) -> Result<Self, PortError> {
        let mut pages = pages.into_iter().collect::<Vec<_>>();
        if pages.is_empty() {
            return Err(PortError::InvalidInputContext {
                context: "OCR page set is empty",
                source: "at least one page is required".to_string(),
            });
        }
        pages.sort_unstable();
        for page in &pages {
            if *page == 0 {
                return Err(PortError::InvalidInputContext {
                    context: "OCR page is zero",
                    source: "PDF pages are one-based".to_string(),
                });
            }
        }
        if pages.windows(2).any(|pair| pair[0] == pair[1]) {
            return Err(PortError::InvalidInputContext {
                context: "OCR page set contains duplicates",
                source: "each page may be requested once".to_string(),
            });
        }
        Ok(Self(pages))
    }
    pub fn as_slice(&self) -> &[u32] {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseOutcome {
    Complete(ParsedArtifact),
    NeedsOcr {
        partial: ParsedArtifact,
        pages: OcrPageSet,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum RepresentationKind {
    Raw,
    Retrieval,
    Contextual,
    Summary,
    Visual,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedRepresentation {
    pub kind: RepresentationKind,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocumentTree {
    root_id: StructureNodeId,
    nodes: Vec<StructureNode>,
}

impl DocumentTree {
    pub const fn root_id(&self) -> StructureNodeId {
        self.root_id
    }

    pub fn nodes(&self) -> &[StructureNode] {
        &self.nodes
    }

    pub fn new(root_id: StructureNodeId, nodes: Vec<StructureNode>) -> Result<Self, PortError> {
        validate_structure_tree(root_id, &nodes).map_err(|error| {
            let (context, source) = match error {
                StructureTreeError::DuplicateNodeIds => (
                    "document tree contains duplicate node IDs",
                    "node identifiers must be unique",
                ),
                StructureTreeError::InvalidRoot => (
                    "document tree root is invalid",
                    "tree must have one declared root matching root_id",
                ),
                StructureTreeError::DanglingLink => (
                    "document tree contains a dangling link",
                    "parent or sibling link targets an unknown node",
                ),
                StructureTreeError::ParentCycle => (
                    "document tree contains a parent cycle",
                    "parent links must terminate at the root",
                ),
                StructureTreeError::SiblingCycle => (
                    "document tree contains a sibling cycle",
                    "sibling links must terminate at a null link",
                ),
            };
            PortError::InvalidInputContext {
                context,
                source: source.to_string(),
            }
        })?;
        Ok(Self { root_id, nodes })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceSpan {
    TextSpan {
        start_line: usize,
        end_line: usize,
    },
    DocxParagraphSpan {
        start_paragraph: usize,
        end_paragraph: usize,
    },
    PdfSpan {
        page: usize,
    },
    PdfRegion {
        page: usize,
        x: u32,
        y: u32,
        width: u32,
        height: u32,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedChunk {
    pub chunk_id: ChunkId,
    pub artifact_id: ArtifactId,
    pub node_id: StructureNodeId,
    pub text: String,
    pub representations: Vec<ParsedRepresentation>,
    pub source_span: SourceSpan,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedCard {
    pub card: CreateCardInput,
    pub node_id: StructureNodeId,
    pub source_span: SourceSpan,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedArtifact {
    pub artifact_id: ArtifactId,
    pub artifact_version_id: ArtifactVersionId,
    pub content_hash: ContentHash,
    pub tree: DocumentTree,
    pub status: ParseStatus,
    pub chunks: Vec<ParsedChunk>,
    pub cards: Vec<ParsedCard>,
}

pub trait Parser: Send + Sync {
    fn id(&self) -> &'static str;
    fn supports(&self, file: &FileMetadata) -> bool;
    fn parse(&self, file: FileHandle, context: ParseContext) -> Result<ParsedArtifact, PortError>;
    fn parse_outcome(
        &self,
        file: FileHandle,
        context: ParseContext,
    ) -> Result<ParseOutcome, PortError> {
        let parsed = self.parse(file, context)?;
        if parsed.status == ParseStatus::NeedsOcr {
            return Err(PortError::InvalidInputContext {
                context: "parser returned OCR pending without a page set",
                source: "OCR-capable parsers must implement parse_outcome".to_string(),
            });
        }
        Ok(ParseOutcome::Complete(parsed))
    }
    fn parse_with_ocr(
        &self,
        file: FileHandle,
        context: ParseContext,
        _pages: &[maestria_domain::OcrPageText],
    ) -> Result<ParsedArtifact, PortError> {
        self.parse(file, context)
    }
}
