use super::*;

fn pdf_with_text(text: &[u8]) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    use lopdf::{
        Object, Stream,
        content::{Content, Operation},
        dictionary,
    };

    let mut document = lopdf::Document::with_version("1.4");
    let pages_id = document.new_object_id();
    let font_id = document.add_object(dictionary! {
        "Type" => "Font",
        "Subtype" => "Type1",
        "BaseFont" => "Courier",
    });
    let contents = Content {
        operations: vec![
            Operation::new("BT", vec![]),
            Operation::new("Tf", vec!["F1".into(), 12.into()]),
            Operation::new("Td", vec![72.into(), 700.into()]),
            Operation::new("Tj", vec![Object::string_literal(text)]),
            Operation::new("ET", vec![]),
        ],
    };
    let content_id = document.add_object(Stream::new(dictionary! {}, contents.encode()?));
    let page_id = document.add_object(dictionary! {
        "Type" => "Page",
        "Parent" => pages_id,
        "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
        "Contents" => content_id,
        "Resources" => dictionary! {
            "Font" => dictionary! { "F1" => font_id },
        },
    });
    document.objects.insert(
        pages_id,
        Object::Dictionary(dictionary! {
            "Type" => "Pages",
            "Kids" => vec![page_id.into()],
            "Count" => 1,
        }),
    );
    let catalog_id = document.add_object(dictionary! {
        "Type" => "Catalog",
        "Pages" => pages_id,
    });
    document.trailer.set("Root", catalog_id);
    let mut bytes = Vec::new();
    document.save_to(&mut bytes)?;
    Ok(bytes)
}

async fn indexed_two_root_provider(
    tmp: &TempDir,
) -> Result<
    (
        InstanceLayout,
        PathBuf,
        PathBuf,
        EvidenceId,
        EvidenceId,
        EvidenceId,
    ),
    Box<dyn std::error::Error>,
> {
    let instance = tmp.path().join("scoped-provider");
    let allowed_root = instance.join("allowed");
    let denied_root = instance.join("allowed-other");
    fs::create_dir_all(&allowed_root)?;
    fs::create_dir_all(&denied_root)?;
    let plan = InstanceService::init_instance_with_roots(
        instance,
        vec![allowed_root.clone(), denied_root.clone()],
        sillage_test_support::realm_id(21)?,
    )?;
    for directory in &plan.directories {
        fs::create_dir_all(directory)?;
    }
    fs::write(&plan.manifest_path, plan.manifest_contents.as_bytes())?;

    let allowed_path = allowed_root.join("a-visible-filename.md");
    let denied_path = denied_root.join("b-secret-filename.md");
    let allowed_bytes = b"# Approved\nThe orchid passage belongs to the approved root.\n";
    let denied_bytes = b"# Private\nThe orchid basalt passage belongs to another root.\n";
    let denied_pdf_bytes = pdf_with_text(b"Private pdfquartz passage on page one.")?;
    let denied_pdf_path = denied_root.join("b-secret-report.pdf");
    fs::write(&allowed_path, allowed_bytes)?;
    fs::write(&denied_path, denied_bytes)?;
    fs::write(&denied_pdf_path, &denied_pdf_bytes)?;
    let session = sillage_daemon::MutationSession::start(
        plan.layout.clone(),
        AutonomyProfile::StrictResearch,
    )
    .await?;
    let indexed_allowed = index_and_verify_artifact(
        &session,
        &plan.layout.database_path,
        ArtifactId::new(1),
        &content_hash(allowed_bytes),
        allowed_bytes,
        allowed_path.to_str().ok_or("invalid allowed path")?,
    )
    .await?;
    let indexed_denied = index_and_verify_artifact(
        &session,
        &plan.layout.database_path,
        ArtifactId::new(2),
        &content_hash(denied_bytes),
        denied_bytes,
        denied_path.to_str().ok_or("invalid denied path")?,
    )
    .await?;
    let indexed_denied_pdf = index_and_verify_artifact(
        &session,
        &plan.layout.database_path,
        ArtifactId::new(3),
        &content_hash(&denied_pdf_bytes),
        &denied_pdf_bytes,
        denied_pdf_path.to_str().ok_or("invalid private PDF path")?,
    )
    .await?;
    finish_session(Some(session), Ok::<_, Box<dyn std::error::Error>>(())).await?;
    Ok((
        plan.layout,
        allowed_root,
        denied_root,
        indexed_allowed.evidence_id,
        indexed_denied.evidence_id,
        indexed_denied_pdf.evidence_id,
    ))
}

