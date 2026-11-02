# riced

Lua-driven Iced Wayland layer-shell daemon.

Copy the contents of `example/` (including `components/`) into
`~/.config/riced/test/`, then run:

```sh
cargo run -- -p test
```

`riced -p test` loads `$XDG_CONFIG_HOME/riced/test/shell.lua`, falling back
to `$HOME/.config`. Missing files, Lua errors, and invalid settings fail with
a nonzero exit status. Profile names must be a single directory name.
`require` also searches the profile directory. Only run trusted Lua configs:
they have access to Lua's standard libraries, including filesystem and process APIs.

### Reload on save

`shell.lua` is checked every 250 ms. Saving changed content reloads the Lua app,
callbacks, theme, and subscriptions in place, resetting Lua state. Syntax errors
and errors found while validating existing views keep the current app running;
check the terminal for the error, fix it, and save again.

Lua modules loaded through `require` (including `iced.component` modules) are watched
too. Saving one reloads that module and its dependents without resetting app state.
Layer modules can be required either at the top of `shell.lua` or inside `app:view`.
Edits are detected from file contents; changing an `id` is not necessary.

Namespace and startup settings changes require a restart and are rejected during
reload. Callback errors first encountered after reload (for example, in `update`)
remain fatal. Config evaluation and validation can run Lua side effects even when
a reload is rejected.

### Editor support

`example/stubs/riced.lua` is a lua-language-server `---@meta` file for the
injected globals (`riced`, `iced`, `theme`, `config`). `example/.luarc.json`
loads it, so completions and undefined-global diagnostics work while editing a
profile copied from `example/`. Copy both, or point `workspace.library` at
`example/stubs`. Each profile needs its own `.luarc.json` because `require`
resolves from the profile directory. The stub is hand-written; update it when
the Lua API changes.

## Lua API

Return `riced.daemon(app, namespace, app.update, app.view)` with the chained
options shown in `example/shell.lua`, ending in `:run()`. Rust starts the event
loop after the script returns. Callbacks receive `app` as `self`:

| Callback       | Argument                                                                                                               | Return                                                      |
| -------------- | ---------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------- |
| `update`       | button's `on_press` value or `{ type = "tick" }`, then the id of the component that raised it (nil outside components) | nothing; mutate `self`                                      |
| `view`         | `{ window = <opaque window ID>, width, height }`                                                                       | an `iced` element, `iced.layer`, or a list of layers        |
| `title`        | window ID                                                                                                              | string or nil                                               |
| `subscription` | `true` (daemon running)                                                                                                | `iced.every(milliseconds)` or nil                           |
| `theme`        | window ID                                                                                                              | palette table or nil                                        |
| `style`        | `config.theme`                                                                                                         | `{ background_color = "#...", text_color = "#..." }` or nil |
| `redraw_scope` | event, before update                                                                                                   | `"all"`, `"none"`, a saved `props.window`, or nil           |

Options other than update/view are optional. `:theme(config.theme)` accepts
a table directly instead of a callback. `config.theme` and its alias `theme`
are globals with `background`, `text`, `primary`, `secondary`, `success`,
`warning`, and `danger` hex colors. Modify their fields in `shell.lua`.
`secondary` is available to widgets; Iced's base palette has no secondary field.
Runtime callback errors print the Lua traceback and terminate the process.

`props.width` and `props.height` are the surface's logical size, filled in once
the compositor reports a resize (nil on the first render). Use them to animate
toward the real width: `width = open and props.width or 30`.

### Elements

