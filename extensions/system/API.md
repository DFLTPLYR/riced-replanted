# riced-system API

`riced-system` publishes host, clock, CPU, memory, disk, GPU, and process data
through the `system` plugin store.

- Binary: `riced-system`
- Lua proxy: `riced.plugin("system")` (also `iced.plugins("system")`)
- Store fields: `host`, `time`, `cpu`, `memory`, `disks`, `gpus`, `processes`
- Methods: none
- Events: none

## Install

```sh
just extension-system
```

Or build directly into a profile:

```sh
riced extension build extensions/system -o <profile>/extensions/system
```

## Common plugin behavior

The proxy is live: `alive` is false and the store is empty until the process
connects. Reads are synchronous and lazy — reading a field for the first time
subscribes to that field. Each publish replaces the snapshot while the proxy
table identity stays the same. On disconnect, `alive` becomes false and the last
snapshot is kept.

See the [riced API](../../API.md#plugins-and-extensions) for shared plugin and
extension semantics.

## Store

| Field       | Shape                                                                                                                                                        |
| ----------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `host`      | `{ name?, os?, version?, kernel?, uptime?, boot_time? }`; `uptime` and `boot_time` are seconds, `boot_time` is a Unix timestamp                              |
| `time`      | `{ epoch, time, date, weekday }` or nil; `epoch` is seconds, `time` is local `HH:MM` (24-hour), `date` is local `YYYY-MM-DD`, `weekday` is the full English name |
| `cpu`       | `{ usage, cores, physical?, brand, frequency, load1, load5, load15 }`; `usage` is percent, `frequency` is the highest core MHz, `physical` may be nil          |
| `memory`    | `{ total, used, available, swap_total, swap_used }`; bytes                                                                                                   |
| `disks`     | array of `{ mount, fs, total, available, used }`; bytes; entries whose total is zero are omitted                                                              |
| `gpus`      | array of `{ name, busy?, vram_used?, vram_total?, temp?, clock? }`; `busy` is percent, memory fields are bytes, `temp` is °C, `clock` is MHz                   |
| `processes` | integer process count                                                                                                                                        |

## Behavior

`host`, `memory`, and `disks` are snapshots; read them again when the plugin
store changes. `time` is nil if the system clock cannot be read.

Only requested groups are refreshed, once per second. Fields nothing reads are
not computed. Optional values may be nil when the OS or hardware does not expose
them.

`disks` comes from `sysinfo`. `gpus` reads DRM cards under `/sys/class/drm`;
optional GPU values are nil when the driver does not expose them.

## Example

```lua
local system = riced.plugin("system")
local cpu = system.cpu or {}
local memory = system.memory or {}

iced.text(string.format(
    "%.0f%%  %.1f / %.1f GiB",
    cpu.usage or 0,
    (memory.used or 0) / 2 ^ 30,
    (memory.total or 0) / 2 ^ 30
))
```
