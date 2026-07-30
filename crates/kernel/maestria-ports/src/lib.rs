#![forbid(unsafe_code)]

//! Capability traits and deterministic in-memory contract adapters for Maestria.
//!
//! This crate defines the side-effect boundaries used by runtime/storage adapters
//! without depending on a specific runtime, database, search engine, parser, or
//! harness implementation.
//!
//! Responsibility map:
//! - `version`: package version metadata.
//! - `learned_sparse`: learned sparse capability contracts.
//! - `lexical`: lexical index capability contracts.
//! - `full_text`: full-text index capability contract.
//! - `traits`: core port traits.
//! - `visual`: visual embedding capability contracts.
//! - `ocr`: OCR capability contracts.
//! - `parsing`: parser capability contracts.
//! - `in_memory`: deterministic in-memory adapters.
//! - `contract_tests`: capability contract test modules.
//! - `graph_contract_tests`: graph contract test modules.
//! - `learned_sparse_contract_tests`: learned sparse contract test modules.

mod version;
pub use version::PORTS_VERSION;

pub mod learned_sparse;
pub use learned_sparse::{
    DEFAULT_MAX_CONTRIBUTIONS, DEFAULT_MAX_SPARSE_TERMS, LearnedSparseIndex, LearnedSparseProvider,
    SPARSE_REPRESENTATION_V1, SparseDocument, SparseFingerprint, SparseIdentity, SparseInputKind,
    SparseSearchHit, SparseSearchQuery, SparseTermContribution, SparseTermWeight, SparseVector,
};
pub mod lexical;
pub use lexical::{
    CardField, ChunkField, FieldSelector, HitReason, IndexedLexicalCard, IndexedLexicalChunk,
    LexicalCardHit, LexicalChunkHit, LexicalHitMetadata, LexicalQuery, MatchMode,
    RetrieverIdentity,
};
mod full_text;
pub use full_text::FullTextIndex;

mod traits;
pub use traits::{
    ApprovalRecord, ApprovalRepository, ApprovalRiskLevel, ApprovalStatus, ArtifactRepository,
    BlobStore, BoundedSearch, CardHit, CardRepository, ChunkRepository, EffectJournal,
    EffectJournalEntry, EffectJournalIntent, EffectJournalStatus, EmbeddingIdentity,
    EmbeddingInputKind, EmbeddingProvenance, EmbeddingProvider, EmbeddingRequest,
    EmbeddingResponse, EventFilter, EventLog, EvidenceRepository, FileHandle, FileMetadata,
    GovernedAgentProposal, GraphIndex, HarnessAdapter, HarnessCapabilities, HarnessCommandClass,
    HarnessOutcome, HarnessRequest, HarnessRunId, IdAllocator, IndexedCard, IndexedChunk,
    ModelAgentProposal, ModelAgentProposalError, PortError, ProviderDisclosure, ProviderEndpoint,
    ProviderTransport, RetentionPolicy, SearchFuture, SearchHit, SearchKnowledgeExecutor,
    SearchQuery, VectorEmbedding, VectorIndex, VectorSearchHit, VectorSearchQuery, WebFetchOptions,
    WebFetcher, WebSnapshotData,
};
mod visual;
pub use visual::{VisualEmbeddingProvider, VisualEmbeddingRequest, VisualSource};

mod ocr;
pub use ocr::{OcrIdentity, OcrPage, OcrProvider, OcrRequest, OcrResponse};
mod parsing;
pub use parsing::{
    DocumentTree, OcrPageSet, ParseContext, ParseOutcome, ParseStatus, ParsedArtifact, ParsedCard,
    ParsedChunk, ParsedRepresentation, Parser, RepresentationKind, SourceSpan,
};

mod in_memory;
pub use in_memory::{
    InMemoryApprovalRepository, InMemoryArtifactRepository, InMemoryBlobStore,
    InMemoryCardRepository, InMemoryChunkRepository, InMemoryEffectJournal, InMemoryEventLog,
    InMemoryEvidenceRepository, InMemoryFullTextIndex, InMemoryGraphIndex, InMemoryHarnessAdapter,
    InMemoryIdAllocator, InMemoryLearnedSparseIndex, InMemoryLearnedSparseProvider, InMemoryParser,
    InMemoryVectorIndex, InMemoryWebFetcher,
};

#[cfg(any(test, feature = "contract-tests"))]
pub mod contract_tests;

#[cfg(any(test, feature = "contract-tests"))]
pub mod graph_contract_tests;

#[cfg(any(test, feature = "contract-tests"))]
pub mod learned_sparse_contract_tests;

#[cfg(test)]
mod tests;
