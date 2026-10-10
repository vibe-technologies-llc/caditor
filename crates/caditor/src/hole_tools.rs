use caditor_document::{
    Document, Edit, FeatureId, FeatureKind, Hole, HoleBottom, HoleDepth, HoleShape, HoleSizing,
    HoleStandard, HoleStep, HoleStyle, SketchAttachment, SketchFeature, TappedThread, Transaction,
    hole_centres,
};
use caditor_expression::{Expression, Unit};
use caditor_geometry::{Plane, Point2, Ray};
use caditor_kernel::{FaceId, Solid};
use caditor_sketch::{Entity, EntityId, Sketch};

use crate::{
    bodies,
    body_selection::face_boundary,
    editing::{self, EditingCommand, SketchEditing},
    hole_on_curve,
    last_values::{Remembered, Starts},
    model::{Action, Model, Notice},
    reference_picking::{Picking, Slot},
    scene,
    selection::{Pickable, Selection},
    sketch_placement::{self, FaceChoice},
    solid_tools,
    units::LengthUnit,
    visibility,
};

pub const TITLE: &str = "Hole";
pub const DESCRIPTION: &str =
    "Drill a hole at every point of the sketch: plain, counterbored, countersunk or stepped";
pub const DEFAULT_DIAMETER: f64 = 6.0;
pub const DEFAULT_DEPTH: f64 = 10.0;
pub const DEFAULT_COUNTERBORE_DIAMETER: f64 = 10.0;
pub const DEFAULT_COUNTERBORE_DEPTH: f64 = 3.0;
pub const DEFAULT_COUNTERSINK_DIAMETER: f64 = 10.0;
pub const DEFAULT_COUNTERSINK_ANGLE: f64 = 90.0;
pub const DEFAULT_DRILL_POINT_ANGLE: f64 = 118.0;
pub const DEFAULT_SLOT_LENGTH: f64 = 10.0;
pub const DEFAULT_STEP_DIAMETER: f64 = 8.0;
pub const DEFAULT_STEP_DEPTH: f64 = 2.0;
const NO_SKETCH: &str = "Select a flat face of a body, or draw a sketch on one and place points \
                         where the holes go; a sketch already drilled or swept is not guessed";
const FACE_SAMPLES: u32 = 16;
const SEARCH_STEPS: u32 = 16;
const REFINEMENTS: usize = 4;
const TIE: f64 = 1e-3;
const NO_POINTS: &str = "Place points or circles in the sketch where the holes go";
const NO_BODY: &str = "Make a body to drill into first";
const NOTHING_TO_DRILL: &str =
    "Select one flat face of a body to drill it, or the points of a sketch";
const GONE: &str = "The hole no longer exists";
const NOT_ONE_POINT: &str = "The hole's sketch holds more than one free point; edit the sketch to \
                             move its points";
