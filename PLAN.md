# Plan: external plugins + Hyprland workspaces/toplevels

Status: host, SDK, supervisor, and the `system` example are implemented. The Hyprland
workspaces/toplevels plugin (Phase 4) is not started.

Goal: `riced` gains an out-of-process plugin host. Plugins are separate processes that
publish watched state and expose async actions. Adding, removing, restarting, or updating
a plugin requires **no rebuild and no restart of `riced`**. The first plugin is a Hyprland
watcher exposing workspaces, monitors, clients/toplevels, and focus state, plus a few
actions.

Non-goals: in-process native plugins (`dlopen`), plugin sandboxing, non-Hyprland
compositors, multiple riced instances per profile.

## Decisions

- **Out-of-process, Unix-socket JSON lines.** Lua callbacks are synchronous on the iced
  UI thread; in-process native plugins would need an ABI we own forever. IPC keeps one
  `riced` binary and survives plugin crashes.
- **Two plugin surfaces:** watched `store` (pushed snapshots, read synchronously in
  `view`) and `methods` (async calls for actions/queries). No synchronous RPC.
- **Host owns plugin data.** Shell reload creates a new `Lua`; the plugin registry and
  last snapshots live in `App` as `serde_json::Value`, and proxy tables are rebuilt into
  the new Lua state.
- **Hyprland IPC for the first plugin, not Wayland protocols.** Workspaces have no
  standard Wayland protocol. `j/workspaces` + `j/clients` + `j/monitors` give toplevels
  with workspace/monitor/focus association in one source. Adding
  `ext-foreign-toplevel-list-v1` / `zwlr-foreign-toplevel-management-v1` would add
  protocol boilerplate and still not provide workspace names.
- **Full refetch on Hyprland events.** Replicate no state machine; on a relevant event,
  debounce ~50 ms, re-request the three JSON endpoints, publish one snapshot.
  `shortcut: full refetch per event, switch to incremental state if it shows up in
  profiles`. Refetch is two small local requests.
- **100 ms plugin poll.** Keeps workspace pills responsive; the existing 250 ms
  `CheckReload` poll stays for files. Plugin timer only runs while a plugin is registered.
- **No plugin sandbox.** Socket dir `0700`, socket `0600`, `SO_PEERCRED` UID check. A
  plugin is trusted like `shell.lua`.

## Protocol v1

Socket: `$XDG_RUNTIME_DIR/riced/<profile>.sock`, newline-delimited JSON, 256 KiB
line cap. Bounded per-plugin queues (16 messages) that drop old state snapshots, so a
stuck plugin cannot grow host memory.

Plugin -> riced on connect:

```json
{"type":"register","v":1,"name":"hyprland",
 "state":["monitors","workspaces","clients","focused"],
 "methods":{"focus_workspace":{"readonly":false},
            "focus_client":{"readonly":false},
            "close_client":{"readonly":false},
            "toggle_special":{"readonly":false}}}
```

Watched state (replace, not merge):

```json
{"type":"state","store":{"workspaces":[],"clients":[],"focused":{}}}
```

Unsolicited event, delivered to `app:update`:

```json
{"type":"event","event":{"type":"hyprland","kind":"workspace","name":"2"}}
```

riced -> plugin when Lua calls a method:

```json
{"type":"call","id":7,"method":"focus_workspace","args":["2"]}
```

Plugin -> riced result:

```json
{"type":"result","id":7,"ok":true,"result":null}
{"type":"result","id":7,"ok":false,"error":"unknown workspace"}
```

Rules:

- `name` matches `^[a-z0-9_-]{1,32}$`. Duplicate name: reject the new connection with
  `{"type":"error","error":"duplicate plugin"}`; the plugin retries after the old socket
  closes.
- Unknown `v` or unknown `type`: error and close.
- EOF = disconnected: keep the last `store`, set `alive = false`, fail pending callbacks,
  redraw.
- `readonly` is SDK metadata only. All methods are async. Read-only *data* belongs in
  `store`; do not add a synchronous call path.

## Lua API

```lua
-- view: synchronous read of last published snapshot
local hypr = riced.plugins("hyprland")      -- nil until the plugin registers
if hypr and hypr.alive then
    for _, ws in ipairs(hypr.store.workspaces) do ... end
end

-- update: async actions
hypr.focus_workspace("2")
hypr.focus_client("0x55d...")
hypr.close_client("0x55d...")

-- optional result for queries
hypr.query("something", { arg = 1 }, function(result, err) ... end)
```

- `riced.plugins(name)` returns the proxy or `nil`. Proxy fields: `name`, `alive`,
  `store` (plain Lua table, same identity; contents replaced on each snapshot), and one function per declared
  method. Call methods with `.` (`hypr.focus_workspace("2")`), not `:`.
- Methods with a trailing Lua function receive `(result, err)` when the result arrives;
  without a callback the call is fire-and-forget.
- Events arrive as `{ type = "plugin", plugin = "hyprland", event = <payload> }`.
- `view` stays side-effect free; only `store` is read there.

## Phases

### Phase 0 - workspace and dependencies

