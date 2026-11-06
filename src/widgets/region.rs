use std::cell::RefCell;

use iced::advanced::layout;
use iced::advanced::widget::{Operation, Tree, tree};
use iced::advanced::{Clipboard, Layout, Shell, Widget, mouse, overlay, renderer};
use iced::{Element, Event, Length, Rectangle, Size, Theme, Vector, window};
use iced_exwlshell::actions::{ExWlShellCustomAction, ExWlShellCustomActionWithId, RegionActionCB};

use crate::runtime::Message;

thread_local! {
    static BOUNDS: RefCell<Vec<Rectangle>> = const { RefCell::new(Vec::new()) };
}

pub(crate) fn root(
    content: Element<'static, Message>,
    window: window::Id,
    events_transparent: bool,
) -> Element<'static, Message> {
    Region {
        content,
        window: Some(window),
        root: true,
        events_transparent,
    }
    .into()
}

#[derive(Debug, Default)]
struct RegionState {
    last: Vec<Rectangle>,
}

impl RegionState {
    // A click-through surface keeps an empty input region when no widget provides one.
    fn changed(
        &mut self,
        mut bounds: Vec<Rectangle>,
        surface: Rectangle,
        events_transparent: bool,
    ) -> bool {
        if bounds.is_empty() && !events_transparent {
            bounds.push(surface);
        }
        if self.last == bounds {
            return false;
        }
        self.last = bounds;
        true
    }
}

/// Wraps content and publishes its layout bounds as the window input region.
pub(super) struct Region<'a> {
    content: Element<'a, Message>,
    window: Option<window::Id>,
    root: bool,
    events_transparent: bool,
}

impl<'a> Region<'a> {
    pub(super) fn new(content: Element<'a, Message>, window: Option<window::Id>) -> Self {
        Self {
            content,
            window,
            root: false,
            events_transparent: false,
        }
    }
}

impl<'a> From<Region<'a>> for Element<'a, Message> {
    fn from(region: Region<'a>) -> Self {
        Element::new(region)
    }
}

impl Widget<Message, Theme, iced::Renderer> for Region<'_> {
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<RegionState>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(RegionState::default())
    }

    fn children(&self) -> Vec<Tree> {
        vec![Tree::new(&self.content)]
    }

    fn diff(&self, tree: &mut Tree) {
        tree.diff_children(&[&self.content]);
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
        let redraw = matches!(event, Event::Window(window::Event::RedrawRequested(_)));
        if self.root && redraw {
            BOUNDS.with_borrow_mut(Vec::clear);
        }
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
        if !redraw {
            return;
        }
        if !self.root {
            BOUNDS.with_borrow_mut(|bounds| bounds.push(layout.bounds()));
            return;
        }
        let Some(window) = self.window else {
            return;
        };
        let bounds = BOUNDS.with_borrow_mut(std::mem::take);
        let state = tree.state.downcast_mut::<RegionState>();
        if !state.changed(bounds, *viewport, self.events_transparent) {
            return;
        }
        let bounds = state.last.clone();
        shell.publish(Message::Surface(ExWlShellCustomActionWithId::new(
            Some(window),
            ExWlShellCustomAction::SetInputRegion(RegionActionCB::new(move |region| {
                for bounds in &bounds {
                    let (x, y) = (bounds.x.round() as i32, bounds.y.round() as i32);
                    let (width, height) =
                        (bounds.width.round() as i32, bounds.height.round() as i32);
                    if width > 0 && height > 0 {
                        let _ = region.add(x, y, width, height);
                    }
                }
            })),
        )));
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

    #[test]
    fn removal_restores_full_bounds_and_follows_resizes() {
        let mut state = RegionState::default();
        let surface = Rectangle::with_size(Size::new(100.0, 50.0));
        let region = Rectangle::with_size(Size::new(10.0, 10.0));
        assert!(state.changed(vec![region], surface, false));
        assert!(!state.changed(vec![region], surface, false));
        assert!(state.changed(Vec::new(), surface, false));
        assert_eq!(state.last, [surface]);
        let resized = Rectangle::with_size(Size::new(200.0, 100.0));
        assert!(state.changed(Vec::new(), resized, false));
        assert_eq!(state.last, [resized]);
    }

    #[test]
    fn transparent_surfaces_keep_an_empty_region() {
        let mut state = RegionState::default();
        let surface = Rectangle::with_size(Size::new(100.0, 50.0));
        let region = Rectangle::with_size(Size::new(10.0, 10.0));
        assert!(!state.changed(Vec::new(), surface, true));
        assert!(state.changed(vec![region], surface, true));
        assert!(state.changed(Vec::new(), surface, true));
        assert!(state.last.is_empty());
    }
}
