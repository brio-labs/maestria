use sillage_ports::PortError;
use std::{
    error::Error,
    fmt,
    sync::{Arc, Mutex, MutexGuard},
};

#[cfg(test)]
use std::sync::Barrier;

#[cfg(test)]
#[derive(Debug)]
struct ReserveBeforeLedgerLockHook {
    entered: Barrier,
    resume: Barrier,
}

struct MemoryLedger {
    used_bytes: u64,
    accounting_corrupted: bool,
}

/// Shared admission quota for session-owned state and declared native memory estimates.
pub struct RamMemoryBudget {
    limit_bytes: u64,
    ledger: Mutex<MemoryLedger>,
    #[cfg(test)]
    reserve_before_lock: Mutex<Option<Arc<ReserveBeforeLedgerLockHook>>>,
}

impl fmt::Debug for RamMemoryBudget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Drop the guard before formatter callbacks, which may reenter this budget.
        let (used_bytes, accounting_corrupted) = {
            let mut ledger = match self.ledger.lock() {
                Ok(ledger) => ledger,
                Err(poisoned) => {
                    let mut ledger = poisoned.into_inner();
                    ledger.accounting_corrupted = true;
                    ledger
                }
            };
            if ledger.used_bytes > self.limit_bytes {
                ledger.accounting_corrupted = true;
            }
            (ledger.used_bytes, ledger.accounting_corrupted)
        };

        f.debug_struct("RamMemoryBudget")
            .field("limit_bytes", &self.limit_bytes)
            .field("used_bytes", &used_bytes)
            .field("accounting_corrupted", &accounting_corrupted)
            .finish()
    }
}

impl RamMemoryBudget {
    /// Create an admission ledger with the supplied quota; native estimates are not RSS.
    pub fn new(limit_bytes: u64) -> Self {
        Self {
            limit_bytes,
            ledger: Mutex::new(MemoryLedger {
                used_bytes: 0,
                accounting_corrupted: false,
            }),
            #[cfg(test)]
            reserve_before_lock: Mutex::new(None),
        }
    }

    fn lock_ledger(&self) -> Result<MutexGuard<'_, MemoryLedger>, MemoryBudgetError> {
        let mut ledger = match self.ledger.lock() {
            Ok(ledger) => ledger,
            Err(poisoned) => {
                let mut ledger = poisoned.into_inner();
                ledger.accounting_corrupted = true;
                return Err(MemoryBudgetError::AccountingCorrupted);
            }
        };
        if ledger.accounting_corrupted {
            return Err(MemoryBudgetError::AccountingCorrupted);
        }
        if ledger.used_bytes > self.limit_bytes {
            ledger.accounting_corrupted = true;
            return Err(MemoryBudgetError::AccountingCorrupted);
        }
        Ok(ledger)
    }

    /// Bytes still available for a new reservation; zero after an accounting fault.
    pub fn available_bytes(&self) -> u64 {
        match self.lock_ledger() {
            Ok(ledger) => self.limit_bytes - ledger.used_bytes,
            Err(_) => 0,
        }
    }

    /// The configured limit for this shared ledger.
    pub fn limit_bytes(&self) -> u64 {
        self.limit_bytes
    }

    /// Charged admission bytes, including conservative opaque-native estimates; not RSS.
    pub fn used_bytes(&self) -> u64 {
        let mut ledger = match self.ledger.lock() {
            Ok(ledger) => ledger,
            Err(poisoned) => {
                let mut ledger = poisoned.into_inner();
                ledger.accounting_corrupted = true;
                ledger
            }
        };
        if ledger.used_bytes > self.limit_bytes {
            ledger.accounting_corrupted = true;
        }
        ledger.used_bytes
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
        #[cfg(test)]
        self.pause_reserve_before_lock()?;

        let mut ledger = self.lock_ledger()?;
        let next = ledger
            .used_bytes
            .checked_add(requested)
            .ok_or(MemoryBudgetError::AccountingOverflow)?;
        let available = self.limit_bytes - ledger.used_bytes;
        if requested > available {
            return Err(MemoryBudgetError::Exhausted {
                requested,
                available,
            });
        }
        ledger.used_bytes = next;
        Ok(())
    }

    #[cfg(test)]
    fn pause_reserve_before_lock(&self) -> Result<(), MemoryBudgetError> {
        let hook = match self.reserve_before_lock.lock() {
            Ok(mut hook) => hook.take(),
            Err(_) => return Err(MemoryBudgetError::AccountingCorrupted),
        };
        if let Some(hook) = hook {
            let _ = hook.entered.wait();
            let _ = hook.resume.wait();
        }
        Ok(())
    }

    fn resize_bytes(&self, reserved: u64, requested: u64) -> Result<(), MemoryBudgetError> {
        let mut ledger = self.lock_ledger()?;
        if requested > reserved {
            let additional = requested - reserved;
            let next = ledger
                .used_bytes
                .checked_add(additional)
                .ok_or(MemoryBudgetError::AccountingOverflow)?;
            let available = self.limit_bytes - ledger.used_bytes;
            if additional > available {
                return Err(MemoryBudgetError::Exhausted {
                    requested: additional,
                    available,
                });
            }
            ledger.used_bytes = next;
        } else if requested < reserved {
            let released = reserved - requested;
            let Some(next) = ledger.used_bytes.checked_sub(released) else {
                ledger.accounting_corrupted = true;
                return Err(MemoryBudgetError::AccountingCorrupted);
            };
            ledger.used_bytes = next;
        }
        Ok(())
    }

    fn release_bytes(&self, bytes: u64) -> Result<(), MemoryBudgetError> {
        if bytes == 0 {
            return Ok(());
        }
        let mut ledger = self.lock_ledger()?;
        let Some(next) = ledger.used_bytes.checked_sub(bytes) else {
            ledger.accounting_corrupted = true;
            return Err(MemoryBudgetError::AccountingCorrupted);
        };
        ledger.used_bytes = next;
        Ok(())
    }
}

