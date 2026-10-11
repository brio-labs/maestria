use sillage_ports::PortError;
use std::{
    error::Error,
    fmt,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

/// Shared admission quota for session-owned state and declared native memory estimates.
#[derive(Debug)]
pub struct RamMemoryBudget {
    limit_bytes: u64,
    used_bytes: AtomicU64,
}

impl RamMemoryBudget {
    /// Create an admission ledger with the supplied quota; native estimates are not RSS.
    pub fn new(limit_bytes: u64) -> Self {
        Self {
            limit_bytes,
            used_bytes: AtomicU64::new(0),
        }
    }

    /// Bytes still available for a new reservation.
    pub fn available_bytes(&self) -> u64 {
        self.limit_bytes
            .saturating_sub(self.used_bytes.load(Ordering::Acquire))
    }
    /// The configured limit for this shared ledger.
    pub fn limit_bytes(&self) -> u64 {
        self.limit_bytes
    }

    /// Charged admission bytes, including conservative opaque-native estimates; not RSS.
    pub fn used_bytes(&self) -> u64 {
        self.used_bytes.load(Ordering::Acquire)
    }
    /// Reserve bytes before allocating or retaining the corresponding memory.
    pub fn reserve(
        self: &Arc<Self>,
        bytes: u64,
    ) -> Result<RamMemoryReservation, MemoryBudgetError> {
        self.reserve_bytes(bytes)?;
        Ok(RamMemoryReservation {
            budget: Arc::clone(self),
            bytes,
        })
    }

    fn reserve_bytes(&self, requested: u64) -> Result<(), MemoryBudgetError> {
        loop {
            let used = self.used_bytes.load(Ordering::Acquire);
            let next = used
                .checked_add(requested)
                .ok_or(MemoryBudgetError::AccountingOverflow)?;
            let available = self.limit_bytes.saturating_sub(used);
            if requested > available {
                return Err(MemoryBudgetError::Exhausted {
                    requested,
                    available,
                });
            }
            match self.used_bytes.compare_exchange_weak(
                used,
                next,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => return Ok(()),
                Err(_) => continue,
            }
        }
    }

    fn release_bytes(&self, bytes: u64) {
        if bytes != 0 {
            let previous = self.used_bytes.fetch_sub(bytes, Ordering::AcqRel);
            debug_assert!(previous >= bytes);
        }
    }
}

/// A typed refusal or arithmetic failure from the shared RAM memory ledger.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryBudgetError {
    /// The additional request exceeds the bytes currently available.
    Exhausted { requested: u64, available: u64 },
    /// A byte count could not be represented without overflow.
    AccountingOverflow,
}

impl fmt::Display for MemoryBudgetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Exhausted {
                requested,
                available,
            } => write!(
                f,
                "RAM admission exhausted: requested {requested} bytes with {available} available"
            ),
            Self::AccountingOverflow => f.write_str("RAM memory accounting overflow"),
        }
    }
}

impl Error for MemoryBudgetError {}

/// A RAM-index construction or indexing failure, preserving memory admission causes.
#[derive(Debug)]
pub enum InMemoryIndexError {
    /// The selected shared RAM quota refused an allocation.
    MemoryBudget(MemoryBudgetError),
    /// Tantivy or the canonical index rejected the operation.
    Index(PortError),
}

impl fmt::Display for InMemoryIndexError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MemoryBudget(error) => write!(f, "{error}"),
            Self::Index(error) => write!(f, "{error}"),
        }
    }
}

impl Error for InMemoryIndexError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::MemoryBudget(error) => Some(error),
            Self::Index(error) => Some(error),
        }
    }
}

impl From<MemoryBudgetError> for InMemoryIndexError {
    fn from(error: MemoryBudgetError) -> Self {
        Self::MemoryBudget(error)
    }
}

impl From<PortError> for InMemoryIndexError {
    fn from(error: PortError) -> Self {
        Self::Index(error)
    }
}

/// RAII reservation; dropping it releases its bytes back to the shared budget.
#[derive(Debug)]
pub struct RamMemoryReservation {
    budget: Arc<RamMemoryBudget>,
    bytes: u64,
}

impl RamMemoryReservation {
    /// Grow before allocation; shrink only after the associated memory is no longer owned.
    pub fn resize(&mut self, bytes: u64) -> Result<(), MemoryBudgetError> {
        if bytes > self.bytes {
            self.budget.reserve_bytes(bytes - self.bytes)?;
        } else {
            self.budget.release_bytes(self.bytes - bytes);
        }
        self.bytes = bytes;
        Ok(())
    }

    /// The number of bytes currently reserved by this handle.
    pub fn reserved_bytes(&self) -> u64 {
        self.bytes
    }
    /// Move charged bytes between handles without changing global availability.
    pub(crate) fn split_off(&mut self, bytes: u64) -> Option<Self> {
        if bytes > self.bytes {
            return None;
        }
        self.bytes -= bytes;
        Some(Self {
            budget: Arc::clone(&self.budget),
            bytes,
        })
    }

    /// Whether a reservation can be absorbed without changing the budget identity.
    pub(crate) fn can_merge(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.budget, &other.budget) && self.bytes.checked_add(other.bytes).is_some()
    }

    /// Absorb a reservation without releasing or acquiring charged bytes.
    pub(crate) fn merge(&mut self, mut other: Self) -> Result<(), Self> {
        let Some(next) = self.bytes.checked_add(other.bytes) else {
            return Err(other);
        };
        if !Arc::ptr_eq(&self.budget, &other.budget) {
            return Err(other);
        }
        self.bytes = next;
        other.bytes = 0;
        Ok(())
    }
}

impl Drop for RamMemoryReservation {
    fn drop(&mut self) {
        self.budget.release_bytes(self.bytes);
    }
}
