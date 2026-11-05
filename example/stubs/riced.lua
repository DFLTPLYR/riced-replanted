---@meta

-- Editor definitions for the globals riced injects into a profile. Loaded by
-- lua-language-server through .luarc.json; never executed at runtime.
-- Keep in sync with src/lua/, src/file.rs, and API.md when the Lua API changes.

---Logs values at info level as `[lua]: information` on stderr.
---Uses Lua tostring and separates multiple values with tabs, like print.
---@class riced.Log
---@overload fun(...: any)
log = {}

---Logs values like `log`, but expands table arguments recursively.
---@param ... any
function log.table(...) end

---@class riced.Theme
---@field background string
---@field text string
---@field primary string
---@field secondary string
---@field success string
---@field warning string
---@field danger string

---@type riced.Theme
theme = {
	background = "",
	text = "",
	primary = "",
	secondary = "",
	success = "",
	warning = "",
	danger = "",
}

---@type { theme: riced.Theme }
config = {}

---@class iced.Element
---@field kind string

---@class iced.Event
---@field event string
---@field args any

---@class iced.ChangeTransition
---@field props? ("width"|"height"|"padding"|"size"|"spacing"|"color"|"value")[]
---@field duration? integer
---@field easing? "linear"|"ease"|"in"|"out"|"in_out"|"ease_in"|"ease_out"|"ease_in_out"
---@field spring? "smooth"|"snappy"|"bouncy"|"instant"
---@field callback? fun(self: iced.Instance?): any

---@class iced.EventTransition
---@field duration? integer
---@field easing? "linear"|"ease"|"in"|"out"|"in_out"|"ease_in"|"ease_out"|"ease_in_out"
---@field spring? "smooth"|"snappy"|"bouncy"|"instant"
---@field from? {x?: number, y?: number, scale?: number}
---@field to? {x?: number, y?: number, scale?: number}
---@field callback? fun(self: iced.Instance?): any

---@class iced.Transition
---@field change? iced.ChangeTransition
---@field enter? iced.EventTransition
---@field exit? iced.EventTransition
---@field displaced? iced.EventTransition
---@field pointer_enter? iced.EventTransition
---@field pointer_exit? iced.EventTransition
---@field press? iced.EventTransition
---@field release? iced.EventTransition
---@field [string] iced.EventTransition

---@class iced.Instance
---@field props table
---@field animate table<string, fun()>

---@class iced.Font
---@field family? string
---@field weight? "thin"|"extra_light"|"light"|"normal"|"medium"|"semibold"|"bold"|"extra_bold"|"black"
---@field style? "normal"|"italic"|"oblique"
---@field stretch? string

---@class iced.Padding
---@field top? number
---@field right? number
---@field bottom? number
---@field left? number

---@class iced.Radius
---@field top_left? number
---@field top_right? number
---@field bottom_right? number
---@field bottom_left? number

---@class iced.Border
---@field color? string
---@field width? number
---@field radius? number|iced.Radius

---@class iced.Shadow
---@field color? string
---@field offset? { x: number, y: number }
---@field blur? number

---@class iced.Style
---@field background? string
---@field text_color? string
---@field border? iced.Border
---@field shadow? iced.Shadow
---@field icon_color? string
---@field dot_color? string
---@field placeholder? string
---@field selection? string
---@field handle? string
---@field foreground? string
---@field rail? string
---@field filled? string
---@field scroller? string
---@field handle_border? iced.Border
---@field active? iced.Style
---@field hovered? iced.Style
---@field pressed? iced.Style
---@field disabled? iced.Style
---@field focused? iced.Style
---@field dragged? iced.Style
---@field opened? iced.Style

---@class iced.RuleStyle
---@field color? string
---@field radius? number|iced.Radius
---@field snap? boolean

---@class iced.ProgressStyle
---@field background? string
---@field bar? string
---@field border? iced.Border

---@class iced.QrStyle
---@field cell? string
---@field background? string

---@class iced.MarkdownStyle
---@field link_color? string
---@field inline_code_color? string

---@class iced.PaneGridStyle
---@field hovered_region? { background?: string, border?: iced.Border }
---@field picked_split? { color?: string, width?: number }
---@field hovered_split? { color?: string, width?: number }

---@class iced.TableColumn
---@field header string|number|iced.Element
---@field width? string|number|{ fill_portion: integer }
---@field align_x? "left"|"center"|"right"
---@field align_y? "top"|"center"|"bottom"

