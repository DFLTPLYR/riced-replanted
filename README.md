# riced

Lua-driven Iced Wayland layer-shell daemon. Profiles are plain Lua: build
widgets with the injected `iced` API, keep state in tables, and return a
`riced.daemon`. Compositor, tray, and system data come from extensions, so the
core is not tied to one compositor.

## Quick start

Copy the example profile, then run it:

```sh
mkdir -p ~/.config/riced/test
cp -r example/. ~/.config/riced/test/
cargo run -- -c test
```

```sh
riced                         # ~/.config/riced/shell.lua
riced -c test                 # ~/.config/riced/test/shell.lua
riced -p ~/path/config        # ~/path/config/shell.lua
```

An absolute `$XDG_CONFIG_HOME` replaces `$HOME/.config` for the default directory
and named configurations. `-c` / `--config` accepts a single directory name;
`-p` / `--path` accepts a configuration directory, with relative paths resolved
from the current directory. The shell expands `~` in unquoted paths. The two
selectors cannot be combined. `require` also searches the selected directory.
Use `riced --help`, `riced ipc --help`, or `riced --version` for CLI information.

Previously, `-p test` selected a named profile; use `-c test` for that now.

Missing files, Lua errors, and invalid settings exit with a nonzero status.
Profiles have access to Lua's standard libraries, including filesystem and
process APIs, so only run trusted configs.

A profile looks like:

```text
~/.config/riced/test/
├── shell.lua              -- entry point, returns riced.daemon(...):run()
├── .luarc.json            -- editor setup for this profile
├── components/            -- watched modules used by iced.component
│   ├── panel.lua
│   ├── system.lua
│   ├── wayland.lua
│   └── layers/
│       └── background.lua
├── extensions/            -- optional extension executables
│   ├── system/riced-system
│   ├── wayland/riced-wayland
│   └── dbus/riced-dbus
└── stubs/riced.lua        -- lua-language-server definitions
```

See [`example/shell.lua`](example/shell.lua) for a working profile.

## Reload on save

`shell.lua` is checked every 250 ms. Saving changed content reloads the Lua app,
callbacks, theme, and subscriptions in place, resetting Lua state. Syntax errors
and errors found while validating existing views keep the current app running;
check the terminal for the error, fix it, and save again.

Lua modules loaded through `require` (including `iced.component` modules) are
watched too. Saving one reloads that module and its dependents without resetting
app state. Layer modules can be required either at the top of `shell.lua` or
inside `app:view`. Edits are detected from file contents; changing an `id` is
not necessary.

Namespace and startup settings changes require a restart and are rejected during
reload. Callback errors first encountered after reload (for example, in `update`)
remain fatal. Config evaluation and validation can run Lua side effects even
when a reload is rejected.

## Editor support

`example/stubs/riced.lua` is a lua-language-server `---@meta` file for the
injected globals (`riced`, `iced`, `theme`, `config`). `example/.luarc.json`
loads it, so completions and undefined-global diagnostics work while editing a
profile copied from `example/`. Copy both, or point `workspace.library` at
`example/stubs`. Each profile needs its own `.luarc.json` because `require`
resolves from the profile directory. The stub is hand-written; update it when
the Lua API changes.

## Extensions

Extensions are separate executables that connect back to `riced` over a Unix
socket. `riced` binds `$XDG_RUNTIME_DIR/riced/config-<device>-<inode>.sock` and
spawns every executable in `<profile>/extensions/` (an executable file, or a directory
containing one). A crashed extension is restarted with backoff; removing the
file stops it. Extensions can also be started manually with `RICED_SOCKET` set.
Socket IDs use the configuration directory's device and inode in hexadecimal:
directory aliases share an instance, while directories with equal names stay distinct.
Restart existing daemons after upgrading to use the new IPC socket names.

Each extension publishes a watched snapshot (`store`) and may declare methods.
Lua reads the store synchronously, so `view` stays side-effect free:

```lua
local wayland = riced.plugin("wayland")
if wayland.connected then
    iced.text(wayland.focused.output or "no output")
end
```

The bundled extensions are:

- `riced-system` — system metrics.
- `riced-wayland` — outputs, toplevels, and workspaces using standard Wayland
  protocols.
- `riced-dbus` — StatusNotifier tray items and freedesktop notifications.
- `riced-themeagen` — Material You palettes and Matugen-compatible templates.
- `riced-lucide` — icon-name lookup and the extension-owned `iced.lucide` widget.
- `riced-openmeteo` — current weather, automatic refresh, and `iced.openmeteo`.

The D-Bus and Wayland extensions bound display text and array snapshots to stay
below the protocol's 256 KiB message limit. Entries that exceed an array's byte
budget are omitted with a diagnostic; identifiers and action keys are preserved.
Notifications retain at most 64 entries, 16 actions per entry, and 1024 characters
each for summary and body. Ambiguous multi-monitor focus is reported as `nil`.

Build and install the examples into `~/.config/riced/test/extensions/`:

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

This builds the package in release mode and packages its executable plus
top-level `*.lua` companions. Omitting `-o` installs into `extensions/` of the
profile named by `-c`/`-p`, or of the only running daemon. Each daemon records
its canonical `shell.lua` in `$XDG_RUNTIME_DIR/riced/<profile>.profile` at
startup, and stale records are rejected. It does not restart daemons.

