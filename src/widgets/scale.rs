use std::time::{Duration, Instant};

use iced::advanced::layout;
use iced::advanced::widget::{Operation, Tree, tree};
use iced::advanced::{Clipboard, Layout, Renderer, Shell, Widget, mouse, overlay, renderer};
use iced::{Element, Event, Length, Rectangle, Size, Theme, Transformation, Vector, window};

use crate::runtime::Message;

#[derive(Debug)]
struct ScaleState {
    start: Instant,
    scale: f32,
    x: f32,
    done: bool,
    entering: bool,
    duration: Duration,
    x_range: (f32, f32),
    leave_token: Option<Instant>,
    window: Option<window::Id>,
}

/// Scales content in or out. Layout is unchanged; only drawing is transformed.
pub(super) struct ScaleItem<'a> {
    content: Element<'a, Message>,
    entering: bool,
    duration: Duration,
    x_range: (f32, f32),
    leave_token: Option<Instant>,
    window: Option<window::Id>,
}

impl<'a> ScaleItem<'a> {
    pub(super) fn new(
        content: Element<'a, Message>,
        entering: bool,
        duration: Duration,
        x_range: (f32, f32),
        leave_token: Option<Instant>,
        window: Option<window::Id>,
    ) -> Self {
        Self {
            content,
            entering,
            duration,
            x_range,
            leave_token,
            window,
        }
    }
}

impl<'a> From<ScaleItem<'a>> for Element<'a, Message> {
    fn from(item: ScaleItem<'a>) -> Self {
        Element::new(item)
    }
}

fn ease_out(t: f32) -> f32 {
    let inverse = 1.0 - t.clamp(0.0, 1.0);
    1.0 - inverse * inverse * inverse
}

impl Widget<Message, Theme, iced::Renderer> for ScaleItem<'_> {
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<ScaleState>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(ScaleState {
            start: Instant::now(),
            scale: if self.entering { 0.0 } else { 1.0 },
            x: if self.entering {
                self.x_range.0
            } else {
                self.x_range.1
            },
            done: false,
            entering: self.entering,
            duration: self.duration,
            x_range: self.x_range,
            leave_token: self.leave_token,
            window: self.window,
        })
    }

    fn children(&self) -> Vec<Tree> {
        vec![Tree::new(&self.content)]
    }

    fn diff(&self, tree: &mut Tree) {
        tree.diff_children(&[&self.content]);
        let state = tree.state.downcast_mut::<ScaleState>();
        let reset = match (self.leave_token, state.leave_token) {
            (Some(new), Some(old)) => new != old,
            (Some(_), None) | (None, Some(_)) => true,
            (None, None) => self.entering && !state.entering,
        };
        if reset {
            *state = ScaleState {
                start: Instant::now(),
                scale: if self.entering { 0.0 } else { 1.0 },
                x: if self.entering {
                    self.x_range.0
                } else {
                    self.x_range.1
                },
                done: false,
                entering: self.entering,
                duration: self.duration,
                x_range: self.x_range,
                leave_token: self.leave_token,
                window: self.window,
            };
        }
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
        let Event::Window(window::Event::RedrawRequested(now)) = event else {
            return;
        };
        let state = tree.state.downcast_mut::<ScaleState>();
        if state.done {
            return;
        }
        let elapsed = now.saturating_duration_since(state.start);
        let duration = state.duration.as_secs_f32().max(f32::EPSILON);
        let t = (elapsed.as_secs_f32() / duration).clamp(0.0, 1.0);
        let eased = ease_out(t);
        let progress = if state.entering { eased } else { 1.0 - eased };
        state.scale = progress;
        state.x = state.x_range.0 + (state.x_range.1 - state.x_range.0) * progress;
        if t >= 1.0 {
            state.done = true;
            state.scale = if state.entering { 1.0 } else { 0.0 };
            state.x = if state.entering {
                state.x_range.1
            } else {
                state.x_range.0
            };
            if !state.entering
                && let Some(window) = state.window
            {
                shell.publish(Message::RedrawWindow(window));
            }
        } else {
            shell.request_redraw();
        }
    }

    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &iced::Renderer,
    ) -> mouse::Interaction {
        let state = tree.state.downcast_ref::<ScaleState>();
        if state.scale <= 0.01 {
            mouse::Interaction::None
        } else {
            self.content.as_widget().mouse_interaction(
                &tree.children[0],
                layout,
                cursor,
                viewport,
                renderer,
            )
        }
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
        let state = tree.state.downcast_ref::<ScaleState>();
        if state.scale <= 0.0 {
            return;
        }
        if state.scale >= 0.999 {
            self.content.as_widget().draw(
                &tree.children[0],
                renderer,
                theme,
                style,
                layout,
                cursor,
                viewport,
            );
            return;
        }
        let bounds = layout.bounds();
        let center = bounds.center();
        let transformation = Transformation::translate(state.x + center.x, center.y)
            * Transformation::scale(state.scale)
            * Transformation::translate(-center.x, -center.y);
        renderer.with_transformation(transformation, |renderer| {
            self.content.as_widget().draw(
                &tree.children[0],
                renderer,
                theme,
                style,
                layout,
                cursor,
                viewport,
            );
        });
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