```lua
iced.text("Hello", {
    size = 24,
    color = theme.primary,
    line_height = 1.4,           -- or { pixels = 28 }
    wrap = "word",               -- none, glyph, word_or_glyph; false disables
    align_x = "center",          -- left, right, justified
    align_y = "center",          -- top, bottom
    shaping = "auto",            -- basic, advanced
})
iced.row({ child1, child2 }, { spacing = 8, align_y = "center", clip = true })
iced.column({ child1, child2 }, { spacing = 8, align_x = "center" })
iced.container(child, {
    padding = 16, width = "fill", height = "fill",
    align_x = "center", align_y = "center",
    max_width = 320, clip = true,
})
iced.button("Click", { on_press = "clicked" }) -- content may also be an element
iced.space(nil, { width = 12, height = 12 })
iced.image("/path/to/icon.png", { width = 24, height = 24, content_fit = "contain" })
iced.svg("/path/to/icon.svg", { width = 24, height = 24, opacity = 0.8 })
iced.rule(nil, { direction = "horizontal", thickness = 1, style = { color = theme.primary, radius = 2 } })
iced.progress_bar(nil, { value = 0.4, girth = 4, style = { bar = theme.primary } })
iced.qr_code("https://example.com", { cell_size = 4, style = { cell = "#111111" } })
iced.stack({ base, overlay })
iced.float(child, { scale = 1.1 })
iced.pin(child, { x = 8, y = 8 })
iced.tooltip(child, { tooltip = "hint", position = "bottom" })
iced.grid({ a, b }, { columns = 2, spacing = 4 })
iced.checkbox(nil, { value = self.on, label = "Enable", on_toggle = "toggle" })
iced.radio(nil, { value = 1, selected = self.mode, label = "One", on_select = "mode" })
iced.toggler(nil, { value = self.on, label = "On", on_toggle = "toggle" })
iced.slider(nil, { min = 0, max = 100, value = self.volume, on_change = "volume" })
iced.text_input(nil, { value = self.text, on_input = "text" })
iced.pick_list(nil, { options = { "a", "b" }, selected = self.pick, on_select = "pick" })
iced.scrollable(child, { direction = "vertical", id = "list", on_scroll = "scroll" })
iced.mouse_area(child, { on_enter = "hover", on_exit = "unhover" })
iced.sensor(child, { on_show = "shown" })
iced.text_editor(nil, { id = "editor", value = self.text, on_input = "text" })
iced.combo_box(nil, { id = "combo", options = { "a", "b" }, value = self.pick, on_select = "pick" })
iced.table({ { "a", "b" }, { "c", "d" } }, {
    columns = { { header = "one" }, { header = "two", width = 120 } },
    spacing = 1,
})
iced.markdown("# Heading\n\nBody", { text_size = 14, on_link_click = "link" })
iced.list_view(self.items, {
    id = "items",
    spacing = 8,
    render = function(item, index) return iced.text(item.name) end,
    on_item_added = function(index, item) self:item_added(item) end,
    on_item_removed = function(index, item) self:item_removed(item) end,
})
iced.responsive(function(size) return iced.text(tostring(size.width)) end)
iced.canvas({
    { type = "rect", x = 0, y = 0, width = 40, height = 20, color = theme.primary },
    { type = "circle", x = 60, y = 10, radius = 8, color = theme.danger },
    { type = "line", from = { x = 0, y = 30 }, to = { x = 80, y = 30 }, color = theme.text },
    { type = "text", x = 4, y = 4, content = "canvas", size = 12, color = theme.text },
})
iced.pane_grid({ pane_a, pane_b }, {
    id = "grid", direction = "horizontal", spacing = 4,
    on_click = "pane", on_resize = "resize", on_drag = "drag",
})
iced.region(iced.container(interactive_component, { width = 320, height = 40 }))
```

`align_x`/`align_y` on text apply when the text has a constrained width or
height, so pass `width`/`height` when aligning.
All elements accept `width`, `height` (`"fill"`, `"shrink"`, non-negative
pixels, or `iced.Length.fill_portion(3)`), and uniform `padding`.
`content_fit` is `contain` (default), `cover`, `fill`, `none`, or `scale_down`.
No `on_press` means a disabled button.
Property names are validated: an unknown or mistyped key is an error.
Not every iced widget is exposed; see the known limits below.

### Known limits

- `table` has no style prop: iced 0.14's `Table` does not expose one.
- `markdown` settings cover text size, spacing, link color, and inline-code
  color; text selection is not exposed.
- `canvas` supports `rect`, `circle`, `line`, `path`, and `text`; arbitrary
  iced `Program`s are not supported.
- `responsive` runs Lua during layout, so its callback must not mutate app
  state; errors fall back to an empty space.
- `pane_grid` delivers resize/drag/click events, but resize ratios are not
  persisted by the host; rebuild the grid with your own state if needed.
- `text_editor` state is keyed by `id` inside the process and is not shared
  between multiple `riced` instances.

### Widget events

`on_input`, `on_submit`, `on_toggle`, `on_change`, `on_release`, `on_select`,
`on_scroll`, `on_show`, `on_resize`, `on_move`, `on_action`, and
`on_link_click` deliver a table to `update`:

```lua
{ event = "text", widget = "search", value = "abc" }
```

