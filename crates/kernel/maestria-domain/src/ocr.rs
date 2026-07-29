use crate::{ArtifactId, BlobId, ContentHash};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OcrValidationError {
    EmptyValue(&'static str),
    InvalidPage(u32),
    EmptyPages,
    DuplicatePage(u32),
    UnexpectedPage(u32),
    MissingPage(u32),
    EmptyText(u32),
    IdentityMismatch,
    DisclosureMismatch,
    RequestIdentityMismatch,
}

impl fmt::Display for OcrValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyValue(name) => write!(f, "OCR {name} must not be empty"),
            Self::InvalidPage(page) => write!(f, "OCR page {page} is invalid"),
            Self::EmptyPages => f.write_str("OCR page set must not be empty"),
            Self::DuplicatePage(page) => write!(f, "OCR page {page} is duplicated"),
            Self::UnexpectedPage(page) => write!(f, "OCR result contains unexpected page {page}"),
            Self::MissingPage(page) => write!(f, "OCR result is missing page {page}"),
            Self::EmptyText(page) => write!(f, "OCR result text for page {page} is empty"),
            Self::IdentityMismatch => f.write_str("OCR provider identity does not match intent"),
            Self::DisclosureMismatch => {
                f.write_str("OCR provider disclosure does not match intent")
            }
            Self::RequestIdentityMismatch => {
                f.write_str("OCR request identity does not match intent")
            }
        }
    }
}
impl std::error::Error for OcrValidationError {}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct OcrRequestId(String);

