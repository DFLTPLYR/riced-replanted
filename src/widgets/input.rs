use iced::Element;
use iced::widget;
use mlua::{Table, Value};

use crate::runtime::{Message, WidgetEvent};

use super::enums::{font_value, horizontal, intern_font};
use super::style::{style_patch, validate};
use super::{Ctx, Layout, element, number};

fn checkbox_patch(style: &Table, key: &str) -> mlua::Result<super::style::StylePatch> {
    style_patch(style, style.get::<Option<Table>>(key)?.as_ref())
}

fn paint_checkbox(style: &mut widget::checkbox::Style, patch: &super::style::StylePatch) {
    if let Some(color) = patch.background {
        style.background = iced::Background::Color(color);
    }
    if let Some(color) = patch.icon_color {
        style.icon_color = color;
    }
    if let Some(border) = patch.border {
        style.border = border;
    }
    if let Some(color) = patch.text_color {
        style.text_color = Some(color);
    }
}

fn paint_radio(style: &mut widget::radio::Style, patch: &super::style::StylePatch) {
    if let Some(color) = patch.background {
        style.background = iced::Background::Color(color);
    }
    if let Some(color) = patch.dot_color {
        style.dot_color = color;
    }
    if let Some(border) = patch.border {
        style.border_color = border.color;
        style.border_width = border.width;
    }
    if let Some(color) = patch.text_color {
        style.text_color = Some(color);
    }
}

fn paint_toggler(style: &mut widget::toggler::Style, patch: &super::style::StylePatch) {
    if let Some(color) = patch.background {
        style.background = iced::Background::Color(color);
    }
    if let Some(color) = patch.foreground {
        style.foreground = iced::Background::Color(color);
    }
    if let Some(border) = patch.border {
        style.background_border_color = border.color;
        style.background_border_width = border.width;
        style.border_radius = Some(border.radius);
    }
    if let Some(color) = patch.text_color {
        style.text_color = Some(color);
    }
}

fn paint_slider(style: &mut widget::slider::Style, patch: &super::style::StylePatch) {
    if let Some(color) = patch.rail {
        style.rail.backgrounds.0 = iced::Background::Color(color);
    }
    if let Some(color) = patch.filled {
        style.rail.backgrounds.1 = iced::Background::Color(color);
    }
    if let Some(border) = patch.border {
        style.rail.border = border;
    }
    if let Some(color) = patch.handle {
        style.handle.background = iced::Background::Color(color);
    }
    if let Some(border) = patch.handle_border {
        style.handle.border_color = border.color;
        style.handle.border_width = border.width;
    }
}

pub(super) fn paint_text_input(
    style: &mut widget::text_input::Style,
    patch: &super::style::StylePatch,
) {
    if let Some(color) = patch.background {
        style.background = iced::Background::Color(color);
    }
    if let Some(border) = patch.border {
        style.border = border;
    }
    if let Some(color) = patch.text_color {
        style.value = color;
    }
    if let Some(color) = patch.placeholder {
        style.placeholder = color;
    }
    if let Some(color) = patch.selection {
        style.selection = color;
    }
    if let Some(color) = patch.icon_color {
        style.icon = color;
    }
}

fn paint_pick_list(style: &mut widget::pick_list::Style, patch: &super::style::StylePatch) {
    if let Some(color) = patch.background {
        style.background = iced::Background::Color(color);
    }
    if let Some(border) = patch.border {
        style.border = border;
    }
    if let Some(color) = patch.text_color {
        style.text_color = color;
    }
    if let Some(color) = patch.placeholder {
        style.placeholder_color = color;
    }
    if let Some(color) = patch.handle {
        style.handle_color = color;
    }
}

fn paint_scrollable(style: &mut widget::scrollable::Style, patch: &super::style::StylePatch) {
    if let Some(color) = patch.background {
        style.container.background = Some(iced::Background::Color(color));
    }
    if let Some(border) = patch.border {
        style.container.border = border;
    }
    if let Some(shadow) = patch.shadow {
        style.container.shadow = shadow;
    }
    if let Some(color) = patch.rail {
        style.vertical_rail.background = Some(iced::Background::Color(color));
        style.horizontal_rail.background = Some(iced::Background::Color(color));
    }
    if let Some(color) = patch.scroller {
        style.vertical_rail.scroller.background = iced::Background::Color(color);
        style.horizontal_rail.scroller.background = iced::Background::Color(color);
    }
}

