//! Host-owned state for widgets that keep data outside the element tree.
//!
//! Iced recreates elements every render, so stateful widgets need a stable
//! allocation. Entries are keyed by Lua generation and widget id; a shell
//! reload gets a fresh generation. Retired generations are kept for one reload
//! so iced can drop the old element tree, then freed. Entries evicted at the
//! id limit record which windows rendered them and are freed once each of
//! those windows has rendered twice more, so the queue stays bounded without
//! freeing state a live element still points into.

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
type PaneKey = (u64, String, u8, usize);
type ListKey = (u64, Option<iced::window::Id>, String);
type Used = Vec<(iced::window::Id, u64)>;

struct EditorState {
    value: String,
    cell: Rc<RefCell<text_editor::Content>>,
    entry: u64,
}

/// A state allocation plus the id its usage record is stored under.
struct Leased<T> {
    entry: u64,
    state: T,
}

/// State removed from a map, kept alive until the windows that rendered it
/// have replaced their element trees.
struct Evicted {
    generation: u64,
    used: Used,
    // Owns the allocation only until this entry is freed.
    #[allow(dead_code)]
    retired: Retired,
}

#[allow(dead_code)] // Fields only keep the allocations alive.
enum Retired {
    Editor(Rc<RefCell<text_editor::Content>>),
    Combo(ComboState),
    Pane(PaneState),
}

thread_local! {
    static EDITORS: RefCell<HashMap<(u64, String), EditorState>> = RefCell::new(HashMap::new());
    static COMBOS: RefCell<HashMap<(u64, String), Leased<ComboState>>> =
        RefCell::new(HashMap::new());
    static PANES: RefCell<HashMap<PaneKey, Leased<PaneState>>> = RefCell::new(HashMap::new());
    static RETIRED: RefCell<Vec<Retired>> = const { RefCell::new(Vec::new()) };
    static EVICTED: RefCell<Vec<Evicted>> = const { RefCell::new(Vec::new()) };
    // Windows whose element for `id` may reference a state allocation, with
    // the render epoch of their last use.
    static USED: RefCell<HashMap<u64, Used>> = RefCell::new(HashMap::new());
    // One counter per window, advanced by `begin_render`.
    static EPOCH: RefCell<HashMap<iced::window::Id, u64>> = RefCell::new(HashMap::new());
    static NEXT_ENTRY: Cell<u64> = const { Cell::new(0) };
    static MARKDOWN: RefCell<HashMap<String, Rc<Vec<iced::widget::markdown::Item>>>> =
        RefCell::new(HashMap::new());
    static LISTS: RefCell<HashMap<ListKey, ListEntry>> = RefCell::new(HashMap::new());
    static NEXT_LIST_TOKEN: Cell<u64> = const { Cell::new(0) };
}

/// Upper bound for id-keyed state; bounds configs that generate unique ids.
const LIMIT: usize = 256;

/// Hashable form of a Lua list key. Exotic values return `None` and fall back
/// to the linear scan, so matching stays exactly `Value::eq`.
#[derive(Clone, PartialEq, Eq, Hash)]
enum Key {
    Nil,
    Bool(bool),
    Int(i64),
    Num(u64), // f64 bit pattern, -0.0 canonicalized to 0.0
    Str(String),
}

fn normalize(value: &Value) -> Option<Key> {
    match value {
        Value::Nil => Some(Key::Nil),
        Value::Boolean(flag) => Some(Key::Bool(*flag)),
        Value::Integer(int) => Some(Key::Int(*int)),
        Value::Number(number) if number.is_finite() => {
            // Lua compares 1 and 1.0 as equal; both must use one key.
            let integral =
                number.fract() == 0.0 && *number >= i64::MIN as f64 && *number < i64::MAX as f64;
            Some(if integral {
                Key::Int(*number as i64)
            } else {
                Key::Num(number.to_bits())
            })
        }
        Value::String(string) => string.to_str().ok().map(|s| Key::Str(s.to_owned())),
        _ => None,
    }
}

pub(crate) fn forget_window(generation: u64, window: iced::window::Id) {
    LISTS.with_borrow_mut(|lists| {
        lists.retain(|(owner, id, _), _| *owner != generation || *id != Some(window))
    });
    // The window's element tree is gone; its state no longer needs to wait.
    EPOCH.with_borrow_mut(|epochs| {
        epochs.remove(&window);
    });
}

/// Marks the start of one window's render and frees evicted state whose last
/// user has since rebuilt its element tree.
pub(crate) fn begin_render(window: iced::window::Id) {
    EPOCH.with_borrow_mut(|epochs| {
        *epochs.entry(window).or_default() += 1;
    });
    free_expired();
}

