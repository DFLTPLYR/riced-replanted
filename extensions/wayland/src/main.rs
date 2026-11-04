//! Generic Wayland extension: outputs, workspaces, and toplevels.
//!
//! Uses only standard protocols:
//! - `wl_output` for outputs
//! - `ext-workspace-v1` for workspaces and workspace groups
//! - `zwlr-foreign-toplevel-management-v1` when available for toplevels,
//!   falling back to `ext-foreign-toplevel-list-v1` (read-only) otherwise

use std::{
    collections::{BTreeMap, HashMap},
    error::Error,
    io,
    sync::{Arc, Mutex},
    thread,
    time::Duration,
};

use riced_extension_sdk::{Plugin, log};
use serde_json::{Value, json};
use wayland_client::{
    Connection, Dispatch, Proxy, QueueHandle, WEnum, delegate_noop, event_created_child,
    protocol::{
        wl_output,
        wl_registry::{self, WlRegistry},
        wl_seat::WlSeat,
    },
};
use wayland_protocols::ext::{
    foreign_toplevel_list::v1::client::{
        ext_foreign_toplevel_handle_v1::{self as ext_handle, ExtForeignToplevelHandleV1},
        ext_foreign_toplevel_list_v1::{self, ExtForeignToplevelListV1},
    },
    workspace::v1::client::{
        ext_workspace_group_handle_v1::{self, ExtWorkspaceGroupHandleV1},
        ext_workspace_handle_v1::{self, ExtWorkspaceHandleV1},
        ext_workspace_manager_v1::{self, ExtWorkspaceManagerV1},
    },
};
use wayland_protocols_wlr::foreign_toplevel::v1::client::{
    zwlr_foreign_toplevel_handle_v1::{self, ZwlrForeignToplevelHandleV1},
    zwlr_foreign_toplevel_manager_v1::{self, ZwlrForeignToplevelManagerV1},
};

fn main() -> io::Result<()> {
    let requests = Arc::new(Mutex::new(Requests::default()));

    let activate = Arc::clone(&requests);
    let close = Arc::clone(&requests);
    let workspace = Arc::clone(&requests);

    Plugin::new("wayland")
        .state([
            "connected",
            "capabilities",
            "outputs",
            "toplevels",
            "workspaces",
            "workspace_groups",
            "focused",
        ])
        .method("activate_toplevel", false, move |args| {
            let id = argument(args, "toplevel")?;
            with_requests(&activate, |requests| requests.activate_toplevel(&id))
        })
        .method("close_toplevel", false, move |args| {
            let id = argument(args, "toplevel")?;
            with_requests(&close, |requests| requests.close_toplevel(&id))
        })
        .method("activate_workspace", false, move |args| {
            let id = argument(args, "workspace")?;
            with_requests(&workspace, |requests| requests.activate_workspace(&id))
        })
        .serve(move |context| run(context, requests))
}

fn argument(args: &Value, what: &str) -> Result<String, String> {
    args.get(0)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| format!("missing {what} id"))
}

fn with_requests<F>(requests: &Arc<Mutex<Requests>>, operation: F) -> Result<Value, String>
where
    F: FnOnce(&Requests) -> Result<(), String>,
{
    let requests = requests.lock().unwrap();
    operation(&requests)?;
    Ok(Value::Null)
}

/// Handles shared with the SDK method reader thread.
#[derive(Default)]
struct Requests {
    connection: Option<Connection>,
    seat: Option<WlSeat>,
    workspace_manager: Option<ExtWorkspaceManagerV1>,
    toplevels: BTreeMap<String, ZwlrForeignToplevelHandleV1>,
    workspaces: BTreeMap<String, ExtWorkspaceHandleV1>,
    workspace_caps: BTreeMap<String, u32>,
}

impl Requests {
    fn clear(&mut self) {
        *self = Self::default();
    }

    fn flush(&self) -> Result<(), String> {
        let connection = self
            .connection
            .as_ref()
            .ok_or_else(|| "wayland is not connected".to_owned())?;
        connection.flush().map_err(|error| error.to_string())
    }

