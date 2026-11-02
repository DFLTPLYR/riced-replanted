use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    rc::Rc,
    time::Duration,
};

use iced::{Element, Subscription, Task, Theme, theme, window};
use iced_exwlshell::{
    Settings,
    actions::{ExwlShellCustomAction, ExwlShellCustomActionWithId},
    redraw::Scope,
    reexport::KeyboardInteractivity,
    settings::StartMode,
};
use mlua::{Function, Lua, LuaSerdeExt, Table, UserData, Value};

use crate::{layers, modules, plugins, update, update::Layer, widgets, widgets::Ctx};

#[derive(Debug, Clone)]
pub enum Message {
    Lua(u64, Option<(String, u64)>, Value),
    Tick,
    CheckReload,
    PollPlugins,
    Reloaded,
    RedrawWindow(window::Id),
    Surface(ExwlShellCustomActionWithId),
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
}

impl TryFrom<Message> for ExwlShellCustomActionWithId {
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
    windows: RefCell<Vec<window::Id>>,
    layers: Vec<Layer>,
    pending: Vec<ExwlShellCustomActionWithId>,
    modules: RefCell<modules::Modules>,
    mounts: RefCell<HashMap<String, Mount>>,
    windows_of: RefCell<HashMap<String, Vec<window::Id>>>,
    plugins: Rc<RefCell<plugins::Plugins>>,
}

