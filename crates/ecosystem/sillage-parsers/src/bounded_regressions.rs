use super::*;

#[test]
fn bounded_copy_charges_utf8_validation_and_copy_separately() {
    let source = "café β".as_bytes();
    let exact_work = (source.len() * 2) as u64;
    {
        let mut continue_parsing = || ParseCheckpoint::Continue;
        let mut exact_budget = ParseBudget::new(exact_work, source.len(), &mut continue_parsing);
        let exact_copy = crate::bounded_output::copy_bytes_range_bounded(
            source,
            0..source.len(),
            &mut exact_budget,
        );
        assert_eq!(exact_copy, Ok("café β".to_string()));
        assert_eq!(exact_budget.work_units_used(), exact_work);
    }

    {
        let mut continue_parsing = || ParseCheckpoint::Continue;
        let mut short_budget =
            ParseBudget::new(exact_work - 1, source.len(), &mut continue_parsing);
        let short_copy = crate::bounded_output::copy_bytes_range_bounded(
            source,
            0..source.len(),
            &mut short_budget,
        );
        assert!(matches!(
            short_copy,
            Err(BoundedParseError::Budget(
                ParseBudgetFailure::WorkLimitExceeded { .. }
            ))
        ));
    }
}

#[test]
fn parse_budget_rejects_work_reservations_larger_than_checkpoint_interval() {
    let mut continue_parsing = || ParseCheckpoint::Continue;
    let mut budget = ParseBudget::new(u64::MAX, usize::MAX, &mut continue_parsing);
    assert!(budget.charge_work(ParseBudget::CHECKPOINT_INTERVAL).is_ok());
    assert_eq!(budget.work_units_used(), ParseBudget::CHECKPOINT_INTERVAL);
    assert!(matches!(
        budget.charge_work(ParseBudget::CHECKPOINT_INTERVAL + 1),
        Err(ParseBudgetFailure::WorkChargeTooLarge {
            max: ParseBudget::CHECKPOINT_INTERVAL,
            requested
        }) if requested == ParseBudget::CHECKPOINT_INTERVAL + 1
    ));
}

#[test]
fn bounded_markdown_preserves_multibyte_scalars_at_segment_boundaries() -> Result<(), Box<dyn Error>>
{
    for (prefix_len, id) in [(127, 38), (128, 39), (129, 40)] {
        let mut bytes = vec![b'a'; prefix_len];
        bytes.extend_from_slice("β".as_bytes());
        let file = handle("boundary.md", &bytes);
        let parser = MarkdownParser::new();
        let expected = parser.parse(file.clone(), context(id))?;
        let mut continue_parsing = || ParseCheckpoint::Continue;
        let mut budget = ParseBudget::new(100_000, 100_000, &mut continue_parsing);
        let outcome = parser.parse_outcome_bounded(
            FileHandleRef::new(&file.path, &file.bytes),
            context(id),
            &mut budget,
        )?;
        let actual = match outcome {
            ParseOutcome::Complete(parsed) => parsed,
            ParseOutcome::NeedsOcr { .. } => {
                return Err(Box::new(PortError::internal(
                    "bounded Markdown boundary test",
                    "Markdown unexpectedly requested OCR",
                )));
            }
        };
        assert_eq!(actual, expected);
    }
    Ok(())
}

#[test]
fn bounded_markdown_preserves_paragraph_blank_line_and_unterminated_eof()
-> Result<(), Box<dyn Error>> {
    let file = handle("paragraph.md", "alpha\r\n \t\r\nbeta λ".as_bytes());
    let parser = MarkdownParser::new();
    let expected = parser.parse(file.clone(), context(41))?;
    let mut continue_parsing = || ParseCheckpoint::Continue;
    let mut budget = ParseBudget::new(100_000, 100_000, &mut continue_parsing);
    let outcome = parser.parse_outcome_bounded(
        FileHandleRef::new(&file.path, &file.bytes),
        context(41),
        &mut budget,
    )?;
    let actual = match outcome {
        ParseOutcome::Complete(parsed) => parsed,
        ParseOutcome::NeedsOcr { .. } => {
            return Err(Box::new(PortError::internal(
                "bounded Markdown paragraph test",
                "Markdown unexpectedly requested OCR",
            )));
        }
    };
    assert_eq!(actual, expected);
    assert_eq!(actual.chunks[0].text, "alpha");
    assert_eq!(actual.chunks[0].representations[0].content, "alpha\r\n");
    assert_eq!(actual.chunks[1].text, "beta λ");
    Ok(())
}

