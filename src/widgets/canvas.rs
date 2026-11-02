use iced::widget::canvas as canvas_widget;
use iced::{Color, Element, Point, Size};
use mlua::{Table, Value};

use crate::runtime::Message;

use super::{color, number};

#[derive(Debug, Clone)]
enum Shape {
    Rect {
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        color: Color,
        stroke: Option<(Color, f32)>,
    },
    Circle {
        x: f32,
        y: f32,
        radius: f32,
        color: Color,
        stroke: Option<(Color, f32)>,
    },
    Line {
        x1: f32,
        y1: f32,
        x2: f32,
        y2: f32,
        color: Color,
        width: f32,
    },
    Path {
        points: Vec<Point>,
        color: Color,
        width: f32,
        fill: bool,
    },
    Text {
        x: f32,
        y: f32,
        content: String,
        size: f32,
        color: Color,
    },
}

struct Drawing {
    shapes: Vec<Shape>,
}

impl canvas_widget::Program<Message> for Drawing {
    type State = ();

    fn draw(
        &self,
        _state: &Self::State,
        renderer: &iced::Renderer,
        _theme: &iced::Theme,
        bounds: iced::Rectangle,
        _cursor: iced::mouse::Cursor,
    ) -> Vec<canvas_widget::Geometry> {
        let mut frame = canvas_widget::Frame::new(renderer, bounds.size());
        for shape in &self.shapes {
            match shape {
                Shape::Rect {
                    x,
                    y,
                    width,
                    height,
                    color,
                    stroke,
                } => {
                    let point = Point::new(*x, *y);
                    let size = Size::new(*width, *height);
                    frame.fill_rectangle(point, size, *color);
                    if let Some((color, width)) = stroke {
                        frame.stroke_rectangle(
                            point,
                            size,
                            canvas_widget::Stroke::default()
                                .with_color(*color)
                                .with_width(*width),
                        );
                    }
                }
                Shape::Circle {
                    x,
                    y,
                    radius,
                    color,
                    stroke,
                } => {
                    let path = canvas_widget::Path::circle(Point::new(*x, *y), *radius);
                    match stroke {
                        Some((color, width)) => frame.stroke(
                            &path,
                            canvas_widget::Stroke::default()
                                .with_color(*color)
                                .with_width(*width),
                        ),
                        None => frame.fill(&path, *color),
                    }
                }
                Shape::Line {
                    x1,
                    y1,
                    x2,
                    y2,
                    color,
                    width,
                } => {
                    let path =
                        canvas_widget::Path::line(Point::new(*x1, *y1), Point::new(*x2, *y2));
                    frame.stroke(
                        &path,
                        canvas_widget::Stroke::default()
                            .with_color(*color)
                            .with_width(*width),
                    );
                }
                Shape::Path {
                    points,
                    color,
                    width,
                    fill,
                } => {
                    let path = canvas_widget::Path::new(|builder| {
                        if let Some(first) = points.first() {
                            builder.move_to(*first);
                            for point in &points[1..] {
                                builder.line_to(*point);
                            }
                            builder.close();
                        }
                    });
                    if *fill {
                        frame.fill(&path, *color);
                    } else {
                        frame.stroke(
                            &path,
                            canvas_widget::Stroke::default()
                                .with_color(*color)
                                .with_width(*width),
                        );
                    }
                }
                Shape::Text {
                    x,
                    y,
                    content,
                    size,
                    color,
                } => {
                    frame.fill_text(canvas_widget::Text {
                        content: content.clone(),
                        position: Point::new(*x, *y),
                        color: *color,
                        size: (*size).into(),
                        ..canvas_widget::Text::default()
                    });
                }
            }
        }
        vec![frame.into_geometry()]
    }
}

fn coordinate(table: &Table, key: &str, default: f32) -> mlua::Result<f32> {
    let value = table.get::<Option<f32>>(key)?.unwrap_or(default);
    if !value.is_finite() {
        return Err(mlua::Error::runtime(format!("canvas {key} must be finite")));
    }
    Ok(value)
}

fn point(value: Value) -> mlua::Result<Point> {
    let Value::Table(table) = value else {
        return Err(mlua::Error::runtime("canvas point must be {x, y}"));
    };
    crate::runtime::keys(&table, &["x", "y"])?;
    Ok(Point::new(
        coordinate(&table, "x", 0.0)?,
        coordinate(&table, "y", 0.0)?,
    ))
}