`value` depends on the widget: boolean, number, string, `{ x, y }`, or
`{ width, height }`. `on_scroll` carries a viewport table with `x`, `y`,
`relative_x`, `relative_y`, `bounds_width`, `bounds_height`, `content_width`,
and `content_height`. `on_action` from `text_editor` also carries `text`; its
`value` is the action name (`edit`, `move`, `select`, `click`, `drag`, or
`scroll`). `on_link_click` delivers the URL as `value`. `widget` is set when a
component raised the event. Plain events (`on_press`, `on_enter`, `on_exit`,
right/middle clicks) keep the existing value behaviour. `pane_grid` sends the
pane index on click, the new ratio on resize, and `value` (`picked`, `dropped`,
or `canceled`) plus `pane` on drag.

`style` is optional and omitted fields come from the Iced theme. The keys below go
inside the `style` table, not directly on the widget:
`background`, `text_color`, `border = { color, width, radius }`, and
`shadow = { color, offset = { x, y }, blur }`. `container`, `button`,
`checkbox`, `radio`, `toggler`, `slider`, `text_input`, `text_editor`,
`combo_box`, `scrollable`, and `pick_list` accept the shared subset they can
represent. Buttons accept `active`, `hovered`, `pressed`, and `disabled`;
checkboxes, togglers, sliders, text inputs, and combo boxes accept the state
keys they have. Additional keys: `icon_color` (checkbox, text input),
`dot_color` (radio), `foreground` (toggler), `rail`/`filled`/`handle`/
`handle_border` (sliders), `placeholder`/`selection` (text inputs and combo
boxes), `rail`/`scroller` (scrollable), `handle` (pick list).
`rule` accepts `{ color, radius, snap }`; `progress_bar` accepts
`{ background, bar, border }`; `qr_code` accepts `{ cell, background }`;
`markdown` accepts `{ link_color, inline_code_color }`.

Style support matrix:

| Widget                                                     | Keys                                                                           | State overrides                            |
| ---------------------------------------------------------- | ------------------------------------------------------------------------------ | ------------------------------------------ |
| `container`                                                | `background`, `text_color`, `border`, `shadow`                                 | —                                          |
| `button`                                                   | `background`, `text_color`, `border`, `shadow`                                 | `active`, `hovered`, `pressed`, `disabled` |
| `checkbox`                                                 | `background`, `text_color`, `border`, `icon_color`                             | `active`, `hovered`, `disabled`            |
| `radio`                                                    | `background`, `text_color`, `border`, `dot_color`                              | `active`, `hovered`                        |
| `toggler`                                                  | `background`, `text_color`, `border`, `foreground`                             | `active`, `hovered`, `disabled`            |
| `slider`, `vertical_slider`                                | `rail`, `filled`, `border`, `handle`, `handle_border`                          | `active`, `hovered`, `dragged`             |
| `text_input`, `combo_box`                                  | `background`, `text_color`, `border`, `placeholder`, `selection`, `icon_color` | `active`, `hovered`, `focused`, `disabled` |
| `text_editor`                                              | `background`, `text_color`, `border`, `placeholder`, `selection`               | `active`, `hovered`, `focused`, `disabled` |
| `scrollable`                                               | `background`, `border`, `shadow`, `rail`, `scroller`                           | `active`, `hovered`, `dragged`             |
| `pick_list`                                                | `background`, `text_color`, `border`, `placeholder`, `handle`                  | `active`, `hovered`, `opened`              |
| `rule`, `progress_bar`, `qr_code`, `markdown`, `pane_grid` | see above                                                                      | —                                          |

### Tasks

`update` may return a task or a list of tasks:

```lua
return {
    { type = "focus", id = "search" },
    { type = "clipboard_write", content = self.text },
    { type = "scroll_to", id = "list", x = 0, y = 100 },
    { type = "font", path = "/path/to/font.ttf" },
    { type = "window", window = props.window, action = "resize", width = 800, height = 600 },
    -- action is resize, move, close, fullscreen, or windowed
    { type = "clipboard_read" },
    { type = "exit" },
}
```

A clipboard read delivers `{ type = "clipboard", value = <string or nil> }` to
`update`. `text_editor`, `combo_box`, and `pane_grid` are stateful and require
a non-empty `id`; their state is reset when its id changes or `riced` reloads
the shell.

### Subscriptions

`subscription` may return one subscription or a list:

