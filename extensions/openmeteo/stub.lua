---@meta

-- Editor definitions for riced-openmeteo. Installed beside the binary and
-- indexed by lua-language-server; never executed at runtime.

---@class riced.OpenmeteoCurrent
---@field time string
---@field interval integer
---@field temperature_2m? number
---@field relative_humidity_2m? number
---@field apparent_temperature? number
---@field is_day? integer
---@field precipitation? number
---@field weather_code? integer
---@field wind_speed_10m? number
---@field wind_direction_10m? number

---@class riced.OpenmeteoStore
---@field ready? boolean
---@field latitude? number
---@field longitude? number
---@field current? riced.OpenmeteoCurrent
---@field current_units? table<string, string>
---@field timezone? string
---@field updated_at? integer Unix seconds
---@field error? string

---@class riced.OpenmeteoExtension: riced.Extension, riced.OpenmeteoStore
---@field store riced.OpenmeteoStore
---@field configure fun(latitude: number, longitude: number, callback?: fun(result: nil, err: string?)): nil

---@overload fun(name: "openmeteo"): riced.OpenmeteoExtension
---@param name string
---@return riced.Extension
function riced.plugins(name) end

---@overload fun(name: "openmeteo"): riced.OpenmeteoExtension
---@param name string
---@return riced.Extension
function riced.plugin(name) end

---@overload fun(name: "openmeteo"): riced.OpenmeteoExtension
---@param name string
---@return riced.Extension
function iced.plugins(name) end

---@overload fun(name: "openmeteo"): riced.OpenmeteoExtension
---@param name string
---@return riced.Extension
function iced.plugin(name) end

---Requires the Open-Meteo companion; configure its location through the returned Lua module.
---Displays current temperature in Celsius, marking retained readings as stale on errors.
---@param props? iced.Props
---@return iced.Element
function iced.openmeteo(props) end
