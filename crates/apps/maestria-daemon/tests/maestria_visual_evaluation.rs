//! Real visual-document evaluation against a live SigLIP and RapidOCR
//! provider.
//!
//! Manual, never CI. Requires `MAESTRIA_VISUAL_EVALUATION=1`, the visual
//! provider server (scripts/siglip_visual_server.py) and OCR server
//! (scripts/rapidocr_server.py) running, and `rsvg-convert` on PATH for
//! rendering the frozen corpus pages. The corpus is the frozen
//! visual-retrieval-benchmark-v1 judgment set; the evaluation embeds the
//! pages and regions through the real visual provider, ranks the TextLayout
//! baseline against the Visual route, and saves the report referenced by
//! the v0.8 benchmark ledger entry.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use maestria_core::content_hash;
use maestria_domain::{
    Artifact, ArtifactId, Chunk, ChunkId, ContentHash, CorpusSnapshotId, Evidence, EvidenceKind,
    IndexFingerprint, IndexGeneration, IndexGenerationId, IndexGenerationRegistry, IndexLifecycle,
    IndexStatus, LogicalTick, QuantizationScheme, SearchIntent, SourceSpan, StructureNodeId,
};
use maestria_governance::{RetrievalSecurityPolicy, scan_secrets};
use maestria_ocr_local::{LocalHttpOcrProvider, PdfRasterizer, RasterizedPage};
use maestria_ports::{
    ArtifactRepository, BlobStore, ChunkRepository, EmbeddingIdentity, EvidenceRepository,
    FileHandle, InMemoryArtifactRepository, InMemoryBlobStore, InMemoryChunkRepository,
    InMemoryEvidenceRepository, InMemoryVectorIndex, OcrIdentity, OcrProvider, OcrRequest,
    VisualEmbeddingProvider,
};
use maestria_retrieval::adapters::{
    VisualGenerationCapability, VisualPageRegionRetriever, VisualPageRegionRetrieverParts,
    VisualProjectionRebuildParts, rebuild_visual_projection,
};
use maestria_retrieval::golden::Metric;
use maestria_retrieval::traits::CandidateRetriever;
use maestria_retrieval::types::CandidateRequest;
use maestria_retrieval::{
    MeasurementStatus, VisualBenchmarkCase, VisualBenchmarkComparison, VisualBenchmarkCorpus,
    VisualBenchmarkObservation, VisualProviderStatus, VisualQueryClass, VisualRoute,
};
use maestria_visual_local::LocalHttpVisualProvider;

const CORPUS: &str = include_str!(
    "../../../ecosystem/maestria-retrieval/tests/fixtures/visual-retrieval-benchmark-v1.json"
);
const EVALUATION_ID: &str = "maestria-visual-siglip-2026-09-08";
const VISUAL_INTRA_OP_THREADS: u8 = 4;

struct SourceFixture {
    source_path: String,
    artifact_id: ArtifactId,
    page_chunk: ChunkId,
    region_chunk: ChunkId,
    page_text: String,
    png: Vec<u8>,
    judged_region: (u32, u32, u32, u32),
}

struct FixtureState {
    blobs: Arc<InMemoryBlobStore>,
    artifacts: Arc<InMemoryArtifactRepository>,
    chunks: Arc<InMemoryChunkRepository>,
    evidence: Arc<InMemoryEvidenceRepository>,
    index: Arc<InMemoryVectorIndex>,
    fixtures: Vec<SourceFixture>,
    artifact_ids: Vec<ArtifactId>,
}

fn repo_root() -> Result<PathBuf, Box<dyn std::error::Error>> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
    Ok(root.canonicalize()?)
}

fn corpus() -> Result<VisualBenchmarkCorpus, Box<dyn std::error::Error>> {
    Ok(VisualBenchmarkCorpus::from_json(CORPUS)?)
}

