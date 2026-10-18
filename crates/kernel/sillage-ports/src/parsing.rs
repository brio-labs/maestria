use sillage_domain::{
    ArtifactId, ArtifactVersionId, ChunkId, ContentHash, CreateCardInput, StructureNode,
    StructureNodeId, StructureTreeError, StructureTreeValidationFailure, validate_structure_tree,
    validate_structure_tree_with_work,
};
use std::path::Path;

use super::traits::{FileHandle, FileMetadata, PortError};

pub use crate::bounded_parsing::{
    BoundedParseError, ParseBudget, ParseBudgetFailure, ParseCheckpoint,
};

/// Borrowed file input for bounded parsing; the parser does not take ownership.
#[derive(Debug, Clone, Copy)]
pub struct FileHandleRef<'a> {
    pub path: &'a Path,
    pub bytes: &'a [u8],
}

impl<'a> FileHandleRef<'a> {
    pub const fn new(path: &'a Path, bytes: &'a [u8]) -> Self {
        Self { path, bytes }
    }
}

/// Borrowed file metadata used to select parsers without allocating.
#[derive(Debug, Clone, Copy)]
pub struct FileMetadataRef<'a> {
    pub path: &'a Path,
    pub size: usize,
    /// Encoded extension bytes, excluding the dot and preserving case.
    pub extension: Option<&'a [u8]>,
}

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
        validate_structure_tree(root_id, &nodes).map_err(structure_tree_port_error)?;
        Ok(Self { root_id, nodes })
    }

    pub fn new_bounded(
        root_id: StructureNodeId,
        nodes: Vec<StructureNode>,
        budget: &mut ParseBudget<'_>,
    ) -> Result<Self, BoundedParseError> {
        validate_structure_tree_bounded(root_id, &nodes, budget)?;
        Ok(Self { root_id, nodes })
    }
}

fn structure_tree_port_error(error: StructureTreeError) -> PortError {
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
}

fn validate_structure_tree_bounded(
    root_id: StructureNodeId,
    nodes: &[StructureNode],
    budget: &mut ParseBudget<'_>,
) -> Result<(), BoundedParseError> {
    let entry_bytes = std::mem::size_of::<(StructureNodeId, usize)>()
        .checked_add(std::mem::size_of::<usize>())
        .ok_or_else(|| {
            PortError::invalid_input("admit bounded tree index", "entry size overflow")
        })?;
    let index_bytes = nodes
        .len()
        .checked_mul(entry_bytes)
        .and_then(|bytes| {
            nodes
                .len()
                .checked_mul(std::mem::size_of::<usize>())
                .and_then(|visits| bytes.checked_add(visits))
        })
        .and_then(|bytes| bytes.checked_add(std::mem::size_of::<Vec<(StructureNodeId, usize)>>()))
        .and_then(|bytes| bytes.checked_add(std::mem::size_of::<Vec<usize>>()))
        .ok_or_else(|| {
            PortError::invalid_input("admit bounded tree index", "index size overflow")
        })?;
    budget.charge_output(index_bytes)?;
    budget.checkpoint()?;
    match validate_structure_tree_with_work(root_id, nodes, |work| budget.charge_work(work)) {
        Ok(()) => Ok(()),
        Err(StructureTreeValidationFailure::Invariant(error)) => {
            Err(structure_tree_port_error(error).into())
        }
        Err(StructureTreeValidationFailure::Work(error)) => Err(BoundedParseError::Budget(error)),
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
    /// Whether this parser can route bounded input using borrowed metadata.
    ///
    /// Implementations must charge any metadata work through `budget`. Parsers
    /// must opt in explicitly; the default avoids an owned metadata fallback.
    fn supports_bounded(
        &self,
        _file: &FileMetadataRef<'_>,
        _budget: &mut ParseBudget<'_>,
    ) -> Result<bool, BoundedParseError> {
        Ok(false)
    }
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
    fn parse_outcome_bounded(
        &self,
        _file: FileHandleRef<'_>,
        _context: ParseContext,
        _budget: &mut ParseBudget<'_>,
    ) -> Result<ParseOutcome, BoundedParseError> {
        Err(BoundedParseError::Unsupported)
    }
    fn parse_with_ocr(
        &self,
        file: FileHandle,
        context: ParseContext,
        _pages: &[sillage_domain::OcrPageText],
    ) -> Result<ParsedArtifact, PortError> {
        self.parse(file, context)
    }
}
