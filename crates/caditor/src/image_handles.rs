use caditor_document::{
    FeatureId, FeatureKind, ImagePlane, ImageSize, ReferenceImage, Transaction, displayed_plane,
    placed_on,
};
use caditor_expression::{Dimension, Expression};
use caditor_geometry::{Plane, Point2, Point3, Ray, Vector2, Vector3};
use caditor_kernel::MAX_SIZE;
use caditor_render::{Batch, Layer, Line, Marker, Stroke, View};

use crate::{
    feature_fields::{POSITION_CAPTIONS, Rule},
    handle_snap::Snap,
    image_panel,
    manipulator::{self, Held},
    model::Model,
    move_manipulator::{HIT_POINTS, Handle, SHAFT_WIDTH, segment_distance, step_for},
    place_handles::{Slide, draw_square, square_around, square_holds},
    scene,
    scene_palette::ScenePalette,
    units::Units,
};

const STEM_POINTS: f64 = 44.0;
const KNOB_DIAMETER: f32 = 14.0;
const TURN_STEP_DEGREES: f64 = 5.0;
const FULL_TURN_DEGREES: f64 = 360.0;
const HALF_TURN_DEGREES: f64 = 180.0;
const SMALLEST_SHARE: f64 = 0.01;
const SMALLEST_RADIUS: f64 = 1e-9;
const LEAST_SPOT_GAP: f64 = 1e-6;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageGrip {
    Move,
    Turn,
    Scale,
}

