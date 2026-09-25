use super::*;

async fn prepare_federation_provider(
    tmp: &TempDir,
) -> Result<(TestEnv, IndexResult), Box<dyn std::error::Error>> {
    let mut provider = prepare_test_env(tmp).await?;
    let indexed = {
        let session = provider
            .session
            .as_ref()
            .ok_or_else(|| std::io::Error::other("provider mutation session missing"))?;
        index_and_verify_artifact(
            session,
            &provider.layout.database_path,
            provider.artifact_id,
            &provider.hash,
            &provider.bytes,
            &provider.source_path,
        )
        .await?
    };
    finish_session(
        provider.session.take(),
        Ok::<_, Box<dyn std::error::Error>>(()),
    )
    .await?;
    Ok((provider, indexed))
}

fn initialize_federation_consumer(
    tmp: &TempDir,
) -> Result<(InstanceLayout, RealmId), Box<dyn std::error::Error>> {
    let consumer_root = tmp.path().join("consumer");
    let consumer_notes = consumer_root.join("notes");
    fs::create_dir_all(&consumer_notes)?;
    let consumer_realm = maestria_test_support::realm_id(11)?;
    let consumer_plan = InstanceService::init_instance_with_roots(
        consumer_root,
        vec![consumer_notes],
        consumer_realm.clone(),
    )?;
    for directory in &consumer_plan.directories {
        fs::create_dir_all(directory)?;
    }
    fs::write(
        &consumer_plan.manifest_path,
        consumer_plan.manifest_contents.as_bytes(),
    )?;
    Ok((consumer_plan.layout, consumer_realm))
}

async fn assert_provider_federation_ready(
    provider: &TestEnv,
    indexed: &IndexResult,
    provider_client: &maestria_daemon::DaemonClient,
) -> Result<(), Box<dyn std::error::Error>> {
    let store = SqliteStore::open(&provider.layout.database_path)?;
    let artifact = ArtifactRepository::get(&store, provider.artifact_id)?
        .ok_or_else(|| std::io::Error::other("provider artifact missing after startup"))?;
    assert_eq!(artifact.index_status, IndexStatus::Indexed);
    let evidence = maestria_ports::EvidenceRepository::get(&store, indexed.evidence_id)?
        .ok_or_else(|| std::io::Error::other("provider evidence missing after startup"))?;
    assert!(evidence.security.read_allowed);
    let local_search = provider_client
        .request(maestria_daemon::ClientOperation::Search {
            query: "GraphRAG knowledge graphs".to_string(),
            limit: 1,
        })
        .await?;
    let maestria_daemon::ClientResponse::Search(local_search) = local_search else {
        return Err("provider returned an unexpected local search response".into());
    };
    assert_eq!(local_search.evidence.len(), 1);
    assert!(
        local_search
            .evidence
            .iter()
            .all(|evidence| evidence.preview.is_none()),
        "internal owner search must not receive federated passage previews"
    );
    Ok(())
}

async fn create_and_install_federation_grant(
    provider_client: &maestria_daemon::DaemonClient,
    consumer_client: &maestria_daemon::DaemonClient,
    provider_layout: &InstanceLayout,
    consumer_realm: &RealmId,
    access: maestria_daemon::RealmGrantAccess,
) -> Result<(String, RealmId), Box<dyn std::error::Error>> {
    let response = provider_client
        .request(maestria_daemon::ClientOperation::RealmGrantCreate {
            consumer_realm: consumer_realm.clone(),
            access,
            max_sensitivity: maestria_daemon::RealmGrantSensitivity::Restricted,
            max_results: 1,
            max_evidence_bytes: 16,
            expires_in_seconds: 86_400,
            allowed_roots: Vec::new(),
        })
        .await?;
    let maestria_daemon::ClientResponse::RealmGrantCreated(created) = response else {
        return Err("provider returned an unexpected grant response".into());
    };
    let token_digest = created.grant.token_digest.clone();
    let provider_realm = created.grant.provider_realm.clone();
    let installed = consumer_client
        .request(maestria_daemon::ClientOperation::InstallFederationBinding {
            provider_realm: provider_realm.clone(),
            provider_socket_path: provider_layout
                .root
                .join("system")
                .join("daemon.sock")
                .display()
                .to_string(),
            credential: created.credential,
        })
        .await?;
    assert!(matches!(
        installed,
        maestria_daemon::ClientResponse::FederationBindingInstalled
    ));
    Ok((token_digest, provider_realm))
}

