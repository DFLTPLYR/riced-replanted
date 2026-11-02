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
    Ok(())
}
