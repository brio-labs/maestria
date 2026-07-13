use maestria_domain::{ApprovalId, LogicalTick, ScopeId, TaskId};
use maestria_ports::{
    ApprovalRecord, ApprovalRepository, ApprovalRiskLevel, ApprovalStatus, PortError,
};
use rusqlite::params;

use crate::{to_port_error, u64_to_i64};

fn risk_to_text(level: ApprovalRiskLevel) -> &'static str {
    match level {
        ApprovalRiskLevel::Low => "low",
        ApprovalRiskLevel::Medium => "medium",
        ApprovalRiskLevel::High => "high",
        ApprovalRiskLevel::Critical => "critical",
    }
}

fn risk_from_text(text: &str) -> Result<ApprovalRiskLevel, PortError> {
    match text {
        "low" => Ok(ApprovalRiskLevel::Low),
        "medium" => Ok(ApprovalRiskLevel::Medium),
        "high" => Ok(ApprovalRiskLevel::High),
        "critical" => Ok(ApprovalRiskLevel::Critical),
        other => Err(PortError::Internal {
            message: format!("unknown approval risk level: {other}"),
        }),
    }
}

fn status_to_text(status: ApprovalStatus) -> &'static str {
    match status {
        ApprovalStatus::Pending => "pending",
        ApprovalStatus::Approved => "approved",
        ApprovalStatus::Denied => "denied",
    }
}

fn status_from_text(text: &str) -> Result<ApprovalStatus, PortError> {
    match text {
        "pending" => Ok(ApprovalStatus::Pending),
        "approved" => Ok(ApprovalStatus::Approved),
        "denied" => Ok(ApprovalStatus::Denied),
        other => Err(PortError::Internal {
            message: format!("unknown approval status: {other}"),
        }),
    }
}

fn read_approval_row(row: &rusqlite::Row<'_>) -> Result<ApprovalRecord, rusqlite::Error> {
    let id: i64 = row.get(0)?;
    let task_id: i64 = row.get(1)?;
    let effect_kind: String = row.get(2)?;
    let risk_text: String = row.get(3)?;
    let capability: String = row.get(4)?;
    let scope_id: i64 = row.get(5)?;
    let tick: i64 = row.get(6)?;
    let status_text: String = row.get(7)?;

    let id = u64::try_from(id).map_err(|_| rusqlite::Error::IntegralValueOutOfRange(0, id))?;
    let task_id =
        u64::try_from(task_id).map_err(|_| rusqlite::Error::IntegralValueOutOfRange(1, task_id))?;
    let scope_id = u64::try_from(scope_id)
        .map_err(|_| rusqlite::Error::IntegralValueOutOfRange(5, scope_id))?;
    let tick =
        u64::try_from(tick).map_err(|_| rusqlite::Error::IntegralValueOutOfRange(6, tick))?;

    let risk_level = risk_from_text(&risk_text).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(3, rusqlite::types::Type::Text, Box::new(e))
    })?;
    let status = status_from_text(&status_text).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(7, rusqlite::types::Type::Text, Box::new(e))
    })?;

    Ok(ApprovalRecord {
        id: ApprovalId::new(id),
        task_id: TaskId::new(task_id),
        effect_kind,
        risk_level,
        capability,
        scope_id: ScopeId::new(scope_id),
        tick: LogicalTick::new(tick),
        status,
    })
}

