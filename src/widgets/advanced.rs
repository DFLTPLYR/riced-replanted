use iced::Element;
use iced::widget;
use iced::widget::{markdown as markdown_widget, text_editor as text_editor_widget};
use mlua::{Function, Table, Value};

use crate::runtime::{Message, WidgetEvent};

use super::enums::{horizontal, vertical, wrapping};
use super::input::paint_text_input;
use super::scale::ScaleItem;
use super::state;
use super::style::{animated, style_patch, validate};
use super::{Ctx, Layout, Transition, color, element, length, number};

fn text_or_element(value: Value, ctx: &Ctx) -> mlua::Result<Element<'static, Message>> {
    match value {
        Value::String(text) => Ok(widget::text(text.to_str()?.to_owned()).into()),
        Value::Integer(number) => Ok(widget::text(number.to_string()).into()),
        Value::Number(number) => Ok(widget::text(number.to_string()).into()),
        Value::Table(node) => element(node, ctx),
        Value::Nil => Ok(widget::Space::new().into()),
        _ => Err(mlua::Error::runtime(
            "table cells must be text, numbers, elements, or nil",
        )),
    }
}

fn paint_text_editor(style: &mut widget::text_editor::Style, patch: &super::style::StylePatch) {
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
}

pub(super) fn text_editor(
    node: &Table,
    ctx: &Ctx,
    _layout: Layout,
    transition: Option<&Transition>,
) -> mlua::Result<Element<'static, Message>> {
    let id: String = node
        .get::<Option<String>>("id")?
        .filter(|id| !id.is_empty())
        .ok_or_else(|| mlua::Error::runtime("text_editor requires a non-empty id"))?;
    let value = node.get::<Option<String>>("value")?.unwrap_or_default();
    let cell = state::editor(ctx.generation, &id, &value);
    let content = state::read_editor(cell);
    let mut editor = text_editor_widget::TextEditor::new(content);
    if let Some(placeholder) = node.get::<Option<String>>("placeholder")? {
        editor = editor.placeholder(placeholder);
    }
    if let Some(size) = node.get::<Option<f32>>("size")? {
        if !size.is_finite() || size <= 0.0 {
            return Err(mlua::Error::runtime(
                "text_editor size must be finite and positive",
            ));
        }
        editor = editor.size(size);
    }
    if let Some(value) = node.get::<Option<Value>>("wrapping")? {
        editor = editor.wrapping(wrapping(value)?);
    }
    if let Some(font) = node.get::<Option<Value>>("font")? {
        editor = editor.font(super::enums::font_value(font)?);
    }
    if let Some(style) = node.get::<Option<Table>>("style")? {
        let fields = [
            "background",
            "text_color",
            "placeholder",
            "selection",
            "border",
        ];
        validate(
            &style,
            &fields,
            &["active", "hovered", "focused", "disabled"],
        )?;
        let active = style_patch(
            &style,
            style.get::<Option<Table>>("active")?.as_ref(),
            transition,
        )?;
        let hovered = style_patch(
            &style,
            style.get::<Option<Table>>("hovered")?.as_ref(),
            transition,
        )?;
        let focused = style_patch(
            &style,
            style.get::<Option<Table>>("focused")?.as_ref(),
            transition,
        )?;
        let disabled = style_patch(
            &style,
            style.get::<Option<Table>>("disabled")?.as_ref(),
            transition,
        )?;
        let animation = transition.cloned();
        editor = editor.style(move |theme, status| {
            let mut out = widget::text_editor::default(theme, status);
            let (patch, state) = match status {
                widget::text_editor::Status::Active => (&active, "active"),
                widget::text_editor::Status::Hovered => (&hovered, "hovered"),
                widget::text_editor::Status::Focused { .. } => (&focused, "focused"),
                widget::text_editor::Status::Disabled => (&disabled, "disabled"),
            };
            paint_text_editor(&mut out, &animated(patch, animation.as_ref(), state));
            out
        });
    }
    let on_action = node.get::<Option<String>>("on_action")?;
    let on_input = node.get::<Option<String>>("on_input")?;
    if let Some((event, input)) = match (on_action, on_input) {
        (Some(event), _) => Some((event, false)),
        (None, Some(event)) => Some((event, true)),
        (None, None) => None,
    } {
        let generation = ctx.generation;
        let component = ctx.component.clone();
        let editor_id = id.clone();
        editor = editor.on_action(move |action| {
            Message::Editor(
                generation,
                component.clone(),
                crate::runtime::EditorAction {
                    id: editor_id.clone(),
                    event: event.clone(),
                    input,
                    action,
                },
            )
        });
    }
    Ok(editor.into())
}

