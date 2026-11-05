use iced::Element;
use iced::widget;
use mlua::{Table, Value};

use crate::runtime::Message;

use super::enums::{font_value, line_height, shaping, text_align, vertical, wrapping};
use super::{Layout, Transition, color, number};

fn styled(
    node: &Table,
    transition: Option<&Transition>,
    layout: Layout,
    content: String,
    font: Option<iced::Font>,
) -> mlua::Result<Element<'static, Message>> {
    let size = match transition.and_then(|t| t.value("size")) {
        Some(size) => size,
        None => number(node, "size", 16.0)?,
    };
    if !(0.0 < size && size <= f32::MAX) {
        return Err(mlua::Error::runtime("size must be positive"));
    }
    let mut text = widget::text(content).size(size);
    if let Some(font) = font {
        text = text.font(font);
    }
    if let Some(value) = node.get::<Option<String>>("color")? {
        let color = transition.and_then(|t| t.color()).unwrap_or(color(&value)?);
        text = text.color(color);
    }
    if let Some(value) = node.get::<Option<Value>>("line_height")? {
        text = text.line_height(line_height(value)?);
    }
    if let Some(value) = node.get::<Option<Value>>("wrap")? {
        text = text.wrapping(wrapping(value)?);
    }
    if let Some(value) = node.get::<Option<String>>("shaping")? {
        text = text.shaping(shaping(&value)?);
    }
    if node.get::<Option<bool>>("center")?.unwrap_or(false) {
        text = text.center();
    }
    let align_x = node.get::<Option<String>>("align_x")?;
    let align_y = node.get::<Option<String>>("align_y")?;
    if let Some(width) = layout.width {
        text = text.width(width);
    }
    if let Some(height) = layout.height {
        text = text.height(height);
    }
    if let Some(value) = &align_x {
        text = text.align_x(text_align(value)?);
    }
    if let Some(value) = &align_y {
        text = text.align_y(vertical(value)?);
    }
    Ok(text.into())
}

pub(super) fn icon(
    node: &Table,
    transition: Option<&Transition>,
    layout: Layout,
) -> mlua::Result<Element<'static, Message>> {
    let glyph = node.get::<String>("content")?;
    if glyph.is_empty() {
        return Err(mlua::Error::runtime("icon glyph must not be empty"));
    }
    if glyph.chars().count() != 1 {
        return Err(mlua::Error::runtime(
            "icon content must be a single unicode char (use riced.plugins(\"lucide\").resolve(name) to get it)",
        ));
    }
    styled(
        node,
        transition,
        layout,
        glyph,
        Some(iced::Font::with_name("lucide")),
    )
}

pub(super) fn build(
    node: &Table,
    transition: Option<&Transition>,
    layout: Layout,
) -> mlua::Result<Element<'static, Message>> {
    // Glyph is a char; text can be any string.
    let content = node.get::<String>("content").map_err(|_| {
        mlua::Error::runtime(
            "text content must be a string or number; use item.name for list items",
        )
    })?;
    let font = node
        .get::<Option<Value>>("font")?
        .map(font_value)
        .transpose()?;
    styled(node, transition, layout, content, font)
}
