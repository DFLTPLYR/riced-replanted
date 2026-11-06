# riced-wayland API

`riced-wayland` publishes compositor-independent Wayland output, toplevel, and
workspace data through the `wayland` plugin store.

- Binary: `riced-wayland`
- Lua proxy: `riced.plugin("wayland")` (also `iced.plugins("wayland")`)
- Store fields: `connected`, `capabilities`, `outputs`, `toplevels`,
  `workspaces`, `workspace_groups`, `focused`
- Methods: `activate_toplevel`, `close_toplevel`, `activate_workspace`
- Events: none

## Install

```sh
just extension-wayland
```

Or build directly into a profile:

```sh
riced extension build extensions/wayland -o <profile>/extensions/wayland
```

## Common plugin behavior

The proxy is live: `alive` is false and the store is empty until the process
connects. Reads are synchronous; each publish replaces the snapshot while the
proxy table identity stays the same. All methods are asynchronous and take an
optional callback as the last argument. On success the callback receives
`(nil, nil)`; on failure it receives `(nil, error)`. Without a callback, method
failures are not surfaced to Lua.

See the [riced API](../../API.md#plugins-and-extensions) for shared plugin and
extension semantics.

## Protocols

The extension uses:

- `wl_output` for outputs.
- `ext-workspace-v1` for workspaces and workspace groups.
- `zwlr-foreign-toplevel-management-v1` for controllable toplevels when
  available.
- `ext-foreign-toplevel-list-v1` as a read-only toplevel fallback.

Missing protocols produce empty data and `false` capability flags. The extension
is not compositor-specific and publishes store data only.

## Store

| Field              | Type / shape                                                                                                                    |
| ------------------ | ------------------------------------------------------------------------------------------------------------------------------- |
| `connected`        | boolean; false while the compositor connection is down                                                                          |
| `capabilities`     | `{ toplevels = boolean, toplevel_control = boolean, workspaces = boolean }`                                                     |
| `outputs`          | array of `{ id, name, description, make, model, x, y, width, height, scale }`                                                   |
| `toplevels`        | array of `{ id, title, app_id, outputs, activated, maximized, minimized, fullscreen }`; the read-only fallback uses `identifier` instead |
| `workspaces`       | array of `{ id, protocol_id, name, coordinates, active, urgent, hidden, capabilities, group, outputs }`                         |
| `workspace_groups` | array of `{ id, capabilities, outputs }`                                                                                        |
| `focused`          | `{ toplevel = string or nil, workspace = string or nil, output = string or nil }`                                               |

`outputs` come from `wl_output`. `id` is the registry name, `name` is the
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
`activate`, `deactivate`, `remove`, and `assign`. `group` is a workspace-group id
or nil. `outputs` is derived from the workspace group and may be empty.
Workspace-group `capabilities` currently contains `create_workspace` when
supported.

`focused` contains only unambiguous values. `toplevel` is nil with the read-only
backend. `workspace` is nil unless exactly one workspace is active. `output` is
usually a connector name; it falls back to an output object id if the output
disappeared, and is nil when focus cannot be attributed to one output.

## Methods

| Method                   | Arguments | Requirements                                                | Errors                                                                                       |
| ------------------------ | --------- | ----------------------------------------------------------- | -------------------------------------------------------------------------------------------- |
| `activate_toplevel(id)`  | string    | `toplevel_control`, a live toplevel handle, and a `wl_seat` | not connected, unknown/stale toplevel, no seat, or Wayland flush failure                     |
| `close_toplevel(id)`     | string    | `toplevel_control` and a live toplevel handle               | not connected, unknown/stale toplevel, or Wayland flush failure                              |
| `activate_workspace(id)` | string    | `ext-workspace-v1` and workspace capability `activate`      | not connected, unknown/stale workspace, no manager, unsupported capability, or flush failure |

All three return no value on success. IDs must come from the current `toplevels`
or `workspaces` snapshot; handles become invalid when the compositor closes the
object.

## Behavior and limits

`capabilities.toplevels` is true with either toplevel backend.
`capabilities.toplevel_control` is false with only the read-only backend.
`capabilities.workspaces` is true only with `ext-workspace-v1`. Standard
protocols do not map toplevels to workspaces.

Snapshots are bounded by the protocol line limit, so very large arrays may omit
entries; text is clamped. On disconnect the extension retries every second;
`connected` is false and the previous arrays may remain until the next
successful snapshot.

## Example

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
