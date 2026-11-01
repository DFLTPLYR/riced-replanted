use iced_exwlshell::reexport::{
    Anchor, BlurOption, BlurRegion, Extent, KeyboardInteractivity, Layer, LayerSize,
    NewLayerShellSettings, OutputOption,
};
use mlua::{Table, Value};

use crate::runtime::keys;

fn sequence(table: &Table) -> mlua::Result<()> {
    let mut count = 0;
    for pair in table.clone().pairs::<Value, Value>() {
        if !matches!(pair?.0, Value::Integer(n) if n >= 1 && n <= table.raw_len() as i64) {
            return Err(mlua::Error::runtime(
                "expected a dense list without named keys",
            ));
        }
        count += 1;
    }
    if count != table.raw_len() {
        return Err(mlua::Error::runtime("expected a dense list without holes"));
    }
    Ok(())
}

pub fn nodes(node: Table) -> mlua::Result<Vec<Table>> {
    match node.get::<Option<String>>("kind")?.as_deref() {
        Some("layer") => Ok(vec![node]),
        Some(_) => Ok(Vec::new()),
        None => {
            sequence(&node)?;
            let nodes = node
                .sequence_values::<Table>()
                .collect::<mlua::Result<Vec<_>>>()?;
            if nodes.is_empty()
                || nodes
                    .iter()
                    .any(|node| !matches!(node.get::<String>("kind").as_deref(), Ok("layer")))
            {
                return Err(mlua::Error::runtime(
                    "view must return an iced element, iced.layer, or a nonempty list of layers",
                ));
            }
            Ok(nodes)
        }
    }
}

fn extent(value: Value) -> mlua::Result<Extent> {
    match value {
        Value::String(s) if s.to_str()? == "fill" => Ok(Extent::Fill),
        Value::Integer(n) if n > 0 && n <= i32::MAX as i64 => Ok(Extent::px(n as u32)),
        _ => Err(mlua::Error::runtime(
            "layer size must be 'fill' or positive integer pixels (at most 2147483647)",
        )),
    }
}

// Layer identity survives reordering; without an explicit id it follows list position.
pub fn id(settings: &Table, index: usize) -> mlua::Result<String> {
    let id = settings
        .get::<Option<String>>("id")?
        .unwrap_or_else(|| format!("#{}", index + 1));
    if id.is_empty() {
        return Err(mlua::Error::runtime("layer id must not be empty"));
    }
    Ok(id)
}