```lua
function app:subscription(active)
    return { iced.every(1000), iced.frames(), iced.on_key(true), iced.on_mouse() }
end
```

`iced.frames()` sends `{ type = "frame" }`, `iced.on_key(pressed)` sends
`{ type = "key", event = "press"|"release", key, text, modifiers = { shift, control, alt, logo } }`,
and `iced.on_mouse()` sends `{ type = "mouse", event = ..., ... }`.

### List views

`iced.list_view(items, props)` renders a vertical list (or `direction = "horizontal"`)
of elements. `render` turns each item into an element. With `id` and callbacks, the
widget tracks the previous item count: `on_item_added(index, item)` fires for appended
items and on the first render, and `on_item_removed(index, item)` fires when trailing
items disappear. `onItemAdded`/`onItemRemoved` are accepted as aliases.

`transition = true` (or a table) scales appended items in and removed items out:

```lua
local items = iced.list_view(self.items, {
    id = "items",
    transition = {
        duration = 180,
        x = { from = -180, to = 0 },
        onAdded = function(index, item) end,
        onRemoved = function(index, item) end,
    },
    render = function(item) return iced.text(item.name) end,
})
```

The transition animates drawing only; the item still occupies layout space while it
scales out and is removed after the duration. `x` slides the item from the given
offset to `0` on enter and back on leave; both direct lifecycle callbacks and
`transition.onAdded`/`transition.onRemoved` are supported. Callbacks run during
render, so state they change takes effect on the next render. Reordering or removing
from the middle is not diffed.

### Custom widgets

`iced.define(name, factory)` creates a reusable widget constructor. The factory runs
once; its returned table needs a `view` method (or a `render` function):

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

The widget can keep private state in the closure. Events still go through
`app:update`; there is no separate per-widget update callback. The factory is not
reloaded from a file — use a component module when you need file-watch reloads.

### Transitions

`transition` springs or eases an element's props toward new values instead of
jumping: `width`, `height`, `padding`, `size` (text), `spacing` (row/column),
and `color` (text, including alpha). `"fill"`, `"shrink"`, and fill portions are
not interpolated.

```lua
iced.container(child, { width = self.width, padding = 16, transition = true })
```

The table form picks props and timing. Defaults are a smooth spring over every
animatable prop present on the element:

```lua
transition = { props = { "width", "padding" }, duration = 300, easing = "out" }
transition = { spring = "bouncy" } -- smooth (default), snappy, bouncy, instant
```

`easing` is `linear`, `ease`, `in`, `out`, or `in_out` (an `ease_` prefix is also
accepted); `duration` is whole milliseconds and applies to either mode. Easing and
spring are mutually exclusive. All selected props share one timing; iced_anim
cannot run different easing curves on one element. The element is rebuilt from its
Lua table on every animation frame, so keep animated subtrees cheap.

### Value helpers

Helpers produce the same strings/tables as handwritten settings; both forms work.

```lua
iced.layer(content, {
    size = iced.Size(iced.Extent.fill, iced.Extent.fill), -- or integer pixels per axis
    layer = iced.Layer.background, -- bottom, top, overlay
    keyboard_interactivity = iced.KeyboardInteractivity.none, -- on_demand, exclusive
    output_option = iced.Output.active, -- last_output; output names/IDs remain plain values
    blur_option = iced.Blur.none, -- full; regions remain a list of tables
})
```

Widget dimensions accept `iced.Length.fill`, `iced.Length.shrink`, pixel numbers,
or `iced.Length.fill_portion(n)` (integer 1–65535). Weighted fill is only for widget
layout, not layer-surface dimensions. `iced.Size(width, height)` requires both axes;
Rust still validates layer dimensions, anchors, and other settings.

### Files

`riced.file` loads and saves JSON or TOML. Paths are relative to the profile
directory. `type` is optional when the extension is `.json` or `.toml`.

```lua
local settings = riced.file({
    path = "data/settings.json",
    type = "json",
    onLoaded = function(file, data)
        -- data is file.data; mutate it and call save
    end,
    onError = function(file, error)
        print(error)
    end,
})

settings.data.theme = "dark"
settings:save()          -- true on success, or false + error string
settings:write({ ... })  -- replace data and save
```

`file.data` is the loaded table, `file.error` is the last load/save error or
nil. `write(value)` replaces `data` and saves; `save()` writes the current
`data`. Missing files are created as an empty JSON/TOML document and
`onLoaded` receives an empty table; missing parent directories are created.
Writes are atomic (temporary file + rename). File callbacks run during
construction. TOML requires a table at the top level and cannot encode nil
values.

