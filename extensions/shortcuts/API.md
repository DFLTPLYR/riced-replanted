# riced-shortcuts API

`riced-shortcuts` registers compositor global keybindings and forwards them to
Lua as `shortcuts` plugin events.

- Binary: `riced-shortcuts`
- Lua proxy: `riced.plugin("shortcuts")` (also `iced.plugins("shortcuts")`)
- Store fields: `connected`, `compositor`, `binds`
- Methods: `bind`, `unbind`
- Events: `{ shortcut = string }`

## Install

```sh
just extension-shortcuts
```

Or build directly into a profile:

```sh
riced extension build extensions/shortcuts -o <profile>/extensions/shortcuts
```

## Backends

Detected at startup:

| Compositor | Detection                        | Key syntax            |
| ---------- | -------------------------------- | --------------------- |
| Hyprland   | `$HYPRLAND_INSTANCE_SIGNATURE`   | `"SUPER SHIFT,x"`     |
| sway       | `$SWAYSOCK`                      | `"Mod4+Shift+x"`      |

`keys` are passed to the compositor verbatim, so use the backend-native syntax
from the table. With no compositor the store reports `connected = false` and
the methods return errors.

## Store

| Field        | Type / shape                                        |
| ------------ | --------------------------------------------------- |
| `connected`  | boolean                                             |
| `compositor` | `"hyprland"`, `"sway"`, or nil                      |
| `binds`      | array of `{ keys, label }`                          |

## Methods

`bind(keys, label)` binds a chord so pressing it reports `label`; `unbind(keys)`
removes a bind. Labels allow `[A-Za-z0-9_.-]` and 1-64 characters. Each bind
issues the backend's unbind first, so re-binding the same keys after an
extension restart stays idempotent. Bind failures (for example an unknown keys
string) return the compositor's error.

```lua
local shortcuts = riced.plugin("shortcuts")

-- In update:
if event.type == "plugin" and event.plugin == "shortcuts" then
    if event.event.shortcut == "launcher" then
        return self:open_launcher()
    end
end

shortcuts.bind("SUPER,Return", "launcher", function(_, error)
    if error then log(error) end
end)
```

## How it works

The extension asks the compositor to run a short shell command when the chord is
pressed; the command appends the label to a queue file under
`$XDG_RUNTIME_DIR/riced/`, which the extension polls and forwards as events.
Labels are validated so they cannot inject shell syntax.

## Limits

Backend `keys` strings are trusted for syntax beyond the character checks.
Event delivery follows the queue poll (at most ~200 ms of latency).
