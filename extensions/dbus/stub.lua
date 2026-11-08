---@meta

-- Editor definitions for riced-dbus. Installed beside the binary and indexed by
-- lua-language-server; never executed at runtime.

---@class riced.DbusCapabilities
---@field tray boolean
---@field tray_host boolean
---@field notifications boolean

---@class riced.DbusTrayItem
---@field id string Opaque id for the methods
---@field title string
---@field icon string Resolve with iced.icon_path
---@field icon_theme_path? string
---@field status string StatusNotifier status
---@field menu? string D-Bus object path
---@field item_is_menu boolean

---@class riced.DbusMenuEntry
---@field id integer
---@field label string
---@field enabled boolean
---@field type "standard"|"separator"
---@field icon? string
---@field toggle? "checkmark"|"radio"
---@field checked? boolean
---@field submenu boolean
---@field children? riced.DbusMenuEntry[]

---@class riced.DbusTrayMenu
---@field id string Tray item id
---@field revision integer
---@field items riced.DbusMenuEntry[]
---@field error? string

---@class riced.DbusNotificationAction
---@field key string
---@field label string

---@class riced.DbusNotification
---@field id integer
---@field app_name string
---@field app_icon string
---@field summary string
---@field body string
---@field actions riced.DbusNotificationAction[]
---@field timeout integer Milliseconds; 0 never expires
---@field timestamp integer Unix milliseconds

---@class riced.DbusPickFileOptions
---@field title? string Dialog title
---@field multiple? boolean Select multiple files
---@field directory? boolean Select a directory instead of a file
---@field path? string Absolute starting directory; a leading ~/ is expanded

---@class riced.DbusStore
---@field connected? boolean
---@field capabilities? riced.DbusCapabilities
---@field tray? riced.DbusTrayItem[]
---@field tray_menu? riced.DbusTrayMenu
---@field notifications? riced.DbusNotification[]

---@class riced.DbusExtension: riced.Extension, riced.DbusStore
---@field store riced.DbusStore
---@field activate_tray fun(id: string, callback?: fun(result: nil, err: string?)): nil
---@field secondary_activate_tray fun(id: string, callback?: fun(result: nil, err: string?)): nil
---@field context_menu_tray fun(id: string, callback?: fun(result: nil, err: string?)): nil
---@field open_tray_menu fun(id: string, callback?: fun(result: nil, err: string?)): nil
---@field close_tray_menu fun(callback?: fun(result: nil, err: string?)): nil
---@field tray_menu_event fun(id: string, menu_id: integer, event: "clicked"|"opened"|"closed"|"hovered", callback?: fun(result: nil, err: string?)): nil
---@field close_notification fun(id: integer, callback?: fun(result: nil, err: string?)): nil
---@field invoke_notification_action fun(id: integer, key: string, callback?: fun(result: nil, err: string?)): nil
---@field pick_file fun(options?: riced.DbusPickFileOptions, callback?: fun(paths: string[]?, err: string?)): nil

---@overload fun(name: "dbus"): riced.DbusExtension
---@param name string
---@return riced.Extension
function riced.plugins(name) end

---@overload fun(name: "dbus"): riced.DbusExtension
---@param name string
---@return riced.Extension
function riced.plugin(name) end

---@overload fun(name: "dbus"): riced.DbusExtension
---@param name string
---@return riced.Extension
function iced.plugins(name) end

---@overload fun(name: "dbus"): riced.DbusExtension
---@param name string
---@return riced.Extension
function iced.plugin(name) end