async fn assert_search_only_federation(
    consumer_client: &maestria_daemon::DaemonClient,
    provider_realm: &RealmId,
    evidence_id: EvidenceId,
    provider_source_path: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let response = consumer_client
        .request(maestria_daemon::ClientOperation::FederationSearch {
            provider_realm: provider_realm.clone(),
            query: "GraphRAG knowledge graphs".to_string(),
            limit: 100,
        })
        .await?;
    let maestria_daemon::ClientResponse::FederationSearch(search) = response else {
        return Err("consumer returned an unexpected federation search response".into());
    };
    assert!(search.graph_degraded);
    assert_eq!(
        search.search.evidence.len(),
        1,
        "federated search: {search:?}"
    );
    assert!(
        search.search.evidence[0].preview.is_some(),
        "search-only grant did not receive an authorized passage preview: {:?}",
        search.search.evidence
    );
    let passage = search.search.evidence[0]
        .preview
        .as_ref()
        .ok_or("search-only grant did not receive an authorized passage preview")?;
    assert!(!passage.excerpt.is_empty());
    assert!(passage.excerpt.len() <= 16);
    let source_hash = content_hash(&fs::read(provider_source_path)?);
    match &passage.location {
        maestria_daemon::api::EvidenceSourceResponse::File {
            path,
            start_line,
            end_line,
            content_hash,
        } => {
            assert_eq!(path, provider_source_path);
            assert!(*start_line > 0);
            assert_eq!(content_hash, &source_hash);
            assert!(*end_line >= *start_line);
        }
        other => return Err(format!("unexpected passage source: {other:?}").into()),
    }
    let encoded = serde_json::to_vec(&maestria_daemon::ClientResponse::FederationSearch(
        search.clone(),
    ))?;
    assert!(encoded.len() + 1 + 4 * 1024 <= 64 * 1024);

    let evidence = consumer_client
        .request(maestria_daemon::ClientOperation::FederationEvidence {
            provider_realm: provider_realm.clone(),
            evidence_id: evidence_id.value(),
        })
        .await;
    assert!(evidence.is_err(), "search-only grant opened evidence");
    Ok(())
}

async fn assert_edited_source_previews_are_current(
    consumer_client: &maestria_daemon::DaemonClient,
    provider_realm: &RealmId,
    evidence_id: EvidenceId,
    provider_source_path: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let updated_source = b"# GraphRAG Survey\n\nGraph Retrieval-Augmented Generation combines knowledge graphs with RAG to verify only current source passages.\n";
    fs::write(provider_source_path, updated_source)?;
    let current_hash = content_hash(updated_source);
    let response = consumer_client
        .request(maestria_daemon::ClientOperation::FederationSearch {
            provider_realm: provider_realm.clone(),
            query: "GraphRAG knowledge graphs".to_string(),
            limit: 100,
        })
        .await?;
    let maestria_daemon::ClientResponse::FederationSearch(search) = response else {
        return Err("consumer returned an unexpected federation search response".into());
    };
    for evidence in &search.search.evidence {
        let Some(preview) = &evidence.preview else {
            continue;
        };
        match &preview.location {
            maestria_daemon::api::EvidenceSourceResponse::File {
                path, content_hash, ..
            } => {
                assert_eq!(path, provider_source_path);
                assert_eq!(content_hash, &current_hash);
            }
            other => return Err(format!("unexpected passage source: {other:?}").into()),
        }
    }

    let stale_open = consumer_client
        .request(maestria_daemon::ClientOperation::FederationEvidence {
            provider_realm: provider_realm.clone(),
            evidence_id: evidence_id.value(),
        })
        .await;
    assert!(
        stale_open.is_err(),
        "edited source left stale evidence openable"
    );
    Ok(())
}

