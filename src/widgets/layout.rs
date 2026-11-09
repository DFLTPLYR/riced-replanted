use iced::Element;
use iced::widget;
use mlua::Table;

use crate::runtime::Message;

use super::enums::{horizontal, vertical};
use super::{Ctx, Layout, children, max_pixels, number};

pub(super) fn row_column(
    node: &Table,
    ctx: &Ctx,
    transition: Option<&super::Transition>,
    layout: Layout,
    kind: &str,
) -> mlua::Result<Element<'static, Message>> {
    let children = children(node, ctx)?;
    let spacing = match transition.and_then(|t| t.value("spacing")) {
        Some(spacing) => spacing,
        None => number(node, "spacing", 0.0)?,
    };
    let clip = node.get::<Option<bool>>("clip")?.unwrap_or(false);
    Ok(if kind == "row" {
        let mut row = widget::Row::with_children(children).spacing(spacing);
        if let Some(width) = layout.width {
            row = row.width(width);
        }
        if let Some(height) = layout.height {
            row = row.height(height);
        }
        if let Some(align) = node.get::<Option<String>>("align_y")? {
            row = row.align_y(vertical(&align)?);
        }
        row.clip(clip).into()
    } else {
        let mut column = widget::Column::with_children(children).spacing(spacing);
        if let Some(width) = layout.width {
            column = column.width(width);
        }
        if let Some(height) = layout.height {
            column = column.height(height);
        }
        if let Some(align) = node.get::<Option<String>>("align_x")? {
            column = column.align_x(horizontal(&align)?);
        }
        column.clip(clip).into()
    })
}

pub(super) fn grid(
    node: &Table,
    ctx: &Ctx,
    transition: Option<&super::Transition>,
) -> mlua::Result<Element<'static, Message>> {
    let columns = node.get::<Option<u32>>("columns")?.unwrap_or(3);
    if columns == 0 {
        return Err(mlua::Error::runtime("grid columns must be at least 1"));
    }
    let mut grid = widget::Grid::with_children(children(node, ctx)?).columns(columns as usize);
    let spacing = match transition.and_then(|transition| transition.value("spacing")) {
        Some(spacing) => spacing,
        None => number(node, "spacing", 0.0)?,
    };
    grid = grid.spacing(spacing);
    if let Some(max) = max_pixels(node, "fluid")? {
        grid = grid.fluid(max);
    }
    Ok(grid.into())
}

pub(super) fn space(layout: Layout) -> Element<'static, Message> {
    let mut space = widget::Space::new();
    if let Some(width) = layout.width {
        space = space.width(width);
    }
    if let Some(height) = layout.height {
        space = space.height(height);
    }
    space.into()
}