pub(super) fn checkbox(
    node: &Table,
    ctx: &Ctx,
    layout: Layout,
) -> mlua::Result<Element<'static, Message>> {
    let mut checkbox = widget::Checkbox::new(node.get::<Option<bool>>("value")?.unwrap_or(false));
    if let Some(width) = layout.width {
        checkbox = checkbox.width(width);
    }
    if let Some(label) = node.get::<Option<String>>("label")? {
        checkbox = checkbox.label(label);
    }
    if let Some(size) = node.get::<Option<f32>>("text_size")? {
        if !size.is_finite() || size <= 0.0 {
            return Err(mlua::Error::runtime(
                "text_size must be finite and positive",
            ));
        }
        checkbox = checkbox.text_size(size);
    }
    if let Some(style) = node.get::<Option<Table>>("style")? {
        const FIELDS: &[&str] = &["background", "icon_color", "border", "text_color"];
        validate(&style, FIELDS, &["active", "hovered", "disabled"])?;
        let active = checkbox_patch(&style, "active")?;
        let hovered = checkbox_patch(&style, "hovered")?;
        let disabled = checkbox_patch(&style, "disabled")?;
        checkbox = checkbox.style(move |theme, status| {
            let mut out = widget::checkbox::primary(theme, status);
            let patch = match status {
                widget::checkbox::Status::Active { .. } => &active,
                widget::checkbox::Status::Hovered { .. } => &hovered,
                widget::checkbox::Status::Disabled { .. } => &disabled,
            };
            paint_checkbox(&mut out, patch);
            out
        });
    }
    if let Some(event) = node.get::<Option<String>>("on_toggle")? {
        let generation = ctx.generation;
        let component = ctx.component.clone();
        checkbox = checkbox.on_toggle(move |value| {
            Message::Widget(
                generation,
                component.clone(),
                event.clone(),
                WidgetEvent::Bool(value),
            )
        });
    }
    Ok(checkbox.into())
}

pub(super) fn radio(
    node: &Table,
    ctx: &Ctx,
    layout: Layout,
) -> mlua::Result<Element<'static, Message>> {
    let value = node.get::<Option<u32>>("value")?.unwrap_or(0) as usize;
    let selected = node.get::<Option<u32>>("selected")?.map(|v| v as usize);
    let label = node.get::<Option<String>>("label")?.unwrap_or_default();
    let event = node.get::<Option<String>>("on_select")?.unwrap_or_default();
    let generation = ctx.generation;
    let component = ctx.component.clone();
    let mut radio = widget::Radio::new(label, value, selected, move |value| {
        Message::Widget(
            generation,
            component.clone(),
            event.clone(),
            WidgetEvent::Number(value as f32),
        )
    });
    if let Some(width) = layout.width {
        radio = radio.width(width);
    }
    if let Some(size) = node.get::<Option<f32>>("text_size")? {
        if !size.is_finite() || size <= 0.0 {
            return Err(mlua::Error::runtime(
                "text_size must be finite and positive",
            ));
        }
        radio = radio.text_size(size);
    }
    if let Some(style) = node.get::<Option<Table>>("style")? {
        const FIELDS: &[&str] = &["background", "dot_color", "border", "text_color"];
        validate(&style, FIELDS, &["active", "hovered"])?;
        let active = checkbox_patch(&style, "active")?;
        let hovered = checkbox_patch(&style, "hovered")?;
        radio = radio.style(move |theme, status| {
            let mut out = widget::radio::default(theme, status);
            let patch = match status {
                widget::radio::Status::Active { .. } => &active,
                widget::radio::Status::Hovered { .. } => &hovered,
            };
            paint_radio(&mut out, patch);
            out
        });
    }
    Ok(radio.into())
}

