use std::{
    collections::{BTreeSet, HashMap, HashSet},
    env,
    hash::{Hash, Hasher},
    path::{Component, Path, PathBuf},
    time::{Duration, Instant},
};

use freedesktop_entry_parser::Entry;
use mlua::{Lua, Table};

#[derive(Hash, PartialEq, Eq)]
struct Query {
    name: String,
    theme: String,
    size: u32,
    scale: u32,
    path: Option<PathBuf>,
}

struct Directory {
    path: String,
    min: u32,
    max: u32,
    scale: u32,
}

struct Theme {
    directories: Vec<Directory>,
    inherits: Vec<String>,
}

fn relative(path: &str) -> bool {
    !path.is_empty()
        && !path.contains('\0')
        && Path::new(path)
            .components()
            .all(|part| matches!(part, Component::Normal(_)))
}

fn name(value: &str) -> bool {
    relative(value) && !value.contains('/')
}

impl Theme {
    fn load(roots: &[PathBuf], theme: &str) -> Option<Self> {
        let entry = roots
            .iter()
            .find_map(|root| Entry::parse_file(root.join(theme).join("index.theme")).ok())?;
        let header = entry.section("Icon Theme");
        let directories = header
            .attr("Directories")
            .or_else(|| header.attr("ScaledDirectories"))?;
        let directories = directories
            .split(',')
            .chain(header.attr("ScaledDirectories").unwrap_or("").split(','))
            .map(str::trim)
            .filter(|path| relative(path))
            .filter_map(|path| {
                let section = entry.section(path);
                let number = |key, default| {
                    section
                        .attr(key)
                        .map_or(Some(default), |v| v.parse::<u32>().ok())
                        .filter(|n| *n <= 65535)
                };
                let size = number("Size", 0).filter(|n| *n > 0)?;
                let scale = number("Scale", 1).filter(|n| *n > 0)?;
                let (min, max) = match section.attr("Type").unwrap_or("Threshold") {
                    "Fixed" => (size, size),
                    "Scalable" => (number("MinSize", size)?, number("MaxSize", size)?),
                    "Threshold" => {
                        let threshold = number("Threshold", 2)?;
                        (
                            size.saturating_sub(threshold),
                            size.saturating_add(threshold).min(65535),
                        )
                    }
                    _ => return None,
                };
                (min <= max).then(|| Directory {
                    path: path.to_owned(),
                    min,
                    max,
                    scale,
                })
            })
            .collect();
        let inherits = header
            .attr("Inherits")
            .unwrap_or("")
            .split(',')
            .map(str::trim)
            .filter(|parent| name(parent))
            .map(str::to_owned)
            .collect();
        Some(Self {
            directories,
            inherits,
        })
    }

    fn find(&self, roots: &[PathBuf], query: &Query, theme: &str) -> Option<PathBuf> {
        let mut directories = self.directories.iter().collect::<Vec<_>>();
        directories.sort_by_key(|dir| {
            let exact = dir.scale == query.scale && (dir.min..=dir.max).contains(&query.size);
            let pixels = query.size * query.scale;
            let distance = pixels.abs_diff(pixels.clamp(dir.min * dir.scale, dir.max * dir.scale));
            (!exact, distance)
        });
        directories.into_iter().find_map(|dir| {
            roots
                .iter()
                .find_map(|root| icon_file(&root.join(theme).join(&dir.path), &query.name))
        })
    }
}

fn icon_file(directory: &Path, name: &str) -> Option<PathBuf> {
    if matches!(
        Path::new(name).extension().and_then(|ext| ext.to_str()),
        Some("png" | "svg")
    ) {
        let path = directory.join(name);
        return path.is_file().then_some(path);
    }
    ["png", "svg"]
        .into_iter()
        .map(|ext| directory.join(format!("{name}.{ext}")))
        .find(|path| path.is_file())
}

fn search_paths() -> (Vec<PathBuf>, Vec<PathBuf>) {
    let home = env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute());
    let data_home = env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| home.as_ref().map(|home| home.join(".local/share")));
    let data_dirs = env::var_os("XDG_DATA_DIRS")
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "/usr/local/share:/usr/share".into());
    let data = data_home
        .into_iter()
        .chain(env::split_paths(&data_dirs).filter(|path| path.is_absolute()))
        .collect::<Vec<_>>();
    let mut roots = home
        .into_iter()
        .map(|home| home.join(".icons"))
        .chain(data.iter().map(|path| path.join("icons")))
        .collect::<Vec<_>>();
    let mut seen = HashSet::new();
    roots.retain(|path| seen.insert(path.clone()));
    let pixmaps = data.into_iter().map(|path| path.join("pixmaps")).collect();
    (roots, pixmaps)
}

