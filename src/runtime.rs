use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    rc::Rc,
    time::Duration,
};

use iced::widget::operation;
use iced::{Element, Settings, Size, Subscription, Task, Theme, theme, window};
use iced_exwlshell::{
    ExWlSettings,
    actions::{ExWlShellCustomAction, ExWlShellCustomActionWithId, IcedXdgWindowSettings},
    redraw::Scope,
    reexport::KeyboardInteractivity,
    settings::StartMode,
};
use mlua::{Function, Lua, LuaSerdeExt, Table, UserData, Value};
use riced_extension_sdk::log;

use crate::{
    layers, modules, plugins, update,
    update::{Layer, Popup, popup_settings},
    widgets,
    widgets::Ctx,
};

#[derive(Debug, Clone)]
pub enum Message {
    Lua(u64, Option<(String, u64, Option<window::Id>)>, Value),
    Widget(
        u64,
        Option<(String, u64, Option<window::Id>)>,
        String,
        WidgetEvent,
    ),
    Clipboard(u64, Option<String>),
    Key(u64, KeyEvent),
    Mouse(u64, MouseEvent),
    Hover(u64),
    Frame(u64),
    Editor(u64, Option<(String, u64, Option<window::Id>)>, EditorAction),
    Animation {
        generation: u64,
        component: Option<(String, u64, Option<window::Id>)>,
        window: Option<window::Id>,
        control: widgets::animation::Control,
        serial: u64,
        callback: Function,
        owner: Value,
        lifetime: Option<widgets::animation::Control>,
    },
    Tick,
    CheckReload,
    PollPlugins,
    Reloaded,
    RedrawWindow(window::Id),
    Resized(window::Id, Size),
    WindowClosed(window::Id),
    Noop,
    Surface(ExWlShellCustomActionWithId),
}

/// Typed value carried by a widget event; converted to a Lua table on delivery.
#[derive(Debug, Clone)]
pub enum WidgetEvent {
    Bool(bool),
    Number(f32),
    Text(String),
    Point(f32, f32),
    Size(f32, f32),
    Pane(&'static str, usize),
    Scroll {
        x: f32,
        y: f32,
        relative_x: f32,
        relative_y: f32,
        bounds_width: f32,
        bounds_height: f32,
        content_width: f32,
        content_height: f32,
    },
    None,
}

#[derive(Debug, Clone)]
pub struct KeyEvent {
    pub pressed: bool,
    pub key: String,
    pub text: Option<String>,
    pub modifiers: u32,
}

#[derive(Debug, Clone)]
pub struct EditorAction {
    pub id: String,
    pub event: String,
    pub input: bool,
    pub action: iced::widget::text_editor::Action,
}

#[derive(Debug, Clone)]
pub enum MouseEvent {
    Press(String),
    Release(String),
    Move(f32, f32),
    Scroll { x: f32, y: f32 },
    Enter,
    Leave,
}

fn key_payload(lua: &Lua, event: &KeyEvent) -> mlua::Result<Value> {
    let payload = lua.create_table()?;
    payload.set("type", "key")?;
    payload.set("event", if event.pressed { "press" } else { "release" })?;
    payload.set("key", event.key.as_str())?;
    if let Some(text) = &event.text {
        payload.set("text", text.as_str())?;
    }
    let table = lua.create_table()?;
    let flags = iced::keyboard::Modifiers::from_bits_truncate(event.modifiers);
    table.set("shift", flags.shift())?;
    table.set("control", flags.control())?;
    table.set("alt", flags.alt())?;
    table.set("logo", flags.logo())?;
    payload.set("modifiers", table)?;
    Ok(Value::Table(payload))
}

fn mouse_payload(lua: &Lua, event: &MouseEvent) -> mlua::Result<Value> {
    let payload = lua.create_table()?;
    payload.set("type", "mouse")?;
    match event {
        MouseEvent::Press(button) => {
            payload.set("event", "press")?;
            payload.set("button", button.as_str())?;
        }
        MouseEvent::Release(button) => {
            payload.set("event", "release")?;
            payload.set("button", button.as_str())?;
        }
        MouseEvent::Move(x, y) => {
            payload.set("event", "move")?;
            payload.set("x", *x)?;
            payload.set("y", *y)?;
        }
        MouseEvent::Scroll { x, y } => {
            payload.set("event", "scroll")?;
            payload.set("x", *x)?;
            payload.set("y", *y)?;
        }
        MouseEvent::Enter => payload.set("event", "enter")?,
        MouseEvent::Leave => payload.set("event", "leave")?,
    }
    Ok(Value::Table(payload))
}

fn frame_payload(lua: &Lua) -> mlua::Result<Value> {
    let payload = lua.create_table()?;
    payload.set("type", "frame")?;
    Ok(Value::Table(payload))
}

fn editor_payload(
    lua: &Lua,
    component: &Option<(String, u64, Option<window::Id>)>,
    action: &EditorAction,
    text: &str,
) -> mlua::Result<Value> {
    let payload = lua.create_table()?;
    payload.set("event", action.event.as_str())?;
    if let Some((id, _, _)) = component {
        payload.set("widget", id.as_str())?;
    }
    if action.input {
        payload.set("value", text)?;
    } else {
        payload.set("value", action_name(&action.action))?;
        payload.set("text", text)?;
    }
    Ok(Value::Table(payload))
}

fn action_name(action: &iced::widget::text_editor::Action) -> &'static str {
    match action {
        iced::widget::text_editor::Action::Edit(_) => "edit",
        iced::widget::text_editor::Action::Move(_) => "move",
        iced::widget::text_editor::Action::Select(_) => "select",
        iced::widget::text_editor::Action::SelectWord => "select_word",
        iced::widget::text_editor::Action::SelectLine => "select_line",
        iced::widget::text_editor::Action::SelectAll => "select_all",
        iced::widget::text_editor::Action::Click(_) => "click",
        iced::widget::text_editor::Action::Drag(_) => "drag",
        iced::widget::text_editor::Action::Scroll { .. } => "scroll",
    }
}

fn widget_event(
    lua: &Lua,
    component: &Option<(String, u64, Option<window::Id>)>,
    event: &str,
    value: &WidgetEvent,
) -> mlua::Result<Value> {
    let payload = lua.create_table()?;
    payload.set("event", event)?;
    if let Some((id, _, _)) = component {
        payload.set("widget", id.as_str())?;
    }
    match value {
        WidgetEvent::Bool(value) => payload.set("value", *value)?,
        WidgetEvent::Number(value) => payload.set("value", *value)?,
        WidgetEvent::Text(value) => payload.set("value", value.as_str())?,
        WidgetEvent::Point(x, y) => {
            let point = lua.create_table()?;
            point.set("x", *x)?;
            point.set("y", *y)?;
            payload.set("value", point)?;
        }
        WidgetEvent::Size(width, height) => {
            let size = lua.create_table()?;
            size.set("width", *width)?;
            size.set("height", *height)?;
            payload.set("value", size)?;
        }
        WidgetEvent::Pane(action, index) => {
            payload.set("value", *action)?;
            payload.set("pane", *index)?;
        }
        WidgetEvent::Scroll {
            x,
            y,
            relative_x,
            relative_y,
            bounds_width,
            bounds_height,
            content_width,
            content_height,
        } => {
            let viewport = lua.create_table()?;
            viewport.set("x", *x)?;
            viewport.set("y", *y)?;
            viewport.set("relative_x", *relative_x)?;
            viewport.set("relative_y", *relative_y)?;
            viewport.set("bounds_width", *bounds_width)?;
            viewport.set("bounds_height", *bounds_height)?;
            viewport.set("content_width", *content_width)?;
            viewport.set("content_height", *content_height)?;
            payload.set("value", viewport)?;
        }
        WidgetEvent::None => {}
    }
    Ok(Value::Table(payload))
}

// What the 250 ms reload poll wants redrawn. Scope only spans one window per message,
// so several windows produce several RedrawWindow messages.
enum Change {
    None,
    Windows(Vec<window::Id>),
    All,
}

#[derive(Clone)]
struct Mount {
    module: String,
    revision: u64,
    props: Table,
    table: Table,
    control: widgets::animation::Control,
}

type Mounts = Rc<RefCell<HashMap<(window::Id, String), Mount>>>;

impl TryFrom<Message> for ExWlShellCustomActionWithId {
    type Error = Message;

    fn try_from(message: Message) -> Result<Self, Self::Error> {
        match message {
            Message::Surface(action) => Ok(action),
            message => Err(message),
        }
    }
}

#[derive(Clone, Copy)]
struct Window(window::Id);
impl UserData for Window {}

pub struct App {
    lua: Lua,
    state: Table,
    update: Function,
    view: Function,
    options: Table,
    namespace: String,
    path: PathBuf,
    source: Option<String>,
    generation: u64,
    windows: Rc<RefCell<Vec<window::Id>>>,
    layers: Vec<Layer>,
    toplevels: Vec<update::Window>,
    popups: Vec<Popup>,
    surfaces_dirty: Cell<bool>,
    pending: Vec<ExWlShellCustomActionWithId>,
    modules: Rc<RefCell<modules::Modules>>,
    mounts: Mounts,
    windows_of: Rc<RefCell<HashMap<String, Vec<window::Id>>>>,
    sizes: RefCell<HashMap<window::Id, Size>>,
    plugins: Rc<RefCell<plugins::Plugins>>,
}

// Iced's rendering callbacks cannot return errors; a broken Lua callback is fatal.
pub(crate) fn checked<T>(result: mlua::Result<T>) -> T {
    result.unwrap_or_else(|error| {
        log::error!(target: "runtime.error", "{error}");
        std::process::exit(1);
    })
}

// Degrade one render/theme/title/subscription into its fallback instead of exiting.
pub(crate) fn checked_fallback<T>(result: mlua::Result<T>, fallback: T, integer: &str) -> T {
    result.unwrap_or_else(|error| {
        log::error!(target: "runtime.error", "falling back after {integer} error: {error}");
        fallback
    })
}

// `iced.<name>` lazily loads the first `<name>.lua` under the profile. Widgets can
// live anywhere; the shell file is never treated as one. A missing name stays nil.
fn install_widget_loader(lua: &Lua, profile: &Path, shell: &Path) -> mlua::Result<()> {
    let iced: Table = lua.globals().get("iced")?;
    let root = profile.to_owned();
    let shell = shell.to_owned();
    let misses = std::sync::Arc::new(std::sync::Mutex::new(HashSet::new()));
    let loader = lua.create_function(move |lua, (_table, name): (Table, String)| {
        if !widget_name(&name) {
            return Ok(Value::Nil);
        }
        if misses.lock().unwrap().contains(&name) {
            return Ok(Value::Nil);
        }
        let mut found = Vec::new();
        find_widget(&root, &shell, &name, &mut found, &mut HashSet::new());
        match found.len() {
            0 => {
                misses.lock().unwrap().insert(name);
                Ok(Value::Nil)
            }
            1 => {
                let path = &found[0];
                let module = module_name(&root, path);
                let require: Function = lua.globals().get("require")?;
                let _: Value = require.call(module.as_str())?;
                let iced: Table = lua.globals().get("iced")?;
                let value: Value = iced.raw_get(name.as_str())?;
                if value.is_nil() {
                    return Err(mlua::Error::runtime(format!(
                        "{} did not define iced.{name}",
                        path.display()
                    )));
                }
                Ok(value)
            }
            _ => Err(mlua::Error::runtime(format!(
                "multiple files define iced.{name}: {}",
                found
                    .iter()
                    .map(|path| path.display().to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            ))),
        }
    })?;
    let meta = lua.create_table()?;
    meta.set("__index", loader)?;
    iced.set_metatable(Some(meta))?;
    Ok(())
}

fn widget_name(name: &str) -> bool {
    let mut chars = name.chars();
    matches!(chars.next(), Some(first) if first == '_' || first.is_ascii_alphabetic())
        && chars.all(|character| character == '_' || character.is_ascii_alphanumeric())
}

fn find_widget(
    directory: &Path,
    shell: &Path,
    name: &str,
    found: &mut Vec<PathBuf>,
    visited: &mut HashSet<PathBuf>,
) {
    let Ok(canonical) = std::fs::canonicalize(directory) else {
        return;
    };
    if !visited.insert(canonical) {
        return;
    }
    let Ok(entries) = std::fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            find_widget(&path, shell, name, found, visited);
        } else if path != shell
            && path.extension().is_some_and(|extension| extension == "lua")
            && path.file_stem().and_then(|stem| stem.to_str()) == Some(name)
        {
            found.push(path);
        }
    }
}

fn module_name(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .with_extension("")
        .components()
        .filter_map(|component| component.as_os_str().to_str())
        .collect::<Vec<_>>()
        .join(".")
}

#[cfg(test)]
pub fn load(source: &str, path: &Path) -> mlua::Result<App> {
    load_with_plugins(
        source,
        path,
        Rc::new(RefCell::new(plugins::Plugins::detached())),
    )
}

pub fn load_with_plugins(
    source: &str,
    path: &Path,
    plugins: Rc<RefCell<plugins::Plugins>>,
) -> mlua::Result<App> {
    let lua = Lua::new();
    crate::lua::load(&lua)?;
    crate::file::install(&lua, path.parent().unwrap_or(Path::new(".")))?;
    lua.load(
        r##"
        config = { theme = {
            background = "#2b2d31", text = "#e6e6e6", primary = "#5865f2",
            secondary = "#949cbb", success = "#12664f", warning = "#ffc14e", danger = "#c3423f"
        } }
        theme = config.theme
    "##,
    )
    .exec()?;
    if let Some(parent) = path.parent() {
        let package: Table = lua.globals().get("package")?;
        let old_path: String = package.get("path")?;
        package.set(
            "path",
            format!("{0}/?.lua;{0}/?/init.lua;{old_path}", parent.display()),
        )?;
    }
    modules::Modules::install(&lua)?;
    plugins.borrow_mut().install(&lua)?;
    if let Some(parent) = path.parent() {
        install_widget_loader(&lua, parent, path)?;
    }
    let definition: Table = lua.load(source).set_name(path.to_string_lossy()).eval()?;
    if !definition.get::<Option<bool>>("started")?.unwrap_or(false) {
        return Err(mlua::Error::runtime(
            "shell.lua must return riced.daemon(...):run()",
        ));
    }
    let app = App {
        state: definition.get("app")?,
        update: definition.get("update")?,
        view: definition.get("view")?,
        options: definition.get("options")?,
        namespace: definition.get("namespace")?,
        lua,
        path: path.to_owned(),
        source: Some(source.to_owned()),
        generation: 0,
        windows: Rc::new(RefCell::new(Vec::new())),
        layers: Vec::new(),
        toplevels: Vec::new(),
        popups: Vec::new(),
        surfaces_dirty: Cell::new(false),
        pending: Vec::new(),
        modules: Rc::new(RefCell::new(modules::Modules::default())),
        mounts: Rc::new(RefCell::new(HashMap::new())),
        windows_of: Rc::new(RefCell::new(HashMap::new())),
        sizes: RefCell::new(HashMap::new()),
        plugins,
    };
    for name in ["title", "subscription", "style", "redraw_scope"] {
        app.options.get::<Option<Function>>(name)?;
    }
    match app.options.get::<Value>("theme")? {
        Value::Nil | Value::Table(_) | Value::Function(_) => {}
        _ => {
            return Err(mlua::Error::runtime(
                "theme must be a palette table or function",
            ));
        }
    }
    app.settings()?;
    app.modules.borrow_mut().watch_all(&app.lua)?;
    Ok(app)
}

impl App {
    fn call(&self, name: &str, argument: Value) -> mlua::Result<Value> {
        match self.options.get::<Option<Function>>(name)? {
            Some(callback) => callback.call((self.state.clone(), argument)),
            None => Ok(Value::Nil),
        }
    }

    fn window(&self, id: window::Id) -> mlua::Result<Value> {
        Ok(Value::UserData(self.lua.create_userdata(Window(id))?))
    }

    fn stale(&self, component: &Option<(String, u64, Option<window::Id>)>) -> bool {
        let Some((id, revision, window)) = component else {
            return false;
        };
        let Some(window) = window else {
            return true;
        };
        let mounts = self.mounts.borrow();
        mounts.get(&(*window, id.clone())).is_none_or(|mount| {
            mount.revision != *revision
                || self.modules.borrow().revision(&mount.module) != *revision
        })
    }

    fn update(&mut self, message: Message) -> mlua::Result<Task<Message>> {
        if let Message::WindowClosed(id) = message {
            return self.window_closed(id);
        }
        let (event, origin) = match message {
            Message::Animation {
                generation,
                component,
                window,
                control,
                serial,
                callback,
                owner,
                lifetime,
            } if generation == self.generation => {
                if self.stale(&component)
                    || lifetime.is_some_and(|control| !control.state().alive)
                    || window.is_some_and(|id| !self.windows.borrow().contains(&id))
                    || !control.accept(serial)
                {
                    return Ok(Task::none());
                }
                let value = callback.call(owner)?;
                return self.tasks(value);
            }
            Message::Lua(generation, component, value) if generation == self.generation => {
                if self.stale(&component) {
                    return Ok(Task::none());
                }
                if let Value::Function(callback) = value {
                    return self.tasks(callback.call(())?);
                }
                (value, component.map(|(id, _, _)| id))
            }
            Message::Widget(generation, component, event, value)
                if generation == self.generation =>
            {
                if self.stale(&component) {
                    return Ok(Task::none());
                }
                (
                    widget_event(&self.lua, &component, &event, &value)?,
                    component.map(|(id, _, _)| id),
                )
            }
            Message::Clipboard(generation, value) if generation == self.generation => {
                let event = self.lua.create_table()?;
                event.set("type", "clipboard")?;
                if let Some(value) = value {
                    event.set("value", value)?;
                }
                (Value::Table(event), None)
            }
            Message::Key(generation, event) if generation == self.generation => {
                (key_payload(&self.lua, &event)?, None)
            }
            Message::Mouse(generation, event) if generation == self.generation => {
                (mouse_payload(&self.lua, &event)?, None)
            }
            Message::Frame(generation) if generation == self.generation => {
                (frame_payload(&self.lua)?, None)
            }
            Message::Editor(generation, component, action) if generation == self.generation => {
                if self.stale(&component) {
                    return Ok(Task::none());
                }
                let Some(text) = widgets::edit(generation, &action.id, action.action.clone())
                else {
                    return Ok(Task::none());
                };
                (
                    editor_payload(&self.lua, &component, &action, &text)?,
                    component.map(|(id, _, _)| id),
                )
            }
            Message::Lua(..)
            | Message::Animation { .. }
            | Message::Widget(..)
            | Message::Clipboard(..)
            | Message::Key(..)
            | Message::Mouse(..)
            | Message::Hover(..)
            | Message::Frame(..)
            | Message::Editor(..)
            | Message::CheckReload
            | Message::PollPlugins
            | Message::Reloaded
            | Message::RedrawWindow(_)
            | Message::WindowClosed(_)
            | Message::Noop
            | Message::Surface(_) => return Ok(Task::none()),
            Message::Resized(id, size) => {
                self.sizes.borrow_mut().insert(id, size);
                return Ok(Task::none());
            }
            Message::Tick => {
                let event = self.lua.create_table()?;
                event.set("type", "tick")?;
                (Value::Table(event), None)
            }
        };
        let value = self.update.call((self.state.clone(), event, origin))?;
        self.tasks(value)
    }

    fn tasks(&self, value: Value) -> mlua::Result<Task<Message>> {
        let Value::Table(table) = value else {
            return match value {
                Value::Nil => Ok(Task::none()),
                _ => Err(mlua::Error::runtime(
                    "update must return nil, a task, or a list of tasks",
                )),
            };
        };
        if table.get::<Option<String>>("type")?.is_some() {
            return self.task(&table);
        }
        Ok(Task::batch(
            table
                .sequence_values::<Table>()
                .map(|task| task.and_then(|task| self.task(&task)))
                .collect::<mlua::Result<Vec<_>>>()?,
        ))
    }

