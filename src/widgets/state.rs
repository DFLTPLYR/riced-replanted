//! Host-owned state for widgets that keep data outside the element tree.
//!
//! Iced recreates elements every render, so stateful widgets need a stable
//! allocation. Entries are keyed by Lua generation and widget id; a shell
//! reload gets a fresh generation. Retired generations are kept for one reload
//! so iced can drop the old element tree, then freed.

use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    rc::Rc,
};

use super::animation::Control;
use iced::widget::{combo_box, pane_grid, text_editor};
use mlua::Value;

type PaneState = Rc<RefCell<pane_grid::State<usize>>>;
type ComboState = Rc<combo_box::State<String>>;
type ListKey = (u64, Option<iced::window::Id>, String);

struct EditorState {
    value: String,
    cell: Rc<RefCell<text_editor::Content>>,
}

#[allow(dead_code)] // Fields only keep the allocations alive.
enum Retired {
    Editor(Rc<RefCell<text_editor::Content>>),
    Combo(ComboState),
    Pane(PaneState),
}

thread_local! {
    static EDITORS: RefCell<HashMap<(u64, String), EditorState>> = RefCell::new(HashMap::new());
    static COMBOS: RefCell<HashMap<(u64, String), ComboState>> = RefCell::new(HashMap::new());
    static PANES: RefCell<HashMap<(u64, String, u8, usize), PaneState>> =
        RefCell::new(HashMap::new());
    static RETIRED: RefCell<Vec<Retired>> = const { RefCell::new(Vec::new()) };
    static MARKDOWN: RefCell<HashMap<String, Rc<Vec<iced::widget::markdown::Item>>>> =
        RefCell::new(HashMap::new());
    static LISTS: RefCell<HashMap<ListKey, ListEntry>> = RefCell::new(HashMap::new());
    static NEXT_LIST_TOKEN: Cell<u64> = const { Cell::new(0) };
}

/// Upper bound for id-keyed state; bounds configs that generate unique ids.
const LIMIT: usize = 256;

pub(crate) fn forget_window(generation: u64, window: iced::window::Id) {
    LISTS.with_borrow_mut(|lists| {
        lists.retain(|(owner, id, _), _| *owner != generation || *id != Some(window))
    });
}

pub(crate) fn forget_generation(generation: u64) {
    LISTS.with_borrow_mut(|lists| lists.retain(|(owner, _, _), _| *owner != generation));
    let mut retiring = Vec::new();
    EDITORS.with_borrow_mut(|editors| {
        editors.retain(|(owner, _), state| {
            if *owner == generation {
                retiring.push(Retired::Editor(Rc::clone(&state.cell)));
                false
            } else {
                true
            }
        });
    });
    COMBOS.with_borrow_mut(|combos| {
        combos.retain(|(owner, _), state| {
            if *owner == generation {
                retiring.push(Retired::Combo(Rc::clone(&*state)));
                false
            } else {
                true
            }
        });
    });
    PANES.with_borrow_mut(|panes| {
        panes.retain(|(owner, _, _, _), state| {
            if *owner == generation {
                retiring.push(Retired::Pane(Rc::clone(state)));
                false
            } else {
                true
            }
        });
    });
    RETIRED.with_borrow_mut(|retired| retired.extend(retiring));
}

/// Drops state retired by the previous reload. Call before building a new
/// generation so iced has replaced the element tree that referenced it.
pub(crate) fn free_retired() {
    RETIRED.with_borrow_mut(Vec::clear);
}

fn static_ref<T>(value: &T) -> &'static T {
    // SAFETY: the owning Rc lives in a thread-local map or the retired list.
    unsafe { &*(value as *const T) }
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

/// Stable editor content. `value` is the last value prop; when it changes the
/// text is replaced in place, so a static value does not erase typing.
pub(super) fn editor(
    generation: u64,
    id: &str,
    value: &str,
) -> &'static RefCell<text_editor::Content> {
    EDITORS.with(|editors| {
        let mut editors = editors.borrow_mut();
        let key = (generation, id.to_owned());
        if let Some(existing) = editors.get_mut(&key) {
            if existing.value != value {
                let mut content = existing.cell.borrow_mut();
                content.perform(text_editor::Action::SelectAll);
                content.perform(text_editor::Action::Edit(text_editor::Edit::Paste(
                    std::sync::Arc::new(value.to_owned()),
                )));
                existing.value = value.to_owned();
            }
            return static_ref(&*existing.cell);
        }
        let cell = Rc::new(RefCell::new(text_editor::Content::with_text(value)));
        if editors.len() >= LIMIT {
            RETIRED.with_borrow_mut(|retired| {
                retired.extend(
                    editors
                        .drain()
                        .map(|(_, state)| Retired::Editor(state.cell)),
                );
            });
        }
        editors.insert(
            key,
            EditorState {
                value: value.to_owned(),
                cell: Rc::clone(&cell),
            },
        );
        static_ref(&*cell)
    })
}