impl OcrRequestId {
    pub fn as_str(&self) -> &str {
        &self.0
    }
    pub fn parse(value: impl Into<String>) -> Result<Self, OcrValidationError> {
        let value = value.into();
        if !value.starts_with("ocr:sha256:") || value.len() != 75 {
            return Err(OcrValidationError::EmptyValue("request id"));
        }
        Ok(Self(value))
    }
}
impl fmt::Display for OcrRequestId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OcrProviderIdentity {
    provider: String,
    model: String,
    revision: String,
    artifact_hash: String,
    preprocessing_version: String,
}
impl OcrProviderIdentity {
    pub fn new(
        provider: impl Into<String>,
        model: impl Into<String>,
        revision: impl Into<String>,
        artifact_hash: impl Into<String>,
        preprocessing_version: impl Into<String>,
    ) -> Result<Self, OcrValidationError> {
        let identity = Self {
            provider: provider.into(),
            model: model.into(),
            revision: revision.into(),
            artifact_hash: artifact_hash.into(),
            preprocessing_version: preprocessing_version.into(),
        };
        for (name, value) in [
            ("provider", &identity.provider),
            ("model", &identity.model),
            ("revision", &identity.revision),
            ("artifact hash", &identity.artifact_hash),
            ("preprocessing version", &identity.preprocessing_version),
        ] {
            if value.trim().is_empty() {
                return Err(OcrValidationError::EmptyValue(name));
            }
        }
        Ok(identity)
    }
    pub fn provider(&self) -> &str {
        &self.provider
    }
    pub fn model(&self) -> &str {
        &self.model
    }
    pub fn revision(&self) -> &str {
        &self.revision
    }
    pub fn artifact_hash(&self) -> &str {
        &self.artifact_hash
    }
    pub fn preprocessing_version(&self) -> &str {
        &self.preprocessing_version
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OcrRetentionPolicy {
    NoRetention,
    ProviderDefined,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OcrDisclosure {
    remote: bool,
    retention: OcrRetentionPolicy,
}
impl OcrDisclosure {
    pub const fn new(remote: bool, retention: OcrRetentionPolicy) -> Self {
        Self { remote, retention }
    }
    pub const fn remote(&self) -> bool {
        self.remote
    }
    pub const fn retention(&self) -> OcrRetentionPolicy {
        self.retention
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OcrIntent {
    request_id: OcrRequestId,
    artifact_id: ArtifactId,
    source_blob: BlobId,
    source_hash: ContentHash,
    pages: Vec<u32>,
    provider: OcrProviderIdentity,
    disclosure: OcrDisclosure,
}

impl OcrIntent {
    pub fn new(
        artifact_id: ArtifactId,
        source_blob: BlobId,
        source_hash: ContentHash,
        pages: impl IntoIterator<Item = u32>,
        provider: OcrProviderIdentity,
        disclosure: OcrDisclosure,
    ) -> Result<Self, OcrValidationError> {
        let pages = validate_pages(pages)?;
        let request_id = OcrRequestId(stable_request_id(
            artifact_id,
            source_blob,
            &source_hash,
            &pages,
            &provider,
            &disclosure,
        ));
        Ok(Self {
            request_id,
            artifact_id,
            source_blob,
            source_hash,
            pages,
            provider,
            disclosure,
        })
    }
    pub fn request_id(&self) -> &OcrRequestId {
        &self.request_id
    }
    pub const fn artifact_id(&self) -> ArtifactId {
        self.artifact_id
    }
    pub const fn source_blob(&self) -> BlobId {
        self.source_blob
    }
    pub fn source_hash(&self) -> &ContentHash {
        &self.source_hash
    }
    pub fn pages(&self) -> &[u32] {
        &self.pages
    }
    pub fn provider(&self) -> &OcrProviderIdentity {
        &self.provider
    }
    pub const fn disclosure(&self) -> &OcrDisclosure {
        &self.disclosure
    }
    pub fn has_page(&self, page: u32) -> bool {
        self.pages.binary_search(&page).is_ok()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OcrPageText {
    page: u32,
    text: String,
}
impl OcrPageText {
    pub fn new(page: u32, text: impl Into<String>) -> Result<Self, OcrValidationError> {
        if page == 0 {
            return Err(OcrValidationError::InvalidPage(page));
        }
        let text = text.into();
        if text.trim().is_empty() {
            return Err(OcrValidationError::EmptyText(page));
        }
        Ok(Self { page, text })
    }
    pub const fn page(&self) -> u32 {
        self.page
    }
    pub fn text(&self) -> &str {
        &self.text
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OcrCompletion {
    request_id: OcrRequestId,
    pages: Vec<OcrPageText>,
}
impl OcrCompletion {
    pub fn new(
        intent: &OcrIntent,
        pages: impl IntoIterator<Item = OcrPageText>,
    ) -> Result<Self, OcrValidationError> {
        let mut values = pages.into_iter().collect::<Vec<_>>();
        if values.is_empty() {
            return Err(OcrValidationError::EmptyPages);
        }
        values.sort_by_key(OcrPageText::page);
        let mut seen = BTreeSet::new();
        for page in &values {
            if !seen.insert(page.page) {
                return Err(OcrValidationError::DuplicatePage(page.page));
            }
            if !intent.has_page(page.page) {
                return Err(OcrValidationError::UnexpectedPage(page.page));
            }
        }
        for expected in intent.pages() {
            if !seen.contains(expected) {
                return Err(OcrValidationError::MissingPage(*expected));
            }
        }
        Ok(Self {
            request_id: intent.request_id.clone(),
            pages: values,
        })
    }
    pub fn from_parts(
        request_id: OcrRequestId,
        pages: Vec<OcrPageText>,
    ) -> Result<Self, OcrValidationError> {
        if pages.is_empty() {
            return Err(OcrValidationError::EmptyPages);
        }
        let mut seen = BTreeSet::new();
        let mut sorted = pages;
        sorted.sort_by_key(OcrPageText::page);
        for page in &sorted {
            if !seen.insert(page.page) {
                return Err(OcrValidationError::DuplicatePage(page.page));
            }
        }
        Ok(Self {
            request_id,
            pages: sorted,
        })
    }
    pub fn request_id(&self) -> &OcrRequestId {
        &self.request_id
    }
    pub fn pages(&self) -> &[OcrPageText] {
        &self.pages
    }
}

fn validate_pages(pages: impl IntoIterator<Item = u32>) -> Result<Vec<u32>, OcrValidationError> {
    let mut values = pages.into_iter().collect::<Vec<_>>();
    if values.is_empty() {
        return Err(OcrValidationError::EmptyPages);
    }
    values.sort_unstable();
    let mut prior = None;
    for page in &values {
        if *page == 0 {
            return Err(OcrValidationError::InvalidPage(*page));
        }
        if prior == Some(*page) {
            return Err(OcrValidationError::DuplicatePage(*page));
        }
        prior = Some(*page);
    }
    Ok(values)
}

fn stable_request_id(
    artifact_id: ArtifactId,
    source_blob: BlobId,
    source_hash: &ContentHash,
    pages: &[u32],
    provider: &OcrProviderIdentity,
    disclosure: &OcrDisclosure,
) -> String {
    let mut input = format!(
        "{}\n{}\n{}\n",
        artifact_id.value(),
        source_blob.value(),
        source_hash.as_str()
    );
    for page in pages {
        input.push_str(&format!("{}\n", page));
    }
    input.push_str(provider.provider());
    input.push('\n');
    input.push_str(provider.model());
    input.push('\n');
    input.push_str(provider.revision());
    input.push('\n');
    input.push_str(provider.artifact_hash());
    input.push('\n');
    input.push_str(provider.preprocessing_version());
    input.push('\n');
    input.push_str(if disclosure.remote { "remote" } else { "local" });
    input.push_str(match disclosure.retention {
        OcrRetentionPolicy::NoRetention => "\nno_retention",
        OcrRetentionPolicy::ProviderDefined => "\nprovider_defined",
    });
    let digest = Sha256::digest(input.as_bytes());
    format!(
        "ocr:sha256:{}",
        digest
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    )
}

impl OcrCompletion {
    pub fn validate_against(&self, intent: &OcrIntent) -> Result<(), OcrValidationError> {
        if self.request_id != *intent.request_id() {
            return Err(OcrValidationError::RequestIdentityMismatch);
        }
        let _ = Self::new(intent, self.pages.clone())?;
        Ok(())
    }
}