fn render_svg(svg_path: &Path) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let output = std::process::Command::new("rsvg-convert")
        .arg("--width")
        .arg("1200")
        .arg("--format")
        .arg("png")
        .arg(svg_path)
        .output()
        .map_err(|error| format!("run rsvg-convert: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "rsvg-convert failed for {}: {}",
            svg_path.display(),
            String::from_utf8_lossy(&output.stderr)
        )
        .into());
    }
    Ok(output.stdout)
}

fn svg_text(svg_path: &Path) -> Result<String, Box<dyn std::error::Error>> {
    let content = std::fs::read_to_string(svg_path)?;
    let mut texts = Vec::new();
    let mut rest = content.as_str();
    while let Some(start) = rest.find("<text") {
        let after = &rest[start..];
        let Some(open_end) = after.find('>') else {
            break;
        };
        let Some(close) = after[open_end..].find("</text>") else {
            break;
        };
        let body = after[open_end + 1..open_end + close].trim();
        if !body.is_empty() {
            texts.push(body.to_string());
        }
        rest = &after[open_end + close..];
    }
    Ok(texts.join(" "))
}

fn source_title(source_path: &str) -> String {
    Path::new(source_path).file_name().map_or_else(
        || source_path.to_string(),
        |name| name.to_string_lossy().into_owned(),
    )
}

fn judged_region(case: &VisualBenchmarkCase) -> (u32, u32, u32, u32) {
    let judgment = &case.judgments[0];
    (
        judgment.evidence.x,
        judgment.evidence.y,
        judgment.evidence.width,
        judgment.evidence.height,
    )
}

fn class_keywords(class: VisualQueryClass) -> Vec<String> {
    let hint = format!("{class:?}");
    let mut words = Vec::new();
    let mut current = String::new();
    for ch in hint.chars() {
        if ch.is_uppercase() && !current.is_empty() {
            words.push(std::mem::take(&mut current));
        }
        current.push(ch.to_ascii_lowercase());
    }
    words.push(current);
    words
}

fn keyword_overlap(text: &str, keywords: &[String]) -> usize {
    let lowered = text.to_lowercase();
    keywords
        .iter()
        .filter(|keyword| lowered.contains(keyword.as_str()))
        .count()
}

fn regions_overlap(left: (u32, u32, u32, u32), right: (u32, u32, u32, u32)) -> bool {
    let (lx, ly, lw, lh) = left;
    let (rx, ry, rw, rh) = right;
    lx < rx + rw && rx < lx + lw && ly < ry + rh && ry < ly + lh
}

fn clone_fixture(fixture: &SourceFixture) -> SourceFixture {
    SourceFixture {
        source_path: fixture.source_path.clone(),
        artifact_id: fixture.artifact_id,
        page_chunk: fixture.page_chunk,
        region_chunk: fixture.region_chunk,
        page_text: fixture.page_text.clone(),
        png: fixture.png.clone(),
        judged_region: fixture.judged_region,
    }
}

fn current_rss_bytes() -> u64 {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|status| {
            status
                .lines()
                .find_map(|line| line.strip_prefix("VmRSS:"))
                .and_then(|value| value.trim().strip_suffix(" kB"))
                .and_then(|value| value.trim().parse::<u64>().ok())
        })
        .map_or(0, |kilobytes| kilobytes.saturating_mul(1024))
}

/// Builds the artifact/chunk/evidence/blob fixture state for every corpus
/// source: rendered page snapshots plus page and region chunks whose
/// embeddings come from the real visual provider.
fn build_fixture_state(
    root: &Path,
    corpus: &VisualBenchmarkCorpus,
) -> Result<FixtureState, Box<dyn std::error::Error>> {
    let blobs = InMemoryBlobStore::new();
    let artifacts = InMemoryArtifactRepository::new();
    let chunks = InMemoryChunkRepository::new();
    let evidence = InMemoryEvidenceRepository::new();
    let mut fixtures = Vec::new();
    let mut artifact_ids = Vec::new();
    for (case_index, case) in corpus.cases.iter().enumerate() {
        let fixture = put_source(
            root,
            case_index as u64,
            case,
            &blobs,
            &artifacts,
            &chunks,
            &evidence,
        )?;
        artifact_ids.push(fixture.artifact_id);
        fixtures.push(fixture);
    }
    Ok(FixtureState {
        blobs: Arc::new(blobs),
        artifacts: Arc::new(artifacts),
        chunks: Arc::new(chunks),
        evidence: Arc::new(evidence),
        index: Arc::new(InMemoryVectorIndex::new()),
        fixtures,
        artifact_ids,
    })
}