async fn search_as_consumer(
    client: &sillage_daemon::SearchApiClient,
    query: &str,
) -> Result<sillage_daemon::SearchResponse, Box<dyn std::error::Error>> {
    let result = client
        .request(sillage_daemon::SearchApiOperation::InteractiveSearch {
            query: query.to_string(),
            limit: 5,
        })
        .await?;
    let sillage_daemon::SearchApiResponse::Search(response) = result else {
        return Err("unexpected search API response".into());
    };
    Ok(response)
}
async fn source_revision_as_consumer(
    client: &sillage_daemon::SearchApiClient,
) -> Result<i64, Box<dyn std::error::Error>> {
    let response = client
        .request(sillage_daemon::SearchApiOperation::SourceRevision)
        .await?;
    let sillage_daemon::SearchApiResponse::SourceRevision { revision } = response else {
        return Err("unexpected source revision response".into());
    };
    Ok(revision)
}

async fn verify_source_revision_grant(
    layout: &InstanceLayout,
    consumer: &sillage_daemon::SearchApiClient,
) -> Result<(), Box<dyn std::error::Error>> {
    let revision = source_revision_as_consumer(consumer).await?;
    let source_revision =
        sillage_storage_sqlite::SqliteStore::open_read_only(&layout.database_path)?
            .searchable_source_revision()?;
    assert!(
        revision > source_revision,
        "indexed source publication must advance the consumer clock beyond tree capture"
    );
    let invalid_credential = sillage_daemon::SearchApiClient::consumer(
        layout.system_dir.join("daemon.sock"),
        sillage_test_support::realm_id(22)?,
        "f".repeat(64),
    )?;
    assert!(
        invalid_credential
            .request(sillage_daemon::SearchApiOperation::SourceRevision)
            .await
            .is_err(),
        "source revision must require the same valid search grant as indexing status"
    );
    search_as_consumer(consumer, "orchid").await?;
    assert_eq!(
        source_revision_as_consumer(consumer).await?,
        revision,
        "search audit events must not trigger a spurious UI refresh"
    );
    Ok(())
}

