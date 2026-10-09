use caditor_document::{DatumResult, FeatureId};
use caditor_geometry::{Plane, Vector3};
use caditor_kernel::FaceId;
use caditor_sketch::{Entity, EntityId, EntityState};

use crate::{
    bodies, datum_tools,
    feature_tree::count,
    model::Model,
    scene,
    sketch_status::{self, SketchSummary},
    units::LengthUnit,
    visibility,
};

const SQUARE: f64 = 1e-9;

const NAMED_AT_MOST: usize = 6;
const ITEMS_AT_MOST: usize = 400;
const FACES_AT_MOST: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Key {
    revision: u64,
    evaluation: u64,
    edited: Option<FeatureId>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    pub name: String,
    pub parts: Vec<String>,
}

impl Item {
    fn alone(name: String) -> Self {
        Self {
            name,
            parts: Vec::new(),
        }
    }
}

#[derive(Debug, Clone)]
struct Described {
    key: Key,
    text: String,
    items: Option<Vec<Item>>,
}

#[derive(Debug, Clone, Default)]
pub struct SceneDescription {
    cached: Option<Described>,
}

impl SceneDescription {
    pub fn of(&mut self, model: &Model, edited: Option<FeatureId>) -> &str {
        self.refreshed(model, edited).text.as_str()
    }

    pub fn items(&mut self, model: &Model, edited: Option<FeatureId>) -> &[Item] {
        let described = self.refreshed(model, edited);
        described.items.get_or_insert_with(|| items(model, edited))
    }

    fn refreshed(&mut self, model: &Model, edited: Option<FeatureId>) -> &mut Described {
        let key = Key {
            revision: model.revision(),
            evaluation: model.evaluation_generation(),
            edited,
        };
        if self
            .cached
            .as_ref()
            .is_some_and(|described| described.key != key)
        {
            self.cached = None;
        }
        self.cached.get_or_insert_with(|| Described {
            key,
            text: describe(model, edited),
            items: None,
        })
    }

    pub fn forget(&mut self) {
        self.cached = None;
    }
}

fn named(names: &[String]) -> String {
    let shown: Vec<&str> = names
        .iter()
        .take(NAMED_AT_MOST)
        .map(String::as_str)
        .collect();
    match (
        names.len().saturating_sub(NAMED_AT_MOST),
        shown.split_last(),
    ) {
        (0, Some((last, []))) => (*last).to_owned(),
        (0, Some((last, rest))) => format!("{} and {last}", rest.join(", ")),
        (more, _) => format!("{} and {more} more", shown.join(", ")),
    }
}

fn group(names: &[String], singular: &str, plural: &str) -> Option<String> {
    (!names.is_empty()).then(|| {
        format!(
            "{} shown: {}",
            count(names.len(), singular, plural),
            named(names)
        )
    })
}

pub fn describe(model: &Model, edited: Option<FeatureId>) -> String {
    let document = model.document();
    if let Some(sketch) = edited.and_then(|feature| edited_sketch(model, feature)) {
        return sketch;
    }
    let evaluation = model.evaluation();
    let unit = model.length_unit();
    let bodies: Vec<String> = evaluation
        .bodies()
        .map(|(body, _)| body)
        .filter(|body| visibility::is_shown(document, *body))
        .enumerate()
        .filter_map(|(index, body)| {
            let name = document.body_name(body)?;
            let placed = (index < NAMED_AT_MOST)
                .then(|| evaluation.body(body)?.bounding_box())
                .flatten()
                .map(|bounds| {
                    let size = bounds.max() - bounds.min();
                    let low = bounds.min();
                    format!(
                        " ({} by {} by {}, lowest corner at {})",
                        unit.spoken_length(size.x),
                        unit.spoken_length(size.y),
                        unit.spoken_length(size.z),
                        unit.spoken_position([low.x, low.y, low.z])
                    )
                })
                .unwrap_or_default();
            Some(format!("{name}{placed}"))
        })
        .collect();
    let hidden_bodies = evaluation
        .bodies()
        .filter(|(body, _)| !visibility::is_shown(document, *body))
        .count();
    let sketches: Vec<String> = document
        .active_features()
        .filter(|feature| !feature.hidden && feature.kind.sketch().is_some())
        .enumerate()
        .map(|(index, feature)| {
            let plane = (index < NAMED_AT_MOST)
                .then(|| scene::sketch_plane(document, evaluation, feature.id()))
                .flatten()
                .map(|plane| format!(" ({})", plane_words(plane, unit)))
                .unwrap_or_default();
            format!("{}{plane}", feature.name)
        })
        .collect();
    let datums: Vec<String> = document
        .active_features()
        .filter(|feature| !feature.hidden && feature.kind.datum().is_some())
        .enumerate()
        .map(|(index, feature)| {
            let place = (index < NAMED_AT_MOST)
                .then(|| datum_tools::result(evaluation, feature.id()))
                .flatten()
                .map(|result| format!(" ({})", datum_words(result, unit)))
                .unwrap_or_default();
            format!("{}{place}", feature.name)
        })
        .collect();
    let mut parts: Vec<String> = [
        group(&bodies, "body", "bodies"),
        group(&sketches, "sketch", "sketches"),
        group(&datums, "datum", "datums"),
    ]
    .into_iter()
    .flatten()
    .collect();
    if hidden_bodies > 0 {
        parts.push(format!("{} hidden", count(hidden_bodies, "body", "bodies")));
    }
    if parts.is_empty() {
        return "Empty: nothing is drawn yet besides the principal planes and axes.".to_owned();
    }
    format!("{}.", parts.join("; "))
}

