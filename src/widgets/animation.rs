use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use iced::Rectangle;
use iced_anim::{Animate, Animated, animated::Mode, spring::Motion, transition::Easing};
use mlua::{Function, Table, UserData, UserDataMethods, Value};

#[derive(Debug, Clone, Copy, PartialEq, Animate)]
pub(crate) struct Transform {
    pub x: f32,
    pub y: f32,
    pub scale: f32,
}

impl Default for Transform {
    fn default() -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            scale: 1.0,
        }
    }
}

impl Transform {
    fn parse(table: Option<Table>) -> mlua::Result<Option<Self>> {
        let Some(table) = table else { return Ok(None) };
        crate::runtime::keys(&table, &["x", "y", "scale"])?;
        let value = Self {
            x: table.get::<Option<f32>>("x")?.unwrap_or(0.0),
            y: table.get::<Option<f32>>("y")?.unwrap_or(0.0),
            scale: table.get::<Option<f32>>("scale")?.unwrap_or(1.0),
        };
        if !value.x.is_finite()
            || !value.y.is_finite()
            || !value.scale.is_finite()
            || value.scale < 0.0
        {
            return Err(mlua::Error::runtime(
                "transition offsets must be finite and scale must be finite and non-negative",
            ));
        }
        Ok(Some(value))
    }
}

pub(super) fn mode(table: &Table) -> mlua::Result<Mode> {
    let duration = table.get::<Option<f64>>("duration")?.unwrap_or(200.0);
    if !(1.0..=86_400_000.0).contains(&duration) || duration.fract() != 0.0 {
        return Err(mlua::Error::runtime(
            "transition duration must be a whole number from 1 to 86400000 ms",
        ));
    }
    let duration = Duration::from_millis(duration as u64);
    let easing = table.get::<Option<String>>("easing")?;
    let spring = table.get::<Option<String>>("spring")?;
    if easing.is_some() && spring.is_some() {
        return Err(mlua::Error::runtime(
            "transition takes easing or spring, not both",
        ));
    }
    if let Some(spring) = spring {
        let motion = match spring.as_str() {
            "smooth" => Motion::SMOOTH,
            "snappy" => Motion::SNAPPY,
            "bouncy" => Motion::BOUNCY,
            "instant" => Motion::INSTANT,
            _ => return Err(mlua::Error::runtime(format!("unknown spring: {spring}"))),
        };
        return Ok(Mode::Spring(motion.with_duration(duration)));
    }
    let easing = match easing.as_deref().unwrap_or("out") {
        "linear" => Easing::LINEAR,
        "ease" => Easing::EASE,
        "in" | "ease_in" => Easing::EASE_IN,
        "out" | "ease_out" => Easing::EASE_OUT,
        "in_out" | "ease_in_out" => Easing::EASE_IN_OUT,
        name => return Err(mlua::Error::runtime(format!("unknown easing: {name}"))),
    };
    Ok(Mode::Transition(easing.with_duration(duration)))
}

#[derive(Debug, Clone)]
pub(crate) struct Effect {
    pub from: Option<Transform>,
    pub to: Option<Transform>,
    pub mode: Mode,
    pub callback: Option<Function>,
}

pub(crate) type Effects = BTreeMap<String, Effect>;

pub(super) fn parse(node: &Table) -> mlua::Result<Effects> {
    let config = match node.get::<Value>("transition")? {
        Value::Nil => return Ok(Effects::new()),
        Value::Table(table) => table,
        _ => {
            return Err(mlua::Error::runtime(
                "transition must be an event table, e.g. {enter = {from = {scale = 0}}}",
            ));
        }
    };
    let mut effects = Effects::new();
    for pair in config.pairs::<String, Table>() {
        let (name, table) = pair?;
        if name == "change" {
            continue;
        }
        if name.is_empty()
            || [
                "props",
                "duration",
                "easing",
                "spring",
                "x",
                "y",
                "onAdded",
                "onRemoved",
                "onDisplaced",
            ]
            .contains(&name.as_str())
        {
            return Err(mlua::Error::runtime(format!(
                "invalid transition event: {name}"
            )));
        }
        crate::runtime::keys(
            &table,
            &["from", "to", "duration", "easing", "spring", "callback"],
        )?;
        if name == "displaced" && (table.contains_key("from")? || table.contains_key("to")?) {
            return Err(mlua::Error::runtime(
                "displaced uses measured positions; it does not accept from or to",
            ));
        }
        effects.insert(
            name,
            Effect {
                from: Transform::parse(table.get("from")?)?,
                to: Transform::parse(table.get("to")?)?,
                mode: mode(&table)?,
                callback: table.get("callback")?,
            },
        );
    }
    Ok(effects)
}

#[derive(Debug, Clone)]
pub(crate) struct Playback {
    pub event: String,
    pub serial: u64,
    pub applied: u64,
    pub motion: Animated<Transform>,
    pub hidden: bool,
    pub alive: bool,
    pub completed: bool,
    pub pending_callback: bool,
    pub bounds: Option<Rectangle>,
}

#[derive(Debug, Clone)]
pub struct Control(Arc<Mutex<Playback>>);

impl Default for Control {
    fn default() -> Self {
        Self(Arc::new(Mutex::new(Playback {
            event: "enter".into(),
            serial: 1,
            applied: 0,
            motion: Animated::new(Transform::default(), Easing::LINEAR),
            hidden: false,
            alive: true,
            completed: false,
            pending_callback: false,
            bounds: None,
        })))
    }
}