async fn assert_revocation_blocks_federation(
    provider_client: &maestria_daemon::DaemonClient,
    consumer_client: &maestria_daemon::DaemonClient,
    provider_realm: &RealmId,
    token_digest: String,
) -> Result<(), Box<dyn std::error::Error>> {
    provider_client
        .request(maestria_daemon::ClientOperation::RealmGrantRevoke { token_digest })
        .await?;
    let revoked_search = consumer_client
        .request(maestria_daemon::ClientOperation::FederationSearch {
            provider_realm: provider_realm.clone(),
            query: "GraphRAG knowledge graphs".to_string(),
            limit: 1,
        })
        .await;
    assert!(revoked_search.is_err(), "revoked grant still searched");
    Ok(())
}

async fn assert_reissued_grant_opens_bounded_evidence(
    provider_client: &maestria_daemon::DaemonClient,
    consumer_client: &maestria_daemon::DaemonClient,
    provider_layout: &InstanceLayout,
    consumer_realm: &RealmId,
    evidence_id: EvidenceId,
) -> Result<(), Box<dyn std::error::Error>> {
    let (_, provider_realm) = create_and_install_federation_grant(
        provider_client,
        consumer_client,
        provider_layout,
        consumer_realm,
        maestria_daemon::RealmGrantAccess::SearchAndOpenEvidence,
    )
    .await?;
    let response = consumer_client
        .request(maestria_daemon::ClientOperation::FederationEvidence {
            provider_realm,
            evidence_id: evidence_id.value(),
        })
        .await?;
    let maestria_daemon::ClientResponse::FederationEvidence(opened) = response else {
        return Err("consumer returned an unexpected federation evidence response".into());
    };
    assert!(opened.evidence.excerpt.len() <= 16);
    Ok(())
}

async fn exercise_federation(
    provider: &TestEnv,
    indexed: &IndexResult,
    consumer_layout: &InstanceLayout,
    consumer_realm: &RealmId,
) -> Result<(), Box<dyn std::error::Error>> {
    let provider_client = wait_for_daemon_client(&provider.layout).await?;
    let consumer_client = wait_for_daemon_client(consumer_layout).await?;
    assert_provider_federation_ready(provider, indexed, &provider_client).await?;
    let (token_digest, provider_realm) = create_and_install_federation_grant(
        &provider_client,
        &consumer_client,
        &provider.layout,
        consumer_realm,
        maestria_daemon::RealmGrantAccess::SearchOnly,
    )
    .await?;
    assert_search_only_federation(
        &consumer_client,
        &provider_realm,
        indexed.evidence_id,
        &provider.source_path,
    )
    .await?;
    assert_revocation_blocks_federation(
        &provider_client,
        &consumer_client,
        &provider_realm,
        token_digest,
    )
    .await?;
    assert_reissued_grant_opens_bounded_evidence(
        &provider_client,
        &consumer_client,
        &provider.layout,
        consumer_realm,
        indexed.evidence_id,
    )
    .await?;
    assert_edited_source_previews_are_current(
        &consumer_client,
        &provider_realm,
        indexed.evidence_id,
        &provider.source_path,
    )
    .await
}

#[tokio::test]
async fn federation_search_is_bounded_and_revocable() -> Result<(), Box<dyn std::error::Error>> {
    let tmp = TempDir::new()?;
    let (provider, indexed) = prepare_federation_provider(&tmp).await?;
    let (consumer_layout, consumer_realm) = initialize_federation_consumer(&tmp)?;

    let provider_shutdown = CancellationToken::new();
    let provider_daemon = tokio::spawn(maestria_daemon::run_instance_with_shutdown(
        provider.layout.root.clone(),
        provider_shutdown.clone(),
        maestria_governance::AutonomyProfile::ReadOnly,
    ));
    let consumer_shutdown = CancellationToken::new();
    let consumer_daemon = tokio::spawn(maestria_daemon::run_instance_with_shutdown(
        consumer_layout.root.clone(),
        consumer_shutdown.clone(),
        maestria_governance::AutonomyProfile::ReadOnly,
    ));
    let result = exercise_federation(&provider, &indexed, &consumer_layout, &consumer_realm).await;

    provider_shutdown.cancel();
    consumer_shutdown.cancel();
    let provider_result = provider_daemon.await?;
    let consumer_result = consumer_daemon.await?;
    provider_result?;
    consumer_result?;
    result
}
