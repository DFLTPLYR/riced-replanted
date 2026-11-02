//! Host-owned state for widgets that keep data outside the element tree.
//!
//! Iced recreates elements every render, so stateful widgets need a stable
//! allocation. Entries are keyed by Lua generation and widget id; a shell
//! reload gets a fresh generation. Replaced entries are leaked:
//! `shortcut: state is kept for the process lifetime, add a generation sweep
//! if profiles churn through ids or options`.

use std::{
    cell::RefCell,
    collections::HashMap,
    time::{Duration, Instant},
};

use iced::widget::{combo_box, pane_grid, text_editor};
use mlua::Value;

type PaneState = &'static RefCell<pane_grid::State<usize>>;

thread_local! {
    static EDITORS: RefCell<HashMap<(u64, String), &'static RefCell<text_editor::Content>>> =
        RefCell::new(HashMap::new());
    static COMBOS: RefCell<HashMap<(u64, String), &'static combo_box::State<String>>> =
        RefCell::new(HashMap::new());
    static PANES: RefCell<HashMap<(u64, String, u8, usize), PaneState>> =
        RefCell::new(HashMap::new());
    static MARKDOWN: RefCell<std::collections::HashMap<String, &'static [iced::widget::markdown::Item]>> =
        RefCell::new(std::collections::HashMap::new());
    static LISTS: RefCell<HashMap<(u64, String), ListEntry>> = RefCell::new(HashMap::new());
}

#[derive(Default)]
struct ListEntry {
    items: Vec<(Value, Value, u64)>,
    leaving: Vec<ListLeaving>,
    next_token: u64,
}

#[derive(Clone)]
pub(super) struct ListLeaving {
    pub(super) item: Value,
    pub(super) index: usize,
    pub(super) token: u64,
    pub(super) at: Instant,
}

pub(super) struct ListState {
    pub(super) tokens: Vec<u64>,
    pub(super) entering: Vec<bool>,
    pub(super) added: Vec<(usize, Value)>,
    pub(super) removed: Vec<(usize, Value)>,
    pub(super) displaced: Vec<(usize, Value)>,
    pub(super) leaving: Vec<ListLeaving>,
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

/// Parsed markdown is stable for a given source string, so a profile that
/// re-renders identical text does not re-parse or leak.
pub(super) fn markdown(content: &str) -> &'static [iced::widget::markdown::Item] {
    MARKDOWN.with(|cache| {
        let mut cache = cache.borrow_mut();
        if let Some(items) = cache.get(content) {
            return *items;
        }
        let items: Vec<_> = iced::widget::markdown::parse(content).collect();
        let items: &'static [_] = Box::leak(items.into_boxed_slice());
        cache.insert(content.to_owned(), items);
        items
    })
}

/// Previous rendered items for a list view, plus items still animating out.
///
/// Items are matched by `keys` so a middle insert/removal keeps the surviving
/// widgets and animations. When nothing matches at all, index matching keeps
/// the old count-based behavior for lists whose Lua values are rebuilt every
/// render.
pub(super) fn list(
    generation: u64,
    id: &str,
    items: Vec<Value>,
    keys: &[Value],
    animate: bool,
    duration: Duration,
) -> ListState {
    LISTS.with(|lists| {
        let mut lists = lists.borrow_mut();
        let entry = lists.entry((generation, id.to_owned())).or_default();
        let previous = std::mem::take(&mut entry.items);
        let mut old_used = vec![false; previous.len()];
        let mut tokens = Vec::with_capacity(items.len());
        let mut entering = Vec::with_capacity(items.len());
        let mut added = Vec::new();
        let mut displaced = Vec::new();
        let mut next_items = Vec::with_capacity(items.len());

        let positional = !previous.is_empty()
            && !keys.is_empty()
            && !previous
                .iter()
                .any(|(_, old_key, _)| keys.iter().any(|key| key == old_key));

        for (index, (item, key)) in items.iter().zip(keys).enumerate() {
            let matched = if positional {
                (index < previous.len()).then_some((index, previous[index].2))
            } else {
                previous
                    .iter()
                    .enumerate()
                    .find_map(|(old_index, (_, old_key, token))| {
                        (!old_used[old_index] && old_key == key).then_some((old_index, *token))
                    })
            };
            let (token, old_index) = match matched {
                Some((old_index, token)) => {
                    old_used[old_index] = true;
                    (token, Some(old_index))
                }
                None => {
                    let token = entry.next_token;
                    entry.next_token += 1;
                    added.push((index, item.clone()));
                    (token, None)
                }
            };
            if let Some(old_index) = old_index
                && old_index != index
            {
                displaced.push((index, item.clone()));
            }
            tokens.push(token);
            entering.push(old_index.is_none());
            next_items.push((item.clone(), key.clone(), token));
        }

        let mut removed = Vec::new();
        let mut leaving = Vec::new();
        for (index, (item, _, token)) in previous.iter().enumerate() {
            if !old_used[index] {
                removed.push((index, item.clone()));
                leaving.push(ListLeaving {
                    item: item.clone(),
                    index,
                    token: *token,
                    at: Instant::now(),
                });
            }
        }

        entry.items = next_items;
        if animate {
            entry
                .leaving
                .retain(|leaving| leaving.at.elapsed() < duration);
            entry.leaving.extend(leaving);
        } else {
            entry.leaving.clear();
        }
        ListState {
            tokens,
            entering,
            added,
            removed,
            displaced,
            leaving: entry.leaving.clone(),
        }
    })
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