    fn task(&self, task: &Table) -> mlua::Result<Task<Message>> {
        let kind: String = task.get("type")?;
        match kind.as_str() {
            "focus" => {
                keys(task, &["type", "id"])?;
                let id: String = task.get("id")?;
                Ok(operation::focus(widget_id(&id)))
            }
            "scroll_to" => {
                keys(task, &["type", "id", "x", "y"])?;
                let id: String = task.get("id")?;
                let x = task.get::<Option<f32>>("x")?.unwrap_or(0.0);
                let y = task.get::<Option<f32>>("y")?.unwrap_or(0.0);
                if !x.is_finite() || !y.is_finite() {
                    return Err(mlua::Error::runtime("scroll_to x and y must be finite"));
                }
                Ok(operation::scroll_to(
                    widget_id(&id),
                    operation::AbsoluteOffset {
                        x: Some(x),
                        y: Some(y),
                    },
                ))
            }
            "clipboard_read" => {
                keys(task, &["type"])?;
                let generation = self.generation;
                Ok(iced::clipboard::read().map(move |value| Message::Clipboard(generation, value)))
            }
            "clipboard_write" => {
                keys(task, &["type", "content"])?;
                let content: String = task.get("content")?;
                Ok(iced::clipboard::write::<Message>(content))
            }
            "font" => {
                keys(task, &["type", "path"])?;
                let path: String = task.get("path")?;
                let bytes = std::fs::read(&path).map_err(|error| {
                    mlua::Error::runtime(format!("cannot read font {path}: {error}"))
                })?;
                Ok(iced::font::load(bytes).discard())
            }
            "window" => self.window_task(task),
            "exit" => {
                keys(task, &["type"])?;
                Ok(iced::exit())
            }
            other => Err(mlua::Error::runtime(format!(
                "unknown task {other:?}; use focus, scroll_to, clipboard_read, clipboard_write, font, window, or exit"
            ))),
        }
    }

    fn window_task(&self, task: &Table) -> mlua::Result<Task<Message>> {
        keys(
            task,
            &[
                "type", "action", "window", "width", "height", "x", "y", "mode",
            ],
        )?;
        let id = task
            .get::<Value>("window")?
            .as_userdata()
            .ok_or_else(|| mlua::Error::runtime("window task needs a props.window"))?
            .borrow::<Window>()?
            .0;
        match task.get::<String>("action")?.as_str() {
            "resize" => {
                let width: f32 = task.get("width")?;
                let height: f32 = task.get("height")?;
                if !width.is_finite() || !height.is_finite() || width <= 0.0 || height <= 0.0 {
                    return Err(mlua::Error::runtime(
                        "window resize needs positive width and height",
                    ));
                }
                Ok(iced::window::resize(id, Size::new(width, height)))
            }
            "move" => {
                let x: f32 = task.get("x")?;
                let y: f32 = task.get("y")?;
                if !x.is_finite() || !y.is_finite() {
                    return Err(mlua::Error::runtime("window move needs finite x and y"));
                }
                Ok(iced::window::move_to(id, iced::Point::new(x, y)))
            }
            "close" => Ok(iced::window::close(id)),
            "fullscreen" => Ok(iced::window::set_mode(id, window::Mode::Fullscreen)),
            "windowed" => Ok(iced::window::set_mode(id, window::Mode::Windowed)),
            other => Err(mlua::Error::runtime(format!(
                "unknown window action {other:?}; use resize, move, close, fullscreen, or windowed"
            ))),
        }
    }

    fn poll_plugins(&mut self) -> mlua::Result<(bool, Task<Message>)> {
        let (changed, events) = self.plugins.borrow_mut().poll(&self.lua)?;
        let mut tasks = Vec::new();
        let mut fired = false;
        for plugins::Update::Event(name, event) in events {
            fired = true;
            let result = (|| -> mlua::Result<Task<Message>> {
                let payload = self.lua.create_table()?;
                payload.set("type", "plugin")?;
                payload.set("plugin", name.as_str())?;
                payload.set("event", crate::plugins::lua_json(&self.lua, &event)?)?;
                let value = self.update.call((self.state.clone(), payload))?;
                self.tasks(value)
            })();
            match result {
                Ok(task) => tasks.push(task),
                Err(error) => {
                    log::error!(target: "plugin.event", "plugin {name} event failed: {error}");
                }
            }
        }
        Ok((changed || fired, Task::batch(tasks)))
    }

    fn poll_ipc(&mut self) -> mlua::Result<Task<Message>> {
        let mut tasks = Vec::new();
        let mut handled = false;
        loop {
            let Some(command) = self.plugins.borrow().take_command() else {
                break;
            };
            handled = true;
            tasks.push(self.run_ipc(&command.command, &command.args)?);
        }
        if handled {
            if let Err(error) = self.reconcile_layers() {
                log::error!(target: "ipc.surface", "IPC changed surfaces but the view is invalid: {error}");
            } else {
                tasks.push(self.surface_tasks());
            }
        }
        Ok(Task::batch(tasks))
    }

    fn window_closed(&mut self, id: window::Id) -> mlua::Result<Task<Message>> {
        let (surface, on_closed) = if let Some(toplevel) = self
            .toplevels
            .iter_mut()
            .find(|toplevel| toplevel.window == id)
        {
            toplevel.closed = true;
            (toplevel.id.clone(), None)
        } else if let Some(popup) = self.popups.iter_mut().find(|popup| popup.window == id) {
            if popup.closed {
                return Ok(Task::none());
            }
            popup.closed = true;
            (popup.id.clone(), popup.on_closed.clone())
        } else {
            return Ok(Task::none());
        };
        self.forget_window(id)?;
        let event = self.lua.create_table()?;
        event.set("type", "window_closed")?;
        event.set("id", surface.as_str())?;
        let task = match self.update.call::<Value>((self.state.clone(), event)) {
            Ok(value) => match self.tasks(value) {
                Ok(task) => task,
                Err(error) => {
                    log::error!(target: "window.close", "window close handler returned an invalid task: {error}");
                    Task::none()
                }
            },
            Err(error) => {
                log::error!(target: "window.close", "window close handler failed: {error}");
                Task::none()
            }
        };
        if let Some(callback) = on_closed
            && let Err(error) = callback.call::<()>(())
        {
            log::error!(target: "popup.close", "popup on_closed callback failed: {error}");
        }
        if let Err(error) = self.reconcile_layers() {
            log::error!(target: "window.close", "window close changed surfaces but the view is invalid: {error}");
        }
        Ok(task.chain(self.surface_tasks()))
    }

    fn run_ipc(&mut self, command: &str, args: &[String]) -> mlua::Result<Task<Message>> {
        let handlers: Table = self.lua.named_registry_value("riced.ipc_handlers")?;
        let Some(handler) = handlers.get::<Option<Function>>(command)? else {
            log::warn!(target: "ipc.command", "no IPC handler for {command:?}");
            return Ok(Task::none());
        };
        let args = self.lua.to_value(args)?;
        match handler.call::<Value>((self.state.clone(), args)) {
            Ok(value) => match self.tasks(value) {
                Ok(task) => Ok(task.chain(Task::done(Message::Reloaded))),
                Err(error) => {
                    log::error!(target: "ipc.command", "IPC {command:?} returned an invalid task: {error}");
                    Ok(Task::none())
                }
            },
            Err(error) => {
                log::error!(target: "ipc.command", "IPC {command:?} failed: {error}");
                Ok(Task::none())
            }
        }
    }

    fn view_node(&self, id: window::Id) -> mlua::Result<Table> {
        self.animations()?
            .get::<Function>("begin")?
            .call::<()>((format!("{id:?}"), self.windows.borrow().contains(&id)))?;
        let props = self.lua.create_table()?;
        props.set("window", self.window(id)?)?;
        if let Some(size) = self.sizes.borrow().get(&id) {
            props.set("width", size.width)?;
            props.set("height", size.height)?;
        }
        let node = widgets::measure::with_window(Some(id), || {
            self.view.call((self.state.clone(), props))
        })?;
        let node = layers::hoist_popups(&self.lua, node)?;
        self.modules.borrow_mut().watch_all(&self.lua)?;
        Ok(node)
    }

    fn animations(&self) -> mlua::Result<Table> {
        self.lua.globals().get::<Table>("riced")?.get("_animations")
    }
}

impl App {
    // Replaces iced.component nodes with their module's rendered children. Children are
    // rebuilt on every render, so props stay fresh; only the module itself survives reloads.
    fn expand(
        &self,
        node: Table,
        window: window::Id,
        path: &mut Vec<String>,
        ids: &mut HashSet<String>,
    ) -> mlua::Result<Table> {
        let node = copy_table(&self.lua, &node)?;
        let expanded = match node.get::<Option<String>>("kind")?.as_deref() {
            Some("component") => self.expand_component(node, window, path, ids),
            Some("row" | "column" | "stack" | "grid" | "pane_grid") => {
                let children = node
                    .get::<Table>("content")?
                    .sequence_values::<Table>()
                    .collect::<mlua::Result<Vec<_>>>()?;
                let list = self.lua.create_table()?;
                for (index, child) in children.into_iter().enumerate() {
                    list.set(index + 1, self.expand(child, window, path, ids)?)?;
                }
                node.set("content", list)?;
                Ok(node)
            }
            Some(
                "container" | "button" | "layer" | "region" | "scrollable" | "float" | "pin"
                | "tooltip" | "mouse_area" | "sensor",
            ) => {
                if let Value::Table(child) = node.get::<Value>("content")? {
                    node.set("content", self.expand(child, window, path, ids)?)?;
                }
                Ok(node)
            }
            Some("table") => {
                for field in ["content", "columns"] {
                    let output = self.lua.create_table()?;
                    for (index, row) in node
                        .get::<Table>(field)?
                        .sequence_values::<Table>()
                        .enumerate()
                    {
                        let row = copy_table(&self.lua, &row?)?;
                        if field == "columns" {
                            if let Value::Table(header) = row.get::<Value>("header")? {
                                row.set("header", self.expand(header, window, path, ids)?)?;
                            }
                        } else {
                            let cells = row
                                .sequence_values::<Value>()
                                .collect::<mlua::Result<Vec<_>>>()?;
                            for (column, value) in cells.into_iter().enumerate() {
                                if let Value::Table(cell) = value {
                                    row.set(column + 1, self.expand(cell, window, path, ids)?)?;
                                }
                            }
                        }
                        output.set(index + 1, row)?;
                    }
                    node.set(field, output)?;
                }
                Ok(node)
            }
            _ => Ok(node),
        }?;
        if let Some(owner) = expanded.get::<Option<Table>>("__animation_self")? {
            self.animations()?
                .get::<Function>("snapshot")?
                .call::<()>((owner.get::<String>("__animation_key")?, expanded.clone()))?;
        }
        Ok(expanded)
    }

    fn expand_component(
        &self,
        node: Table,
        window: window::Id,
        path: &mut Vec<String>,
        ids: &mut HashSet<String>,
    ) -> mlua::Result<Table> {
        let id: String = node.get("id")?;
        let module: String = node.get("module")?;
        if id.is_empty() || module.is_empty() {
            return Err(mlua::Error::runtime(
                "component id and module must be nonempty strings",
            ));
        }
        if path.contains(&id) {
            return Err(mlua::Error::runtime(format!(
                "component cycle through {id}"
            )));
        }
        if !ids.insert(id.clone()) {
            return Err(mlua::Error::runtime(format!(
                "duplicate component id: {id}"
            )));
        }
        let props = match node.get::<Option<Table>>("props")? {
            Some(props) => props,
            None => self.lua.create_table()?,
        };
        let table = self.modules.borrow_mut().require(&self.lua, &module)?;
        let view: Function = table.get("view").map_err(|_| {
            mlua::Error::runtime(format!("module {module} must define a view function"))
        })?;
        let revision = self.modules.borrow().revision(&module);
        let _ = view;
        let key = format!("component:{id}");
        if self
            .mounts
            .borrow()
            .get(&(window, id.clone()))
            .is_some_and(|mount| mount.revision != revision)
        {
            self.animations()?
                .get::<Function>("remove")?
                .call::<()>(key.clone())?;
        }
        let (child, instance): (Table, Table) = self
            .animations()?
            .get::<Function>("render")?
            .call((key, table, props.clone(), self.window(window)?))?;
        let control = child
            .get::<mlua::AnyUserData>("__animation")?
            .borrow::<widgets::animation::Control>()?
            .clone();
        let mut windows_of = self.windows_of.borrow_mut();
        let owners = windows_of.entry(module.clone()).or_default();
        if !owners.contains(&window) {
            owners.push(window);
        }
        drop(windows_of);
        path.push(id.clone());
        let owner: Function = self.animations()?.get("owner")?;
        let previous: Value = owner.call(format!("component:{id}"))?;
        let child = self.expand(child, window, path, ids);
        owner.call::<()>(previous)?;
        let child = child?;
        path.pop();
        // The discovery pass renders with a throwaway window id; only tracked windows
        // get mounts and lifecycle signals.
        if self.windows.borrow().contains(&window) {
            let mounted = (window, id.clone());
            let first = !self.mounts.borrow().contains_key(&mounted);
            self.mounts.borrow_mut().insert(
                mounted,
                Mount {
                    module,
                    revision,
                    props: props.clone(),
                    table: instance.clone(),
                    control,
                },
            );
            if first && let Some(hook) = instance.get::<Option<Function>>("on_visible")? {
                hook.call::<()>((instance, props, self.window(window)?))?;
            }
        }
        child.set("__component_id", id)?;
        child.set("__component_revision", revision)?;
        Ok(child)
    }

    // Components expanded in this render are in `ids`; the rest of this window's mounts
    // disappeared from the tree.
    fn destroy_unmounted(&self, window: window::Id, ids: &HashSet<String>) -> mlua::Result<()> {
        let destroyed = self
            .mounts
            .borrow()
            .iter()
            .filter(|((owner, id), mount)| {
                *owner == window && !ids.contains(id) && mount.control.removable()
            })
            .map(|(key, mount)| (key.clone(), mount.clone()))
            .collect::<Vec<_>>();
        for (key, mount) in destroyed {
            self.mounts.borrow_mut().remove(&key);
            if let Some(hook) = mount.table.get::<Option<Function>>("on_destroyed")? {
                hook.call::<()>((mount.table, mount.props, self.window(window)?))?;
            }
        }
        Ok(())
    }
}

impl App {
    fn prepare_layers(&mut self) -> mlua::Result<()> {
        let node = self.view_node(window::Id::unique())?;
        let mut layers = Vec::new();
        let mut toplevels = Vec::new();
        let mut popups = Vec::new();
        let mut ids = HashSet::new();
        for (index, node) in layers::nodes(node)?.into_iter().enumerate() {
            let settings = node.get("settings")?;
            let id = layers::id(&settings, index)?;
            if !ids.insert(id.clone()) {
                return Err(mlua::Error::runtime(format!("duplicate surface id: {id}")));
            }
            let kind: String = node.get("kind")?;
            if node.get::<Option<Table>>("content")?.is_none() {
                return Err(mlua::Error::runtime(
                    "surface content must be an iced element",
                ));
            }
            if kind == "window" {
                let window = layers::window(settings)?;
                toplevels.push(update::Window {
                    id,
                    window: window::Id::unique(),
                    title: window.title,
                    size: window.size,
                    client_side_decorations: window.client_side_decorations,
                    closed: false,
                });
            } else if kind == "popup" {
                let popup = layers::popup(settings)?;
                popups.push(Popup {
                    id,
                    window: window::Id::unique(),
                    parent: popup.parent,
                    parent_window: None,
                    size: popup.size,
                    position: popup.position,
                    anchor: popup.anchor,
                    anchor_edge: popup.anchor_edge,
                    gravity: popup.gravity,
                    constraint_adjustment: popup.constraint_adjustment,
                    on_closed: popup.on_closed,
                    closed: false,
                });
            } else {
                layers.push(Layer {
                    id,
                    window: window::Id::unique(),
                    settings: layers::settings(settings)?,
                });
            }
        }
        let layer_ids = layers
            .iter()
            .map(|layer| layer.id.as_str())
            .collect::<HashSet<_>>();
        for popup in &mut popups {
            if !layer_ids.contains(popup.parent.as_str()) {
                return Err(mlua::Error::runtime(format!(
                    "popup {} parent {:?} must be an iced.layer declared in the same view",
                    popup.id, popup.parent
                )));
            }
            popup.parent_window = layers
                .iter()
                .find(|layer| layer.id == popup.parent)
                .map(|layer| layer.window);
        }
        self.layers = layers;
        self.toplevels = toplevels;
        self.popups = popups;
        Ok(())
    }

    fn layer_tasks(&self) -> Task<Message> {
        let mut tasks = Task::none();
        for layer in &self.layers {
            tasks = tasks.chain(Task::done(Message::Surface(
                ExWlShellCustomActionWithId::new(
                    None,
                    ExWlShellCustomAction::NewLayerShell {
                        id: layer.window,
                        settings: layer.settings.clone(),
                    },
                ),
            )));
        }
        for window in &self.toplevels {
            tasks = tasks.chain(Task::done(Message::Surface(
                ExWlShellCustomActionWithId::new(
                    None,
                    ExWlShellCustomAction::NewBaseWindow {
                        settings: IcedXdgWindowSettings {
                            size: Some(window.size),
                            client_side_decorations: window.client_side_decorations,
                            app_id: None,
                        },
                        id: window.window,
                    },
                ),
            )));
        }
        for popup in &self.popups {
            if popup.closed {
                continue;
            }
            let Some(parent) = self
                .layers
                .iter()
                .find(|layer| layer.id == popup.parent)
                .map(|layer| layer.window)
            else {
                continue;
            };
            tasks = tasks.chain(Task::done(Message::Surface(
                ExWlShellCustomActionWithId::new(
                    None,
                    ExWlShellCustomAction::NewPopUp {
                        settings: popup_settings(popup, parent),
                        id: popup.window,
                    },
                ),
            )));
        }
        tasks
    }

    fn reconcile_layers(&mut self) -> mlua::Result<()> {
        let previous = self.layers.clone();
        let previous_toplevels = self.toplevels.clone();
        let previous_popups = self.popups.clone();
        let had_surfaces =
            !previous.is_empty() || !previous_toplevels.is_empty() || !previous_popups.is_empty();
        self.prepare_layers()?;
        let has_surfaces =
            !self.layers.is_empty() || !self.toplevels.is_empty() || !self.popups.is_empty();
        if !previous.is_empty() && !has_surfaces {
            let has_base = self.windows.borrow().iter().any(|id| {
                !previous.iter().any(|layer| layer.window == *id)
                    && !previous_toplevels.iter().any(|window| window.window == *id)
            });
            if !has_base {
                self.layers = previous;
                self.toplevels = previous_toplevels.clone();
                return Err(mlua::Error::runtime(
                    "cannot replace layer views with a widget-only view without a base window",
                ));
            }
        }
        self.pending
            .extend(update::reconcile(&previous, &mut self.layers));
        self.pending.extend(update::reconcile_windows(
            &previous_toplevels,
            &mut self.toplevels,
        ));
        let parents = self
            .layers
            .iter()
            .map(|layer| (layer.id.clone(), layer.window))
            .collect::<HashMap<_, _>>();
        self.pending.extend(update::reconcile_popups(
            &previous_popups,
            &mut self.popups,
            &parents,
        ));
        if had_surfaces {
            let mut live = self
                .layers
                .iter()
                .map(|layer| layer.window)
                .chain(
                    self.toplevels
                        .iter()
                        .filter(|window| !window.closed)
                        .map(|window| window.window),
                )
                .chain(
                    self.popups
                        .iter()
                        .filter(|popup| !popup.closed)
                        .map(|popup| popup.window),
                )
                .collect::<HashSet<_>>();
            if !has_surfaces {
                live.extend(self.windows.borrow().iter().copied().filter(|id| {
                    !previous.iter().any(|layer| layer.window == *id)
                        && !previous_toplevels.iter().any(|window| window.window == *id)
                }));
            }
            let removed = self
                .windows
                .borrow()
                .iter()
                .copied()
                .filter(|id| !live.contains(id))
                .collect::<Vec<_>>();
            for id in removed {
                self.forget_window(id)?;
            }
        }
        Ok(())
    }

