// Copyright 2023 The AccessKit Authors. All rights reserved.
// Licensed under the Apache License, Version 2.0 (found in
// the LICENSE-APACHE file) or the MIT license (found in
// the LICENSE-MIT file), at your option.

use accesskit::{ActivationHandler, DeactivationHandler};
use accesskit_atspi_common::{
    Adapter as AdapterImpl, AppContext, Event, NodeId, NodeIdOrRoot, ObjectEvent, PlatformNode,
};
#[cfg(not(feature = "tokio"))]
use async_channel::{Receiver, Sender};
use atspi::{InterfaceSet, State, proxy::bus::StatusProxy};
#[cfg(not(feature = "tokio"))]
use futures_util::{StreamExt, pin_mut as pin, select};
use std::{
    sync::{Arc, Mutex, OnceLock, RwLock},
    thread,
};
#[cfg(feature = "tokio")]
use tokio::{
    pin, select,
    sync::mpsc::{UnboundedReceiver as Receiver, UnboundedSender as Sender},
};
#[cfg(feature = "tokio")]
use tokio_stream::{StreamExt, wrappers::UnboundedReceiverStream};
use zbus::{Connection, connection::Builder, proxy::PropertyChanged};

use crate::{
    adapter::{AdapterState, Callback, Message},
    atspi::{Bus, map_or_ignoring_recoverable_error, zbus_error_is_unrecoverable},
    executor::Executor,
    util::block_on,
};

static APP_CONTEXT: OnceLock<Arc<RwLock<AppContext>>> = OnceLock::new();
static MESSAGES: OnceLock<Sender<Message>> = OnceLock::new();

fn app_name() -> Option<String> {
    std::env::current_exe().ok().and_then(|path| {
        path.file_name()
            .map(|name| name.to_string_lossy().to_string())
    })
}

pub(crate) fn get_or_init_app_context<'a>() -> &'a Arc<RwLock<AppContext>> {
    APP_CONTEXT.get_or_init(|| AppContext::new(app_name()))
}

pub(crate) fn get_or_init_messages() -> Sender<Message> {
    MESSAGES
        .get_or_init(|| {
            #[cfg(not(feature = "tokio"))]
            let (tx, rx) = async_channel::unbounded();
            #[cfg(feature = "tokio")]
            let (tx, rx) = tokio::sync::mpsc::unbounded_channel();

            thread::spawn(|| {
                let executor = Executor::new();
                block_on(executor.run(async {
                    if let Ok(session_bus) = Builder::session() {
                        if let Ok(session_bus) = session_bus.internal_executor(false).build().await
                        {
                            if let Err(error) = run_event_loop(&executor, session_bus, rx).await {
                                if zbus_error_is_unrecoverable(&error) {
                                    panic!("Accessibility event loop failed: {error}");
                                }
                            }
                        }
                    }
                }))
            });

            tx
        })
        .clone()
}

struct AdapterEntry {
    id: usize,
    activation_handler: Box<dyn ActivationHandler>,
    deactivation_handler: Box<dyn DeactivationHandler>,
    state: Arc<Mutex<AdapterState>>,
}

fn activate_adapter(entry: &mut AdapterEntry) {
    let mut state = entry.state.lock().unwrap();
    if let AdapterState::Inactive {
        is_window_focused,
        root_window_bounds,
        action_handler,
    } = &*state
    {
        *state = match entry.activation_handler.request_initial_tree() {
            Some(initial_state) => {
                let r#impl = AdapterImpl::with_wrapped_action_handler(
                    entry.id,
                    get_or_init_app_context(),
                    Callback::new(),
                    initial_state,
                    *is_window_focused,
                    *root_window_bounds,
                    Arc::clone(action_handler),
                );
                AdapterState::Active(r#impl)
            }
            None => AdapterState::Pending {
                is_window_focused: *is_window_focused,
                root_window_bounds: *root_window_bounds,
                action_handler: Arc::clone(action_handler),
            },
        };
    }
}

fn deactivate_adapter(entry: &mut AdapterEntry) {
    let mut state = entry.state.lock().unwrap();
    match &*state {
        AdapterState::Inactive { .. } => (),
        AdapterState::Pending {
            is_window_focused,
            root_window_bounds,
            action_handler,
        } => {
            *state = AdapterState::Inactive {
                is_window_focused: *is_window_focused,
                root_window_bounds: *root_window_bounds,
                action_handler: Arc::clone(action_handler),
            };
            drop(state);
            entry.deactivation_handler.deactivate_accessibility();
        }
        AdapterState::Active(r#impl) => {
            *state = AdapterState::Inactive {
                is_window_focused: r#impl.is_window_focused(),
                root_window_bounds: r#impl.root_window_bounds(),
                action_handler: r#impl.wrapped_action_handler(),
            };
            drop(state);
            entry.deactivation_handler.deactivate_accessibility();
        }
    }
}

