use iced::{Color, Element, Length, widget};
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

fn number(table: &Table, key: &str, default: f32) -> mlua::Result<f32> {
    let value = table.get::<Option<f32>>(key)?.unwrap_or(default);
    if !value.is_finite() || value < 0.0 {
        return Err(mlua::Error::runtime(format!(
            "{key} must be finite and non-negative"
        )));
    }
    Ok(value)
}

#[derive(Clone)]
pub struct Ctx {
    pub generation: u64,
    pub component: Option<(String, u64)>,
}

impl Ctx {
    pub fn new(generation: u64) -> Self {
        Self {
            generation,
            component: None,
        }
    }
}

pub fn element(node: Table, ctx: &Ctx) -> mlua::Result<Element<'static, Message>> {
    let tagged;
    let ctx = match node.get::<Option<String>>("__component_id")? {
        Some(id) => {
            tagged = Ctx {
                generation: ctx.generation,
                component: Some((id, node.get::<u64>("__component_revision")?)),
            };
            &tagged
        }
        None => ctx,
    };
    let kind: String = node.get("kind")?;
    let width = length(node.get("width")?)?;
    let height = length(node.get("height")?)?;
    let padding = number(&node, "padding", 0.0)?;
    let element: Element<'static, Message> = match kind.as_str() {
        "text" => {
            let mut text =
                widget::text(node.get::<String>("content")?).size(number(&node, "size", 16.0)?);
            if let Some(value) = node.get::<Option<String>>("color")? {
                text = text.color(color(&value)?);
            }
            text.into()
        }
        "row" | "column" => {
            let children = node
                .get::<Table>("content")?
                .sequence_values::<Table>()
                .map(|child| element(child?, ctx))
                .collect::<mlua::Result<Vec<_>>>()?;
            let spacing = number(&node, "spacing", 0.0)?;
            if kind == "row" {
                widget::Row::with_children(children).spacing(spacing).into()
            } else {
                widget::Column::with_children(children)
                    .spacing(spacing)
                    .into()
            }
        }
        "container" => element(node.get("content")?, ctx)?,
        "button" => {
            let content = match node.get::<Value>("content")? {
                Value::String(s) => widget::text(s.to_str()?.to_owned()).into(),
                Value::Table(t) => element(t, ctx)?,
                _ => {
                    return Err(mlua::Error::runtime(
                        "button content must be text or an iced element",
                    ));
                }
            };
            let mut button = widget::button(content);
            let event: Value = node.get("on_press")?;
            if !event.is_nil() {
                button =
                    button.on_press(Message::Lua(ctx.generation, ctx.component.clone(), event));
            }
            button.into()
        }
        "space" => widget::Space::new().into(),
        _ => {
            return Err(mlua::Error::runtime(format!(
                "unsupported iced element: {kind}"
            )));
        }
    };
    Ok(widget::container(element)
        .width(width)
        .height(height)
        .padding(padding)
        .into())
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
}