/// # Safety
/// `cell` is owned by the editor map or the retired list.
pub(super) fn read_editor(
    cell: &'static RefCell<text_editor::Content>,
) -> &'static text_editor::Content {
    unsafe { &*cell.as_ptr() }
}

pub(super) fn perform(generation: u64, id: &str, action: text_editor::Action) -> Option<String> {
    EDITORS.with(|editors| {
        let editors = editors.borrow();
        let state = editors.get(&(generation, id.to_owned()))?;
        let mut content = state.cell.borrow_mut();
        content.perform(action);
        Some(content.text())
    })
}

pub(super) fn text(generation: u64, id: &str) -> Option<String> {
    EDITORS.with(|editors| {
        let editors = editors.borrow();
        let state = editors.get(&(generation, id.to_owned()))?;
        Some(state.cell.borrow().text())
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
            return static_ref(&**existing);
        }
        let state = Rc::new(combo_box::State::new(options.to_vec()));
        if combos.len() >= LIMIT && !combos.contains_key(&key) {
            RETIRED.with_borrow_mut(|retired| {
                retired.extend(combos.drain().map(|(_, state)| Retired::Combo(state)));
            });
        }
        if let Some(old) = combos.insert(key, Rc::clone(&state)) {
            RETIRED.with_borrow_mut(|retired| retired.push(Retired::Combo(old)));
        }
        static_ref(&*state)
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
        let key = (generation, window, id.to_owned());
        if lists.len() >= LIMIT && !lists.contains_key(&key) {
            lists.clear();
        }
        let entry = lists.entry(key).or_default();
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
            return static_ref(&**existing);
        }
        let (mut state, mut current) = pane_grid::State::new(0);
        for index in 1..count {
            let Some((next, _)) = state.split(axis, current, index) else {
                break;
            };
            current = next;
        }
        let state = Rc::new(RefCell::new(state));
        if panes.len() >= LIMIT && !panes.contains_key(&key) {
            RETIRED.with_borrow_mut(|retired| {
                retired.extend(panes.drain().map(|(_, state)| Retired::Pane(state)));
            });
        }
        if let Some(old) = panes.insert(key, Rc::clone(&state)) {
            RETIRED.with_borrow_mut(|retired| retired.push(Retired::Pane(old)));
        }
        static_ref(&*state)
    })
}

/// # Safety
/// `cell` is owned by the pane map or the retired list.
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
    fn editor_keeps_typed_text_until_the_value_prop_changes() {
        let generation = 993;
        editor(generation, "doc", "start");
        let typed = perform(
            generation,
            "doc",
            text_editor::Action::Edit(text_editor::Edit::Insert('!')),
        );
        assert_ne!(typed.as_deref(), Some("start"));
        editor(generation, "doc", "start");
        assert_eq!(text(generation, "doc"), typed);
        editor(generation, "doc", "reset");
        assert_eq!(text(generation, "doc").as_deref(), Some("reset"));
        forget_generation(generation);
        free_retired();
    }

    #[test]
    fn generation_sweep_frees_widget_state_after_one_reload() {
        let generation = 994;
        editor(generation, "doc", "text");
        combo(generation, "pick", &["a".to_owned()]);
        panes(generation, "grid", 2, pane_grid::Axis::Horizontal);
        let editor = EDITORS
            .with_borrow(|editors| Rc::downgrade(&editors[&(generation, "doc".to_owned())].cell));
        let combo =
            COMBOS.with_borrow(|combos| Rc::downgrade(&combos[&(generation, "pick".to_owned())]));
        let pane = PANES
            .with_borrow(|panes| Rc::downgrade(&panes[&(generation, "grid".to_owned(), 0, 2)]));

        forget_generation(generation);
        assert!(
            editor.upgrade().is_some(),
            "iced may still hold the old tree"
        );

        free_retired();
        assert!(editor.upgrade().is_none());
        assert!(combo.upgrade().is_none());
        assert!(pane.upgrade().is_none());
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

    #[test]
    fn stateful_widget_maps_are_bounded() {
        let generation = 987_654_321;
        for index in 0..LIMIT + 8 {
            editor(generation, &format!("editor-{index}"), "");
            combo(generation, &format!("combo-{index}"), &[]);
            panes(
                generation,
                &format!("pane-{index}"),
                1,
                pane_grid::Axis::Horizontal,
            );
        }
        assert!(
            EDITORS.with_borrow(|editors| editors.keys().filter(|key| key.0 == generation).count())
                <= LIMIT
        );
        assert!(
            COMBOS.with_borrow(|combos| combos.keys().filter(|key| key.0 == generation).count())
                <= LIMIT
        );
        assert!(
            PANES.with_borrow(|panes| panes.keys().filter(|key| key.0 == generation).count())
                <= LIMIT
        );
        for index in 0..LIMIT + 8 {
            let items = vec![Value::Integer(index as i64)];
            list(
                generation,
                None,
                &format!("list-{index}"),
                items.clone(),
                &items,
                false,
                None,
            );
        }
        assert!(
            LISTS.with_borrow(|lists| lists.keys().filter(|key| key.0 == generation).count())
                <= LIMIT
        );
        free_retired();
    }
}