pub(super) fn combo_box(
    node: &Table,
    ctx: &Ctx,
    _layout: Layout,
    transition: Option<&Transition>,
) -> mlua::Result<Element<'static, Message>> {
    let id: String = node
        .get::<Option<String>>("id")?
        .filter(|id| !id.is_empty())
        .ok_or_else(|| mlua::Error::runtime("combo_box requires a non-empty id"))?;
    let options = node
        .get::<Table>("options")?
        .sequence_values::<String>()
        .collect::<mlua::Result<Vec<_>>>()?;
    let value = node.get::<Option<String>>("value")?;
    let placeholder = node
        .get::<Option<String>>("placeholder")?
        .unwrap_or_default();
    let state = state::combo(ctx.generation, &id, &options);
    let generation = ctx.generation;
    let component = ctx.component.clone();
    let on_select = node.get::<Option<String>>("on_select")?;
    let select_component = component.clone();
    let mut combo =
        widget::ComboBox::new(state, &placeholder, value.as_ref(), move |value: String| {
            match &on_select {
                Some(event) => Message::Widget(
                    generation,
                    select_component.clone(),
                    event.clone(),
                    WidgetEvent::Text(value),
                ),
                None => Message::Noop,
            }
        });
    if let Some(event) = node.get::<Option<String>>("on_input")? {
        let component = component.clone();
        combo = combo.on_input(move |value| {
            Message::Widget(
                generation,
                component.clone(),
                event.clone(),
                WidgetEvent::Text(value),
            )
        });
    }
    if let Some(style) = node.get::<Option<Table>>("style")? {
        let fields = [
            "background",
            "text_color",
            "placeholder",
            "selection",
            "icon_color",
            "border",
        ];
        validate(
            &style,
            &fields,
            &["active", "hovered", "focused", "disabled"],
        )?;
        let active = style_patch(
            &style,
            style.get::<Option<Table>>("active")?.as_ref(),
            transition,
        )?;
        let hovered = style_patch(
            &style,
            style.get::<Option<Table>>("hovered")?.as_ref(),
            transition,
        )?;
        let focused = style_patch(
            &style,
            style.get::<Option<Table>>("focused")?.as_ref(),
            transition,
        )?;
        let disabled = style_patch(
            &style,
            style.get::<Option<Table>>("disabled")?.as_ref(),
            transition,
        )?;
        let animation = transition.cloned();
        combo = combo.input_style(move |theme, status| {
            let mut out = widget::text_input::default(theme, status);
            let (patch, state) = match status {
                widget::text_input::Status::Active => (&active, "active"),
                widget::text_input::Status::Hovered => (&hovered, "hovered"),
                widget::text_input::Status::Focused { .. } => (&focused, "focused"),
                widget::text_input::Status::Disabled => (&disabled, "disabled"),
            };
            paint_text_input(&mut out, &animated(patch, animation.as_ref(), state));
            out
        });
    }
    Ok(combo.into())
}