impl Control {
    pub(crate) fn state(&self) -> MutexGuard<'_, Playback> {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    pub(crate) fn request(&self, event: &str) {
        let mut state = self.state();
        if !state.alive || (state.event == event && (!state.completed || event == "exit")) {
            return;
        }
        state.event = event.to_owned();
        state.serial += 1;
        state.completed = false;
        state.pending_callback = false;
    }

    pub(crate) fn sync(&self, effects: &Effects) {
        let mut state = self.state();
        if state.applied == state.serial {
            return;
        }
        let effect = effects.get(&state.event);
        let initial = state.applied == 0;
        let current = *state.motion.value();
        let from = if initial || !state.motion.is_animating() {
            effect.and_then(|e| e.from).unwrap_or(current)
        } else {
            current
        };
        let callback_only = state.event != "enter"
            && effect.is_some_and(|effect| effect.from.is_none() && effect.to.is_none());
        let target = if callback_only {
            current
        } else {
            effect.and_then(|e| e.to).unwrap_or_default()
        };
        state.motion = Animated::new(from, effect.map(|e| e.mode).unwrap_or_default()).to(target);
        state.hidden = false;
        state.applied = state.serial;
    }

    pub(crate) fn cancel(&self) {
        let mut state = self.state();
        state.alive = false;
        state.pending_callback = false;
    }

    pub(crate) fn removable(&self) -> bool {
        let state = self.state();
        !state.alive || (state.hidden && !state.pending_callback)
    }

    pub(crate) fn accept(&self, serial: u64) -> bool {
        let mut state = self.state();
        if !state.alive || state.serial != serial || !state.pending_callback {
            return false;
        }
        state.pending_callback = false;
        true
    }
}

impl UserData for Control {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("checkpoint", |lua, this, ()| {
            let control = this.clone();
            let playback = this.state().clone();
            lua.create_function(move |_, ()| {
                *control.state() = playback.clone();
                Ok(())
            })
        });
        methods.add_method("start", |_, this, event: String| {
            this.request(&event);
            Ok(())
        });
        methods.add_method("removable", |_, this, ()| Ok(this.removable()));
        methods.add_method("cancel", |_, this, ()| {
            this.cancel();
            Ok(())
        });
    }
}

pub(crate) fn install(lua: &mlua::Lua) -> mlua::Result<()> {
    let riced: Table = lua.globals().get("riced")?;
    riced.set(
        "_new_animation",
        lua.create_function(|_, ()| Ok(Control::default()))?,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    #[test]
    fn reentry_retargets_the_displayed_value_and_invalidates_exit_completion() {
        let lua = mlua::Lua::new();
        let node = lua.load("{transition={enter={from={scale=0},duration=100,easing='linear'},exit={to={scale=0},duration=100,easing='linear'}}}").eval().unwrap();
        let effects = parse(&node).unwrap();
        let control = Control::default();
        control.sync(&effects);
        control.state().motion.settle();
        control.state().completed = true;
        control.request("exit");
        control.sync(&effects);
        control
            .state()
            .motion
            .tick(Instant::now() + Duration::from_millis(50));
        let midpoint = *control.state().motion.value();
        assert!(midpoint.scale > 0.0 && midpoint.scale < 1.0);
        let old_serial = control.state().serial;
        control.request("enter");
        control.sync(&effects);
        assert_eq!(*control.state().motion.value(), midpoint);
        assert_eq!(control.state().motion.target().scale, 1.0);
        assert!(!control.accept(old_serial));
    }

    #[test]
    fn completion_is_consumed_once_and_cancellation_suppresses_it() {
        let control = Control::default();
        control.state().pending_callback = true;
        assert!(control.accept(1));
        assert!(!control.accept(1));
        control.state().pending_callback = true;
        control.cancel();
        assert!(!control.accept(1));
    }

    #[test]
    fn callback_only_events_do_not_wait_for_an_interrupted_transform() {
        let lua = mlua::Lua::new();
        let effects = parse(&lua.load("{transition={enter={from={scale=0},duration=100},press={callback=function() end}}}").eval().unwrap()).unwrap();
        let control = Control::default();
        control.sync(&effects);
        control
            .state()
            .motion
            .tick(Instant::now() + Duration::from_millis(30));
        let current = *control.state().motion.value();
        control.request("press");
        control.sync(&effects);
        assert_eq!(*control.state().motion.value(), current);
        assert!(!control.state().motion.is_animating());
    }

    #[test]
    fn exit_is_idempotent_and_retained_until_callback_delivery() {
        let control = Control::default();
        control.request("exit");
        let serial = control.state().serial;
        control.request("exit");
        assert_eq!(control.state().serial, serial);
        control.state().hidden = true;
        control.state().pending_callback = true;
        assert!(!control.removable());
        assert!(control.accept(serial));
        assert!(control.removable());
    }

    #[test]
    fn event_parser_rejects_legacy_and_invalid_values() {
        let lua = mlua::Lua::new();
        for config in [
            "true",
            "false",
            "{duration=100}",
            "{onAdded=function() end}",
            "{enter={from={scale=-1}}}",
            "{exit={to={x=0/0}}}",
            "{enter={duration=0}}",
            "{enter={easing='linear',spring='smooth'}}",
            "{exit={callback=true}}",
            "{displaced={from={x=1}}}",
        ] {
            let node = lua.load(format!("{{transition={config}}}")).eval().unwrap();
            assert!(parse(&node).is_err(), "{config}");
        }
    }
}
