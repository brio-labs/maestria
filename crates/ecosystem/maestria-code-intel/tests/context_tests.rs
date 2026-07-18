use maestria_code_intel::*;
use std::error::Error;

fn make_provenance(file_path: &str, start_line: usize) -> RecordProvenance {
    RecordProvenance {
        repository_root: "/work".to_string(),
        commit_sha: "0000000".to_string(),
        worktree_identity: "local".to_string(),
        file_path: file_path.to_string(),
        source_range: SourceRange {
            start_line,
            end_line: start_line,
        },
        parser_generation: "test".to_string(),
    }
}

fn make_symbol(id: &str, file_path: &str, line: usize) -> SymbolRecord {
    SymbolRecord {
        record_id: id.to_string(),
        package: "pkg".to_string(),
        target: "target".to_string(),
        kind: SymbolKind::Function,
        name: id.to_string(),
        qualified_name: id.to_string(),
        visibility: Visibility::Public,
        is_public_api: false,
        is_async: false,
        is_unsafe: false,
        is_test: false,
        is_bench: false,
        signature: None,
        imports: Vec::new(),
        markers: SymbolMarkers::default(),
        provenance: make_provenance(file_path, line),
    }
}

fn make_relation(
    source: &SymbolRecord,
    target: &SymbolRecord,
    kind: CodeRelationKind,
) -> CodeRelationRecord {
    CodeRelationRecord {
        kind,
        source_record_id: source.record_id.clone(),
        target_record_id: target.record_id.clone(),
        source_provenance: source.provenance.clone(),
        target_provenance: target.provenance.clone(),
        parser_generation: "test".to_string(),
        confidence_milli: 1000,
        source_kind: RelationSourceKind::Ast,
    }
}

fn context_fixture() -> RepositoryCodeIndex {
    let seed_a = make_symbol("seed_a", "crate/lib.rs", 1);
    let seed_b = make_symbol("seed_b", "crate/lib.rs", 2);
    let mid = make_symbol("mid", "crate/lib.rs", 3);
    let leaf = make_symbol("leaf", "crate/lib.rs", 4);
    let extra = make_symbol("extra", "crate/lib.rs", 5);

    let symbols = vec![
        seed_a.clone(),
        seed_b.clone(),
        mid.clone(),
        leaf.clone(),
        extra.clone(),
    ];

    // Intentionally unsorted relation input order to assert deterministic output.
    let relations = vec![
        make_relation(&seed_b, &mid, CodeRelationKind::Calls),
        make_relation(&seed_a, &extra, CodeRelationKind::Imports),
        make_relation(&seed_a, &mid, CodeRelationKind::Calls),
        make_relation(&leaf, &extra, CodeRelationKind::Calls),
        make_relation(&mid, &leaf, CodeRelationKind::Calls),
    ];

    RepositoryCodeIndex {
        summary: CodeIndexSummary {
            repository_root: "/work".to_string(),
            commit_sha: "0000000".to_string(),
            worktree_identity: "local".to_string(),
            parser_generation: "test".to_string(),
            package_count: 1,
            target_count: 1,
            symbol_count: symbols.len(),
            file_count: 1,
            packages: vec!["pkg".to_string()],
            excluded_patterns: Vec::new(),
            relation_summary: CodeRelationSummary {
                total_relations: relations.len(),
                source_statuses: Vec::new(),
            },
        },
        packages: Vec::new(),
        symbols,
        relations,
    }
}

#[test]
fn context_outgoing_traverses_outgoing_relations() {
    let index = context_fixture();
    let result = index.context(RepositoryContextQuery {
        query: CodeQuery::Symbol {
            pattern: "seed_a".to_string(),
        },
        direction: ContextDirection::Outgoing,
        relation_kinds: None,
        max_depth: 3,
        max_nodes: 10,
    });

    let nodes: Vec<_> = result
        .nodes
        .iter()
        .map(|node| node.record.record_id.as_str())
        .collect();
    assert_eq!(nodes, vec!["seed_a", "mid", "leaf", "extra"]);

    let edges: Vec<_> = result
        .edges
        .iter()
        .map(|edge| {
            (
                edge.relation.source_record_id.as_str(),
                edge.relation.kind,
                edge.relation.target_record_id.as_str(),
            )
        })
        .collect();
    assert_eq!(
        edges,
        vec![
            ("seed_a", CodeRelationKind::Imports, "extra"),
            ("leaf", CodeRelationKind::Calls, "extra"),
            ("mid", CodeRelationKind::Calls, "leaf"),
            ("seed_a", CodeRelationKind::Calls, "mid"),
        ]
    );
}