pub(super) fn list_view(
    node: &Table,
    ctx: &Ctx,
    layout: Layout,
) -> mlua::Result<Element<'static, Message>> {
    let id: String = node
        .get::<Option<String>>("id")?
        .filter(|id| !id.is_empty())
        .ok_or_else(|| mlua::Error::runtime("list_view requires a non-empty id"))?;
    let id = if let Value::Table(owner) = &ctx.owner {
        format!("{}:{id}", owner.get::<String>("__animation_key")?)
    } else {
        id
    };
    let items = node
        .get::<Table>("content")?
        .sequence_values::<Value>()
        .collect::<mlua::Result<Vec<_>>>()?;
    let key = match node.get::<Option<String>>("key")? {
        Some(key) if key.is_empty() => {
            return Err(mlua::Error::runtime("list_view key must be non-empty"));
        }
        key => key,
    };
    let keys = items
        .iter()
        .map(|item| list_key(item, key.as_deref()))
        .collect::<mlua::Result<Vec<_>>>()?;

    let effects = super::animation::parse(node)?;
    if node
        .get::<Option<Table>>("transition")?
        .is_some_and(|table| table.contains_key("change").unwrap_or(false))
    {
        return Err(mlua::Error::runtime(
            "list_view transitions animate items; change belongs on the rendered widget",
        ));
    }
    let animate = !effects.is_empty();
    let exiting = effects.contains_key("exit");

    state::sweep_animations();
    let previous = state::list(
        ctx.generation,
        ctx.window(),
        &id,
        items.clone(),
        &keys,
        exiting,
        ctx.lifetime.clone(),
    );

    let horizontal = match node.get::<Option<String>>("direction")?.as_deref() {
        None | Some("vertical") => false,
        Some("horizontal") => true,
        Some(other) => {
            return Err(mlua::Error::runtime(format!(
                "unknown list_view direction {other:?}; use vertical or horizontal"
            )));
        }
    };

    let render = node.get::<Option<Function>>("render")?;
    let mut children: Vec<Element<'static, Message>> =
        Vec::with_capacity(items.len() + previous.leaving.len());
    let mut keyed: Vec<(u64, Element<'static, Message>)> =
        Vec::with_capacity(items.len() + previous.leaving.len());
    for (index, item) in items.into_iter().enumerate() {
        let Some(child) = list_item(
            item,
            index,
            render.as_ref(),
            ctx,
            exiting.then(|| (id.as_str(), previous.tokens[index])),
        )?
        else {
            continue;
        };
        if animate {
            let child = ScaleItem::new(
                child,
                effects.clone(),
                Some(previous.controls[index].clone()),
                ctx.clone(),
                ctx.owner.clone(),
                false,
            )
            .into();
            keyed.push((previous.tokens[index], child));
        } else {
            children.push(child);
        }
    }
    let live_len = keyed.len();
    for leaving in previous.leaving {
        let child = if let Some(node) = leaving.node {
            if let Some(lua) = &ctx.lua {
                lua.globals()
                    .get::<Table>("riced")?
                    .get::<Table>("_animations")?
                    .get::<Function>("retain")?
                    .call::<()>((
                        node.clone(),
                        leaving.control.clone(),
                        ctx.window().map(|id| format!("{id:?}")),
                    ))?;
            }
            list_item(Value::Table(node), leaving.index, None, ctx, None)?
        } else {
            list_item(leaving.item, leaving.index, None, ctx, None)?
        };
        let Some(child) = child else {
            // Hidden, but a queued exit callback may still need its control alive.
            continue;
        };
        let child = ScaleItem::new(
            child,
            effects.clone(),
            Some(leaving.control),
            ctx.clone(),
            ctx.owner.clone(),
            true,
        )
        .into();
        keyed.push((leaving.token, child));
    }

    let spacing = number(node, "spacing", 0.0)?;
    let content = if animate {
        let mut list = super::keyed::KeyedList::new(keyed, spacing, horizontal).live_len(live_len);
        if let Some(width) = layout.width {
            list = list.width(width);
        }
        if let Some(height) = layout.height {
            list = list.height(height);
        }
        list.into()
    } else if horizontal {
        let mut row = widget::Row::with_children(children).spacing(spacing);
        if let Some(width) = layout.width {
            row = row.width(width);
        }
        if let Some(height) = layout.height {
            row = row.height(height);
        }
        row.into()
    } else {
        let mut column = widget::Column::with_children(children).spacing(spacing);
        if let Some(width) = layout.width {
            column = column.width(width);
        }
        if let Some(height) = layout.height {
            column = column.height(height);
        }
        column.into()
    };
    Ok(content)
}

