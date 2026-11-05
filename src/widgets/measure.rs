use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
};

use iced::advanced::layout;
use iced::advanced::widget::{Operation, Tree, tree};
use iced::advanced::{Clipboard, Layout, Shell, Widget, mouse, overlay, renderer};
use iced::{Element, Event, Length, Rectangle, Size, Theme, Vector, window};
use mlua::{Lua, Table, Value};

use crate::runtime::Message;

type Key = (Option<window::Id>, String);

thread_local! {
    static BOUNDS: RefCell<HashMap<Key, Rectangle>> = RefCell::new(HashMap::new());
    static LATEST: RefCell<HashMap<String, (Option<window::Id>, Rectangle)>> =
        RefCell::new(HashMap::new());
    static CURRENT_WINDOW: Cell<Option<window::Id>> = const { Cell::new(None) };
}

/// Makes `iced.bounds` resolve against the window currently being built.
pub(crate) fn with_window<T>(window: Option<window::Id>, build: impl FnOnce() -> T) -> T {
    let previous = CURRENT_WINDOW.replace(window);
    let result = build();
    CURRENT_WINDOW.set(previous);
    result
}

pub(crate) fn forget_window(window: window::Id) {
    BOUNDS.with_borrow_mut(|bounds| bounds.retain(|(owner, _), _| *owner != Some(window)));
    LATEST.with_borrow_mut(|bounds| {
        bounds.retain(|_, (owner, _)| *owner != Some(window));
    });
}

pub(crate) fn install(lua: &Lua) -> mlua::Result<()> {
    let iced: Table = lua.globals().get("iced")?;
    iced.set(
        "bounds",
        lua.create_function(|lua, id: String| {
            let Some(bounds) = bounds(&id) else {
                return Ok(Value::Nil);
            };
            let table = lua.create_table()?;
            table.set("x", bounds.x.round() as i64)?;
            table.set("y", bounds.y.round() as i64)?;
            table.set("width", bounds.width.round() as i64)?;
            table.set("height", bounds.height.round() as i64)?;
            Ok(Value::Table(table))
        })?,
    )?;
    Ok(())
}

fn bounds(id: &str) -> Option<Rectangle> {
    let current = CURRENT_WINDOW.get();
    if let Some(bounds) =
        BOUNDS.with_borrow(|bounds| bounds.get(&(current, id.to_owned())).copied())
    {
        return Some(bounds);
    }
    LATEST.with_borrow(|bounds| bounds.get(id).map(|(_, bounds)| *bounds))
}

fn record(id: &str, window: Option<window::Id>, bounds: Rectangle) {
    BOUNDS.with_borrow_mut(|stored| {
        stored.insert((window, id.to_owned()), bounds);
    });
    LATEST.with_borrow_mut(|stored| {
        stored.insert(id.to_owned(), (window, bounds));
    });
}

pub(super) struct Measured<'a> {
    content: Element<'a, Message>,
    id: String,
    window: Option<window::Id>,
}

impl<'a> Measured<'a> {
    pub(super) fn new(
        content: Element<'a, Message>,
        id: String,
        window: Option<window::Id>,
    ) -> Self {
        Self {
            content,
            id,
            window,
        }
    }
}

impl<'a> From<Measured<'a>> for Element<'a, Message> {
    fn from(measured: Measured<'a>) -> Self {
        Element::new(measured)
    }
}

impl Widget<Message, Theme, iced::Renderer> for Measured<'_> {
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<()>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(())
    }

    fn children(&self) -> Vec<Tree> {
        vec![Tree::new(&self.content)]
    }

    fn diff(&self, tree: &mut Tree) {
        tree.diff_children(std::slice::from_ref(&self.content));
    }

    fn size(&self) -> Size<Length> {
        self.content.as_widget().size()
    }

    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &iced::Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        self.content
            .as_widget_mut()
            .layout(&mut tree.children[0], renderer, limits)
    }

    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &iced::Renderer,
        operation: &mut dyn Operation,
    ) {
        self.content
            .as_widget_mut()
            .operate(&mut tree.children[0], layout, renderer, operation);
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &iced::Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        self.content.as_widget_mut().update(
            &mut tree.children[0],
            event,
            layout,
            cursor,
            renderer,
            clipboard,
            shell,
            viewport,
        );
    }

    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &iced::Renderer,
    ) -> mouse::Interaction {
        self.content.as_widget().mouse_interaction(
            &tree.children[0],
            layout,
            cursor,
            viewport,
            renderer,
        )
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut iced::Renderer,
        theme: &Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        record(&self.id, self.window, layout.bounds());
        self.content.as_widget().draw(
            &tree.children[0],
            renderer,
            theme,
            style,
            layout,
            cursor,
            viewport,
        );
    }

    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut Tree,
        layout: Layout<'b>,
        renderer: &iced::Renderer,
        viewport: &Rectangle,
        translation: Vector,
    ) -> Option<overlay::Element<'b, Message, Theme, iced::Renderer>> {
        self.content.as_widget_mut().overlay(
            &mut tree.children[0],
            layout,
            renderer,
            viewport,
            translation,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use iced::Point;

    #[test]
    fn bounds_fall_back_to_the_last_measured_surface() {
        let id = format!("measure-test-{}", std::process::id());
        let owner = window::Id::unique();
        let other = window::Id::unique();
        let rect = Rectangle::new(Point::new(12.0, 30.0), Size::new(32.0, 32.0));
        record(&id, Some(owner), rect);
        assert_eq!(with_window(Some(other), || bounds(&id)), Some(rect));
        forget_window(owner);
        assert_eq!(with_window(Some(other), || bounds(&id)), None);
    }
}