fn datum_words(result: DatumResult, unit: LengthUnit) -> String {
    match result {
        DatumResult::Plane(plane) => plane_words(plane, unit),
        DatumResult::Axis(ray) => {
            let origin = ray.origin();
            let through = unit.spoken_position([origin.x, origin.y, origin.z]);
            format!("{} through {through}", direction_words(ray.direction()))
        }
        DatumResult::Point(point) => {
            format!("at {}", unit.spoken_position([point.x, point.y, point.z]))
        }
        DatumResult::Frame(frame) => {
            let origin = frame.origin();
            format!(
                "at {}, its X axis {} and its Z axis {}",
                unit.spoken_position([origin.x, origin.y, origin.z]),
                direction_words(frame.x_axis()),
                direction_words(frame.normal())
            )
        }
    }
}

fn direction_words(direction: Vector3) -> String {
    let axes = [(Vector3::X, "x"), (Vector3::Y, "y"), (Vector3::Z, "z")];
    for (axis, name) in axes {
        let along = direction.dot(axis);
        if (along.abs() - 1.0).abs() <= SQUARE {
            return format!("along {name}");
        }
    }
    let rounded = |value: f64| {
        format!("{:.3}", value)
            .trim_end_matches('0')
            .trim_end_matches('.')
            .to_owned()
    };
    format!(
        "along {}, {}, {}",
        rounded(direction.x),
        rounded(direction.y),
        rounded(direction.z)
    )
}

fn items(model: &Model, edited: Option<FeatureId>) -> Vec<Item> {
    if let Some(feature) = edited {
        return sketch_items(model, feature);
    }
    let document = model.document();
    let evaluation = model.evaluation();
    let unit = model.length_unit();
    let mut items = Vec::new();
    for (body, _) in evaluation.bodies() {
        if items.len() >= ITEMS_AT_MOST || !visibility::is_shown(document, body) {
            continue;
        }
        let Some(name) = document.body_name(body) else {
            continue;
        };
        let Some(shown) = bodies::shown(evaluation, body) else {
            items.push(Item::alone(format!("Body {name}")));
            continue;
        };
        let place = evaluation
            .body(body)
            .and_then(|solid| solid.bounding_box())
            .map(|bounds| {
                let size = bounds.max() - bounds.min();
                let low = bounds.min();
                format!(
                    ", {} by {} by {}, lowest corner at {}",
                    unit.spoken_length(size.x),
                    unit.spoken_length(size.y),
                    unit.spoken_length(size.z),
                    unit.spoken_position([low.x, low.y, low.z])
                )
            })
            .unwrap_or_default();
        let faces: Vec<FaceId> = shown.solid.faces().map(|(face, _)| face).collect();
        let mut parts: Vec<String> = faces
            .iter()
            .take(FACES_AT_MOST)
            .map(|face| bodies::describe_face_id(document, shown, *face))
            .collect();
        if let Some(more) = faces
            .len()
            .checked_sub(FACES_AT_MOST)
            .filter(|more| *more > 0)
        {
            parts.push(format!("and {} more", count(more, "face", "faces")));
        }
        items.push(Item {
            name: format!(
                "Body {name}{place}, {}",
                count(faces.len(), "face", "faces")
            ),
            parts,
        });
    }
    for feature in document.active_features() {
        if items.len() >= ITEMS_AT_MOST || feature.hidden {
            continue;
        }
        if let Some(sketch) = feature.kind.sketch() {
            let plane = scene::sketch_plane(document, evaluation, feature.id())
                .map(|plane| format!(", {}", plane_words(plane, unit)))
                .unwrap_or_default();
            let curves = sketch
                .entities()
                .filter(|(_, entity)| !matches!(entity, Entity::Point(_)))
                .count();
            items.push(Item::alone(format!(
                "Sketch {}{plane}, {}",
                feature.name,
                count(curves, "curve", "curves")
            )));
        } else if feature.kind.datum().is_some() {
            let place = datum_tools::result(evaluation, feature.id())
                .map(|result| format!(", {}", datum_words(result, unit)))
                .unwrap_or_else(|| ", not computed".to_owned());
            items.push(Item::alone(format!("Datum {}{place}", feature.name)));
        }
    }
    items
}

