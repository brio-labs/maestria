use std::collections::BTreeSet;

use maestria_domain::{
    FederatedEvidenceBounds, FederatedReadAccess, GrantTokenDigest, RealmId, RealmReadGrant,
    RealmReadGrantState, Sensitivity,
};
use maestria_ports::{PortError, RealmReadGrantRepository};
use rusqlite::{Row, params};

use crate::sqlite_store::{to_port_error, usize_to_i64};

impl RealmReadGrantRepository for crate::SqliteStore {
    fn get(&self, token_digest: &GrantTokenDigest) -> Result<Option<RealmReadGrant>, PortError> {
        let connection = self.lock()?;
        let mut statement = connection
            .prepare_cached(
                "SELECT token_digest, provider_realm, consumer_realm, access, max_sensitivity,
                max_results, max_evidence_bytes, state
         FROM realm_read_grants WHERE token_digest = ?1",
            )
            .map_err(to_port_error)?;
        let mut rows = statement
            .query(params![token_digest.as_str()])
            .map_err(to_port_error)?;
        rows.next()
            .map_err(to_port_error)?
            .map(read_realm_read_grant)
            .transpose()
    }

    fn put(&self, grant: RealmReadGrant) -> Result<(), PortError> {
        let connection = self.lock()?;
        connection
            .execute(
                "INSERT INTO realm_read_grants
                     (token_digest, provider_realm, consumer_realm, access, max_sensitivity,
                      max_results, max_evidence_bytes, state)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                 ON CONFLICT(token_digest) DO UPDATE SET
                     provider_realm = excluded.provider_realm,
                     consumer_realm = excluded.consumer_realm,
                     access = excluded.access,
                     max_sensitivity = excluded.max_sensitivity,
                     max_results = excluded.max_results,
                     max_evidence_bytes = excluded.max_evidence_bytes,
                     state = excluded.state",
                params![
                    grant.token_digest().as_str(),
                    grant.provider_realm().as_str(),
                    grant.consumer_realm().as_str(),
                    access_name(grant.access()),
                    sensitivity_name(grant.max_sensitivity()),
                    usize_to_i64(grant.bounds().max_results())?,
                    usize_to_i64(grant.bounds().max_evidence_bytes())?,
                    state_name(grant.state()),
                ],
            )
            .map(|_| ())
            .map_err(to_port_error)
    }

    fn list(&self) -> Result<Vec<RealmReadGrant>, PortError> {
        let connection = self.lock()?;
        let mut statement = connection
            .prepare_cached(
                "SELECT token_digest, provider_realm, consumer_realm, access, max_sensitivity,
                max_results, max_evidence_bytes, state
         FROM realm_read_grants ORDER BY token_digest ASC",
            )
            .map_err(to_port_error)?;
        let mut rows = statement.query([]).map_err(to_port_error)?;
        let mut grants = Vec::new();
        while let Some(row) = rows.next().map_err(to_port_error)? {
            grants.push(read_realm_read_grant(row)?);
        }
        Ok(grants)
    }

    fn delete_not_in(&self, token_digests: &BTreeSet<GrantTokenDigest>) -> Result<(), PortError> {
        let mut connection = self.lock()?;
        let existing = {
            let mut statement = connection
                .prepare_cached("SELECT token_digest FROM realm_read_grants")
                .map_err(to_port_error)?;
            let mut rows = statement.query([]).map_err(to_port_error)?;
            let mut digests = Vec::new();
            while let Some(row) = rows.next().map_err(to_port_error)? {
                digests.push(row.get::<_, String>(0).map_err(to_port_error)?);
            }
            digests
        };
        let transaction = connection.transaction().map_err(to_port_error)?;
        for digest in existing {
            let digest = GrantTokenDigest::try_from(digest).map_err(|error| {
                PortError::internal(
                    "decode realm read grant digest for cleanup",
                    error.to_string(),
                )
            })?;
            if !token_digests.contains(&digest) {
                transaction
                    .execute(
                        "DELETE FROM realm_read_grants WHERE token_digest = ?1",
                        params![digest.as_str()],
                    )
                    .map_err(to_port_error)?;
            }
        }
        transaction.commit().map_err(to_port_error)
    }
}