---@class iced.CanvasRect
---@field type "rect"
---@field x? number
---@field y? number
---@field width number
---@field height number
---@field color string
---@field border? iced.Border

---@class iced.CanvasCircle
---@field type "circle"
---@field x? number
---@field y? number
---@field radius number
---@field color string
---@field border? iced.Border

---@class iced.CanvasLine
---@field type "line"
---@field from { x: number, y: number }
---@field to { x: number, y: number }
---@field color string
---@field width? number

---@class iced.CanvasPath
---@field type "path"
---@field points { x: number, y: number }[]
---@field color string
---@field width? number
---@field fill? boolean

---@class iced.CanvasText
---@field type "text"
---@field x? number
---@field y? number
---@field content string
---@field size? number
---@field color string

---@alias iced.CanvasShape iced.CanvasRect|iced.CanvasCircle|iced.CanvasLine|iced.CanvasPath|iced.CanvasText

---@class iced.Task
---@field type "focus"|"scroll_to"|"clipboard_read"|"clipboard_write"|"font"|"window"|"exit"
---@field id? string
---@field x? number
---@field y? number
---@field content? string
---@field path? string
---@field action? "resize"|"move"|"close"|"fullscreen"|"windowed"
---@field window? any
---@field width? number
---@field height? number

---@class iced.Props
---@field width? string|number|{ fill_portion: integer }
---@field height? string|number|{ fill_portion: integer }
---@field padding? number|iced.Padding
---@field spacing? number
---@field size? number
---@field color? string
---@field on_press? any
---@field content_fit? "contain"|"cover"|"fill"|"none"|"scale_down"
---@field transition? iced.Transition
---@field line_height? number|{ pixels: number }
---@field wrap? boolean|"none"|"word"|"glyph"|"word_or_glyph"
---@field align_x? "left"|"center"|"right"|"justified"
---@field align_y? "top"|"center"|"bottom"
---@field shaping? "auto"|"basic"|"advanced"
---@field center? boolean
---@field clip? boolean
---@field max_width? number
---@field max_height? number
---@field direction? "horizontal"|"vertical"|"both"
---@field scrollbar_hidden? boolean Hide scrollbars with zero width; wheel/touch scrolling stays enabled.
---@field thickness? number
---@field value? number|boolean|string
---@field girth? number
---@field vertical? boolean
---@field opacity? number
---@field font? string|iced.Font
---@field text_size? number
---@field label? string
---@field min? number
---@field max? number
---@field step? number
---@field options? string[]
---@field selected? string|number
---@field placeholder? string
---@field secure? boolean
---@field id? string
---@field scale? number
---@field x? number
---@field y? number
---@field tooltip? string
---@field position? "top"|"bottom"|"left"|"right"|"follow_cursor"
---@field gap? number
---@field columns? number
---@field fluid? number
---@field style? iced.Style|iced.RuleStyle|iced.ProgressStyle|iced.QrStyle|iced.MarkdownStyle|iced.PaneGridStyle
---@field cell_size? number
---@field total_size? number
---@field on_input? string
---@field on_submit? string
---@field on_paste? string
---@field on_toggle? string
---@field on_select? string
---@field on_change? string
---@field on_release? string
---@field on_scroll? string
---@field on_show? string
---@field on_hide? any
---@field on_resize? string
---@field on_enter? any
---@field on_exit? any
---@field on_move? string
---@field on_double_click? any
---@field on_right_press? any
---@field on_right_release? any
---@field on_middle_press? any
---@field on_middle_release? any
---@field on_action? string
---@field on_click? string
---@field on_drag? string
---@field on_link_click? string
---@field wrapping? boolean|"none"|"word"|"glyph"|"word_or_glyph"
---@field columns? iced.TableColumn[]
---@field key? string
---@field render? fun(item: any, index: integer): iced.Element
---@field min_size? number

---@class iced.LayerSettings
---@field id? string
---@field size? { width: string|integer, height: string|integer }
---@field layer? "background"|"bottom"|"top"|"overlay"
---@field anchor? ("top"|"bottom"|"left"|"right")[]
---@field exclusive_zone? integer
---@field margin? integer[]
---@field keyboard_interactivity? "none"|"on_demand"|"exclusive"
---@field output_option? "active"|"last_output"|string|integer
---@field events_transparent? boolean
---@field namespace? string
---@field blur_option? "none"|"full"|{ x: integer, y: integer, width: integer, height: integer }[]

