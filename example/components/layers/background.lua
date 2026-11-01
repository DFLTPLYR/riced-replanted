local background = {}

function background.view(count)
    return iced.layer(
        iced.component("panel", "components.panel", { count = count }),
        {
            id = "background",
            size = { width = "fill", height = "fill" },
            layer = "background",
            anchor = { "top", "bottom", "left", "right" },
            exclusive_zone = -1,
            margin = { 0, 0, 0, 0 },
            keyboard_interactivity = "none",
            output_option = "active",
            events_transparent = false,
            namespace = "riced-background",
            blur_option = "none",
        }
    )
end

return background
