local background = require("components.layers.background")
local app = { count = 0 }

function app:update(event)
    if event == "increment" then
        self.count = self.count + 1
    elseif type(event) == "table" and event.type == "workspace" then
        riced.plugin("wayland").activate_workspace(event.id)
    end
end

-- Optional generic Wayland UI (requires the wayland extension):
-- local wayland = require("components.wayland")
-- return wayland.view()
function app:view(props)
    return background.view(self.count)
end

function app:title(window) return "riced" end
function app:subscription(active) return nil end -- or iced.every(1000), sends { type = "tick" }
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