---@class iced.WindowSettings
---@field id? string
---@field title? string
---@field width? integer
---@field height? integer
---@field client_side_decorations? boolean
---@field content iced.Element

---@class iced.LayerValues
---@field background "background"
---@field bottom "bottom"
---@field top "top"
---@field overlay "overlay"

---@class iced.ExtentValues
---@field fill "fill"

---@class iced.LengthValues
---@field fill "fill"
---@field shrink "shrink"
---@field fill_portion fun(portion: integer): { fill_portion: integer }

---@class iced.KeyboardInteractivityValues
---@field none "none"
---@field on_demand "on_demand"
---@field exclusive "exclusive"

---@class iced.OutputValues
---@field active "active"
---@field last_output "last_output"

---@class iced.BlurValues
---@field none "none"
---@field full "full"

-- Elements and value helpers.

iced = {}

---@type iced.LayerValues
iced.Layer = {
	background = "background",
	bottom = "bottom",
	top = "top",
	overlay = "overlay",
}

---@type iced.ExtentValues
iced.Extent = { fill = "fill" }

---@param portion integer
---@return { fill_portion: integer }
local function fill_portion(portion)
	return { fill_portion = portion }
end

---@type iced.LengthValues
iced.Length = {
	fill = "fill",
	shrink = "shrink",
	fill_portion = fill_portion,
}

---@type iced.KeyboardInteractivityValues
iced.KeyboardInteractivity = {
	none = "none",
	on_demand = "on_demand",
	exclusive = "exclusive",
}

---@type iced.OutputValues
iced.Output = { active = "active", last_output = "last_output" }

---@type iced.BlurValues
iced.Blur = { none = "none", full = "full" }

---@param width string|integer
---@param height string|integer
---@return { width: string|integer, height: string|integer }
function iced.Size(width, height) end

---@param color string
---@param percent number
---@return string
function iced.opacity(color, percent) end

---@param content string|number
---@param props? iced.Props
---@return iced.Element
function iced.text(content, props) end

---@param content iced.Element[]
---@param props? iced.Props
---@return iced.Element
function iced.row(content, props) end

---@param content iced.Element[]
---@param props? iced.Props
---@return iced.Element
function iced.column(content, props) end

---@param content iced.Element
---@param props? iced.Props
---@return iced.Element
function iced.container(content, props) end

---@param content string|number|iced.Element
---@param props? iced.Props
---@return iced.Element
function iced.button(content, props) end

---@param content nil
---@param props? iced.Props
---@return iced.Element
function iced.space(content, props) end

---@param content string
---@param props? iced.Props
---@return iced.Element
function iced.image(content, props) end

---@param content string
---@param props? iced.Props
---@return iced.Element
function iced.svg(content, props) end

---@param content nil
---@param props? iced.Props
---@return iced.Element
function iced.rule(content, props) end

---@param content nil
---@param props? iced.Props
---@return iced.Element
function iced.progress_bar(content, props) end

---@param content string
---@param props? iced.Props
---@return iced.Element
function iced.qr_code(content, props) end

---@param content iced.Element[]
---@param props? iced.Props
---@return iced.Element
function iced.stack(content, props) end

---@param content iced.Element
---@param props? iced.Props
---@return iced.Element
function iced.float(content, props) end

---@param content iced.Element
---@param props? iced.Props
---@return iced.Element
function iced.pin(content, props) end

---@param content iced.Element
---@param props? iced.Props
---@return iced.Element
function iced.tooltip(content, props) end

---@param content iced.Element[]
---@param props? iced.Props
---@return iced.Element
function iced.grid(content, props) end

---@param content nil
---@param props? iced.Props
---@return iced.Element
function iced.checkbox(content, props) end

---@param content nil
---@param props? iced.Props
---@return iced.Element
function iced.radio(content, props) end

---@param content nil
---@param props? iced.Props
---@return iced.Element
function iced.toggler(content, props) end

---@param content nil
---@param props? iced.Props
---@return iced.Element
function iced.slider(content, props) end

---@param content nil
---@param props? iced.Props
---@return iced.Element
function iced.vertical_slider(content, props) end

---@param content iced.Element
---@param props? iced.Props
---@return iced.Element
function iced.mouse_area(content, props) end

---@param content iced.Element
---@param props? iced.Props
---@return iced.Element
function iced.sensor(content, props) end