    fn activate_toplevel(&self, id: &str) -> Result<(), String> {
        let handle = self
            .toplevels
            .get(id)
            .filter(|handle| handle.is_alive())
            .ok_or_else(|| format!("unknown toplevel: {id}"))?;
        let seat = self
            .seat
            .as_ref()
            .ok_or_else(|| "compositor has no wl_seat".to_owned())?;
        handle.activate(seat);
        self.flush()
    }

    fn close_toplevel(&self, id: &str) -> Result<(), String> {
        let handle = self
            .toplevels
            .get(id)
            .filter(|handle| handle.is_alive())
            .ok_or_else(|| format!("unknown toplevel: {id}"))?;
        handle.close();
        self.flush()
    }

    fn activate_workspace(&self, id: &str) -> Result<(), String> {
        let handle = self
            .workspaces
            .get(id)
            .filter(|handle| handle.is_alive())
            .ok_or_else(|| format!("unknown workspace: {id}"))?;
        if self.workspace_caps.get(id).is_none_or(|caps| caps & 1 == 0) {
            return Err(format!("workspace {id} does not support activate"));
        }
        let manager = self
            .workspace_manager
            .as_ref()
            .filter(|manager| manager.is_alive())
            .ok_or_else(|| "compositor has no ext-workspace-manager-v1".to_owned())?;
        handle.activate();
        manager.commit();
        self.flush()
    }
}

fn run(context: riced_extension_sdk::Context, requests: Arc<Mutex<Requests>>) {
    let mut last = empty(false);
    context.publish(last.clone());
    loop {
        if let Err(error) = connected(&context, &requests, &mut last) {
            log::warn!(target: "wayland.connection", "{error}");
        }
        requests.lock().unwrap().clear();
        if let Some(object) = last.as_object_mut() {
            object.insert("connected".into(), json!(false));
        }
        context.publish(last.clone());
        thread::sleep(Duration::from_secs(1));
    }
}

fn connected(
    context: &riced_extension_sdk::Context,
    requests: &Arc<Mutex<Requests>>,
    last: &mut Value,
) -> Result<(), Box<dyn Error>> {
    let connection = Connection::connect_to_env()?;
    let mut queue = connection.new_event_queue();
    let qh = queue.handle();
    let registry = connection.display().get_registry(&qh, ());
    let mut state = Wayland::new(connection.clone(), registry.clone(), Arc::clone(requests));
    queue.roundtrip(&mut state)?;

    state.sync_requests();
    let store = state.store();
    context.publish(store.clone());
    *last = store;
    state.dirty = false;

    loop {
        queue.blocking_dispatch(&mut state)?;
        state.sync_requests();
        if state.dirty {
            let store = state.store();
            context.publish(store.clone());
            *last = store;
            state.dirty = false;
        }
    }
}

const MAX_TEXT: usize = 1024;

fn clamp(mut value: String, max: usize) -> String {
    if let Some((index, _)) = value.char_indices().nth(max) {
        value.truncate(index);
    }
    value
}

fn empty(connected: bool) -> Value {
    json!({
        "connected": connected,
        "capabilities": {
            "toplevels": false,
            "toplevel_control": false,
            "workspaces": false,
        },
        "outputs": [],
        "toplevels": [],
        "workspaces": [],
        "workspace_groups": [],
        "focused": {},
    })
}

#[derive(Debug)]
struct Output {
    global: u32,
    id: wayland_client::backend::ObjectId,
    name: String,
    description: String,
    make: String,
    model: String,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    scale: i32,
}

impl Default for Output {
    fn default() -> Self {
        Self {
            global: 0,
            id: wayland_client::backend::ObjectId::null(),
            name: String::new(),
            description: String::new(),
            make: String::new(),
            model: String::new(),
            x: 0,
            y: 0,
            width: 0,
            height: 0,
            scale: 1,
        }
    }
}

