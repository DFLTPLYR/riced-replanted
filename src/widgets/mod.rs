mod advanced;
mod canvas;
mod containers;
mod enums;
mod input;
mod layout;
mod media;
mod region;
mod scale;
mod state;
mod style;
mod text;

use std::time::Duration;

use iced::widget::Space;
use iced::{Color, Element, Length, widget};
use iced_anim::animated::Mode;
use iced_anim::spring::Motion as SpringMotion;
use iced_anim::transition::Easing;
use iced_anim::{Animate, AnimationBuilder};
use mlua::{Table, Value};

use crate::runtime::Message;

pub fn color(value: &str) -> mlua::Result<Color> {
    if !value.is_ascii() {
        return Err(mlua::Error::runtime("colors must be ASCII hex strings"));
    }
    value.parse().map_err(|_| {
        mlua::Error::runtime(format!("invalid color {value:?}; use #RRGGBB or #RRGGBBAA"))
    })
}

pub(crate) fn intern(value: &str) -> &'static str {
    enums::intern_font(value)
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

#[derive(Clone, PartialEq, Animate)]
struct Motion {
    width: f32,
    height: f32,
    padding: f32,
    spacing: f32,
    size: f32,
    color: Color,
}

impl Motion {
    fn target(table: &Table) -> mlua::Result<Self> {
        let color = match table.get::<String>("kind")?.as_str() {
            "text" => match table.get::<Option<String>>("color")? {
                Some(value) => color(&value)?,
                None => Color::WHITE,
            },
            _ => Color::WHITE,
        };
        Ok(Self {
            width: pixels(table.get::<Value>("width")?).unwrap_or(0.0),
            height: pixels(table.get::<Value>("height")?).unwrap_or(0.0),
            padding: number(table, "padding", 0.0)?,
            spacing: number(table, "spacing", 0.0)?,
            size: number(table, "size", 16.0)?,
            color,
        })
    }
}

const ANIMATABLE: [&str; 6] = ["width", "height", "padding", "spacing", "size", "color"];

/// An element's `transition` prop: which pixel props interpolate, and how.
#[derive(Clone)]
struct Transition {
    motion: Motion,
    props: Vec<String>,
    mode: Mode,
}

impl Transition {
    fn parse(node: &Table) -> mlua::Result<Option<Self>> {
        let config = match node.get::<Value>("transition")? {
            Value::Nil | Value::Boolean(false) => return Ok(None),
            Value::Boolean(true) => None,
            Value::Table(table) => Some(table),
            _ => {
                return Err(mlua::Error::runtime(
                    "transition must be true, false, or a table",
                ));
            }
        };
        let mut props = ANIMATABLE.map(String::from).to_vec();
        let mut easing = None;
        let mut spring = None;
        let mut duration = None;
        if let Some(config) = &config {
            crate::runtime::keys(config, &["props", "easing", "spring", "duration"])?;
            if let Some(list) = config.get::<Option<Table>>("props")? {
                props = list
                    .sequence_values::<String>()
                    .collect::<mlua::Result<Vec<_>>>()?;
                if let Some(name) = props
                    .iter()
                    .find(|name| !ANIMATABLE.contains(&name.as_str()))
                {
                    return Err(mlua::Error::runtime(format!(
                        "unknown transition prop: {name}"
                    )));
                }
            }
            easing = config
                .get::<Option<String>>("easing")?
                .map(|name| match name.as_str() {
                    "linear" => Ok(Easing::LINEAR),
                    "ease" => Ok(Easing::EASE),
                    "in" | "ease_in" => Ok(Easing::EASE_IN),
                    "out" | "ease_out" => Ok(Easing::EASE_OUT),
                    "in_out" | "ease_in_out" => Ok(Easing::EASE_IN_OUT),
                    _ => Err(mlua::Error::runtime(format!("unknown easing: {name}"))),
                })
                .transpose()?;
            spring = config
                .get::<Option<String>>("spring")?
                .map(|name| match name.as_str() {
                    "smooth" => Ok(SpringMotion::SMOOTH),
                    "snappy" => Ok(SpringMotion::SNAPPY),
                    "bouncy" => Ok(SpringMotion::BOUNCY),
                    "instant" => Ok(SpringMotion::INSTANT),
                    _ => Err(mlua::Error::runtime(format!("unknown spring: {name}"))),
                })
                .transpose()?;
            if let Some(ms) = config.get::<Option<f64>>("duration")? {
                if !(1.0..=86_400_000.0).contains(&ms) || ms.fract() != 0.0 {
                    return Err(mlua::Error::runtime(
                        "transition duration must be a whole number from 1 to 86400000 ms",
                    ));
                }
                duration = Some(Duration::from_millis(ms as u64));
            }
        }
        let mode = match (spring, easing) {
            (Some(_), Some(_)) => {
                return Err(mlua::Error::runtime(
                    "transition takes easing or spring, not both",
                ));
            }
            (Some(motion), None) => {
                Mode::Spring(duration.map_or(motion, |d| motion.with_duration(d)))
            }
            (None, Some(easing)) => {
                Mode::Transition(duration.map_or(easing, |d| easing.with_duration(d)))
            }
            (None, None) => Mode::Spring(duration.map_or(SpringMotion::SMOOTH, |d| {
                SpringMotion::SMOOTH.with_duration(d)
            })),
        };
        Ok(Some(Self {
            motion: Motion::target(node)?,
            props,
            mode,
        }))
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
            _ => None,
        }
    }

    fn color(&self) -> Option<Color> {
        self.props
            .iter()
            .any(|prop| prop == "color")
            .then_some(self.motion.color)
    }
}

