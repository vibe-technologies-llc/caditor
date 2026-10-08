use caditor_document::{Document, FeatureId};
use caditor_geometry::{Plane, Vector3};
use caditor_sketch::Entity;

use crate::{
    feature_tree::count, model::Model, scene, sketch_status::SketchSummary, units::LengthUnit,
    visibility,
};

const SQUARE: f64 = 1e-9;

const NAMED_AT_MOST: usize = 6;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Key {
    revision: u64,
    evaluation: u64,
    edited: Option<FeatureId>,
}

#[derive(Debug, Clone, Default)]
pub struct SceneDescription {
    cached: Option<(Key, String)>,
}

impl SceneDescription {
    pub fn of(&mut self, model: &Model, edited: Option<FeatureId>) -> &str {
        let key = Key {
            revision: model.revision(),
            evaluation: model.evaluation_generation(),
            edited,
        };
        if self
            .cached
            .as_ref()
            .is_none_or(|(cached, _)| *cached != key)
        {
            self.cached = Some((key, describe(model, edited)));
        }
        self.cached.as_ref().map_or("", |(_, text)| text.as_str())
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
    let datums = shown_names(document, |feature| feature.kind.datum().is_some());
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

fn shown_names(
    document: &Document,
    keep: impl Fn(&caditor_document::Feature) -> bool,
) -> Vec<String> {
    document
        .active_features()
        .filter(|feature| !feature.hidden && keep(feature))
        .map(|feature| feature.name.clone())
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