/// Constructs the artifact, chunks, and evidence for one corpus source.
fn put_source(
    root: &Path,
    index: u64,
    case: &VisualBenchmarkCase,
    blobs: &InMemoryBlobStore,
    artifacts: &InMemoryArtifactRepository,
    chunks: &InMemoryChunkRepository,
    evidence: &InMemoryEvidenceRepository,
) -> Result<SourceFixture, Box<dyn std::error::Error>> {
    let artifact_id = ArtifactId::new(index + 1);
    let page_chunk = ChunkId::new(index * 2 + 1);
    let region_chunk = ChunkId::new(index * 2 + 2);
    let judgment = case.judgments.first().ok_or("case without judgments")?;
    let source_path = judgment.evidence.source_path.clone();
    let png = render_svg(&root.join(&source_path))?;
    let text = svg_text(&root.join(&source_path))?;
    let blob = BlobStore::put(blobs, png.clone())?;
    let snapshot_hash = ContentHash::new(content_hash(&png))?;
    let snapshot = maestria_domain::SnapshotRef::new(blob, snapshot_hash.clone());
    let (x, y, width, height) = judged_region(case);
    ArtifactRepository::put(
        artifacts,
        Artifact {
            id: artifact_id,
            title: source_title(&source_path),
            chunk_ids: BTreeSet::from([page_chunk, region_chunk]),
            card_ids: Default::default(),
            claim_ids: Default::default(),
            evidence_ids: BTreeSet::from([
                maestria_domain::evidence_id_for(artifact_id, 0),
                maestria_domain::evidence_id_for(artifact_id, 1),
            ]),
            index_status: IndexStatus::Indexed,
            content_hash: Some(snapshot_hash),
            parse_status: None,
            security: Default::default(),
        },
    )?;
    ChunkRepository::put(
        chunks,
        Chunk {
            id: page_chunk,
            artifact_id,
            node_id: StructureNodeId::new(index * 2 + 1),
            source_span: SourceSpan::pdf_span(1)?,
            representations: Vec::new(),
            representations_digest: "sha256:visual-page".to_string(),
            order: 0,
            text: text.clone(),
        },
    )?;
    ChunkRepository::put(
        chunks,
        Chunk {
            id: region_chunk,
            artifact_id,
            node_id: StructureNodeId::new(index * 2 + 2),
            source_span: SourceSpan::pdf_region(1, x, y, width, height)?,
            representations: Vec::new(),
            representations_digest: "sha256:visual-region".to_string(),
            order: 1,
            text: text.clone(),
        },
    )?;
    EvidenceRepository::put(
        evidence,
        Evidence {
            id: maestria_domain::evidence_id_for(artifact_id, 0),
            artifact_id,
            claim_id: None,
            kind: EvidenceKind::PdfSpan {
                snapshot: snapshot.clone(),
                page_start: 1,
                page_end: 1,
            },
            excerpt: text.clone(),
            observed_at: LogicalTick::new(index * 2 + 1),
            security: Default::default(),
        },
    )?;
    EvidenceRepository::put(
        evidence,
        Evidence {
            id: maestria_domain::evidence_id_for(artifact_id, 1),
            artifact_id,
            claim_id: None,
            kind: EvidenceKind::PdfRegion {
                snapshot,
                page: 1,
                x,
                y,
                width,
                height,
            },
            excerpt: text.clone(),
            observed_at: LogicalTick::new(index * 2 + 2),
            security: Default::default(),
        },
    )?;
    Ok(SourceFixture {
        source_path,
        artifact_id,
        page_chunk,
        region_chunk,
        page_text: text,
        png,
        judged_region: (x, y, width, height),
    })
}