#[test]
fn context_incoming_traverses_incoming_relations() {
    let index = context_fixture();
    let result = index.context(RepositoryContextQuery {
        query: CodeQuery::Symbol {
            pattern: "leaf".to_string(),
        },
        direction: ContextDirection::Incoming,
        relation_kinds: None,
        max_depth: 4,
        max_nodes: 10,
    });

    let nodes: Vec<_> = result
        .nodes
        .iter()
        .map(|node| node.record.record_id.as_str())
        .collect();
    assert_eq!(nodes, vec!["seed_a", "seed_b", "mid", "leaf"]);

    let edges: Vec<_> = result
        .edges
        .iter()
        .map(|edge| {
            (
                edge.relation.source_record_id.as_str(),
                edge.relation.kind,
                edge.relation.target_record_id.as_str(),
            )
        })
        .collect();
    assert_eq!(
        edges,
        vec![
            ("mid", CodeRelationKind::Calls, "leaf"),
            ("seed_a", CodeRelationKind::Calls, "mid"),
            ("seed_b", CodeRelationKind::Calls, "mid"),
        ]
    );
}

#[test]
fn context_filters_relation_kinds() {
    let index = context_fixture();
    let result = index.context(RepositoryContextQuery {
        query: CodeQuery::Symbol {
            pattern: "seed".to_string(),
        },
        direction: ContextDirection::Both,
        relation_kinds: Some(vec![CodeRelationKind::Calls]),
        max_depth: 4,
        max_nodes: 10,
    });

    let relation_kinds: Vec<_> = result.edges.iter().map(|edge| edge.relation.kind).collect();
    assert!(
        relation_kinds
            .iter()
            .all(|kind| matches!(kind, CodeRelationKind::Calls))
    );

    let nodes: Vec<_> = result
        .nodes
        .iter()
        .map(|node| node.record.record_id.as_str())
        .collect();
    assert_eq!(nodes, vec!["seed_a", "seed_b", "mid", "leaf", "extra"]);

    let edges: Vec<_> = result
        .edges
        .iter()
        .map(|edge| {
            (
                edge.relation.source_record_id.as_str(),
                edge.relation.target_record_id.as_str(),
            )
        })
        .collect();
    assert_eq!(
        edges,
        vec![
            ("leaf", "extra"),
            ("mid", "leaf"),
            ("seed_a", "mid"),
            ("seed_b", "mid"),
        ]
    );
}

#[test]
fn context_rejects_relation_with_forged_endpoint_provenance() {
    let mut index = context_fixture();
    index.relations[0].source_provenance.file_path = "forged.rs".to_string();

    let result = index.context(RepositoryContextQuery {
        query: CodeQuery::Symbol {
            pattern: "seed_b".to_string(),
        },
        direction: ContextDirection::Outgoing,
        relation_kinds: None,
        max_depth: 1,
        max_nodes: 10,
    });

    assert!(result.edges.is_empty());
    assert_eq!(result.nodes.len(), 1);
}

#[test]
fn context_respects_depth_and_node_caps() {
    let index = context_fixture();

    let depth_limited = index.context(RepositoryContextQuery {
        query: CodeQuery::Symbol {
            pattern: "seed_a".to_string(),
        },
        direction: ContextDirection::Outgoing,
        relation_kinds: None,
        max_depth: 1,
        max_nodes: 10,
    });

    let depth_nodes: Vec<_> = depth_limited
        .nodes
        .iter()
        .map(|node| node.record.record_id.as_str())
        .collect();
    assert_eq!(depth_nodes, vec!["seed_a", "mid", "extra"]);
    assert!(depth_limited.nodes.iter().all(|node| node.depth <= 1));
    assert!(
        depth_limited
            .nodes
            .iter()
            .all(|node| node.record.record_id != "leaf")
    );

    let node_capped = index.context(RepositoryContextQuery {
        query: CodeQuery::Symbol {
            pattern: "seed".to_string(),
        },
        direction: ContextDirection::Both,
        relation_kinds: None,
        max_depth: 4,
        max_nodes: 3,
    });

    let capped_nodes: Vec<_> = node_capped
        .nodes
        .iter()
        .map(|node| node.record.record_id.as_str())
        .collect();
    assert_eq!(capped_nodes, vec!["seed_a", "seed_b", "extra"]);
    assert!(node_capped.summary.nodes_truncated);
}

