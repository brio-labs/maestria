use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
};

use maestria_extensions::{
    CapabilityRequest, CapabilityResponse, HostMessage, MAX_ACTIVE_REQUESTS_PER_COMMAND,
    PROTOCOL_VERSION, WorkerMessage,
};
use tokio::sync::{mpsc, oneshot};
use tokio::time::{self, Instant};

use crate::{
    arguments::WorkerArguments,
    bundle::{ExtensionBundle, load_bundle},
    error::WorkerError,
    input::{InputEvent, start_reader},
    javascript::{self, create_runtime},
    output::write_worker_message,
    protocol::{
        BridgeEvent, BridgeFailure, CapabilityCall, Invocation, invocation_from_host,
        response_matches_request,
    },
};

struct PendingCapability {
    request_id: String,
    request: CapabilityRequest,
    response: oneshot::Sender<CapabilityResponse>,
}

struct CommandChannels {
    bridge_events: mpsc::Receiver<BridgeEvent>,
    bridge_sender: mpsc::Sender<BridgeEvent>,
    request_sequence: Arc<AtomicU64>,
    active_requests: Arc<AtomicUsize>,
    interrupted: Arc<AtomicBool>,
    deadline: Instant,
}

pub(crate) async fn run(arguments: WorkerArguments) -> Result<(), WorkerError> {
    let deadline = Instant::now() + arguments.timeout;
    let bundle = load_bundle(&arguments)?;
    if bundle.command_ids.is_empty() {
        return Err(WorkerError::EmptyEntrypoint);
    }

    let interrupted = Arc::new(AtomicBool::new(false));
    let mut input = start_reader(Arc::clone(&interrupted)).map_err(|source| WorkerError::Io {
        operation: "starting host input reader",
        source,
    })?;
    let invocation = read_initial_invocation(&mut input, deadline).await?;
    if !bundle.command_ids.contains(&invocation.command_id) {
        return Err(WorkerError::UndeclaredCommand);
    }
    let command_id = invocation.command_id.clone();

    let (runtime, context) = create_runtime(Arc::clone(&interrupted), deadline).await?;
    let (bridge_sender, bridge_events) = mpsc::channel(32);
    let channels = CommandChannels {
        bridge_events,
        bridge_sender,
        request_sequence: Arc::new(AtomicU64::new(1)),
        active_requests: Arc::new(AtomicUsize::new(0)),
        interrupted,
        deadline,
    };
    let view = execute_command(&context, bundle, invocation, &mut input, channels).await?;

    write_worker_message(&WorkerMessage::ViewUpdate {
        protocol_version: PROTOCOL_VERSION,
        command_id: command_id.clone(),
        view,
    })
    .await?;
    write_worker_message(&WorkerMessage::CommandComplete {
        protocol_version: PROTOCOL_VERSION,
        command_id,
    })
    .await?;
    drop(runtime);
    Ok(())
}

async fn read_initial_invocation(
    input: &mut mpsc::Receiver<InputEvent>,
    deadline: Instant,
) -> Result<Invocation, WorkerError> {
    let event = time::timeout_at(deadline, input.recv())
        .await
        .map_err(|_| WorkerError::Deadline)?
        .ok_or(WorkerError::InputClosed)?;
    match event {
        InputEvent::Message(message) => invocation_from_host(message),
        InputEvent::Failure(error) => Err(WorkerError::Input(error)),
        InputEvent::Closed => Err(WorkerError::InputClosed),
    }
}

async fn execute_command(
    context: &rquickjs::AsyncContext,
    bundle: ExtensionBundle,
    invocation: Invocation,
    input: &mut mpsc::Receiver<InputEvent>,
    channels: CommandChannels,
) -> Result<maestria_extensions::View, WorkerError> {
    let command_id = invocation.command_id.clone();
    let CommandChannels {
        mut bridge_events,
        bridge_sender,
        request_sequence,
        active_requests,
        interrupted,
        deadline,
    } = channels;
    let execution = javascript::execute(
        context,
        bundle,
        invocation,
        bridge_sender,
        request_sequence,
        Arc::clone(&active_requests),
    );
    tokio::pin!(execution);
    let deadline_timer = time::sleep_until(deadline);
    tokio::pin!(deadline_timer);
    let mut pending = Vec::with_capacity(MAX_ACTIVE_REQUESTS_PER_COMMAND);
    let mut bridge_is_open = true;

    let outcome = loop {
        tokio::select! {
            result = &mut execution => {
                if interrupted.load(Ordering::Acquire) {
                    break None;
                }
                if Instant::now() >= deadline {
                    return Err(WorkerError::Deadline);
                }
                break Some(result);
            }
            event = bridge_events.recv(), if bridge_is_open => {
                match event {
                    Some(event) => handle_bridge_event(event, &command_id, &mut pending).await?,
                    None => bridge_is_open = false,
                }
            }
            event = input.recv() => {
                let event = event.ok_or(WorkerError::InputClosed)?;
                handle_host_event(event, &command_id, &mut pending)?;
            }
            _ = &mut deadline_timer => {
                return Err(WorkerError::Deadline);
            }
        }
    };
    let view = match outcome {
        Some(result) => result?,
        None => return Err(interruption_error(input, &command_id, deadline).await),
    };
    ensure_no_pending_requests(&mut bridge_events, &pending, &active_requests)?;
    ensure_no_trailing_input(input, &command_id)?;
    Ok(view)
}

