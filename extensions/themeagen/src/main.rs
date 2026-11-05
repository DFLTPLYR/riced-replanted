//! Material You theme generation and Matugen-compatible template rendering.
//!
//! Reads the existing `<config>/riced/data/templates/config.toml`, renders every
//! `[templates.*]` entry with `{{ colors.* }}` variables, and publishes both
//! palettes as `riced.plugins("themeagen").colors`.

use std::{
    collections::{BTreeMap, HashMap},
    env, fs, io,
    path::{Path, PathBuf},
    process::Command,
    sync::{Arc, Mutex, MutexGuard, PoisonError},
    thread,
    time::Duration,
};

use material_colors::{color::Argb, dynamic_color::Variant, scheme::Scheme, theme::ThemeBuilder};
use riced_extension_sdk::write_atomic::write_atomic;
use riced_extension_sdk::{Context, Plugin, log};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Map, Value, json};

const CORE: &[&str] = &[
    "primary",
    "onprimary",
    "secondary",
    "onsecondary",
    "tertiary",
    "ontertiary",
    "error",
    "onerror",
    "surface",
    "onsurface",
    "surfacevariant",
    "onsurfacevariant",
    "outline",
    "shadow",
    "hover",
    "onhover",
];

const TERMINAL_NAMES: &[&str] = &[
    "black", "red", "green", "yellow", "blue", "magenta", "cyan", "white",
];

const TERMINAL_SCALARS: &[(&str, &[&str])] = &[
    ("foreground", &["foreground"]),
    ("background", &["background"]),
    (
        "selectionfg",
        &["selectionfg", "selectionFg", "selection_fg"],
    ),
    (
        "selectionbg",
        &["selectionbg", "selectionBg", "selection_bg"],
    ),
    ("cursortext", &["cursortext", "cursorText", "cursor_text"]),
    ("cursor", &["cursor"]),
];

#[derive(Default)]
struct Model {
    state: Value,
    dirty: bool,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct Persisted {
    mode: String,
    source: Source,
    colors: Value,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(tag = "kind", rename_all = "lowercase")]
enum Source {
    Seed { seed: String, variant: String },
    Image { image: String, variant: String },
    Manual,
}

impl Source {
    fn seed(&self) -> Result<Argb, String> {
        match self {
            Self::Seed { seed, .. } => parse_seed(seed),
            Self::Image { image, .. } => seed_from_image(image),
            Self::Manual => Err("manual palette has no source".to_owned()),
        }
    }

    fn variant(&self) -> Option<&str> {
        match self {
            Self::Seed { variant, .. } | Self::Image { variant, .. } => Some(variant),
            Self::Manual => None,
        }
    }

    fn label(&self) -> String {
        match self {
            Self::Seed { seed, .. } => seed.clone(),
            Self::Image { image, .. } => image.clone(),
            Self::Manual => String::new(),
        }
    }
}

impl Persisted {
    fn published(&self) -> Value {
        json!({
            "ready": true,
            "mode": &self.mode,
            "source": &self.source,
            "colors": &self.colors,
        })
    }
}

struct Stored {
    persisted: Persisted,
    applied: usize,
    errors: Vec<String>,
}

impl Stored {
    fn published(&self) -> Value {
        self.persisted.published()
    }

    fn summary(&self) -> Value {
        json!({
            "mode": &self.persisted.mode,
            "source": &self.persisted.source,
            "colors": &self.persisted.colors,
            "applied": self.applied,
            "errors": &self.errors,
        })
    }
}

fn main() -> io::Result<()> {
    let state = match state_path().and_then(|path| load_from(&path)) {
        Ok(Some(persisted)) => persisted.published(),
        Ok(None) => json!({"ready": false, "mode": "dark"}),
        Err(error) => {
            log::warn!(target: "themeagen.state", "{error}");
            json!({"ready": false, "mode": "dark"})
        }
    };
    let model = Arc::new(Mutex::new(Model { state, dirty: true }));

    let generate_model = Arc::clone(&model);
    let populate_model = Arc::clone(&model);
    let regenerate_model = Arc::clone(&model);

    Plugin::new("themeagen")
        .state(["ready", "mode", "source", "colors"])
        .method("generate", false, move |args| {
            handle_generate(&generate_model, args)
        })
        .method("populate", false, move |args| {
            handle_populate(&populate_model, args)
        })
        .method("regenerate", false, move |args| {
            handle_regenerate(&regenerate_model, args)
        })
        .serve(move |context| run(context, model))
}

fn run(context: Context, model: Arc<Mutex<Model>>) -> ! {
    loop {
        let state = {
            let mut model = model_lock(&model);
            if model.dirty {
                model.dirty = false;
                Some(model.state.clone())
            } else {
                None
            }
        };
        if let Some(state) = state {
            context.publish(state);
        }
        thread::sleep(Duration::from_millis(100));
    }
}

fn model_lock(model: &Arc<Mutex<Model>>) -> MutexGuard<'_, Model> {
    model.lock().unwrap_or_else(PoisonError::into_inner)
}