impl ImageGrip {
    pub fn words(self) -> &'static str {
        match self {
            Self::Move => "Drag to move the picture on its plane",
            Self::Turn => "Drag to turn the picture about its middle",
            Self::Scale => "Drag to size the picture, keeping its shape",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Values {
    at: Point2,
    turn: f64,
    width: f64,
    size: f64,
}

fn committed(model: &Model, feature: FeatureId) -> Option<&ReferenceImage> {
    match &model.document().feature(feature)?.kind {
        FeatureKind::ReferenceImage(image) => Some(image),
        _ => None,
    }
}

fn shown(model: &Model, feature: FeatureId) -> Option<&ReferenceImage> {
    match model.draft_kind(feature) {
        Some(FeatureKind::ReferenceImage(image)) => Some(image),
        _ => committed(model, feature),
    }
}

fn values(model: &Model, feature: FeatureId, image: &ReferenceImage) -> Option<Values> {
    let parameters = model.shown_parameters(feature);
    let evaluate = |expression: &Expression, dimension: Dimension| {
        expression
            .evaluate_as(dimension, &|id| parameters.value(id))
            .ok()
    };
    let [x, y] = image
        .at
        .each_ref()
        .map(|expression| evaluate(expression, Dimension::LENGTH));
    let turn = evaluate(&image.turn, Dimension::ANGLE)?;
    let size = evaluate(image.size.expression(), Dimension::LENGTH)?;
    let width = match &image.size {
        ImageSize::Width(_) => size,
        ImageSize::Calibrated(calibration) => {
            let gap = calibration.spot_gap(image.picture.size().aspect());
            (gap > LEAST_SPOT_GAP).then(|| size / gap)?
        }
    };
    (turn.is_finite() && width.is_finite() && width > 0.0).then_some(Values {
        at: Point2::new(x?, y?),
        turn,
        width,
        size,
    })
}

fn base_plane(model: &Model, feature: FeatureId, image: &ReferenceImage) -> Option<Plane> {
    let evaluation = model
        .draft_evaluation_of(feature)
        .unwrap_or_else(|| model.evaluation());
    match &image.plane {
        ImagePlane::Reference(reference) => displayed_plane(evaluation, feature, reference),
        ImagePlane::Sketch(sketch) => scene::sketch_plane(model.document(), evaluation, *sketch),
    }
}

fn size_caption(size: &ImageSize) -> &'static str {
    match size {
        ImageSize::Width(_) => image_panel::WIDTH,
        ImageSize::Calibrated(_) => image_panel::DISTANCE,
    }
}

fn held_position(model: &Model, feature: FeatureId, image: &ReferenceImage, index: usize) -> Held {
    match (POSITION_CAPTIONS.get(index), image.at.get(index)) {
        (Some(caption), Some(expression)) => {
            Held::of(model.document(), feature, caption, expression)
        }
        _ => Held::Typed,
    }
}

fn held_turn(model: &Model, feature: FeatureId, image: &ReferenceImage) -> Held {
    Held::of(model.document(), feature, image_panel::ANGLE, &image.turn)
}

fn held_size(model: &Model, feature: FeatureId, image: &ReferenceImage) -> Held {
    Held::of(
        model.document(),
        feature,
        size_caption(&image.size),
        image.size.expression(),
    )
}

fn per_point_at(view: &View, pixels_per_point: f64, point: Point3) -> Option<f64> {
    let depth = view.view_depth(point);
    let per_point = pixels_per_point * view.units_per_pixel_at(depth);
    (view.is_drawn_at(depth) && per_point.is_finite() && per_point > 0.0).then_some(per_point)
}

fn degrees_of(offset: Vector2) -> Option<f64> {
    (offset.length() > SMALLEST_RADIUS).then(|| offset.y.atan2(offset.x).to_degrees())
}

fn wrapped(degrees: f64) -> f64 {
    let turned = (degrees + HALF_TURN_DEGREES).rem_euclid(FULL_TURN_DEGREES);
    turned - HALF_TURN_DEGREES
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ImageHandles {
    pub feature: FeatureId,
    plane: Plane,
    frame: Plane,
    forward: Vector3,
    centre: Point3,
    corner: Point3,
    stem: [Point3; 2],
    per_point: [f64; 3],
    values: Values,
    calibrated: bool,
}

impl ImageHandles {
    pub fn of(
        model: &Model,
        feature: FeatureId,
        view: &View,
        pixels_per_point: f64,
    ) -> Option<Self> {
        let image = shown(model, feature)?;
        let values = values(model, feature, image)?;
        let plane = base_plane(model, feature, image)?;
        let aspect = image.picture.size().aspect();
        let placed = placed_on(
            &plane,
            values.at,
            values.turn,
            values.width,
            aspect,
            image.flipped,
        );
        let frame = placed.plane()?;
        let centre = placed.centre;
        let corner =
            centre + placed.across * (placed.width / 2.0) + placed.up * (placed.height / 2.0);
        let top = centre + placed.up * (placed.height / 2.0);
        let per_point = [
            per_point_at(view, pixels_per_point, centre)?,
            per_point_at(view, pixels_per_point, corner)?,
            per_point_at(view, pixels_per_point, top)?,
        ];
        let knob = top + placed.up * STEM_POINTS * per_point[2];
        Some(Self {
            feature,
            plane,
            frame,
            forward: view.forward(),
            centre,
            corner,
            stem: [top, knob],
            per_point,
            values,
            calibrated: image.size.calibration().is_some(),
        })
    }

    fn square(&self, grip: ImageGrip) -> Option<[Point3; 4]> {
        let (point, per_point) = match grip {
            ImageGrip::Move => (self.centre, self.per_point[0]),
            ImageGrip::Scale => (self.corner, self.per_point[1]),
            ImageGrip::Turn => return None,
        };
        square_around(&self.frame, point, per_point, self.forward)
    }

    fn turns(&self) -> bool {
        self.frame.normal().dot(self.forward).abs() >= 0.2
    }

    pub fn step(&self) -> f64 {
        step_for(self.per_point[1])
    }

    pub fn driven(&self, model: &Model, handle: Handle) -> Option<String> {
        let Handle::Image(grip) = handle else {
            return None;
        };
        let image = committed(model, self.feature)?;
        match grip {
            ImageGrip::Move => (0..image.at.len())
                .find_map(|index| held_position(model, self.feature, image, index).driven()),
            ImageGrip::Turn => held_turn(model, self.feature, image).driven(),
            ImageGrip::Scale => held_size(model, self.feature, image).driven(),
        }
    }

    pub fn typing(&self, grip: ImageGrip) -> (String, Dimension, Vec<Rule>) {
        let [x, y] = POSITION_CAPTIONS;
        match grip {
            ImageGrip::Move => (format!("{x}, {y}"), Dimension::LENGTH, vec![Rule::Any; 2]),
            ImageGrip::Turn => (
                image_panel::ANGLE.to_owned(),
                Dimension::ANGLE,
                vec![Rule::Any],
            ),
            ImageGrip::Scale => {
                let caption = if self.calibrated {
                    image_panel::DISTANCE
                } else {
                    image_panel::WIDTH
                };
                (caption.to_owned(), Dimension::LENGTH, vec![Rule::AboveZero])
            }
        }
    }

    pub fn hit(&self, view: &View, cursor: Vector2, pixels_per_point: f64) -> Option<Handle> {
        let reach = HIT_POINTS * pixels_per_point;
        let on_stem = self.turns()
            && view
                .project(self.stem[0])
                .zip(view.project(self.stem[1]))
                .is_some_and(|(from, to)| segment_distance(from, to, cursor) <= reach);
        let inside = |grip: ImageGrip| {
            self.square(grip)
                .is_some_and(|corners| square_holds(&corners, view, cursor))
        };
        if on_stem {
            Some(Handle::Image(ImageGrip::Turn))
        } else if inside(ImageGrip::Scale) {
            Some(Handle::Image(ImageGrip::Scale))
        } else if inside(ImageGrip::Move) {
            Some(Handle::Image(ImageGrip::Move))
        } else {
            None
        }
    }

    #[cfg(test)]
    pub fn grip(&self, handle: Handle, _along: f64) -> Option<Point3> {
        match handle {
            Handle::Image(ImageGrip::Move) => Some(self.centre),
            Handle::Image(ImageGrip::Turn) => Some(self.stem[1]),
            Handle::Image(ImageGrip::Scale) => Some(self.corner),
            _ => None,
        }
    }

    pub fn add_to(&self, batch: &mut Batch, palette: &ScenePalette, highlighted: Option<Handle>) {
        let colour = |grip: ImageGrip| {
            if highlighted == Some(Handle::Image(grip)) {
                palette.handle_highlighted
            } else {
                palette.handle
            }
        };
        for grip in [ImageGrip::Move, ImageGrip::Scale] {
            if let Some(corners) = self.square(grip) {
                draw_square(batch, &corners, colour(grip));
            }
        }
        if self.turns() {
            let turning = colour(ImageGrip::Turn);
            batch.lines.push(Line {
                start: self.stem[0],
                end: self.stem[1],
                color: turning,
                width: SHAFT_WIDTH,
                layer: Layer::Front,
                pick: None,
                stroke: Stroke::Solid,
            });
            batch.markers.push(Marker {
                position: self.stem[1],
                color: turning,
                diameter: KNOB_DIAMETER,
                layer: Layer::Front,
                pick: None,
            });
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ImageDrag {
    start: ReferenceImage,
    grip: ImageGrip,
    slide: Slide,
    from: Values,
    now: Values,
    diagonal: Vector2,
    grabbed: Point2,
    last_degrees: f64,
    swept: f64,
    step: f64,
    most_width: f64,
}

impl ImageDrag {
    pub fn begin(model: &Model, handles: &ImageHandles, grip: ImageGrip, ray: Ray) -> Option<Self> {
        let start = committed(model, handles.feature)?.clone();
        let from = values(model, handles.feature, &start)?;
        let step = handles.step();
        let slide = Slide::begin(handles.plane, ray, step, from.at, None)?;
        let grabbed = handles
            .plane
            .to_local(ray.at(ray.intersect_plane(&handles.plane)?));
        let diagonal = (handles.plane.to_local(handles.corner) - from.at).try_normalize()?;
        let last_degrees = degrees_of(grabbed - from.at).unwrap_or_default();
        let most_width = MAX_SIZE / start.picture.size().aspect().max(1.0);
        Some(Self {
            start,
            grip,
            slide,
            from,
            now: from,
            diagonal,
            grabbed,
            last_degrees,
            swept: 0.0,
            step,
            most_width,
        })
    }

    fn pointed_at(&self, ray: Ray, free: bool, snap: Option<Snap>) -> Option<Point2> {
        let plane = self.slide.plane();
        let landed = snap.filter(|_| !free).and_then(|snap| snap.on_plane(plane));
        landed.or_else(|| {
            ray.intersect_plane(plane)
                .map(|distance| plane.to_local(ray.at(distance)))
        })
    }

    pub fn follow(&mut self, ray: Ray, free: bool, snap: Option<Snap>) -> bool {
        let before = self.now;
        match self.grip {
            ImageGrip::Move => {
                self.slide.follow(ray, free, snap);
                self.now.at = self.slide.at();
            }
            ImageGrip::Turn => self.follow_turn(ray, free, snap),
            ImageGrip::Scale => self.follow_scale(ray, free, snap),
        }
        self.now != before
    }

    fn follow_turn(&mut self, ray: Ray, free: bool, snap: Option<Snap>) {
        let Some(pointed) = self.pointed_at(ray, free, snap) else {
            return;
        };
        let Some(degrees) = degrees_of(pointed - self.from.at) else {
            return;
        };
        self.swept += wrapped(degrees - self.last_degrees);
        self.last_degrees = degrees;
        let wanted = self.from.turn + self.swept;
        let landed = snap.is_some_and(|snap| !free && snap.on_plane(self.slide.plane()).is_some());
        self.now.turn = if free || landed {
            wanted
        } else {
            (wanted / TURN_STEP_DEGREES).round() * TURN_STEP_DEGREES
        };
    }

    fn follow_scale(&mut self, ray: Ray, free: bool, snap: Option<Snap>) {
        let Some(pointed) = self.pointed_at(ray, free, snap) else {
            return;
        };
        let held = (self.grabbed - self.from.at).dot(self.diagonal);
        if held.abs() <= SMALLEST_RADIUS {
            return;
        }
        let ratio = (pointed - self.from.at).dot(self.diagonal) / held;
        let wanted = self.from.width * ratio;
        let landed = snap.is_some_and(|snap| !free && snap.on_plane(self.slide.plane()).is_some());
        let width = if free || landed {
            wanted
        } else {
            (wanted / self.step).round() * self.step
        };
        let width = width
            .max(self.from.width * SMALLEST_SHARE)
            .min(self.most_width);
        self.now.width = width;
        self.now.size = self.from.size * width / self.from.width;
    }

    pub fn has_moved(&self) -> bool {
        self.now != self.from
    }

    pub fn transaction(&self, model: &Model, feature: FeatureId) -> Option<Transaction> {
        let unit = model.units().length;
        let mut moved = self.start.clone();
        let mut named = Vec::new();
        match self.grip {
            ImageGrip::Move => {
                let positions = [
                    (self.from.at.x, self.now.at.x),
                    (self.from.at.y, self.now.at.y),
                ];
                for (index, (from, to)) in positions.into_iter().enumerate() {
                    if from == to {
                        continue;
                    }
                    if let Some(slot) = moved.at.get_mut(index) {
                        held_position(model, feature, &self.start, index).set(
                            slot,
                            unit.measured(to),
                            &mut named,
                        );
                    }
                }
            }
            ImageGrip::Turn => held_turn(model, feature, &self.start).set(
                &mut moved.turn,
                model.units().angle.measured(self.now.turn),
                &mut named,
            ),
            ImageGrip::Scale => held_size(model, feature, &self.start).set(
                moved.size.expression_mut(),
                unit.measured(self.now.size),
                &mut named,
            ),
        }
        manipulator::keeping_names(
            model.document(),
            feature,
            FeatureKind::ReferenceImage(Box::new(moved)),
            named,
        )
    }

    pub fn readout(&self, units: Units) -> String {
        match self.grip {
            ImageGrip::Move => {
                let [x, y] = POSITION_CAPTIONS;
                let shown = format!(
                    "{x} {}, {y} {}",
                    units.readout_text(self.now.at.x),
                    units.readout_text(self.now.at.y)
                );
                match self.slide.snapped() {
                    Some(snap) => format!("{shown} {}", snap.kind.words()),
                    None => shown,
                }
            }
            ImageGrip::Turn => format!(
                "{} {}",
                image_panel::ANGLE,
                units.angle_readout(self.now.turn)
            ),
            ImageGrip::Scale => format!(
                "{} {}",
                image_panel::WIDTH,
                units.readout_text(self.now.width)
            ),
        }
    }
}

pub fn typed(
    model: &Model,
    feature: FeatureId,
    grip: ImageGrip,
    values: &[Expression],
) -> Option<Transaction> {
    let image = committed(model, feature)?;
    let mut moved = image.clone();
    let mut named = Vec::new();
    match grip {
        ImageGrip::Move => {
            for (index, value) in values.iter().take(2).enumerate() {
                if let Some(slot) = moved.at.get_mut(index) {
                    held_position(model, feature, image, index).set(
                        slot,
                        value.clone(),
                        &mut named,
                    );
                }
            }
        }
        ImageGrip::Turn => held_turn(model, feature, image).set(
            &mut moved.turn,
            values.first()?.clone(),
            &mut named,
        ),
        ImageGrip::Scale => held_size(model, feature, image).set(
            moved.size.expression_mut(),
            values.first()?.clone(),
            &mut named,
        ),
    }
    manipulator::keeping_names(
        model.document(),
        feature,
        FeatureKind::ReferenceImage(Box::new(moved)),
        named,
    )
}
