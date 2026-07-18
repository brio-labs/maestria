use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};

use super::common::{TempDir, assert_index_ok, assert_init_ok, assert_ok, write_file};

struct LocalEmbeddingServer {
    endpoint: String,
    stop: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
}

impl LocalEmbeddingServer {
    fn start() -> Result<Self, Box<dyn std::error::Error>> {
        let listener = TcpListener::bind(("127.0.0.1", 0))?;
        listener.set_nonblocking(true)?;
        let port = listener.local_addr()?.port();
        let endpoint = format!("http://127.0.0.1:{port}/v1/embeddings");
        let stop = Arc::new(AtomicBool::new(false));
        let stop_for_thread = Arc::clone(&stop);
        let handle = thread::spawn(move || {
            while !stop_for_thread.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        let _ = respond_to_embedding_request(&mut stream);
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(5));
                    }
                    Err(_) => break,
                }
            }
        });
        Ok(Self {
            endpoint,
            stop,
            handle: Some(handle),
        })
    }

    fn endpoint(&self) -> &str {
        &self.endpoint
    }
}

fn respond_to_embedding_request(stream: &mut TcpStream) -> Result<(), std::io::Error> {
    stream.set_read_timeout(Some(Duration::from_secs(2)))?;
    let mut request = Vec::new();
    let mut buffer = [0_u8; 1024];
    let header_end = loop {
        let read = stream.read(&mut buffer)?;
        if read == 0 {
            return Ok(());
        }
        request.extend_from_slice(&buffer[..read]);
        if let Some(position) = request.windows(4).position(|window| window == b"\r\n\r\n") {
            break position + 4;
        }
        if request.len() > 64 * 1024 {
            return Ok(());
        }
    };
    let headers = String::from_utf8_lossy(&request[..header_end]);
    let content_length = headers
        .lines()
        .find_map(|line| {
            line.strip_prefix("Content-Length:")
                .or_else(|| line.strip_prefix("content-length:"))
        })
        .and_then(|value| value.trim().parse::<usize>().ok())
        .map_or(0, |value| value);
    while request.len() < header_end.saturating_add(content_length) {
        let read = stream.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        request.extend_from_slice(&buffer[..read]);
    }
    let body = br#"{"data":[{"embedding":[0.4,0.6]}],"model":"local-model","usage":{"prompt_tokens":1,"total_tokens":1}}"#;
    write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )?;
    stream.write_all(body)?;
    stream.flush()
}

impl Drop for LocalEmbeddingServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        let _ = TcpStream::connect(
            self.endpoint["http://".len()..]
                .split_once('/')
                .map_or_else(
                    || self.endpoint["http://".len()..].to_string(),
                    |(host, _)| host.to_string(),
                ),
        );
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

#[test]
fn lexical_generation_is_active_after_index_startup() -> Result<(), Box<dyn std::error::Error>> {
    let workspace = TempDir::new("maestria-release-generation-workspace")?;
    let instance = TempDir::new("maestria-release-generation-instance")?;
    let instance_path = instance.path().to_string_lossy().into_owned();
    let workspace_path = workspace.path().to_string_lossy().into_owned();
    assert_init_ok(&instance_path, &workspace_path)?;
    write_file(workspace.path(), "notes.md", "generation recovery evidence")?;
    assert_index_ok(
        &instance_path,
        &workspace.path().join("notes.md").to_string_lossy(),
    )?;

    let generations = assert_ok(&["index", "generations", "-i", &instance_path])?;
    assert!(
        generations.contains("name=lexical_text_v1"),
        "{generations}"
    );
    assert!(generations.contains("lifecycle=Active"), "{generations}");
    assert!(generations.contains("serveable=true"), "{generations}");
    Ok(())
}

#[test]
fn configured_dense_generation_survives_projection_rebuild_and_fallback()
-> Result<(), Box<dyn std::error::Error>> {
    let server = LocalEmbeddingServer::start()?;
    let workspace = TempDir::new("maestria-release-dense-workspace")?;
    let instance = TempDir::new("maestria-release-dense-instance")?;
    let instance_path = instance.path().to_string_lossy().into_owned();
    let workspace_path = workspace.path().to_string_lossy().into_owned();
    assert_init_ok(&instance_path, &workspace_path)?;
    let manifest = format!(
        "schema_version=1\nroot={instance_path}\nread_root={workspace_path}\nexcluded_pattern=.env\nembedding_enabled=true\nembedding_endpoint={}\nembedding_provider=local\nembedding_revision=v1\nembedding_artifact_hash=sha256:0000000000000000000000000000000000000000000000000000000000000000\nembedding_preprocessing_version=v1\nembedding_model=local-model\nembedding_dimensions=2\n",
        server.endpoint()
    );
    write_file(instance.path(), "manifest.txt", &manifest)?;
    write_file(
        workspace.path(),
        "notes.md",
        "dense generation evidence for restart recovery",
    )?;
    assert_index_ok(
        &instance_path,
        &workspace.path().join("notes.md").to_string_lossy(),
    )?;

    let generations_before = assert_ok(&["index", "generations", "-i", &instance_path])?;
    assert!(generations_before.contains("name=lexical_text_v1"));
    assert!(generations_before.contains("name=dense_text_v1"));
    assert!(generations_before.matches("lifecycle=Active").count() >= 2);
    let dense_id = generations_before
        .lines()
        .find(|line| line.contains("name=dense_text_v1"))
        .and_then(|line| {
            line.split_whitespace()
                .find_map(|field| field.strip_prefix("generation="))
        })
        .ok_or("dense generation id missing")?
        .to_string();

    let search = assert_ok(&["search", "-i", &instance_path, "dense generation"])?;
    assert!(search.contains("rank=") || search.contains("search_status=NoEvidenceFound"));
    let explained = assert_ok(&[
        "search",
        "explain",
        "-i",
        &instance_path,
        "dense generation",
    ])?;
    assert!(
        explained.contains("retrieval_mode=hybrid-shadow"),
        "{explained}"
    );
    assert!(explained.contains("dense_chunks"), "{explained}");
    assert!(
        explained.contains("retriever_generations=[Some("),
        "{explained}"
    );
    let projection = instance.path().join("indexes/vector/projection.db");
    std::fs::remove_file(&projection)?;
    let rebuilt = assert_ok(&["search", "-i", &instance_path, "dense generation"])?;
    assert!(rebuilt.contains("rank=") || rebuilt.contains("search_status=NoEvidenceFound"));
    let generations_after = assert_ok(&["index", "generations", "-i", &instance_path])?;
    assert!(generations_after.contains(&format!("generation={dense_id}")));
    assert!(generations_after.contains("name=dense_text_v1"));

    drop(server);
    let fallback = assert_ok(&["search", "-i", &instance_path, "dense generation"])?;
    assert!(fallback.contains("rank=") || fallback.contains("search_status=NoEvidenceFound"));
    let fallback_explained = assert_ok(&[
        "search",
        "explain",
        "-i",
        &instance_path,
        "dense generation",
    ])?;
    assert!(
        fallback_explained.contains("retrieval_mode=lexical-only"),
        "{fallback_explained}"
    );
    assert!(
        fallback_explained.contains("dense_chunks"),
        "{fallback_explained}"
    );
    Ok(())
}