#[test]
fn context_reports_deterministic_ordering() {
    let index = context_fixture();
    let first = index.context(RepositoryContextQuery {
        query: CodeQuery::Symbol {
            pattern: "seed".to_string(),
        },
        direction: ContextDirection::Both,
        relation_kinds: None,
        max_depth: 2,
        max_nodes: 10,
    });

    let second = index.context(RepositoryContextQuery {
        query: CodeQuery::Symbol {
            pattern: "seed".to_string(),
        },
        direction: ContextDirection::Both,
        relation_kinds: None,
        max_depth: 2,
        max_nodes: 10,
    });

    let first_nodes: Vec<_> = first
        .nodes
        .iter()
        .map(|node| node.record.record_id.as_str())
        .collect();
    let second_nodes: Vec<_> = second
        .nodes
        .iter()
        .map(|node| node.record.record_id.as_str())
        .collect();

    assert_eq!(
        first_nodes,
        vec!["seed_a", "seed_b", "mid", "leaf", "extra"]
    );
    assert_eq!(first_nodes, second_nodes);

    let first_edges: Vec<_> = first
        .edges
        .iter()
        .map(|edge| {
            (
                edge.relation.source_record_id.as_str(),
                edge.relation.kind,
                edge.relation.target_record_id.as_str(),
                edge.depth,
            )
        })
        .collect();
    let second_edges: Vec<_> = second
        .edges
        .iter()
        .map(|edge| {
            (
                edge.relation.source_record_id.as_str(),
                edge.relation.kind,
                edge.relation.target_record_id.as_str(),
                edge.depth,
            )
        })
        .collect();
    assert_eq!(first_edges, second_edges);
}

#[test]
fn context_preserves_seed_lineage_for_nodes_and_edges() -> Result<(), Box<dyn Error>> {
    let index = context_fixture();
    let result = index.context(RepositoryContextQuery {
        query: CodeQuery::Symbol {
            pattern: "seed".to_string(),
        },
        direction: ContextDirection::Both,
        relation_kinds: Some(vec![CodeRelationKind::Calls]),
        max_depth: 4,
        max_nodes: 10,
    });

    let mid_node = result
        .nodes
        .iter()
        .find(|node| node.record.record_id == "mid")
        .ok_or("missing mid node")?;
    let leaf_node = result
        .nodes
        .iter()
        .find(|node| node.record.record_id == "leaf")
        .ok_or("missing leaf node")?;

    assert_eq!(mid_node.seed_record_ids, vec!["seed_a", "seed_b"]);
    assert_eq!(leaf_node.seed_record_ids, vec!["seed_a", "seed_b"]);

    let mid_to_leaf = result
        .edges
        .iter()
        .find(|edge| {
            edge.relation.source_record_id == "mid" && edge.relation.target_record_id == "leaf"
        })
        .ok_or("missing mid->leaf edge")?;
    assert_eq!(mid_to_leaf.seed_record_ids, vec!["seed_a", "seed_b"]);

    for edge in &result.edges {
        let source = index
            .symbols
            .iter()
            .find(|symbol| symbol.record_id == edge.relation.source_record_id)
            .ok_or("source symbol missing in fixture")?;
        let target = index
            .symbols
            .iter()
            .find(|symbol| symbol.record_id == edge.relation.target_record_id)
            .ok_or("target symbol missing in fixture")?;

        assert_eq!(source.provenance, edge.relation.source_provenance);
        assert_eq!(target.provenance, edge.relation.target_provenance);
    }
    Ok(())
}
