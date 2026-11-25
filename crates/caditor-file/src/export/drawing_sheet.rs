use std::{f64::consts::TAU, path::Path};

use caditor_document::{
    Annotation, CancelToken, PlacedView, SheetAnnotation, SheetMark, SheetMeasure, SheetPlan,
    SheetText, TextAnchor, ViewLines, drawn_hatching,
};
use caditor_geometry::{Point2, Vector2};
use caditor_kernel::{
    Curve2, Interval, LINEAR_RESOLUTION, SamplingTolerance, ViewPiece, Visibility,
};

use super::{
    ExportError,
    figure::{Anchor, Dimension, Ellipse, Figure, Layer, Measure, Shape, Spline, Text},
    pdf,
};
use crate::{reason::WriteFailure, save::write_atomically};

const SHEET_CHORD: f64 = 0.01;
const SHEET_ANGLE: f64 = 2.0 * std::f64::consts::PI / 180.0;

#[derive(Clone, Copy, Debug, Default, Hash, PartialEq, Eq)]
pub enum SheetFormat {
    #[default]
    Pdf,
    Svg,
    Dxf,
}

impl SheetFormat {
    pub const ALL: [Self; 3] = [Self::Pdf, Self::Svg, Self::Dxf];

    pub fn extension(self) -> &'static str {
        match self {
            Self::Pdf => "pdf",
            Self::Svg => "svg",
            Self::Dxf => "dxf",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Pdf => "PDF",
            Self::Svg => "SVG",
            Self::Dxf => "DXF",
        }
    }

    pub fn of(path: &Path) -> Option<Self> {
        let extension = path.extension()?;
        Self::ALL
            .into_iter()
            .find(|format| extension.eq_ignore_ascii_case(format.extension()))
    }
}

#[derive(Clone, Copy, Debug)]
pub struct SheetView<'a> {
    pub placed: PlacedView,
    pub lines: &'a ViewLines,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SheetExported {
    pub views: usize,
    pub visible: usize,
    pub hidden: usize,
    pub hatching: usize,
    pub approximated: usize,
    pub dimensions: usize,
    pub notes: usize,
    pub centres: usize,
    pub callouts: usize,
    pub left_out: usize,
}

fn sheet_tolerance(scale: f64) -> SamplingTolerance {
    let chord = SHEET_CHORD / scale.max(f64::MIN_POSITIVE);
    SamplingTolerance::new(chord, SHEET_ANGLE).unwrap_or_else(|| SamplingTolerance::for_extent(1.0))
}

fn sampled(piece: &ViewPiece, placed: &PlacedView) -> Vec<Point2> {
    piece
        .curve
        .sample(piece.range, &sheet_tolerance(placed.scale))
        .into_iter()
        .map(|sample| placed.on_sheet(sample.point))
        .collect()
}

fn is_full_turn(range: Interval) -> bool {
    range.length() >= TAU - LINEAR_RESOLUTION
}

fn turned(axis: Vector2) -> f64 {
    axis.y.atan2(axis.x)
}

fn counter_clockwise_range(range: Interval, counter_clockwise: bool) -> (f64, f64) {
    if counter_clockwise {
        (range.start(), range.end())
    } else {
        (-range.end(), -range.start())
    }
}

fn piece_shape(piece: &ViewPiece, placed: &PlacedView) -> (Shape, bool) {
    let scale = placed.scale;
    let exact = match &piece.curve {
        Curve2::Line(_) => Some(Shape::Line(
            placed.on_sheet(piece.curve.point(piece.range.start())),
            placed.on_sheet(piece.curve.point(piece.range.end())),
        )),
        Curve2::Circle(circle) => {
            let center = placed.on_sheet(circle.center());
            let radius = circle.radius() * scale;
            if is_full_turn(piece.range) {
                Some(Shape::Circle { center, radius })
            } else {
                let (start, end) =
                    counter_clockwise_range(piece.range, circle.is_counter_clockwise());
                let turn = turned(circle.x_axis());
                Some(Shape::Arc {
                    center,
                    radius,
                    start: turn + start,
                    end: turn + end,
                })
            }
        }
        Curve2::Ellipse(ellipse) => {
            let (start, end) = if is_full_turn(piece.range) {
                (0.0, TAU)
            } else {
                counter_clockwise_range(piece.range, ellipse.is_counter_clockwise())
            };
            let along_x = ellipse.major_radius() * scale;
            let along_y = ellipse.minor_radius() * scale;
            let x_axis = ellipse.x_axis();
            let full = is_full_turn(piece.range);
            Some(Shape::Ellipse(if along_x >= along_y {
                Ellipse {
                    center: placed.on_sheet(ellipse.center()),
                    major: x_axis * along_x,
                    ratio: along_y / along_x.max(f64::MIN_POSITIVE),
                    start,
                    end,
                }
            } else {
                let quarter = std::f64::consts::FRAC_PI_2;
                let (start, end) = if full {
                    (0.0, TAU)
                } else {
                    (start - quarter, end - quarter)
                };
                Ellipse {
                    center: placed.on_sheet(ellipse.center()),
                    major: x_axis.perp() * along_y,
                    ratio: along_x / along_y,
                    start,
                    end,
                }
            }))
        }
        Curve2::BSpline(spline) => spline.restricted(piece.range).map(|restricted| {
            Shape::Spline(Spline {
                degree: restricted.degree(),
                knots: restricted.knots().to_vec(),
                control_points: restricted
                    .control_points()
                    .iter()
                    .map(|point| placed.on_sheet(*point))
                    .collect(),
                weights: restricted.weights().map(<[f64]>::to_vec),
                polyline: sampled(piece, placed),
            })
        }),
        _ => None,
    };
    match exact {
        Some(shape) if piece.exact => (shape, true),
        Some(Shape::Spline(_)) | None => (Shape::Polyline(sampled(piece, placed)), false),
        Some(shape) => (shape, false),
    }
}

