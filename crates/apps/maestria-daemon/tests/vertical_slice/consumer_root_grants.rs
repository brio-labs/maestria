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
        maestria_test_support::realm_id(21)?,
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
    let session = maestria_daemon::MutationSession::start(
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
    client: &maestria_daemon::SearchApiClient,
    query: &str,
) -> Result<maestria_daemon::SearchResponse, Box<dyn std::error::Error>> {
    let result = client
        .request(maestria_daemon::SearchApiOperation::InteractiveSearch {
            query: query.to_string(),
            limit: 5,
        })
        .await?;
    let maestria_daemon::SearchApiResponse::Search(response) = result else {
        return Err("unexpected search API response".into());
    };
    Ok(response)
}

#[tokio::test]
async fn consumer_grant_isolates_roots_before_search_and_evidence_io_and_after_restart()
-> Result<(), Box<dyn std::error::Error>> {
    let tmp = TempDir::new()?;
    let (layout, allowed_root, denied_root, allowed_evidence, denied_evidence, denied_pdf_evidence) =
        indexed_two_root_provider(&tmp).await?;
    let shutdown = CancellationToken::new();
    let daemon = tokio::spawn(maestria_daemon::run_instance_with_shutdown(
        layout.root.clone(),
        shutdown.clone(),
        AutonomyProfile::ReadOnly,
    ));
    let initial = async {
        let owner = wait_for_daemon_client(&layout).await?;
        let unapproved_root = tmp.path().join("not-approved");
        fs::create_dir_all(&unapproved_root)?;
        let invalid_grant = owner
            .request(maestria_daemon::ClientOperation::RealmGrantCreate {
                consumer_realm: maestria_test_support::realm_id(23)?,
                access: maestria_daemon::RealmGrantAccess::SearchOnly,
                max_sensitivity: maestria_daemon::RealmGrantSensitivity::Restricted,
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
        let consumer_realm = maestria_test_support::realm_id(22)?;
        let response = owner
            .request(maestria_daemon::ClientOperation::RealmGrantCreate {
                consumer_realm: consumer_realm.clone(),
                access: maestria_daemon::RealmGrantAccess::SearchAndOpenEvidence,
                max_sensitivity: maestria_daemon::RealmGrantSensitivity::Restricted,
                max_results: 5,
                max_evidence_bytes: 2048,
                expires_in_seconds: 86_400,
                allowed_roots: vec![allowed_root.display().to_string()],
            })
            .await?;
        let maestria_daemon::ClientResponse::RealmGrantCreated(created) = response else {
            return Err("grant creation did not return a credential".into());
        };
        assert_eq!(
            created.grant.allowed_roots,
            Some(vec![allowed_root.display().to_string()])
        );
        let consumer = maestria_daemon::SearchApiClient::consumer(
            layout.system_dir.join("daemon.sock"),
            consumer_realm,
            created.credential.expose().to_string(),
        )?;
        let other_realm = maestria_test_support::realm_id(24)?;
        let other_grant = owner
            .request(maestria_daemon::ClientOperation::RealmGrantCreate {
                consumer_realm: other_realm.clone(),
                access: maestria_daemon::RealmGrantAccess::SearchAndOpenEvidence,
                max_sensitivity: maestria_daemon::RealmGrantSensitivity::Restricted,
                max_results: 5,
                max_evidence_bytes: 2048,
                expires_in_seconds: 86_400,
                allowed_roots: vec![denied_root.display().to_string()],
            })
            .await?;
        let maestria_daemon::ClientResponse::RealmGrantCreated(other_grant) = other_grant else {
            return Err("second root grant did not return a credential".into());
        };
        let other_consumer = maestria_daemon::SearchApiClient::consumer(
            layout.system_dir.join("daemon.sock"),
            other_realm,
            other_grant.credential.expose().to_string(),
        )?;
        let opened_pdf = other_consumer
            .request(maestria_daemon::SearchApiOperation::Evidence {
                evidence_id: denied_pdf_evidence.value(),
            })
            .await?;
        let maestria_daemon::SearchApiResponse::Evidence(opened_pdf) = opened_pdf else {
            return Err("authorized PDF evidence did not open".into());
        };
        assert!(opened_pdf.excerpt.contains("pdfquartz"));
        assert!(matches!(
            opened_pdf.source,
            maestria_daemon::api::EvidenceSourceResponse::Pdf {
                page_start: 1,
                path: Some(path),
                ..
            } if Path::new(&path).starts_with(&denied_root)
                && Path::new(&path).ends_with("b-secret-report.pdf")
        ));

        let shared = search_as_consumer(&consumer, "orchid").await?;
        assert!(!shared.evidence.is_empty(), "approved passage is missing");
        assert!(shared.evidence.iter().all(|evidence| {
            matches!(
                evidence.preview.as_ref().map(|preview| &preview.location),
                Some(maestria_daemon::api::EvidenceSourceResponse::File { path, .. })
                    if Path::new(path).starts_with(&allowed_root)
            )
        }));
        let other_passage = search_as_consumer(&other_consumer, "basalt").await?;
        assert_eq!(other_passage.evidence.len(), 1);
        assert!(matches!(
            other_passage.evidence[0].preview.as_ref().map(|preview| &preview.location),
            Some(maestria_daemon::api::EvidenceSourceResponse::File { path, .. })
                if Path::new(path).starts_with(&denied_root)
        ));
        let private = search_as_consumer(&consumer, "basalt").await?;
        assert!(private.evidence.is_empty());
        assert!(private.path_results.is_empty());
        assert!(
            search_as_consumer(&consumer, "pdfquartz")
                .await?
                .evidence
                .is_empty(),
            "PDF from the other root appeared in A's passage search"
        );
        let normal_search = consumer
            .request(maestria_daemon::SearchApiOperation::Search {
                query: "basalt".to_string(),
                limit: 5,
            })
            .await?;
        let maestria_daemon::SearchApiResponse::Search(normal_search) = normal_search else {
            return Err("unexpected noninteractive search response".into());
        };
        assert!(normal_search.evidence.is_empty());
        let private_filename = search_as_consumer(&consumer, "b-secret-filename").await?;
        assert!(private_filename.path_results.is_empty());
        let allowed_filename = search_as_consumer(&consumer, "a-visible-filename").await?;
        assert_eq!(allowed_filename.path_results.len(), 1);
        assert!(Path::new(&allowed_filename.path_results[0].path).starts_with(&allowed_root));
        for evidence_id in [denied_evidence, denied_pdf_evidence] {
            let denied = match consumer
                .request(maestria_daemon::SearchApiOperation::Evidence {
                    evidence_id: evidence_id.value(),
                })
                .await
            {
                Ok(_) => return Err("a scoped grant opened evidence from another root".into()),
                Err(error) => error,
            };
            assert_eq!(
                denied.code,
                maestria_daemon::ClientErrorCode::SourceNotSelected
            );
        }
        assert!(matches!(
            consumer
                .request(maestria_daemon::SearchApiOperation::Evidence {
                    evidence_id: allowed_evidence.value(),
                })
                .await?,
            maestria_daemon::SearchApiResponse::Evidence(_)
        ));

        let mut scoped_inventory = None;
        for _ in 0..30 {
            let result = consumer
                .request(maestria_daemon::SearchApiOperation::IndexingStatus)
                .await?;
            let maestria_daemon::SearchApiResponse::IndexingStatus(status) = result else {
                return Err("unexpected indexing status response".into());
            };
            if status.indexed_file_count == 1 && !status.scanning && status.pending_file_count == 0
            {
                scoped_inventory = Some(status);
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        let inventory = scoped_inventory.ok_or("scoped index did not settle")?;
        assert_eq!(inventory.approved_root_count, 1);
        assert_eq!(inventory.indexed_file_count, 1);

        Ok::<maestria_daemon::SearchApiClient, Box<dyn std::error::Error>>(consumer)
    }
    .await;
    shutdown.cancel();
    daemon.await??;
    let consumer = initial?;
    let restarted_shutdown = CancellationToken::new();
    let restarted_daemon = tokio::spawn(maestria_daemon::run_instance_with_shutdown(
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
            .request(maestria_daemon::ClientOperation::SearchRootRemove {
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
            .request(maestria_daemon::SearchApiOperation::IndexingStatus)
            .await?;
        let maestria_daemon::SearchApiResponse::IndexingStatus(status) = status else {
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