/// A typed refusal or accounting failure from the shared RAM memory ledger.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryBudgetError {
    /// The additional request exceeds the bytes currently available.
    Exhausted { requested: u64, available: u64 },
    /// A byte count could not be represented without overflow.
    AccountingOverflow,
    /// A ledger invariant failed; further admission is refused.
    AccountingCorrupted,
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
            Self::AccountingCorrupted => f.write_str("RAM memory accounting invariant failed"),
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
        self.budget.resize_bytes(self.bytes, bytes)?;
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
        // Drop cannot return the fault; the latch reports it at the next reserve or resize.
        let _ = self.budget.release_bytes(self.bytes);
    }
}
#[cfg(test)]
mod invariant_tests {
    use super::{MemoryBudgetError, RamMemoryBudget, ReserveBeforeLedgerLockHook};
    use std::{
        error::Error,
        sync::{Arc, Barrier},
    };

    #[test]
    fn resize_underflow_is_typed_nonwrapping_and_poisoned_budget_refuses()
    -> Result<(), Box<dyn Error>> {
        let budget = Arc::new(RamMemoryBudget::new(8));
        let mut reservation = budget.reserve(3)?;
        reservation.bytes = 4;

        assert!(matches!(
            reservation.resize(0),
            Err(MemoryBudgetError::AccountingCorrupted)
        ));
        assert_eq!(reservation.reserved_bytes(), 4);
        assert_eq!(budget.used_bytes(), 3);
        assert_eq!(budget.available_bytes(), 0);
        assert!(matches!(
            reservation.resize(4),
            Err(MemoryBudgetError::AccountingCorrupted)
        ));
        assert!(matches!(
            reservation.resize(5),
            Err(MemoryBudgetError::AccountingCorrupted)
        ));
        assert!(matches!(
            reservation.resize(2),
            Err(MemoryBudgetError::AccountingCorrupted)
        ));
        assert_eq!(reservation.reserved_bytes(), 4);
        assert_eq!(budget.used_bytes(), 3);
        assert!(matches!(
            budget.reserve(0),
            Err(MemoryBudgetError::AccountingCorrupted)
        ));
        drop(reservation);
        assert_eq!(budget.used_bytes(), 3);
        Ok(())
    }

    #[test]
    fn drop_underflow_is_typed_at_the_next_admission() -> Result<(), Box<dyn Error>> {
        let budget = Arc::new(RamMemoryBudget::new(8));
        let mut reservation = budget.reserve(3)?;
        reservation.bytes = 4;
        drop(reservation);

        assert_eq!(budget.used_bytes(), 3);
        assert_eq!(budget.available_bytes(), 0);
        assert!(matches!(
            budget.reserve(0),
            Err(MemoryBudgetError::AccountingCorrupted)
        ));
        Ok(())
    }

    #[test]
    fn reserve_refuses_fault_committed_before_ledger_lock() -> Result<(), Box<dyn Error>> {
        let budget = Arc::new(RamMemoryBudget::new(8));
        let seed = budget.reserve(3)?;
        let mut invalid_drop = budget.reserve(0)?;
        invalid_drop.bytes = 4;

        let hook = Arc::new(ReserveBeforeLedgerLockHook {
            entered: Barrier::new(2),
            resume: Barrier::new(2),
        });
        let mut hook_slot = budget
            .reserve_before_lock
            .lock()
            .map_err(|_| std::io::Error::other("test reserve gate mutex was poisoned"))?;
        *hook_slot = Some(Arc::clone(&hook));
        drop(hook_slot);

        let admission_budget = Arc::clone(&budget);
        let admission = std::thread::spawn(move || admission_budget.reserve(2));

        let _ = hook.entered.wait();
        drop(invalid_drop);
        let _ = hook.resume.wait();

        let result = admission
            .join()
            .map_err(|_| std::io::Error::other("reserve worker panicked"))?;
        let reported_fault = matches!(&result, Err(MemoryBudgetError::AccountingCorrupted));
        assert_eq!(
            (reported_fault, budget.used_bytes(), seed.reserved_bytes()),
            (true, 3, 3),
            "reserve must observe a fault committed before it acquires the ledger authority"
        );
        drop(result);
        Ok(())
    }

    #[test]
    fn full_u64_quota_and_accounting_boundary_are_preserved() -> Result<(), Box<dyn Error>> {
        let budget = Arc::new(RamMemoryBudget::new(u64::MAX));
        let mut reservation = budget.reserve(u64::MAX - 1)?;

        reservation.resize(u64::MAX)?;
        assert_eq!(reservation.reserved_bytes(), u64::MAX);
        assert_eq!(budget.used_bytes(), u64::MAX);
        assert_eq!(budget.available_bytes(), 0);
        assert!(matches!(
            budget.reserve(1),
            Err(MemoryBudgetError::AccountingOverflow)
        ));

        reservation.resize(0)?;
        assert_eq!(budget.used_bytes(), 0);
        Ok(())
    }
}