pub(super) fn toggler(
    node: &Table,
    ctx: &Ctx,
    layout: Layout,
) -> mlua::Result<Element<'static, Message>> {
    let mut toggler = widget::Toggler::new(node.get::<Option<bool>>("value")?.unwrap_or(false));
    if let Some(width) = layout.width {
        toggler = toggler.width(width);
    }
    if let Some(label) = node.get::<Option<String>>("label")? {
        toggler = toggler.label(label);
    }
    if let Some(size) = node.get::<Option<f32>>("text_size")? {
        if !size.is_finite() || size <= 0.0 {
            return Err(mlua::Error::runtime(
                "text_size must be finite and positive",
            ));
        }
        toggler = toggler.text_size(size);
    }
    if let Some(style) = node.get::<Option<Table>>("style")? {
        const FIELDS: &[&str] = &["background", "foreground", "border", "text_color"];
        validate(&style, FIELDS, &["active", "hovered", "disabled"])?;
        let active = checkbox_patch(&style, "active")?;
        let hovered = checkbox_patch(&style, "hovered")?;
        let disabled = checkbox_patch(&style, "disabled")?;
        toggler = toggler.style(move |theme, status| {
            let mut out = widget::toggler::default(theme, status);
            let patch = match status {
                widget::toggler::Status::Active { .. } => &active,
                widget::toggler::Status::Hovered { .. } => &hovered,
                widget::toggler::Status::Disabled { .. } => &disabled,
            };
            paint_toggler(&mut out, patch);
            out
        });
    }
    if let Some(event) = node.get::<Option<String>>("on_toggle")? {
        let generation = ctx.generation;
        let component = ctx.component.clone();
        toggler = toggler.on_toggle(move |value| {
            Message::Widget(
                generation,
                component.clone(),
                event.clone(),
                WidgetEvent::Bool(value),
            )
        });
    }
    Ok(toggler.into())
}

pub(super) fn slider(
    node: &Table,
    ctx: &Ctx,
    kind: &str,
    layout: Layout,
) -> mlua::Result<Element<'static, Message>> {
    let min = node.get::<Option<f32>>("min")?.unwrap_or(0.0);
    let max = node.get::<Option<f32>>("max")?.unwrap_or(1.0);
    if !min.is_finite() || !max.is_finite() || min >= max {
        return Err(mlua::Error::runtime("slider requires finite min < max"));
    }
    let value = node
        .get::<Option<f32>>("value")?
        .unwrap_or(min)
        .clamp(min, max);
    let event = node.get::<Option<String>>("on_change")?.unwrap_or_default();
    let release = node.get::<Option<String>>("on_release")?;
    let step = node.get::<Option<f32>>("step")?;
    if let Some(step) = step
        && (!step.is_finite() || step <= 0.0)
    {
        return Err(mlua::Error::runtime("step must be finite and positive"));
    }
    let styles = match node.get::<Option<Table>>("style")? {
        Some(style) => {
            const FIELDS: &[&str] = &["rail", "filled", "border", "handle", "handle_border"];
            validate(&style, FIELDS, &["active", "hovered", "dragged"])?;
            Some((
                checkbox_patch(&style, "active")?,
                checkbox_patch(&style, "hovered")?,
                checkbox_patch(&style, "dragged")?,
            ))
        }
        None => None,
    };
    let generation = ctx.generation;
    let component = ctx.component.clone();
    Ok(if kind == "slider" {
        let mut slider = {
            let component = component.clone();
            let event = event.clone();
            widget::Slider::new(min..=max, value, move |value: f32| {
                Message::Widget(
                    generation,
                    component.clone(),
                    event.clone(),
                    WidgetEvent::Number(value),
                )
            })
        };
        if let Some(width) = layout.width {
            slider = slider.width(width);
        }
        if let Some(step) = step {
            slider = slider.step(step);
        }
        if let Some((active, hovered, dragged)) = styles.clone() {
            slider = slider.style(move |theme, status| {
                let mut out = widget::slider::default(theme, status);
                let patch = match status {
                    widget::slider::Status::Active => &active,
                    widget::slider::Status::Hovered => &hovered,
                    widget::slider::Status::Dragged => &dragged,
                };
                paint_slider(&mut out, patch);
                out
            });
        }
        if let Some(release) = release {
            slider = slider.on_release(Message::Widget(
                generation,
                component,
                release,
                WidgetEvent::Number(value),
            ));
        }
        slider.into()
    } else {
        let mut slider = {
            let component = component.clone();
            let event = event.clone();
            widget::VerticalSlider::new(min..=max, value, move |value: f32| {
                Message::Widget(
                    generation,
                    component.clone(),
                    event.clone(),
                    WidgetEvent::Number(value),
                )
            })
        };
        if let Some(height) = layout.height {
            slider = slider.height(height);
        }
        if let Some(step) = step {
            slider = slider.step(step);
        }
        if let Some((active, hovered, dragged)) = styles.clone() {
            slider = slider.style(move |theme, status| {
                let mut out = widget::slider::default(theme, status);
                let patch = match status {
                    widget::slider::Status::Active => &active,
                    widget::slider::Status::Hovered => &hovered,
                    widget::slider::Status::Dragged => &dragged,
                };
                paint_slider(&mut out, patch);
                out
            });
        }
        if let Some(release) = release {
            slider = slider.on_release(Message::Widget(
                generation,
                component,
                release,
                WidgetEvent::Number(value),
            ));
        }
        slider.into()
    })
}

