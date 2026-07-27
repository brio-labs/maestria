use super::*;
use maestria_ports::FileHandle;
use std::path::PathBuf;
use std::sync::Mutex;

#[derive(Default)]
struct FixtureRasterizer;

impl PdfRasterizer for FixtureRasterizer {
    fn rasterize(&self, _pdf: &[u8], pages: &[u32]) -> Result<Vec<RasterizedPage>, PortError> {
        Ok(pages
            .iter()
            .map(|page| RasterizedPage {
                page: *page,
                mime_type: "image/png".to_string(),
                bytes: format!("page-{page}").into_bytes(),
            })
            .collect())
    }

    fn check_available(&self) -> Result<(), PortError> {
        Ok(())
    }
}

struct FixtureTransport {
    requests: Mutex<Vec<Vec<u8>>>,
}

impl OcrTransport for FixtureTransport {
    fn post(&self, _endpoint: &str, body: Vec<u8>) -> Result<Vec<u8>, PortError> {
        self.requests
            .lock()
            .map_err(|_| PortError::Internal {
                message: "fixture transport mutex poisoned".to_string(),
            })?
            .push(body);
        Ok(br#"{"choices":[{"message":{"content":"recognized page"}}]}"#.to_vec())
    }
}

fn identity() -> OcrIdentity {
    OcrIdentity {
        provider: "baidu".to_string(),
        model: "Unlimited-OCR".to_string(),
        revision: "main".to_string(),
        artifact_hash: "sha256:0000000000000000000000000000000000000000000000000000000000000000"
            .to_string(),
        preprocessing_version: "pdf-pdftoppm-v1".to_string(),
    }
}

#[test]
fn rejects_non_loopback_endpoints() {
    let result = LocalHttpOcrProvider::with_parts(
        "https://example.com/v1/chat/completions",
        "Unlimited-OCR",
        identity(),
        Arc::new(FixtureRasterizer),
        Arc::new(FixtureTransport {
            requests: Mutex::new(Vec::new()),
        }),
    );
    assert!(matches!(result, Err(PortError::InvalidInput { .. })));
}

struct ErrorTransport {
    error: PortError,
}

impl OcrTransport for ErrorTransport {
    fn post(&self, _endpoint: &str, _body: Vec<u8>) -> Result<Vec<u8>, PortError> {
        Err(self.error.clone())
    }
}

#[test]
fn rejects_empty_pages() -> Result<(), PortError> {
    let provider = LocalHttpOcrProvider::with_parts(
        "http://127.0.0.1:10000/v1/chat/completions",
        "Unlimited-OCR",
        identity(),
        Arc::new(FixtureRasterizer),
        Arc::new(FixtureTransport {
            requests: Mutex::new(Vec::new()),
        }),
    )?;
    let result = provider.recognize(OcrRequest {
        file: FileHandle {
            path: PathBuf::from("scan.pdf"),
            bytes: b"pdf".to_vec(),
        },
        pages: vec![],
    });
    assert!(
        matches!(result, Err(PortError::InvalidInput { .. })),
        "expected InvalidInput for empty pages, got {result:?}"
    );
    Ok(())
}

#[test]
fn propagates_transport_error() -> Result<(), PortError> {
    let provider = LocalHttpOcrProvider::with_parts(
        "http://127.0.0.1:10000/v1/chat/completions",
        "Unlimited-OCR",
        identity(),
        Arc::new(FixtureRasterizer),
        Arc::new(ErrorTransport {
            error: PortError::Downstream {
                message: "ocr transport failed".to_string(),
            },
        }),
    )?;
    let result = provider.recognize(OcrRequest {
        file: FileHandle {
            path: PathBuf::from("scan.pdf"),
            bytes: b"pdf".to_vec(),
        },
        pages: vec![1],
    });
    assert!(
        matches!(result, Err(PortError::Downstream { .. })),
        "expected Downstream error, got {result:?}"
    );
    Ok(())
}

#[test]
fn rejects_malformed_json_response() -> Result<(), PortError> {
    struct MalformedTransport;

    impl OcrTransport for MalformedTransport {
        fn post(&self, _endpoint: &str, _body: Vec<u8>) -> Result<Vec<u8>, PortError> {
            Ok(br#"not-json"#.to_vec())
        }
    }

    let provider = LocalHttpOcrProvider::with_parts(
        "http://127.0.0.1:10000/v1/chat/completions",
        "Unlimited-OCR",
        identity(),
        Arc::new(FixtureRasterizer),
        Arc::new(MalformedTransport),
    )?;
    let result = provider.recognize(OcrRequest {
        file: FileHandle {
            path: PathBuf::from("scan.pdf"),
            bytes: b"pdf".to_vec(),
        },
        pages: vec![1],
    });
    assert!(
        matches!(
            result,
            Err(PortError::Downstream { .. } | PortError::DownstreamContext { .. })
        ),
        "expected Downstream error for malformed JSON, got {result:?}"
    );
    Ok(())
}

#[test]
fn rejects_zero_byte_pdf() -> Result<(), PortError> {
    let provider = LocalHttpOcrProvider::with_parts(
        "http://127.0.0.1:10000/v1/chat/completions",
        "Unlimited-OCR",
        identity(),
        Arc::new(FixtureRasterizer),
        Arc::new(FixtureTransport {
            requests: Mutex::new(Vec::new()),
        }),
    )?;
    let result = provider.recognize(OcrRequest {
        file: FileHandle {
            path: PathBuf::from("scan.pdf"),
            bytes: vec![],
        },
        pages: vec![1],
    });
    assert!(
        matches!(result, Err(PortError::InvalidInput { .. })),
        "expected InvalidInput for zero-byte PDF, got {result:?}"
    );
    Ok(())
}

#[test]
fn propagates_rasterizer_failure() -> Result<(), PortError> {
    struct ErrorRasterizer;

    impl PdfRasterizer for ErrorRasterizer {
        fn rasterize(&self, _pdf: &[u8], _pages: &[u32]) -> Result<Vec<RasterizedPage>, PortError> {
            Err(PortError::Downstream {
                message: "rasterizer failed".to_string(),
            })
        }

        fn check_available(&self) -> Result<(), PortError> {
            Ok(())
        }
    }

    let provider = LocalHttpOcrProvider::with_parts(
        "http://127.0.0.1:10000/v1/chat/completions",
        "Unlimited-OCR",
        identity(),
        Arc::new(ErrorRasterizer),
        Arc::new(FixtureTransport {
            requests: Mutex::new(Vec::new()),
        }),
    )?;
    let result = provider.recognize(OcrRequest {
        file: FileHandle {
            path: PathBuf::from("scan.pdf"),
            bytes: b"pdf".to_vec(),
        },
        pages: vec![1],
    });
    assert!(
        matches!(result, Err(PortError::Downstream { .. })),
        "expected Downstream error for rasterizer failure, got {result:?}"
    );
    Ok(())
}

#[test]
fn rejects_empty_rasterize_bytes() -> Result<(), PortError> {
    struct EmptyRasterizer;

    impl PdfRasterizer for EmptyRasterizer {
        fn rasterize(&self, _pdf: &[u8], pages: &[u32]) -> Result<Vec<RasterizedPage>, PortError> {
            Ok(pages
                .iter()
                .map(|page| RasterizedPage {
                    page: *page,
                    mime_type: "image/png".to_string(),
                    bytes: vec![],
                })
                .collect())
        }

        fn check_available(&self) -> Result<(), PortError> {
            Ok(())
        }
    }

    let provider = LocalHttpOcrProvider::with_parts(
        "http://127.0.0.1:10000/v1/chat/completions",
        "Unlimited-OCR",
        identity(),
        Arc::new(EmptyRasterizer),
        Arc::new(FixtureTransport {
            requests: Mutex::new(Vec::new()),
        }),
    )?;
    let result = provider.recognize(OcrRequest {
        file: FileHandle {
            path: PathBuf::from("scan.pdf"),
            bytes: b"pdf".to_vec(),
        },
        pages: vec![1],
    });
    assert!(
        matches!(result, Err(PortError::InvalidInput { .. })),
        "expected InvalidInput for empty rasterized bytes, got {result:?}"
    );
    Ok(())
}

#[test]
fn sends_one_image_request_per_page_and_preserves_identity() -> Result<(), PortError> {
    let transport = Arc::new(FixtureTransport {
        requests: Mutex::new(Vec::new()),
    });
    let provider = LocalHttpOcrProvider::with_parts(
        "http://127.0.0.1:10000/v1/chat/completions",
        "Unlimited-OCR",
        identity(),
        Arc::new(FixtureRasterizer),
        transport.clone(),
    )?;
    let response = provider.recognize(OcrRequest {
        file: FileHandle {
            path: PathBuf::from("scan.pdf"),
            bytes: b"pdf".to_vec(),
        },
        pages: vec![1, 3],
    })?;
    assert_eq!(response.pages.len(), 2);
    assert_eq!(response.pages[1].page, 3);
    assert_eq!(response.pages[0].text, "recognized page");
    assert_eq!(response.identity, identity());
    assert_eq!(
        transport
            .requests
            .lock()
            .map_err(|_| PortError::Internal {
                message: "fixture transport mutex poisoned".to_string(),
            })?
            .len(),
        2
    );
    Ok(())
}
