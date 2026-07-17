use clap::Parser;

use crate::cli_types::{Cli, Commands, EvidenceCommands, IndexCommands, SearchCommands};

#[test]
fn observability_commands_have_exact_nested_grammar() -> Result<(), Box<dyn std::error::Error>> {
    let explain = Cli::try_parse_from(["maestria", "search", "explain", "refresh tokens"])?;
    assert!(matches!(
        explain.command,
        Commands::Search {
            command: Some(SearchCommands::Explain { .. }),
            ..
        }
    ));

    let trace = Cli::try_parse_from(["maestria", "search", "trace", "42"])?;
    assert!(matches!(
        trace.command,
        Commands::Search {
            command: Some(SearchCommands::Trace { trace_id: 42, .. }),
            ..
        }
    ));

    let compare = Cli::try_parse_from(["maestria", "search", "compare", "11", "12"])?;
    assert!(matches!(
        compare.command,
        Commands::Search {
            command: Some(SearchCommands::Compare {
                experiment_a: 11,
                experiment_b: 12,
                ..
            }),
            ..
        }
    ));

    let generations = Cli::try_parse_from(["maestria", "index", "generations"])?;
    assert!(matches!(
        generations.command,
        Commands::Index {
            command: Some(IndexCommands::Generations { .. }),
            ..
        }
    ));

    let coverage = Cli::try_parse_from(["maestria", "evidence", "coverage", "7"])?;
    assert!(matches!(
        coverage.command,
        Commands::Evidence {
            command: EvidenceCommands::Coverage { task_id: 7, .. },
            ..
        }
    ));
    Ok(())
}

#[test]
fn direct_search_and_index_commands_remain_parseable() -> Result<(), Box<dyn std::error::Error>> {
    let search = Cli::try_parse_from(["maestria", "search", "local query"])?;
    assert!(matches!(
        &search.command,
        Commands::Search {
            command: None,
            query: Some(query),
            limit: 10,
            ..
        } if query == "local query"
    ));

    let index = Cli::try_parse_from(["maestria", "index", "--recursive", "notes"])?;
    assert!(matches!(
        &index.command,
        Commands::Index {
            command: None,
            path: Some(path),
            recursive: true,
            ..
        } if path == std::path::Path::new("notes")
    ));

    let reserved_search = Cli::try_parse_from(["maestria", "search", "--", "trace"])?;
    assert!(matches!(
        reserved_search.command,
        Commands::Search {
            command: None,
            query: Some(query),
            ..
        } if query == "trace"
    ));
    let reserved_index = Cli::try_parse_from(["maestria", "index", "--", "generations"])?;
    assert!(matches!(
        reserved_index.command,
        Commands::Index {
            command: None,
            path: Some(path),
            ..
        } if path == std::path::Path::new("generations")
    ));
    Ok(())
}