#[derive(Clone)]
pub struct Ctx {
    pub generation: u64,
    pub component: Option<(String, u64)>,
    lua: Option<mlua::Lua>,
    window: Option<iced::window::Id>,
}

impl Ctx {
    pub fn new(generation: u64) -> Self {
        Self {
            generation,
            component: None,
            lua: None,
            window: None,
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

    fn window(&self) -> Option<iced::window::Id> {
        self.window
    }

    fn lua(&self) -> mlua::Result<&mlua::Lua> {
        self.lua
            .as_ref()
            .ok_or_else(|| mlua::Error::runtime("this widget needs a Lua context"))
    }
}

pub fn element(node: Table, ctx: &Ctx) -> mlua::Result<Element<'static, Message>> {
    let tagged;
    let ctx = match node.get::<Option<String>>("__component_id")? {
        Some(id) => {
            tagged = Ctx {
                generation: ctx.generation,
                component: Some((id, node.get::<u64>("__component_revision")?)),
                lua: ctx.lua.clone(),
                window: ctx.window,
            };
            &tagged
        }
        None => ctx,
    };
    // list_view owns its transition table for item enter/leave; do not treat it
    // as the generic whole-widget property transition.
    if node.get::<Option<String>>("kind")?.as_deref() == Some("list_view") {
        return build(&node, ctx, None);
    }
    let Some(transition) = Transition::parse(&node)? else {
        return build(&node, ctx, None);
    };
    // Build once so Lua errors reach the caller; the builder closure cannot report them.
    build(&node, ctx, Some(&transition))?;
    let ctx = ctx.clone();
    let target = transition.motion.clone();
    let mode = transition.mode;
    Ok(AnimationBuilder::new(target, move |motion| {
        let mut transition = transition.clone();
        transition.motion = motion;
        build(&node, &ctx, Some(&transition)).unwrap_or_else(|_| Space::new().into())
    })
    .animation(mode)
    .animates_layout(true)
    .into())
}

#[derive(Clone, Copy)]
pub(super) struct Layout {
    pub(super) width: Option<Length>,
    pub(super) height: Option<Length>,
    pub(super) padding: f32,
}

fn build(
    node: &Table,
    ctx: &Ctx,
    transition: Option<&Transition>,
) -> mlua::Result<Element<'static, Message>> {
    const COMMON: &[&str] = &[
        "kind",
        "content",
        "width",
        "height",
        "padding",
        "transition",
        "__component_id",
        "__component_revision",
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
        "image" => &["content_fit"],
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
        "list_view" => &[
            "id",
            "direction",
            "spacing",
            "render",
            "on_item_added",
            "on_item_removed",
            "onItemAdded",
            "onItemRemoved",
        ],
        "region" => &[],
        "markdown" => &["text_size", "spacing", "on_link_click", "style"],
        "responsive" => &[],
        "canvas" => &[],
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
            Some(padding) => padding,
            None => number(node, "padding", 0.0)?,
        },
    };
    let element = match kind.as_str() {
        "text" => text::build(node, transition, layout)?,
        "image" => media::image(node, layout)?,
        "svg" => media::svg(node, layout)?,
        "rule" => media::rule(node)?,
        "progress_bar" => media::progress_bar(node)?,
        "qr_code" => media::qr_code(node)?,
        "row" | "column" => layout::row_column(node, ctx, transition, layout, &kind)?,
        "grid" => layout::grid(node, ctx)?,
        "space" => layout::space(layout),
        "container" => containers::container(node, ctx, layout)?,
        "button" => containers::button(node, ctx, layout)?,
        "stack" => containers::stack(node, ctx, layout)?,
        "float" => containers::float(node, ctx)?,
        "pin" => containers::pin(node, ctx, layout)?,
        "tooltip" => containers::tooltip(node, ctx)?,
        "checkbox" => input::checkbox(node, ctx, layout)?,
        "radio" => input::radio(node, ctx, layout)?,
        "toggler" => input::toggler(node, ctx, layout)?,
        "slider" | "vertical_slider" => input::slider(node, ctx, &kind, layout)?,
        "mouse_area" => input::mouse_area(node, ctx)?,
        "sensor" => input::sensor(node, ctx)?,
        "scrollable" => input::scrollable(node, ctx, layout)?,
        "text_input" => input::text_input(node, ctx, layout)?,
        "pick_list" => input::pick_list(node, ctx, layout)?,
        "text_editor" => advanced::text_editor(node, ctx, layout)?,
        "combo_box" => advanced::combo_box(node, ctx, layout)?,
        "table" => advanced::table(node, ctx, layout)?,
        "list_view" => advanced::list_view(node, ctx, layout)?,
        "region" => {
            let content = element(node.get::<Table>("content")?, ctx)?;
            region::Region::new(content, ctx.window()).into()
        }
        "markdown" => advanced::markdown(node, ctx)?,
        "responsive" => advanced::responsive(node, ctx, layout)?,
        "canvas" => canvas::canvas(node, layout)?,
        "pane_grid" => advanced::pane_grid(node, ctx, layout)?,
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
    if kind == "container" || (layout.padding == 0.0 && !needs_width && !needs_height) {
        return element;
    }
    let mut wrapper = widget::container(element);
    if needs_width && let Some(width) = layout.width {
        wrapper = wrapper.width(width);
    }
    if needs_height && let Some(height) = layout.height {
        wrapper = wrapper.height(height);
    }
    if layout.padding > 0.0 {
        wrapper = wrapper.padding(layout.padding);
    }
    wrapper.into()
}

