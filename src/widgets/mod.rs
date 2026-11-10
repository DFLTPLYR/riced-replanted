mod advanced;
pub(crate) mod animation;
mod canvas;
mod containers;
mod enums;
mod input;
mod keyed;
mod layout;
pub(crate) mod measure;
mod media;
mod region;
mod scale;
mod shader;
mod state;
mod style;
mod text;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use iced::{Border, Color, Element, Length, Padding, Shadow, widget};
use iced_anim::Animate;
use iced_anim::Animated;
use iced_anim::animated::Mode;
use mlua::{Table, Value};

use crate::runtime::Message;

pub(crate) use region::root as with_input_regions;
pub(crate) use state::{forget_generation, forget_window, free_retired, sweep_animations};

pub fn color(value: &str) -> mlua::Result<Color> {
    if !value.is_ascii() {
        return Err(mlua::Error::runtime("colors must be ASCII hex strings"));
    }
    value.parse().map_err(|_| {
        mlua::Error::runtime(format!("invalid color {value:?}; use #RRGGBB or #RRGGBBAA"))
    })
}

pub(crate) fn edit(
    generation: u64,
    id: &str,
    action: iced::widget::text_editor::Action,
) -> Option<String> {
    state::perform(generation, id, action)
}

pub(crate) fn editor_text(generation: u64, id: &str) -> Option<String> {
    state::text(generation, id)
}

fn length(value: Value) -> mlua::Result<Length> {
    match value {
        Value::Nil => Ok(Length::Shrink),
        Value::String(s) if s.to_str()? == "fill" => Ok(Length::Fill),
        Value::String(s) if s.to_str()? == "shrink" => Ok(Length::Shrink),
        Value::Table(table) => {
            crate::runtime::keys(&table, &["fill_portion"])?;
            match table.get::<Value>("fill_portion")? {
                Value::Integer(n) if (1..=u16::MAX as i64).contains(&n) => {
                    Ok(Length::FillPortion(n as u16))
                }
                _ => Err(mlua::Error::runtime(
                    "fill portion must be an integer from 1 to 65535",
                )),
            }
        }
        Value::Integer(n) if n >= 0 => Ok(Length::Fixed(n as f32)),
        Value::Number(n) if n.is_finite() && n >= 0.0 && n <= f32::MAX as f64 => {
            Ok(Length::Fixed(n as f32))
        }
        _ => Err(mlua::Error::runtime(
            "length must be 'fill', 'shrink', non-negative pixels, or {fill_portion = 1..65535}",
        )),
    }
}

fn pixels(value: Value) -> Option<f32> {
    match value {
        Value::Integer(n) if n >= 0 => Some(n as f32),
        Value::Number(n) if n.is_finite() && n >= 0.0 && n <= f32::MAX as f64 => Some(n as f32),
        _ => None,
    }
}

fn number(table: &Table, key: &str, default: f32) -> mlua::Result<f32> {
    let value = table.get::<Option<f32>>(key)?.unwrap_or(default);
    if !value.is_finite() || value < 0.0 {
        return Err(mlua::Error::runtime(format!(
            "{key} must be finite and non-negative"
        )));
    }
    Ok(value)
}

fn padding_value(table: &Table, key: &str) -> mlua::Result<Padding> {
    match table.get::<Value>(key)? {
        Value::Nil => Ok(Padding::default()),
        Value::Integer(_) | Value::Number(_) => Ok(Padding::new(number(table, key, 0.0)?)),
        Value::Table(sides) => {
            crate::runtime::keys(&sides, &["top", "right", "bottom", "left"])?;
            Ok(Padding {
                top: number(&sides, "top", 0.0)?,
                right: number(&sides, "right", 0.0)?,
                bottom: number(&sides, "bottom", 0.0)?,
                left: number(&sides, "left", 0.0)?,
            })
        }
        _ => Err(mlua::Error::runtime(
            "padding must be a non-negative number or {top, right, bottom, left}",
        )),
    }
}

fn max_pixels(table: &Table, key: &str) -> mlua::Result<Option<f32>> {
    let value = table.get::<Option<f32>>(key)?;
    if let Some(value) = value
        && (!value.is_finite() || value < 0.0)
    {
        return Err(mlua::Error::runtime(format!(
            "{key} must be finite and non-negative"
        )));
    }
    Ok(value)
}

fn children(node: &Table, ctx: &Ctx) -> mlua::Result<Vec<Element<'static, Message>>> {
    node.get::<Table>("content")?
        .sequence_values::<Table>()
        .map(|child| element(child?, ctx))
        .collect()
}

#[derive(Debug, Clone, PartialEq, Animate)]
struct Motion {
    width: f32,
    height: f32,
    padding: f32,
    spacing: f32,
    size: f32,
    color: Color,
    value: f32,
    background: Color,
    text_color: Color,
    border: Border,
    shadow: Shadow,
    extras: [f32; 16],
    opacity: f32,
    girth: f32,
    thickness: f32,
    cell_size: f32,
    total_size: f32,
}

impl Motion {
    fn target(table: &Table) -> mlua::Result<Self> {
        let kind = table.get::<String>("kind")?;
        let color = match kind.as_str() {
            "text" | "icon" => match table.get::<Option<String>>("color")? {
                Some(value) => color(&value)?,
                None => Color::WHITE,
            },
            _ => Color::WHITE,
        };
        let value = match kind.as_str() {
            "progress_bar" => table.get::<Option<f32>>("value")?.unwrap_or(0.0),
            _ => 0.0,
        };
        let opacity = match kind.as_str() {
            "image" | "svg" => {
                let opacity = number(table, "opacity", 1.0)?;
                if opacity > 1.0 {
                    return Err(mlua::Error::runtime("opacity must be from 0 to 1"));
                }
                opacity
            }
            _ => 1.0,
        };
        let girth = match kind.as_str() {
            "progress_bar" => {
                let girth = number(
                    table,
                    "girth",
                    widget::ProgressBar::<iced::Theme>::DEFAULT_GIRTH,
                )?;
                if girth <= 0.0 {
                    return Err(mlua::Error::runtime(
                        "progress_bar girth must be finite and positive",
                    ));
                }
                girth
            }
            _ => 0.0,
        };
        let thickness = match kind.as_str() {
            "rule" => number(table, "thickness", 1.0)?,
            _ => 0.0,
        };
        let cell_size = match kind.as_str() {
            "qr_code" => number(table, "cell_size", 0.0)?,
            _ => 0.0,
        };
        let total_size = match kind.as_str() {
            "qr_code" => number(table, "total_size", 0.0)?,
            _ => 0.0,
        };
        let (background, text_color, border, shadow) = match table.get::<Option<Table>>("style")? {
            Some(style) => {
                let patch = style::style_patch(&style, None, None)?;
                (
                    patch.background.unwrap_or(Color::TRANSPARENT),
                    patch.text_color.unwrap_or(Color::TRANSPARENT),
                    patch.border.unwrap_or_default(),
                    patch.shadow.unwrap_or_default(),
                )
            }
            None => (
                Color::TRANSPARENT,
                Color::TRANSPARENT,
                Border::default(),
                Shadow::default(),
            ),
        };
        Ok(Self {
            width: pixels(table.get::<Value>("width")?).unwrap_or(0.0),
            height: pixels(table.get::<Value>("height")?).unwrap_or(0.0),
            padding: match table.get::<Value>("padding")? {
                Value::Table(_) => 0.0,
                _ => number(table, "padding", 0.0)?,
            },
            spacing: number(table, "spacing", 0.0)?,
            size: number(table, "size", 16.0)?,
            color,
            value,
            background,
            text_color,
            border,
            shadow,
            extras: [0.0; 16],
            opacity,
            girth,
            thickness,
            cell_size,
            total_size,
        })
    }
}

fn animatable(kind: &str) -> Vec<&'static str> {
    let mut props = vec!["width", "height", "padding"];
    match kind {
        "text" | "icon" => props.extend(["size", "color"]),
        "row" | "column" | "grid" | "table" | "scrollable" | "markdown" | "pane_grid" => {
            props.push("spacing")
        }
        "progress_bar" => props.extend(["value", "girth"]),
        "image" | "svg" => props.push("opacity"),
        "rule" => props.push("thickness"),
        "qr_code" => props.extend(["cell_size", "total_size"]),
        _ => {}
    }
    match kind {
        "container" | "button" => props.extend(["background", "text_color", "border", "shadow"]),
        "checkbox" | "radio" | "toggler" | "text_input" | "pick_list" | "text_editor"
        | "combo_box" => props.extend(["background", "text_color", "border"]),
        "scrollable" => props.extend(["background", "border", "shadow"]),
        "slider" | "vertical_slider" => props.push("border"),
        _ => {}
    }
    props
}

fn style_states(kind: &str) -> &'static [&'static str] {
    match kind {
        "button" => &["active", "hovered", "pressed", "disabled"],
        "checkbox" | "toggler" => &["active", "hovered", "disabled"],
        "radio" => &["active", "hovered"],
        "slider" | "vertical_slider" | "scrollable" => &["active", "hovered", "dragged"],
        "text_input" | "text_editor" | "combo_box" => &["active", "hovered", "focused", "disabled"],
        "pick_list" => &["active", "hovered", "opened"],
        _ => &[],
    }
}

const STATE_FIELDS: &[&str] = &["background", "text_color", "border", "shadow"];

#[derive(Clone)]
struct StateAnimation {
    base: Motion,
    animated: Option<Animated<Motion>>,
}

