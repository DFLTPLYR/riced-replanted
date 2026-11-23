//! `riced.file`: small JSON/TOML files with load callbacks and save methods.

use std::{
    fs,
    path::{Path, PathBuf},
    time::SystemTime,
};

use mlua::{
    AnyUserData, Function, Lua, LuaSerdeExt, Table, UserData, UserDataFields, UserDataMethods,
    Value,
};
use riced_extension_sdk::write_atomic::write_atomic;
use serde_json::Value as Json;

use crate::runtime::{keys, shell_stamp};

#[derive(Clone, Copy)]
enum Kind {
    Json,
    Toml,
}

impl Kind {
    fn parse(value: &str) -> mlua::Result<Self> {
        match value.trim_start_matches('.').to_ascii_lowercase().as_str() {
            "json" => Ok(Self::Json),
            "toml" => Ok(Self::Toml),
            other => Err(mlua::Error::runtime(format!(
                "file type must be json or toml, not {other:?}"
            ))),
        }
    }

    fn from_path(path: &Path) -> mlua::Result<Self> {
        match path
            .extension()
            .and_then(|extension| extension.to_str())
            .map(str::to_ascii_lowercase)
            .as_deref()
        {
            Some("json") => Ok(Self::Json),
            Some("toml") => Ok(Self::Toml),
            _ => Err(mlua::Error::runtime(format!(
                "cannot infer file type from {}; pass type = 'json' or 'toml'",
                path.display()
            ))),
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Json => "json",
            Self::Toml => "toml",
        }
    }

    fn parse_contents(self, contents: &str) -> mlua::Result<Json> {
        match self {
            Self::Json => serde_json::from_str(contents)
                .map_err(|error| mlua::Error::runtime(format!("invalid JSON: {error}"))),
            Self::Toml => toml::from_str::<toml::Value>(contents)
                .map_err(|error| mlua::Error::runtime(format!("invalid TOML: {error}")))
                .and_then(|value| {
                    toml_json(value)
                        .map_err(|error| mlua::Error::runtime(format!("invalid TOML: {error}")))
                }),
        }
    }

    fn encode(self, value: &Json) -> mlua::Result<String> {
        match self {
            Self::Json => serde_json::to_string_pretty(value)
                .map_err(|error| mlua::Error::runtime(format!("cannot encode JSON: {error}"))),
            Self::Toml => toml::to_string_pretty(value)
                .map_err(|error| mlua::Error::runtime(format!("cannot encode TOML: {error}"))),
        }
    }
}

/// Converts TOML to the JSON-shaped value Lua sees. Datetimes become strings:
/// serde would otherwise turn them into a `$__toml_private_datetime` table
/// that cannot be written back as TOML.
fn toml_json(value: toml::Value) -> serde_json::Result<Json> {
    match value {
        toml::Value::Datetime(datetime) => Ok(Json::String(datetime.to_string())),
        toml::Value::Array(values) => values
            .into_iter()
            .map(toml_json)
            .collect::<serde_json::Result<Vec<_>>>()
            .map(Json::Array),
        toml::Value::Table(table) => table
            .into_iter()
            .map(|(key, value)| toml_json(value).map(|value| (key, value)))
            .collect::<serde_json::Result<serde_json::Map<String, Json>>>()
            .map(Json::Object),
        value => serde_json::to_value(value),
    }
}

struct File {
    path: PathBuf,
    kind: Kind,
    data: Value,
    error: Option<String>,
    // Construction failed to read/parse the original file; saving the empty
    // fallback table would overwrite the user's real contents.
    load_error: bool,
    // Stat at the last read; the poll re-reads only when this changed.
    stamp: Option<(Option<SystemTime>, u64)>,
}