const NO_FACE: &str = "Select one flat face of a body to drill the hole there";
const MISSED: &str = "The click missed the face's plane";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HoleSource {
    pub sketch: FeatureId,
    pub body: FeatureId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HoleStart {
    Sketch(HoleSource),
    Face(FaceChoice),
    CurvedFace(FaceChoice),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Plain,
    Counterbore,
    Countersink,
    Stepped,
}

impl Kind {
    pub const ALL: [Self; 4] = [
        Self::Plain,
        Self::Counterbore,
        Self::Countersink,
        Self::Stepped,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Plain => "Plain",
            Self::Counterbore => "Counterbore",
            Self::Countersink => "Countersink",
            Self::Stepped => "Stepped",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Self::Plain => "A straight hole",
            Self::Counterbore => "A wider flat-bottomed step at the mouth, for a bolt head",
            Self::Countersink => "A cone at the mouth, for a flat-head screw",
            Self::Stepped => {
                "Flat-bottomed steps narrowing into the hole, each with its own diameter and depth"
            }
        }
    }

    pub fn of(style: &HoleStyle) -> Self {
        match style {
            HoleStyle::Plain => Self::Plain,
            HoleStyle::Counterbore { .. } => Self::Counterbore,
            HoleStyle::Countersink { .. } => Self::Countersink,
            HoleStyle::Stepped(_) => Self::Stepped,
        }
    }

    pub fn style_for(self, standard: Option<HoleStandard>, unit: LengthUnit) -> HoleStyle {
        let Some(standard) = standard else {
            return self.default_style(unit);
        };
        match self {
            Self::Plain => HoleStyle::Plain,
            Self::Counterbore => {
                let (diameter, depth) = standard.counterbore();
                HoleStyle::Counterbore {
                    diameter: millimetres(diameter),
                    depth: millimetres(depth),
                }
            }
            Self::Countersink => HoleStyle::Countersink {
                diameter: millimetres(standard.countersink()),
                angle: solid_tools::degrees(DEFAULT_COUNTERSINK_ANGLE),
            },
            Self::Stepped => {
                let (diameter, depth) = standard.counterbore();
                let tenths = |value: f64| millimetres((value * 10.0).round() / 10.0);
                HoleStyle::Stepped(vec![
                    HoleStep {
                        diameter: millimetres(diameter),
                        depth: millimetres(depth),
                    },
                    HoleStep {
                        diameter: tenths((diameter + standard.diameter()) / 2.0),
                        depth: tenths(depth / 2.0),
                    },
                ])
            }
        }
    }

    pub fn default_style(self, unit: LengthUnit) -> HoleStyle {
        match self {
            Self::Plain => HoleStyle::Plain,
            Self::Counterbore => HoleStyle::Counterbore {
                diameter: unit.default_length(DEFAULT_COUNTERBORE_DIAMETER),
                depth: unit.default_length(DEFAULT_COUNTERBORE_DEPTH),
            },
            Self::Countersink => HoleStyle::Countersink {
                diameter: unit.default_length(DEFAULT_COUNTERSINK_DIAMETER),
                angle: solid_tools::degrees(DEFAULT_COUNTERSINK_ANGLE),
            },
            Self::Stepped => HoleStyle::Stepped(vec![
                HoleStep {
                    diameter: unit.default_length(DEFAULT_COUNTERBORE_DIAMETER),
                    depth: unit.default_length(DEFAULT_COUNTERBORE_DEPTH),
                },
                HoleStep {
                    diameter: unit.default_length(DEFAULT_STEP_DIAMETER),
                    depth: unit.default_length(DEFAULT_STEP_DEPTH),
                },
            ]),
        }
    }
}

pub fn default_drill_point() -> HoleBottom {
    HoleBottom::DrillPoint(solid_tools::degrees(DEFAULT_DRILL_POINT_ANGLE))
}

pub fn default_depth(unit: LengthUnit) -> Expression {
    unit.default_length(DEFAULT_DEPTH)
}

pub fn millimetres(value: f64) -> Expression {
    Expression::measure(value, Unit::Millimetre)
}

pub fn with_standard(hole: &Hole, standard: HoleStandard, unit: LengthUnit) -> Hole {
    Hole {
        diameter: millimetres(standard.diameter()),
        style: Kind::of(&hole.style).style_for(Some(standard), unit),
        standard: Some(standard),
        ..hole.clone()
    }
}

pub fn default_slot(unit: LengthUnit) -> HoleShape {
    HoleShape::Slot {
        length: unit.default_length(DEFAULT_SLOT_LENGTH),
        angle: solid_tools::degrees(0.0),
    }
}

pub fn start(
    model: &Model,
    selection: &Selection,
    editing: &SketchEditing,
) -> Result<HoleStart, &'static str> {
    let sketch_chosen = selection
        .iter()
        .any(|pickable| matches!(pickable, Pickable::SketchEntity { .. }));
    let face = sketch_placement::selected_face(selection)
        .filter(|_| editing.feature().is_none() && !sketch_chosen);
    match face {
        Some(face) if sketch_placement::is_flat(model, face) => Ok(HoleStart::Face(face)),
        Some(face) => Ok(HoleStart::CurvedFace(face)),
        None => source(model, selection, editing).map(HoleStart::Sketch),
    }
}

