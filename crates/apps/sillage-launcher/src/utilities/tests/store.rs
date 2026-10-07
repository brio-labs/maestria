use std::error::Error;

use super::super::entry::{self, Entry};
use super::super::persistence;
use super::super::{Utilities, UtilityEffect, UtilityKind};
use super::{entry_rows, test_directory};

#[test]
fn persistent_crud_round_trips_stable_ids_and_search_caches() -> Result<(), Box<dyn Error>> {
    let directory = test_directory()?;
    let mut utilities = Utilities::load(Ok(directory.clone()))?;
    utilities.upsert(
        UtilityKind::Quicklink,
        "",
        "Search",
        "https://example.test/",
    )?;
    utilities.upsert(
        UtilityKind::Snippet,
        "hello",
        "Hello",
        "Good morning {query}",
    )?;
    let quicklink_id = entry_rows(&mut utilities, UtilityKind::Quicklink)[0]
        .0
        .clone();
    let path = directory.join(persistence::UTILITIES_FILE);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&path)?.permissions().mode() & 0o777,
            0o600
        );
    }

    utilities.upsert(
        UtilityKind::Snippet,
        "hello",
        "Updated",
        "Good evening {query}",
    )?;
    assert!(
        utilities
            .entry(UtilityKind::Snippet, "hello")?
            .matches("updated")
    );
    assert!(
        utilities
            .entry(UtilityKind::Snippet, "hello")?
            .matches("good evening")
    );
    assert_eq!(
        utilities.expand("hello", "Ada")?,
        UtilityEffect::Copy("Good evening Ada".to_string())
    );

    utilities.capture_clipboard("ephemeral")?;
    assert_eq!(entry_rows(&mut utilities, UtilityKind::Clipboard).len(), 1);
    utilities.clear_clipboard();
    assert!(entry_rows(&mut utilities, UtilityKind::Clipboard).is_empty());
    utilities.capture_clipboard("ephemeral")?;
    let persisted = std::fs::read_to_string(&path)?;
    assert!(!persisted.contains("ephemeral"));
    let mut reloaded = Utilities::load(Ok(directory.clone()))?;
    assert_eq!(
        entry_rows(&mut reloaded, UtilityKind::Quicklink)[0].0,
        quicklink_id
    );
    assert_eq!(
        entry_rows(&mut reloaded, UtilityKind::Snippet)[0].1,
        "Updated"
    );
    assert!(
        reloaded
            .entry(UtilityKind::Snippet, "hello")?
            .matches("good evening")
    );
    assert!(entry_rows(&mut reloaded, UtilityKind::Clipboard).is_empty());
    reloaded.remove(UtilityKind::Quicklink, &quicklink_id)?;
    assert!(entry_rows(&mut reloaded, UtilityKind::Quicklink).is_empty());

    std::fs::remove_dir_all(directory)?;
    Ok(())
}

#[test]
fn borrowed_listing_selection_and_entry_bounds_are_enforced() -> Result<(), Box<dyn Error>> {
    let directory = test_directory()?;
    let mut utilities = Utilities::load(Ok(directory.clone()))?;
    utilities.upsert(UtilityKind::Snippet, "selected", "Selected", "large body")?;
    utilities.upsert(
        UtilityKind::Quicklink,
        "other",
        "Other",
        "https://example.test/",
    )?;

    let mut listed = Vec::new();
    utilities.visit_entries(UtilityKind::Snippet, |entry| {
        listed.push(entry.id.clone());
    });
    assert_eq!(listed, vec!["selected".to_string()]);
    {
        let selected = utilities.entry(UtilityKind::Snippet, "selected")?;
        assert_eq!(selected.content.as_str(), "large body");
        assert!(selected.matches("select"));
        assert!(selected.matches("large"));
        assert!(selected.matches(""));
        assert!(!selected.matches("missing"));
    }
    assert!(utilities.entry(UtilityKind::Quicklink, "selected").is_err());

    let mut maximum = (0..entry::MAX_ENTRIES)
        .map(|index| {
            Entry::new(
                format!("snippet-{index}"),
                UtilityKind::Snippet,
                "Snippet".to_string(),
                String::new(),
            )
        })
        .collect::<Vec<_>>();
    assert!(persistence::validate_stored_entries(&maximum).is_ok());
    let duplicate_ids = vec![maximum[0].clone(), maximum[0].clone()];
    assert!(persistence::validate_stored_entries(&duplicate_ids).is_err());
    maximum.push(Entry::new(
        "snippet-over-limit".to_string(),
        UtilityKind::Snippet,
        "Snippet".to_string(),
        String::new(),
    ));
    assert!(persistence::validate_stored_entries(&maximum).is_err());

    utilities.upsert(
        UtilityKind::Snippet,
        "max-content",
        &"x".repeat(entry::MAX_TITLE_BYTES),
        &"x".repeat(entry::MAX_CONTENT_BYTES),
    )?;
    assert!(
        utilities
            .upsert(
                UtilityKind::Snippet,
                "too-long",
                "Snippet",
                &"x".repeat(entry::MAX_CONTENT_BYTES + 1),
            )
            .is_err()
    );
    assert!(
        utilities
            .upsert(
                UtilityKind::Snippet,
                "bad-title",
                &"x".repeat(entry::MAX_TITLE_BYTES + 1),
                "body",
            )
            .is_err()
    );
    assert!(
        utilities
            .upsert(UtilityKind::Snippet, "invalid/id", "Title", "body")
            .is_err()
    );
    assert!(
        utilities
            .upsert(UtilityKind::Clipboard, "", "Title", "body")
            .is_err()
    );

    std::fs::remove_dir_all(directory)?;
    Ok(())
}
