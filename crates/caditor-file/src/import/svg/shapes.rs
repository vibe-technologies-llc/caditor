use std::f64::consts::{FRAC_PI_2, PI, TAU};

use caditor_geometry::{Point2, Vector2};

use crate::import::svg::{
    path::{Outline, path_outline},
    sizing::{Fit, ViewBox},
    syntax::{Length, Matrix, numbers},
    xml::Node,
};

#[derive(Debug, Clone, Copy)]
pub(super) enum Axis {
    Horizontal,
    Vertical,
    Diagonal,
}

impl Axis {
    pub fn reference(self, viewport: Vector2) -> f64 {
        match self {
            Self::Horizontal => viewport.x,
            Self::Vertical => viewport.y,
            Self::Diagonal => (viewport.length_squared() / 2.0).sqrt(),
        }
    }
}

pub(super) fn length(node: Node<'_, '_>, name: &str, axis: Axis, viewport: Vector2) -> Option<f64> {
    node.attribute(name)
        .and_then(Length::parse)
        .map(|length| length.pixels(axis.reference(viewport)))
        .filter(|value| value.is_finite())
}

pub(super) fn nested_viewport(node: Node<'_, '_>, parent: Vector2) -> (Matrix, Vector2) {
    let x = length(node, "x", Axis::Horizontal, parent).unwrap_or(0.0);
    let y = length(node, "y", Axis::Vertical, parent).unwrap_or(0.0);
    let width = length(node, "width", Axis::Horizontal, parent).unwrap_or(parent.x);
    let height = length(node, "height", Axis::Vertical, parent).unwrap_or(parent.y);
    let (fitted, viewport) = fitted(node, Vector2::new(width, height));
    (fitted.then(&Matrix::translation(x, y)), viewport)
}

pub(super) fn symbol_viewport(
    used: Node<'_, '_>,
    symbol: Node<'_, '_>,
    parent: Vector2,
) -> Option<(Matrix, Vector2)> {
    let side = |name: &str, axis: Axis, fallback: f64| {
        length(used, name, axis, parent)
            .or_else(|| length(symbol, name, axis, parent))
            .unwrap_or(fallback)
    };
    let size = Vector2::new(
        side("width", Axis::Horizontal, parent.x),
        side("height", Axis::Vertical, parent.y),
    );
    let drawn = size.min_element() > 0.0;
    if !drawn {
        return None;
    }
    let x = length(symbol, "x", Axis::Horizontal, parent).unwrap_or(0.0);
    let y = length(symbol, "y", Axis::Vertical, parent).unwrap_or(0.0);
    let (fitted, viewport) = fitted(symbol, size);
    Some((fitted.then(&Matrix::translation(x, y)), viewport))
}

pub(super) fn fitted(node: Node<'_, '_>, size: Vector2) -> (Matrix, Vector2) {
    let Some(view_box) = ViewBox::of(node) else {
        return (Matrix::IDENTITY, size);
    };
    let fit = Fit::of(node);
    let factors = fit.scale(size / view_box.size);
    let spare = size - view_box.size * factors;
    let shift = spare * alignment(node);
    let matrix = Matrix::translation(-view_box.origin.x, -view_box.origin.y)
        .then(&Matrix::scale(factors.x, factors.y))
        .then(&Matrix::translation(shift.x, shift.y));
    (matrix, view_box.size)
}

fn alignment(node: Node<'_, '_>) -> Vector2 {
    let align = node
        .attribute("preserveAspectRatio")
        .unwrap_or_default()
        .trim();
    let along = |low: &str, high: &str| {
        if align.contains(low) {
            0.0
        } else if align.contains(high) {
            1.0
        } else {
            0.5
        }
    };
    Vector2::new(along("xMin", "xMax"), along("YMin", "YMax"))
}

