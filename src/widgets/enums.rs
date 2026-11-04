use iced::ContentFit;
use iced::widget::text::{Alignment as TextAlignment, LineHeight, Shaping, Wrapping};
use iced::{Pixels, alignment};
use mlua::Value;

pub(super) fn text_align(value: &str) -> mlua::Result<TextAlignment> {
    match value {
        "left" => Ok(TextAlignment::Left),
        "center" => Ok(TextAlignment::Center),
        "right" => Ok(TextAlignment::Right),
        "justified" => Ok(TextAlignment::Justified),
        _ => Err(mlua::Error::runtime(format!(
            "unknown text align {value:?}; use left, center, right, or justified"
        ))),
    }
}

pub(super) fn horizontal(value: &str) -> mlua::Result<alignment::Horizontal> {
    match value {
        "left" => Ok(alignment::Horizontal::Left),
        "center" => Ok(alignment::Horizontal::Center),
        "right" => Ok(alignment::Horizontal::Right),
        _ => Err(mlua::Error::runtime(format!(
            "unknown horizontal align {value:?}; use left, center, or right"
        ))),
    }
}

pub(super) fn vertical(value: &str) -> mlua::Result<alignment::Vertical> {
    match value {
        "top" => Ok(alignment::Vertical::Top),
        "center" => Ok(alignment::Vertical::Center),
        "bottom" => Ok(alignment::Vertical::Bottom),
        _ => Err(mlua::Error::runtime(format!(
            "unknown vertical align {value:?}; use top, center, or bottom"
        ))),
    }
}

pub(super) fn wrapping(value: Value) -> mlua::Result<Wrapping> {
    match value {
        Value::Boolean(true) => Ok(Wrapping::Word),
        Value::Boolean(false) => Ok(Wrapping::None),
        Value::String(text) => {
            let text = text.to_str()?;
            match &*text {
                "none" => Ok(Wrapping::None),
                "word" => Ok(Wrapping::Word),
                "glyph" => Ok(Wrapping::Glyph),
                "word_or_glyph" => Ok(Wrapping::WordOrGlyph),
                _ => Err(mlua::Error::runtime(format!(
                    "unknown wrap {text:?}; use none, word, glyph, or word_or_glyph"
                ))),
            }
        }
        _ => Err(mlua::Error::runtime(
            "wrap must be a boolean or none, word, glyph, word_or_glyph",
        )),
    }
}

pub(super) fn line_height(value: Value) -> mlua::Result<LineHeight> {
    match value {
        Value::Integer(n) if n > 0 => Ok(LineHeight::Relative(n as f32)),
        Value::Number(n) if n.is_finite() && n > 0.0 => Ok(LineHeight::Relative(n as f32)),
        Value::Table(table) => {
            crate::runtime::keys(&table, &["pixels"])?;
            let pixels: f32 = table.get("pixels")?;
            if !pixels.is_finite() || pixels <= 0.0 {
                return Err(mlua::Error::runtime(
                    "line_height.pixels must be finite and positive",
                ));
            }
            Ok(LineHeight::Absolute(Pixels(pixels)))
        }
        _ => Err(mlua::Error::runtime(
            "line_height must be a positive factor or {pixels = n}",
        )),
    }
}

pub(super) fn shaping(value: &str) -> mlua::Result<Shaping> {
    match value {
        "auto" => Ok(Shaping::Auto),
        "basic" => Ok(Shaping::Basic),
        "advanced" => Ok(Shaping::Advanced),
        _ => Err(mlua::Error::runtime(format!(
            "unknown shaping {value:?}; use auto, basic, or advanced"
        ))),
    }
}

