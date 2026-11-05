//! `riced.file`: small JSON/TOML files with load callbacks and save methods.

use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use mlua::{
    AnyUserData, Function, Lua, LuaSerdeExt, Table, UserData, UserDataFields, UserDataMethods,
    Value,
};
use serde_json::Value as Json;

use crate::runtime::keys;

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
                    serde_json::to_value(value)
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

struct File {
    path: PathBuf,
    kind: Kind,
    data: Value,
    error: Option<String>,
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
            Ok(())
        });
        fields.add_field_method_get("error", |_, file| Ok(file.error.clone()));
    }

    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method_mut("save", |lua, file, ()| Ok(file.save(lua)));
        methods.add_method_mut("write", |lua, file, value: Option<Value>| {
            if let Some(value) = value {
                file.data = value;
            }
            Ok(file.save(lua))
        });
    }
}

impl File {
    /// Writes `data`. Returns `true` or `false, error`; file errors are not fatal.
    fn save(&mut self, lua: &Lua) -> (bool, Option<String>) {
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

static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

fn write_atomic(path: &Path, contents: &str) -> std::io::Result<()> {
    let resolved;
    let path = match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            resolved = fs::canonicalize(path)?;
            resolved.as_path()
        }
        Ok(_) => path,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => path,
        Err(error) => return Err(error),
    };
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)?;
    }
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("data");
    let mode = match fs::metadata(path) {
        Ok(metadata) => Some(metadata.permissions()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(error),
    };
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let temporary = path.with_file_name(format!(
        ".{name}.tmp-{}-{suffix}-{}",
        std::process::id(),
        NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
    ));
    // create_new refuses to follow a symlink planted at the temporary path.
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&temporary)?;
    let result = (|| {
        file.write_all(contents.as_bytes())?;
        if let Some(mode) = mode {
            file.set_permissions(mode)?;
        }
        file.sync_all()?;
        fs::rename(&temporary, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
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
        let path = PathBuf::from(path);
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
        let file = File {
            path,
            kind,
            data,
            error,
        };
        let userdata: AnyUserData = lua.create_userdata(file)?;
        let error = userdata.borrow::<File>()?.error.clone();
        if let Some(error) = error {
            if let Some(callback) = on_error {
                callback.call::<()>((userdata.clone(), error))?;
            }
        } else if let Some(callback) = on_loaded {
            let data = userdata.borrow::<File>().unwrap().data.clone();
            callback.call::<()>((userdata.clone(), data))?;
        }
        Ok(userdata)
    })?;
    let riced: Table = lua.globals().get("riced")?;
    riced.set("file", constructor)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_callback_can_repair_file() {
        let (lua, dir) = setup("repair");
        fs::write(dir.join("data.json"), "invalid").unwrap();
        lua.load("riced.file({path='data.json', onError=function(file) assert(file:write({fixed=true})) end})")
            .exec().unwrap();
        let saved: Json =
            serde_json::from_str(&fs::read_to_string(dir.join("data.json")).unwrap()).unwrap();
        assert_eq!(saved["fixed"], true);
    }

    fn temp(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("riced-file-{}-{name}", std::process::id()))
    }

    fn setup(name: &str) -> (Lua, PathBuf) {
        let lua = Lua::new();
        crate::lua::load(&lua).unwrap();
        let dir = temp(name);
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        install(&lua, &dir).unwrap();
        (lua, dir)
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
            fs::read_to_string(dir.join("data.json")).unwrap(),
            "{\n  \"value\": 2\n}"
        );
        assert_eq!(result.borrow::<File>().unwrap().error, None);
    }

    #[test]
    fn loads_toml_and_creates_missing_files() {
        let (lua, dir) = setup("toml");
        fs::write(dir.join("data.toml"), "value = 3\n").unwrap();
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
            fs::read_to_string(dir.join("data.toml")).unwrap(),
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
            fs::read_to_string(dir.join("nested/missing.json")).unwrap(),
            "{}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn atomic_save_does_not_follow_the_old_predictable_temporary_symlink() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let (_, dir) = setup("symlink");
        let victim = dir.join("victim");
        fs::write(&victim, "untouched").unwrap();
        let temporary = dir.join(format!(".data.json.tmp-{}", std::process::id()));
        symlink(&victim, &temporary).unwrap();
        let path = dir.join("data.json");
        write_atomic(&path, "private").unwrap();
        assert_eq!(fs::read_to_string(victim).unwrap(), "untouched");
        assert_eq!(fs::read_to_string(&path).unwrap(), "private");
        assert_eq!(
            fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }

    #[cfg(unix)]
    #[test]
    fn save_keeps_existing_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let (lua, dir) = setup("mode");
        let path = dir.join("data.json");
        fs::write(&path, "{}").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        lua.load(r#"riced.file({path="data.json"}):write({value=1})"#)
            .exec()
            .unwrap();
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }

    #[test]
    fn atomic_save_preserves_destination_symlinks() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let (_, dir) = setup("destination-symlink");
        let target = dir.join("actual.json");
        fs::write(&target, "old").unwrap();
        fs::set_permissions(&target, fs::Permissions::from_mode(0o640)).unwrap();
        symlink("actual.json", dir.join("relative.json")).unwrap();
        let link = dir.join("link.json");
        symlink(dir.join("relative.json"), &link).unwrap();
        write_atomic(&link, "new").unwrap();
        assert_eq!(fs::read_to_string(&target).unwrap(), "new");
        assert!(
            fs::symlink_metadata(&link)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(
            fs::read_link(dir.join("relative.json")).unwrap(),
            Path::new("actual.json")
        );
        assert_eq!(
            fs::metadata(&target).unwrap().permissions().mode() & 0o777,
            0o640
        );

        fs::remove_file(&target).unwrap();
        assert!(write_atomic(&link, "lost").is_err());
        assert!(
            fs::symlink_metadata(&link)
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
