-- The extension tracks up to 8 configured locations per process; its legacy
-- top-level fields mirror the most recently configured one.
local ext = riced.plugins("openmeteo")
local weather = { plugin = ext }
local desired, configured

function weather.configure(latitude, longitude)
    assert(type(latitude) == "number" and latitude >= -90 and latitude <= 90, "latitude must be -90..90")
    assert(type(longitude) == "number" and longitude >= -180 and longitude <= 180, "longitude must be -180..180")
    desired = { latitude, longitude }
    configured = nil
    if ext.alive and type(ext.configure) == "function" then
        ext.configure(latitude, longitude)
        configured = ext.configure
    end
end

-- Find the reading for the desired location. The extension keys readings by
-- "<lat>,<lon>" with Rust float formatting, which differs from Lua tostring,
-- so match by coordinates instead of building keys. Falls back to the legacy
-- single-location fields when the extension predates `readings`.
local function reading()
    if not desired then
        return nil
    end
    if type(ext.readings) == "table" then
        for _, entry in pairs(ext.readings) do
            if type(entry) == "table"
                and entry.latitude == desired[1]
                and entry.longitude == desired[2] then
                return entry
            end
        end
        return nil
    end
    if ext.latitude == desired[1] and ext.longitude == desired[2] then
        return ext
    end
    return nil
end

iced.define("openmeteo", function()
    local widget = {}
    function widget:view(props)
        if desired and configured ~= ext.configure then
            weather.configure(desired[1], desired[2])
        end
        local entry = reading()
        local text = "Weather unavailable"
        local current = entry and entry.current
        if entry and entry.ready and type(current) == "table" and type(current.temperature_2m) == "number" then
            text = string.format("%.1f°C", current.temperature_2m)
            if not ext.alive or type(entry.error) == "string" then text = text .. " (stale)" end
        elseif ext.alive and (entry == nil or type(entry.error) ~= "string") then
            text = desired and "Loading weather…" or "Weather not configured"
        end
        return iced.text(text, props)
    end
    return widget
end)

return weather
