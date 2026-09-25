use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
};

use maestria_extensions::{View, validate_view};
use rquickjs::{AsyncContext, AsyncRuntime, Ctx, Function, Module, Object, Promise, Value};
use tokio::time::Instant;

use crate::{
    bridge::create_capability_function,
    bundle::ExtensionBundle,
    error::WorkerError,
    protocol::{BridgeEvent, Invocation},
};

const QUICKJS_HEAP_LIMIT: usize = 64 * 1024 * 1024;
const QUICKJS_STACK_LIMIT: usize = 1024 * 1024;
const MAX_VIEW_JSON_BYTES: usize = maestria_extensions::MAX_JSON_LINE_BYTES;

#[derive(Debug)]
enum ExecutionError {
    QuickJs(rquickjs::Error),
    InvalidEntrypointExports,
}

impl From<rquickjs::Error> for ExecutionError {
    fn from(error: rquickjs::Error) -> Self {
        Self::QuickJs(error)
    }
}

impl From<ExecutionError> for WorkerError {
    fn from(error: ExecutionError) -> Self {
        match error {
            ExecutionError::QuickJs(error) => Self::JavaScript(error),
            ExecutionError::InvalidEntrypointExports => Self::InvalidEntrypointExports,
        }
    }
}

pub(crate) async fn create_runtime(
    interrupted: Arc<AtomicBool>,
    deadline: Instant,
) -> Result<(AsyncRuntime, AsyncContext), WorkerError> {
    let runtime = AsyncRuntime::new()?;
    runtime.set_memory_limit(QUICKJS_HEAP_LIMIT).await;
    runtime.set_max_stack_size(QUICKJS_STACK_LIMIT).await;
    runtime
        .set_interrupt_handler(Some(Box::new(move || {
            interrupted.load(Ordering::Acquire) || Instant::now() >= deadline
        })))
        .await;
    let context = AsyncContext::full(&runtime).await?;
    Ok((runtime, context))
}

pub(crate) async fn execute(
    context: &AsyncContext,
    bundle: ExtensionBundle,
    invocation: Invocation,
    bridge_sender: tokio::sync::mpsc::Sender<BridgeEvent>,
    request_sequence: Arc<AtomicU64>,
    active_requests: Arc<AtomicUsize>,
) -> Result<View, WorkerError> {
    let command_id = invocation.command_id;
    let invocation_json = serde_json::to_string(&invocation.context_invocation)?;
    let result_json = context
        .async_with(async move |context| {
            evaluate_and_invoke(
                context,
                bundle,
                command_id,
                invocation_json,
                bridge_sender,
                request_sequence,
                active_requests,
            )
            .await
        })
        .await
        .map_err(WorkerError::from)?;
    if result_json.len() > MAX_VIEW_JSON_BYTES {
        return Err(WorkerError::InvalidView);
    }
    let view: View = serde_json::from_str(&result_json)?;
    validate_view(&view).map_err(|_| WorkerError::InvalidView)?;
    Ok(view)
}

async fn evaluate_and_invoke<'js>(
    context: Ctx<'js>,
    bundle: ExtensionBundle,
    command_id: String,
    invocation_json: String,
    bridge_sender: tokio::sync::mpsc::Sender<BridgeEvent>,
    request_sequence: Arc<AtomicU64>,
    active_requests: Arc<AtomicUsize>,
) -> Result<String, ExecutionError> {
    let host_intrinsics = context.eval::<Object, _>(HOST_INTRINSICS)?;
    let module = Module::declare(context.clone(), bundle.module_name, bundle.source)?;
    let (evaluated, evaluation) = module.eval()?;
    evaluation.into_future::<()>().await?;

    let default_export = evaluated
        .get::<_, Value>("default")
        .map_err(|_| ExecutionError::InvalidEntrypointExports)?;
    let validate = host_intrinsics.get::<_, Function>("validateExports")?;
    let handler = validate
        .call::<_, Function>((default_export, bundle.command_ids, command_id.clone()))
        .map_err(|_| ExecutionError::InvalidEntrypointExports)?;

    let capability_function = create_capability_function(
        context.clone(),
        bridge_sender,
        request_sequence,
        active_requests,
    )?;
    let capability_factory = host_intrinsics.get::<_, Function>("capabilityFactory")?;
    let request_capability = capability_factory.call::<_, Function>((capability_function,))?;
    let invocation_value = parse_json(&host_intrinsics, &invocation_json)?;
    let handler_context = Object::new(context.clone())?;
    handler_context.set("invocation", invocation_value)?;
    handler_context.set("requestCapability", request_capability)?;
    handler_context.set("extensionId", bundle.extension_id)?;
    handler_context.set("commandId", command_id)?;

    let result = handler.call::<_, Value>((handler_context,))?;
    let result = if result.is_promise() {
        Promise::from_value(result)?.into_future::<Value>().await?
    } else {
        result
    };
    Ok(serialize_json(&host_intrinsics, result)?)
}