pub fn source(
    model: &Model,
    selection: &Selection,
    editing: &SketchEditing,
) -> Result<HoleSource, &'static str> {
    let document = model.document();
    let may_guess = selection.is_empty();
    let swept = solid_tools::sweep_source(
        document,
        model.evaluation(),
        selection,
        editing,
        if may_guess {
            solid_tools::Guess::Drill
        } else {
            solid_tools::Guess::Never
        },
    )?
    .ok_or(if may_guess {
        NO_SKETCH
    } else {
        NOTHING_TO_DRILL
    })?;
    let sketch = swept.sketch;
    let definition = editing::edited_sketch(document, sketch).ok_or(NO_SKETCH)?;
    if hole_centres(definition).is_empty() {
        return Err(NO_POINTS);
    }
    let attached = document
        .feature(sketch)
        .and_then(|feature| feature.kind.attachment())
        .and_then(|attachment| attachment.body());
    let body = attached
        .or(swept.body)
        .or_else(|| document.bodies_standing().last().copied())
        .ok_or(NO_BODY)?;
    Ok(HoleSource { sketch, body })
}

pub fn new_hole(sketch: FeatureId, body: FeatureId, starts: &Starts) -> FeatureKind {
    FeatureKind::Hole(Hole {
        sketch,
        body,
        diameter: starts.length(Remembered::HoleDiameter, DEFAULT_DIAMETER),
        depth: HoleDepth::Blind(starts.length(Remembered::HoleDepth, DEFAULT_DEPTH)),
        style: HoleStyle::Plain,
        reversed: false,
        shape: HoleShape::Round,
        standard: None,
        sizing: HoleSizing::Typed,
        bottom: HoleBottom::Flat,
        thread: TappedThread::default(),
    })
}

pub fn create_on_face(
    model: &Model,
    face: FaceChoice,
) -> Result<(Transaction, FeatureId, String), &'static str> {
    let document = model.document();
    let (attachment, plane) = sketch_placement::attachment_at(model, face, document.bar_index())?;
    let middle = face_middle(model, face, &plane).ok_or(sketch_placement::NOT_FLAT)?;
    let sketch_name = editing::next_sketch_name(document);
    let name = editing::next_feature_name(document, TITLE);
    let mut sketch = Sketch::new(plane);
    sketch.add_point(middle);
    let mut transaction = document.transaction(format!("Create {name}"));
    let placed = transaction.add_feature(
        sketch_name.clone(),
        FeatureKind::Sketch(SketchFeature::on_face(sketch, attachment)),
    );
    transaction.edit(Edit::SetFeatureHidden {
        id: placed,
        hidden: true,
    });
    let feature = transaction.add_feature(
        name.clone(),
        new_hole(placed, face.body, &Starts::of(model)),
    );
    let told = format!(
        "{name} is drilled in the middle of the face, as far from its edges as it can be. From \
         its panel, type its Position, measure it from edges, centre it on a round edge or add \
         more holes on the face; {sketch_name} holds its points."
    );
    Ok((transaction.finish(), feature, told))
}

pub fn face_middle(model: &Model, face: FaceChoice, plane: &Plane) -> Option<Point2> {
    let body = bodies::shown(model.evaluation(), face.body)?;
    let id = bodies::find_face(body, face.face)?;
    middle_of(&body.solid, id, plane)
}

pub fn middle_of(solid: &Solid, id: FaceId, plane: &Plane) -> Option<Point2> {
    deepest_point(&boundary_segments(solid, id, plane))
}

pub fn clear_spot(solid: &Solid, id: FaceId, plane: &Plane, taken: &[Point2]) -> Option<Point2> {
    let mut segments = boundary_segments(solid, id, plane);
    segments.extend(taken.iter().map(|point| [*point, *point]));
    deepest_point(&segments)
}

