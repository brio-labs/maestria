use crate::types::*;
use crate::{GrantTokenDigest, RealmId};

impl KernelState {
    pub(crate) fn process_issue_realm_read_grant(
        &mut self,
        input: IssueRealmReadGrantInput,
    ) -> Result<KernelOutput, DomainError> {
        self.apply_realm_read_grant_issued(&input.grant)?;
        let envelope = self.emit_event(DomainEvent::RealmReadGrantIssued { grant: input.grant });
        Ok(persisted_event_output(envelope))
    }

    pub(crate) fn process_revoke_realm_read_grant(
        &mut self,
        input: RevokeRealmReadGrantInput,
    ) -> Result<KernelOutput, DomainError> {
        self.apply_realm_read_grant_revoked(&input.token_digest)?;
        let envelope = self.emit_event(DomainEvent::RealmReadGrantRevoked {
            token_digest: input.token_digest,
        });
        Ok(persisted_event_output(envelope))
    }

    pub(crate) fn process_record_federated_access(
        &mut self,
        input: RecordFederatedAccessInput,
    ) -> Result<KernelOutput, DomainError> {
        self.apply_federated_access_recorded(
            &input.token_digest,
            &input.provider_realm,
            &input.consumer_realm,
            &input.record,
        )?;
        let envelope = self.emit_event(DomainEvent::FederatedReadAccessRecorded {
            token_digest: input.token_digest,
            provider_realm: input.provider_realm,
            consumer_realm: input.consumer_realm,
            record: input.record,
        });
        Ok(persisted_event_output(envelope))
    }

    pub(crate) fn apply_realm_read_grant_issued(
        &mut self,
        grant: &RealmReadGrant,
    ) -> Result<(), DomainError> {
        let digest = grant.token_digest().clone();
        if grant.state() != RealmReadGrantState::Active {
            return Err(DomainError::InternalInvariantViolation {
                detail: "realm read grant issuance must be active",
            });
        }
        if self.realm_read_grants.contains_key(&digest) {
            return Err(DomainError::DuplicateRealmReadGrantDigest { digest });
        }
        if self.realm_read_grants.values().any(|existing| {
            existing.consumer_realm() == grant.consumer_realm()
                && existing.state() == RealmReadGrantState::Active
        }) {
            return Err(DomainError::DuplicateActiveRealmReadGrant {
                consumer_realm: grant.consumer_realm().clone(),
            });
        }
        self.realm_read_grants.insert(digest, grant.clone());
        Ok(())
    }

    pub(crate) fn apply_realm_read_grant_revoked(
        &mut self,
        digest: &GrantTokenDigest,
    ) -> Result<(), DomainError> {
        let grant = self.realm_read_grants.get_mut(digest).ok_or_else(|| {
            DomainError::MissingRealmReadGrant {
                digest: digest.clone(),
            }
        })?;
        if grant.state() == RealmReadGrantState::Revoked {
            return Err(DomainError::RealmReadGrantAlreadyRevoked {
                digest: digest.clone(),
            });
        }
        grant.revoke();
        Ok(())
    }
    pub(crate) fn apply_federated_access_recorded(
        &self,
        digest: &GrantTokenDigest,
        provider_realm: &RealmId,
        consumer_realm: &RealmId,
        record: &FederatedAccessRecord,
    ) -> Result<(), DomainError> {
        let grant = self.realm_read_grants.get(digest).ok_or_else(|| {
            DomainError::MissingRealmReadGrant {
                digest: digest.clone(),
            }
        })?;
        if grant.state() == RealmReadGrantState::Revoked {
            return Err(DomainError::RealmReadGrantRevoked {
                digest: digest.clone(),
            });
        }
        if grant.provider_realm() != provider_realm {
            return Err(DomainError::RealmReadGrantProviderMismatch {
                expected: grant.provider_realm().clone(),
                actual: provider_realm.clone(),
            });
        }
        if grant.consumer_realm() != consumer_realm {
            return Err(DomainError::RealmReadGrantConsumerMismatch {
                expected: grant.consumer_realm().clone(),
                actual: consumer_realm.clone(),
            });
        }
        if matches!(record, FederatedAccessRecord::Evidence { .. })
            && !grant.access().allows_evidence_open()
        {
            return Err(DomainError::RealmReadGrantUnsupportedAccess {
                digest: digest.clone(),
            });
        }
        Ok(())
    }
}

fn persisted_event_output(envelope: DomainEventEnvelope) -> KernelOutput {
    KernelOutput {
        events: vec![envelope.clone()],
        effects: vec![MaestriaEffect::PersistEvent {
            envelope: Box::new(envelope),
        }],
    }
}