fn current_mode(model: &Arc<Mutex<Model>>) -> String {
    model_lock(model)
        .state
        .get("mode")
        .and_then(Value::as_str)
        .unwrap_or("dark")
        .to_owned()
}

fn update(model: &Arc<Mutex<Model>>, result: Result<Stored, String>) -> Result<Value, String> {
    let stored = result?;
    let summary = stored.summary();
    let published = stored.published();
    let mut model = model_lock(model);
    model.state = published;
    model.dirty = true;
    Ok(summary)
}

fn handle_generate(model: &Arc<Mutex<Model>>, args: &Value) -> Result<Value, String> {
    let options: GenerateOptions = parse_options(args)?;
    let mode = current_mode(model);
    update(model, generate(options, &mode))
}

fn handle_populate(model: &Arc<Mutex<Model>>, args: &Value) -> Result<Value, String> {
    let options: PopulateOptions = parse_options(args)?;
    let mode = current_mode(model);
    update(model, populate(options, &mode))
}

fn handle_regenerate(model: &Arc<Mutex<Model>>, args: &Value) -> Result<Value, String> {
    let options: RegenerateOptions = parse_options(args)?;
    update(model, regenerate(options))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GenerateOptions {
    #[serde(default)]
    seed: Option<String>,
    #[serde(default)]
    image: Option<String>,
    #[serde(default)]
    mode: Option<String>,
    #[serde(default)]
    variant: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PopulateOptions {
    #[serde(default)]
    colors: Option<Value>,
    #[serde(default)]
    mode: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RegenerateOptions {
    #[serde(default)]
    mode: Option<String>,
}

fn parse_options<T: DeserializeOwned>(args: &Value) -> Result<T, String> {
    let value = match args {
        Value::Array(values) => values.first().cloned().unwrap_or_else(|| json!({})),
        Value::Object(_) => args.clone(),
        _ => json!({}),
    };
    serde_json::from_value(value).map_err(|error| format!("invalid options: {error}"))
}

fn generate(options: GenerateOptions, current_mode: &str) -> Result<Stored, String> {
    let variant = options.variant.unwrap_or_else(|| "tonalspot".to_owned());
    let parsed_variant = parse_variant(&variant)?;
    let source = match (options.seed, options.image) {
        (Some(seed), None) => {
            parse_seed(&seed)?;
            Source::Seed {
                seed: seed.to_ascii_lowercase(),
                variant,
            }
        }
        (None, Some(image)) => Source::Image { image, variant },
        (Some(_), Some(_)) => return Err("provide seed or image, not both".to_owned()),
        (None, None) => return Err("provide a seed or image".to_owned()),
    };
    let seed = source.seed()?;
    let colors = generate_palette(seed, parsed_variant)?;
    let mode = mode(options.mode.as_deref().unwrap_or(current_mode))?;
    finish(Persisted {
        mode,
        source,
        colors,
    })
}

fn populate(options: PopulateOptions, current_mode: &str) -> Result<Stored, String> {
    let supplied = options.colors.is_some();
    let mut persisted = match options.colors {
        Some(colors) => Persisted {
            mode: mode(current_mode)?,
            source: Source::Manual,
            colors: normalize_palette(&colors)?,
        },
        None => load_state()?.ok_or_else(|| "no saved palette; call generate first".to_owned())?,
    };
    if let Some(value) = options.mode {
        persisted.mode = mode(&value)?;
    } else if supplied {
        persisted.mode = mode(current_mode)?;
    }
    finish(persisted)
}

fn regenerate(options: RegenerateOptions) -> Result<Stored, String> {
    let persisted =
        load_state()?.ok_or_else(|| "no saved palette; call generate first".to_owned())?;
    let seed = persisted
        .source
        .seed()
        .map_err(|_| "saved palette was supplied manually; call generate first".to_owned())?;
    let variant = persisted
        .source
        .variant()
        .ok_or_else(|| "saved palette was supplied manually; call generate first".to_owned())?;
    let colors = generate_palette(seed, parse_variant(variant)?)?;
    let mode = mode(options.mode.as_deref().unwrap_or(&persisted.mode))?;
    finish(Persisted {
        mode,
        source: persisted.source,
        colors,
    })
}

fn finish(persisted: Persisted) -> Result<Stored, String> {
    persist(&persisted)?;
    let (applied, errors) = apply_or_report(&persisted);
    Ok(Stored {
        persisted,
        applied,
        errors,
    })
}

fn apply_or_report(persisted: &Persisted) -> (usize, Vec<String>) {
    let (applied, errors) = match config_path().and_then(|path| {
        apply_templates(
            &path,
            &persisted.colors,
            &persisted.mode,
            &persisted.source.label(),
        )
    }) {
        Ok(result) => (result.applied, result.errors),
        Err(error) => (0, vec![error]),
    };
    for error in &errors {
        log::warn!(target: "themeagen.templates", "{error}");
    }
    (applied, errors)
}

fn mode(value: &str) -> Result<String, String> {
    match value {
        "dark" | "light" => Ok(value.to_owned()),
        other => Err(format!("mode must be dark or light, got {other:?}")),
    }
}

fn parse_seed(seed: &str) -> Result<Argb, String> {
    let digits = seed
        .strip_prefix('#')
        .filter(|digits| digits.len() == 6 && digits.bytes().all(|b| b.is_ascii_hexdigit()))
        .ok_or_else(|| "seed must be #rrggbb".to_owned())?;
    let rgb = u32::from_str_radix(digits, 16).map_err(|error| error.to_string())?;
    Ok(Argb::from_u32(0xff00_0000 | rgb))
}

fn parse_variant(name: &str) -> Result<Variant, String> {
    let variant = match name.trim().to_ascii_lowercase().replace('-', "_").as_str() {
        "content" => Variant::Content,
        "tonalspot" => Variant::TonalSpot,
        "monochrome" => Variant::Monochrome,
        "neutral" => Variant::Neutral,
        "vibrant" => Variant::Vibrant,
        "expressive" => Variant::Expressive,
        "fidelity" => Variant::Fidelity,
        "rainbow" => Variant::Rainbow,
        "fruit_salad" | "fruitsalad" => Variant::FruitSalad,
        _ => {
            return Err(format!(
                "unknown variant {name:?}; use content, tonalspot, monochrome, neutral, vibrant, expressive, fidelity, rainbow, or fruit_salad"
            ));
        }
    };
    Ok(variant)
}

fn seed_from_image(path: &str) -> Result<Argb, String> {
    let path = expand_home(path);
    let image = image::open(&path)
        .map_err(|error| format!("cannot read image {}: {error}", path.display()))?;
    let sample = image
        .resize(128, 128, image::imageops::FilterType::Triangle)
        .to_rgba8();
    let data = material_colors::image::Image::new(sample);
    Ok(material_colors::image::ImageReader::extract_color(&data))
}

fn generate_palette(seed: Argb, variant: Variant) -> Result<Value, String> {
    let theme = ThemeBuilder::with_source(seed).variant(variant).build();
    let colors = json!({
        "dark": mode_json(&theme.schemes.dark),
        "light": mode_json(&theme.schemes.light),
    });
    normalize_palette(&colors)
}

fn mode_json(scheme: &Scheme) -> Value {
    let mut colors = Map::new();
    for (name, color) in [
        ("primary", scheme.primary),
        ("onprimary", scheme.on_primary),
        ("secondary", scheme.secondary),
        ("onsecondary", scheme.on_secondary),
        ("tertiary", scheme.tertiary),
        ("ontertiary", scheme.on_tertiary),
        ("error", scheme.error),
        ("onerror", scheme.on_error),
        ("surface", scheme.surface),
        ("onsurface", scheme.on_surface),
        ("surfacevariant", scheme.surface_variant),
        ("onsurfacevariant", scheme.on_surface_variant),
        ("outline", scheme.outline),
        ("shadow", scheme.shadow),
        ("hover", scheme.primary),
        ("onhover", scheme.on_primary),
    ] {
        colors.insert(name.to_owned(), Value::String(hex(color)));
    }
    for (name, color) in [
        ("primary_container", scheme.primary_container),
        ("on_primary_container", scheme.on_primary_container),
        ("inverse_primary", scheme.inverse_primary),
        ("primary_fixed", scheme.primary_fixed),
        ("primary_fixed_dim", scheme.primary_fixed_dim),
        ("on_primary_fixed", scheme.on_primary_fixed),
        ("on_primary_fixed_variant", scheme.on_primary_fixed_variant),
        ("secondary_container", scheme.secondary_container),
        ("on_secondary_container", scheme.on_secondary_container),
        ("secondary_fixed", scheme.secondary_fixed),
        ("secondary_fixed_dim", scheme.secondary_fixed_dim),
        ("on_secondary_fixed", scheme.on_secondary_fixed),
        (
            "on_secondary_fixed_variant",
            scheme.on_secondary_fixed_variant,
        ),
        ("tertiary_container", scheme.tertiary_container),
        ("on_tertiary_container", scheme.on_tertiary_container),
        ("tertiary_fixed", scheme.tertiary_fixed),
        ("tertiary_fixed_dim", scheme.tertiary_fixed_dim),
        ("on_tertiary_fixed", scheme.on_tertiary_fixed),
        (
            "on_tertiary_fixed_variant",
            scheme.on_tertiary_fixed_variant,
        ),
        ("error_container", scheme.error_container),
        ("on_error_container", scheme.on_error_container),
        ("surface_dim", scheme.surface_dim),
        ("surface_tint", scheme.surface_tint),
        ("surface_bright", scheme.surface_bright),
        ("surface_container_lowest", scheme.surface_container_lowest),
        ("surface_container_low", scheme.surface_container_low),
        ("surface_container", scheme.surface_container),
        ("surface_container_high", scheme.surface_container_high),
        (
            "surface_container_highest",
            scheme.surface_container_highest,
        ),
        ("outline_variant", scheme.outline_variant),
        ("inverse_surface", scheme.inverse_surface),
        ("inverse_on_surface", scheme.inverse_on_surface),
        ("background", scheme.background),
        ("on_background", scheme.on_background),
        ("scrim", scheme.scrim),
    ] {
        colors.insert(name.to_owned(), Value::String(hex(color)));
    }
    colors.insert("terminal".to_owned(), terminal_json(scheme));
    Value::Object(colors)
}

fn terminal_json(scheme: &Scheme) -> Value {
    let section = |pairs: [(&str, Argb); 8]| {
        let mut colors = Map::new();
        for (name, color) in pairs {
            colors.insert(name.to_owned(), Value::String(hex(color)));
        }
        Value::Object(colors)
    };
    json!({
        "normal": section([
            ("black", scheme.surface_container_high),
            ("red", scheme.error),
            ("green", scheme.secondary),
            ("yellow", scheme.tertiary),
            ("blue", scheme.primary),
            ("magenta", scheme.tertiary_fixed_dim),
            ("cyan", scheme.secondary_fixed_dim),
            ("white", scheme.surface_bright),
        ]),
        "bright": section([
            ("black", scheme.outline_variant),
            ("red", scheme.error_container),
            ("green", scheme.secondary_container),
            ("yellow", scheme.tertiary_container),
            ("blue", scheme.primary_fixed_dim),
            ("magenta", scheme.tertiary_fixed),
            ("cyan", scheme.secondary_fixed),
            ("white", scheme.on_surface),
        ]),
        "foreground": hex(scheme.on_surface),
        "background": hex(scheme.surface),
        "selectionfg": hex(scheme.surface),
        "selectionbg": hex(scheme.primary),
        "cursortext": hex(scheme.surface),
        "cursor": hex(scheme.primary),
    })
}

fn hex(color: Argb) -> String {
    format!("#{:02x}{:02x}{:02x}", color.red, color.green, color.blue)
}

fn normalize_palette(value: &Value) -> Result<Value, String> {
    let root = value
        .as_object()
        .ok_or_else(|| "colors must be an object".to_owned())?;
    let mut palette = Map::new();
    for name in ["dark", "light"] {
        let scheme = root
            .get(name)
            .and_then(Value::as_object)
            .ok_or_else(|| format!("missing {name} colors"))?;
        palette.insert(name.to_owned(), Value::Object(normalize_scheme(scheme)?));
    }
    Ok(Value::Object(palette))
}

fn normalize_scheme(input: &Map<String, Value>) -> Result<Map<String, Value>, String> {
    let mut output = Map::new();
    for (key, value) in input {
        if key == "terminal" {
            continue;
        }
        let name = core_name(key).unwrap_or(key);
        let color = normalize_hex(value).map_err(|error| format!("{key}: {error}"))?;
        output.insert(name.to_owned(), Value::String(color));
    }
    for name in CORE {
        if !output.contains_key(*name) {
            return Err(format!("missing {name}"));
        }
    }
    let terminal = input
        .get("terminal")
        .ok_or_else(|| "missing terminal".to_owned())?;
    output.insert("terminal".to_owned(), normalize_terminal(terminal)?);
    Ok(output)
}

fn core_name(key: &str) -> Option<&'static str> {
    Some(match key {
        "primary" => "primary",
        "onprimary" | "on_primary" => "onprimary",
        "secondary" => "secondary",
        "onsecondary" | "on_secondary" => "onsecondary",
        "tertiary" => "tertiary",
        "ontertiary" | "on_tertiary" => "ontertiary",
        "error" => "error",
        "onerror" | "on_error" => "onerror",
        "surface" => "surface",
        "onsurface" | "on_surface" => "onsurface",
        "surfacevariant" | "surface_variant" => "surfacevariant",
        "onsurfacevariant" | "on_surface_variant" => "onsurfacevariant",
        "outline" => "outline",
        "shadow" => "shadow",
        "hover" => "hover",
        "onhover" | "on_hover" => "onhover",
        _ => return None,
    })
}

fn normalize_hex(value: &Value) -> Result<String, String> {
    let text = value
        .as_str()
        .ok_or_else(|| "must be a #rrggbb color".to_owned())?;
    let digits = text
        .strip_prefix('#')
        .ok_or_else(|| format!("{text:?} must start with #"))?;
    if digits.len() != 6 || !digits.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(format!("{text:?} is not #rrggbb"));
    }
    Ok(format!("#{}", digits.to_ascii_lowercase()))
}

fn normalize_terminal(value: &Value) -> Result<Value, String> {
    let terminal = value
        .as_object()
        .ok_or_else(|| "terminal must be an object".to_owned())?;
    let mut output = Map::new();
    for section in ["normal", "bright"] {
        let colors = terminal
            .get(section)
            .and_then(Value::as_object)
            .ok_or_else(|| format!("missing terminal.{section}"))?;
        let mut normalized = Map::new();
        for name in TERMINAL_NAMES {
            let color = colors
                .get(*name)
                .ok_or_else(|| format!("missing terminal.{section}.{name}"))?;
            normalized.insert(
                (*name).to_owned(),
                Value::String(
                    normalize_hex(color)
                        .map_err(|error| format!("terminal.{section}.{name}: {error}"))?,
                ),
            );
        }
        output.insert(section.to_owned(), Value::Object(normalized));
    }
    for (name, aliases) in TERMINAL_SCALARS {
        let color = aliases
            .iter()
            .find_map(|alias| terminal.get(*alias))
            .ok_or_else(|| format!("missing terminal.{name}"))?;
        output.insert(
            (*name).to_owned(),
            Value::String(
                normalize_hex(color).map_err(|error| format!("terminal.{name}: {error}"))?,
            ),
        );
    }
    Ok(Value::Object(output))
}

#[derive(Deserialize)]
struct TemplatesConfig {
    #[serde(default)]
    templates: BTreeMap<String, TemplateEntry>,
}

#[derive(Deserialize)]
struct TemplateEntry {
    input_path: String,
    output_path: String,
    #[serde(default)]
    pre_hook: Option<String>,
    #[serde(default)]
    post_hook: Option<String>,
}

struct RenderedTemplate {
    name: String,
    text: String,
    output: PathBuf,
    pre: Option<String>,
    post: Option<String>,
}

#[derive(Default)]
struct ApplyResult {
    applied: usize,
    errors: Vec<String>,
}

fn apply_templates(
    config: &Path,
    colors: &Value,
    mode: &str,
    image: &str,
) -> Result<ApplyResult, String> {
    let source = fs::read_to_string(config)
        .map_err(|error| format!("cannot read {}: {error}", config.display()))?;
    let parsed: TemplatesConfig = toml::from_str(&source)
        .map_err(|error| format!("cannot parse {}: {error}", config.display()))?;
    if parsed.templates.is_empty() {
        return Err(format!("no [templates] sections in {}", config.display()));
    }
    let dir = config.parent().unwrap_or_else(|| Path::new("."));
    let variables = variables(colors, mode, image)?;
    let mut templates = Vec::with_capacity(parsed.templates.len());
    for (name, entry) in &parsed.templates {
        if entry.input_path.is_empty() {
            return Err(format!("{name}: input_path is empty"));
        }
        if entry.output_path.is_empty() {
            return Err(format!("{name}: output_path is empty"));
        }
        let input = dir.join(&entry.input_path);
        let source = fs::read_to_string(&input)
            .map_err(|error| format!("{name}: cannot read {}: {error}", input.display()))?;
        let text = render(&source, &variables).map_err(|error| format!("{name}: {error}"))?;
        let pre = entry
            .pre_hook
            .as_deref()
            .map(|hook| render(hook, &variables))
            .transpose()
            .map_err(|error| format!("{name}: pre_hook: {error}"))?;
        let post = entry
            .post_hook
            .as_deref()
            .map(|hook| render(hook, &variables))
            .transpose()
            .map_err(|error| format!("{name}: post_hook: {error}"))?;
        let output = expand_home(&entry.output_path);
        let output = if output.is_absolute() {
            output
        } else {
            dir.join(output)
        };
        templates.push(RenderedTemplate {
            name: name.clone(),
            text,
            output,
            pre,
            post,
        });
    }

    let mut result = ApplyResult::default();
    for template in templates {
        if let Some(pre) = template.pre
            && !run_hook(&pre)
        {
            result
                .errors
                .push(format!("{}: pre_hook failed", template.name));
            continue;
        }
        match write_atomic(&template.output, &template.text) {
            Ok(()) => {
                result.applied += 1;
                if let Some(post) = template.post
                    && !run_hook(&post)
                {
                    result
                        .errors
                        .push(format!("{}: post_hook failed", template.name));
                }
            }
            Err(error) => result.errors.push(format!(
                "{}: cannot write {}: {error}",
                template.name,
                template.output.display()
            )),
        }
    }
    Ok(result)
}

fn variables(colors: &Value, mode: &str, image: &str) -> Result<HashMap<String, String>, String> {
    let scheme = colors
        .get(mode)
        .and_then(Value::as_object)
        .ok_or_else(|| format!("missing {mode} colors"))?;
    let mut variables = HashMap::new();
    for (key, value) in scheme {
        if key == "terminal" {
            continue;
        }
        let color = value
            .as_str()
            .ok_or_else(|| format!("{key} is not a color"))?;
        for name in [key.clone(), snake(key), flat(key)] {
            for suffix in ["", ".hex", ".default.hex"] {
                variables.insert(format!("colors.{name}{suffix}"), color.to_owned());
            }
        }
    }
    for name in ["hover", "on_hover"] {
        if let Some(color) = variables.get(&format!("colors.{name}")).cloned() {
            variables.insert(name.to_owned(), color);
        }
    }

    let terminal = scheme
        .get("terminal")
        .and_then(Value::as_object)
        .ok_or_else(|| "missing terminal".to_owned())?;
    for section in ["normal", "bright"] {
        let colors = terminal
            .get(section)
            .and_then(Value::as_object)
            .ok_or_else(|| format!("missing terminal.{section}"))?;
        for name in TERMINAL_NAMES {
            let color = colors
                .get(*name)
                .and_then(Value::as_str)
                .ok_or_else(|| format!("missing terminal.{section}.{name}"))?;
            for prefix in ["colors.", ""] {
                variables.insert(
                    format!("{prefix}terminal.{section}.{name}"),
                    color.to_owned(),
                );
            }
        }
    }
    for (name, aliases) in TERMINAL_SCALARS {
        let color = terminal
            .get(*name)
            .and_then(Value::as_str)
            .ok_or_else(|| format!("missing terminal.{name}"))?;
        for alias in *aliases {
            for prefix in ["colors.", ""] {
                variables.insert(format!("{prefix}terminal.{alias}"), color.to_owned());
            }
        }
    }
    variables.insert("mode".to_owned(), mode.to_owned());
    variables.insert("image".to_owned(), image.to_owned());
    Ok(variables)
}

fn snake(key: &str) -> String {
    match key {
        "onprimary" => "on_primary",
        "onsecondary" => "on_secondary",
        "ontertiary" => "on_tertiary",
        "onerror" => "on_error",
        "onsurface" => "on_surface",
        "surfacevariant" => "surface_variant",
        "onsurfacevariant" => "on_surface_variant",
        "onhover" => "on_hover",
        other => other,
    }
    .to_owned()
}

fn flat(key: &str) -> String {
    key.replace('_', "")
}

fn render(text: &str, variables: &HashMap<String, String>) -> Result<String, String> {
    let mut output = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find("{{") {
        output.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        let end = after.find("}}").ok_or_else(|| "unclosed '{{'".to_owned())?;
        let key = after[..end].trim();
        if key.is_empty() {
            return Err("empty template variable".to_owned());
        }
        let key = match key.strip_prefix("color.") {
            Some(name) => format!("colors.{name}"),
            None => key.to_owned(),
        };
        let value = variables
            .get(&key)
            .ok_or_else(|| format!("unknown template variable: {key}"))?;
        output.push_str(value);
        rest = &after[end + 2..];
    }
    output.push_str(rest);
    Ok(output)
}

fn run_hook(command: &str) -> bool {
    Command::new("sh")
        .arg("-c")
        .arg(command)
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

fn expand_home(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/")
        && let Some(home) = env::var_os("HOME")
    {
        return PathBuf::from(home).join(rest);
    }
    if path == "~"
        && let Some(home) = env::var_os("HOME")
    {
        return PathBuf::from(home);
    }
    PathBuf::from(path)
}

fn config_path() -> Result<PathBuf, String> {
    if let Some(path) = env::var_os("RICED_THEMEAGEN_CONFIG") {
        return Ok(PathBuf::from(path));
    }
    Ok(config_root()?.join("riced/data/templates/config.toml"))
}

fn state_path() -> Result<PathBuf, String> {
    if let Some(path) = env::var_os("RICED_THEMEAGEN_STATE") {
        return Ok(PathBuf::from(path));
    }
    Ok(config_root()?.join("riced/data/theme/themeagen.json"))
}

fn config_root() -> Result<PathBuf, String> {
    env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
        .ok_or_else(|| "XDG_CONFIG_HOME and HOME are unset".to_owned())
}

fn persist(persisted: &Persisted) -> Result<(), String> {
    let path = state_path()?;
    persist_to(&path, persisted)
}

fn persist_to(path: &Path, persisted: &Persisted) -> Result<(), String> {
    let text = serde_json::to_string_pretty(persisted)
        .map_err(|error| format!("cannot encode palette: {error}"))?;
    write_atomic(path, &text).map_err(|error| format!("cannot write {}: {error}", path.display()))
}

fn load_state() -> Result<Option<Persisted>, String> {
    let path = state_path()?;
    load_from(&path)
}

fn load_from(path: &Path) -> Result<Option<Persisted>, String> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(format!("cannot read {}: {error}", path.display()));
        }
    };
    let mut persisted: Persisted = serde_json::from_str(&text)
        .map_err(|error| format!("cannot parse {}: {error}", path.display()))?;
    persisted.mode = mode(&persisted.mode)?;
    persisted.colors = normalize_palette(&persisted.colors)?;
    Ok(Some(persisted))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(name: &str) -> Self {
            let path = env::temp_dir().join(format!(
                "riced-themeagen-{}-{}-{}",
                std::process::id(),
                name,
                NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn colors() -> Value {
        generate_palette(parse_seed("#39bae6").unwrap(), Variant::TonalSpot).unwrap()
    }

    fn terminal_of<'a>(palette: &'a Value, mode: &str) -> &'a Map<String, Value> {
        palette[mode]["terminal"].as_object().unwrap()
    }

    #[test]
    fn seeds_and_modes_are_validated() {
        assert_eq!(hex(parse_seed("#39BAE6").unwrap()), "#39bae6");
        for seed in ["39bae6", "#39bae", "#39baez", "#39bae66"] {
            assert!(parse_seed(seed).is_err(), "{seed}");
        }
        assert_eq!(mode("dark").unwrap(), "dark");
        assert!(mode("auto").is_err());
        assert!(parse_variant("bogus").is_err());
    }

    #[test]
    fn generated_palettes_cover_both_modes() {
        let palette = colors();
        for name in ["dark", "light"] {
            let scheme = palette[name].as_object().unwrap();
            for role in CORE {
                assert!(
                    scheme[*role].as_str().unwrap().starts_with('#'),
                    "{name}.{role}"
                );
            }
            for role in [
                "primary_container",
                "surface_container_high",
                "outline_variant",
                "inverse_surface",
            ] {
                assert!(
                    scheme[role].as_str().unwrap().starts_with('#'),
                    "{name}.{role}"
                );
            }
            let terminal = terminal_of(&palette, name);
            assert!(terminal["normal"]["red"].as_str().unwrap().starts_with('#'));
            assert!(terminal["selectionfg"].as_str().unwrap().starts_with('#'));
            assert_eq!(
                terminal["foreground"], scheme["onsurface"],
                "{name} terminal foreground"
            );
        }
        assert_eq!(palette["dark"]["hover"], palette["dark"]["primary"]);
        assert_eq!(palette["dark"]["onhover"], palette["dark"]["onprimary"]);
    }

    #[test]
    fn manual_palettes_accept_matugen_names() {
        let palette = colors();
        let mut dark = palette["dark"].as_object().unwrap().clone();
        for (from, to) in [
            ("onprimary", "on_primary"),
            ("surfacevariant", "surface_variant"),
            ("onsurfacevariant", "on_surface_variant"),
            ("onhover", "on_hover"),
        ] {
            let value = dark.remove(from).unwrap();
            dark.insert(to.to_owned(), value);
        }
        let manual = json!({"dark": dark, "light": palette["light"]});
        let normalized = normalize_palette(&manual).unwrap();
        assert_eq!(
            normalized["dark"]["onprimary"],
            palette["dark"]["onprimary"]
        );
        assert_eq!(
            normalized["dark"]["surfacevariant"],
            palette["dark"]["surfacevariant"]
        );

        let mut broken = palette.clone();
        broken["dark"]["primary"] = json!("not-a-color");
        assert!(normalize_palette(&broken).is_err());

        let mut missing = palette;
        missing["light"].as_object_mut().unwrap().remove("error");
        assert!(normalize_palette(&missing).is_err());
    }

    #[test]
    fn templates_use_both_role_spellings_and_terminal_aliases() {
        let palette = colors();
        let variables = variables(&palette, "dark", "#39bae6").unwrap();
        assert_eq!(variables["colors.primary"], palette["dark"]["primary"]);
        assert_eq!(variables["colors.on_primary"], palette["dark"]["onprimary"]);
        assert_eq!(
            variables["colors.surface_variant"],
            palette["dark"]["surfacevariant"]
        );
        assert_eq!(
            variables["colors.primary.default.hex"],
            palette["dark"]["primary"]
        );
        assert_eq!(
            variables["colors.terminal.cursorText"],
            palette["dark"]["terminal"]["cursortext"]
        );
        assert_eq!(
            variables["colors.terminal.selectionFg"],
            palette["dark"]["terminal"]["selectionfg"]
        );

        let rendered = render(
            "{{ colors.on_primary }} {{colors.primary}} {{ color.primary }} {{ colors.terminal.cursorText }}",
            &variables,
        )
        .unwrap();
        assert!(rendered.contains(palette["dark"]["onprimary"].as_str().unwrap()));
        assert!(rendered.contains(palette["dark"]["terminal"]["cursortext"].as_str().unwrap()));
        assert!(render("{{ colors.nope }}", &variables).is_err());
        assert!(render("{{ colors.primary", &variables).is_err());
    }

    #[test]
    fn apply_templates_renders_config_and_ignores_catalog() {
        let dir = TempDir::new("templates");
        let config = dir.0.join("config.toml");
        let input = dir.0.join("input.conf");
        fs::write(
            &input,
            "primary={{ colors.primary }}\nselection={{ colors.terminal.selectionFg }}\n",
        )
        .unwrap();
        fs::write(
            &config,
            r#"
[catalog.example]
name = "Example"

[templates.example]
input_path = "input.conf"
output_path = "out/colors.conf"
"#,
        )
        .unwrap();
        let palette = colors();
        let result = apply_templates(&config, &palette, "dark", "#39bae6").unwrap();
        assert_eq!(result.applied, 1);
        assert!(result.errors.is_empty());
        let rendered = fs::read_to_string(dir.0.join("out/colors.conf")).unwrap();
        assert!(rendered.contains(palette["dark"]["primary"].as_str().unwrap()));
        assert!(rendered.contains(palette["dark"]["terminal"]["selectionfg"].as_str().unwrap()));
    }

    #[test]
    fn unknown_variables_and_failed_hooks_do_not_write_outputs() {
        let dir = TempDir::new("failure");
        let config = dir.0.join("config.toml");
        let input = dir.0.join("input.conf");
        let output = dir.0.join("colors.conf");
        fs::write(&input, "{{ colors.nope }}").unwrap();
        fs::write(
            &config,
            format!(
                r#"
[templates.bad]
input_path = "input.conf"
output_path = "{}"
"#,
                output.display()
            ),
        )
        .unwrap();
        let palette = colors();
        assert!(apply_templates(&config, &palette, "dark", "").is_err());
        assert!(!output.exists());

        fs::write(&input, "{{ colors.primary }}").unwrap();
        fs::write(
            &config,
            format!(
                r#"
[templates.bad]
input_path = "input.conf"
output_path = "{}"
pre_hook = "false"
"#,
                output.display()
            ),
        )
        .unwrap();
        let result = apply_templates(&config, &palette, "dark", "").unwrap();
        assert_eq!(result.applied, 0);
        assert_eq!(result.errors.len(), 1);
        assert!(!output.exists());
    }

    #[test]
    fn state_round_trips_and_requires_a_generated_source() {
        let dir = TempDir::new("state");
        let path = dir.0.join("state.json");
        let persisted = Persisted {
            mode: "dark".to_owned(),
            source: Source::Seed {
                seed: "#39bae6".to_owned(),
                variant: "tonalspot".to_owned(),
            },
            colors: colors(),
        };
        persist_to(&path, &persisted).unwrap();
        let loaded = load_from(&path).unwrap().unwrap();
        assert_eq!(loaded.mode, "dark");
        assert_eq!(
            loaded.colors["dark"]["primary"],
            persisted.colors["dark"]["primary"]
        );
        assert_eq!(loaded.source.variant(), Some("tonalspot"));

        let manual = Persisted {
            mode: "light".to_owned(),
            source: Source::Manual,
            colors: persisted.colors,
        };
        assert!(manual.source.seed().is_err());
    }

    #[test]
    fn image_sources_extract_a_seed() {
        let dir = TempDir::new("image");
        let path = dir.0.join("wallpaper.png");
        let image = image::RgbaImage::from_fn(64, 64, |x, y| {
            if (x / 8 + y / 8) % 2 == 0 {
                image::Rgba([57, 186, 230, 255])
            } else {
                image::Rgba([11, 14, 20, 255])
            }
        });
        image.save(&path).unwrap();
        let seed = seed_from_image(path.to_str().unwrap()).unwrap();
        assert!(hex(seed).starts_with('#'));
    }

    #[cfg(unix)]
    #[test]
    fn writes_preserve_destination_symlinks() {
        use std::os::unix::fs::symlink;

        let dir = TempDir::new("symlink");
        let target = dir.0.join("actual.conf");
        let link = dir.0.join("colors.conf");
        fs::write(&target, "old").unwrap();
        symlink("actual.conf", &link).unwrap();
        write_atomic(&link, "new").unwrap();
        assert_eq!(fs::read_to_string(&target).unwrap(), "new");
        assert!(
            fs::symlink_metadata(&link)
                .unwrap()
                .file_type()
                .is_symlink()
        );
    }
}