async fn handle_bridge_event(
    event: BridgeEvent,
    command_id: &str,
    pending: &mut Vec<PendingCapability>,
) -> Result<(), WorkerError> {
    match event {
        BridgeEvent::Failure(failure) => Err(map_bridge_failure(failure)),
        BridgeEvent::Request(call) => {
            if pending.len() >= MAX_ACTIVE_REQUESTS_PER_COMMAND {
                return Err(WorkerError::CapabilityRequestLimit);
            }
            send_capability_request(command_id, call, pending).await
        }
    }
}

async fn send_capability_request(
    command_id: &str,
    call: CapabilityCall,
    pending: &mut Vec<PendingCapability>,
) -> Result<(), WorkerError> {
    let CapabilityCall {
        request_id,
        request,
        response,
    } = call;
    let message = WorkerMessage::CapabilityRequest {
        protocol_version: PROTOCOL_VERSION,
        command_id: command_id.to_owned(),
        request_id: request_id.clone(),
        request: request.clone(),
    };
    write_worker_message(&message).await?;
    pending.push(PendingCapability {
        request_id,
        request,
        response,
    });
    Ok(())
}

fn handle_host_event(
    event: InputEvent,
    command_id: &str,
    pending: &mut Vec<PendingCapability>,
) -> Result<(), WorkerError> {
    match event {
        InputEvent::Failure(error) => Err(WorkerError::Input(error)),
        InputEvent::Closed => Err(WorkerError::InputClosed),
        InputEvent::Message(HostMessage::CapabilityResponse {
            request_id,
            response,
            ..
        }) => {
            let Some(index) = pending
                .iter()
                .position(|call| call.request_id == request_id)
            else {
                return Err(WorkerError::InvalidCapabilityResponse);
            };
            let call = pending.remove(index);
            if !response_matches_request(&call.request, &response) {
                return Err(WorkerError::InvalidCapabilityResponse);
            }
            call.response
                .send(response)
                .map_err(|_| WorkerError::AbandonedCapabilityRequest)
        }
        InputEvent::Message(HostMessage::CommandCancel {
            command_id: cancelled_id,
            ..
        }) if cancelled_id == command_id => Err(WorkerError::Cancelled),
        InputEvent::Message(_) => Err(WorkerError::UnexpectedHostMessage),
    }
}

fn ensure_no_pending_requests(
    bridge_events: &mut mpsc::Receiver<BridgeEvent>,
    pending: &[PendingCapability],
    active_requests: &AtomicUsize,
) -> Result<(), WorkerError> {
    if !pending.is_empty() || active_requests.load(Ordering::Acquire) != 0 {
        return Err(WorkerError::OutstandingCapabilityRequests);
    }
    match bridge_events.try_recv() {
        Ok(BridgeEvent::Failure(failure)) => Err(map_bridge_failure(failure)),
        Ok(BridgeEvent::Request(_)) => Err(WorkerError::OutstandingCapabilityRequests),
        Err(mpsc::error::TryRecvError::Empty) | Err(mpsc::error::TryRecvError::Disconnected) => {
            Ok(())
        }
    }
}

fn ensure_no_trailing_input(
    input: &mut mpsc::Receiver<InputEvent>,
    command_id: &str,
) -> Result<(), WorkerError> {
    loop {
        match input.try_recv() {
            Ok(event) => {
                let mut no_pending = Vec::new();
                handle_host_event(event, command_id, &mut no_pending)?;
            }
            Err(mpsc::error::TryRecvError::Empty) => return Ok(()),
            Err(mpsc::error::TryRecvError::Disconnected) => return Err(WorkerError::InputClosed),
        }
    }
}

async fn interruption_error(
    input: &mut mpsc::Receiver<InputEvent>,
    command_id: &str,
    deadline: Instant,
) -> WorkerError {
    if Instant::now() >= deadline {
        return WorkerError::Deadline;
    }
    match input.recv().await {
        Some(InputEvent::Failure(error)) => WorkerError::Input(error),
        Some(InputEvent::Closed) | None => WorkerError::InputClosed,
        Some(InputEvent::Message(HostMessage::CommandCancel {
            command_id: cancelled_id,
            ..
        })) if cancelled_id == command_id => WorkerError::Cancelled,
        Some(InputEvent::Message(_)) => WorkerError::UnexpectedHostMessage,
    }
}

fn map_bridge_failure(failure: BridgeFailure) -> WorkerError {
    match failure {
        BridgeFailure::InvalidRequest => WorkerError::InvalidCapabilityRequest,
        BridgeFailure::RequestLimit => WorkerError::CapabilityRequestLimit,
        BridgeFailure::RequestIdExhausted => WorkerError::CapabilityRequestIdExhausted,
        BridgeFailure::ResponseEncoding => WorkerError::InvalidCapabilityResponse,
    }
}
