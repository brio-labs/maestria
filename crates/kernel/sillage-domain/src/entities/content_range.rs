#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    ::serde::Serialize,
    ::serde::Deserialize,
)]
#[serde(try_from = "ContentRangeDto")]
pub struct ContentRange {
    start: usize,
    end: usize,
}

impl ContentRange {
    /// Builds a content-relative range whose start does not exceed its end.
    pub fn new(start: usize, end: usize) -> Result<Self, ContentRangeError> {
        if start > end {
            return Err(ContentRangeError::StartAfterEnd { start, end });
        }
        Ok(Self { start, end })
    }

    pub const fn start(&self) -> usize {
        self.start
    }

    pub const fn end(&self) -> usize {
        self.end
    }
}

/// Failure while building a validated [`ContentRange`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContentRangeError {
    StartAfterEnd { start: usize, end: usize },
}

impl std::fmt::Display for ContentRangeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::StartAfterEnd { start, end } => {
                write!(f, "content range start {start} must not exceed end {end}")
            }
        }
    }
}

impl std::error::Error for ContentRangeError {}

#[derive(::serde::Deserialize)]
struct ContentRangeDto {
    start: usize,
    end: usize,
}

impl TryFrom<ContentRangeDto> for ContentRange {
    type Error = ContentRangeError;

    fn try_from(dto: ContentRangeDto) -> Result<Self, Self::Error> {
        Self::new(dto.start, dto.end)
    }
}