pub(super) fn font_value(value: Value) -> mlua::Result<iced::Font> {
    use iced::font::{Family, Stretch, Style as FontStyle, Weight};

    fn family(name: &str) -> mlua::Result<Family> {
        Ok(match name {
            "sans_serif" | "sans-serif" => Family::SansSerif,
            "serif" => Family::Serif,
            "monospace" | "mono" => Family::Monospace,
            "cursive" => Family::Cursive,
            "fantasy" => Family::Fantasy,
            _ => Family::Name(intern_font(name)?),
        })
    }

    fn weight(name: &str) -> mlua::Result<Weight> {
        match name {
            "thin" => Ok(Weight::Thin),
            "extra_light" => Ok(Weight::ExtraLight),
            "light" => Ok(Weight::Light),
            "normal" => Ok(Weight::Normal),
            "medium" => Ok(Weight::Medium),
            "semibold" | "semi_bold" => Ok(Weight::Semibold),
            "bold" => Ok(Weight::Bold),
            "extra_bold" => Ok(Weight::ExtraBold),
            "black" => Ok(Weight::Black),
            _ => Err(mlua::Error::runtime(format!(
                "unknown font weight {name:?}"
            ))),
        }
    }

    fn stretch(name: &str) -> mlua::Result<Stretch> {
        match name {
            "ultra_condensed" => Ok(Stretch::UltraCondensed),
            "extra_condensed" => Ok(Stretch::ExtraCondensed),
            "condensed" => Ok(Stretch::Condensed),
            "semi_condensed" => Ok(Stretch::SemiCondensed),
            "normal" => Ok(Stretch::Normal),
            "semi_expanded" => Ok(Stretch::SemiExpanded),
            "expanded" => Ok(Stretch::Expanded),
            "extra_expanded" => Ok(Stretch::ExtraExpanded),
            "ultra_expanded" => Ok(Stretch::UltraExpanded),
            _ => Err(mlua::Error::runtime(format!(
                "unknown font stretch {name:?}"
            ))),
        }
    }

    match value {
        Value::String(name) => {
            let name = name.to_str()?;
            Ok(iced::Font {
                family: family(&name)?,
                ..iced::Font::DEFAULT
            })
        }
        Value::Table(table) => {
            crate::runtime::keys(&table, &["family", "weight", "style", "stretch"])?;
            let mut font = iced::Font::DEFAULT;
            if let Some(name) = table.get::<Option<String>>("family")? {
                font.family = family(&name)?;
            }
            if let Some(name) = table.get::<Option<String>>("weight")? {
                font.weight = weight(&name)?;
            }
            if let Some(name) = table.get::<Option<String>>("style")? {
                font.style = match name.as_str() {
                    "normal" => FontStyle::Normal,
                    "italic" => FontStyle::Italic,
                    "oblique" => FontStyle::Oblique,
                    other => {
                        return Err(mlua::Error::runtime(format!(
                            "unknown font style {other:?}; use normal, italic, or oblique"
                        )));
                    }
                };
            }
            if let Some(name) = table.get::<Option<String>>("stretch")? {
                font.stretch = stretch(&name)?;
            }
            Ok(font)
        }
        _ => Err(mlua::Error::runtime(
            "font must be a family name or {family, weight, style, stretch}",
        )),
    }
}

fn intern_font(name: &str) -> mlua::Result<&'static str> {
    use std::collections::HashSet;
    use std::sync::{Mutex, OnceLock};

    static NAMES: OnceLock<Mutex<HashSet<&'static str>>> = OnceLock::new();
    let mut names = NAMES
        .get_or_init(|| Mutex::new(HashSet::new()))
        .lock()
        .unwrap();
    if let Some(name) = names.get(name) {
        return Ok(name);
    }
    // Iced requires static family names; bound the process-lifetime allocation.
    if names.len() >= 256 || name.len() > 1024 {
        return Err(mlua::Error::runtime(
            "font family limit: 256 distinct names, 1024 bytes each",
        ));
    }
    let leaked: &'static str = Box::leak(name.to_owned().into_boxed_str());
    names.insert(leaked);
    Ok(leaked)
}

pub(super) fn content_fit(value: &str) -> mlua::Result<ContentFit> {
    match value {
        "contain" => Ok(ContentFit::Contain),
        "cover" => Ok(ContentFit::Cover),
        "fill" => Ok(ContentFit::Fill),
        "none" => Ok(ContentFit::None),
        "scale_down" => Ok(ContentFit::ScaleDown),
        _ => Err(mlua::Error::runtime(format!(
            "unknown content_fit {value:?}; use contain, cover, fill, none, or scale_down"
        ))),
    }
}
