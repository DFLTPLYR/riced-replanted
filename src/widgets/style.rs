use iced::border::Radius;
use iced::{Border, Color, Shadow, Vector};
use mlua::{Table, Value};

use crate::runtime::keys;

use super::Transition;
use super::color;

#[derive(Clone, Default)]
pub(super) struct StylePatch {
    pub(super) background: Option<Color>,
    pub(super) text_color: Option<Color>,
    pub(super) border: Option<Border>,
    pub(super) shadow: Option<Shadow>,
    pub(super) icon_color: Option<Color>,
    pub(super) dot_color: Option<Color>,
    pub(super) placeholder: Option<Color>,
    pub(super) selection: Option<Color>,
    pub(super) handle: Option<Color>,
    pub(super) foreground: Option<Color>,
    pub(super) rail: Option<Color>,
    pub(super) filled: Option<Color>,
    pub(super) scroller: Option<Color>,
    pub(super) handle_border: Option<Border>,
}

fn read(table: &Table, patch: &mut StylePatch) -> mlua::Result<()> {
    if let Some(value) = table.get::<Option<String>>("background")? {
        patch.background = Some(color(&value)?);
    }
    if let Some(value) = table.get::<Option<String>>("text_color")? {
        patch.text_color = Some(color(&value)?);
    }
    if let Some(value) = table.get::<Option<Table>>("border")? {
        patch.border = Some(border_value(&value)?);
    }
    if let Some(value) = table.get::<Option<Table>>("shadow")? {
        patch.shadow = Some(shadow_value(&value)?);
    }
    for (key, field) in [
        ("icon_color", &mut patch.icon_color),
        ("dot_color", &mut patch.dot_color),
        ("placeholder", &mut patch.placeholder),
        ("selection", &mut patch.selection),
        ("handle", &mut patch.handle),
        ("foreground", &mut patch.foreground),
        ("rail", &mut patch.rail),
        ("filled", &mut patch.filled),
        ("scroller", &mut patch.scroller),
    ] {
        if let Some(value) = table.get::<Option<String>>(key)? {
            *field = Some(color(&value)?);
        }
    }
    if let Some(value) = table.get::<Option<Table>>("handle_border")? {
        patch.handle_border = Some(border_value(&value)?);
    }
    Ok(())
}

pub(super) fn style_patch(
    base: &Table,
    over: Option<&Table>,
    transition: Option<&Transition>,
) -> mlua::Result<StylePatch> {
    let mut patch = StylePatch::default();
    for table in [Some(base), over].into_iter().flatten() {
        read(table, &mut patch)?;
    }
    if let Some(transition) = transition {
        // A state override wins so hover/press colors stay static.
        if !overridden(over, "background")?
            && patch.background.is_some()
            && let Some(value) = transition.background()
        {
            patch.background = Some(value);
        }
        if !overridden(over, "text_color")?
            && patch.text_color.is_some()
            && let Some(value) = transition.text_color()
        {
            patch.text_color = Some(value);
        }
        if !overridden(over, "border")?
            && patch.border.is_some()
            && let Some(value) = transition.border()
        {
            patch.border = Some(value);
        }
        if !overridden(over, "shadow")?
            && patch.shadow.is_some()
            && let Some(value) = transition.shadow()
        {
            patch.shadow = Some(value);
        }
    }
    Ok(patch)
}

/// Reads only the fields present in one state table, without inheriting base values.
pub(super) fn style_overlay(table: &Table) -> mlua::Result<StylePatch> {
    let mut patch = StylePatch::default();
    read(table, &mut patch)?;
    Ok(patch)
}

/// Applies animated state values over a static state patch, when enabled.
pub(super) fn animated(
    patch: &StylePatch,
    transition: Option<&Transition>,
    status: &str,
) -> StylePatch {
    let mut patch = patch.clone();
    if let Some(transition) = transition {
        transition.animate(&mut patch, status);
    }
    patch
}

fn overridden(over: Option<&Table>, key: &str) -> mlua::Result<bool> {
    match over {
        Some(over) => over.contains_key(key),
        None => Ok(false),
    }
}

/// Validates a widget style table and every per-state override.
pub(super) fn validate(style: &Table, fields: &[&str], states: &[&str]) -> mlua::Result<()> {
    let mut allowed = fields.to_vec();
    allowed.extend_from_slice(states);
    keys(style, &allowed)?;
    for state in states {
        if let Some(table) = style.get::<Option<Table>>(*state)? {
            keys(&table, fields)?;
        }
    }
    Ok(())
}

#[derive(Clone, Copy, Default)]
pub(super) struct RulePatch {
    pub(super) color: Option<Color>,
    pub(super) radius: Option<Radius>,
    pub(super) snap: Option<bool>,
}