### Components

`iced.component(id, module, props)` swaps in a module's `view` output and watches
that module's file for edits. Saving the module reloads components that use it and
redraws only their windows; rest of the UI keeps running.

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

- The module is required by name from `package.path` and must return a table with
  a `view` method. Write it with `function panel:view(props)`: `self` is the module
  table, so module-level constants stay reachable.
- `props` defaults to `{}`. The third argument is the window currently rendering.
- Component `id`s must be nonempty, unique within one window, and free of cycles.
  The id identifies an instance; the module name identifies the file that reloads.
- Props are rebuilt on every render, so state-driven UI stays live. Module locals
  are recreated when the module reloads; keep state in `app` and pass it as props.
- Editing a module reloads it and every module that `require`d it, including
  dependencies loaded before components mount. Returned module tables keep their
  identity so `local panel = require(...)` sees updated methods. A separately cached
  function/value (e.g. `local view = panel.view` in `shell.lua`) needs a shell reload.
- A broken save keeps the previous modules and UI, and is not retried until the
  file changes again.
- Events raised by a component are dropped once its module has reloaded again.
- Component modules may return any element. Return `iced.layer` at the root of
  `app:view`, not from inside a component.

### Per-component state and lifecycle

`app:update` receives the id of the component that raised the event as a second
argument, so one callback can keep state for all components:

```lua
function app:update(event, component)
    if component == "panel" and event == "toggle" then
        self.panel.open = not self.panel.open
    end
end
```

Pass that state back through `iced.component(id, module, { ... })`; props are
rebuilt every render.

A module may define two optional hooks, called as methods with the props that
were last rendered and the window:

```lua
function panel:on_visible(props, window) ... end
function panel:on_destroyed(props, window) ... end
```

- `on_visible` fires once when an `id` first appears in a window's rendered tree,
  and again after a shell reload (Lua state resets; every component remounts).
  It means "mounted and rendered", not "in the viewport": Iced exposes no
  visibility signal.
- `on_destroyed` fires when an id stops appearing in a window that renders. The
  component's props are no longer available from `app:view`, so use the `props`
  argument to clear the state keyed by them.
- Hooks run during render. State they change takes effect on the next render.
  A component dropped by closing its layer surface does not fire `on_destroyed`;
  the surface teardown removes the window without another render.
- Hook errors are fatal, like `view` errors. On a rejected reload the mount table
  is restored, but side effects a hook already made are not.

### Extensions

Extensions are separate executables that connect back to `riced` over a Unix socket.
`riced` binds `$XDG_RUNTIME_DIR/riced/<profile>.sock` and spawns every executable in
`<profile>/extensions/` (an executable file, or a directory containing one). A crashed
extension is restarted with backoff; removing the file stops it. Extensions can also be
started manually with `RICED_SOCKET` set.

An extension publishes a watched snapshot (`store`) and may declare methods. Lua reads
`store` synchronously, so `view` stays side-effect free:

```lua
local system = riced.plugin("system")
local cpu = system.cpu -- same as system.store.cpu
if cpu then
    iced.text(string.format("CPU %.0f%%", cpu.usage or 0))
end
```

`riced.plugin(name)` and `riced.plugins(name)` are the same function. Unknown keys on
the proxy read through to `store`, so `plugin.cpu` and `plugin.store.cpu` are
equivalent; declared methods keep their names.

Store fields are lazy: reading a key that is not published yet subscribes to it, and the
extension may compute only the requested fields. Requests are monotonic for the process
lifetime (there is no unsubscribe when a component stops using a field).

A lazy field reads as `nil` until the extension answers the first request, so guard
every access (`local cpu = system.cpu; if cpu then ... end`). `alive` alone is
not enough: the proxy can be alive while a newly requested field is still pending.

`riced.plugin(name)` always returns a proxy table. Before the extension registers,
`alive` is `false` and `store` is empty; the same table is filled when it connects, so
capturing it at module load is safe. The snapshot is replaced whenever the extension
publishes and the table identity is kept, so a cached `system.store` reference stays
valid. `alive` is `false` after a disconnect and the last snapshot is kept.

A lazy field reads as `nil` until the extension answers the first request, so guard
every access (`local cpu = system.store.cpu; if cpu then ... end`). `alive` alone is
not enough: the proxy can be alive while a newly requested field is still pending.

