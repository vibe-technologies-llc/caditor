use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use caditor_document::{
    Document, Evaluation, ExtrudeEnd, ExtrudeExtent, FeatureId, FeatureResult, ParameterValues,
    RegionChoice, RevolveAxis, RevolveExtent, SolidFeature, displayed_axis, profile_curve,
};
use caditor_expression::Expression;
use caditor_geometry::{Aabb, Aabb2, Plane, Point2, Point3, Vector2};
use caditor_kernel::{
    AngularExtent, Axis2, Interference, Interrupt, LINEAR_RESOLUTION, LinearExtent, Profile,
    ProfileCurve, Region, SamplingTolerance, Selection, Solid, extrude, interference,
    interruptible, mass_properties_tessellating, resolve_regions, revolve, wall_regions,
};
use caditor_sketch::{Reference, Sketch};

use crate::{editing, visibility};

pub const REACH_BUDGET: Duration = Duration::from_millis(150);
const OUTLINE_SEGMENT_ANGLE: f64 = 0.02;
const OUTLINE_SLACK_PER_SIZE: f64 = 1e-3;
const BOX_SLACK: f64 = 100.0 * LINEAR_RESOLUTION;
const SAME_VOLUME: f64 = 1e-6;

pub fn reached_body(
    document: &Document,
    evaluation: &Evaluation,
    parameters: &ParameterValues,
    solid: &SolidFeature,
) -> Option<FeatureId> {
    let standing = document.bodies_standing();
    if standing.is_empty() {
        return None;
    }
    let sketch = solved_sketch(document, evaluation, solid.sketch())?;
    let swept = Swept::of(evaluation, parameters, solid, sketch)?;
    let reach = swept.bounds(sketch)?;
    let near: Vec<Standing<'_>> = standing
        .into_iter()
        .filter(|body| visibility::is_shown(document, *body))
        .filter_map(|body| Standing::of(evaluation, body))
        .filter(|body| boxes_meet(&body.bounds, &reach))
        .collect();
    if near.is_empty() {
        return None;
    }
    let deadline = Instant::now() + REACH_BUDGET;
    let interrupt: Interrupt = Arc::new(move || Instant::now() >= deadline);
    interruptible(interrupt, || swept.most_reached(sketch, solid, &near))
}

struct Standing<'a> {
    body: FeatureId,
    solid: &'a Solid,
    bounds: Aabb,
}

impl<'a> Standing<'a> {
    fn of(evaluation: &'a Evaluation, body: FeatureId) -> Option<Self> {
        let result = evaluation.body_result(body)?.solid()?;
        Some(Self {
            body,
            solid: &result.solid,
            bounds: result.bounding_box()?,
        })
    }
}

enum Contact {
    Overlapping(Solid),
    Touching,
}

fn solved_sketch<'a>(
    document: &'a Document,
    evaluation: &'a Evaluation,
    sketch: FeatureId,
) -> Option<&'a Sketch> {
    evaluation
        .feature(sketch)
        .and_then(|status| status.result.as_deref())
        .and_then(FeatureResult::sketch)
        .map(|result| &result.geometry)
        .or_else(|| editing::edited_sketch(document, sketch))
}

struct Swept {
    plane: Plane,
    motion: Motion,
    wall: Option<f64>,
}

enum Motion {
    Along { start: f64, end: f64 },
    About(Axis2),
}

impl Swept {
    fn of(
        evaluation: &Evaluation,
        parameters: &ParameterValues,
        solid: &SolidFeature,
        sketch: &Sketch,
    ) -> Option<Self> {
        if solid.start().is_some() {
            return None;
        }
        let length = |expression: &Expression| {
            parameters
                .evaluate_expression(expression)
                .ok()
                .map(|quantity| quantity.value)
        };
        let wall = match solid.wall() {
            Some(wall) => Some(length(&wall.thickness)?),
            None => None,
        };
        let plane = sketch.plane();
        let motion = match solid {
            SolidFeature::Extrude(extrude) => {
                if extrude.direction.is_some() || extrude.taper.is_some() {
                    return None;
                }
                let ExtrudeExtent::OneSide {
                    end: ExtrudeEnd::Distance(distance),
                    reversed,
                } = &extrude.extent
                else {
                    return None;
                };
                let distance = length(distance)?;
                match reversed {
                    false => Motion::Along {
                        start: 0.0,
                        end: distance,
                    },
                    true => Motion::Along {
                        start: -distance,
                        end: 0.0,
                    },
                }
            }
            SolidFeature::Revolve(revolve) => {
                if !matches!(revolve.extent, RevolveExtent::Full) || revolve.side.is_some() {
                    return None;
                }
                Motion::About(revolution_axis(
                    evaluation,
                    solid.sketch(),
                    sketch,
                    &plane,
                    &revolve.axis,
                )?)
            }
        };
        Some(Self {
            plane,
            motion,
            wall,
        })
    }

