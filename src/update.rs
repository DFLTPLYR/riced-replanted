use iced::window;
use iced_exwlshell::{
    actions::{
        ExwlShellCustomAction as Action, ExwlShellCustomActionWithId, IcedXdgWindowSettings,
    },
    reexport::{NewLayerShellSettings, PixelSize},
};

#[derive(Clone)]
pub struct Layer {
    pub id: String,
    pub window: window::Id,
    pub settings: NewLayerShellSettings,
}

#[derive(Clone)]
pub struct Window {
    pub id: String,
    pub window: window::Id,
    pub title: String,
    pub size: PixelSize,
    pub client_side_decorations: bool,
    pub closed: bool,
}

pub fn reconcile(previous: &[Layer], next: &mut [Layer]) -> Vec<ExwlShellCustomActionWithId> {
    let mut actions = Vec::new();
    let mut used = Vec::new();
    let next_ids = next
        .iter()
        .map(|layer| layer.id.clone())
        .collect::<Vec<_>>();
    for (index, layer) in next.iter_mut().enumerate() {
        let available = |old: &&Layer| !next_ids.contains(&old.id) && !used.contains(&old.window);
        let old = previous
            .iter()
            .find(|old| old.id == layer.id)
            .or_else(|| previous.get(index).filter(available))
            .or_else(|| previous.iter().find(available));
        if let Some(old) = old {
            used.push(old.window);
            let before = &old.settings;
            let after = &layer.settings;
            // Output, namespace and input transparency have no native update action.
            if before.output_option != after.output_option
                || before.namespace != after.namespace
                || before.events_transparent != after.events_transparent
            {
                actions.push(ExwlShellCustomActionWithId::new(
                    Some(old.window),
                    Action::RemoveWindow,
                ));
            } else {
                layer.window = old.window;
                let mut push = |action| {
                    actions.push(ExwlShellCustomActionWithId::new(Some(old.window), action))
                };
                if before.size != after.size || before.anchor != after.anchor {
                    push(Action::LayoutChange {
                        anchor: after.anchor,
                        size: after.size,
                    });
                }
                if before.layer != after.layer {
                    push(Action::LayerChange(after.layer));
                }
                if before.margin != after.margin {
                    push(Action::MarginChange(after.margin.unwrap_or_default()));
                }
                if before.exclusive_zone != after.exclusive_zone {
                    push(Action::ExclusiveZoneChange(
                        after.exclusive_zone.unwrap_or(0),
                    ));
                }
                if before.keyboard_interactivity != after.keyboard_interactivity {
                    push(Action::KeyboardInteractivityChange(
                        after.keyboard_interactivity,
                    ));
                }
                if before.blur_option != after.blur_option {
                    push(Action::BlurOptionChange(after.blur_option.clone()));
                }
                continue;
            }
        }
        actions.push(ExwlShellCustomActionWithId::new(
            None,
            Action::NewLayerShell {
                id: layer.window,
                settings: layer.settings.clone(),
            },
        ));
    }
    for old in previous {
        if !used.contains(&old.window) {
            actions.push(ExwlShellCustomActionWithId::new(
                Some(old.window),
                Action::RemoveWindow,
            ));
        }
    }
    actions
}

pub fn reconcile_windows(
    previous: &[Window],
    next: &mut [Window],
) -> Vec<ExwlShellCustomActionWithId> {
    let mut actions = Vec::new();
    let mut used = Vec::new();
    let next_ids = next
        .iter()
        .map(|window| window.id.clone())
        .collect::<Vec<_>>();
    for (index, window) in next.iter_mut().enumerate() {
        let available = |old: &&Window| !next_ids.contains(&old.id) && !used.contains(&old.window);
        let old = previous
            .iter()
            .find(|old| old.id == window.id)
            .or_else(|| previous.get(index).filter(available))
            .or_else(|| previous.iter().find(available));
        let Some(old) = old else {
            actions.push(open_window(window));
            continue;
        };
        used.push(old.window);
        window.closed = old.closed;
        if old.closed {
            window.window = old.window;
        } else if old.size != window.size
            || old.client_side_decorations != window.client_side_decorations
        {
            actions.push(ExwlShellCustomActionWithId::new(
                Some(old.window),
                Action::RemoveWindow,
            ));
            window.window = window::Id::unique();
            actions.push(open_window(window));
        } else {
            window.window = old.window;
        }
    }
    for old in previous {
        if !used.contains(&old.window) {
            actions.push(ExwlShellCustomActionWithId::new(
                Some(old.window),
                Action::RemoveWindow,
            ));
        }
    }
    actions
}