/// Identity for list matching. `key` names a scalar field on table items; when
/// it is absent, the item value itself is the identity.
fn list_key(item: &Value, key: Option<&str>) -> mlua::Result<Value> {
    let Some(field) = key else {
        return Ok(item.clone());
    };
    let Value::Table(table) = item else {
        return Err(mlua::Error::runtime(
            "list_view key requires table items; use render for data items",
        ));
    };
    let value = table.get::<Value>(field)?;
    match value {
        Value::String(_) | Value::Integer(_) | Value::Number(_) | Value::Boolean(_) => Ok(value),
        _ => Err(mlua::Error::runtime(format!(
            "list_view item key {field:?} must be a string, number, or boolean"
        ))),
    }
}

fn list_item(
    item: Value,
    index: usize,
    render: Option<&Function>,
    ctx: &Ctx,
    snapshot: Option<(&str, u64)>,
) -> mlua::Result<Option<Element<'static, Message>>> {
    let item = if let Some(render) = render {
        let node: Table = if let Some(lua) = &ctx.lua {
            lua.globals()
                .get::<Table>("riced")?
                .get::<Table>("_animations")?
                .get::<Function>("render_item")?
                .call((
                    render.clone(),
                    item,
                    index,
                    ctx.owner.clone(),
                    ctx.rebuilding,
                    ctx.window().map(|id| format!("{id:?}")),
                ))?
        } else {
            render.call((item, index))?
        };
        Value::Table(node)
    } else {
        item
    };
    let child = match item {
        Value::Table(node) => {
            if let Some((id, token)) = snapshot {
                state::list_node(ctx.generation, ctx.window(), id, token, node.clone());
            }
            let hidden =
                if let Some(control) = node.get::<Option<mlua::AnyUserData>>("__animation")? {
                    let control = control.borrow::<super::animation::Control>()?;
                    let playback = control.state();
                    // Keep the source item and instance, but release its slot after exit.
                    !playback.alive || (playback.hidden && playback.applied == playback.serial)
                } else {
                    false
                };
            let child = element(node, ctx)?;
            if hidden {
                return Ok(None);
            }
            child
        }
        Value::String(text) => widget::text(text.to_str()?.to_owned()).into(),
        Value::Integer(number) => widget::text(number.to_string()).into(),
        Value::Number(number) => widget::text(number.to_string()).into(),
        Value::Nil => widget::Space::new().into(),
        _ => {
            return Err(mlua::Error::runtime(
                "list_view items must be elements, text, numbers, or nil",
            ));
        }
    };
    Ok(Some(child))
}