fn boundary_segments(solid: &Solid, id: FaceId, plane: &Plane) -> Vec<[Point2; 2]> {
    face_boundary(solid, id)
        .into_iter()
        .filter_map(|edge| solid.edge(edge))
        .flat_map(|edge| {
            let interval = edge.interval();
            let points: Vec<Point2> = (0..=FACE_SAMPLES)
                .map(|step| {
                    let t = f64::from(step) / f64::from(FACE_SAMPLES);
                    let at = interval.start() + (interval.end() - interval.start()) * t;
                    plane.to_local(edge.curve().point(at))
                })
                .collect();
            points
                .windows(2)
                .filter_map(|pair| match pair {
                    [start, end] => Some([*start, *end]),
                    _ => None,
                })
                .collect::<Vec<_>>()
        })
        .collect()
}

fn deepest_point(boundary: &[[Point2; 2]]) -> Option<Point2> {
    let first = boundary.first()?[0];
    let (low, high) = boundary
        .iter()
        .flatten()
        .fold((first, first), |(low, high), point| {
            (low.min(*point), high.max(*point))
        });
    let middle = (low + high) / 2.0;
    let mut best = inside(boundary, middle).then(|| (clearance(boundary, middle), middle));
    let mut centre = middle;
    let mut half = (high - low) / 2.0;
    for _ in 0..REFINEMENTS {
        for row in 0..=SEARCH_STEPS {
            for column in 0..=SEARCH_STEPS {
                let fraction =
                    Point2::new(f64::from(column), f64::from(row)) / f64::from(SEARCH_STEPS) * 2.0
                        - Point2::new(1.0, 1.0);
                let candidate = centre + fraction * half;
                if !inside(boundary, candidate) {
                    continue;
                }
                let distance = clearance(boundary, candidate);
                let better = best.is_none_or(|(deepest, chosen): (f64, Point2)| {
                    distance > deepest * (1.0 + TIE)
                        || (distance >= deepest * (1.0 - TIE)
                            && candidate.distance(middle) < chosen.distance(middle))
                });
                if better {
                    best = Some((distance, candidate));
                }
            }
        }
        let (_, chosen) = best?;
        centre = chosen;
        half *= 2.0 / f64::from(SEARCH_STEPS);
    }
    best.map(|(_, point)| point)
}

fn inside(boundary: &[[Point2; 2]], point: Point2) -> bool {
    boundary
        .iter()
        .filter(|[start, end]| {
            (start.y > point.y) != (end.y > point.y)
                && point.x < start.x + (point.y - start.y) / (end.y - start.y) * (end.x - start.x)
        })
        .count()
        % 2
        == 1
}

fn clearance(boundary: &[[Point2; 2]], point: Point2) -> f64 {
    boundary
        .iter()
        .map(|[start, end]| {
            let along = *end - *start;
            let length = along.length_squared();
            let t = if length > 0.0 {
                ((point - *start).dot(along) / length).clamp(0.0, 1.0)
            } else {
                0.0
            };
            point.distance(*start + along * t)
        })
        .fold(f64::INFINITY, f64::min)
}

pub fn create(
    document: &Document,
    source: HoleSource,
    starts: &Starts,
) -> (Transaction, FeatureId) {
    let name = editing::next_feature_name(document, TITLE);
    let mut transaction = document.transaction(format!("Create {name}"));
    let feature = transaction.add_feature(name, new_hole(source.sketch, source.body, starts));
    if visibility::is_shown(document, source.sketch) {
        transaction.edit(Edit::SetFeatureHidden {
            id: source.sketch,
            hidden: true,
        });
    }
    (transaction.finish(), feature)
}

