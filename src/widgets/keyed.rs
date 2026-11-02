use iced::advanced::layout::{self, flex};
use iced::advanced::widget::{Operation, Tree, tree};
use iced::advanced::{Clipboard, Layout, Shell, Widget, mouse, overlay, renderer};
use iced::{Alignment, Element, Event, Length, Padding, Rectangle, Size, Theme, Vector};

use crate::runtime::Message;

#[derive(Debug)]
struct State {
    keys: Vec<u64>,
}

/// Row or column that moves widget state with its key. Unlike iced's
/// `keyed_column`, this preserves state across arbitrary reorders, which the
/// list animation needs to measure each item's old layout slot.
pub(super) struct KeyedList<'a> {
    keys: Vec<u64>,
    children: Vec<Element<'a, Message>>,
    spacing: f32,
    width: Length,
    height: Length,
    horizontal: bool,
}

impl<'a> KeyedList<'a> {
    pub(super) fn new(
        children: Vec<(u64, Element<'a, Message>)>,
        spacing: f32,
        horizontal: bool,
    ) -> Self {
        let (keys, children) = children.into_iter().unzip();
        Self {
            keys,
            children,
            spacing,
            width: Length::Shrink,
            height: Length::Shrink,
            horizontal,
        }
    }

    pub(super) fn width(mut self, width: Length) -> Self {
        self.width = width;
        self
    }

    pub(super) fn height(mut self, height: Length) -> Self {
        self.height = height;
        self
    }
}

impl<'a> From<KeyedList<'a>> for Element<'a, Message> {
    fn from(column: KeyedList<'a>) -> Self {
        Element::new(column)
    }
}

fn positions(old: &[u64], new: &[u64]) -> Vec<Option<usize>> {
    let mut used = vec![false; old.len()];
    new.iter()
        .map(|key| {
            let index = old
                .iter()
                .enumerate()
                .position(|(index, old_key)| !used[index] && old_key == key)?;
            used[index] = true;
            Some(index)
        })
        .collect()
}

impl Widget<Message, Theme, iced::Renderer> for KeyedList<'_> {
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<State>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(State {
            keys: self.keys.clone(),
        })
    }

    fn children(&self) -> Vec<Tree> {
        self.children.iter().map(Tree::new).collect()
    }

    fn diff(&self, tree: &mut Tree) {
        let Tree {
            state,
            children: old_children,
            ..
        } = tree;
        let state = state.downcast_mut::<State>();
        let order = positions(&state.keys, &self.keys);
        let mut old: Vec<Option<Tree>> =
            std::mem::take(old_children).into_iter().map(Some).collect();
        let mut children = Vec::with_capacity(self.children.len());
        for (child, old_index) in self.children.iter().zip(order) {
            let mut child_tree = old_index
                .and_then(|index| old[index].take())
                .unwrap_or_else(|| Tree::new(child));
            child_tree.diff(child);
            children.push(child_tree);
        }
        *old_children = children;
        state.keys.clear();
        state.keys.extend_from_slice(&self.keys);
    }

    fn size(&self) -> Size<Length> {
        Size {
            width: self.width,
            height: self.height,
        }
    }

    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &iced::Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        let limits = limits.width(self.width).height(self.height);
        flex::resolve(
            if self.horizontal {
                flex::Axis::Horizontal
            } else {
                flex::Axis::Vertical
            },
            renderer,
            &limits,
            self.width,
            self.height,
            Padding::ZERO,
            self.spacing,
            Alignment::Start,
            &mut self.children,
            &mut tree.children,
        )
    }

    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &iced::Renderer,
        operation: &mut dyn Operation,
    ) {
        operation.container(None, layout.bounds());
        operation.traverse(&mut |operation| {
            self.children
                .iter_mut()
                .zip(&mut tree.children)
                .zip(layout.children())
                .for_each(|((child, state), layout)| {
                    child
                        .as_widget_mut()
                        .operate(state, layout, renderer, operation);
                });
        });
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
        for ((child, tree), layout) in self
            .children
            .iter_mut()
            .zip(&mut tree.children)
            .zip(layout.children())
        {
            child.as_widget_mut().update(
                tree, event, layout, cursor, renderer, clipboard, shell, viewport,
            );
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
        self.children
            .iter()
            .zip(&tree.children)
            .zip(layout.children())
            .map(|((child, tree), layout)| {
                child
                    .as_widget()
                    .mouse_interaction(tree, layout, cursor, viewport, renderer)
            })
            .max()
            .unwrap_or_default()
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
        for ((child, state), layout) in self
            .children
            .iter()
            .zip(&tree.children)
            .zip(layout.children())
        {
            child
                .as_widget()
                .draw(state, renderer, theme, style, layout, cursor, viewport);
        }
    }

    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut Tree,
        layout: Layout<'b>,
        renderer: &iced::Renderer,
        viewport: &Rectangle,
        translation: Vector,
    ) -> Option<overlay::Element<'b, Message, Theme, iced::Renderer>> {
        overlay::from_children(
            &mut self.children,
            tree,
            layout,
            renderer,
            viewport,
            translation,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::positions;

    #[test]
    fn matches_children_by_key_across_inserts_removals_and_reorders() {
        assert_eq!(positions(&[1, 2, 3], &[1, 3]), [Some(0), Some(2)]);
        assert_eq!(positions(&[1, 3], &[1, 2, 3]), [Some(0), None, Some(1)]);
        assert_eq!(positions(&[1, 2], &[2, 1]), [Some(1), Some(0)]);
        assert_eq!(positions(&[1, 2, 3], &[4]), [None]);
        assert_eq!(positions(&[1, 1], &[1]), [Some(0)]);
    }
}
