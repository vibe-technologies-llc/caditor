use caditor_document::{FeatureId, Transaction};
use caditor_expression::{Dimension, Expression, Unit};
use caditor_geometry::Point2;
use caditor_sketch::{
    Bevel, ChamferSize, Corner, Dimensioned, Entity, EntityId, Faceting, FilletError, Rounding,
    Sketch,
};

use crate::{
    drawing::Preview,
    editing::Tool,
    feature_tree::count,
    field::Expected,
    model::Model,
    modifying::{Hint, Outcome, Prompt, Value, ValueField, length_text},
    patterning,
    snap::{self, Pointer, Screen},
    trimming::{self, capitalized},
    typed_point,
    units::LengthUnit,
};

pub const CORNER_PROMPT: &str =
    "Click the corner to round, where two lines or arcs meet, or drag from it";
pub const RADIUS_PROMPT: &str = "Click where the fillet should pass, or type its radius";
pub const CHAMFER_CORNER_PROMPT: &str =
    "Click the corner to cut, where two lines or arcs meet, or drag from it";
pub const DISTANCE_PROMPT: &str = "Click where the chamfer should pass, or type how far from the corner it cuts: 5, or 5, 3 for a different distance on each curve, or 5 < 45 for a distance and an angle";
pub const TRANSACTION: &str = "Fillet corner";
pub const CHAMFER_TRANSACTION: &str = "Chamfer corner";
pub const CORNERS_TRANSACTION: &str = "Fillet corners";
pub const CHAMFER_CORNERS_TRANSACTION: &str = "Chamfer corners";
const RADIUS_KEYS: &str = "Type the radius   Enter: round it here   Esc: choose another corner";
const DISTANCE_KEYS: &str = "Type a distance, two distances, or a distance < angle   Enter: cut it here   Esc: choose another corner";
const TYPE_RADIUS: &str = "Type the radius";
const TYPE_DISTANCE: &str = "Type a distance, two distances, or a distance < angle";
const CHOOSE_ANOTHER: &str = "Esc: choose another corner";
const NO_CORNER_HIGHLIGHTED: &str =
    "Highlight a corner first, with Highlight the next item in the view";
const NOTHING_TO_ROUND: &str = "The sketch has no corner where two lines or arcs meet";
const CHOOSE_FIRST: &str = "Choose the corner first";
const SAME_CORNER: f64 = 1e-6;
pub const FIELD: ValueField = ValueField {
    label: "Fillet radius",
    placeholder: "radius",
    action: "round the corner",
};
pub const CHAMFER_FIELD: ValueField = ValueField {
    label: "Chamfer",
    placeholder: "5  or  5, 3  or  5 < 45",
    action: "cut the corner",
};
const CHAMFER_FORMS: &str = "Type a distance such as 5, two distances such as 5, 3, or a distance and an angle such as 5 < 45";
const LENGTH: Expected = Expected {
    dimension: Some(Dimension::LENGTH),
    non_negative: false,
};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum CornerCut {
    #[default]
    Round,
    Chamfer,
}

impl CornerCut {
    fn tool(self) -> Tool {
        match self {
            Self::Round => Tool::Fillet,
            Self::Chamfer => Tool::Chamfer,
        }
    }

    fn field(self) -> ValueField {
        match self {
            Self::Round => FIELD,
            Self::Chamfer => CHAMFER_FIELD,
        }
    }

