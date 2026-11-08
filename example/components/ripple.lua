-- Centered ripple shader on a transparent overlay.
--
-- Mount it from a shell:
--   local ripple = require("components.ripple")
--   surfaces[#surfaces + 1] = ripple.view()
--
-- Requires the wgpu renderer; the tiny-skia fallback renders nothing.
local ripple = {}

local SIZE = 400

local function content()
    return iced.responsive(function(size)
        return iced.pin(
            iced.container(
                iced.shader({
                    path = "components/shaders/ripple.wgsl",
                    width = iced.Length.fill,
                    height = iced.Length.fill,
                    animated = true,
                    fps = 165,
                    extraArgs = { opacity = 0.35, speed = 1.0 },
                }),
                { width = SIZE, height = SIZE, style = { background = theme.primary } }
            ),
            {
                width = iced.Length.fill,
                height = iced.Length.fill,
                x = size.width / 2 - SIZE / 2,
                y = size.height / 2 - SIZE / 2,
            }
        )
    end)
end

function ripple.view()
    return iced.layer(content(), {
        id = "ripple",
        size = iced.Size(iced.Extent.fill, iced.Extent.fill),
        layer = iced.Layer.overlay,
        anchor = { "top", "bottom", "left", "right" },
        exclusive_zone = -1,
        keyboard_interactivity = iced.KeyboardInteractivity.none,
        output_option = iced.Output.active,
        events_transparent = true,
        namespace = "riced-ripple",
    })
end

return ripple
