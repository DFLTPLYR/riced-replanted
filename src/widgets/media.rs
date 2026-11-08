use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::{Rc, Weak};

use iced::advanced::widget::{Tree, tree};
use iced::advanced::{Layout as IcedLayout, Widget, layout, mouse, renderer};
use iced::{Element, Length, Rectangle, Size, Theme, widget};
use mlua::Table;

use crate::runtime::Message;

use super::enums::content_fit;
use super::style::{QrPatch, progress_bar_patch, qr_patch, rule_patch};
use super::{Layout, number};

thread_local! {
    // Widgets only need QR data while they are rendered. Keep a weak cache so
    // changing content releases the old data instead of leaking it.
    static QR_DATA: RefCell<HashMap<String, Weak<widget::qr_code::Data>>> =
        RefCell::new(HashMap::new());
}

fn qr_data(contents: &str) -> Result<Rc<widget::qr_code::Data>, widget::qr_code::Error> {
    QR_DATA.with_borrow_mut(|cache| {
        if let Some(data) = cache.get(contents).and_then(Weak::upgrade) {
            return Ok(data);
        }
        let data = Rc::new(widget::qr_code::Data::new(contents)?);
        cache.retain(|_, weak| weak.strong_count() > 0);
        cache.insert(contents.to_owned(), Rc::downgrade(&data));
        Ok(data)
    })
}

/// Owns the QR data so the cache can drop it when the last widget is gone.
struct OwnedQrCode {
    data: Rc<widget::qr_code::Data>,
    cell_size: Option<f32>,
    total_size: Option<f32>,
    style: Option<QrPatch>,
}

impl OwnedQrCode {
    fn qr(&self) -> widget::qr_code::QRCode<'_, Theme> {
        let mut code = widget::qr_code::QRCode::new(&self.data);
        if let Some(cell_size) = self.cell_size {
            code = code.cell_size(cell_size);
        }
        if let Some(total_size) = self.total_size {
            code = code.total_size(total_size);
        }
        if let Some(patch) = self.style {
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
        code
    }
}

impl Widget<Message, Theme, iced::Renderer> for OwnedQrCode {
    fn tag(&self) -> tree::Tag {
        Widget::<Message, Theme, iced::Renderer>::tag(&self.qr())
    }

    fn state(&self) -> tree::State {
        Widget::<Message, Theme, iced::Renderer>::state(&self.qr())
    }

    fn size(&self) -> Size<Length> {
        Widget::<Message, Theme, iced::Renderer>::size(&self.qr())
    }

    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &iced::Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        let mut code = self.qr();
        Widget::<Message, Theme, iced::Renderer>::layout(&mut code, tree, renderer, limits)
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut iced::Renderer,
        theme: &Theme,
        style: &renderer::Style,
        layout: IcedLayout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        Widget::<Message, Theme, iced::Renderer>::draw(
            &self.qr(),
            tree,
            renderer,
            theme,
            style,
            layout,
            cursor,
            viewport,
        );
    }
}

impl From<OwnedQrCode> for Element<'static, Message> {
    fn from(code: OwnedQrCode) -> Self {
        Element::new(code)
    }
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
    if let Some(opacity) = node.get::<Option<f32>>("opacity")? {
        if !(0.0..=1.0).contains(&opacity) {
            return Err(mlua::Error::runtime("image opacity must be from 0 to 1"));
        }
        image = image.opacity(opacity);
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
                style.radius = radius;
            }
            if let Some(snap) = patch.snap {
                style.snap = snap;
            }
            style
        });
    }
    Ok(rule.into())
}

pub(super) fn progress_bar(
    node: &Table,
    transition: Option<&super::Transition>,
) -> mlua::Result<Element<'static, Message>> {
    let value = node.get::<Option<f32>>("value")?.unwrap_or(0.0);
    if !(0.0..=1.0).contains(&value) {
        return Err(mlua::Error::runtime(
            "progress_bar value must be from 0 to 1",
        ));
    }
    // Springs may overshoot valid targets; only the displayed value is bounded.
    let value = transition
        .and_then(|transition| transition.value("value"))
        .unwrap_or(value)
        .clamp(0.0, 1.0);
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
    let mut code = OwnedQrCode {
        data,
        cell_size: None,
        total_size: None,
        style: None,
    };
    if let Some(cell_size) = node.get::<Option<f32>>("cell_size")? {
        if !cell_size.is_finite() || cell_size <= 0.0 {
            return Err(mlua::Error::runtime(
                "qr_code cell_size must be finite and positive",
            ));
        }
        code.cell_size = Some(cell_size);
    }
    if let Some(total_size) = node.get::<Option<f32>>("total_size")? {
        if !total_size.is_finite() || total_size <= 0.0 {
            return Err(mlua::Error::runtime(
                "qr_code total_size must be finite and positive",
            ));
        }
        code.total_size = Some(total_size);
    }
    if let Some(style) = node.get::<Option<Table>>("style")? {
        code.style = Some(qr_patch(&style)?);
    }
    Ok(code.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn qr_data_is_shared_while_used_and_released_when_unused() {
        QR_DATA.with_borrow_mut(HashMap::clear);

        let first = qr_data("shared").unwrap();
        let second = qr_data("shared").unwrap();
        assert!(Rc::ptr_eq(&first, &second));

        drop(first);
        drop(second);
        let other = qr_data("other").unwrap();
        assert_eq!(QR_DATA.with_borrow(HashMap::len), 1);

        drop(other);
        let third = qr_data("third").unwrap();
        assert_eq!(QR_DATA.with_borrow(HashMap::len), 1);
        drop(third);
    }
}