    fn typing(self) -> &'static str {
        match self {
            Self::Round => TYPE_RADIUS,
            Self::Chamfer => TYPE_DISTANCE,
        }
    }

    fn verb(self) -> &'static str {
        match self {
            Self::Round => "round",
            Self::Chamfer => "cut",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Cut {
    Round(Rounding),
    Bevel(Bevel),
}

impl Cut {
    fn size(self) -> f64 {
        match self {
            Self::Round(rounding) => rounding.radius,
            Self::Bevel(bevel) => bevel.distances[0],
        }
    }

    fn touches(self) -> [Point2; 2] {
        match self {
            Self::Round(rounding) => rounding.touches,
            Self::Bevel(bevel) => bevel.touches,
        }
    }

    fn faceted(self, faceting: Faceting) -> Vec<Point2> {
        match self {
            Self::Round(rounding) => rounding.faceted(faceting),
            Self::Bevel(bevel) => bevel.touches.to_vec(),
        }
    }
}

#[derive(Debug, Clone)]
enum Size {
    Radius(Value),
    Chamfer(ChamferSize),
}

#[derive(Debug, Clone)]
enum Shown {
    Radius(f64),
    Chamfer(ChamferSize),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct LeftOut {
    count: usize,
    reason: String,
}

impl LeftOut {
    fn words(&self) -> String {
        format!(
            "{}: {}",
            count(
                self.count,
                "selected item left out",
                "selected items left out"
            ),
            self.reason
        )
    }
}

#[derive(Debug, Clone, Default)]
pub struct Filleting {
    cut: CornerCut,
    chosen: Option<Corner>,
    more: Vec<Corner>,
    left_out: Option<LeftOut>,
    hover: Option<Result<Corner, FilletError>>,
    highlight: Option<Corner>,
    pointer: Option<Point2>,
    typed: Option<f64>,
    typed_chamfer: Option<Result<ChamferSize, String>>,
    rounding: Option<Result<Vec<Cut>, FilletError>>,
    last: Option<String>,
}

impl Filleting {
    pub fn new(cut: CornerCut, last: Option<String>) -> Self {
        Self {
            cut,
            last,
            ..Self::default()
        }
    }

    pub fn starting(
        sketch: &Sketch,
        selected: &[EntityId],
        cut: CornerCut,
        last: Option<String>,
    ) -> Self {
        let (corners, left_out) = gathered(sketch, selected);
        let mut corners = corners.into_iter();
        Self {
            cut,
            chosen: corners.next(),
            more: corners.collect(),
            left_out,
            last,
            ..Self::default()
        }
    }

    pub fn cut(&self) -> CornerCut {
        self.cut
    }

    pub fn last(&self) -> Option<&str> {
        self.last.as_deref()
    }

    pub fn field(&self) -> ValueField {
        self.cut.field()
    }

    pub fn sync(&mut self, sketch: &Sketch) {
        let still = |corner: &Corner| {
            sketch
                .corner_at(corner.point)
                .is_ok_and(|now| now.curves == corner.curves)
        };
        self.chosen = self.chosen.filter(still);
        self.highlight = self.highlight.filter(still);
        self.more.retain(still);
        if self.chosen.is_none() && !self.more.is_empty() {
            self.chosen = Some(self.more.remove(0));
        }
    }

    fn corners(&self) -> Vec<Corner> {
        let more = self.chosen.map(|_| self.more.clone()).unwrap_or_default();
        self.target().into_iter().chain(more).collect()
    }

    pub fn hover(&mut self, sketch: &Sketch, screen: &impl Screen, pointer: Option<Pointer>) {
        self.pointer = pointer.map(|pointer| pointer.sketch);
        self.hover = match self.chosen {
            Some(_) => None,
            None => pointer
                .and_then(|pointer| end_under(sketch, screen, pointer))
                .map(|point| sketch.corner_at(point)),
        };
        let aim = self.chosen.zip(self.pointer);
        let shown = match self.cut {
            CornerCut::Round => self
                .typed
                .or_else(|| {
                    aim.and_then(|(chosen, pointer)| sketch.radius_through(&chosen, pointer))
                })
                .map(Shown::Radius),
            CornerCut::Chamfer => match &self.typed_chamfer {
                Some(Ok(size)) => Some(Shown::Chamfer(size.clone())),
                Some(Err(_)) => None,
                None => aim
                    .and_then(|(chosen, pointer)| sketch.distance_through(&chosen, pointer))
                    .map(|distance| Shown::Chamfer(pointed(distance))),
            },
        };
        let corners = self.corners();
        self.rounding = shown.filter(|_| !corners.is_empty()).map(|shown| {
            corners
                .iter()
                .map(|corner| match &shown {
                    Shown::Radius(radius) => sketch.rounding(corner, *radius).map(Cut::Round),
                    Shown::Chamfer(size) => sketch.bevel(corner, size).map(Cut::Bevel),
                })
                .collect()
        });
    }

    pub fn leave(&mut self) {
        self.pointer = None;
        self.hover = None;
        if self.typed.is_none() && self.typed_chamfer.is_none() {
            self.rounding = None;
        }
    }

    fn reused(&self, text: &str) -> String {
        match self.last.as_deref() {
            Some(last) if text.trim().is_empty() => last.to_owned(),
            _ => text.trim().to_owned(),
        }
    }

    pub fn show_text(&mut self, model: &Model, text: Option<&str>) {
        let text = text.map(|text| self.reused(text));
        match self.cut {
            CornerCut::Round => {
                self.typed = text
                    .and_then(|text| Value::typed(model, &text).ok())
                    .map(|value| value.millimetres);
            }
            CornerCut::Chamfer => {
                self.typed_chamfer = text
                    .filter(|text| !text.is_empty())
                    .map(|text| parse_chamfer(model, &text));
            }
        }
    }

    pub fn enter_text(
        &mut self,
        model: &Model,
        feature: FeatureId,
        text: &str,
    ) -> Result<Outcome, String> {
        let text = self.reused(text);
        let size = match self.cut {
            CornerCut::Round => Size::Radius(Value::typed(model, &text)?),
            CornerCut::Chamfer => Size::Chamfer(parse_chamfer(model, &text)?),
        };
        self.round(model, feature, size, text).map(Outcome::Apply)
    }

    fn target(&self) -> Option<Corner> {
        self.chosen
            .or(self.highlight)
            .or_else(|| self.hover.as_ref()?.as_ref().ok().copied())
    }

    pub fn preview(&self, faceting: Faceting) -> Preview {
        let mut preview = Preview::default();
        if let Some(corner) = self.target() {
            preview.snap = Some(corner.position);
        }
        if let Some(Ok(cuts)) = &self.rounding {
            for cut in cuts {
                preview.curves.push(cut.faceted(faceting));
                preview.points.extend(cut.touches());
            }
        }
        preview
    }

    pub fn highlighted_entities(&self) -> Vec<EntityId> {
        self.corners()
            .into_iter()
            .flat_map(|corner| corner.curves)
            .collect()
    }

    pub fn label(&self, sketch: &Sketch, unit: LengthUnit) -> Option<String> {
        let corners = self.corners();
        let left_out = self.left_out.as_ref().map(LeftOut::words);
        let subject = match (corners.as_slice(), &self.hover) {
            ([], Some(Err(error))) => return Some(capitalized(&error.to_string())),
            ([], _) => return left_out.map(|words| capitalized(&words)),
            ([corner], _) => {
                let [first, second] = corner.curves.map(|curve| sketch.entity_label(curve));
                format!(
                    "{} the corner of {first} and {second}",
                    capitalized(self.cut.verb())
                )
            }
            (many, _) => format!(
                "{} {}",
                capitalized(self.cut.verb()),
                count(many.len(), "corner", "corners")
            ),
        };
        let said = match &self.rounding {
            Some(Ok(cuts)) => match cuts.first() {
                Some(Cut::Round(rounding)) => format!(
                    "{subject} with a radius of {}",
                    length_text(unit, rounding.radius)
                ),
                Some(Cut::Bevel(bevel)) => format!("{subject} {}", bevel_words(unit, bevel)),
                None => subject,
            },
            Some(Err(error)) => capitalized(&error.to_string()),
            None => match &self.typed_chamfer {
                Some(Err(reason)) if self.cut == CornerCut::Chamfer => capitalized(reason),
                _ => subject,
            },
        };
        Some(match left_out.filter(|_| self.chosen.is_some()) {
            Some(words) => format!("{said}; {words}"),
            None => said,
        })
    }

    pub fn prompt(&self) -> Prompt {
        let them = if self.more.is_empty() { "it" } else { "them" };
        match (self.chosen.is_some(), self.cut, self.last.as_deref()) {
            (true, cut, Some(last)) => Prompt {
                text: match cut {
                    CornerCut::Round => RADIUS_PROMPT,
                    CornerCut::Chamfer => DISTANCE_PROMPT,
                },
                hint: Hint::Text(format!(
                    "{}   Enter: {} {them} with {last}, as last time   {CHOOSE_ANOTHER}",
                    cut.typing(),
                    cut.verb()
                )),
            },
            (true, CornerCut::Round, None) => Prompt {
                text: RADIUS_PROMPT,
                hint: Hint::Keys(RADIUS_KEYS),
            },
            (true, CornerCut::Chamfer, None) => Prompt {
                text: DISTANCE_PROMPT,
                hint: Hint::Keys(DISTANCE_KEYS),
            },
            (false, CornerCut::Round, _) => Prompt {
                text: CORNER_PROMPT,
                hint: Hint::Targets,
            },
            (false, CornerCut::Chamfer, _) => Prompt {
                text: CHAMFER_CORNER_PROMPT,
                hint: Hint::Targets,
            },
        }
    }

    pub fn click(&mut self, model: &Model, feature: FeatureId) -> Outcome {
        if self.chosen.is_some() {
            return self.round_at_pointer(model, feature);
        }
        match self.hover.clone() {
            Some(Ok(corner)) => {
                self.choose(corner);
                self.hover = None;
                Outcome::Nothing
            }
            Some(Err(error)) => {
                Outcome::Refused(trimming::refusal(self.cut.tool(), &error.to_string()))
            }
            None => Outcome::Nothing,
        }
    }

    fn choose(&mut self, corner: Corner) {
        let known = |other: &Corner| same_corner(other, &corner);
        match self.chosen {
            None => {
                self.chosen = Some(corner);
                self.more.clear();
                self.left_out = None;
            }
            Some(chosen) if known(&chosen) => {}
            Some(_) if self.more.iter().any(known) => {}
            Some(_) => self.more.push(corner),
        }
    }

    pub fn begin_pull(&mut self) -> bool {
        if self.chosen.is_none()
            && let Some(Ok(corner)) = self.hover.clone()
        {
            self.choose(corner);
        }
        self.chosen.is_some()
    }

    pub fn activate(&mut self, model: &Model, feature: FeatureId) -> Outcome {
        if let Some(corner) = self.highlight.take() {
            self.choose(corner);
            return Outcome::Nothing;
        }
        if self.chosen.is_none() {
            return Outcome::Nothing;
        }
        match self.last.clone() {
            Some(last) => self
                .enter_text(model, feature, &last)
                .unwrap_or_else(Outcome::Refused),
            None => self.round_at_pointer(model, feature),
        }
    }

    fn round_at_pointer(&mut self, model: &Model, feature: FeatureId) -> Outcome {
        let size = match &self.rounding {
            Some(Ok(cuts)) => cuts.first().map(|cut| cut.size()),
            Some(Err(error)) => {
                return Outcome::Refused(trimming::refusal(self.cut.tool(), &error.to_string()));
            }
            None => None,
        };
        let Some(value) = size.and_then(|size| Value::pointed(model, size)) else {
            return Outcome::Nothing;
        };
        let text = length_text(model.length_unit(), value.millimetres);
        self.round(model, feature, Size::Radius(value), text).into()
    }

    fn round(
        &mut self,
        model: &Model,
        feature: FeatureId,
        size: Size,
        text: String,
    ) -> Result<Transaction, String> {
        let cut = self.cut;
        let corners = self.corners();
        if corners.is_empty() {
            return Err(trimming::refusal(cut.tool(), CHOOSE_FIRST));
        }
        let label = match (cut, corners.len()) {
            (CornerCut::Round, 1) => TRANSACTION,
            (CornerCut::Round, _) => CORNERS_TRANSACTION,
            (CornerCut::Chamfer, 1) => CHAMFER_TRANSACTION,
            (CornerCut::Chamfer, _) => CHAMFER_CORNERS_TRANSACTION,
        };
        let refused = |error: FilletError| trimming::refusal(cut.tool(), &error.to_string());
        let rounded = trimming::reshaped(model, feature, label.to_owned(), |sketch| {
            for corner in &corners {
                let current = sketch.corner_at(corner.point).map_err(refused)?;
                match (cut, &size) {
                    (CornerCut::Round, Size::Radius(value)) => sketch
                        .fillet(&current, value.millimetres, value.expression.clone())
                        .map(|_| ()),
                    (CornerCut::Chamfer, Size::Radius(value)) => {
                        sketch.chamfer(&current, &equal(value)).map(|_| ())
                    }
                    (_, Size::Chamfer(size)) => sketch.chamfer(&current, size).map(|_| ()),
                }
                .map_err(refused)?;
            }
            Ok(())
        })?;
        self.chosen = None;
        self.more.clear();
        self.left_out = None;
        self.highlight = None;
        self.rounding = None;
        self.typed_chamfer = None;
        self.last = Some(text);
        Ok(rounded)
    }

    pub fn steppable(&self, sketch: &Sketch) -> Result<(), &'static str> {
        let curves = sketch
            .entities()
            .filter(|(_, entity)| matches!(entity, Entity::Line { .. } | Entity::Arc { .. }))
            .take(2)
            .count();
        if curves == 2 {
            Ok(())
        } else {
            Err(NOTHING_TO_ROUND)
        }
    }

    pub fn step(&mut self, sketch: &Sketch, step: isize) {
        let corners = sketch.fillet_corners();
        let count = corners.len() as isize;
        if count == 0 {
            return;
        }
        let current = self.highlight.or(self.chosen).and_then(|aim| {
            corners
                .iter()
                .position(|corner| corner.curves == aim.curves && corner.position == aim.position)
        });
        let next = match current {
            Some(index) => (index as isize + step).rem_euclid(count),
            None if step < 0 => count - 1,
            None => 0,
        };
        self.highlight = usize::try_from(next)
            .ok()
            .and_then(|next| corners.get(next).copied());
    }

    pub fn highlight_needed(&self) -> Result<(), &'static str> {
        match self.highlight {
            Some(_) => Ok(()),
            None => Err(NO_CORNER_HIGHLIGHTED),
        }
    }

    pub fn clear_highlight(&mut self) {
        self.highlight = None;
    }

    pub fn can_back_out(&self) -> bool {
        self.highlight.is_some() || self.chosen.is_some()
    }

    pub fn back_out(&mut self) {
        if self.highlight.take().is_none() && self.chosen.take().is_some() {
            self.more.clear();
            self.left_out = None;
            self.rounding = None;
        }
    }
}