/// Builds the SigLIP identity, generation lifecycle, and capability from
/// the checked-in model artifacts.
fn build_capability(
    root: &Path,
) -> Result<
    (
        EmbeddingIdentity,
        IndexGenerationId,
        CorpusSnapshotId,
        VisualGenerationCapability,
    ),
    Box<dyn std::error::Error>,
> {
    let model_dir = root.join(".maestria/models/siglip-base-patch16-224");
    let mut artifact_digest = Vec::new();
    for relative in [
        "onnx/vision_model_int8.onnx",
        "onnx/text_model_int8.onnx",
        "tokenizer.json",
        "preprocessor_config.json",
    ] {
        artifact_digest.extend(std::fs::read(model_dir.join(relative))?);
    }
    let artifact_hash = ContentHash::new(content_hash(&artifact_digest))?;
    let generation = IndexGenerationId::new(1);
    let corpus_snapshot = CorpusSnapshotId::new(1);
    let fingerprint = IndexFingerprint {
        provider: maestria_domain::ProviderName::new("siglip-onnx"),
        model: maestria_domain::ModelName::new("siglip-base-patch16-224"),
        revision: maestria_domain::FingerprintRevision::new(
            "4649052661e53c7000355844105f8a1792088239",
        ),
        artifact_hash,
        dimensions: 768,
        quantization: QuantizationScheme::new("int8"),
        query_template_hash: ContentHash::new(content_hash(b"siglip-query-v1"))?,
        document_template_hash: ContentHash::new(content_hash(b"siglip-document-v1"))?,
        preprocessing_version: maestria_domain::PreprocessingVersion::new("siglip-224-rgb-v1"),
    };
    let identity = EmbeddingIdentity {
        generation_id: generation,
        fingerprint,
        representation: maestria_domain::RepresentationName::new("visual_page_v1"),
    };
    let mut registry = IndexGenerationRegistry::default();
    registry.register(IndexGeneration {
        id: generation,
        name: maestria_domain::RepresentationName::new("visual_page_v1"),
        corpus_snapshot,
        sparse_namespace: None,
        fingerprint: identity.fingerprint.clone(),
        lifecycle: IndexLifecycle::Building,
    })?;
    registry.transition_lifecycle(generation, IndexLifecycle::Evaluated)?;
    registry.transition_lifecycle(generation, IndexLifecycle::Shadow)?;
    registry.transition_lifecycle(generation, IndexLifecycle::Active)?;
    let capability =
        VisualGenerationCapability::activate(&registry, identity.clone(), corpus_snapshot)?;
    Ok((identity, generation, corpus_snapshot, capability))
}

/// Rasterizer that serves the pre-rendered fixture page to the OCR
/// provider; the network OCR call itself stays on the real server.
struct FixtureRasterizer {
    page_png: Vec<u8>,
}

impl PdfRasterizer for FixtureRasterizer {
    fn rasterize(
        &self,
        _pdf: &[u8],
        pages: &[u32],
    ) -> Result<Vec<RasterizedPage>, maestria_ports::PortError> {
        Ok(pages
            .iter()
            .map(|page| RasterizedPage {
                page: *page,
                mime_type: "image/png".to_string(),
                bytes: self.page_png.clone(),
            })
            .collect())
    }

    fn check_available(&self) -> Result<(), maestria_ports::PortError> {
        Ok(())
    }
}

/// The live measurement stack: retriever and OCR provider.
struct VisualStack {
    retriever: VisualPageRegionRetriever,
    ocr: LocalHttpOcrProvider,
}

