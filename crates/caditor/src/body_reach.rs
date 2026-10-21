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
    AngularExtent, Axis2, BooleanError, BooleanOperation, Interference, Interrupt,
    LINEAR_RESOLUTION, LinearExtent, Profile, ProfileCurve, Region, SamplingTolerance, Selection,
    Solid, boolean, extrude, interference, interruptible, mass_properties_tessellating,
    resolve_regions, revolve, wall_regions,
};
use caditor_sketch::{Reference, Sketch};

use crate::{editing, visibility};

pub const REACH_BUDGET: Duration = Duration::from_millis(150);
const OUTLINE_SEGMENT_ANGLE: f64 = 0.02;
const OUTLINE_SLACK_PER_SIZE: f64 = 1e-3;
const BOX_SLACK: f64 = 100.0 * LINEAR_RESOLUTION;
const SAME_VOLUME: f64 = 1e-6;
const SLAB_PER_SIZE: f64 = 1e-3;
const SLAB_FLOOR: f64 = 1e3 * LINEAR_RESOLUTION;

pub fn reached_body(
    document: &Document,
    evaluation: &Evaluation,
    parameters: &ParameterValues,
    solid: &SolidFeature,
) -> Option<FeatureId> {
    let deadline = Instant::now() + REACH_BUDGET;
    reaches(document, evaluation, parameters, solid, deadline, None)?
        .most()
        .map(|reached| reached.body)
}

