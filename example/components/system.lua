local system = {}

local function gibibytes(bytes)
    return string.format("%.1f GiB", bytes / 1024 / 1024 / 1024)
end

local function percent(used, total)
    if not total or total == 0 then return 0 end
    return used / total * 100
end

function system:view()
    local extension = riced.plugins("system")
    if not extension or not extension.alive then
        return iced.text("system extension offline", { color = theme.secondary })
    end
    local store = extension.store
    local host = store.host or {}
    local clock = store.time or {}
    local cpu = store.cpu or {}
    local memory = store.memory or {}
    local gpu = (store.gpus or {})[1] or {}
    local disk = (store.disks or {})[1] or {}
    local gpu_text = gpu.name or "no gpu"
    if gpu.busy then gpu_text = gpu_text .. string.format(" %.0f%%", gpu.busy) end
    if gpu.temp then gpu_text = gpu_text .. string.format(" %d°C", gpu.temp) end
    return iced.column({
        iced.text(string.format("%s  %s  %s", clock.weekday or "", clock.date or "", clock.time or ""),
            { color = theme.secondary }),
        iced.text(string.format("CPU %.0f%%  %s", cpu.usage or 0, cpu.brand or ""),
            { color = theme.primary }),
        iced.text(string.format("RAM %.0f%%  %s / %s", percent(memory.used, memory.total),
            gibibytes(memory.used or 0), gibibytes(memory.total or 0)), { color = theme.text }),
        iced.text(gpu_text, { color = theme.text }),
        iced.text(string.format("%s %.0f%%  %s", disk.mount or "", percent(disk.used, disk.total),
            gibibytes(disk.used or 0)), { color = theme.secondary }),
        iced.text(string.format("%s · up %dh · %d procs", host.name or "?",
            (host.uptime or 0) // 3600, store.processes or 0), { color = theme.secondary }),
    }, { spacing = 4 })
end

return system