#[derive(Default)]
struct Resolver {
    roots: Vec<PathBuf>,
    pixmaps: Vec<PathBuf>,
    themes: HashMap<String, Option<Theme>>,
    cache: HashMap<Query, Option<PathBuf>>,
    checked: Option<Instant>,
}

/// How long a resolved icon path stays cached before the cache is re-checked
/// against the filesystem.
const REFRESH: Duration = Duration::from_secs(1);

impl Resolver {
    fn theme(
        &mut self,
        query: &Query,
        theme: &str,
        visited: &mut HashSet<String>,
    ) -> Option<PathBuf> {
        if !visited.insert(theme.to_owned()) {
            return None;
        }
        let theme_data = self
            .themes
            .entry(theme.to_owned())
            .or_insert_with(|| Theme::load(&self.roots, theme))
            .as_ref()?;
        if let Some(path) = theme_data.find(&self.roots, query, theme) {
            return Some(path);
        }
        let parents = theme_data.inherits.clone();
        parents
            .into_iter()
            .find_map(|parent| self.theme(query, &parent, visited))
    }

    fn resolve(&mut self, query: Query) -> Option<PathBuf> {
        // Installed themes and icons are re-checked once a second so new files
        // appear without a shell reload, including newly installed themes.
        let stale = self
            .checked
            .is_none_or(|checked| checked.elapsed() >= REFRESH);
        if stale {
            self.cache.clear();
            self.themes.clear();
            self.checked = Some(Instant::now());
        }
        if let Some(path) = self.cache.get(&query) {
            return path.clone();
        }
        let path = if Path::new(&query.name).is_absolute() {
            let path = PathBuf::from(&query.name);
            path.is_file().then_some(path)
        } else {
            query
                .path
                .as_ref()
                .and_then(|path| icon_file(path, &query.name))
                .or_else(|| self.theme(&query, &query.theme, &mut HashSet::new()))
                .or_else(|| self.theme(&query, "hicolor", &mut HashSet::new()))
                .or_else(|| {
                    self.roots
                        .iter()
                        .chain(&self.pixmaps)
                        .find_map(|root| icon_file(root, &query.name))
                })
        };
        // shortcut: icon lookups re-check the filesystem once a second, add a watcher if instant visibility is needed.
        if self.cache.len() >= 1024 {
            self.cache.clear();
            self.themes.clear();
        }
        self.cache.insert(query, path.clone());
        path
    }
}

fn query(iced: &Table, icon: String, options: Option<Table>) -> mlua::Result<(Query, bool)> {
    let theme = iced.get::<String>("icon_theme")?;
    if !name(&theme) {
        return Err(mlua::Error::runtime(
            "iced.icon_theme must be a nonempty theme name",
        ));
    }
    if icon.contains('\0') || (!name(&icon) && !Path::new(&icon).is_absolute()) {
        return Err(mlua::Error::runtime(
            "icon must be a nonempty name or an absolute file path",
        ));
    }
    let mut query = Query {
        name: icon,
        theme,
        size: 24,
        scale: 1,
        path: None,
    };
    let mut thumbnail = false;
    if let Some(options) = options {
        crate::runtime::keys(&options, &["size", "scale", "path", "thumbnail"])?;
        let integer = |key, default, max| -> mlua::Result<u32> {
            let value = options.get::<Option<f64>>(key)?.unwrap_or(default as f64);
            if !(1.0..=max as f64).contains(&value) || value.fract() != 0.0 {
                return Err(mlua::Error::runtime(format!(
                    "icon {key} must be a whole number from 1 to {max}"
                )));
            }
            Ok(value as u32)
        };
        query.size = integer("size", 24, 4096)?;
        query.scale = integer("scale", 1, 16)?;
        query.path = options
            .get::<Option<String>>("path")?
            .filter(|path| !path.is_empty())
            .map(PathBuf::from);
        if query.path.as_ref().is_some_and(|path| {
            !path.is_absolute() || path.as_os_str().as_encoded_bytes().contains(&0)
        }) {
            return Err(mlua::Error::runtime(
                "icon path must be an absolute directory",
            ));
        }
        thumbnail = options.get::<Option<bool>>("thumbnail")?.unwrap_or(false);
    }
    Ok((query, thumbnail))
}