impl StateAnimation {
    fn value(&mut self, target: Motion, mode: Mode) -> Motion {
        match &mut self.animated {
            Some(animated) if animated.target() != &target => {
                *animated = Animated::new(animated.value().clone(), mode).to(target);
            }
            None => self.animated = Some(Animated::new(target, mode)),
            _ => {}
        }
        self.animated
            .as_ref()
            .expect("state animation is initialized")
            .value()
            .clone()
    }

    fn tick(&mut self, now: Instant) {
        if let Some(animated) = &mut self.animated {
            animated.tick(now);
        }
    }

    fn is_animating(&self) -> bool {
        self.animated.as_ref().is_some_and(Animated::is_animating)
    }
}

/// An element's `transition` prop: which pixel props interpolate, and how.
#[derive(Clone)]
struct Transition {
    motion: Motion,
    props: Vec<String>,
    mode: Mode,
    callback: Option<mlua::Function>,
    extra_names: Vec<String>,
    state_fields: Vec<String>,
    state_patches: BTreeMap<String, style::StylePatch>,
    state_animation: Option<Arc<Mutex<StateAnimation>>>,
}

impl Transition {
    fn parse(node: &Table) -> mlua::Result<Option<Self>> {
        let Some(events) = node.get::<Option<Table>>("transition")? else {
            return Ok(None);
        };
        let Some(config) = events.get::<Option<Table>>("change")? else {
            return Ok(None);
        };
        let kind = node.get::<String>("kind")?;
        let allowed = animatable(&kind);
        let extras = if kind == "shader" {
            shader::extra_args(node)?
        } else {
            Vec::new()
        };
        let states = style_states(&kind);
        let mut props = allowed
            .iter()
            .map(|name| (*name).to_owned())
            .collect::<Vec<_>>();
        if kind == "shader" {
            props.extend(extras.iter().map(|(name, _)| format!("extraArgs.{name}")));
        }
        let mut state_fields = Vec::new();
        crate::runtime::keys(
            &config,
            &["props", "easing", "spring", "duration", "callback"],
        )?;
        if let Some(list) = config.get::<Option<Table>>("props")? {
            let mut base = Vec::new();
            for name in list.sequence_values::<String>() {
                let name = name?;
                if let Some((state, field)) = name.split_once('.')
                    && states.contains(&state)
                {
                    if !allowed.contains(&field) || !STATE_FIELDS.contains(&field) {
                        return Err(mlua::Error::runtime(format!(
                            "unknown state transition prop: {name}; use background, text_color, border, or shadow"
                        )));
                    }
                    if !state_fields.iter().any(|known: &String| known == field) {
                        state_fields.push(field.to_owned());
                    }
                    continue;
                }
                if let Some(extra) = name.strip_prefix("extraArgs.") {
                    if !extras.iter().any(|(known, _)| known == extra) {
                        return Err(mlua::Error::runtime(format!(
                            "unknown transition prop: {name}"
                        )));
                    }
                } else if !allowed.contains(&name.as_str()) {
                    return Err(mlua::Error::runtime(format!(
                        "unknown transition prop: {name}"
                    )));
                }
                base.push(name);
            }
            props = base;
        }
        if matches!(node.get::<Value>("padding")?, Value::Table(_)) {
            // Per-side padding is static; scalar transitions keep animating.
            props.retain(|prop| prop != "padding");
        }
        let mode = animation::mode(&config)?;
        let mut motion = Motion::target(node)?;
        let state_base = motion.clone();
        // shortcut: extras animate by sorted position, so keep extraArgs keys stable while a tween runs.
        for (index, (name, value)) in extras.iter().enumerate() {
            motion.extras[index] = *value;
            let selected = props
                .iter()
                .any(|prop| prop.strip_prefix("extraArgs.") == Some(name.as_str()));
            if !selected {
                motion.extras[index] = 0.0;
            }
        }
        for (name, value) in [
            ("width", &mut motion.width),
            ("height", &mut motion.height),
            ("padding", &mut motion.padding),
            ("spacing", &mut motion.spacing),
            ("size", &mut motion.size),
            ("value", &mut motion.value),
            ("opacity", &mut motion.opacity),
            ("girth", &mut motion.girth),
            ("thickness", &mut motion.thickness),
            ("cell_size", &mut motion.cell_size),
            ("total_size", &mut motion.total_size),
        ] {
            if !props.iter().any(|prop| prop == name) {
                *value = 0.0;
            }
        }
        if !props.iter().any(|prop| prop == "color") {
            motion.color = Color::WHITE;
        }
        if !props.iter().any(|prop| prop == "background") {
            motion.background = Color::TRANSPARENT;
        }
        if !props.iter().any(|prop| prop == "text_color") {
            motion.text_color = Color::TRANSPARENT;
        }
        if !props.iter().any(|prop| prop == "border") {
            motion.border = Border::default();
        }
        if !props.iter().any(|prop| prop == "shadow") {
            motion.shadow = Shadow::default();
        }
        let mut state_patches = BTreeMap::new();
        let state_animation = if state_fields.is_empty() {
            None
        } else {
            if let Some(style) = node.get::<Option<Table>>("style")? {
                for state in states {
                    if let Some(table) = style.get::<Option<Table>>(*state)? {
                        state_patches.insert((*state).to_owned(), style::style_overlay(&table)?);
                    }
                }
            }
            Some(Arc::new(Mutex::new(StateAnimation {
                base: state_base,
                animated: None,
            })))
        };
        Ok(Some(Self {
            motion,
            props,
            mode,
            callback: config.get("callback")?,
            extra_names: extras.into_iter().map(|(name, _)| name).collect(),
            state_fields,
            state_patches,
            state_animation,
        }))
    }

    fn extra(&self, name: &str) -> Option<f32> {
        if !self
            .props
            .iter()
            .any(|prop| prop.strip_prefix("extraArgs.") == Some(name))
        {
            return None;
        }
        self.extra_names
            .iter()
            .position(|known| known == name)
            .map(|index| self.motion.extras[index])
    }

    fn value(&self, key: &str) -> Option<f32> {
        if !self.props.iter().any(|prop| prop == key) {
            return None;
        }
        match key {
            "width" => Some(self.motion.width),
            "height" => Some(self.motion.height),
            "padding" => Some(self.motion.padding),
            "spacing" => Some(self.motion.spacing),
            "size" => Some(self.motion.size),
            "value" => Some(self.motion.value),
            "opacity" => Some(self.motion.opacity),
            "girth" => Some(self.motion.girth),
            "thickness" => Some(self.motion.thickness),
            "cell_size" => Some(self.motion.cell_size),
            "total_size" => Some(self.motion.total_size),
            _ => None,
        }
    }

    fn color(&self) -> Option<Color> {
        self.props
            .iter()
            .any(|prop| prop == "color")
            .then_some(self.motion.color)
    }

    fn background(&self) -> Option<Color> {
        self.props
            .iter()
            .any(|prop| prop == "background")
            .then_some(self.motion.background)
    }

    fn text_color(&self) -> Option<Color> {
        self.props
            .iter()
            .any(|prop| prop == "text_color")
            .then_some(self.motion.text_color)
    }

    fn border(&self) -> Option<Border> {
        self.props
            .iter()
            .any(|prop| prop == "border")
            .then_some(self.motion.border)
    }

    fn shadow(&self) -> Option<Shadow> {
        self.props
            .iter()
            .any(|prop| prop == "shadow")
            .then_some(self.motion.shadow)
    }

    /// Overlays the animated values for a widget state onto a static style patch.
    fn animate(&self, patch: &mut style::StylePatch, status: &str) {
        let Some(animation) = &self.state_animation else {
            return;
        };
        let motion = {
            let mut animation = animation.lock().unwrap();
            let mut target = animation.base.clone();
            if let Some(state) = self.state_patches.get(status) {
                if let Some(value) = state.background {
                    target.background = value;
                }
                if let Some(value) = state.text_color {
                    target.text_color = value;
                }
                if let Some(value) = state.border {
                    target.border = value;
                }
                if let Some(value) = state.shadow {
                    target.shadow = value;
                }
            }
            animation.value(target, self.mode)
        };
        for field in &self.state_fields {
            match field.as_str() {
                "background" => patch.background = Some(motion.background),
                "text_color" => patch.text_color = Some(motion.text_color),
                "border" => patch.border = Some(motion.border),
                "shadow" => patch.shadow = Some(motion.shadow),
                _ => {}
            }
        }
    }

    fn copy_state_from(&self, previous: &Arc<Mutex<StateAnimation>>) {
        if let Some(new) = &self.state_animation {
            let animated = previous.lock().unwrap().animated.clone();
            new.lock().unwrap().animated = animated;
        }
    }
}

#[derive(Clone)]
pub struct Ctx {
    pub generation: u64,
    pub component: Option<(String, u64, Option<iced::window::Id>)>,
    lua: Option<mlua::Lua>,
    window: Option<iced::window::Id>,
    profile: Option<PathBuf>,
    owner: Value,
    rebuilding: bool,
    lifetime: Option<animation::Control>,
}

impl Ctx {
    pub fn new(generation: u64) -> Self {
        Self {
            generation,
            component: None,
            lua: None,
            window: None,
            profile: None,
            owner: Value::Nil,
            rebuilding: false,
            lifetime: None,
        }
    }

    pub fn with_lua(mut self, lua: mlua::Lua) -> Self {
        self.lua = Some(lua);
        self
    }

    pub fn with_window(mut self, window: iced::window::Id) -> Self {
        self.window = Some(window);
        self
    }

