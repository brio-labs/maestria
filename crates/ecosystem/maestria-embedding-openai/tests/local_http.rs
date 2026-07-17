use maestria_embedding_openai::LocalHttpEmbeddingProvider;
use maestria_ports::{EmbeddingInputKind, EmbeddingProvider, EmbeddingRequest, PortError};
use std::{
    io::{Error, ErrorKind, Read, Write},
    net::TcpListener,
    thread,
};

#[test]
fn posts_to_local_http_endpoint() -> Result<(), PortError> {
    let listener = TcpListener::bind(("127.0.0.1", 0)).map_err(|error| PortError::Internal {
        message: format!("bind test embedding server: {error}"),
    })?;
    let port = listener
        .local_addr()
        .map_err(|error| PortError::Internal {
            message: format!("read test embedding server address: {error}"),
        })?
        .port();
    let server = thread::spawn(move || {
        listener.accept().and_then(|(mut stream, _)| {
            let mut request = Vec::new();
            let mut buffer = [0_u8; 1024];
            let header_end = loop {
                let read = stream.read(&mut buffer)?;
                if read == 0 {
                    return Err(Error::new(
                        ErrorKind::UnexpectedEof,
                        "embedding request ended before headers",
                    ));
                }
                request.extend_from_slice(&buffer[..read]);
                if let Some(index) = request
                    .windows(4)
                    .position(|window| window == b"\r\n\r\n")
                {
                    break index + 4;
                }
            };
            let headers = String::from_utf8_lossy(&request[..header_end]);
            let content_length = headers
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length")
                        .then_some(value.trim())
                })
                .map(str::parse::<usize>)
                .transpose()
                .map_err(|error| {
                    Error::new(
                        ErrorKind::InvalidData,
                        format!("invalid embedding content length: {error}"),
                    )
                })?
                .unwrap_or(0);
            let request_end = header_end.checked_add(content_length).ok_or_else(|| {
                Error::new(ErrorKind::InvalidData, "embedding request length overflowed")
            })?;
            while request.len() < request_end {
                let read = stream.read(&mut buffer)?;
                if read == 0 {
                    return Err(Error::new(
                        ErrorKind::UnexpectedEof,
                        "embedding request ended before body",
                    ));
                }
                request.extend_from_slice(&buffer[..read]);
            }

            let body = br#"{"data":[{"embedding":[0.4,0.6]}],"model":"fake-v1"}"#;
            write!(
                stream,
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\nconnection: close\r\ncontent-length: {}\r\n\r\n",
                body.len()
            )?;
            stream.write_all(body)?;
            stream.flush()
        })
    });
    let provider = LocalHttpEmbeddingProvider::new(
        &format!("http://127.0.0.1:{port}/v1/embeddings"),
        "local-model",
        Some(2),
    )?;
    let result = provider.embed(EmbeddingRequest {
        text: "local text".to_string(),
        model: "local-model".to_string(),
        kind: EmbeddingInputKind::Query,
        identity: provider.identity().clone(),
    })?;
    let server_result = server.join().map_err(|_| PortError::Internal {
        message: "test embedding server panicked".to_string(),
    })?;
    server_result.map_err(|error| PortError::Internal {
        message: format!("test embedding server failed: {error}"),
    })?;
    assert_eq!(result.vector, vec![0.4, 0.6]);
    assert_eq!(result.model_version, "fake-v1");
    Ok(())
}
