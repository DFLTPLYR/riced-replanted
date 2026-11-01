local panel = {}

function panel:view(props)
    return iced.container(iced.column({
        iced.text("riced", { size = 28, color = theme.primary }),
        iced.text("Count: " .. props.count, { color = theme.secondary }),
        iced.button("Increment", { on_press = "increment" }),
        iced.component("system", "components.system"),
    }, { spacing = 12 }), { padding = 24, width = "fill", height = "fill" })
end

return panel
