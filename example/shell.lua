local background = require("components.layers.background")
local launcher = require("components.launcher")
local app = { count = 0, launcher_open = false, launcher_query = "", launcher_selected = 1 }

function app:update(event)
    launcher.update(self, event)
    if event == "increment" then
        self.count = self.count + 1
    elseif type(event) == "table" and event.type == "workspace" then
        riced.plugin("wayland").activate_workspace(event.id)
    end
end

-- Bind a compositor shortcut to `riced ipc launcher` to open the launcher.
riced.ipc({
    launcher = function(self, args)
        if #args ~= 0 then error("usage: riced ipc launcher") end
        return launcher.toggle(self)
    end,
})

-- Optional generic Wayland UI (requires the wayland extension):
-- local wayland = require("components.wayland")
-- return wayland.view()
--
-- Optional centered ripple overlay (requires the wgpu renderer):
-- local ripple = require("components.ripple")
-- then add `surfaces[#surfaces + 1] = ripple.view()` in app:view
function app:view(props)
    local surfaces = { background.view(self.count) }
    if self.launcher_open then
        surfaces[#surfaces + 1] = launcher.view(self)
    end
    return surfaces
end

function app:title(window) return "riced" end
function app:subscription(active)
    if self.launcher_open then return iced.on_key(true) end
    return nil -- or iced.every(1000), sends { type = "tick" }
end
function app:style(palette)
    return { background_color = palette.background, text_color = palette.text }
end
function app:redraw_scope(event) return "all" end

return riced
    .daemon(app, "riced", app.update, app.view)
    :title(app.title)
    :subscription(app.subscription)
    :theme(config.theme)
    :style(app.style)
    :redraw_scope(app.redraw_scope)
    :settings({
        id = "riced",
        -- iced.layer creates its own surface, without an extra startup surface.
        layer_settings = { start_mode = "background", keyboard_interactivity = "none" },
    })
    :run()
