//! Session D-Bus extension: StatusNotifier tray items, DBusMenu trees, and a notification server.
//!
//! The extension owns `org.kde.StatusNotifierWatcher` when no other watcher is
//! running; otherwise it registers as a host with the existing watcher. It owns
//! `org.freedesktop.Notifications` when no other notification daemon is running.

use std::{
    collections::{BTreeMap, HashMap, HashSet},
    error::Error,
    sync::{Arc, Mutex, mpsc},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use riced_extension_sdk::{Plugin, bounded_array, log, protocol::MAX_LINE};
use serde_json::{Value, json};
type ItemInfo = (String, String, Option<String>, String, Option<String>, bool);

use zbus::{
    blocking::{Connection, Proxy},
    fdo::RequestNameFlags,
    interface,
    message::Header,
    zvariant::{OwnedObjectPath, OwnedValue, Value as VariantValue},
};

const WATCHER: &str = "org.kde.StatusNotifierWatcher";
const WATCHER_PATH: &str = "/StatusNotifierWatcher";
const WATCHER_IFACE: &str = "org.kde.StatusNotifierWatcher";
const ITEM_IFACE: &str = "org.kde.StatusNotifierItem";
const MENU_IFACE: &str = "com.canonical.dbusmenu";
const NOTIFICATIONS: &str = "org.freedesktop.Notifications";
const NOTIFICATIONS_PATH: &str = "/org/freedesktop/Notifications";
const METHOD_TIMEOUT: Duration = Duration::from_secs(1);
const MAX_TEXT: usize = 1024;
const MAX_NAME: usize = 256;
const MAX_ACTIONS: usize = 16;
const MAX_NOTIFICATIONS: usize = 64;
const MAX_MENU_DEPTH: usize = 8;
const MAX_MENU_ITEMS: usize = 128;
const MAX_MENU_TEXT: usize = 128;

fn main() -> std::io::Result<()> {
    let model = Arc::new(Mutex::new(Model::default()));

    let activate = Arc::clone(&model);
    let secondary = Arc::clone(&model);
    let menu = Arc::clone(&model);
    let open_menu = Arc::clone(&model);
    let close_menu = Arc::clone(&model);
    let menu_event = Arc::clone(&model);
    let close = Arc::clone(&model);
    let invoke = Arc::clone(&model);

    Plugin::new("dbus")
        .state([
            "connected",
            "capabilities",
            "tray",
            "tray_menu",
            "notifications",
        ])
        .method("activate_tray", false, move |args| {
            tray_call(&activate, args, "Activate")
        })
        .method("secondary_activate_tray", false, move |args| {
            tray_call(&secondary, args, "SecondaryActivate")
        })
        .method("context_menu_tray", false, move |args| {
            tray_call(&menu, args, "ContextMenu")
        })
        .method("open_tray_menu", false, move |args| {
            open_tray_menu(&open_menu, args)
        })
        .method("close_tray_menu", false, move |_| {
            close_tray_menu(&close_menu)
        })
        .method("tray_menu_event", false, move |args| {
            tray_menu_event(&menu_event, args)
        })
        .method("close_notification", false, move |args| {
            let id = argument(args, "notification")?;
            close_notification(&close, id)?;
            Ok(Value::Null)
        })
        .method("invoke_notification_action", false, move |args| {
            let id = argument(args, "notification")?;
            let action = args
                .get(1)
                .and_then(Value::as_str)
                .filter(|action| !action.is_empty())
                .ok_or_else(|| "missing notification action".to_owned())?;
            invoke_notification_action(&invoke, id, action)?;
            Ok(Value::Null)
        })
        .serve(move |context| run(context, model))
}

fn argument(args: &Value, what: &str) -> Result<u32, String> {
    args.get(0)
        .and_then(Value::as_u64)
        .and_then(|value| u32::try_from(value).ok())
        .ok_or_else(|| format!("missing {what} id"))
}

fn clamp(mut value: String, max: usize) -> String {
    if let Some((index, _)) = value.char_indices().nth(max) {
        value.truncate(index);
    }
    value
}

/// StatusNotifier items often use an `Id` like `vesktop_status_icon_1` while the
/// installed desktop icon is just `vesktop`. Strip the Electron suffix so an
/// item without `IconName` still resolves through the icon theme.
fn fallback_icon_name(id: &str) -> String {
    let name = id
        .split_once("_status_icon")
        .map_or(id, |(name, _)| name)
        .trim_end_matches(['_', '-']);
    if name.is_empty() {
        id.to_owned()
    } else {
        name.to_ascii_lowercase()
    }
}

fn tray_call(model: &Arc<Mutex<Model>>, args: &Value, method: &str) -> Result<Value, String> {
    let id = args
        .get(0)
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())
        .ok_or_else(|| "missing tray item id".to_owned())?;
    let (connection, item) = {
        let model = model.lock().unwrap();
        let connection = model
            .connection
            .clone()
            .ok_or_else(|| "session bus is not connected".to_owned())?;
        let item = model
            .tray
            .get(id)
            .cloned()
            .ok_or_else(|| format!("unknown tray item: {id}"))?;
        (connection, item)
    };
    let proxy = Proxy::new(
        &connection,
        item.bus.as_str(),
        item.path.as_str(),
        ITEM_IFACE,
    )
    .map_err(|error| error.to_string())?;
    proxy
        .call_method(method, &(0i32, 0i32))
        .map_err(|error| error.to_string())?;
    Ok(Value::Null)
}