impl ApprovalRepository for crate::SqliteStore {
    fn save(&self, record: &ApprovalRecord) -> Result<(), PortError> {
        let connection = self.lock()?;
        connection
            .execute(
                "INSERT OR REPLACE INTO approval_requests \
                 (id, task_id, effect_kind, risk_level, capability, scope_id, tick, status) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    u64_to_i64(record.id.value())?,
                    u64_to_i64(record.task_id.value())?,
                    record.effect_kind,
                    risk_to_text(record.risk_level),
                    record.capability,
                    u64_to_i64(record.scope_id.value())?,
                    u64_to_i64(record.tick.value())?,
                    status_to_text(record.status),
                ],
            )
            .map_err(to_port_error)?;
        Ok(())
    }

    fn find_pending(&self) -> Result<Vec<ApprovalRecord>, PortError> {
        let connection = self.lock()?;
        let mut stmt = connection
            .prepare(
                "SELECT id, task_id, effect_kind, risk_level, capability, scope_id, tick, status \
                 FROM approval_requests WHERE status = 'pending' ORDER BY id",
            )
            .map_err(to_port_error)?;
        let rows = stmt
            .query_map([], read_approval_row)
            .map_err(to_port_error)?;
        let mut records = Vec::new();
        for row in rows {
            records.push(row.map_err(to_port_error)?);
        }
        Ok(records)
    }

    fn find_by_id(&self, id: ApprovalId) -> Result<Option<ApprovalRecord>, PortError> {
        let connection = self.lock()?;
        let mut stmt = connection
            .prepare(
                "SELECT id, task_id, effect_kind, risk_level, capability, scope_id, tick, status \
                 FROM approval_requests WHERE id = ?1",
            )
            .map_err(to_port_error)?;
        let mut rows = stmt
            .query_map(params![u64_to_i64(id.value())?], read_approval_row)
            .map_err(to_port_error)?;
        match rows.next() {
            Some(Ok(record)) => Ok(Some(record)),
            Some(Err(e)) => Err(to_port_error(e)),
            None => Ok(None),
        }
    }

    fn resolve(&self, id: ApprovalId, approved: bool) -> Result<Option<ApprovalRecord>, PortError> {
        let connection = self.lock()?;
        let new_status = if approved { "approved" } else { "denied" };
        let affected = connection
            .execute(
                "UPDATE approval_requests SET status = ?1 WHERE id = ?2 AND status = 'pending'",
                params![new_status, u64_to_i64(id.value())?],
            )
            .map_err(to_port_error)?;
        if affected == 0 {
            return Ok(None);
        }
        // Read the updated record within the same lock scope — do NOT
        // call find_by_id here, which would deadlock on the Mutex.
        let mut stmt = connection
            .prepare(
                "SELECT id, task_id, effect_kind, risk_level, capability, scope_id, tick, status \
                 FROM approval_requests WHERE id = ?1",
            )
            .map_err(to_port_error)?;
        let mut rows = stmt
            .query_map(params![u64_to_i64(id.value())?], read_approval_row)
            .map_err(to_port_error)?;
        match rows.next() {
            Some(Ok(record)) => Ok(Some(record)),
            Some(Err(e)) => Err(to_port_error(e)),
            None => Ok(None),
        }
    }

    fn find_by_task_id(&self, task_id: TaskId) -> Result<Vec<ApprovalRecord>, PortError> {
        let connection = self.lock()?;
        let mut stmt = connection
            .prepare(
                "SELECT id, task_id, effect_kind, risk_level, capability, scope_id, tick, status \
                 FROM approval_requests WHERE task_id = ?1 ORDER BY id",
            )
            .map_err(to_port_error)?;
        let rows = stmt
            .query_map(params![u64_to_i64(task_id.value())?], read_approval_row)
            .map_err(to_port_error)?;
        let mut records = Vec::new();
        for row in rows {
            records.push(row.map_err(to_port_error)?);
        }
        Ok(records)
    }
}

#[cfg(test)]
mod tests {
    use maestria_domain::{ApprovalId, LogicalTick, ScopeId, TaskId};
    use maestria_ports::{
        ApprovalRecord, ApprovalRepository, ApprovalRiskLevel, ApprovalStatus, PortError,
    };

    use crate::SqliteStore;

    fn pending_record(id: u64) -> ApprovalRecord {
        ApprovalRecord {
            id: ApprovalId::new(id),
            task_id: TaskId::new(100 + id),
            effect_kind: "task_activation".to_string(),
            risk_level: ApprovalRiskLevel::Medium,
            capability: String::new(),
            scope_id: ScopeId::new(0),
            tick: LogicalTick::new(id),
            status: ApprovalStatus::Pending,
        }
    }

