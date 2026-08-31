use crate::helpers;
use anyhow::{Result, anyhow, bail};
use maestria_daemon::MutationSession;
use maestria_domain::{DomainInput, RetrievalEventsRetired};
use maestria_governance::AutonomyProfile;
use std::io::IsTerminal;
use std::path::PathBuf;

/// Retire retrieval audit events strictly below `before_sequence`
/// (ADR-0009).
///
/// The command emits one governed `RetrievalEventsRetired` marker through
/// the runtime; nothing is deleted. Rows below the boundary stop being
/// decoded at open, and trace lookups report retirement explicitly.
///
/// # Cancellation
/// Dropping this future tears down the CLI-side session (instance lock
/// released, runtime shutdown requested). If the marker input was already
/// accepted by the runtime, it stays durable; re-running the command is
/// safe (a repeat marker below the high-water is a recorded no-op).
pub async fn run(
    instance_dir: PathBuf,
    before_sequence: u64,
    reason: String,
    yes: bool,
) -> Result<()> {
    let layout = helpers::validated_instance(instance_dir)?;
    let database_path = layout.database_path.clone();
    if before_sequence == 0 {
        bail!("--before-sequence must be greater than zero");
    }
    if reason.trim().is_empty() {
        bail!(
            "--reason is required: the retirement marker must record why the audit trail narrows"
        );
    }
    if std::io::stdin().is_terminal() && !yes {
        confirm(before_sequence)?;
    }

    let session = MutationSession::start(layout, AutonomyProfile::TrustedWorkspace).await?;
    session
        .submit(DomainInput::RetrievalEventsRetired(
            RetrievalEventsRetired {
                before_sequence,
                reason: reason.clone(),
            },
        ))
        .await
        .map_err(|error| anyhow!("submit retirement marker: {error}"))?;
    session.finish(Ok(())).await?;
    // The session state is a startup snapshot; the durable high-water is
    // read back from the recorded markers after shutdown.
    let store = maestria_storage_sqlite::SqliteStore::open_read_only(&database_path)?;
    let retired_through = store.retrieval_retired_through()?;
    println!("retired_through={retired_through}");
    Ok(())
}

fn confirm(before_sequence: u64) -> Result<()> {
    use std::io::Write;
    print!("retire retrieval audit events below sequence {before_sequence}? [y/N] ");
    std::io::stdout().flush()?;
    let mut answer = String::new();
    std::io::stdin().read_line(&mut answer)?;
    if !matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes") {
        bail!("retirement cancelled");
    }
    Ok(())
}
