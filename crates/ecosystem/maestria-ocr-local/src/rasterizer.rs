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
            return Err(PortError::InvalidInputContext {
                context: "OCR PDF is empty",
                source: "PDF bytes must contain data".to_string(),
            });
        }
        let temporary = temporary_directory()?;
        let pdf_path = temporary.join("input.pdf");
        fs::write(&pdf_path, pdf).map_err(|error| {
            PortError::internal("write temporary PDF for OCR", error.to_string())
        })?;
        let mut rendered = Vec::with_capacity(pages.len());
        for &page in pages {
            if page == 0 {
                let _ = fs::remove_dir_all(&temporary);
                return Err(PortError::InvalidInputContext {
                    context: "OCR page number is zero",
                    source: "PDF page numbers are one-based".to_string(),
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
                .map_err(|error| PortError::downstream("launch pdftoppm", error.to_string()))?;
            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                let _ = fs::remove_dir_all(&temporary);
                return Err(PortError::DownstreamContext {
                    context: "pdftoppm failed",
                    source: format!("pdftoppm failed for page {page}: {stderr}"),
                });
            }
            let image_path = output_prefix.with_extension("png");
            let bytes = fs::read(&image_path).map_err(|error| {
                PortError::downstream("read rendered OCR page", error.to_string())
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
            .map_err(|error| PortError::downstream("pdftoppm is unavailable", error.to_string()))?;
        if output.status.success() || !output.stderr.is_empty() {
            return Ok(());
        }
        Err(PortError::DownstreamContext {
            context: "pdftoppm is unavailable",
            source: "pdftoppm did not report a usable version".to_string(),
        })
    }
}

fn temporary_directory() -> Result<PathBuf, PortError> {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| {
            PortError::internal(
                "read system clock for OCR temporary directory",
                error.to_string(),
            )
        })?
        .as_nanos();
    let path =
        std::env::temp_dir().join(format!("maestria-ocr-{}-{timestamp}", std::process::id()));
    fs::create_dir(&path).map_err(|error| {
        PortError::internal("create OCR temporary directory", error.to_string())
    })?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_zero_based_page_numbers() {
        let result = PdftoppmRasterizer.rasterize(b"pdf", &[0]);
        assert!(result.is_err_and(|error| error.is_invalid_input()));
    }
}