pub fn settings(table: Table) -> mlua::Result<NewLayerShellSettings> {
    keys(
        &table,
        &[
            "size",
            "layer",
            "anchor",
            "exclusive_zone",
            "margin",
            "keyboard_interactivity",
            "output_option",
            "events_transparent",
            "namespace",
            "blur_option",
            "id",
        ],
    )?;
    let mut settings = NewLayerShellSettings::default();
    if let Some(size) = table.get::<Option<Table>>("size")? {
        keys(&size, &["width", "height"])?;
        settings.size = LayerSize {
            width: extent(size.get("width")?)?,
            height: extent(size.get("height")?)?,
        };
    }
    if let Some(layer) = table.get::<Option<String>>("layer")? {
        settings.layer = match layer.as_str() {
            "background" => Layer::Background,
            "bottom" => Layer::Bottom,
            "top" => Layer::Top,
            "overlay" => Layer::Overlay,
            _ => return Err(mlua::Error::runtime(format!("unknown layer: {layer}"))),
        };
    }
    if let Some(anchor) = table.get::<Option<Table>>("anchor")? {
        sequence(&anchor)?;
        settings.anchor = Anchor::empty();
        for edge in anchor.sequence_values::<String>() {
            settings.anchor |= match edge?.as_str() {
                "top" => Anchor::Top,
                "bottom" => Anchor::Bottom,
                "left" => Anchor::Left,
                "right" => Anchor::Right,
                edge => return Err(mlua::Error::runtime(format!("unknown anchor: {edge}"))),
            };
        }
    }
    if !settings.size.missing_edges(settings.anchor).is_empty() {
        return Err(mlua::Error::runtime(
            "fill size requires both opposite anchors on that axis",
        ));
    }
    settings.exclusive_zone = table.get("exclusive_zone")?;
    if settings.exclusive_zone.is_some_and(|zone| zone < -1) {
        return Err(mlua::Error::runtime("exclusive_zone must be -1 or greater"));
    }
    if let Some(margin) = table.get::<Option<Table>>("margin")? {
        sequence(&margin)?;
        if margin.raw_len() != 4 {
            return Err(mlua::Error::runtime(
                "margin must contain top, right, bottom, left",
            ));
        }
        settings.margin = Some((
            margin.get(1)?,
            margin.get(2)?,
            margin.get(3)?,
            margin.get(4)?,
        ));
    }
    if let Some(mode) = table.get::<Option<String>>("keyboard_interactivity")? {
        settings.keyboard_interactivity = match mode.as_str() {
            "none" => KeyboardInteractivity::None,
            "on_demand" => KeyboardInteractivity::OnDemand,
            "exclusive" => KeyboardInteractivity::Exclusive,
            _ => {
                return Err(mlua::Error::runtime(format!(
                    "unknown keyboard_interactivity: {mode}"
                )));
            }
        };
    }
    settings.output_option = match table.get::<Value>("output_option")? {
        Value::Nil => OutputOption::Active,
        Value::String(s) => match s.to_str()?.as_ref() {
            "active" => OutputOption::Active,
            "last_output" => OutputOption::LastOutput,
            name if !name.is_empty() && !name.contains('\0') => {
                OutputOption::OutputName(name.to_owned())
            }
            _ => {
                return Err(mlua::Error::runtime(
                    "output name must be nonempty and contain no NUL",
                ));
            }
        },
        Value::Integer(n) if n > 0 && n <= u32::MAX as i64 => OutputOption::GlobalName(n as u32),
        _ => {
            return Err(mlua::Error::runtime(
                "output_option must be 'active', 'last_output', an output name, or a positive global ID",
            ));
        }
    };
    settings.events_transparent = table
        .get::<Option<bool>>("events_transparent")?
        .unwrap_or(false);
    settings.namespace = table.get("namespace")?;
    if settings
        .namespace
        .as_ref()
        .is_some_and(|name| name.is_empty() || name.contains('\0'))
    {
        return Err(mlua::Error::runtime(
            "layer namespace must be nonempty and contain no NUL",
        ));
    }
    settings.blur_option = match table.get::<Value>("blur_option")? {
        Value::Nil => BlurOption::None,
        Value::String(s) if s.to_str()? == "none" => BlurOption::None,
        Value::String(s) if s.to_str()? == "full" => BlurOption::FullRegion,
        Value::Table(regions) => {
            sequence(&regions)?;
            let mut result = Vec::new();
            for region in regions.sequence_values::<Table>() {
                let region = region?;
                keys(&region, &["x", "y", "width", "height"])?;
                let region = BlurRegion {
                    x: region.get("x")?,
                    y: region.get("y")?,
                    width: region.get("width")?,
                    height: region.get("height")?,
                };
                if region.width <= 0 || region.height <= 0 {
                    return Err(mlua::Error::runtime(
                        "blur region width and height must be positive",
                    ));
                }
                result.push(region);
            }
            BlurOption::Region(result)
        }
        _ => {
            return Err(mlua::Error::runtime(
                "blur_option must be 'none', 'full', or a list of regions",
            ));
        }
    };
    Ok(settings)
}

#[cfg(test)]
mod tests {
    use super::*;
    use mlua::Lua;