pub(super) fn mouse_area(node: &Table, ctx: &Ctx) -> mlua::Result<Element<'static, Message>> {
    let content = element(node.get::<Table>("content")?, ctx)?;
    let mut area = widget::MouseArea::new(content);
    let generation = ctx.generation;
    let component = ctx.component.clone();
    for (key, set) in [
        ("on_press", 0usize),
        ("on_release", 1),
        ("on_double_click", 2),
        ("on_right_press", 3),
        ("on_right_release", 4),
        ("on_middle_press", 5),
        ("on_middle_release", 6),
        ("on_enter", 7),
        ("on_exit", 8),
    ] {
        if let Some(value) = node.get::<Option<Value>>(key)? {
            let message = Message::Lua(generation, component.clone(), value);
            area = match set {
                0 => area.on_press(message),
                1 => area.on_release(message),
                2 => area.on_double_click(message),
                3 => area.on_right_press(message),
                4 => area.on_right_release(message),
                5 => area.on_middle_press(message),
                6 => area.on_middle_release(message),
                7 => area.on_enter(message),
                _ => area.on_exit(message),
            };
        }
    }
    if let Some(event) = node.get::<Option<String>>("on_move")? {
        let component = component.clone();
        area = area.on_move(move |point| {
            Message::Widget(
                generation,
                component.clone(),
                event.clone(),
                WidgetEvent::Point(point.x, point.y),
            )
        });
    }
    if let Some(event) = node.get::<Option<String>>("on_scroll")? {
        let component = component.clone();
        area = area.on_scroll(move |delta| {
            let (x, y) = match delta {
                iced::mouse::ScrollDelta::Lines { x, y } => (x, y),
                iced::mouse::ScrollDelta::Pixels { x, y } => (x, y),
            };
            Message::Widget(
                generation,
                component.clone(),
                event.clone(),
                WidgetEvent::Point(x, y),
            )
        });
    }
    Ok(area.into())
}

pub(super) fn sensor(node: &Table, ctx: &Ctx) -> mlua::Result<Element<'static, Message>> {
    let content = element(node.get::<Table>("content")?, ctx)?;
    let mut sensor = widget::Sensor::new(content);
    let generation = ctx.generation;
    let component = ctx.component.clone();
    if let Some(event) = node.get::<Option<String>>("on_show")? {
        let component = component.clone();
        sensor = sensor.on_show(move |size| {
            Message::Widget(
                generation,
                component.clone(),
                event.clone(),
                WidgetEvent::Size(size.width, size.height),
            )
        });
    }
    if let Some(event) = node.get::<Option<String>>("on_resize")? {
        let component = component.clone();
        sensor = sensor.on_resize(move |size| {
            Message::Widget(
                generation,
                component.clone(),
                event.clone(),
                WidgetEvent::Size(size.width, size.height),
            )
        });
    }
    if let Some(value) = node.get::<Option<Value>>("on_hide")? {
        sensor = sensor.on_hide(Message::Lua(generation, component, value));
    }
    Ok(sensor.into())
}

