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
