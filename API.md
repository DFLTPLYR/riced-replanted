# riced API reference

This is the complete Lua reference for a `riced` profile. It covers the injected
`riced` and `iced` globals, daemon callbacks, widgets, events, tasks,
subscriptions, files, and extensions. For installation, profile layout, reload
behavior, and examples, see [README.md](README.md).

The injected API is defined in `src/lua/`, `src/file.rs`, `src/plugins.rs`, and
`src/widgets/`. Core editor definitions live in `example/stubs/riced.lua`;
extension definitions live in each extension's `stub.lua`. Update them when the
Lua API changes.

## Contents

- [Conventions](#conventions)
- [riced API](#riced-api)
  - [`log`](#log)
  - [`riced.daemon`](#riceddaemon)
  - [Daemon callbacks](#daemon-callbacks)
  - [`riced.ipc`](#ricedipc)
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
  - [Popup surfaces](#popup-surfaces)
  - [Native windows](#native-windows)
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
  `iced` constructors, `iced.layer`, `iced.popup`, `iced.component`, or return them
  from `app:view`.
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

`iced.fonts()` returns a sorted, unique array of family names from the renderer's
font database. It includes system fonts and any fonts already loaded into the
renderer. Pass a returned name as the `font` prop.

### Icon themes and lookup

Set `iced.icon_theme` to an installed theme's **directory name**. Every
`iced.icon_path` call reads the current value; changing it affects the next view
rebuild. It defaults to `"hicolor"` and does not change the desktop's GTK settings.
`iced.icon_themes()` returns a sorted, unique array of installed theme directory
names, suitable for assigning to `iced.icon_theme`.

```lua
iced.icon_theme = "Papirus-Dark"
local themes = iced.icon_themes()
local path = iced.icon_path("firefox", { size = 24, scale = 1, thumbnail = true })
local icon = iced.text("?")
if path then
    local image = path:lower():match("%.svg$") and iced.svg or iced.image
    icon = image(path, { width = 24, height = 24 })
end
```

`iced.icon_path(name, options?)` returns an existing file path or nil:

- `name`: an icon name (optionally ending in `.png`/`.svg`) or an absolute file path.
- `size`: whole logical pixels, 1–4096; default 24.
- `scale`: whole display scale, 1–16; default 1.
- `path`: optional absolute application icon directory, such as a tray item's
  `icon_theme_path`. Direct files in this directory take precedence over themed
  icons. An empty directory string is treated as absent.
- `thumbnail`: when true, raster icons are decoded and downscaled to `size *
  scale` pixels (capped at 4096) and cached under `$XDG_CACHE_HOME/riced/icons`
  (default `~/.cache/riced/icons`). SVG icons and decode failures return the
  original path. Repeated lookups reuse the cached file until the source
  changes; the cache keeps at most 256 files and removes the oldest.

Theme lookup searches `~/.icons`, `$XDG_DATA_HOME/icons` (default
`~/.local/share/icons`), and each `$XDG_DATA_DIRS/icons` (default
`/usr/local/share/icons:/usr/share/icons`). It honors fixed/scalable/threshold
sizes and scale, then chooses the nearest size in that theme. Inherited themes
are searched in order, with cycle protection, followed by `hicolor`, unthemed
icons, and the corresponding XDG `pixmaps` directories. This includes Nix
profile paths when present in `XDG_DATA_DIRS`.

Named lookup supports PNG and SVG. Absolute paths are returned unchanged if
they exist; use a renderer that supports their format. Invalid arguments raise
Lua errors. Results are cached by theme/name/size/scale/application directory;
reload `shell.lua` after installing or replacing icons to refresh the cache.
`iced.icon` continues to render a Lucide glyph; it is separate from theme lookup.

## riced API

### `log`

`log(...)` is a global callable available in profiles, modules, and callbacks.
It writes an info-level message through the shared logger to stderr with the
event label `[lua]`. Values use Lua `tostring` (including `__tostring`) and are
separated by tabs, like `print`. It returns no values; `log()` emits an empty
message. Errors from `tostring` propagate to the caller.

`log.table(...)` behaves the same, but expands table arguments recursively
instead of using `tostring`. Keys are sorted, sequences print one item per
line, and cycles print as `table: 0x...`. Non-table values still use
`tostring`:

```lua
riced.ipc({
    settings = function(self, args)
        if #args ~= 0 then
            error("usage: riced ipc settings")
        end
        log("called") -- [lua]: called
        log.table(self) -- [lua]: {
        --   settings = false,
        -- }
        self.settings = not self.settings
    end,
})
```

The `[lua]` label is green on a terminal. Redirected output stays plain, and
`NO_COLOR` disables coloring. Standard Lua `print` still writes to stdout.

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

| Argument    | Type     | Required | Meaning                                                   |
| ----------- | -------- | -------- | --------------------------------------------------------- |
| `app`       | table    | yes      | State table passed as `self` to every callback            |
| `namespace` | string   | yes      | Nonempty Iced theme namespace and surface namespace       |
| `update`    | function | yes      | Event handler, called as `update(self, event, component)` |
| `view`      | function | yes      | Renderer, called as `view(self, props)`                   |

`riced.daemon` returns a builder. Chain any of the options below and finish with
`:run()`. `run()` must be called exactly once. Rust starts the event loop after
`shell.lua` returns.

| Builder method         | Value                                          |
| ---------------------- | ---------------------------------------------- |
| `:title(value)`        | string, or `function(self, window) -> string?` |
| `:subscription(value)` | `function(self, active) -> subscription(s)?`   |
| `:theme(value)`        | palette table, or `function(self, window)`     |
| `:style(value)`        | `function(self, palette) -> style table?`      |
| `:redraw_scope(value)` | `function(self, event) -> scope`               |
| `:settings(value)`     | settings table                                 |
| `:on_new_shell(value)` | `function(self, info) -> event?`               |

Every callback except `update` and `view` is optional.

### Daemon callbacks

| Callback       | Signature                  | Return                                                                           |
| -------------- | -------------------------- | -------------------------------------------------------------------------------- |
| `update`       | `(self, event, component)` | `nil`, a task, or a list of tasks                                                |
| `view`         | `(self, props)`            | element, `iced.layer`/`iced.window`/`iced.popup`, or a nonempty list of surfaces |
| `title`        | `(self, window)`           | string or `nil`                                                                  |
| `subscription` | `(self, active)`           | subscription, list of subscriptions, or `nil`                                    |
| `theme`        | `(self, window)`           | palette table or `nil`                                                           |
| `style`        | `(self, palette)`          | `{ background_color?, text_color? }` or `nil`                                    |
| `redraw_scope` | `(self, event)`            | `"all"`, `"none"`, `props.window`, or `nil`                                      |

`component` is the component id that raised the event, or `nil` for global
events. See [Widget events](#widget-events).

`props` passed to `view` has:

| Field    | Type            | Meaning                                                          |
| -------- | --------------- | ---------------------------------------------------------------- |
| `window` | opaque userdata | Surface identifier for this render; use with window tasks        |
| `width`  | number or nil   | Current logical surface width; available after the first resize  |
| `height` | number or nil   | Current logical surface height; available after the first resize |
| `shell`  | string or nil   | `"layer"`, `"popup"`, `"window"`, `"input_panel"`, or `"session_lock"` |

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

### Shell surfaces

`:on_new_shell(function(self, info) return event end)` runs for every surface
the runtime creates, synchronously before that surface's first frame. `info`
has:

| Field    | Type            | Meaning                                                        |
| -------- | --------------- | -------------------------------------------------------------- |
| `window` | opaque userdata | Surface identifier; the same value as `props.window`           |
| `shell`  | string          | `"layer"`, `"popup"`, `"window"`, `"input_panel"`, or `"session_lock"` |

Return an event to deliver it to `update` before the first view, or `nil`.
`props.shell` in `view` is the same string.

Declared surfaces (`layer`, `popup`, `window`) still return surface descriptors
from `view`. `input_panel` and `session_lock` surfaces must return an element
instead; it is rendered to fill the surface:

```lua
function app:view(props)
    if props.shell == "session_lock" then
        return lock.view(props)
    end
    return { bar.view(props) }
end
```

Request a session lock with the `lock` task and release it with `unlock`:

```lua
return { type = "lock" }
```

`lock` is a request. If the compositor denies it or lacks session-lock support,
no surface appears; this API does not report the denial.

When an `input_panel` or `session_lock` surface closes, `update` receives
`{ type = "shell_closed", window = <props.window>, shell = <string> }`.

### `riced.ipc`

Register commands that can be sent to the running daemon:

```lua
riced.ipc({
    settings = function(self, args)
        if #args ~= 0 then
            error("usage: riced ipc settings")
        end
        self.settings_open = not self.settings_open
    end,
    add = function(self, args)
        return self:add_bar(args)
    end,
})
```

```sh
riced ipc settings
riced ipc -c work add --anchor top --output DP-1
riced ipc -p ~/path/config settings
```

Handlers receive the daemon's state table as `self` and the command-line
arguments as a 1-based Lua array, including flags. They run on the daemon's Lua
thread. A returned task or list of tasks behaves like an `update` return value;
windows redraw after the handler finishes. Handler errors and unknown commands
are printed on the daemon's stderr and do not stop it. Registering a name again
replaces the previous handler, so editing the module that registers handlers
keeps working. An empty command name is an error.

The CLI only queues the request, so success means it was queued, not that a
daemon handled it. Each profile has its own queue; starting a new daemon discards
requests left by the previous instance. Without `-c` or `-p`, the CLI targets
the only running profile discovered through `$XDG_RUNTIME_DIR/riced/*.sock`;
with several running profiles it asks for `-c <name>` or `-p <directory>`.
Selectors go before the command name; everything after it is passed to the
handler unchanged. To target the default configuration explicitly, use
`riced ipc -p ~/.config/riced settings` (adjust for `$XDG_CONFIG_HOME` if set).
Directory aliases share an IPC destination; directories with equal basenames
have distinct destinations.

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

| Option     | Type     | Required | Meaning                                              |
| ---------- | -------- | -------- | ---------------------------------------------------- |
| `path`     | string   | yes      | Nonempty absolute path or profile-relative path      |
| `type`     | string   | no       | `"json"` or `"toml"`; inferred from the extension    |
| `onLoaded` | function | no       | `(file, data)` after a successful load/create        |
| `onError`  | function | no       | `(file, error)` after a read, create, or parse error |

`riced.file` returns a userdata with:

| Field/method         | Type               | Meaning                                         |
| -------------------- | ------------------ | ----------------------------------------------- |
| `path`               | string             | Resolved path                                   |
| `type`               | string             | `"json"` or `"toml"`                            |
| `data`               | any                | Loaded value; assign to replace it              |
| `error`              | string or nil      | Last load/save error                            |
| `file:save()`        | `boolean, string?` | Writes `data`; returns `true` or `false, error` |
| `file:write(value?)` | `boolean, string?` | Replaces `data` when provided, then saves       |

Missing files are created before `onLoaded` runs: `{}` for JSON and an empty
document for TOML. Missing parent directories are created. Writes are atomic
(temporary file plus rename). Existing symbolic links are preserved and their
destinations are updated; dangling links return an error. File errors are
returned, not raised; `file.error` is updated.

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
safe. `iced.plugin(name)` and `iced.plugins(name)` are aliases for the same
lookup.

```lua
local wayland = riced.plugin("wayland")

if wayland.connected then
    for _, toplevel in ipairs(wayland.toplevels or {}) do
        iced.text(toplevel.title)
    end
end
```

| Field   | Type    | Meaning                            |
| ------- | ------- | ---------------------------------- |
| `name`  | string  | Extension name                     |
| `alive` | boolean | Whether the extension is connected |
| `store` | table   | Last published snapshot            |

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

#### Bundled extension APIs

| Extension   | Store fields                                                                                     | Methods                                                                                                                                                                     | Events |
| ----------- | ------------------------------------------------------------------------------------------------ | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------ |
| `system`    | `host`, `time`, `cpu`, `memory`, `disks`, `gpus`, `processes`                                    | none                                                                                                                                                                        | none   |
| `wayland`   | `connected`, `capabilities`, `outputs`, `toplevels`, `workspaces`, `workspace_groups`, `focused` | `activate_toplevel`, `close_toplevel`, `activate_workspace`                                                                                                                 | none   |
| `dbus`      | `connected`, `capabilities`, `tray`, `tray_menu`, `notifications`                                | `activate_tray`, `secondary_activate_tray`, `context_menu_tray`, `open_tray_menu`, `close_tray_menu`, `tray_menu_event`, `close_notification`, `invoke_notification_action`, `pick_file` | none   |
| `themeagen` | `ready`, `mode`, `source`, `colors`                                                              | `generate`, `populate`, `regenerate`                                                                                                                                        | none   |
| `lucide`    | `count`                                                                                          | `resolve`, `exists`, `list`                                                                                                                                                 | none   |
| `openmeteo` | `ready`, `latitude`, `longitude`, `current`, `current_units`, `timezone`, `updated_at`, `error`  | `configure`                                                                                                                                                                 | none   |
| `applications` | `applications`                                                                                | `launch`                                                                                                                                                                    | none   |

All bundled extensions publish whole snapshots rather than pushing events.
Each publish replaces the snapshot contents, so re-read a field in `view`
instead of caching a nested table across updates; the proxy table itself stays
live. Store reads are synchronous and subscribe to fields the first time Lua
reads them. JSON `null` fields are exposed as `nil`, and `null` array entries are
removed. Methods are asynchronous and use positional arguments. `system` has no
methods. Wayland, most D-Bus methods, and `applications.launch` return no value
on success, so their callback receives `(nil, nil)`; `dbus.pick_file` returns
selected paths and themeagen methods return a result table. On failure every
method calls its callback with `(nil, error)`. Without a callback, method
failures are not surfaced to Lua. Use a callback when an action can fail. `?`
below means the field or value may be absent or `nil`.

Methods exist only after the extension connects. During the first style pass
(before the host polls the extension), proxy methods are `nil`, so guard calls
made from `app:theme` with `plugin.alive`.

#### Themeagen extension

`riced-themeagen` builds Material You palettes from a seed color or image and
renders Matugen-style `[templates.*]` entries. It reads the existing template
directory by default:

- config: `<profile>/data/templates/config.toml`
- state: `<profile>/data/theme/themeagen.json`

`<profile>` is the directory riced was started from, e.g.
`~/.config/riced/replanted` for `riced -c replanted`; standalone extensions
fall back to `$XDG_CONFIG_HOME/riced/data/...`.

`RICED_THEMEAGEN_CONFIG` and `RICED_THEMEAGEN_STATE` override those paths.
Relative `input_path` and `output_path` values resolve against the config
directory; `~/` in `output_path` expands to `$HOME`.

```lua
local theme = iced.plugins("themeagen") -- same proxy as riced.plugins
local colors = theme.colors

if colors then
    local active = colors[theme.mode]
    iced.text("theme", { color = active.primary })
end
```

Published state:

| Field    | Type         | Meaning                                                                                                              |
| -------- | ------------ | -------------------------------------------------------------------------------------------------------------------- |
| `ready`  | boolean      | A saved palette loaded and can be used                                                                               |
| `mode`   | string       | Active mode, `"dark"` or `"light"`                                                                                   |
| `source` | table or nil | How the palette was produced: `{kind="seed", seed, variant}`, `{kind="image", image, variant}`, or `{kind="manual"}` |
| `colors` | table or nil | Both palettes, keyed by `dark` and `light`; nil before the first palette exists                                      |

Before a palette exists, `ready` is false, `mode` is `dark`, and `source` and
`colors` are nil. Methods are asynchronous; the last argument is a callback
`(result, error)`.
`result` contains `mode`, `source`, `colors`, `applied`, and `errors`. `applied`
counts written templates; `errors` lists template or hook failures. `colors` is
also published as `theme.colors`. Use `result.colors` inside the callback when
you need the new palette immediately; the published `theme.colors` table is
replaced on the next snapshot.

| Method                                        | Behavior                                                                             |
| --------------------------------------------- | ------------------------------------------------------------------------------------ |
| `generate({seed=..., mode=..., variant=...})` | Build both modes from `#rrggbb`, save, publish, and render templates.                |
| `generate({image=..., ...})`                  | Same, with the seed sampled from a local image.                                      |
| `populate({colors=..., mode=...})`            | Apply a supplied `{dark=..., light=...}` palette. Without `colors`, re-render saved. |
| `populate({mode="light"})`                    | Switch mode and re-render from the saved palette.                                    |
| `regenerate({mode=...})`                      | Rebuild from the saved seed or image.                                                |

Method arguments:

| Method       | Required arguments               | Optional arguments | Errors                                                                                                |
| ------------ | -------------------------------- | ------------------ | ----------------------------------------------------------------------------------------------------- |
| `generate`   | exactly one of `seed` or `image` | `mode`, `variant`  | invalid seed/image/variant/mode, unreadable image, or state write failure                             |
| `populate`   | none                             | `colors`, `mode`   | invalid or incomplete palette, invalid mode, no saved palette, or state write failure                 |
| `regenerate` | none                             | `mode`             | no saved palette, manual-only palette, unreadable image, invalid saved source, or state write failure |

Every successful call returns a result table:

| Field     | Type            | Meaning                                                          |
| --------- | --------------- | ---------------------------------------------------------------- |
| `mode`    | string          | Active mode after the call                                       |
| `source`  | table           | Saved source, in the same shape as `theme.source`                |
| `colors`  | table           | Both palettes, in the same shape as `theme.colors`               |
| `applied` | integer         | Number of template files written                                 |
| `errors`  | array of string | Template, hook, config, or output errors; empty when all applied |

Template and hook failures are reported in `errors` rather than failing the
call, because the palette itself was generated and saved. Argument, generation,
and persistence failures use the callback error. `generate` saves the palette
before rendering templates, so a template failure does not discard a valid
palette.

`variant` defaults to `tonalspot`; accepted values are `content`, `tonalspot`,
`monochrome`, `neutral`, `vibrant`, `expressive`, `fidelity`, `rainbow`, and
`fruit_salad`. `mode` defaults to the active mode. Image paths may be absolute
or start with `~/`.

```lua
theme.generate({ seed = "#39bae6", mode = "dark" }, function(result, error)
    if error then
        log(error)
    end
end)
```

The palette has the same shape as the supplied Matugen colors:

```json
{
  "dark": {
    "primary": "#39bae6",
    "onprimary": "#0b0e14",
    "terminal": {
      "normal": { "black": "#171b24", "red": "#ed8274" },
      "bright": { "black": "#686868", "red": "#f28779" },
      "foreground": "#d1d1c7",
      "background": "#1f2430",
      "selectionfg": "#1f2430",
      "selectionbg": "#409fff",
      "cursortext": "#1f2430",
      "cursor": "#ffcc66"
    }
  },
  "light": { "primary": "#55b4d4" }
}
```

Manual palettes may use Matugen snake_case names (`on_primary`,
`surface_variant`, `on_surface_variant`, `on_hover`); themeagen normalizes them
to the flat names above. Core roles and terminal colors are required. Extended
Material roles (`primary_container`, `surface_container_high`, `outline_variant`,
and so on) are preserved for templates. A manual palette must include any
extended role a template references; generated palettes include all of them.
`regenerate` requires a palette produced by `generate`, not one supplied to
`populate`.

Core roles (flat names):

`primary`, `onprimary`, `secondary`, `onsecondary`, `tertiary`, `ontertiary`,
`error`, `onerror`, `surface`, `onsurface`, `surfacevariant`,
`onsurfacevariant`, `outline`, `shadow`, `hover`, `onhover`.

Extended roles (snake_case) are added for templates:

`primary_container`, `on_primary_container`, `inverse_primary`,
`primary_fixed`, `primary_fixed_dim`, `on_primary_fixed`,
`on_primary_fixed_variant`, `secondary_container`, `on_secondary_container`,
`secondary_fixed`, `secondary_fixed_dim`, `on_secondary_fixed`,
`on_secondary_fixed_variant`, `tertiary_container`, `on_tertiary_container`,
`tertiary_fixed`, `tertiary_fixed_dim`, `on_tertiary_fixed`,
`on_tertiary_fixed_variant`, `error_container`, `on_error_container`,
`surface_dim`, `surface_tint`, `surface_bright`, `surface_container_lowest`,
`surface_container_low`, `surface_container`, `surface_container_high`,
`surface_container_highest`, `outline_variant`, `inverse_surface`,
`inverse_on_surface`, `background`, `on_background`, `scrim`.

`terminal.normal` and `terminal.bright` each contain `black`, `red`, `green`,
`yellow`, `blue`, `magenta`, `cyan`, and `white`. Terminal scalars are
`foreground`, `background`, `selectionfg`, `selectionbg`, `cursortext`, and
`cursor`. Template variables also accept the camelCase spellings
`selectionFg`, `selectionBg`, and `cursorText`.

Template variables use Matugen syntax. Whitespace is optional:

```conf
primary {{ colors.primary }}
hover   {{colors.hover}}
terminal {{ colors.terminal.normal.red }}
selection {{ colors.terminal.selectionFg }}
```

Both `colors.on_primary` and `colors.onprimary` resolve; `.hex` and
`.default.hex` suffixes are accepted. `{{ mode }}` is the active mode;
`{{ image }}` is the seed string for seed generation or the image path for
image generation. Unknown variables fail the apply instead of leaving a
placeholder in the output. Matugen filters and conditionals are not supported.

`config.toml` uses the existing Matugen layout:

```toml
[catalog.example]
name = "Example"

[templates.kitty]
input_path = "kitty/kitty-colors.conf"
output_path = "~/.config/kitty/current-theme.conf"
pre_hook = "optional shell command"
post_hook = "optional shell command"
```

| Key           | Type   | Meaning                                                          |
| ------------- | ------ | ---------------------------------------------------------------- |
| `input_path`  | string | Template file, relative to the config directory or absolute      |
| `output_path` | string | Output file; `~/`, absolute, or relative to the config directory |
| `pre_hook`    | string | Optional `sh -c` command run before writing this template        |
| `post_hook`   | string | Optional `sh -c` command run after writing this template         |

Unknown sections such as `[catalog.*]` are ignored. Every input is rendered
before any file is written, so a bad placeholder does not leave a partially
updated theme. Outputs are written atomically, existing symlinks are preserved,
and hooks run through `sh -c` with the active template variables. Hooks and
profile files have the same trust as `shell.lua`.

#### Wayland extension

`riced-wayland` uses `wl_output`, `ext-workspace-v1`, and
`zwlr-foreign-toplevel-management-v1`, with read-only
`ext-foreign-toplevel-list-v1` as a fallback. It is not compositor-specific.
Missing protocols produce empty data and `false` capability flags. It publishes
store data only and pushes no events.

```lua
local wayland = riced.plugin("wayland")
local focused = wayland.focused or {}

for _, workspace in ipairs(wayland.workspaces or {}) do
    iced.button(workspace.name, {
        on_press = workspace.active and nil or {
            event = "workspace",
            args = { id = workspace.id },
        },
    })
end

local workspace = (wayland.workspaces or {})[1]
if workspace then
    wayland.activate_workspace(workspace.id, function(_, error)
        if error then log(error) end
    end)
end
```

Store fields:

| Field              | Type / shape                                                                                                                             |
| ------------------ | ---------------------------------------------------------------------------------------------------------------------------------------- |
| `connected`        | boolean; false while the compositor connection is down                                                                                   |
| `capabilities`     | `{ toplevels = boolean, toplevel_control = boolean, workspaces = boolean }`                                                              |
| `outputs`          | array of `{ id, name, description, make, model, x, y, width, height, scale }`                                                            |
| `toplevels`        | array of `{ id, title, app_id, outputs, activated, maximized, minimized, fullscreen }`; the read-only fallback uses `identifier` instead |
| `workspaces`       | array of `{ id, protocol_id, name, coordinates, active, urgent, hidden, capabilities, group, outputs }`                                  |
| `workspace_groups` | array of `{ id, capabilities, outputs }`                                                                                                 |
| `focused`          | `{ toplevel = id                                                                                                                         | nil, workspace = id | nil, output = name | nil }` |

`outputs` come from `wl_output`: `id` is the registry name, `name` is the
connector name or `output-<id>`, and `width`/`height` are the current mode
geometry. `scale` is the output scale. Text values are clamped to 1024
characters.

Toplevel `id` is an opaque string for use with the methods. With
`zwlr-foreign-toplevel-management-v1`, each item also has the four state
booleans and the names of the outputs it is on. With only the read-only
`ext-foreign-toplevel-list-v1`, the item has `identifier`, has no state
booleans, and `outputs` is empty.

Workspace `id` is an opaque string; `protocol_id` is the compositor's workspace
identifier or nil. `coordinates` is the compositor's raw coordinate list.
`active`, `urgent`, and `hidden` are booleans. `capabilities` is a subset of
`activate`, `deactivate`, `remove`, and `assign`. `group` is a workspace-group
id or nil. `outputs` is derived from the workspace group and may be empty.
Workspace-group `capabilities` currently contains `create_workspace` when
supported.

`focused` contains only unambiguous values. `toplevel` is nil with the
read-only backend. `workspace` is nil unless exactly one workspace is active.
`output` is usually a connector name; it falls back to an output object id if
the output disappeared, and is nil when focus cannot be attributed to one
output.

Methods:

| Method                   | Arguments | Requirements                                                | Errors                                                                                       |
| ------------------------ | --------- | ----------------------------------------------------------- | -------------------------------------------------------------------------------------------- |
| `activate_toplevel(id)`  | string    | `toplevel_control`, a live toplevel handle, and a `wl_seat` | not connected, unknown/stale toplevel, no seat, or Wayland flush failure                     |
| `close_toplevel(id)`     | string    | `toplevel_control` and a live toplevel handle               | not connected, unknown/stale toplevel, or Wayland flush failure                              |
| `activate_workspace(id)` | string    | `ext-workspace-v1` and workspace capability `activate`      | not connected, unknown/stale workspace, no manager, unsupported capability, or flush failure |

All three return no value on success. Use a callback to observe errors. IDs must
come from the current `toplevels` or `workspaces` snapshot; handles become
invalid when the compositor closes the object.

`capabilities.toplevels` is true with either toplevel backend.
`capabilities.toplevel_control` is false with only the read-only backend.
`capabilities.workspaces` is true only with `ext-workspace-v1`. Standard
protocols do not map toplevels to workspaces. Snapshots are bounded by the
protocol line limit, so very large arrays may omit entries; text is clamped.
On disconnect the extension retries every second; `connected` is false and the
previous arrays may remain until the next successful snapshot.

#### D-Bus extension

`riced-dbus` provides StatusNotifier tray items and an
`org.freedesktop.Notifications` server. It owns
`org.kde.StatusNotifierWatcher` when the name is free; otherwise it registers
as a host with the existing watcher. It owns the notification name only if no
other notification daemon does. It publishes store data only and pushes no
plugin events.

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

Store fields:

| Field           | Type / shape                                                                      |
| --------------- | --------------------------------------------------------------------------------- |
| `connected`     | boolean; session bus connection                                                   |
| `capabilities`  | `{ tray = boolean, tray_host = boolean, notifications = boolean }`                |
| `tray`          | array of `{ id, title, icon, icon_theme_path, status, menu, item_is_menu }`       |
| `tray_menu`     | active menu `{ id, revision, items, error }`, or nil                              |
| `notifications` | array of `{ id, app_name, app_icon, summary, body, actions, timeout, timestamp }` |

`capabilities.tray` means a StatusNotifier watcher is available.
`capabilities.tray_host` is true only when this extension owns the watcher; it
is false when it registered with another watcher.
`capabilities.notifications` is true only when this extension owns
`org.freedesktop.Notifications`. If another daemon already owns the name, the
extension does not receive notifications and the `notifications` array stays
empty.

Tray item fields:

| Field             | Meaning                                                                                                                                                                                     |
| ----------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `id`              | Opaque string (`bus + object path`), for example `:1.42/StatusNotifierItem`; pass to methods                                                                                                |
| `title`           | Item title, falling back to the item `Id`; clamped to 256 characters                                                                                                                        |
| `icon`            | Sender's `IconName`, or a normalized fallback derived from the item `Id` (for example `vesktop_status_icon_1` becomes `vesktop`); clamped to 256 characters. Resolve with `iced.icon_path`. |
| `icon_theme_path` | Sender's optional absolute `IconThemePath` directory, or nil; paths over 4096 bytes or containing NUL are ignored.                                                                          |
| `status`          | Raw StatusNotifier status such as `Active`, `Passive`, or `NeedsAttention`, clamped to 256 characters; default `Passive`                                                                    |
| `menu`            | D-Bus object path string or nil; `/` is treated as no menu                                                                                                                                  |
| `item_is_menu`    | Whether activating the item should open the menu instead                                                                                                                                    |

Tray properties are refreshed about every 500 ms. An item that fails to answer
three times is removed from the store.

Use `iced.icon_path(item.icon, { size = 24, path = item.icon_theme_path })` to
resolve tray icons with the current `iced.icon_theme`. Items that only send an
`IconPixmap` still need a profile-level fallback; raw tray pixmaps are not
currently exposed. Try `item.title` as a second icon name before using a
placeholder.

Tray menus use `com.canonical.dbusmenu`. Call `open_tray_menu(item.id)` while a
popup is visible; the extension calls `AboutToShow` and `GetLayout`, then fills
`tray_menu`. The active menu is refreshed about every 500 ms. Call
`close_tray_menu()` when the popup closes. Menu entries look like:

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

Notification fields:

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

Methods:

| Method                                           | Arguments               | Behavior                                                                                             |
| ------------------------------------------------ | ----------------------- | ---------------------------------------------------------------------------------------------------- |
| `activate_tray(id)`                              | string                  | Calls `Activate(0, 0)` on the tray item                                                              |
| `secondary_activate_tray(id)`                    | string                  | Calls `SecondaryActivate(0, 0)` on the tray item                                                     |
| `context_menu_tray(id)`                          | string                  | Calls `ContextMenu(0, 0)` on the tray item                                                           |
| `open_tray_menu(id, callback?)`                  | string                  | Reads the item's `com.canonical.dbusmenu` layout into `tray_menu`; errors if it has no menu          |
| `close_tray_menu()`                              | none                    | Clears `tray_menu`; safe to call repeatedly                                                          |
| `tray_menu_event(id, menu_id, event, callback?)` | string, integer, string | Sends `clicked`, `opened`, `closed`, or `hovered` for a menu item                                    |
| `close_notification(id)`                         | integer                 | Removes the notification if present and emits `NotificationClosed`; unknown ids are ignored          |
| `invoke_notification_action(id, key)`            | integer, string         | Validates the action key, removes the notification, and emits `ActionInvoked` + `NotificationClosed` |
| `pick_file(options?, callback?)`                 | table                   | Opens the XDG desktop portal file chooser; the callback receives selected absolute paths              |

`pick_file` options: `title` (string), `multiple` (boolean), `directory`
(boolean), and `path` (absolute starting directory; a leading `~/` is expanded).
On success the callback receives `(paths, nil)`, where `paths` is a non-empty
array of absolute paths. A cancel or dismissed dialog is not success: the
callback receives `(nil, "cancelled")`. Other failures receive `(nil, error)`.
The callback is the only way to get the selection or see a cancel. The dialog
times out after 120 seconds; it is closed and the callback receives
`(nil, "the file chooser timed out")`.

```lua
dbus.pick_file({ title = "Open image", multiple = true }, function(paths, error)
    if error then
        if error ~= "cancelled" then log(error) end
        return
    end
    for _, path in ipairs(paths) do
        log(path)
    end
end)
```

`error(error)` inside the callback only logs through riced's callback wrapper;
it does not propagate to the calling code.

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

#### System extension

`riced-system` reads host, clock, CPU, memory, disk, GPU, and process data. It
has no methods and pushes no plugin events; everything is in its store.

Reading a field for the first time subscribes to it. The extension refreshes
only requested groups, once per second, so fields nothing reads are not
computed. Values may be nil when the OS or hardware does not expose them.

```lua
local system = riced.plugin("system")
local cpu = system.cpu or {}
local memory = system.memory or {}

iced.text(string.format("%.0f%%  %.1f / %.1f GiB", cpu.usage or 0,
    (memory.used or 0) / 2 ^ 30, (memory.total or 0) / 2 ^ 30))
```

Store fields:

| Field       | Shape                                                                                                                                                        |
| ----------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `host`      | `{ name?, os?, version?, kernel?, uptime?, boot_time? }`; uptime and boot_time are seconds, boot_time is a Unix timestamp                                    |
| `time`      | `{ epoch, time, date, weekday }` or nil; epoch is seconds, `time` is local `HH:MM` (24-hour), `date` is local `YYYY-MM-DD`, weekday is the full English name |
| `cpu`       | `{ usage, cores, physical?, brand, frequency, load1, load5, load15 }`; usage is percent, frequency is the highest core MHz, physical may be nil              |
| `memory`    | `{ total, used, available, swap_total, swap_used }`; bytes                                                                                                   |
| `disks`     | array of `{ mount, fs, total, available, used }`; bytes; entries whose total is zero are omitted                                                             |
| `gpus`      | array of `{ name, busy?, vram_used?, vram_total?, temp?, clock? }`; busy is percent, memory fields are bytes, temp is °C, clock is MHz                       |
| `processes` | integer process count                                                                                                                                        |

`disks` comes from `sysinfo`; `gpus` reads DRM cards under `/sys/class/drm`.
Optional GPU values are nil when the driver does not expose them. `time` is nil
if the system clock cannot be read. The host, memory, and disk numbers are
snapshots; read them again when the plugin store changes.

#### Open-Meteo extension

`riced-openmeteo` retrieves current conditions for one location per process.
Install it and its Lua widget with `just extension-openmeteo`, then configure
coordinates once in your profile (safe before the executable connects):

```lua
local weather = require("extensions.openmeteo.openmeteo")
weather.configure(52.52, 13.41)

function app:view()
    return iced.openmeteo({ size = 18, color = theme.text })
end
```

The Lua module remembers the desired coordinates and sends them when the widget
first renders after connection/reconnection. All widget instances share that
location; repeated instances need distinct `id` props. The widget uses normal
text props, displays `12.5°C`, and shows loading/unavailable text before a reading.
Retained readings are marked `(stale)` after a failed refresh or disconnect.

For a custom UI, use `riced.plugins("openmeteo")`. Its asynchronous
`configure(latitude, longitude, callback?)` method accepts finite numeric WGS84
coordinates (`-90..90`, `-180..180`). Success acknowledges configuration, with
`(nil, nil)`; the weather arrives separately in the store. Call only once the
extension is alive. Configuration is in memory; resend it after reconnection.

| Store field             | Meaning                                                                                                                                                                                    |
| ----------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `ready`                 | A successful reading exists for the configured location                                                                                                                                    |
| `latitude`, `longitude` | Requested coordinates, absent until configured                                                                                                                                             |
| `current`               | Open-Meteo values: `time`, `interval`, `temperature_2m`, `relative_humidity_2m`, `apparent_temperature`, `is_day`, `precipitation`, `weather_code`, `wind_speed_10m`, `wind_direction_10m` |
| `current_units`         | Units returned by Open-Meteo: Celsius, km/h, mm, percent, etc.                                                                                                                             |
| `timezone`              | Resolved IANA timezone; `current.time` is local to it                                                                                                                                      |
| `updated_at`            | Unix seconds of the last successful fetch                                                                                                                                                  |
| `error`                 | Last fetch error, absent after a successful refresh                                                                                                                                        |

Fetching starts after configuration and repeats every 15 minutes without a Lua
timer. Failures retry after 60 seconds and keep the last reading. Changing the
location clears old weather. Requests time out after 15 seconds, reject HTTP
errors/redirects and invalid responses, and cap response bodies at 64 KiB.
Other current values may be null when unavailable; handle missing values in
custom views. `weather_code` follows Open-Meteo's WMO interpretation codes.

Weather data: [Open-Meteo](https://open-meteo.com/),
[API documentation](https://open-meteo.com/en/docs). This version uses its public
non-commercial endpoint without an API key; see the provider's usage terms.

#### Applications extension

`riced-applications` lists installed XDG desktop entries and launches them.
Install it with `just extension-applications`.

```lua
local applications = riced.plugin("applications")

for _, entry in ipairs(applications.applications or {}) do
    iced.button(entry.name, {
        on_press = { type = "launch", id = entry.id },
    })
end

applications.launch("firefox", function(_, error)
    if error then log(error) end
end)
```

The `applications` store field is a name-sorted array:

| Field      | Meaning                                                        |
| ---------- | -------------------------------------------------------------- |
| `id`       | Desktop file id; subdirectories use dashes, for example `kde-konsole` |
| `name`     | Display name                                                   |
| `icon`     | Icon name or nil; resolve with `iced.icon_path`                |
| `comment`  | Description or nil                                             |
| `terminal` | Whether launching needs a terminal                             |

Entries come from `$XDG_DATA_HOME/applications` and each
`$XDG_DATA_DIRS/applications`. The first directory that defines an id wins;
`Hidden`, `NoDisplay`, and non-`Application` entries are skipped. The list is
rescanned every 30 seconds and published when it changes.

`launch(id, callback?)` resolves a known id, starts the desktop `Exec` command
without a shell, and drops document/URL field codes. `%c` and `%k` expand, `%%`
is a literal percent, and `Terminal=true` entries run in `$TERMINAL -e`
(falling back to `x-terminal-emulator`). Unknown ids and spawn failures call the
callback with `(nil, error)`.

The example profile includes `example/components/launcher.lua`, a searchable
overlay opened with `riced ipc launcher`. Bind that command to a compositor
shortcut. It subscribes to key events only while the overlay is open.

#### Writing extensions

Extensions are executables, or directories containing an executable, under
`<profile>/extensions/`. `riced` spawns each one, restarts crashed extensions
with backoff, and stops one when its file is removed. An extension may also be
started manually with `RICED_SOCKET` set.

The SDK is `crates/riced-extension-sdk`. Protocol v1 is newline-delimited JSON
with a 256 KiB line limit, bounded queues, a `0700` socket directory, and a
`0600` socket. Extensions run as your user and have the same trust as
`shell.lua`.

An SDK method reply that exceeds the line limit returns an error to its caller
instead of leaving the callback pending.

Extensions can bundle Lua widgets and objects alongside their executable:

```text
<profile>/extensions/demo/
  riced-demo     # executable (0755)
  demo.lua       # Lua module (0644)
```

`demo.lua` registers `iced.demo(props)` with `iced.define("demo", factory)`.
The existing widget loader finds it on first access; no `require` or host
rebuild is needed. Use `riced.plugins("demo")` inside the module to access the
process, and handle `alive == false` before making calls. Plugin replies trigger
a redraw. Keep Lua companions non-executable so the supervisor does not launch
them as processes.

Modules can also return ordinary Lua objects, imported with
`require("extensions.demo.demo")`. For lifecycle hooks, use
`iced.component(id, "extensions.demo.component", props)` with a component module.
The same module watching and failed-reload recovery apply. Widget filenames must
match their constructor names, names must be unique across the profile, and
`iced.define` cannot replace built-ins. After adding a previously missing widget,
reload `shell.lua` or restart to clear the loader's cached miss.

```sh
just extension-system
just extension-wayland
just extension-dbus
just extension-themeagen
just extension-lucide
just extension-openmeteo
```

Build any Rust extension into an output directory:

```sh
riced extension build extensions/openmeteo -o ~/.config/riced/test/extensions/openmeteo
```

The source is a Cargo package directory containing exactly one `[[bin]]`.
It builds in release mode and copies that executable plus top-level `*.lua`
files into the output directory (created if needed). The executable remains
`0755` and Lua companions are `0644`. Omitting `-o` targets `extensions/` in
the profile named by `-c`/`-p`, or of the only running daemon. Startups record
the canonical `shell.lua` in `$XDG_RUNTIME_DIR/riced/<profile>.profile`; stale
records that no longer match the live socket are rejected. This command does not
restart daemons.

## iced components

### Constructors and common props

Every widget constructor has the form `iced.name(content, props?)`. `content` is
the widget's child element, list of children, or primary value; pass `nil` when
the widget has no content. `props` defaults to `{}`.

Every constructor also accepts these common props:

| Prop         | Type                 | Meaning                                                                                                   |
| ------------ | -------------------- | --------------------------------------------------------------------------------------------------------- |
| `width`      | [length](#lengths)   | Layout width                                                                                              |
| `height`     | [length](#lengths)   | Layout height                                                                                             |
| `padding`    | number or side table | Padding around the widget; `{ top, right, bottom, left }`                                                 |
| `transition` | event table          | Configure `change`, `enter`, `exit`, and interaction animations; see [transitions](#property-transitions) |

Widget-specific props are listed below. `transition` on `list_view` means item
enter/leave/displacement animation instead of property animation.

### Widget reference

#### `iced.icon(content, props)`

`content`: a single unicode char (glyph) from the Lucide font, e.g.
`"\u{e038}"` for `activity`. The host has **no** lucide-icons lookup;
resolve a kebab-case name (`"activity"`, `"arrow-right"`) via the
`lucide` extension process:

```lua
local lucide = riced.plugins("lucide")
lucide.resolve("activity", function(result, err)
    if result then
        -- Cache the raw glyph; replies automatically trigger a redraw.
        glyph = result.char
    end
end)
-- iced.icon(content, props) renders the glyph:
if glyph then
    iced.icon(glyph, { size = 16, color = theme.text })
end
```

Props: `size` (positive, default 16), `color`, `center` (boolean), `align_x` (`"left"`, `"center"`, `"right"`, `"justified"`), `align_y` (`"top"`, `"center"`, `"bottom"`). `center`/`align_*` only take effect with a constrained `width`/`height`. The icon renders as a
text glyph in the bundled `lucide` font family (vendored `src/fonts/lucide.ttf`,
loaded at startup via `Settings.fonts`), so `size`/`color` animate like
`text` via `transition.change.props`.

The extension owns all `lucide-icons` logic (`extensions/lucide`); the
host only embeds the TTF, so `lucide-icons` appears **only** in
`extensions/lucide/Cargo.toml`.

#### `iced.lucide(props)` — extension-owned widget

Install the executable and its companion with `just extension-lucide`, or place
`riced-lucide` and `extensions/lucide/lucide.lua` together under
`<profile>/extensions/lucide/` (Lua file mode `0644`). Then call:

```lua
iced.lucide({ name = "activity", size = 20, color = theme.text })
```

`name` is a required, nonempty kebab-case Lucide icon name. Other props are the
same as `iced.icon`; repeated instances need distinct `id` values, like other
custom widgets. It renders blank text while the extension is unavailable or a
lookup is pending. Successful lookups are cached and trigger a redraw. Failed
lookups stay blank and are retried after reconnection or module reload, avoiding
a request/redraw loop. Cached glyphs remain usable while disconnected.

The module also exports the existing helpers through
`require("extensions.lucide.lucide")`: `icon(name, props, fallback)`,
`exists(name, callback)`, `list(prefix, callback)`, `warm(names)`, and lazy glyph
fields such as `lucide.activity` (nil until resolved).

Migration: replace old `iced.lucide(glyph, props)` calls with
`iced.icon(glyph, props)`. If the old installer placed `lucide.lua` at the profile
root, move any customizations into the bundled module and remove the old copy;
two files named `lucide.lua` cause a duplicate-widget error. Update old
`require("lucide")` calls to `require("extensions.lucide.lucide")`.

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
`"scale_down"`) and `opacity` from 0 to 1.

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

QR data is cached by content while a widget uses it and released once no
rendered widget references it.

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

| Widget     | Props                                                                                 | Event       |
| ---------- | ------------------------------------------------------------------------------------- | ----------- |
| `checkbox` | `value` (boolean, default false), `label`, `text_size`, `on_toggle`                   | `on_toggle` |
| `radio`    | `value` (integer, default 0), `selected` (integer), `label`, `text_size`, `on_select` | `on_select` |
| `toggler`  | `value` (boolean, default false), `label`, `text_size`, `on_toggle`                   | `on_toggle` |

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

State is kept by `id` until `riced` reloads the shell. The runtime retains at
most 256 editor states; exceeding that resets the editor cache. Typing is
preserved across renders; changing `value` from the previous render replaces
the text, and changing the id starts a fresh editor.

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
`on_scroll`, `auto_scroll` (boolean), `scrollbar_hidden` (boolean, default false),
`spacing`, and `style`.

`scrollbar_hidden = true` uses Iced's native zero-width scrollbar for each enabled
axis. Wheel/touch scrolling stays enabled. Unlike transparent scrollbar colors,
this removes the scrollbar's width. Leave `spacing` at zero to avoid an extra gap.

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

#### `iced.measure(content, props)`

Wraps content and records its laid-out rectangle. `props.id` is a required
nonempty string. Layout is unchanged.

```lua
local close = iced.measure(
    iced.button(iced.text("x"), { on_press = "close" }),
    { id = "close_button" }
)

local bounds = iced.bounds("close_button")
-- { x, y, width, height } in surface-local logical pixels, or nil
```

`iced.bounds(id)` returns the last measured rectangle for the current surface.
If that surface has no measurement for `id`, it falls back to the most recent
measurement of that id on another surface. This matters because `app:view` runs
once per layer/window/popup. It is `nil` before the first layout/draw, so keep
a fallback in the first view. The daemon rebuilds views once after that first
measurement, so the next `view` sees the real rectangle. The rectangle is
relative to the layer or window where the measured widget lives, which is the
coordinate space popup anchors use.

There is no `z` coordinate. Iced layouts are two-dimensional; draw order comes
from tree order, overlays, and layer levels. Output-global coordinates are also
not exposed; bounds are surface-local.

Widgets with an `id` prop are measured automatically, so `iced.measure` is
only needed to wrap content that has no id of its own. Popups can use the id
directly instead of reading bounds:

```lua
iced.popup(iced.text("menu"), {
    id = "menu",
    parent = "bar",
    width = 120,
    height = 80,
    anchor = "close_button",
})
```

#### `iced.table(content, props)`

`content`: array of rows. Each row is an array of cells; a cell may be a string,
number, element, or `nil`. Missing cells render as empty space.

Props:

| Prop      | Type                                            | Meaning                  |
| --------- | ----------------------------------------------- | ------------------------ |
| `columns` | array of column tables (required, at least one) | Header and column layout |
| `spacing` | non-negative number                             | Separator size           |

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

| Prop         | Type        | Meaning                                                      |
| ------------ | ----------- | ------------------------------------------------------------ |
| `id`         | string      | Required, nonempty identity for item tracking                |
| `key`        | string      | Table field used as item identity; requires `render`         |
| `direction`  | string      | `"vertical"` default or `"horizontal"`                       |
| `spacing`    | number      | Gap between items                                            |
| `render`     | function    | `(item, index) -> element`; index is zero-based              |
| `transition` | event table | Independent item `enter`, `exit`, and `displaced` animations |

Diffing matches items by `key` when set, otherwise by value. Inserting,
removing, or reordering distinct items is detected and their positions animate;
duplicate keys are matched in order. Rebuilt data tables need a stable `key`
(for example `"id"`); there is no positional fallback. Reappearing keys cancel
their pending exit and resume from the displayed values.

```lua
transition = {
    enter = { from = { x = -180, scale = 0 }, duration = 180 },
    exit = { to = { x = 180, scale = 0 }, duration = 120 },
    displaced = { duration = 180 },
}
```

`displaced` uses measured layout positions and accepts timing and `callback`,
not explicit `from`/`to`. Outgoing items remain drawn at their previous bounds
without receiving input until exit completes. Each event accepts an optional
`callback(self)` delivered after completion, outside rendering. `self` is the
owning custom/component instance, or nil outside a component.
For independently controlled custom cards, put `enter`/`exit` on each card's
root and use only `displaced` on the list.

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

| Shape    | Fields                                                                                           |
| -------- | ------------------------------------------------------------------------------------------------ |
| `rect`   | `x`, `y`, `width` (positive), `height` (positive), `color`, optional `border = { color, width }` |
| `circle` | `x`, `y`, `radius` (positive), `color`, optional `border`                                        |
| `line`   | `from = { x, y }`, `to = { x, y }`, `color`, `width` (default 1)                                 |
| `path`   | `points` (at least two `{ x, y }`), `color`, `width`, `fill` (boolean)                           |
| `text`   | `x`, `y`, `content`, `size` (default 16), `color`                                                |

Coordinates default to 0. `border.width` defaults to 1. Arbitrary Iced canvas
programs are not supported.

#### `iced.shader(props)`

`source`: inline WGSL fragment shader. `path`: WGSL file resolved against the
profile directory (absolute paths allowed). Exactly one is required. See
[`example/components/shaders/plasma.wgsl`](example/components/shaders/plasma.wgsl)
for a runnable shader; `example/components/ripple.lua` shows an animated,
centered shader overlay.

`textures`: optional array of up to two raster image paths (absolute, or
relative to the profile directory). They are bound as `riced_texture0` /
`riced_sampler0` and `riced_texture1` / `riced_sampler1`; sample them with
`textureSample(riced_texture0, riced_sampler0, uv)`. Unused slots are a 1x1
white texture. Images are decoded as RGBA8 in the renderer color space
(no linear conversion). A texture is reused while any slot still references
it, so moving the new wallpaper from slot 1 to slot 0 at the end of a
transition does not upload it again; a texture is destroyed as soon as no slot
references it, and all textures are released when the shader is removed.

`extraArgs`: optional table of named finite numbers. Riced generates the fields
from the keys and exposes them as `riced.<name>`:

```lua
iced.shader({
    source = [[
        @fragment
        fn fs_main(@location(0) uv: vec2<f32>) -> @location(0) vec4<f32> {
            return vec4<f32>(riced.extra_x, riced.extra_y, 0.0, 1.0);
        }
    ]],
    extraArgs = { extra_x = 0.5, extra_y = 0.75 },
})
```

Keys must be WGSL identifiers that do not start with `riced_`; `resolution`,
`time`, `mouse`, `hover`, and `_pad` are reserved. At most 16 values. Keys are sorted for a stable
layout; changing the key set recompiles the pipeline, while changing values only
updates the uniform buffer.

Riced supplies the vertex stage and this reserved interface:

```wgsl
struct Riced {
    resolution: vec2<f32>,
    time: f32,
    hover: f32,
    mouse: vec2<f32>,
    _pad: vec2<f32>,
    // extraArgs fields are appended here
};

@group(0) @binding(0) var<uniform> riced: Riced;
@group(0) @binding(1) var riced_texture0: texture_2d<f32>;
@group(0) @binding(2) var riced_sampler0: sampler;
@group(0) @binding(3) var riced_texture1: texture_2d<f32>;
@group(0) @binding(4) var riced_sampler1: sampler;

@fragment
fn fs_main(@location(0) uv: vec2<f32>) -> @location(0) vec4<f32>;
```

`riced.mouse` is the cursor position in widget pixels; `x` is negative when the
cursor is outside. `riced.hover` is `1.0` inside and `0.0` outside. Cursor
updates require the shader's surface to receive pointer events, so an
`events_transparent` background layer stays centered.

`uv` runs from 0 to 1 across the widget. `riced.time` is seconds since the
first shader render; `riced.resolution` is the widget size. `width` and `height`
use the common props.

`animated = true` makes the shader request redraws so `riced.time` keeps
advancing without a Lua `iced.frames()` subscription. `fps` caps those redraws
(1–240, default 60); use a lower value for a full-screen background.

```lua
iced.shader({
    path = "components/shaders/water.wgsl",
    width = "fill",
    height = "fill",
    animated = true,
    fps = 30,
    extraArgs = { opacity = 0.35, speed = 1.0 },
})

-- Two textures are available for crossfades and transitions:
iced.shader({
    path = "components/shaders/wallpaper.wgsl",
    width = "fill",
    height = "fill",
    textures = { old_path, new_path },
    extraArgs = { progress = 0.5 },
})
```

Shaders render only with the wgpu backend; the tiny-skia fallback draws nothing.
Invalid WGSL, a missing file, or a wrong `fs_main` signature logs an error and
renders empty space. Files are re-read when their mtime or size changes.

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

| Key                      | Values                                                                                  | Default        |
| ------------------------ | --------------------------------------------------------------------------------------- | -------------- |
| `id`                     | nonempty string                                                                         | position-based |
| `size`                   | `{ width, height }`, each `"fill"` or positive integer pixels                           | fill/fill      |
| `layer`                  | `"background"`, `"bottom"`, `"top"`, `"overlay"`                                        | `"top"`        |
| `anchor`                 | array of `"top"`, `"bottom"`, `"left"`, `"right"`                                       | all edges      |
| `exclusive_zone`         | integer at least -1, or nil                                                             | none/0         |
| `margin`                 | four integers: top, right, bottom, left                                                 | zeros          |
| `keyboard_interactivity` | `"none"`, `"on_demand"`, `"exclusive"`                                                  | `"on_demand"`  |
| `output_option`          | `"active"`, `"last_output"`, output name, or positive Wayland registry global ID        | `"active"`     |
| `events_transparent`     | boolean                                                                                 | false          |
| `namespace`              | nonempty string without NUL                                                             | inherited      |
| `blur_option`            | `"none"`, `"full"`, or an array of `{ x, y, width, height }` with positive width/height | `"none"`       |

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

| Value               | Widgets / callbacks                                                                                                                |
| ------------------- | ---------------------------------------------------------------------------------------------------------------------------------- |
| boolean             | `checkbox.on_toggle`, `toggler.on_toggle`                                                                                          |
| number              | `radio.on_select`, `slider.on_change`/`on_release`, `vertical_slider`, `pane_grid.on_click` (index), `pane_grid.on_resize` (ratio) |
| string              | `text_input`, `pick_list`, `combo_box`, `markdown.on_link_click`, `text_editor.on_input`                                           |
| `{ x, y }`          | `mouse_area.on_move`, `mouse_area.on_scroll`                                                                                       |
| `{ width, height }` | `sensor.on_show`, `sensor.on_resize`                                                                                               |
| scroll table        | `scrollable.on_scroll`                                                                                                             |
| action table        | `text_editor.on_action` adds `text`; `pane_grid.on_drag` adds `pane`                                                               |

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

| Field        | Type                                                       |
| ------------ | ---------------------------------------------------------- |
| `background` | color                                                      |
| `text_color` | color                                                      |
| `border`     | `{ color?, width?, radius? }`; width/radius non-negative   |
| `shadow`     | `{ color?, offset? = { x, y }, blur? }`; blur non-negative |

`radius` is a uniform number or a table with `top_left`, `top_right`,
`bottom_right`, and `bottom_left`. Omitted corners are zero:

```lua
border = {
    color = "#404040",
    width = 1,
    radius = { top_left = 8, top_right = 8 },
}
```

The same radius forms are accepted by `rule.style.radius` and pane-grid
highlight borders.

Widget style support:

| Widget                      | Fields                                                                         | State overrides                            |
| --------------------------- | ------------------------------------------------------------------------------ | ------------------------------------------ |
| `container`                 | `background`, `text_color`, `border`, `shadow`                                 | —                                          |
| `button`                    | `background`, `text_color`, `border`, `shadow`                                 | `active`, `hovered`, `pressed`, `disabled` |
| `checkbox`                  | `background`, `text_color`, `border`, `icon_color`                             | `active`, `hovered`, `disabled`            |
| `radio`                     | `background`, `text_color`, `border`, `dot_color`                              | `active`, `hovered`                        |
| `toggler`                   | `background`, `text_color`, `border`, `foreground`                             | `active`, `hovered`, `disabled`            |
| `slider`, `vertical_slider` | `rail`, `filled`, `border`, `handle`, `handle_border`                          | `active`, `hovered`, `dragged`             |
| `text_input`, `combo_box`   | `background`, `text_color`, `border`, `placeholder`, `selection`, `icon_color` | `active`, `hovered`, `focused`, `disabled` |
| `text_editor`               | `background`, `text_color`, `border`, `placeholder`, `selection`               | `active`, `hovered`, `focused`, `disabled` |
| `scrollable`                | `background`, `border`, `shadow`, `rail`, `scroller`                           | `active`, `hovered`, `dragged`             |
| `pick_list`                 | `background`, `text_color`, `border`, `placeholder`, `handle`                  | `active`, `hovered`, `opened`              |

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

iced.opacity("#89b4fa", 50) -- "#89b4fa80"
```

`iced.Length.fill_portion(n)` requires an integer from 1 to 65535. `iced.Size`
requires both axes. Weighted fill portions are for widget layout, not
layer-surface dimensions.

`iced.opacity(color, percent)` returns a `#RRGGBBAA` color with alpha set from
`percent` (0 is transparent, 100 is opaque). The input may be `#RRGGBB` or
`#RRGGBBAA`; an existing alpha channel is replaced.

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

| Task              | Fields                                 | Effect                                                        |
| ----------------- | -------------------------------------- | ------------------------------------------------------------- |
| `focus`           | `id`                                   | Focus the widget with that id                                 |
| `scroll_to`       | `id`, `x?`, `y?`                       | Scroll the widget with that id to an absolute offset          |
| `clipboard_read`  | —                                      | Deliver `{ type = "clipboard", value = string? }` to `update` |
| `clipboard_write` | `content` (string)                     | Write text to the clipboard                                   |
| `font`            | `path`                                 | Load a font file at runtime                                   |
| `window`          | `window`, `action`, plus action fields | Resize, move, close, fullscreen, or window the surface        |
| `lock`            | —                                      | Request a session lock                                        |
| `unlock`          | —                                      | Release the session lock                                      |
| `exit`            | —                                      | Exit `riced`                                                  |

Window actions:

| Action       | Fields                       |
| ------------ | ---------------------------- |
| `resize`     | `width`, `height` (positive) |
| `move`       | `x`, `y` (finite)            |
| `close`      | —                            |
| `fullscreen` | —                            |
| `windowed`   | —                            |

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

| Constructor          | Event delivered to `update`                                           |
| -------------------- | --------------------------------------------------------------------- |
| `iced.every(ms)`     | `{ type = "tick" }`; interval is a whole number from 1 to 86400000 ms |
| `iced.frames()`      | `{ type = "frame" }` on every frame                                   |
| `iced.on_key()`      | key press and release events; see below                               |
| `iced.on_key(true)`  | key press events only                                                 |
| `iced.on_key(false)` | key release events only                                               |
| `iced.on_mouse()`    | mouse events; see below                                               |

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

`transition` is a table keyed by event. Omit it to disable animations.
Use `iced.transition("name", overrides)` to reuse a named preset, or
`iced.transition({ duration = 180, enter = {}, exit = {} })` to share timing
across events. See [named values and transitions](#named-values-and-transitions).
`change` interpolates `width`, `height`, scalar `padding`, `size`/`color` (text),
`spacing` (row/column), `value` (progress_bar), and the style fields
`background`, `text_color`, `border`, and `shadow` on widgets that support them.

| Widgets                                                                               | Animatable style fields                        |
| ------------------------------------------------------------------------------------- | ---------------------------------------------- |
| `container`, `button`                                                                 | `background`, `text_color`, `border`, `shadow` |
| `checkbox`, `radio`, `toggler`, `text_input`, `text_editor`, `combo_box`, `pick_list` | `background`, `text_color`, `border`           |
| `scrollable`                                                                          | `background`, `border`, `shadow`               |
| `slider`, `vertical_slider`                                                           | `border`                                       |

Unsupported transition properties are rejected. Progress-bar targets must stay
within `0..1`; spring overshoot is clamped for display. Width and height targets
must remain valid lengths even when animated.

```lua
iced.container(child, {
    width = self.width,
    transition = { change = { props = { "width" }, duration = 150 } },
})

iced.button("Workspace", {
    style = {
        background = theme.primary,
        border = { color = theme.background, width = 1, radius = 4 },
        hovered = { background = "#303030" },
    },
    transition = {
        change = { props = { "background", "border" }, duration = 180 },
    },
})
```

Style state overrides (`active`, `hovered`, `pressed`, `focused`, `disabled`,
and so on) stay static and win over the animated base value. Only the base
style fields animate.

Each event has independent timing. `change` accepts:

| Key        | Values                                                                        |
| ---------- | ----------------------------------------------------------------------------- |
| `props`    | array of animatable prop names                                                |
| `duration` | whole milliseconds from 1 to 86400000                                         |
| `easing`   | `linear`, `ease`, `in`, `out`, `in_out`, `ease_in`, `ease_out`, `ease_in_out` |
| `spring`   | `smooth`, `snappy`, `bouncy`, `instant`                                       |
| `callback` | Optional `function(self)` called once after completion; may return tasks      |

Default timing is 200 ms with `out` easing. `easing` and `spring` are mutually
exclusive. `"fill"`, `"shrink"`, fill portions, and per-side padding stay static.
Property animation rebuilds the subtree each frame; keep animated subtrees cheap.

```lua
transition = {
    enter = { from = { x = 20, scale = 0.9 }, duration = 180 },
    exit = {
        to = { x = 20, scale = 0.9 }, duration = 120,
        callback = function(self) self.closed = true end,
    },
    press = { callback = function(self) self.animate.exit() end },
    pointer_enter = { to = { scale = 1.05 }, duration = 80 },
    pointer_exit = { to = { scale = 1 }, duration = 80 },
}
```

`enter`, `exit`, `pointer_enter`, `pointer_exit`, `press`, `release`, and custom
event names accept `from`/`to` tables containing `x`, `y`, and `scale`. Defaults
are zero offsets and scale 1. These are drawing transforms, not layout changes.
Offsets must be finite; scale must also be non-negative. A callback-only event
completes on the next animation frame. Keep native controls for keyboard actions;
pointer animation triggers do not make a container keyboard-focusable.

Custom widgets and `iced.component` receive instance-local `self.animate` and
`self.props`. Call `self.animate.exit()` to start exit early, `self.animate.enter()`
to show it again, or `self.animate.<event>()` to run a configured custom event.
Repeated exit calls are ignored. Exit leaves the instance hidden and noninteractive
until removal or re-entry. In `list_view`, a completed exit also releases the item's
layout slot and spacing, letting surviving items displace without changing the
source data. `self.animate.enter()` restores the item to its source position.
The optional exit callback can update parent/shared state when data removal is wanted.
An interrupted animation retargets from its displayed values and cancels the old
completion callback. Reload and window closure also cancel pending callbacks.

Automatic exit on removal retains custom/component roots and native widgets with
an explicit `id` until completion. Repeated custom widgets require distinct
`props.id` values; identity is scoped to the widget name and window. Ordinary
unidentified widgets can animate entry and interactions, but cannot be retained
after removal. Raw function-valued `on_press` handlers run in update and may call
`self.animate.exit()` through their captured instance.

Legacy booleans, flat transition options, and list `onAdded`/`onRemoved`/
`onDisplaced`/`on_item_added`/`on_item_removed` aliases have been removed.

### Custom widgets

`iced.define(name, factory)` creates a reusable constructor. Accessing
`iced.name` while it is undefined searches the profile for a Lua file named
`name.lua` in any directory (the shell file is skipped), loads it on demand, and
returns the constructor. No top-level `require` and no fixed widget directory
are needed. Two files with the same name are an error, and the file basename
must match the widget name (`workspaces.lua` defines `iced.workspaces`). The
factory runs once per definition. Returning a prototype table with a `view`
method (or a `render` function) creates a widget; other non-nil returns create
[named values](#named-values-and-transitions):

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

The name must be a valid identifier and must not already exist on `iced`.
Defining an existing custom definition again replaces it, so edits reload. If a
reload fails, the previous definition is restored and the error is logged; the
shell keeps running. The constructor accepts any number of table arguments and
forwards them to `view`/`render`; `props.id` on the first argument identifies
the instance. Events still go through
`app:update`; there is no per-widget update callback. Build the element inside
`view` when it reads live state or plugin store values. Use a component module
for mounted widgets with lifecycle hooks.

#### Named values and transitions

`iced.define(name, factory)` also registers easing aliases, transition presets,
styles, and other reusable values. Access them with `iced.name()`. The factory
must return a non-nil value; `false` is valid. Tables are snapshotted at definition
time and copied recursively on every call (without metatables); functions and
userdata retain their identity. A table with a callable `view` or `render` is
treated as a widget prototype. A returned element without those methods is a
static value, without a custom widget's instance identity.

```lua
-- soft.lua
iced.define("soft", function() return "ease_in_out" end)

-- fade.lua
iced.define("fade", function()
    return {
        duration = 150,
        easing = iced.soft(),
        enter = { from = { scale = 0 } },
        exit = { to = { scale = 0 }, duration = 80 },
        change = { props = { "width" } },
    }
end)

-- In a view:
iced.container(child, {
    id = "panel",
    transition = iced.transition("fade", {
        duration = 90,
        exit = { duration = 60 },
    }),
})
```

`iced.transition(preset, overrides?)` accepts a definition name or an inline table:

- Names use the same lazy `<name>.lua` loader as widgets. Unknown names, widget
  definitions, and non-table values are errors.
- Overrides merge nested fields without mutating the preset or caller's tables.
  Arrays and `props` replace whole values, including `props = {}`. Empty event
  tables retain their existing fields; they do not remove events.
- Root `duration`, `easing`, and `spring` become defaults for each existing event,
  including `change`, `displaced`, and custom events. Event fields win. No events
  are added automatically, and callbacks remain per-event.
- `easing` and `spring` cannot coexist, including when inherited from the root.
  Event timing can override the same kind of timing, but cannot switch between
  easing and spring while inheriting the other.
- The result is a fresh event table accepted by the existing animation parser.
  Timing values, transforms, and widget-specific `change.props` are validated
  when the widget is built.

Use `easing = iced.soft()` for a named alias; `easing = "soft"` is not resolved.
Easing aliases use the existing built-in curves; custom curve functions and
cubic-bezier tables are not supported. Direct `transition = { ... }` tables keep
their existing event-only syntax; root timing defaults require `iced.transition`.
To observe redefinitions in a view, call `iced.name()` or `iced.transition("name")`
there rather than caching their returned values in `shell.lua`.

### Components

See [`iced.component`](#icedcomponentid-module-props) for the component
reference and lifecycle hooks.

Editing a component module reloads it and every module that required it.
Returned module tables keep their identity, so a cached `require` result sees
updated methods. A separately cached function or value (for example
`local view = panel.view` in `shell.lua`) needs a shell reload. A broken save
keeps the previous modules and UI and is not retried until the file changes
again.

Components are expanded through layout containers, `table` cells, and
`pane_grid` panes. Elements returned by a `responsive` callback or a
`list_view` `render` callback are rendered directly and are not mounted as
components.

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

### Popup surfaces

`iced.popup(content, settings)` declares a native XDG popup parented to a layer
surface. Keep it in `app:view` and set `visible = false` to hide it, or omit it
to close it. The parent must be an `iced.layer` declared in the same view.

```lua
function app:view()
    return iced.layer(
        iced.row({
            iced.button(iced.text("Menu"), { id = "bar_button", on_press = "menu" }),
            iced.popup(iced.text("menu"), {
                id = "menu",
                visible = self.menu_open,
                width = 180,
                height = 120,
                anchor = "bar_button",
                anchor_edge = iced.PopupAnchor.bottom,
                gravity = iced.PopupGravity.bottom,
            }),
        }),
        { id = "bar", size = { width = 200, height = 30 } }
    )
end
```

A popup nested anywhere in a layer or window content is hoisted to a
surface automatically, so `app:view` can return only the layer. Its `parent`
defaults to the containing layer. This also works inside custom widgets: return
a row containing the popup from the widget's `view`. Popups returned from a
`list_view` `render` callback are not hoisted; declare them as siblings of the
list instead.

| Key                     | Values                                         | Default                                              |
| ----------------------- | ---------------------------------------------- | ---------------------------------------------------- |
| `id`                    | nonempty string                                | position-based                                       |
| `visible`               | boolean                                        | true                                                 |
| `parent`                | `iced.layer` id                                | containing layer for nested popups; required at root |
| `width`, `height`       | number in 1..32768, rounded                    | required                                             |
| `x`, `y`                | finite number, rounded                         | 0, 0                                                 |
| `anchor`                | widget id string, or `{ x, y, width, height }` | none                                                 |
| `anchor_edge`           | `iced.PopupAnchor` value                       | `"bottom_left"`                                      |
| `gravity`               | `iced.PopupGravity` value                      | `"bottom_right"`                                     |
| `constraint_adjustment` | `iced.PopupConstraintAdjustment` value or list | flip and slide both axes                             |
| `on_closed`             | function                                       | none                                                 |

`width`, `height`, `x`, `y`, and anchor values are logical pixels. `x` and `y`
are relative to the parent layer surface and are used when `anchor` is absent.
When `anchor` is absent, `(x, y)` acts as a 1x1 anchor rectangle, so
`anchor_edge` and `gravity` still apply.
`anchor` attaches the popup to a rectangle in the parent surface. Pass a widget
`id` to use that widget's measured bounds; any widget with an `id` prop is
measured automatically, so `iced.measure` is not required. Until the target has
been laid out once, the popup stays hidden; the first measurement rebuild opens
it. The target should live on the popup's parent layer so both use the same
surface coordinates. A `{ x, y, width, height }` table is also accepted for a
rectangle that is not a widget. Popup ids share the surface-id namespace
with layers and windows. The size is fixed; the popup does not measure or resize
itself. Changing `width`, `height`, `x`, `y`, `anchor`, `anchor_edge`, `gravity`,
or `constraint_adjustment` repositions the open popup; changing `parent` recreates
it. Popups are created after their parent layer.

`anchor_edge` selects the point on the anchor rectangle that the popup is
attached to. `gravity` selects the direction the popup grows from that point:

```lua
-- Below a bar button.
anchor_edge = iced.PopupAnchor.bottom,
gravity = iced.PopupGravity.bottom,

-- To the right of a button.
anchor_edge = iced.PopupAnchor.right,
gravity = iced.PopupGravity.right,
```

Both accept one value, not a list. `"none"` centers the popup on the anchor
rectangle. Corners such as `"top_left"` attach or grow diagonally. The compositor
may still move the popup to keep it on screen.

`constraint_adjustment` controls how the compositor may adjust a constrained
popup. Pass one value or a list:

```lua
constraint_adjustment = iced.PopupConstraintAdjustment.all,
-- or
constraint_adjustment = { "flip_x", "slide_y" },
```

Available values are `"none"`, `"all"`, `"flip_x"`, `"flip_y"`, `"slide_x"`,
`"slide_y"`, `"resize_x"`, and `"resize_y"`. `"all"` combines flip, slide, and
resize on both axes; `"none"` disables adjustment.

`visible = false` removes the native popup but keeps the declaration in place.
Setting it back to true opens a fresh popup. This is equivalent to omitting and
re-adding the declaration.

`on_closed` runs when the compositor dismisses the popup or a window-close task
closes it:

```lua
on_closed = function()
    shell.menu = false
end
```

The usual `{ type = "window_closed", id = "menu" }` update event is still sent.
Use either path, not both, or make both idempotent. `on_closed` is not called
when the view itself hides or removes the popup, because that state change has
already happened.

The popup can contain components and ordinary widgets, but it cannot contain
another surface root. The popup surface paints only its content; wrap it in a
styled `iced.container` for a background, border, radius, or padding. The
compositor may flip or slide it to stay on screen.

Closing works like native windows:

- Omitting the popup from `app:view` removes it. Inside the popup,
  `props.window` is its window handle, so a
  `{ type = "window", window = props.window, action = "close" }` task closes it.
- When the compositor dismisses it, `update` receives
  `{ type = "window_closed", id = "menu" }`. Set your state to false there.
- If the view keeps declaring the popup after it was closed, it stays closed
  until `visible` is set to false and true again, or the declaration is removed
  and added again.
- Removing the parent layer while a popup is still declared rejects the surface
  change and keeps the previous surfaces.

Popups have no open/close animation or lifecycle callbacks. Return a different
`id` or remove and re-add the declaration to reopen one.

### Native windows

`iced.window` declares a normal XDG toplevel in `app:view`, alone or mixed with
layer surfaces:

```lua
function app:view()
    if not self.settings_open then return iced.text("closed") end
    return iced.window({
        id = "settings",
        title = "Settings",
        width = 800,
        height = 600,
        content = iced.text("settings"),
    })
end
```

| Key                       | Values              | Default        |
| ------------------------- | ------------------- | -------------- |
| `id`                      | nonempty string     | position-based |
| `title`                   | string without NUL  | `"Riced"`      |
| `width`, `height`         | integer in 1..32768 | 960, 720       |
| `client_side_decorations` | boolean             | false          |
| `content`                 | iced element        | required       |

Returning the window declares it and opens it; omitting it closes it. Window
ids share a namespace with layer ids. Changing `width`, `height`, or
`client_side_decorations` recreates the window; title changes reuse it.

When the compositor closes a window, `update` receives:

```lua
{ type = "window_closed", id = "settings" }
```

Set your visibility state to false there. If the view keeps declaring the same
window after it was closed, it stays closed until the declaration is removed
and added again. Surface changes apply after IPC commands and reloads, or on
the next 250 ms reload check when a render sees a different surface list.

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
- `shader` is wgpu-only; the tiny-skia fallback renders nothing. Shaders use a
  fixed vertex stage, `resolution`/`time` uniforms, and up to 16 named
  `extraArgs` floats.
- `responsive` runs Lua during layout; its callback must not mutate app state.
  Errors fall back to empty space.
- `pane_grid` delivers resize and drag events, but resize ratios cannot be set
  from Lua; the host keeps the live layout per `id` until a shell reload.
- `text_editor`, `combo_box`, and `pane_grid` state is keyed by `id`, bounded to
  256 entries per widget kind, and is not shared between multiple `riced`
  instances. Reuse ids to preserve state; exceeding the limit resets the cache,
  and the replaced state is freed at the next shell reload.
- `list_view` identity is `key` when set, otherwise value equality. With
  recreated table items, set `key` (for example `"id"`) or middle
  inserts/removals fall back to index matching and do not displace.
- Generic transitions cannot fade a whole widget group. `svg` exposes its own
  `opacity`, and text color transitions include alpha; there is no group
  opacity.
- Standard Wayland protocols do not associate toplevels with workspaces.
- `pane_grid` styles and other widget behavior follow iced 0.14. A widget not
  listed in this document is not exposed.