/// Cache directory for downscaled icon copies.
fn thumbnail_root() -> Option<PathBuf> {
    env::var_os("XDG_CACHE_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            env::var_os("HOME")
                .filter(|value| !value.is_empty())
                .map(|home| PathBuf::from(home).join(".cache"))
        })
        .map(|root| root.join("riced/icons"))
}

/// Downscales a raster icon to `pixels` and caches it by source path, size,
/// and mtime. Returns None for vector icons and decode or write failures.
fn thumbnail_path(source: &Path, pixels: u32, cache: &Path) -> Option<PathBuf> {
    let format = source.extension()?.to_str()?.to_ascii_lowercase();
    if !matches!(
        format.as_str(),
        "png" | "jpg" | "jpeg" | "webp" | "bmp" | "gif"
    ) {
        return None;
    }
    let metadata = source.metadata().ok()?;
    let modified = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())?
        .as_nanos();
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    source.hash(&mut hasher);
    metadata.len().hash(&mut hasher);
    modified.hash(&mut hasher);
    pixels.hash(&mut hasher);
    let target = cache.join(format!("{:016x}.png", hasher.finish()));
    if target.is_file() {
        return Some(target);
    }

    let image = image::open(source).ok()?;
    std::fs::create_dir_all(cache).ok()?;
    let temp = cache.join(format!(".{}.tmp", std::process::id()));
    image
        .thumbnail(pixels, pixels)
        .save_with_format(&temp, image::ImageFormat::Png)
        .ok()?;
    std::fs::rename(&temp, &target).ok()?;
    prune(cache);
    Some(target)
}

/// Keeps the thumbnail cache bounded by removing the oldest files.
fn prune(cache: &Path) {
    const LIMIT: usize = 256;
    let Ok(entries) = std::fs::read_dir(cache) else {
        return;
    };
    let mut files = entries
        .flatten()
        .filter_map(|entry| {
            let metadata = entry.metadata().ok()?;
            metadata
                .is_file()
                .then(|| (metadata.modified().ok(), entry.path()))
        })
        .collect::<Vec<_>>();
    if files.len() <= LIMIT {
        return;
    }
    files.sort_by_key(|(modified, _)| *modified);
    let excess = files.len() - LIMIT;
    for (_, path) in files.into_iter().take(excess) {
        let _ = std::fs::remove_file(path);
    }
}

pub(super) fn install(lua: &Lua) -> mlua::Result<()> {
    let (roots, pixmaps) = search_paths();
    install_with_paths(lua, roots, pixmaps, thumbnail_root())
}