pub fn create_actions(model: &Model, start: HoleStart) -> Vec<Action> {
    match start {
        HoleStart::Sketch(source) => {
            let (transaction, feature) = create(model.document(), source, &Starts::of(model));
            vec![
                Action::Apply(transaction),
                Action::Editing(EditingCommand::OpenSolid(feature)),
            ]
        }
        HoleStart::Face(face) => match create_on_face(model, face) {
            Ok((transaction, feature, told)) => vec![
                Action::Apply(transaction),
                Action::Editing(EditingCommand::OpenSolid(feature)),
                Action::Inform(Notice::info(told)),
            ],
            Err(reason) => vec![Action::Inform(Notice::warning(format!(
                "{TITLE}: {reason}."
            )))],
        },
        HoleStart::CurvedFace(face) => match hole_on_curve::create(model, face) {
            Ok((transaction, feature, told)) => vec![
                Action::Apply(transaction),
                Action::Editing(EditingCommand::OpenSolid(feature)),
                Action::Editing(EditingCommand::Pick(Picking::new(feature, Slot::HolePlace))),
                Action::Inform(Notice::info(told)),
            ],
            Err(reason) => vec![Action::Inform(Notice::warning(format!(
                "{TITLE}: {reason}."
            )))],
        },
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LonePoint {
    pub entity: EntityId,
    pub at: Point2,
}

pub fn lone_point(document: &Document, hole: &Hole) -> Option<LonePoint> {
    let sketch = document.feature(hole.sketch)?.kind.sketch()?;
    if sketch.constraints().len() > 0 {
        return None;
    }
    let mut entities = sketch.entities();
    match (entities.next(), entities.next()) {
        (Some((entity, Entity::Point(at))), None) => Some(LonePoint { entity, at: *at }),
        _ => None,
    }
}

pub fn moved(
    document: &Document,
    feature: FeatureId,
    hole: &Hole,
    at: Point2,
) -> Result<Transaction, String> {
    let name = &document.feature(feature).ok_or(GONE)?.name;
    let point = lone_point(document, hole).ok_or(NOT_ONE_POINT)?;
    Ok(Transaction::single(
        format!("Move {name}"),
        Edit::SetSketchEntity {
            feature: hole.sketch,
            id: point.entity,
            entity: Entity::Point(at),
        },
    ))
}

fn placed(
    model: &Model,
    feature: FeatureId,
    hole: &Hole,
    face: FaceChoice,
    ray: Option<Ray>,
) -> Result<Transaction, String> {
    let document = model.document();
    if hole_on_curve::mount(document, hole).is_some() || !sketch_placement::is_flat(model, face) {
        return hole_on_curve::moved(model, feature, hole, face, ray);
    }
    let name = &document.feature(feature).ok_or(GONE)?.name;
    let point = lone_point(document, hole).ok_or(NOT_ONE_POINT)?;
    let index = document.feature_index(hole.sketch).ok_or(GONE)?;
    let (attachment, plane) = sketch_placement::attachment_at(model, face, index)?;
    let current = document
        .feature(hole.sketch)
        .and_then(|sketch| sketch.kind.attachment())
        .and_then(|attachment| attachment.face());
    let stays = current == Some(&attachment);
    let frame = if stays {
        scene::sketch_plane(document, model.evaluation(), hole.sketch).unwrap_or(plane)
    } else {
        plane
    };
    let at = match ray {
        Some(ray) => frame.to_local(ray.at(ray.intersect_plane(&frame).ok_or(MISSED)?)),
        None => face_middle(model, face, &frame).ok_or(sketch_placement::NOT_FLAT)?,
    };
    let mut transaction = document.transaction(format!("Move {name}"));
    if !stays {
        transaction.edit(Edit::SetSketchPlacement {
            feature: hole.sketch,
            plane,
            attachment: Some(SketchAttachment::Face(attachment)),
        });
    }
    transaction.edit(Edit::SetSketchEntity {
        feature: hole.sketch,
        id: point.entity,
        entity: Entity::Point(at),
    });
    if hole.body != face.body {
        transaction.edit(Edit::SetFeatureKind {
            id: feature,
            kind: FeatureKind::Hole(Hole {
                body: face.body,
                ..hole.clone()
            }),
        });
    }
    let transaction = transaction.finish();
    document
        .check(&transaction)
        .map_err(|_| "The hole cannot stand on that face".to_owned())?;
    Ok(transaction)
}

pub fn place_change(
    model: &Model,
    selection: &Selection,
    feature: FeatureId,
    hole: &Hole,
) -> Result<Transaction, String> {
    let face = sketch_placement::selected_face(selection).ok_or(NO_FACE)?;
    placed(model, feature, hole, face, None)
}

pub fn place_click(
    model: &Model,
    feature: FeatureId,
    pickable: Pickable,
    ray: Option<Ray>,
) -> Vec<Action> {
    let Some(hole) = model
        .document()
        .feature(feature)
        .and_then(|owner| owner.kind.hole())
    else {
        return vec![Action::Editing(EditingCommand::StopPicking)];
    };
    let result = FaceChoice::of(pickable)
        .ok_or_else(|| NO_FACE.to_owned())
        .and_then(|face| placed(model, feature, hole, face, ray));
    match result {
        Ok(transaction) => vec![
            Action::Apply(transaction),
            Action::Editing(EditingCommand::StopPicking),
        ],
        Err(reason) => vec![Action::Inform(Notice::warning(reason))],
    }
}

pub fn edit(document: &Document, feature: FeatureId, hole: Hole) -> Option<Transaction> {
    let name = &document.feature(feature)?.name;
    Some(Transaction::single(
        format!("Edit {name}"),
        Edit::SetFeatureKind {
            id: feature,
            kind: FeatureKind::Hole(hole),
        },
    ))
}

#[cfg(test)]
mod tests {
    use caditor_geometry::Point2;

    use super::deepest_point;

    fn outline(corners: &[(f64, f64)]) -> Vec<[Point2; 2]> {
        let points: Vec<Point2> = corners.iter().map(|(x, y)| Point2::new(*x, *y)).collect();
        points
            .iter()
            .zip(points.iter().cycle().skip(1))
            .map(|(start, end)| [*start, *end])
            .collect()
    }

    #[test]
    fn a_rectangle_is_drilled_at_its_middle() {
        let rectangle = outline(&[(0.0, 0.0), (100.0, 0.0), (100.0, 40.0), (0.0, 40.0)]);

        let point = deepest_point(&rectangle).unwrap();

        assert!(point.distance(Point2::new(50.0, 20.0)) < 1e-9);
    }

    #[test]
    fn an_l_shaped_face_is_drilled_on_its_material() {
        let l_shape = outline(&[
            (0.0, 0.0),
            (100.0, 0.0),
            (100.0, 20.0),
            (20.0, 20.0),
            (20.0, 100.0),
            (0.0, 100.0),
        ]);

        let point = deepest_point(&l_shape).unwrap();

        let in_the_foot = point.y < 20.0 && point.x < 100.0;
        let in_the_leg = point.x < 20.0 && point.y < 100.0;
        assert!(in_the_foot || in_the_leg, "{point:?}");
        assert!(point.x > 9.0 && point.y > 9.0, "{point:?}");
    }

    #[test]
    fn a_face_with_a_hole_in_its_middle_is_drilled_beside_the_hole() {
        let mut face = outline(&[(0.0, 0.0), (60.0, 0.0), (60.0, 60.0), (0.0, 60.0)]);
        face.extend(outline(&[
            (20.0, 20.0),
            (40.0, 20.0),
            (40.0, 40.0),
            (20.0, 40.0),
        ]));

        let point = deepest_point(&face).unwrap();

        let clear_of_the_hole =
            !(20.0..=40.0).contains(&point.x) || !(20.0..=40.0).contains(&point.y);
        assert!(clear_of_the_hole, "{point:?}");
    }

    #[test]
    fn a_u_shaped_face_is_drilled_in_one_of_its_arms_or_its_base() {
        let u_shape = outline(&[
            (0.0, 0.0),
            (60.0, 0.0),
            (60.0, 60.0),
            (40.0, 60.0),
            (40.0, 20.0),
            (20.0, 20.0),
            (20.0, 60.0),
            (0.0, 60.0),
        ]);

        let point = deepest_point(&u_shape).unwrap();

        let in_the_gap = (20.0..=40.0).contains(&point.x) && point.y > 20.0;
        assert!(!in_the_gap, "{point:?}");
    }
}
