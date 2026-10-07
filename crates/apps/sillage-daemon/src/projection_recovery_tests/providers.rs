//! Test embedding providers for projection recovery tests.
use sillage_ports::{EmbeddingProvider, EmbeddingRequest, EmbeddingResponse, PortError};

pub(super) struct RecoveryEmbeddingProvider;

impl EmbeddingProvider for RecoveryEmbeddingProvider {
    fn disclosure(&self) -> sillage_ports::ProviderDisclosure {
        sillage_ports::ProviderDisclosure {
            remote: false,
            retention: sillage_ports::RetentionPolicy::NoRetention,
        }
    }
    fn embed(&self, request: EmbeddingRequest) -> Result<EmbeddingResponse, PortError> {
        let vector = if request.text.contains("first") {
            vec![1.0, 0.0]
        } else {
            vec![0.0, 1.0]
        };
        Ok(EmbeddingResponse {
            vector,
            provider_id: "recovery-provider".to_string(),
            model: request.model,
            model_version: "recovery-v1".to_string(),
            identity: request.identity,
            disclosure: sillage_ports::ProviderDisclosure {
                remote: false,
                retention: sillage_ports::RetentionPolicy::NoRetention,
            },
        })
    }

    fn identity(&self) -> Option<sillage_ports::EmbeddingIdentity> {
        sillage_ports::contract_tests::fixture_embedding_identity("recovery-model", 2).ok()
    }
}

/// Provider that cannot embed the "second chunk" text; proves the vector
/// reconcile degrades per chunk instead of failing the startup path.
pub(super) struct FlakyRecoveryEmbeddingProvider;

impl EmbeddingProvider for FlakyRecoveryEmbeddingProvider {
    fn disclosure(&self) -> sillage_ports::ProviderDisclosure {
        sillage_ports::ProviderDisclosure {
            remote: false,
            retention: sillage_ports::RetentionPolicy::NoRetention,
        }
    }
    fn embed(&self, request: EmbeddingRequest) -> Result<EmbeddingResponse, PortError> {
        if request.text.contains("second") {
            return Err(PortError::Downstream {
                message: "model rejects chunk".to_string(),
            });
        }
        Ok(EmbeddingResponse {
            vector: vec![1.0, 0.0],
            provider_id: "recovery-provider".to_string(),
            model: request.model,
            model_version: "recovery-v1".to_string(),
            identity: request.identity,
            disclosure: sillage_ports::ProviderDisclosure {
                remote: false,
                retention: sillage_ports::RetentionPolicy::NoRetention,
            },
        })
    }
    fn identity(&self) -> Option<sillage_ports::EmbeddingIdentity> {
        sillage_ports::contract_tests::fixture_embedding_identity("recovery-model", 2).ok()
    }
}

pub(super) struct CountingEmbeddingProvider {
    pub(super) calls: std::sync::atomic::AtomicUsize,
}

impl CountingEmbeddingProvider {
    pub(super) fn new() -> Self {
        Self {
            calls: std::sync::atomic::AtomicUsize::new(0),
        }
    }
}

impl EmbeddingProvider for CountingEmbeddingProvider {
    fn disclosure(&self) -> sillage_ports::ProviderDisclosure {
        sillage_ports::ProviderDisclosure {
            remote: false,
            retention: sillage_ports::RetentionPolicy::NoRetention,
        }
    }
    fn embed(&self, request: EmbeddingRequest) -> Result<EmbeddingResponse, PortError> {
        self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(EmbeddingResponse {
            vector: vec![1.0, 0.0],
            provider_id: "recovery-provider".to_string(),
            model: request.model,
            model_version: "recovery-v1".to_string(),
            identity: request.identity,
            disclosure: sillage_ports::ProviderDisclosure {
                remote: false,
                retention: sillage_ports::RetentionPolicy::NoRetention,
            },
        })
    }
    fn identity(&self) -> Option<sillage_ports::EmbeddingIdentity> {
        sillage_ports::contract_tests::fixture_embedding_identity("recovery-model", 2).ok()
    }
}