pub(super) fn outline_of(node: Node<'_, '_>, viewport: Vector2) -> Outline {
    let get = |name: &str, axis: Axis| length(node, name, axis, viewport);
    let at = |x: &str, y: &str| {
        Point2::new(
            get(x, Axis::Horizontal).unwrap_or(0.0),
            get(y, Axis::Vertical).unwrap_or(0.0),
        )
    };
    let mut outline = Outline::default();
    match node.name() {
        "path" => return path_outline(node.attribute("d").unwrap_or_default()),
        "line" => {
            let from = at("x1", "y1");
            outline.move_to(from);
            outline.line_through(from, at("x2", "y2"));
        }
        "circle" => {
            if let Some(radius) = get("r", Axis::Diagonal).filter(|radius| *radius > 0.0) {
                outline.ellipse(
                    at("cx", "cy"),
                    Vector2::X * radius,
                    Vector2::Y * radius,
                    0.0,
                    TAU,
                );
            }
        }
        "ellipse" => {
            let rx = get("rx", Axis::Horizontal);
            let ry = get("ry", Axis::Vertical);
            if let (Some(rx), Some(ry)) = (rx.or(ry), ry.or(rx))
                && rx > 0.0
                && ry > 0.0
            {
                outline.ellipse(at("cx", "cy"), Vector2::X * rx, Vector2::Y * ry, 0.0, TAU);
            }
        }
        "rect" => rectangle(&mut outline, node, viewport),
        name => points(&mut outline, node, name == "polygon"),
    }
    outline
}

fn rectangle(outline: &mut Outline, node: Node<'_, '_>, viewport: Vector2) {
    let get = |name: &str, axis: Axis| length(node, name, axis, viewport);
    let origin = Point2::new(
        get("x", Axis::Horizontal).unwrap_or(0.0),
        get("y", Axis::Vertical).unwrap_or(0.0),
    );
    let (Some(width), Some(height)) = (
        get("width", Axis::Horizontal),
        get("height", Axis::Vertical),
    ) else {
        return;
    };
    if width <= 0.0 || height <= 0.0 {
        return;
    }
    let rx = get("rx", Axis::Horizontal).filter(|radius| *radius >= 0.0);
    let ry = get("ry", Axis::Vertical).filter(|radius| *radius >= 0.0);
    let radii = Vector2::new(
        rx.or(ry).unwrap_or(0.0).min(width / 2.0),
        ry.or(rx).unwrap_or(0.0).min(height / 2.0),
    );
    let size = Vector2::new(width, height);
    let corner = |x: f64, y: f64| origin + Vector2::new(x, y);
    if radii.x <= 0.0 || radii.y <= 0.0 {
        let corners = [
            corner(0.0, 0.0),
            corner(size.x, 0.0),
            corner(size.x, size.y),
            corner(0.0, size.y),
        ];
        for (index, from) in corners.iter().enumerate() {
            if let Some(to) = corners.get((index + 1) % corners.len()) {
                outline.line(*from, *to);
            }
        }
        return;
    }
    let (rx, ry) = (radii.x, radii.y);
    let rounds = [
        (corner(size.x - rx, ry), -FRAC_PI_2),
        (corner(size.x - rx, size.y - ry), 0.0),
        (corner(rx, size.y - ry), FRAC_PI_2),
        (corner(rx, ry), PI),
    ];
    let edges = [
        (corner(rx, 0.0), corner(size.x - rx, 0.0)),
        (corner(size.x, ry), corner(size.x, size.y - ry)),
        (corner(size.x - rx, size.y), corner(rx, size.y)),
        (corner(0.0, size.y - ry), corner(0.0, ry)),
    ];
    for ((from, to), (center, start)) in edges.into_iter().zip(rounds) {
        outline.line(from, to);
        outline.ellipse(center, Vector2::X * rx, Vector2::Y * ry, start, FRAC_PI_2);
    }
}

fn points(outline: &mut Outline, node: Node<'_, '_>, closed: bool) {
    let (values, complete) = numbers(node.attribute("points").unwrap_or_default());
    let (pairs, remainder) = values.as_chunks::<2>();
    outline.damaged = !complete || !remainder.is_empty();
    let corners: Vec<Point2> = pairs.iter().map(|[x, y]| Point2::new(*x, *y)).collect();
    if let Some(first) = corners.first() {
        outline.move_to(*first);
    }
    for pair in corners.windows(2) {
        if let [from, to] = pair {
            outline.line_through(*from, *to);
        }
    }
    if closed
        && corners.len() > 2
        && let (Some(first), Some(last)) = (corners.first(), corners.last())
    {
        outline.line_through(*last, *first);
    }
}