    fn bounds(&self, sketch: &Sketch) -> Option<Aabb> {
        let outline = Aabb2::from_points(
            sketch
                .entities()
                .filter(|(id, _)| !sketch.is_construction(*id))
                .filter_map(|(id, _)| sketch.polyline(id, OUTLINE_SEGMENT_ANGLE))
                .flatten(),
        )?;
        let slack =
            outline.size().length() * OUTLINE_SLACK_PER_SIZE + self.wall.unwrap_or(0.0) + BOX_SLACK;
        let outline = outline.expanded(slack);
        let corners = [
            outline.min(),
            Point2::new(outline.min().x, outline.max().y),
            Point2::new(outline.max().x, outline.min().y),
            outline.max(),
        ];
        match self.motion {
            Motion::Along { start, end } => {
                Aabb::from_points(corners.into_iter().flat_map(|corner| {
                    let at = self.plane.to_world(corner);
                    [start, end].map(|height| at + self.plane.normal() * height)
                }))
            }
            Motion::About(axis) => {
                let radius = corners
                    .iter()
                    .map(|corner| axis.signed_distance(*corner).abs())
                    .fold(0.0, f64::max);
                let along = corners.map(|corner| axis.direction().dot(corner - axis.origin()));
                let ends = [
                    along.into_iter().fold(f64::INFINITY, f64::min),
                    along.into_iter().fold(f64::NEG_INFINITY, f64::max),
                ]
                .map(|distance| {
                    self.plane
                        .to_world(axis.origin() + axis.direction() * distance)
                });
                Aabb::from_points(ends).map(|bounds| bounds.expanded(radius))
            }
        }
    }

    fn tool(&self, sketch: &Sketch, solid: &SolidFeature) -> Option<Solid> {
        let curves: Vec<ProfileCurve> = sketch
            .entities()
            .filter(|(id, _)| !sketch.is_construction(*id))
            .filter_map(|(id, _)| profile_curve(sketch, id))
            .collect();
        let regions = match (self.wall, solid.wall()) {
            (Some(thickness), Some(wall)) => wall_regions(&curves, thickness, wall.side).ok()?,
            _ => chosen_regions(&curves, solid.regions())?,
        };
        let feature = solid.sketch().raw();
        match self.motion {
            Motion::Along { start, end } => extrude(
                &self.plane,
                &regions,
                LinearExtent::two_sided(end, -start).ok()?,
                feature,
            )
            .ok(),
            Motion::About(axis) => {
                revolve(&self.plane, &regions, axis, AngularExtent::full(), feature).ok()
            }
        }
    }

    fn most_reached(
        &self,
        sketch: &Sketch,
        solid: &SolidFeature,
        near: &[Standing<'_>],
    ) -> Option<FeatureId> {
        let tool = self.tool(sketch, solid)?;
        let contacts: Vec<(FeatureId, Contact)> = near
            .iter()
            .filter_map(|body| {
                let contact = match interference(&tool, body.solid) {
                    Ok(Interference::Apart) => return None,
                    Ok(Interference::Overlapping(overlap)) => Contact::Overlapping(overlap),
                    Ok(Interference::Touching(_)) | Err(_) => Contact::Touching,
                };
                Some((body.body, contact))
            })
            .collect();
        let overlaps: Vec<(FeatureId, &Solid)> = contacts
            .iter()
            .filter_map(|(body, contact)| match contact {
                Contact::Overlapping(overlap) => Some((*body, overlap)),
                Contact::Touching => None,
            })
            .collect();
        match overlaps.as_slice() {
            [] => contacts.last().map(|(body, _)| *body),
            [(body, _)] => Some(*body),
            [_, _, ..] => overlaps
                .iter()
                .map(|(body, overlap)| (*body, overlap_volume(overlap)))
                .fold(
                    None,
                    |most: Option<(FeatureId, f64)>, (body, volume)| match most {
                        Some((_, largest)) if largest > volume * (1.0 + SAME_VOLUME) => most,
                        Some(_) | None => Some((body, volume)),
                    },
                )
                .map(|(body, _)| body),
        }
    }
}

fn chosen_regions(curves: &[ProfileCurve], choice: &RegionChoice) -> Option<Vec<Region>> {
    let profile = Profile::new(curves).ok()?;
    let selection = match choice {
        RegionChoice::All => Selection::EvenDepth,
        RegionChoice::Chosen(references) => {
            Selection::Regions(resolve_regions(references, profile.regions()).ok()?.keys)
        }
    };
    profile.select(&selection).ok()
}

fn revolution_axis(
    evaluation: &Evaluation,
    sketch_feature: FeatureId,
    sketch: &Sketch,
    plane: &Plane,
    axis: &RevolveAxis,
) -> Option<Axis2> {
    match axis {
        RevolveAxis::Sketch(line) => match line.reference() {
            Some(Reference::HorizontalAxis) => Axis2::new(Point2::ZERO, Vector2::X).ok(),
            Some(Reference::VerticalAxis) => Axis2::new(Point2::ZERO, Vector2::Y).ok(),
            Some(Reference::Origin) => None,
            None => sketch
                .line_endpoints(*line)
                .and_then(|(start, end)| Axis2::through(start, end).ok()),
        },
        RevolveAxis::Model(reference) => {
            let ray = displayed_axis(evaluation, sketch_feature, reference)?;
            Axis2::through(
                plane.to_local(ray.origin()),
                plane.to_local(ray.origin() + ray.direction()),
            )
            .ok()
        }
    }
}

fn overlap_volume(overlap: &Solid) -> f64 {
    let size = overlap
        .bounding_box()
        .map_or(0.0, |bounds| bounds.diagonal());
    mass_properties_tessellating(overlap, &SamplingTolerance::for_extent(size)).map_or_else(
        |_| {
            overlap.bounding_box().map_or(0.0, |bounds| {
                let size = bounds.max() - bounds.min();
                size.x * size.y * size.z
            })
        },
        |mass| mass.properties.volume,
    )
}

fn boxes_meet(first: &Aabb, second: &Aabb) -> bool {
    let (first_min, first_max) = (first.min(), first.max());
    let (second_min, second_max) = (second.min(), second.max());
    let apart = |low: Point3, high: Point3| {
        low.x > high.x + BOX_SLACK || low.y > high.y + BOX_SLACK || low.z > high.z + BOX_SLACK
    };
    !apart(first_min, second_max) && !apart(second_min, first_max)
}