    pub fn with_profile(mut self, profile: Option<PathBuf>) -> Self {
        self.profile = profile;
        self
    }

    fn profile(&self) -> Option<&Path> {
        self.profile.as_deref()
    }

    fn window(&self) -> Option<iced::window::Id> {
        self.window
    }

    fn lua(&self) -> mlua::Result<&mlua::Lua> {
        self.lua
            .as_ref()
            .ok_or_else(|| mlua::Error::runtime("this widget needs a Lua context"))
    }
}

fn resolve_defaults(node: Table, ctx: &Ctx) -> mlua::Result<Table> {
    let Some(lua) = &ctx.lua else {
        return Ok(node);
    };
    let riced: Table = lua.globals().get("riced")?;
    let Some(resolve) = riced.get::<Option<mlua::Function>>("_defaults")? else {
        return Ok(node);
    };
    resolve.call(node)
}

pub fn element(node: Table, ctx: &Ctx) -> mlua::Result<Element<'static, Message>> {
    let node = resolve_defaults(node, ctx)?;
    let tagged;
    let ctx = match node.get::<Option<String>>("__component_id")? {
        Some(id) => {
            tagged = Ctx {
                generation: ctx.generation,
                component: Some((id, node.get::<u64>("__component_revision")?, ctx.window)),
                lua: ctx.lua.clone(),
                window: ctx.window,
                profile: ctx.profile.clone(),
                owner: ctx.owner.clone(),
                rebuilding: ctx.rebuilding,
                lifetime: ctx.lifetime.clone(),
            };
            &tagged
        }
        None => ctx,
    };
    let owned;
    let ctx = if let Value::Table(owner) = node.get::<Value>("__animation_self")? {
        owned = Ctx {
            owner: Value::Table(owner),
            lifetime: node
                .get::<Option<mlua::AnyUserData>>("__animation")?
                .map(|value| {
                    value
                        .borrow::<animation::Control>()
                        .map(|control| control.clone())
                })
                .transpose()?,
            ..ctx.clone()
        };
        &owned
    } else {
        ctx
    };
    let effects = animation::parse(&node)?;
    let kind = node.get::<Option<String>>("kind")?;
    if kind.as_deref() == Some("list_view") {
        return build(&node, ctx, None);
    }
    let measurable = (kind.as_deref() != Some("measure"))
        .then(|| node.get::<Option<String>>("id"))
        .transpose()?
        .flatten()
        .filter(|id| !id.is_empty());
    let transition = Transition::parse(&node)?;
    let content = build(&node, ctx, transition.as_ref())?;
    let control = node
        .get::<Option<mlua::AnyUserData>>("__animation")?
        .map(|value| {
            value
                .borrow::<animation::Control>()
                .map(|control| control.clone())
        })
        .transpose()?;
    let element = if effects.is_empty() && transition.is_none() && control.is_none() {
        content
    } else {
        scale::ScaleItem::new(
            content,
            effects,
            control,
            ctx.clone(),
            ctx.owner.clone(),
            node.get::<Option<bool>>("__retained")?.unwrap_or(false),
        )
        .property(node, transition)
        .into()
    };
    // Every id'd widget is measurable, so popups can anchor to any of them.
    if let Some(id) = measurable {
        return Ok(measure::Measured::new(element, id, ctx.window()).into());
    }
    Ok(element)
}

#[derive(Clone, Copy)]
pub(super) struct Layout {
    pub(super) width: Option<Length>,
    pub(super) height: Option<Length>,
    pub(super) padding: Padding,
}

fn build(
    node: &Table,
    ctx: &Ctx,
    transition: Option<&Transition>,
) -> mlua::Result<Element<'static, Message>> {
    const COMMON: &[&str] = &[
        "kind",
        "id",
        "content",
        "width",
        "height",
        "padding",
        "transition",
        "__component_id",
        "__component_revision",
        "__animation",
        "__animation_self",
        "__retained",
    ];
    let kind: String = node.get("kind")?;
    let extra: &[&str] = match kind.as_str() {
        "text" => &[
            "size",
            "color",
            "line_height",
            "wrap",
            "align_x",
            "align_y",
            "shaping",
            "center",
            "font",
        ],
        "icon" => &["size", "color", "center", "align_x", "align_y"],
        "image" => &["content_fit", "opacity"],
        "svg" => &["content_fit", "opacity"],
        "rule" => &["direction", "thickness", "style"],
        "progress_bar" => &["value", "girth", "vertical", "style"],
        "qr_code" => &["cell_size", "total_size", "style"],
        "row" => &["spacing", "align_y", "clip"],
        "column" => &["spacing", "align_x", "clip"],
        "container" => &[
            "align_x",
            "align_y",
            "max_width",
            "max_height",
            "clip",
            "style",
        ],
        "button" => &["on_press", "clip", "style"],
        "space" => &[],
        "stack" => &["clip"],
        "float" => &["scale"],
        "pin" => &["x", "y"],
        "tooltip" => &["tooltip", "position", "gap"],
        "grid" => &["columns", "spacing", "fluid"],
        "checkbox" => &["value", "label", "text_size", "on_toggle", "style"],
        "radio" => &[
            "value",
            "selected",
            "label",
            "text_size",
            "on_select",
            "style",
        ],
        "toggler" => &["value", "label", "text_size", "on_toggle", "style"],
        "slider" | "vertical_slider" => &[
            "min",
            "max",
            "value",
            "step",
            "on_change",
            "on_release",
            "style",
        ],
        "mouse_area" => &[
            "on_press",
            "on_release",
            "on_double_click",
            "on_right_press",
            "on_right_release",
            "on_middle_press",
            "on_middle_release",
            "on_enter",
            "on_exit",
            "on_move",
            "on_scroll",
        ],
        "sensor" => &["on_show", "on_hide", "on_resize"],
        "scrollable" => &[
            "direction",
            "scrollbar_hidden",
            "id",
            "on_scroll",
            "auto_scroll",
            "spacing",
            "style",
        ],
        "text_input" => &[
            "placeholder",
            "value",
            "secure",
            "id",
            "size",
            "align_x",
            "font",
            "on_input",
            "on_submit",
            "on_paste",
            "style",
        ],
        "pick_list" => &[
            "options",
            "selected",
            "placeholder",
            "on_select",
            "text_size",
            "style",
        ],
        "text_editor" => &[
            "placeholder",
            "value",
            "id",
            "size",
            "font",
            "wrapping",
            "on_action",
            "on_input",
            "style",
        ],
        "combo_box" => &[
            "options",
            "value",
            "placeholder",
            "id",
            "on_select",
            "on_input",
            "style",
        ],
        "table" => &["columns", "spacing"],
        "list_view" => &["id", "key", "direction", "spacing", "render"],
        "region" => &[],
        "markdown" => &["text_size", "spacing", "on_link_click", "style"],
        "responsive" => &[],
        "canvas" => &[],
        "shader" => &["source", "path", "extraArgs", "textures", "animated", "fps"],
        "pane_grid" => &[
            "id",
            "direction",
            "spacing",
            "min_size",
            "on_click",
            "on_resize",
            "on_drag",
            "style",
        ],
        "measure" => &[],
        _ => {
            return Err(mlua::Error::runtime(format!(
                "unsupported iced element: {kind}"
            )));
        }
    };
    let mut allowed = COMMON.to_vec();
    allowed.extend_from_slice(extra);
    crate::runtime::keys(node, &allowed)?;
    let layout = Layout {
        width: axis(node, "width", transition.and_then(|t| t.value("width")))?,
        height: axis(node, "height", transition.and_then(|t| t.value("height")))?,
        padding: match transition.and_then(|t| t.value("padding")) {
            Some(padding) => Padding::new(padding),
            None => padding_value(node, "padding")?,
        },
    };
    let element = match kind.as_str() {
        "text" => text::build(node, transition, layout)?,
        "icon" => text::icon(node, transition, layout)?,
        "image" => media::image(node, layout, transition)?,
        "svg" => media::svg(node, layout, transition)?,
        "rule" => media::rule(node, transition)?,
        "progress_bar" => media::progress_bar(node, transition)?,
        "qr_code" => media::qr_code(node, transition)?,
        "row" | "column" => layout::row_column(node, ctx, transition, layout, &kind)?,
        "grid" => layout::grid(node, ctx, transition)?,
        "space" => layout::space(layout),
        "container" => containers::container(node, ctx, layout, transition)?,
        "button" => containers::button(node, ctx, layout, transition)?,
        "stack" => containers::stack(node, ctx, layout)?,
        "float" => containers::float(node, ctx)?,
        "pin" => containers::pin(node, ctx, layout)?,
        "tooltip" => containers::tooltip(node, ctx)?,
        "checkbox" => input::checkbox(node, ctx, layout, transition)?,
        "radio" => input::radio(node, ctx, layout, transition)?,
        "toggler" => input::toggler(node, ctx, layout, transition)?,
        "slider" | "vertical_slider" => input::slider(node, ctx, &kind, layout, transition)?,
        "mouse_area" => input::mouse_area(node, ctx)?,
        "sensor" => input::sensor(node, ctx)?,
        "scrollable" => input::scrollable(node, ctx, layout, transition)?,
        "text_input" => input::text_input(node, ctx, layout, transition)?,
        "pick_list" => input::pick_list(node, ctx, layout, transition)?,
        "text_editor" => advanced::text_editor(node, ctx, layout, transition)?,
        "combo_box" => advanced::combo_box(node, ctx, layout, transition)?,
        "table" => advanced::table(node, ctx, layout, transition)?,
        "list_view" => advanced::list_view(node, ctx, layout)?,
        "region" => {
            let content = element(node.get::<Table>("content")?, ctx)?;
            region::Region::new(content, ctx.window()).into()
        }
        "measure" => {
            let id = node
                .get::<Option<String>>("id")?
                .filter(|id| !id.is_empty())
                .ok_or_else(|| mlua::Error::runtime("measure requires a nonempty id"))?;
            let content = element(node.get::<Table>("content")?, ctx)?;
            measure::Measured::new(content, id, ctx.window()).into()
        }
        "markdown" => advanced::markdown(node, ctx, transition)?,
        "responsive" => advanced::responsive(node, ctx, layout)?,
        "canvas" => canvas::canvas(node, layout)?,
        "shader" => shader::shader(node, ctx, layout, transition)?,
        "pane_grid" => advanced::pane_grid(node, ctx, layout, transition)?,
        _ => {
            return Err(mlua::Error::runtime(format!(
                "unsupported iced element: {kind}"
            )));
        }
    };
    Ok(finish(element, &kind, layout))
}