#[tokio::test]
async fn consumer_grant_isolates_roots_before_search_and_evidence_io_and_after_restart()
-> Result<(), Box<dyn std::error::Error>> {
    let tmp = TempDir::new()?;
    let (layout, allowed_root, denied_root, allowed_evidence, denied_evidence, denied_pdf_evidence) =
        indexed_two_root_provider(&tmp).await?;
    let shutdown = CancellationToken::new();
    let daemon = tokio::spawn(sillage_daemon::run_instance_with_shutdown(
        layout.root.clone(),
        shutdown.clone(),
        AutonomyProfile::ReadOnly,
    ));
    let initial = async {
        let (consumer, other_consumer) =
            create_scoped_consumers(&tmp, &layout, &allowed_root, &denied_root).await?;
        verify_other_consumer_pdf(&other_consumer, &denied_root, denied_pdf_evidence).await?;
        verify_scoped_search_and_evidence(
            &consumer,
            &other_consumer,
            &allowed_root,
            &denied_root,
            allowed_evidence,
            denied_evidence,
            denied_pdf_evidence,
        )
        .await?;
        verify_scoped_inventory(&consumer).await?;
        verify_source_revision_grant(&layout, &consumer).await?;

        Ok::<sillage_daemon::SearchApiClient, Box<dyn std::error::Error>>(consumer)
    }
    .await;
    shutdown.cancel();
    daemon.await??;
    let consumer = initial?;
    let restarted_shutdown = CancellationToken::new();
    let restarted_daemon = tokio::spawn(sillage_daemon::run_instance_with_shutdown(
        layout.root.clone(),
        restarted_shutdown.clone(),
        AutonomyProfile::ReadOnly,
    ));
    let result = async {
        let restarted_owner = wait_for_daemon_client(&layout).await?;
        assert_eq!(
            search_as_consumer(&consumer, "orchid")
                .await?
                .evidence
                .len(),
            1
        );
        assert!(
            search_as_consumer(&consumer, "basalt")
                .await?
                .evidence
                .is_empty()
        );
        restarted_owner
            .request(sillage_daemon::ClientOperation::SearchRootRemove {
                root: allowed_root.display().to_string(),
            })
            .await?;
        assert!(
            search_as_consumer(&consumer, "orchid")
                .await?
                .evidence
                .is_empty()
        );
        assert!(
            search_as_consumer(&consumer, "b-secret-filename")
                .await?
                .path_results
                .is_empty()
        );
        let status = consumer
            .request(sillage_daemon::SearchApiOperation::IndexingStatus)
            .await?;
        let sillage_daemon::SearchApiResponse::IndexingStatus(status) = status else {
            return Err::<(), Box<dyn std::error::Error>>("unexpected indexing status".into());
        };
        assert_eq!(status.approved_root_count, 0);
        assert!(
            denied_root.exists(),
            "the test must retain the disallowed root"
        );
        Ok::<(), Box<dyn std::error::Error>>(())
    }
    .await;
    restarted_shutdown.cancel();
    restarted_daemon.await??;
    result
}

async fn create_scoped_consumers(
    tmp: &TempDir,
    layout: &InstanceLayout,
    allowed_root: &Path,
    denied_root: &Path,
) -> Result<
    (
        sillage_daemon::SearchApiClient,
        sillage_daemon::SearchApiClient,
    ),
    Box<dyn std::error::Error>,
> {
    let owner = wait_for_daemon_client(layout).await?;
    let unapproved_root = tmp.path().join("not-approved");
    fs::create_dir_all(&unapproved_root)?;
    let invalid_grant = owner
        .request(sillage_daemon::ClientOperation::RealmGrantCreate {
            consumer_realm: sillage_test_support::realm_id(23)?,
            access: sillage_daemon::RealmGrantAccess::SearchOnly,
            max_sensitivity: sillage_daemon::RealmGrantSensitivity::Restricted,
            max_results: 5,
            max_evidence_bytes: 2048,
            expires_in_seconds: 86_400,
            allowed_roots: vec![unapproved_root.display().to_string()],
        })
        .await;
    assert!(
        invalid_grant.is_err(),
        "owner granted an unapproved source root"
    );
    let consumer_realm = sillage_test_support::realm_id(22)?;
    let response = owner
        .request(sillage_daemon::ClientOperation::RealmGrantCreate {
            consumer_realm: consumer_realm.clone(),
            access: sillage_daemon::RealmGrantAccess::SearchAndOpenEvidence,
            max_sensitivity: sillage_daemon::RealmGrantSensitivity::Restricted,
            max_results: 5,
            max_evidence_bytes: 2048,
            expires_in_seconds: 86_400,
            allowed_roots: vec![allowed_root.display().to_string()],
        })
        .await?;
    let sillage_daemon::ClientResponse::RealmGrantCreated(created) = response else {
        return Err("grant creation did not return a credential".into());
    };
    assert_eq!(
        created.grant.allowed_roots,
        Some(vec![allowed_root.display().to_string()])
    );
    let consumer = sillage_daemon::SearchApiClient::consumer(
        layout.system_dir.join("daemon.sock"),
        consumer_realm,
        created.credential.expose().to_string(),
    )?;
    let other_realm = sillage_test_support::realm_id(24)?;
    let other_grant = owner
        .request(sillage_daemon::ClientOperation::RealmGrantCreate {
            consumer_realm: other_realm.clone(),
            access: sillage_daemon::RealmGrantAccess::SearchAndOpenEvidence,
            max_sensitivity: sillage_daemon::RealmGrantSensitivity::Restricted,
            max_results: 5,
            max_evidence_bytes: 2048,
            expires_in_seconds: 86_400,
            allowed_roots: vec![denied_root.display().to_string()],
        })
        .await?;
    let sillage_daemon::ClientResponse::RealmGrantCreated(other_grant) = other_grant else {
        return Err("second root grant did not return a credential".into());
    };
    let other_consumer = sillage_daemon::SearchApiClient::consumer(
        layout.system_dir.join("daemon.sock"),
        other_realm,
        other_grant.credential.expose().to_string(),
    )?;
    Ok((consumer, other_consumer))
}

