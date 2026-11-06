//! Lua API, split by function. Parts run in order: each adds to the globals the previous ones created.

mod icons;

const PARTS: &[(&str, &str)] = &[
    ("daemon", include_str!("daemon.lua")),
    ("plugins", include_str!("plugins.lua")),
    ("values", include_str!("values.lua")),
    ("elements", include_str!("elements.lua")),
    ("subscriptions", include_str!("subscriptions.lua")),
];

fn write_log(
    lua: &mlua::Lua,
    values: impl IntoIterator<Item = mlua::Value>,
    expand_tables: bool,
) -> mlua::Result<()> {
    let tostring: mlua::Function = lua.globals().get("tostring")?;
    let parts = values
        .into_iter()
        .map(|value| match value {
            mlua::Value::Table(table) if expand_tables => Ok(format!("{table:#?}")),
            value => tostring
                .call::<mlua::LuaString>(value)
                .map(|text| text.to_string_lossy()),
        })
        .collect::<mlua::Result<Vec<_>>>()?;
    riced_extension_sdk::log::info!(target: "lua", "{}", parts.join("\t"));
    Ok(())
}

pub fn load(lua: &mlua::Lua) -> mlua::Result<()> {
    let log = lua.create_table()?;
    let metatable = lua.create_table()?;
    metatable.set(
        "__call",
        // `__call` receives the callable table as its first argument.
        lua.create_function(|lua, values: mlua::Variadic<mlua::Value>| {
            write_log(lua, values.into_iter().skip(1), false)
        })?,
    )?;
    log.set_metatable(Some(metatable))?;
    log.set(
        "table",
        lua.create_function(|lua, values: mlua::Variadic<mlua::Value>| {
            write_log(lua, values, true)
        })?,
    )?;
    lua.globals().set("log", log)?;
    for (name, source) in PARTS {
        lua.load(*source)
            .set_name(format!("riced API: {name}"))
            .exec()?;
    }
    crate::widgets::animation::install(lua)?;
    crate::widgets::measure::install(lua)?;
    icons::install(lua)?;
    let handlers = lua.create_table()?;
    lua.set_named_registry_value("riced.ipc_handlers", handlers.clone())?;
    let riced: mlua::Table = lua.globals().get("riced")?;
    riced.set(
        "ipc",
        lua.create_function(move |lua, handlers: mlua::Table| {
            let registered: mlua::Table = lua.named_registry_value("riced.ipc_handlers")?;
            for pair in handlers.pairs::<String, mlua::Function>() {
                let (name, handler) = pair?;
                if name.is_empty() {
                    return Err(mlua::Error::runtime("IPC command name must not be empty"));
                }
                // Replacement keeps handlers working when their module reloads.
                registered.set(name, handler)?;
            }
            Ok(())
        })?,
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn named_values_return_isolated_snapshots() {
        let lua = mlua::Lua::new();
        load(&lua).unwrap();
        lua.load(r##"
            local original = {border = {width = 1}, props = {'width', 'height'}}
            original.self = original
            iced.define('card_style', function() return original end)
            original.border.width = 9
            local first = iced.card_style()
            first.border.width, first.props[1] = 2, 'padding'
            local second = iced.card_style()
            assert(second.border.width == 1 and second.props[1] == 'width')
            assert(second.self == second and first ~= second)
            iced.define('soft', function() return 'ease_in_out' end)
            iced.define('speed', function() return 180 end)
            iced.define('disabled', function() return false end)
            iced.define('static_label', function() return iced.text('hello') end)
            assert(iced.soft() == 'ease_in_out' and iced.speed() == 180 and iced.disabled() == false)
            assert(iced.static_label().kind == 'text')
            iced.define('soft', function() return 'out' end)
            assert(iced.soft() == 'out')
            assert(not pcall(iced.define, 'empty', function() end))
            assert(not pcall(iced.define, 'text', function() return {} end))
        "##).exec().unwrap();
    }

    #[test]
    fn transition_presets_expand_defaults_and_isolate_overrides() {
        let lua = mlua::Lua::new();
        load(&lua).unwrap();
        lua.load(r#"
            iced.define('soft', function() return 'out' end)
            local callback = function() end
            iced.define('fade', function()
                return {
                    duration = 180, easing = iced.soft(),
                    enter = {from = {x = 20, scale = 0.9}},
                    exit = {to = {x = 20}, duration = 80, callback = callback},
                    change = {props = {'width', 'height'}},
                    displaced = {}, pulse = {}, pointer_enter = {},
                }
            end)
            local overrides = {duration = 90, enter = {from = {x = 30}}, change = {props = {'padding'}}}
            local result = iced.transition('fade', overrides)
            assert(result.duration == nil and result.easing == nil and result.spring == nil)
            assert(result.enter.duration == 90 and result.enter.easing == 'out')
            assert(result.enter.from.x == 30 and result.enter.from.scale == 0.9)
            assert(result.exit.duration == 80 and result.exit.callback == callback)
            assert(result.displaced.duration == 90 and result.pulse.easing == 'out')
            assert(result.pointer_enter.duration == 90)
            assert(#result.change.props == 1 and result.change.props[1] == 'padding')
            assert(overrides.enter.from.scale == nil and overrides.change.duration == nil)
            result.enter.from.x = 50
            assert(iced.transition('fade').enter.from.x == 20)
            assert(#iced.transition('fade', {change = {props = {}}}).change.props == 0)
            local shared = {duration = 100}
            local inline = {enter = shared, exit = shared}
            local merged = iced.transition(inline, {enter = {duration = 50}})
            assert(merged.exit.duration == 100 and inline.enter.duration == 100)
            local spring = iced.transition({spring = 'bouncy', change = {}})
            assert(spring.change.spring == 'bouncy')
        "#).exec().unwrap();
    }

    #[test]
    fn transition_presets_reject_invalid_shapes_and_timing_conflicts() {
        let lua = mlua::Lua::new();
        load(&lua).unwrap();
        lua.load(
            r#"
            iced.define('soft', function() return 'out' end)
            iced.define('card', function()
                return {view = function() error('component must not be rendered') end}
            end)
        "#,
        )
        .exec()
        .unwrap();
        for (source, message) in [
            ("iced.transition('missing')", "unknown transition"),
            ("iced.transition('card')", "must name a value definition"),
            ("iced.transition('text')", "must name a value definition"),
            ("iced.transition('soft')", "preset must be a table"),
            ("iced.transition(false)", "preset must be a table"),
            ("iced.transition({}, false)", "overrides must be a table"),
            ("iced.transition({enter = false})", "must be an event table"),
            (
                "iced.transition({easing = 'out', spring = 'bouncy'})",
                "not both",
            ),
            (
                "iced.transition({easing = 'out', enter = {spring = 'bouncy'}})",
                "not both",
            ),
            (
                "iced.transition({spring = 'bouncy', enter = {easing = 'in'}})",
                "not both",
            ),
            (
                "iced.transition({enter = {easing = 'out'}}, {enter = {spring = 'bouncy'}})",
                "not both",
            ),
        ] {
            let error = lua.load(source).exec().unwrap_err().to_string();
            assert!(error.contains(message), "{source}: {error}");
        }
    }

    #[test]
    fn log_propagates_tostring_errors() {
        let lua = mlua::Lua::new();
        load(&lua).unwrap();
        let error = lua
            .load(
                r#"log(setmetatable({}, {
            __tostring = function() error('cannot format value') end
        }))"#,
            )
            .exec()
            .unwrap_err();
        assert!(error.to_string().contains("cannot format value"));
    }

    #[test]
    fn log_remains_callable_and_formats_tables_on_request() {
        let lua = mlua::Lua::new();
        load(&lua).unwrap();
        lua.load("log('called')").exec().unwrap();
        lua.load("log.table({ name = 'test', nested = { 1, 2 } })")
            .exec()
            .unwrap();
    }

    #[test]
    fn iced_exposes_the_plugin_lookup() {
        let lua = mlua::Lua::new();
        load(&lua).unwrap();
        assert!(
            lua.load("return iced.plugins == riced.plugins")
                .eval::<bool>()
                .unwrap()
        );
        let proxy: mlua::Table = lua.load("return iced.plugins('themeagen')").eval().unwrap();
        assert_eq!(proxy.get::<String>("name").unwrap(), "themeagen");
    }

    #[test]
    fn popup_enum_tables_are_exposed() {
        let lua = mlua::Lua::new();
        load(&lua).unwrap();
        for (source, expected) in [
            ("iced.PopupAnchor.bottom_left", "bottom_left"),
            ("iced.PopupAnchor.none", "none"),
            ("iced.PopupGravity.top_right", "top_right"),
            ("iced.PopupConstraintAdjustment.all", "all"),
            ("iced.PopupConstraintAdjustment.resize_y", "resize_y"),
        ] {
            assert_eq!(lua.load(source).eval::<String>().unwrap(), expected);
        }
    }

    #[test]
    fn opacity_sets_the_alpha_channel() {
        let lua = mlua::Lua::new();
        load(&lua).unwrap();
        for (source, expected) in [
            ("iced.opacity('#112233', 100)", "#112233ff"),
            ("iced.opacity('#112233', 50)", "#11223380"),
            ("iced.opacity('#112233', 0)", "#11223300"),
            ("iced.opacity('#112233AA', 50)", "#11223380"),
        ] {
            assert_eq!(
                lua.load(format!("return {source}"))
                    .eval::<String>()
                    .unwrap(),
                expected
            );
        }
        for source in [
            "iced.opacity('red', 50)",
            "iced.opacity('#11223', 50)",
            "iced.opacity('#112233', -1)",
            "iced.opacity('#112233', 101)",
            "iced.opacity('#112233', '50')",
        ] {
            assert!(
                lua.load(format!("return {source}"))
                    .eval::<String>()
                    .is_err(),
                "{source}"
            );
        }
    }
}
