---@meta

-- Editor definitions for riced-shortcuts. Installed beside the binary and
-- indexed by lua-language-server; never executed at runtime.

---@class riced.ShortcutsBind
---@field keys string Backend-native chord syntax
---@field label string Event label

---@class riced.ShortcutsStore
---@field connected boolean
---@field compositor? 'hyprland' | 'sway'
---@field binds riced.ShortcutsBind[]

---@class riced.ShortcutsExtension: riced.Extension, riced.ShortcutsStore
---@field store riced.ShortcutsStore

---Binds a chord so pressing it sends `{ shortcut = label }` as a plugin event.
---Hyprland keys: "SUPER,Return"; sway keys: "Super+Return". Labels allow
---[A-Za-z0-9_.-] and 1-64 characters.
---@param keys string
---@param label string
---@param callback? fun(self: table, result: any?, error: string?)
function riced.ShortcutsExtension:bind(keys, label, callback) end

---@param keys string
---@param callback? fun(self: table, result: any?, error: string?)
function riced.ShortcutsExtension:unbind(keys, callback) end

---@overload fun(name: "shortcuts"): riced.ShortcutsExtension
---@param name string
---@return riced.Extension
function riced.plugins(name) end

---@overload fun(name: "shortcuts"): riced.ShortcutsExtension
---@param name string
---@return riced.Extension
function riced.plugin(name) end

---@overload fun(name: "shortcuts"): riced.ShortcutsExtension
---@param name string
---@return riced.Extension
function iced.plugins(name) end

---@overload fun(name: "shortcuts"): riced.ShortcutsExtension
---@param name string
---@return riced.Extension
function iced.plugin(name) end