async fn verify_other_consumer_pdf(
    other_consumer: &sillage_daemon::SearchApiClient,
    denied_root: &Path,
    denied_pdf_evidence: EvidenceId,
) -> Result<(), Box<dyn std::error::Error>> {
    let opened_pdf = other_consumer
        .request(sillage_daemon::SearchApiOperation::Evidence {
            evidence_id: denied_pdf_evidence.value(),
        })
        .await?;
    let sillage_daemon::SearchApiResponse::Evidence(opened_pdf) = opened_pdf else {
        return Err("authorized PDF evidence did not open".into());
    };
    assert!(opened_pdf.excerpt.contains("pdfquartz"));
    assert!(matches!(
        opened_pdf.source,
        sillage_daemon::api::EvidenceSourceResponse::Pdf {
            page_start: 1,
            path: Some(path),
            ..
        } if Path::new(&path).starts_with(denied_root)
            && Path::new(&path).ends_with("b-secret-report.pdf")
    ));
    Ok(())
}

async fn verify_scoped_search_and_evidence(
    consumer: &sillage_daemon::SearchApiClient,
    other_consumer: &sillage_daemon::SearchApiClient,
    allowed_root: &Path,
    denied_root: &Path,
    allowed_evidence: EvidenceId,
    denied_evidence: EvidenceId,
    denied_pdf_evidence: EvidenceId,
) -> Result<(), Box<dyn std::error::Error>> {
    let shared = search_as_consumer(consumer, "orchid").await?;
    assert!(!shared.evidence.is_empty(), "approved passage is missing");
    assert!(shared.evidence.iter().all(|evidence| {
        matches!(
            evidence.preview.as_ref().map(|preview| &preview.location),
            Some(sillage_daemon::api::EvidenceSourceResponse::File { path, .. })
                if Path::new(path).starts_with(allowed_root)
        )
    }));
    let other_passage = search_as_consumer(other_consumer, "basalt").await?;
    assert_eq!(other_passage.evidence.len(), 1);
    assert!(matches!(
        other_passage.evidence[0].preview.as_ref().map(|preview| &preview.location),
        Some(sillage_daemon::api::EvidenceSourceResponse::File { path, .. })
            if Path::new(path).starts_with(denied_root)
    ));
    let private = search_as_consumer(consumer, "basalt").await?;
    assert!(private.evidence.is_empty());
    assert!(private.path_results.is_empty());
    assert!(
        search_as_consumer(consumer, "pdfquartz")
            .await?
            .evidence
            .is_empty(),
        "PDF from the other root appeared in A's passage search"
    );
    let normal_search = consumer
        .request(sillage_daemon::SearchApiOperation::Search {
            query: "basalt".to_string(),
            limit: 5,
        })
        .await?;
    let sillage_daemon::SearchApiResponse::Search(normal_search) = normal_search else {
        return Err("unexpected noninteractive search response".into());
    };
    assert!(normal_search.evidence.is_empty());
    let private_filename = search_as_consumer(consumer, "b-secret-filename").await?;
    assert!(private_filename.path_results.is_empty());
    let allowed_filename = search_as_consumer(consumer, "a-visible-filename").await?;
    assert_eq!(allowed_filename.path_results.len(), 1);
    assert!(Path::new(&allowed_filename.path_results[0].path).starts_with(allowed_root));
    for evidence_id in [denied_evidence, denied_pdf_evidence] {
        let denied = match consumer
            .request(sillage_daemon::SearchApiOperation::Evidence {
                evidence_id: evidence_id.value(),
            })
            .await
        {
            Ok(_) => return Err("a scoped grant opened evidence from another root".into()),
            Err(error) => error,
        };
        assert_eq!(
            denied.code,
            sillage_daemon::ClientErrorCode::SourceNotSelected
        );
    }
    assert!(matches!(
        consumer
            .request(sillage_daemon::SearchApiOperation::Evidence {
                evidence_id: allowed_evidence.value(),
            })
            .await?,
        sillage_daemon::SearchApiResponse::Evidence(_)
    ));
    Ok(())
}

