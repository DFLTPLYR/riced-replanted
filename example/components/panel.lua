local panel = {}

function panel:view(props)
    return iced.container(iced.column({
        iced.row({
            iced.icon("\u{e038}", { size = 28, color = theme.primary }), -- "activity" glyph; resolve names via riced.plugins("lucide").resolve("activity")
            iced.text("riced", { size = 28, color = theme.primary }),
        }, { spacing = 8 }),
        iced.text("Count: " .. props.count, { color = theme.secondary }),
        iced.button("Increment", { on_press = "increment" }),
        iced.component("system", "components.system"),
    }, { spacing = 12 }), { padding = 24, width = "fill", height = "fill" })
end

return panel