/// Which axes a widget's own builder accepts as `Length` (see `iced_widget`).
fn direct_size(kind: &str) -> (bool, bool) {
    match kind {
        "text" | "image" | "svg" | "row" | "column" | "container" | "button" | "space"
        | "stack" | "pin" | "scrollable" => (true, true),
        "checkbox" | "radio" | "toggler" | "slider" | "text_input" | "pick_list" => (true, false),
        "table" => (true, false),
        "text_editor" | "combo_box" | "markdown" => (false, false),
        "responsive" | "pane_grid" | "canvas" => (true, true),
        "list_view" => (true, true),
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
    match (value.clone(), animated) {
        (Value::Integer(_) | Value::Number(_), Some(pixels)) => Ok(Some(Length::Fixed(pixels))),
        _ => length(value).map(Some),
    }
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
        ] {
            assert!(
                element(lua.load(source).eval().unwrap(), &Ctx::new(0)).is_err(),
                "{source}"
            );
        }
    }

    #[test]
    fn transitions_animate_listed_props_and_keep_fill_static() {
        let lua = mlua::Lua::new();
        for source in [
            "{kind='container',content={kind='text',content='x'},transition=true,padding=8,width=120}",
            "{kind='container',content={kind='text',content='x'},transition={props={'width'}},width=120}",
            "{kind='container',content={kind='text',content='x'},transition={duration=300,easing='out'},width=120}",
            "{kind='container',content={kind='text',content='x'},transition={spring='bouncy'},width=120}",
            "{kind='container',content={kind='text',content='x'},transition=true,width='fill'}",
            "{kind='row',content={},transition=true,spacing=4}",
            "{kind='text',content='x',color='#ff0000',transition=true}",
            "{kind='text',content='x',color='#ff0000',transition={props={'color'}}}",
        ] {
            assert!(
                element(lua.load(source).eval().unwrap(), &Ctx::new(0)).is_ok(),
                "{source}"
            );
        }
        for source in [
            "{kind='text',content='x',transition={props={'nope'}}}",
            "{kind='text',content='x',transition={easing='nope'}}",
            "{kind='text',content='x',transition={spring='nope'}}",
            "{kind='text',content='x',transition={easing='linear',spring='smooth'}}",
            "{kind='text',content='x',transition={duration=0}}",
            "{kind='text',content='x',transition={bogus=true}}",
            "{kind='text',content='x',transition='yes'}",
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
            "{kind='list_view',id='list',content={{kind='text',content='x'}},spacing=4,render=function(item,index) return item end,on_item_added=function() end,on_item_removed=function() end}",
            "{kind='list_view',id='list-animated',content={{kind='text',content='x'}},transition={duration=200,x={from=-180,to=0},onAdded=function() end,onRemoved=function() end}}",
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
            "{kind='button',content='x',style={border={width=-1}}}",
            "{kind='scrollable',content={},direction='diagonal'}",
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
            .load("{kind='text',content='x',color='#ff0000',transition={props={'color'}}}")
            .eval()
            .unwrap();
        let transition = Transition::parse(&node).unwrap().unwrap();
        assert_eq!(transition.color(), Some(color("#ff0000").unwrap()));
        assert_eq!(transition.value("width"), None);

        let node: Table = lua
            .load("{kind='text',content='x',color='#ff0000',transition={props={'width'}}}")
            .eval()
            .unwrap();
        let transition = Transition::parse(&node).unwrap().unwrap();
        assert_eq!(transition.color(), None);
    }

    #[test]
    fn generic_props_build_for_every_kind() {
        let lua = mlua::Lua::new();
        for kind in [
            "{kind='text',content='x'}",
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
    fn list_view_reports_appended_and_removed_items() {
        let lua = mlua::Lua::new();
        lua.load("added = 0; removed = 0").exec().unwrap();
        let ctx = Ctx::new(7);
        let first: Table = lua
            .load(
                "{kind='list_view',id='list-test',content={{kind='text',content='a'},{kind='text',content='b'}},\
                 on_item_added=function() added = added + 1 end,\
                 on_item_removed=function() removed = removed + 1 end}",
            )
            .eval()
            .unwrap();
        element(first, &ctx).unwrap();
        assert_eq!(lua.globals().get::<u32>("added").unwrap(), 2);
        assert_eq!(lua.globals().get::<u32>("removed").unwrap(), 0);

        let second: Table = lua
            .load(
                "{kind='list_view',id='list-test',content={{kind='text',content='a'}},\
                 on_item_added=function() added = added + 1 end,\
                 on_item_removed=function() removed = removed + 1 end}",
            )
            .eval()
            .unwrap();
        element(second, &ctx).unwrap();
        assert_eq!(lua.globals().get::<u32>("added").unwrap(), 2);
        assert_eq!(lua.globals().get::<u32>("removed").unwrap(), 1);
    }

    #[test]
    fn builds_images_and_validates_their_props() {
        let lua = mlua::Lua::new();
        for source in [
            "{kind='image',content='/tmp/x.png'}",
            "{kind='image',content='x.png',content_fit='cover',width=64,height=64}",
        ] {
            assert!(
                element(lua.load(source).eval().unwrap(), &Ctx::new(0)).is_ok(),
                "{source}"
            );
        }
        for source in [
            "{kind='image',content=''}",
            "{kind='image',content='x.png',content_fit='bogus'}",
        ] {
            assert!(
                element(lua.load(source).eval().unwrap(), &Ctx::new(0)).is_err(),
                "{source}"
            );
        }
    }
}