async fn verify_scoped_inventory(
    consumer: &sillage_daemon::SearchApiClient,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut scoped_inventory = None;
    for _ in 0..30 {
        let result = consumer
            .request(sillage_daemon::SearchApiOperation::IndexingStatus)
            .await?;
        let sillage_daemon::SearchApiResponse::IndexingStatus(status) = result else {
            return Err("unexpected indexing status response".into());
        };
        if status.indexed_file_count == 1 && !status.scanning && status.pending_file_count == 0 {
            scoped_inventory = Some(status);
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let inventory = scoped_inventory.ok_or("scoped index did not settle")?;
    assert_eq!(inventory.approved_root_count, 1);
    assert_eq!(inventory.indexed_file_count, 1);
    Ok(())
}
async fn consumer_scan_time(
    consumer: &sillage_daemon::SearchApiClient,
) -> Result<u64, Box<dyn std::error::Error>> {
    let response = consumer
        .request(sillage_daemon::SearchApiOperation::IndexingStatus)
        .await?;
    let sillage_daemon::SearchApiResponse::IndexingStatus(status) = response else {
        return Err("unexpected indexing status response".into());
    };
    status
        .last_scan_unix_ms
        .ok_or_else(|| "watcher has not completed an indexing scan".into())
}

async fn wait_for_indexed_source_count(
    consumer: &sillage_daemon::SearchApiClient,
    expected_count: usize,
    after: Option<(u64, i64)>,
) -> Result<(), Box<dyn std::error::Error>> {
    for _ in 0..120 {
        let response = consumer
            .request(sillage_daemon::SearchApiOperation::IndexingStatus)
            .await?;
        let sillage_daemon::SearchApiResponse::IndexingStatus(status) = response else {
            return Err("unexpected indexing status response".into());
        };
        let transition_observed = if let Some((scan_time, revision)) = after {
            status
                .last_scan_unix_ms
                .is_some_and(|observed| observed > scan_time)
                && source_revision_as_consumer(consumer).await? > revision
        } else {
            status.last_scan_unix_ms.is_some()
        };
        if transition_observed
            && status.indexed_file_count == expected_count
            && !status.scanning
            && status.pending_file_count == 0
            && !status.last_scan_error
        {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    Err(format!("indexing did not settle at {expected_count} files").into())
}

async fn verify_fresh_consumer_source(
    consumer: &sillage_daemon::SearchApiClient,
    root: &Path,
    filename: &str,
    token: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let response = search_as_consumer(consumer, token).await?;
    assert_eq!(response.evidence.len(), 1);
    let preview = response.evidence[0]
        .preview
        .as_ref()
        .ok_or("authorized fresh evidence was not reopened")?;
    assert!(preview.excerpt.contains(token));
    assert!(matches!(
        &preview.location,
        sillage_daemon::api::EvidenceSourceResponse::File { path, .. }
            if Path::new(path).starts_with(root) && Path::new(path).ends_with(filename)
    ));
    Ok(())
}

async fn consumer_publication_clock(
    consumer: &sillage_daemon::SearchApiClient,
) -> Result<(u64, i64), Box<dyn std::error::Error>> {
    Ok((
        consumer_scan_time(consumer).await?,
        source_revision_as_consumer(consumer).await?,
    ))
}

#[tokio::test]
async fn consumer_search_observes_post_start_source_only_after_snapshot_readiness()
-> Result<(), Box<dyn std::error::Error>> {
    let tmp = TempDir::new()?;
    let instance = tmp.path().join("source-transition-provider");
    let allowed_root = instance.join("allowed");
    fs::create_dir_all(&allowed_root)?;
    let plan = InstanceService::init_instance_with_roots(
        instance,
        vec![allowed_root.clone()],
        sillage_test_support::realm_id(31)?,
    )?;
    for directory in &plan.directories {
        fs::create_dir_all(directory)?;
    }
    fs::write(&plan.manifest_path, plan.manifest_contents.as_bytes())?;

    let shutdown = CancellationToken::new();
    let daemon = tokio::spawn(sillage_daemon::run_instance_with_shutdown(
        plan.layout.root.clone(),
        shutdown.clone(),
        AutonomyProfile::ReadOnly,
    ));
    let result = async {
        let layout = &plan.layout;
        let owner = wait_for_daemon_client(layout).await?;
        let consumer_realm = sillage_test_support::realm_id(32)?;
        let response = owner
            .request(sillage_daemon::ClientOperation::RealmGrantCreate {
                consumer_realm: consumer_realm.clone(),
                access: sillage_daemon::RealmGrantAccess::SearchAndOpenEvidence,
                max_sensitivity: sillage_daemon::RealmGrantSensitivity::Restricted,
                max_results: 5,
                max_evidence_bytes: 4096,
                expires_in_seconds: 86_400,
                allowed_roots: vec![allowed_root.display().to_string()],
            })
            .await?;
        let sillage_daemon::ClientResponse::RealmGrantCreated(created) = response else {
            return Err("grant creation did not return a credential".into());
        };
        let consumer = sillage_daemon::SearchApiClient::consumer(
            layout.system_dir.join("daemon.sock"),
            consumer_realm,
            created.credential.expose().to_string(),
        )?;

        let source_path = allowed_root.join("post-start-source.md");
        wait_for_indexed_source_count(&consumer, 0, None).await?;
        let before_first_publication = consumer_publication_clock(&consumer).await?;
        fs::write(
            &source_path,
            "# Fresh source\nfreshsourceevidence is published after startup.\n",
        )?;
        wait_for_indexed_source_count(&consumer, 1, Some(before_first_publication)).await?;
        verify_fresh_consumer_source(
            &consumer,
            &allowed_root,
            "post-start-source.md",
            "freshsourceevidence",
        )
        .await?;

        let before_edit_publication = consumer_publication_clock(&consumer).await?;
        fs::write(
            &source_path,
            "# Updated source\neditedsourceevidence replaces the previous content.\n",
        )?;
        wait_for_indexed_source_count(&consumer, 1, Some(before_edit_publication)).await?;
        assert!(
            search_as_consumer(&consumer, "freshsourceevidence")
                .await?
                .evidence
                .is_empty(),
            "edited source content must not leave old searchable evidence"
        );
        verify_fresh_consumer_source(
            &consumer,
            &allowed_root,
            "post-start-source.md",
            "editedsourceevidence",
        )
        .await?;

        let before_removal_publication = consumer_publication_clock(&consumer).await?;
        fs::remove_file(source_path)?;
        wait_for_indexed_source_count(&consumer, 0, Some(before_removal_publication)).await?;
        assert!(
            search_as_consumer(&consumer, "editedsourceevidence")
                .await?
                .evidence
                .is_empty(),
            "deleted source content must not remain searchable"
        );
        Ok::<(), Box<dyn std::error::Error>>(())
    }
    .await;
    shutdown.cancel();
    daemon.await??;
    result
}
