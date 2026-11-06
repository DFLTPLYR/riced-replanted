-- shortcut: one location per process; add keyed state when multiple cities are needed.
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

iced.define("openmeteo", function()
    local widget = {}
    function widget:view(props)
        if desired and configured ~= ext.configure then
            weather.configure(desired[1], desired[2])
        end
        local text = "Weather unavailable"
        local matches = not desired or (ext.latitude == desired[1] and ext.longitude == desired[2])
        local current = ext.current
        if matches and ext.ready and type(current) == "table" and type(current.temperature_2m) == "number" then
            text = string.format("%.1f°C", current.temperature_2m)
            if not ext.alive or type(ext.error) == "string" then text = text .. " (stale)" end
        elseif ext.alive and type(ext.error) ~= "string" then
            text = desired and "Loading weather…" or "Weather not configured"
        end
        return iced.text(text, props)
    end
    return widget
end)

return weather