    fn forget_window(&self, id: window::Id) -> mlua::Result<()> {
        self.animations()?
            .get::<Function>("forget")?
            .call::<()>(format!("{id:?}"))?;
        self.windows.borrow_mut().retain(|window| *window != id);
        self.windows_of.borrow_mut().retain(|_, windows| {
            windows.retain(|window| *window != id);
            !windows.is_empty()
        });
        self.sizes.borrow_mut().remove(&id);
        widgets::measure::forget_window(id);
        widgets::forget_window(self.generation, id);
        let mounts = self
            .mounts
            .borrow()
            .iter()
            .filter(|((window, _), _)| *window == id)
            .map(|(key, mount)| (key.clone(), mount.clone()))
            .collect::<Vec<_>>();
        self.mounts
            .borrow_mut()
            .retain(|(window, _), _| *window != id);
        for (_, mount) in mounts {
            if let Some(hook) = mount.table.get::<Option<Function>>("on_destroyed")? {
                hook.call::<()>((mount.table, mount.props, self.window(id)?))?;
            }
        }
        Ok(())
    }

    fn surface_tasks(&mut self) -> Task<Message> {
        std::mem::take(&mut self.pending)
            .into_iter()
            .fold(Task::none(), |task, action| {
                task.chain(Task::done(Message::Surface(action)))
            })
    }

    fn reload_tasks(&mut self, change: Change) -> Task<Message> {
        let surfaces = self.surface_tasks();
        surfaces.chain(match change {
            Change::All => Task::done(Message::Reloaded),
            Change::Windows(ids) => Task::batch(
                ids.into_iter()
                    .map(|id| Task::done(Message::RedrawWindow(id))),
            ),
            Change::None => Task::none(),
        })
    }

    fn view(&self, id: window::Id) -> mlua::Result<Element<'static, Message>> {
        if self
            .toplevels
            .iter()
            .any(|window| window.window == id && window.closed)
            || self
                .popups
                .iter()
                .any(|popup| popup.window == id && popup.closed)
        {
            return Ok(iced::widget::Space::new().into());
        }
        let events_transparent = self
            .layers
            .iter()
            .any(|layer| layer.window == id && layer.settings.events_transparent);
        // Registered before rendering so component mount/unmount signals skip the
        // surface-discovery pass, which renders with a throwaway window id.
        if !self.windows.borrow().contains(&id) {
            self.windows.borrow_mut().push(id);
        }
        let mut node = self.view_node(id)?;
        if !self.layers.is_empty() || !self.toplevels.is_empty() || !self.popups.is_empty() {
            let nodes = layers::nodes(node)?;
            if nodes.len() != self.layers.len() + self.toplevels.len() + self.popups.len() {
                self.surfaces_dirty.set(true);
            }
            let mut by_id = HashMap::new();
            for (index, node) in nodes.into_iter().enumerate() {
                let settings = node.get("settings")?;
                let id = layers::id(&settings, index)?;
                if node.get::<String>("kind")? == "window" {
                    let window = layers::window(settings)?;
                    match self.toplevels.iter().find(|toplevel| toplevel.id == id) {
                        Some(toplevel)
                            if window.size == toplevel.size
                                && window.title == toplevel.title
                                && window.client_side_decorations
                                    == toplevel.client_side_decorations => {}
                        _ => self.surfaces_dirty.set(true),
                    }
                } else if node.get::<String>("kind")? == "popup" {
                    let popup = layers::popup(settings)?;
                    match self.popups.iter().find(|current| current.id == id) {
                        Some(current)
                            if popup.parent == current.parent
                                && popup.size == current.size
                                && popup.position == current.position
                                && popup.anchor == current.anchor
                                && popup.anchor_edge == current.anchor_edge
                                && popup.gravity == current.gravity
                                && popup.constraint_adjustment == current.constraint_adjustment => {
                        }
                        _ => self.surfaces_dirty.set(true),
                    }
                } else if let Some(layer) = self.layers.iter().find(|layer| layer.id == id) {
                    if layers::settings(settings)? != layer.settings {
                        self.surfaces_dirty.set(true);
                    }
                } else {
                    self.surfaces_dirty.set(true);
                }
                if by_id.insert(id, node).is_some() {
                    return Err(mlua::Error::runtime("duplicate surface id"));
                }
            }
            let surface = self
                .layers
                .iter()
                .find(|layer| layer.window == id)
                .map(|layer| layer.id.as_str())
                .or_else(|| {
                    self.toplevels
                        .iter()
                        .find(|toplevel| toplevel.window == id)
                        .map(|toplevel| toplevel.id.as_str())
                })
                .or_else(|| {
                    self.popups
                        .iter()
                        .find(|popup| popup.window == id)
                        .map(|popup| popup.id.as_str())
                });
            // A replaced surface can receive a final render before its close action runs.
            let Some(surface) = surface else {
                self.surfaces_dirty.set(true);
                return Ok(widgets::with_input_regions(
                    iced::widget::Space::new().into(),
                    id,
                    events_transparent,
                ));
            };
            let Some(surface_node) = by_id.remove(surface) else {
                self.surfaces_dirty.set(true);
                return Ok(widgets::with_input_regions(
                    iced::widget::Space::new().into(),
                    id,
                    events_transparent,
                ));
            };
            node = surface_node.get("content")?;
        }
        node = self.animations()?.get::<Function>("select")?.call(node)?;
        let mut path = Vec::new();
        let mut ids = HashSet::new();
        let node = self.expand(node, id, &mut path, &mut ids)?;
        let element = widgets::element(
            node,
            &Ctx::new(self.generation)
                .with_lua(self.lua.clone())
                .with_window(id),
        )?;
        let leaving: Table = self.animations()?.get::<Function>("finish")?.call(())?;
        self.destroy_unmounted(id, &ids)?;
        widgets::sweep_animations();
        let mut elements = vec![element];
        for node in leaving.sequence_values::<Table>() {
            let child = widgets::element(
                node?,
                &Ctx::new(self.generation)
                    .with_lua(self.lua.clone())
                    .with_window(id),
            )?;
            elements.push(iced::widget::pin(child).into());
        }
        let element = if elements.len() == 1 {
            elements.remove(0)
        } else {
            iced::widget::Stack::with_children(elements)
                .width(iced::Length::Fill)
                .height(iced::Length::Fill)
                .into()
        };
        Ok(widgets::with_input_regions(element, id, events_transparent))
    }

    fn theme(&self, id: window::Id) -> mlua::Result<Theme> {
        let value = match self.options.get::<Value>("theme")? {
            Value::Function(callback) => callback.call((self.state.clone(), self.window(id)?))?,
            value => value,
        };
        let palette = match value {
            Value::Nil => self
                .lua
                .globals()
                .get::<Table>("config")?
                .get::<Table>("theme")?,
            Value::Table(table) => table,
            _ => {
                return Err(mlua::Error::runtime(
                    "theme callback must return a palette table or nil",
                ));
            }
        };
        let mut colors = theme::Palette::DARK;
        for (name, color) in [
            ("background", &mut colors.background),
            ("text", &mut colors.text),
            ("primary", &mut colors.primary),
            ("success", &mut colors.success),
            ("warning", &mut colors.warning),
            ("danger", &mut colors.danger),
        ] {
            if let Some(value) = palette.get::<Option<String>>(name)? {
                *color = widgets::color(&value)?;
            }
        }
        if let Some(value) = palette.get::<Option<String>>("secondary")? {
            widgets::color(&value)?;
        }
        Ok(Theme::custom(self.namespace.clone(), colors))
    }

    fn style(&self, theme: &Theme) -> mlua::Result<theme::Style> {
        let palette = theme.palette();
        let mut style = theme::Style {
            background_color: palette.background,
            text_color: palette.text,
        };
        let colors: Table = self.lua.globals().get::<Table>("config")?.get("theme")?;
        match self.call("style", Value::Table(colors))? {
            Value::Nil => {}
            Value::Table(table) => {
                for (key, color) in [
                    ("background_color", &mut style.background_color),
                    ("text_color", &mut style.text_color),
                ] {
                    if let Some(value) = table.get::<Option<String>>(key)? {
                        *color = widgets::color(&value)?;
                    }
                }
            }
            _ => return Err(mlua::Error::runtime("style must return a table or nil")),
        }
        Ok(style)
    }

    fn subscription(&self) -> mlua::Result<Subscription<Message>> {
        let value = self.call("subscription", Value::Boolean(true))?;
        let descriptors = match value {
            Value::Nil => Vec::new(),
            Value::Table(table) if table.get::<Option<f64>>("every")?.is_some() => vec![table],
            Value::Table(table) if table.get::<Option<String>>("type")?.is_some() => vec![table],
            Value::Table(list) => list
                .sequence_values::<Table>()
                .collect::<mlua::Result<Vec<_>>>()?,
            _ => {
                return Err(mlua::Error::runtime(
                    "subscription must return a subscription, a list, or nil",
                ));
            }
        };
        let mut subscriptions = Vec::new();
        for descriptor in descriptors {
            subscriptions.push(self.subscription_item(&descriptor)?);
        }
        subscriptions.push(window::events().filter_map(|(id, event)| match event {
            window::Event::Opened { size, .. } | window::Event::Resized(size) => {
                Some(Message::Resized(id, size))
            }
            window::Event::Closed => Some(Message::WindowClosed(id)),
            _ => None,
        }));
        // Popup surfaces do not repaint on their own cursor motion; refresh the open
        // popup so hover styles update on the next frame.
        if self.popups.iter().any(|popup| !popup.closed) {
            subscriptions.push(
                iced::event::listen()
                    .filter_map(|event| match event {
                        iced::Event::Mouse(iced::mouse::Event::CursorMoved { .. }) => Some(()),
                        _ => None,
                    })
                    .with(self.generation)
                    .map(|(generation, ())| Message::Hover(generation)),
            );
        }
        Ok(Subscription::batch(subscriptions))
    }

    fn subscription_item(&self, descriptor: &Table) -> mlua::Result<Subscription<Message>> {
        if let Some(ms) = descriptor.get::<Option<f64>>("every")? {
            keys(descriptor, &["every"])?;
            if !(1.0..=86_400_000.0).contains(&ms) || ms.fract() != 0.0 {
                return Err(mlua::Error::runtime(
                    "timer interval must be a whole number from 1 to 86400000 ms",
                ));
            }
            return Ok(iced::time::every(Duration::from_millis(ms as u64)).map(|_| Message::Tick));
        }
        let generation = self.generation;
        match descriptor.get::<String>("type")?.as_str() {
            "frames" => {
                keys(descriptor, &["type"])?;
                Ok(window::frames().with(generation).map(frame_message))
            }
            "key" => {
                keys(descriptor, &["type", "pressed"])?;
                let pressed = descriptor.get::<Option<bool>>("pressed")?;
                Ok(iced::event::listen()
                    .filter_map(keyboard_event)
                    .with((generation, pressed))
                    .filter_map(key_message))
            }
            "mouse" => {
                keys(descriptor, &["type"])?;
                Ok(iced::event::listen()
                    .filter_map(mouse_event)
                    .with(generation)
                    .map(mouse_message))
            }
            other => Err(mlua::Error::runtime(format!(
                "unknown subscription {other:?}; use every, frames, key, or mouse"
            ))),
        }
    }

    fn redraw(&self, message: &Message) -> mlua::Result<Scope> {
        let event = match message {
            Message::Animation {
                generation, window, ..
            } => {
                return Ok(if *generation == self.generation {
                    window.map(Scope::Window).unwrap_or(Scope::All)
                } else {
                    Scope::None
                });
            }
            Message::CheckReload | Message::PollPlugins => return Ok(Scope::None),
            Message::Reloaded => return Ok(Scope::All),
            Message::Noop | Message::Surface(_) => return Ok(Scope::None),
            Message::RedrawWindow(id) => return Ok(Scope::Window(*id)),
            Message::Resized(id, _) => return Ok(Scope::Window(*id)),
            Message::WindowClosed(_) => return Ok(Scope::All),
            Message::Key(generation, event) if *generation == self.generation => {
                key_payload(&self.lua, event)?
            }
            Message::Mouse(generation, event) if *generation == self.generation => {
                mouse_payload(&self.lua, event)?
            }
            Message::Hover(generation) if *generation == self.generation => {
                return Ok(self
                    .popups
                    .iter()
                    .find(|popup| !popup.closed)
                    .map_or(Scope::None, |popup| Scope::Window(popup.window)));
            }
            Message::Hover(..) => return Ok(Scope::None),
            Message::Frame(generation) if *generation == self.generation => {
                frame_payload(&self.lua)?
            }
            Message::Editor(generation, component, action) if *generation == self.generation => {
                if self.stale(component) {
                    return Ok(Scope::None);
                }
                let text = widgets::editor_text(*generation, &action.id).unwrap_or_default();
                editor_payload(&self.lua, component, action, &text)?
            }
            Message::Key(..) | Message::Mouse(..) | Message::Frame(..) | Message::Editor(..) => {
                return Ok(Scope::None);
            }
            Message::Clipboard(generation, value) if *generation == self.generation => {
                let event = self.lua.create_table()?;
                event.set("type", "clipboard")?;
                if let Some(value) = value {
                    event.set("value", value.as_str())?;
                }
                Value::Table(event)
            }
            Message::Clipboard(..) => return Ok(Scope::None),
            Message::Lua(generation, _, Value::Function(_)) if *generation == self.generation => {
                return Ok(Scope::All);
            }
            Message::Lua(generation, _, value) if *generation == self.generation => value.clone(),
            Message::Lua(..) => return Ok(Scope::None),
            Message::Widget(generation, component, event, value)
                if *generation == self.generation =>
            {
                widget_event(&self.lua, component, event, value)?
            }
            Message::Widget(..) => return Ok(Scope::None),
            Message::Tick => {
                let event = self.lua.create_table()?;
                event.set("type", "tick")?;
                Value::Table(event)
            }
        };
        match self.call("redraw_scope", event)? {
            Value::Nil => Ok(Scope::All),
            Value::String(s) if s.to_str()? == "all" => Ok(Scope::All),
            Value::String(s) if s.to_str()? == "none" => Ok(Scope::None),
            Value::UserData(id) => Ok(Scope::Window(id.borrow::<Window>()?.0)),
            _ => Err(mlua::Error::runtime(
                "redraw_scope must return 'all', 'none', a props.window, or nil",
            )),
        }
    }

    fn settings(&self) -> mlua::Result<(Settings, ExWlSettings)> {
        let mut settings = Settings::default();
        let mut wl_settings = ExWlSettings::default();
        // Vendored Lucide font for iced.icon / iced.lucide (extension-owned lookup).
        settings.fonts.push(std::borrow::Cow::Borrowed(
            include_bytes!("fonts/lucide.ttf") as &[u8],
        ));
        let Some(table) = self.options.get::<Option<Table>>("settings")? else {
            return Ok((settings, wl_settings));
        };
        keys(&table, &["id", "layer_settings"])?;
        settings.id = table.get("id")?;
        if let Some(layer) = table.get::<Option<Table>>("layer_settings")? {
            keys(&layer, &["start_mode", "keyboard_interactivity"])?;
            if let Some(mode) = layer.get::<Option<String>>("start_mode")? {
                wl_settings.layer_settings.start_mode = match mode.as_str() {
                    "background" => StartMode::Background,
                    "active" => StartMode::Active,
                    "all_screens" => StartMode::AllScreens,
                    _ => return Err(mlua::Error::runtime(format!("unknown start_mode: {mode}"))),
                };
            }
            if let Some(mode) = layer.get::<Option<String>>("keyboard_interactivity")? {
                wl_settings.layer_settings.keyboard_interactivity = match mode.as_str() {
                    "none" => KeyboardInteractivity::None,
                    "on_demand" => KeyboardInteractivity::OnDemand,
                    "exclusive" => KeyboardInteractivity::Exclusive,
                    _ => {
                        return Err(mlua::Error::runtime(format!(
                            "unknown keyboard_interactivity: {mode}"
                        )));
                    }
                };
            }
        }
        Ok((settings, wl_settings))
    }

    fn reload(&mut self, source: &str) -> mlua::Result<()> {
        let result = self.replace(source);
        self.plugins.borrow_mut().retain_pending(&self.lua);
        result
    }

    fn replace(&mut self, source: &str) -> mlua::Result<()> {
        let mut candidate = load_with_plugins(source, &self.path, Rc::clone(&self.plugins))?;
        let current = self.settings()?;
        let next = candidate.settings()?;
        if self.namespace != candidate.namespace
            || current.0.id != next.0.id
            || current.1.layer_settings.keyboard_interactivity
                != next.1.layer_settings.keyboard_interactivity
            || std::mem::discriminant(&current.1.layer_settings.start_mode)
                != std::mem::discriminant(&next.1.layer_settings.start_mode)
        {
            return Err(mlua::Error::runtime(
                "namespace or startup settings changed; restart riced to apply them",
            ));
        }
        candidate.generation = self.generation + 1;
        candidate.layers = self.layers.clone();
        candidate.toplevels = self.toplevels.clone();
        candidate.popups = self.popups.clone();
        *candidate.windows.borrow_mut() = self.windows.borrow().clone();
        *candidate.sizes.borrow_mut() = self.sizes.borrow().clone();
        candidate.reconcile_layers()?;
        for layer in &candidate.layers {
            candidate.view(layer.window)?;
        }
        for toplevel in &candidate.toplevels {
            candidate.view(toplevel.window)?;
        }
        for popup in &candidate.popups {
            candidate.view(popup.window)?;
        }
        for &id in candidate.windows.borrow().clone().iter() {
            candidate.view(id)?;
            candidate.title(id)?;
            candidate.style(&candidate.theme(id)?)?;
        }
        let _ = candidate.subscription()?;
        candidate
            .pending
            .splice(0..0, std::mem::take(&mut self.pending));
        widgets::forget_generation(self.generation);
        *self = candidate;
        Ok(())
    }

    fn check_reload(&mut self) -> mlua::Result<Change> {
        let change = self.reload_modules()?;
        // The first view runs before layout, so `iced.bounds` is empty until the
        // first draw. Rebuild once when recorded bounds appear or change.
        let change = if widgets::measure::take_changed() {
            Change::All
        } else {
            change
        };
        let source = match std::fs::read_to_string(&self.path) {
            Ok(source) => source,
            Err(error) => {
                if self.source.take().is_some() {
                    return Err(mlua::Error::runtime(format!(
                        "cannot read {}: {error}",
                        self.path.display()
                    )));
                }
                return Ok(change);
            }
        };
        if self.source.as_deref() == Some(&source) {
            return Ok(change);
        }
        self.source = Some(source.clone());
        self.reload(&source)?;
        log::info!(target: "reload.shell", "reloaded {}", self.path.display());
        Ok(Change::All)
    }

    // Re-renders every window that shows a changed module. A broken module restores the
    // previous one so the current UI keeps running, matching shell.lua reload behaviour.
    fn reload_modules(&mut self) -> mlua::Result<Change> {
        let snapshot = self.modules.borrow().snapshot(&self.lua)?;
        let changed = {
            let mut modules = self.modules.borrow_mut();
            modules.poll(&self.lua)?
        };
        if changed.is_empty() {
            return Ok(Change::None);
        }
        let previous_layers = self.layers.clone();
        let previous_toplevels = self.toplevels.clone();
        let previous_popups = self.popups.clone();
        let previous_mounts = self.mounts.borrow().clone();
        let previous_owners = self.windows_of.borrow().clone();
        let previous_windows = self.windows.borrow().clone();
        let previous_sizes = self.sizes.borrow().clone();
        let pending_count = self.pending.len();
        let handlers: Table = self.lua.named_registry_value("riced.ipc_handlers")?;
        let previous_handlers = copy_table(&self.lua, &handlers)?;
        let widgets: Table = self.lua.globals().get("iced")?;
        let previous_widgets = copy_table(&self.lua, &widgets)?;
        let mut affected = Vec::new();
        {
            let windows_of = self.windows_of.borrow();
            for module in &changed {
                for &id in windows_of.get(module).into_iter().flatten() {
                    if !affected.contains(&id) {
                        affected.push(id);
                    }
                }
            }
        }
        let restore_animations: Function =
            self.animations()?.get::<Function>("checkpoint")?.call(())?;
        let result = (|| {
            self.modules
                .borrow_mut()
                .refresh(&self.lua, &changed, &snapshot)?;
            self.reconcile_layers()?;
            // Ordinary require modules can provide layers without an iced.component mount.
            if affected.is_empty()
                || self.layers.iter().any(|layer| {
                    !previous_layers
                        .iter()
                        .any(|old| old.window == layer.window && old.settings == layer.settings)
                })
            {
                affected = self.windows.borrow().clone();
                for layer in &self.layers {
                    if !affected.contains(&layer.window) {
                        affected.push(layer.window);
                    }
                }
            }
            for layer in &self.layers {
                self.view(layer.window)?;
            }
            for toplevel in &self.toplevels {
                self.view(toplevel.window)?;
                if !affected.contains(&toplevel.window) {
                    affected.push(toplevel.window);
                }
            }
            for popup in &self.popups {
                self.view(popup.window)?;
                if !affected.contains(&popup.window) {
                    affected.push(popup.window);
                }
            }
            for &id in &affected {
                self.view(id)?;
            }
            Ok(())
        })();
        if let Err(error) = result {
            self.modules.borrow_mut().restore(&self.lua, snapshot)?;
            restore_animations.call::<()>(())?;
            handlers.clear()?;
            for pair in previous_handlers.pairs::<Value, Value>() {
                let (name, handler) = pair?;
                handlers.raw_set(name, handler)?;
            }
            widgets.clear()?;
            for pair in previous_widgets.clone().pairs::<Value, Value>() {
                let (name, widget) = pair?;
                widgets.raw_set(name, widget)?;
            }
            self.layers = previous_layers;
            self.toplevels = previous_toplevels;
            self.popups = previous_popups;
            *self.mounts.borrow_mut() = previous_mounts;
            *self.windows_of.borrow_mut() = previous_owners;
            *self.windows.borrow_mut() = previous_windows;
            *self.sizes.borrow_mut() = previous_sizes;
            self.pending.truncate(pending_count);
            return Err(error);
        }
        if !affected.is_empty() {
            log::info!(target: "reload.modules", "reloaded modules {}", changed.join(", "));
        }
        Ok(Change::Windows(affected))
    }

    fn title(&self, id: window::Id) -> mlua::Result<Option<String>> {
        if let Some(toplevel) = self.toplevels.iter().find(|window| window.window == id) {
            return Ok(Some(toplevel.title.clone()));
        }
        match self.call("title", self.window(id)?)? {
            Value::Nil => Ok(None),
            Value::String(s) => Ok(Some(s.to_str()?.to_owned())),
            _ => Err(mlua::Error::runtime("title must return a string or nil")),
        }
    }

    pub fn run(mut self) -> iced_exwlshell::Result {
        checked(self.prepare_layers());
        let (settings, mut wl_settings) = checked(self.settings());
        if !self.layers.is_empty() || !self.toplevels.is_empty() {
            wl_settings.layer_settings.start_mode = StartMode::Background;
        }
        let namespace = self.namespace.clone();
        let state = Rc::new(RefCell::new(self));
        let redraw = Rc::clone(&state);
        iced_exwlshell::daemon(
            move || (Rc::clone(&state), state.borrow().layer_tasks()),
            move || namespace.clone(),
            |app: &mut Rc<RefCell<App>>, message| {
                let mut app = app.borrow_mut();
                if matches!(message, Message::CheckReload) {
                    let reload = match app.check_reload() {
                        Ok(change) => app.reload_tasks(change),
                        Err(error) => {
                            log::warn!(target: "reload.rejected", "reload rejected; keeping current app: {error}");
                            Task::none()
                        }
                    };
                    let surfaces = if app.surfaces_dirty.replace(false) {
                        match app.reconcile_layers() {
                            Ok(()) => app.surface_tasks(),
                            Err(error) => {
                                log::warn!(target: "surface.rejected", "surface change rejected: {error}");
                                Task::none()
                            }
                        }
                    } else {
                        Task::none()
                    };
                    let ipc = app.poll_ipc().unwrap_or_else(|error| {
                        log::error!(target: "ipc.poll", "IPC poll failed: {error}");
                        Task::none()
                    });
                    return reload.chain(surfaces).chain(ipc);
                }
                if matches!(message, Message::PollPlugins) {
                    let (changed, tasks) = checked_fallback(
                        app.poll_plugins(),
                        (false, Task::none()),
                        "poll_plugins",
                    );
                    return if changed {
                        tasks.chain(Task::done(Message::Reloaded))
                    } else {
                        tasks
                    };
                }
                checked_fallback(app.update(message), Task::none(), "update")
            },
            App::render,
        )
        .title(|app: &Rc<RefCell<App>>, id| checked_fallback(app.borrow().title(id), None, "title"))
        .subscription(|app| {
            Subscription::batch([
                checked_fallback(app.borrow().subscription(), Subscription::none(), "subscription"),
                iced::time::every(Duration::from_millis(250)).map(|_| Message::CheckReload),
                iced::time::every(Duration::from_millis(100)).map(|_| Message::PollPlugins),
            ])
        })
        .theme(|app: &Rc<RefCell<App>>, id| checked_fallback(app.borrow().theme(id), Theme::Dark, "theme"))
        .style(|app, theme| checked_fallback(app.borrow().style(theme), theme::Style { background_color: iced::Color::TRANSPARENT, text_color: iced::Color::TRANSPARENT }, "style"))
        .redraw_scope(move |message| checked_fallback(redraw.borrow().redraw(message), Scope::None, "redraw_scope"))
        .settings(settings)
        .wl_settings(wl_settings)
        .run()
    }

    fn render(app: &Rc<RefCell<App>>, id: window::Id) -> Element<'_, Message> {
        checked_fallback(
            app.borrow().view(id),
            iced::widget::Space::new().into(),
            "view",
        )
    }
}

