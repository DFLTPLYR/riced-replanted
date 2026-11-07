---@meta

-- Editor definitions for riced-lucide. Installed beside the binary and indexed
-- by lua-language-server; never executed at runtime.

---@class riced.LucideIcon
---@field name string
---@field char string Single glyph char
---@field unicode string For example `U+E038`

---@class riced.LucideStore
---@field count? integer Catalog marker

---@class riced.LucideExtension: riced.Extension, riced.LucideStore
---@field store riced.LucideStore
---@field resolve fun(name: string, callback: fun(icon?: riced.LucideIcon, err?: string)): nil
---@field exists fun(name: string, callback: fun(exists?: boolean, err?: string)): nil
---@field list fun(prefix: string?, callback: fun(names?: string[], err?: string)): nil

---@class iced.LucideProps: iced.Props
---@field name string Kebab-case Lucide icon name

---Requires the Lucide extension's lucide.lua companion. Blank until resolved.
---@param props iced.LucideProps
---@return iced.Element
function iced.lucide(props) end

---@overload fun(name: "lucide"): riced.LucideExtension
---@param name string
---@return riced.Extension
function riced.plugins(name) end

---@overload fun(name: "lucide"): riced.LucideExtension
---@param name string
---@return riced.Extension
function riced.plugin(name) end

---@overload fun(name: "lucide"): riced.LucideExtension
---@param name string
---@return riced.Extension
function iced.plugins(name) end

---@overload fun(name: "lucide"): riced.LucideExtension
---@param name string
---@return riced.Extension
function iced.plugin(name) end