fn mark_shape(mark: &SheetMark) -> Shape {
    match *mark {
        SheetMark::Line(from, to) => Shape::Line(from, to),
        SheetMark::Circle { center, radius } => Shape::Circle { center, radius },
        SheetMark::Arc {
            center,
            radius,
            start,
            end,
        } => Shape::Arc {
            center,
            radius,
            start,
            end,
        },
    }
}

fn text_shape(text: &SheetText) -> Text {
    Text {
        at: text.at,
        height: text.height,
        angle: text.angle,
        content: text.content.clone(),
        anchor: match text.anchor {
            TextAnchor::Start => Anchor::Start,
            TextAnchor::Middle => Anchor::Middle,
        },
    }
}

fn measure_of(measure: SheetMeasure) -> Measure {
    match measure {
        SheetMeasure::Linear {
            first,
            second,
            line,
            angle,
        } => Measure::Linear {
            first,
            second,
            line,
            angle,
        },
        SheetMeasure::Angular {
            vertex,
            first,
            second,
            arc,
        } => Measure::Angular {
            vertex,
            first,
            second,
            arc,
        },
        SheetMeasure::Radius { center, on_curve } => Measure::Radius { center, on_curve },
        SheetMeasure::Diameter { near, far } => Measure::Diameter { near, far },
    }
}

fn annotation_shapes(figure: &mut Figure, laid: &SheetAnnotation, exported: &mut SheetExported) {
    let lost = laid.problem.is_some();

    if lost {
        exported.left_out += 1;
    }

    let texts = || laid.texts.iter().map(text_shape);
    let marks = || laid.marks.iter().map(mark_shape);

    match (laid.annotation, laid.measure) {
        (Annotation::Dimension(_), Some(measure)) if !lost => {
            let mut written = texts();

            let Some(text) = written.next() else {
                return;
            };

            exported.dimensions += 1;
            figure.push(
                Layer::Annotation,
                Shape::Dimension(Box::new(Dimension {
                    measure: measure_of(measure),
                    marks: marks().collect(),
                    text,
                    more: written.collect(),
                })),
            );
        }
        (Annotation::Note(_), _) => {
            exported.notes += 1;
            for shape in marks().chain(texts().map(Shape::Text)) {
                figure.push(Layer::Annotation, shape);
            }
        }
        (Annotation::Centre(_), _) if !lost => {
            exported.centres += 1;
            for shape in marks() {
                figure.push(Layer::Centre, shape);
            }
        }
        (Annotation::Callout(_), _) if !lost => {
            exported.callouts += 1;
            for shape in marks().chain(texts().map(Shape::Text)) {
                figure.push(Layer::Annotation, shape);
            }
        }
        (Annotation::Dimension(_) | Annotation::Centre(_) | Annotation::Callout(_), _) => {}
    }
}

pub(super) fn sheet_figure(
    plan: &SheetPlan,
    views: &[SheetView<'_>],
    hidden_lines: bool,
) -> (Figure, SheetExported) {
    let mut figure = Figure::default();
    let mut exported = SheetExported::default();

    for mark in &plan.marks {
        figure.push(Layer::Frame, mark_shape(mark));
    }
    for text in &plan.texts {
        figure.push(Layer::Frame, Shape::Text(text_shape(text)));
    }
    for line in &plan.cutting {
        figure.push(Layer::Cutting, Shape::Line(line.chain.0, line.chain.1));
        for (from, to) in line.ends {
            figure.push(Layer::Visible, Shape::Line(from, to));
        }
    }
    for view in views {
        exported.views += 1;
        for (from, to) in drawn_hatching(view.lines, &view.placed) {
            exported.hatching += 1;
            figure.push(Layer::Hatching, Shape::Line(from, to));
        }
        for piece in view.lines.drawn_pieces(hidden_lines) {
            let (shape, exact) = piece_shape(piece, &view.placed);
            let layer = match piece.visibility {
                Visibility::Visible => {
                    exported.visible += 1;
                    Layer::Visible
                }
                Visibility::Hidden => {
                    exported.hidden += 1;
                    Layer::Hidden
                }
            };

            if !exact {
                exported.approximated += 1;
            }
            figure.push(layer, shape);
        }
    }
    for laid in &plan.annotations {
        annotation_shapes(&mut figure, laid, &mut exported);
    }
    (figure, exported)
}

pub fn export_sheet(
    path: &Path,
    plan: &SheetPlan,
    views: &[SheetView<'_>],
    hidden_lines: bool,
    format: SheetFormat,
    cancel: &CancelToken,
) -> Result<SheetExported, ExportError> {
    let (figure, exported) = sheet_figure(plan, views, hidden_lines);

    if cancel.is_cancelled() {
        return Err(ExportError::Cancelled);
    }

    let bytes = match format {
        SheetFormat::Dxf => super::dxf::encode(&figure).into_bytes(),
        SheetFormat::Svg => super::svg::encode_page(&figure, plan.width, plan.height)?.into_bytes(),
        SheetFormat::Pdf => pdf::encode(&figure, plan.width, plan.height)?,
    };

    if cancel.is_cancelled() {
        return Err(ExportError::Cancelled);
    }
    write_atomically(path, &bytes)
        .map_err(|error| ExportError::Writing(WriteFailure::of(&error)))?;
    Ok(exported)
}
