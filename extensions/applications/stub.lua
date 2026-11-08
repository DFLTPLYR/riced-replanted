---@meta

-- Editor definitions for riced-applications. Installed beside the binary and
-- indexed by lua-language-server; never executed at runtime.

---@class riced.Application
---@field id string Desktop file id, for example "firefox"
---@field name string
---@field icon? string Icon name; resolve with iced.icon_path
---@field comment? string
---@field terminal boolean Launched inside $TERMINAL

---@class riced.ApplicationsStore
---@field applications? riced.Application[]

---@class riced.ApplicationsExtension: riced.Extension, riced.ApplicationsStore
---@field store riced.ApplicationsStore
---@field launch fun(id: string, callback?: fun(result: nil, err: string?)): nil

---@overload fun(name: "applications"): riced.ApplicationsExtension
---@param name string
---@return riced.Extension
function riced.plugins(name) end

---@overload fun(name: "applications"): riced.ApplicationsExtension
---@param name string
---@return riced.Extension
function riced.plugin(name) end

---@overload fun(name: "applications"): riced.ApplicationsExtension
---@param name string
---@return riced.Extension
function iced.plugins(name) end

---@overload fun(name: "applications"): riced.ApplicationsExtension
---@param name string
---@return riced.Extension
function iced.plugin(name) end
