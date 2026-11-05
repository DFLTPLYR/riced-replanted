-- Generic Wayland workspace pills and focused title.
-- Requires the `wayland` extension; renders an empty row without it.

local wayland = {}

function wayland.view()
	local plugin = riced.plugin("wayland")
	local focused = plugin.focused or {}
	local workspaces = iced.list_view(plugin.workspaces or {}, {
		id = "wayland-workspaces",
		key = "id",
		spacing = 4,
		direction = "horizontal",
		transition = {
			enter = { duration = 180, from = { x = -180, scale = 0 } },
			exit = { duration = 120, to = { x = -180, scale = 0 } },
			displaced = { duration = 180 },
		},
		render = function(workspace, index)
			local can_activate = false
			for _, capability in ipairs(workspace.capabilities or {}) do
				if capability == "activate" then
					can_activate = true
				end
			end
			return iced.button(workspace.name, {
				on_press = (not workspace.active and can_activate)
						and { type = "workspace", id = workspace.id }
					or nil,
				style = workspace.active and { background = theme.primary } or nil,
			})
		end,
	})
	local title = ""
	for _, toplevel in ipairs(plugin.toplevels or {}) do
		if toplevel.id == focused.toplevel then
			title = toplevel.title
			break
		end
	end
	return iced.column({
		workspaces,
		iced.text(title),
	}, { spacing = 4 })
end

return wayland