- Add `[workspace]` to root `Cargo.toml`: `members = [".", "crates/riced-extension-sdk", "extensions/hyprland"]`.
  Root package stays the `riced` binary; `justfile` already runs `cargo ... --workspace`.
- Add `serde` + `serde_json` to `riced` and `riced-extension-sdk`. No new system libraries.
- No changes to `iced`/Wayland dependencies.

### Third-party plugin crates

Cargo workspace membership is compile-time only and is not required for plugins. An
external plugin is its own crate/binary that depends on the SDK by git (or crates.io),
and never becomes a member of riced's workspace:

```toml
[dependencies]
riced-extension-sdk = { git = "https://example.com/riced", tag = "v0.1.0", features = ["process"] }
```

Local development can use `path = "../riced/crates/riced-extension-sdk"`, but a `path`
dependency embeds the author's filesystem layout, so distribution uses git or a published
crate. `members = ["crates/*", "extensions/*"]` in riced's workspace only auto-adds crates
inside riced's checkout (handy for in-repo examples); it does not make a running `riced`
load anything. The plugin author/user builds the plugin binary; `riced` only connects to
it at runtime, so no `riced` rebuild is involved.

### Phase 1 - SDK crate `crates/riced-extension-sdk`

Files: `src/lib.rs`, `src/protocol.rs`.

- `protocol` types mirroring v1 (`Register`, `State`, `Event`, `Call`, `Result`, `Error`)
  with `#[serde(tag = "type", rename_all = "lowercase")]`.
- `Plugin::new(name)` + `.state_keys([...])` + `.method(name, readonly, handler)`.
- `serve(socket_path) -> !`: connect loop, spawn a reader thread dispatching `call` to
  handlers, writer via `mpsc` (never write from the handler thread directly).
- `Context` given to the plugin's blocking loop: `ctx.publish(json)` and
  `ctx.event(json)`.
- Reconnect with 100 ms -> 2 s backoff while `riced` is down.
- Unit tests: message round-trips, unknown type rejected, oversized line rejected.

### Phase 2 - host registry and socket `src/plugins.rs`

- `Plugins::start(profile: &str) -> io::Result<Plugins>`:
  - resolve `$XDG_RUNTIME_DIR`, create `riced/` at `0700`, bind `<profile>.sock` at
    `0600`, remove stale socket, fail clearly if already bound.
  - listener thread: accept, `SO_PEERCRED` UID check, one reader thread per connection,
    `std::sync::mpsc` of `Incoming::{Register,State,Event,Result,Closed}`.
  - per-plugin writer thread fed by `mpsc<Vec<u8>>`.
- `Handle { name, writer, store: serde_json::Value, alive: bool, pending: HashMap<u64, RegistryKey> }`.
- `Plugins::poll(&mut self, lua: &Lua) -> mlua::Result<Vec<Update>>`:
  drain the channel, coalesce per-plugin states (last wins), build/refresh Lua proxies,
  invoke result callbacks, return staged events for `App` to feed into `update`.
  `Update::{Redraw, Event(String, Value)}`.
- `Plugins::install(&mut self, lua: &Lua)` rebuilds `__riced_plugins` and every proxy
  from host-side snapshots (used after shell reload).
- Shell-reload semantics: `Plugins` is moved into the candidate `App` **before** shell
  validation (views may read `riced.plugins`), and moved back if validation fails.
  Pending callbacks are dropped (their Lua state is gone); log one line.
- Limits/validation: name regex, duplicate name, 256 KiB line cap, 16-deep queues,
  64 pending calls per plugin with a 5 s timeout, 128 KiB thread stacks, unknown `v`/`type`,
  malformed JSON = close connection with a log line.
- Tests (no Wayland): register/state/replace, duplicate reject, disconnect sets
  `alive=false` and keeps store, result callback fires once, oversized/malformed
  message closes only that connection, reload re-installs proxies.
- `crates/riced-extension-sdk` is used by tests as a fake plugin where practical.

### Phase 3 - wiring `src/api.lua`, `src/runtime.rs`, `src/main.rs`

- `api.lua`: `__riced_plugins = {}` and
  `function riced.plugins(name) return __riced_plugins[name] end`.
- `main.rs`: derive profile string, `Plugins::start(profile)`, pass into `runtime::load`.
- `runtime.rs`:
  - `App` gains `plugins: Plugins`.
  - `load(source, path, plugins)`: install existing proxies after `api.lua`.
  - `Message::PollPlugins`; subscription adds
    `iced::time::every(100ms).map(|_| Message::PollPlugins)` only when
    `app.plugins.active()`; `redraw()` returns `Scope::None` for it.
  - iced closure: handle `PollPlugins` before `CheckReload`; `app.poll_plugins()` merges
    `Change::All` and feeds `Update::Event` into the update path.
  - `reload()`: carry `Plugins` into the candidate before validation, restore on error.
  - `Module::reload` must not touch plugin state.
- `runtime::load` signature change touches unit tests; update them to pass a test
  `Plugins` (temp socket) or `Plugins::none()`.

### Phase 4 - Hyprland plugin `extensions/hyprland`