/// Widgets sized by their own builder; the rest get a container wrapper only
/// when a non-default size or padding actually needs one.
fn finish(
    element: Element<'static, Message>,
    kind: &str,
    layout: Layout,
) -> Element<'static, Message> {
    let (sized_width, sized_height) = direct_size(kind);
    let needs_width = !sized_width && layout.width.is_some_and(|width| width != Length::Shrink);
    let needs_height =
        !sized_height && layout.height.is_some_and(|height| height != Length::Shrink);
    if kind == "container"
        || kind == "button"
        || (layout.padding == Padding::default() && !needs_width && !needs_height)
    {
        return element;
    }
    let mut wrapper = widget::container(element);
    if needs_width && let Some(width) = layout.width {
        wrapper = wrapper.width(width);
    }
    if needs_height && let Some(height) = layout.height {
        wrapper = wrapper.height(height);
    }
    if layout.padding != Padding::default() {
        wrapper = wrapper.padding(layout.padding);
    }
    wrapper.into()
}

/// Which axes a widget's own builder accepts as `Length` (see `iced_widget`).
fn direct_size(kind: &str) -> (bool, bool) {
    match kind {
        "text" | "icon" | "image" | "svg" | "row" | "column" | "container" | "button" | "space"
        | "stack" | "pin" | "scrollable" => (true, true),
        "checkbox" | "radio" | "toggler" | "slider" | "text_input" | "pick_list" => (true, false),
        "table" => (true, false),
        "text_editor" | "combo_box" | "markdown" => (false, false),
        "responsive" | "pane_grid" | "canvas" => (true, true),
        "shader" | "list_view" => (true, true),
        "region" => (false, false),
        "vertical_slider" => (false, true),
        _ => (false, false),
    }
}