async fn bus_after_status_change(
    change: Option<PropertyChanged<'_, bool>>,
    session_bus: &Connection,
    executor: &Executor<'_>,
) -> zbus::Result<Option<Bus>> {
    let enabled = match change {
        Some(change) => change.get().await?,
        None => false,
    };
    if enabled {
        map_or_ignoring_recoverable_error(Bus::new(session_bus, executor).await, None, Some)
    } else {
        Ok(None)
    }
}

fn sync_adapters(adapters: &mut [AdapterEntry], atspi_bus: &Option<Bus>) {
    let active = atspi_bus.is_some();
    for entry in adapters {
        if active {
            activate_adapter(entry);
        } else {
            deactivate_adapter(entry);
        }
    }
}

// Slint can recycle NodeIds before callback messages reach the bus thread.
// Reconcile lifecycle messages against the current filtered tree so an old
// component cannot tear down a new object at the same AT-SPI path.
fn current_accessible_node(
    adapters: &[AdapterEntry],
    adapter_id: usize,
    node_id: NodeId,
) -> Option<(PlatformNode, InterfaceSet)> {
    let index = adapters
        .binary_search_by(|entry| entry.id.cmp(&adapter_id))
        .ok()?;
    let state = adapters[index].state.lock().unwrap();
    let AdapterState::Active(adapter) = &*state else {
        return None;
    };

    let node = adapter.platform_node(node_id);
    if !node.state().contains(State::Visible) {
        return None;
    }
    let interfaces = node.interfaces().ok()?;
    Some((node, interfaces))
}

fn interfaces_to_unregister(
    adapters: &[AdapterEntry],
    adapter_id: usize,
    node_id: NodeId,
    requested: InterfaceSet,
) -> InterfaceSet {
    match current_accessible_node(adapters, adapter_id, node_id) {
        Some((_, current)) => requested ^ (requested & current),
        None => requested,
    }
}

fn should_emit_cache_remove(adapters: &[AdapterEntry], adapter_id: usize, node_id: NodeId) -> bool {
    current_accessible_node(adapters, adapter_id, node_id).is_none()
}

fn current_node_has_parent(
    adapters: &[AdapterEntry],
    adapter_id: usize,
    parent: &NodeIdOrRoot,
    child_id: NodeId,
) -> bool {
    let Some((child, _)) = current_accessible_node(adapters, adapter_id, child_id) else {
        return false;
    };
    match (child.parent(), parent) {
        (Ok(NodeIdOrRoot::Root), NodeIdOrRoot::Root) => true,
        (Ok(NodeIdOrRoot::Node(actual)), NodeIdOrRoot::Node(expected)) => actual == *expected,
        _ => false,
    }
}

fn should_emit_object_event(
    adapters: &[AdapterEntry],
    adapter_id: usize,
    target: &NodeIdOrRoot,
    event: &ObjectEvent,
) -> bool {
    match event {
        ObjectEvent::StateChanged(State::Defunct, true) => match target {
            NodeIdOrRoot::Node(node_id) => {
                current_accessible_node(adapters, adapter_id, *node_id).is_none()
            }
            NodeIdOrRoot::Root => true,
        },
        ObjectEvent::ChildAdded(_, child_id) => {
            current_node_has_parent(adapters, adapter_id, target, *child_id)
        }
        ObjectEvent::ChildRemoved(child_id) => {
            !current_node_has_parent(adapters, adapter_id, target, *child_id)
        }
        _ => true,
    }
}

async fn run_event_loop(
    executor: &Executor<'_>,
    session_bus: Connection,
    rx: Receiver<Message>,
) -> zbus::Result<()> {
    let session_bus_copy = session_bus.clone();
    let _session_bus_task = executor.spawn(
        async move {
            loop {
                session_bus_copy.executor().tick().await;
            }
        },
        "accesskit_session_bus_task",
    );

    let status = StatusProxy::new(&session_bus).await?;
    let changes = status.receive_is_enabled_changed().await.fuse();
    pin!(changes);

    #[cfg(not(feature = "tokio"))]
    let messages = rx.fuse();
    #[cfg(feature = "tokio")]
    let messages = UnboundedReceiverStream::new(rx).fuse();
    pin!(messages);

    let mut atspi_bus = None;
    let mut adapters: Vec<AdapterEntry> = Vec::new();

    loop {
        select! {
            change = changes.next() => {
                atspi_bus = bus_after_status_change(change, &session_bus, executor).await?;
                sync_adapters(&mut adapters, &atspi_bus);

            }
            message = messages.next() => {
                if let Some(message) = message {
                    process_adapter_message(&atspi_bus, &mut adapters, message).await?;
                }
            }
        }
    }
}