`riced.plugins(name)` always returns a proxy table. Before the extension registers,
`alive` is `false` and `store` is empty; the same table is filled when it connects, so
capturing it at module load is safe. The snapshot is replaced whenever the extension
publishes and the table identity is kept, so a cached `system.store` reference stays
valid. `alive` is `false` after a disconnect and the last snapshot is kept.

Methods are asynchronous. Call them with `.` (not `:`) and pass a Lua function as the
last argument to receive the result:

```lua
system.refresh()
system.query("memory", function(result, err) ... end)
```

Extensions may also push events, which arrive in `update`:

```lua
function app:update(event)
    if event.type == "plugin" and event.plugin == "system" then
        self.value = event.event.value
    end
end
```

Build and install the example (`crates/riced-extension-sdk` + `sysinfo`):

```sh
just extension-system
# -> ~/.config/riced/test/extensions/system/riced-system
```

The SDK is `crates/riced-extension-sdk`; third-party extensions depend on it by git or
path, build their own binary, and never need `riced` rebuilt. Protocol v1 is
newline-delimited JSON, 256 KiB per line, with bounded queues. Extensions run as your
user and have the same trust as `shell.lua`; the socket directory is `0700` and the
socket is `0600`.

#### Wayland extension

`riced-wayland` is the generic compositor extension. It uses only standard protocols:
`wl_output`, `ext-workspace-v1`, and `zwlr-foreign-toplevel-management-v1` (falling
back to read-only `ext-foreign-toplevel-list-v1`). Compositors without a protocol get
empty data and the matching `capabilities` flag is `false`; nothing is Hyprland-specific.

```lua
local wayland = riced.plugin("wayland")
if wayland.connected then
    for _, toplevel in ipairs(wayland.toplevels or {}) do
        iced.text(toplevel.title)
    end
    for _, workspace in ipairs(wayland.workspaces or {}) do
        iced.button(workspace.name, {
            on_press = workspace.active and nil or ("workspace:" .. workspace.id),
        })
    end
end
```

Store fields:

| Field              | Shape                                                                                                                                    |
| ------------------ | ---------------------------------------------------------------------------------------------------------------------------------------- |
| `connected`        | boolean; false while the compositor connection is down                                                                                   |
| `capabilities`     | `{ toplevels, toplevel_control, workspaces }` booleans                                                                                   |
| `outputs`          | `{ id, name, description, make, model, x, y, width, height, scale }`                                                                     |
| `toplevels`        | `{ id, title, app_id, outputs, activated, maximized, minimized, fullscreen }`; the read-only fallback also has `identifier` and no state |
| `workspaces`       | `{ id, protocol_id, name, coordinates, active, urgent, hidden, capabilities, group, outputs }`                                           |
| `workspace_groups` | `{ id, capabilities, outputs }`                                                                                                          |
| `focused`          | `{ toplevel, workspace, output }` ids or names, `nil` when unknown                                                                       |

Methods (fire-and-forget): `activate_toplevel(id)`, `close_toplevel(id)`, and
`activate_workspace(id)`. `capabilities.toplevel_control` is false when only the
read-only toplevel list is available; workspace activation needs
`ext-workspace-v1` and the workspace's `capabilities` list.

Standard protocols do not associate toplevels with workspaces; use the compositor's
own extension if you need that mapping.

```sh
just extension-wayland
# -> ~/.config/riced/test/extensions/wayland/riced-wayland
```

#### D-Bus extension

`riced-dbus` provides StatusNotifier tray items and an `org.freedesktop.Notifications`
server. It owns `org.kde.StatusNotifierWatcher` when the name is free; otherwise it
registers as a host with the existing watcher. It owns the notification name only if
no other notification daemon already does.

```lua
local dbus = riced.plugin("dbus")

for _, item in ipairs(dbus.tray or {}) do
    iced.text(item.title)
end

for _, notification in ipairs(dbus.notifications or {}) do
    iced.text(notification.summary .. " — " .. notification.body)
end

dbus.activate_tray(item.id)
dbus.context_menu_tray(item.id)
dbus.close_notification(notification.id)
```

Store fields:

| Field | Shape |
| --- | --- |
| `connected` | boolean; session bus connection |
| `capabilities` | `{ tray, tray_host, notifications }` booleans |
| `tray` | `{ id, title, icon, status, menu, item_is_menu }` |
| `notifications` | `{ id, app_name, app_icon, summary, body, actions, timeout, timestamp }` |