impl Output {
    fn display_name(&self) -> String {
        if self.name.is_empty() {
            format!("output-{}", self.global)
        } else {
            self.name.clone()
        }
    }
}

#[derive(Debug, Default, Clone, Copy)]
struct ToplevelState {
    maximized: bool,
    minimized: bool,
    activated: bool,
    fullscreen: bool,
}

impl ToplevelState {
    fn decode(bytes: &[u8]) -> Self {
        let mut state = Self::default();
        for chunk in bytes.as_chunks::<4>().0 {
            match u32::from_ne_bytes(*chunk) {
                0 => state.maximized = true,
                1 => state.minimized = true,
                2 => state.activated = true,
                3 => state.fullscreen = true,
                _ => {}
            }
        }
        state
    }
}

#[derive(Debug)]
struct WlrToplevel {
    handle: ZwlrForeignToplevelHandleV1,
    title: String,
    app_id: String,
    state: ToplevelState,
    outputs: Vec<wayland_client::backend::ObjectId>,
}

#[derive(Debug)]
struct ExtToplevel {
    title: String,
    app_id: String,
    identifier: String,
}

#[derive(Debug)]
struct Workspace {
    handle: ExtWorkspaceHandleV1,
    protocol_id: Option<String>,
    name: String,
    coordinates: Vec<u32>,
    state: u32,
    capabilities: u32,
    group: Option<String>,
}

#[derive(Debug)]
struct Group {
    outputs: Vec<wayland_client::backend::ObjectId>,
    capabilities: u32,
}

struct Wayland {
    connection: Connection,
    _registry: WlRegistry,
    requests: Arc<Mutex<Requests>>,
    outputs: BTreeMap<u32, Output>,
    output_ids: HashMap<wayland_client::backend::ObjectId, u32>,
    wlr_manager: Option<ZwlrForeignToplevelManagerV1>,
    ext_list: Option<ExtForeignToplevelListV1>,
    workspace_manager: Option<ExtWorkspaceManagerV1>,
    seat: Option<WlSeat>,
    wlr_toplevels: BTreeMap<String, WlrToplevel>,
    ext_toplevels: BTreeMap<String, ExtToplevel>,
    workspaces: BTreeMap<String, Workspace>,
    groups: BTreeMap<String, Group>,
    dirty: bool,
}

impl Wayland {
    fn new(connection: Connection, registry: WlRegistry, requests: Arc<Mutex<Requests>>) -> Self {
        Self {
            connection,
            _registry: registry,
            requests,
            outputs: BTreeMap::new(),
            output_ids: HashMap::new(),
            wlr_manager: None,
            ext_list: None,
            workspace_manager: None,
            seat: None,
            wlr_toplevels: BTreeMap::new(),
            ext_toplevels: BTreeMap::new(),
            workspaces: BTreeMap::new(),
            groups: BTreeMap::new(),
            dirty: false,
        }
    }

    fn bind(
        &mut self,
        registry: &WlRegistry,
        name: u32,
        interface: &str,
        version: u32,
        qh: &QueueHandle<Self>,
    ) {
        match interface {
            "wl_output" => {
                let output =
                    registry.bind::<wl_output::WlOutput, _, _>(name, version.min(4), qh, name);
                self.output_ids.insert(output.id(), name);
                self.outputs.insert(
                    name,
                    Output {
                        global: name,
                        id: output.id(),
                        ..Output::default()
                    },
                );
                self.dirty = true;
            }
            "wl_seat" if self.seat.is_none() => {
                self.seat = Some(registry.bind::<WlSeat, _, _>(name, version.min(7), qh, ()));
                self.dirty = true;
            }
            "zwlr_foreign_toplevel_manager_v1" if self.wlr_manager.is_none() => {
                self.wlr_manager = Some(registry.bind::<ZwlrForeignToplevelManagerV1, _, _>(
                    name,
                    version.min(3),
                    qh,
                    (),
                ));
                self.dirty = true;
            }
            "ext_foreign_toplevel_list_v1" if self.ext_list.is_none() => {
                self.ext_list =
                    Some(registry.bind::<ExtForeignToplevelListV1, _, _>(name, 1, qh, ()));
                self.dirty = true;
            }
            "ext_workspace_manager_v1" if self.workspace_manager.is_none() => {
                self.workspace_manager =
                    Some(registry.bind::<ExtWorkspaceManagerV1, _, _>(name, 1, qh, ()));
                self.dirty = true;
            }
            _ => {}
        }
    }