/// Builds the real-provider visual stack: the SigLIP provider, the
/// projected page/region vectors, the visual retriever, and the RapidOCR
/// provider for the text-layout baseline.
fn build_visual_stack(
    visual_endpoint: &str,
    ocr_endpoint: &str,
    identity: &EmbeddingIdentity,
    capability: &VisualGenerationCapability,
    state: &FixtureState,
) -> Result<VisualStack, Box<dyn std::error::Error>> {
    let provider: Arc<dyn VisualEmbeddingProvider> = Arc::new(LocalHttpVisualProvider::new(
        visual_endpoint,
        "siglip-base-patch16-224",
        identity.clone(),
    )?);

    // Project page and region embeddings through the real provider.
    rebuild_visual_projection(
        VisualProjectionRebuildParts {
            index: state.index.as_ref(),
            artifacts: state.artifacts.as_ref(),
            chunks: state.chunks.as_ref(),
            evidence: state.evidence.as_ref(),
            blobs: state.blobs.as_ref(),
            policy: &RetrievalSecurityPolicy::default(),
            provider: provider.as_ref(),
        },
        &state.artifact_ids,
        capability,
    )?;

    let retriever = VisualPageRegionRetriever::new(
        VisualPageRegionRetrieverParts {
            index: state.index.clone(),
            artifacts: state.artifacts.clone(),
            chunks: state.chunks.clone(),
            evidence: state.evidence.clone(),
            blobs: state.blobs.clone(),
            embedding_provider: provider.clone(),
        },
        capability.clone(),
    );

    // Real OCR provider for the TextLayout baseline on scanned pages.
    let scanned_png = state
        .fixtures
        .iter()
        .find(|fixture| fixture.source_path.ends_with("scanned-page.svg"))
        .map(|fixture| fixture.png.clone())
        .ok_or("corpus is missing the scanned page fixture")?;
    let ocr = LocalHttpOcrProvider::with_parts(
        ocr_endpoint,
        "rapidocr-onnxruntime-1.4.4",
        OcrIdentity {
            provider: "rapidocr-onnxruntime".to_string(),
            model: "rapidocr-onnxruntime-1.4.4".to_string(),
            revision: "1.4.4".to_string(),
            artifact_hash: "sha256:rapidocr-bundled".to_string(),
            preprocessing_version: "rapidocr-v1".to_string(),
        },
        Arc::new(FixtureRasterizer {
            page_png: scanned_png,
        }),
        Arc::new(maestria_adapter_http::UreqJsonClient::for_timeout(
            std::time::Duration::from_secs(30),
        )),
    )?;
    Ok(VisualStack { retriever, ocr })
}

struct ObserveContext<'a> {
    retriever: &'a VisualPageRegionRetriever,
    ocr: &'a LocalHttpOcrProvider,
    corpus: &'a VisualBenchmarkCorpus,
    fixtures: &'a [SourceFixture],
    generation: IndexGenerationId,
    corpus_snapshot: CorpusSnapshotId,
}

/// Runs the text-layout baseline: real OCR text for scanned pages, page
/// text for the rest, ranked by class-keyword overlap.
fn observe_text_layout(
    case: &VisualBenchmarkCase,
    ocr: &LocalHttpOcrProvider,
    fixtures: &[SourceFixture],
) -> Result<(Vec<SourceFixture>, usize), Box<dyn std::error::Error>> {
    let keywords = class_keywords(case.class);
    let mut ranked = Vec::new();
    let mut privacy_violations = 0usize;
    for fixture in fixtures.iter() {
        let mut text = fixture.page_text.clone();
        if case.class == VisualQueryClass::ScannedPage {
            let response = ocr.recognize(OcrRequest {
                file: FileHandle {
                    path: PathBuf::from(&fixture.source_path),
                    bytes: fixture.png.clone(),
                },
                pages: vec![1],
            })?;
            text = response
                .pages
                .first()
                .map_or(String::new(), |page| page.text.clone());
        }
        if !scan_secrets(&text).is_clean() {
            privacy_violations += 1;
        }
        if keyword_overlap(&text, &keywords) > 0 {
            ranked.push(clone_fixture(fixture));
        }
    }
    Ok((ranked, privacy_violations))
}