The extension SDK is [`crates/riced-extension-sdk`](crates/riced-extension-sdk).
Third-party extensions depend on it by git or path, build their own binary, and
never need `riced` rebuilt. Extensions run as your user and have the same trust
as `shell.lua`.

Extensions can ship non-executable Lua files beside their binary. A file named
`demo.lua` can use `iced.define("demo", factory)` to provide `iced.demo(props)`;
the existing loader discovers it automatically. Returned Lua objects are also
available through `require("extensions.demo.demo")`. See
[extension packaging](API.md#writing-extensions).

Lucide installs its own `lucide.lua` companion:

```lua
iced.lucide({ name = "activity", size = 20 })
```

It resolves names asynchronously and caches glyphs. For raw glyphs, use
`iced.icon(glyph, props)`; this replaces the old `iced.lucide(glyph, props)` alias.
When upgrading, reconcile and remove any old profile-root `lucide.lua` to avoid
duplicate widget names, and change `require("lucide")` to
`require("extensions.lucide.lucide")`.

Open-Meteo uses one location per extension, configured from Lua:

```lua
local weather = require("extensions.openmeteo.openmeteo")
weather.configure(52.52, 13.41)
-- Inside view:
iced.openmeteo({ size = 18 })
```

It refreshes every 15 minutes and marks retained readings stale on errors.
See the [Open-Meteo API](API.md#open-meteo-extension) for raw weather fields.

### Logging

Core and SDK diagnostics use the shared `log` facade in `[event]: information`
form. The daemon stores every record, including extension stderr, in
`$XDG_RUNTIME_DIR/riced/logs/` and stays quiet by default:

```sh
riced --logs                  # mirror records to stderr
riced logs                    # print the stored records
riced logs --clear            # remove them
riced --log-retention 60      # seconds to keep records; 0 keeps until cleared
```

Retention defaults to 30 seconds. Fatal startup errors still print to stderr.
`riced logs -c <name>` or `-p <directory>` selects a configuration; without
either, exactly one running daemon is required. `Plugin::serve` / `serve_at`
install the default logger automatically, preserving any logger the extension
has already installed.
On a terminal, `[event]` is colored by severity: errors red, warnings yellow,
info green, debug cyan, and trace gray. Redirected logs stay plain; setting
`NO_COLOR` disables color.

Lua profiles can use `log(...)` instead of `print(...)`:

```lua
log("called") -- [lua]: called
log("settings", state.settings) -- values are tab-separated, like print
log.table(state) -- tables expand recursively with sorted keys
```

Lua logs use info level and the same terminal colors and `NO_COLOR` behavior.

```rust
use riced_extension_sdk::log;

log::info!(target: "dbus.connected", "connected to the session bus");
// [dbus.connected]: connected to the session bus
log::warn!(target: "dbus.retry", "retrying after {error}");
```

For logging before `serve`, call `riced_extension_sdk::logging::init()` first.
Info, warnings, and errors are enabled by default; use `log::set_max_level` to
change verbosity. Logging is separate from `Context::event`, which sends an
application event to Lua.

## Documentation

[`API.md`](API.md) is the complete Lua and component reference. Start with:

| Topic | Reference |
| --- | --- |
| Daemon builder and callbacks | [`riced.daemon`](API.md#riceddaemon) |
| IPC commands | [`riced.ipc`](API.md#ricedipc) |
| Theme and window style | [`config` and `theme`](API.md#config-and-theme) |
| JSON/TOML files with save callbacks | [`riced.file`](API.md#ricedfile) |
| Extension stores, methods, events | [Plugins and extensions](API.md#plugins-and-extensions) |
| All widget constructors and props | [Widget reference](API.md#widget-reference) |
| Widget events and styles | [Widget events](API.md#widget-events), [Styles](API.md#styles) |
| Tasks and subscriptions | [Tasks](API.md#tasks), [Subscriptions](API.md#subscriptions) |
| Animations | [Property transitions](API.md#property-transitions) |
| Reusable widgets and watched components | [Custom widgets](API.md#custom-widgets), [Components](API.md#components) |
| Layer surfaces and click-through regions | [`iced.layer`](API.md#icedlayercontent-settings), [`iced.region`](API.md#icedregioncontent) |
| Native windows | [`iced.window`](API.md#native-windows) |
| Known limits | [Known limits](API.md#known-limits) |

`list_view`, `table`, `markdown`, `canvas`, `pane_grid`, and the remaining
widgets are documented in the [widget reference](API.md#widget-reference).

## Building

Building requires the native Wayland/XKB development libraries, including
`xkbcommon` discoverable by `pkg-config`. Running requires a Wayland compositor
supporting layer-shell.

```sh
just check       # fmt, tests, clippy, whitespace
just build       # release build through the dev shell
just test        # build, replace a running riced, load the test profile
just install     # install the binary to ~/.local/bin
```

Without `just`, `cargo build`, `cargo test --workspace`, and `cargo clippy
--workspace --all-targets` work the same way inside the development
environment.
