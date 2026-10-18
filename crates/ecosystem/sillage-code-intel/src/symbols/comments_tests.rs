use super::*;

fn kinds(source: &str) -> Vec<CodeMarkerKind> {
    scan_comment_markers(source)
        .into_iter()
        .map(|marker| marker.kind)
        .collect()
}

#[test]
fn line_comments_and_forms_are_detected() {
    assert_eq!(
        kinds("// todo: fix this\n// fixme(panic)\n// hack[temp]\n// NOT_todo\n"),
        vec![
            CodeMarkerKind::Todo,
            CodeMarkerKind::Fixme,
            CodeMarkerKind::Hack
        ]
    );
    assert_eq!(kinds("//todo"), vec![CodeMarkerKind::Todo]);
    assert_eq!(kinds("//! todo inner doc"), vec![CodeMarkerKind::Todo]);
    assert_eq!(kinds("// todoXY not a marker"), Vec::new());
    assert_eq!(kinds("// Todays is not a marker"), Vec::new());
}

#[test]
fn block_comments_span_lines() {
    let markers = scan_comment_markers("/* todo: start\n * more\n * done */\nfn f() {}");
    assert_eq!(markers.len(), 1);
    assert_eq!(markers[0].start_line, 1);
    assert_eq!(markers[0].end_line, 3);
    assert_eq!(
        kinds("let x = 1; /* hack */ let y = 2;"),
        vec![CodeMarkerKind::Hack]
    );
    assert_eq!(kinds("/* plain */\n"), Vec::new());
}

#[test]
fn strings_and_chars_are_skipped() {
    assert_eq!(
        kinds("let s = \"// todo: not a marker\";\nlet c = '/'; // todo: real\n"),
        vec![CodeMarkerKind::Todo]
    );
    assert_eq!(
        kinds("let s = r#\"// fixme: inside raw\"#;\nlet t = r\"// hack\";\n"),
        Vec::new()
    );
    assert_eq!(
        kinds("let s = \"a\\\\\" // todo: after escaped string\n"),
        vec![CodeMarkerKind::Todo]
    );
    assert_eq!(
        kinds("let l: &'a str = \"\"; // fixme real\n"),
        vec![CodeMarkerKind::Fixme]
    );
}

#[test]
fn markers_inside_block_comments_and_nesting() {
    assert_eq!(
        kinds("/* outer /* todo: nested */ still comment */ // fixme: after\n"),
        vec![CodeMarkerKind::Fixme]
    );
    assert_eq!(
        kinds("/* todo: opens\nstill open\n*/\n"),
        vec![CodeMarkerKind::Todo]
    );
}

#[test]
fn attachment_prefers_innermost_symbol() -> Result<(), Box<dyn std::error::Error>> {
    let mut symbols = vec![symbol(1, 10)?, symbol(2, 4)?, symbol(3, 3)?, symbol(7, 9)?];
    let markers = vec![
        CommentMarker {
            kind: CodeMarkerKind::Todo,
            start_line: 3,
            end_line: 3,
        },
        CommentMarker {
            kind: CodeMarkerKind::Fixme,
            start_line: 8,
            end_line: 8,
        },
        CommentMarker {
            kind: CodeMarkerKind::Hack,
            start_line: 12,
            end_line: 12,
        },
    ];
    let orphans = attach_comment_markers(&mut symbols, markers)?;
    assert_eq!(symbols[0].markers.code_markers.len(), 0);
    assert_eq!(symbols[1].markers.code_markers.len(), 0);
    assert_eq!(
        symbols[2].markers.code_markers[0].kind(),
        CodeMarkerKind::Todo
    );
    assert_eq!(
        symbols[3].markers.code_markers[0].kind(),
        CodeMarkerKind::Fixme
    );
    assert_eq!(orphans.len(), 1);
    assert_eq!(orphans[0].kind, CodeMarkerKind::Hack);
    Ok(())
}

fn symbol(start: usize, end: usize) -> Result<SymbolRecord, Box<dyn std::error::Error>> {
    Ok(SymbolRecord {
        record_id: format!("{start}-{end}"),
        package: "pkg".to_string(),
        target: "target".to_string(),
        kind: crate::SymbolKind::Function,
        name: "f".to_string(),
        qualified_name: "f".to_string(),
        visibility: crate::Visibility::Private,
        is_public_api: false,
        is_async: false,
        is_unsafe: false,
        is_test: false,
        is_bench: false,
        signature: None,
        imports: Vec::new(),
        doc_comment: None,
        markers: crate::SymbolMarkers::default(),
        provenance: crate::RecordProvenance {
            repository_root: "/work".to_string(),
            commit_sha: crate::CommitSha::new("0000000"),
            worktree_identity: crate::WorktreeIdentity::new("local"),
            content_hash: "sha256:0000000000000000000000000000000000000000000000000000000000000000"
                .to_string(),
            file_path: "src/lib.rs".to_string(),
            source_range: crate::SourceRange::new(start, end)?,
            parser_generation: crate::ParserGeneration::new("test"),
        },
    })
}
