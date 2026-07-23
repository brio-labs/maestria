#![forbid(unsafe_code)]

//! Capability traits and deterministic in-memory contract adapters for Maestria.
//!
//! This crate defines the side-effect boundaries used by runtime/storage adapters
//! without depending on a specific runtime, database, search engine, parser, or
//! harness implementation.

pub const PORTS_VERSION: &str = env!("CARGO_PKG_VERSION");

pub mod learned_sparse;
pub use learned_sparse::*;
pub mod lexical;
pub use lexical::*;
mod full_text;
pub use full_text::FullTextIndex;

mod traits;
pub use traits::*;
mod visual;
pub use visual::*;

mod ocr;
pub use ocr::*;
mod parsing;
pub use parsing::*;

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