fn parse_json<'js>(intrinsics: &Object<'js>, source: &str) -> rquickjs::Result<Object<'js>> {
    let parse = intrinsics.get::<_, Function>("parse")?;
    parse.call((source,))
}

fn serialize_json<'js>(intrinsics: &Object<'js>, value: Value<'js>) -> rquickjs::Result<String> {
    let stringify = intrinsics.get::<_, Function>("stringify")?;
    let serialized = stringify.call::<_, Option<String>>((value,))?;
    serialized.ok_or(rquickjs::Error::Exception)
}

const HOST_INTRINSICS: &str = r#"
(() => {
  const parse = JSON.parse;
  const stringify = JSON.stringify;
  const isArray = Array.isArray;
  const getOwnPropertyDescriptor = Object.getOwnPropertyDescriptor;
  const getPrototypeOf = Object.getPrototypeOf;
  const ownKeys = Reflect.ownKeys;
  const apply = Reflect.apply;
  const objectPrototype = Object.prototype;
  const hasOwnProperty = Object.prototype.hasOwnProperty;
  const hasOwn = (object, key) => apply(hasOwnProperty, object, [key]);
  const validateExports = (entry, expectedIds, selectedId) => {
    if (entry === null || typeof entry !== "object" || isArray(entry)) {
      throw new TypeError("default export must be an entrypoint object");
    }
    const entryDescriptor = getOwnPropertyDescriptor(entry, "commands");
    if (!entryDescriptor || !hasOwn(entryDescriptor, "value")) {
      throw new TypeError("entrypoint must have an own data property named commands");
    }
    const commands = entryDescriptor.value;
    if (commands === null || typeof commands !== "object" || isArray(commands)) {
      throw new TypeError("commands must be an object");
    }
    const prototype = getPrototypeOf(commands);
    if (prototype !== null && prototype !== objectPrototype) {
      throw new TypeError("commands must be a plain record");
    }
    const keys = ownKeys(commands);
    if (keys.length !== expectedIds.length) {
      throw new TypeError("commands do not match the declared command IDs");
    }
    for (let i = 0; i < expectedIds.length; i += 1) {
      const id = expectedIds[i];
      let found = false;
      for (let j = 0; j < keys.length; j += 1) {
        if (keys[j] === id) {
          found = true;
          break;
        }
      }
      if (!found) {
        throw new TypeError("commands do not match the declared command IDs");
      }
      const descriptor = getOwnPropertyDescriptor(commands, id);
      if (!descriptor || !hasOwn(descriptor, "value") ||
          typeof descriptor.value !== "function") {
        throw new TypeError("every declared command must be an own handler function");
      }
    }
    const selected = getOwnPropertyDescriptor(commands, selectedId);
    if (!selected || !hasOwn(selected, "value") || typeof selected.value !== "function") {
      throw new TypeError("selected command handler is not declared");
    }
    return selected.value;
  };
  const capabilityFactory = (bridge) => async (request) => {
    const encoded = stringify(request);
    if (typeof encoded !== "string" || encoded.length > 32768) {
      throw new TypeError("capability request is too large");
    }
    return parse(await bridge(encoded));
  };
  return { parse, stringify, validateExports, capabilityFactory };
})()
"#;
