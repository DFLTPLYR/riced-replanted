---@meta

-- Editor definitions for riced-wayland. Installed beside the binary and indexed
-- by lua-language-server; never executed at runtime.

---@class riced.WaylandCapabilities
---@field toplevels boolean
---@field toplevel_control boolean
---@field workspaces boolean

---@class riced.WaylandOutput
---@field id integer Registry name
---@field name string Connector name or `output-<id>`
---@field description string
---@field make string
---@field model string
---@field x integer
---@field y integer
---@field width integer
---@field height integer
---@field scale number

---@class riced.WaylandToplevel
---@field id string Opaque id for the methods
---@field identifier? string Present only with the read-only backend
---@field title string
---@field app_id string
---@field outputs string[]
---@field activated? boolean Present only with toplevel control
---@field maximized? boolean Present only with toplevel control
---@field minimized? boolean Present only with toplevel control
---@field fullscreen? boolean Present only with toplevel control

---@alias riced.WaylandWorkspaceCapability "activate"|"deactivate"|"remove"|"assign"

---@class riced.WaylandWorkspace
---@field id string Opaque id for the methods
---@field protocol_id? string
---@field name string
---@field coordinates integer[]
---@field active boolean
---@field urgent boolean
---@field hidden boolean
---@field capabilities riced.WaylandWorkspaceCapability[]
---@field group? string
---@field outputs string[]

---@alias riced.WaylandGroupCapability "create_workspace"

---@class riced.WaylandWorkspaceGroup
---@field id string
---@field capabilities riced.WaylandGroupCapability[]
---@field outputs string[]

---@class riced.WaylandFocused
---@field toplevel? string
---@field workspace? string
---@field output? string

---@class riced.WaylandStore
---@field connected? boolean
---@field capabilities? riced.WaylandCapabilities
---@field outputs? riced.WaylandOutput[]
---@field toplevels? riced.WaylandToplevel[]
---@field workspaces? riced.WaylandWorkspace[]
---@field workspace_groups? riced.WaylandWorkspaceGroup[]
---@field focused? riced.WaylandFocused

---@class riced.WaylandExtension: riced.Extension, riced.WaylandStore
---@field store riced.WaylandStore
---@field activate_toplevel fun(id: string, callback?: fun(result: nil, err: string?)): nil
---@field close_toplevel fun(id: string, callback?: fun(result: nil, err: string?)): nil
---@field activate_workspace fun(id: string, callback?: fun(result: nil, err: string?)): nil

---@overload fun(name: "wayland"): riced.WaylandExtension
---@param name string
---@return riced.Extension
function riced.plugins(name) end

---@overload fun(name: "wayland"): riced.WaylandExtension
---@param name string
---@return riced.Extension
function riced.plugin(name) end

---@overload fun(name: "wayland"): riced.WaylandExtension
---@param name string
---@return riced.Extension
function iced.plugins(name) end

---@overload fun(name: "wayland"): riced.WaylandExtension
---@param name string
---@return riced.Extension
function iced.plugin(name) end
