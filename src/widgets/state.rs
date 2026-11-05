//! Host-owned state for widgets that keep data outside the element tree.
//!
//! Iced recreates elements every render, so stateful widgets need a stable
//! allocation. Entries are keyed by Lua generation and widget id; a shell
//! reload gets a fresh generation. Replaced entries are leaked:
//! `shortcut: state is kept for the process lifetime, add a generation sweep
//! if profiles churn through ids or options`.

use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    rc::Rc,
};

use super::animation::Control;
use iced::widget::{combo_box, pane_grid, text_editor};
use mlua::Value;

type PaneState = &'static RefCell<pane_grid::State<usize>>;
type ListKey = (u64, Option<iced::window::Id>, String);

thread_local! {
    static EDITORS: RefCell<HashMap<(u64, String), &'static RefCell<text_editor::Content>>> =
        RefCell::new(HashMap::new());
    static COMBOS: RefCell<HashMap<(u64, String), &'static combo_box::State<String>>> =
        RefCell::new(HashMap::new());
    static PANES: RefCell<HashMap<(u64, String, u8, usize), PaneState>> =
        RefCell::new(HashMap::new());
    static MARKDOWN: RefCell<HashMap<String, Rc<Vec<iced::widget::markdown::Item>>>> =
        RefCell::new(HashMap::new());
    static LISTS: RefCell<HashMap<ListKey, ListEntry>> = RefCell::new(HashMap::new());
    static NEXT_LIST_TOKEN: Cell<u64> = const { Cell::new(0) };
}

pub(crate) fn forget_window(generation: u64, window: iced::window::Id) {
    LISTS.with_borrow_mut(|lists| {
        lists.retain(|(owner, id, _), _| *owner != generation || *id != Some(window))
    });
}

pub(crate) fn forget_generation(generation: u64) {
    LISTS.with_borrow_mut(|lists| lists.retain(|(owner, _, _), _| *owner != generation));
}

#[derive(Default)]
struct ListEntry {
    items: Vec<(Value, Value, u64, Control)>,
    leaving: Vec<ListLeaving>,
    nodes: HashMap<u64, mlua::Table>,
    owner: Option<Control>,
}

impl Drop for ListEntry {
    fn drop(&mut self) {
        for (_, _, _, control) in &self.items {
            control.cancel();
        }
        for leaving in &self.leaving {
            leaving.control.cancel();
        }
    }
}

#[derive(Clone)]
pub(super) struct ListLeaving {
    pub(super) item: Value,
    pub(super) index: usize,
    pub(super) token: u64,
    pub(super) key: Value,
    pub(super) control: Control,
    pub(super) node: Option<mlua::Table>,
}

pub(super) struct ListState {
    pub(super) tokens: Vec<u64>,
    pub(super) leaving: Vec<ListLeaving>,
    pub(super) controls: Vec<Control>,
}

pub(crate) fn sweep_animations() {
    LISTS.with_borrow_mut(|lists| {
        lists.retain(|_, entry| entry.owner.as_ref().is_none_or(|owner| owner.state().alive))
    });
}

/// Stable editor content. `value` wins when it differs from the stored text, so
/// a profile can reset the document; ordinary typing does not allocate.
pub(super) fn editor(
    generation: u64,
    id: &str,
    value: &str,
) -> &'static RefCell<text_editor::Content> {
    EDITORS.with(|editors| {
        let mut editors = editors.borrow_mut();
        let key = (generation, id.to_owned());
        if let Some(existing) = editors.get(&key)
            && existing.borrow().text() == value
        {
            return *existing;
        }
        let state = Box::leak(Box::new(RefCell::new(text_editor::Content::with_text(
            value,
        ))));
        editors.insert(key, state);
        state
    })
}

/// # Safety
/// `cell` is a leaked `RefCell`, so the pointer is valid for the process.
pub(super) fn read_editor(
    cell: &'static RefCell<text_editor::Content>,
) -> &'static text_editor::Content {
    unsafe { &*cell.as_ptr() }
}

pub(super) fn perform(generation: u64, id: &str, action: text_editor::Action) -> Option<String> {
    EDITORS.with(|editors| {
        let editors = editors.borrow();
        let cell = editors.get(&(generation, id.to_owned()))?;
        let mut content = cell.borrow_mut();
        content.perform(action);
        Some(content.text())
    })
}