fn stroke(table: &Table) -> mlua::Result<Option<(Color, f32)>> {
    let Some(border) = table.get::<Option<Table>>("border")? else {
        return Ok(None);
    };
    crate::runtime::keys(&border, &["color", "width"])?;
    let color = super::color(&border.get::<String>("color")?)?;
    let width = border.get::<Option<f32>>("width")?.unwrap_or(1.0);
    if !width.is_finite() || width <= 0.0 {
        return Err(mlua::Error::runtime(
            "canvas border width must be finite and positive",
        ));
    }
    Ok(Some((color, width)))
}

fn shape(node: &Table) -> mlua::Result<Shape> {
    let kind: String = node.get("type")?;
    match kind.as_str() {
        "rect" => {
            crate::runtime::keys(
                node,
                &["type", "x", "y", "width", "height", "color", "border"],
            )?;
            let width = number(node, "width", 0.0)?;
            let height = number(node, "height", 0.0)?;
            if width == 0.0 || height == 0.0 {
                return Err(mlua::Error::runtime(
                    "canvas rect needs positive width and height",
                ));
            }
            Ok(Shape::Rect {
                x: coordinate(node, "x", 0.0)?,
                y: coordinate(node, "y", 0.0)?,
                width,
                height,
                color: color(&node.get::<String>("color")?)?,
                stroke: stroke(node)?,
            })
        }
        "circle" => {
            crate::runtime::keys(node, &["type", "x", "y", "radius", "color", "border"])?;
            let radius = number(node, "radius", 0.0)?;
            if radius == 0.0 {
                return Err(mlua::Error::runtime(
                    "canvas circle needs a positive radius",
                ));
            }
            Ok(Shape::Circle {
                x: coordinate(node, "x", 0.0)?,
                y: coordinate(node, "y", 0.0)?,
                radius,
                color: color(&node.get::<String>("color")?)?,
                stroke: stroke(node)?,
            })
        }
        "line" => {
            crate::runtime::keys(node, &["type", "from", "to", "color", "width"])?;
            let from = point(node.get::<Value>("from")?)?;
            let to = point(node.get::<Value>("to")?)?;
            let width = number(node, "width", 1.0)?;
            if width == 0.0 {
                return Err(mlua::Error::runtime("canvas line needs a positive width"));
            }
            Ok(Shape::Line {
                x1: from.x,
                y1: from.y,
                x2: to.x,
                y2: to.y,
                color: color(&node.get::<String>("color")?)?,
                width,
            })
        }
        "path" => {
            crate::runtime::keys(node, &["type", "points", "color", "width", "fill"])?;
            let points = node
                .get::<Table>("points")?
                .sequence_values::<Value>()
                .map(|value| value.and_then(point))
                .collect::<mlua::Result<Vec<_>>>()?;
            if points.len() < 2 {
                return Err(mlua::Error::runtime(
                    "canvas path needs at least two points",
                ));
            }
            let width = number(node, "width", 1.0)?;
            Ok(Shape::Path {
                points,
                color: color(&node.get::<String>("color")?)?,
                width,
                fill: node.get::<Option<bool>>("fill")?.unwrap_or(false),
            })
        }
        "text" => {
            crate::runtime::keys(node, &["type", "x", "y", "content", "size", "color"])?;
            let size = node.get::<Option<f32>>("size")?.unwrap_or(16.0);
            if !size.is_finite() || size <= 0.0 {
                return Err(mlua::Error::runtime(
                    "canvas text size must be finite and positive",
                ));
            }
            Ok(Shape::Text {
                x: coordinate(node, "x", 0.0)?,
                y: coordinate(node, "y", 0.0)?,
                content: node.get::<String>("content")?,
                size,
                color: color(&node.get::<String>("color")?)?,
            })
        }
        other => Err(mlua::Error::runtime(format!(
            "unknown canvas shape {other:?}; use rect, circle, line, path, or text"
        ))),
    }
}

pub(super) fn canvas(
    node: &Table,
    layout: super::Layout,
) -> mlua::Result<Element<'static, Message>> {
    let shapes = node
        .get::<Table>("content")?
        .sequence_values::<Table>()
        .map(|shape_node| shape_node.and_then(|shape_node| shape(&shape_node)))
        .collect::<mlua::Result<Vec<_>>>()?;
    let mut widget = canvas_widget::Canvas::new(Drawing { shapes });
    if let Some(width) = layout.width {
        widget = widget.width(width);
    }
    if let Some(height) = layout.height {
        widget = widget.height(height);
    }
    Ok(widget.into())
}
