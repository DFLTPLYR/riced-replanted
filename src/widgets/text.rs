use iced::Element;
use iced::widget;
use mlua::{Table, Value};

use crate::runtime::Message;

use super::enums::{font_value, line_height, shaping, text_align, vertical, wrapping};
use super::{Layout, Transition, color, number};

pub(super) fn build(
    node: &Table,
    transition: Option<&Transition>,
    layout: Layout,
) -> mlua::Result<Element<'static, Message>> {
    let size = match transition.and_then(|t| t.value("size")) {
        Some(size) => size,
        None => number(node, "size", 16.0)?,
    };
    let content = node.get::<String>("content").map_err(|_| {
        mlua::Error::runtime(
            "text content must be a string or number; use item.name for list items",
        )
    })?;
    let mut text = widget::text(content).size(size);
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
    if let Some(value) = node.get::<Option<Value>>("font")? {
        text = text.font(font_value(value)?);
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
