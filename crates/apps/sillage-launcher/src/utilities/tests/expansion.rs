use std::cell::Cell;
use std::error::Error;

use super::super::quicklinks::MAX_ARGUMENT_BYTES;
use super::super::{MAX_EXPANSION_BYTES, UtcSnapshot, Utilities, UtilityEffect, UtilityKind};
use super::{entry_rows, test_directory};

#[test]
fn quicklinks_encode_query_as_one_uri_component_and_reject_unsafe_templates()
-> Result<(), Box<dyn Error>> {
    let directory = test_directory()?;
    let mut utilities = Utilities::load(Ok(directory.clone()))?;
    utilities.upsert(
        UtilityKind::Quicklink,
        "",
        "Search",
        "https://example.test/search?q={query}",
    )?;
    let id = entry_rows(&mut utilities, UtilityKind::Quicklink)
        .remove(0)
        .0;

    assert_eq!(
        utilities.expand(&id, "A b&c/é?#", UtcSnapshot::now)?,
        UtilityEffect::OpenUri(
            "https://example.test/search?q=A%20b%26c%2F%C3%A9%3F%23".to_string()
        )
    );
    assert!(
        utilities
            .expand(&id, &"x".repeat(MAX_ARGUMENT_BYTES + 1), UtcSnapshot::now)
            .is_err()
    );

    for unsafe_template in [
        "file:///etc/passwd",
        "https://user:password@example.test/path",
        "https://example.test/{other}",
        "https://{query}.example.test/path",
        "https://example.test/{query}/{query}",
        "https://",
    ] {
        assert!(
            utilities
                .upsert(UtilityKind::Quicklink, "bad-link", "Bad", unsafe_template)
                .is_err()
        );
    }
    assert_eq!(entry_rows(&mut utilities, UtilityKind::Quicklink).len(), 1);

    std::fs::remove_dir_all(directory)?;
    Ok(())
}

#[test]
fn snippets_expand_only_literal_query_and_bound_the_result() -> Result<(), Box<dyn Error>> {
    let directory = test_directory()?;
    let mut utilities = Utilities::load(Ok(directory.clone()))?;
    utilities.upsert(
        UtilityKind::Snippet,
        "greeting",
        "Greeting",
        "Hello {query}; {env:HOME}; $HOME; {date}",
    )?;
    assert_eq!(
        utilities.expand("greeting", "A $HOME\nvalue", UtcSnapshot::now)?,
        UtilityEffect::Copy("Hello A $HOME\nvalue; {env:HOME}; $HOME; {date}".to_string())
    );

    let exact_limit = "{query}".repeat(4);
    utilities.upsert(UtilityKind::Snippet, "bounded", "Bounded", &exact_limit)?;
    assert_eq!(
        utilities.expand("bounded", &"x".repeat(MAX_ARGUMENT_BYTES), UtcSnapshot::now)?,
        UtilityEffect::Copy("x".repeat(MAX_EXPANSION_BYTES))
    );
    let too_large = "{query}".repeat(5);
    utilities.upsert(UtilityKind::Snippet, "oversized", "Oversized", &too_large)?;
    assert!(
        utilities
            .expand(
                "oversized",
                &"x".repeat(MAX_ARGUMENT_BYTES),
                UtcSnapshot::now,
            )
            .is_err()
    );

    std::fs::remove_dir_all(directory)?;
    Ok(())
}

#[test]
fn snippets_preserve_nested_marker_bytes_without_clock() -> Result<(), Box<dyn Error>> {
    let directory = test_directory()?;
    let mut utilities = Utilities::load(Ok(directory.clone()))?;
    let template = concat!(
        "{query}|",
        "{unknown {date:UTC:ISO8601}}|",
        "{{datetime:UTC:ISO8601}}|",
        "{unmatched {time:UTC:ISO8601}|",
    );
    let expected = concat!(
        "{date:UTC:ISO8601}|",
        "{unknown {date:UTC:ISO8601}}|",
        "{{datetime:UTC:ISO8601}}|",
        "{unmatched {time:UTC:ISO8601}|",
    );
    utilities.upsert(UtilityKind::Snippet, "nested", "Nested", template)?;
    let calls = Cell::new(0);
    assert_eq!(
        utilities.expand("nested", "{date:UTC:ISO8601}", || {
            calls.set(calls.get() + 1);
            Ok(UtcSnapshot {
                year: 2000,
                month: 1,
                day: 1,
                hour: 0,
                minute: 0,
                second: 0,
            })
        })?,
        UtilityEffect::Copy(expected.to_string())
    );
    assert_eq!(calls.get(), 0);

    std::fs::remove_dir_all(directory)?;
    Ok(())
}