impl UserData for File {
    fn add_fields<F: UserDataFields<Self>>(fields: &mut F) {
        fields.add_field_method_get("path", |_, file| {
            Ok(file.path.to_string_lossy().into_owned())
        });
        fields.add_field_method_get("type", |_, file| Ok(file.kind.name()));
        fields.add_field_method_get("data", |_, file| Ok(file.data.clone()));
        fields.add_field_method_set("data", |_, file, value: Value| {
            file.data = value;
            file.load_error = false;
            Ok(())
        });
        fields.add_field_method_get("error", |_, file| Ok(file.error.clone()));
    }

    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method_mut("save", |lua, file, ()| Ok(file.save(lua)));
        methods.add_method_mut("write", |lua, file, value: Option<Value>| {
            if let Some(value) = value {
                file.data = value;
                file.load_error = false;
            }
            Ok(file.save(lua))
        });
    }
}

impl File {
    /// Writes `data`. Returns `true` or `false, error`; file errors are not fatal.
    fn save(&mut self, lua: &Lua) -> (bool, Option<String>) {
        if self.load_error {
            let error = format!(
                "{} failed to load; replace its contents with file:write(value) before saving",
                self.path.display()
            );
            self.error = Some(error.clone());
            return (false, Some(error));
        }
        let json = match lua.from_value::<Json>(self.data.clone()) {
            Ok(json) => json,
            Err(error) => {
                let error = error.to_string();
                self.error = Some(error.clone());
                return (false, Some(error));
            }
        };
        let contents = match self.kind.encode(&json) {
            Ok(contents) => contents,
            Err(error) => {
                let error = error.to_string();
                self.error = Some(error.clone());
                return (false, Some(error));
            }
        };
        match write_atomic(&self.path, &contents) {
            Ok(()) => {
                // The poll must not treat our own write as an external change.
                self.stamp = shell_stamp(&self.path);
                self.error = None;
                (true, None)
            }
            Err(error) => {
                let error = format!("cannot write {}: {error}", self.path.display());
                self.error = Some(error.clone());
                (false, Some(error))
            }
        }
    }
}

/// Installs `riced.file` onto the `riced` table. Relative paths are resolved
/// against the profile directory.
pub fn install(lua: &Lua, base: &Path) -> mlua::Result<()> {
    let base = base.to_owned();
    let constructor = lua.create_function(move |lua, options: Table| {
        keys(&options, &["path", "type", "onLoaded", "onError"])?;
        let path = options.get::<String>("path")?;
        if path.is_empty() {
            return Err(mlua::Error::runtime("file path must not be empty"));
        }
        // Expand `~` so `~/notes.json` does not land in a literal tilde
        // directory under the profile.
        let path = if path == "~" || path.starts_with("~/") {
            let home = std::env::var_os("HOME")
                .filter(|home| !home.is_empty())
                .ok_or_else(|| mlua::Error::runtime("cannot expand file path: HOME is not set"))?;
            let home = PathBuf::from(home);
            if path == "~" {
                home
            } else {
                home.join(&path[2..])
            }
        } else {
            PathBuf::from(path)
        };
        let path = if path.is_absolute() {
            path
        } else {
            base.join(path)
        };
        let kind = match options.get::<Option<String>>("type")? {
            Some(kind) => Kind::parse(&kind)?,
            None => Kind::from_path(&path)?,
        };
        let on_loaded = options.get::<Option<Function>>("onLoaded")?;
        let on_error = options.get::<Option<Function>>("onError")?;

        let mut data = Value::Table(lua.create_table()?);
        let mut error = None;
        match fs::read_to_string(&path) {
            Ok(contents) => match kind.parse_contents(&contents) {
                Ok(json) => data = lua.to_value(&json)?,
                Err(parse) => error = Some(format!("{}: {parse}", path.display())),
            },
            Err(read) if read.kind() == std::io::ErrorKind::NotFound => {
                let empty = match kind {
                    Kind::Json => "{}",
                    Kind::Toml => "",
                };
                if let Err(create) = write_atomic(&path, empty) {
                    error = Some(format!("cannot create {}: {create}", path.display()));
                }
            }
            Err(read) => error = Some(format!("cannot read {}: {read}", path.display())),
        }
        let stamp = shell_stamp(&path);
        let file = File {
            path,
            kind,
            data,
            error: error.clone(),
            load_error: error.is_some(),
            stamp,
        };
        let userdata: AnyUserData = lua.create_userdata(file)?;
        // Track the mount for the poll; weak values drop entries once the
        // profile stops holding the file and its callbacks.
        let mounted = lua.create_table()?;
        mounted.raw_set("file", userdata.clone())?;
        if let Some(callback) = &on_loaded {
            mounted.raw_set("onLoaded", callback.clone())?;
        }
        if let Some(callback) = &on_error {
            mounted.raw_set("onError", callback.clone())?;
        }
        let next: i64 = lua.named_registry_value("riced.files_next")?;
        let instances: Table = lua.named_registry_value("riced.files")?;
        instances.raw_set(next + 1, mounted)?;
        lua.set_named_registry_value("riced.files_next", next + 1)?;
        let error = userdata.borrow::<File>()?.error.clone();
        if let Some(error) = error {
            if let Some(callback) = on_error {
                callback.call::<()>((userdata.clone(), error))?;
            }
        } else if let Some(callback) = on_loaded {
            let data = userdata.borrow::<File>()?.data.clone();
            callback.call::<()>((userdata.clone(), data))?;
        }
        Ok(userdata)
    })?;
    let riced: Table = lua.globals().get("riced")?;
    riced.set("file", constructor)?;
    let instances = lua.create_table()?;
    let weak = lua.create_table()?;
    weak.set("__mode", "v")?;
    instances.set_metatable(Some(weak))?;
    lua.set_named_registry_value("riced.files", instances)?;
    lua.set_named_registry_value("riced.files_next", 0i64)?;
    Ok(())
}