    #[test]
    fn save_and_find_pending() -> Result<(), PortError> {
        let store = SqliteStore::in_memory()?;
        let rec = pending_record(1);
        store.save(&rec)?;
        let pending = store.find_pending()?;
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].id, ApprovalId::new(1));
        assert_eq!(pending[0].status, ApprovalStatus::Pending);
        Ok(())
    }

    #[test]
    fn find_by_id_returns_correct_record() -> Result<(), PortError> {
        let store = SqliteStore::in_memory()?;
        store.save(&pending_record(1))?;
        store.save(&pending_record(2))?;

        let found = store.find_by_id(ApprovalId::new(1))?;
        assert!(found.is_some());
        assert_eq!(found.expect("record should exist").id, ApprovalId::new(1));

        let missing = store.find_by_id(ApprovalId::new(99))?;
        assert!(missing.is_none());
        Ok(())
    }

    #[test]
    fn resolve_approve_updates_status() -> Result<(), PortError> {
        let store = SqliteStore::in_memory()?;
        store.save(&pending_record(1))?;

        let resolved = store.resolve(ApprovalId::new(1), true)?;
        assert!(resolved.is_some());
        assert_eq!(
            resolved.expect("record should exist").status,
            ApprovalStatus::Approved
        );

        let pending = store.find_pending()?;
        assert!(pending.is_empty());
        Ok(())
    }

    #[test]
    fn resolve_deny_updates_status() -> Result<(), PortError> {
        let store = SqliteStore::in_memory()?;
        store.save(&pending_record(1))?;

        let resolved = store.resolve(ApprovalId::new(1), false)?;
        assert!(resolved.is_some());
        assert_eq!(
            resolved.expect("record should exist").status,
            ApprovalStatus::Denied
        );
        Ok(())
    }

    #[test]
    fn resolve_already_resolved_is_idempotent() -> Result<(), PortError> {
        let store = SqliteStore::in_memory()?;
        store.save(&pending_record(1))?;

        let first = store.resolve(ApprovalId::new(1), true)?;
        assert!(first.is_some());
        assert_eq!(
            first.expect("record should exist").status,
            ApprovalStatus::Approved
        );

        let second = store.resolve(ApprovalId::new(1), false)?;
        assert!(second.is_none());

        let found = store.find_by_id(ApprovalId::new(1))?;
        assert_eq!(
            found.expect("record should exist").status,
            ApprovalStatus::Approved
        );
        Ok(())
    }

    #[test]
    fn resolve_missing_id_returns_none() -> Result<(), PortError> {
        let store = SqliteStore::in_memory()?;
        let resolved = store.resolve(ApprovalId::new(999), true)?;
        assert!(resolved.is_none());
        Ok(())
    }

    #[test]
    fn find_pending_respects_multiple_records() -> Result<(), PortError> {
        let store = SqliteStore::in_memory()?;
        store.save(&pending_record(1))?;
        store.save(&pending_record(2))?;
        store.save(&pending_record(3))?;

        store.resolve(ApprovalId::new(2), true)?;

        let pending = store.find_pending()?;
        assert_eq!(pending.len(), 2);
        let ids: Vec<u64> = pending.iter().map(|r| r.id.value()).collect();
        assert!(ids.contains(&1));
        assert!(ids.contains(&3));
        assert!(!ids.contains(&2));
        Ok(())
    }

    #[test]
    fn risk_level_roundtrips() -> Result<(), PortError> {
        let store = SqliteStore::in_memory()?;
        for level in &[
            ApprovalRiskLevel::Low,
            ApprovalRiskLevel::Medium,
            ApprovalRiskLevel::High,
            ApprovalRiskLevel::Critical,
        ] {
            let mut rec = pending_record(1);
            rec.risk_level = *level;
            store.save(&rec)?;
            let found = store.find_by_id(ApprovalId::new(1))?;
            assert_eq!(found.expect("record should exist").risk_level, *level);
        }
        Ok(())
    }
}
