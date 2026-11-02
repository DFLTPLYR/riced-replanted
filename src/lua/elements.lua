-- Elements: widgets, layer surfaces, and component references.

for _, kind in ipairs({
	"text",
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
		return node
	end
end

function iced.layer(content, settings)
	assert(type(content) == "table", "layer content must be an iced element")
	assert(settings == nil or type(settings) == "table", "layer settings must be a table")
	return { kind = "layer", content = content, settings = settings or {} }
end

-- Custom widgets: define a named constructor once, then use iced.name(props).
-- The factory returns a table with a view (or render) method, or an element.
function iced.define(name, factory)
	assert(type(name) == "string" and name:match("^[%a_][%w_]*$"), "widget name must be a valid identifier")
	assert(type(factory) == "function", "widget factory must be a function")
	assert(iced[name] == nil, "widget already exists: " .. name)
	local widget = factory()
	assert(type(widget) == "table", "widget factory must return a table")
	local view = widget.view or widget.render
	assert(type(view) == "function", "custom widget needs a view or render method")
	iced[name] = function(props)
		assert(props == nil or type(props) == "table", "widget props must be a table")
		return view(widget, props or {})
	end
end


function iced.component(id, module, props)
	assert(type(id) == "string" and id ~= "", "component id must be a nonempty string")
	assert(type(module) == "string" and module ~= "", "component module must be a nonempty string")
	assert(props == nil or type(props) == "table", "component props must be a table")
	return { kind = "component", id = id, module = module, props = props or {} }
end