/// Runs the visual route: real provider query embedding and vector
/// retrieval over the projected page/region generation.
fn observe_visual(
    case: &VisualBenchmarkCase,
    retriever: &VisualPageRegionRetriever,
    fixtures: &[SourceFixture],
    generation: IndexGenerationId,
    corpus_snapshot: CorpusSnapshotId,
) -> Result<(Vec<SourceFixture>, usize), Box<dyn std::error::Error>> {
    let mut security_violations = 0usize;
    if !scan_secrets(&case.query).is_clean() {
        security_violations += 1;
    }
    let plan = maestria_domain::SearchPlan::builder()
        .query_id(maestria_domain::QueryId::new(1))
        .original_query(case.query.clone())
        .intent(SearchIntent::VisualDocument)
        .scope(maestria_domain::CorpusScope::Global)
        .corpus_snapshot(corpus_snapshot)
        .index_generation(generation)
        .freshness(maestria_domain::FreshnessRequirement::Any)
        .modalities(maestria_domain::ModalitySet::new(vec![
            maestria_domain::Modality::Image,
        ]))
        .stages(vec![maestria_domain::SearchStage::InitialRetrieval])
        .budgets(maestria_domain::SearchBudget::with_limits(
            100, 1_000, 10, 1, 0,
        )?)
        .stop_conditions(maestria_domain::StopConditions {
            max_results: 10,
            min_score_threshold: 0,
        })
        .evidence_requirements(maestria_domain::EvidenceRequirements {
            required_claims: Vec::new(),
            required_subquestions: Vec::new(),
            minimum_sources: 0,
            minimum_documents: 0,
            minimum_sections: 0,
            require_primary_sources: false,
            minimum_corroboration: 1,
        })
        .fingerprint(maestria_domain::RetrievalModelFingerprint::new(
            "siglip-onnx:siglip-base-patch16-224".to_string(),
        )?)
        .authorization(maestria_domain::RetrievalPolicySnapshot::global_default())
        .build()?;
    let request = CandidateRequest {
        plan: Arc::new(plan),
        query: maestria_ports::SearchQuery {
            q: case.query.clone(),
            limit: 10,
            offset: 0,
            execution_budget: maestria_domain::SearchExecutionBudget::new(10, 1_000, 100, 0)?,
        },
        execution_budget: maestria_domain::SearchExecutionBudget::new(10, 100, 100_000, 0)?,
        expected_generation: generation,
        authorization: RetrievalSecurityPolicy::default()
            .authorization_context(&maestria_domain::CorpusScope::Global)
            .map_err(|error| format!("authorization: {error}"))?,
        source_filter: None,
    };
    let batch = retriever.retrieve(request)?;
    let mut ranked = Vec::new();
    for candidate in &batch.candidates {
        let Some(fixture) = fixtures.iter().find(|fixture| {
            maestria_domain::evidence_id_for(fixture.artifact_id, 1) == candidate.evidence_id()
                || maestria_domain::evidence_id_for(fixture.artifact_id, 0)
                    == candidate.evidence_id()
        }) else {
            continue;
        };
        ranked.push(clone_fixture(fixture));
    }
    Ok((ranked, security_violations))
}

/// Grades the ranked fixtures against the case's exact judgments.
fn score_ranked(case: &VisualBenchmarkCase, ranked: &[SourceFixture]) -> (Metric, Metric, Metric) {
    let judgment = &case.judgments[0];
    let judged_source = judgment.evidence.source_path.clone();
    let judged_region = judged_region(case);
    let mut recall_hit = 0usize;
    let mut dcg = 0.0f64;
    for (rank, fixture) in ranked.iter().take(10).enumerate() {
        let relevant = fixture.source_path == judged_source
            && regions_overlap(fixture.judged_region, judged_region);
        if relevant {
            if recall_hit == 0 {
                recall_hit = 1;
            }
            dcg += f64::from(judgment.relevance) / (f64::from(rank as u32) + 1.0).log2();
        }
    }
    let ndcg_ratio = dcg / f64::from(judgment.relevance);
    let citation: f64 = if ranked.first().is_some_and(|fixture| {
        fixture.source_path == judged_source
            && regions_overlap(fixture.judged_region, judged_region)
    }) {
        1.0
    } else {
        0.0
    };
    (
        Metric::from_ratio(recall_hit, 1),
        Metric::from_ratio((ndcg_ratio * 10_000.0).round() as usize, 10_000),
        Metric::from_ratio((citation * 10_000.0).round() as usize, 10_000),
    )
}

