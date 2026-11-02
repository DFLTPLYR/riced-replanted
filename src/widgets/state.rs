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
    items: Vec<Value>,
    leaving: Vec<(Value, usize, Instant)>,
}

pub(super) struct ListState {
    pub(super) items: Vec<Value>,
    pub(super) leaving: Vec<(Value, usize, Instant)>,
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
pub(super) fn list(
    generation: u64,
    id: &str,
    items: Vec<Value>,
    animate: bool,
    duration: Duration,
) -> ListState {
    LISTS.with(|lists| {
        let mut lists = lists.borrow_mut();
        let entry = lists.entry((generation, id.to_owned())).or_default();
        let previous = std::mem::replace(&mut entry.items, items.clone());
        entry.leaving.retain(|(_, _, at)| at.elapsed() < duration);
        if animate {
            for (index, item) in previous.iter().enumerate().skip(items.len()) {
                if !entry
                    .leaving
                    .iter()
                    .any(|(existing, _, _)| existing == item)
                {
                    entry.leaving.push((item.clone(), index, Instant::now()));
                }
            }
        } else {
            entry.leaving.clear();
        }
        ListState {
            items: previous,
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
