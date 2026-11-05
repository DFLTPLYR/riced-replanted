-- Lucide icon helper for riced profiles.
--
-- Copy this file into your profile as `lucide.lua` (next to `shell.lua`,
-- not under data/ or components/) and copy the extension binary:
--
--   ln -s "$PWD/extensions/lucide/example/lucide.lua" ~/.config/riced/<profile>/lucide.lua
--   # or:
--   cp extensions/lucide/example/lucide.lua ~/.config/riced/<profile>/lucide.lua
--
-- Import once from any profile module:
--   local lucide = require("lucide")
--
-- Then use it two ways:
--
--   -- 1. Magic table: first read resolves via the extension and caches.
--   local activity_glyph = lucide.activity -- "\u{e038}" or nil if unresolved
--
--   -- 2. Widget helper: builds an iced element from a name.
--   lucide.icon("activity", { size = 16, color = theme.text })
--   lucide.icon("arrow-right", { size = 20 })
--
-- Names are kebab-case lucide names ("activity", "arrow-right", "settings").
-- The glyph is cached in the module table so repeated reads are free.
-- The extension is asynchronous; the first view after a fresh require may
-- render a placeholder (or nil) until the callback fires. Re-render by
-- returning any task / event that triggers a view.
--
-- Copy this file into your profile (e.g. `<profile>/lucide.lua`) and the
-- `riced-lucide` extension binary into `<profile>/extensions/lucide/`.

local ext = riced.plugins("lucide")

local lucide = {}
local glyphs = {}
local pending = {}

--- Extension proxy. Methods appear on the proxy only after `riced-lucide`
--- registers. Guard every call so a cold start (first view before the child
--- process is listening) returns the fallback / nil instead of erroring.
--- The next view retries automatically.
local function call_method(name, ...)
    local method = ext[name]
    if type(method) ~= "function" then
        return false
    end
    method(...)
    return true
end

local function fetch(name)
    if glyphs[name] ~= nil then
        return glyphs[name]
    end
    if pending[name] then
        return nil
    end
    if type(ext.resolve) ~= "function" then
        -- Extension not registered yet; retry on next view.
        return nil
    end
    pending[name] = true
    ext.resolve(name, function(result, err)
        pending[name] = nil
        if err then
            -- Keep name unresolved so the next view retries.
            return
        end
        if result and result.char then
            glyphs[name] = result.char
        end
    end)
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

return lucide