pub(super) fn table(
    node: &Table,
    ctx: &Ctx,
    layout: Layout,
    transition: Option<&Transition>,
) -> mlua::Result<Element<'static, Message>> {
    let columns = node
        .get::<Table>("columns")?
        .sequence_values::<Table>()
        .collect::<mlua::Result<Vec<_>>>()?;
    if columns.is_empty() {
        return Err(mlua::Error::runtime("table needs at least one column"));
    }
    let rows = node
        .get::<Table>("content")?
        .sequence_values::<Table>()
        .collect::<mlua::Result<Vec<_>>>()?;
    // Validate every cell now; the widget's view closure cannot report errors.
    for row in &rows {
        for column in 0..columns.len() {
            text_or_element(row.get::<Value>(column + 1)?, ctx)?;
        }
    }
    let rows = std::rc::Rc::new(rows);
    let mut widget_columns = Vec::with_capacity(columns.len());
    for (index, column) in columns.iter().enumerate() {
        validate(column, &["header", "width", "align_x", "align_y"], &[])?;
        let header = text_or_element(column.get::<Value>("header")?, ctx)?;
        let rows = std::rc::Rc::clone(&rows);
        let ctx = ctx.clone();
        let mut widget_column = widget::table::column(header, move |row: usize| {
            let cell = rows[row].get::<Value>(index + 1).unwrap_or(Value::Nil);
            text_or_element(cell, &ctx).unwrap_or_else(|_| widget::Space::new().into())
        });
        if let Some(width) = column.get::<Option<Value>>("width")? {
            widget_column = widget_column.width(length(width)?);
        }
        if let Some(align) = column.get::<Option<String>>("align_x")? {
            widget_column = widget_column.align_x(horizontal(&align)?);
        }
        if let Some(align) = column.get::<Option<String>>("align_y")? {
            widget_column = widget_column.align_y(vertical(&align)?);
        }
        widget_columns.push(widget_column);
    }
    let row_count = rows.len();
    let mut table = widget::table(widget_columns, 0..row_count);
    if let Some(width) = layout.width {
        table = table.width(width);
    }
    if let Some(spacing) = transition.and_then(|transition| transition.value("spacing")) {
        table = table.separator(spacing.max(0.0));
    } else if let Some(spacing) = node.get::<Option<f32>>("spacing")? {
        if !spacing.is_finite() || spacing < 0.0 {
            return Err(mlua::Error::runtime(
                "table spacing must be finite and non-negative",
            ));
        }
        table = table.separator(spacing);
    }
    Ok(table.into())
}

#[allow(deprecated)] // Component safely owns the data borrowed by Iced's markdown view.
pub(super) fn markdown(
    node: &Table,
    ctx: &Ctx,
    transition: Option<&Transition>,
) -> mlua::Result<Element<'static, Message>> {
    let content = node.get::<String>("content")?;
    let items = state::markdown(&content);
    let text_size = node.get::<Option<f32>>("text_size")?.unwrap_or(16.0);
    if !text_size.is_finite() || text_size <= 0.0 {
        return Err(mlua::Error::runtime(
            "markdown text_size must be finite and positive",
        ));
    }
    let mut settings = markdown_widget::Settings::with_text_size(text_size, iced::Theme::Dark);
    if let Some(spacing) = transition.and_then(|transition| transition.value("spacing")) {
        settings.spacing = spacing.max(0.0).into();
    } else if let Some(spacing) = node.get::<Option<f32>>("spacing")? {
        if !spacing.is_finite() || spacing < 0.0 {
            return Err(mlua::Error::runtime(
                "markdown spacing must be finite and non-negative",
            ));
        }
        settings.spacing = spacing.into();
    }
    if let Some(style) = node.get::<Option<Table>>("style")? {
        validate(&style, &["link_color", "inline_code_color"], &[])?;
        if let Some(value) = style.get::<Option<String>>("link_color")? {
            settings.style.link_color = color(&value)?;
        }
        if let Some(value) = style.get::<Option<String>>("inline_code_color")? {
            settings.style.inline_code_color = color(&value)?;
        }
    }
    let generation = ctx.generation;
    let component = ctx.component.clone();
    let event = node.get::<Option<String>>("on_link_click")?;
    Ok(
        widget::component(Markdown { items, settings }).map(move |uri| match &event {
            Some(event) => Message::Widget(
                generation,
                component.clone(),
                event.clone(),
                WidgetEvent::Text(uri.to_string()),
            ),
            None => Message::Noop,
        }),
    )
}

struct Markdown {
    items: std::rc::Rc<Vec<markdown_widget::Item>>,
    settings: markdown_widget::Settings,
}