fn same_corner(first: &Corner, second: &Corner) -> bool {
    let mut one = first.curves;
    let mut other = second.curves;
    one.sort();
    other.sort();
    one == other && first.position.distance(second.position) <= SAME_CORNER
}

fn gathered(sketch: &Sketch, selected: &[EntityId]) -> (Vec<Corner>, Option<LeftOut>) {
    let mut corners: Vec<Corner> = Vec::new();
    let mut refused: Vec<String> = Vec::new();
    let mut take = |corner: Corner| {
        if !corners.iter().any(|known| same_corner(known, &corner)) {
            corners.push(corner);
        }
    };
    let (points, curves): (Vec<EntityId>, Vec<EntityId>) = selected
        .iter()
        .copied()
        .partition(|entity| sketch.point(*entity).is_some());
    for point in points {
        match sketch.corner_at(point) {
            Ok(corner) => take(corner),
            Err(error) => refused.push(error.to_string()),
        }
    }
    sketch
        .fillet_corners()
        .into_iter()
        .filter(|corner| corner.curves.iter().all(|curve| curves.contains(curve)))
        .for_each(&mut take);
    for curve in curves {
        if !corners.iter().any(|corner| corner.curves.contains(&curve)) {
            refused.push(format!(
                "{} meets no other selected line or arc at a corner",
                sketch.entity_label(curve)
            ));
        }
    }
    let left_out = refused.first().map(|reason| LeftOut {
        count: refused.len(),
        reason: reason.clone(),
    });
    (corners, left_out)
}

