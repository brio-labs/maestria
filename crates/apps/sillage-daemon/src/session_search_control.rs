use std::{
    future::Future,
    sync::{
        Arc, Mutex, OnceLock, Weak,
        atomic::{AtomicBool, AtomicU8, Ordering},
    },
};

use anyhow::{Result, bail};
use sillage_domain::SourceAvailability;
use tokio::sync::{Mutex as AsyncMutex, Notify};
use tokio_util::sync::CancellationToken;

use super::{operations::OperationTracker, resources::SessionResourceState};
pub(super) struct SessionControl {
    availability: AtomicU8,
    revoked: AtomicBool,
    retired: AtomicBool,
    operations: Arc<OperationTracker>,
    resources: Mutex<ResourceLink>,
    resource_cleanup: Notify,
    retirement: OnceLock<Arc<RetirementCompletion>>,
}

enum ResourceLink {
    Pending,
    Attached(Weak<SessionResourceState>),
    Clearing,
    Cleared,
}

enum RetirementState {
    Running,
    Finished(Option<Result<()>>),
}

struct RetirementCompletion {
    state: AsyncMutex<RetirementState>,
    changed: Notify,
    task: Mutex<Option<tokio::task::JoinHandle<()>>>,
}

impl RetirementCompletion {
    fn new() -> Self {
        Self {
            state: AsyncMutex::new(RetirementState::Running),
            changed: Notify::new(),
            task: Mutex::new(None),
        }
    }

    fn hold_task(&self, handle: tokio::task::JoinHandle<()>) {
        let mut task = match self.task.lock() {
            Ok(task) => task,
            Err(poisoned) => poisoned.into_inner(),
        };
        *task = Some(handle);
    }

    async fn complete(&self, result: Result<()>) {
        *self.state.lock().await = RetirementState::Finished(Some(result));
        self.changed.notify_waiters();
    }

    async fn wait_for_completion(&self) {
        loop {
            let notified = self.changed.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if matches!(&*self.state.lock().await, RetirementState::Finished(_)) {
                return;
            }
            notified.await;
        }
    }

    async fn take_result(&self) -> Result<()> {
        self.wait_for_completion().await;
        let mut state = self.state.lock().await;
        match &mut *state {
            RetirementState::Finished(result) => match result.take() {
                Some(result) => result,
                None => Ok(()),
            },
            RetirementState::Running => {
                Err(super::SessionLifecycleFailure::ControlResourceLink.into())
            }
        }
    }
}

impl SessionControl {
    pub(super) fn new(availability: SourceAvailability) -> Self {
        Self {
            availability: AtomicU8::new(availability_code(availability)),
            revoked: AtomicBool::new(availability == SourceAvailability::Disabled),
            retired: AtomicBool::new(false),
            operations: Arc::new(OperationTracker::new()),
            resources: Mutex::new(ResourceLink::Pending),
            resource_cleanup: Notify::new(),
            retirement: OnceLock::new(),
        }
    }
    pub(super) fn attach_resources(&self, resources: &Arc<SessionResourceState>) -> Result<()> {
        let mut link = self
            .resources
            .lock()
            .map_err(|_| super::SessionLifecycleFailure::ControlResourceLink)?;
        if self.retired.load(Ordering::Acquire) {
            bail!("cannot attach resources to a retired session");
        }
        if !matches!(*link, ResourceLink::Pending) {
            bail!("session resource ownership was already initialized");
        }
        *link = ResourceLink::Attached(Arc::downgrade(resources));
        Ok(())
    }

    pub(super) fn availability(&self) -> SourceAvailability {
        match self.availability.load(Ordering::Acquire) {
            1 => SourceAvailability::Paused,
            2 => SourceAvailability::Disabled,
            _ => SourceAvailability::Enabled,
        }
    }

    pub(super) fn set_availability(&self, availability: SourceAvailability) {
        self.availability
            .store(availability_code(availability), Ordering::Release);
        if availability == SourceAvailability::Disabled {
            self.revoked.store(true, Ordering::Release);
        }
        if availability != SourceAvailability::Enabled {
            self.operations.cancel_active();
        }
    }