fn copy_table(lua: &Lua, source: &Table) -> mlua::Result<Table> {
    let copy = lua.create_table()?;
    for pair in source.clone().pairs::<Value, Value>() {
        let (key, value) = pair?;
        copy.raw_set(key, value)?;
    }
    Ok(copy)
}

pub(crate) fn keys(table: &Table, allowed: &[&str]) -> mlua::Result<()> {
    for pair in table.clone().pairs::<String, Value>() {
        let (key, _) = pair?;
        if !allowed.contains(&key.as_str()) {
            return Err(mlua::Error::runtime(format!("unsupported setting: {key}")));
        }
    }
    Ok(())
}

fn widget_id(id: &str) -> iced::widget::Id {
    iced::widget::Id::from(id.to_owned())
}

fn key_name(key: iced::keyboard::Key) -> String {
    match key {
        iced::keyboard::Key::Named(named) => format!("{named:?}"),
        iced::keyboard::Key::Character(character) => character.to_string(),
        iced::keyboard::Key::Unidentified => "Unidentified".to_owned(),
    }
}

fn button_name(button: iced::mouse::Button) -> &'static str {
    match button {
        iced::mouse::Button::Left => "left",
        iced::mouse::Button::Right => "right",
        iced::mouse::Button::Middle => "middle",
        iced::mouse::Button::Back => "back",
        iced::mouse::Button::Forward => "forward",
        iced::mouse::Button::Other(_) => "other",
    }
}

fn frame_message((generation, _): (u64, iced::time::Instant)) -> Message {
    Message::Frame(generation)
}

fn keyboard_event(event: iced::Event) -> Option<KeyEvent> {
    let iced::Event::Keyboard(event) = event else {
        return None;
    };
    match event {
        iced::keyboard::Event::KeyPressed {
            key,
            text,
            modifiers,
            ..
        } => Some(KeyEvent {
            pressed: true,
            key: key_name(key),
            text: text.map(|text| text.to_string()),
            modifiers: modifiers.bits(),
        }),
        iced::keyboard::Event::KeyReleased { key, modifiers, .. } => Some(KeyEvent {
            pressed: false,
            key: key_name(key),
            text: None,
            modifiers: modifiers.bits(),
        }),
        iced::keyboard::Event::ModifiersChanged(_) => None,
    }
}

fn key_message(((generation, pressed), event): ((u64, Option<bool>), KeyEvent)) -> Option<Message> {
    if pressed.is_some_and(|pressed| pressed != event.pressed) {
        return None;
    }
    Some(Message::Key(generation, event))
}

fn mouse_event(event: iced::Event) -> Option<MouseEvent> {
    let iced::Event::Mouse(event) = event else {
        return None;
    };
    Some(match event {
        iced::mouse::Event::CursorEntered => MouseEvent::Enter,
        iced::mouse::Event::CursorLeft => MouseEvent::Leave,
        iced::mouse::Event::CursorMoved { position } => MouseEvent::Move(position.x, position.y),
        iced::mouse::Event::ButtonPressed(button) => {
            MouseEvent::Press(button_name(button).to_owned())
        }
        iced::mouse::Event::ButtonReleased(button) => {
            MouseEvent::Release(button_name(button).to_owned())
        }
        iced::mouse::Event::WheelScrolled { delta } => {
            let (x, y) = match delta {
                iced::mouse::ScrollDelta::Lines { x, y } => (x, y),
                iced::mouse::ScrollDelta::Pixels { x, y } => (x, y),
            };
            MouseEvent::Scroll { x, y }
        }
    })
}

fn mouse_message((generation, event): (u64, MouseEvent)) -> Message {
    Message::Mouse(generation, event)
}

#[cfg(test)]
mod tests {
    use super::*;

    const RELOAD_SHELL: &str = r#"
        local app = { count = 0 }
        function app:update(event) self.count = self.count + 1 end
        function app:view(props) self.window = props.window; return iced.text('before') end
        function app:title(window) return 'before' end
        function app:redraw_scope(event) return 'none' end
        return riced.daemon(app, 'test', app.update, app.view)
            :title(app.title):redraw_scope(app.redraw_scope):run()
    "#;

    struct TestShell(PathBuf);

