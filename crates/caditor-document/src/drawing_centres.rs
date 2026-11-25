use caditor_geometry::{Point2, Vector2};
use caditor_kernel::{Curve2, EdgeName, FaceId, FaceName, ViewSource};

use crate::{
    drawing_dimensions::{
        AnnotationProblem, DrawnLines, SheetAnnotation, measured_lines_of, nearest_piece, seen_edge,
    },
    drawing_sheet::{PlacedView, SheetMark, SheetPlan},
    drawing_views::ViewLines,
    drawings::{Annotation, CentreMark, CentreTarget, Drawing, DrawnView},
};

pub const CENTRE_OVERSHOOT: f64 = 2.0;
const AXIS_SINE: f64 = 1e-6;
const SAME_PLACE: f64 = 1e-6;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CentreGeometry {
    Mark { centre: Point2, radius: f64 },
    Line { from: Point2, to: Point2 },
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Axis {
    from: Point2,
    to: Point2,
    along: Vector2,
    offset: f64,
}

impl Axis {
    fn length(&self) -> f64 {
        self.from.distance(self.to)
    }

    fn same_line(&self, other: &Self) -> bool {
        let slack = SAME_PLACE * (1.0 + self.offset.abs().max(other.offset.abs()));
        let parallel = self.along.perp_dot(other.along).abs() <= AXIS_SINE;
        parallel && (other.from - self.from).perp_dot(self.along).abs() <= slack
    }
}

fn outline_pieces(
    lines: &ViewLines,
    face: FaceName,
) -> Result<Vec<(Point2, Point2)>, AnnotationProblem> {
    let mut owner: Option<(usize, FaceId)> = None;
    let mut segments = Vec::new();

    for piece in &lines.pieces {
        let ViewSource::Silhouette {
            solid,
            face: id,
            name,
        } = piece.source
        else {
            continue;
        };

        if name != face {
            continue;
        }
        match owner {
            None => owner = Some((solid, id)),
            Some(kept) if kept == (solid, id) => {}
            Some(_) => return Err(AnnotationProblem::EdgeAmbiguous),
        }

        let Curve2::Line(_) = piece.curve else {
            return Err(AnnotationProblem::NotCylinder);
        };

        segments.push((
            piece.curve.point(piece.range.start()),
            piece.curve.point(piece.range.end()),
        ));
    }
    if segments.is_empty() {
        return Err(AnnotationProblem::OutlineGone);
    }
    Ok(segments)
}

fn cylinder_axis(lines: &ViewLines, face: FaceName) -> Result<Axis, AnnotationProblem> {
    let segments = outline_pieces(lines, face)?;

    let along = segments
        .iter()
        .find_map(|(start, end)| (*end - *start).try_normalize())
        .ok_or(AnnotationProblem::Degenerate)?;
    let across = along.perp();
    let ends: Vec<Point2> = segments
        .iter()
        .flat_map(|(start, end)| [*start, *end])
        .collect();

    for (start, end) in &segments {
        if let Some(direction) = (*end - *start).try_normalize()
            && direction.perp_dot(along).abs() > AXIS_SINE
        {
            return Err(AnnotationProblem::NotCylinder);
        }
    }

    let offsets: Vec<f64> = ends.iter().map(|end| end.dot(across)).collect();
    let low = offsets.iter().copied().fold(f64::INFINITY, f64::min);
    let high = offsets.iter().copied().fold(f64::NEG_INFINITY, f64::max);

    let slack = SAME_PLACE * (1.0 + low.abs().max(high.abs()));

    let two_lines = high - low > slack
        && offsets
            .iter()
            .all(|offset| (offset - low).abs() <= slack || (offset - high).abs() <= slack);

    if !two_lines {
        return Err(AnnotationProblem::NotCylinder);
    }

    let reach: Vec<f64> = ends.iter().map(|end| end.dot(along)).collect();
    let first = reach.iter().copied().fold(f64::INFINITY, f64::min);
    let last = reach.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let offset = (low + high) / 2.0;
    let on_axis = |at: f64| Point2::ZERO + along * at + across * offset;

    if last - first <= SAME_PLACE {
        return Err(AnnotationProblem::Degenerate);
    }
    Ok(Axis {
        from: on_axis(first),
        to: on_axis(last),
        along,
        offset,
    })
}

pub fn centre_geometry(
    target: CentreTarget,
    lines: &ViewLines,
) -> Result<CentreGeometry, AnnotationProblem> {
    match target {
        CentreTarget::Circle(edge) => {
            let (centre, radius) = seen_edge(lines, edge)?
                .round()
                .ok_or(AnnotationProblem::NotRound)?;
            Ok(CentreGeometry::Mark { centre, radius })
        }
        CentreTarget::Cylinder(face) => {
            let axis = cylinder_axis(lines, face)?;
            Ok(CentreGeometry::Line {
                from: axis.from,
                to: axis.to,
            })
        }
    }
}

fn whole_circles(lines: &ViewLines) -> Vec<CentreTarget> {
    let mut names: Vec<EdgeName> = Vec::new();

    for piece in &lines.pieces {
        if let ViewSource::Edge { name, .. } = piece.source
            && !names.contains(&name)
        {
            names.push(name);
        }
    }

    let mut kept: Vec<(EdgeName, Point2, f64)> = Vec::new();

    for name in names {
        let Ok(seen) = seen_edge(lines, name) else {
            continue;
        };
        let Some((centre, radius)) = seen.round().filter(|_| seen.is_whole_circle()) else {
            continue;
        };
        let slack = SAME_PLACE * (1.0 + centre.x.abs().max(centre.y.abs()));

        match kept
            .iter_mut()
            .find(|(_, other, _)| other.distance(centre) <= slack)
        {
            Some(larger) if larger.2 >= radius => {}
            Some(smaller) => *smaller = (name, centre, radius),
            None => kept.push((name, centre, radius)),
        }
    }
    kept.into_iter()
        .map(|(name, ..)| CentreTarget::Circle(name))
        .collect()
}

fn cylinders(lines: &ViewLines) -> Vec<CentreTarget> {
    let mut faces: Vec<FaceName> = Vec::new();

    for piece in &lines.pieces {
        if let ViewSource::Silhouette { name, .. } = piece.source
            && !faces.contains(&name)
        {
            faces.push(name);
        }
    }

    let mut kept: Vec<(FaceName, Axis)> = Vec::new();

    for face in faces {
        let Ok(axis) = cylinder_axis(lines, face) else {
            continue;
        };

        match kept.iter_mut().find(|(_, other)| other.same_line(&axis)) {
            Some(longer) if longer.1.length() >= axis.length() => {}
            Some(shorter) => *shorter = (face, axis),
            None => kept.push((face, axis)),
        }
    }
    kept.into_iter()
        .map(|(face, _)| CentreTarget::Cylinder(face))
        .collect()
}

pub fn centre_targets(lines: &ViewLines) -> Vec<CentreTarget> {
    let mut targets = whole_circles(lines);

    targets.extend(cylinders(lines));
    targets
}

pub fn centre_target_at(lines: &ViewLines, source: ViewSource) -> Option<CentreTarget> {
    match source {
        ViewSource::Edge { name, .. } => {
            seen_edge(lines, name).ok()?.round()?;
            Some(CentreTarget::Circle(name))
        }
        ViewSource::Silhouette { name, .. } => {
            cylinder_axis(lines, name).ok()?;
            Some(CentreTarget::Cylinder(name))
        }
    }
}

pub fn centre_problem(
    drawing: &Drawing,
    centre: &CentreMark,
    lines: &DrawnLines<'_>,
) -> Option<AnnotationProblem> {
    if !drawing.has_view(centre.view) {
        return Some(AnnotationProblem::ViewGone);
    }

    let measured = lines.get(&measured_lines_of(drawing, centre.view))?;
    centre_geometry(centre.target, measured).err()
}

fn centre_marks(geometry: CentreGeometry, placed: &PlacedView) -> Vec<SheetMark> {
    match geometry {
        CentreGeometry::Mark { centre, radius } => {
            let centre = placed.on_sheet(centre);
            let arm = radius * placed.scale + CENTRE_OVERSHOOT;

            [Vector2::X, Vector2::Y]
                .into_iter()
                .map(|direction| {
                    SheetMark::Line(centre - direction * arm, centre + direction * arm)
                })
                .collect()
        }
        CentreGeometry::Line { from, to } => {
            let (from, to) = (placed.on_sheet(from), placed.on_sheet(to));

            let beyond = (to - from).try_normalize().unwrap_or(Vector2::X) * CENTRE_OVERSHOOT;

            vec![SheetMark::Line(from - beyond, to + beyond)]
        }
    }
}

pub(crate) fn sheet_centre(
    drawing: &Drawing,
    centre: &CentreMark,
    plan: &SheetPlan,
    lines: &DrawnLines<'_>,
) -> Option<SheetAnnotation> {
    let annotation = Annotation::Centre(centre.id);
    let lost = |problem| SheetAnnotation {
        annotation,
        marks: Vec::new(),
        texts: Vec::new(),
        measure: None,
        problem: Some(problem),
    };

    if !drawing.has_view(centre.view) {
        return Some(lost(AnnotationProblem::ViewGone));
    }

    let placed = plan.placed(centre.view)?;
    let measured = lines.get(&measured_lines_of(drawing, centre.view))?;
    Some(match centre_geometry(centre.target, measured) {
        Ok(geometry) => SheetAnnotation {
            annotation,
            marks: centre_marks(geometry, placed),
            texts: Vec::new(),
            measure: None,
            problem: None,
        },
        Err(problem) => lost(problem),
    })
}

pub fn new_centres(drawing: &Drawing, view: DrawnView, lines: &ViewLines) -> Vec<CentreTarget> {
    centre_targets(lines)
        .into_iter()
        .filter(|target| !drawing.has_centre(view, *target))
        .collect()
}

pub fn pick_centre(
    drawing: &Drawing,
    plan: &SheetPlan,
    lines: &DrawnLines<'_>,
    point: Point2,
    reach: f64,
) -> Option<(DrawnView, CentreTarget)> {
    let (view, source, _) =
        nearest_piece(plan, lines, drawing.hidden_lines, (point, reach), |_| true)?;
    let measured = lines.get(&measured_lines_of(drawing, view))?;
    Some((view, centre_target_at(measured, source)?))
}
