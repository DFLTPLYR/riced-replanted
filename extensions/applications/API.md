# riced-applications API

`riced-applications` lists installed XDG desktop entries and launches them.

- Binary: `riced-applications`
- Lua proxy: `riced.plugin("applications")` (also `iced.plugins("applications")`)
- Store fields: `applications`
- Methods: `launch`
- Events: none

## Install

```sh
just extension-applications
```

Or build directly into a profile:

```sh
riced extension build extensions/applications -o <profile>/extensions/applications
```

## Store

`applications` is a name-sorted array:

```lua
{
    id = "firefox",                 -- desktop file id (subdirectories use dashes)
    name = "Firefox",
    icon = "firefox",               -- icon name, or nil
    comment = "Browse the Web",     -- or nil
    terminal = false,
}
```

Entries are read from `$XDG_DATA_HOME/applications` and each
`$XDG_DATA_DIRS/applications`, including subdirectories. A desktop file id is
provided by only the first directory that defines it. `Hidden`, `NoDisplay`, and
non-`Application` entries are skipped. The list is rescanned every 30 seconds and
published when it changes.

Resolve `icon` with `iced.icon_path(entry.icon, { size = 24 })`; entries without
an icon render a fallback.

## Methods

| Method       | Arguments | Behavior                                                     |
| ------------ | --------- | ------------------------------------------------------------ |
| `launch(id)` | string    | Starts the application detached from `riced`                 |

`launch` uses the desktop `Exec` line without a shell and drops field codes that
would need a document argument (`%f`, `%F`, `%u`, `%U`). `%c` and `%k` are
expanded; `%%` becomes a literal percent. `Terminal=true` entries are wrapped in
`$TERMINAL -e` (falling back to `x-terminal-emulator`). Unknown ids and spawn
failures call the callback with `(nil, error)`.

## Example

```lua
local applications = riced.plugin("applications")

for _, entry in ipairs(applications.applications or {}) do
    iced.button(entry.name, {
        on_press = "launch:" .. entry.id,
    })
end

function app:update(event)
    if type(event) == "string" and event:sub(1, 7) == "launch:" then
        applications.launch(event:sub(8), function(_, error)
            if error then log(error) end
        end)
    end
end
```