    fn remove_global(&mut self, name: u32) {
        if let Some(output) = self.outputs.remove(&name) {
            self.output_ids.remove(&output.id);
            for toplevel in self.wlr_toplevels.values_mut() {
                toplevel.outputs.retain(|existing| existing != &output.id);
            }
            for group in self.groups.values_mut() {
                group.outputs.retain(|existing| existing != &output.id);
            }
            self.dirty = true;
        }
    }

    fn output_name(&self, id: &wayland_client::backend::ObjectId) -> String {
        self.output_ids
            .get(id)
            .and_then(|global| self.outputs.get(global))
            .map(Output::display_name)
            .unwrap_or_else(|| id.protocol_id().to_string())
    }

    fn output_names(&self, outputs: &[wayland_client::backend::ObjectId]) -> Vec<String> {
        let mut names = outputs
            .iter()
            .map(|output| self.output_name(output))
            .collect::<Vec<_>>();
        names.sort();
        names
    }

    fn workspace_outputs(&self, workspace: &Workspace) -> Vec<String> {
        workspace
            .group
            .as_ref()
            .and_then(|group| self.groups.get(group))
            .map(|group| self.output_names(&group.outputs))
            .unwrap_or_default()
    }

    fn sync_requests(&self) {
        let mut requests = self.requests.lock().unwrap();
        requests.connection = Some(self.connection.clone());
        requests.seat = self.seat.clone();
        requests.workspace_manager = self.workspace_manager.clone();
        requests.toplevels = self
            .wlr_toplevels
            .iter()
            .map(|(id, toplevel)| (id.clone(), toplevel.handle.clone()))
            .collect();
        requests.workspaces = self
            .workspaces
            .iter()
            .map(|(id, workspace)| (id.clone(), workspace.handle.clone()))
            .collect();
        requests.workspace_caps = self
            .workspaces
            .iter()
            .map(|(id, workspace)| (id.clone(), workspace.capabilities))
            .collect();
    }

