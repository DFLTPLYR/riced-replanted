# riced-openmeteo API

`riced-openmeteo` retrieves current weather for up to 8 configured locations per
process and publishes it through the `openmeteo` plugin store. The companion
`openmeteo.lua` provides the `iced.openmeteo(props)` widget.

- Binary: `riced-openmeteo`
- Lua proxy: `riced.plugin("openmeteo")` (also `iced.plugins("openmeteo")`)
- Store fields: `ready`, `latitude`, `longitude`, `current`, `current_units`,
  `timezone`, `updated_at`, `error`, `readings`
- Methods: `configure`
- Events: none
- Companion: `openmeteo.lua` provides `iced.openmeteo(props)` and a
  `configure(latitude, longitude)` helper

## Install

```sh
just extension-openmeteo
```

Or build directly into a profile:

```sh
riced extension build extensions/openmeteo -o <profile>/extensions/openmeteo
```

The companion must be installed as a non-executable Lua file
(`<profile>/extensions/openmeteo/openmeteo.lua`, mode `0644`). `riced extension
build` copies it automatically.

## Widget

```lua
local weather = require("extensions.openmeteo.openmeteo")
weather.configure(52.52, 13.41)

function app:view()
    return iced.openmeteo({ size = 18, color = theme.text })
end
```

`weather.configure(latitude, longitude)` validates finite WGS84 coordinates
(`-90..90`, `-180..180`) and remembers the desired location. The module sends it
when the widget first renders after connection or reconnection, so it is safe to
call before the executable is listening.

All widget instances share that location. Repeated instances need distinct `id`
props, like other custom widgets. The widget accepts the normal text props,
displays `12.5°C`, and shows loading/unavailable text before a reading. A
retained reading is marked `(stale)` after a failed refresh or disconnect.

## Plugin method

`configure(latitude, longitude, callback?)` is asynchronous and accepts finite
numeric WGS84 coordinates (`-90..90`, `-180..180`). Success acknowledges
configuration with `(nil, nil)`; weather arrives separately in the store. Call it
only once the extension is alive. Configuration is in memory; resend it after
reconnection.

Each call adds a tracked location, deduplicated by coordinates, most recently
configured first, bounded at 8 (the oldest is dropped). Every tracked location
refreshes independently: a failing city keeps its last reading and does not
affect the others.

The Lua `weather.configure` helper does this lifecycle handling for you.

## Store

| Field           | Meaning                                                                                                                                                                                                 |
| --------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `ready`         | A successful reading exists for the most recently configured location                                                                                                                                    |
| `latitude`, `longitude` | Coordinates of the most recently configured location, absent until configured                                                                                                                    |
| `current`       | Open-Meteo values: `time`, `interval`, `temperature_2m`, `relative_humidity_2m`, `apparent_temperature`, `is_day`, `precipitation`, `weather_code`, `wind_speed_10m`, `wind_direction_10m`             |
| `current_units` | Units returned by Open-Meteo: Celsius, km/h, mm, percent, etc.                                                                                                                                           |
| `timezone`      | Resolved IANA timezone; `current.time` is local to it                                                                                                                                                    |
| `updated_at`    | Unix seconds of the last successful fetch                                                                                                                                                               |
| `error`         | Last fetch error of the most recently configured location, absent after a successful refresh                                                                                                             |
| `readings`      | Object keyed by `"<lat>,<lon>"` in Rust float formatting; match entries by `latitude`/`longitude` numerically instead of building keys; each entry repeats the fields above |

## Behavior

Fetching starts after configuration and repeats every 15 minutes per location
without a Lua timer. Failures retry after 60 seconds and keep the last reading.
Configuring a new location publishes a placeholder for it before the first fetch
completes, so its city never shows another city's weather.

Requests time out after 15 seconds, reject HTTP errors and redirects, and cap
response bodies at 64 KiB. Other current values may be null when unavailable;
handle missing values in custom views. `weather_code` follows Open-Meteo's WMO
interpretation codes.

## Custom UI

```lua
local weather = riced.plugin("openmeteo")

if weather.ready then
    local current = weather.current or {}
    iced.text(string.format("%.1f°C", current.temperature_2m or 0))
elseif weather.error then
    iced.text("Weather unavailable")
else
    iced.text("Loading weather…")
end
```

For several cities, read `readings` and match entries by coordinates — keys are
formatted by Rust, so `tostring(52.0)` does not rebuild them:

```lua
for _, entry in pairs(weather.readings or {}) do
    if entry.ready and entry.current then
        iced.text(string.format("%.1f°C", entry.current.temperature_2m or 0))
    end
end
```

Weather data: [Open-Meteo](https://open-meteo.com/),
[API documentation](https://open-meteo.com/en/docs). This version uses its public
non-commercial endpoint without an API key; see the provider's usage terms.
