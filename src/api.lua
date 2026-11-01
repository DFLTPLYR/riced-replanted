local builder = {}
builder.__index = builder

riced = {}
__riced_plugins = {}
function riced.plugins(name)
    local proxy = __riced_plugins[name]
    if not proxy then
        proxy = { name = name, alive = false, store = {} }
        __riced_plugins[name] = proxy
    end
    return proxy
end

function riced.daemon(app, namespace, update, view)
    assert(type(app) == "table", "daemon app must be a table")
    assert(type(namespace) == "string" and namespace ~= "", "namespace must not be empty")
    assert(type(update) == "function", "update must be a function")
    assert(type(view) == "function", "view must be a function")
    return setmetatable({ app = app, namespace = namespace, update = update, view = view, options = {} }, builder)
end

for _, name in ipairs({ "title", "subscription", "theme", "style", "redraw_scope", "settings" }) do
    builder[name] = function(self, value)
        self.options[name] = value
        return self
    end
end

function builder:run()
    assert(not self.started, "daemon already started")
    self.started = true
    return self
end

iced = {}
iced.Layer = { background = "background", bottom = "bottom", top = "top", overlay = "overlay" }
iced.Extent = { fill = "fill" }
iced.Length = { fill = "fill", shrink = "shrink" }
iced.KeyboardInteractivity = { none = "none", on_demand = "on_demand", exclusive = "exclusive" }
iced.Output = { active = "active", last_output = "last_output" }
iced.Blur = { none = "none", full = "full" }

function iced.Size(width, height)
    assert(width ~= nil and height ~= nil, "size requires width and height")
    return { width = width, height = height }
end

function iced.Length.fill_portion(portion)
    assert(type(portion) == "number" and portion % 1 == 0 and portion >= 1 and portion <= 65535,
        "fill portion must be an integer from 1 to 65535")
    return { fill_portion = portion }
end

for _, kind in ipairs({ "text", "row", "column", "container", "button", "space" }) do
    iced[kind] = function(content, props)
        props = props or {}
        assert(type(props) == "table", "widget props must be a table")
        local node = {}
        for key, value in pairs(props) do node[key] = value end
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

function iced.component(id, module, props)
    assert(type(id) == "string" and id ~= "", "component id must be a nonempty string")
    assert(type(module) == "string" and module ~= "", "component module must be a nonempty string")
    assert(props == nil or type(props) == "table", "component props must be a table")
    return { kind = "component", id = id, module = module, props = props or {} }
end

function iced.every(milliseconds)
    assert(type(milliseconds) == "number" and milliseconds >= 1 and milliseconds <= 86400000
        and milliseconds % 1 == 0, "timer interval must be an integer from 1 to 86400000 ms")
    return { every = milliseconds }
end
