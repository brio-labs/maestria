use std::sync::{
    Arc,
    atomic::{AtomicU64, AtomicUsize, Ordering},
};

use maestria_extensions::{MAX_ACTIVE_REQUESTS_PER_COMMAND, MAX_CAPABILITY_RESPONSE_BYTES};
use rquickjs::function::Async;
use rquickjs::{Ctx, Error as QuickJsError, Function, Result as QuickJsResult};
use tokio::sync::{mpsc, oneshot};

use crate::protocol::{
    BridgeEvent, BridgeFailure, CapabilityCall, capability_response_json, parse_capability_request,
};

const MAX_CAPABILITY_REQUEST_JSON_BYTES: usize = 32_768;

pub(crate) fn create_capability_function<'js>(
    context: Ctx<'js>,
    bridge_sender: mpsc::Sender<BridgeEvent>,
    next_request_id: Arc<AtomicU64>,
    active_requests: Arc<AtomicUsize>,
) -> QuickJsResult<Function<'js>> {
    Function::new(
        context,
        Async(move |request_json: String| {
            let sender = bridge_sender.clone();
            let next_request_id = Arc::clone(&next_request_id);
            let active_requests = Arc::clone(&active_requests);
            let guard = ActiveRequestGuard::acquire(active_requests);

            async move {
                let _guard = guard;
                if _guard.is_none() {
                    return send_failure(sender, BridgeFailure::RequestLimit).await;
                }
                if request_json.len() > MAX_CAPABILITY_REQUEST_JSON_BYTES {
                    return send_failure(sender, BridgeFailure::InvalidRequest).await;
                }
                let request = match parse_capability_request(&request_json) {
                    Ok(request) => request,
                    Err(_) => return send_failure(sender, BridgeFailure::InvalidRequest).await,
                };
                let request_number = match next_request_id.fetch_update(
                    Ordering::Relaxed,
                    Ordering::Relaxed,
                    |current| current.checked_add(1),
                ) {
                    Ok(number) => number,
                    Err(_) => return send_failure(sender, BridgeFailure::RequestIdExhausted).await,
                };
                let request_id = format!("cap-{request_number}");
                let (response_sender, response_receiver) = oneshot::channel();
                let call = CapabilityCall {
                    request_id,
                    request,
                    response: response_sender,
                };
                if sender.send(BridgeEvent::Request(call)).await.is_err() {
                    return Err(QuickJsError::Exception);
                }
                let response = match response_receiver.await {
                    Ok(response) => response,
                    Err(_) => return Err(QuickJsError::Exception),
                };
                let response_json = match capability_response_json(&response) {
                    Ok(response_json) => response_json,
                    Err(_) => return send_failure(sender, BridgeFailure::ResponseEncoding).await,
                };
                if response_json.len() > MAX_CAPABILITY_RESPONSE_BYTES {
                    return send_failure(sender, BridgeFailure::ResponseEncoding).await;
                }
                Ok(response_json)
            }
        }),
    )
}

async fn send_failure(
    sender: mpsc::Sender<BridgeEvent>,
    failure: BridgeFailure,
) -> QuickJsResult<String> {
    if sender.send(BridgeEvent::Failure(failure)).await.is_err() {
        return Err(QuickJsError::Exception);
    }
    Err(QuickJsError::Exception)
}

struct ActiveRequestGuard(Arc<AtomicUsize>);

impl ActiveRequestGuard {
    fn acquire(active_requests: Arc<AtomicUsize>) -> Option<Self> {
        let acquired =
            active_requests.fetch_update(Ordering::AcqRel, Ordering::Acquire, |active| {
                if active < MAX_ACTIVE_REQUESTS_PER_COMMAND {
                    active.checked_add(1)
                } else {
                    None
                }
            });
        acquired.ok().map(|_| Self(active_requests))
    }
}

impl Drop for ActiveRequestGuard {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}