fn end_under(sketch: &Sketch, screen: &impl Screen, pointer: Pointer) -> Option<EntityId> {
    sketch
        .entities()
        .flat_map(|(_, entity)| match entity {
            Entity::Line { start, end }
            | Entity::Arc { start, end, .. }
            | Entity::EllipticalArc { start, end, .. } => vec![*start, *end],
            spline @ Entity::Spline { .. } => spline
                .spline_ends()
                .map_or_else(Vec::new, |(first, last)| vec![first, last]),
            Entity::Point(_) | Entity::Circle { .. } | Entity::Ellipse { .. } => Vec::new(),
        })
        .filter_map(|point| {
            let offset = screen
                .to_screen(sketch.point(point)?)?
                .distance(pointer.screen);
            (offset <= snap::POINT_TOLERANCE).then_some((offset, point))
        })
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, point)| point)
}

fn equal(value: &Value) -> ChamferSize {
    ChamferSize::Equal(Dimensioned {
        expression: value.expression.clone(),
        value: value.millimetres,
    })
}

fn pointed(millimetres: f64) -> ChamferSize {
    ChamferSize::Equal(Dimensioned {
        expression: Expression::Measure(millimetres, Unit::Millimetre),
        value: millimetres,
    })
}

fn bevel_words(unit: LengthUnit, bevel: &Bevel) -> String {
    let [first, second] = bevel.distances.map(|distance| length_text(unit, distance));
    if first == second {
        format!("{first} from it on each side")
    } else {
        format!("{first} from it on the first and {second} on the second")
    }
}

fn parse_chamfer(model: &Model, text: &str) -> Result<ChamferSize, String> {
    let distance = |term: &str| patterning::quantity(model, term, LENGTH, "distance");
    let terms = typed_point::coordinates(text);
    match terms.as_slice() {
        [term] => match typed_point::polar(term).as_slice() {
            [only] => Ok(ChamferSize::Equal(distance(only)?)),
            [only, angle] => Ok(ChamferSize::DistanceAndAngle {
                distance: distance(only)?,
                angle: patterning::degrees(model, angle)?,
            }),
            _ => Err(CHAMFER_FORMS.to_owned()),
        },
        [first, second] => {
            let plain = |term: &&str| typed_point::polar(term).len() == 1;
            if !(plain(first) && plain(second)) {
                return Err(CHAMFER_FORMS.to_owned());
            }
            Ok(ChamferSize::Distances {
                first: distance(first)?,
                second: distance(second)?,
            })
        }
        _ => Err(CHAMFER_FORMS.to_owned()),
    }
}
