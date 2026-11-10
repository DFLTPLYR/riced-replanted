---@meta

-- Editor definitions for riced-themeagen. Installed beside the binary and
-- indexed by lua-language-server; never executed at runtime.

---@alias riced.ThemeagenVariant "content"|"tonalspot"|"monochrome"|"neutral"|"vibrant"|"expressive"|"fidelity"|"rainbow"|"fruit_salad"

---@class riced.ThemeagenSource
---@field kind "seed"|"image"|"manual"
---@field seed? string
---@field image? string
---@field variant? riced.ThemeagenVariant

---@class riced.ThemeagenAnsi
---@field black string
---@field red string
---@field green string
---@field yellow string
---@field blue string
---@field magenta string
---@field cyan string
---@field white string

---@class riced.ThemeagenTerminal
---@field normal riced.ThemeagenAnsi
---@field bright riced.ThemeagenAnsi
---@field foreground string
---@field background string
---@field selectionfg string
---@field selectionbg string
---@field cursortext string
---@field cursor string

---@class riced.ThemeagenPalette
---@field primary string
---@field onprimary string
---@field secondary string
---@field onsecondary string
---@field tertiary string
---@field ontertiary string
---@field error string
---@field onerror string
---@field surface string
---@field onsurface string
---@field surfacevariant string
---@field onsurfacevariant string
---@field outline string
---@field shadow string
---@field hover string
---@field onhover string
---@field terminal riced.ThemeagenTerminal
---@field [string] any Extended Material roles and manual palette values

---@class riced.ThemeagenColors
---@field dark riced.ThemeagenPalette
---@field light riced.ThemeagenPalette

---@class riced.ThemeagenResult
---@field mode "dark"|"light"
---@field source riced.ThemeagenSource
---@field colors riced.ThemeagenColors
---@field applied integer Number of template files written
---@field errors string[]

---@class riced.ThemeagenGenerateProps
---@field seed? string Exactly one of seed or image
---@field image? string Exactly one of seed or image
---@field mode? "dark"|"light"
---@field variant? riced.ThemeagenVariant

---@class riced.ThemeagenPopulateProps
---@field colors? riced.ThemeagenColors
---@field mode? "dark"|"light"

---@class riced.ThemeagenRegenerateProps
---@field mode? "dark"|"light"

---@class riced.ThemeagenStore
---@field ready? boolean
---@field mode? "dark"|"light"
---@field source? riced.ThemeagenSource
---@field colors? riced.ThemeagenColors

---@class riced.ThemeagenExtension: riced.Extension, riced.ThemeagenStore
---@field store riced.ThemeagenStore
---@field generate fun(props: riced.ThemeagenGenerateProps, callback?: fun(result?: riced.ThemeagenResult, err?: string)): nil
---@field populate fun(props?: riced.ThemeagenPopulateProps, callback?: fun(result?: riced.ThemeagenResult, err?: string)): nil
---@field regenerate fun(props?: riced.ThemeagenRegenerateProps, callback?: fun(result?: riced.ThemeagenResult, err?: string)): nil

---@overload fun(name: "themeagen"): riced.ThemeagenExtension
---@param name string
---@return riced.Extension
function riced.plugins(name) end

---@overload fun(name: "themeagen"): riced.ThemeagenExtension
---@param name string
---@return riced.Extension
function riced.plugin(name) end

---@overload fun(name: "themeagen"): riced.ThemeagenExtension
---@param name string
---@return riced.Extension
function iced.plugins(name) end

---@overload fun(name: "themeagen"): riced.ThemeagenExtension
---@param name string
---@return riced.Extension
function iced.plugin(name) end