// Iced's rendering callbacks cannot return errors; a broken Lua callback is fatal.
fn checked<T>(result: mlua::Result<T>) -> T {
    result.unwrap_or_else(|error| {
        eprintln!("riced: {error}");
        std::process::exit(1);
    })
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
        windows: RefCell::new(Vec::new()),
        layers: Vec::new(),
        pending: Vec::new(),
        modules: RefCell::new(modules::Modules::default()),
        mounts: RefCell::new(HashMap::new()),
        windows_of: RefCell::new(HashMap::new()),
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

    fn update(&mut self, message: Message) -> mlua::Result<()> {
        let event = match message {
            Message::Lua(generation, component, value) if generation == self.generation => {
                if let Some((id, revision)) = component {
                    // Events from a component whose module reloaded in the meantime are stale.
                    let current = self.mounts.borrow().get(&id).cloned();
                    let stale = current.is_none_or(|mount| {
                        mount.revision != revision
                            || self.modules.borrow().revision(&mount.module) != revision
                    });
                    if stale {
                        return Ok(());
                    }
                }
                value
            }
            Message::Lua(..)
            | Message::CheckReload
            | Message::PollPlugins
            | Message::Reloaded
            | Message::RedrawWindow(_)
            | Message::Surface(_) => return Ok(()),
            Message::Tick => {
                let event = self.lua.create_table()?;
                event.set("type", "tick")?;
                Value::Table(event)
            }
        };
        self.update.call((self.state.clone(), event))
    }

    fn poll_plugins(&mut self) -> mlua::Result<bool> {
        let (changed, events) = self.plugins.borrow_mut().poll(&self.lua)?;
        let mut events = events.into_iter();
        let mut fired = false;
        while let Some(plugins::Update::Event(name, event)) = events.next() {
            fired = true;
            let payload = self.lua.create_table()?;
            payload.set("type", "plugin")?;
            payload.set("plugin", name)?;
            payload.set("event", self.lua.to_value(&event)?)?;
            self.update.call::<()>((self.state.clone(), payload))?;
        }
        Ok(changed || fired)
    }

    fn view_node(&self, id: window::Id) -> mlua::Result<Table> {
        let props = self.lua.create_table()?;
        props.set("window", self.window(id)?)?;
        let node = self.view.call((self.state.clone(), props))?;
        self.modules.borrow_mut().watch_all(&self.lua)?;
        Ok(node)
    }

    // Replaces iced.component nodes with their module's rendered children. Children are
    // rebuilt on every render, so props stay fresh; only the module itself survives reloads.
    fn expand(
        &self,
        node: Table,
        window: window::Id,
        path: &mut Vec<String>,
        ids: &mut HashSet<String>,
    ) -> mlua::Result<Table> {
        match node.get::<Option<String>>("kind")?.as_deref() {
            Some("component") => self.expand_component(node, window, path, ids),
            Some("row" | "column") => {
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
            Some("container" | "button" | "layer") => {
                if let Value::Table(child) = node.get::<Value>("content")? {
                    node.set("content", self.expand(child, window, path, ids)?)?;
                }
                Ok(node)
            }
            _ => Ok(node),
        }
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
        let child: Table = view.call((table, props, self.window(window)?))?;
        self.mounts.borrow_mut().insert(
            id.clone(),
            Mount {
                module: module.clone(),
                revision,
            },
        );
        let mut windows_of = self.windows_of.borrow_mut();
        let owners = windows_of.entry(module).or_default();
        if !owners.contains(&window) {
            owners.push(window);
        }
        drop(windows_of);
        path.push(id.clone());
        let child = self.expand(child, window, path, ids)?;
        path.pop();
        child.set("__component_id", id)?;
        child.set("__component_revision", revision)?;
        Ok(child)
    }

    fn prepare_layers(&mut self) -> mlua::Result<()> {
        let node = self.view_node(window::Id::unique())?;
        let mut layers = Vec::new();
        let mut ids = HashSet::new();
        for (index, node) in layers::nodes(node)?.into_iter().enumerate() {
            let settings = node.get("settings")?;
            let id = layers::id(&settings, index)?;
            if !ids.insert(id.clone()) {
                return Err(mlua::Error::runtime(format!("duplicate layer id: {id}")));
            }
            if node.get::<Option<Table>>("content")?.is_none() {
                return Err(mlua::Error::runtime(
                    "layer content must be an iced element",
                ));
            }
            layers.push(Layer {
                id,
                window: window::Id::unique(),
                settings: layers::settings(settings)?,
            });
        }
        self.layers = layers;
        Ok(())
    }

    fn layer_tasks(&self) -> Task<Message> {
        Task::batch(self.layers.iter().map(|layer| {
            Task::done(Message::Surface(ExwlShellCustomActionWithId::new(
                None,
                ExwlShellCustomAction::NewLayerShell {
                    id: layer.window,
                    settings: layer.settings.clone(),
                },
            )))
        }))
    }

    fn reconcile_layers(&mut self) -> mlua::Result<()> {
        let previous = self.layers.clone();
        self.prepare_layers()?;
        if !previous.is_empty() && self.layers.is_empty() {
            self.layers = previous;
            return Err(mlua::Error::runtime(
                "cannot replace layer views with a widget-only view during reload",
            ));
        }
        self.pending
            .extend(update::reconcile(&previous, &mut self.layers));
        if !previous.is_empty() {
            self.windows
                .borrow_mut()
                .retain(|id| self.layers.iter().any(|layer| layer.window == *id));
        }
        Ok(())
    }

    fn reload_tasks(&mut self, change: Change) -> Task<Message> {
        let surfaces = std::mem::take(&mut self.pending)
            .into_iter()
            .fold(Task::none(), |task, action| {
                task.chain(Task::done(Message::Surface(action)))
            });
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
        let mut node = self.view_node(id)?;
        if !self.layers.is_empty() {
            let nodes = layers::nodes(node)?;
            if nodes.len() != self.layers.len() {
                return Err(mlua::Error::runtime(
                    "layer count changed; restart riced to apply it",
                ));
            }
            let mut by_id = HashMap::new();
            for (index, node) in nodes.into_iter().enumerate() {
                let settings = node.get("settings")?;
                let id = layers::id(&settings, index)?;
                let layer = self
                    .layers
                    .iter()
                    .find(|layer| layer.id == id)
                    .ok_or_else(|| {
                        mlua::Error::runtime("layer ids changed; restart riced to apply them")
                    })?;
                if layers::settings(settings)? != layer.settings {
                    return Err(mlua::Error::runtime(
                        "layer settings changed; restart riced to apply them",
                    ));
                }
                if by_id.insert(id, node).is_some() {
                    return Err(mlua::Error::runtime("duplicate layer id"));
                }
            }
            let layer = self.layers.iter().find(|layer| layer.window == id);
            // A replaced surface can receive a final render before its close action runs.
            let Some(layer) = layer else {
                return Ok(iced::widget::Space::new().into());
            };
            node = by_id
                .remove(&layer.id)
                .ok_or_else(|| {
                    mlua::Error::runtime("layer ids changed; restart riced to apply them")
                })?
                .get("content")?;
        }
        let mut path = Vec::new();
        let mut ids = HashSet::new();
        let node = self.expand(node, id, &mut path, &mut ids)?;
        let element = widgets::element(node, &Ctx::new(self.generation))?;
        let mut windows = self.windows.borrow_mut();
        if !windows.contains(&id) {
            windows.push(id);
        }
        Ok(element)
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
        match self.call("subscription", Value::Boolean(true))? {
            Value::Nil => Ok(Subscription::none()),
            Value::Table(table) => {
                let ms: u64 = table.get("every")?;
                if !(1..=86_400_000).contains(&ms) {
                    return Err(mlua::Error::runtime(
                        "timer interval must be from 1 to 86400000 ms",
                    ));
                }
                Ok(iced::time::every(Duration::from_millis(ms)).map(|_| Message::Tick))
            }
            _ => Err(mlua::Error::runtime(
                "subscription must return iced.every(ms) or nil",
            )),
        }
    }

    fn redraw(&self, message: &Message) -> mlua::Result<Scope> {
        let event = match message {
            Message::CheckReload | Message::PollPlugins => return Ok(Scope::None),
            Message::Reloaded => return Ok(Scope::All),
            Message::Surface(_) => return Ok(Scope::None),
            Message::RedrawWindow(id) => return Ok(Scope::Window(*id)),
            Message::Lua(generation, _, value) if *generation == self.generation => value.clone(),
            Message::Lua(..) => return Ok(Scope::None),
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

    fn settings(&self) -> mlua::Result<Settings> {
        let mut settings = Settings::default();
        let Some(table) = self.options.get::<Option<Table>>("settings")? else {
            return Ok(settings);
        };
        keys(&table, &["id", "layer_settings"])?;
        settings.id = table.get("id")?;
        if let Some(layer) = table.get::<Option<Table>>("layer_settings")? {
            keys(&layer, &["start_mode", "keyboard_interactivity"])?;
            if let Some(mode) = layer.get::<Option<String>>("start_mode")? {
                settings.layer_settings.start_mode = match mode.as_str() {
                    "background" => StartMode::Background,
                    "active" => StartMode::Active,
                    "all_screens" => StartMode::AllScreens,
                    _ => return Err(mlua::Error::runtime(format!("unknown start_mode: {mode}"))),
                };
            }
            if let Some(mode) = layer.get::<Option<String>>("keyboard_interactivity")? {
                settings.layer_settings.keyboard_interactivity = match mode.as_str() {
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
        Ok(settings)
    }

    fn reload(&mut self, source: &str) -> mlua::Result<()> {
        self.plugins.borrow_mut().clear_pending();
        let mut candidate = load_with_plugins(source, &self.path, Rc::clone(&self.plugins))?;
        let current = self.settings()?;
        let next = candidate.settings()?;
        if self.namespace != candidate.namespace
            || current.id != next.id
            || current.layer_settings.keyboard_interactivity
                != next.layer_settings.keyboard_interactivity
            || std::mem::discriminant(&current.layer_settings.start_mode)
                != std::mem::discriminant(&next.layer_settings.start_mode)
        {
            return Err(mlua::Error::runtime(
                "namespace or startup settings changed; restart riced to apply them",
            ));
        }
        candidate.generation = self.generation + 1;
        candidate.layers = self.layers.clone();
        candidate.reconcile_layers()?;
        for layer in &candidate.layers {
            candidate.view(layer.window)?;
        }
        for &id in self.windows.borrow().iter() {
            candidate.view(id)?;
            candidate.title(id)?;
            candidate.style(&candidate.theme(id)?)?;
        }
        let _ = candidate.subscription()?;
        candidate
            .pending
            .splice(0..0, std::mem::take(&mut self.pending));
        *self = candidate;
        Ok(())
    }

    fn check_reload(&mut self) -> mlua::Result<Change> {
        let change = self.reload_modules()?;
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
        eprintln!("riced: reloaded {}", self.path.display());
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
        let previous_mounts = self.mounts.borrow().clone();
        let previous_owners = self.windows_of.borrow().clone();
        let previous_windows = self.windows.borrow().clone();
        let pending_count = self.pending.len();
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
                affected = self.layers.iter().map(|layer| layer.window).collect();
            }
            for layer in &self.layers {
                self.view(layer.window)?;
            }
            for &id in &affected {
                self.view(id)?;
            }
            Ok(())
        })();
        if let Err(error) = result {
            self.modules.borrow_mut().restore(&self.lua, snapshot)?;
            self.layers = previous_layers;
            *self.mounts.borrow_mut() = previous_mounts;
            *self.windows_of.borrow_mut() = previous_owners;
            *self.windows.borrow_mut() = previous_windows;
            self.pending.truncate(pending_count);
            return Err(error);
        }
        if !affected.is_empty() {
            eprintln!("riced: reloaded modules {}", changed.join(", "));
        }
        Ok(Change::Windows(affected))
    }

    fn title(&self, id: window::Id) -> mlua::Result<Option<String>> {
        match self.call("title", self.window(id)?)? {
            Value::Nil => Ok(None),
            Value::String(s) => Ok(Some(s.to_str()?.to_owned())),
            _ => Err(mlua::Error::runtime("title must return a string or nil")),
        }
    }

    pub fn run(mut self) -> iced_exwlshell::Result {
        checked(self.prepare_layers());
        let mut settings = checked(self.settings());
        if !self.layers.is_empty() {
            settings.layer_settings.start_mode = StartMode::Background;
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
                    match app.check_reload() {
                        Ok(change) => return app.reload_tasks(change),
                        Err(error) => {
                            eprintln!("riced: reload rejected; keeping current app: {error}")
                        }
                    }
                } else if matches!(message, Message::PollPlugins) {
                    return if checked(app.poll_plugins()) {
                        Task::done(Message::Reloaded)
                    } else {
                        Task::none()
                    };
                } else {
                    checked(app.update(message));
                }
                Task::none()
            },
            App::render,
        )
        .title(|app: &Rc<RefCell<App>>, id| checked(app.borrow().title(id)))
        .subscription(|app| {
            Subscription::batch([
                checked(app.borrow().subscription()),
                iced::time::every(Duration::from_millis(250)).map(|_| Message::CheckReload),
                iced::time::every(Duration::from_millis(100)).map(|_| Message::PollPlugins),
            ])
        })
        .theme(|app: &Rc<RefCell<App>>, id| checked(app.borrow().theme(id)))
        .style(|app, theme| checked(app.borrow().style(theme)))
        .redraw_scope(move |message| checked(redraw.borrow().redraw(message)))
        .settings(settings)
        .run()
    }

    fn render(app: &Rc<RefCell<App>>, id: window::Id) -> Element<'_, Message> {
        checked(app.borrow().view(id))
    }
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
        app.update(Message::Tick).unwrap();
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
    fn invalid_saves_keep_current_app_and_recover_on_next_save() {
        let file = TestShell::new();
        let mut app = load(RELOAD_SHELL, &file.0).unwrap();
        let id = window::Id::unique();
        app.view(id).unwrap();
        app.update(Message::Tick).unwrap();

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
        app.update(event).unwrap();
        assert_eq!(app.state.get::<u32>("count").unwrap(), 0);
        app.update(Message::Lua(app.generation, None, Value::Nil))
            .unwrap();
        assert_eq!(app.state.get::<u32>("count").unwrap(), 1);
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
        app.update(Message::Tick).unwrap();
        app.view(app.layers[1].window).unwrap();
        assert_eq!(app.state.get::<u32>("count").unwrap(), 1);
        let id = app.layers[0].window;
        let action = ExwlShellCustomActionWithId::new(
            None,
            ExwlShellCustomAction::NewLayerShell {
                id,
                settings: app.layers[0].settings.clone(),
            },
        );
        let converted = ExwlShellCustomActionWithId::try_from(Message::Surface(action)).unwrap();
        assert!(
            matches!(converted.1, ExwlShellCustomAction::NewLayerShell {id: actual, ..} if actual == id)
        );
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
                .any(|action| matches!(action.1, ExwlShellCustomAction::LayerChange(_)))
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
        assert_eq!(app.mounts.borrow()["left"].revision, 0);
        assert_eq!(app.windows_of.borrow()["left"], [id]);
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
        assert_eq!(app.mounts.borrow()["left"].revision, 1);
        assert_eq!(app.mounts.borrow()["right"].revision, 0);
        assert!(matches!(app.check_reload().unwrap(), Change::None));
    }

    #[test]
    fn stale_component_events_are_ignored_after_its_module_reloads() {
        let profile = component_profile(&versioned("v1"));
        let mut app = load(COMPONENT_SHELL, &profile.shell()).unwrap();
        let id = window::Id::unique();
        app.view(id).unwrap();

        let old = Message::Lua(
            app.generation,
            Some(("left".into(), 0)),
            Value::String(app.lua.create_string("click").unwrap()),
        );
        profile.write("left.lua", &versioned("v2"));
        app.check_reload().unwrap();
        app.view(id).unwrap();

        app.update(old).unwrap();
        assert_eq!(app.state.get::<u32>("count").unwrap(), 0);
        app.update(Message::Lua(
            app.generation,
            Some(("left".into(), 1)),
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
        assert_eq!(app.mounts.borrow()["left"].revision, 0);
        assert!(matches!(app.check_reload().unwrap(), Change::None));
        app.view(id).unwrap();
        assert_eq!(seen(&app, "seen_left"), "v1:0");

        profile.write("left.lua", &versioned("v3"));
        assert!(matches!(app.check_reload().unwrap(), Change::Windows(_)));
        app.view(id).unwrap();
        assert_eq!(seen(&app, "seen_left"), "v3:0");
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
                .contains("duplicate layer id")
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
        app.update(Message::Tick).unwrap();
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
            && matches!(action.1, ExwlShellCustomAction::LayoutChange { .. })));
        assert!(
            app.pending
                .iter()
                .any(|action| matches!(action.1, ExwlShellCustomAction::ExclusiveZoneChange(64)))
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
}