fn lookup_tray(model: &Arc<Mutex<Model>>, id: &str) -> Result<(Connection, TrayItem), String> {
    let model = model.lock().unwrap();
    let connection = model
        .connection
        .clone()
        .ok_or_else(|| "session bus is not connected".to_owned())?;
    let item = model
        .tray
        .get(id)
        .cloned()
        .ok_or_else(|| format!("unknown tray item: {id}"))?;
    Ok((connection, item))
}

fn open_tray_menu(model: &Arc<Mutex<Model>>, args: &Value) -> Result<Value, String> {
    let id = args
        .get(0)
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())
        .ok_or_else(|| "missing tray item id".to_owned())?;
    let (connection, item) = lookup_tray(model, id)?;
    let (revision, items, ids) = fetch_menu(&connection, &item, true)?;
    let mut model = model.lock().unwrap();
    model.tray_menu = Some(TrayMenu {
        id: id.to_owned(),
        revision,
        items,
        ids,
        error: None,
    });
    model.dirty = true;
    Ok(Value::Null)
}

fn close_tray_menu(model: &Arc<Mutex<Model>>) -> Result<Value, String> {
    let mut model = model.lock().unwrap();
    if model.tray_menu.take().is_some() {
        model.dirty = true;
    }
    Ok(Value::Null)
}

fn tray_menu_event(model: &Arc<Mutex<Model>>, args: &Value) -> Result<Value, String> {
    let id = args
        .get(0)
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())
        .ok_or_else(|| "missing tray item id".to_owned())?;
    let menu_id = args
        .get(1)
        .and_then(Value::as_i64)
        .and_then(|id| i32::try_from(id).ok())
        .ok_or_else(|| "missing tray menu item id".to_owned())?;
    let event = args
        .get(2)
        .and_then(Value::as_str)
        .filter(|event| matches!(*event, "clicked" | "opened" | "closed" | "hovered"))
        .ok_or_else(|| "unknown tray menu event".to_owned())?;
    let (connection, item) = lookup_tray(model, id)?;
    {
        let model = model.lock().unwrap();
        let valid = model
            .tray_menu
            .as_ref()
            .is_some_and(|menu| menu.id == id && menu.ids.contains(&menu_id));
        if !valid {
            return Err("unknown tray menu item".to_owned());
        }
    }
    let path = item
        .menu
        .as_deref()
        .ok_or_else(|| "tray item has no menu".to_owned())?;
    let proxy = Proxy::new(&connection, item.bus.as_str(), path, MENU_IFACE)
        .map_err(|error| error.to_string())?;
    if event == "opened" {
        let _: Result<(bool,), _> = proxy.call("AboutToShow", &(menu_id,));
    }
    proxy
        .call::<_, _, ()>("Event", &(menu_id, event, VariantValue::from(0i32), 0u32))
        .map_err(|error| error.to_string())?;
    Ok(Value::Null)
}

type RawLayout = (i32, HashMap<String, OwnedValue>, Vec<OwnedValue>);