`actions` is a list of `{ key, label }`. Methods: `activate_tray(id)`,
`secondary_activate_tray(id)`, `context_menu_tray(id)`,
`close_notification(id)`, `invoke_notification_action(id, key)`.

Tray icons are freedesktop icon names, not paths; resolving them to an image is
left to the profile. Notifications expire according to the sender's timeout and
disappear from the store.

```sh
just extension-dbus
# -> ~/.config/riced/test/extensions/dbus/riced-dbus
```

### Input regions

`iced.region(content)` makes the window's Wayland input region match the wrapped
element's real layout rectangle, so clicks outside it pass through the layer. Bounds
are updated on redraw. Use one region per layer and wrap all interactive content in
it:

```lua
iced.region(iced.container(iced.component("panel", "components.panel"), {
    width = 320,
    height = 40,
}))
```

The region is applied to the surface that renders the widget; it does not work for
nested windows or components rendered in another layer.

### Layer surfaces

`iced.layer(content, settings)` describes a separate Wayland layer-shell surface,
not a widget inside another window. A component module can return one:

```lua
-- components/layers/background.lua
local background = {}
function background.view()
    return iced.layer(iced.text("Background"), {
        id = "background",
        size = { width = "fill", height = "fill" },
        layer = "background",
        anchor = { "top", "bottom", "left", "right" },
        exclusive_zone = -1,
        margin = { 0, 0, 0, 0 }, -- top, right, bottom, left
        keyboard_interactivity = "none",
        output_option = "active",
        events_transparent = true, -- click through
        namespace = "riced-background",
        blur_option = "none",
    })
end
return background
```

```lua
local background = require("components.layers.background")
function app:view(props)
    return background.view()
    -- Or: return { background.view(), panel.view() }
end
```

All settings are optional and default to `NewLayerShellSettings` defaults:
fill size, all anchors, top layer, no exclusive zone, zero margins, on-demand
keyboard, active output, normal input, inherited namespace, no blur.

- `size`: `{ width = "fill" | positive_integer, height = "fill" | positive_integer }`.
  A fill axis requires both opposite anchors; pixels must fit a signed 32-bit integer.
- `layer`: `"background"`, `"bottom"`, `"top"`, or `"overlay"`.
- `anchor`: a list of `"top"`, `"bottom"`, `"left"`, `"right"`; `{}` centers an exact-sized layer.
- `exclusive_zone`: integer >= -1, or nil; `margin`: four signed integers, or nil.
- `keyboard_interactivity`: `"none"`, `"on_demand"`, or `"exclusive"`.
- `output_option`: `"active"`, `"last_output"`, output name (e.g. `"DP-1"`), or a positive
  Wayland registry global ID. Raw Wayland output objects are not exposed to Lua.
- `events_transparent`: boolean; `namespace`: nonempty string or nil.
- `blur_option`: `"none"`, `"full"`, or a list of `{ x = 0, y = 0, width = 100, height = 100 }`
  regions with positive width/height. Blur requires compositor support.

Layers are created without an extra startup surface. Saving a layer module updates
changed size/anchors, layer level, margins, exclusive zone, keyboard mode and blur
through native actions on the existing surface. Output, namespace and input
transparency changes recreate only the affected surface (possibly briefly flickering).
Adding/removing layers in a nonempty layer list also creates/closes their surfaces.
An optional `id` preserves identity across reordering; without it identity follows
list position. Renaming/removing an id alone does not require surface recreation;
unmatched layers reuse available surfaces, preferring their list position.
Switching from layer views to a widget-only view still requires restart.
Keep `view` free of side effects:
it also runs once for surface discovery, and builds the layer list for each window.
`props.window` identifies the surface currently being rendered, not every layer in the list.
Layer descriptors must be returned at the root, not nested inside widget containers.

### Settings

Supported keys: `id`, `layer_settings.start_mode` (`"background"`, `"active"`,
`"all_screens"`), and `layer_settings.keyboard_interactivity` (`"none"`,
`"on_demand"`, `"exclusive"`). Unknown settings are errors.
`"background"` starts without a surface. Use `"active"` for visible widget-only views;
`iced.layer` views create their own surfaces regardless of this setting.

Building requires the native Wayland/XKB development libraries, including
`xkbcommon` discoverable by `pkg-config`. Running requires a Wayland compositor
supporting layer-shell.