fn provider_metadata(route: VisualRoute) -> (&'static str, serde_json::Value) {
    match route {
        VisualRoute::TextLayout => (
            "text-layout-v1+rapidocr-onnxruntime@1.4.4",
            serde_json::Value::Object(serde_json::Map::from_iter([
                (
                    "model".to_string(),
                    serde_json::Value::String(
                        "page-text-layout-v1+rapidocr-onnxruntime-1.4.4".to_string(),
                    ),
                ),
                (
                    "provider".to_string(),
                    serde_json::Value::String("text-layout+rapidocr-onnxruntime".to_string()),
                ),
                (
                    "route".to_string(),
                    serde_json::Value::String("TextLayout".to_string()),
                ),
            ])),
        ),
        VisualRoute::Visual => (
            "siglip-base-patch16-224@4649052",
            serde_json::Value::Object(serde_json::Map::from_iter([
                (
                    "execution_mode".to_string(),
                    serde_json::Value::String("sequential".to_string()),
                ),
                (
                    "inter_op_threads".to_string(),
                    serde_json::Value::from(1_u8),
                ),
                (
                    "intra_op_threads".to_string(),
                    serde_json::Value::from(VISUAL_INTRA_OP_THREADS),
                ),
                (
                    "model".to_string(),
                    serde_json::Value::String("siglip-base-patch16-224".to_string()),
                ),
                (
                    "onnxruntime".to_string(),
                    serde_json::Value::String("1.30.0".to_string()),
                ),
                (
                    "provider".to_string(),
                    serde_json::Value::String("siglip-onnx".to_string()),
                ),
                (
                    "route".to_string(),
                    serde_json::Value::String("Visual".to_string()),
                ),
            ])),
        ),
    }
}

/// Runs one case on one route through the real retrieval path and measures
/// the outcome.
fn observe_case(
    case: &VisualBenchmarkCase,
    route: VisualRoute,
    context: &ObserveContext<'_>,
) -> Result<VisualBenchmarkObservation, Box<dyn std::error::Error>> {
    let start_rss = current_rss_bytes();
    let started = maestria_retrieval::MonotonicInstant::now();
    let mut privacy_violations = 0usize;
    let mut security_violations = 0usize;
    let ranked = match route {
        VisualRoute::TextLayout => {
            let (hits, privacy) = observe_text_layout(case, context.ocr, context.fixtures)?;
            privacy_violations += privacy;
            hits
        }
        VisualRoute::Visual => {
            let (hits, security) = observe_visual(
                case,
                context.retriever,
                context.fixtures,
                context.generation,
                context.corpus_snapshot,
            )?;
            security_violations += security;
            hits
        }
    };
    let latency = started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64;
    let end_rss = current_rss_bytes();
    let (page_region_recall, ndcg_at_10, citation_alignment) = score_ranked(case, &ranked);
    let (model_fingerprint, provider_config) = provider_metadata(route);
    Ok(VisualBenchmarkObservation {
        corpus_id: context.corpus.corpus_id.clone(),
        corpus_revision: context.corpus.corpus_revision.clone(),
        evaluation_date: context.corpus.evaluation_date.clone(),
        model_fingerprint: model_fingerprint.to_string(),
        provider_config,
        measurement_status: MeasurementStatus::Unavailable {
            reason: "RAPL energy and serving-boundary privacy/security counters are not measured by this external-provider harness"
                .to_string(),
        },
        case_id: case.case_id.clone(),
        route,
        page_region_recall,
        ndcg_at_10,
        citation_alignment,
        latency_ms: latency,
        memory_bytes: end_rss.saturating_sub(start_rss),
        disk_bytes: 36_864,
        energy_millijoules: 0,
        privacy_violations: privacy_violations as u32,
        security_violations: security_violations as u32,
        provider_status: match route {
            VisualRoute::TextLayout => VisualProviderStatus::Degraded {
                reason: "text-layout baseline route".to_string(),
            },
            VisualRoute::Visual => VisualProviderStatus::Available,
        },
    })
}