    fn store(&self) -> Value {
        let outputs = self
            .outputs
            .values()
            .map(|output| {
                json!({
                    "id": output.id.protocol_id(),
                    "name": output.display_name(),
                    "description": clamp(output.description.clone(), MAX_TEXT),
                    "make": clamp(output.make.clone(), MAX_TEXT),
                    "model": clamp(output.model.clone(), MAX_TEXT),
                    "x": output.x,
                    "y": output.y,
                    "width": output.width,
                    "height": output.height,
                    "scale": output.scale,
                })
            })
            .collect::<Vec<_>>();

        let has_wlr = self.wlr_manager.is_some();
        let toplevels = if has_wlr {
            self.wlr_toplevels
                .iter()
                .map(|(id, toplevel)| {
                    json!({
                        "id": id,
                        "title": clamp(toplevel.title.clone(), MAX_TEXT),
                        "app_id": clamp(toplevel.app_id.clone(), MAX_TEXT),
                        "activated": toplevel.state.activated,
                        "maximized": toplevel.state.maximized,
                        "minimized": toplevel.state.minimized,
                        "fullscreen": toplevel.state.fullscreen,
                        "outputs": self.output_names(&toplevel.outputs),
                    })
                })
                .collect::<Vec<_>>()
        } else {
            self.ext_toplevels
                .iter()
                .map(|(id, toplevel)| {
                    json!({
                        "id": id,
                        "title": clamp(toplevel.title.clone(), MAX_TEXT),
                        "app_id": clamp(toplevel.app_id.clone(), MAX_TEXT),
                        "identifier": toplevel.identifier,
                        "outputs": [],
                    })
                })
                .collect::<Vec<_>>()
        };

        let workspaces = self
            .workspaces
            .iter()
            .map(|(id, workspace)| {
                json!({
                    "id": id,
                    "protocol_id": workspace.protocol_id,
                    "name": clamp(workspace.name.clone(), MAX_TEXT),
                    "coordinates": workspace.coordinates,
                    "active": workspace.state & 1 != 0,
                    "urgent": workspace.state & 2 != 0,
                    "hidden": workspace.state & 4 != 0,
                    "capabilities": workspace_capabilities(workspace.capabilities),
                    "group": workspace.group,
                    "outputs": self.workspace_outputs(workspace),
                })
            })
            .collect::<Vec<_>>();

        let groups = self
            .groups
            .iter()
            .map(|(id, group)| {
                json!({
                    "id": id,
                    "capabilities": group_capabilities(group.capabilities),
                    "outputs": self.output_names(&group.outputs),
                })
            })
            .collect::<Vec<_>>();

        let focused_toplevel = if has_wlr {
            self.wlr_toplevels
                .iter()
                .find(|(_, toplevel)| toplevel.state.activated)
                .map(|(id, _)| id.clone())
        } else {
            None
        };
        let focused_workspace = only(
            self.workspaces
                .iter()
                .filter(|(_, workspace)| workspace.state & 1 != 0),
        )
        .map(|(id, _)| id.clone());
        let focused_output = focused_toplevel
            .as_ref()
            .and_then(|id| self.wlr_toplevels.get(id))
            .and_then(|toplevel| only(toplevel.outputs.iter()))
            .map(|output| self.output_name(output))
            .or_else(|| {
                focused_workspace
                    .as_ref()
                    .and_then(|id| self.workspaces.get(id))
                    .and_then(|workspace| workspace.group.as_ref())
                    .and_then(|id| self.groups.get(id))
                    .and_then(|group| only(group.outputs.iter()))
                    .map(|output| self.output_name(output))
            })
            .or_else(|| only(self.outputs.values()).map(Output::display_name));

        json!({
            "connected": true,
            "capabilities": {
                "toplevels": has_wlr || self.ext_list.is_some(),
                "toplevel_control": has_wlr,
                "workspaces": self.workspace_manager.is_some(),
            },
            "outputs": riced_extension_sdk::bounded_array(outputs, riced_extension_sdk::protocol::MAX_LINE / 8),
            "toplevels": riced_extension_sdk::bounded_array(toplevels, riced_extension_sdk::protocol::MAX_LINE / 8),
            "workspaces": riced_extension_sdk::bounded_array(workspaces, riced_extension_sdk::protocol::MAX_LINE / 8),
            "workspace_groups": riced_extension_sdk::bounded_array(groups, riced_extension_sdk::protocol::MAX_LINE / 8),
            "focused": {
                "toplevel": focused_toplevel,
                "workspace": focused_workspace,
                "output": focused_output.filter(|name| name.len() <= MAX_TEXT),
            },
        })
    }
}

fn only<T>(items: impl IntoIterator<Item = T>) -> Option<T> {
    let mut items = items.into_iter();
    let first = items.next()?;
    items.next().is_none().then_some(first)
}

fn workspace_capabilities(bits: u32) -> Vec<&'static str> {
    let mut names = Vec::new();
    if bits & 1 != 0 {
        names.push("activate");
    }
    if bits & 2 != 0 {
        names.push("deactivate");
    }
    if bits & 4 != 0 {
        names.push("remove");
    }
    if bits & 8 != 0 {
        names.push("assign");
    }
    names
}

fn group_capabilities(bits: u32) -> Vec<&'static str> {
    if bits & 1 != 0 {
        vec!["create_workspace"]
    } else {
        Vec::new()
    }
}