fn open_window(window: &Window) -> ExwlShellCustomActionWithId {
    ExwlShellCustomActionWithId::new(
        None,
        Action::NewBaseWindow {
            settings: IcedXdgWindowSettings {
                size: Some(window.size),
                client_side_decorations: window.client_side_decorations,
            },
            id: window.window,
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use iced_exwlshell::reexport::{
        Anchor, BlurOption, KeyboardInteractivity, Layer as Level, LayerSize, OutputOption,
    };

    fn layer(id: &str) -> Layer {
        Layer {
            id: id.into(),
            window: window::Id::unique(),
            settings: NewLayerShellSettings::default(),
        }
    }

    #[test]
    fn emits_native_actions_for_mutable_settings() {
        let old = layer("panel");
        let mut next = old.clone();
        next.settings.size = LayerSize::fill_width(64);
        next.settings.anchor = Anchor::Top | Anchor::Left | Anchor::Right;
        next.settings.layer = Level::Bottom;
        next.settings.margin = Some((1, 2, 3, 4));
        next.settings.exclusive_zone = Some(64);
        next.settings.keyboard_interactivity = KeyboardInteractivity::Exclusive;
        next.settings.blur_option = BlurOption::FullRegion;
        let actions = reconcile(std::slice::from_ref(&old), std::slice::from_mut(&mut next));
        assert_eq!(next.window, old.window);
        assert_eq!(actions.len(), 6);
        assert!(actions.iter().all(|action| action.0 == Some(old.window)));
        assert!(matches!(actions[0].1, Action::LayoutChange { .. }));
        assert!(matches!(actions[1].1, Action::LayerChange(Level::Bottom)));
        assert!(matches!(actions[2].1, Action::MarginChange((1, 2, 3, 4))));
        assert!(matches!(actions[3].1, Action::ExclusiveZoneChange(64)));
        assert!(matches!(
            actions[4].1,
            Action::KeyboardInteractivityChange(KeyboardInteractivity::Exclusive)
        ));
        assert!(matches!(
            actions[5].1,
            Action::BlurOptionChange(BlurOption::FullRegion)
        ));
    }

    #[test]
    fn clearing_optional_settings_resets_native_values() {
        let mut old = layer("panel");
        old.settings.exclusive_zone = Some(32);
        old.settings.margin = Some((1, 2, 3, 4));
        let mut next = vec![layer("panel")];
        let actions = reconcile(&[old], &mut next);
        assert!(matches!(actions[0].1, Action::MarginChange((0, 0, 0, 0))));
        assert!(matches!(actions[1].1, Action::ExclusiveZoneChange(0)));
    }

    #[test]
    fn recreates_only_surfaces_with_creation_only_changes() {
        for change in 0..3 {
            let old = layer("panel");
            let mut next = vec![layer("panel")];
            match change {
                0 => next[0].settings.output_option = OutputOption::OutputName("DP-1".into()),
                1 => next[0].settings.namespace = Some("new".into()),
                _ => next[0].settings.events_transparent = true,
            }
            let actions = reconcile(std::slice::from_ref(&old), &mut next);
            assert_eq!(actions.len(), 2);
            assert_eq!(actions[0].0, Some(old.window));
            assert!(matches!(actions[0].1, Action::RemoveWindow));
            assert!(
                matches!(actions[1].1, Action::NewLayerShell { id, .. } if id == next[0].window)
            );
            assert_ne!(next[0].window, old.window);
        }
    }

    fn window(id: &str) -> Window {
        Window {
            id: id.into(),
            window: window::Id::unique(),
            title: "Riced".into(),
            size: PixelSize::px(640, 480),
            client_side_decorations: false,
            closed: false,
        }
    }

    #[test]
    fn reconciles_native_windows() {
        let old = window("settings");
        let mut next = vec![window("settings")];
        assert!(reconcile_windows(std::slice::from_ref(&old), &mut next).is_empty());
        assert_eq!(next[0].window, old.window);

        next[0].size = PixelSize::px(800, 600);
        let actions = reconcile_windows(std::slice::from_ref(&old), &mut next);
        assert_eq!(actions.len(), 2);
        assert!(matches!(actions[0].1, Action::RemoveWindow));
        assert!(matches!(actions[1].1, Action::NewBaseWindow { id, .. } if id == next[0].window));
        assert_ne!(next[0].window, old.window);

        let removed = reconcile_windows(&next, &mut []);
        assert!(matches!(removed[0].1, Action::RemoveWindow));
    }

    #[test]
    fn closed_windows_do_not_reopen_until_removed() {
        let mut old = window("settings");
        old.closed = true;
        let mut next = vec![window("settings")];
        next[0].size = PixelSize::px(800, 600);
        assert!(reconcile_windows(std::slice::from_ref(&old), &mut next).is_empty());
        assert_eq!(next[0].window, old.window);
        assert!(next[0].closed);
    }

    #[test]
    fn reorders_renames_adds_and_removes_layers() {
        let a = layer("a");
        let b = layer("b");
        let mut next = vec![layer("b"), layer("a")];
        assert!(reconcile(&[a.clone(), b.clone()], &mut next).is_empty());
        assert_eq!(next[0].window, b.window);
        assert_eq!(next[1].window, a.window);
        next[1].id = "#2".into();
        assert!(reconcile(&[a.clone(), b.clone()], &mut next).is_empty());
        assert_eq!(next[1].window, a.window);
        let mut next = vec![layer("b")];
        let actions = reconcile(&[a, b.clone()], &mut next);
        assert_eq!(actions.len(), 1);
        assert!(matches!(actions[0].1, Action::RemoveWindow));
        next.push(layer("c"));
        let actions = reconcile(&[b], &mut next);
        assert_eq!(actions.len(), 1);
        assert!(matches!(actions[0].1, Action::NewLayerShell { .. }));
    }
}
