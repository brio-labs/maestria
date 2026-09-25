use maestria_extensions::{
    CapabilityRequest, CopyFormat, GrantError, InvocationOrigin, OpenRequestTarget, OpenTarget,
    Permission, authorize,
};

#[test]
fn search_cannot_run_without_grant_or_exceed_its_limit() {
    let request = CapabilityRequest::FileSearch {
        query: "receipt".into(),
        limit: 6,
    };
    assert!(matches!(
        authorize(&[], &request, InvocationOrigin::Command),
        Err(GrantError::Missing("fileSearch"))
    ));
    let grants = [Permission::FileSearch {
        max_results: Some(5),
    }];
    assert!(matches!(
        authorize(&grants, &request, InvocationOrigin::Command),
        Err(GrantError::Scope("fileSearch.maxResults"))
    ));
    let request = CapabilityRequest::FileSearch {
        query: "receipt".into(),
        limit: 5,
    };
    assert!(authorize(&grants, &request, InvocationOrigin::Command).is_ok());
}

#[test]
fn http_grant_is_not_a_suffix_match_or_downgrade() {
    let grants = [Permission::Http {
        origins: vec!["https://api.example.test".into()],
    }];
    for denied in [
        "https://api.example.test.evil.test/v1",
        "https://other.example.test/v1",
        "http://api.example.test/v1",
        "https://user:secret@api.example.test/v1",
    ] {
        let request = CapabilityRequest::Http {
            url: denied.into(),
            method: maestria_extensions::HttpMethod::Get,
            body: None,
        };
        assert!(
            matches!(
                authorize(&grants, &request, InvocationOrigin::Command),
                Err(GrantError::Scope("http.origin"))
            ),
            "accepted {denied}"
        );
    }
    let request = CapabilityRequest::Http {
        url: "https://api.example.test/v1".into(),
        method: maestria_extensions::HttpMethod::Get,
        body: None,
    };
    assert!(authorize(&grants, &request, InvocationOrigin::Command).is_ok());
}

#[test]
fn clipboard_and_open_require_both_active_grant_and_user_action() {
    let grants = [
        Permission::Copy {
            formats: vec![CopyFormat::Text],
        },
        Permission::Open {
            targets: vec![OpenTarget::Url],
        },
    ];
    let copy = CapabilityRequest::Copy {
        text: "passage".into(),
    };
    assert!(matches!(
        authorize(&grants, &copy, InvocationOrigin::Command),
        Err(GrantError::UserAction("copy"))
    ));
    assert!(authorize(&grants, &copy, InvocationOrigin::ExplicitAction).is_ok());
    let open = CapabilityRequest::Open {
        target: OpenRequestTarget::Url {
            url: "https://example.test".into(),
        },
    };
    assert!(matches!(
        authorize(&grants, &open, InvocationOrigin::Command),
        Err(GrantError::UserAction("open"))
    ));
    assert!(authorize(&grants, &open, InvocationOrigin::ExplicitAction).is_ok());
    let selected = CapabilityRequest::Open {
        target: OpenRequestTarget::SelectedFile {
            selection_id: "selected".into(),
        },
    };
    assert!(matches!(
        authorize(&grants, &selected, InvocationOrigin::ExplicitAction),
        Err(GrantError::Scope("open.targets"))
    ));
}