    #[test]
    fn helpers_produce_the_existing_layer_settings() {
        let lua = Lua::new();
        lua.load(include_str!("api.lua")).exec().unwrap();
        let helpers = lua
            .load(
                r#"{
            size = iced.Size(iced.Extent.fill, 32),
            layer = iced.Layer.bottom,
            anchor = {'top', 'left', 'right'},
            keyboard_interactivity = iced.KeyboardInteractivity.none,
            output_option = iced.Output.last_output,
            blur_option = iced.Blur.full
        }"#,
            )
            .eval()
            .unwrap();
        let plain = lua
            .load(
                r#"{
            size = {width='fill', height=32}, layer='bottom',
            anchor = {'top', 'left', 'right'},
            keyboard_interactivity='none', output_option='last_output', blur_option='full'
        }"#,
            )
            .eval()
            .unwrap();
        assert_eq!(settings(helpers).unwrap(), settings(plain).unwrap());
        let weighted = lua
            .load("{size=iced.Size(iced.Length.fill_portion(3), 32)}")
            .eval()
            .unwrap();
        assert!(settings(weighted).is_err());
    }

    #[test]
    fn omitted_settings_use_native_defaults() {
        let lua = Lua::new();
        assert_eq!(
            settings(lua.create_table().unwrap()).unwrap(),
            NewLayerShellSettings::default()
        );
    }

    #[test]
    fn parses_every_layer_setting() {
        let lua = Lua::new();
        let table = lua
            .load(
                r#"{
            size = {width = 'fill', height = 32},
            layer = 'bottom', anchor = {'top', 'left', 'right'},
            exclusive_zone = 32, margin = {1, 2, 3, -4},
            keyboard_interactivity = 'none', output_option = 'DP-1',
            events_transparent = true, namespace = 'panel',
            blur_option = {{x = -1, y = 2, width = 300, height = 32}}
        }"#,
            )
            .eval()
            .unwrap();
        assert_eq!(
            settings(table).unwrap(),
            NewLayerShellSettings {
                size: LayerSize::fill_width(32),
                layer: Layer::Bottom,
                anchor: Anchor::Top | Anchor::Left | Anchor::Right,
                exclusive_zone: Some(32),
                margin: Some((1, 2, 3, -4)),
                keyboard_interactivity: KeyboardInteractivity::None,
                output_option: OutputOption::OutputName("DP-1".into()),
                events_transparent: true,
                namespace: Some("panel".into()),
                blur_option: BlurOption::Region(vec![BlurRegion {
                    x: -1,
                    y: 2,
                    width: 300,
                    height: 32
                }]),
            }
        );
    }

    #[test]
    fn parses_remaining_output_keyboard_and_blur_modes() {
        let lua = Lua::new();
        for (source, output, keyboard, blur) in [
            (
                "{output_option='last_output', keyboard_interactivity='exclusive', blur_option='full'}",
                OutputOption::LastOutput,
                KeyboardInteractivity::Exclusive,
                BlurOption::FullRegion,
            ),
            (
                "{output_option=42, keyboard_interactivity='on_demand', blur_option='none'}",
                OutputOption::GlobalName(42),
                KeyboardInteractivity::OnDemand,
                BlurOption::None,
            ),
        ] {
            let parsed = settings(lua.load(source).eval().unwrap()).unwrap();
            assert_eq!(
                (
                    parsed.output_option,
                    parsed.keyboard_interactivity,
                    parsed.blur_option
                ),
                (output, keyboard, blur)
            );
        }
    }

    #[test]
    fn rejects_invalid_settings_before_sending_wayland_requests() {
        let lua = Lua::new();
        for source in [
            "{typo=true}",
            "{size={width=0,height=20}}",
            "{size={width=-1,height=20}}",
            "{size={width=1.5,height=20}}",
            "{size={width=2147483648,height=20}}",
            "{size={width=20}}",
            "{size={width='fill',height=20},anchor={'top'}}",
            "{layer='typo'}",
            "{anchor={'typo'}}",
            "{anchor={top=true}}",
            "{exclusive_zone=-2}",
            "{margin={1,2,3}}",
            "{keyboard_interactivity='typo'}",
            "{output_option=0}",
            "{output_option=''}",
            "{namespace=''}",
            "{namespace='a\\0b'}",
            "{blur_option='typo'}",
            "{blur_option={{x=0,y=0,width=0,height=10}}}",
            "{blur_option={full=true}}",
            "{anchor={[1]='top',[3]='left'}}",
        ] {
            assert!(
                settings(lua.load(source).eval().unwrap()).is_err(),
                "accepted {source}"
            );
        }
    }
}
