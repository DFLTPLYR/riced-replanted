# riced API reference

This is the complete Lua reference for a `riced` profile. It covers the injected
`riced` and `iced` globals, daemon callbacks, widgets, events, tasks,
subscriptions, files, and extensions. For installation, profile layout, reload
behavior, and examples, see [README.md](README.md).

The injected API is defined in `src/lua/`, `src/file.rs`, `src/plugins.rs`, and
`src/widgets/`. Editor definitions live in `example/stubs/riced.lua`; update the
stub when this document changes.

## Contents

- [Conventions](#conventions)
- [riced API](#riced-api)
  - [`riced.daemon`](#riceddaemon)
  - [Daemon callbacks](#daemon-callbacks)
  - [`config` and `theme`](#config-and-theme)
  - [`riced.file`](#ricedfile)
  - [Plugins and extensions](#plugins-and-extensions)
- [iced components](#iced-components)
  - [Constructors and common props](#constructors-and-common-props)
  - [Widget reference](#widget-reference)
  - [Widget events](#widget-events)
  - [Styles](#styles)
  - [Value helpers](#value-helpers)
  - [Tasks](#tasks)
  - [Subscriptions](#subscriptions)
  - [Property transitions](#property-transitions)
  - [Custom widgets](#custom-widgets)
  - [Components](#components)
  - [Layer surfaces](#layer-surfaces)
  - [Input regions](#input-regions)
- [Known limits](#known-limits)

## Conventions

- Profiles use Lua 5.4 and have access to the standard libraries. Only run
  trusted profiles.
- Widget props use snake_case names. There is no automatic camelCase conversion.
- Table keys are validated. Unknown or mistyped keys raise an error instead of
  being ignored. Widget props are checked when `app:view` returns.
- Colors are ASCII strings in `#RRGGBB` or `#RRGGBBAA` form.
- Widget indices in `render` and widget callbacks are zero-based.
- A widget constructor returns an opaque element table. Pass elements back to
  `iced` constructors, `iced.layer`, `iced.component`, or return them from
  `app:view`.
- `update` may return `nil`, one task table, or a list of task tables. See
  [Tasks](#tasks).

### Lengths

Widget `width` and `height` accept:

- `"fill"` or `iced.Length.fill`
- `"shrink"` or `iced.Length.shrink`
- a non-negative number of pixels
- `{ fill_portion = n }` or `iced.Length.fill_portion(n)`, where `n` is an
  integer from 1 to 65535

Layer sizes use a narrower extent form: `"fill"` or a positive integer.

### Fonts

`font` accepts a family name or a table:

```lua
iced.text("Hello", { font = "monospace" })
iced.text("Hello", {
    font = { family = "serif", weight = "bold", style = "italic", stretch = "condensed" },
})
```

Family names: `sans_serif`, `sans-serif`, `serif`, `monospace`, `mono`,
`cursive`, `fantasy`, or a custom family name.

Weights: `thin`, `extra_light`, `light`, `normal`, `medium`, `semibold`,
`semi_bold`, `bold`, `extra_bold`, `black`.

Styles: `normal`, `italic`, `oblique`.

Stretches: `ultra_condensed`, `extra_condensed`, `condensed`, `semi_condensed`,
`normal`, `semi_expanded`, `expanded`, `extra_expanded`, `ultra_expanded`.

## riced API

### `riced.daemon`

```lua
local app = { count = 0 }

function app:update(event, component) end
function app:view(props) end

return riced.daemon(app, "my-profile", app.update, app.view)
    :title("my bar")
    :subscription(app.subscription)
    :theme(config.theme)
    :style(app.style)
    :redraw_scope(app.redraw_scope)
    :settings({ id = "my-bar", layer_settings = { start_mode = "active" } })
    :run()
```

Arguments:

| Argument    | Type     | Required | Meaning                                                |
| ----------- | -------- | -------- | ------------------------------------------------------ |
| `app`       | table    | yes      | State table passed as `self` to every callback         |
| `namespace` | string   | yes      | Nonempty Iced theme namespace and surface namespace    |
| `update`    | function | yes      | Event handler, called as `update(self, event, component)` |
| `view`      | function | yes      | Renderer, called as `view(self, props)`                |

`riced.daemon` returns a builder. Chain any of the options below and finish with
`:run()`. `run()` must be called exactly once. Rust starts the event loop after
`shell.lua` returns.

| Builder method             | Value                                             |
| -------------------------- | ------------------------------------------------- |
| `:title(value)`            | string, or `function(self, window) -> string?`    |
| `:subscription(value)`     | `function(self, active) -> subscription(s)?`      |
| `:theme(value)`            | palette table, or `function(self, window)`        |
| `:style(value)`            | `function(self, palette) -> style table?`         |
| `:redraw_scope(value)`     | `function(self, event) -> scope`                  |
| `:settings(value)`         | settings table                                    |

Every callback except `update` and `view` is optional.

### Daemon callbacks

| Callback       | Signature                                | Return                                               |
| -------------- | ---------------------------------------- | ---------------------------------------------------- |
| `update`       | `(self, event, component)`               | `nil`, a task, or a list of tasks                    |
| `view`         | `(self, props)`                          | element, `iced.layer`, or a nonempty list of layers  |
| `title`        | `(self, window)`                         | string or `nil`                                      |
| `subscription` | `(self, active)`                         | subscription, list of subscriptions, or `nil`        |
| `theme`        | `(self, window)`                         | palette table or `nil`                               |
| `style`        | `(self, palette)`                        | `{ background_color?, text_color? }` or `nil`        |
| `redraw_scope` | `(self, event)`                          | `"all"`, `"none"`, `props.window`, or `nil`           |

`component` is the component id that raised the event, or `nil` for global
events. See [Widget events](#widget-events).

`props` passed to `view` has:

| Field    | Type          | Meaning                                                       |
| -------- | ------------- | ------------------------------------------------------------- |
| `window` | opaque userdata | Surface identifier for this render; use with window tasks    |
| `width`  | number or nil | Current logical surface width; available after the first resize |
| `height` | number or nil | Current logical surface height; available after the first resize |

`redraw_scope` runs for every incoming message before `update`. Return `nil` or
`"all"` to redraw every window, `"none"` to skip redraws, or `props.window` to
redraw only that window.

`settings` accepts:

```lua
{
    id = "my-bar",
    layer_settings = {
        start_mode = "background", -- background, active, all_screens
        keyboard_interactivity = "on_demand", -- none, on_demand, exclusive
    },
}
```

`start_mode = "active"` is the usual choice for visible widget-only views.
`"background"` starts without a surface. Layer views created with `iced.layer`
get their own surfaces independently. `"all_screens"` is also accepted.

### `config` and `theme`

The globals are initialized to:

```lua
config = {
    theme = {
        background = "#2b2d31",
        text = "#e6e6e6",
        primary = "#5865f2",
        secondary = "#949cbb",
        success = "#12664f",
        warning = "#ffc14e",
        danger = "#c3423f",
    },
}
theme = config.theme
```

Modify fields in place, or assign `config.theme` before creating the daemon.
`secondary` is available to your Lua code and widgets; Iced's base palette does
not use it.

The `theme` callback may return a palette table. When it returns `nil`, the
global `config.theme` is used. The palette's `background`, `text`, `primary`,
`success`, `warning`, and `danger` fields become the Iced palette. `secondary`
is validated as a color but is not part of the Iced palette.

The `style` callback receives `config.theme` and returns window chrome colors:

```lua
function app:style(palette)
    return {
        background_color = palette.background,
        text_color = palette.text,
    }
end
```

### `riced.file`

Loads and saves JSON or TOML files. Relative paths resolve against the profile
directory.

```lua
local settings = riced.file({
    path = "data/settings.json",
    type = "json", -- optional for .json/.toml
    onLoaded = function(file, data) end,
    onError = function(file, error) end,
})
```

Options:

| Option     | Type     | Required | Meaning                                             |
| ---------- | -------- | -------- | --------------------------------------------------- |
| `path`     | string   | yes      | Nonempty absolute path or profile-relative path     |
| `type`     | string   | no       | `"json"` or `"toml"`; inferred from the extension   |
| `onLoaded` | function | no       | `(file, data)` after a successful load/create       |
| `onError`  | function | no       | `(file, error)` after a read, create, or parse error |

`riced.file` returns a userdata with:

| Field/method | Type                     | Meaning                                                     |
| ------------ | ------------------------ | ----------------------------------------------------------- |
| `path`       | string                   | Resolved path                                               |
| `type`       | string                   | `"json"` or `"toml"`                                        |
| `data`       | any                      | Loaded value; assign to replace it                          |
| `error`      | string or nil            | Last load/save error                                        |
| `file:save()` | `boolean, string?`      | Writes `data`; returns `true` or `false, error`             |
| `file:write(value?)` | `boolean, string?` | Replaces `data` when provided, then saves                   |

Missing files are created before `onLoaded` runs: `{}` for JSON and an empty
document for TOML. Missing parent directories are created. Writes are atomic
(temporary file plus rename). File errors are returned, not raised; `file.error`
is updated.

Callbacks run during construction, so they run before the daemon starts. A file
created as empty gets `onLoaded`, not `onError`. TOML requires a table at the top
level and cannot encode `nil` values.

```lua
settings.data.theme = "dark"
local ok, err = settings:save()
settings:write({ theme = "light" })
```

### Plugins and extensions

Extensions are separate executables that connect to `riced` over a Unix socket.
`riced` supervises executables under `<profile>/extensions/`. See
[Writing extensions](#writing-extensions).

#### `riced.plugin(name)` and `riced.plugins(name)`

Both names refer to the same function and always return a proxy table. Before
an extension registers, `alive` is `false` and `store` is empty. The same proxy
table is filled when the extension connects, so capturing it at module load is
safe.

```lua
local wayland = riced.plugin("wayland")

if wayland.connected then
    for _, toplevel in ipairs(wayland.toplevels or {}) do
        iced.text(toplevel.title)
    end
end
```

| Field   | Type    | Meaning                                                       |
| ------- | ------- | ------------------------------------------------------------- |
| `name`  | string  | Extension name                                                |
| `alive` | boolean | Whether the extension is connected                            |
| `store` | table   | Last published snapshot                                       |

Unknown keys on the proxy read through to `store`, so `plugin.cpu` and
`plugin.store.cpu` are equivalent. Declared extension methods keep their names.

Store reads are lazy: reading a missing key subscribes to that key, and the
extension may compute only requested fields. A lazy field is `nil` until the
first response arrives. Requests are monotonic for the process lifetime; there
is no unsubscribe.

The snapshot is replaced whenever the extension publishes, while table identity
is preserved. Cached `plugin.store` references stay valid. On disconnect,
`alive` becomes `false` and the last snapshot is kept.

Declared methods are asynchronous. Call them with `.` and pass a callback as the
last argument:

```lua
system.refresh()
system.query("memory", function(result, err) end)
```

On success the callback receives `(result, nil)`. On failure it receives
`(nil, error)`. Arguments and results must be JSON-serializable. Without a
callback the call is fire-and-forget. A full pending queue raises an error.

Extensions may push events. They arrive in `update` as:

```lua
{ type = "plugin", plugin = "system", event = <value> }
```

#### Wayland extension

`riced-wayland` uses `wl_output`, `ext-workspace-v1`, and
`zwlr-foreign-toplevel-management-v1`, with read-only
`ext-foreign-toplevel-list-v1` as a fallback. It is not Hyprland-specific.
Missing protocols produce empty data and a `false` capability flag.

```lua
local wayland = riced.plugin("wayland")
local focused = wayland.focused or {}

for _, workspace in ipairs(wayland.workspaces or {}) do
    iced.button(workspace.name, {
        on_press = workspace.active and nil or { event = "workspace", args = { id = workspace.id } },
    })
end
```

Store fields:

| Field              | Shape                                                                                                    |
| ------------------ | -------------------------------------------------------------------------------------------------------- |
| `connected`        | boolean; false while the compositor connection is down                                                   |
| `capabilities`     | `{ toplevels, toplevel_control, workspaces }` booleans                                                   |
| `outputs`          | `{ id, name, description, make, model, x, y, width, height, scale }`                                     |
| `toplevels`        | `{ id, title, app_id, outputs, activated, maximized, minimized, fullscreen }`; the read-only fallback also has `identifier` and no state |
| `workspaces`       | `{ id, protocol_id, name, coordinates, active, urgent, hidden, capabilities, group, outputs }`           |
| `workspace_groups` | `{ id, capabilities, outputs }`                                                                          |
| `focused`          | `{ toplevel, workspace, output }` ids or names; `nil` when unknown                                        |

Methods: `activate_toplevel(id)`, `close_toplevel(id)`, and
`activate_workspace(id)`.

`capabilities.toplevel_control` is `false` when only the read-only toplevel list
is available. Workspace activation needs `ext-workspace-v1` and the workspace's
own `capabilities` list. Standard protocols do not map toplevels to workspaces.

#### D-Bus extension

`riced-dbus` provides StatusNotifier tray items and an
`org.freedesktop.Notifications` server. It owns
`org.kde.StatusNotifierWatcher` when the name is free; otherwise it registers
with the existing watcher. It owns the notification name only if no other
notification daemon does.

```lua
local dbus = riced.plugin("dbus")

for _, item in ipairs(dbus.tray or {}) do
    iced.text(item.title)
end

dbus.activate_tray(item.id)
dbus.context_menu_tray(item.id)
dbus.close_notification(notification.id)
```

Store fields:

| Field           | Shape                                                                              |
| --------------- | ---------------------------------------------------------------------------------- |
| `connected`     | boolean; session bus connection                                                    |
| `capabilities`  | `{ tray, tray_host, notifications }` booleans                                      |
| `tray`          | `{ id, title, icon, status, menu, item_is_menu }`                                  |
| `notifications` | `{ id, app_name, app_icon, summary, body, actions, timeout, timestamp }`           |

`actions` is a list of `{ key, label }`.

Methods: `activate_tray(id)`, `secondary_activate_tray(id)`,
`context_menu_tray(id)`, `close_notification(id)`, and
`invoke_notification_action(id, key)`.

Tray icons are freedesktop icon names, not paths. Notifications expire according
to the sender's timeout and disappear from the store.

#### Writing extensions

Extensions are executables, or directories containing an executable, under
`<profile>/extensions/`. `riced` spawns each one, restarts crashed extensions
with backoff, and stops one when its file is removed. An extension may also be
started manually with `RICED_SOCKET` set.

The SDK is `crates/riced-extension-sdk`. Protocol v1 is newline-delimited JSON
with a 256 KiB line limit, bounded queues, a `0700` socket directory, and a
`0600` socket. Extensions run as your user and have the same trust as
`shell.lua`.

```sh
just extension-system
just extension-wayland
just extension-dbus
```

## iced components

### Constructors and common props

Every widget constructor has the form `iced.name(content, props?)`. `content` is
the widget's child element, list of children, or primary value; pass `nil` when
the widget has no content. `props` defaults to `{}`.

Every constructor also accepts these common props:

| Prop         | Type                        | Meaning                                               |
| ------------ | --------------------------- | ----------------------------------------------------- |
| `width`      | [length](#lengths)          | Layout width                                          |
| `height`     | [length](#lengths)          | Layout height                                         |
| `padding`    | non-negative number         | Uniform padding around the widget                     |
| `transition` | `true`, table, or `false`   | Animate supported property changes; see [transitions](#property-transitions) |

Widget-specific props are listed below. `transition` on `list_view` means item
enter/leave animation instead of property animation.

### Widget reference

#### `iced.text(content, props)`

`content`: string or number.

Props: `size` (default 16), `color`, `line_height` (positive factor or
`{ pixels = n }`), `wrap` (`"none"`, `"word"`, `"glyph"`, `"word_or_glyph"`,
`true`, or `false`), `align_x` (`"left"`, `"center"`, `"right"`, `"justified"`),
`align_y` (`"top"`, `"center"`, `"bottom"`), `shaping` (`"auto"`, `"basic"`,
`"advanced"`), `center` (boolean), and `font`.

`align_x` and `align_y` only take effect when the text has a constrained size,
so set `width` or `height` when aligning.

#### `iced.image(content, props)` and `iced.svg(content, props)`

`content`: nonempty file path.

Props: `content_fit` (`"contain"`, `"cover"`, `"fill"`, `"none"`,
`"scale_down"`). `svg` also accepts `opacity` from 0 to 1.

#### `iced.rule(content, props)`

Props: `direction` (`"horizontal"` or `"vertical"`, default horizontal),
`thickness` (non-negative, default 1), and `style` (`{ color, radius, snap }`).

#### `iced.progress_bar(content, props)`

Props: `value` (0 to 1, default 0), `girth` (positive), `vertical` (boolean), and
`style` (`{ background, bar, border }`).

#### `iced.qr_code(content, props)`

`content`: nonempty string to encode.

Props: `cell_size` (positive), `total_size` (positive), and `style`
(`{ cell, background }`).

#### `iced.row(content, props)` and `iced.column(content, props)`

`content`: array of elements.

Props: `spacing` (non-negative, default 0), `clip` (boolean), and `align_y` for
rows or `align_x` for columns.

#### `iced.container(content, props)`

Props: `align_x`, `align_y`, `max_width`, `max_height`, `clip`, and
`style` (`{ background, text_color, border, shadow }`).

#### `iced.space(content, props)`

Pass `nil` as content. Usable props are the common `width` and `height`.

#### `iced.grid(content, props)`

`content`: array of elements.

Props: `columns` (integer at least 1, default 3), `spacing` (non-negative), and
`fluid` (maximum pixel width for fluid columns).

#### `iced.stack(content, props)`

`content`: array of elements, drawn in order.

Props: `clip` (boolean).

#### `iced.float(content, props)`

Props: `scale` (finite positive number, default 1).

#### `iced.pin(content, props)`

Props: `x` and `y` offsets.

#### `iced.tooltip(content, props)`

Props: `tooltip` (required string), `position` (`"top"` default, `"bottom"`,
`"left"`, `"right"`, `"follow_cursor"`), and `gap`.

#### `iced.button(content, props)`

`content`: string or element.

Props: `on_press` (raw event value), `clip`, and `style`. Omitting `on_press`
makes the button disabled. `on_press` is delivered to `update` as its raw value,
not as a widget-event table.

#### `iced.checkbox(content, props)`, `iced.radio(content, props)`, and `iced.toggler(content, props)`

Pass `nil` as content.

| Widget      | Props                                                                 | Event        |
| ----------- | --------------------------------------------------------------------- | ------------ |
| `checkbox`  | `value` (boolean, default false), `label`, `text_size`, `on_toggle`   | `on_toggle`  |
| `radio`     | `value` (integer, default 0), `selected` (integer), `label`, `text_size`, `on_select` | `on_select` |
| `toggler`   | `value` (boolean, default false), `label`, `text_size`, `on_toggle`   | `on_toggle`  |

These widgets send wrapped widget events with boolean or numeric `value`.
`radio` values are cast to unsigned integers. `radio` defaults `on_select` to an
empty event name, so set it if you want to handle the event.

#### `iced.slider(content, props)` and `iced.vertical_slider(content, props)`

Pass `nil` as content. `vertical_slider` uses the `height` prop for its track.

Props: `min` (default 0), `max` (default 1), `value` (default `min`, clamped to
the range), `step` (finite positive), `on_change`, `on_release`, and `style`.

`min` must be finite and less than `max`. `on_change` fires while dragging;
`on_release` fires when the drag ends. Both send a numeric `value`.

#### `iced.text_input(content, props)`

Pass `nil` as content.

Props: `placeholder`, `value`, `secure` (boolean), `id`, `size` (positive),
`align_x` (`"left"`, `"center"`, `"right"`), `font`, `on_input`, `on_submit`,
`on_paste`, and `style`.

`on_input` sends the new text, `on_submit` sends the submitted value, and
`on_paste` sends pasted text. All are wrapped events.

#### `iced.pick_list(content, props)`

Pass `nil` as content.

Props: `options` (required array of strings), `selected` (string or nil),
`placeholder`, `text_size` (positive), `on_select`, and `style`.

`on_select` sends the selected string.

#### `iced.text_editor(content, props)`

Pass `nil` as content. Requires a nonempty `id`.

Props: `value` (initial text, default `""`), `placeholder`, `id`, `size`
(positive), `font`, `wrapping` (boolean or the text `wrap` values), `on_action`,
`on_input`, and `style`.

State is kept by `id` for the lifetime of the process or until `riced` reloads
the shell. Passing a `value` that differs from the current text replaces the
text; changing the id starts a fresh editor.

If `on_action` is set, it takes precedence over `on_input`. `on_action` sends:

```lua
{ event = "action", widget = "editor", value = "edit", text = "<current text>" }
```

`value` is one of `edit`, `move`, `select`, `select_word`, `select_line`,
`select_all`, `click`, `drag`, or `scroll`. With `on_input` instead, only the
text is sent as `value`.

#### `iced.combo_box(content, props)`

Pass `nil` as content. Requires a nonempty `id`.

Props: `options` (required array of strings), `value` (selected string or nil),
`placeholder`, `on_select`, `on_input`, and `style`.

The widget keeps its filtered options and selection state by `id`. `on_select`
sends the selected string; `on_input` sends the filter text.

#### `iced.mouse_area(content, props)`

Props: `on_press`, `on_release`, `on_double_click`, `on_right_press`,
`on_right_release`, `on_middle_press`, `on_middle_release`, `on_enter`,
`on_exit`, `on_move`, and `on_scroll`.

The button, enter, and exit handlers deliver their configured value directly to
`update` (the same as `button.on_press`). `on_move` and `on_scroll` send wrapped
events with `value = { x, y }`.

#### `iced.sensor(content, props)`

Props: `on_show`, `on_hide`, and `on_resize`.

`on_show` and `on_resize` send wrapped events with
`value = { width, height }`. `on_hide` delivers its raw value.

#### `iced.scrollable(content, props)`

Props: `direction` (`"vertical"` default, `"horizontal"`, or `"both"`), `id`,
`on_scroll`, `auto_scroll` (boolean), `spacing`, and `style`.

`on_scroll` sends:

```lua
{
    event = "scroll",
    value = {
        x = 0, y = 0,
        relative_x = 0, relative_y = 0,
        bounds_width = 0, bounds_height = 0,
        content_width = 0, content_height = 0,
    },
}
```

`id` identifies the scrollable for `{ type = "scroll_to", id = ... }` and
`{ type = "focus", id = ... }`.

#### `iced.region(content)`

Makes the Wayland input region of the surface match the wrapped element's real
layout rectangle, so clicks outside it pass through the layer. Bounds update on
redraw.

Use one region per layer and wrap all interactive content in it. The last region
applied in a layer wins. A region applies to the surface that renders it; it does
not apply to nested windows or components rendered in another layer.

```lua
iced.region(iced.container(iced.component("panel", "components.panel"), {
    width = 320,
    height = 40,
}))
```

#### `iced.table(content, props)`

`content`: array of rows. Each row is an array of cells; a cell may be a string,
number, element, or `nil`. Missing cells render as empty space.

Props:

| Prop       | Type                                                         | Meaning                    |
| ---------- | ------------------------------------------------------------ | -------------------------- |
| `columns`  | array of column tables (required, at least one)              | Header and column layout   |
| `spacing`  | non-negative number                                          | Separator size             |

A column accepts:

```lua
{
    header = "Name",             -- string, number, or element
    width = 120,                 -- length
    align_x = "left",            -- left, center, right
    align_y = "center",          -- top, center, bottom
}
```

`table` has no style prop in iced 0.14.

#### `iced.list_view(content, props)`

`content`: array of items. `render(item, index)` turns an item into an element.
Without `render`, items must already be elements, strings, numbers, or `nil`.

Props:

| Prop             | Type       | Meaning                                                     |
| ---------------- | ---------- | ----------------------------------------------------------- |
| `id`             | string     | Required, nonempty identity for item tracking               |
| `direction`      | string     | `"vertical"` default or `"horizontal"`                      |
| `spacing`        | number     | Gap between items                                           |
| `render`         | function   | `(item, index) -> element`; index is zero-based             |
| `on_item_added`  | function   | `(index, item)` when items are appended or first rendered   |
| `on_item_removed`| function   | `(index, item)` when trailing items disappear               |
| `onItemAdded`    | function   | Alias for `on_item_added`                                   |
| `onItemRemoved`  | function   | Alias for `on_item_removed`                                 |
| `transition`     | boolean/table | Item enter/leave animation; see below                    |

Diffing is by item count only. Appended items and removals from the end are
detected. Reordering or removing from the middle is not. Callbacks run during
render, so state they change takes effect on the next render. The direct
callbacks take precedence over `transition.onAdded`/`onRemoved` when both are
set.

`transition = true` uses a 200 ms scale animation. A table accepts:

```lua
transition = {
    duration = 180,                       -- whole milliseconds
    x = { from = -180, to = 0 },          -- slide range; number means { n, 0 }
    onAdded = function(index, item) end,
    onRemoved = function(index, item) end,
}
```

New items transition from `x.from` to `x.to` while scaling from 0 to 1. Leaving
items reverse that. Leaving items keep their layout space until the animation
finishes. The transition only changes drawing; layout is not animated.

#### `iced.markdown(content, props)`

`content`: Markdown string.

Props: `text_size` (positive, default 16), `spacing` (non-negative),
`on_link_click` (sends the URL as a wrapped string value), and `style`
(`{ link_color, inline_code_color }`). Text selection is not exposed.

#### `iced.responsive(content, props)`

`content`: `function(size) -> element`, where `size` is `{ width, height }`.

The callback runs during layout and must not mutate app state. Errors fall back
to empty space. Common `width` and `height` props are passed to the responsive
layout.

#### `iced.canvas(content, props)`

`content`: array of shape tables.

| Shape    | Fields                                                                 |
| -------- | ---------------------------------------------------------------------- |
| `rect`   | `x`, `y`, `width` (positive), `height` (positive), `color`, optional `border = { color, width }` |
| `circle` | `x`, `y`, `radius` (positive), `color`, optional `border`              |
| `line`   | `from = { x, y }`, `to = { x, y }`, `color`, `width` (default 1)        |
| `path`   | `points` (at least two `{ x, y }`), `color`, `width`, `fill` (boolean)   |
| `text`   | `x`, `y`, `content`, `size` (default 16), `color`                      |

Coordinates default to 0. `border.width` defaults to 1. Arbitrary Iced canvas
programs are not supported.

#### `iced.pane_grid(content, props)`

`content`: nonempty array of pane elements. Requires a nonempty `id`.

Props: `direction` (`"horizontal"` default or `"vertical"`), `spacing`,
`min_size`, `on_click`, `on_resize`, `on_drag`, and `style`.

Events:

- `on_click` sends the zero-based pane index as a numeric `value`.
- `on_resize` sends the new ratio as a numeric `value`.
- `on_drag` sends wrapped `value` (`"picked"`, `"dropped"`, or `"canceled"`)
  plus `pane = index`.

`style` accepts:

```lua
{
    hovered_region = { background = "#...", border = { color = "#...", width = 1 } },
    picked_split = { color = "#...", width = 1 },
    hovered_split = { color = "#...", width = 1 },
}
```

Resize ratios are delivered to Lua but cannot be set back. The host keeps the
live pane layout for the process lifetime, keyed by `id`, pane count, and
direction; a shell reload starts from equal panes.

#### `iced.component(id, module, props)`

Swaps in a module's `view` output and watches that module's file for edits.
`id` and `module` are nonempty strings. `props` defaults to `{}` and must be a
table.

The module is required by name from `package.path` and must return a table with
a `view` method:

```lua
-- components/panel.lua
local panel = {}

function panel:view(props, window)
    return iced.container(iced.column({
        iced.text("Count: " .. props.count),
        iced.button("Increment", { on_press = "increment" }),
    }))
end

return panel
```

```lua
function app:view(props)
    return iced.component("panel", "components.panel", { count = self.count })
end
```

Component ids must be unique within one window and free of cycles. Props are
rebuilt on every render. Module locals are recreated when the module reloads;
keep persistent state in `app` and pass it as props. Events from a component
whose module has reloaded since the render are dropped.

A component module may define:

```lua
function panel:on_visible(props, window) end
function panel:on_destroyed(props, window) end
```

`on_visible` fires once when an id first appears in a window's rendered tree and
again after a shell reload. It means mounted, not visible in the viewport.
`on_destroyed` fires when the id stops appearing in a window's rendered tree.
Hooks run during render; changes take effect on the next render. Closing a layer
surface does not fire `on_destroyed`.

Return `iced.layer` at the root of `app:view`, not from inside a component.

#### `iced.layer(content, settings)`

Describes a separate Wayland layer-shell surface. It is not a widget inside
another window, and layer descriptors must be returned at the root of
`app:view`, either alone or in a list.

```lua
iced.layer(content, {
    id = "background",
    size = { width = "fill", height = "fill" },
    layer = "background",
    anchor = { "top", "bottom", "left", "right" },
    exclusive_zone = -1,
    margin = { 0, 0, 0, 0 },
    keyboard_interactivity = "none",
    output_option = "active",
    events_transparent = true,
    namespace = "riced-background",
    blur_option = "none",
})
```

Settings:

| Key                    | Values                                                                                       | Default |
| ---------------------- | -------------------------------------------------------------------------------------------- | ------- |
| `id`                   | nonempty string                                                                              | position-based |
| `size`                 | `{ width, height }`, each `"fill"` or positive integer pixels                                | fill/fill |
| `layer`                | `"background"`, `"bottom"`, `"top"`, `"overlay"`                                             | `"top"` |
| `anchor`               | array of `"top"`, `"bottom"`, `"left"`, `"right"`                                            | all edges |
| `exclusive_zone`       | integer at least -1, or nil                                                                  | none/0 |
| `margin`               | four integers: top, right, bottom, left                                                      | zeros |
| `keyboard_interactivity` | `"none"`, `"on_demand"`, `"exclusive"`                                                     | `"on_demand"` |
| `output_option`        | `"active"`, `"last_output"`, output name, or positive Wayland registry global ID             | `"active"` |
| `events_transparent`   | boolean                                                                                      | false |
| `namespace`            | nonempty string without NUL                                                                  | inherited |
| `blur_option`          | `"none"`, `"full"`, or an array of `{ x, y, width, height }` with positive width/height      | `"none"` |

A `"fill"` axis requires both opposite anchors on that axis. Pixel dimensions
must fit a signed 32-bit integer.

Saving a layer module updates changed size, anchors, level, margins, exclusive
zone, keyboard mode, and blur through native actions on the existing surface.
Output, namespace, and input-transparency changes recreate only the affected
surface. Adding or removing layers creates or closes surfaces. An explicit `id`
preserves identity across reordering.

### Widget events

Widget callbacks (`on_input`, `on_submit`, `on_toggle`, `on_change`,
`on_release`, `on_select`, `on_scroll`, `on_show`, `on_resize`, `on_move`,
`on_action`, `on_link_click`, etc.) deliver a table to `update`:

```lua
{
    event = "search",       -- the callback name/string configured on the widget
    widget = "panel",       -- present when a component raised the event
    value = "hello",        -- widget-specific value
}
```

`value` types by widget:

| Value              | Widgets / callbacks                                                   |
| ------------------ | --------------------------------------------------------------------- |
| boolean            | `checkbox.on_toggle`, `toggler.on_toggle`                             |
| number             | `radio.on_select`, `slider.on_change`/`on_release`, `vertical_slider`, `pane_grid.on_click` (index), `pane_grid.on_resize` (ratio) |
| string             | `text_input`, `pick_list`, `combo_box`, `markdown.on_link_click`, `text_editor.on_input` |
| `{ x, y }`         | `mouse_area.on_move`, `mouse_area.on_scroll`                          |
| `{ width, height }`| `sensor.on_show`, `sensor.on_resize`                                   |
| scroll table       | `scrollable.on_scroll`                                                |
| action table       | `text_editor.on_action` adds `text`; `pane_grid.on_drag` adds `pane`  |

`scrollable.on_scroll` carries:

```lua
{
    x = 0, y = 0,
    relative_x = 0, relative_y = 0,
    bounds_width = 0, bounds_height = 0,
    content_width = 0, content_height = 0,
}
```

`pane_grid.on_drag` uses `value = "picked" | "dropped" | "canceled"` and also
sets `pane`.

Plain handlers deliver their configured value directly, without the wrapper:
`button.on_press`, `mouse_area.on_press`/`on_release`/`on_double_click`/right and
middle button handlers/`on_enter`/`on_exit`, and `sensor.on_hide`. A plain
handler value may be a string, number, boolean, or table such as
`{ event = "open", args = { id = 3 } }`; it is passed to `update` unchanged.

### Styles

`style` is a table. Omitted fields keep the widget's built-in style. Base
fields apply as fallbacks for every state; state tables override them.

```lua
style = {
    background = "#202020",
    text_color = "#ffffff",
    border = { color = "#404040", width = 1, radius = 4 },
    shadow = { color = "#00000080", offset = { x = 0, y = 2 }, blur = 8 },
    hovered = { background = "#303030" },
}
```

Shared fields:

| Field        | Type                                                    |
| ------------ | ------------------------------------------------------- |
| `background` | color                                                   |
| `text_color` | color                                                   |
| `border`     | `{ color?, width?, radius? }`; width/radius non-negative |
| `shadow`     | `{ color?, offset? = { x, y }, blur? }`; blur non-negative |

Widget style support:

| Widget                                                     | Fields                                                                          | State overrides                            |
| ---------------------------------------------------------- | ------------------------------------------------------------------------------- | ------------------------------------------ |
| `container`                                                | `background`, `text_color`, `border`, `shadow`                                  | —                                          |
| `button`                                                   | `background`, `text_color`, `border`, `shadow`                                  | `active`, `hovered`, `pressed`, `disabled` |
| `checkbox`                                                 | `background`, `text_color`, `border`, `icon_color`                              | `active`, `hovered`, `disabled`            |
| `radio`                                                    | `background`, `text_color`, `border`, `dot_color`                               | `active`, `hovered`                        |
| `toggler`                                                  | `background`, `text_color`, `border`, `foreground`                              | `active`, `hovered`, `disabled`            |
| `slider`, `vertical_slider`                                | `rail`, `filled`, `border`, `handle`, `handle_border`                           | `active`, `hovered`, `dragged`             |
| `text_input`, `combo_box`                                  | `background`, `text_color`, `border`, `placeholder`, `selection`, `icon_color`  | `active`, `hovered`, `focused`, `disabled` |
| `text_editor`                                              | `background`, `text_color`, `border`, `placeholder`, `selection`                | `active`, `hovered`, `focused`, `disabled` |
| `scrollable`                                               | `background`, `border`, `shadow`, `rail`, `scroller`                            | `active`, `hovered`, `dragged`             |
| `pick_list`                                                | `background`, `text_color`, `border`, `placeholder`, `handle`                   | `active`, `hovered`, `opened`              |

Special styles:

```lua
rule.style        = { color, radius, snap }
progress_bar.style = { background, bar, border }
qr_code.style     = { cell, background }
markdown.style    = { link_color, inline_code_color }
pane_grid.style   = { hovered_region, picked_split, hovered_split }
```

`table`, `list_view`, `image`, `svg`, `text`, and layout containers other than
`container` do not take a `style` table.

### Value helpers

These helpers produce the same values as handwritten strings/tables:

```lua
iced.Layer                 -- background, bottom, top, overlay
iced.Extent                -- fill
iced.Length                -- fill, shrink, fill_portion(n)
iced.KeyboardInteractivity -- none, on_demand, exclusive
iced.Output                -- active, last_output
iced.Blur                  -- none, full

iced.Size(width, height)   -- { width = width, height = height }
```

`iced.Length.fill_portion(n)` requires an integer from 1 to 65535. `iced.Size`
requires both axes. Weighted fill portions are for widget layout, not
layer-surface dimensions.

### Tasks

`update` may return a task table or a list of task tables:

```lua
return {
    { type = "focus", id = "search" },
    { type = "clipboard_write", content = self.text },
    { type = "scroll_to", id = "list", x = 0, y = 100 },
    { type = "font", path = "/path/to/font.ttf" },
    { type = "window", window = props.window, action = "resize", width = 800, height = 600 },
    { type = "clipboard_read" },
    { type = "exit" },
}
```

| Task              | Fields                                         | Effect                                                           |
| ----------------- | ---------------------------------------------- | ---------------------------------------------------------------- |
| `focus`           | `id`                                           | Focus the widget with that id                                    |
| `scroll_to`       | `id`, `x?`, `y?`                              | Scroll the widget with that id to an absolute offset             |
| `clipboard_read`  | —                                              | Deliver `{ type = "clipboard", value = string? }` to `update`    |
| `clipboard_write` | `content` (string)                             | Write text to the clipboard                                      |
| `font`            | `path`                                         | Load a font file at runtime                                      |
| `window`          | `window`, `action`, plus action fields         | Resize, move, close, fullscreen, or window the surface            |
| `exit`            | —                                              | Exit `riced`                                                     |

Window actions:

| Action       | Fields                    |
| ------------ | ------------------------- |
| `resize`     | `width`, `height` (positive) |
| `move`       | `x`, `y` (finite)         |
| `close`      | —                         |
| `fullscreen` | —                         |
| `windowed`   | —                         |

`window` must be the opaque `props.window` value. A table with a `type` key is
treated as one task; any other table is treated as a list of tasks. Returning an
empty list does nothing.

### Subscriptions

`subscription` may return one subscription, a list, or `nil`:

```lua
function app:subscription(active)
    return { iced.every(1000), iced.frames(), iced.on_key(true), iced.on_mouse() }
end
```

| Constructor       | Event delivered to `update`                                                                 |
| ----------------- | ------------------------------------------------------------------------------------------- |
| `iced.every(ms)`  | `{ type = "tick" }`; interval is a whole number from 1 to 86400000 ms                       |
| `iced.frames()`   | `{ type = "frame" }` on every frame                                                         |
| `iced.on_key()`   | key press and release events; see below                                                     |
| `iced.on_key(true)` | key press events only                                                                     |
| `iced.on_key(false)` | key release events only                                                                   |
| `iced.on_mouse()` | mouse events; see below                                                                     |

Key events:

```lua
{
    type = "key",
    event = "press",       -- or "release"
    key = "Escape",        -- Iced key name or character
    text = "x",            -- present on press when available
    modifiers = { shift = false, control = false, alt = false, logo = false },
}
```

Named keys use Iced's names (`Escape`, `Enter`, `ArrowLeft`, `Space`, etc.).
Character keys use the character. `text` is only present for key presses.

Mouse events:

```lua
{ type = "mouse", event = "move", x = 0, y = 0 }
{ type = "mouse", event = "press", button = "left" }
{ type = "mouse", event = "release", button = "left" }
{ type = "mouse", event = "scroll", x = 0, y = 0 }
{ type = "mouse", event = "enter" }
{ type = "mouse", event = "leave" }
```

Buttons: `left`, `right`, `middle`, `back`, `forward`, or `other`.

### Property transitions

For widgets other than `list_view`, `transition` animates property changes
instead of jumping. Animatable props are `width`, `height`, `padding`, `size`
(text), `spacing` (row/column), and `color` (text).

```lua
iced.container(child, { width = self.width, padding = 16, transition = true })
```

`transition = true` uses a smooth spring over every animatable prop present on
the element. The table form accepts:

| Key        | Values                                                                 |
| ---------- | ---------------------------------------------------------------------- |
| `props`    | array of animatable prop names                                         |
| `duration` | whole milliseconds from 1 to 86400000                                  |
| `easing`   | `linear`, `ease`, `in`, `out`, `in_out`, `ease_in`, `ease_out`, `ease_in_out` |
| `spring`   | `smooth` (default), `snappy`, `bouncy`, `instant`                      |

`easing` and `spring` are mutually exclusive. All selected props share one
timing. `"fill"`, `"shrink"`, and fill portions are not interpolated. The
element is rebuilt from its Lua table on every animation frame, so keep
animated subtrees cheap.

```lua
transition = { props = { "width", "padding" }, duration = 300, easing = "out" }
transition = { spring = "bouncy" }
```

### Custom widgets

`iced.define(name, factory)` creates a reusable constructor. The factory runs
once and must return a table with a `view` method (or a `render` function):

```lua
iced.define("counted", function()
    local widget = {}

    function widget:view(props)
        return iced.text("Count: " .. props.count)
    end

    return widget
end)

iced.counted({ count = self.count })
```

The name must be a valid identifier and must not already exist on `iced`. The
constructor takes only props. Events still go through `app:update`; there is no
per-widget update callback. The factory is not reloaded from a file — use a
component module when you need file-watch reloads.

### Components

See [`iced.component`](#icedcomponentid-module-props) for the component
reference and lifecycle hooks.

Editing a component module reloads it and every module that required it.
Returned module tables keep their identity, so a cached `require` result sees
updated methods. A separately cached function or value (for example
`local view = panel.view` in `shell.lua`) needs a shell reload. A broken save
keeps the previous modules and UI and is not retried until the file changes
again.

### Layer surfaces

See [`iced.layer`](#icedlayercontent-settings). Layer views can be returned from
`app:view` directly or through layer modules:

```lua
local background = require("components.layers.background")
function app:view(props)
    return background.view()
    -- or return { background.view(), panel.view() }
end
```

`view` also runs once for surface discovery, so keep it free of side effects.
`props.window` identifies the surface currently being rendered, not every layer
in the list.

### Input regions

See [`iced.region`](#icedregioncontent). A region applies per layer/window and
updates on redraw. Use one region per layer and wrap all interactive content in
it.

## Known limits

- `table` has no style prop in iced 0.14.
- `markdown` settings cover text size, spacing, link color, and inline-code
  color; text selection is not exposed.
- `canvas` supports `rect`, `circle`, `line`, `path`, and `text`; arbitrary Iced
  canvas programs are not supported.
- `responsive` runs Lua during layout; its callback must not mutate app state.
  Errors fall back to empty space.
- `pane_grid` delivers resize and drag events, but resize ratios cannot be set
  from Lua; the host keeps the live layout per `id` until a shell reload.
- `text_editor`, `combo_box`, and `pane_grid` state is keyed by `id` inside the
  process and is not shared between multiple `riced` instances.
- `list_view` diffing is length-based; middle reorders and removals are not
  detected.
- Generic transitions cannot fade a whole widget group. `svg` exposes its own
  `opacity`, and text color transitions include alpha; there is no group
  opacity.
- Standard Wayland protocols do not associate toplevels with workspaces.
- `pane_grid` styles and other widget behavior follow iced 0.14. A widget not
  listed in this document is not exposed.
