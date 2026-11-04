use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
};

use mlua::{FromLua, Function, Lua, Table, Value};
use riced_extension_sdk::log;

#[derive(Clone)]
pub struct Loaded {
    path: PathBuf,
    source: String,
    warned: bool,
}

#[derive(Clone, Default)]
pub struct Snapshot {
    deps: HashMap<String, Vec<String>>,
    revisions: HashMap<String, u64>,
    package: HashMap<String, Value>,
    tables: Vec<(Table, Table)>,
}

#[derive(Default)]
pub struct Modules {
    loaded: HashMap<String, Loaded>,
    deps: HashMap<String, Vec<String>>,
    revisions: HashMap<String, u64>,
}

fn loaded_table(lua: &Lua) -> mlua::Result<Table> {
    let package: Table = lua.globals().get("package")?;
    package.get("loaded")
}

fn names(table: &Table) -> mlua::Result<HashSet<String>> {
    table
        .clone()
        .pairs::<String, Value>()
        .map(|pair| pair.map(|(name, _)| name))
        .collect()
}

impl Modules {
    pub fn install(lua: &Lua) -> mlua::Result<()> {
        lua.load(
            r#"
            local original = require
            local stack = {}
            __riced_dependencies = {}
            function require(name)
                local parent = stack[#stack]
                if parent then
                    local deps = __riced_dependencies[parent] or {}
                    deps[name] = true
                    __riced_dependencies[parent] = deps
                end
                stack[#stack + 1] = name
                local result = table.pack(pcall(original, name))
                stack[#stack] = nil
                if not result[1] then error(result[2], 2) end
                return table.unpack(result, 2, result.n)
            end
        "#,
        )
        .exec()
    }

    pub fn watch_all(&mut self, lua: &Lua) -> mlua::Result<()> {
        for name in names(&loaded_table(lua)?)? {
            // Standard/preloaded modules may not have a Lua source file.
            let _ = self.watch(lua, &name);
        }
        if let Some(deps) = lua.globals().get::<Option<Table>>("__riced_dependencies")? {
            for pair in deps.pairs::<String, Table>() {
                let (name, children) = pair?;
                self.deps
                    .insert(name, names(&children)?.into_iter().collect());
            }
        }
        Ok(())
    }

    // Preserve table identity so locals captured by shell.lua see new module methods.
    pub fn refresh(
        &mut self,
        lua: &Lua,
        changed: &[String],
        snapshot: &Snapshot,
    ) -> mlua::Result<()> {
        let require: Function = lua.globals().get("require")?;
        let mut replacements = Vec::new();
        for name in changed {
            let value: Value = require.call(name.as_str())?;
            if let (Some(Value::Table(old)), Value::Table(new)) =
                (snapshot.package.get(name), value)
            {
                replacements.push((name, old.clone(), new));
            }
        }
        let loaded = loaded_table(lua)?;
        let exports = replacements
            .iter()
            .map(|(_, old, new)| (new.to_pointer(), old.clone()))
            .collect::<HashMap<_, _>>();
        let mut seen = snapshot
            .package
            .values()
            .map(Value::to_pointer)
            .collect::<HashSet<_>>();
        seen.insert(lua.globals().to_pointer());
        for (name, old, new) in replacements {
            old.clear()?;
            for pair in new.pairs::<Value, Value>() {
                let (key, value) = pair?;
                old.raw_set(
                    rebind_exports(lua, key, &exports, &mut seen)?,
                    rebind_exports(lua, value, &exports, &mut seen)?,
                )?;
            }
            let meta = new
                .metatable()
                .map(|table| {
                    rebind_exports(lua, Value::Table(table), &exports, &mut seen)
                        .and_then(|value| Table::from_lua(value, lua))
                })
                .transpose()?;
            old.set_metatable(meta)?;
            loaded.set(name.as_str(), old)?;
        }
        self.watch_all(lua)
    }

    pub fn revision(&self, module: &str) -> u64 {
        self.revisions.get(module).copied().unwrap_or(0)
    }

    pub fn require(&mut self, lua: &Lua, name: &str) -> mlua::Result<Table> {
        let loaded = loaded_table(lua)?;
        if let Some(table) = loaded.get::<Option<Table>>(name)? {
            self.watch(lua, name)?;
            return Ok(table);
        }
        let before = names(&loaded)?;
        let require: Function = lua.globals().get("require")?;
        let table: Table = require.call(name)?;
        let deps = names(&loaded)?
            .difference(&before)
            .cloned()
            .collect::<Vec<_>>();
        self.watch(lua, name)?;
        for dep in &deps {
            // Best effort: compiled modules and preloaded names have no file to watch.
            let _ = self.watch(lua, dep);
        }
        self.deps.insert(name.to_owned(), deps);
        Ok(table)
    }

    fn watch(&mut self, lua: &Lua, name: &str) -> mlua::Result<()> {
        if self.loaded.contains_key(name) {
            return Ok(());
        }
        let package: Table = lua.globals().get("package")?;
        let searchpath: Function = package.get("searchpath")?;
        let path: Option<String> = searchpath.call((name, package.get::<String>("path")?))?;
        let Some(path) = path else {
            return Err(mlua::Error::runtime(format!(
                "cannot find module {name} in package.path"
            )));
        };
        let source = std::fs::read_to_string(&path)
            .map_err(|error| mlua::Error::runtime(format!("cannot read {path}: {error}")))?;
        self.loaded.insert(
            name.to_owned(),
            Loaded {
                path: path.into(),
                source,
                warned: false,
            },
        );
        Ok(())
    }

    // Polls watched module files and evicts changed modules plus their dependents.
    // A changed file becomes the new baseline immediately, so a broken save is not retried.
    pub fn poll(&mut self, lua: &Lua) -> mlua::Result<Vec<String>> {
        let mut changed = Vec::new();
        for (name, loaded) in &mut self.loaded {
            match std::fs::read_to_string(&loaded.path) {
                Ok(source) => {
                    loaded.warned = false;
                    if source != loaded.source {
                        loaded.source = source;
                        changed.push(name.clone());
                    }
                }
                Err(error) => {
                    if !loaded.warned {
                        loaded.warned = true;
                        log::warn!(target: "module.watch", "cannot watch {}: {error}", loaded.path.display());
                    }
                }
            }
        }
        if changed.is_empty() {
            return Ok(changed);
        }
        changed.sort();
        let mut dirty = changed.iter().cloned().collect::<HashSet<_>>();
        loop {
            let mut grew = false;
            for (module, deps) in &self.deps {
                if !dirty.contains(module) && deps.iter().any(|dep| dirty.contains(dep)) {
                    grew |= dirty.insert(module.clone());
                }
            }
            if !grew {
                break;
            }
        }
        let loaded = loaded_table(lua)?;
        for name in &dirty {
            loaded.set(name.as_str(), Value::Nil)?;
            *self.revisions.entry(name.clone()).or_default() += 1;
        }
        let mut dirty = dirty.into_iter().collect::<Vec<_>>();
        dirty.sort();
        Ok(dirty)
    }

    pub fn snapshot(&self, lua: &Lua) -> mlua::Result<Snapshot> {
        let package: HashMap<String, Value> = loaded_table(lua)?
            .clone()
            .pairs::<String, Value>()
            .collect::<mlua::Result<_>>()?;
        let mut tables = Vec::new();
        for name in self.loaded.keys() {
            if let Some(Value::Table(table)) = package.get(name) {
                let copy = lua.create_table()?;
                for pair in table.clone().pairs::<Value, Value>() {
                    let (key, value) = pair?;
                    copy.raw_set(key, value)?;
                }
                copy.set_metatable(table.metatable())?;
                tables.push((table.clone(), copy));
            }
        }
        Ok(Snapshot {
            deps: self.deps.clone(),
            revisions: self.revisions.clone(),
            package,
            tables,
        })
    }

    pub fn restore(&mut self, lua: &Lua, snapshot: Snapshot) -> mlua::Result<()> {
        for (table, copy) in snapshot.tables {
            table.clear()?;
            for pair in copy.clone().pairs::<Value, Value>() {
                let (key, value) = pair?;
                table.raw_set(key, value)?;
            }
            table.set_metatable(copy.metatable())?;
        }
        let loaded = loaded_table(lua)?;
        for name in names(&loaded)? {
            if !snapshot.package.contains_key(&name) {
                loaded.set(name, Value::Nil)?;
            }
        }
        for (name, value) in snapshot.package {
            loaded.set(name.as_str(), value)?;
        }
        self.deps = snapshot.deps;
        self.revisions = snapshot.revisions;
        let deps = lua.create_table()?;
        for (name, children) in &self.deps {
            let entry = lua.create_table()?;
            for child in children {
                entry.set(child.as_str(), true)?;
            }
            deps.set(name.as_str(), entry)?;
        }
        lua.globals().set("__riced_dependencies", deps)?;
        Ok(())
    }
}

// Lua functions capture their module's new export table; point those references
// at the canonical table retained by callers, without exposing the debug library.
fn rebind_exports(
    lua: &Lua,
    value: Value,
    exports: &HashMap<*const std::ffi::c_void, Table>,
    seen: &mut HashSet<*const std::ffi::c_void>,
) -> mlua::Result<Value> {
    if let Some(table) = exports.get(&value.to_pointer()) {
        return Ok(Value::Table(table.clone()));
    }
    if !matches!(value, Value::Table(_) | Value::Function(_)) || !seen.insert(value.to_pointer()) {
        return Ok(value);
    }
    match &value {
        Value::Table(table) => {
            let pairs = table
                .pairs::<Value, Value>()
                .collect::<mlua::Result<Vec<_>>>()?;
            for (key, value) in pairs {
                let new_key = rebind_exports(lua, key.clone(), exports, seen)?;
                let value = rebind_exports(lua, value, exports, seen)?;
                if key != new_key {
                    table.raw_set(key, Value::Nil)?;
                }
                table.raw_set(new_key, value)?;
            }
            if let Some(meta) = table.metatable() {
                let meta = rebind_exports(lua, Value::Table(meta), exports, seen)?;
                table.set_metatable(Some(Table::from_lua(meta, lua)?))?;
            }
        }
        Value::Function(function) if function.info().what != "C" => {
            for index in 1..=i32::from(function.info().num_upvalues) {
                // exec_raw locks/restores the stack; index is a valid Lua upvalue.
                let (name, captured): (String, Value) = unsafe {
                    lua.exec_raw(function, |state| {
                        let name = mlua::ffi::lua_getupvalue(state, 1, index);
                        mlua::ffi::lua_pushstring(state, name);
                        mlua::ffi::lua_remove(state, 1);
                        mlua::ffi::lua_insert(state, 1);
                    })?
                };
                if name == "_ENV" {
                    continue;
                }
                let replacement = rebind_exports(lua, captured.clone(), exports, seen)?;
                if replacement != captured {
                    // Both arguments belong to this Lua; setupvalue consumes the value.
                    unsafe {
                        lua.exec_raw::<()>((function, replacement), |state| {
                            mlua::ffi::lua_setupvalue(state, 1, index);
                            mlua::ffi::lua_pop(state, 1);
                        })?;
                    }
                }
            }
        }
        _ => {}
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Profile(PathBuf);

    impl Profile {
        fn new(files: &[(&str, &str)]) -> Self {
            static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let root = std::env::temp_dir().join(format!(
                "riced-modules-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            ));
            for (name, source) in files {
                let path = root.join(name);
                std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                std::fs::write(path, source).unwrap();
            }
            Self(root)
        }

        fn write(&self, name: &str, source: &str) {
            std::fs::write(self.0.join(name), source).unwrap();
        }

        fn lua(&self) -> Lua {
            let lua = Lua::new();
            let package: Table = lua.globals().get("package").unwrap();
            package
                .set(
                    "path",
                    format!("{}/?.lua;{}/?/init.lua", self.0.display(), self.0.display()),
                )
                .unwrap();
            lua
        }
    }

    impl Drop for Profile {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn changed_module_file_bumps_only_its_revision() {
        let profile = Profile::new(&[("a.lua", "return {}"), ("b.lua", "return {}")]);
        let lua = profile.lua();
        let mut modules = Modules::default();
        modules.require(&lua, "a").unwrap();
        modules.require(&lua, "b").unwrap();

        profile.write("a.lua", "return {changed = true}");
        let changed = modules.poll(&lua).unwrap();

        assert_eq!(changed, ["a"]);
        assert_eq!(modules.revision("a"), 1);
        assert_eq!(modules.revision("b"), 0);
    }

    #[test]
    fn changed_helper_invalidates_its_dependent_module() {
        let profile = Profile::new(&[
            (
                "component.lua",
                "local helper = require('helper'); return {v = helper.v}",
            ),
            ("helper.lua", "return {v = 1}"),
            ("other.lua", "return {}"),
        ]);
        let lua = profile.lua();
        let mut modules = Modules::default();
        modules.require(&lua, "component").unwrap();
        modules.require(&lua, "other").unwrap();

        profile.write("helper.lua", "return {v = 2}");
        let changed = modules.poll(&lua).unwrap();

        assert_eq!(changed, ["component", "helper"]);
        assert_eq!(modules.revision("component"), 1);
        assert_eq!(modules.revision("other"), 0);
        let component: Table = modules.require(&lua, "component").unwrap();
        assert_eq!(component.get::<u32>("v").unwrap(), 2);
    }

    #[test]
    fn reload_rebinds_self_references_to_the_retained_export_table() {
        let profile = Profile::new(&[(
            "component.lua",
            "local m = {}; function m.self() return m end; return m",
        )]);
        let lua = profile.lua();
        let mut modules = Modules::default();
        let old = modules.require(&lua, "component").unwrap();
        let snapshot = modules.snapshot(&lua).unwrap();
        profile.write(
            "component.lua",
            "local m = {}; function m.self() return m end; function m.marker() return 'new' end; return m",
        );
        let changed = modules.poll(&lua).unwrap();
        modules.refresh(&lua, &changed, &snapshot).unwrap();

        let new = modules.require(&lua, "component").unwrap();
        assert_eq!(old.to_pointer(), new.to_pointer());
        let captured: Table = new.get::<Function>("self").unwrap().call(()).unwrap();
        assert_eq!(captured.to_pointer(), old.to_pointer());
        assert_eq!(
            new.get::<Function>("marker")
                .unwrap()
                .call::<String>(())
                .unwrap(),
            "new"
        );
    }

    #[test]
    fn snapshot_restores_modules_after_a_failed_reload() {
        let profile = Profile::new(&[("component.lua", "return {v = 1}")]);
        let lua = profile.lua();
        let mut modules = Modules::default();
        modules.require(&lua, "component").unwrap();
        let snapshot = modules.snapshot(&lua).unwrap();

        profile.write("component.lua", "error('broken')");
        modules.poll(&lua).unwrap();
        assert_eq!(modules.revision("component"), 1);
        assert!(modules.require(&lua, "component").is_err());

        modules.restore(&lua, snapshot).unwrap();
        assert_eq!(modules.revision("component"), 0);
        let component: Table = modules.require(&lua, "component").unwrap();
        assert_eq!(component.get::<u32>("v").unwrap(), 1);
        assert!(modules.poll(&lua).unwrap().is_empty());
    }

    #[test]
    fn a_missing_module_does_not_hide_another_edit() {
        let profile = Profile::new(&[("a.lua", "return {}"), ("b.lua", "return {}")]);
        let lua = profile.lua();
        let mut modules = Modules::default();
        modules.require(&lua, "a").unwrap();
        modules.require(&lua, "b").unwrap();

        profile.write("a.lua", "return {changed = true}");
        std::fs::remove_file(profile.0.join("b.lua")).unwrap();
        assert_eq!(modules.poll(&lua).unwrap(), ["a"]);

        profile.write("b.lua", "return {}");
        assert!(modules.poll(&lua).unwrap().is_empty());
    }

    #[test]
    fn missing_module_reports_the_failing_path() {
        let profile = Profile::new(&[("present.lua", "return {}")]);
        let lua = profile.lua();
        let mut modules = Modules::default();
        assert!(
            modules
                .require(&lua, "missing")
                .unwrap_err()
                .to_string()
                .contains("missing")
        );
    }
}
