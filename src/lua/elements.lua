-- Elements: widgets, layer surfaces, and component references.

for _, kind in ipairs({
	"text",
	"icon",
	"row",
	"column",
	"container",
	"button",
	"space",
	"image",
	"svg",
	"rule",
	"progress_bar",
	"qr_code",
	"stack",
	"float",
	"pin",
	"tooltip",
	"grid",
	"checkbox",
	"radio",
	"toggler",
	"slider",
	"vertical_slider",
	"mouse_area",
	"sensor",
	"scrollable",
	"text_input",
	"pick_list",
	"text_editor",
	"combo_box",
	"table",
	"list_view",
	"region",
	"markdown",
	"responsive",
	"canvas",
	"pane_grid",
}) do
	iced[kind] = function(content, props)
		props = props or {}
		assert(type(props) == "table", "widget props must be a table")
		local node = {}
		for key, value in pairs(props) do
			node[key] = value
		end
		node.kind = kind
		node.content = content
		if node.transition and node.id and kind ~= "list_view" then
			return riced._animations.native(node)
		end
		return node
	end
end

function iced.layer(content, settings)
	assert(type(content) == "table", "layer content must be an iced element")
	assert(settings == nil or type(settings) == "table", "layer settings must be a table")
	return { kind = "layer", content = content, settings = settings or {} }
end

function iced.window(settings)
	assert(type(settings) == "table", "window settings must be a table")
	assert(type(settings.content) == "table", "window content must be an iced element")
	local node = { kind = "window", content = settings.content, settings = {} }
	for key, value in pairs(settings) do
		if key ~= "content" then
			node.settings[key] = value
		end
	end
	return node
end

function iced.popup(content, settings)
	assert(type(content) == "table", "popup content must be an iced element")
	assert(settings == nil or type(settings) == "table", "popup settings must be a table")
	return { kind = "popup", content = content, settings = settings or {} }
end

-- Named definitions: widgets use iced.name(props, ...), values use iced.name().
-- Matching Lua files anywhere in the profile load on demand; redefining a name
-- replaces the previous constructor so edits reload.
-- Key by constructor so restoring iced after a failed reload also restores its kind.
local defined = setmetatable({}, { __mode = "k" })
local windows = {}
local records = {}
local parent
local rebuilding = false
local defer_native = true
local animations = {}
riced._animations = animations

function animations.begin(window, tracked)
	defer_native = true
	if tracked then
		windows[window] = windows[window] or {}
		records = windows[window]
	else
		records = {}
	end
	parent = nil
	for _, record in pairs(records) do record.seen = false end
end

function animations.checkpoint()
	local tables, controls = {}, {}
	local function save(value)
		local copy = {}
		for key, field in pairs(value) do copy[key] = field end
		tables[value] = copy
	end
	save(windows)
	for _, window in pairs(windows) do
		save(window)
		for _, record in pairs(window) do
			save(record)
			save(record.self)
			controls[record.control] = record.control:checkpoint()
		end
	end
	local previous_records, previous_parent = records, parent
	local previous_defer, previous_rebuild = defer_native, rebuilding
	defer_native = true
	return function()
		for _, window in pairs(windows) do
			for _, record in pairs(window) do
				if not controls[record.control] then record.control:cancel() end
			end
		end
		for value, copy in pairs(tables) do
			for key in pairs(value) do value[key] = nil end
			for key, field in pairs(copy) do value[key] = field end
		end
		for _, restore in pairs(controls) do restore() end
		records, parent = previous_records, previous_parent
		defer_native, rebuilding = previous_defer, previous_rebuild
	end
end

function animations.instance(key, definition, props)
	local record = records[key]
	if record and record.definition ~= definition then
		record.control:cancel()
		record = nil
	end
	if not record then
		local control = riced._new_animation()
		local instance = setmetatable({}, { __index = definition })
		record = { definition = definition, self = instance, control = control, key = key }
		instance.__animation_key = key
		instance.animate = setmetatable({}, { __index = function(_, event)
			assert(type(event) == "string", "animation event must be a string")
			return function()
				assert(event == "enter" or event == "exit" or
					(record.node and record.node.transition and record.node.transition[event]),
					"unknown animation event: " .. event)
				control:start(event)
			end
		end })
		records[key] = record
	end
	assert(not record.seen or rebuilding, "duplicate widget instance: " .. key .. "; provide a distinct props.id")
	if record.leaving then record.control:start("enter") end
	record.seen, record.leaving, record.parent, record.external = true, false, parent, nil
	record.self.props = props
	return record.self
