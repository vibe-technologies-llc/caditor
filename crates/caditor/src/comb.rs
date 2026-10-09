use std::sync::Arc;

use caditor_document::profile_curve;
use caditor_geometry::{Aabb, Point3, Vector3};
use caditor_kernel::{Curve, Interval};
use caditor_render::{Batch, Color, Layer, Line, Stroke};

use crate::{
    bodies,
    model::Model,
    scene,
    scene_palette::ScenePalette,
    selection::{Pickable, Selection},
};

pub const DEFAULT_TEETH: usize = 40;
pub const MIN_TEETH: usize = 4;
pub const MAX_TEETH: usize = 200;
pub const DEFAULT_SCALE: f64 = 1.0;
pub const MIN_SCALE: f64 = 0.1;
pub const MAX_SCALE: f64 = 10.0;
pub const MOST_COMBED: usize = 32;
pub const LONGEST_TOOTH: f64 = 0.25;
const STEPS_PER_TOOTH: usize = 4;
const JOINT_GAP: f64 = 1e-3;
const TANGENT_SINE: f64 = 1.745e-3;
const CURVATURE_SLACK: f64 = 1e-2;
const FLAT_CURVATURE: f64 = 1e-9;
const HALO_WIDTH: f32 = 2.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Tooth {
    pub at: Point3,
    pub tangent: Vector3,
    pub curvature: Vector3,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Combed {
    pub pickable: Pickable,
    pub name: String,
    pub teeth: Vec<Tooth>,
}

impl Combed {
    pub fn greatest_curvature(&self) -> f64 {
        self.teeth
            .iter()
            .map(|tooth| tooth.curvature.length())
            .fold(0.0, f64::max)
    }

    pub fn smallest_radius(&self) -> Option<f64> {
        let greatest = self.greatest_curvature();
        (greatest > FLAT_CURVATURE).then(|| 1.0 / greatest)
    }

    fn end(&self, end: End) -> Option<&Tooth> {
        match end {
            End::Start => self.teeth.first(),
            End::Finish => self.teeth.last(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum End {
    Start,
    Finish,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Continuity {
    Corner,
    Tangent,
    Curvature,
}

impl Continuity {
    pub fn grade(self) -> &'static str {
        match self {
            Self::Corner => "G0",
            Self::Tangent => "G1",
            Self::Curvature => "G2",
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Self::Corner => "Meet at a corner",
            Self::Tangent => "Tangent, but the curvature jumps",
            Self::Curvature => "Curvature continues",
        }
    }

    fn between(first: &Tooth, first_end: End, second: &Tooth, second_end: End) -> Self {
        let leaving = |tooth: &Tooth, end: End| match end {
            End::Start => tooth.tangent,
            End::Finish => -tooth.tangent,
        };
        let (out, back) = (leaving(first, first_end), leaving(second, second_end));
        let opposed = out.dot(back) < 0.0 && out.cross(back).length() <= TANGENT_SINE;
        if !opposed {
            return Self::Corner;
        }
        let jump = (first.curvature - second.curvature).length();
        let largest = first.curvature.length().max(second.curvature.length());
        if jump <= CURVATURE_SLACK * largest + FLAT_CURVATURE {
            Self::Curvature
        } else {
            Self::Tangent
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Joint {
    pub at: Point3,
    pub curves: [usize; 2],
    pub ends: [End; 2],
    pub continuity: Continuity,
    pub curvatures: [f64; 2],
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Comb {
    pub curves: Vec<Combed>,
    pub joints: Vec<Joint>,
    pub gone: usize,
}

impl Comb {
    fn of(model: &Model, curves: &[Pickable], teeth: usize) -> Self {
        let mut comb = Self::default();
        for pickable in curves {
            match teeth_of(model, *pickable, teeth) {
                Some(found) if !found.is_empty() => comb.curves.push(Combed {
                    pickable: *pickable,
                    name: pickable.describe(model.document(), model.evaluation()),
                    teeth: found,
                }),
                _ => comb.gone += 1,
            }
        }
        comb.joints = joints(&comb.curves);
        comb
    }

    pub fn greatest_curvature(&self) -> f64 {
        self.curves
            .iter()
            .map(Combed::greatest_curvature)
            .fold(0.0, f64::max)
    }

    fn size(&self) -> f64 {
        Aabb::from_points(
            self.curves
                .iter()
                .flat_map(|curve| curve.teeth.iter().map(|tooth| tooth.at)),
        )
        .map_or(0.0, |bounds| bounds.diagonal())
    }

    pub fn reach(&self, scale: f64) -> f64 {
        let greatest = self.greatest_curvature();
        if greatest > FLAT_CURVATURE {
            LONGEST_TOOTH * self.size() * scale / greatest
        } else {
            0.0
        }
    }

    pub fn drawing(&self, scale: f64) -> CombDrawing {
        let reach = self.reach(scale);
        let tip = |tooth: &Tooth| tooth.at - tooth.curvature * reach;
        let mut drawing = CombDrawing::default();
        for curve in &self.curves {
            drawing.teeth.extend(
                curve
                    .teeth
                    .iter()
                    .filter(|tooth| tooth.curvature.length() > FLAT_CURVATURE)
                    .map(|tooth| [tooth.at, tip(tooth)]),
            );
            drawing
                .envelope
                .extend(curve.teeth.windows(2).filter_map(|pair| match pair {
                    [from, to] => Some([tip(from), tip(to)]),
                    _ => None,
                }));
        }
        for joint in &self.joints {
            let [first, second] = joint.curves;
            let [first_end, second_end] = joint.ends;
            let tips = self
                .curves
                .get(first)
                .and_then(|curve| curve.end(first_end))
                .zip(
                    self.curves
                        .get(second)
                        .and_then(|curve| curve.end(second_end)),
                );
            if let Some((from, to)) = tips {
                let step = [tip(from), tip(to)];
                if step[0].distance(step[1]) > 0.0 {
                    drawing.envelope.push(step);
                }
            }
        }
        drawing
    }
}

fn teeth_of(model: &Model, pickable: Pickable, teeth: usize) -> Option<Vec<Tooth>> {
    let evaluation = model.evaluation();
    match pickable {
        Pickable::Edge { body, edge } => {
            let shown = bodies::shown(evaluation, body)?;
            let id = bodies::find_edge(shown, edge)?;
            let edge = shown.solid.edge(id)?;
            Some(spaced_teeth(edge.curve(), edge.interval(), teeth))
        }
        Pickable::SketchEntity { feature, entity } => {
            let owner = model.document().feature(feature)?;
            let sketch = model.displayed_sketch(owner)?;
            if sketch.point(entity).is_some() {
                return None;
            }
            let plane = scene::sketch_plane(model.document(), evaluation, feature)?;
            let (curve, interval) = profile_curve(&sketch, entity)?.curve().ok()?;
            let curve = curve.on_plane(&plane).ok()?;
            Some(spaced_teeth(&curve, interval, teeth))
        }
        _ => None,
    }
}

fn tooth_at(curve: &Curve, parameter: f64) -> Option<Tooth> {
    let derivatives = curve.evaluate(parameter);
    Some(Tooth {
        at: derivatives.point,
        tangent: derivatives.first.normalize_or_zero(),
        curvature: derivatives.curvature()?,
    })
}

pub fn spaced_teeth(curve: &Curve, interval: Interval, teeth: usize) -> Vec<Tooth> {
    let teeth = teeth.max(1);
    let steps = teeth * STEPS_PER_TOOTH;
    let parameter_at =
        |step: usize| interval.start() + interval.length() * step as f64 / steps as f64;
    let points: Vec<Point3> = (0..=steps)
        .map(|step| curve.point(parameter_at(step)))
        .collect();
    let mut travelled = Vec::with_capacity(points.len());
    let mut total = 0.0;
    travelled.push(total);
    for pair in points.windows(2) {
        if let [from, to] = pair {
            total += from.distance(*to);
        }
        travelled.push(total);
    }
    let parameter_along = |tooth: usize| {
        if tooth == 0 {
            return interval.start();
        }
        if tooth >= teeth {
            return interval.end();
        }
        if total <= 0.0 {
            return interval.start() + interval.length() * tooth as f64 / teeth as f64;
        }
        let target = total * tooth as f64 / teeth as f64;
        let after = travelled
            .partition_point(|length| *length < target)
            .clamp(1, steps);
        let (Some(low), Some(high)) = (travelled.get(after - 1), travelled.get(after)) else {
            return interval.end();
        };
        let span = high - low;
        let fraction = if span > 0.0 {
            ((target - low) / span).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let below = parameter_at(after - 1);
        below + (parameter_at(after) - below) * fraction
    };
    (0..=teeth)
        .filter_map(|tooth| tooth_at(curve, parameter_along(tooth)))
        .collect()
}

fn joints(curves: &[Combed]) -> Vec<Joint> {
    let mut joints = Vec::new();
    for (first, one) in curves.iter().enumerate() {
        for (second, other) in curves.iter().enumerate().skip(first + 1) {
            for first_end in [End::Start, End::Finish] {
                for second_end in [End::Start, End::Finish] {
                    let (Some(from), Some(to)) = (one.end(first_end), other.end(second_end)) else {
                        continue;
                    };
                    if from.at.distance(to.at) > JOINT_GAP {
                        continue;
                    }
                    joints.push(Joint {
                        at: from.at,
                        curves: [first, second],
                        ends: [first_end, second_end],
                        continuity: Continuity::between(from, first_end, to, second_end),
                        curvatures: [from.curvature.length(), to.curvature.length()],
                    });
                }
            }
        }
    }
    joints
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct CombDrawing {
    pub teeth: Vec<[Point3; 2]>,
    pub envelope: Vec<[Point3; 2]>,
}

impl CombDrawing {
    pub fn add_to(&self, batch: &mut Batch, palette: &ScenePalette) {
        let look = palette.comb;
        let line = |[start, end]: [Point3; 2], color: Color, width: f32| Line {
            start,
            end,
            color,
            width,
            layer: Layer::Front,
            pick: None,
            stroke: Stroke::Solid,
        };
        let halos = self
            .teeth
            .iter()
            .map(|segment| line(*segment, palette.hole, look.tooth_width + HALO_WIDTH))
            .chain(
                self.envelope
                    .iter()
                    .map(|segment| line(*segment, palette.hole, look.envelope_width + HALO_WIDTH)),
            );
        let teeth = self
            .teeth
            .iter()
            .map(|segment| line(*segment, look.teeth, look.tooth_width));
        let envelope = self
            .envelope
            .iter()
            .map(|segment| line(*segment, look.envelope, look.envelope_width));
        batch.lines.extend(halos.chain(teeth).chain(envelope));
    }
}

#[derive(Debug, Clone, PartialEq)]
struct Basis {
    curves: Vec<Pickable>,
    teeth: usize,
    revision: u64,
    evaluation: u64,
    sketches: u64,
}

pub struct CombTool {
    pub open: bool,
    pub teeth: usize,
    pub scale: f64,
    curves: Vec<Pickable>,
    left_out: usize,
    followed: Option<u64>,
    combed: Option<(Basis, Arc<Comb>)>,
    drawn: Option<(f64, Arc<Comb>, Arc<CombDrawing>)>,
}

impl Default for CombTool {
    fn default() -> Self {
        Self {
            open: false,
            teeth: DEFAULT_TEETH,
            scale: DEFAULT_SCALE,
            curves: Vec::new(),
            left_out: 0,
            followed: None,
            combed: None,
            drawn: None,
        }
    }
}

pub fn is_combable(model: &Model, pickable: Pickable) -> bool {
    match pickable {
        Pickable::Edge { .. } => true,
        Pickable::SketchEntity { feature, entity } => model
            .document()
            .feature(feature)
            .and_then(|owner| model.displayed_sketch(owner))
            .is_some_and(|sketch| profile_curve(&sketch, entity).is_some()),
        _ => false,
    }
}

impl CombTool {
    pub fn toggle(&mut self) {
        if self.open {
            *self = Self {
                teeth: self.teeth,
                scale: self.scale,
                ..Self::default()
            };
        } else {
            self.open = true;
        }
    }

    pub fn forget(&mut self) {
        self.curves.clear();
        self.left_out = 0;
        self.followed = None;
        self.combed = None;
        self.drawn = None;
    }

    pub fn curves(&self) -> &[Pickable] {
        &self.curves
    }

    pub fn left_out(&self) -> usize {
        self.left_out
    }

    pub fn follow(&mut self, model: &Model, selection: &Selection) {
        if self.followed == Some(selection.generation()) {
            return;
        }
        self.followed = Some(selection.generation());
        let chosen: Vec<Pickable> = selection
            .in_pick_order()
            .into_iter()
            .filter(|pickable| is_combable(model, *pickable))
            .collect();
        if chosen.is_empty() {
            return;
        }
        self.left_out = chosen.len().saturating_sub(MOST_COMBED);
        self.curves = chosen.into_iter().take(MOST_COMBED).collect();
    }

    pub fn comb(&mut self, model: &Model) -> Arc<Comb> {
        let basis = Basis {
            curves: self.curves.clone(),
            teeth: self.teeth.clamp(MIN_TEETH, MAX_TEETH),
            revision: model.revision(),
            evaluation: model.evaluation_generation(),
            sketches: model.display().sketches.generation(),
        };
        if let Some((combed_basis, comb)) = &self.combed
            && *combed_basis == basis
        {
            return Arc::clone(comb);
        }
        let comb = Arc::new(Comb::of(model, &basis.curves, basis.teeth));
        self.combed = Some((basis, Arc::clone(&comb)));
        comb
    }

    pub fn drawing(&mut self, comb: &Arc<Comb>) -> Arc<CombDrawing> {
        let scale = self.scale.clamp(MIN_SCALE, MAX_SCALE);
        if let Some((drawn_scale, drawn_comb, drawing)) = &self.drawn
            && *drawn_scale == scale
            && Arc::ptr_eq(drawn_comb, comb)
        {
            return Arc::clone(drawing);
        }
        let drawing = Arc::new(comb.drawing(scale));
        self.drawn = Some((scale, Arc::clone(comb), Arc::clone(&drawing)));
        drawing
    }
}

#[cfg(test)]
mod tests {
    use caditor_geometry::Plane;
    use caditor_kernel::{BSplineCurve, Circle, Line};

    use super::*;

    fn arc(radius: f64) -> (Curve, Interval) {
        (
            Circle::new(Plane::XY, radius).unwrap().into(),
            Interval::new(0.0, std::f64::consts::FRAC_PI_2).unwrap(),
        )
    }

    fn combed(curve: &Curve, interval: Interval, teeth: usize) -> Combed {
        Combed {
            pickable: Pickable::Origin,
            name: String::new(),
            teeth: spaced_teeth(curve, interval, teeth),
        }
    }

    #[test]
    fn an_arc_gets_evenly_spaced_teeth_of_its_curvature_from_end_to_end() {
        let (curve, interval) = arc(5.0);

        let teeth = spaced_teeth(&curve, interval, 10);

        assert_eq!(teeth.len(), 11);
        assert!(
            teeth
                .iter()
                .all(|tooth| (tooth.curvature.length() - 0.2).abs() < 1e-12)
        );
        assert!(
            teeth
                .first()
                .unwrap()
                .at
                .distance(Point3::new(5.0, 0.0, 0.0))
                < 1e-12
        );
        assert!(
            teeth
                .last()
                .unwrap()
                .at
                .distance(Point3::new(0.0, 5.0, 0.0))
                < 1e-12
        );
        let gaps: Vec<f64> = teeth
            .windows(2)
            .map(|pair| pair[0].at.distance(pair[1].at))
            .collect();
        assert!(gaps.iter().all(|gap| (gap - gaps[0]).abs() < 1e-3));
    }

    #[test]
    fn a_spline_is_combed_at_even_lengths_along_it_whatever_its_parameter() {
        let spline: Curve = BSplineCurve::new(
            2,
            vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
            vec![
                Point3::new(-1.0, 1.0, 0.0),
                Point3::new(0.0, -1.0, 0.0),
                Point3::new(1.0, 1.0, 0.0),
            ],
        )
        .unwrap()
        .into();

        let teeth = spaced_teeth(&spline, Interval::UNIT, 20);
        let middle = teeth.get(10).unwrap();
        let gaps: Vec<f64> = teeth
            .windows(2)
            .map(|pair| pair[0].at.distance(pair[1].at))
            .collect();

        assert!((middle.curvature - Vector3::new(0.0, 2.0, 0.0)).length() < 1e-9);
        let (shortest, longest) = gaps
            .iter()
            .fold((f64::INFINITY, 0.0_f64), |(low, high), gap| {
                (low.min(*gap), high.max(*gap))
            });
        assert!(longest / shortest < 1.02, "{shortest} {longest}");
    }

    #[test]
    fn the_longest_tooth_points_away_from_the_centre_by_a_quarter_of_the_size_times_the_scale() {
        let (curve, interval) = arc(5.0);
        let comb = Comb {
            curves: vec![combed(&curve, interval, 8)],
            joints: Vec::new(),
            gone: 0,
        };
        let size = 5.0 * std::f64::consts::SQRT_2;

        let drawing = comb.drawing(2.0);
        let [at, tip] = *drawing.teeth.first().unwrap();

        assert_eq!(drawing.teeth.len(), 9);
        assert_eq!(drawing.envelope.len(), 8);
        assert!((at.distance(tip) - 2.0 * LONGEST_TOOTH * size).abs() < 1e-9);
        assert!(tip.x > at.x);
    }

    #[test]
    fn a_line_meeting_an_arc_is_tangent_and_the_envelope_steps_across_the_jump() {
        let (curve, interval) = arc(5.0);
        let line: Curve = Line::new(Point3::new(5.0, -10.0, 0.0), Vector3::Y)
            .unwrap()
            .into();
        let curves = vec![
            combed(&line, Interval::new(0.0, 10.0).unwrap(), 4),
            combed(&curve, interval, 4),
        ];

        let found = joints(&curves);
        let comb = Comb {
            curves,
            joints: found.clone(),
            gone: 0,
        };
        let drawing = comb.drawing(1.0);

        assert_eq!(found.len(), 1);
        let joint = found.first().unwrap();
        assert_eq!(joint.continuity, Continuity::Tangent);
        assert_eq!(joint.ends, [End::Finish, End::Start]);
        assert!(joint.at.distance(Point3::new(5.0, 0.0, 0.0)) < 1e-9);
        assert_eq!(drawing.teeth.len(), 5);
        assert_eq!(drawing.envelope.len(), 4 + 4 + 1);
    }

    #[test]
    fn two_arcs_of_one_circle_continue_their_curvature_and_a_kink_is_a_corner() {
        let circle: Curve = Circle::new(Plane::XY, 5.0).unwrap().into();
        let quarter = std::f64::consts::FRAC_PI_2;
        let smooth = vec![
            combed(&circle, Interval::new(0.0, quarter).unwrap(), 4),
            combed(&circle, Interval::new(quarter, 2.0 * quarter).unwrap(), 4),
        ];
        let line: Curve = Line::new(Point3::new(5.0, 0.0, 0.0), Vector3::X)
            .unwrap()
            .into();
        let kinked = vec![
            combed(&circle, Interval::new(0.0, quarter).unwrap(), 4),
            combed(&line, Interval::new(0.0, 3.0).unwrap(), 4),
        ];

        let continuities = |curves: &[Combed]| -> Vec<Continuity> {
            joints(curves)
                .iter()
                .map(|joint| joint.continuity)
                .collect()
        };

        assert_eq!(continuities(&smooth), [Continuity::Curvature]);
        assert_eq!(continuities(&kinked), [Continuity::Corner]);
    }
}
