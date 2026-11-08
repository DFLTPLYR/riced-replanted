-- Daemon builder: riced.daemon() plus the chained options.

local builder = {}
builder.__index = builder

riced = {}

function riced.daemon(app, namespace, update, view)
	assert(type(app) == "table", "daemon app must be a table")
	assert(type(namespace) == "string" and namespace ~= "", "namespace must not be empty")
	assert(type(update) == "function", "update must be a function")
	assert(type(view) == "function", "view must be a function")
	return setmetatable({ app = app, namespace = namespace, update = update, view = view, options = {} }, builder)
end

for _, name in ipairs({ "title", "subscription", "theme", "style", "redraw_scope", "settings", "on_new_shell" }) do
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
