# riced-screenshot API

`riced-screenshot` captures Wayland outputs to PNG files through the
`screenshot` plugin.

- Binary: `riced-screenshot`
- Lua proxy: `riced.plugin("screenshot")` (also `iced.plugins("screenshot")`)
- Store fields: `connected`, `capturing`, `last`
- Methods: `capture`, `capture_region`
- Events: none

## Install

```sh
just extension-screenshot
```

Or build directly into a profile:

```sh
riced extension build extensions/screenshot -o <profile>/extensions/screenshot
```

## Common plugin behavior

The proxy is live: `alive` is false and the store is empty until the process
connects. Reads are synchronous; each publish replaces the snapshot while the
proxy table identity stays the same. Methods are asynchronous and take an
optional callback as the last argument. See the
[riced API](../../API.md#plugins-and-extensions) for shared semantics.

## Store

| Field       | Type / shape                                              |
| ----------- | --------------------------------------------------------- |
| `connected` | boolean; true when `zwlr-screencopy-manager-v1` is bound  |
| `capturing` | boolean; true while a capture is in progress              |
| `last`      | `{ path, output, bytes, at }` or nil before any capture   |

`at` is unix seconds. `output` is the connector name the frame came from.

## Methods

`capture(output, path)` captures one output and writes a PNG. `output` is an
output name from `riced.plugin("wayland").outputs`, or nil for the first
output. `path` may be `~/`-prefixed or absolute; the file is written atomically
(0600). On success the callback receives `(last, nil)`; on failure
`(nil, error)`. A second capture while one runs fails immediately.

`capture_region(x, y, width, height, path)` captures a rectangle in global
logical desktop coordinates, the same space `slurp` prints. Coordinates are
mapped to the containing output through `xdg-output`, so any monitor works and
HiDPI scale is handled by the compositor. The rectangle must fit inside one
output; a selection spanning two monitors fails with an error listing the
output rectangles. `path` follows the same rules as `capture`.

```lua
local screenshot = riced.plugin("screenshot")
-- 800x600 at (100, 100) on the left monitor
screenshot.capture_region(100, 100, 800, 600, "~/shot.png", function(last, err)
    if err then
        log(err)
    end
end)
```

## Limits

Requires a wlroots-family compositor (`zwlr-screencopy-v1`: sway, Hyprland,
niri, river, wlroots-based). GNOME and KDE are not supported. Region capture
also needs screencopy manager v2 and `xdg-output` for output positions. Frames
are converted from ARGB/XRGB8888 to RGBA8 with `y_invert` handled. A capture
that hangs past the 30-second callback timeout keeps running in the background
and still updates the store when it finishes.
