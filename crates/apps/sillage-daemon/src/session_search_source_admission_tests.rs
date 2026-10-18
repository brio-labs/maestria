use std::sync::Arc;

use anyhow::{Result, bail};
use sillage_domain::SourceAvailability;
use sillage_search_tantivy::{MemoryBudgetError, RamMemoryBudget};

use super::{SourceLease, source_path_admission_bytes};

#[test]
fn selected_path_lease_reserves_and_releases_the_shared_memory_ledger() -> Result<()> {
    let source = std::env::temp_dir().join(format!(
        ".sillage-source-lease-admission-{}.md",
        std::process::id()
    ));
    let required = source_path_admission_bytes(&source)?;
    let refused_memory = Arc::new(RamMemoryBudget::new(required.saturating_sub(1)));
    let refusal = SourceLease::acquire(&source, SourceAvailability::Enabled, &refused_memory);
    let refused_for_capacity = matches!(
        &refusal,
        Err(error)
            if error.chain().any(|cause| {
                matches!(
                    cause.downcast_ref::<MemoryBudgetError>(),
                    Some(MemoryBudgetError::Exhausted { .. })
                )
            })
    );
    drop(refusal);
    if !refused_for_capacity || refused_memory.used_bytes() != 0 {
        bail!("path admission did not refuse atomically before retaining path ownership");
    }

    let memory = Arc::new(RamMemoryBudget::new(required));
    let lease = SourceLease::acquire(&source, SourceAvailability::Enabled, &memory)?;
    if memory.used_bytes() != required {
        bail!("source lease path ownership was not charged to its shared ledger");
    }
    match memory.reserve(1) {
        Err(MemoryBudgetError::Exhausted { .. }) => {}
        Err(error) => bail!("unexpected shared-ledger refusal: {error}"),
        Ok(reservation) => {
            drop(reservation);
            bail!("source lease path ownership left uncharged ledger capacity");
        }
    }

    let clone = lease.clone_lease();
    drop(lease);
    if memory.used_bytes() != required {
        bail!("cloning or dropping one lease released shared path admission early");
    }

    let competing_memory = Arc::new(RamMemoryBudget::new(required));
    let competing = SourceLease::acquire(&source, SourceAvailability::Enabled, &competing_memory);
    if competing.is_ok() || competing_memory.used_bytes() != 0 {
        bail!("a retained lease clone did not exclude another source owner");
    }

    drop(clone);
    if memory.used_bytes() != 0 {
        bail!("final source lease drop did not release its path reservation");
    }

    let replacement = SourceLease::acquire(&source, SourceAvailability::Enabled, &memory)?;
    if memory.used_bytes() != required {
        bail!("same-source replacement did not reacquire the shared path admission");
    }
    drop(replacement);
    if memory.used_bytes() != 0 {
        bail!("same-source replacement drop did not release its path reservation");
    }
    Ok(())
}