fn enum_bits<T: Copy>(value: WEnum<T>, bits: impl FnOnce(T) -> u32) -> u32 {
    match value {
        WEnum::Value(value) => bits(value),
        WEnum::Unknown(raw) => raw,
    }
}

fn decode_coordinates(bytes: &[u8]) -> Vec<u32> {
    bytes
        .as_chunks::<4>()
        .0
        .iter()
        .map(|chunk| u32::from_ne_bytes(*chunk))
        .collect()
}

fn id(proxy: &impl Proxy) -> String {
    proxy.id().protocol_id().to_string()
}

impl Dispatch<WlRegistry, ()> for Wayland {
    fn event(
        state: &mut Self,
        registry: &WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        match event {
            wl_registry::Event::Global {
                name,
                interface,
                version,
            } => state.bind(registry, name, &interface, version, qh),
            wl_registry::Event::GlobalRemove { name } => state.remove_global(name),
            _ => {}
        }
    }
}

impl Dispatch<wl_output::WlOutput, u32> for Wayland {
    fn event(
        state: &mut Self,
        _: &wl_output::WlOutput,
        event: wl_output::Event,
        global: &u32,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let Some(output) = state.outputs.get_mut(global) else {
            return;
        };
        match event {
            wl_output::Event::Geometry {
                x, y, make, model, ..
            } => {
                output.x = x;
                output.y = y;
                output.make = make;
                output.model = model;
            }
            wl_output::Event::Mode {
                flags,
                width,
                height,
                ..
            } => {
                if matches!(flags, WEnum::Value(flags) if flags.contains(wl_output::Mode::Current))
                {
                    output.width = width;
                    output.height = height;
                }
            }
            wl_output::Event::Scale { factor } => output.scale = factor,
            wl_output::Event::Name { name } => output.name = name,
            wl_output::Event::Description { description } => output.description = description,
            _ => {}
        }
        state.dirty = true;
    }
}

delegate_noop!(Wayland: ignore WlSeat);

impl Dispatch<ZwlrForeignToplevelManagerV1, ()> for Wayland {
    fn event(
        state: &mut Self,
        _: &ZwlrForeignToplevelManagerV1,
        event: zwlr_foreign_toplevel_manager_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            zwlr_foreign_toplevel_manager_v1::Event::Toplevel { toplevel } => {
                state.wlr_toplevels.insert(
                    id(&toplevel),
                    WlrToplevel {
                        handle: toplevel,
                        title: String::new(),
                        app_id: String::new(),
                        state: ToplevelState::default(),
                        outputs: Vec::new(),
                    },
                );
                state.dirty = true;
            }
            zwlr_foreign_toplevel_manager_v1::Event::Finished => {
                state.wlr_manager = None;
                state.wlr_toplevels.clear();
                state.dirty = true;
            }
            _ => {}
        }
    }

    event_created_child!(Wayland, ZwlrForeignToplevelManagerV1, [
        zwlr_foreign_toplevel_manager_v1::EVT_TOPLEVEL_OPCODE => (ZwlrForeignToplevelHandleV1, ())
    ]);
}

impl Dispatch<ZwlrForeignToplevelHandleV1, ()> for Wayland {
    fn event(
        state: &mut Self,
        handle: &ZwlrForeignToplevelHandleV1,
        event: zwlr_foreign_toplevel_handle_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let key = id(handle);
        let Some(toplevel) = state.wlr_toplevels.get_mut(&key) else {
            return;
        };
        match event {
            zwlr_foreign_toplevel_handle_v1::Event::Title { title } => toplevel.title = title,
            zwlr_foreign_toplevel_handle_v1::Event::AppId { app_id } => toplevel.app_id = app_id,
            zwlr_foreign_toplevel_handle_v1::Event::State { state: bytes } => {
                toplevel.state = ToplevelState::decode(&bytes);
            }
            zwlr_foreign_toplevel_handle_v1::Event::OutputEnter { output } => {
                if !toplevel.outputs.contains(&output.id()) {
                    toplevel.outputs.push(output.id());
                }
            }
            zwlr_foreign_toplevel_handle_v1::Event::OutputLeave { output } => {
                toplevel.outputs.retain(|existing| existing != &output.id());
            }
            zwlr_foreign_toplevel_handle_v1::Event::Done => state.dirty = true,
            zwlr_foreign_toplevel_handle_v1::Event::Closed => {
                state.wlr_toplevels.remove(&key);
                handle.destroy();
                state.dirty = true;
            }
            _ => {}
        }
    }
}