/// Reads and parses the current contents for the poll. A failure keeps the
/// mount's last good data and sets `load_error`, so save refuses until
/// `file:write` replaces the data.
fn reload(lua: &Lua, path: &Path, kind: Kind) -> Result<Value, String> {
    let contents = fs::read_to_string(path)
        .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
    let json = kind
        .parse_contents(&contents)
        .map_err(|error| format!("{}: {error}", path.display()))?;
    lua.to_value(&json).map_err(|error| error.to_string())
}

/// Re-reads live `riced.file` mounts whose stat changed since the last read
/// and re-runs `onLoaded` with the fresh data (or `onError`). Returns every
/// `onLoaded` return value; the caller runs them as tasks.
pub fn poll(lua: &Lua) -> mlua::Result<Vec<Value>> {
    let instances: Table = lua.named_registry_value("riced.files")?;
    let entries: Vec<Table> = instances
        .pairs::<mlua::Integer, Table>()
        .filter_map(|pair| pair.ok())
        .map(|(_, entry)| entry)
        .collect();
    let mut returns = Vec::new();
    for entry in entries {
        let file: AnyUserData = entry.get("file")?;
        let (path, kind, stamp) = {
            let file = file.borrow::<File>()?;
            (file.path.clone(), file.kind, file.stamp)
        };
        let fresh = shell_stamp(&path);
        if fresh == stamp {
            continue;
        }
        let outcome = reload(lua, &path, kind);
        let (loaded, failed) = {
            let mut file = file.borrow_mut::<File>()?;
            file.stamp = fresh;
            match outcome {
                Ok(data) => {
                    file.data = data.clone();
                    file.error = None;
                    file.load_error = false;
                    (Some(data), None)
                }
                Err(error) => {
                    file.error = Some(error.clone());
                    file.load_error = true;
                    (None, Some(error))
                }
            }
        };
        if let Some(data) = loaded {
            if let Some(callback) = entry.get::<Option<Function>>("onLoaded")? {
                let value = callback.call::<Value>((file.clone(), data))?;
                if !value.is_nil() {
                    returns.push(value);
                }
            }
        } else if let (Some(error), Some(callback)) =
            (failed, entry.get::<Option<Function>>("onError")?)
        {
            callback.call::<()>((file.clone(), error))?;
        }
    }
    Ok(returns)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn setup(name: &str) -> (Lua, PathBuf) {
        let lua = Lua::new();
        crate::lua::load(&lua).unwrap();
        let dir = std::env::temp_dir().join(format!("riced-file-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        install(&lua, &dir).unwrap();
        (lua, dir)
    }

    #[test]
    fn corrupt_load_refuses_save_until_data_is_replaced() {
        let (lua, dir) = setup("corrupt");
        let path = dir.join("data.json");
        std::fs::write(&path, "{broken").unwrap();
        lua.load(
            r#"
            local f = riced.file({path = "data.json", onError = function() end})
            local ok, err = f:save()
            assert(not ok)
            assert(err:find("failed to load"))
            "#,
        )
        .exec()
        .unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{broken");
        lua.load(
            r#"
            local f = riced.file({path = "data.json", onError = function() end})
            assert(not f:save())
            assert(f:write({ value = 2 }))
            assert(f:save())
            "#,
        )
        .exec()
        .unwrap();
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "{\n  \"value\": 2\n}"
        );
    }

    #[test]
    fn file_reloads_when_the_file_changes_externally() {
        let (lua, dir) = setup("sync");
        let path = dir.join("data.json");
        std::fs::write(&path, r#"{"value": 1}"#).unwrap();
        lua.load(
            r#"
            FIRES = 0
            f = riced.file({path = "data.json", onLoaded = function(file, data)
                FIRES = FIRES + 1
                LAST = data.value
            end})
            "#,
        )
        .exec()
        .unwrap();
        assert_eq!(
            lua.globals().get::<i32>("LAST").unwrap(),
            1,
            "constructor load"
        );

        // An unchanged file does not fire again.
        crate::file::poll(&lua).unwrap();
        assert_eq!(lua.globals().get::<i32>("FIRES").unwrap(), 1);

        std::fs::write(&path, r#"{"value": 2}"#).unwrap();
        poll(&lua).unwrap();
        assert_eq!(lua.globals().get::<i32>("FIRES").unwrap(), 2);
        assert_eq!(lua.globals().get::<i32>("LAST").unwrap(), 2);
        let value: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(value["value"], 2);
    }

    #[test]
    fn poll_after_save_does_not_reload() {
        let (lua, dir) = setup("save-poll");
        std::fs::write(dir.join("data.json"), r#"{"value": 1}"#).unwrap();
        lua.load(
            r#"
            LOADED = 0
            f = riced.file({path = "data.json", onLoaded = function() LOADED = LOADED + 1 end})
            "#,
        )
        .exec()
        .unwrap();
        lua.load("f.data.value = 2; assert(f:save())")
            .exec()
            .unwrap();
        poll(&lua).unwrap();
        assert_eq!(lua.globals().get::<i32>("LOADED").unwrap(), 1);
        assert_eq!(lua.load("return f.data.value").eval::<i32>().unwrap(), 2);
    }

    #[test]
    fn a_corrupt_external_change_keeps_data_and_refuses_save() {
        let (lua, dir) = setup("corrupt-sync");
        let path = dir.join("data.json");
        std::fs::write(&path, r#"{"value": 1}"#).unwrap();
        lua.load(
            r#"
            ERRORS = 0
            f = riced.file({path = "data.json", onError = function() ERRORS = ERRORS + 1 end})
            "#,
        )
        .exec()
        .unwrap();
        std::fs::write(&path, "{broken").unwrap();
        poll(&lua).unwrap();
        assert_eq!(lua.globals().get::<i32>("ERRORS").unwrap(), 1);
        // The last good data survives, save refuses, and write clears it.
        let ok: (bool, Option<String>) = lua.load("return f:save()").eval().unwrap();
        assert!(!ok.0);
        assert_eq!(lua.load("return f.data.value").eval::<i32>().unwrap(), 1);
        lua.load("f:write({value = 3})").exec().unwrap();
        let value: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(value["value"], 3);
    }

    #[test]
    fn a_deleted_file_errors_once_and_recovers() {
        let (lua, dir) = setup("delete-sync");
        let path = dir.join("data.json");
        std::fs::write(&path, r#"{"value": 1}"#).unwrap();
        lua.load(
            r#"
            ERRORS = 0
            LOADED = 0
            f = riced.file({
                path = "data.json",
                onLoaded = function() LOADED = LOADED + 1 end,
                onError = function() ERRORS = ERRORS + 1 end,
            })
            "#,
        )
        .exec()
        .unwrap();
        std::fs::remove_file(&path).unwrap();
        poll(&lua).unwrap();
        assert_eq!(lua.globals().get::<i32>("ERRORS").unwrap(), 1);
        poll(&lua).unwrap();
        assert_eq!(
            lua.globals().get::<i32>("ERRORS").unwrap(),
            1,
            "errors once"
        );
        std::fs::write(&path, r#"{"value": 7}"#).unwrap();
        poll(&lua).unwrap();
        assert_eq!(lua.globals().get::<i32>("LOADED").unwrap(), 2);
    }

    static HOME_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn home_paths_are_expanded_and_require_home() {
        let _guard = HOME_LOCK.lock().unwrap();
        let (lua, _) = setup("home");
        let home = std::env::temp_dir().join(format!("riced-home-{}", std::process::id()));
        std::fs::create_dir_all(&home).unwrap();
        let previous = std::env::var_os("HOME");
        // SAFETY: single-threaded under the HOME_LOCK guard, then restored.
        unsafe { std::env::set_var("HOME", &home) };

        lua.load(
            r#"
            local f = riced.file({path = "~/notes.json", type = "json"})
            f:write({ value = 1 })
            "#,
        )
        .exec()
        .unwrap();
        assert_eq!(
            std::fs::read_to_string(home.join("notes.json")).unwrap(),
            "{\n  \"value\": 1\n}"
        );

        // SAFETY: single-threaded under the HOME_LOCK guard, then restored.
        unsafe { std::env::remove_var("HOME") };
        let result: Result<AnyUserData, _> = lua
            .load(r#"return riced.file({path = "~/notes.json", type = "json"})"#)
            .eval();
        let error = result.unwrap_err().to_string();
        assert!(error.contains("HOME is not set"), "{error}");

        // SAFETY: single-threaded under the HOME_LOCK guard, restoring the
        // value captured above.
        match previous {
            Some(home) => unsafe { std::env::set_var("HOME", home) },
            None => unsafe { std::env::remove_var("HOME") },
        }
        let _ = std::fs::remove_dir_all(home);
    }

    #[test]
    fn error_callback_can_repair_file() {
        let (lua, dir) = setup("repair");
        std::fs::write(dir.join("data.json"), "invalid").unwrap();
        lua.load("riced.file({path='data.json', onError=function(file) assert(file:write({fixed=true})) end})")
            .exec().unwrap();
        let saved: Json =
            serde_json::from_str(&std::fs::read_to_string(dir.join("data.json")).unwrap()).unwrap();
        assert_eq!(saved["fixed"], true);
    }

    #[test]
    fn loads_and_saves_json() {
        let (lua, dir) = setup("json");
        let result: AnyUserData = lua
            .load(
                r#"
                local f = riced.file({
                    path = "data.json",
                    type = "json",
                    onLoaded = function(file, data) loaded = data.value end,
                })
                assert(f.path == "PATH/data.json")
                assert(f.type == "json")
                assert(loaded == nil)
                f.data = { value = 2 }
                assert(f:save())
                return f
            "#
                .replace("PATH", dir.to_str().unwrap()),
            )
            .eval()
            .unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.join("data.json")).unwrap(),
            "{\n  \"value\": 2\n}"
        );
        assert_eq!(result.borrow::<File>().unwrap().error, None);
    }

    #[test]
    fn loads_toml_and_creates_missing_files() {
        let (lua, dir) = setup("toml");
        std::fs::write(dir.join("data.toml"), "value = 3\n").unwrap();
        let result: AnyUserData = lua
            .load(
                r#"
                local failed = false
                local f = riced.file({
                    path = "data.toml",
                    onLoaded = function(file, data) loaded = data.value end,
                    onError = function() failed = true end,
                })
                assert(f.type == "toml")
                assert(loaded == 3)
                assert(not failed)
                assert(f:write({ value = 4 }))
                assert(f.data.value == 4)
                assert(f:write())
                return f
            "#,
            )
            .eval()
            .unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.join("data.toml")).unwrap(),
            "value = 4\n"
        );
        assert_eq!(result.borrow::<File>().unwrap().error, None);

        let missing: AnyUserData = lua
            .load(
                r#"
                local loaded = false
                local failed = false
                local f = riced.file({
                    path = "nested/missing.json",
                    type = "json",
                    onLoaded = function() loaded = true end,
                    onError = function() failed = true end,
                })
                assert(f.data ~= nil)
                assert(loaded and not failed)
                return f
            "#,
            )
            .eval()
            .unwrap();
        assert!(missing.borrow::<File>().unwrap().error.is_none());
        assert_eq!(
            std::fs::read_to_string(dir.join("nested/missing.json")).unwrap(),
            "{}"
        );
    }

    #[test]
    fn toml_dates_load_as_strings_and_save_without_magic_keys() {
        let (lua, dir) = setup("toml-datetime");
        std::fs::write(
            dir.join("data.toml"),
            "created = 2024-01-02T03:04:05Z\nupdated = 2024-01-02\n",
        )
        .unwrap();
        lua.load(
            r#"
            local loaded
            local f = riced.file({
                path = "data.toml",
                onLoaded = function(_, data) loaded = data end,
            })
            assert(loaded.created == "2024-01-02T03:04:05Z")
            assert(loaded.updated == "2024-01-02")
            f.data.extra = true
            assert(f:save())
            "#,
        )
        .exec()
        .unwrap();
        let saved = std::fs::read_to_string(dir.join("data.toml")).unwrap();
        assert!(!saved.contains("$__toml_private_datetime"));
        assert!(saved.contains("created = \"2024-01-02T03:04:05Z\""));
        assert!(saved.contains("updated = \"2024-01-02\""));
    }

    #[cfg(unix)]
    #[test]
    fn atomic_save_does_not_follow_the_old_predictable_temporary_symlink() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let (_, dir) = setup("symlink");
        let victim = dir.join("victim");
        std::fs::write(&victim, "untouched").unwrap();
        let temporary = dir.join(format!(".data.json.tmp-{}", std::process::id()));
        symlink(&victim, &temporary).unwrap();
        let path = dir.join("data.json");
        write_atomic(&path, "private").unwrap();
        assert_eq!(std::fs::read_to_string(&victim).unwrap(), "untouched");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "private");
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }

    #[cfg(unix)]
    #[test]
    fn save_keeps_existing_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let (lua, dir) = setup("mode");
        let path = dir.join("data.json");
        std::fs::write(&path, "{}").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        lua.load(r#"riced.file({path="data.json"}):write({value=1})"#)
            .exec()
            .unwrap();
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }

    #[test]
    fn atomic_save_preserves_destination_symlinks() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let (_, dir) = setup("destination-symlink");
        let target = dir.join("actual.json");
        std::fs::write(&target, "old").unwrap();
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o640)).unwrap();
        symlink("actual.json", dir.join("relative.json")).unwrap();
        let link = dir.join("link.json");
        symlink(dir.join("relative.json"), &link).unwrap();
        write_atomic(&link, "new").unwrap();
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "new");
        assert!(
            std::fs::symlink_metadata(&link)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(
            std::fs::read_link(dir.join("relative.json")).unwrap(),
            Path::new("actual.json")
        );
        assert_eq!(
            std::fs::metadata(&target).unwrap().permissions().mode() & 0o777,
            0o640
        );

        std::fs::remove_file(&target).unwrap();
        assert!(write_atomic(&link, "lost").is_err());
        assert!(
            std::fs::symlink_metadata(&link)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert!(!target.exists());
    }

    #[test]
    fn save_reports_values_toml_cannot_encode() {
        let (lua, _dir) = setup("invalid");
        let (ok, error): (bool, Option<String>) = lua
            .load(r#"return riced.file({path="x.toml"}):save()"#)
            .eval()
            .unwrap();
        assert!(ok);
        assert!(error.is_none());
        let (ok, error): (bool, Option<String>) = lua
            .load(r#"return riced.file({path="y.toml"}):write({ value = function() end })"#)
            .eval()
            .unwrap();
        assert!(!ok);
        assert!(error.is_some());
    }
}