/// Two renders after the last use, the element from that use has been replaced
/// by the next one and then by the current one; no tree can still point into it.
fn free_expired() {
    let epochs = EPOCH.with_borrow(|epochs| epochs.clone());
    EVICTED.with_borrow_mut(|evicted| {
        evicted.retain(|entry| {
            // No recorded user means we cannot prove it safe; keep it until reload.
            entry.used.is_empty()
                || entry.used.iter().any(|(window, used)| {
                    epochs
                        .get(window)
                        .is_some_and(|current| *current <= used.saturating_add(1))
                })
        });
    });
}

fn next_entry(window: Option<iced::window::Id>) -> u64 {
    let entry = NEXT_ENTRY.with(|next| {
        let entry = next.get().wrapping_add(1);
        next.set(entry);
        entry
    });
    touch(entry, window);
    entry
}

/// Records which window rendered this allocation and when.
fn touch(entry: u64, window: Option<iced::window::Id>) {
    let Some(window) = window else {
        return;
    };
    let epoch = EPOCH.with_borrow(|epochs| epochs.get(&window).copied().unwrap_or(0));
    USED.with_borrow_mut(|used| {
        let list = used.entry(entry).or_default();
        if let Some(slot) = list.iter_mut().find(|(id, _)| *id == window) {
            slot.1 = epoch;
        } else {
            list.push((window, epoch));
        }
    });
}