impl Dispatch<ExtForeignToplevelListV1, ()> for Wayland {
    fn event(
        state: &mut Self,
        _: &ExtForeignToplevelListV1,
        event: ext_foreign_toplevel_list_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            ext_foreign_toplevel_list_v1::Event::Toplevel { toplevel } => {
                state.ext_toplevels.insert(
                    id(&toplevel),
                    ExtToplevel {
                        title: String::new(),
                        app_id: String::new(),
                        identifier: String::new(),
                    },
                );
                state.dirty = true;
            }
            ext_foreign_toplevel_list_v1::Event::Finished => {
                state.ext_list = None;
                state.ext_toplevels.clear();
                state.dirty = true;
            }
            _ => {}
        }
    }

    event_created_child!(Wayland, ExtForeignToplevelListV1, [
        ext_foreign_toplevel_list_v1::EVT_TOPLEVEL_OPCODE => (ExtForeignToplevelHandleV1, ())
    ]);
}

impl Dispatch<ExtForeignToplevelHandleV1, ()> for Wayland {
    fn event(
        state: &mut Self,
        handle: &ExtForeignToplevelHandleV1,
        event: ext_handle::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let key = id(handle);
        let Some(toplevel) = state.ext_toplevels.get_mut(&key) else {
            return;
        };
        match event {
            ext_handle::Event::Title { title } => toplevel.title = title,
            ext_handle::Event::AppId { app_id } => toplevel.app_id = app_id,
            ext_handle::Event::Identifier { identifier } => toplevel.identifier = identifier,
            ext_handle::Event::Done => state.dirty = true,
            ext_handle::Event::Closed => {
                state.ext_toplevels.remove(&key);
                handle.destroy();
                state.dirty = true;
            }
            _ => {}
        }
    }
}

impl Dispatch<ExtWorkspaceManagerV1, ()> for Wayland {
    fn event(
        state: &mut Self,
        _: &ExtWorkspaceManagerV1,
        event: ext_workspace_manager_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            ext_workspace_manager_v1::Event::WorkspaceGroup { workspace_group } => {
                state.groups.insert(
                    id(&workspace_group),
                    Group {
                        outputs: Vec::new(),
                        capabilities: 0,
                    },
                );
                state.dirty = true;
            }
            ext_workspace_manager_v1::Event::Workspace { workspace } => {
                state.workspaces.insert(
                    id(&workspace),
                    Workspace {
                        handle: workspace,
                        protocol_id: None,
                        name: String::new(),
                        coordinates: Vec::new(),
                        state: 0,
                        capabilities: 0,
                        group: None,
                    },
                );
                state.dirty = true;
            }
            ext_workspace_manager_v1::Event::Done => state.dirty = true,
            ext_workspace_manager_v1::Event::Finished => {
                state.workspace_manager = None;
                state.workspaces.clear();
                state.groups.clear();
                state.dirty = true;
            }
            _ => {}
        }
    }

    event_created_child!(Wayland, ExtWorkspaceManagerV1, [
        ext_workspace_manager_v1::EVT_WORKSPACE_GROUP_OPCODE => (ExtWorkspaceGroupHandleV1, ()),
        ext_workspace_manager_v1::EVT_WORKSPACE_OPCODE => (ExtWorkspaceHandleV1, ())
    ]);
}

