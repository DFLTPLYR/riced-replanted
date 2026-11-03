//! Lua API, split by function. Parts run in order: each adds to the globals the previous ones created.

const PARTS: &[(&str, &str)] = &[
    ("daemon", include_str!("daemon.lua")),
    ("plugins", include_str!("plugins.lua")),
    ("values", include_str!("values.lua")),
    ("elements", include_str!("elements.lua")),
    ("subscriptions", include_str!("subscriptions.lua")),
];

pub fn load(lua: &mlua::Lua) -> mlua::Result<()> {
    for (name, source) in PARTS {
        lua.load(*source)
            .set_name(format!("riced API: {name}"))
            .exec()?;
    }
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
