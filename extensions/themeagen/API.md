# riced-themeagen API

`riced-themeagen` builds Material You palettes from a seed color or image and
renders Matugen-style `[templates.*]` entries through the `themeagen` plugin
store.

- Binary: `riced-themeagen`
- Lua proxy: `riced.plugin("themeagen")` (also `iced.plugins("themeagen")`)
- Store fields: `ready`, `mode`, `source`, `colors`
- Methods: `generate`, `populate`, `regenerate`
- Events: none

## Install

```sh
just extension-themeagen
```

Or build directly into a profile:

```sh
riced extension build extensions/themeagen -o <profile>/extensions/themeagen
```

## Paths

Default paths:

- Config: `<profile>/data/templates/config.toml`
- State: `<profile>/data/theme/themeagen.json`

`<profile>` is the directory riced was started from, e.g.
`~/.config/riced/replanted` for `riced -c replanted`. Extensions started
outside riced fall back to `$XDG_CONFIG_HOME/riced/data/...`.

`RICED_THEMEAGEN_CONFIG` and `RICED_THEMEAGEN_STATE` override those paths.

Relative `input_path` and `output_path` values resolve against the config
directory; `~/` in `output_path` expands to `$HOME`.

## Common plugin behavior

The proxy is live: `alive` is false and the store is empty until the process
connects, and `generate`/`populate`/`regenerate` are `nil` until then. Guard
calls from `app:theme` (which runs before the first poll) with
`themeagen.alive`. Reads are synchronous; each publish replaces the snapshot
while the proxy table identity stays the same. Methods are asynchronous and
take a callback as the last argument. On success the callback receives
`(result, nil)`; on failure it receives `(nil, error)`. Without a callback,
method failures are not surfaced to Lua.

See the [riced API](../../API.md#plugins-and-extensions) for shared plugin and
extension semantics.

## Store

| Field    | Type         | Meaning                                                                                                              |
| -------- | ------------ | -------------------------------------------------------------------------------------------------------------------- |
| `ready`  | boolean      | A saved palette loaded and can be used                                                                               |
| `mode`   | string       | Active mode, `"dark"` or `"light"`                                                                                   |
| `source` | table or nil | How the palette was produced: `{kind="seed", seed, variant}`, `{kind="image", image, variant}`, or `{kind="manual"}` |
| `colors` | table or nil | Both palettes, keyed by `dark` and `light`; nil before the first palette exists                                      |

Before a palette exists, `ready` is false, `mode` is `dark`, and `source` and
`colors` are nil.

```lua
local theme = iced.plugins("themeagen") -- same proxy as riced.plugins
local colors = theme.colors

if colors then
    local active = colors[theme.mode]
    iced.text("theme", { color = active.primary })
end
```

Use `result.colors` inside a method callback when you need the new palette
immediately; the published `theme.colors` table is replaced on the next
snapshot.

## Methods

| Method                                        | Behavior                                                                             |
| --------------------------------------------- | ------------------------------------------------------------------------------------ |
| `generate({seed=..., mode=..., variant=...})` | Build both modes from `#rrggbb`, save, publish, and render templates.                |
| `generate({image=..., ...})`                  | Same, with the seed sampled from a local image.                                      |
| `populate({colors=..., mode=...})`            | Apply a supplied `{dark=..., light=...}` palette. Without `colors`, re-render saved. |
| `populate({mode="light"})`                    | Switch mode and re-render from the saved palette.                                    |
| `regenerate({mode=...})`                      | Rebuild from the saved seed or image.                                                |

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

## Palette shape

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

## Templates

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