    impl TestShell {
        fn new() -> Self {
            static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "riced-reload-{}-{}.lua",
                std::process::id(),
                NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            ));
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
                .unwrap();
            std::io::Write::write_all(&mut file, RELOAD_SHELL.as_bytes()).unwrap();
            Self(path)
        }
    }

    impl Drop for TestShell {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }

    #[test]
    fn saved_shell_reloads_state_callbacks_and_redraw_policy() {
        let file = TestShell::new();
        let mut app = load(RELOAD_SHELL, &file.0).unwrap();
        let id = window::Id::unique();
        app.view(id).unwrap();
        let _ = app.update(Message::Tick).unwrap();
        assert!(matches!(app.check_reload().unwrap(), Change::None));
        assert_eq!(app.state.get::<u32>("count").unwrap(), 1);

        std::fs::write(
            &file.0,
            RELOAD_SHELL
                .replace("before", "after")
                .replace("return 'none'", "return self.window"),
        )
        .unwrap();
        assert!(matches!(
            app.check_reload().unwrap(),
            Change::All | Change::Windows(_)
        ));
        assert_eq!(app.state.get::<u32>("count").unwrap(), 0);
        assert_eq!(app.title(id).unwrap().as_deref(), Some("after"));
        assert_eq!(app.redraw(&Message::Tick).unwrap(), Scope::Window(id));
        assert_eq!(app.redraw(&Message::Reloaded).unwrap(), Scope::All);
        assert_eq!(app.redraw(&Message::CheckReload).unwrap(), Scope::None);
    }

    #[test]
    fn bounds_changes_do_not_skip_saved_shells() {
        let file = TestShell::new();
        let source = RELOAD_SHELL.replace(
            "iced.text('before')",
            "iced.measure(iced.text('before'), {id = 'reload-target'})",
        );
        std::fs::write(&file.0, &source).unwrap();
        let mut app = load(&source, &file.0).unwrap();
        let id = window::Id::unique();
        let mut tree = iced::advanced::widget::Tree::empty();
        for title in ["after", "after again"] {
            animation_frame(&mut app, id, &mut tree, std::time::Instant::now());
            std::fs::write(&file.0, source.replace("before", title)).unwrap();
            assert!(matches!(app.check_reload().unwrap(), Change::All));
            assert_eq!(app.title(id).unwrap().as_deref(), Some(title));
        }
        app.forget_window(id).unwrap();
    }

    #[test]
    fn invalid_saves_keep_current_app_and_recover_on_next_save() {
        let file = TestShell::new();
        let mut app = load(RELOAD_SHELL, &file.0).unwrap();
        let id = window::Id::unique();
        app.view(id).unwrap();
        let _ = app.update(Message::Tick).unwrap();

        for source in [
            "not valid Lua".to_owned(),
            RELOAD_SHELL.replace(
                "iced.text('before')",
                "iced.text('before', {color='invalid'})",
            ),
        ] {
            std::fs::write(&file.0, source).unwrap();
            assert!(app.check_reload().is_err());
            assert!(
                matches!(app.check_reload().unwrap(), Change::None),
                "do not retry/log the same failed save"
            );
            assert_eq!(app.state.get::<u32>("count").unwrap(), 1);
            app.view(id).unwrap();
        }

        std::fs::write(&file.0, RELOAD_SHELL.replace("before", "fixed")).unwrap();
        assert!(matches!(
            app.check_reload().unwrap(),
            Change::All | Change::Windows(_)
        ));
        assert_eq!(app.title(id).unwrap().as_deref(), Some("fixed"));
    }

    #[test]
    fn ipc_handlers_receive_state_and_arguments() {
        let mut app = load(
            r#"
            local app = { hits = 0 }
            function app:update() end
            function app:view() return iced.text('ipc') end
            riced.ipc({
                add = function(self, args)
                    log("called")
                    self.hits = self.hits + 1
                    self.args = args
                end,
                zero = function(self, args) self.zero = #args end,
            })
            return riced.daemon(app, 'test', app.update, app.view):run()
            "#,
            Path::new("shell.lua"),
        )
        .unwrap();

        let _ = app
            .run_ipc("add", &["--anchor".into(), "top bar".into(), "".into()])
            .unwrap();
        assert_eq!(app.state.get::<u32>("hits").unwrap(), 1);
        let args: Table = app.state.get("args").unwrap();
        assert_eq!(args.get::<String>(1).unwrap(), "--anchor");
        assert_eq!(args.get::<String>(2).unwrap(), "top bar");
        assert_eq!(args.get::<String>(3).unwrap(), "");

        let _ = app.run_ipc("zero", &[]).unwrap();
        assert_eq!(app.state.get::<u32>("zero").unwrap(), 0);

        let _ = app.run_ipc("unknown", &[]).unwrap();
        assert_eq!(app.state.get::<u32>("hits").unwrap(), 1);
    }

    #[test]
    fn ipc_reregistration_replaces_the_handler() {
        let mut app = load(
            r#"
            local app = { hits = 0 }
            function app:update() end
            function app:view() return iced.text('ipc') end
            riced.ipc({ add = function(self) self.hits = self.hits + 1 end })
            riced.ipc({ add = function(self) self.hits = self.hits + 10 end })
            return riced.daemon(app, 'test', app.update, app.view):run()
            "#,
            Path::new("shell.lua"),
        )
        .unwrap();

        let _ = app.run_ipc("add", &[]).unwrap();
        assert_eq!(app.state.get::<u32>("hits").unwrap(), 10);
    }

    #[test]
    fn ipc_rejects_an_empty_command_name() {
        let error = match load(
            r#"
            local app = { }
            function app:update() end
            function app:view() return iced.text('ipc') end
            riced.ipc({ [""] = function() end })
            return riced.daemon(app, 'test', app.update, app.view):run()
            "#,
            Path::new("shell.lua"),
        ) {
            Ok(_) => panic!("empty IPC command was accepted"),
            Err(error) => error,
        };

        assert!(error.to_string().contains("must not be empty"));
    }

    #[test]
    fn failed_module_reload_restores_ipc_handlers() {
        let profile = Profile::new(&[
            (
                "shell.lua",
                r#"
                local state = require('state')
                local app = { hits = 0 }
                function app:update() end
                function app:view() return iced.text('ipc') end
                return riced.daemon(app, 'test', app.update, app.view):run()
                "#,
            ),
            (
                "state.lua",
                "riced.ipc({ add = function(self) self.hits = self.hits + 1 end })\nreturn {}",
            ),
        ]);
        let mut app = load(
            r#"
            local state = require('state')
            local app = { hits = 0 }
            function app:update() end
            function app:view() return iced.text('ipc') end
            return riced.daemon(app, 'test', app.update, app.view):run()
            "#,
            &profile.shell(),
        )
        .unwrap();
        let _ = app.run_ipc("add", &[]).unwrap();
        assert_eq!(app.state.get::<u32>("hits").unwrap(), 1);

        profile.write(
            "state.lua",
            "riced.ipc({ add = function(self) self.hits = self.hits + 10 end })\nerror('broken')",
        );
        assert!(app.check_reload().is_err());

        let _ = app.run_ipc("add", &[]).unwrap();
        assert_eq!(app.state.get::<u32>("hits").unwrap(), 2);
    }

    #[test]
    fn atomic_save_and_temporary_missing_file_recover() {
        let file = TestShell::new();
        let mut app = load(RELOAD_SHELL, &file.0).unwrap();
        std::fs::remove_file(&file.0).unwrap();
        assert!(app.check_reload().is_err());
        assert!(matches!(app.check_reload().unwrap(), Change::None));

        let replacement = TestShell::new();
        std::fs::write(&replacement.0, RELOAD_SHELL.replace("before", "replaced")).unwrap();
        std::fs::rename(&replacement.0, &file.0).unwrap();
        assert!(matches!(
            app.check_reload().unwrap(),
            Change::All | Change::Windows(_)
        ));
        assert_eq!(
            app.title(window::Id::unique()).unwrap().as_deref(),
            Some("replaced")
        );
    }

    #[test]
    fn reload_rejects_startup_changes() {
        let mut app = load(RELOAD_SHELL, Path::new("shell.lua")).unwrap();
        for source in [
            RELOAD_SHELL.replace("'test'", "'other'"),
            RELOAD_SHELL.replace(":run()", ":settings({id='other'}):run()"),
            RELOAD_SHELL.replace(
                ":run()",
                ":settings({layer_settings={start_mode='background'}}):run()",
            ),
        ] {
            assert!(
                app.reload(&source)
                    .unwrap_err()
                    .to_string()
                    .contains("restart riced")
            );
            assert_eq!(app.generation, 0);
        }
    }

    #[test]
    fn queued_messages_from_previous_lua_vm_are_ignored() {
        let mut app = load(RELOAD_SHELL, Path::new("shell.lua")).unwrap();
        let event = Message::Lua(
            app.generation,
            None,
            Value::Table(app.lua.create_table().unwrap()),
        );
        app.reload(&RELOAD_SHELL.replace("before", "after"))
            .unwrap();
        assert_eq!(app.redraw(&event).unwrap(), Scope::None);
        let _ = app.update(event).unwrap();
        assert_eq!(app.state.get::<u32>("count").unwrap(), 0);
        let _ = app
            .update(Message::Lua(app.generation, None, Value::Nil))
            .unwrap();
        assert_eq!(app.state.get::<u32>("count").unwrap(), 1);
    }

    #[test]
    fn custom_widget_definition_renders() {
        let app = load(
            r#"
            iced.define("greeting", function()
                local widget = {}
                function widget:view(props)
                    return iced.text(props.text)
                end
                return widget
            end)
            return riced.daemon({}, 'test', function() end, function()
                return iced.greeting({ text = "hello" })
            end):run()
        "#,
            Path::new("shell.lua"),
        )
        .unwrap();
        app.view(window::Id::unique()).unwrap();
    }

    #[test]
    fn widgets_load_from_any_path_without_a_require() {
        let profile = Profile::new(&[
            ("shell.lua", ""),
            (
                "widgets/deep/greeting.lua",
                r#"iced.define("greeting", function()
                    local widget = {}
                    function widget:view() seen = "loaded"; return iced.text("hi") end
                    return widget
                end)"#,
            ),
        ]);
        let app = load(
            r#"
            return riced.daemon({}, 'test', function() end, function()
                return iced.greeting()
            end):run()
        "#,
            &profile.shell(),
        )
        .unwrap();
        app.view(window::Id::unique()).unwrap();
        assert_eq!(app.lua.globals().get::<String>("seen").unwrap(), "loaded");
    }

    #[test]
    fn named_transitions_lazy_load_reload_and_restore_after_rejection() {
        let preset = r#"iced.define('fade', function()
            return {duration = 180, easing = 'out', enter = {from = {scale = 0}}, change = {props = {'width'}}}
        end)"#;
        let profile = Profile::new(&[("shell.lua", ""), ("presets/fade.lua", preset)]);
        let mut app = load(
            r#"
            return riced.daemon({}, 'test', function() end, function()
                local transition = iced.transition('fade')
                seen = transition.enter.duration
                return iced.container(iced.text('hello'), {id = 'card', width = 80, transition = transition})
            end):run()
            "#,
            &profile.shell(),
        )
        .unwrap();
        let id = window::Id::unique();
        app.view(id).unwrap();
        assert_eq!(app.lua.globals().get::<u32>("seen").unwrap(), 180);

        profile.write("presets/fade.lua", &preset.replace("180", "90"));
        app.reload_modules().unwrap();
        app.view(id).unwrap();
        assert_eq!(app.lua.globals().get::<u32>("seen").unwrap(), 90);

        for broken in [
            preset.replace("180", "0"),
            preset.replace("'out'", "'invalid'"),
            preset.replace("{'width'}", "{'unsupported'}"),
            "iced.define('fade', function() return {view = function() return iced.text('wrong kind') end} end)".into(),
        ] {
            profile.write("presets/fade.lua", &broken);
            assert!(app.reload_modules().is_err());
            app.view(id).unwrap();
            assert_eq!(app.lua.globals().get::<u32>("seen").unwrap(), 90);
        }
        profile.write("presets/fade.lua", &preset.replace("180", "60"));
        app.reload_modules().unwrap();
        app.view(id).unwrap();
        assert_eq!(app.lua.globals().get::<u32>("seen").unwrap(), 60);
    }

    #[test]
    fn extension_widget_loads_from_a_symlink_and_reloads() {
        let source = include_str!("../extensions/lucide/lucide.lua");
        let extension = Profile::new(&[("lucide.lua", source)]);
        let profile = Profile::new(&[("shell.lua", ""), ("extensions/.keep", "")]);
        std::os::unix::fs::symlink(&extension.0, profile.0.join("extensions/lucide")).unwrap();
        let mut app = load(
            r#"
            return riced.daemon({}, 'test', function() end, function()
                node = iced.lucide({ name = 'activity', size = 20, color = '#ffffff' })
                return node
            end):run()
            "#,
            &profile.shell(),
        )
        .unwrap();
        let id = window::Id::unique();
        app.view(id).unwrap();
        app.lua
            .load("assert(node.kind == 'text' and node.content == '' and node.name == nil)")
            .exec()
            .unwrap();

        extension.write(
            "lucide.lua",
            &source.replace("iced.text(\"\", props)", "iced.text(\"loading\", props)"),
        );
        assert!(matches!(app.reload_modules().unwrap(), Change::Windows(_)));
        app.view(id).unwrap();
        app.lua
            .load("assert(node.content == 'loading')")
            .exec()
            .unwrap();

        extension.write(
            "lucide.lua",
            &source.replace("return lucide", "error('broken extension')\nreturn lucide"),
        );
        assert!(app.reload_modules().is_err());
        app.view(id).unwrap();
        app.lua
            .load("assert(node.content == 'loading')")
            .exec()
            .unwrap();

        extension.write("lucide.lua", source);
        app.reload_modules().unwrap();
        app.lua
            .load(
                r#"
                local ext = riced.plugins('lucide')
                ext.alive = true
                ext.resolve = function(name, callback)
                    assert(name == 'activity')
                    callback({ char = '\u{e038}' })
                end
            "#,
            )
            .exec()
            .unwrap();
        app.view(id).unwrap();
        app.view(id).unwrap();
        app.lua
            .load("assert(node.kind == 'icon' and node.name == nil and node.size == 20)")
            .exec()
            .unwrap();
    }

    #[test]
    fn openmeteo_widget_configures_on_connect_and_marks_stale_weather() {
        let profile = Profile::new(&[
            ("shell.lua", ""),
            (
                "extensions/openmeteo/openmeteo.lua",
                include_str!("../extensions/openmeteo/openmeteo.lua"),
            ),
        ]);
        let app = load(
            r#"
            weather = require('extensions.openmeteo.openmeteo')
            weather.configure(52.52, 13.41)
            return riced.daemon({}, 'test', function() end, function()
                node = iced.openmeteo({ size = 18 })
                return node
            end):run()
        "#,
            &profile.shell(),
        )
        .unwrap();
        let id = window::Id::unique();
        app.view(id).unwrap();
        app.lua
            .load(
                r#"
            assert(node.content == 'Weather unavailable')
            ext = weather.plugin
            ext.alive = true
            calls = 0
            ext.configure = function(latitude, longitude)
                assert(latitude == 52.52 and longitude == 13.41)
                calls = calls + 1
            end
        "#,
            )
            .exec()
            .unwrap();
        app.view(id).unwrap();
        app.view(id).unwrap();
        app.lua
            .load(
                r#"
            assert(calls == 1 and node.content == 'Loading weather…')
            ext.latitude, ext.longitude = 52.52, 13.41
            ext.ready = true
            ext.current = { temperature_2m = 12.5 }
        "#,
            )
            .exec()
            .unwrap();
        app.view(id).unwrap();
        app.lua
            .load("assert(node.content == '12.5°C' and node.size == 18); ext.error = 'offline'")
            .exec()
            .unwrap();
        app.view(id).unwrap();
        app.lua
            .load("assert(node.content == '12.5°C (stale)'); ext.error = nil; ext.alive = false")
            .exec()
            .unwrap();
        app.view(id).unwrap();
        app.lua
            .load(
                r#"
            assert(node.content == '12.5°C (stale)' and calls == 1)
            ext.alive = true
            ext.configure = function() calls = calls + 1 end
        "#,
            )
            .exec()
            .unwrap();
        app.view(id).unwrap();
        app.lua
            .load(
                r#"
            assert(calls == 2)
            weather.configure(0, 0)
            assert(not pcall(weather.configure, 91, 0))
            assert(not pcall(weather.configure, 0, 0/0))
        "#,
            )
            .exec()
            .unwrap();
        app.view(id).unwrap();
        app.lua
            .load("assert(node.content == 'Loading weather…' and calls == 3)")
            .exec()
            .unwrap();
    }

    #[test]
    fn extension_widgets_cannot_replace_builtin_widgets() {
        let profile = Profile::new(&[
            ("shell.lua", ""),
            (
                "extensions/demo/demo.lua",
                "iced.define('icon', function() return { view = function() return iced.text('bad') end } end)",
            ),
        ]);
        let error = match load("require('extensions.demo.demo')", &profile.shell()) {
            Ok(_) => panic!("built-in widget replacement must fail"),
            Err(error) => error.to_string(),
        };
        assert!(error.contains("widget already exists: icon"), "{error}");
    }

    #[test]
    fn duplicate_widget_names_are_rejected() {
        let profile = Profile::new(&[
            ("shell.lua", ""),
            (
                "left/greeting.lua",
                r#"iced.define("greeting", function()
                    local widget = {}
                    function widget:view() seen = "left"; return iced.text("hi") end
                    return widget
                end)"#,
            ),
            (
                "extensions/demo/greeting.lua",
                r#"iced.define("greeting", function()
                    local widget = {}
                    function widget:view() seen = "right"; return iced.text("hi") end
                    return widget
                end)"#,
            ),
        ]);
        let app = load(
            r#"
            return riced.daemon({}, 'test', function() end, function()
                return iced.greeting()
            end):run()
        "#,
            &profile.shell(),
        )
        .unwrap();
        let error = match app.view(window::Id::unique()) {
            Ok(_) => panic!("duplicate widget names must fail"),
            Err(error) => error.to_string(),
        };
        assert!(
            error.contains("multiple files define iced.greeting"),
            "{error}"
        );
    }

    #[test]
    fn widget_discovery_visits_symlinked_directories_once() {
        use std::os::unix::fs::symlink;

        let profile = Profile::new(&[("shell.lua", ""), ("widgets/greeting.lua", "")]);
        let external = Profile::new(&[("linked.lua", "")]);
        symlink("..", profile.0.join("widgets/loop")).unwrap();
        symlink("widgets", profile.0.join("alias")).unwrap();
        symlink(&external.0, profile.0.join("external")).unwrap();

        for name in ["greeting", "linked", "missing"] {
            let mut found = Vec::new();
            find_widget(
                &profile.0,
                &profile.shell(),
                name,
                &mut found,
                &mut HashSet::new(),
            );
            assert_eq!(
                found.len(),
                usize::from(name != "missing"),
                "{name}: {found:?}"
            );
        }
    }

    #[test]
    fn widget_redefinition_replaces_the_constructor() {
        let app = load(
            r#"
            iced.define("greeting", function()
                local widget = {}
                function widget:view() seen = "first"; return iced.text("one") end
                return widget
            end)
            iced.define("greeting", function()
                local widget = {}
                function widget:view() seen = "second"; return iced.text("two") end
                return widget
            end)
            return riced.daemon({}, 'test', function() end, function()
                return iced.greeting()
            end):run()
        "#,
            Path::new("shell.lua"),
        )
        .unwrap();
        app.view(window::Id::unique()).unwrap();
        assert_eq!(app.lua.globals().get::<String>("seen").unwrap(), "second");
    }

    #[test]
    fn rejected_widget_reload_keeps_the_previous_definition() {
        let profile = Profile::new(&[
            ("shell.lua", ""),
            (
                "widgets/greeting.lua",
                r#"iced.define("greeting", function()
                    local widget = {}
                    function widget:view() return iced.text("ok") end
                    return widget
                end)"#,
            ),
        ]);
        let mut app = load(
            r#"
            return riced.daemon({}, 'test', function() end, function()
                return iced.greeting()
            end):run()
        "#,
            &profile.shell(),
        )
        .unwrap();
        let id = window::Id::unique();
        app.view(id).unwrap();
        profile.write(
            "widgets/greeting.lua",
            r#"iced.define("greeting", function()
                local widget = {}
                function widget:view() return iced.text("bad", { nope = true }) end
                return widget
            end)"#,
        );
        assert!(app.reload_modules().is_err());
        app.view(id).unwrap();
    }

    #[test]
    fn example_loads_and_renders_without_compositor() {
        let mut app = load(
            include_str!("../example/shell.lua"),
            Path::new("example/shell.lua"),
        )
        .unwrap();
        app.prepare_layers().unwrap();
        app.view(app.layers[0].window).unwrap();
    }

    const LAYER_SHELL: &str = r#"
        local app = {count = 0}
        function app:update(event) self.count = self.count + 1 end
        function app:view(props)
            return {
                iced.layer(iced.text('background'), {layer='background', namespace='background'}),
                iced.layer(iced.button(tostring(self.count), {on_press='increment'}), {
                    size={width='fill', height=32}, anchor={'top','left','right'},
                    exclusive_zone=32, namespace='panel'
                }),
            }
        end
        return riced.daemon(app, 'test', app.update, app.view):run()
    "#;

    #[test]
    fn popup_surfaces_open_reposition_and_close() {
        let source = r#"
            open, px, py, pw, ph = true, 4, 30, 120, 80
            local app = {}
            function app:update(event)
                if type(event) == 'table' and event.type == 'window_closed' then
                    open = false
                end
            end
            function app:view()
                local surfaces = {
                    iced.layer(iced.text('bar'), {
                        id = 'bar',
                        size = { width = 200, height = 30 },
                        anchor = { 'top', 'left', 'right' },
                    }),
                }
                if open then
                    surfaces[#surfaces + 1] = iced.popup(iced.text('menu'), {
                        id = 'menu',
                        parent = 'bar',
                        width = pw,
                        height = ph,
                        x = px,
                        y = py,
                        on_closed = function() closed_callback = true end,
                    })
                end
                return surfaces
            end
            return riced.daemon(app, 'test', app.update, app.view):run()
        "#;
        let mut app = load(source, Path::new("shell.lua")).unwrap();
        app.reconcile_layers().unwrap();
        assert_eq!(app.layers.len(), 1);
        assert_eq!(app.popups.len(), 1);
        let popup = app.popups[0].clone();
        assert_eq!(popup.parent, "bar");
        assert!(
            app.pending
                .iter()
                .any(|action| matches!(&action.1, ExWlShellCustomAction::NewPopUp { id, .. } if *id == popup.window))
        );
        app.view(popup.window).unwrap();

        app.pending.clear();
        app.lua.globals().set("px", 8).unwrap();
        app.reconcile_layers().unwrap();
        assert_eq!(app.popups[0].window, popup.window);
        assert!(app.pending.iter().any(|action| matches!(
            &action.1,
            ExWlShellCustomAction::PopUpReposition { settings }
                if settings.placement == iced_exwlshell::reexport::PopupPlacement::Position(iced_exwlshell::reexport::Position::new(8, 30))
        )));

        app.pending.clear();
        app.lua.globals().set("open", false).unwrap();
        app.reconcile_layers().unwrap();
        assert!(app.popups.is_empty());
        assert!(app.pending.iter().any(|action| {
            matches!(&action.1, ExWlShellCustomAction::RemoveWindow)
                && action.0 == Some(popup.window)
        }));

        app.pending.clear();
        app.lua.globals().set("open", true).unwrap();
        app.reconcile_layers().unwrap();
        let reopened = app.popups[0].clone();
        assert_ne!(reopened.window, popup.window);
        app.view(reopened.window).unwrap();
        let _ = app.update(Message::WindowClosed(reopened.window)).unwrap();
        assert!(!app.lua.globals().get::<bool>("open").unwrap());
        assert!(app.lua.globals().get::<bool>("closed_callback").unwrap());
        assert!(app.popups.is_empty());
    }

    #[test]
    fn measured_widgets_publish_surface_local_bounds() {
        let source = r#"
            local app = {}
            function app:update() end
            function app:view()
                seen = iced.bounds("target")
                return iced.layer(
                    iced.container(
                        iced.measure(iced.button(iced.text("x"), { on_press = "noop" }), {
                            id = "target",
                        }),
                        { padding = 10 }
                    ),
                    { id = "top", size = { width = 200, height = 50 } }
                )
            end
            return riced.daemon(app, 'test', app.update, app.view):run()
        "#;
        let mut app = load(source, Path::new("shell.lua")).unwrap();
        app.prepare_layers().unwrap();
        let window = app.layers[0].window;
        let mut tree = iced::advanced::widget::Tree::empty();
        animation_frame(&mut app, window, &mut tree, std::time::Instant::now());
        app.view(window).unwrap();
        let seen: Table = app.lua.globals().get("seen").unwrap();
        assert_eq!(seen.get::<f32>("x").unwrap(), 10.0);
        assert_eq!(seen.get::<f32>("y").unwrap(), 10.0);
        assert!(seen.get::<f32>("width").unwrap() > 0.0);
        assert!(seen.get::<f32>("height").unwrap() > 0.0);
    }

    #[test]
    fn popups_anchor_to_widget_ids_without_a_measure_wrapper() {
        let source = r#"
            local app = {}
            function app:update() end
            function app:view()
                return iced.layer(
                    iced.row({
                        iced.button(iced.text("open"), { id = "open_button", on_press = "noop" }),
                        iced.popup(iced.text("menu"), {
                            id = "menu", width = 120, height = 80,
                            anchor = "open_button",
                        }),
                    }),
                    { id = "bar", size = { width = 200, height = 50 } }
                )
            end
            return riced.daemon(app, 'test', app.update, app.view):run()
        "#;
        let mut app = load(source, Path::new("shell.lua")).unwrap();
        app.prepare_layers().unwrap();
        assert!(app.popups.is_empty());
        let window = app.layers[0].window;
        let mut tree = iced::advanced::widget::Tree::empty();
        animation_frame(&mut app, window, &mut tree, std::time::Instant::now());

        app.reconcile_layers().unwrap();
        assert_eq!(app.popups.len(), 1);
        assert_eq!(app.popups[0].parent, "bar");
        assert!(app.popups[0].anchor.is_some());
    }

    #[test]
    fn popup_buttons_publish_lua_press_callbacks() {
        let source = r#"
            clicked = false
            local app = {}
            function app:update() end
            function app:view()
                return iced.layer(iced.row({
                    iced.text('bar'),
                    iced.popup(iced.container(
                        iced.button('click', {
                            width = 100,
                            height = 30,
                            on_press = function() clicked = true end,
                        }),
                        { width = 120, height = 60 }
                    ), {
                        id = 'menu',
                        width = 120,
                        height = 60,
                        anchor = { x = 0, y = 0, width = 10, height = 10 },
                    }),
                }), {
                    id = 'bar',
                    size = { width = 200, height = 30 },
                    anchor = { 'top', 'left', 'right' },
                })
            end
            return riced.daemon(app, 'test', app.update, app.view):run()
        "#;
        let mut app = load(source, Path::new("shell.lua")).unwrap();
        app.prepare_layers().unwrap();
        let popup = app.popups[0].clone();
        use iced::advanced::renderer::Headless;
        let mut tree = iced::advanced::widget::Tree::empty();
        let mut element = app.view(popup.window).unwrap();
        tree.diff(&element);
        let renderer = iced::futures::executor::block_on(iced::Renderer::new(
            iced::Font::DEFAULT,
            iced::Pixels(16.0),
            Some("tiny-skia"),
        ))
        .unwrap();
        let size = Size::new(120.0, 60.0);
        let layout = element.as_widget_mut().layout(
            &mut tree,
            &renderer,
            &iced::advanced::layout::Limits::new(Size::ZERO, size),
        );
        use iced::advanced::{Layout, Shell};
        let mut messages = Vec::new();
        for event in [
            iced::mouse::Event::ButtonPressed(iced::mouse::Button::Left),
            iced::mouse::Event::ButtonReleased(iced::mouse::Button::Left),
        ] {
            element.as_widget_mut().update(
                &mut tree,
                &iced::Event::Mouse(event),
                Layout::new(&layout),
                iced::mouse::Cursor::Available(iced::Point::new(10.0, 10.0)),
                &renderer,
                &mut iced::advanced::clipboard::Null,
                &mut Shell::new(&mut messages),
                &iced::Rectangle::with_size(size),
            );
        }
        for message in messages {
            let _ = app.update(message).unwrap();
        }
        assert!(app.lua.globals().get::<bool>("clicked").unwrap());
    }

    #[test]
    fn hidden_popups_are_not_opened() {
        let source = r#"
            visible = false
            return riced.daemon({}, 'test', function() end, function()
                return {
                    iced.layer(iced.text('bar'), { id = 'bar', size = { width = 200, height = 30 } }),
                    iced.popup(iced.text('menu'), {
                        id = 'menu', visible = visible, parent = 'bar',
                        width = 120, height = 80, x = 4, y = 30,
                    }),
                }
            end):run()
        "#;
        let mut app = load(source, Path::new("shell.lua")).unwrap();
        app.reconcile_layers().unwrap();
        assert_eq!(app.layers.len(), 1);
        assert!(app.popups.is_empty());

        app.lua.globals().set("visible", true).unwrap();
        app.reconcile_layers().unwrap();
        assert_eq!(app.popups.len(), 1);
        app.view(app.popups[0].window).unwrap();
    }

    #[test]
    fn popup_parent_must_be_a_layer() {
        let source = r#"
            return riced.daemon({}, 'test', function() end, function()
                return {
                    iced.window({ id = 'settings', content = iced.text('settings') }),
                    iced.popup(iced.text('menu'), {
                        id = 'menu', parent = 'settings', width = 100, height = 50,
                    }),
                }
            end):run()
        "#;
        let mut app = load(source, Path::new("shell.lua")).unwrap();
        let error = app.prepare_layers().unwrap_err().to_string();
        assert!(error.contains("parent \"settings\" must be an iced.layer"));
    }

    #[test]
    fn layers_create_distinct_surfaces_and_render_after_updates() {
        let mut app = load(LAYER_SHELL, Path::new("shell.lua")).unwrap();
        app.prepare_layers().unwrap();
        assert_eq!(app.layers.len(), 2);
        assert_ne!(app.layers[0].window, app.layers[1].window);
        assert_eq!(
            app.layers[0].settings.namespace.as_deref(),
            Some("background")
        );
        assert_eq!(app.layers[1].settings.exclusive_zone, Some(32));
        for layer in &app.layers {
            app.view(layer.window).unwrap();
        }
        let _ = app.update(Message::Tick).unwrap();
        app.view(app.layers[1].window).unwrap();
        assert_eq!(app.state.get::<u32>("count").unwrap(), 1);
        let id = app.layers[0].window;
        let action = ExWlShellCustomActionWithId::new(
            None,
            ExWlShellCustomAction::NewLayerShell {
                id,
                settings: app.layers[0].settings.clone(),
            },
        );
        let converted = ExWlShellCustomActionWithId::try_from(Message::Surface(action)).unwrap();
        assert!(
            matches!(converted.1, ExWlShellCustomAction::NewLayerShell {id: actual, ..} if actual == id)
        );
    }

    #[test]
    fn windows_open_render_and_close() {
        let source = r#"
            local app = { open = true }
            function app:update(event)
                if type(event) == 'table' and event.type == 'window_closed' then
                    self.open = false
                end
            end
            function app:view()
                if not self.open then return iced.text('closed') end
                return iced.window({
                    id = 'settings', title = 'Settings', width = 640, height = 480,
                    content = iced.text('settings'),
                })
            end
            return riced.daemon(app, 'test', app.update, app.view):run()
        "#;
        let mut app = load(source, Path::new("shell.lua")).unwrap();
        app.prepare_layers().unwrap();
        assert_eq!(app.toplevels.len(), 1);
        assert_eq!(app.toplevels[0].id, "settings");
        assert_eq!(app.toplevels[0].title, "Settings");
        assert_eq!(app.toplevels[0].size.to_set(), (640, 480));
        let id = app.toplevels[0].window;
        app.view(id).unwrap();
        assert_eq!(app.title(id).unwrap().as_deref(), Some("Settings"));

        app.reload(&source.replace("'Settings'", "'Preferences'"))
            .unwrap();
        assert_eq!(app.toplevels[0].window, id);
        assert_eq!(app.title(id).unwrap().as_deref(), Some("Preferences"));

        let _ = app.update(Message::WindowClosed(id)).unwrap();
        assert!(app.toplevels.is_empty());
        assert!(!app.state.get::<bool>("open").unwrap());
        assert_eq!(app.redraw(&Message::WindowClosed(id)).unwrap(), Scope::All);
    }

    #[test]
    fn layer_reload_preserves_windows_and_rejects_invalid_content() {
        let mut app = load(LAYER_SHELL, Path::new("shell.lua")).unwrap();
        app.prepare_layers().unwrap();
        let ids = app
            .layers
            .iter()
            .map(|layer| (layer.id.clone(), layer.window))
            .collect::<Vec<_>>();
        app.reload(&LAYER_SHELL.replace("iced.text('background')", "iced.text('changed')"))
            .unwrap();
        assert_eq!(
            app.layers
                .iter()
                .map(|layer| (layer.id.clone(), layer.window))
                .collect::<Vec<_>>(),
            ids
        );
        assert_eq!(app.generation, 1);
        assert!(
            app.reload(&LAYER_SHELL.replace(
                "iced.text('background')",
                "iced.text('background', {color='invalid'})"
            ))
            .is_err()
        );
        assert_eq!(app.generation, 1);
        app.reload(&LAYER_SHELL.replace("layer='background'", "layer='overlay'"))
            .unwrap();
        assert_eq!(app.layers[0].window, ids[0].1);
        assert!(
            app.pending
                .iter()
                .any(|action| matches!(action.1, ExWlShellCustomAction::LayerChange(_)))
        );
    }

    #[test]
    fn rejects_layers_nested_in_widgets_and_mixed_root_lists() {
        for view in [
            "iced.column({iced.layer(iced.text('x'))})",
            "{iced.layer(iced.text('x')), iced.text('y')}",
        ] {
            let source = format!(
                "return riced.daemon({{}}, 'test', function() end, function() return {view} end):run()"
            );
            let mut app = load(&source, Path::new("shell.lua")).unwrap();
            assert!(
                app.prepare_layers()
                    .and_then(|_| app.view(window::Id::unique()).map(|_| ()))
                    .is_err()
            );
        }
    }

    const COMPONENT_SHELL: &str = r#"
        local app = { count = 0 }
        function app:update(event) self.count = self.count + 1 end
        function app:view(props)
            return iced.column({
                iced.component('left', 'left', { count = self.count }),
                iced.component('right', 'right', { count = self.count }),
            })
        end
        return riced.daemon(app, 'test', app.update, app.view):run()
    "#;

    struct Profile(PathBuf);

    impl Profile {
        fn new(files: &[(&str, &str)]) -> Self {
            static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let root = std::env::temp_dir().join(format!(
                "riced-runtime-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            ));
            let profile = Self(root);
            for (name, source) in files {
                profile.write(name, source);
            }
            profile
        }

        fn write(&self, name: &str, source: &str) {
            let path = self.0.join(name);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, source).unwrap();
        }

        fn shell(&self) -> PathBuf {
            self.0.join("shell.lua")
        }
    }

    impl Drop for Profile {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn component_profile(left: &str) -> Profile {
        Profile::new(&[
            ("shell.lua", COMPONENT_SHELL),
            ("left.lua", left),
            (
                "right.lua",
                "local m = {}
function m:view(props) seen_right = 'right' return iced.text('right') end
return m",
            ),
        ])
    }

    // Component modules keep their version in a module local so tests can tell which
    // source a render came from without inspecting iced elements.
    fn versioned(version: &str) -> String {
        format!(
            "local version = '{version}'\nlocal m = {{}}\nfunction m:view(props) seen_left = version .. ':' .. props.count return iced.text('left') end\nreturn m"
        )
    }

    fn seen(app: &App, name: &str) -> String {
        app.lua.globals().get::<String>(name).unwrap()
    }

    #[test]
    fn components_render_props_and_are_mounted() {
        let profile = component_profile(&versioned("v1"));
        let app = load(COMPONENT_SHELL, &profile.shell()).unwrap();
        let id = window::Id::unique();
        app.view(id).unwrap();
        assert_eq!(seen(&app, "seen_left"), "v1:0");
        assert_eq!(seen(&app, "seen_right"), "right");
        assert_eq!(app.mounts.borrow()[&(id, "left".to_owned())].revision, 0);
        assert_eq!(app.windows_of.borrow()["left"], [id]);
    }

    fn animation_frame(
        app: &mut App,
        window: window::Id,
        tree: &mut iced::advanced::widget::Tree,
        now: std::time::Instant,
    ) -> usize {
        use iced::advanced::{Layout, Renderer as _, renderer::Headless};
        let mut renderer = iced::futures::executor::block_on(iced::Renderer::new(
            iced::Font::DEFAULT,
            iced::Pixels(16.0),
            Some("tiny-skia"),
        ))
        .unwrap();
        let mut element = app.view(window).unwrap();
        tree.diff(&element);
        let size = Size::new(640.0, 480.0);
        let layout = element.as_widget_mut().layout(
            tree,
            &renderer,
            &iced::advanced::layout::Limits::new(Size::ZERO, size),
        );
        let mut messages = Vec::new();
        element.as_widget_mut().update(
            tree,
            &iced::Event::Window(window::Event::RedrawRequested(now)),
            Layout::new(&layout),
            iced::mouse::Cursor::Unavailable,
            &renderer,
            &mut iced::advanced::clipboard::Null,
            &mut iced::advanced::Shell::new(&mut messages),
            &iced::Rectangle::with_size(size),
        );
        renderer.reset(iced::Rectangle::with_size(size));
        element.as_widget().draw(
            tree,
            &mut renderer,
            &Theme::Dark,
            &iced::advanced::renderer::Style {
                text_color: iced::Color::WHITE,
            },
            Layout::new(&layout),
            iced::mouse::Cursor::Unavailable,
            &iced::Rectangle::with_size(size),
        );
        let pixels = renderer
            .screenshot(Size::new(640, 480), 1.0, iced::Color::TRANSPARENT)
            .chunks_exact(4)
            .filter(|pixel| pixel[3] != 0)
            .count();
        for message in messages {
            let _ = app.update(message).unwrap();
        }
        pixels
    }

    #[test]
    fn early_exit_completes_once_stays_hidden_and_can_reenter() {
        let shell = r#"
            iced.define('card', function()
                return {view=function(self, props)
                    instance = self
                    return iced.container(iced.space(), {width=80,height=40,transition={
                        enter={from={scale=0},duration=10,easing='linear'},
                        exit={to={x=80,scale=0},duration=10,easing='linear',callback=function(owner)
                            assert(owner == instance)
                            completed = (completed or 0) + 1
                        end}
                    }})
                end}
            end)
            local app = {}
            function app:update() end
            function app:view() return iced.card({id='one'}) end
            return riced.daemon(app,'test',app.update,app.view):run()
        "#;
        let mut app = load(shell, Path::new("shell.lua")).unwrap();
        let window = window::Id::unique();
        let mut tree = iced::advanced::widget::Tree::empty();
        animation_frame(
            &mut app,
            window,
            &mut tree,
            std::time::Instant::now() + Duration::from_secs(1),
        );
        app.lua
            .load("instance.animate.exit(); instance.animate.exit()")
            .exec()
            .unwrap();
        animation_frame(
            &mut app,
            window,
            &mut tree,
            std::time::Instant::now() + Duration::from_secs(1),
        );
        animation_frame(
            &mut app,
            window,
            &mut tree,
            std::time::Instant::now() + Duration::from_secs(2),
        );
        assert_eq!(app.lua.globals().get::<u32>("completed").unwrap(), 1);
        let instance: Table = app.lua.globals().get("instance").unwrap();
        let node = app.view_node(window).unwrap();
        let control = node
            .get::<mlua::AnyUserData>("__animation")
            .unwrap()
            .borrow::<widgets::animation::Control>()
            .unwrap()
            .clone();
        assert!(control.state().hidden);
        app.lua.load("instance.animate.enter()").exec().unwrap();
        animation_frame(
            &mut app,
            window,
            &mut tree,
            std::time::Instant::now() + Duration::from_secs(3),
        );
        assert_eq!(
            instance,
            app.lua.globals().get::<Table>("instance").unwrap()
        );
        assert!(!control.state().hidden);
        app.lua.load("instance.animate.exit()").exec().unwrap();
        animation_frame(
            &mut app,
            window,
            &mut tree,
            std::time::Instant::now() + Duration::from_secs(4),
        );
        assert_eq!(app.lua.globals().get::<u32>("completed").unwrap(), 2);
    }

    #[test]
    fn component_instances_and_animation_controls_are_window_local() {
        let shell = r#"
            local app={}
            function app:update() end
            function app:view() return iced.row({iced.component('a','card'),iced.component('b','card')}) end
            return riced.daemon(app,'test',app.update,app.view):run()
        "#;
        let profile = Profile::new(&[
            ("shell.lua", shell),
            (
                "card.lua",
                r#"
            return {view=function(self)
                return iced.container(iced.space(),{transition={exit={to={scale=0}}}})
            end}
        "#,
            ),
        ]);
        let app = load(shell, &profile.shell()).unwrap();
        let a = window::Id::unique();
        let b = window::Id::unique();
        app.view(a).unwrap();
        app.view(b).unwrap();
        let mounts = app.mounts.borrow();
        let first = &mounts[&(a, "a".into())];
        first
            .table
            .get::<Table>("animate")
            .unwrap()
            .get::<Function>("exit")
            .unwrap()
            .call::<()>(())
            .unwrap();
        assert_eq!(first.control.state().event, "exit");
        assert_eq!(mounts[&(a, "b".into())].control.state().event, "enter");
        assert_eq!(mounts[&(b, "a".into())].control.state().event, "enter");
    }

    #[test]
    fn removed_component_finishes_exit_before_destruction() {
        let shell = r#"
            show=true
            local app={}
            function app:update() end
            function app:view() if show then return iced.component('card','card') end return iced.space() end
            return riced.daemon(app,'test',app.update,app.view):run()
        "#;
        let profile = Profile::new(&[
            ("shell.lua", shell),
            (
                "card.lua",
                r#"
            return {view=function(self)
                return iced.container(iced.space(),{width=80,height=40,transition={
                    exit={to={scale=0},duration=10,callback=function() finished=true end}
                }})
            end,on_destroyed=function() assert(finished); destroyed=(destroyed or 0)+1 end}
        "#,
            ),
        ]);
        let mut app = load(shell, &profile.shell()).unwrap();
        let window = window::Id::unique();
        let mut tree = iced::advanced::widget::Tree::empty();
        animation_frame(&mut app, window, &mut tree, std::time::Instant::now());
        app.lua.globals().set("show", false).unwrap();
        app.view(window).unwrap();
        assert_eq!(app.mounts.borrow().len(), 1);
        animation_frame(
            &mut app,
            window,
            &mut tree,
            std::time::Instant::now() + Duration::from_secs(1),
        );
        app.view(window).unwrap();
        assert!(app.mounts.borrow().is_empty());
        assert_eq!(app.lua.globals().get::<u32>("destroyed").unwrap(), 1);
    }

    #[test]
    fn change_callback_runs_after_interpolation_not_on_first_render() {
        let shell = r#"
            width=40
            local app={}
            function app:update() end
            function app:view()
                return iced.container(iced.space(),{width=width,height=20,transition={change={
                    props={'width'},duration=10,easing='linear',callback=function() finished=(finished or 0)+1 end
                }}})
            end
            return riced.daemon(app,'test',app.update,app.view):run()
        "#;
        let mut app = load(shell, Path::new("shell.lua")).unwrap();
        let window = window::Id::unique();
        let mut tree = iced::advanced::widget::Tree::empty();
        animation_frame(&mut app, window, &mut tree, std::time::Instant::now());
        assert!(
            app.lua
                .globals()
                .get::<Option<u32>>("finished")
                .unwrap()
                .is_none()
        );
        app.lua.globals().set("width", 80).unwrap();
        animation_frame(
            &mut app,
            window,
            &mut tree,
            std::time::Instant::now() + Duration::from_secs(1),
        );
        assert_eq!(app.lua.globals().get::<u32>("finished").unwrap(), 1);
        animation_frame(
            &mut app,
            window,
            &mut tree,
            std::time::Instant::now() + Duration::from_secs(2),
        );
        assert_eq!(app.lua.globals().get::<u32>("finished").unwrap(), 1);
    }

    #[test]
    fn list_events_have_independent_completion_callbacks() {
        let shell = r#"
            items={'a','b'}; added=0; removed=0; displaced=0
            local app={}
            function app:update() end
            function app:view()
                return iced.list_view(items,{id='items',render=function()
                    return iced.container(iced.space(),{width=80,height=20})
                end,transition={
                    enter={from={scale=0},duration=10,callback=function() added=added+1 end},
                    exit={to={scale=0},duration=20,callback=function() removed=removed+1 end},
                    displaced={duration=30,callback=function() displaced=displaced+1 end}
                }})
            end
            return riced.daemon(app,'test',app.update,app.view):run()
        "#;
        let mut app = load(shell, Path::new("shell.lua")).unwrap();
        let window = window::Id::unique();
        let mut tree = iced::advanced::widget::Tree::empty();
        animation_frame(
            &mut app,
            window,
            &mut tree,
            std::time::Instant::now() + Duration::from_secs(1),
        );
        assert_eq!(app.lua.globals().get::<u32>("added").unwrap(), 2);
        app.lua.load("items={'b'}").exec().unwrap();
        animation_frame(
            &mut app,
            window,
            &mut tree,
            std::time::Instant::now() + Duration::from_secs(2),
        );
        animation_frame(
            &mut app,
            window,
            &mut tree,
            std::time::Instant::now() + Duration::from_secs(3),
        );
        assert_eq!(app.lua.globals().get::<u32>("removed").unwrap(), 1);
        assert_eq!(app.lua.globals().get::<u32>("displaced").unwrap(), 1);
        assert_eq!(app.lua.globals().get::<u32>("added").unwrap(), 2);
    }

    #[test]
    fn parent_exit_retains_nested_component_content_and_cancels_children_on_removal() {
        let shell = r#"
            show=true
            local app={}
            function app:update() end
            function app:view() if show then return iced.component('parent','parent') end return iced.space() end
            return riced.daemon(app,'test',app.update,app.view):run()
        "#;
        let profile = Profile::new(&[
            ("shell.lua", shell),
            (
                "parent.lua",
                r#"
            return {view=function(self)
                return iced.container(iced.component('child','child'),{transition={exit={to={scale=0},duration=10}}})
            end}
        "#,
            ),
            (
                "child.lua",
                "return {view=function() return iced.container(iced.space(),{width=80,height=20}) end}",
            ),
        ]);
        let mut app = load(shell, &profile.shell()).unwrap();
        let window = window::Id::unique();
        let mut tree = iced::advanced::widget::Tree::empty();
        animation_frame(&mut app, window, &mut tree, std::time::Instant::now());
        let child = app.mounts.borrow()[&(window, "child".into())]
            .control
            .clone();
        app.lua.globals().set("show", false).unwrap();
        app.view(window).unwrap();
        assert!(child.state().alive);
        assert_eq!(app.mounts.borrow().len(), 2);
        animation_frame(
            &mut app,
            window,
            &mut tree,
            std::time::Instant::now() + Duration::from_secs(1),
        );
        app.view(window).unwrap();
        assert!(!child.state().alive);
        assert!(app.mounts.borrow().is_empty());
    }

    #[test]
    fn closed_window_discards_queued_animation_callbacks() {
        let shell = r#"
            local app={}
            function app:update() end
            function app:view() return iced.space() end
            return riced.daemon(app,'test',app.update,app.view):run()
        "#;
        let mut app = load(shell, Path::new("shell.lua")).unwrap();
        let window = window::Id::unique();
        app.view(window).unwrap();
        let control = widgets::animation::Control::default();
        control.state().pending_callback = true;
        let callback = app
            .lua
            .load("function() error('stale callback ran') end")
            .eval()
            .unwrap();
        app.forget_window(window).unwrap();
        let _ = app
            .update(Message::Animation {
                generation: app.generation,
                component: None,
                window: Some(window),
                control,
                serial: 1,
                callback,
                owner: Value::Nil,
                lifetime: None,
            })
            .unwrap();
    }

    #[test]
    fn early_exit_reflows_list_without_removing_source_items_and_can_reenter() {
        use iced::advanced::renderer::Headless;
        let shell = r##"
            notifications={{id=1},{id=2},{id=3}}
            instances={}; displaced=0; exited=0
            iced.define('card', function() return {view=function(self, item)
                instances[item.id]=self
                return iced.container(iced.space(),{width=80,height=40,
                    style={background='#ff0000'},transition={
                        enter={from={x=20},duration=10,easing='linear'},
                        exit={to={x=20},duration=10,easing='linear',callback=function() exited=exited+1 end}
                    }})
            end} end)
            local app={}
            function app:update() end
            function app:view()
                return iced.list_view(notifications,{id='notifications',key='id',spacing=8,
                    render=function(item) return iced.card(item) end,
                    transition={displaced={duration=10,easing='linear',callback=function() displaced=displaced+1 end}}
                })
            end
            return riced.daemon(app,'test',app.update,app.view):run()
        "##;
        let mut app = load(shell, Path::new("shell.lua")).unwrap();
        let window = window::Id::unique();
        let mut tree = iced::advanced::widget::Tree::empty();
        animation_frame(
            &mut app,
            window,
            &mut tree,
            std::time::Instant::now() + Duration::from_secs(1),
        );
        app.lua.load("instances[2].animate.exit()").exec().unwrap();
        animation_frame(
            &mut app,
            window,
            &mut tree,
            std::time::Instant::now() + Duration::from_secs(1),
        );
        animation_frame(
            &mut app,
            window,
            &mut tree,
            std::time::Instant::now() + Duration::from_secs(2),
        );

        let renderer = iced::futures::executor::block_on(iced::Renderer::new(
            iced::Font::DEFAULT,
            iced::Pixels(16.0),
            Some("tiny-skia"),
        ))
        .unwrap();
        let limits = iced::advanced::layout::Limits::new(Size::ZERO, Size::new(640.0, 480.0));
        let mut element = app.view(window).unwrap();
        tree.diff(&element);
        let layout = element
            .as_widget_mut()
            .layout(&mut tree, &renderer, &limits);
        assert_eq!(layout.children().len(), 2);
        assert_eq!(layout.size().height, 88.0);
        assert_eq!(layout.children()[1].bounds().y, 48.0);
        assert_eq!(app.lua.globals().get::<u32>("displaced").unwrap(), 1);
        assert_eq!(app.lua.globals().get::<u32>("exited").unwrap(), 1);
        assert_eq!(
            app.lua
                .globals()
                .get::<Table>("notifications")
                .unwrap()
                .raw_len(),
            3
        );

        app.lua.load("instances[2].animate.enter()").exec().unwrap();
        animation_frame(
            &mut app,
            window,
            &mut tree,
            std::time::Instant::now() + Duration::from_secs(3),
        );
        let mut element = app.view(window).unwrap();
        tree.diff(&element);
        let layout = element
            .as_widget_mut()
            .layout(&mut tree, &renderer, &limits);
        assert_eq!(layout.children().len(), 3);
        assert_eq!(layout.size().height, 136.0);
        assert_eq!(
            app.lua
                .globals()
                .get::<Table>("notifications")
                .unwrap()
                .raw_len(),
            3
        );

        app.lua
            .load("for _, instance in pairs(instances) do instance.animate.exit() end")
            .exec()
            .unwrap();
        animation_frame(
            &mut app,
            window,
            &mut tree,
            std::time::Instant::now() + Duration::from_secs(4),
        );
        let mut element = app.view(window).unwrap();
        tree.diff(&element);
        let layout = element
            .as_widget_mut()
            .layout(&mut tree, &renderer, &limits);
        assert!(layout.children().is_empty());
        assert_eq!(layout.size().height, 0.0);
        assert_eq!(
            app.lua
                .globals()
                .get::<Table>("notifications")
                .unwrap()
                .raw_len(),
            3
        );
    }

    #[test]
    fn last_list_item_remains_drawn_until_exit_finishes() {
        let shell = r##"
            items={'one'}
            local app={}
            function app:update() end
            function app:view()
                return iced.list_view(items,{id='items',render=function()
                    return iced.container(iced.space(),{width=80,height=40,style={background='#ff0000'}})
                end,transition={exit={to={scale=0},duration=100}}})
            end
            return riced.daemon(app,'test',app.update,app.view):run()
        "##;
        let mut app = load(shell, Path::new("shell.lua")).unwrap();
        let window = window::Id::unique();
        let mut tree = iced::advanced::widget::Tree::empty();
        assert!(animation_frame(&mut app, window, &mut tree, std::time::Instant::now()) > 0);
        app.lua.load("items={}").exec().unwrap();
        assert!(animation_frame(&mut app, window, &mut tree, std::time::Instant::now()) > 0);
        assert_eq!(
            animation_frame(
                &mut app,
                window,
                &mut tree,
                std::time::Instant::now() + Duration::from_secs(1)
            ),
            0
        );
    }

    #[test]
    fn identified_native_widget_exits_when_removed_from_a_component() {
        let shell = r##"
            show=true
            iced.define('panel', function() return {view=function()
                return iced.column(show and {iced.container(iced.space(),{id='card',width=80,height=40,
                    style={background='#ff0000'},transition={exit={to={scale=0},duration=100,callback=function() finished=true end}}
                })} or {})
            end} end)
            local app={}
            function app:update() end
            function app:view() return iced.panel() end
            return riced.daemon(app,'test',app.update,app.view):run()
        "##;
        let mut app = load(shell, Path::new("shell.lua")).unwrap();
        let window = window::Id::unique();
        let mut tree = iced::advanced::widget::Tree::empty();
        assert!(animation_frame(&mut app, window, &mut tree, std::time::Instant::now()) > 0);
        app.lua.globals().set("show", false).unwrap();
        assert!(animation_frame(&mut app, window, &mut tree, std::time::Instant::now()) > 0);
        assert_eq!(
            animation_frame(
                &mut app,
                window,
                &mut tree,
                std::time::Instant::now() + Duration::from_secs(1)
            ),
            0
        );
        assert!(app.lua.globals().get::<bool>("finished").unwrap());
    }

    #[test]
    fn native_exit_is_retained_only_on_its_own_surface() {
        let source = r#"
            local app={show=true}
            function app:view()
                return {
                    iced.layer(self.show and iced.container(iced.space(), {
                        id='card',width=50,height=50,transition={exit={to={scale=0}}}
                    }) or iced.space(),{id='left'}),
                    iced.layer(iced.space(),{id='right'})
                }
            end
            return riced.daemon(app,'test',function() end,app.view):run()
        "#;
        let mut app = load(source, Path::new("shell.lua")).unwrap();
        app.prepare_layers().unwrap();
        let left = app.layers[0].window;
        let right = app.layers[1].window;
        app.view(left).unwrap();
        app.view(right).unwrap();
        app.state.set("show", false).unwrap();
        let left = app.view(left).unwrap();
        let right = app.view(right).unwrap();
        let left = iced::advanced::widget::Tree::new(&left);
        let right = iced::advanced::widget::Tree::new(&right);
        assert!(!left.children[0].children.is_empty());
        assert!(right.children[0].children.is_empty());
    }

    #[test]
    fn native_ids_can_be_reused_on_different_surfaces() {
        let source = r#"
            local cached = iced.container(iced.space(),{id='cached',transition={exit={to={scale=0}}}})
            return riced.daemon({},'test',function() end,function()
                local function card()
                    return iced.column({cached, iced.container(iced.space(),{id='card',transition={exit={to={scale=0}}}})})
                end
                return {iced.layer(card(),{id='left'}),iced.layer(card(),{id='right'})}
            end):run()
        "#;
        let mut app = load(source, Path::new("shell.lua")).unwrap();
        app.prepare_layers().unwrap();
        for layer in &app.layers {
            app.view(layer.window).unwrap();
        }
    }

    #[test]
    fn surface_selection_preserves_list_data_identity() {
        let source = r#"
            local item={kind='container',id='data',transition={exit={to={scale=0}}}}
            return riced.daemon({},'test',function() end,function()
                return iced.list_view({item},{id='items',render=function(value)
                    assert(value == item and value.__animation == nil)
                    return iced.text(value.id)
                end})
            end):run()
        "#;
        let app = load(source, Path::new("shell.lua")).unwrap();
        let window = window::Id::unique();
        app.view(window).unwrap();
        app.view(window).unwrap();
    }

    #[test]
    fn update_receives_the_originating_component() {
        let shell = r#"
            local app = { origin = 'none' }
            function app:update(event, component) self.origin = component or 'none' end
            function app:view() return iced.component('left', 'left') end
            return riced.daemon(app, 'test', app.update, app.view):run()
        "#;
        let left = "local m = {}
function m:view() return iced.text('left') end
return m";
        let profile = Profile::new(&[("shell.lua", shell), ("left.lua", left)]);
        let mut app = load(shell, &profile.shell()).unwrap();
        let id = window::Id::unique();
        app.view(id).unwrap();

        let click = Value::String(app.lua.create_string("click").unwrap());
        let _ = app
            .update(Message::Lua(
                app.generation,
                Some(("left".into(), 0, Some(id))),
                click,
            ))
            .unwrap();
        assert_eq!(app.state.get::<String>("origin").unwrap(), "left");

        let _ = app.update(Message::Tick).unwrap();
        assert_eq!(app.state.get::<String>("origin").unwrap(), "none");
    }

    #[test]
    fn plugin_events_return_their_tasks() {
        let plugins = Rc::new(RefCell::new(plugins::Plugins::with_event(
            serde_json::json!({"ready": true}),
        )));
        let shell = r#"
            local app = {}
            function app:update(event)
                assert(event.type == 'plugin' and event.plugin == 'demo' and event.event.ready)
                return {type = 'clipboard_write', content = 'ready'}
            end
            function app:view() return iced.text('x') end
            return riced.daemon(app, 'test', app.update, app.view):run()
        "#;
        let mut app = load_with_plugins(shell, Path::new("shell.lua"), plugins).unwrap();
        let (changed, tasks) = app.poll_plugins().unwrap();
        assert!(changed);
        assert!(tasks.units() > 0);
    }

    #[test]
    fn one_bad_plugin_event_does_not_drop_the_rest() {
        let plugins = Rc::new(RefCell::new(plugins::Plugins::with_events([
            serde_json::json!({"n": 1}),
            serde_json::json!({"n": 2}),
        ])));
        let shell = r#"
            local app = { seen = {} }
            function app:update(event)
                if event.event.n == 1 then error('bad event') end
                self.seen[#self.seen + 1] = event.event.n
            end
            function app:view() return iced.text('x') end
            return riced.daemon(app, 'test', app.update, app.view):run()
        "#;
        let mut app = load_with_plugins(shell, Path::new("shell.lua"), plugins).unwrap();
        let (changed, _) = app.poll_plugins().unwrap();
        assert!(changed);
        let seen: Table = app.state.get("seen").unwrap();
        assert_eq!(seen.get::<u32>(1).unwrap(), 2);
    }

    #[test]
    fn widget_events_reach_update_with_typed_values() {
        let shell = r#"
            local app = {}
            function app:update(event) seen = event end
            function app:view() return iced.text('x') end
            return riced.daemon(app, 'test', app.update, app.view):run()
        "#;
        let mut app = load(shell, Path::new("shell.lua")).unwrap();
        let _ = app
            .update(Message::Widget(
                app.generation,
                None,
                "input".into(),
                WidgetEvent::Text("abc".into()),
            ))
            .unwrap();
        let seen: Table = app.lua.globals().get("seen").unwrap();
        assert_eq!(seen.get::<String>("event").unwrap(), "input");
        assert_eq!(seen.get::<String>("value").unwrap(), "abc");

        let _ = app
            .update(Message::Widget(
                app.generation,
                None,
                "toggle".into(),
                WidgetEvent::Bool(true),
            ))
            .unwrap();
        let seen: Table = app.lua.globals().get("seen").unwrap();
        assert!(seen.get::<bool>("value").unwrap());
    }

    #[test]
    fn component_lifecycle_signals_fire_once_per_mount() {
        let shell = r#"
            show = true
            local app = {}
            function app:update(event) end
            function app:view()
                if not show then return iced.text('empty') end
                return iced.component('left', 'left')
            end
            return riced.daemon(app, 'test', app.update, app.view):run()
        "#;
        let left = r#"
            local m = {}
            function m:view() return iced.text('left') end
            function m:on_visible(props, window)
                visible = (visible or 0) + 1
            end
            function m:on_destroyed(props, window)
                destroyed = (destroyed or 0) + 1
            end
            return m
        "#;
        let profile = Profile::new(&[("shell.lua", shell), ("left.lua", left)]);
        let app = load(shell, &profile.shell()).unwrap();
        let id = window::Id::unique();

        app.view(id).unwrap();
        app.view(id).unwrap();
        assert_eq!(app.lua.globals().get::<u32>("visible").unwrap(), 1);
        assert!(app.mounts.borrow().contains_key(&(id, "left".to_owned())));

        app.lua.globals().set("show", false).unwrap();
        app.view(id).unwrap();
        assert_eq!(app.lua.globals().get::<u32>("destroyed").unwrap(), 1);
        assert!(app.mounts.borrow().is_empty());
    }

    #[test]
    fn components_inside_containers_render() {
        let panel = "return {view=function() return iced.text('ok') end}";
        let profile = Profile::new(&[("panel.lua", panel)]);
        let source = r#"
            local app = {}
            function app:update() end
            function app:view() return iced.region(iced.component('panel', 'panel')) end
            return riced.daemon(app, 'test', app.update, app.view):run()
        "#;
        let app = load(source, &profile.shell()).unwrap();
        app.view(window::Id::unique()).unwrap();
    }

    #[test]
    fn failed_reload_keeps_plugin_callbacks_in_the_active_lua() {
        let (plugins, reply) = plugins::audit_ready();
        let source = r#"
            local app = {}
            function app:update() end
            function app:view() return iced.text('ok') end
            return riced.daemon(app, 'test', app.update, app.view):run()
        "#;
        let mut app = load_with_plugins(source, Path::new("shell.lua"), plugins).unwrap();
        assert!(
            app.reload("riced.plugins('demo').query(function() end); error('reject candidate')")
                .is_err()
        );
        reply();
        let _ = app.poll_plugins().unwrap();
    }

    #[test]
    fn plain_widget_module_reload_validates_live_windows() {
        let source = r#"
            local helper = require('helper')
            local app = {}
            function app:update() end
            function app:view() return helper.view() end
            return riced.daemon(app, 'test', app.update, app.view):run()
        "#;
        let profile = Profile::new(&[
            ("shell.lua", source),
            (
                "helper.lua",
                "return {view=function() return iced.text('ok') end}",
            ),
        ]);
        let mut app = load(source, &profile.shell()).unwrap();
        app.prepare_layers().unwrap();
        app.view(window::Id::unique()).unwrap();
        profile.write(
            "helper.lua",
            "return {view=function() return iced.text('bad', {unsupported=true}) end}",
        );
        assert!(app.reload_modules().is_err());
    }

    #[test]
    fn title_only_changes_mark_the_surface_dirty() {
        let mut app = load(
            r#"
            local app = {title='before'}
            function app:update() end
            function app:view()
                return iced.window({id='settings', title=self.title, content=iced.text('ok')})
            end
            return riced.daemon(app, 'test', app.update, app.view):run()
        "#,
            Path::new("shell.lua"),
        )
        .unwrap();
        app.prepare_layers().unwrap();
        let id = app.toplevels[0].window;
        app.view(id).unwrap();
        app.state.set("title", "after").unwrap();
        app.view(id).unwrap();
        assert!(app.surfaces_dirty.get());
    }

    #[test]
    fn same_component_id_mounts_once_per_window() {
        let shell = r#"
            show_layer = true
            local app = {}
            function app:update() end
            function app:view()
                local layer = show_layer and iced.component('left', 'left') or iced.text('layer')
                return {
                    iced.layer(layer, {id = 'layer', layer = 'background'}),
                    iced.window({id = 'window', content = iced.component('left', 'left')}),
                }
            end
            return riced.daemon(app, 'test', app.update, app.view):run()
        "#;
        let left = "local m = {}
function m:view() return iced.text('x') end
function m:on_visible() visible = (visible or 0) + 1 end
function m:on_destroyed() destroyed = (destroyed or 0) + 1 end
return m";
        let profile = Profile::new(&[("shell.lua", shell), ("left.lua", left)]);
        let mut app = load(shell, &profile.shell()).unwrap();
        app.prepare_layers().unwrap();
        let layer = app.layers[0].window;
        let window = app.toplevels[0].window;

        app.view(layer).unwrap();
        app.view(window).unwrap();
        app.view(layer).unwrap();
        app.view(window).unwrap();
        assert_eq!(app.lua.globals().get::<u32>("visible").unwrap(), 2);
        assert_eq!(app.mounts.borrow().len(), 2);
        assert!(!app.stale(&Some(("left".into(), 0, Some(layer)))));
        assert!(!app.stale(&Some(("left".into(), 0, Some(window)))));

        app.lua.globals().set("show_layer", false).unwrap();
        app.view(layer).unwrap();
        assert_eq!(app.lua.globals().get::<u32>("destroyed").unwrap(), 1);
        assert_eq!(app.mounts.borrow().len(), 1);
        assert!(app.stale(&Some(("left".into(), 0, Some(layer)))));
        assert!(!app.stale(&Some(("left".into(), 0, Some(window)))));
    }

    #[test]
    fn closing_a_window_drops_its_mounts() {
        let shell = r#"
            local app = { open = true }
            function app:update(event)
                if type(event) == 'table' and event.type == 'window_closed' then
                    self.open = false
                end
            end
            function app:view()
                if not self.open then return iced.text('closed') end
                return iced.window({
                    id = 'settings',
                    content = iced.component('left', 'left'),
                })
            end
            return riced.daemon(app, 'test', app.update, app.view):run()
        "#;
        let left = "local m = {}
function m:view() return iced.text('left') end
function m:on_destroyed() destroyed = (destroyed or 0) + 1 end
return m";
        let profile = Profile::new(&[("shell.lua", shell), ("left.lua", left)]);
        let mut app = load(shell, &profile.shell()).unwrap();
        app.prepare_layers().unwrap();
        let window = app.toplevels[0].window;
        app.view(window).unwrap();
        assert_eq!(app.mounts.borrow().len(), 1);

        let _ = app.update(Message::WindowClosed(window)).unwrap();
        assert!(app.toplevels.is_empty());
        assert!(app.mounts.borrow().is_empty());
        assert!(!app.windows_of.borrow().contains_key("left"));
        assert_eq!(app.lua.globals().get::<u32>("destroyed").unwrap(), 1);
    }

    #[test]
    fn reload_can_replace_layers_with_windows() {
        let layers = "return riced.daemon({}, 'test', function() end, function() \
            return iced.layer(iced.text('x'), {id='panel'}) end):run()";
        let windows = "return riced.daemon({}, 'test', function() end, function() \
            return iced.window({id='settings', content=iced.text('x')}) end):run()";
        let mut app = load(layers, Path::new("shell.lua")).unwrap();
        app.prepare_layers().unwrap();
        app.view(app.layers[0].window).unwrap();

        app.reload(windows).unwrap();
        assert!(app.layers.is_empty());
        assert_eq!(app.toplevels.len(), 1);
    }

    #[test]
    fn closed_windows_are_cleaned_up_even_if_the_view_keeps_declaring_them() {
        let shell = r#"
            local app = {}
            function app:update() end
            function app:view()
                return iced.window({id='settings', content=iced.component('left', 'left')})
            end
            return riced.daemon(app, 'test', app.update, app.view):run()
        "#;
        let profile = Profile::new(&[
            ("shell.lua", shell),
            (
                "left.lua",
                "return {view=function() return iced.text('left') end}",
            ),
        ]);
        let mut app = load(shell, &profile.shell()).unwrap();
        app.prepare_layers().unwrap();
        let id = app.toplevels[0].window;
        app.view(id).unwrap();
        app.sizes.borrow_mut().insert(id, Size::new(100.0, 50.0));
        let _ = app.update(Message::WindowClosed(id)).unwrap();
        app.view(id).unwrap();
        assert!(app.toplevels[0].closed);
        assert!(app.mounts.borrow().is_empty());
        assert!(app.windows.borrow().is_empty());
        assert!(app.sizes.borrow().is_empty());
        assert!(app.windows_of.borrow().is_empty());
    }

    #[test]
    fn rejects_duplicate_and_cyclic_components() {
        for view in [
            "iced.column({iced.component('same', 'left'), iced.component('same', 'right')})",
            "iced.component('loop', 'loop')",
            "iced.component('', 'left')",
            "iced.component('missing', 'nope')",
            "iced.component('left', 'no_view')",
        ] {
            let profile = Profile::new(&[
                (
                    "shell.lua",
                    &format!(
                        "return riced.daemon({{}}, 'test', function() end, function() return {view} end):run()"
                    ),
                ),
                (
                    "left.lua",
                    "local m = {}\nfunction m:view() return iced.text('x') end\nreturn m",
                ),
                (
                    "right.lua",
                    "local m = {}\nfunction m:view() return iced.text('y') end\nreturn m",
                ),
                (
                    "loop.lua",
                    "local m = {}\nfunction m:view() return iced.component('loop', 'loop') end\nreturn m",
                ),
                ("no_view.lua", "return {}"),
            ]);
            let app = load(&format!("return riced.daemon({{}}, 'test', function() end, function() return {view} end):run()"), &profile.shell()).unwrap();
            assert!(app.view(window::Id::unique()).is_err(), "accepted {view}");
        }
    }

    #[test]
    fn module_save_reloads_only_the_affected_component() {
        let profile = component_profile(&versioned("v1"));
        let mut app = load(COMPONENT_SHELL, &profile.shell()).unwrap();
        let id = window::Id::unique();
        app.view(id).unwrap();

        profile.write("left.lua", &versioned("v2"));
        assert!(matches!(app.check_reload().unwrap(), Change::Windows(windows) if windows == [id]));
        app.view(id).unwrap();

        assert_eq!(seen(&app, "seen_left"), "v2:0");
        assert_eq!(app.mounts.borrow()[&(id, "left".to_owned())].revision, 1);
        assert_eq!(app.mounts.borrow()[&(id, "right".to_owned())].revision, 0);
        assert!(matches!(app.check_reload().unwrap(), Change::None));
    }

    #[test]
    fn bounds_changes_do_not_skip_saved_modules() {
        let source = versioned("v1").replace(
            "iced.text('left')",
            "iced.measure(iced.text('left'), {id = 'module-target'})",
        );
        let profile = component_profile(&source);
        let mut app = load(COMPONENT_SHELL, &profile.shell()).unwrap();
        let id = window::Id::unique();
        let mut tree = iced::advanced::widget::Tree::empty();
        animation_frame(&mut app, id, &mut tree, std::time::Instant::now());
        profile.write("left.lua", &versioned("v2"));
        assert!(matches!(app.check_reload().unwrap(), Change::All));
        app.view(id).unwrap();
        assert_eq!(seen(&app, "seen_left"), "v2:0");
        app.forget_window(id).unwrap();
    }

    #[test]
    fn stale_component_events_are_ignored_after_its_module_reloads() {
        let profile = component_profile(&versioned("v1"));
        let mut app = load(COMPONENT_SHELL, &profile.shell()).unwrap();
        let id = window::Id::unique();
        app.view(id).unwrap();

        let old = Message::Lua(
            app.generation,
            Some(("left".into(), 0, Some(id))),
            Value::String(app.lua.create_string("click").unwrap()),
        );
        profile.write("left.lua", &versioned("v2"));
        app.check_reload().unwrap();
        app.view(id).unwrap();

        let _ = app.update(old).unwrap();
        assert_eq!(app.state.get::<u32>("count").unwrap(), 0);
        let _ = app
            .update(Message::Lua(
                app.generation,
                Some(("left".into(), 1, Some(id))),
                Value::String(app.lua.create_string("click").unwrap()),
            ))
            .unwrap();
        assert_eq!(app.state.get::<u32>("count").unwrap(), 1);
    }

    #[test]
    fn broken_component_save_keeps_the_previous_module_and_does_not_retry() {
        let profile = component_profile(&versioned("v1"));
        let mut app = load(COMPONENT_SHELL, &profile.shell()).unwrap();
        let id = window::Id::unique();
        app.view(id).unwrap();

        profile.write(
            "left.lua",
            "local m = {}\nfunction m:view() error('broken') end\nreturn m",
        );
        assert!(app.check_reload().is_err());
        assert_eq!(app.mounts.borrow()[&(id, "left".to_owned())].revision, 0);
        assert!(matches!(app.check_reload().unwrap(), Change::None));
        app.view(id).unwrap();
        assert_eq!(seen(&app, "seen_left"), "v1:0");

        profile.write("left.lua", &versioned("v3"));
        assert!(matches!(app.check_reload().unwrap(), Change::Windows(_)));
        app.view(id).unwrap();
        assert_eq!(seen(&app, "seen_left"), "v3:0");
    }

    #[test]
    fn rejected_module_reload_restores_instances_and_animation_controls() {
        let shell = "return riced.daemon({},'test',function() end,function() return iced.component('card','card') end):run()";
        let original = "return {view=function(self) return iced.text('ok') end, on_visible=function(self) self.count=42 end}";
        let profile = Profile::new(&[("shell.lua", shell), ("card.lua", original)]);
        let mut app = load(shell, &profile.shell()).unwrap();
        let window = window::Id::unique();
        app.view(window).unwrap();
        let old = app.mounts.borrow()[&(window, "card".into())].clone();
        old.control.state().pending_callback = true;
        profile.write(
            "card.lua",
            &original.replace("iced.text('ok')", "iced.text('bad',{bogus=true})"),
        );
        assert!(app.reload_modules().is_err());
        assert!(old.control.state().alive);
        assert!(old.control.state().pending_callback);
        app.view(window).unwrap();
        let current = app.mounts.borrow()[&(window, "card".into())].clone();
        assert_eq!(current.table, old.table);
        assert_eq!(current.table.get::<u32>("count").unwrap(), 42);
        current
            .table
            .get::<Table>("animate")
            .unwrap()
            .get::<Function>("exit")
            .unwrap()
            .call::<()>(())
            .unwrap();
        assert_eq!(old.control.state().event, "exit");

        profile.write("card.lua", &original.replace("'ok'", "'updated'"));
        app.reload_modules().unwrap();
        assert!(!old.control.state().alive);
    }

    #[test]
    fn layers_are_matched_by_id_not_position() {
        let source = "return riced.daemon({}, 'test', function() end, function() return { \
            iced.layer(iced.text('a'), {id='a', layer='background'}), \
            iced.layer(iced.text('b'), {id='b', layer='top'}), \
        } end):run()";
        let mut app = load(source, Path::new("shell.lua")).unwrap();
        app.prepare_layers().unwrap();
        assert_eq!(app.layers[0].id, "a");
        assert_eq!(app.layers[1].id, "b");
        for layer in &app.layers {
            app.view(layer.window).unwrap();
        }

        let reordered = source
            .replace(
                "iced.layer(iced.text('a'), {id='a', layer='background'})",
                "TEMP",
            )
            .replace(
                "iced.layer(iced.text('b'), {id='b', layer='top'})",
                "iced.layer(iced.text('a'), {id='a', layer='background'})",
            )
            .replace("TEMP", "iced.layer(iced.text('b'), {id='b', layer='top'})");
        app.reload(&reordered).unwrap();

        let renamed = source.replace("{id='b'", "{id='c'");
        app.reload(&renamed).unwrap();
        assert!(app.layers.iter().any(|layer| layer.id == "c"));

        let duplicate = "return riced.daemon({}, 'test', function() end, function() return { \
            iced.layer(iced.text('a'), {id='a'}), iced.layer(iced.text('b'), {id='a'}) } end):run()";
        let mut app = load(duplicate, Path::new("shell.lua")).unwrap();
        assert!(
            app.prepare_layers()
                .unwrap_err()
                .to_string()
                .contains("duplicate surface id")
        );
    }

    #[test]
    fn updates_lua_state_and_builds_widgets() {
        let mut app = load(r#"
            local app = { count = 0 }
            function app:update(event) self.count = self.count + 1 end
            function app:view(props)
                self.window = props.window
                return iced.column({ iced.text(tostring(self.count)), iced.button('Add', {on_press='add'}) })
            end
            function app:redraw_scope(event) return self.window end
            return riced.daemon(app, 'test', app.update, app.view):redraw_scope(app.redraw_scope):run()
        "#, Path::new("shell.lua")).unwrap();
        let id = window::Id::unique();
        app.view(id).unwrap();
        let _ = app.update(Message::Tick).unwrap();
        assert_eq!(app.state.get::<u32>("count").unwrap(), 1);
        assert_eq!(app.redraw(&Message::Tick).unwrap(), Scope::Window(id));
    }

    #[test]
    fn invalid_settings_fail_before_startup() {
        let error = load("local a = {}; return riced.daemon(a, 'test', function() end, function() end):settings({layer_settings={start_mode='typo'}}):run()", Path::new("shell.lua")).err().unwrap();
        assert!(error.to_string().contains("unknown start_mode"));
    }

    #[test]
    fn callback_errors_keep_lua_context() {
        let mut app = load("return riced.daemon({}, 'test', function() error('broken update') end, function() return iced.text('x') end):run()", Path::new("shell.lua")).unwrap();
        assert!(
            app.update(Message::Tick)
                .unwrap_err()
                .to_string()
                .contains("broken update")
        );
    }

    #[test]
    fn global_theme_is_shared_and_applied() {
        let app = load(
            r##"
            config.theme.primary = "#abcdef"
            assert(theme.primary == config.theme.primary)
            return riced.daemon({}, 'test', function() end, function()
                return iced.text('x', {color=theme.secondary})
            end):theme(config.theme):run()
        "##,
            Path::new("shell.lua"),
        )
        .unwrap();
        assert_eq!(
            app.theme(window::Id::unique()).unwrap().palette().primary,
            iced::Color::from_rgb8(0xab, 0xcd, 0xef)
        );
    }

    #[test]
    fn view_receives_surface_size_after_resize() {
        let shell = r#"
            local app = {}
            function app:view(props)
                seen_width = props.width
                seen_height = props.height
                return iced.text('x')
            end
            return riced.daemon(app, 'test', function() end, app.view):run()
        "#;
        let mut app = load(shell, Path::new("shell.lua")).unwrap();
        let id = window::Id::unique();
        app.view(id).unwrap();
        assert!(
            app.lua
                .globals()
                .get::<Option<f32>>("seen_width")
                .unwrap()
                .is_none()
        );
        let _ = app
            .update(Message::Resized(id, Size::new(800.0, 60.0)))
            .unwrap();
        app.view(id).unwrap();
        assert_eq!(app.lua.globals().get::<f32>("seen_width").unwrap(), 800.0);
        assert_eq!(app.lua.globals().get::<f32>("seen_height").unwrap(), 60.0);
    }

    #[test]
    fn timer_subscription_validates_interval() {
        let app = load("return riced.daemon({}, 'test', function() end, function() end):subscription(function() return iced.every(1000) end):run()", Path::new("shell.lua")).unwrap();
        assert!(app.subscription().is_ok());
        app.options
            .set(
                "subscription",
                app.lua
                    .load("function() return {every=0} end")
                    .eval::<Function>()
                    .unwrap(),
            )
            .unwrap();
        assert!(app.subscription().is_err());
    }

    #[test]
    fn required_layer_module_updates_settings_without_id_changes_or_state_reset() {
        let source = r#"
            local panel = require('panel')
            local app = { count = 7 }
            function app:view() return { panel.view() } end
            return riced.daemon(app, 'test', function() end, app.view):run()
        "#;
        let before = "local m = {}; function m.view() return iced.layer(iced.text('before'), {id='panel', size=iced.Size(iced.Extent.fill,32), anchor={'top','left','right'}, exclusive_zone=32}) end; return m";
        let profile = Profile::new(&[("shell.lua", source), ("panel.lua", before)]);
        let mut app = load(source, &profile.shell()).unwrap();
        app.prepare_layers().unwrap();
        let window = app.layers[0].window;
        app.view(window).unwrap();
        let after = before.replace("32", "64").replace("before", "after");
        profile.write("panel.lua", &after);
        assert!(matches!(app.check_reload().unwrap(), Change::Windows(ids) if ids == [window]));
        assert_eq!(app.layers[0].window, window);
        assert_eq!(app.layers[0].settings.exclusive_zone, Some(64));
        assert_eq!(app.state.get::<u32>("count").unwrap(), 7);
        assert_eq!(app.generation, 0);
        assert!(app.pending.iter().any(|action| action.0 == Some(window)
            && matches!(action.1, ExWlShellCustomAction::LayoutChange { .. })));
        assert!(
            app.pending
                .iter()
                .any(|action| matches!(action.1, ExWlShellCustomAction::ExclusiveZoneChange(64)))
        );
        app.pending.clear();

        profile.write("panel.lua", &after.replace("id='panel', ", ""));
        app.check_reload().unwrap();
        assert_eq!(app.layers[0].window, window);
        assert_eq!(app.layers[0].id, "#1");
        assert!(app.pending.is_empty());

        profile.write(
            "panel.lua",
            &after.replace("iced.text('after')", "iced.text(nil)"),
        );
        assert!(app.check_reload().is_err());
        assert_eq!(app.layers[0].id, "#1");
        assert!(app.pending.is_empty());
        app.view(window).unwrap();
        assert!(matches!(app.check_reload().unwrap(), Change::None));

        profile.write("panel.lua", &after.replace("64", "96"));
        app.check_reload().unwrap();
        assert_eq!(app.layers[0].settings.exclusive_zone, Some(96));
    }

    #[test]
    fn require_dependencies_reload_even_when_loaded_before_components() {
        let source = "local panel=require('panel'); return riced.daemon({}, 'test', function() end, function() return panel.view() end):run()";
        let profile = Profile::new(&[
            ("shell.lua", source),
            (
                "panel.lua",
                "local height=require('height'); local m={}; function m.view() return iced.layer(iced.text('panel'), {size=iced.Size(iced.Extent.fill,height)}) end; return m",
            ),
            ("height.lua", "return 32"),
        ]);
        let mut app = load(source, &profile.shell()).unwrap();
        app.prepare_layers().unwrap();
        let window = app.layers[0].window;
        profile.write("height.lua", "return 64");
        app.check_reload().unwrap();
        assert_eq!(
            app.layers[0].settings.size,
            iced_exwlshell::reexport::LayerSize::fill_width(64)
        );
        assert_eq!(app.layers[0].window, window);
    }

    #[test]
    fn sizing_helpers_validate_and_render() {
        let app = load(
            r#"
            assert(iced.Layer.background == 'background' and iced.Layer.top == 'top'
                and iced.Layer.overlay == 'overlay')
            assert(iced.Output.active == 'active' and iced.Blur.none == 'none')
            assert(iced.KeyboardInteractivity.on_demand == 'on_demand'
                and iced.KeyboardInteractivity.exclusive == 'exclusive')
            for _, value in ipairs({0, -1, 1.5, 65536, '3', false, math.huge, 0/0}) do
                assert(not pcall(iced.Length.fill_portion, value))
            end
            assert(not pcall(iced.Length.fill_portion))
            assert(not pcall(iced.Size, iced.Extent.fill))
            local function view()
                return iced.row({
                    iced.text('a', {width=iced.Length.fill_portion(3)}),
                    iced.text('b', {width=iced.Length.fill, height=iced.Length.shrink}),
                })
            end
            return riced.daemon({}, 'test', function() end, view):run()
        "#,
            Path::new("shell.lua"),
        )
        .unwrap();
        app.view(window::Id::unique()).unwrap();
    }

    #[test]
    fn widget_constructors_do_not_mutate_shared_props() {
        let app = load(
            r#"
            local props = {size=20}
            local a = iced.text('a', props)
            local b = iced.text('b', props)
            assert(a.content == 'a' and b.content == 'b' and props.kind == nil)
            return riced.daemon({}, 'test', function() end, function() return a end):run()
        "#,
            Path::new("shell.lua"),
        )
        .unwrap();
        app.view(window::Id::unique()).unwrap();
    }

    #[test]
    fn update_tasks_are_built_and_clipboard_results_return() {
        let mut app = load(
            r#"
            local app = { result = nil }
            function app:update(event)
                if type(event) == 'table' and event.type == 'clipboard' then
                    self.result = event.value
                    return nil
                end
                return {
                    { type = 'clipboard_write', content = 'hello' },
                    { type = 'focus', id = 'search' },
                }
            end
            return riced.daemon(app, 'test', app.update, function() return iced.text('x') end):run()
        "#,
            Path::new("shell.lua"),
        )
        .unwrap();
        let task = app.update(Message::Tick).unwrap();
        assert!(task.units() > 0);
        let _ = app
            .update(Message::Clipboard(app.generation, Some("copied".into())))
            .unwrap();
        assert_eq!(app.state.get::<String>("result").unwrap(), "copied");
    }

    #[test]
    fn invalid_tasks_are_rejected() {
        let mut app = load(
            r#"
            return riced.daemon({}, 'test', function()
                return { type = 'bogus' }
            end, function() return iced.text('x') end):run()
        "#,
            Path::new("shell.lua"),
        )
        .unwrap();
        assert!(app.update(Message::Tick).is_err());
    }

    #[test]
    fn text_editor_actions_update_content_and_reach_update() {
        let mut app = load(
            r#"
            local app = { got = nil }
            function app:update(event)
                if type(event) == 'table' then self.got = event.value end
            end
            function app:view()
                return iced.text_editor(nil, { id = 'editor', value = self.text or '', on_input = 'changed' })
            end
            return riced.daemon(app, 'test', app.update, app.view):run()
        "#,
            Path::new("shell.lua"),
        )
        .unwrap();
        app.view(window::Id::unique()).unwrap();
        let action =
            iced::widget::text_editor::Action::Edit(iced::widget::text_editor::Edit::Insert('x'));
        let _ = app
            .update(Message::Editor(
                app.generation,
                None,
                EditorAction {
                    id: "editor".into(),
                    event: "changed".into(),
                    input: true,
                    action,
                },
            ))
            .unwrap();
        assert_eq!(
            widgets::editor_text(app.generation, "editor").as_deref(),
            Some("x")
        );
        assert_eq!(app.state.get::<String>("got").unwrap(), "x");

        // A static value prop must not reset the editor on the next render.
        app.view(window::Id::unique()).unwrap();
        assert_eq!(
            widgets::editor_text(app.generation, "editor").as_deref(),
            Some("x")
        );
    }

    #[test]
    fn subscription_lists_build_and_validate() {
        let app = load(
            r#"
            local app = {}
            function app:subscription()
                return {
                    iced.every(500),
                    iced.frames(),
                    iced.on_key(true),
                    iced.on_key(false),
                    iced.on_mouse(),
                }
            end
            return riced.daemon(app, 'test', function() end, function() return iced.text('x') end)
                :subscription(app.subscription):run()
        "#,
            Path::new("shell.lua"),
        )
        .unwrap();
        let _ = app.subscription().unwrap();

        let app = load(
            r#"
            local app = {}
            function app:subscription() return { type = 'bogus' } end
            return riced.daemon(app, 'test', function() end, function() return iced.text('x') end)
                :subscription(app.subscription):run()
        "#,
            Path::new("shell.lua"),
        )
        .unwrap();
        assert!(app.subscription().is_err());
    }
}