pub(super) fn scrollable(
    node: &Table,
    ctx: &Ctx,
    layout: Layout,
) -> mlua::Result<Element<'static, Message>> {
    let content = element(node.get::<Table>("content")?, ctx)?;
    let mut scrollable = widget::Scrollable::new(content);
    if let Some(width) = layout.width {
        scrollable = scrollable.width(width);
    }
    if let Some(height) = layout.height {
        scrollable = scrollable.height(height);
    }
    if let Some(direction) = node.get::<Option<String>>("direction")? {
        let direction = match direction.as_str() {
            "vertical" => {
                widget::scrollable::Direction::Vertical(widget::scrollable::Scrollbar::default())
            }
            "horizontal" => {
                widget::scrollable::Direction::Horizontal(widget::scrollable::Scrollbar::default())
            }
            "both" => widget::scrollable::Direction::Both {
                vertical: widget::scrollable::Scrollbar::default(),
                horizontal: widget::scrollable::Scrollbar::default(),
            },
            other => {
                return Err(mlua::Error::runtime(format!(
                    "unknown scroll direction {other:?}; use vertical, horizontal, or both"
                )));
            }
        };
        scrollable = scrollable.direction(direction);
    }
    scrollable = scrollable.spacing(number(node, "spacing", 0.0)?);
    if let Some(auto_scroll) = node.get::<Option<bool>>("auto_scroll")? {
        scrollable = scrollable.auto_scroll(auto_scroll);
    }
    if let Some(id) = node.get::<Option<String>>("id")? {
        scrollable = scrollable.id(widget::Id::new(intern_font(&id)));
    }
    if let Some(event) = node.get::<Option<String>>("on_scroll")? {
        let generation = ctx.generation;
        let component = ctx.component.clone();
        scrollable = scrollable.on_scroll(move |viewport| {
            let absolute = viewport.absolute_offset();
            let relative = viewport.relative_offset();
            let bounds = viewport.bounds();
            let content = viewport.content_bounds();
            Message::Widget(
                generation,
                component.clone(),
                event.clone(),
                WidgetEvent::Scroll {
                    x: absolute.x,
                    y: absolute.y,
                    relative_x: if relative.x.is_finite() {
                        relative.x
                    } else {
                        0.0
                    },
                    relative_y: if relative.y.is_finite() {
                        relative.y
                    } else {
                        0.0
                    },
                    bounds_width: bounds.width,
                    bounds_height: bounds.height,
                    content_width: content.width,
                    content_height: content.height,
                },
            )
        });
    }
    if let Some(style) = node.get::<Option<Table>>("style")? {
        const FIELDS: &[&str] = &["background", "border", "shadow", "rail", "scroller"];
        validate(&style, FIELDS, &["active", "hovered", "dragged"])?;
        let active = checkbox_patch(&style, "active")?;
        let hovered = checkbox_patch(&style, "hovered")?;
        let dragged = checkbox_patch(&style, "dragged")?;
        scrollable = scrollable.style(move |theme, status| {
            let mut out = widget::scrollable::default(theme, status);
            let patch = match status {
                widget::scrollable::Status::Active { .. } => &active,
                widget::scrollable::Status::Hovered { .. } => &hovered,
                widget::scrollable::Status::Dragged { .. } => &dragged,
            };
            paint_scrollable(&mut out, patch);
            out
        });
    }
    Ok(scrollable.into())
}

