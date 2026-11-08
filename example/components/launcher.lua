-- Application launcher overlay. Open it from the compositor with
-- `riced ipc launcher`; the applications extension must be installed.

local launcher = {}

local function online()
    local plugin = riced.plugins("applications")
    return plugin and plugin.alive or false
end

local function available()
    if not online() then
        return {}
    end
    return riced.plugins("applications").store.applications or {}
end

function launcher.filter(entries, query)
    query = (query or ""):lower()
    if query == "" then
        return entries or {}
    end
    local matches = {}
    for _, entry in ipairs(entries or {}) do
        local name = (entry.name or ""):lower()
        local id = (entry.id or ""):lower()
        local comment = (entry.comment or ""):lower()
        if name:find(query, 1, true)
            or id:find(query, 1, true)
            or comment:find(query, 1, true)
        then
            matches[#matches + 1] = entry
        end
    end
    return matches
end

function launcher.matches(app)
    return launcher.filter(available(), app.launcher_query)
end

function launcher.close(app)
    app.launcher_open = false
    app.launcher_query = ""
    app.launcher_selected = 1
end

function launcher.toggle(app)
    if app.launcher_open then
        launcher.close(app)
        return nil
    end
    app.launcher_open = true
    app.launcher_query = ""
    app.launcher_selected = 1
    return { type = "focus", id = "launcher-search" }
end

function launcher.launch(app, id)
    local plugin = riced.plugins("applications")
    if not plugin or not plugin.alive then
        log("applications extension is offline")
        return
    end
    plugin.launch(id, function(_, error)
        if error then log(error) end
    end)
    launcher.close(app)
end

function launcher.update(app, event)
    if event == "launcher_close" then
        launcher.close(app)
        return
    end
    if type(event) ~= "table" then
        return
    end
    if event.event == "launcher_input" then
        app.launcher_query = event.value or ""
        app.launcher_selected = 1
        return
    end
    if event.type == "launcher_launch" then
        launcher.launch(app, event.id)
        return
    end
    if not app.launcher_open or event.type ~= "key" or event.event ~= "press" then
        return
    end

    if event.key == "Escape" then
        launcher.close(app)
        return
    end
    local matches = launcher.matches(app)
    local count = #matches
    if count == 0 then
        return
    end
    local selected = math.min(math.max(app.launcher_selected or 1, 1), count)
    if event.key == "ArrowDown" then
        app.launcher_selected = selected % count + 1
    elseif event.key == "ArrowUp" then
        app.launcher_selected = (selected - 2) % count + 1
    elseif event.key == "Enter" then
        app.launcher_selected = selected
        launcher.launch(app, matches[selected].id)
    end
end

local function icon(entry)
    local path = entry.icon and iced.icon_path(entry.icon, { size = 24, thumbnail = true })
    if not path then
        return iced.space(nil, { width = 24, height = 24 })
    end
    local props = { width = 24, height = 24, content_fit = "contain" }
    if path:sub(-4):lower() == ".svg" then
        return iced.svg(path, props)
    end
    return iced.image(path, props)
end

local function row(entry, active)
    local text = active and theme.background or theme.text
    local secondary = active and theme.background or theme.secondary
    local detail = entry.comment or entry.id or ""
    return iced.button(iced.row({
        icon(entry),
        iced.column({
            iced.text(entry.name or entry.id or "?", { color = text }),
            iced.text(detail, { size = 12, color = secondary, wrap = "none" }),
        }, { spacing = 2, width = "fill" }),
    }, { spacing = 12, align_y = "center", width = "fill" }), {
        width = "fill",
        on_press = { type = "launcher_launch", id = entry.id },
        style = { background = active and theme.primary or theme.background },
    })
end

function launcher.view(app)
    local entries = launcher.matches(app)
    local selected = math.min(
        math.max(app.launcher_selected or 1, 1),
        math.max(#entries, 1)
    )
    local results
    if #entries == 0 then
        results = iced.container(
            iced.text(
                online() and "no matches" or "applications extension is offline",
                { color = theme.secondary }
            ),
            { width = "fill", height = "fill", align_x = "center", align_y = "center" }
        )
    else
        results = iced.list_view(entries, {
            id = "launcher-results",
            key = "id",
            spacing = 2,
            height = "fill",
            render = function(entry, index)
                return row(entry, index + 1 == selected)
            end,
        })
    end

    local panel = iced.container(iced.column({
        iced.row({
            iced.text("Applications", { size = 20, color = theme.primary }),
            iced.space(nil, { width = "fill" }),
            iced.button("Close", { on_press = "launcher_close" }),
        }, { width = "fill", align_y = "center" }),
        iced.text_input(nil, {
            id = "launcher-search",
            value = app.launcher_query or "",
            placeholder = "Search applications",
            on_input = "launcher_input",
            width = "fill",
        }),
        results,
        iced.text("Enter launch · Esc close · ↑↓ select", {
            size = 12,
            color = theme.secondary,
        }),
    }, { spacing = 12, width = "fill", height = "fill" }), {
        width = 560,
        height = 420,
        padding = 16,
        style = {
            background = theme.background,
            border = { color = theme.primary, width = 1 },
        },
    })

    return iced.layer(
        iced.container(panel, {
            width = "fill",
            height = "fill",
            align_x = "center",
            align_y = "center",
        }),
        {
            id = "launcher",
            size = { width = "fill", height = "fill" },
            layer = "overlay",
            anchor = { "top", "bottom", "left", "right" },
            exclusive_zone = -1,
            keyboard_interactivity = "exclusive",
            namespace = "riced-launcher",
        }
    )
end

return launcher