pub(crate) fn forget_generation(generation: u64) {
    LISTS.with_borrow_mut(|lists| lists.retain(|(owner, _, _), _| *owner != generation));
    let mut retiring = Vec::new();
    EDITORS.with_borrow_mut(|editors| {
        editors.retain(|(owner, _), state| {
            if *owner == generation {
                USED.with_borrow_mut(|used| {
                    used.remove(&state.entry);
                });
                retiring.push(Retired::Editor(Rc::clone(&state.cell)));
                false
            } else {
                true
            }
        });
    });
    COMBOS.with_borrow_mut(|combos| {
        combos.retain(|(owner, _), lease| {
            if *owner == generation {
                USED.with_borrow_mut(|used| {
                    used.remove(&lease.entry);
                });
                retiring.push(Retired::Combo(Rc::clone(&lease.state)));
                false
            } else {
                true
            }
        });
    });
    PANES.with_borrow_mut(|panes| {
        panes.retain(|(owner, _, _, _), lease| {
            if *owner == generation {
                USED.with_borrow_mut(|used| {
                    used.remove(&lease.entry);
                });
                retiring.push(Retired::Pane(Rc::clone(&lease.state)));
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

/// Drops state evicted in older generations. Call after a reload has been
/// accepted; evictions from the new generation may still be referenced by the
/// tree that was just built.
pub(crate) fn free_evicted(generation: u64) {
    EVICTED.with_borrow_mut(|evicted| evicted.retain(|entry| entry.generation >= generation));
}

/// Evicts one entry instead of the whole map, which would reset every live
/// widget in the view.
fn retire_one<K: std::hash::Hash + Eq + Clone, V>(
    generation: u64,
    map: &mut HashMap<K, V>,
    entry: impl Fn(&V) -> u64,
    retire: impl FnOnce(V) -> Retired,
) {
    let Some(key) = map.keys().next().cloned() else {
        return;
    };
    if let Some(value) = map.remove(&key) {
        let used = USED.with_borrow_mut(|used| used.remove(&entry(&value)).unwrap_or_default());
        EVICTED.with_borrow_mut(|evicted| {
            evicted.push(Evicted {
                generation,
                used,
                retired: retire(value),
            })
        });
    }
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
    window: Option<iced::window::Id>,
    id: &str,
    value: &str,
) -> &'static RefCell<text_editor::Content> {
    EDITORS.with(|editors| {
        let mut editors = editors.borrow_mut();
        let key = (generation, id.to_owned());
        if let Some(existing) = editors.get_mut(&key) {
            touch(existing.entry, window);
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
        let entry = next_entry(window);
        if editors.len() >= LIMIT {
            retire_one(
                generation,
                &mut editors,
                |state| state.entry,
                |state| Retired::Editor(state.cell),
            );
        }
        editors.insert(
            key,
            EditorState {
                value: value.to_owned(),
                cell: Rc::clone(&cell),
                entry,
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
    window: Option<iced::window::Id>,
    id: &str,
    options: &[String],
) -> &'static combo_box::State<String> {
    COMBOS.with(|combos| {
        let mut combos = combos.borrow_mut();
        let key = (generation, id.to_owned());
        if let Some(existing) = combos.get(&key)
            && existing.state.options() == options
        {
            touch(existing.entry, window);
            return static_ref(&*existing.state);
        }
        let state = Rc::new(combo_box::State::new(options.to_vec()));
        let entry = next_entry(window);
        if combos.len() >= LIMIT && !combos.contains_key(&key) {
            retire_one(
                generation,
                &mut combos,
                |lease| lease.entry,
                |lease| Retired::Combo(lease.state),
            );
        }
        if let Some(old) = combos.insert(
            key,
            Leased {
                entry,
                state: Rc::clone(&state),
            },
        ) {
            USED.with_borrow_mut(|used| {
                used.remove(&old.entry);
            });
            RETIRED.with_borrow_mut(|retired| retired.push(Retired::Combo(old.state)));
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
            // Evict one entry like editors/combos do; clearing the whole map
            // would cancel every list's exit animations at once.
            if let Some(oldest) = lists.keys().next().cloned() {
                lists.remove(&oldest);
            }
        }
        let entry = lists.entry(key).or_default();
        entry.owner = owner;
        let previous = std::mem::take(&mut entry.items);
        let mut old_used = vec![false; previous.len()];
        let mut tokens = Vec::with_capacity(items.len());
        let mut controls = Vec::with_capacity(items.len());
        let mut next_items = Vec::with_capacity(items.len());

        // Index the previous items by normalized key. Every occurrence is
        // kept so first-unused matching matches the linear scan it replaces.
        let mut index: HashMap<Key, Vec<usize>> = HashMap::new();
        for (old_index, (_, old_key, _, _)) in previous.iter().enumerate() {
            if let Some(normalized) = normalize(old_key) {
                index.entry(normalized).or_default().push(old_index);
            }
        }

        for (item, key) in items.iter().zip(keys) {
            let matched =
                match normalize(key) {
                    Some(normalized) => index.get(&normalized).and_then(|candidates| {
                        candidates
                            .iter()
                            .copied()
                            .find(|&old_index| !old_used[old_index])
                            .map(|old_index| (old_index, previous[old_index].2))
                    }),
                    // Exotic keys (tables, functions, invalid UTF-8) fall back to
                    // the linear scan so matching stays exactly `Value::eq`.
                    None => previous.iter().enumerate().find_map(
                        |(old_index, (_, old_key, token, _))| {
                            (!old_used[old_index] && old_key == key).then_some((old_index, *token))
                        },
                    ),
                };
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
    window: Option<iced::window::Id>,
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
            touch(existing.entry, window);
            return static_ref(&*existing.state);
        }
        let (mut state, mut current) = pane_grid::State::new(0);
        for index in 1..count {
            let Some((next, _)) = state.split(axis, current, index) else {
                break;
            };
            current = next;
        }
        let state = Rc::new(RefCell::new(state));
        let entry = next_entry(window);
        if panes.len() >= LIMIT && !panes.contains_key(&key) {
            retire_one(
                generation,
                &mut panes,
                |lease| lease.entry,
                |lease| Retired::Pane(lease.state),
            );
        }
        if let Some(old) = panes.insert(
            key,
            Leased {
                entry,
                state: Rc::clone(&state),
            },
        ) {
            USED.with_borrow_mut(|used| {
                used.remove(&old.entry);
            });
            RETIRED.with_borrow_mut(|retired| retired.push(Retired::Pane(old.state)));
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
    fn mixed_type_keys_match_across_rebuilds() {
        let lua = mlua::Lua::new();
        let key = |name: &str| Value::String(lua.create_string(name).unwrap());
        // String, integer, float, and boolean keys; the float is -0.0, which
        // must match the plain 0.0 from the first build.
        let initial = list(
            995,
            None,
            "mixed",
            vec![
                key("a"),
                Value::Integer(2),
                Value::Number(0.0),
                Value::Boolean(true),
            ],
            &[
                key("a"),
                Value::Integer(2),
                Value::Number(-0.0),
                Value::Boolean(true),
            ],
            true,
            None,
        );
        let next = list(
            995,
            None,
            "mixed",
            vec![
                Value::Boolean(true),
                Value::Number(0.0),
                Value::Integer(2),
                key("a"),
            ],
            &[
                Value::Boolean(true),
                Value::Number(0.0),
                Value::Integer(2),
                key("a"),
            ],
            true,
            None,
        );
        assert_eq!(
            next.tokens,
            [
                initial.tokens[3],
                initial.tokens[2],
                initial.tokens[1],
                initial.tokens[0]
            ]
        );
        assert!(next.leaving.is_empty());
        forget_generation(995);
    }

    #[test]
    fn integral_float_keys_match_integers() {
        // Lua compares 1 and 1.0 as equal; a key that flips type between
        // renders must not reset the item's state or animation.
        let integral = list(
            997,
            None,
            "num",
            vec![Value::Integer(1)],
            &[Value::Integer(1)],
            true,
            None,
        );
        let float = list(
            997,
            None,
            "num",
            vec![Value::Number(1.0)],
            &[Value::Number(1.0)],
            true,
            None,
        );
        assert_eq!(float.tokens, integral.tokens);
        assert!(float.leaving.is_empty());
        forget_generation(997);
    }

    #[test]
    fn duplicate_previous_keys_still_match_in_order() {
        let items = vec![Value::Integer(1), Value::Integer(1), Value::Integer(1)];
        let initial = list(996, None, "dup", items.clone(), &items, true, None);
        let next = list(996, None, "dup", items.clone(), &items, true, None);
        // First-unused matching keeps every token in place.
        assert_eq!(next.tokens, initial.tokens);
        forget_generation(996);
    }

    #[test]
    fn editor_keeps_typed_text_until_the_value_prop_changes() {
        let generation = 993;
        editor(generation, None, "doc", "start");
        let typed = perform(
            generation,
            "doc",
            text_editor::Action::Edit(text_editor::Edit::Insert('!')),
        );
        assert_ne!(typed.as_deref(), Some("start"));
        editor(generation, None, "doc", "start");
        assert_eq!(text(generation, "doc"), typed);
        editor(generation, None, "doc", "reset");
        assert_eq!(text(generation, "doc").as_deref(), Some("reset"));
        forget_generation(generation);
        free_retired();
    }

    #[test]
    fn generation_sweep_frees_widget_state_after_one_reload() {
        let generation = 994;
        editor(generation, None, "doc", "text");
        combo(generation, None, "pick", &["a".to_owned()]);
        panes(generation, None, "grid", 2, pane_grid::Axis::Horizontal);
        let editor = EDITORS
            .with_borrow(|editors| Rc::downgrade(&editors[&(generation, "doc".to_owned())].cell));
        let combo = COMBOS
            .with_borrow(|combos| Rc::downgrade(&combos[&(generation, "pick".to_owned())].state));
        let pane = PANES.with_borrow(|panes| {
            Rc::downgrade(&panes[&(generation, "grid".to_owned(), 0, 2)].state)
        });

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
    fn eviction_at_the_state_limit_keeps_the_rest_of_the_map() {
        let generation = 995;
        EDITORS.with_borrow_mut(HashMap::clear);
        free_retired();
        free_evicted(u64::MAX);
        for index in 0..LIMIT {
            editor(generation, None, &format!("editor-{index}"), "");
        }
        editor(generation, None, "overflow", "");
        assert_eq!(
            EDITORS.with_borrow(|editors| editors.keys().filter(|key| key.0 == generation).count()),
            LIMIT
        );
        assert_eq!(EVICTED.with_borrow(|evicted| evicted.len()), 1);
        forget_generation(generation);
        free_retired();
        free_evicted(u64::MAX);
    }

    #[test]
    fn evicted_state_is_freed_after_its_window_renders_twice() {
        let generation = 998;
        let window = iced::window::Id::unique();
        EDITORS.with_borrow_mut(HashMap::clear);
        USED.with_borrow_mut(HashMap::clear);
        EVICTED.with_borrow_mut(Vec::clear);
        EPOCH.with_borrow_mut(HashMap::clear);
        begin_render(window); // epoch 1
        for index in 0..LIMIT {
            editor(generation, Some(window), &format!("e-{index}"), "");
        }
        let cells = EDITORS.with_borrow(|editors| {
            editors
                .values()
                .map(|state| Rc::clone(&state.cell))
                .collect::<Vec<_>>()
        });
        editor(generation, Some(window), "overflow", "");
        assert_eq!(EVICTED.with_borrow(|evicted| evicted.len()), 1);
        // One render is not enough: the previous tree may still reference it.
        begin_render(window); // epoch 2
        assert_eq!(EVICTED.with_borrow(|evicted| evicted.len()), 1);
        begin_render(window); // epoch 3
        assert!(EVICTED.with_borrow(|evicted| evicted.is_empty()));
        // Exactly the evicted allocation lost its extra owner.
        assert_eq!(
            cells
                .iter()
                .filter(|cell| Rc::strong_count(cell) == 1)
                .count(),
            1
        );
        forget_window(generation, window);
        forget_generation(generation);
        free_retired();
        free_evicted(u64::MAX);
    }

    #[test]
    fn stateful_widget_maps_are_bounded() {
        let generation = 987_654_321;
        for index in 0..LIMIT + 8 {
            editor(generation, None, &format!("editor-{index}"), "");
            combo(generation, None, &format!("combo-{index}"), &[]);
            panes(
                generation,
                None,
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
        free_evicted(u64::MAX);
    }
}
