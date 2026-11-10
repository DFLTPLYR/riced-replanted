use iced::advanced::layout;
use iced::advanced::widget::{Operation, Tree, tree};
use iced::advanced::{Clipboard, Layout, Renderer, Shell, Widget, mouse, overlay, renderer};
use iced::{Element, Event, Length, Rectangle, Size, Theme, Transformation, Vector, window};
use iced_anim::Animated;
use mlua::{Table, Value};
use std::sync::{Arc, Mutex};

use super::animation::{Control, Effects, Transform};
use super::{Ctx, Motion, StateAnimation, Transition};
use crate::runtime::Message;

struct State {
    control: Control,
    hovered: bool,
    pressed: bool,
    shift: Animated<Vector>,
    shift_control: Control,
    change: Option<Animated<Motion>>,
    change_control: Control,
    state_animation: Option<Arc<Mutex<StateAnimation>>>,
    redraw: bool,
    owns_control: bool,
}

impl Drop for State {
    fn drop(&mut self) {
        if self.owns_control {
            self.control.cancel();
        }
        self.shift_control.cancel();
        self.change_control.cancel();
    }
}

impl State {
    fn cursor(&self, cursor: mouse::Cursor, bounds: Rectangle) -> mouse::Cursor {
        let playback = self.control.state();
        let transform = playback.motion.value();
        let Some(point) = cursor.position().filter(|_| transform.scale > 0.0) else {
            return mouse::Cursor::Unavailable;
        };
        let center = bounds.center();
        let shift = self.shift.value();
        mouse::Cursor::Available(iced::Point::new(
            center.x + (point.x - center.x - transform.x - shift.x) / transform.scale,
            center.y + (point.y - center.y - transform.y - shift.y) / transform.scale,
        ))
    }
}

/// Event animation shared by ordinary widgets, component roots, and keyed items.
pub(super) struct ScaleItem<'a> {
    content: Element<'a, Message>,
    effects: Effects,
    control: Option<Control>,
    ctx: Ctx,
    owner: Value,
    retained: bool,
    property: Option<(Table, Transition)>,
}

impl<'a> ScaleItem<'a> {
    pub(super) fn new(
        content: Element<'a, Message>,
        effects: Effects,
        control: Option<Control>,
        ctx: Ctx,
        owner: Value,
        retained: bool,
    ) -> Self {
        Self {
            content,
            effects,
            control,
            ctx,
            owner,
            retained,
            property: None,
        }
    }

    pub(super) fn property(mut self, node: Table, transition: Option<Transition>) -> Self {
        self.property = transition.map(|transition| (node, transition));
        self
    }

    fn callback(
        &self,
        control: &Control,
        callback: Option<&mlua::Function>,
        shell: &mut Shell<'_, Message>,
    ) {
        let mut state = control.state();
        if state.completed {
            return;
        }
        state.completed = true;
        state.pending_callback = callback.is_some();
        if let Some(callback) = callback {
            shell.publish(Message::Animation {
                generation: self.ctx.generation,
                component: self.ctx.component.clone(),
                window: self.ctx.window(),
                control: control.clone(),
                serial: state.serial,
                callback: callback.clone(),
                owner: self.owner.clone(),
                lifetime: self.ctx.lifetime.clone(),
            });
        }
    }
}

impl<'a> From<ScaleItem<'a>> for Element<'a, Message> {
    fn from(item: ScaleItem<'a>) -> Self {
        Element::new(item)
    }
}