end

local native = {}
function animations.native(node)
	assert(type(node.id) == "string" and node.id ~= "", "animated widget id must be a nonempty string")
	if defer_native then return node end
	local key = "native:" .. (parent or "") .. ":" .. node.kind .. ":" .. node.id
	animations.instance(key, native, node)
	return animations.track(key, node)
end

-- The root view constructs every surface; only register the selected subtree.
function animations.select(node)
	defer_native = false
	local copies = {}
	local function visit(value)
		if type(value) ~= "table" then return value end
		if copies[value] then return copies[value] end
		local copy = {}
		copies[value] = value
		for key, field in pairs(value) do copy[key] = field end
		local previous = parent
		if copy.__animation_self then parent = copy.__animation_self.__animation_key end
		if copy.kind then
			for _, field in ipairs({ "content", "columns", "header", "tooltip" }) do
				if not (copy.kind == "list_view" and copy.render and field == "content") then
					copy[field] = visit(copy[field])
				end
			end
			if copy.transition and copy.id and copy.kind ~= "list_view" and not copy.__animation then
				copy = animations.native(copy)
			end
		else
			for key, field in pairs(copy) do copy[key] = visit(field) end
		end
		parent = previous
		for key, field in pairs(copy) do
			if field ~= value[key] then
				copies[value] = copy
				return copy
			end
		end
		return value
	end
	return visit(node)
end

function animations.owner(key)
	local previous = parent
	parent = key
	return previous
end

function animations.render_item(render, item, index, owner, rebuild, window)
	local previous = parent
	local previous_rebuild = rebuilding
	local previous_records = records
	if window and windows[window] then records = windows[window] end
	rebuilding = rebuild or false
	parent = owner and owner.__animation_key or parent
	local ok, node = pcall(render, item, index)
	parent = previous
	rebuilding = previous_rebuild
	records = previous_records
	if not ok then error(node, 0) end
	return node
end

function animations.track(key, node)
	local record = records[key]
	assert(type(node) == "table", "component view must return an iced element")
	local copy = {}
	for field, value in pairs(node) do copy[field] = value end
	node = copy
	-- A component owns its root animation; nested roots retain their own controls.
	local native_key = node.__animation_self and node.__animation_self.__animation_key
	if native_key and native_key:sub(1, 7) == "native:" and records[native_key] and records[native_key].parent == key then
		animations.remove(native_key)
	elseif (node.__animation and node.__animation_self ~= record.self) or node.kind == "list_view" then
		node = iced.container(node)
	end
	node.__animation, node.__animation_self = record.control, record.self
	record.node = node
	return node
end

function animations.render(key, definition, props, window, args)
	local instance = animations.instance(key, definition, props)
	local previous = parent
	parent = key
	local ok, node
	if args then
		ok, node = pcall(definition.view or definition.render, instance, table.unpack(args, 1, args.n))
	else
		ok, node = pcall(definition.view or definition.render, instance, props, window)
	end
	parent = previous
	if not ok then error(node, 0) end
	return animations.track(key, node), instance
end