---@param content iced.Element
---@param props? iced.Props
---@return iced.Element
function iced.scrollable(content, props) end

---@param content nil
---@param props? iced.Props
---@return iced.Element
function iced.text_input(content, props) end

---@param content nil
---@param props? iced.Props
---@return iced.Element
function iced.pick_list(content, props) end

---@param content nil
---@param props? iced.Props
---@return iced.Element
function iced.text_editor(content, props) end

---@param content nil
---@param props? iced.Props
---@return iced.Element
function iced.combo_box(content, props) end

---@param content (string|number|iced.Element)[][]
---@param props? iced.Props
---@return iced.Element
function iced.table(content, props) end

---@param content any[]
---@param props? iced.Props
---@return iced.Element
function iced.list_view(content, props) end

---@param content string
---@param props? iced.Props
---@return iced.Element
function iced.markdown(content, props) end

---@param content fun(size: { width: number, height: number }): iced.Element
---@param props? iced.Props
---@return iced.Element
function iced.responsive(content, props) end

---@param content iced.CanvasShape[]
---@param props? iced.Props
---@return iced.Element
function iced.canvas(content, props) end

---@param content iced.Element[]
---@param props? iced.Props
---@return iced.Element
function iced.pane_grid(content, props) end

---@param content iced.Element
---@param props? iced.Props
---@return iced.Element
function iced.region(content, props) end

---@param content iced.Element
---@param settings? iced.LayerSettings
---@return iced.Element
function iced.layer(content, settings) end

---@param settings iced.WindowSettings
---@return iced.Element
function iced.window(settings) end

---@param name string
---@param factory fun(): table
function iced.define(name, factory) end

---@param id string
---@param module string
---@param props? table<string, any>
---@return iced.Element
function iced.component(id, module, props) end

---@class iced.Subscription

---@param milliseconds integer
---@return iced.Subscription
function iced.every(milliseconds) end

---@return iced.Subscription
function iced.frames() end

---@param pressed? boolean
---@return iced.Subscription
function iced.on_key(pressed) end

---@return iced.Subscription
function iced.on_mouse() end

-- Daemon builder and extension proxies.

riced = {}

---@class riced.FileOptions
---@field path string
---@field type? "json"|"toml"
---@field onLoaded? fun(file: riced.File, data: any)
---@field onError? fun(file: riced.File, error: string)

---@class riced.File
---@field path string
---@field type "json"|"toml"
---@field data any
---@field error string?
---@field write fun(self: riced.File, data?: any): boolean, string?
---@field save fun(self: riced.File): boolean, string?

---@param options riced.FileOptions
---@return riced.File
function riced.file(options) end

---@class riced.LayerSettings
---@field start_mode? "background"|"active"|"all_screens"
---@field keyboard_interactivity? "none"|"on_demand"|"exclusive"

---@class riced.Settings
---@field id? string
---@field layer_settings? riced.LayerSettings

---@class riced.Daemon
---@field app table
---@field namespace string
local Daemon = {}

---@param app table
---@param namespace string
---@param update fun(self: table, event: any, component: string?): nil|iced.Task|iced.Task[]
---@param view fun(self: table, props: { window: any, width: number?, height: number? }): iced.Element|iced.Element[]|nil
---@return riced.Daemon
function riced.daemon(app, namespace, update, view) end

---@param value string|fun(window: any): string?
---@return riced.Daemon
function Daemon:title(value) end

---@param value fun(active: boolean): iced.Subscription|iced.Subscription[]|nil
---@return riced.Daemon
function Daemon:subscription(value) end

---@param value riced.Theme|fun(window: any): riced.Theme?
---@return riced.Daemon
function Daemon:theme(value) end

---@param value fun(palette: riced.Theme): { background_color?: string, text_color?: string }|nil
---@return riced.Daemon
function Daemon:style(value) end

---@param value fun(event: any): any
---@return riced.Daemon
function Daemon:redraw_scope(value) end

---@param value riced.Settings
---@return riced.Daemon
function Daemon:settings(value) end

---@return riced.Daemon
function Daemon:run() end

---@param handlers table<string, fun(self: table, args: string[])>
function riced.ipc(handlers) end

---@class riced.Extension
---@field name string
---@field alive boolean
---@field store table<string, any>

---@param name string
---@return riced.Extension
function riced.plugins(name) end

---@param name string
---@return riced.Extension
function riced.plugin(name) end
