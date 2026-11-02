use iced::{Border, Color, Shadow, Vector};
use mlua::Table;

use crate::runtime::keys;

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

pub(super) fn style_patch(base: &Table, over: Option<&Table>) -> mlua::Result<StylePatch> {
    let mut patch = StylePatch::default();
    for table in [Some(base), over].into_iter().flatten() {
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
    }
    Ok(patch)
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
    pub(super) radius: Option<f32>,
    pub(super) snap: Option<bool>,
}

pub(super) fn rule_patch(table: &Table) -> mlua::Result<RulePatch> {
    keys(table, &["color", "radius", "snap"])?;
    let mut patch = RulePatch::default();
    if let Some(value) = table.get::<Option<String>>("color")? {
        patch.color = Some(color(&value)?);
    }
    if let Some(radius) = table.get::<Option<f32>>("radius")? {
        if !radius.is_finite() || radius < 0.0 {
            return Err(mlua::Error::runtime(
                "rule radius must be finite and non-negative",
            ));
        }
        patch.radius = Some(radius);
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
    if let Some(radius) = table.get::<Option<f32>>("radius")? {
        if !radius.is_finite() || radius < 0.0 {
            return Err(mlua::Error::runtime(
                "border radius must be finite and non-negative",
            ));
        }
        border.radius = radius.into();
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
