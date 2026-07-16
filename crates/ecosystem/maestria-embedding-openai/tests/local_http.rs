use maestria_embedding_openai::LocalHttpEmbeddingProvider;
use maestria_ports::{EmbeddingInputKind, EmbeddingProvider, EmbeddingRequest, PortError};
use std::{
    io::{Read, Write},
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
            let mut request = [0_u8; 4096];
            let _ = stream.read(&mut request)?;
            let body = br#"{"data":[{"embedding":[0.4,0.6]}],"model":"fake-v1"}"#;
            write!(
                stream,
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n",
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