/// An axis is absent unless the profile (or a transition) supplies it; an animated
/// axis replaces only pixel values, leaving `"fill"`, `"shrink"`, and portions static.
fn axis(node: &Table, key: &str, animated: Option<f32>) -> mlua::Result<Option<Length>> {
    let value: Value = node.get(key)?;
    if value.is_nil() {
        return Ok(None);
    }
    let length = length(value)?;
    Ok(Some(match (length, animated) {
        (Length::Fixed(_), Some(pixels)) => Length::Fixed(pixels.max(0.0)),
        _ => length,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_fill_portions() {
        let lua = mlua::Lua::new();
        for portion in [1, 3, 65535] {
            let value = lua
                .load(format!("{{fill_portion={portion}}}"))
                .eval()
                .unwrap();
            assert_eq!(length(value).unwrap(), Length::FillPortion(portion));
        }
    }

    #[test]
    fn rejects_invalid_fill_portions() {
        let lua = mlua::Lua::new();
        for source in [
            "{}",
            "{fill_portion=0}",
            "{fill_portion=-1}",
            "{fill_portion=65536}",
            "{fill_portion=1.5}",
            "{fill_portion='3'}",
            "{fill_portion=3, typo=true}",
        ] {
            assert!(
                length(lua.load(source).eval().unwrap()).is_err(),
                "{source}"
            );
        }
    }

    #[test]
    fn rejects_invalid_widget_values() {
        let lua = mlua::Lua::new();
        for source in [
            "{kind='unknown'}",
            "{kind='text',content='x',width=-1}",
            "{kind='text',content='x',color='invalid'}",
            "{kind='column',content={false}}",
            "{kind='measure',content={kind='text',content='x'}}",
        ] {
            assert!(
                element(lua.load(source).eval().unwrap(), &Ctx::new(0)).is_err(),
                "{source}"
            );
        }
    }

    #[test]
    fn shader_accepts_inline_source_and_profile_paths() {
        let shader = r#"
            @fragment
            fn fs_main(@location(0) uv: vec2<f32>) -> @location(0) vec4<f32> {
                return vec4<f32>(uv, 0.0, 1.0);
            }
        "#;
        let lua = mlua::Lua::new();
        let extra_shader = r#"
            @fragment
            fn fs_main(@location(0) uv: vec2<f32>) -> @location(0) vec4<f32> {
                return vec4<f32>(riced.extra_x, riced.extra_y, uv.x, 1.0);
            }
        "#;
        let example_shader = include_str!("../../example/components/shaders/plasma.wgsl");
        let mouse_shader = r#"
            @fragment
            fn fs_main(@location(0) uv: vec2<f32>) -> @location(0) vec4<f32> {
                let center = select(
                    vec2<f32>(0.5),
                    riced.mouse / max(riced.resolution, vec2<f32>(1.0)),
                    riced.mouse.x >= 0.0,
                );
                return vec4<f32>(uv, center.x, riced.hover);
            }
        "#;
        let dir = std::env::temp_dir().join(format!("riced-shader-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("test.wgsl"), shader).unwrap();
        image::RgbaImage::from_pixel(2, 2, image::Rgba([255, 0, 0, 255]))
            .save(dir.join("texture.png"))
            .unwrap();
        let ctx = Ctx::new(0).with_profile(Some(dir.clone()));
        for source in [
            format!("{{kind='shader',source=[[{shader}]],width=20,height=10}}"),

            format!("{{kind='shader',source=[[{example_shader}]]}}"),

            format!(
                "{{kind='shader',source=[[{extra_shader}]],extraArgs={{extra_x=1.5,extra_y=2.5}}}}"
            ),
            format!("{{kind='shader',source=[[{mouse_shader}]],animated=true,fps=30}}"),
            format!(
                "{{kind='shader',source=[[{shader}]],textures={{'texture.png'}}}}"
            ),
            format!(
                "{{kind='shader',source=[[{shader}]],textures={{'{}/texture.png'}}}}",
                dir.display()
            ),
            "{kind='shader',path='test.wgsl',width=20,height=10}".to_owned(),
            "{kind='shader',path='missing.wgsl'}".to_owned(),
            "{kind='shader',source='not wgsl'}".to_owned(),
            "{kind='shader',source='@fragment fn fs_main() -> @location(0) vec4<f32> { return vec4<f32>(1.0); }'}".to_owned(),
        ] {
            let result = element(lua.load(&source).eval().unwrap(), &ctx);
            assert!(result.is_ok(), "{source}: {:?}", result.err());
        }
        for source in [
            "{kind='shader'}",
            "{kind='shader',source='x',path='test.wgsl'}",
            "{kind='shader',source='   '}",
            "{kind='shader',path='   '}",
            "{kind='shader',source='x',extraArgs={1,2}}",
            "{kind='shader',source='x',extraArgs={mouse_x='x'}}",
            "{kind='shader',source='x',extraArgs={['bad-key']=1}}",
            "{kind='shader',source='x',extraArgs={resolution=1}}",
            "{kind='shader',source='x',extraArgs={x=0/0}}",
            "{kind='shader',source='x',extraArgs={mouse=1}}",
            "{kind='shader',source='x',extraArgs={hover=1}}",
            "{kind='shader',source='x',textures={'/missing/texture.png'}}",
            "{kind='shader',source='x',textures={'a','b','c'}}",
            "{kind='shader',source='x',textures={'   '}}",
        ] {
            assert!(
                element(lua.load(source).eval().unwrap(), &Ctx::new(0)).is_err(),
                "{source}"
            );
        }
        let minimal = "@fragment fn fs_main(@location(0) uv: vec2<f32>) -> @location(0) vec4<f32> { return vec4<f32>(uv, 0.0, 1.0); }";
        for fps in [0, 241] {
            let source = format!("{{kind='shader',source=[[{minimal}]],animated=true,fps={fps}}}");
            assert!(
                element(lua.load(&source).eval().unwrap(), &Ctx::new(0)).is_err(),
                "{source}"
            );
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn transitions_animate_listed_props_and_keep_fill_static() {
        let lua = mlua::Lua::new();
        for source in [
            "{kind='container',content={kind='text',content='x'},transition={change={}},padding=8,width=120}",
            "{kind='container',content={kind='text',content='x'},transition={change={props={'width'}}},width=120}",
            "{kind='container',content={kind='text',content='x'},transition={change={duration=300,easing='out'}},width=120}",
            "{kind='container',content={kind='text',content='x'},transition={change={spring='bouncy'}},width=120}",
            "{kind='container',content={kind='text',content='x'},transition={change={}},width='fill'}",
            "{kind='row',content={},transition={change={}},spacing=4}",
            "{kind='text',content='x',color='#ff0000',transition={change={}}}",
            "{kind='text',content='x',color='#ff0000',transition={change={props={'color'}}}}",
            "{kind='progress_bar',value=0.5,transition={change={}}}",
            "{kind='progress_bar',value=0.5,transition={change={props={'value'},duration=300}}}",
            "{kind='shader',source='x',extraArgs={progress=0},transition={change={}}}",
            "{kind='shader',source='x',extraArgs={progress=0},transition={change={props={'extraArgs.progress'}}}}",
            "{kind='image',content='x.png',opacity=0.5,transition={change={props={'opacity'}}}}",
            "{kind='svg',content='x.svg',opacity=0.5,transition={change={props={'opacity'}}}}",
            "{kind='rule',thickness=2,transition={change={props={'thickness'}}}}",
            "{kind='progress_bar',value=0.5,girth=10,transition={change={props={'girth'}}}}",
            "{kind='qr_code',content='hello',cell_size=4,transition={change={props={'cell_size','total_size'}}}}",
            "{kind='grid',content={{kind='text',content='a'}},columns=1,spacing=4,transition={change={props={'spacing'}}}}",
            "{kind='table',columns={{header='a'}},content={{'1'}},spacing=1,transition={change={props={'spacing'}}}}",
            "{kind='scrollable',content={kind='text',content='x'},spacing=4,transition={change={props={'spacing'}}}}",
            "{kind='markdown',content='# hi',spacing=4,transition={change={props={'spacing'}}}}",
            "{kind='pane_grid',id='grid',content={{kind='text',content='a'}},spacing=4,transition={change={props={'spacing'}}}}",
        ] {
            assert!(
                element(lua.load(source).eval().unwrap(), &Ctx::new(0)).is_ok(),
                "{source}"
            );
        }
        for source in [
            "{kind='text',content='x',transition={change={props={'nope'}}}}",
            "{kind='text',content='x',transition={change={easing='nope'}}}",
            "{kind='text',content='x',transition={change={spring='nope'}}}",
            "{kind='text',content='x',transition={change={easing='linear',spring='smooth'}}}",
            "{kind='text',content='x',transition={change={duration=0}}}",
            "{kind='text',content='x',transition={bogus=true}}",
            "{kind='text',content='x',transition='yes'}",
            "{kind='text',content='x',transition={change={props={'value'}}}}",
            "{kind='progress_bar',value=2,transition={change={}}}",
            "{kind='text',content='x',transition=true}",
            "{kind='text',content='x',transition=false}",
            "{kind='text',content='x',transition={props={'color'}}}",
            "{kind='shader',source='x',extraArgs={progress=0},transition={change={props={'extraArgs.missing'}}}}",
            "{kind='shader',source='x',transition={change={props={'extraArgs.progress'}}}}",
            "{kind='text',content='x',transition={change={props={'opacity'}}}}",
            "{kind='text',content='x',transition={change={props={'hovered.background'}}}}",
            "{kind='button',content='x',transition={change={props={'hovered.width'}}}}",
            "{kind='button',content='x',transition={change={props={'hovered.icon_color'}}}}",
            "{kind='checkbox',content='x',transition={change={props={'pressed.background'}}}}",
            "{kind='image',content='x.png',transition={change={props={'girth'}}}}",
            "{kind='progress_bar',value=0.5,transition={change={props={'thickness'}}}}",
            "{kind='qr_code',content='x',transition={change={props={'spacing'}}}}",
        ] {
            assert!(
                element(lua.load(source).eval().unwrap(), &Ctx::new(0)).is_err(),
                "{source}"
            );
        }
    }

    #[test]
    fn transitions_animate_state_style_values() {
        let lua = mlua::Lua::new();
        let node: Table = lua
            .load(
                r##"
                {
                    kind = 'button',
                    content = 'x',
                    style = {
                        background = '#101010',
                        text_color = '#ffffff',
                        hovered = { background = '#ff0000' },
                        pressed = { background = '#00ff00' },
                    },
                    transition = {
                        change = {
                            props = { 'background', 'hovered.background', 'pressed.background' },
                            duration = 100,
                            easing = 'linear',
                        },
                    },
                }
                "##,
            )
            .eval()
            .unwrap();
        let transition = Transition::parse(&node).unwrap().unwrap();
        assert_eq!(transition.state_fields, ["background"]);

        let mut patch = style::StylePatch::default();
        transition.animate(&mut patch, "active");
        assert_eq!(patch.background, Some(color("#101010").unwrap()));

        transition.animate(&mut patch, "hovered");
        assert_eq!(patch.background, Some(color("#101010").unwrap()));
        let now = Instant::now();
        transition
            .state_animation
            .as_ref()
            .unwrap()
            .lock()
            .unwrap()
            .tick(now + std::time::Duration::from_millis(50));
        transition.animate(&mut patch, "hovered");
        let middle = patch.background.unwrap();
        assert_ne!(middle, color("#101010").unwrap());
        assert_ne!(middle, color("#ff0000").unwrap());

        transition
            .state_animation
            .as_ref()
            .unwrap()
            .lock()
            .unwrap()
            .tick(now + std::time::Duration::from_millis(200));
        transition.animate(&mut patch, "hovered");
        assert_eq!(patch.background, Some(color("#ff0000").unwrap()));

        transition.animate(&mut patch, "pressed");
        transition
            .state_animation
            .as_ref()
            .unwrap()
            .lock()
            .unwrap()
            .tick(now + std::time::Duration::from_millis(400));
        transition.animate(&mut patch, "pressed");
        assert_eq!(patch.background, Some(color("#00ff00").unwrap()));
    }

    #[test]
    fn shader_extra_args_tween_through_change() {
        let lua = mlua::Lua::new();
        let node = |value: f32| -> Table {
            lua.load(format!(
                "{{kind='shader',source='x',extraArgs={{progress={value}}},transition={{change={{props={{'extraArgs.progress'}},duration=200,easing='linear'}}}}}}"
            ))
            .eval()
            .unwrap()
        };
        let mut transition = Transition::parse(&node(0.0)).unwrap().unwrap();
        let target = Transition::parse(&node(1.0)).unwrap().unwrap();
        assert_eq!(transition.extra("progress"), Some(0.0));
        let mut change = iced_anim::Animated::new(transition.motion.clone(), transition.mode)
            .to(target.motion.clone());
        change.tick(std::time::Instant::now() + std::time::Duration::from_millis(100));
        assert!(change.is_animating());
        transition.motion = change.value().clone();
        let value = transition.extra("progress").unwrap();
        assert!(value > 0.0 && value < 1.0, "{value}");
    }

    #[test]
    fn numeric_change_props_expose_selected_values() {
        let lua = mlua::Lua::new();
        for (source, prop, value) in [
            (
                "{kind='image',content='x.png',opacity=0.5,transition={change={props={'opacity'}}}}",
                "opacity",
                0.5,
            ),
            (
                "{kind='rule',thickness=2,transition={change={props={'thickness'}}}}",
                "thickness",
                2.0,
            ),
            (
                "{kind='progress_bar',value=0.5,girth=10,transition={change={props={'girth'}}}}",
                "girth",
                10.0,
            ),
            (
                "{kind='qr_code',content='x',cell_size=4,transition={change={props={'cell_size'}}}}",
                "cell_size",
                4.0,
            ),
            (
                "{kind='qr_code',content='x',total_size=40,transition={change={props={'total_size'}}}}",
                "total_size",
                40.0,
            ),
            (
                "{kind='grid',content={},spacing=4,transition={change={props={'spacing'}}}}",
                "spacing",
                4.0,
            ),
        ] {
            let node: Table = lua.load(source).eval().unwrap();
            let transition = Transition::parse(&node).unwrap().unwrap();
            assert_eq!(transition.value(prop), Some(value), "{source}");
            assert_eq!(
                transition.value("opacity"),
                if prop == "opacity" { Some(value) } else { None }
            );
        }
    }

    #[test]
    fn padding_and_radius_accept_per_side_tables() {
        let lua = mlua::Lua::new();
        for source in [
            "{kind='container',content={kind='text',content='x'},padding={left=1,right=2,top=3,bottom=4}}",
            "{kind='container',content={kind='text',content='x'},padding={left=1}}",
            "{kind='container',content={kind='text',content='x'},padding={left=1},transition={change={}}}",
            "{kind='rule',style={radius={top_left=1,top_right=2,bottom_right=3,bottom_left=4}}}",
            "{kind='progress_bar',value=0.5,style={border={radius={top_left=1}}}}",
            "{kind='container',content={kind='text',content='x'},style={border={radius={top_left=1}}}}",
        ] {
            assert!(
                element(lua.load(source).eval().unwrap(), &Ctx::new(0)).is_ok(),
                "{source}"
            );
        }
        for source in [
            "{kind='container',content={kind='text',content='x'},padding={left=-1}}",
            "{kind='container',content={kind='text',content='x'},padding={left=1,typo=2}}",
            "{kind='container',content={kind='text',content='x'},padding='4'}",
            "{kind='rule',style={radius={top_left=-1}}}",
            "{kind='rule',style={radius={top_left=1,typo=2}}}",
            "{kind='rule',style={radius='4'}}",
        ] {
            assert!(
                element(lua.load(source).eval().unwrap(), &Ctx::new(0)).is_err(),
                "{source}"
            );
        }
    }

    #[test]
    fn text_and_layout_props_build_and_validate() {
        let lua = mlua::Lua::new();
        for source in [
            "{kind='text',content='x',line_height=1.4}",
            "{kind='text',content='x',line_height={pixels=20}}",
            "{kind='text',content='x',wrap='none'}",
            "{kind='text',content='x',wrap='glyph'}",
            "{kind='text',content='x',wrap=false}",
            "{kind='text',content='x',align_x='center',align_y='bottom',width=100,height=40}",
            "{kind='text',content='x',shaping='advanced'}",
            "{kind='text',content='x',center=true}",
            "{kind='row',content={},align_y='center',clip=true}",
            "{kind='column',content={},align_x='right',clip=true}",
            "{kind='container',content={kind='text',content='x'},align_x='center',align_y='bottom',max_width=100,max_height=50,clip=true}",
            "{kind='svg',content='x.svg'}",
            "{kind='svg',content='x.svg',opacity=0.5,content_fit='cover'}",
            "{kind='rule'}",
            "{kind='rule',direction='vertical',thickness=2}",
            "{kind='rule',style={color='#ff0000',radius=2,snap=false}}",
            "{kind='progress_bar',value=0.5,girth=4}",
            "{kind='progress_bar',value=1,vertical=true}",
            "{kind='progress_bar',value=0.5,style={background='#111111',bar='#22ff22',border={color='#333333',width=1,radius=2}}}",
            "{kind='qr_code',content='hello'}",
            "{kind='qr_code',content='hello',cell_size=4,style={cell='#000000',background='#ffffff'}}",
            "{kind='icon',content='1'}",
            "{kind='icon',content='a',size=20,color='#ff0000',width=24,height=24}",
        ] {
            assert!(
                element(lua.load(source).eval().unwrap(), &Ctx::new(0)).is_ok(),
                "{source}"
            );
        }
        for source in [
            "{kind='text',content='x',line_height=0}",
            "{kind='text',content='x',line_height={pixels=-1}}",
            "{kind='text',content='x',wrap='sideways'}",
            "{kind='text',content='x',align_x='up'}",
            "{kind='text',content='x',align_y='left'}",
            "{kind='text',content='x',shaping='nope'}",
            "{kind='row',content={},align_y='left'}",
            "{kind='column',content={},align_x='top'}",
            "{kind='container',content={kind='text',content='x'},max_width=-1}",
            "{kind='svg',content='',opacity=0.5}",
            "{kind='svg',content='x.svg',opacity=2}",
            "{kind='svg',content='x.svg',content_fit='nope'}",
            "{kind='rule',direction='diagonal'}",
            "{kind='rule',thickness=-1}",
            "{kind='rule',style={radius=-1}}",
            "{kind='rule',style={bogus=1}}",
            "{kind='progress_bar',value=2}",
            "{kind='progress_bar',value=0.5,girth=0}",
            "{kind='progress_bar',value=0.5,style={bar='nope'}}",
            "{kind='qr_code',content=''}",
            "{kind='qr_code',content='x',cell_size=0}",
            "{kind='qr_code',content='x',style={cell='bogus'}}",
            "{kind='icon',content=''}",
            "{kind='icon',content='ab'}",
            "{kind='icon',content='a',font='monospace'}",
            "{kind='icon',content='a',size=0}",
            "{kind='icon',content='a',color='nope'}",
        ] {
            assert!(
                element(lua.load(source).eval().unwrap(), &Ctx::new(0)).is_err(),
                "{source}"
            );
        }
    }

    #[test]
    fn interactive_widgets_build_and_validate() {
        let lua = mlua::Lua::new();
        for source in [
            "{kind='stack',content={{kind='text',content='x'}}}",
            "{kind='float',content={kind='text',content='x'},scale=1.5}",
            "{kind='pin',content={kind='text',content='x'},x=4,y=8}",
            "{kind='tooltip',content={kind='text',content='x'},tooltip='tip',position='bottom',gap=4}",
            "{kind='grid',content={{kind='text',content='a'},{kind='text',content='b'}},columns=2,spacing=4,fluid=200}",
            "{kind='checkbox',value=true,label='x',text_size=14,on_toggle='toggle'}",
            "{kind='radio',value=1,selected=1,label='x',text_size=14,on_select='select'}",
            "{kind='toggler',value=true,label='x',on_toggle='toggle'}",
            "{kind='slider',min=0,max=10,value=5,step=1,on_change='change',on_release='release'}",
            "{kind='vertical_slider',min=0,max=1,value=0.5,on_change='change'}",
            "{kind='mouse_area',content={kind='text',content='x'},on_press='p',on_move='m',on_scroll='s',on_enter='e',on_exit='x'}",
            "{kind='sensor',content={kind='text',content='x'},on_show='show',on_resize='resize',on_hide='hide'}",
            "{kind='scrollable',content={kind='text',content='x'},direction='both',spacing=4,auto_scroll=true,id='scroll',on_scroll='scroll'}",
            "{kind='scrollable',content={kind='space'},scrollbar_hidden=true}",
            "{kind='scrollable',content={kind='space'},direction='vertical',scrollbar_hidden=true}",
            "{kind='scrollable',content={kind='space'},direction='horizontal',scrollbar_hidden=true}",
            "{kind='scrollable',content={kind='space'},direction='both',scrollbar_hidden=true}",
            "{kind='scrollable',content={kind='space'},scrollbar_hidden=false}",
            "{kind='text_input',placeholder='p',value='v',secure=true,id='input',size=14,align_x='center',font={family='monospace',weight='bold',style='italic'},on_input='input',on_submit='submit',on_paste='paste'}",
            "{kind='pick_list',options={'a','b'},selected='a',placeholder='pick',text_size=14,on_select='select'}",
            "{kind='checkbox',value=true,style={background='#ff0000',icon_color='#ffffff',border={color='#00ff00',width=1,radius=4},hovered={background='#222222'}}}",
            "{kind='radio',value=1,selected=1,style={background='#ff0000',dot_color='#ffffff',border={color='#00ff00',width=1},text_color='#ffffff'}}",
            "{kind='toggler',value=true,style={background='#ff0000',foreground='#ffffff',border={color='#00ff00',width=1,radius=4}}}",
            "{kind='slider',min=0,max=10,style={rail='#111111',filled='#22ff22',border={color='#333333',width=1},handle='#ffffff',handle_border={color='#000000',width=1}}}",
            "{kind='vertical_slider',min=0,max=1,style={rail='#111111',hovered={filled='#22ff22'}}}",
            "{kind='scrollable',content={kind='text',content='x'},style={background='#111111',border={color='#333333',width=1},rail='#222222',scroller='#444444'}}",
            "{kind='text_input',style={background='#111111',text_color='#ffffff',placeholder='#888888',selection='#333333',border={color='#333333',width=1},focused={border={color='#22ff22',width=2}}}}",
            "{kind='pick_list',options={'a'},style={background='#111111',text_color='#ffffff',placeholder='#888888',handle='#ffffff',border={color='#333333',width=1}}}",
            "{kind='text_editor',id='editor',value='text',placeholder='p',size=14,font={family='monospace'},wrapping='word',on_action='edit',style={background='#111111',text_color='#ffffff'}}",
            "{kind='combo_box',id='combo',options={'a','b'},value='a',placeholder='pick',on_select='select',on_input='input',style={background='#111111',text_color='#ffffff'}}",
            "{kind='table',columns={{header='a',width=100,align_x='left'},{header='b',align_y='center'}},content={{'1','2'},{'3','4'}},spacing=1}",
            "{kind='list_view',id='list',content={{kind='text',content='x'}},spacing=4,render=function(item,index) return item end}",
            "{kind='list_view',id='list-animated',content={{kind='text',content='x'}},transition={enter={duration=200,from={x=-180},callback=function() end},exit={to={scale=0}},displaced={callback=function() end}}}",
            "{kind='region',content={kind='text',content='x'}}",
            "{kind='markdown',content='# hi',text_size=14,spacing=8,on_link_click='link',style={link_color='#22ff22',inline_code_color='#ffffff'}}",
            "{kind='responsive',content=function(size) return {kind='text',content=tostring(size.width)} end}",
            "{kind='canvas',content={{type='rect',x=0,y=0,width=10,height=10,color='#ff0000'},{type='circle',x=5,y=5,radius=2,color='#00ff00'},{type='line',from={x=0,y=0},to={x=10,y=10},color='#0000ff',width=2},{type='path',points={{x=0,y=0},{x=1,y=1}},color='#ffffff',fill=true},{type='text',x=1,y=1,content='hi',size=12,color='#ffffff'}}}",
            "{kind='pane_grid',id='grid',content={{kind='text',content='a'},{kind='text',content='b'}},direction='vertical',spacing=2,min_size=10,on_click='click',on_resize='resize',on_drag='drag'}",
            "{kind='button',content='x',clip=true,style={background='#ff0000',text_color='#ffffff',border={color='#00ff00',width=1,radius=4},active={background='#111111'},hovered={background='#222222'}}}",
            "{kind='container',content={kind='text',content='x'},style={background='#ff0000',border={color='#00ff00',width=2,radius=4},shadow={color='#00000080',offset={x=0,y=2},blur=8}}}",
        ] {
            assert!(
                element(lua.load(source).eval().unwrap(), &Ctx::new(0)).is_ok(),
                "{source}"
            );
        }
        for source in [
            "{kind='text',content='x',bogus=1}",
            "{kind='float',content={kind='text',content='x'},scale=0}",
            "{kind='tooltip',content={kind='text',content='x'},tooltip='t',position='middle'}",
            "{kind='grid',content={},columns=0}",
            "{kind='slider',min=1,max=0}",
            "{kind='text_input',size=0}",
            "{kind='text_input',font={weight='heavy'}}",
            "{kind='text_input',font=string.rep('x',1025)}",
            "{kind='button',content='x',style={border={width=-1}}}",
            "{kind='scrollable',content={},direction='diagonal'}",
            "{kind='scrollable',content={kind='space'},scrollbar_hidden='true'}",
            "{kind='scrollable',content={kind='space'},scrollbar_hidden=0}",
            "{kind='pick_list',options='a'}",
            "{kind='checkbox',style={bogus=1}}",
            "{kind='radio',style={hovered={bogus=1}}}",
            "{kind='slider',style={handle='nope'}}",
            "{kind='text_input',style={focused={nope=1}}}",
            "{kind='text_editor',value='x'}",
            "{kind='text_editor',id='e',style={bogus=1}}",
            "{kind='combo_box',options={'a'}}",
            "{kind='combo_box',id='c',options={'a'},style={bogus=1}}",
            "{kind='table',columns={},content={}}",
            "{kind='table',columns={{header='a'}},content={{false}}}",
            "{kind='list_view',content={}}",
            "{kind='list_view',id='list',content={},direction='diagonal'}",
            "{kind='list_view',id='list',content={function() end}}",
            "{kind='list_view',id='list',content={},transition={duration=0}}",
            "{kind='list_view',id='list',content={},transition={x={from='left'}}}",
            "{kind='list_view',id='list',content={},transition={bogus=true}}",
            "{kind='list_view',id='list',key='',content={}}",
            "{kind='list_view',id='list',key='id',content={{kind='text',content='x'}}}",
            "{kind='region'}",
            "{kind='markdown',content='x',style={bogus='#ffffff'}}",
            "{kind='canvas',content={{type='rect',x=0,y=0,width=0,height=10,color='#ff0000'}}}",
            "{kind='canvas',content={{type='circle',x=0,y=0,radius=1}}}",
            "{kind='canvas',content={{type='path',points={{x=0,y=0}},color='#ffffff'}}}",
            "{kind='pane_grid',content={{kind='text',content='a'}},direction='diagonal'}",
            "{kind='pane_grid',content={}}",
        ] {
            assert!(
                element(lua.load(source).eval().unwrap(), &Ctx::new(0)).is_err(),
                "{source}"
            );
        }
    }

    #[test]
    fn transitions_interpolate_text_color() {
        let lua = mlua::Lua::new();
        let node: Table = lua
            .load("{kind='text',content='x',color='#ff0000',transition={change={props={'color'}}}}")
            .eval()
            .unwrap();
        let transition = Transition::parse(&node).unwrap().unwrap();
        assert_eq!(transition.color(), Some(color("#ff0000").unwrap()));
        assert_eq!(transition.value("width"), None);

        let node: Table = lua
            .load("{kind='text',content='x',color='#ff0000',transition={change={props={'width'}}}}")
            .eval()
            .unwrap();
        let transition = Transition::parse(&node).unwrap().unwrap();
        assert_eq!(transition.color(), None);
    }

    #[test]
    fn transitions_interpolate_style_values() {
        let lua = mlua::Lua::new();
        let node: Table = lua
            .load(
                "{kind='button',content='x',style={background='#ff0000',text_color='#ffffff',border={color='#00ff00',width=1,radius=4},shadow={color='#00000080',offset={x=0,y=2},blur=8},hovered={background='#00ff00'}},transition={change={props={'background','text_color','border','shadow'}}}}",
            )
            .eval()
            .unwrap();
        let mut transition = Transition::parse(&node).unwrap().unwrap();
        assert_eq!(transition.background(), Some(color("#ff0000").unwrap()));
        assert_eq!(transition.text_color(), Some(color("#ffffff").unwrap()));
        assert_eq!(transition.border().unwrap().width, 1.0);
        assert_eq!(transition.shadow().unwrap().blur_radius, 8.0);

        let style: Table = node.get("style").unwrap();
        transition.motion.background = color("#0000ff").unwrap();
        transition.motion.text_color = color("#000000").unwrap();
        transition.motion.border.width = 2.0;
        transition.motion.shadow.blur_radius = 4.0;
        let patch = style::style_patch(&style, None, Some(&transition)).unwrap();
        assert_eq!(patch.background, Some(color("#0000ff").unwrap()));
        assert_eq!(patch.text_color, Some(color("#000000").unwrap()));
        assert_eq!(patch.border.unwrap().width, 2.0);
        assert_eq!(patch.shadow.unwrap().blur_radius, 4.0);

        let hovered: Table = style.get("hovered").unwrap();
        let patch = style::style_patch(&style, Some(&hovered), Some(&transition)).unwrap();
        assert_eq!(patch.background, Some(color("#00ff00").unwrap()));
    }

    #[test]
    fn transitions_interpolate_progress_bar_value() {
        let lua = mlua::Lua::new();
        let node: Table = lua
            .load("{kind='progress_bar',value=0.4,transition={change={props={'value'},easing='linear'}}}")
            .eval()
            .unwrap();
        let transition = Transition::parse(&node).unwrap().unwrap();
        assert_eq!(transition.value("value"), Some(0.4));
        assert_eq!(transition.value("width"), None);

        let node: Table = lua
            .load("{kind='text',content='x',transition={change={props={'value'}}}}")
            .eval()
            .unwrap();
        assert!(Transition::parse(&node).is_err());
    }

    #[test]
    fn spring_progress_rebuilds_at_both_boundaries() {
        let lua = mlua::Lua::new();
        let node: Table = lua.load("{kind='progress_bar',value=0,transition={change={props={'value'},spring='bouncy',duration=200}}}").eval().unwrap();
        for (from, to) in [(0.0, 1.0), (1.0, 0.0)] {
            node.set("value", from).unwrap();
            let start = Transition::parse(&node).unwrap().unwrap();
            node.set("value", to).unwrap();
            let mut target = Transition::parse(&node).unwrap().unwrap();
            let mut motion =
                iced_anim::Animated::new(start.motion, target.mode).to(target.motion.clone());
            let start = std::time::Instant::now();
            let mut overshot = false;
            for ms in (10..400).step_by(10) {
                motion.tick(start + std::time::Duration::from_millis(ms));
                target.motion = motion.value().clone();
                overshot |= !(0.0..=1.0).contains(&target.motion.value);
                build(&node, &Ctx::new(0), Some(&target)).unwrap();
            }
            assert!(overshot);
            for invalid in [-1.0, 2.0, f32::NAN, f32::INFINITY] {
                node.set("value", invalid).unwrap();
                assert!(build(&node, &Ctx::new(0), Some(&target)).is_err());
            }
        }
    }

    #[test]
    fn animated_dimensions_still_validate_the_target() {
        let lua = mlua::Lua::new();
        for axis in ["width", "height"] {
            for invalid in ["-1", "0/0", "math.huge", "1e100"] {
                let node = lua.load(format!("{{kind='space',{axis}={invalid},transition={{change={{props={{'{axis}'}}}}}}}}" )).eval().unwrap();
                assert!(element(node, &Ctx::new(0)).is_err(), "{axis}={invalid}");
            }
        }
        let node = lua.load("{width=0}").eval().unwrap();
        assert_eq!(
            axis(&node, "width", Some(-0.1)).unwrap(),
            Some(Length::Fixed(0.0))
        );
    }

    #[test]
    fn style_transitions_match_widget_capabilities() {
        let lua = mlua::Lua::new();
        for (kind, supported) in [
            (
                "container",
                &["background", "text_color", "border", "shadow"][..],
            ),
            (
                "button",
                &["background", "text_color", "border", "shadow"][..],
            ),
            ("checkbox", &["background", "text_color", "border"][..]),
            ("radio", &["background", "text_color", "border"][..]),
            ("toggler", &["background", "text_color", "border"][..]),
            ("text_input", &["background", "text_color", "border"][..]),
            ("pick_list", &["background", "text_color", "border"][..]),
            ("text_editor", &["background", "text_color", "border"][..]),
            ("combo_box", &["background", "text_color", "border"][..]),
            ("scrollable", &["background", "border", "shadow"][..]),
            ("slider", &["border"][..]),
            ("vertical_slider", &["border"][..]),
        ] {
            for prop in ["background", "text_color", "border", "shadow"] {
                let node = lua
                    .load(format!(
                        "{{kind='{kind}',transition={{change={{props={{'{prop}'}}}}}}}}"
                    ))
                    .eval()
                    .unwrap();
                assert_eq!(
                    Transition::parse(&node).is_ok(),
                    supported.contains(&prop),
                    "{kind}.{prop}"
                );
            }
        }
    }

    #[test]
    fn generic_props_build_for_every_kind() {
        let lua = mlua::Lua::new();
        for kind in [
            "{kind='text',content='x'}",
            "{kind='icon',content='1'}",
            "{kind='image',content='/tmp/x.png'}",
            "{kind='svg',content='/tmp/x.svg'}",
            "{kind='rule'}",
            "{kind='progress_bar',value=0.5}",
            "{kind='qr_code',content='x'}",
            "{kind='row',content={}}",
            "{kind='column',content={}}",
            "{kind='container',content={kind='text',content='x'}}",
            "{kind='button',content='x'}",
            "{kind='space'}",
            "{kind='stack',content={{kind='text',content='x'}}}",
            "{kind='float',content={kind='text',content='x'}}",
            "{kind='pin',content={kind='text',content='x'}}",
            "{kind='tooltip',content={kind='text',content='x'},tooltip='t'}",
            "{kind='grid',content={{kind='text',content='x'}}}",
            "{kind='checkbox'}",
            "{kind='radio'}",
            "{kind='toggler'}",
            "{kind='slider'}",
            "{kind='vertical_slider'}",
            "{kind='mouse_area',content={kind='text',content='x'}}",
            "{kind='sensor',content={kind='text',content='x'}}",
            "{kind='region',content={kind='text',content='x'}}",
            "{kind='measure',id='target',content={kind='text',content='x'}}",
            "{kind='scrollable',content={kind='text',content='x'}}",
            "{kind='text_input'}",
            "{kind='pick_list',options={}}",
        ] {
            let source = format!(
                "{}{}",
                &kind[..kind.len() - 1],
                ",width=10,height=10,padding=2}"
            );
            assert!(
                element(lua.load(&source).eval().unwrap(), &Ctx::new(0)).is_ok(),
                "{source}"
            );
        }
    }

    #[test]
    fn absent_axes_stay_absent_and_present_axes_parse() {
        let lua = mlua::Lua::new();
        let node: Table = lua
            .load("{width='fill', height={fill_portion=2}}")
            .eval()
            .unwrap();
        assert_eq!(axis(&node, "width", None).unwrap(), Some(Length::Fill));
        assert_eq!(
            axis(&node, "height", None).unwrap(),
            Some(Length::FillPortion(2))
        );
        let empty: Table = lua.load("{}").eval().unwrap();
        assert_eq!(axis(&empty, "width", Some(10.0)).unwrap(), None);
    }

    #[test]
    fn direct_size_matches_widget_capabilities() {
        for kind in [
            "text",
            "icon",
            "image",
            "svg",
            "row",
            "column",
            "container",
            "button",
            "space",
            "stack",
            "pin",
            "scrollable",
        ] {
            assert_eq!(direct_size(kind), (true, true), "{kind}");
        }
        for kind in [
            "checkbox",
            "radio",
            "toggler",
            "slider",
            "text_input",
            "pick_list",
        ] {
            assert_eq!(direct_size(kind), (true, false), "{kind}");
        }
        assert_eq!(direct_size("vertical_slider"), (false, true));
        for kind in [
            "rule",
            "progress_bar",
            "qr_code",
            "float",
            "tooltip",
            "grid",
            "mouse_area",
            "sensor",
            "region",
        ] {
            assert_eq!(direct_size(kind), (false, false), "{kind}");
        }
    }

    #[test]
    fn list_view_callbacks_do_not_run_during_view_construction() {
        let lua = mlua::Lua::new();
        lua.load("added = 0; removed = 0").exec().unwrap();
        let ctx = Ctx::new(7);
        let first: Table = lua
            .load(
                "{kind='list_view',id='list-test',content={{kind='text',content='a'},{kind='text',content='b'}},\
                 transition={enter={callback=function() added = added + 1 end},\
                 exit={to={scale=0},callback=function() removed = removed + 1 end}}}",
            )
            .eval()
            .unwrap();
        element(first, &ctx).unwrap();
        assert_eq!(lua.globals().get::<u32>("added").unwrap(), 0);
        assert_eq!(lua.globals().get::<u32>("removed").unwrap(), 0);

        let second: Table = lua
            .load(
                "{kind='list_view',id='list-test',content={{kind='text',content='a'}},\
                 transition={enter={callback=function() added = added + 1 end},\
                 exit={to={scale=0},callback=function() removed = removed + 1 end}}}",
            )
            .eval()
            .unwrap();
        element(second, &ctx).unwrap();
        assert_eq!(lua.globals().get::<u32>("added").unwrap(), 0);
        assert_eq!(lua.globals().get::<u32>("removed").unwrap(), 0);
    }

    #[test]
    fn list_view_transition_slides_on_either_axis() {
        let lua = mlua::Lua::new();
        let ctx = Ctx::new(41);
        for source in [
            "{kind='list_view',id='axis-y',content={{kind='text',content='a'}},transition={enter={duration=180,from={y=-30}}}}",
            "{kind='list_view',id='axis-exit',content={{kind='text',content='a'}},transition={exit={duration=80,to={y=30}}}}",
            "{kind='list_view',id='axis-both',content={{kind='text',content='a'}},transition={enter={duration=180,from={x=-30,y=-10}}}}",
        ] {
            element(lua.load(source).eval().unwrap(), &ctx).unwrap();
        }
        let bad: Table = lua
            .load(
                "{kind='list_view',id='axis-bad',content={{kind='text',content='a'}},\
                  transition={enter={duration=180,from={z=-30}}}}",
            )
            .eval()
            .unwrap();
        assert!(element(bad, &ctx).is_err());
    }

    #[test]
    fn list_view_displacement_callbacks_wait_for_animation_completion() {
        let lua = mlua::Lua::new();
        lua.load("added = ''; removed = ''; moved = ''")
            .exec()
            .unwrap();
        let ctx = Ctx::new(8);
        let first: Table = lua
            .load(
                "{kind='list_view',id='list-displaced',content={'a','b','c'},\
                  transition={displaced={duration=100,callback=function() moved = moved .. 'm' end},\
                  enter={callback=function() added = added .. 'a' end},\
                  exit={callback=function() removed = removed .. 'r' end}}}",
            )
            .eval()
            .unwrap();
        element(first, &ctx).unwrap();
        assert_eq!(lua.globals().get::<String>("added").unwrap(), "");
        assert_eq!(lua.globals().get::<String>("removed").unwrap(), "");
        assert_eq!(lua.globals().get::<String>("moved").unwrap(), "");

        let second: Table = lua
            .load(
                "{kind='list_view',id='list-displaced',content={'b','c'},\
                  transition={displaced={duration=100,callback=function() moved = moved .. 'm' end},\
                  enter={callback=function() added = added .. 'a' end},\
                  exit={callback=function() removed = removed .. 'r' end}}}",
            )
            .eval()
            .unwrap();
        element(second, &ctx).unwrap();
        assert_eq!(lua.globals().get::<String>("added").unwrap(), "");
        assert_eq!(lua.globals().get::<String>("removed").unwrap(), "");
        assert_eq!(lua.globals().get::<String>("moved").unwrap(), "");
    }

    #[test]
    fn list_view_key_keeps_identity_for_rebuilt_data_items() {
        let lua = mlua::Lua::new();
        lua.load("added = ''; moved = ''").exec().unwrap();
        let ctx = Ctx::new(9);
        let first: Table = lua
            .load(
                "{kind='list_view',id='list-key',key='id',content={{id='a'},{id='b'}},\
                 render=function(item) return {kind='text',content=item.id} end,\
                  transition={enter={duration=100,callback=function() added = added .. 'a' end},\
                  displaced={callback=function() moved = moved .. 'm' end}}}",
            )
            .eval()
            .unwrap();
        element(first, &ctx).unwrap();
        assert_eq!(lua.globals().get::<String>("added").unwrap(), "");

        let second: Table = lua
            .load(
                "{kind='list_view',id='list-key',key='id',content={{id='a'},{id='x'},{id='b'}},\
                 render=function(item) return {kind='text',content=item.id} end,\
                  transition={enter={duration=100,callback=function() added = added .. 'a' end},\
                  displaced={callback=function() moved = moved .. 'm' end}}}",
            )
            .eval()
            .unwrap();
        element(second, &ctx).unwrap();
        assert_eq!(lua.globals().get::<String>("added").unwrap(), "");
        assert_eq!(lua.globals().get::<String>("moved").unwrap(), "");
    }

    #[test]
    fn prebuilt_elements_receive_defaults_at_render_time() {
        let lua = mlua::Lua::new();
        crate::lua::load(&lua).unwrap();
        let node: Table = lua
            .load(
                r#"
                local image = iced.image('x.png')
                iced.overrides(iced.image, {opacity = 2})
                return image
            "#,
            )
            .eval()
            .unwrap();
        let ctx = Ctx::new(0).with_lua(lua);
        assert!(element(node, &ctx).is_err());
    }

    #[test]
    fn variant_selection_builds_without_leaking_the_selector() {
        let lua = mlua::Lua::new();
        crate::lua::load(&lua).unwrap();
        let ctx = Ctx::new(0).with_lua(lua.clone());
        lua.load(
            r##"
            iced.stylesheet({
                button = {
                    default = {padding = 4, style = {background = '#202020'}},
                    primary = {style = {background = '#5865f2'}},
                },
            })
        "##,
        )
        .exec()
        .unwrap();

        // `variant` must be consumed before Rust key validation sees the props.
        let node: Table = lua
            .load("return iced.button('save', iced.variant('primary'))")
            .eval()
            .unwrap();
        assert!(element(node, &ctx).is_ok());

        let node: Table = lua
            .load("return iced.button('save', iced.variant('primary', {padding = 6}))")
            .eval()
            .unwrap();
        assert!(element(node, &ctx).is_ok());
    }

    #[test]
    fn builds_images_and_validates_their_props() {
        let lua = mlua::Lua::new();
        for source in [
            "{kind='image',content='/tmp/x.png'}",
            "{kind='image',content='x.png',content_fit='cover',width=64,height=64}",
            "{kind='image',content='x.png',opacity=0.5}",
        ] {
            assert!(
                element(lua.load(source).eval().unwrap(), &Ctx::new(0)).is_ok(),
                "{source}"
            );
        }
        for source in [
            "{kind='image',content=''}",
            "{kind='image',content='x.png',content_fit='bogus'}",
            "{kind='image',content='x.png',opacity=2}",
        ] {
            assert!(
                element(lua.load(source).eval().unwrap(), &Ctx::new(0)).is_err(),
                "{source}"
            );
        }
    }
}
