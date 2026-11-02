-- Generic Wayland workspace pills and focused title.
-- Requires the `wayland` extension; renders an empty row without it.

local wayland = {}

function wayland.view()
	local plugin = riced.plugin("wayland")
	local workspaces = plugin.workspaces or {}
	local focused = plugin.focused or {}
	local pills = {}
	for _, workspace in ipairs(workspaces) do
		local can_activate = false
		for _, capability in ipairs(workspace.capabilities or {}) do
			if capability == "activate" then
				can_activate = true
			end
		end
		table.insert(
			pills,
			iced.button(workspace.name, {
				on_press = (not workspace.active and can_activate)
						and { type = "workspace", id = workspace.id }
					or nil,
				style = workspace.active and { background = theme.primary } or nil,
			})
		)
	end
	local title = ""
	for _, toplevel in ipairs(plugin.toplevels or {}) do
		if toplevel.id == focused.toplevel then
			title = toplevel.title
			break
		end
	end
	return iced.column({
		iced.row(pills, { spacing = 4 }),
		iced.text(title),
	}, { spacing = 4 })
end

return wayland