    pub(super) fn ensure_enabled(&self) -> Result<()> {
        if self.retired.load(Ordering::Acquire) {
            bail!("session source has been retired");
        }
        if self.revoked.load(Ordering::Acquire) {
            bail!("session source authorization has been revoked");
        }
        if self.availability() != SourceAvailability::Enabled {
            bail!("session source is not enabled for retrieval");
        }
        Ok(())
    }

    pub(super) async fn retire(self: &Arc<Self>) -> Result<()> {
        self.request_retirement();
        let completion = self.ensure_retirement();
        completion.take_result().await
    }
    pub(super) fn request_retirement(self: &Arc<Self>) {
        self.retired.store(true, Ordering::Release);
        self.revoked.store(true, Ordering::Release);
        self.availability.store(
            availability_code(SourceAvailability::Disabled),
            Ordering::Release,
        );
        self.operations.close_and_cancel();
        self.ensure_retirement();
    }

    fn ensure_retirement(self: &Arc<Self>) -> Arc<RetirementCompletion> {
        self.retirement
            .get_or_init(|| {
                let completion = Arc::new(RetirementCompletion::new());
                let task_completion = completion.clone();
                let control = self.clone();
                let task = tokio::spawn(async move {
                    let result = control.finish_retirement().await;
                    task_completion.complete(result).await;
                });
                completion.hold_task(task);
                completion
            })
            .clone()
    }

    async fn finish_retirement(&self) -> Result<()> {
        let mut failures = self.operations.wait_and_collect_errors().await;
        if let Err(error) = self.clear_resources().await {
            failures.push(error);
        }
        super::operations::errors_result(failures)
    }

    pub(super) async fn wait_for_idle(self: &Arc<Self>) {
        self.operations.wait().await;
        if self.retired.load(Ordering::Acquire) {
            let completion = self.ensure_retirement();
            completion.wait_for_completion().await;
        }
    }

    pub(super) fn can_be_replaced(&self) -> Result<bool> {
        if !self.retired.load(Ordering::Acquire) || !self.operations.is_idle()? {
            return Ok(false);
        }
        let link = self
            .resources
            .lock()
            .map_err(|_| super::SessionLifecycleFailure::ControlResourceLink)?;
        Ok(matches!(*link, ResourceLink::Cleared))
    }

    pub(super) fn can_be_purged(&self) -> Result<bool> {
        self.can_be_replaced()
    }

    pub(super) async fn track<T, F>(
        self: &Arc<Self>,
        cancellation: CancellationToken,
        operation: F,
    ) -> Result<T>
    where
        T: Send + 'static,
        F: Future<Output = Result<T>> + Send + 'static,
    {
        self.ensure_enabled()?;
        self.operations.track(cancellation, operation).await
    }
    async fn clear_resources(&self) -> Result<()> {
        let mut failures = Vec::new();
        loop {
            let notified = self.resource_cleanup.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            let action = {
                let mut link = match self.resources.lock() {
                    Ok(link) => link,
                    Err(poisoned) => {
                        failures.push(super::SessionLifecycleFailure::ControlResourceLink.into());
                        poisoned.into_inner()
                    }
                };
                match &*link {
                    ResourceLink::Pending => {
                        *link = ResourceLink::Clearing;
                        Some(None)
                    }
                    ResourceLink::Attached(resources) => {
                        let resources = resources.clone();
                        *link = ResourceLink::Clearing;
                        Some(Some(resources))
                    }
                    ResourceLink::Clearing => None,
                    ResourceLink::Cleared => {
                        drop(link);
                        return super::operations::errors_result(failures);
                    }
                }
            };
            if let Some(resources) = action {
                let result = resources
                    .and_then(|resources| resources.upgrade())
                    .map_or(Ok(()), |resources| resources.clear());
                if let Err(error) = result {
                    failures.push(error);
                }
                let mut link = match self.resources.lock() {
                    Ok(link) => link,
                    Err(poisoned) => {
                        failures.push(super::SessionLifecycleFailure::ControlResourceLink.into());
                        poisoned.into_inner()
                    }
                };
                *link = ResourceLink::Cleared;
                self.resource_cleanup.notify_waiters();
                return super::operations::errors_result(failures);
            }
            notified.await;
        }
    }
}

fn availability_code(availability: SourceAvailability) -> u8 {
    match availability {
        SourceAvailability::Enabled => 0,
        SourceAvailability::Paused => 1,
        SourceAvailability::Disabled => 2,
    }
}