pub(super) fn text(generation: u64, id: &str) -> Option<String> {
    EDITORS.with(|editors| {
        let editors = editors.borrow();
        let cell = editors.get(&(generation, id.to_owned()))?;
        Some(cell.borrow().text())
    })
}

/// Stable combo state; rebuilt only when the option list changes.
pub(super) fn combo(
    generation: u64,
    id: &str,
    options: &[String],
) -> &'static combo_box::State<String> {
    COMBOS.with(|combos| {
        let mut combos = combos.borrow_mut();
        let key = (generation, id.to_owned());
        if let Some(existing) = combos.get(&key)
            && existing.options() == options
        {
            return *existing;
        }
        let state = Box::leak(Box::new(combo_box::State::new(options.to_vec())));
        combos.insert(key, state);
        state
    })
}

/// A small parse cache; live widgets retain their own references across eviction.
pub(super) fn markdown(content: &str) -> Rc<Vec<iced::widget::markdown::Item>> {
    MARKDOWN.with(|cache| {
        let mut cache = cache.borrow_mut();
        if let Some(items) = cache.get(content) {
            return Rc::clone(items);
        }
        if cache.len() >= 32 {
            cache.clear();
        }
        let items = Rc::new(iced::widget::markdown::parse(content).collect());
        cache.insert(content.to_owned(), Rc::clone(&items));
        items
    })
}

/// Previous rendered items for a list view, plus items still animating out.
///
/// Keys preserve both widget state and in-flight animations across reorders.
pub(super) fn list(
    generation: u64,
    window: Option<iced::window::Id>,
    id: &str,
    items: Vec<Value>,
    keys: &[Value],
    animate: bool,
    owner: Option<Control>,
) -> ListState {
    LISTS.with(|lists| {
        let mut lists = lists.borrow_mut();
        let entry = lists
            .entry((generation, window, id.to_owned()))
            .or_default();
        entry.owner = owner;
        let previous = std::mem::take(&mut entry.items);
        let mut old_used = vec![false; previous.len()];
        let mut tokens = Vec::with_capacity(items.len());
        let mut controls = Vec::with_capacity(items.len());
        let mut next_items = Vec::with_capacity(items.len());

        for (item, key) in items.iter().zip(keys) {
            let matched =
                previous
                    .iter()
                    .enumerate()
                    .find_map(|(old_index, (_, old_key, token, _))| {
                        (!old_used[old_index] && old_key == key).then_some((old_index, *token))
                    });
            let returning = entry.leaving.iter().position(|leaving| leaving.key == *key);
            let (token, control) = match (matched, returning) {
                (Some((old_index, token)), _) => {
                    old_used[old_index] = true;
                    (token, previous[old_index].3.clone())
                }
                (None, Some(index)) => {
                    let leaving = entry.leaving.remove(index);
                    leaving.control.request("enter");
                    (leaving.token, leaving.control)
                }
                (None, None) => {
                    let token = NEXT_LIST_TOKEN.with(|next| {
                        let token = next.get();
                        next.set(token + 1);
                        token
                    });
                    (token, Control::default())
                }
            };
            tokens.push(token);
            controls.push(control.clone());
            next_items.push((item.clone(), key.clone(), token, control));
        }

        let mut leaving = Vec::new();
        for (index, (item, key, token, control)) in previous.iter().enumerate() {
            if !old_used[index] {
                leaving.push(ListLeaving {
                    item: item.clone(),
                    index,
                    token: *token,
                    key: key.clone(),
                    control: control.clone(),
                    node: entry.nodes.remove(token),
                });
                control.request("exit");
            }
        }

        entry.items = next_items;
        if animate {
            entry.leaving.retain(|leaving| !leaving.control.removable());
            entry.leaving.extend(leaving);
        } else {
            for leaving in &entry.leaving {
                leaving.control.cancel();
            }
            for leaving in &leaving {
                leaving.control.cancel();
            }
            entry.leaving.clear();
        }
        ListState {
            tokens,
            leaving: entry.leaving.clone(),
            controls,
        }
    })
}

pub(super) fn list_node(
    generation: u64,
    window: Option<iced::window::Id>,
    id: &str,
    token: u64,
    node: mlua::Table,
) {
    LISTS.with_borrow_mut(|lists| {
        if let Some(entry) = lists.get_mut(&(generation, window, id.to_owned())) {
            entry.nodes.insert(token, node);
        }
    });
}

