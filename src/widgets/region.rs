use iced::advanced::layout;
use iced::advanced::widget::{Operation, Tree, tree};
use iced::advanced::{Clipboard, Layout, Shell, Widget, mouse, overlay, renderer};
use iced::{Element, Event, Length, Rectangle, Size, Theme, Vector, window};
use iced_exwlshell::actions::{ActionCallback, ExwlShellCustomAction, ExwlShellCustomActionWithId};

use crate::runtime::Message;

#[derive(Debug, Default)]
struct RegionState {
    last: Option<Rectangle>,
}

/// Wraps content and publishes its layout bounds as the window input region.
pub(super) struct Region<'a> {
    content: Element<'a, Message>,
    window: Option<window::Id>,
}

impl<'a> Region<'a> {
    pub(super) fn new(content: Element<'a, Message>, window: Option<window::Id>) -> Self {
        Self { content, window }
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
        let Event::Window(window::Event::RedrawRequested(_)) = event else {
            return;
        };
        let Some(window) = self.window else {
            return;
        };
        let bounds = layout.bounds();
        let state = tree.state.downcast_mut::<RegionState>();
        if state.last == Some(bounds) {
            return;
        }
        state.last = Some(bounds);
        let (x, y) = (bounds.x.round() as i32, bounds.y.round() as i32);
        let (width, height) = (bounds.width.round() as i32, bounds.height.round() as i32);
        if width <= 0 || height <= 0 {
            return;
        }
        shell.publish(Message::Surface(ExwlShellCustomActionWithId::new(
            Some(window),
            ExwlShellCustomAction::SetInputRegion(ActionCallback::new(move |region| {
                region.add(x, y, width, height);
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
