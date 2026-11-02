use iced::Background;
use iced::Element;
use iced::widget;
use mlua::{Table, Value};

use crate::runtime::Message;

use super::enums::{horizontal, vertical};
use super::style::style_patch;
use super::{Ctx, Layout, children, element, max_pixels};

pub(super) fn container(
    node: &Table,
    ctx: &Ctx,
    layout: Layout,
) -> mlua::Result<Element<'static, Message>> {
    let padding = layout.padding;
    let content = element(node.get::<Table>("content")?, ctx)?;
    let mut container = widget::container(content).padding(padding);
    if let Some(width) = layout.width {
        container = container.width(width);
    }
    if let Some(height) = layout.height {
        container = container.height(height);
    }
    if let Some(align) = node.get::<Option<String>>("align_x")? {
        container = container.align_x(horizontal(&align)?);
    }
    if let Some(align) = node.get::<Option<String>>("align_y")? {
        container = container.align_y(vertical(&align)?);
    }
    if let Some(max) = max_pixels(node, "max_width")? {
        container = container.max_width(max);
    }
    if let Some(max) = max_pixels(node, "max_height")? {
        container = container.max_height(max);
    }
    if let Some(clip) = node.get::<Option<bool>>("clip")? {
        container = container.clip(clip);
    }
    if let Some(style) = node.get::<Option<Table>>("style")? {
        crate::runtime::keys(&style, &["background", "text_color", "border", "shadow"])?;
        let patch = style_patch(&style, None)?;
        container = container.style(move |_theme| widget::container::Style {
            background: patch.background.map(Background::Color),
            text_color: patch.text_color,
            border: patch.border.unwrap_or_default(),
            shadow: patch.shadow.unwrap_or_default(),
            ..widget::container::Style::default()
        });
    }
    Ok(container.into())
}

pub(super) fn button(
    node: &Table,
    ctx: &Ctx,
    layout: Layout,
) -> mlua::Result<Element<'static, Message>> {
    let content = match node.get::<Value>("content")? {
        Value::String(s) => widget::text(s.to_str()?.to_owned()).into(),
        Value::Table(t) => element(t, ctx)?,
        _ => {
            return Err(mlua::Error::runtime(
                "button content must be text or an iced element",
            ));
        }
    };
    let mut button = widget::button(content);
    if let Some(width) = layout.width {
        button = button.width(width);
    }
    if let Some(height) = layout.height {
        button = button.height(height);
    }
    if let Some(clip) = node.get::<Option<bool>>("clip")? {
        button = button.clip(clip);
    }
    let event: Value = node.get("on_press")?;
    if !event.is_nil() {
        button = button.on_press(Message::Lua(ctx.generation, ctx.component.clone(), event));
    }
    if let Some(style) = node.get::<Option<Table>>("style")? {
        crate::runtime::keys(
            &style,
            &[
                "background",
                "text_color",
                "border",
                "shadow",
                "active",
                "hovered",
                "pressed",
                "disabled",
            ],
        )?;
        let active = style_patch(&style, style.get::<Option<Table>>("active")?.as_ref())?;
        let hovered = style_patch(&style, style.get::<Option<Table>>("hovered")?.as_ref())?;
        let pressed = style_patch(&style, style.get::<Option<Table>>("pressed")?.as_ref())?;
        let disabled = style_patch(&style, style.get::<Option<Table>>("disabled")?.as_ref())?;
        button = button.style(move |theme, status| {
            let patch = match status {
                widget::button::Status::Active => &active,
                widget::button::Status::Hovered => &hovered,
                widget::button::Status::Pressed => &pressed,
                widget::button::Status::Disabled => &disabled,
            };
            let mut style = widget::button::primary(theme, status);
            if let Some(background) = patch.background {
                style.background = Some(Background::Color(background));
            }
            if let Some(text_color) = patch.text_color {
                style.text_color = text_color;
            }
            if let Some(border) = patch.border {
                style.border = border;
            }
            if let Some(shadow) = patch.shadow {
                style.shadow = shadow;
            }
            style
        });
    }
    Ok(button.into())
}

pub(super) fn stack(
    node: &Table,
    ctx: &Ctx,
    layout: Layout,
) -> mlua::Result<Element<'static, Message>> {
    let mut stack = widget::Stack::with_children(children(node, ctx)?);
    if let Some(width) = layout.width {
        stack = stack.width(width);
    }
    if let Some(height) = layout.height {
        stack = stack.height(height);
    }
    if let Some(clip) = node.get::<Option<bool>>("clip")? {
        stack = stack.clip(clip);
    }
    Ok(stack.into())
}

pub(super) fn float(node: &Table, ctx: &Ctx) -> mlua::Result<Element<'static, Message>> {
    let content = element(node.get::<Table>("content")?, ctx)?;
    let mut float = widget::Float::new(content);
    if let Some(scale) = node.get::<Option<f32>>("scale")? {
        if !scale.is_finite() || scale <= 0.0 {
            return Err(mlua::Error::runtime(
                "float scale must be finite and positive",
            ));
        }
        float = float.scale(scale);
    }
    Ok(float.into())
}

pub(super) fn pin(
    node: &Table,
    ctx: &Ctx,
    layout: Layout,
) -> mlua::Result<Element<'static, Message>> {
    let content = element(node.get::<Table>("content")?, ctx)?;
    let mut pin = widget::Pin::new(content);
    if let Some(width) = layout.width {
        pin = pin.width(width);
    }
    if let Some(height) = layout.height {
        pin = pin.height(height);
    }
    if let Some(x) = node.get::<Option<f32>>("x")? {
        pin = pin.x(x);
    }
    if let Some(y) = node.get::<Option<f32>>("y")? {
        pin = pin.y(y);
    }
    Ok(pin.into())
}

pub(super) fn tooltip(node: &Table, ctx: &Ctx) -> mlua::Result<Element<'static, Message>> {
    let content = element(node.get::<Table>("content")?, ctx)?;
    let tooltip = node.get::<String>("tooltip")?;
    let position = match node.get::<Option<String>>("position")?.as_deref() {
        None | Some("top") => widget::tooltip::Position::Top,
        Some("bottom") => widget::tooltip::Position::Bottom,
        Some("left") => widget::tooltip::Position::Left,
        Some("right") => widget::tooltip::Position::Right,
        Some("follow_cursor") => widget::tooltip::Position::FollowCursor,
        Some(other) => {
            return Err(mlua::Error::runtime(format!(
                "unknown tooltip position {other:?}; use top, bottom, left, right, or follow_cursor"
            )));
        }
    };
    let mut tip = widget::Tooltip::new(content, widget::text(tooltip), position);
    if let Some(gap) = node.get::<Option<f32>>("gap")? {
        tip = tip.gap(gap);
    }
    Ok(tip.into())
}