#[test]
fn bounded_summary_admits_long_path_capacity_at_exact_output_limit() -> Result<(), Box<dyn Error>> {
    let path = PathBuf::from(format!("{}.md", "p".repeat(4096)));
    let bytes = b"# title\n";
    let parser = MarkdownParser::new();
    let output_used = {
        let mut continue_parsing = || ParseCheckpoint::Continue;
        let mut measured = ParseBudget::new(1_000_000, 1_000_000, &mut continue_parsing);
        let outcome = parser.parse_outcome_bounded(
            FileHandleRef::new(&path, bytes),
            context(42),
            &mut measured,
        )?;
        let parsed = match outcome {
            ParseOutcome::Complete(parsed) => parsed,
            ParseOutcome::NeedsOcr { .. } => {
                return Err(Box::new(PortError::internal(
                    "bounded Markdown long-path test",
                    "Markdown unexpectedly requested OCR",
                )));
            }
        };
        let output_used = measured.output_bytes_used();
        assert!(output_used >= parsed.cards[0].card.body.capacity());
        output_used
    };

    {
        let mut continue_parsing = || ParseCheckpoint::Continue;
        let mut exact = ParseBudget::new(1_000_000, output_used, &mut continue_parsing);
        let exact_result =
            parser.parse_outcome_bounded(FileHandleRef::new(&path, bytes), context(42), &mut exact);
        assert!(matches!(exact_result, Ok(ParseOutcome::Complete(_))));
    }

    {
        let mut continue_parsing = || ParseCheckpoint::Continue;
        let mut below = ParseBudget::new(1_000_000, output_used - 1, &mut continue_parsing);
        let below_result =
            parser.parse_outcome_bounded(FileHandleRef::new(&path, bytes), context(42), &mut below);
        assert!(matches!(
            below_result,
            Err(BoundedParseError::Budget(
                ParseBudgetFailure::OutputLimitExceeded { .. }
            ))
        ));
    }
    Ok(())
}

#[test]
fn bounded_registry_budgets_routing_paths_before_dispatch() {
    let registry = ParserRegistry::with_defaults();
    let supported = PathBuf::from(format!("{}.md", "s".repeat(4096)));
    let unsupported = PathBuf::from(format!("notes.{}", "x".repeat(4096)));
    for path in [&supported, &unsupported] {
        let mut continue_parsing = || ParseCheckpoint::Continue;
        let mut budget = ParseBudget::new(0, 0, &mut continue_parsing);
        let result = registry.parse_outcome_bounded(
            FileHandleRef::new(path, b"# title\n"),
            context(43),
            &mut budget,
        );
        assert!(matches!(
            result,
            Err(BoundedParseError::Budget(
                ParseBudgetFailure::WorkLimitExceeded { .. }
            ))
        ));
    }
    for path in [&supported, &unsupported] {
        let mut calls = 0;
        {
            let mut cancel_at_entry = || {
                calls += 1;
                ParseCheckpoint::Cancelled
            };
            let mut budget = ParseBudget::new(0, 0, &mut cancel_at_entry);
            let result = registry.parse_outcome_bounded(
                FileHandleRef::new(path, b"# title\n"),
                context(44),
                &mut budget,
            );
            assert!(matches!(
                result,
                Err(BoundedParseError::Budget(ParseBudgetFailure::Cancelled))
            ));
        }
        assert_eq!(calls, 1);
    }
}

#[test]
fn bounded_registry_matches_normalized_trailing_markdown_paths() -> Result<(), Box<dyn Error>> {
    let registry = ParserRegistry::with_defaults();
    for (id, path) in [(80, "guide.md/"), (81, "guide.md/.")] {
        let file = handle(path, b"# title\n");
        let expected = registry.parse(file.clone(), context(id))?;
        let actual = {
            let mut continue_parsing = || ParseCheckpoint::Continue;
            let mut budget = ParseBudget::new(100_000, 100_000, &mut continue_parsing);
            registry.parse_outcome_bounded(
                FileHandleRef::new(&file.path, &file.bytes),
                context(id),
                &mut budget,
            )?
        };
        let ParseOutcome::Complete(actual) = actual else {
            return Err(Box::new(PortError::internal(
                "bounded Markdown routing regression",
                "Markdown unexpectedly requested OCR",
            )));
        };
        assert_eq!(actual, expected);
    }
    Ok(())
}