fn read_realm_read_grant(row: &Row<'_>) -> Result<RealmReadGrant, PortError> {
    let token_digest = parse_token_digest(row.get(0).map_err(to_port_error)?)?;
    let provider_realm = parse_realm_id(row.get(1).map_err(to_port_error)?, "provider realm")?;
    let consumer_realm = parse_realm_id(row.get(2).map_err(to_port_error)?, "consumer realm")?;
    let access = match row.get::<_, String>(3).map_err(to_port_error)?.as_str() {
        "search_only" => FederatedReadAccess::SearchOnly,
        "search_and_open_evidence" => FederatedReadAccess::SearchAndOpenEvidence,
        value => return Err(invalid("decode realm read grant access", value)),
    };
    let max_sensitivity = match row.get::<_, String>(4).map_err(to_port_error)?.as_str() {
        "public" => Sensitivity::Public,
        "internal" => Sensitivity::Internal,
        "confidential" => Sensitivity::Confidential,
        "restricted" => Sensitivity::Restricted,
        value => return Err(invalid("decode realm read grant sensitivity", value)),
    };
    let max_results = i64_to_usize(row.get(5).map_err(to_port_error)?, "max results")?;
    let max_evidence_bytes =
        i64_to_usize(row.get(6).map_err(to_port_error)?, "max evidence bytes")?;
    let bounds =
        FederatedEvidenceBounds::try_new(max_results, max_evidence_bytes).map_err(|error| {
            PortError::InternalContext {
                context: "decode realm read grant bounds",
                source: error.to_string(),
            }
        })?;
    let state = match row.get::<_, String>(7).map_err(to_port_error)?.as_str() {
        "active" => RealmReadGrantState::Active,
        "revoked" => RealmReadGrantState::Revoked,
        value => return Err(invalid("decode realm read grant state", value)),
    };
    Ok(RealmReadGrant::from_current_state(
        token_digest,
        provider_realm,
        consumer_realm,
        access,
        max_sensitivity,
        bounds,
        state,
    ))
}

fn access_name(access: FederatedReadAccess) -> &'static str {
    match access {
        FederatedReadAccess::SearchOnly => "search_only",
        FederatedReadAccess::SearchAndOpenEvidence => "search_and_open_evidence",
    }
}

fn sensitivity_name(sensitivity: &Sensitivity) -> &'static str {
    match sensitivity {
        Sensitivity::Public => "public",
        Sensitivity::Internal => "internal",
        Sensitivity::Confidential => "confidential",
        Sensitivity::Restricted => "restricted",
    }
}

fn state_name(state: RealmReadGrantState) -> &'static str {
    match state {
        RealmReadGrantState::Active => "active",
        RealmReadGrantState::Revoked => "revoked",
    }
}

fn parse_token_digest(value: String) -> Result<GrantTokenDigest, PortError> {
    GrantTokenDigest::try_from(value)
        .map_err(|error| PortError::internal("decode realm read grant digest", error.to_string()))
}

fn parse_realm_id(value: String, context: &'static str) -> Result<RealmId, PortError> {
    RealmId::try_from(value).map_err(|error| PortError::InternalContext {
        context,
        source: error.to_string(),
    })
}

fn invalid(context: &'static str, source: impl ToString) -> PortError {
    PortError::internal(context, source.to_string())
}
fn i64_to_usize(value: i64, context: &'static str) -> Result<usize, PortError> {
    crate::sqlite_store::i64_to_usize(value)
        .map_err(|_| PortError::internal(context, "stored realm read grant bound is negative"))
}