// Iced's component owns the data borrowed by its view; no encapsulated mutable state.
#[allow(deprecated)]
impl widget::Component<markdown_widget::Uri> for Markdown {
    type State = ();
    type Event = markdown_widget::Uri;

    fn update(&mut self, _: &mut (), event: Self::Event) -> Option<Self::Event> {
        Some(event)
    }

    fn view(&self, _: &()) -> Element<'_, Self::Event> {
        markdown_widget::view(self.items.iter(), self.settings)
    }
}

pub(super) fn responsive(
    node: &Table,
    ctx: &Ctx,
    layout: Layout,
) -> mlua::Result<Element<'static, Message>> {
    let callback: Function = node.get("content")?;
    let ctx = ctx.clone();
    let mut responsive = widget::Responsive::new(move |size| {
        let result = (|| {
            let lua = ctx.lua()?;
            let props = lua.create_table()?;
            props.set("width", size.width)?;
            props.set("height", size.height)?;
            let node: Table = callback.call(props)?;
            element(node, &ctx)
        })();
        result.unwrap_or_else(|_| widget::Space::new().into())
    });
    if let Some(width) = layout.width {
        responsive = responsive.width(width);
    }
    if let Some(height) = layout.height {
        responsive = responsive.height(height);
    }
    Ok(responsive.into())
}

fn pane_highlight(table: &Table) -> mlua::Result<widget::pane_grid::Highlight> {
    crate::runtime::keys(table, &["background", "border"])?;
    let mut highlight = widget::pane_grid::Highlight {
        background: iced::Background::Color(iced::Color::TRANSPARENT),
        border: iced::Border::default(),
    };
    if let Some(value) = table.get::<Option<String>>("background")? {
        highlight.background = iced::Background::Color(color(&value)?);
    }
    if let Some(value) = table.get::<Option<Table>>("border")? {
        crate::runtime::keys(&value, &["color", "width", "radius"])?;
        if let Some(value) = value.get::<Option<String>>("color")? {
            highlight.border.color = color(&value)?;
        }
        if let Some(width) = value.get::<Option<f32>>("width")? {
            highlight.border.width = width;
        }
        match value.get::<Value>("radius")? {
            Value::Nil => {}
            radius => highlight.border.radius = super::style::radius_value(radius)?,
        }
    }
    Ok(highlight)
}

fn pane_line(table: &Table) -> mlua::Result<widget::pane_grid::Line> {
    crate::runtime::keys(table, &["color", "width"])?;
    let mut line = widget::pane_grid::Line {
        color: iced::Color::BLACK,
        width: 1.0,
    };
    if let Some(value) = table.get::<Option<String>>("color")? {
        line.color = color(&value)?;
    }
    if let Some(width) = table.get::<Option<f32>>("width")? {
        if !width.is_finite() || width < 0.0 {
            return Err(mlua::Error::runtime(
                "pane_grid line width must be finite and non-negative",
            ));
        }
        line.width = width;
    }
    Ok(line)
}

