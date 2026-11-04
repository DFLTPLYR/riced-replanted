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
cargo run -- -p test
```

`riced -p test` loads `$XDG_CONFIG_HOME/riced/test/shell.lua`, falling back to
`$HOME/.config`. Profile names must be a single directory name. `require` also
searches the profile directory.

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
socket. `riced` binds `$XDG_RUNTIME_DIR/riced/<profile>.sock` and spawns every
executable in `<profile>/extensions/` (an executable file, or a directory
containing one). A crashed extension is restarted with backoff; removing the
file stops it. Extensions can also be started manually with `RICED_SOCKET` set.

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
```

The extension SDK is [`crates/riced-extension-sdk`](crates/riced-extension-sdk).
Third-party extensions depend on it by git or path, build their own binary, and
never need `riced` rebuilt. Extensions run as your user and have the same trust
as `shell.lua`.

### Logging

Core and SDK diagnostics use the shared `log` facade and write to stderr as
`[event]: information`. `Plugin::serve` / `serve_at` install the default logger
automatically, preserving any logger the extension has already installed.
On a terminal, `[event]` is colored by severity: errors red, warnings yellow,
info green, debug cyan, and trace gray. Redirected logs stay plain; setting
`NO_COLOR` disables color.

Lua profiles can use `log(...)` instead of `print(...)`:

```lua
log("called") -- [lua]: called
log("settings", state.settings) -- values are tab-separated, like print
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