#[test]
fn bounded_summary_fallback_titles_match_path_filename_semantics() -> Result<(), Box<dyn Error>> {
    let parser = MarkdownParser::new();
    for (id, path) in [
        (45, PathBuf::from("guide.md")),
        (46, PathBuf::from("nested/")),
        (47, PathBuf::from("nested/.")),
        (48, PathBuf::from("nested/..")),
        (49, PathBuf::new()),
    ] {
        let file = FileHandle {
            path,
            bytes: b"#\n".to_vec(),
        };
        let expected = parser.parse(file.clone(), context(id))?;
        let mut continue_parsing = || ParseCheckpoint::Continue;
        let mut budget = ParseBudget::new(100_000, 100_000, &mut continue_parsing);
        let outcome = parser.parse_outcome_bounded(
            FileHandleRef::new(&file.path, &file.bytes),
            context(id),
            &mut budget,
        )?;
        let actual = match outcome {
            ParseOutcome::Complete(parsed) => parsed,
            ParseOutcome::NeedsOcr { .. } => {
                return Err(Box::new(PortError::internal(
                    "bounded Markdown fallback-title test",
                    "Markdown unexpectedly requested OCR",
                )));
            }
        };
        assert_eq!(actual, expected);
    }
    Ok(())
}

#[cfg(unix)]
#[test]
fn bounded_summary_uses_artifact_title_for_non_utf8_filename() -> Result<(), Box<dyn Error>> {
    use std::os::unix::ffi::OsStringExt;

    let file = FileHandle {
        path: PathBuf::from(std::ffi::OsString::from_vec(vec![
            b'n', b'a', b'm', b'e', 0xff,
        ])),
        bytes: b"#\n".to_vec(),
    };
    let parser = MarkdownParser::new();
    let expected = parser.parse(file.clone(), context(50))?;
    let mut continue_parsing = || ParseCheckpoint::Continue;
    let mut budget = ParseBudget::new(100_000, 100_000, &mut continue_parsing);
    let outcome = parser.parse_outcome_bounded(
        FileHandleRef::new(&file.path, &file.bytes),
        context(50),
        &mut budget,
    )?;
    let actual = match outcome {
        ParseOutcome::Complete(parsed) => parsed,
        ParseOutcome::NeedsOcr { .. } => {
            return Err(Box::new(PortError::internal(
                "bounded Markdown non-UTF-8 path test",
                "Markdown unexpectedly requested OCR",
            )));
        }
    };
    assert_eq!(actual, expected);
    assert_eq!(actual.cards[0].card.title, "artifact");
    Ok(())
}

#[cfg(unix)]
#[test]
fn bounded_summary_preserves_lossy_filename_replacement_boundaries() -> Result<(), Box<dyn Error>> {
    use std::os::unix::ffi::OsStringExt;

    let paths: [&[u8]; 4] = [
        b"name\xe2a",
        b"name\xf0\x9fa",
        b"name\xe2\x82",
        b"name\xf0\x9f\x92",
    ];
    let parser = MarkdownParser::new();
    for (index, path_bytes) in paths.into_iter().enumerate() {
        let id = 51 + index as u64;
        let file = FileHandle {
            path: PathBuf::from(std::ffi::OsString::from_vec(path_bytes.to_vec())),
            bytes: b"#\n".to_vec(),
        };
        let expected = parser.parse(file.clone(), context(id))?;
        let mut continue_parsing = || ParseCheckpoint::Continue;
        let mut budget = ParseBudget::new(100_000, 100_000, &mut continue_parsing);
        let outcome = parser.parse_outcome_bounded(
            FileHandleRef::new(&file.path, &file.bytes),
            context(id),
            &mut budget,
        )?;
        let actual = match outcome {
            ParseOutcome::Complete(parsed) => parsed,
            ParseOutcome::NeedsOcr { .. } => {
                return Err(Box::new(PortError::internal(
                    "bounded Markdown lossy-path test",
                    "Markdown unexpectedly requested OCR",
                )));
            }
        };
        assert_eq!(actual, expected);
    }
    Ok(())
}