Binary `riced-hyprland`.

- Discover `HYPRLAND_INSTANCE_SIGNATURE` and `$XDG_RUNTIME_DIR/hypr/<sig>/`; error out
  with a clear message if absent.
- Connect `.socket.sock` for requests and `.socket2.sock` for events. Hyprland 0.56
  in this environment; only use documented events (`workspace`, `workspacev2`,
  `focusedmon`, `activewindow`, `openwindow`, `closewindow`, `movewindow`,
  `createworkspace`, `destroyworkspace`, `renameworkspace`, `moveworkspace`,
  `windowtitle`, `fullscreen`, `changefloatingmode`, `monitoradded`, `monitorremoved`).
- On startup and on a debounced (~50 ms) relevant event: fetch `j/monitors`,
  `j/workspaces`, `j/clients`; build and publish:

```json
{
  "connected": true,
  "monitors": [{"id":0,"name":"DP-1","focused":true,"active_workspace":{"id":1,"name":"1"},"special":false}],
  "workspaces": [{"id":1,"name":"1","monitor":"DP-1","windows":3,"focused":true,"special":false}],
  "clients": [{"address":"0x...","title":"kitty","class":"kitty","workspace":{"id":1,"name":"1"},"monitor":"DP-1","focused":true,"floating":false,"fullscreen":0}],
  "focused": {"workspace":"1","client":"0x..."}
}
```

- Methods (all fire-and-forget): `focus_workspace(name)`, `focus_client(address)`,
  `close_client(address)`, `toggle_special(name)`. Each maps to one `dispatch` command;
  no generic passthrough.
- Hyprland restart: reconnect with backoff, publish `{"connected":false,...}` and keep
  the last snapshot until the refetch succeeds.
- Unit tests: event filter, `j/*` JSON -> snapshot transform from fixtures, command
  strings. Optional `#[ignore]` live smoke test against the running socket.

### Phase 5 - example, docs, validation

- `example/components/hyprland.lua`: workspace pills from `hypr.store.workspaces` and
  the focused client title; guard for `nil`/`alive=false` so the example runs without
  the plugin.
- `example/shell.lua`: optional use of that component (commented hint is enough).
- README: plugin protocol summary, socket path, how to run `riced-hyprland`
  (`exec-once = riced-hyprland` in the Hyprland config), the async/no-sync-call rule.
- Run `just check`; run `graphify update .` after code changes.

### Phase 6 (optional) - supervisor: spawn plugins as children

Same IPC protocol; this only changes who starts the process. `riced` scans
`<profile>/extensions/` for executable files, binds the socket first, then spawns each with
`RICED_SOCKET` and `RICED_PROFILE` in the environment. The SDK's `serve_from_env()`
uses them; manual launch still works with an explicit path.

- `Supervisor { dir, socket, children: HashMap<PathBuf, Child>, backoff }` lives inside
  `Plugins` so shell reloads do not respawn anything.
- Each plugin poll: `try_wait()` every child to reap zombies; respawn a child that exited
  while its file still exists, with 1 s backoff and a give-up after 5 quick failures;
  spawn newly added executables in the directory (hot add).
- Shutdown: on `riced` exit, close the socket and `kill()` + `wait()` remaining children.
  The SDK also exits on socket EOF, so a `SIGKILL`ed `riced` does not leave orphans behind.
- Trust: only executables in the profile directory, no `PATH` lookup. Same trust level as
  `shell.lua`, which can already run processes.
- Cross-check: the `register` message should include the plugin `pid` so the host can
  associate a connection with the spawned child; unknown/unspawned connections are still
  accepted (manual launch).
- Tests: spawn a fake plugin binary from a temp dir, hot-add a second, kill one and assert
  respawn + `alive=false` churn, verify no zombies after shutdown.

## Failure modes

| Event | Behaviour |
| --- | --- |
| Plugin exits | EOF -> `alive=false`, keep store, fail pending callbacks, redraw |
| Plugin restarts | New connection registers; proxy/state replaced |
| Duplicate name | New connection rejected; plugin retries |
| `riced` reloads shell | Proxies rebuilt from host snapshots; pending callbacks dropped |
| Malformed/oversized message | Close that connection, keep daemon and other plugins |
| Hyprland exits | Plugin reconnects with backoff, pushes `connected=false` |
| Lua call with no plugin | `riced.plugins(name) == nil`; config must guard |
| Slow plugin | Calls queue per plugin; `store` updates coalesce, last wins |

## Open questions

1. Toplevel source: Hyprland `j/clients` (recommended, has workspace/monitor) vs Wayland
   `ext-foreign-toplevel-list-v1` (portable, no workspace data). Plan assumes Hyprland.
2. Should the plugin be able to contribute widgets/layers, or only data/actions? Plan
   assumes data/actions only.
3. Autostart: assume the user runs `riced-hyprland` from Hyprland `exec-once`; no
   autostart/plugin-directory scanning inside `riced`.

## Rough size

- SDK ~200 lines + tests, host ~300 + tests, Hyprland plugin ~250 + tests, runtime
  wiring ~80, docs/example ~120. No new system dependencies.