fn fetch_menu(
    connection: &Connection,
    item: &TrayItem,
    about_to_show: bool,
) -> Result<(u32, Value, HashSet<i32>), String> {
    let path = item
        .menu
        .as_deref()
        .ok_or_else(|| "tray item has no menu".to_owned())?;
    let proxy = Proxy::new(connection, item.bus.as_str(), path, MENU_IFACE)
        .map_err(|error| error.to_string())?;
    if about_to_show {
        let _: Result<(bool,), _> = proxy.call("AboutToShow", &(0i32,));
    }
    let properties = vec![
        "type",
        "label",
        "enabled",
        "visible",
        "children-display",
        "icon-name",
        "toggle-type",
        "toggle-state",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect::<Vec<_>>();
    let layout =
        proxy.call::<_, _, (u32, RawLayout)>("GetLayout", &(0i32, -1i32, properties.clone()));
    let (revision, layout) = match layout {
        Ok(layout) => layout,
        Err(_) => proxy
            .call::<_, _, (u32, RawLayout)>("GetLayout", &(0i32, 1i32, properties))
            .map_err(|error| error.to_string())?,
    };
    let mut count = 0;
    let mut ids = HashSet::new();
    let root = parse_layout(layout, 0, &mut count, &mut ids)
        .ok_or_else(|| "tray menu is empty or exceeds limits".to_owned())?;
    let items = root
        .get("children")
        .cloned()
        .unwrap_or_else(|| Value::Array(Vec::new()));
    Ok((revision, items, ids))
}

fn parse_layout(
    raw: RawLayout,
    depth: usize,
    count: &mut usize,
    ids: &mut HashSet<i32>,
) -> Option<Value> {
    if depth > MAX_MENU_DEPTH || *count >= MAX_MENU_ITEMS {
        return None;
    }
    *count += 1;
    let (id, properties, children) = raw;
    let text = |key: &str| {
        properties
            .get(key)
            .and_then(|value| <&str>::try_from(value).ok())
            .unwrap_or("")
    };
    let flag = |key: &str, default: bool| {
        properties
            .get(key)
            .and_then(|value| bool::try_from(value).ok())
            .unwrap_or(default)
    };
    let number = |key: &str| {
        properties
            .get(key)
            .and_then(|value| i32::try_from(value).ok())
    };
    if !flag("visible", true) {
        return None;
    }
    ids.insert(id);
    let kind = match text("type") {
        "" => "standard",
        kind => kind,
    };
    let label = clamp(text("label").to_owned(), MAX_MENU_TEXT);
    let icon = clamp(text("icon-name").to_owned(), MAX_MENU_TEXT);
    let toggle = text("toggle-type");
    let checked = number("toggle-state").unwrap_or(0) == 1;
    let mut items = Vec::new();
    for child in children {
        let Ok(child) = RawLayout::try_from(child) else {
            continue;
        };
        if let Some(value) = parse_layout(child, depth + 1, count, ids) {
            items.push(value);
        }
    }
    let submenu = !items.is_empty() || text("children-display") == "submenu";
    Some(json!({
        "id": id,
        "label": label,
        "enabled": flag("enabled", true),
        "type": kind,
        "icon": if icon.is_empty() { Value::Null } else { json!(icon) },
        "toggle": if toggle.is_empty() { Value::Null } else { json!(toggle) },
        "checked": checked,
        "submenu": submenu,
        "children": items,
    }))
}

fn refresh_tray_menu(connection: &Connection, model: &Arc<Mutex<Model>>) {
    let active = {
        let model = model.lock().unwrap();
        model.tray_menu.as_ref().map(|menu| menu.id.clone())
    };
    let Some(id) = active else {
        return;
    };
    let item = {
        let model = model.lock().unwrap();
        model.tray.get(&id).cloned()
    };
    let Some(item) = item else {
        let mut model = model.lock().unwrap();
        if model.tray_menu.as_ref().is_some_and(|menu| menu.id == id) {
            model.tray_menu = None;
            model.dirty = true;
        }
        return;
    };
    match fetch_menu(connection, &item, false) {
        Ok((revision, items, ids)) => {
            let mut model = model.lock().unwrap();
            let changed = model.tray_menu.as_ref().is_none_or(|menu| {
                menu.revision != revision || menu.items != items || menu.error.is_some()
            });
            if changed {
                model.tray_menu = Some(TrayMenu {
                    id,
                    revision,
                    items,
                    ids,
                    error: None,
                });
                model.dirty = true;
            }
        }
        Err(error) => {
            let mut model = model.lock().unwrap();
            if let Some(menu) = &mut model.tray_menu
                && menu.id == id
                && menu.error.as_deref() != Some(error.as_str())
            {
                menu.error = Some(error);
                model.dirty = true;
            }
        }
    }
}

fn close_notification(model: &Arc<Mutex<Model>>, id: u32) -> Result<(), String> {
    let mut model = model.lock().unwrap();
    if model.notifications.remove(&id).is_some() {
        model.signals.push(Signal::Closed(id, 2));
        model.dirty = true;
    }
    Ok(())
}

fn invoke_notification_action(
    model: &Arc<Mutex<Model>>,
    id: u32,
    action: &str,
) -> Result<(), String> {
    let mut model = model.lock().unwrap();
    if let Some(notification) = model.notifications.get(&id)
        && !notification.actions.iter().any(|(key, _)| key == action)
    {
        return Err(format!("unknown notification action: {action}"));
    }
    if model.notifications.remove(&id).is_some() {
        model.signals.push(Signal::Action(id, action.to_owned()));
        model.signals.push(Signal::Closed(id, 2));
        model.dirty = true;
    }
    Ok(())
}

#[derive(Default)]
struct Model {
    connection: Option<Connection>,
    tray: BTreeMap<String, TrayItem>,
    tray_menu: Option<TrayMenu>,
    notifications: BTreeMap<u32, Notification>,
    next_notification: u32,
    own_watcher: bool,
    tray_available: bool,
    notifications_available: bool,
    dirty: bool,
    signals: Vec<Signal>,
}

#[derive(Clone, Default)]
struct TrayItem {
    bus: String,
    path: String,
    title: String,
    icon: String,
    icon_theme_path: Option<String>,
    status: String,
    menu: Option<String>,
    item_is_menu: bool,
    failures: u8,
}

#[derive(Clone)]
struct TrayMenu {
    id: String,
    revision: u32,
    items: Value,
    ids: HashSet<i32>,
    error: Option<String>,
}

#[derive(Clone)]
struct Notification {
    id: u32,
    app_name: String,
    app_icon: String,
    summary: String,
    body: String,
    actions: Vec<(String, String)>,
    timeout: u32,
    created: u64,
    expires: Option<Instant>,
}

#[derive(Clone)]
enum Signal {
    Action(u32, String),
    Closed(u32, u32),
}

fn run(context: riced_extension_sdk::Context, model: Arc<Mutex<Model>>) {
    let mut last = empty(false);
    context.publish(last.clone());
    loop {
        if let Err(error) = serve(&context, &model, &mut last) {
            log::warn!(target: "dbus.connection", "{error}");
        }
        {
            let mut model = model.lock().unwrap();
            model.connection = None;
            model.tray.clear();
            model.tray_menu = None;
            model.notifications.clear();
            model.own_watcher = false;
            model.tray_available = false;
            model.notifications_available = false;
            model.signals.clear();
        }
        if let Some(object) = last.as_object_mut() {
            object.insert("connected".into(), json!(false));
        }
        context.publish(last.clone());
        thread::sleep(Duration::from_secs(1));
    }
}

fn serve(
    context: &riced_extension_sdk::Context,
    model: &Arc<Mutex<Model>>,
    last: &mut Value,
) -> Result<(), Box<dyn Error>> {
    let connection = zbus::blocking::connection::Builder::session()?
        .method_timeout(METHOD_TIMEOUT)
        .build()?;
    {
        let mut model = model.lock().unwrap();
        model.connection = Some(connection.clone());
        model.tray.clear();
        model.tray_menu = None;
        model.notifications.clear();
        model.dirty = true;
    }
    let (tray, own_watcher) = setup_watcher(&connection, model)?;
    let notifications = setup_notifications(&connection, model)?;
    {
        let mut model = model.lock().unwrap();
        model.tray_available = tray;
        model.own_watcher = own_watcher;
        model.notifications_available = notifications;
        model.dirty = true;
    }

    thread::scope(|scope| {
        let (stop, stopped) = mpsc::channel::<()>();
        let connection = &connection;
        // Remote tray clients can time out without delaying notification delivery.
        scope.spawn(move || {
            loop {
                if !own_watcher {
                    let _ = sync_external_watcher(connection, model);
                }
                refresh_tray(connection, model);
                refresh_tray_menu(connection, model);
                if stopped.recv_timeout(Duration::from_millis(500))
                    != Err(mpsc::RecvTimeoutError::Timeout)
                {
                    break;
                }
            }
        });
        while !connection.is_closed() {
            expire_notifications(model);
            flush_signals(connection, model);
            publish(context, model, last);
            thread::sleep(Duration::from_millis(100));
        }
        drop(stop);
    });
    Err("session bus disconnected".into())
}

fn setup_watcher(
    connection: &Connection,
    model: &Arc<Mutex<Model>>,
) -> Result<(bool, bool), Box<dyn Error>> {
    if name_has_owner(connection, WATCHER)? {
        register_host(connection);
        sync_external_watcher(connection, model)?;
        return Ok((true, false));
    }

    connection.object_server().at(
        WATCHER_PATH,
        Watcher {
            model: Arc::clone(model),
        },
    )?;
    let flags = RequestNameFlags::DoNotQueue | RequestNameFlags::AllowReplacement;
    let reply = connection.request_name_with_flags(WATCHER, flags)?;
    if matches!(
        reply,
        zbus::fdo::RequestNameReply::PrimaryOwner | zbus::fdo::RequestNameReply::AlreadyOwner
    ) {
        Ok((true, true))
    } else {
        register_host(connection);
        sync_external_watcher(connection, model)?;
        Ok((true, false))
    }
}

fn setup_notifications(
    connection: &Connection,
    model: &Arc<Mutex<Model>>,
) -> Result<bool, Box<dyn Error>> {
    if name_has_owner(connection, NOTIFICATIONS)? {
        return Ok(false);
    }
    connection.object_server().at(
        NOTIFICATIONS_PATH,
        NotificationsServer {
            model: Arc::clone(model),
        },
    )?;
    let reply =
        connection.request_name_with_flags(NOTIFICATIONS, RequestNameFlags::DoNotQueue.into())?;
    Ok(matches!(
        reply,
        zbus::fdo::RequestNameReply::PrimaryOwner | zbus::fdo::RequestNameReply::AlreadyOwner
    ))
}

fn name_has_owner(connection: &Connection, name: &str) -> Result<bool, Box<dyn Error>> {
    let reply = connection.call_method(
        Some("org.freedesktop.DBus"),
        "/org/freedesktop/DBus",
        Some("org.freedesktop.DBus"),
        "NameHasOwner",
        &(name),
    )?;
    Ok(reply.body().deserialize()?)
}

fn register_host(connection: &Connection) {
    let Some(name) = connection.unique_name() else {
        return;
    };
    let _ = connection.call_method(
        Some(WATCHER),
        WATCHER_PATH,
        Some(WATCHER_IFACE),
        "RegisterStatusNotifierHost",
        &(name.as_str()),
    );
}

fn sync_external_watcher(
    connection: &Connection,
    model: &Arc<Mutex<Model>>,
) -> Result<(), Box<dyn Error>> {
    let proxy = Proxy::new(connection, WATCHER, WATCHER_PATH, WATCHER_IFACE)?;
    let services: Vec<String> = proxy.get_property("RegisteredStatusNotifierItems")?;
    let mut next = BTreeMap::new();
    for service in services {
        let (bus, path) = parse_registration(&service, None);
        if bus.is_empty() {
            continue;
        }
        let key = format!("{bus}{path}");
        next.insert(
            key,
            TrayItem {
                bus,
                path,
                ..TrayItem::default()
            },
        );
    }
    let mut model = model.lock().unwrap();
    let same =
        model.tray.len() == next.len() && next.keys().all(|key| model.tray.contains_key(key));
    if !same {
        model.tray.retain(|key, _| next.contains_key(key));
        for (key, item) in next {
            model.tray.entry(key).or_insert(item);
        }
        model.dirty = true;
    }
    Ok(())
}

fn refresh_tray(connection: &Connection, model: &Arc<Mutex<Model>>) {
    let items = {
        let model = model.lock().unwrap();
        model
            .tray
            .iter()
            .map(|(key, item)| (key.clone(), item.clone()))
            .collect::<Vec<_>>()
    };
    for (key, item) in items {
        match item_properties(connection, &item) {
            Ok((title, icon, icon_theme_path, status, menu, item_is_menu)) => {
                let mut model = model.lock().unwrap();
                let mut changed = false;
                if let Some(current) = model.tray.get_mut(&key) {
                    if current.title != title
                        || current.icon != icon
                        || current.icon_theme_path != icon_theme_path
                        || current.status != status
                        || current.menu != menu
                        || current.item_is_menu != item_is_menu
                    {
                        current.title = title;
                        current.icon = icon;
                        current.icon_theme_path = icon_theme_path;
                        current.status = status;
                        current.menu = menu;
                        current.item_is_menu = item_is_menu;
                        changed = true;
                    }
                    current.failures = 0;
                }
                if changed {
                    model.dirty = true;
                }
            }
            Err(_) => {
                let removed = {
                    let mut model = model.lock().unwrap();
                    let mut removed = false;
                    if let Some(current) = model.tray.get_mut(&key) {
                        current.failures += 1;
                        if current.failures > 2 {
                            model.tray.remove(&key);
                            if model.tray_menu.as_ref().is_some_and(|menu| menu.id == key) {
                                model.tray_menu = None;
                            }
                            model.dirty = true;
                            removed = model.own_watcher;
                        }
                    }
                    removed
                };
                if removed {
                    let _ = connection.emit_signal(
                        None::<&str>,
                        WATCHER_PATH,
                        WATCHER_IFACE,
                        "StatusNotifierItemUnregistered",
                        &(key.as_str()),
                    );
                }
            }
        }
    }
}

fn item_properties(connection: &Connection, item: &TrayItem) -> Result<ItemInfo, Box<dyn Error>> {
    let proxy = Proxy::new(
        connection,
        item.bus.as_str(),
        item.path.as_str(),
        ITEM_IFACE,
    )?;
    let id = clamp(proxy.get_property::<String>("Id")?, MAX_NAME);
    let title: String = proxy.get_property("Title").unwrap_or_else(|_| id.clone());
    let icon: String = proxy.get_property("IconName").unwrap_or_default();
    let icon = if icon.trim().is_empty() {
        fallback_icon_name(&id)
    } else {
        icon
    };
    let icon_theme_path = proxy
        .get_property::<String>("IconThemePath")
        .ok()
        .filter(|path| {
            path.len() <= 4096 && !path.contains('\0') && std::path::Path::new(path).is_absolute()
        });
    let status: String = proxy
        .get_property("Status")
        .unwrap_or_else(|_| "Passive".to_owned());
    let menu = proxy
        .get_property::<OwnedObjectPath>("Menu")
        .ok()
        .map(|path| path.to_string())
        .filter(|path| path != "/");
    let item_is_menu = proxy.get_property("ItemIsMenu").unwrap_or(false);
    Ok((
        clamp(if title.is_empty() { id } else { title }, MAX_NAME),
        clamp(icon, MAX_NAME),
        icon_theme_path,
        clamp(status, MAX_NAME),
        menu,
        item_is_menu,
    ))
}

fn expire_notifications(model: &Arc<Mutex<Model>>) {
    let now = Instant::now();
    let mut model = model.lock().unwrap();
    let expired = model
        .notifications
        .iter()
        .filter(|(_, notification)| notification.expires.is_some_and(|at| at <= now))
        .map(|(id, _)| *id)
        .collect::<Vec<_>>();
    for id in expired {
        model.notifications.remove(&id);
        model.signals.push(Signal::Closed(id, 1));
        model.dirty = true;
    }
}

fn flush_signals(connection: &Connection, model: &Arc<Mutex<Model>>) {
    let signals = {
        let mut model = model.lock().unwrap();
        std::mem::take(&mut model.signals)
    };
    for signal in signals {
        let result = match signal {
            Signal::Action(id, action) => connection.emit_signal(
                None::<&str>,
                NOTIFICATIONS_PATH,
                NOTIFICATIONS,
                "ActionInvoked",
                &(id, action),
            ),
            Signal::Closed(id, reason) => connection.emit_signal(
                None::<&str>,
                NOTIFICATIONS_PATH,
                NOTIFICATIONS,
                "NotificationClosed",
                &(id, reason),
            ),
        };
        if let Err(error) = result {
            log::error!(target: "dbus.signal", "signal failed: {error}");
        }
    }
}

fn publish(context: &riced_extension_sdk::Context, model: &Arc<Mutex<Model>>, last: &mut Value) {
    let store = {
        let mut model = model.lock().unwrap();
        if !model.dirty {
            return;
        }
        model.dirty = false;
        snapshot(&model)
    };
    context.publish(store.clone());
    *last = store;
}

fn snapshot(model: &Model) -> Value {
    let tray = model
        .tray
        .iter()
        .map(|(id, item)| {
            json!({
                "id": id,
                "title": clamp(item.title.clone(), MAX_NAME),
                "icon": clamp(item.icon.clone(), MAX_NAME),
                "icon_theme_path": item.icon_theme_path,
                "status": clamp(item.status.clone(), MAX_NAME),
                "menu": item.menu.clone(),
                "item_is_menu": item.item_is_menu,
            })
        })
        .collect::<Vec<_>>();
    let notifications = model
        .notifications
        .values()
        .map(|notification| {
            json!({
                "id": notification.id,
                "app_name": notification.app_name,
                "app_icon": notification.app_icon,
                "summary": notification.summary,
                "body": notification.body,
                "actions": notification.actions.iter().map(|(key, label)| {
                    json!({"key": key, "label": label})
                }).collect::<Vec<_>>(),
                "timeout": notification.timeout,
                "timestamp": notification.created,
            })
        })
        .collect::<Vec<_>>();
    json!({
        "connected": true,
        "capabilities": {
            "tray": model.tray_available,
            "tray_host": model.own_watcher,
            "notifications": model.notifications_available,
        },
        "tray": bounded_array(tray, MAX_LINE / 4),
        "tray_menu": model.tray_menu.as_ref().map(|menu| json!({
            "id": menu.id,
            "revision": menu.revision,
            "items": menu.items,
            "error": menu.error,
        })),
        "notifications": bounded_array(notifications, MAX_LINE / 4),
    })
}

fn empty(connected: bool) -> Value {
    json!({
        "connected": connected,
        "capabilities": {
            "tray": false,
            "tray_host": false,
            "notifications": false,
        },
        "tray": [],
        "tray_menu": Value::Null,
        "notifications": [],
    })
}

fn parse_registration(service: &str, sender: Option<&str>) -> (String, String) {
    if service.starts_with('/') {
        return (sender.unwrap_or_default().to_owned(), service.to_owned());
    }
    if let Some((bus, path)) = service.split_once('/')
        && !bus.is_empty()
        && !path.is_empty()
    {
        let path = if path.starts_with('/') {
            path.to_owned()
        } else {
            format!("/{path}")
        };
        return (bus.to_owned(), path);
    }
    (service.to_owned(), "/StatusNotifierItem".to_owned())
}

struct Watcher {
    model: Arc<Mutex<Model>>,
}

#[interface(name = "org.kde.StatusNotifierWatcher")]
impl Watcher {
    async fn register_status_notifier_item(
        &self,
        service: String,
        #[zbus(header)] header: Header<'_>,
        #[zbus(signal_emitter)] emitter: zbus::object_server::SignalEmitter<'_>,
    ) -> zbus::fdo::Result<()> {
        let sender = header.sender().map(|sender| sender.to_string());
        let (bus, path) = parse_registration(&service, sender.as_deref());
        if bus.is_empty() || !path.starts_with('/') {
            return Err(zbus::fdo::Error::InvalidArgs("invalid tray item".into()));
        }
        let key = format!("{bus}{path}");
        {
            let mut model = self.model.lock().unwrap();
            model.tray.insert(
                key.clone(),
                TrayItem {
                    bus,
                    path,
                    ..TrayItem::default()
                },
            );
            model.dirty = true;
        }
        Self::status_notifier_item_registered(&emitter, &key).await?;
        Ok(())
    }

    async fn register_status_notifier_host(
        &self,
        _service: String,
        #[zbus(signal_emitter)] emitter: zbus::object_server::SignalEmitter<'_>,
    ) -> zbus::fdo::Result<()> {
        Self::status_notifier_host_registered(&emitter).await?;
        Ok(())
    }

    #[zbus(signal)]
    async fn status_notifier_item_registered(
        emitter: &zbus::object_server::SignalEmitter<'_>,
        service: &str,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn status_notifier_item_unregistered(
        emitter: &zbus::object_server::SignalEmitter<'_>,
        service: &str,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn status_notifier_host_registered(
        emitter: &zbus::object_server::SignalEmitter<'_>,
    ) -> zbus::Result<()>;

    #[zbus(property)]
    fn registered_status_notifier_items(&self) -> Vec<String> {
        self.model.lock().unwrap().tray.keys().cloned().collect()
    }

    #[zbus(property)]
    fn is_status_notifier_host_registered(&self) -> bool {
        true
    }

    #[zbus(property)]
    fn protocol_version(&self) -> i32 {
        0
    }
}

struct NotificationsServer {
    model: Arc<Mutex<Model>>,
}

#[interface(name = "org.freedesktop.Notifications")]
impl NotificationsServer {
    #[allow(clippy::too_many_arguments)]
    fn notify(
        &self,
        app_name: String,
        replaces_id: u32,
        app_icon: String,
        summary: String,
        body: String,
        actions: Vec<String>,
        _hints: HashMap<String, zbus::zvariant::OwnedValue>,
        expire_timeout: i32,
    ) -> u32 {
        let mut model = self.model.lock().unwrap();
        let id = if replaces_id != 0 && model.notifications.contains_key(&replaces_id) {
            replaces_id
        } else {
            loop {
                model.next_notification = model.next_notification.wrapping_add(1).max(1);
                if !model.notifications.contains_key(&model.next_notification) {
                    break;
                }
            }
            model.next_notification
        };
        let timeout = match expire_timeout {
            -1 => 5_000,
            value if value > 0 => value as u32,
            _ => 0,
        };
        let actions: Vec<(String, String)> = actions
            .chunks_exact(2)
            .take(MAX_ACTIONS)
            .map(|pair| (pair[0].clone(), clamp(pair[1].clone(), MAX_NAME)))
            .collect();
        if model.notifications.len() >= MAX_NOTIFICATIONS
            && !model.notifications.contains_key(&id)
            && let Some(oldest) = model
                .notifications
                .values()
                .min_by_key(|notification| notification.created)
                .map(|notification| notification.id)
        {
            model.notifications.remove(&oldest);
            model.signals.push(Signal::Closed(oldest, 4));
        }
        let created = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_millis() as u64)
            .unwrap_or(0);
        model.notifications.insert(
            id,
            Notification {
                id,
                app_name: clamp(app_name, MAX_NAME),
                app_icon: clamp(app_icon, MAX_NAME),
                summary: clamp(summary, MAX_TEXT),
                body: clamp(body, MAX_TEXT),
                actions,
                timeout,
                created,
                expires: (timeout > 0)
                    .then(|| Instant::now() + Duration::from_millis(timeout as u64)),
            },
        );
        model.dirty = true;
        id
    }

    fn close_notification(&self, id: u32) {
        let mut model = self.model.lock().unwrap();
        if model.notifications.remove(&id).is_some() {
            model.signals.push(Signal::Closed(id, 3));
            model.dirty = true;
        }
    }

    fn get_capabilities(&self) -> Vec<String> {
        vec![
            "body".into(),
            "actions".into(),
            "icon-static".into(),
            "persistence".into(),
        ]
    }

    fn get_server_information(&self) -> (String, String, String, String) {
        (
            "riced".into(),
            "riced".into(),
            env!("CARGO_PKG_VERSION").into(),
            "1.2".into(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_item_registrations() {
        assert_eq!(
            parse_registration("org.example.Item", None),
            (
                "org.example.Item".to_owned(),
                "/StatusNotifierItem".to_owned()
            )
        );
        assert_eq!(
            parse_registration(":1.42/StatusNotifierItem", None),
            (":1.42".to_owned(), "/StatusNotifierItem".to_owned())
        );
        assert_eq!(
            parse_registration("/Custom/Path", Some(":1.7")),
            (":1.7".to_owned(), "/Custom/Path".to_owned())
        );
    }

    #[test]
    fn icon_fallback_strips_status_icon_suffix() {
        assert_eq!(fallback_icon_name("vesktop_status_icon_1"), "vesktop");
        assert_eq!(fallback_icon_name("TelegramDesktop"), "telegramdesktop");
        assert_eq!(fallback_icon_name("steam"), "steam");
        assert_eq!(fallback_icon_name("_status_icon_1"), "_status_icon_1");
    }

    #[test]
    fn empty_store_is_valid() {
        let store = empty(false);
        assert_eq!(store["connected"], false);
        assert_eq!(store["tray"], json!([]));
        assert_eq!(store["tray_menu"], Value::Null);
    }

    #[test]
    fn large_notification_snapshots_fit_the_wire_limit() {
        let model = Arc::new(Mutex::new(Model::default()));
        let server = NotificationsServer {
            model: Arc::clone(&model),
        };
        for _ in 0..MAX_NOTIFICATIONS + 1 {
            server.notify(
                "app".into(),
                0,
                "icon".into(),
                "\0".repeat(MAX_TEXT),
                "🦀".repeat(MAX_TEXT),
                vec!["action".into(), "\0".repeat(MAX_NAME)],
                HashMap::new(),
                0,
            );
        }
        let model = model.lock().unwrap();
        assert_eq!(model.notifications.len(), MAX_NOTIFICATIONS);
        let store = snapshot(&model);
        assert!(!store["notifications"].as_array().unwrap().is_empty());
        let message = riced_extension_sdk::ToHost::State { store };
        assert!(serde_json::to_vec(&message).unwrap().len() < MAX_LINE);
    }

    #[test]
    fn notification_actions_and_close_reasons_are_validated() {
        let model = Arc::new(Mutex::new(Model::default()));
        let server = NotificationsServer {
            model: Arc::clone(&model),
        };
        let id = server.notify(
            "app".into(),
            0,
            "".into(),
            "summary".into(),
            "body".into(),
            vec!["open".into(), "Open".into()],
            HashMap::new(),
            0,
        );
        assert!(invoke_notification_action(&model, id, "unknown").is_err());
        expire_notifications(&model);
        assert!(model.lock().unwrap().notifications.contains_key(&id));
        server.close_notification(id);
        assert!(
            matches!(model.lock().unwrap().signals.as_slice(), [Signal::Closed(closed, 3)] if *closed == id)
        );
    }

    #[test]
    fn notifications_replace_expire_and_close() {
        let model = Arc::new(Mutex::new(Model::default()));
        let server = NotificationsServer {
            model: Arc::clone(&model),
        };
        let hints = HashMap::new();

        let id = server.notify(
            "app".into(),
            0,
            "icon".into(),
            "summary".into(),
            "body".into(),
            Vec::new(),
            hints.clone(),
            0,
        );
        assert_eq!(id, 1);
        let replacement = server.notify(
            "app".into(),
            id,
            "icon".into(),
            "new".into(),
            "body".into(),
            Vec::new(),
            hints.clone(),
            0,
        );
        assert_eq!(replacement, id);
        assert_eq!(model.lock().unwrap().notifications.len(), 1);

        model
            .lock()
            .unwrap()
            .notifications
            .get_mut(&id)
            .unwrap()
            .expires = Some(Instant::now() - Duration::from_millis(1));
        expire_notifications(&model);
        assert!(model.lock().unwrap().notifications.is_empty());
        assert!(matches!(
            model.lock().unwrap().signals.as_slice(),
            [Signal::Closed(closed, 1)] if *closed == id
        ));

        model.lock().unwrap().signals.clear();
        let second = server.notify(
            "app".into(),
            0,
            "icon".into(),
            "summary".into(),
            "body".into(),
            vec!["default".into(), "Open".into()],
            hints,
            0,
        );
        invoke_notification_action(&model, second, "default").unwrap();
        assert!(matches!(
            model.lock().unwrap().signals.as_slice(),
            [Signal::Action(action_id, action), Signal::Closed(closed, 2)]
                if *action_id == second && action == "default" && *closed == second
        ));
    }

    fn text(value: &str) -> OwnedValue {
        VariantValue::from(value).try_into().unwrap()
    }

    fn flag(value: bool) -> OwnedValue {
        VariantValue::from(value).try_into().unwrap()
    }

    fn number(value: i32) -> OwnedValue {
        VariantValue::from(value).try_into().unwrap()
    }

    fn layout(id: i32, properties: &[(&str, OwnedValue)], children: Vec<RawLayout>) -> RawLayout {
        let properties = properties
            .iter()
            .map(|(name, value)| ((*name).to_owned(), value.clone()))
            .collect();
        let children = children
            .into_iter()
            .map(|child| {
                let value: VariantValue<'static> = child.into();
                OwnedValue::try_from(value).unwrap()
            })
            .collect();
        (id, properties, children)
    }

    #[test]
    fn parses_nested_menu_items_and_skips_hidden_entries() {
        let child = layout(
            7,
            &[("label", text("Open")), ("icon-name", text("firefox"))],
            vec![],
        );
        let separator = layout(8, &[("type", text("separator"))], vec![]);
        let hidden = layout(
            9,
            &[("label", text("Hidden")), ("visible", flag(false))],
            vec![],
        );
        let toggle = layout(
            10,
            &[
                ("label", text("Enabled")),
                ("toggle-type", text("checkmark")),
                ("toggle-state", number(1)),
            ],
            vec![],
        );
        let root = layout(0, &[], vec![child, separator, hidden, toggle]);
        let mut count = 0;
        let mut ids = HashSet::new();
        let root = parse_layout(root, 0, &mut count, &mut ids).unwrap();
        let items = root["children"].as_array().unwrap();

        assert_eq!(items.len(), 3);
        assert_eq!(items[0]["label"], "Open");
        assert_eq!(items[0]["icon"], "firefox");
        assert_eq!(items[1]["type"], "separator");
        assert_eq!(items[2]["checked"], true);
        assert_eq!(items[2]["toggle"], "checkmark");
        assert!(!ids.contains(&9));
    }

    #[test]
    fn menu_parser_bounds_depth_siblings_and_text() {
        let mut child = layout(1, &[("label", text("leaf"))], vec![]);
        for id in 2..=MAX_MENU_DEPTH + 2 {
            child = layout(id as i32, &[], vec![child]);
        }
        let mut count = 0;
        let mut ids = HashSet::new();
        let items = parse_layout(child, 0, &mut count, &mut ids);
        assert!(items.is_some());
        assert!(count <= MAX_MENU_DEPTH + 1);

        let children = (1..=MAX_MENU_ITEMS + 10)
            .map(|id| {
                layout(
                    id as i32,
                    &[("label", text(&"x".repeat(MAX_MENU_TEXT + 20)))],
                    vec![],
                )
            })
            .collect();
        let root = layout(0, &[], children);
        let mut count = 0;
        let mut ids = HashSet::new();
        let root = parse_layout(root, 0, &mut count, &mut ids).unwrap();
        let items = root["children"].as_array().unwrap();
        assert_eq!(items.len(), MAX_MENU_ITEMS - 1);
        assert_eq!(
            items[0]["label"].as_str().unwrap().chars().count(),
            MAX_MENU_TEXT
        );
    }
}
