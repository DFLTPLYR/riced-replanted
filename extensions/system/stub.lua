---@meta

-- Editor definitions for riced-system. Installed beside the binary and indexed
-- by lua-language-server; never executed at runtime.

---@class riced.SystemHost
---@field name? string
---@field os? string
---@field version? string
---@field kernel? string
---@field uptime? integer Seconds
---@field boot_time? integer Unix seconds

---@class riced.SystemTime
---@field epoch integer Unix seconds
---@field time string Local HH:MM (24-hour)
---@field date string Local YYYY-MM-DD
---@field weekday string Full English weekday name

---@class riced.SystemCpu
---@field usage number Percent
---@field cores integer
---@field physical? integer
---@field brand string
---@field frequency number Highest core MHz
---@field load1 number
---@field load5 number
---@field load15 number

---@class riced.SystemMemory
---@field total integer Bytes
---@field used integer Bytes
---@field available integer Bytes
---@field swap_total integer Bytes
---@field swap_used integer Bytes

---@class riced.SystemDisk
---@field mount string
---@field fs string
---@field total integer Bytes
---@field available integer Bytes
---@field used integer Bytes

---@class riced.SystemGpu
---@field name string
---@field busy? number Percent
---@field vram_used? integer Bytes
---@field vram_total? integer Bytes
---@field temp? number Celsius
---@field clock? number MHz

---@class riced.SystemStore
---@field host? riced.SystemHost
---@field time? riced.SystemTime
---@field cpu? riced.SystemCpu
---@field memory? riced.SystemMemory
---@field disks? riced.SystemDisk[]
---@field gpus? riced.SystemGpu[]
---@field processes? integer

---@class riced.SystemExtension: riced.Extension, riced.SystemStore
---@field store riced.SystemStore

---@overload fun(name: "system"): riced.SystemExtension
---@param name string
---@return riced.Extension
function riced.plugins(name) end

---@overload fun(name: "system"): riced.SystemExtension
---@param name string
---@return riced.Extension
function riced.plugin(name) end

---@overload fun(name: "system"): riced.SystemExtension
---@param name string
---@return riced.Extension
function iced.plugins(name) end

---@overload fun(name: "system"): riced.SystemExtension
---@param name string
---@return riced.Extension
function iced.plugin(name) end
