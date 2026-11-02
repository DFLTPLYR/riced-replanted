use std::cell::RefCell;
use std::collections::HashMap;

use iced::Element;
use iced::widget;
use mlua::Table;

use crate::runtime::Message;

use super::enums::content_fit;
use super::style::{progress_bar_patch, qr_patch, rule_patch};
use super::{Layout, number};

thread_local! {
    // QR data borrows into the widget for its whole lifetime; cache by content so a
    // steady profile leaks once per distinct value instead of once per frame.
    // shortcut: unbounded unique QR contents are leaked, switch to an arena if that shows up in profiles.
    static QR_DATA: RefCell<HashMap<String, &'static widget::qr_code::Data>> =
        RefCell::new(HashMap::new());
}

fn qr_data(contents: &str) -> Result<&'static widget::qr_code::Data, widget::qr_code::Error> {
    QR_DATA.with_borrow_mut(|cache| {
        if let Some(data) = cache.get(contents) {
            return Ok(*data);
        }
        let data: &'static _ = Box::leak(Box::new(widget::qr_code::Data::new(contents)?));
        cache.insert(contents.to_owned(), data);
        Ok(data)
    })
}

pub(super) fn image(node: &Table, layout: Layout) -> mlua::Result<Element<'static, Message>> {
    let source = node.get::<String>("content")?;
    if source.is_empty() {
        return Err(mlua::Error::runtime("image source must not be empty"));
    }
    let mut image = widget::image(widget::image::Handle::from_path(source));
    if let Some(width) = layout.width {
        image = image.width(width);
    }
    if let Some(height) = layout.height {
        image = image.height(height);
    }
    if let Some(fit) = node.get::<Option<String>>("content_fit")? {
        image = image.content_fit(content_fit(&fit)?);
    }
    Ok(image.into())
}

pub(super) fn svg(node: &Table, layout: Layout) -> mlua::Result<Element<'static, Message>> {
    let source = node.get::<String>("content")?;
    if source.is_empty() {
        return Err(mlua::Error::runtime("svg source must not be empty"));
    }
    let mut svg = widget::svg(widget::svg::Handle::from_path(source));
    if let Some(width) = layout.width {
        svg = svg.width(width);
    }
    if let Some(height) = layout.height {
        svg = svg.height(height);
    }
    if let Some(fit) = node.get::<Option<String>>("content_fit")? {
        svg = svg.content_fit(content_fit(&fit)?);
    }
    if let Some(opacity) = node.get::<Option<f32>>("opacity")? {
        if !(0.0..=1.0).contains(&opacity) {
            return Err(mlua::Error::runtime("opacity must be from 0 to 1"));
        }
        svg = svg.opacity(opacity);
    }
    Ok(svg.into())
}

pub(super) fn rule(node: &Table) -> mlua::Result<Element<'static, Message>> {
    let thickness = number(node, "thickness", 1.0)?;
    let mut rule = match node.get::<Option<String>>("direction")?.as_deref() {
        None | Some("horizontal") => widget::rule::horizontal(thickness),
        Some("vertical") => widget::rule::vertical(thickness),
        Some(other) => {
            return Err(mlua::Error::runtime(format!(
                "unknown rule direction {other:?}; use horizontal or vertical"
            )));
        }
    };
    if let Some(style) = node.get::<Option<Table>>("style")? {
        let patch = rule_patch(&style)?;
        rule = rule.style(move |theme| {
            let mut style = widget::rule::default(theme);
            if let Some(color) = patch.color {
                style.color = color;
            }
            if let Some(radius) = patch.radius {
                style.radius = radius.into();
            }
            if let Some(snap) = patch.snap {
                style.snap = snap;
            }
            style
        });
    }
    Ok(rule.into())
}

pub(super) fn progress_bar(node: &Table) -> mlua::Result<Element<'static, Message>> {
    let value = node.get::<Option<f32>>("value")?.unwrap_or(0.0);
    if !(0.0..=1.0).contains(&value) {
        return Err(mlua::Error::runtime(
            "progress_bar value must be from 0 to 1",
        ));
    }
    let mut bar = widget::ProgressBar::new(0.0..=1.0, value);
    if let Some(girth) = node.get::<Option<f32>>("girth")? {
        if !girth.is_finite() || girth <= 0.0 {
            return Err(mlua::Error::runtime(
                "progress_bar girth must be finite and positive",
            ));
        }
        bar = bar.girth(girth);
    }
    if node.get::<Option<bool>>("vertical")?.unwrap_or(false) {
        bar = bar.vertical();
    }
    if let Some(style) = node.get::<Option<Table>>("style")? {
        let patch = progress_bar_patch(&style)?;
        bar = bar.style(move |theme| {
            let mut style = widget::progress_bar::primary(theme);
            if let Some(background) = patch.background {
                style.background = iced::Background::Color(background);
            }
            if let Some(bar) = patch.bar {
                style.bar = iced::Background::Color(bar);
            }
            if let Some(border) = patch.border {
                style.border = border;
            }
            style
        });
    }
    Ok(bar.into())
}

pub(super) fn qr_code(node: &Table) -> mlua::Result<Element<'static, Message>> {
    let contents = node.get::<String>("content")?;
    if contents.is_empty() {
        return Err(mlua::Error::runtime("qr_code content must not be empty"));
    }
    let data = qr_data(&contents)
        .map_err(|error| mlua::Error::runtime(format!("invalid qr_code content: {error}")))?;
    let mut code = widget::qr_code::QRCode::new(data);
    if let Some(cell_size) = node.get::<Option<f32>>("cell_size")? {
        if !cell_size.is_finite() || cell_size <= 0.0 {
            return Err(mlua::Error::runtime(
                "qr_code cell_size must be finite and positive",
            ));
        }
        code = code.cell_size(cell_size);
    }
    if let Some(total_size) = node.get::<Option<f32>>("total_size")? {
        if !total_size.is_finite() || total_size <= 0.0 {
            return Err(mlua::Error::runtime(
                "qr_code total_size must be finite and positive",
            ));
        }
        code = code.total_size(total_size);
    }
    if let Some(style) = node.get::<Option<Table>>("style")? {
        let patch = qr_patch(&style)?;
        code = code.style(move |theme| {
            let mut style = widget::qr_code::default(theme);
            if let Some(cell) = patch.cell {
                style.cell = cell;
            }
            if let Some(background) = patch.background {
                style.background = background;
            }
            style
        });
    }
    Ok(code.into())
}
