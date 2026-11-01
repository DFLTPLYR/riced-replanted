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

## Lua API

Return `riced.daemon(app, namespace, app.update, app.view)` with the chained
options shown in `example/shell.lua`, ending in `:run()`. Rust starts the event
loop after the script returns. Callbacks receive `app` as `self`:

| Callback | Argument | Return |
| --- | --- | --- |
| `update` | button's `on_press` value or `{ type = "tick" }` | nothing; mutate `self` |
| `view` | `{ window = <opaque window ID> }` | an `iced` element, `iced.layer`, or a list of layers |
| `title` | window ID | string or nil |
| `subscription` | `true` (daemon running) | `iced.every(milliseconds)` or nil |
| `theme` | window ID | palette table or nil |
| `style` | `config.theme` | `{ background_color = "#...", text_color = "#..." }` or nil |
| `redraw_scope` | event, before update | `"all"`, `"none"`, a saved `props.window`, or nil |

Options other than update/view are optional. `:theme(config.theme)` accepts
a table directly instead of a callback. `config.theme` and its alias `theme`
are globals with `background`, `text`, `primary`, `secondary`, `success`,
`warning`, and `danger` hex colors. Modify their fields in `shell.lua`.
`secondary` is available to widgets; Iced's base palette has no secondary field.
Runtime callback errors print the Lua traceback and terminate the process.

### Elements

```lua
iced.text("Hello", { size = 24, color = theme.primary })
iced.row({ child1, child2 }, { spacing = 8 })
iced.column({ child1, child2 }, { spacing = 8 })
iced.container(child, { padding = 16, width = "fill", height = "fill" })
iced.button("Click", { on_press = "clicked" }) -- content may also be an element
iced.space(nil, { width = 12, height = 12 })
```

All elements accept `width`, `height` (`"fill"`, `"shrink"`, non-negative
pixels, or `iced.Length.fill_portion(3)`), and uniform `padding`.
No `on_press` means a disabled button.
These six elements and timer subscriptions are the initial supported bindings,
not the full Iced widget/subscription API.

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

### Extensions

Extensions are separate executables that connect back to `riced` over a Unix socket.
`riced` binds `$XDG_RUNTIME_DIR/riced/<profile>.sock` and spawns every executable in
`<profile>/extensions/` (an executable file, or a directory containing one). A crashed
extension is restarted with backoff; removing the file stops it. Extensions can also be
started manually with `RICED_SOCKET` set.

An extension publishes a watched snapshot (`store`) and may declare methods. Lua reads
`store` synchronously, so `view` stays side-effect free:

```lua
local system = riced.plugins("system")
if system and system.alive then
    iced.text(string.format("CPU %.0f%%", system.store.cpu.usage))
end
```

Store fields are lazy: reading a key that is not published yet subscribes to it, and the
extension may compute only the requested fields. Requests are monotonic for the process
lifetime (there is no unsubscribe when a component stops using a field).

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
