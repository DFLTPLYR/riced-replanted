-- Extension proxies: riced.plugin(name) / riced.plugins(name).
--
-- Unknown keys on the proxy read through to the store, so
-- riced.plugin("wayland").toplevels is the same as
-- riced.plugin("wayland").store.toplevels.

__riced_plugins = {}

__riced_plugin_meta = {
	__index = function(proxy, key)
		local store = rawget(proxy, "store")
		if store then
			return store[key]
		end
	end,
}

function riced.plugins(name)
	local proxy = __riced_plugins[name]
	if not proxy then
		proxy = { name = name, alive = false, store = {} }
		__riced_plugins[name] = proxy
	end
	if getmetatable(proxy) == nil then
		setmetatable(proxy, __riced_plugin_meta)
	end
	return proxy
end

riced.plugin = riced.plugins