function animations.finish()
	local leaving = {}
	local roots = {}
	for key, record in pairs(records) do
		local ancestor = record.parent and records[record.parent]
		if not record.seen and (not ancestor or ancestor.seen) then roots[#roots + 1] = key end
	end
	local function remove(key)
		for child, record in pairs(records) do
			if record.parent == key then remove(child) end
		end
		animations.remove(key)
	end
	for _, key in ipairs(roots) do
		local record = records[key]
		if record then
			local exit = record.node and record.node.transition and record.node.transition.exit
            if record.external or not exit or record.control:removable() then
				remove(key)
			else
				record.leaving = true
				record.control:start("exit")
				local node = {}
				for field, value in pairs(record.node) do node[field] = value end
				node.__retained = true
				leaving[#leaving + 1] = node
			end
		end
	end
	return leaving
end

function animations.retain(node, control, window)
	local retained = window and windows[window] or records
	local function mark(key)
		local record = retained[key]
		if not record then return end
		record.seen, record.external = true, control
		for child, value in pairs(retained) do
			if value.parent == key then mark(child) end
		end
	end
	local seen = {}
	local function visit(value)
		if type(value) ~= "table" or seen[value] then return end
		seen[value] = true
		if value.__animation_self then mark(value.__animation_self.__animation_key) end
		if value.kind then visit(value.content) else
			for _, child in ipairs(value) do visit(child) end
		end
	end
	visit(node)
end

function animations.snapshot(key, node)
	if records[key] then records[key].node = node end
end

function animations.forget(window)
	for _, record in pairs(windows[window] or {}) do record.control:cancel() end
	windows[window] = nil
end

function animations.remove(key)
	if records[key] then records[key].control:cancel() end
	records[key] = nil
end

function iced.measure(content, props)
	assert(type(content) == "table", "measure content must be an iced element")
	assert(props == nil or type(props) == "table", "measure props must be a table")
	local id = props and props.id
	assert(type(id) == "string" and id ~= "", "measure requires a nonempty id")
	return { kind = "measure", content = content, id = id }
end

local function copy(value, seen)
	if type(value) ~= "table" then return value end
	seen = seen or {}
	if seen[value] then return seen[value] end
	local result = {}
	seen[value] = result
	for key, field in pairs(value) do result[copy(key, seen)] = copy(field, seen) end
	return result
end

function iced.define(name, factory)
	assert(type(name) == "string" and name:match("^[%a_][%w_]*$"), "widget name must be a valid identifier")
	assert(type(factory) == "function", "definition factory must be a function")
	assert(rawget(iced, name) == nil or defined[rawget(iced, name)], "widget already exists: " .. name)
	local widget = factory()
	assert(widget ~= nil, "definition factory must return a value")
	local view = type(widget) == "table" and (widget.view or widget.render)
	if type(view) ~= "function" then
		local value = copy(widget)
		iced[name] = function() return copy(value) end
		defined[iced[name]] = "value"
		return
	end
	iced[name] = function(...)
		local args = table.pack(...)
		for index = 1, args.n do
			assert(args[index] == nil or type(args[index]) == "table", "widget arguments must be tables")
		end
		args[1] = args[1] or {}
		local node = animations.render("widget:" .. name .. ":" .. tostring(args[1].id or name), widget, args[1], nil, args)
		return node
	end
	defined[iced[name]] = "widget"
end

local function array(value)
	for key in pairs(value) do
		if type(key) == "number" then return true end
	end
	return false
end

local function merge(target, overrides, active)
	active = active or {}
	assert(not active[overrides], "transition overrides must not contain cycles")
	active[overrides] = true
	for key, value in pairs(overrides) do
		if key ~= "props" and type(value) == "table" and type(target[key]) == "table"
			and not array(value) and not array(target[key]) then
			target[key] = merge(copy(target[key]), value, active)
		else
			target[key] = copy(value)
		end
	end
	active[overrides] = nil
	return target
end

function iced.transition(preset, overrides)
	if type(preset) == "string" then
		local constructor = iced[preset]
		assert(constructor ~= nil, "unknown transition: " .. preset)
		assert(defined[constructor] == "value", "transition must name a value definition: " .. preset)
		preset = constructor()
	end
	assert(type(preset) == "table", "transition preset must be a table")
	assert(overrides == nil or type(overrides) == "table", "transition overrides must be a table")
	local result = merge(copy(preset), overrides or {})
	local duration, easing, spring = result.duration, result.easing, result.spring
	assert(easing == nil or spring == nil, "transition takes easing or spring, not both")
	result.duration, result.easing, result.spring = nil, nil, nil
	for event, config in pairs(result) do
		assert(type(event) == "string" and event ~= "" and type(config) == "table", "transition must be an event table")
		if config.duration == nil then config.duration = duration end
		if config.easing == nil then config.easing = easing end
		if config.spring == nil then config.spring = spring end
		assert(config.easing == nil or config.spring == nil, "transition takes easing or spring, not both")
	end
	return result
end


function iced.component(id, module, props)
	assert(type(id) == "string" and id ~= "", "component id must be a nonempty string")
	assert(type(module) == "string" and module ~= "", "component module must be a nonempty string")
	assert(props == nil or type(props) == "table", "component props must be a table")
	return { kind = "component", id = id, module = module, props = props or {} }
end