async fn process_adapter_message(
    atspi_bus: &Option<Bus>,
    adapters: &mut Vec<AdapterEntry>,
    message: Message,
) -> zbus::Result<()> {
    match message {
        Message::AddAdapter {
            id,
            activation_handler,
            deactivation_handler,
            state,
        } => {
            adapters.push(AdapterEntry {
                id,
                activation_handler,
                deactivation_handler,
                state,
            });
            if atspi_bus.is_some() {
                let entry = adapters.last_mut().unwrap();
                activate_adapter(entry);
            }
        }
        Message::RemoveAdapter { id } => {
            if let Ok(index) = adapters.binary_search_by(|entry| entry.id.cmp(&id)) {
                adapters.remove(index);
            }
        }
        Message::RegisterInterfaces { node, interfaces } => {
            if let Some(bus) = atspi_bus {
                if let Some((current_node, current_interfaces)) =
                    current_accessible_node(adapters, node.adapter_id(), node.id())
                {
                    let interfaces = interfaces & current_interfaces;
                    if interfaces != InterfaceSet::empty() {
                        bus.register_interfaces(current_node, interfaces).await?
                    }
                }
            }
        }
        Message::UnregisterInterfaces {
            adapter_id,
            node_id,
            interfaces,
        } => {
            if let Some(bus) = atspi_bus {
                let interfaces =
                    interfaces_to_unregister(adapters, adapter_id, node_id, interfaces);
                if interfaces != InterfaceSet::empty() {
                    bus.unregister_interfaces(adapter_id, node_id, interfaces)
                        .await?
                }
            }
        }
        Message::EmitEvent {
            adapter_id,
            event: Event::Object { target, event },
        } => {
            if let Some(bus) = atspi_bus {
                if should_emit_object_event(adapters, adapter_id, &target, &event) {
                    bus.emit_object_event(adapter_id, target, event).await?
                }
            }
        }
        Message::EmitEvent {
            adapter_id,
            event:
                Event::Window {
                    target,
                    name,
                    event,
                },
        } => {
            if let Some(bus) = atspi_bus {
                bus.emit_window_event(adapter_id, target, name, event)
                    .await?;
            }
        }
        Message::EmitEvent {
            event: Event::Cache(_),
            ..
        } => unreachable!("cache events are sent as EmitCacheAdd/EmitCacheRemove"),
        Message::EmitCacheAdd { node } => {
            if let Some(bus) = atspi_bus {
                if let Some((current_node, _)) =
                    current_accessible_node(adapters, node.adapter_id(), node.id())
                {
                    bus.emit_cache_add(current_node).await?;
                }
            }
        }
        Message::EmitCacheRemove {
            adapter_id,
            node_id,
        } => {
            if let Some(bus) = atspi_bus {
                if should_emit_cache_remove(adapters, adapter_id, node_id) {
                    bus.emit_cache_remove(adapter_id, node_id).await?;
                }
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        AdapterEntry, AdapterImpl, AdapterState, AppContext, Event, NodeId, NodeIdOrRoot,
        ObjectEvent, State, current_accessible_node, interfaces_to_unregister,
        should_emit_cache_remove, should_emit_object_event,
    };
    use accesskit::{
        Action, ActionHandler, ActionRequest, ActivationHandler, DeactivationHandler, Node,
        NodeId as LocalNodeId, Rect, Role, Tree, TreeId, TreeUpdate,
    };
    use accesskit_atspi_common::{AdapterCallback, WindowBounds};
    use atspi::{Interface, InterfaceSet};
    use std::sync::{Arc, Mutex};

    struct NoOpActionHandler;

    impl ActionHandler for NoOpActionHandler {
        fn do_action(&mut self, _request: ActionRequest) {}
    }

    struct NoOpActivationHandler;

    impl ActivationHandler for NoOpActivationHandler {
        fn request_initial_tree(&mut self) -> Option<TreeUpdate> {
            None
        }
    }

    struct NoOpDeactivationHandler;

    impl DeactivationHandler for NoOpDeactivationHandler {
        fn deactivate_accessibility(&mut self) {}
    }

    struct NoOpCallback;

    impl AdapterCallback for NoOpCallback {
        fn register_interfaces(&self, _: &AdapterImpl, _: NodeId, _: InterfaceSet) {}

        fn unregister_interfaces(&self, _: &AdapterImpl, _: NodeId, _: InterfaceSet) {}

        fn emit_event(&self, _: &AdapterImpl, _: Event) {}
    }

    fn tree_with_child(child_role: Option<Role>) -> TreeUpdate {
        let mut root = Node::new(Role::Window);
        root.set_children(
            child_role
                .map(|_| LocalNodeId(1))
                .into_iter()
                .collect::<Vec<_>>(),
        );
        let mut nodes = vec![(LocalNodeId(0), root)];
        if let Some(role) = child_role {
            let mut child = Node::new(role);
            child.set_bounds(Rect {
                x0: 0.0,
                y0: 0.0,
                x1: 100.0,
                y1: 100.0,
            });
            if role == Role::Button {
                child.add_action(Action::Click);
            }
            nodes.push((LocalNodeId(1), child));
        }

        TreeUpdate {
            nodes,
            tree: Some(Tree::new(LocalNodeId(0))),
            tree_id: TreeId::ROOT,
            focus: LocalNodeId(0),
        }
    }

    fn active_entry(initial_tree: TreeUpdate) -> AdapterEntry {
        let app_context = AppContext::new(None);
        let adapter = AdapterImpl::new(
            &app_context,
            NoOpCallback,
            initial_tree,
            false,
            WindowBounds::default(),
            NoOpActionHandler,
        );
        AdapterEntry {
            id: adapter.id(),
            activation_handler: Box::new(NoOpActivationHandler),
            deactivation_handler: Box::new(NoOpDeactivationHandler),
            state: Arc::new(Mutex::new(AdapterState::Active(adapter))),
        }
    }

    fn first_child_id(entry: &AdapterEntry) -> NodeId {
        let state = entry.state.lock().unwrap();
        let AdapterState::Active(adapter) = &*state else {
            panic!("test adapter must be active");
        };
        adapter
            .platform_node(adapter.root_id())
            .child_at_index(0)
            .unwrap()
            .unwrap()
    }

    fn update(entry: &AdapterEntry, tree: TreeUpdate) {
        let mut state = entry.state.lock().unwrap();
        let AdapterState::Active(adapter) = &mut *state else {
            panic!("test adapter must be active");
        };
        adapter.update(tree);
    }

    #[test]
    fn stale_lifecycle_messages_do_not_remove_reused_live_node() {
        let entry = active_entry(tree_with_child(Some(Role::Button)));
        let adapter_id = entry.id;
        let child_id = first_child_id(&entry);
        let adapters = std::slice::from_ref(&entry);
        let old_interfaces = current_accessible_node(adapters, adapter_id, child_id)
            .unwrap()
            .1;
        assert!(old_interfaces.contains(Interface::Action));
        let parent = current_accessible_node(adapters, adapter_id, child_id)
            .unwrap()
            .0
            .parent()
            .unwrap();

        update(&entry, tree_with_child(None));
        assert!(current_accessible_node(adapters, adapter_id, child_id).is_none());
        update(&entry, tree_with_child(Some(Role::Group)));

        let current = current_accessible_node(adapters, adapter_id, child_id)
            .expect("reused node ID is still accessible")
            .1;

        let removed = interfaces_to_unregister(adapters, adapter_id, child_id, old_interfaces);
        assert!(!current.contains(Interface::Action));
        assert!(current.contains(Interface::Accessible));
        assert!(removed.contains(Interface::Action));
        assert!(!removed.contains(Interface::Accessible));
        assert!(!removed.contains(Interface::Component));
        assert!(!should_emit_cache_remove(adapters, adapter_id, child_id));
        assert!(!should_emit_object_event(
            adapters,
            adapter_id,
            &parent,
            &ObjectEvent::ChildRemoved(child_id),
        ));
        assert!(should_emit_object_event(
            adapters,
            adapter_id,
            &parent,
            &ObjectEvent::ChildAdded(0, child_id),
        ));
        assert!(!should_emit_object_event(
            adapters,
            adapter_id,
            &NodeIdOrRoot::Node(child_id),
            &ObjectEvent::StateChanged(State::Defunct, true),
        ));

        update(&entry, tree_with_child(None));
        assert!(current_accessible_node(adapters, adapter_id, child_id).is_none());
        assert!(should_emit_cache_remove(adapters, adapter_id, child_id));
        assert!(should_emit_object_event(
            adapters,
            adapter_id,
            &parent,
            &ObjectEvent::ChildRemoved(child_id),
        ));
        assert!(!should_emit_object_event(
            adapters,
            adapter_id,
            &parent,
            &ObjectEvent::ChildAdded(0, child_id),
        ));
        assert!(should_emit_object_event(
            adapters,
            adapter_id,
            &NodeIdOrRoot::Node(child_id),
            &ObjectEvent::StateChanged(State::Defunct, true),
        ));
        assert_eq!(
            interfaces_to_unregister(adapters, adapter_id, child_id, old_interfaces),
            old_interfaces
        );
    }
}
