# riced-dbus API

`riced-dbus` provides StatusNotifier tray items, D-BusMenu trees, and a
`org.freedesktop.Notifications` server through the `dbus` plugin store.

- Binary: `riced-dbus`
- Lua proxy: `riced.plugin("dbus")` (also `iced.plugins("dbus")`)
- Store fields: `connected`, `capabilities`, `tray`, `tray_menu`,
  `notifications`
- Methods: `activate_tray`, `secondary_activate_tray`, `context_menu_tray`,
  `open_tray_menu`, `close_tray_menu`, `tray_menu_event`, `close_notification`,
  `invoke_notification_action`, `pick_file`
- Events: none

## Install

```sh
just extension-dbus
```

Or build directly into a profile:

```sh
riced extension build extensions/dbus -o <profile>/extensions/dbus
```

## Common plugin behavior

The proxy is live: `alive` is false and the store is empty until the process
connects. Reads are synchronous; each publish replaces the snapshot while the
proxy table identity stays the same. All methods are asynchronous and take an
optional callback as the last argument. On success the callback receives
`(nil, nil)`, except `pick_file`, which returns the selected paths. On failure it
receives `(nil, error)`. Without a callback, method failures are not surfaced to
Lua.

See the [riced API](../../API.md#plugins-and-extensions) for shared plugin and
extension semantics.

## Store

| Field           | Type / shape                                                                      |
| --------------- | --------------------------------------------------------------------------------- |
| `connected`     | boolean; session bus connection                                                   |
| `capabilities`  | `{ tray = boolean, tray_host = boolean, notifications = boolean }`                |
| `tray`          | array of `{ id, title, icon, icon_theme_path, status, menu, item_is_menu }`       |
| `tray_menu`     | active menu `{ id, revision, items, error }`, or nil                              |
| `notifications` | array of `{ id, app_name, app_icon, summary, body, actions, timeout, timestamp }` |

The extension owns `org.kde.StatusNotifierWatcher` when the name is free;
otherwise it registers as a host with the existing watcher. It owns
`org.freedesktop.Notifications` only if no other notification daemon does.

`capabilities.tray` means a StatusNotifier watcher is available.
`capabilities.tray_host` is true only when this extension owns the watcher; it
is false when it registered with another watcher.
`capabilities.notifications` is true only when this extension owns
`org.freedesktop.Notifications`. If another daemon already owns the name, the
extension does not receive notifications and the `notifications` array stays
empty.

## Tray items

| Field             | Meaning                                                                                                                |
| ----------------- | ---------------------------------------------------------------------------------------------------------------------- |
| `id`              | Opaque string (`bus + object path`), for example `:1.42/StatusNotifierItem`; pass to methods                           |
| `title`           | Item title, falling back to the item `Id`; clamped to 256 characters                                                   |
| `icon`            | Sender's `IconName`, or a normalized fallback derived from the item `Id` (for example `vesktop_status_icon_1` becomes `vesktop`); clamped to 256 characters. Resolve with `iced.icon_path`. |
| `icon_theme_path` | Sender's optional absolute `IconThemePath` directory, or nil; paths over 4096 bytes or containing NUL are ignored       |
| `status`          | Raw StatusNotifier status such as `Active`, `Passive`, or `NeedsAttention`, clamped to 256 characters; default `Passive` |
| `menu`            | D-Bus object path string or nil; `/` is treated as no menu                                                             |
| `item_is_menu`    | Whether activating the item should open the menu instead                                                               |

Tray properties are refreshed about every 500 ms. An item that fails to answer
three times is removed from the store.

Use `iced.icon_path(item.icon, { size = 24, path = item.icon_theme_path })` to
resolve tray icons with the current `iced.icon_theme`. Items that only send an
`IconPixmap` still need a profile-level fallback; raw tray pixmaps are not
currently exposed. Try `item.title` as a second icon name before using a
placeholder.

## Tray menus

Tray menus use `com.canonical.dbusmenu`. Call `open_tray_menu(item.id)` while a
popup is visible; the extension calls `AboutToShow` and `GetLayout`, then fills
`tray_menu`. The active menu is refreshed about every 500 ms. Call
`close_tray_menu()` when the popup closes.

Menu entries look like:

```lua
{
    id = 12,
    label = "Open",
    enabled = true,
    type = "standard",      -- or "separator"
    icon = "firefox",       -- icon-name, or nil
    toggle = nil,           -- "checkmark" or "radio"
    checked = false,
    submenu = false,
    children = {},          -- nested entries when submenu is true
}
```

`tray_menu = { id, revision, items, error }`, where `id` is the tray item id.
Hidden entries are omitted. Labels and icon names are clamped to 128 characters;
menus are limited to 128 entries and 8 levels. Click a leaf with
`tray_menu_event(item.id, entry.id, "clicked")`; use `"opened"`/`"closed"` when
expanding a submenu. `icon-data` pixmaps are not exposed. If the item exports no
`Menu` path, call `context_menu_tray(item.id)` and let the application draw its
own menu. The menu API is a thin D-BusMenu bridge; rendering, submenu expansion,
and popup sizing stay in Lua.

## Notifications

| Field       | Meaning                                                                                     |
| ----------- | ------------------------------------------------------------------------------------------- |
| `id`        | Positive integer notification id                                                            |
| `app_name`  | Sender application name; clamped to 256 characters                                          |
| `app_icon`  | Icon name or path from the sender; clamped to 256 characters                                |
| `summary`   | Summary text; clamped to 1024 characters                                                    |
| `body`      | Body text; clamped to 1024 characters                                                       |
| `actions`   | Array of `{ key, label }`, at most 16; labels are clamped to 256 characters                 |
| `timeout`   | Effective timeout in milliseconds; `0` means it does not expire; sender `-1` becomes `5000` |
| `timestamp` | Creation time in milliseconds since the Unix epoch                                          |

At most 64 notifications are kept. When full, the oldest is removed. A `notify`
call whose `replaces_id` matches a live notification replaces that entry;
otherwise it creates a new id. Notifications also disappear when their timeout
expires. The extension emits the standard D-Bus `ActionInvoked` and
`NotificationClosed` signals; closed reasons are 1 for expiry, 2 for dismissal,
3 for an explicit close, and 4 for eviction.

## File picker

`pick_file` opens the desktop file chooser through
`org.freedesktop.portal.FileChooser`. It needs a running
`xdg-desktop-portal` with a file-chooser backend (for example
`xdg-desktop-portal-gtk`); the dialog belongs to the portal, not to `riced`.

Options:

| Option      | Type    | Meaning                                              |
| ----------- | ------- | ---------------------------------------------------- |
| `title`     | string  | Dialog title                                         |
| `multiple`  | boolean | Allow several selections                             |
| `directory` | boolean | Select directories instead of files                  |
| `path`      | string  | Absolute starting directory; a leading `~/` is expanded |

```lua
local dbus = riced.plugin("dbus")

dbus.pick_file({
    title = "Choose a shader",
    path = "~/Pictures",
}, function(paths, error)
    if error == "cancelled" then return end
    if error then log(error) return end
    log("picked", paths[1])
end)
```

The callback always receives a non-empty array of absolute paths, even when
`multiple` is false. Cancelling the dialog returns `(nil, "cancelled")`; a
dismissed or failed chooser returns another error string. Options may be
omitted. The callback is the only way to get the selection or see a cancel, and
`error(error)` inside it only logs through riced's callback wrapper; it does not
propagate to the calling code.

The chooser is served synchronously on the extension's method thread, so other
`riced-dbus` method calls wait until the dialog closes. Store updates and tray
refreshes keep running. A chooser left open for 120 seconds is closed and the
callback receives `(nil, "the file chooser timed out")`. The dialog times out after 120 seconds; the request is
closed and the callback receives `(nil, "the file chooser timed out")`.

## Methods

| Method                                | Arguments               | Behavior                                                                                             |
| ------------------------------------- | ----------------------- | ---------------------------------------------------------------------------------------------------- |
| `activate_tray(id)`                   | string                  | Calls `Activate(0, 0)` on the tray item                                                              |
| `secondary_activate_tray(id)`         | string                  | Calls `SecondaryActivate(0, 0)` on the tray item                                                     |
| `context_menu_tray(id)`               | string                  | Calls `ContextMenu(0, 0)` on the tray item                                                           |
| `open_tray_menu(id, callback?)`       | string                  | Reads the item's `com.canonical.dbusmenu` layout into `tray_menu`; errors if it has no menu          |
| `close_tray_menu()`                   | none                    | Clears `tray_menu`; safe to call repeatedly                                                          |
| `tray_menu_event(id, menu_id, event, callback?)` | string, integer, string | Sends `clicked`, `opened`, `closed`, or `hovered` for a menu item |
| `close_notification(id)`              | integer                 | Removes the notification if present and emits `NotificationClosed`; unknown ids are ignored          |
| `invoke_notification_action(id, key)` | integer, string         | Validates the action key, removes the notification, and emits `ActionInvoked` + `NotificationClosed` |
| `pick_file(options?, callback?)`      | table                   | Opens the XDG desktop portal file chooser; the callback receives selected absolute paths              |

Tray methods fail if the item id is unknown, the session bus is disconnected, or
the D-Bus call fails. They use a one-second D-Bus method timeout.
`open_tray_menu` also fails when the item exports no `Menu` path.
`tray_menu_event` rejects unknown menu ids and events, and only the active menu
is accepted. `invoke_notification_action` fails if the notification exists but
the key is not one of its actions. A missing or unknown notification id is
ignored. All methods except `pick_file` return no value on success; use callbacks
to observe errors.

Snapshots are bounded by the protocol line limit. Tray text and notification
names are clamped as shown; very large arrays may omit entries with a
diagnostic. On disconnect the extension retries every second; `connected` is
false and the previous arrays may remain until the next successful snapshot.

## Example

```lua
local dbus = riced.plugin("dbus")

for _, item in ipairs(dbus.tray or {}) do
    iced.text(item.title)
end

local item = (dbus.tray or {})[1]
if item then
    dbus.activate_tray(item.id, function(_, error)
        if error then log(error) end
    end)
    dbus.context_menu_tray(item.id)
end

local notification = (dbus.notifications or {})[1]
if notification then
    dbus.close_notification(notification.id)
end
```
