---@meta

-- Editor definitions for riced-screenshot. Installed beside the binary and
-- indexed by lua-language-server; never executed at runtime.

---@class riced.ScreenshotLast
---@field path string PNG file path
---@field output string Connector name the frame came from
---@field bytes integer PNG size in bytes
---@field at integer Unix seconds

---@class riced.ScreenshotStore
---@field connected boolean
---@field capturing boolean
---@field last riced.ScreenshotLast?

---@class riced.ScreenshotExtension: riced.Extension, riced.ScreenshotStore
---@field store riced.ScreenshotStore

---Captures one output to a PNG. `output` is an output name or nil for the
---first; `path` may be `~/`-prefixed and is written atomically.
---@param output? string
---@param path string
---@param callback? fun(self: table, result: riced.ScreenshotLast?, error: string?)
function riced.ScreenshotExtension:capture(output, path, callback) end

---@overload fun(name: "screenshot"): riced.ScreenshotExtension
---@param name string
---@return riced.Extension
function riced.plugins(name) end

---@overload fun(name: "screenshot"): riced.ScreenshotExtension
---@param name string
---@return riced.Extension
function riced.plugin(name) end

---@overload fun(name: "screenshot"): riced.ScreenshotExtension
---@param name string
---@return riced.Extension
function iced.plugins(name) end

---@overload fun(name: "screenshot"): riced.ScreenshotExtension
---@param name string
---@return riced.Extension
function iced.plugin(name) end