impl Dispatch<ExtWorkspaceGroupHandleV1, ()> for Wayland {
    fn event(
        state: &mut Self,
        handle: &ExtWorkspaceGroupHandleV1,
        event: ext_workspace_group_handle_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let key = id(handle);
        let Some(group) = state.groups.get_mut(&key) else {
            return;
        };
        match event {
            ext_workspace_group_handle_v1::Event::Capabilities { capabilities } => {
                group.capabilities = enum_bits(capabilities, |value| value.bits());
            }
            ext_workspace_group_handle_v1::Event::OutputEnter { output } => {
                if !group.outputs.contains(&output.id()) {
                    group.outputs.push(output.id());
                }
            }
            ext_workspace_group_handle_v1::Event::OutputLeave { output } => {
                group.outputs.retain(|existing| existing != &output.id());
            }
            ext_workspace_group_handle_v1::Event::WorkspaceEnter { workspace } => {
                let workspace = id(&workspace);
                if let Some(workspace) = state.workspaces.get_mut(&workspace) {
                    workspace.group = Some(key.clone());
                }
            }
            ext_workspace_group_handle_v1::Event::WorkspaceLeave { workspace } => {
                let workspace = id(&workspace);
                if let Some(workspace) = state.workspaces.get_mut(&workspace)
                    && workspace.group.as_deref() == Some(&key)
                {
                    workspace.group = None;
                }
            }
            ext_workspace_group_handle_v1::Event::Removed => {
                state.groups.remove(&key);
                for workspace in state.workspaces.values_mut() {
                    if workspace.group.as_deref() == Some(&key) {
                        workspace.group = None;
                    }
                }
                handle.destroy();
                state.dirty = true;
            }
            _ => {}
        }
    }
}

impl Dispatch<ExtWorkspaceHandleV1, ()> for Wayland {
    fn event(
        state: &mut Self,
        handle: &ExtWorkspaceHandleV1,
        event: ext_workspace_handle_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let key = id(handle);
        let Some(workspace) = state.workspaces.get_mut(&key) else {
            return;
        };
        match event {
            ext_workspace_handle_v1::Event::Id { id } => workspace.protocol_id = Some(id),
            ext_workspace_handle_v1::Event::Name { name } => workspace.name = name,
            ext_workspace_handle_v1::Event::Coordinates { coordinates } => {
                workspace.coordinates = decode_coordinates(&coordinates);
            }
            ext_workspace_handle_v1::Event::State { state: new_state } => {
                workspace.state = enum_bits(new_state, |value| value.bits());
            }
            ext_workspace_handle_v1::Event::Capabilities { capabilities } => {
                workspace.capabilities = enum_bits(capabilities, |value| value.bits());
            }
            ext_workspace_handle_v1::Event::Removed => {
                state.workspaces.remove(&key);
                handle.destroy();
                state.dirty = true;
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn focus_requires_an_unambiguous_candidate() {
        assert_eq!(only(Vec::<u32>::new()), None);
        assert_eq!(only([1]), Some(1));
        assert_eq!(only([1, 2]), None);
    }

    #[test]
    fn decodes_toplevel_states() {
        let mut bytes = Vec::new();
        for state in [2u32, 3] {
            bytes.extend_from_slice(&state.to_ne_bytes());
        }
        let state = ToplevelState::decode(&bytes);
        assert!(state.activated);
        assert!(state.fullscreen);
        assert!(!state.maximized);
        assert!(!state.minimized);
    }

    #[test]
    fn decodes_workspace_coordinates() {
        let mut bytes = Vec::new();
        for coordinate in [1u32, 4] {
            bytes.extend_from_slice(&coordinate.to_ne_bytes());
        }
        assert_eq!(decode_coordinates(&bytes), [1, 4]);
    }

    #[test]
    fn names_capabilities() {
        assert_eq!(
            workspace_capabilities(1 | 4 | 8),
            ["activate", "remove", "assign"]
        );
        assert_eq!(group_capabilities(1), ["create_workspace"]);
    }

    #[test]
    fn empty_store_is_valid() {
        let store = empty(false);
        assert_eq!(store["connected"], false);
        assert_eq!(store["toplevels"], json!([]));
    }
}