fn install_with_paths(
    lua: &Lua,
    roots: Vec<PathBuf>,
    pixmaps: Vec<PathBuf>,
    cache: Option<PathBuf>,
) -> mlua::Result<()> {
    let iced: Table = lua.globals().get("iced")?;
    iced.set("icon_theme", "hicolor")?;
    let theme_roots = roots.clone();
    iced.set(
        "icon_themes",
        lua.create_function(move |_, ()| {
            let mut themes = BTreeSet::new();
            for root in &theme_roots {
                for entry in std::fs::read_dir(root).into_iter().flatten().flatten() {
                    if let Some(theme) = entry.file_name().to_str()
                        && name(theme)
                        && Theme::load(&theme_roots, theme).is_some()
                    {
                        themes.insert(theme.to_owned());
                    }
                }
            }
            Ok(themes.into_iter().collect::<Vec<_>>())
        })?,
    )?;
    let mut resolver = Resolver {
        roots,
        pixmaps,
        ..Resolver::default()
    };
    iced.set(
        "icon_path",
        lua.create_function_mut(move |lua, (icon, options): (String, Option<Table>)| {
            let iced = lua.globals().get::<Table>("iced")?;
            let (query, wants_thumbnail) = query(&iced, icon, options)?;
            let pixels = query.size.saturating_mul(query.scale).min(4096);
            let path = resolver.resolve(query);
            Ok(path.map(|path| {
                let path = if wants_thumbnail {
                    cache
                        .as_deref()
                        .and_then(|cache| thumbnail_path(&path, pixels, cache))
                        .unwrap_or(path)
                } else {
                    path
                };
                path.to_string_lossy().into_owned()
            }))
        })?,
    )?;
    iced.set(
        "fonts",
        lua.create_function(|_, ()| {
            let mut system = iced::advanced::graphics::text::font_system()
                .write()
                .map_err(|_| mlua::Error::runtime("font database lock poisoned"))?;
            let families = system
                .raw()
                .db()
                .faces()
                .flat_map(|face| face.families.iter().map(|(name, _)| name.clone()))
                .collect::<BTreeSet<_>>();
            Ok(families.into_iter().collect::<Vec<_>>())
        })?,
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Icons(PathBuf);

    impl Icons {
        fn new() -> Self {
            static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let root = env::temp_dir().join(format!(
                "riced-icons-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            ));
            std::fs::create_dir(&root).unwrap();
            Self(root)
        }

        fn write(&self, path: &str, data: &str) {
            let path = self.0.join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, data).unwrap();
        }

        fn png(&self, path: &str, width: u32, height: u32) {
            let path = self.0.join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            image::RgbaImage::from_pixel(width, height, image::Rgba([200, 30, 40, 255]))
                .save(&path)
                .unwrap();
        }

        fn theme(&self, root: &str, theme: &str, inherits: &str) {
            self.write(&format!("{root}/{theme}/index.theme"), &format!(
                "[Icon Theme]\nName=Display {theme}\nDirectories=16/apps,32/apps,16@2/apps,scalable/apps\nInherits={inherits}\n\n[16/apps]\nSize=16\nType=Fixed\n\n[32/apps]\nSize=32\nType=Fixed\n\n[16@2/apps]\nSize=16\nScale=2\nType=Fixed\n\n[scalable/apps]\nSize=24\nType=Scalable\nMinSize=16\nMaxSize=64\n"
            ));
        }

        fn lua(&self) -> Lua {
            let lua = Lua::new();
            lua.globals()
                .set("iced", lua.create_table().unwrap())
                .unwrap();
            install_with_paths(
                &lua,
                vec![self.0.join("user"), self.0.join("system")],
                vec![self.0.join("pixmaps")],
                Some(self.0.join("cache")),
            )
            .unwrap();
            lua.globals().set("root", self.0.to_str().unwrap()).unwrap();
            lua
        }
    }

    impl Drop for Icons {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn current_theme_size_scale_and_user_precedence_drive_resolution() {
        let icons = Icons::new();
        for theme in ["light", "dark"] {
            icons.theme("system", theme, "");
            for directory in ["16/apps", "32/apps", "16@2/apps", "scalable/apps"] {
                icons.write(&format!("system/{theme}/{directory}/app.svg"), "");
            }
        }
        icons.write("user/dark/16/apps/app.png", "");
        let lua = icons.lua();
        lua.load(r#"
            iced.icon_theme = 'light'
            assert(iced.icon_path('app', {size = 16}) == root .. '/system/light/16/apps/app.svg')
            iced.icon_theme = 'dark'
            assert(iced.icon_path('app', {size = 16}) == root .. '/user/dark/16/apps/app.png')
            assert(iced.icon_path('app', {size = 16, scale = 2}) == root .. '/system/dark/16@2/apps/app.svg')
            assert(iced.icon_path('app', {size = 24}) == root .. '/system/dark/scalable/apps/app.svg')
            assert(iced.icon_path('app', {size = 80}) == root .. '/system/dark/scalable/apps/app.svg')
            assert(table.concat(iced.icon_themes(), ',') == 'dark,light')
            iced.icon_theme = 'light'
            assert(iced.icon_path('app', {size = 16}) == root .. '/system/light/16/apps/app.svg')
        "#).exec().unwrap();
    }

    #[test]
    fn inherits_multiple_parents_without_cycles_then_falls_back() {
        let icons = Icons::new();
        for (theme, inherits) in [
            ("selected", "missing, loop, parent"),
            ("loop", "selected"),
            ("parent", "grandparent"),
            ("grandparent", ""),
            ("hicolor", ""),
        ] {
            icons.theme("system", theme, inherits);
        }
        icons.write("system/grandparent/16/apps/inherited.svg", "");
        icons.write("system/hicolor/16/apps/fallback.png", "");
        icons.write("pixmaps/unthemed.png", "");
        icons.write("private/custom.svg", "");
        let lua = icons.lua();
        lua.load(r#"
            iced.icon_theme = 'selected'
            assert(iced.icon_path('inherited', {size = 16}) == root .. '/system/grandparent/16/apps/inherited.svg')
            assert(iced.icon_path('fallback') == root .. '/system/hicolor/16/apps/fallback.png')
            assert(iced.icon_path('unthemed') == root .. '/pixmaps/unthemed.png')
            assert(iced.icon_path('custom', {path = root .. '/private'}) == root .. '/private/custom.svg')
            assert(iced.icon_path('custom.svg', {path = root .. '/private'}) == root .. '/private/custom.svg')
            assert(iced.icon_path(root .. '/private/custom.svg') == root .. '/private/custom.svg')
            assert(iced.icon_path('absent') == nil)
            iced.icon_theme = 'not-installed'
            assert(iced.icon_path('fallback') == root .. '/system/hicolor/16/apps/fallback.png')
        "#).exec().unwrap();
    }

    #[test]
    fn invalid_lookup_options_are_rejected_and_fonts_are_sorted() {
        let icons = Icons::new();
        let lua = icons.lua();
        for source in [
            "iced.icon_path('')",
            "iced.icon_path('../app')",
            "iced.icon_path('app', {size = 0})",
            "iced.icon_path('app', {scale = 1.5})",
            "iced.icon_path('app', {size = 0/0})",
            "iced.icon_path('app', {size = 4097})",
            "iced.icon_path('app', {path = '../icons'})",
            "iced.icon_path('app', {theme = 'unused'})",
            "iced.icon_theme = '../bad'; iced.icon_path('app')",
        ] {
            assert!(lua.load(source).exec().is_err(), "{source}");
        }
        lua.load(
            r#"
            local fonts = iced.fonts()
            assert(#fonts > 0)
            for index = 2, #fonts do assert(fonts[index - 1] < fonts[index]) end
        "#,
        )
        .exec()
        .unwrap();
    }

    #[test]
    fn thumbnails_downscale_raster_icons_and_reuse_the_cache() {
        let icons = Icons::new();
        icons.png("icon.png", 64, 64);
        let source = icons.0.join("icon.png");
        let cache = icons.0.join("cache");

        let first = thumbnail_path(&source, 24, &cache).unwrap();
        assert!(first.starts_with(&cache));
        assert_eq!(image::image_dimensions(&first).unwrap(), (24, 24));

        let modified = first.metadata().unwrap().modified().unwrap();
        let second = thumbnail_path(&source, 24, &cache).unwrap();
        assert_eq!(first, second);
        assert_eq!(
            modified,
            second.metadata().unwrap().modified().unwrap(),
            "unchanged sources should reuse the cached thumbnail"
        );

        icons.write("icon.svg", "<svg/>");
        assert!(thumbnail_path(&icons.0.join("icon.svg"), 24, &cache).is_none());
    }

    #[test]
    fn thumbnail_option_returns_a_cached_copy() {
        let icons = Icons::new();
        icons.png("icon.png", 96, 96);
        icons.write("icon.svg", "<svg/>");
        let lua = icons.lua();
        lua.load(
            r#"
            local png = iced.icon_path(root .. '/icon.png', {size = 24, thumbnail = true})
            assert(png and png ~= root .. '/icon.png')
            assert(png:match('/cache/'))
            local svg = iced.icon_path(root .. '/icon.svg', {thumbnail = true})
            assert(svg == root .. '/icon.svg')
        "#,
        )
        .exec()
        .unwrap();
        let cache = icons.0.join("cache");
        assert_eq!(
            std::fs::read_dir(cache).unwrap().count(),
            1,
            "one source produces one cached thumbnail"
        );
    }

    #[test]
    fn icon_lookups_recheck_the_filesystem_after_the_refresh_window() {
        let icons = Icons::new();
        let mut resolver = Resolver {
            roots: vec![icons.0.join("system")],
            ..Resolver::default()
        };
        let query = || Query {
            name: "late".to_owned(),
            theme: "hicolor".to_owned(),
            size: 16,
            scale: 1,
            path: None,
        };
        assert!(resolver.resolve(query()).is_none());
        icons.theme("system", "hicolor", "");
        icons.write("system/hicolor/16/apps/late.svg", "");
        assert!(
            resolver.resolve(query()).is_none(),
            "a hit inside the refresh window stays cached"
        );
        resolver.checked = Some(Instant::now() - REFRESH);
        assert!(resolver.resolve(query()).is_some());
    }
}