impl Widget<Message, Theme, iced::Renderer> for ScaleItem<'_> {
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<State>()
    }

    fn state(&self) -> tree::State {
        let control = self.control.clone().unwrap_or_default();
        control.sync(&self.effects);
        let change_control = Control::default();
        change_control.state().completed = true;
        let shift_control = Control::default();
        shift_control.state().completed = true;
        tree::State::new(State {
            control,
            hovered: false,
            pressed: false,
            shift: Animated::new(Vector::ZERO, iced_anim::transition::Easing::LINEAR),
            shift_control,
            change: self
                .property
                .as_ref()
                .map(|(_, t)| Animated::new(t.motion.clone(), t.mode)),
            change_control,
            state_animation: self
                .property
                .as_ref()
                .and_then(|(_, t)| t.state_animation.clone()),
            redraw: false,
            owns_control: self.control.is_none(),
        })
    }

    fn children(&self) -> Vec<Tree> {
        vec![Tree::new(&self.content)]
    }

    fn diff(&self, tree: &mut Tree) {
        let state = tree.state.downcast_mut::<State>();
        if let Some(control) = &self.control {
            state.control = control.clone();
            state.owns_control = false;
        }
        state.control.sync(&self.effects);
        if let Some((_, transition)) = &self.property
            && let Some(previous) = &state.state_animation
        {
            transition.copy_state_from(previous);
        }
        state.state_animation = self
            .property
            .as_ref()
            .and_then(|(_, transition)| transition.state_animation.clone());
        state.redraw = state.state_animation.is_some();
        match (&mut state.change, &self.property) {
            (Some(change), Some((_, transition))) if change.target() != &transition.motion => {
                *change = Animated::new(change.value().clone(), transition.mode)
                    .to(transition.motion.clone());
                state.change_control.request("change");
            }
            (None, Some((_, transition))) => {
                state.change = Some(Animated::new(transition.motion.clone(), transition.mode));
                state.change_control = Control::default();
                state.change_control.state().completed = true;
            }
            (_, None) => {
                state.change = None;
                state.change_control.cancel();
            }
            _ => {}
        }
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
        let state = tree.state.downcast_ref::<State>();
        if let (Some(change), Some((node, transition))) = (&state.change, &self.property) {
            let mut transition = transition.clone();
            transition.motion = change.value().clone();
            self.content = crate::runtime::checked(super::build(
                node,
                &Ctx {
                    rebuilding: true,
                    ..self.ctx.clone()
                },
                Some(&transition),
            ));
            tree.children[0].diff(&self.content);
        }
        let bounds = state.control.state().bounds;
        let limits = if self.retained
            && let Some(bounds) = bounds
        {
            layout::Limits::new(Size::ZERO, limits.max())
                .width(bounds.width)
                .height(bounds.height)
        } else {
            *limits
        };
        self.content
            .as_widget_mut()
            .layout(&mut tree.children[0], renderer, &limits)
    }

    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &iced::Renderer,
        operation: &mut dyn Operation,
    ) {
        let state = tree.state.downcast_ref::<State>().control.state();
        if !state.alive || state.hidden || state.event == "exit" {
            return;
        }
        drop(state);
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
        let state = tree.state.downcast_mut::<State>();
        state.control.sync(&self.effects);
        let interactive = {
            let playback = state.control.state();
            playback.alive && !playback.hidden && playback.event != "exit" && !self.retained
        };
        if interactive {
            let local_cursor = state.cursor(cursor, layout.bounds());
            let inside = local_cursor.is_over(layout.bounds()) && cursor.is_over(*viewport);
            if matches!(event, Event::Mouse(_)) {
                if inside != state.hovered {
                    state.hovered = inside;
                    let name = if inside {
                        "pointer_enter"
                    } else {
                        "pointer_exit"
                    };
                    if self.effects.contains_key(name) {
                        state.control.request(name);
                    }
                }
                let trigger = match event {
                    Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) if inside => {
                        state.pressed = true;
                        Some("press")
                    }
                    Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left))
                        if state.pressed =>
                    {
                        state.pressed = false;
                        Some("release")
                    }
                    _ => None,
                };
                if let Some(name) = trigger.filter(|name| self.effects.contains_key(*name)) {
                    state.control.request(name);
                }
                state.control.sync(&self.effects);
                shell.request_redraw();
            }
            self.content.as_widget_mut().update(
                &mut tree.children[0],
                event,
                layout,
                local_cursor,
                renderer,
                clipboard,
                shell,
                viewport,
            );
        }
        let Event::Window(window::Event::RedrawRequested(now)) = event else {
            return;
        };
        let mut playback = state.control.state();
        if !playback.alive {
            return;
        }
        if !self.retained {
            let bounds = layout.bounds();
            if let Some(effect) = self.effects.get("displaced")
                && let Some(previous) = playback.bounds
                && (previous.x != bounds.x || previous.y != bounds.y)
            {
                let shift = *state.shift.value()
                    + Vector::new(previous.x - bounds.x, previous.y - bounds.y);
                state.shift = Animated::new(shift, effect.mode).to(Vector::ZERO);
                state.shift_control.request("displaced");
            }
            playback.bounds = Some(bounds);
        }
        playback.motion.tick(*now);
        let animating = playback.motion.is_animating();
        let name = playback.event.clone();
        let completed = playback.completed;
        if !animating && name == "exit" {
            playback.hidden = true;
        }
        drop(playback);
        if !animating && !completed {
            self.callback(
                &state.control,
                self.effects.get(&name).and_then(|e| e.callback.as_ref()),
                shell,
            );
            if let Some(window) = self.ctx.window() {
                shell.publish(Message::RedrawWindow(window));
            }
        }
        state.shift.tick(*now);
        if !state.shift.is_animating() {
            self.callback(
                &state.shift_control,
                self.effects
                    .get("displaced")
                    .and_then(|e| e.callback.as_ref()),
                shell,
            );
        }
        let state_animating = state.state_animation.as_ref().is_some_and(|animation| {
            let mut animation = animation.lock().unwrap();
            animation.tick(*now);
            animation.is_animating()
        });
        let state_redraw = std::mem::take(&mut state.redraw);
        let mut changing = false;
        if let (Some(change), Some((node, transition))) = (&mut state.change, &self.property) {
            changing = change.is_animating();
            change.tick(*now);
            if changing {
                let mut transition = transition.clone();
                transition.motion = change.value().clone();
                self.content = crate::runtime::checked(super::build(
                    node,
                    &Ctx {
                        rebuilding: true,
                        ..self.ctx.clone()
                    },
                    Some(&transition),
                ));
                tree.children[0].diff(&self.content);
                shell.invalidate_layout();
            }
            if !change.is_animating() {
                self.callback(&state.change_control, transition.callback.as_ref(), shell);
            }
        }
        if animating || state.shift.is_animating() || changing || state_animating || state_redraw {
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
        let state = tree.state.downcast_ref::<State>();
        let playback = state.control.state();
        if !playback.alive || playback.hidden || playback.event == "exit" || self.retained {
            return mouse::Interaction::None;
        }
        drop(playback);
        self.content.as_widget().mouse_interaction(
            &tree.children[0],
            layout,
            state.cursor(cursor, layout.bounds()),
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
        let state = tree.state.downcast_ref::<State>();
        let playback = state.control.state();
        let Transform { x, y, scale } = *playback.motion.value();
        if !playback.alive || playback.hidden || scale <= 0.0 {
            return;
        }
        let bounds = layout.bounds();
        let delta = if self.retained {
            playback
                .bounds
                .map(|b| Vector::new(b.x - bounds.x, b.y - bounds.y))
                .unwrap_or_default()
        } else {
            Vector::ZERO
        };
        drop(playback);
        let cursor = state.cursor(cursor, bounds);
        let shift = *state.shift.value();
        let center = bounds.center();
        let transform = Transformation::translate(
            delta.x + x + shift.x + center.x,
            delta.y + y + shift.y + center.y,
        ) * Transformation::scale(scale)
            * Transformation::translate(-center.x, -center.y);
        renderer.with_transformation(transform, |renderer| {
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
        let playback = tree.state.downcast_ref::<State>().control.state();
        if !playback.alive || playback.hidden || playback.event == "exit" || self.retained {
            return None;
        }
        drop(playback);
        self.content.as_widget_mut().overlay(
            &mut tree.children[0],
            layout,
            renderer,
            viewport,
            translation,
        )
    }
}
