use maestria_ports::PortError;
use std::{
    fs,
    path::PathBuf,
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

const DEFAULT_DPI: u32 = 300;

pub trait PdfRasterizer: Send + Sync {
    fn rasterize(&self, pdf: &[u8], pages: &[u32]) -> Result<Vec<RasterizedPage>, PortError>;
    fn check_available(&self) -> Result<(), PortError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RasterizedPage {
    pub page: u32,
    pub mime_type: String,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct PdftoppmRasterizer;

impl PdfRasterizer for PdftoppmRasterizer {
    fn rasterize(&self, pdf: &[u8], pages: &[u32]) -> Result<Vec<RasterizedPage>, PortError> {
        if pdf.is_empty() {
            return Err(PortError::InvalidInput {
                message: "cannot OCR an empty PDF".to_string(),
            });
        }
        let temporary = temporary_directory()?;
        let pdf_path = temporary.join("input.pdf");
        fs::write(&pdf_path, pdf).map_err(|error| PortError::Internal {
            message: format!("write temporary PDF for OCR: {error}"),
        })?;
        let mut rendered = Vec::with_capacity(pages.len());
        for &page in pages {
            if page == 0 {
                let _ = fs::remove_dir_all(&temporary);
                return Err(PortError::InvalidInput {
                    message: "PDF page numbers are one-based".to_string(),
                });
            }
            let output_prefix = temporary.join(format!("page-{page}"));
            let dpi = DEFAULT_DPI.to_string();
            let page_number = page.to_string();
            let output = Command::new("pdftoppm")
                .args([
                    "-png",
                    "-r",
                    dpi.as_str(),
                    "-f",
                    page_number.as_str(),
                    "-l",
                    page_number.as_str(),
                    "-singlefile",
                ])
                .arg(&pdf_path)
                .arg(&output_prefix)
                .output()
                .map_err(|error| PortError::Downstream {
                    message: format!("launch pdftoppm for page {page}: {error}"),
                })?;
            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                let _ = fs::remove_dir_all(&temporary);
                return Err(PortError::Downstream {
                    message: format!("pdftoppm failed for page {page}: {stderr}"),
                });
            }
            let image_path = output_prefix.with_extension("png");
            let bytes = fs::read(&image_path).map_err(|error| PortError::Downstream {
                message: format!("read rendered OCR page {page}: {error}"),
            })?;
            rendered.push(RasterizedPage {
                page,
                mime_type: "image/png".to_string(),
                bytes,
            });
        }
        let _ = fs::remove_dir_all(&temporary);
        Ok(rendered)
    }

    fn check_available(&self) -> Result<(), PortError> {
        let output = Command::new("pdftoppm")
            .arg("-v")
            .output()
            .map_err(|error| PortError::Downstream {
                message: format!("pdftoppm is unavailable: {error}"),
            })?;
        if output.status.success() || !output.stderr.is_empty() {
            return Ok(());
        }
        Err(PortError::Downstream {
            message: "pdftoppm is unavailable".to_string(),
        })
    }
}

fn temporary_directory() -> Result<PathBuf, PortError> {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| PortError::Internal {
            message: format!("read system clock for OCR temporary directory: {error}"),
        })?
        .as_nanos();
    let path =
        std::env::temp_dir().join(format!("maestria-ocr-{}-{timestamp}", std::process::id()));
    fs::create_dir(&path).map_err(|error| PortError::Internal {
        message: format!("create OCR temporary directory: {error}"),
    })?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_zero_based_page_numbers() {
        let result = PdftoppmRasterizer.rasterize(b"pdf", &[0]);
        assert!(matches!(result, Err(PortError::InvalidInput { .. })));
    }
}