fn sketch_items(model: &Model, feature: FeatureId) -> Vec<Item> {
    let Some(owner) = model.document().feature(feature) else {
        return Vec::new();
    };
    let Some(sketch) = model.displayed_sketch(owner) else {
        return Vec::new();
    };
    let unit = model.length_unit();
    let solution = sketch_status::up_to_date_solution(model.evaluation(), feature);
    let point = |id: EntityId| {
        sketch
            .point(id)
            .map(|at| unit.spoken_point([at.x, at.y]))
            .unwrap_or_default()
    };
    sketch
        .entities()
        .filter(|(id, _)| id.reference().is_none())
        .take(ITEMS_AT_MOST)
        .map(|(id, entity)| {
            let shape = match *entity {
                Entity::Point(_) => format!("at {}", point(id)),
                Entity::Line { start, end } => {
                    let length = sketch
                        .line_endpoints(id)
                        .map(|(from, to)| unit.spoken_length(from.distance(to)))
                        .unwrap_or_default();
                    format!("from {} to {}, {length} long", point(start), point(end))
                }
                Entity::Circle { center, .. } => {
                    let radius = sketch.circle(id).map_or(0.0, |(_, radius)| radius);
                    format!(
                        "centred at {}, radius {}",
                        point(center),
                        unit.spoken_length(radius)
                    )
                }
                Entity::Arc { center, start, end } => {
                    let (radius, sweep) = sketch
                        .arc(id)
                        .map_or((0.0, 0.0), |arc| (arc.radius, arc.sweep.to_degrees()));
                    format!(
                        "centred at {}, radius {}, from {} to {}, sweeping {:.0}°",
                        point(center),
                        unit.spoken_length(radius),
                        point(start),
                        point(end),
                        sweep
                    )
                }
                Entity::Spline { ref control_points } => {
                    let ends = control_points
                        .first()
                        .zip(control_points.last())
                        .map(|(first, last)| format!(" from {} to {}", point(*first), point(*last)))
                        .unwrap_or_default();
                    format!(
                        "{}{ends}",
                        count(control_points.len(), "control point", "control points")
                    )
                }
            };
            let construction = if sketch.is_construction(id) {
                ", construction"
            } else {
                ""
            };
            let state = match solution.and_then(|solution| solution.entity_state(id)) {
                Some(EntityState::FullyConstrained) => ", fully constrained",
                Some(EntityState::UnderConstrained) => ", not fully constrained",
                None => "",
            };
            Item::alone(format!(
                "{} {shape}{construction}{state}",
                sketch.entity_label(id)
            ))
        })
        .collect()
}

fn plane_words(plane: Plane, unit: LengthUnit) -> String {
    let normal = plane.normal();
    let origin = plane.origin();
    let axes = [
        (Vector3::X, "x", origin.x),
        (Vector3::Y, "y", origin.y),
        (Vector3::Z, "z", origin.z),
    ];
    for (axis, name, offset) in axes {
        let along = normal.dot(axis);
        if (along.abs() - 1.0).abs() <= SQUARE {
            let sign = if along > 0.0 { "+" } else { "-" };
            return format!(
                "facing {sign}{name}, at {name} = {}",
                unit.spoken_length(offset)
            );
        }
    }
    format!(
        "on a slanted plane through {}",
        unit.spoken_position([origin.x, origin.y, origin.z])
    )
}

fn edited_sketch(model: &Model, feature: FeatureId) -> Option<String> {
    let owner = model.document().feature(feature)?;
    let sketch = model.displayed_sketch(owner)?;
    let (mut points, mut curves) = (0, 0);
    for (id, entity) in sketch.entities() {
        if id.reference().is_some() {
            continue;
        }
        match entity {
            Entity::Point(_) => points += 1,
            Entity::Line { .. }
            | Entity::Circle { .. }
            | Entity::Arc { .. }
            | Entity::Spline { .. } => {
                curves += 1;
            }
        }
    }
    let constraints = sketch.constraints().count();
    let unit = model.length_unit();
    let spans = sketch
        .entities()
        .filter(|(id, _)| id.reference().is_none())
        .filter_map(|(id, _)| sketch.point(id))
        .fold(
            None,
            |bounds: Option<(caditor_geometry::Point2, caditor_geometry::Point2)>, point| {
                Some(match bounds {
                    Some((low, high)) => (low.min(point), high.max(point)),
                    None => (point, point),
                })
            },
        )
        .map(|(low, high)| {
            format!(
                "; its points span x from {} to {} and y from {} to {}",
                unit.spoken_length(low.x),
                unit.spoken_length(high.x),
                unit.spoken_length(low.y),
                unit.spoken_length(high.y)
            )
        })
        .unwrap_or_default();
    let summary = SketchSummary::of(model.evaluation(), feature);
    let open: String = [summary.open_ends_text(), summary.beyond_text()]
        .into_iter()
        .flatten()
        .map(|note| format!("; {note}"))
        .collect();
    Some(format!(
        "Editing {}: {}, {} and {}; {}{spans}{open}.",
        owner.name,
        count(curves, "curve", "curves"),
        count(points, "point", "points"),
        count(constraints, "constraint", "constraints"),
        summary.status_text().to_lowercase()
    ))
}
