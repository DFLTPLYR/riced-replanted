-- Example shader component using the current iced.shader API.
--
-- Use it from a layer or window, for example:
--   local shader = require("components.shader")
--   return iced.layer(shader.view(), { id = "background", anchor = { "top", "bottom", "left", "right" }, size = { width = "fill", height = "fill" } })
--
-- Shader files resolve from the profile root. Requires the wgpu renderer.
local shader = {}

function shader.view(props)
    return iced.shader({
        path = "components/shaders/plasma.wgsl",
        width = props and props.width or "fill",
        height = props and props.height or "fill",
        animated = true,
        fps = 30,
    })
end

return shader