/// Stable pane layout; rebuilt when the pane count changes.
pub(super) fn panes(
    generation: u64,
    id: &str,
    count: usize,
    axis: pane_grid::Axis,
) -> &'static RefCell<pane_grid::State<usize>> {
    let axis_key = match axis {
        pane_grid::Axis::Horizontal => 0,
        pane_grid::Axis::Vertical => 1,
    };
    PANES.with(|panes| {
        let mut panes = panes.borrow_mut();
        let key = (generation, id.to_owned(), axis_key, count);
        if let Some(existing) = panes.get(&key) {
            return *existing;
        }
        let (mut state, mut current) = pane_grid::State::new(0);
        for index in 1..count {
            let Some((next, _)) = state.split(axis, current, index) else {
                break;
            };
            current = next;
        }
        let state = Box::leak(Box::new(RefCell::new(state)));
        panes.insert(key, state);
        state
    })
}

/// # Safety
/// `cell` is a leaked `RefCell`, so the pointer is valid for the process.
pub(super) fn read_panes(
    cell: &'static RefCell<pane_grid::State<usize>>,
) -> &'static pane_grid::State<usize> {
    unsafe { &*cell.as_ptr() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn removed_items_keep_their_last_bounds() {
        let lua = mlua::Lua::new();
        let item = Value::Table(lua.create_table().unwrap());
        let bounds = iced::Rectangle {
            x: 4.0,
            y: 20.0,
            width: 320.0,
            height: 80.0,
        };
        let first = list(77, None, "bounds", vec![item.clone()], &[item], true, None);
        first.controls[0].state().bounds = Some(bounds);
        let removed = list(77, None, "bounds", Vec::new(), &[], true, None);
        assert_eq!(removed.leaving.len(), 1);
        assert_eq!(removed.leaving[0].control.state().bounds, Some(bounds));
    }

    #[test]
    fn recreated_table_items_can_grow() {
        let lua = mlua::Lua::new();
        for count in [1, 2] {
            let items = (0..count)
                .map(|_| Value::Table(lua.create_table().unwrap()))
                .collect::<Vec<_>>();
            let result = list(991, None, "grow", items.clone(), &items, false, None);
            assert_eq!(result.tokens.len(), count);
        }
        forget_generation(991);
    }

    #[test]
    fn stable_keys_preserve_tokens_on_reorder_and_reentry() {
        let items = vec![Value::Integer(1), Value::Integer(2)];
        let initial = list(992, None, "keys", items.clone(), &items, true, None);
        let reordered = vec![Value::Integer(2), Value::Integer(1)];
        let next = list(992, None, "keys", reordered.clone(), &reordered, true, None);
        assert_eq!(next.tokens, [initial.tokens[1], initial.tokens[0]]);
        list(992, None, "keys", vec![], &[], true, None);
        let restored = list(992, None, "keys", items.clone(), &items, true, None);
        assert_eq!(restored.tokens, initial.tokens);
        assert!(restored.leaving.is_empty());
        assert_eq!(restored.controls[0].state().event, "enter");
        forget_generation(992);
        assert!(!initial.controls[0].state().alive);
    }

    #[test]
    fn markdown_cache_releases_evicted_content_without_invalidating_live_widgets() {
        MARKDOWN.with_borrow_mut(HashMap::clear);
        let live = markdown("live");
        let old = markdown("old");
        let weak = Rc::downgrade(&old);
        drop(old);
        for index in 0..100 {
            markdown(&format!("entry {index}"));
        }
        assert!(weak.upgrade().is_none());
        assert!(!live.is_empty());
        assert!(MARKDOWN.with_borrow(|cache| cache.len() <= 32));
    }

    #[test]
    fn closing_one_window_releases_only_its_list_state() {
        let a = iced::window::Id::unique();
        let b = iced::window::Id::unique();
        for window in [a, b] {
            list(
                0,
                Some(window),
                "rows",
                vec![Value::Integer(1)],
                &[Value::Integer(1)],
                false,
                None,
            );
        }
        forget_window(0, a);
        LISTS.with_borrow(|lists| {
            assert!(!lists.contains_key(&(0, Some(a), "rows".into())));
            assert!(lists.contains_key(&(0, Some(b), "rows".into())));
        });
        forget_window(0, b);
    }
}
