-- Install alongside riced-lucide as <profile>/extensions/lucide/lucide.lua (mode 0644).
-- iced.lucide({ name = "activity", size = 16 }) loads this module on demand.
-- Helpers: local lucide = require("extensions.lucide.lucide")
-- Lookups are asynchronous; plugin replies trigger a redraw automatically.

local ext = riced.plugins("lucide")

local lucide = {}
local glyphs = {}
local pending = {}
local failed = {}

--- Extension proxy. Methods appear on the proxy only after `riced-lucide`
--- registers. Guard every call so a cold start (first view before the child
--- process is listening) returns the fallback / nil instead of erroring.
--- The next view retries automatically.
local function call_method(name, ...)
    local method = ext[name]
    if not ext.alive or type(method) ~= "function" then
        return false
    end
    method(...)
    return true
end

local function fetch(name)
    assert(type(name) == "string" and name ~= "", "icon name must be a nonempty string")
    if glyphs[name] ~= nil then
        return glyphs[name]
    end
    if pending[name] then
        return nil
    end
    local resolve = ext.resolve
    if not ext.alive or type(resolve) ~= "function" or failed[name] == resolve then
        return nil
    end
    pending[name] = true
    local ok, error_message = pcall(resolve, name, function(result, err)
        pending[name] = nil
        if not err and result and result.char then
            glyphs[name] = result.char
        else
            -- Avoid a request/redraw loop; a new connection or module reload retries.
            failed[name] = resolve
        end
    end)
    if not ok then
        pending[name] = nil
        error(error_message, 0)
    end
    return nil
end

setmetatable(lucide, {
    __index = function(_, name)
        if type(name) ~= "string" or name == "" then
            return nil
        end
        return fetch(name)
    end,
})

-- Build an iced.icon element from an icon name.
-- Returns nil if the glyph isn't cached yet (first render after require).
-- Pass `fallback` to render something while pending.
function lucide.icon(name, props, fallback)
    local g = fetch(name)
    if g == nil then
        return fallback
    end
    return iced.icon(g, props)
end

-- Check whether a name is a valid lucide icon (async).
function lucide.exists(name, cb)
    assert(type(name) == "string" and name ~= "", "icon name must be a nonempty string")
    assert(type(cb) == "function", "callback must be a function")
    if not call_method("exists", name, cb) then
        cb(false, "lucide extension not ready")
    end
end

-- List candidate icon names by prefix (extension returns a small sample).
function lucide.list(prefix, cb)
    if not call_method("list", prefix or "", cb) then
        cb({}, "lucide extension not ready")
    end
end

-- Pre-resolve a list of names eagerly.
function lucide.warm(names)
    assert(type(names) == "table", "names must be a table")
    for _, name in ipairs(names) do
        if type(name) == "string" then fetch(name) end
    end
end

iced.define("lucide", function()
    local widget = {}
    function widget:view(props)
        local node = lucide.icon(props.name, props) or iced.text("", props)
        node.name = nil
        return node
    end
    return widget
end)

return lucide
