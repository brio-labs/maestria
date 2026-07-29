#![forbid(unsafe_code)]

//!
//! Responsibility map:
//! - `rasterizer`: PDF rasterization.
//! - `transport`: local OCR transport.
//! - `ocr_provider`: OCR provider implementation.
mod rasterizer;
mod transport;

pub use rasterizer::{PdfRasterizer, PdftoppmRasterizer, RasterizedPage};
pub use transport::{OcrTransport, UreqTransport};

mod ocr_provider;
pub use ocr_provider::LocalHttpOcrProvider;