pub fn reaches(
    document: &Document,
    evaluation: &Evaluation,
    parameters: &ParameterValues,
    solid: &SolidFeature,
    deadline: Instant,
    footprint_of: Option<FeatureId>,
) -> Option<Reaches> {
    let sketch = solved_sketch(document, evaluation, solid.sketch())?;
    let swept = Swept::of(evaluation, parameters, solid, sketch)?;
    let reach = swept.bounds(sketch)?;
    let shown: Vec<Standing<'_>> = document
        .bodies_standing_in(evaluation)
        .into_iter()
        .filter(|body| visibility::is_shown(document, *body))
        .filter_map(|body| Standing::of(evaluation, body))
        .collect();
    let examined: Vec<FeatureId> = shown.iter().map(|body| body.body).collect();
    let near: Vec<Standing<'_>> = shown
        .into_iter()
        .filter(|body| boxes_meet(&body.bounds, &reach))
        .collect();
    let length = swept.length();
    if near.is_empty() {
        return Some(Reaches {
            examined,
            reached: Vec::new(),
            length,
            tool: None,
        });
    }
    let interrupt: Interrupt = Arc::new(move || Instant::now() >= deadline);
    let (reached, tool) = interruptible(interrupt, || {
        let tool = swept.tool(sketch, solid)?;
        let reached = swept.contacts(&tool, sketch, solid, &near, footprint_of);
        Some((reached, tool))
    })?;
    Some(Reaches {
        examined,
        reached,
        length,
        tool: Some(Arc::new(tool)),
    })
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Touch {
    Overlap { volume: f64 },
    Contact,
    Unsure,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Reached {
    pub body: FeatureId,
    pub touch: Touch,
    pub depth: Option<f64>,
    pub spans_footprint: Option<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Meeting {
    Joins,
    OnlyAlongAnEdge,
    Unchecked,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Verdict {
    Overlaps(Reached),
    Misses,
    Unknown,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Reaches {
    examined: Vec<FeatureId>,
    reached: Vec<Reached>,
    length: Option<f64>,
    tool: Option<Arc<Solid>>,
}

impl Reaches {
    pub fn reaching(&self, target: Option<FeatureId>) -> Option<&Reached> {
        match target {
            Some(body) => self.reached.iter().find(|reached| reached.body == body),
            None => self.most(),
        }
    }

    pub fn meeting(&self, evaluation: &Evaluation, reached: &Reached) -> Meeting {
        let (Touch::Contact, Some(tool)) = (reached.touch, self.tool.as_deref()) else {
            return Meeting::Joins;
        };
        let Some(body) = Standing::of(evaluation, reached.body) else {
            return Meeting::Joins;
        };
        let deadline = Instant::now() + REACH_BUDGET;
        let interrupt: Interrupt = Arc::new(move || Instant::now() >= deadline);
        let joined = interruptible(interrupt, || {
            boolean(tool, body.solid, BooleanOperation::Union)
        });
        match joined {
            Err(BooleanError::NonManifold(_)) => Meeting::OnlyAlongAnEdge,
            Err(BooleanError::Cancelled(_)) => Meeting::Unchecked,
            Ok(_) | Err(_) => Meeting::Joins,
        }
    }

    pub fn verdict(&self, body: FeatureId) -> Verdict {
        if !self.examined.contains(&body) {
            return Verdict::Unknown;
        }
        match self.reached.iter().find(|reached| reached.body == body) {
            Some(reached) => match reached.touch {
                Touch::Overlap { .. } => Verdict::Overlaps(*reached),
                Touch::Unsure => Verdict::Unknown,
                Touch::Contact => Verdict::Misses,
            },
            None => Verdict::Misses,
        }
    }

    pub fn clears(&self, reached: &Reached) -> bool {
        match (reached.depth, self.length) {
            (Some(depth), Some(length)) => depth + BOX_SLACK < length,
            _ => false,
        }
    }

    fn most(&self) -> Option<&Reached> {
        let largest = self
            .reached
            .iter()
            .fold(None, |most: Option<(&Reached, f64)>, reached| {
                let volume = match reached.touch {
                    Touch::Overlap { volume } => volume,
                    Touch::Contact | Touch::Unsure => return most,
                };
                match most {
                    Some((_, largest)) if largest > volume * (1.0 + SAME_VOLUME) => most,
                    Some(_) | None => Some((reached, volume)),
                }
            });
        match largest {
            Some((reached, _)) => Some(reached),
            None => self.reached.last(),
        }
    }
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

#[derive(Clone, Copy)]
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
            SolidFeature::Sweep(_) => return None,
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
        self.tool_over(sketch, solid, self.motion)
    }

    fn tool_over(&self, sketch: &Sketch, solid: &SolidFeature, motion: Motion) -> Option<Solid> {
        let curves: Vec<ProfileCurve> = sketch
            .entities()
            .filter(|(id, _)| !sketch.is_construction(*id))
            .filter_map(|(id, _)| profile_curve(sketch, id))
            .collect();
        let regions = match (self.wall, solid.wall()) {
            (Some(thickness), Some(wall)) => wall_regions(&curves, thickness, wall.side).ok()?,
            _ => chosen_regions(&curves, solid.regions())?,
        };
        self.swept_regions(&regions, solid.sketch().raw(), motion)
    }

    fn swept_regions(&self, regions: &[Region], feature: u64, motion: Motion) -> Option<Solid> {
        match motion {
            Motion::Along { start, end } => extrude(
                &self.plane,
                regions,
                LinearExtent::two_sided(end, -start).ok()?,
                feature,
            )
            .ok(),
            Motion::About(axis) => {
                revolve(&self.plane, regions, axis, AngularExtent::full(), feature).ok()
            }
        }
    }

    fn spans_footprint(
        &self,
        sketch: &Sketch,
        solid: &SolidFeature,
        body: &Standing<'_>,
    ) -> Option<bool> {
        let Motion::Along { start, end } = self.motion else {
            return None;
        };
        let thickness = (body.bounds.diagonal() * SLAB_PER_SIZE).max(SLAB_FLOOR);
        let slab = match end > -start {
            true => Motion::Along {
                start: 0.0,
                end: thickness,
            },
            false => Motion::Along {
                start: -thickness,
                end: 0.0,
            },
        };
        let footprint = Aabb2::from_points(
            body.bounds
                .corners()
                .into_iter()
                .map(|corner| self.plane.to_local(corner)),
        )?
        .expanded(BOX_SLACK);
        let mut around = Sketch::new(self.plane);
        let corners = [
            footprint.min(),
            Point2::new(footprint.max().x, footprint.min().y),
            footprint.max(),
            Point2::new(footprint.min().x, footprint.max().y),
        ];
        let curves: Vec<ProfileCurve> = (0..corners.len())
            .filter_map(|index| {
                let line = around.add_line(*corners.get(index)?, *corners.get((index + 1) % 4)?);
                profile_curve(&around, line)
            })
            .collect();
        let whole = chosen_regions(&curves, &RegionChoice::All)?;
        let feature = solid.sketch().raw();
        let cover = self.swept_regions(&whole, feature, slab)?;
        let profile = self.tool_over(sketch, solid, slab)?;
        let reach_of = |tool: &Solid| match interference(tool, body.solid) {
            Ok(Interference::Overlapping(overlap)) => Aabb2::from_points(
                overlap
                    .bounding_box()?
                    .corners()
                    .map(|corner| self.plane.to_local(corner)),
            ),
            Ok(Interference::Apart | Interference::Touching(_)) | Err(_) => None,
        };
        let (taken, available) = (reach_of(&profile), reach_of(&cover));
        let tolerance = BOX_SLACK + footprint.size().length() * OUTLINE_SLACK_PER_SIZE;
        match (taken, available) {
            (Some(taken), Some(available)) => Some(
                (taken.min() - available.min()).abs().max_element() <= tolerance
                    && (taken.max() - available.max()).abs().max_element() <= tolerance,
            ),
            (None, Some(_)) => Some(false),
            (_, None) => None,
        }
    }

    fn contacts(
        &self,
        tool: &Solid,
        sketch: &Sketch,
        solid: &SolidFeature,
        near: &[Standing<'_>],
        footprint_of: Option<FeatureId>,
    ) -> Vec<Reached> {
        let touches: Vec<(&Standing<'_>, Result<Solid, Touch>)> = near
            .iter()
            .filter_map(|body| match interference(tool, body.solid) {
                Ok(Interference::Apart) => None,
                Ok(Interference::Overlapping(overlap)) => Some((body, Ok(overlap))),
                Ok(Interference::Touching(_)) => Some((body, Err(Touch::Contact))),
                Err(_) => Some((body, Err(Touch::Unsure))),
            })
            .collect();
        let several = touches
            .iter()
            .filter(|(_, overlap)| overlap.is_ok())
            .count()
            > 1;
        touches
            .into_iter()
            .map(|(body, overlap)| {
                let overlap_is_solid = overlap.is_ok();
                Reached {
                    body: body.body,
                    touch: match overlap {
                        Ok(overlap) if several => Touch::Overlap {
                            volume: overlap_volume(&overlap),
                        },
                        Ok(_) => Touch::Overlap { volume: 0.0 },
                        Err(touch) => touch,
                    },
                    depth: self.depth_of(&body.bounds),
                    spans_footprint: match (footprint_of == Some(body.body), overlap_is_solid) {
                        (true, true) => self.spans_footprint(sketch, solid, body),
                        _ => None,
                    },
                }
            })
            .collect()
    }

    fn length(&self) -> Option<f64> {
        match self.motion {
            Motion::Along { start, end } => Some(end - start),
            Motion::About(_) => None,
        }
    }

    fn depth_of(&self, bounds: &Aabb) -> Option<f64> {
        let Motion::Along { start, end } = self.motion else {
            return None;
        };
        let direction = match end > -start {
            true => self.plane.normal(),
            false => -self.plane.normal(),
        };
        bounds
            .corners()
            .into_iter()
            .map(|corner| (corner - self.plane.origin()).dot(direction))
            .reduce(f64::max)
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
