//! Lua API, split by function. Parts run in order: each adds to the globals the previous ones created.

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