pub(super) fn text_input(
    node: &Table,
    ctx: &Ctx,
    layout: Layout,
) -> mlua::Result<Element<'static, Message>> {
    let placeholder = node
        .get::<Option<String>>("placeholder")?
        .unwrap_or_default();
    let value = node.get::<Option<String>>("value")?.unwrap_or_default();
    let mut input = widget::TextInput::new(&placeholder, &value);
    if let Some(width) = layout.width {
        input = input.width(width);
    }
    if let Some(secure) = node.get::<Option<bool>>("secure")? {
        input = input.secure(secure);
    }
    if let Some(id) = node.get::<Option<String>>("id")? {
        input = input.id(widget::Id::new(intern_font(&id)));
    }
    if let Some(size) = node.get::<Option<f32>>("size")? {
        if !size.is_finite() || size <= 0.0 {
            return Err(mlua::Error::runtime(
                "text_input size must be finite and positive",
            ));
        }
        input = input.size(size);
    }
    if let Some(align) = node.get::<Option<String>>("align_x")? {
        input = input.align_x(horizontal(&align)?);
    }
    if let Some(font) = node.get::<Option<Value>>("font")? {
        input = input.font(font_value(font)?);
    }
    if let Some(style) = node.get::<Option<Table>>("style")? {
        const FIELDS: &[&str] = &[
            "background",
            "text_color",
            "placeholder",
            "selection",
            "icon_color",
            "border",
        ];
        validate(
            &style,
            FIELDS,
            &["active", "hovered", "focused", "disabled"],
        )?;
        let active = checkbox_patch(&style, "active")?;
        let hovered = checkbox_patch(&style, "hovered")?;
        let focused = checkbox_patch(&style, "focused")?;
        let disabled = checkbox_patch(&style, "disabled")?;
        input = input.style(move |theme, status| {
            let mut out = widget::text_input::default(theme, status);
            let patch = match status {
                widget::text_input::Status::Active => &active,
                widget::text_input::Status::Hovered => &hovered,
                widget::text_input::Status::Focused { .. } => &focused,
                widget::text_input::Status::Disabled => &disabled,
            };
            paint_text_input(&mut out, patch);
            out
        });
    }
    let generation = ctx.generation;
    let component = ctx.component.clone();
    if let Some(event) = node.get::<Option<String>>("on_input")? {
        let component = component.clone();
        input = input.on_input(move |value| {
            Message::Widget(
                generation,
                component.clone(),
                event.clone(),
                WidgetEvent::Text(value),
            )
        });
    }
    if let Some(event) = node.get::<Option<String>>("on_submit")? {
        input = input.on_submit(Message::Widget(
            generation,
            component.clone(),
            event,
            WidgetEvent::Text(value.clone()),
        ));
    }
    if let Some(event) = node.get::<Option<String>>("on_paste")? {
        let component = component.clone();
        input = input.on_paste(move |value| {
            Message::Widget(
                generation,
                component.clone(),
                event.clone(),
                WidgetEvent::Text(value),
            )
        });
    }
    Ok(input.into())
}

pub(super) fn pick_list(
    node: &Table,
    ctx: &Ctx,
    layout: Layout,
) -> mlua::Result<Element<'static, Message>> {
    let options = node
        .get::<Table>("options")?
        .sequence_values::<String>()
        .collect::<mlua::Result<Vec<_>>>()?;
    let selected = node.get::<Option<String>>("selected")?;
    let event = node.get::<Option<String>>("on_select")?.unwrap_or_default();
    let generation = ctx.generation;
    let component = ctx.component.clone();
    let mut list = widget::PickList::new(options, selected, move |value: String| {
        Message::Widget(
            generation,
            component.clone(),
            event.clone(),
            WidgetEvent::Text(value),
        )
    });
    if let Some(width) = layout.width {
        list = list.width(width);
    }
    if let Some(placeholder) = node.get::<Option<String>>("placeholder")? {
        list = list.placeholder(placeholder);
    }
    if let Some(size) = node.get::<Option<f32>>("text_size")? {
        if !size.is_finite() || size <= 0.0 {
            return Err(mlua::Error::runtime(
                "text_size must be finite and positive",
            ));
        }
        list = list.text_size(size);
    }
    if let Some(style) = node.get::<Option<Table>>("style")? {
        const FIELDS: &[&str] = &[
            "background",
            "text_color",
            "placeholder",
            "handle",
            "border",
        ];
        validate(&style, FIELDS, &["active", "hovered", "opened"])?;
        let active = checkbox_patch(&style, "active")?;
        let hovered = checkbox_patch(&style, "hovered")?;
        let opened = checkbox_patch(&style, "opened")?;
        list = list.style(move |theme, status| {
            let mut out = widget::pick_list::default(theme, status);
            let patch = match status {
                widget::pick_list::Status::Active => &active,
                widget::pick_list::Status::Hovered => &hovered,
                widget::pick_list::Status::Opened { .. } => &opened,
            };
            paint_pick_list(&mut out, patch);
            out
        });
    }
    Ok(list.into())
}