pub(super) fn rule_patch(table: &Table) -> mlua::Result<RulePatch> {
    keys(table, &["color", "radius", "snap"])?;
    let mut patch = RulePatch::default();
    if let Some(value) = table.get::<Option<String>>("color")? {
        patch.color = Some(color(&value)?);
    }
    match table.get::<Value>("radius")? {
        Value::Nil => {}
        radius => patch.radius = Some(radius_value(radius)?),
    }
    patch.snap = table.get::<Option<bool>>("snap")?;
    Ok(patch)
}

#[derive(Clone, Copy, Default)]
pub(super) struct ProgressBarPatch {
    pub(super) background: Option<Color>,
    pub(super) bar: Option<Color>,
    pub(super) border: Option<Border>,
}

pub(super) fn progress_bar_patch(table: &Table) -> mlua::Result<ProgressBarPatch> {
    keys(table, &["background", "bar", "border"])?;
    let mut patch = ProgressBarPatch::default();
    if let Some(value) = table.get::<Option<String>>("background")? {
        patch.background = Some(color(&value)?);
    }
    if let Some(value) = table.get::<Option<String>>("bar")? {
        patch.bar = Some(color(&value)?);
    }
    if let Some(value) = table.get::<Option<Table>>("border")? {
        patch.border = Some(border_value(&value)?);
    }
    Ok(patch)
}

#[derive(Clone, Copy, Default)]
pub(super) struct QrPatch {
    pub(super) cell: Option<Color>,
    pub(super) background: Option<Color>,
}

pub(super) fn qr_patch(table: &Table) -> mlua::Result<QrPatch> {
    keys(table, &["cell", "background"])?;
    let mut patch = QrPatch::default();
    if let Some(value) = table.get::<Option<String>>("cell")? {
        patch.cell = Some(color(&value)?);
    }
    if let Some(value) = table.get::<Option<String>>("background")? {
        patch.background = Some(color(&value)?);
    }
    Ok(patch)
}

pub(super) fn radius_value(value: Value) -> mlua::Result<Radius> {
    let side = |table: &Table, key: &str| -> mlua::Result<f32> {
        let value = table.get::<Option<f32>>(key)?.unwrap_or(0.0);
        if !value.is_finite() || value < 0.0 {
            return Err(mlua::Error::runtime(format!(
                "radius {key} must be finite and non-negative"
            )));
        }
        Ok(value)
    };
    match value {
        Value::Integer(n) if n >= 0 => Ok(Radius::from(n as f32)),
        Value::Number(n) if n.is_finite() && n >= 0.0 && n <= f32::MAX as f64 => {
            Ok(Radius::from(n as f32))
        }
        Value::Table(table) => {
            keys(
                &table,
                &["top_left", "top_right", "bottom_right", "bottom_left"],
            )?;
            Ok(Radius {
                top_left: side(&table, "top_left")?,
                top_right: side(&table, "top_right")?,
                bottom_right: side(&table, "bottom_right")?,
                bottom_left: side(&table, "bottom_left")?,
            })
        }
        _ => Err(mlua::Error::runtime(
            "radius must be a finite non-negative number or {top_left, top_right, bottom_right, bottom_left}",
        )),
    }
}

fn border_value(table: &Table) -> mlua::Result<Border> {
    crate::runtime::keys(table, &["color", "width", "radius"])?;
    let mut border = Border::default();
    if let Some(value) = table.get::<Option<String>>("color")? {
        border.color = color(&value)?;
    }
    if let Some(width) = table.get::<Option<f32>>("width")? {
        if !width.is_finite() || width < 0.0 {
            return Err(mlua::Error::runtime(
                "border width must be finite and non-negative",
            ));
        }
        border.width = width;
    }
    match table.get::<Value>("radius")? {
        Value::Nil => {}
        radius => border.radius = radius_value(radius)?,
    }
    Ok(border)
}

fn shadow_value(table: &Table) -> mlua::Result<Shadow> {
    crate::runtime::keys(table, &["color", "offset", "blur"])?;
    let mut shadow = Shadow::default();
    if let Some(value) = table.get::<Option<String>>("color")? {
        shadow.color = color(&value)?;
    }
    if let Some(offset) = table.get::<Option<Table>>("offset")? {
        crate::runtime::keys(&offset, &["x", "y"])?;
        shadow.offset = Vector::new(
            offset.get::<Option<f32>>("x")?.unwrap_or(0.0),
            offset.get::<Option<f32>>("y")?.unwrap_or(0.0),
        );
    }
    if let Some(blur) = table.get::<Option<f32>>("blur")? {
        if !blur.is_finite() || blur < 0.0 {
            return Err(mlua::Error::runtime(
                "shadow blur must be finite and non-negative",
            ));
        }
        shadow.blur_radius = blur;
    }
    Ok(shadow)
}
