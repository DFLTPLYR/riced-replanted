# riced-lucide API

`riced-lucide` resolves Lucide icon names to font glyphs. The extension owns the
`lucide-icons` lookup; the host renders the resolved glyph with its bundled
`lucide` font.

- Binary: `riced-lucide`
- Lua proxy: `riced.plugin("lucide")` (also `iced.plugins("lucide")`)
- Store fields: `count`
- Methods: `resolve`, `exists`, `list`
- Events: none
- Companion: `lucide.lua` provides `iced.lucide(props)` and Lua helpers

## Install

```sh
just extension-lucide
```

Or build directly into a profile:

```sh
riced extension build extensions/lucide -o <profile>/extensions/lucide
```

The companion must be installed as a non-executable Lua file
(`<profile>/extensions/lucide/lucide.lua`, mode `0644`). `riced extension build`
copies it automatically. Do not also keep a profile-root `lucide.lua`; duplicate
widget names are rejected.

## Common plugin behavior

The proxy is live: `alive` is false and the store is empty until the process
connects. Reads are synchronous; each publish replaces the snapshot while the
proxy table identity stays the same. Methods are asynchronous and take a
callback as the last argument. Replies trigger a redraw automatically.

See the [riced API](../../API.md#plugins-and-extensions) for shared plugin and
extension semantics.

## Store

| Field   | Meaning                                                                 |
| ------- | ----------------------------------------------------------------------- |
| `count` | Constant catalog marker (`1`) published to keep the process alive. It is not a total icon count. |

## Plugin methods

| Method                 | Arguments | Result                                                                                                  |
| ---------------------- | --------- | ------------------------------------------------------------------------------------------------------- |
| `resolve(name, cb)`    | string    | `{ name, char, unicode }`, where `char` is the glyph and `unicode` is a string such as `U+E038`          |
| `exists(name, cb)`     | string    | boolean; unknown names and the empty string are false                                                   |
| `list(prefix, cb)`     | string?   | array of names matching `prefix`, sorted, capped at 512; the full catalog is 2310 icons                  |

`resolve` fails for empty, non-string, or unknown names. Raw methods are
available only while the extension is connected; guard calls with
`plugin.alive` or use the Lua helpers, which return fallbacks while a cold start
is in progress.

## Lua module

```lua
local lucide = require("extensions.lucide.lucide")
```

### `lucide.icon(name, props, fallback)`

Returns `iced.icon(glyph, props)` once the name is resolved, `fallback` while a
lookup is pending, and nil if the extension is unavailable.

### `lucide.exists(name, callback)`

Asynchronous check. Calls `callback(boolean, error?)`.

### `lucide.list(prefix, callback)`

Asynchronous name lookup. Calls `callback(names, error?)` with up to 512 names
matching the prefix, from the crate's full catalog.

### `lucide.warm(names)`

Starts lookups for an array of icon names. Results are cached for later renders.

### `lucide.<name>`

Lazy glyph field, for example `lucide.activity`. It is nil until resolved and
stays cached across reconnects.

Lookups are cached. Failed lookups stay blank and are retried after reconnection
or a module reload, avoiding a request/redraw loop.

## `iced.lucide(props)`

`lucide.lua` registers the `lucide` widget on load:

```lua
iced.lucide({ name = "activity", size = 20, color = theme.text })
```

`name` is a required, nonempty kebab-case Lucide icon name. Other props are the
same as `iced.icon`: `size` (positive, default 16), `color`, `center`,
`align_x`, `align_y`, and the common layout props. Repeated instances need
distinct `id` values, like other custom widgets.

The widget renders blank text while the extension is unavailable or a lookup is
pending. Cached glyphs remain usable while disconnected.

For raw glyphs, use `iced.icon(glyph, props)`. This replaces the old
`iced.lucide(glyph, props)` alias.

## Migration

- Replace old `iced.lucide(glyph, props)` calls with `iced.icon(glyph, props)`.
- Update `require("lucide")` to `require("extensions.lucide.lucide")`.
- If an old installer placed `lucide.lua` at the profile root, remove it after
  moving any customizations into the bundled module.