pub(super) fn pane_grid(
    node: &Table,
    ctx: &Ctx,
    layout: Layout,
    transition: Option<&Transition>,
) -> mlua::Result<Element<'static, Message>> {
    let id: String = node
        .get::<Option<String>>("id")?
        .filter(|id| !id.is_empty())
        .ok_or_else(|| mlua::Error::runtime("pane_grid requires a non-empty id"))?;
    let panes = node
        .get::<Table>("content")?
        .sequence_values::<Table>()
        .collect::<mlua::Result<Vec<_>>>()?;
    if panes.is_empty() {
        return Err(mlua::Error::runtime("pane_grid needs at least one pane"));
    }
    let axis = match node.get::<Option<String>>("direction")?.as_deref() {
        None | Some("horizontal") => iced::widget::pane_grid::Axis::Vertical,
        Some("vertical") => iced::widget::pane_grid::Axis::Horizontal,
        Some(other) => {
            return Err(mlua::Error::runtime(format!(
                "unknown pane_grid direction {other:?}; use horizontal or vertical"
            )));
        }
    };
    let cell = state::panes(ctx.generation, &id, panes.len(), axis);
    let state = state::read_panes(cell);
    let generation = ctx.generation;
    let component = ctx.component.clone();
    let on_click = node.get::<Option<String>>("on_click")?;
    let on_resize = node.get::<Option<String>>("on_resize")?;
    let on_drag = node.get::<Option<String>>("on_drag")?;
    let panes = std::rc::Rc::new(panes);
    let mut grid = widget::PaneGrid::new(state, move |_pane, index, _maximized| {
        let content = panes[*index].clone();
        let body = element(content, ctx).unwrap_or_else(|_| widget::Space::new().into());
        widget::pane_grid::Content::new(body)
    });
    if let Some(width) = layout.width {
        grid = grid.width(width);
    }
    if let Some(height) = layout.height {
        grid = grid.height(height);
    }
    if let Some(spacing) = transition.and_then(|transition| transition.value("spacing")) {
        grid = grid.spacing(spacing.max(0.0));
    } else if let Some(spacing) = node.get::<Option<f32>>("spacing")? {
        if !spacing.is_finite() || spacing < 0.0 {
            return Err(mlua::Error::runtime(
                "pane_grid spacing must be finite and non-negative",
            ));
        }
        grid = grid.spacing(spacing);
    }
    if let Some(min_size) = node.get::<Option<f32>>("min_size")? {
        if !min_size.is_finite() || min_size < 0.0 {
            return Err(mlua::Error::runtime(
                "pane_grid min_size must be finite and non-negative",
            ));
        }
        grid = grid.min_size(min_size);
    }
    if let Some(style) = node.get::<Option<Table>>("style")? {
        crate::runtime::keys(&style, &["hovered_region", "picked_split", "hovered_split"])?;
        let region = style
            .get::<Option<Table>>("hovered_region")?
            .map(|table| pane_highlight(&table))
            .transpose()?;
        let picked = style
            .get::<Option<Table>>("picked_split")?
            .map(|table| pane_line(&table))
            .transpose()?;
        let hovered = style
            .get::<Option<Table>>("hovered_split")?
            .map(|table| pane_line(&table))
            .transpose()?;
        grid = grid.style(move |theme| {
            let mut out = widget::pane_grid::default(theme);
            if let Some(region) = region {
                out.hovered_region = region;
            }
            if let Some(line) = picked {
                out.picked_split = line;
            }
            if let Some(line) = hovered {
                out.hovered_split = line;
            }
            out
        });
    }
    if let Some(event) = on_click {
        let component = component.clone();
        grid = grid.on_click(move |pane| {
            let index = state.get(pane).copied().unwrap_or(0);
            Message::Widget(
                generation,
                component.clone(),
                event.clone(),
                WidgetEvent::Number(index as f32),
            )
        });
    }
    if let Some(event) = on_resize {
        let component = component.clone();
        grid = grid.on_resize(4, move |resize| {
            Message::Widget(
                generation,
                component.clone(),
                event.clone(),
                WidgetEvent::Number(resize.ratio),
            )
        });
    }
    if let Some(event) = on_drag {
        let component = component.clone();
        grid = grid.on_drag(move |drag| {
            let (name, pane) = match drag {
                widget::pane_grid::DragEvent::Picked { pane } => ("picked", pane),
                widget::pane_grid::DragEvent::Dropped { pane, .. } => ("dropped", pane),
                widget::pane_grid::DragEvent::Canceled { pane } => ("canceled", pane),
            };
            let index = state.get(pane).copied().unwrap_or(0);
            Message::Widget(
                generation,
                component.clone(),
                event.clone(),
                WidgetEvent::Pane(name, index),
            )
        });
    }
    Ok(grid.into())
}