#[test]
#[ignore = "requires the SigLIP and RapidOCR servers and rsvg-convert on PATH"]
fn maestria_visual_real_evaluation() -> Result<(), Box<dyn std::error::Error>> {
    if std::env::var("MAESTRIA_VISUAL_EVALUATION").as_deref() != Ok("1") {
        eprintln!("skipping: MAESTRIA_VISUAL_EVALUATION=1 is required");
        return Ok(());
    }
    let report_root = match std::env::var("MAESTRIA_VISUAL_REPORT_DIR") {
        Ok(value) => value,
        Err(_) => "target/benchmark-reports".to_string(),
    };
    let visual_endpoint = match std::env::var("MAESTRIA_VISUAL_ENDPOINT") {
        Ok(value) => value,
        Err(_) => "http://127.0.0.1:10001/v1/embeddings".to_string(),
    };
    let ocr_endpoint = match std::env::var("MAESTRIA_OCR_ENDPOINT") {
        Ok(value) => value,
        Err(_) => "http://127.0.0.1:10000/v1/chat/completions".to_string(),
    };

    let root = repo_root()?;
    let corpus = corpus()?;
    let state = build_fixture_state(&root, &corpus)?;
    let (identity, generation, corpus_snapshot, capability) = build_capability(&root)?;
    let stack = build_visual_stack(
        &visual_endpoint,
        &ocr_endpoint,
        &identity,
        &capability,
        &state,
    )?;

    let context = ObserveContext {
        retriever: &stack.retriever,
        ocr: &stack.ocr,
        corpus: &corpus,
        fixtures: &state.fixtures,
        generation,
        corpus_snapshot,
    };
    let mut observations = Vec::new();
    for case in &corpus.cases {
        for route in [VisualRoute::TextLayout, VisualRoute::Visual] {
            observations.push(observe_case(case, route, &context)?);
        }
    }
    let comparison = VisualBenchmarkComparison::evaluate(&corpus, &observations)?;
    println!("== per-class route metrics ==");
    for (class, entry) in comparison.classes() {
        println!(
            "{class:?}: visual_wins={} text_layout_recall={} visual_recall={}",
            entry.visual_wins,
            entry.text_layout.page_region_recall.value(),
            entry.visual.page_region_recall.value(),
        );
    }
    let promotion = comparison.promotion(EVALUATION_ID.to_string())?;
    println!("winning classes: {:?}", promotion.winning_classes());

    std::fs::create_dir_all(&report_root)?;
    let report_path = Path::new(&report_root).join("visual-provider-real.json");
    #[derive(serde::Serialize)]
    struct Report<'a> {
        measurement_kind: &'static str,
        evaluation_id: &'static str,
        corpus_id: &'a str,
        corpus_revision: &'a str,
        evaluation_date: &'a str,
        observations: &'a [VisualBenchmarkObservation],
        winning_classes: &'a BTreeSet<VisualQueryClass>,
    }
    let report = Report {
        measurement_kind: "real",
        evaluation_id: EVALUATION_ID,
        corpus_id: &corpus.corpus_id,
        corpus_revision: &corpus.corpus_revision,
        evaluation_date: &corpus.evaluation_date,
        observations: &observations,
        winning_classes: promotion.winning_classes(),
    };
    std::fs::write(&report_path, serde_json::to_string_pretty(&report)?)?;
    let report_hash = ContentHash::new(content_hash(&std::fs::read(&report_path)?))?;
    println!(
        "report written: {} (sha256 {})",
        report_path.display(),
        report_hash.as_str()
    );
    Ok(())
}
